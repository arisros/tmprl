//! tmprl, a terminal client for Temporal.

mod app;
mod clipboard;
mod config;
mod event;
mod keys;
mod theme;
mod ui;
mod view;

use tmprl_client::{Conn, ProfileRef};
use tokio::sync::mpsc::unbounded_channel;

const USAGE: &str = "\
tmprl, a terminal client for Temporal

USAGE:
    tmprl [OPTIONS]

OPTIONS:
    -p, --profile <NAME>        Profile from ~/.config/temporalio/temporal.toml
        --temporal-config <P>   Override that file's path (alias: --config)
        --config-path           Print where tmprl reads its own config, and exit
    -h, --help                  Print this message
    -V, --version               Print version

Connection settings come from the same files and TEMPORAL_* variables the
`temporal` CLI uses. tmprl's own config (config.toml, keys.toml, views.toml)
is a different directory; --config-path prints it. Press ? inside the
application for keybindings.
";

fn parse_args() -> Result<ProfileRef, String> {
    let mut profile = ProfileRef::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("tmprl {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-p" | "--profile" => {
                profile.name = Some(args.next().ok_or("--profile needs a value")?);
            }
            // `--config` predates tmprl having a config of its own and names the
            // *Temporal* profile file. Kept as an alias so existing wrappers keep working.
            "--temporal-config" | "--config" => {
                profile.config_file = Some(args.next().ok_or("--temporal-config needs a value")?);
            }
            "--config-path" => {
                // Parsed in order, so a --temporal-config before it is reflected.
                print!("{}", config::describe_paths(profile.config_file.as_deref()));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    Ok(profile)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let profile = match parse_args() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tmprl: {e}");
            return std::process::ExitCode::from(2);
        }
    };

    // Connect *before* touching the terminal, so a connection error is an ordinary message
    // on stderr rather than a flash of alternate screen followed by a stack trace.
    let conn = match Conn::connect(&profile).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tmprl: {e}");
            eprintln!("\nIs a server reachable? Try `temporal server start-dev`,");
            eprintln!("or select a profile with `tmprl --profile <name>`.");
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
    let (keys, views, config) = (
        read_config("keys.toml"),
        read_config("views.toml"),
        read_config("config.toml"),
    );
    app.apply_config(keys.as_deref(), views.as_deref(), config.as_deref());

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
