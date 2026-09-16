//! Connecting to a Temporal frontend.
//!
//! Profile resolution is delegated to `ClientOptions::load_from_config`, which is
//! Temporal's own loader: it reads `~/.config/temporalio/temporal.toml`, applies the
//! `TEMPORAL_*` environment variables over it, and resolves TLS material (including
//! reading cert/key files off disk). Reimplementing that would only drift from what
//! the `temporal` CLI does, so we don't.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use temporalio_client::{
    Client, ClientOptions,
    envconfig::{DataSource, LoadClientConfigProfileOptions},
    grpc::{CloudService, OperatorService, WorkflowService},
};

/// Which profile to connect as.
#[derive(Debug, Clone, Default)]
pub struct ProfileRef {
    /// Profile name from the TOML config. `None` uses `TEMPORAL_PROFILE`, else `default`.
    pub name: Option<String>,
    /// Override the config file path. `None` uses `TEMPORAL_CONFIG_FILE`, else the OS default.
    pub config_file: Option<String>,
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

impl Conn {
    pub async fn connect(profile: &ProfileRef) -> Result<Self, ConnectError> {
        let load = LoadClientConfigProfileOptions::builder()
            .maybe_config_file_profile(profile.name.clone())
            .maybe_config_source(config_source(profile.config_file.as_deref()))
            .build();

        let (conn_opts, client_opts) = ClientOptions::load_from_config(load)
            .map_err(|e| ConnectError::Config(e.to_string()))?;
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

    #[test]
    fn config_file_in_use_always_names_something() {
        // `--config-path` has to print a path even when no file exists yet, otherwise it
        // cannot answer "where should I create it".
        assert!(config_file_in_use(None).is_some());
    }
}
