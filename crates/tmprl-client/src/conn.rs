//! Connecting to a Temporal frontend.
//!
//! Profile resolution is delegated to `ClientOptions::load_from_config`, which is
//! Temporal's own loader: it reads `~/.config/temporalio/temporal.toml`, applies the
//! `TEMPORAL_*` environment variables over it, and resolves TLS material (including
//! reading cert/key files off disk). Reimplementing that would only drift from what
//! the `temporal` CLI does, so we don't.

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

/// A live, namespace-bound connection. Cheap to clone — clones share one HTTP/2 channel,
/// which is what makes multi-namespace fan-out cheap.
#[derive(Clone)]
pub struct Conn {
    client: Client,
    profile: Arc<str>,
    namespace: Arc<str>,
}

impl Conn {
    pub async fn connect(profile: &ProfileRef) -> Result<Self, ConnectError> {
        let load = LoadClientConfigProfileOptions::builder()
            .maybe_config_file_profile(profile.name.clone())
            .maybe_config_source(
                profile
                    .config_file
                    .as_ref()
                    .map(|p| DataSource::Path(p.clone())),
            )
            .build();

        let (conn_opts, client_opts) = ClientOptions::load_from_config(load)
            .map_err(|e| ConnectError::Config(e.to_string()))?;
        let namespace: Arc<str> = client_opts.namespace.as_str().into();
        let client = Client::connect(conn_opts, client_opts)
            .await
            .map_err(|e| ConnectError::Connect(e.to_string()))?;

        Ok(Self {
            client,
            profile: profile.name.as_deref().unwrap_or("default").into(),
            namespace,
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn profile(&self) -> &str {
        &self.profile
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
