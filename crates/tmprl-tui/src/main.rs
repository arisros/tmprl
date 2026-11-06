//! tmprl, a terminal client for Temporal.

mod app;
mod cli;
mod clipboard;
mod config;
mod event;
mod keys;
mod log;
mod public_api;
mod theme;
mod ui;
mod view;

use cli::Cli;
use tmprl_client::Conn;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let (mut profile, startup) = match cli::parse(std::env::args().skip(1)) {
        Ok(Cli::Run { profile, startup }) => (profile, *startup),
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

    if let Some(path) = &startup.log
        && let Err(e) = log::start(path)
    {
        eprintln!("tmprl: --log {path}: {e}");
        return std::process::ExitCode::FAILURE;
    }

    if profile.name.is_none() {
        profile.name = std::env::var("TEMPORAL_PROFILE")
            .ok()
            .filter(|p| !p.is_empty());
    }
    if let Some(cfg) = config::read("config.toml")
        .ok()
        .flatten()
        .and_then(|src| tmprl_core::config::parse_config(&src).ok())
    {
        let resolved = cfg.resolve(profile.name.as_deref().unwrap_or("default"));
        profile.api_key = resolved.api_key;
        profile.ignore_env = !resolved.env;
    }

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
    app.set_temporal_config(profile.config_file.clone());

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
    app.apply_theme(theme::depth_from_env(), theme.as_deref());
    app.apply_dashboard(dashboard.as_deref());
    app.apply_config(keys.as_deref(), views.as_deref(), config.as_deref());
    app.start(startup);

    let mouse = app.mouse();
    let terminal = ratatui::init();
    let _ = event::bracketed_paste(true);
    if mouse {
        // After ratatui::init, so this hook runs before its own and the mouse is released first.
        let restore = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = event::capture_mouse(false);
            restore(info);
        }));
        if let Err(e) = event::capture_mouse(true) {
            event::release_terminal(mouse);
            eprintln!("tmprl: {e}");
            return std::process::ExitCode::FAILURE;
        }
    }
    let result = event::run(terminal, app, rx, tx).await;
    event::release_terminal(mouse);

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tmprl: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
