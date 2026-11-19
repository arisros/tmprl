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

use std::io::IsTerminal;

use cli::Cli;
use ratatui::text::{Line, Span};
use tmprl_client::Conn;
use tmprl_core::theme::ThemeOverrides;
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

    let splash = Splash::new(&profile);
    let drawn = splash.show(vec![Line::from(Span::styled(
        "connecting…",
        splash.theme.faint,
    ))]);
    let conn = match Conn::connect(&profile).await {
        Ok(c) => {
            Splash::erase(drawn);
            c
        }
        Err(e) => {
            Splash::erase(drawn);
            let hint = [
                "Is a server reachable? Try `temporal server start-dev`,",
                "or point tmprl at one with `--profile <name>` or `--address <host:port>`.",
            ];
            let mut status = vec![
                Line::from(Span::styled(e.to_string(), splash.theme.err)),
                Line::default(),
            ];
            status.extend(hint.map(|h| Line::from(Span::styled(h, splash.theme.faint))));
            if splash.show(status) == 0 {
                eprintln!("tmprl: {e}");
                eprintln!("\n{}", hint[0]);
                eprintln!("{}", hint[1]);
            }
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
    app.icons = theme::icons_from_env(
        config
            .as_deref()
            .and_then(|src| tmprl_core::config::parse_config(src).ok())
            .and_then(|cfg| cfg.icons),
    );
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

struct Splash {
    theme: theme::Theme,
    place: String,
    on: bool,
}

impl Splash {
    fn new(profile: &tmprl_client::ProfileRef) -> Self {
        let place = match (&profile.address, &profile.name) {
            (Some(address), _) => address.clone(),
            (None, Some(name)) => format!("profile {name}"),
            (None, None) => "the default profile".to_string(),
        };
        Self {
            theme: theme::Theme::new(theme::depth_from_env(), &ThemeOverrides::default()),
            place,
            on: std::io::stderr().is_terminal(),
        }
    }

    fn show(&self, status: Vec<Line<'static>>) -> usize {
        let columns = crossterm::terminal::size().map_or(0, |(w, _)| w as usize);
        match ui::brand::splash(&self.theme, &self.place, status, columns) {
            Some((width, lines)) if self.on && width < columns => {
                for line in &lines {
                    eprintln!("{line}");
                }
                lines.len()
            }
            _ => 0,
        }
    }

    fn erase(lines: usize) {
        if lines > 0 {
            eprint!("\x1b[{lines}A\r\x1b[J");
        }
    }
}
