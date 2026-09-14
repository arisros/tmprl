//! The event loop.
//!
//! One `select!` over three sources: the terminal, the backend channel, and a tick. The
//! reducer it calls is synchronous, so a slow RPC can never delay a keystroke, the RPC is
//! on a spawned task and returns through the channel like any other message.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::{App, Msg};
use crate::{keys, ui};

/// Redraw cadence while idle. Only needed so relative timestamps stay honest; input and
/// backend messages drive their own redraws immediately.
const TICK: Duration = Duration::from_secs(1);

pub async fn run(
    mut terminal: DefaultTerminal,
    mut app: App,
    mut rx: UnboundedReceiver<Msg>,
    tx: UnboundedSender<Msg>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // A terminated process should still restore the terminal. The panic hook that
    // `ratatui::init` installs covers panics; this covers SIGTERM.
    #[cfg(unix)]
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            if let Ok(mut s) = signal(SignalKind::terminate()) {
                s.recv().await;
                let _ = tx.send(Msg::Quit);
            }
        });
    }
    #[cfg(not(unix))]
    let _ = &tx;

    app.load_namespaces();

    loop {
        // Before drawing, because the editor wants the screen this draw would paint over.
        if let Some(request) = app.take_edit_request() {
            // Put the terminal back the way we found it, run the editor with stdin, stdout
            // and stderr inherited so it is a normal interactive program, then take the
            // terminal again. `ratatui::init` reinstalls the panic hook, so a panic after
            // this still restores.
            ratatui::restore();
            let outcome = run_editor(&request.path);
            terminal = ratatui::init();
            terminal.clear()?;
            app.finish_edit(&request, outcome.err().map(|e| e.to_string()));
        }
        if app.dirty {
            terminal.draw(|f| ui::render(f, &mut app))?;
            app.dirty = false;
        }
        if app.should_quit {
            return Ok(());
        }

        tokio::select! {
            maybe_event = events.next() => match maybe_event {
                Some(Ok(Event::Key(k))) => {
                    if let Some(chord) = keys::to_chord(k) {
                        app.handle(Msg::Key(chord));
                    }
                }
                Some(Ok(Event::Resize(_, _))) => app.handle(Msg::Redraw),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                // stdin closed, nothing more can arrive.
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => app.handle(msg),
            _ = tick.tick() => app.handle(Msg::Tick),
        }
    }
}

/// Run the user's editor over a file, blocking until it exits.
///
/// `$VISUAL` before `$EDITOR` before `vi`, which is the order every other terminal program
/// uses and therefore the order people have already configured for.
///
/// The command is split on whitespace rather than run through a shell: `EDITOR="code -w"`
/// is common and has to work, while a shell would also make `EDITOR` an injection point for
/// a file name tmprl chose. Blocking is correct here, the TUI is not on screen and there is
/// nothing else for this task to be doing.
fn run_editor(path: &std::path::Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        anyhow::bail!("$EDITOR is set but empty");
    };

    let status = std::process::Command::new(program)
        .args(parts)
        .arg(path)
        .status()
        .map_err(|e| anyhow::anyhow!("could not run `{program}`: {e}"))?;

    if !status.success() {
        anyhow::bail!("`{program}` exited with {status}");
    }
    Ok(())
}
