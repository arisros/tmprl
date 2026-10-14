//! tmprl, a terminal client for Temporal.

mod app;
mod cli;
mod clipboard;
mod config;
mod event;
mod keys;
mod theme;
mod ui;
mod view;

use cli::Cli;
use tmprl_client::Conn;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let (mut profile, startup) = match cli::parse(std::env::args().skip(1)) {
        Ok(Cli::Run { profile, startup }) => (profile, startup),
        Ok(Cli::Help) => {
            print!("{}", cli::USAGE);
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("tmprl {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Cli::ConfigPath(temporal_config)) => {
            print!("{}", config::describe_paths(temporal_config.as_deref()));
            return std::process::ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("tmprl: {e}");
            return std::process::ExitCode::from(2);
        }
    };

    // Named here rather than left to the loader, so `config.toml` is looked up under the
    // profile that is actually connected: `readonly` must hold for `TEMPORAL_PROFILE=prod`.
    if profile.name.is_none() {
        profile.name = std::env::var("TEMPORAL_PROFILE")
            .ok()
            .filter(|p| !p.is_empty());
    }
    // A bad config.toml is reported once the app is up; here it only means no API key from it.
    if let Some(cfg) = config::read("config.toml")
        .ok()
        .flatten()
        .and_then(|src| tmprl_core::config::parse_config(&src).ok())
    {
        let resolved = cfg.resolve(profile.name.as_deref().unwrap_or("default"));
        profile.api_key = resolved.api_key;
        profile.ignore_env = !resolved.env;
    }

    // Connect *before* touching the terminal, so a connection error is an ordinary message
    // on stderr rather than a flash of alternate screen followed by a stack trace.
    let conn = match Conn::connect(&profile).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tmprl: {e}");
            eprintln!("\nIs a server reachable? Try `temporal server start-dev`,");
            eprintln!("or point tmprl at one with `--profile <name>` or `--address <host:port>`.");
            return std::process::ExitCode::FAILURE;
        }
    };

    let (tx, rx) = unbounded_channel();
    let mut app = app::App::new(conn, tx.clone());

    // Config is applied before the terminal is touched, so a bad keys.toml is a plain
    // message on stderr rather than an error flashed behind an alternate screen. A file
    // that exists but cannot be read is reported; an absent one is simply no config.
    let read_config = |name: &str| match config::read(name) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("tmprl: {e}");
            None
        }
    };
    let (keys, views, config, theme, dashboard) = (
        read_config("keys.toml"),
        read_config("views.toml"),
        read_config("config.toml"),
        read_config("theme.toml"),
        read_config("dashboard.toml"),
    );
    // The theme first: both report through the statusline, which holds one message, and a
    // key that does not work matters more than a colour that is off.
    app.apply_theme(theme::depth_from_env(), theme.as_deref());
    app.apply_dashboard(dashboard.as_deref());
    app.apply_config(keys.as_deref(), views.as_deref(), config.as_deref());
    app.start(startup);

    let terminal = ratatui::init();
    let result = event::run(terminal, app, rx, tx).await;
    ratatui::restore();

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tmprl: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
