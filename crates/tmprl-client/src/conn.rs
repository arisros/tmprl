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
use temporalio_common::envconfig::{
    LoadClientConfigOptions, load_client_config, load_client_config_profile,
};
use tmprl_core::config::ApiKey;

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
    /// The API key from tmprl's `config.toml`, over the file's and `TEMPORAL_API_KEY`.
    pub api_key: Option<ApiKey>,
    /// Load the profile without the `TEMPORAL_*` variables layered over it.
    pub ignore_env: bool,
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

/// A profile in `temporal.toml`, as a list of them shows it. Never carries credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    pub name: String,
    pub address: Option<String>,
    pub namespace: Option<String>,
}

/// Every profile in the file the connection would be read from, sorted by name.
///
/// As written in the file: the `TEMPORAL_*` variables are applied when a profile is
/// connected, not here, so the list shows what each profile says rather than what the
/// environment would turn it into.
pub fn list_profiles(config_file: Option<&str>) -> Result<Vec<ProfileSummary>, ConnectError> {
    profiles_from(config_source(config_file))
}

fn profiles_from(source: Option<DataSource>) -> Result<Vec<ProfileSummary>, ConnectError> {
    let options = LoadClientConfigOptions::builder()
        .maybe_config_source(source)
        .build();
    let config =
        load_client_config(options, None).map_err(|e| ConnectError::Config(e.to_string()))?;
    let mut profiles: Vec<ProfileSummary> = config
        .profiles
        .into_iter()
        .map(|(name, p)| ProfileSummary {
            name,
            address: p.address,
            namespace: p.namespace,
        })
        .collect();
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
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

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn")
            .field("profile", &self.profile)
            .field("namespace", &self.namespace)
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
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

/// Run `api_key_cmd` without a shell. Its output is never put in an error: it is the key.
async fn run_api_key_cmd(command: &str) -> Result<String, ConnectError> {
    let home = std::env::var("HOME").ok();
    let (program, args) = tmprl_core::source::argv(command, home.as_deref())
        .ok_or_else(|| ConnectError::Config("api_key_cmd is empty".into()))?;
    let output = tokio::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| ConnectError::Config(format!("api_key_cmd `{program}` did not run: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let why = stderr.lines().next().unwrap_or("").trim();
        return Err(ConnectError::Config(format!(
            "api_key_cmd `{program}` failed ({}){}",
            output.status,
            if why.is_empty() {
                String::new()
            } else {
                format!(": {why}")
            }
        )));
    }
    first_line(&output.stdout)
        .ok_or_else(|| ConnectError::Config(format!("api_key_cmd `{program}` printed no key")))
}

/// `pass show` prints the secret on the first line and metadata after it.
fn first_line(stdout: &[u8]) -> Option<String> {
    let line = String::from_utf8_lossy(stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!line.is_empty()).then_some(line)
}

impl Conn {
    pub async fn connect(profile: &ProfileRef) -> Result<Self, ConnectError> {
        let load = LoadClientConfigProfileOptions::builder()
            .maybe_config_file_profile(profile.name.clone())
            .maybe_config_source(config_source(profile.config_file.as_deref()))
            .disable_env(profile.ignore_env)
            .build();

        // The loader applies the environment, so a flag outranks `TEMPORAL_ADDRESS` and
        // `TEMPORAL_NAMESPACE`, which outrank the file.
        let mut loaded = load_client_config_profile(load, None)
            .map_err(|e| ConnectError::Config(e.to_string()))?;
        match &profile.api_key {
            None => {}
            Some(ApiKey::Literal(key)) => loaded.api_key = Some(key.clone()),
            Some(ApiKey::Command(command)) => {
                loaded.api_key = Some(run_api_key_cmd(command).await?)
            }
        }
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
        let mut profile = ClientConfigProfile::default();
        profile.address = Some("my-ns.a1b2c.tmprl.cloud:7233".into());
        profile.namespace = Some("my-ns.a1b2c".into());
        profile.api_key = Some("secret".into());
        profile
    }

    fn overriding(address: Option<&str>, namespace: Option<&str>) -> ProfileRef {
        ProfileRef {
            address: address.map(str::to_string),
            namespace: namespace.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn the_key_is_the_first_line_and_metadata_after_it_is_dropped() {
        assert_eq!(
            first_line(b"  secret  \nlogin: me\n").as_deref(),
            Some("secret")
        );
        assert_eq!(first_line(b"\n"), None);
        assert_eq!(first_line(b""), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_api_key_cmd_runs_without_a_shell() {
        assert_eq!(
            run_api_key_cmd("printf secret\\nmeta").await.unwrap(),
            "secret"
        );
        // Through a shell `$HOME` would expand; here it is just an argument.
        assert_eq!(run_api_key_cmd("echo $HOME").await.unwrap(), "$HOME");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_api_key_cmd_names_the_program() {
        let err = run_api_key_cmd("tmprl-no-such-program")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("`tmprl-no-such-program` did not run"), "{err}");
        let err = run_api_key_cmd("false").await.unwrap_err().to_string();
        assert!(err.contains("`false` failed"), "{err}");
        let err = run_api_key_cmd("true").await.unwrap_err().to_string();
        assert!(err.contains("printed no key"), "{err}");
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
    fn profiles_are_listed_by_name_without_their_credentials() {
        let toml = br#"
[profile.prod]
address = "prod.example:7233"
namespace = "orders"
api_key = "secret"

[profile.dev]
address = "localhost:7233"
"#;
        let profiles = profiles_from(Some(DataSource::Data(toml.to_vec()))).unwrap();
        assert_eq!(
            profiles,
            vec![
                ProfileSummary {
                    name: "dev".into(),
                    address: Some("localhost:7233".into()),
                    namespace: None,
                },
                ProfileSummary {
                    name: "prod".into(),
                    address: Some("prod.example:7233".into()),
                    namespace: Some("orders".into()),
                },
            ]
        );
    }

    #[test]
    fn a_config_that_does_not_parse_is_an_error_not_an_empty_list() {
        let err = profiles_from(Some(DataSource::Data(b"[profile.dev\n".to_vec())))
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("could not load Temporal profile"), "{err}");
    }

    #[test]
    fn a_listed_profile_never_carries_tls_or_grpc_meta() {
        let toml = br#"
[profile.cloud]
address = "ns.a1b2c.tmprl.cloud:7233"
namespace = "ns.a1b2c"
api_key = "secret"
[profile.cloud.tls]
client_cert_path = "/keys/client.pem"
[profile.cloud.grpc_meta]
authorization = "Bearer secret"
"#;
        let profiles = profiles_from(Some(DataSource::Data(toml.to_vec()))).unwrap();
        let shown = format!("{profiles:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(!shown.contains("client.pem"), "{shown}");
        assert_eq!(profiles[0].namespace.as_deref(), Some("ns.a1b2c"));
    }

    #[test]
    fn an_empty_config_lists_no_profiles() {
        assert_eq!(
            profiles_from(Some(DataSource::Data(Vec::new()))).unwrap(),
            vec![]
        );
    }

    #[test]
    fn config_file_in_use_always_names_something() {
        // `--config-path` has to print a path even when no file exists yet, otherwise it
        // cannot answer "where should I create it".
        assert!(config_file_in_use(None).is_some());
    }
}
