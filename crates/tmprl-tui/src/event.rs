//! The event loop.
//!
//! One `select!` over three sources: the terminal, the backend channel, and a tick. The
//! reducer it calls is synchronous, so a slow RPC can never delay a keystroke — the RPC is
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
                // stdin closed — nothing more can arrive.
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => app.handle(msg),
            _ = tick.tick() => app.handle(Msg::Tick),
        }
    }
}
