//! `--log`: what a bug report needs, written to a file the terminal does not own.

use std::fs::OpenOptions;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// The SDK's and gRPC's warnings, and tmprl's own messages at every level.
const DEFAULT_FILTER: &str = "warn,tmprl=info";

pub fn start(path: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .try_init()
        .map_err(std::io::Error::other)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "tmprl started");
    Ok(())
}
