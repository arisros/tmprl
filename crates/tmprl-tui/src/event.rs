//! The event loop.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::{App, Msg};
use crate::{keys, ui};

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

    loop {
        if let Some(request) = app.take_edit_request() {
            release_terminal(app.mouse());
            let outcome = run_editor(&request.path, None, None);
            terminal = retake_terminal(app.mouse())?;
            app.finish_edit(&request, outcome.err().map(|e| e.to_string()));
        }
        if let Some(request) = app.take_source_request() {
            release_terminal(app.mouse());
            let outcome = run_editor(&request.path, request.line, request.root.as_deref());
            terminal = retake_terminal(app.mouse())?;
            app.finish_source(&request, outcome.err().map(|e| e.to_string()));
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
                Some(Ok(Event::Mouse(m))) => {
                    if let Some(mouse) = keys::to_mouse(m) {
                        app.handle(Msg::Mouse(mouse));
                    }
                }
                Some(Ok(Event::Paste(text))) => app.handle(Msg::Paste(text)),
                Some(Ok(Event::Resize(_, _))) => app.handle(Msg::Redraw),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => app.handle(msg),
            _ = tick.tick() => app.handle(Msg::Tick),
        }
    }
}

// Not Terminal::clear: it asks the terminal for the cursor on stdin, which the key reader also reads.
fn retake_terminal(mouse: bool) -> Result<DefaultTerminal> {
    let terminal = ratatui::init();
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;
    drop_pending_input()?;
    bracketed_paste(true)?;
    if mouse {
        capture_mouse(true)?;
    }
    Ok(terminal)
}

pub fn release_terminal(mouse: bool) {
    if mouse {
        let _ = capture_mouse(false);
    }
    let _ = bracketed_paste(false);
    ratatui::restore();
}

pub fn bracketed_paste(on: bool) -> std::io::Result<()> {
    use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
    if on {
        crossterm::execute!(std::io::stdout(), EnableBracketedPaste)
    } else {
        crossterm::execute!(std::io::stdout(), DisableBracketedPaste)
    }
}

pub fn capture_mouse(on: bool) -> std::io::Result<()> {
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    if on {
        crossterm::execute!(std::io::stdout(), EnableMouseCapture)
    } else {
        crossterm::execute!(std::io::stdout(), DisableMouseCapture)
    }
}

// An editor's queries to the terminal can be answered after it exits, and would read as typing.
fn drop_pending_input() -> Result<()> {
    const QUIET: Duration = Duration::from_millis(60);
    const LONGEST: Duration = Duration::from_millis(400);
    let started = std::time::Instant::now();
    while started.elapsed() < LONGEST && crossterm::event::poll(QUIET)? {
        let _ = crossterm::event::read()?;
    }
    Ok(())
}

fn run_editor(
    path: &std::path::Path,
    line: Option<u32>,
    root: Option<&std::path::Path>,
) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        anyhow::bail!("$EDITOR is set but empty");
    };

    let mut command = std::process::Command::new(program);
    if let Some(root) = root {
        command.current_dir(root);
    }
    let status = command
        .args(parts)
        .args(line.map(|n| format!("+{n}")))
        .arg(path)
        .status()
        .map_err(|e| anyhow::anyhow!("could not run `{program}`: {e}"))?;

    if !status.success() {
        anyhow::bail!("`{program}` exited with {status}");
    }
    Ok(())
}
