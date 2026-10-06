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

    loop {
        // Before drawing, because the editor wants the screen this draw would paint over.
        if let Some(request) = app.take_edit_request() {
            // Put the terminal back the way we found it, run the editor with stdin, stdout
            // and stderr inherited so it is a normal interactive program, then take the
            // terminal again. `ratatui::init` reinstalls the panic hook, so a panic after
            // this still restores.
            ratatui::restore();
            let outcome = run_editor(&request.path, None, None);
            terminal = retake_terminal()?;
            app.finish_edit(&request, outcome.err().map(|e| e.to_string()));
        }
        if let Some(request) = app.take_source_request() {
            ratatui::restore();
            let outcome = run_editor(&request.path, request.line, request.root.as_deref());
            terminal = retake_terminal()?;
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

/// Take the terminal back after an editor has had it.
///
/// Not `Terminal::clear`: that asks the terminal where its cursor is and waits for the
/// answer on stdin, which the key reader is reading too. When the answer went to the
/// reader, or came late, the wait timed out and took tmprl down with it. A screen can be
/// cleared without asking it anything.
fn retake_terminal() -> Result<DefaultTerminal> {
    let terminal = ratatui::init();
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;
    drop_pending_input()?;
    Ok(terminal)
}

/// Throw away what is waiting on stdin.
///
/// An editor asks the terminal things as it runs, its background colour, what it is, and
/// does not always stay to hear the answer. The answer then arrives here and reads as
/// typing: a colour report opened the command line and filled it with `1717/1717/1717`.
/// Nothing that arrives in the moment an editor closes was meant for tmprl, so it is
/// dropped: until the input has been quiet for a moment, and never for long.
fn drop_pending_input() -> Result<()> {
    const QUIET: Duration = Duration::from_millis(60);
    const LONGEST: Duration = Duration::from_millis(400);
    let started = std::time::Instant::now();
    while started.elapsed() < LONGEST && crossterm::event::poll(QUIET)? {
        let _ = crossterm::event::read()?;
    }
    Ok(())
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
    // In the project's own directory, so the editor's file tree, search and language
    // server see the project the file belongs to and not wherever tmprl was started.
    if let Some(root) = root {
        command.current_dir(root);
    }
    let status = command
        .args(parts)
        // `+N file`, the form vi, vim, nvim, emacs, nano and helix all take.
        .args(line.map(|n| format!("+{n}")))
        .arg(path)
        .status()
        .map_err(|e| anyhow::anyhow!("could not run `{program}`: {e}"))?;

    if !status.success() {
        anyhow::bail!("`{program}` exited with {status}");
    }
    Ok(())
}
