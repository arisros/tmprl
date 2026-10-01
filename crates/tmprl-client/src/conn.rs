//! Connecting to a Temporal frontend.
//!
//! Profile resolution is delegated to Temporal's own loader, the two steps behind
//! `ClientOptions::load_from_config`: it reads `~/.config/temporalio/temporal.toml`,
//! applies the `TEMPORAL_*` environment variables over it, and resolves TLS material
//! (including reading cert/key files off disk). Reimplementing that would only drift from
//! what the `temporal` CLI does, so we don't. The command line's `--address` and
//! `--namespace` are set on the loaded profile between those two steps.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use temporalio_client::{
    Client, ClientOptions, ConnectionOptions,
    envconfig::{ClientConfigProfile, DataSource, LoadClientConfigProfileOptions},
    grpc::{CloudService, OperatorService, WorkflowService},
};
use temporalio_common::envconfig::load_client_config_profile;

/// Which profile to connect as, and what the command line overrides in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileRef {
    /// Profile name from the TOML config. `None` uses `TEMPORAL_PROFILE`, else `default`.
    pub name: Option<String>,
    /// Override the config file path. `None` uses `TEMPORAL_CONFIG_FILE`, else the OS default.
    pub config_file: Option<String>,
    /// Override the profile's server address, `host:port` or a URL. Everything else the
    /// profile says about the connection, TLS and the API key included, still applies.
    pub address: Option<String>,
    /// Override the profile's namespace.
    pub namespace: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// `envconfig::ConfigError` boxes a `dyn Error` source that is not `Sync`, which makes
    /// the whole error unusable across `anyhow` and tokio task boundaries. Flatten it here
    /// so everything above this crate gets a `Send + Sync` error.
    #[error("could not load Temporal profile: {0}")]
    Config(String),
    #[error("could not connect to Temporal: {0}")]
    Connect(String),
}

/// Which file the connection profiles are read from.
///
/// The platform path is authoritative, because it is the one the `temporal` CLI uses:
/// `$HOME/.config` on Unix, `$HOME/Library/Application Support` on macOS. Returning `None`
/// lets the loader apply it, which also preserves the `TEMPORAL_CONFIG_FILE` step in the
/// precedence chain.
///
/// `~/.config/temporalio/temporal.toml` is consulted only as a fallback, and only on a
/// platform where it is not already the default. On macOS that directory is where people
/// reasonably expect the file to live, and a config that is present but silently unread is
/// a bad half hour; matching the CLI still wins whenever the CLI's own file exists.
fn config_source(explicit: Option<&str>) -> Option<DataSource> {
    if let Some(p) = explicit {
        return Some(DataSource::Path(p.to_string()));
    }
    // Set: the loader reads it, and overriding here would silently outrank it.
    if std::env::var_os("TEMPORAL_CONFIG_FILE").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    // The CLI's own file wins whenever it is there.
    if platform_config_file().is_some_and(|p| p.is_file()) {
        return None;
    }
    let path = xdg_config_file()?;
    Path::new(&path)
        .is_file()
        .then(|| DataSource::Path(path.to_string_lossy().into_owned()))
}

/// `$XDG_CONFIG_HOME/temporalio/temporal.toml`, else `~/.config/temporalio/temporal.toml`.
pub fn xdg_config_file() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("temporalio").join("temporal.toml"))
}

/// Where connection profiles will actually be read from, for `--config-path`.
pub fn config_file_in_use(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("TEMPORAL_CONFIG_FILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let platform = platform_config_file();
    if platform.as_ref().is_some_and(|p| p.is_file()) {
        return platform;
    }
    match xdg_config_file() {
        Some(p) if p.is_file() => Some(p),
        // Neither exists: name the platform path, since that is where it should be created.
        _ => platform,
    }
}

/// `temporal.toml` under the platform config directory, the path the CLI documents.
pub fn platform_config_file() -> Option<PathBuf> {
    dirs_config_dir().map(|d| d.join("temporalio").join("temporal.toml"))
}

/// The platform config directory `temporalio-common` itself uses.
fn dirs_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(d) if !d.is_empty() => Some(PathBuf::from(d)),
            _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")),
        }
    }
}

/// A live, namespace-bound connection. Cheap to clone, clones share one HTTP/2 channel,
/// which is what makes multi-namespace fan-out cheap.
#[derive(Clone)]
pub struct Conn {
    client: Client,
    profile: Arc<str>,
    namespace: Arc<str>,
    address: Arc<str>,
}

/// The namespace Temporal's loader falls back to when a profile names none.
const DEFAULT_NAMESPACE: &str = "default";

/// Turn a loaded profile into connection options, with the command line applied over it.
///
/// This is `ClientOptions::load_from_config` taken in its two halves, so the overrides can
/// land between them, on the profile and before Temporal's own conversion. An address set
/// here therefore gets the same treatment as one in the file: a bare `host:port` takes
/// `https` when the profile has TLS or an API key, and both are kept. Patching the built
/// options instead would mean choosing the scheme a second time, differently.
fn options_from(
    mut loaded: ClientConfigProfile,
    profile: &ProfileRef,
) -> Result<(ConnectionOptions, ClientOptions), ConnectError> {
    if let Some(address) = &profile.address {
        loaded.address = Some(address.clone());
    }
    if let Some(namespace) = &profile.namespace {
        loaded.namespace = Some(namespace.clone());
    }
    let namespace = loaded
        .namespace
        .clone()
        .unwrap_or_else(|| DEFAULT_NAMESPACE.to_owned());
    let conn_opts =
        ConnectionOptions::try_from(loaded).map_err(|e| ConnectError::Config(e.to_string()))?;
    Ok((conn_opts, ClientOptions::new(namespace).build()))
}

impl Conn {
    pub async fn connect(profile: &ProfileRef) -> Result<Self, ConnectError> {
        let load = LoadClientConfigProfileOptions::builder()
            .maybe_config_file_profile(profile.name.clone())
            .maybe_config_source(config_source(profile.config_file.as_deref()))
            .build();

        // The loader applies the environment, so a flag outranks `TEMPORAL_ADDRESS` and
        // `TEMPORAL_NAMESPACE`, which outrank the file.
        let loaded = load_client_config_profile(load, None)
            .map_err(|e| ConnectError::Config(e.to_string()))?;
        let (conn_opts, client_opts) = options_from(loaded, profile)?;
        let namespace: Arc<str> = client_opts.namespace.as_str().into();
        // Read before `connect` consumes the options. A namespace name is not unique
        // across clusters, so the audit log needs the target to be readable later.
        let address: Arc<str> = conn_opts.target.to_string().into();
        let client = Client::connect(conn_opts, client_opts)
            .await
            .map_err(|e| ConnectError::Connect(e.to_string()))?;

        Ok(Self {
            client,
            profile: profile.name.as_deref().unwrap_or("default").into(),
            namespace,
            address,
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The frontend this is connected to, as a URL. Never carries credentials: an API key
    /// lives in the connection options, not the target.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Raw `WorkflowService`. Requests take `tonic::Request<T>` and the connection's
    /// retry policy is already applied underneath.
    pub fn wf(&self) -> Box<dyn WorkflowService> {
        self.client.connection().workflow_service()
    }

    pub fn operator(&self) -> Box<dyn OperatorService> {
        self.client.connection().operator_service()
    }

    pub fn cloud(&self) -> Box<dyn CloudService> {
        self.client.connection().cloud_service()
    }

    /// The high-level client, for the handful of operations where `temporalio-client`
    /// already does the assembly work for us (schedules, workflow handles).
    pub fn raw(&self) -> &Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_path_wins() {
        // `--temporal-config` must outrank every discovery rule below it.
        assert!(matches!(
            config_source(Some("/tmp/explicit.toml")),
            Some(DataSource::Path(p)) if p == "/tmp/explicit.toml"
        ));
        assert_eq!(
            config_file_in_use(Some("/tmp/explicit.toml")),
            Some(PathBuf::from("/tmp/explicit.toml"))
        );
    }

    #[test]
    fn the_platform_path_is_the_one_the_cli_documents() {
        // `temporal config --help`: $HOME/.config on Unix, $HOME/Library/Application Support
        // on macOS. tmprl must not disagree with the CLI about which file it is reading.
        let path = platform_config_file().expect("HOME is set in a test run");
        assert!(path.ends_with("temporalio/temporal.toml"), "{path:?}");
        #[cfg(target_os = "macos")]
        assert!(
            path.to_string_lossy()
                .contains("Library/Application Support"),
            "{path:?}"
        );
    }

    #[test]
    fn the_xdg_path_is_only_a_fallback() {
        // Consulted when the CLI's own file is absent, so a config someone put in
        // ~/.config on a Mac is found rather than silently ignored.
        let path = xdg_config_file().expect("HOME is set in a test run");
        assert!(path.ends_with("temporalio/temporal.toml"), "{path:?}");
    }

    fn cloud_profile() -> ClientConfigProfile {
        ClientConfigProfile {
            address: Some("my-ns.a1b2c.tmprl.cloud:7233".into()),
            namespace: Some("my-ns.a1b2c".into()),
            api_key: Some("secret".into()),
            ..Default::default()
        }
    }

    fn overriding(address: Option<&str>, namespace: Option<&str>) -> ProfileRef {
        ProfileRef {
            address: address.map(str::to_string),
            namespace: namespace.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn a_profile_with_no_overrides_connects_as_written() {
        let (conn, client) = options_from(cloud_profile(), &ProfileRef::default()).unwrap();
        assert_eq!(
            conn.target.as_str(),
            "https://my-ns.a1b2c.tmprl.cloud:7233/"
        );
        assert_eq!(client.namespace, "my-ns.a1b2c");
    }

    #[test]
    fn an_address_override_keeps_the_profiles_tls_and_api_key() {
        // `--address` moves the target and nothing else: a port-forward to a cluster that
        // wants the profile's key must still send it, over TLS.
        let over = overriding(Some("localhost:7233"), None);
        let (conn, client) = options_from(cloud_profile(), &over).unwrap();
        assert_eq!(conn.target.as_str(), "https://localhost:7233/");
        assert_eq!(conn.api_key.as_deref(), Some("secret"));
        assert!(conn.tls_options.is_some(), "TLS came from the profile");
        assert_eq!(
            client.namespace, "my-ns.a1b2c",
            "the namespace is untouched"
        );
    }

    #[test]
    fn an_address_override_on_a_plain_profile_stays_plain() {
        let over = overriding(Some("temporal.internal:7233"), None);
        let (conn, _) = options_from(ClientConfigProfile::default(), &over).unwrap();
        assert_eq!(conn.target.as_str(), "http://temporal.internal:7233/");
        assert!(conn.tls_options.is_none());
    }

    #[test]
    fn an_address_with_a_scheme_is_taken_as_written() {
        let over = overriding(Some("http://localhost:7233"), None);
        let (conn, _) = options_from(cloud_profile(), &over).unwrap();
        assert_eq!(conn.target.as_str(), "http://localhost:7233/");
    }

    #[test]
    fn a_namespace_override_replaces_the_profiles() {
        let over = overriding(None, Some("orders"));
        let (conn, client) = options_from(cloud_profile(), &over).unwrap();
        assert_eq!(client.namespace, "orders");
        assert_eq!(
            conn.target.as_str(),
            "https://my-ns.a1b2c.tmprl.cloud:7233/"
        );
    }

    #[test]
    fn a_profile_naming_no_namespace_falls_back_to_default() {
        let (_, client) =
            options_from(ClientConfigProfile::default(), &ProfileRef::default()).unwrap();
        assert_eq!(client.namespace, "default");
    }

    #[test]
    fn config_file_in_use_always_names_something() {
        // `--config-path` has to print a path even when no file exists yet, otherwise it
        // cannot answer "where should I create it".
        assert!(config_file_in_use(None).is_some());
    }
}
