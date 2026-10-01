//! Taking the terminal and giving it back, bracketed paste included.
//!
//! Bracketed paste is a mode the terminal stays in until told otherwise, like raw mode and
//! the alternate screen, so it is switched on and off in the same two places they are.
//! Left on, the shell tmprl returns to receives every paste wrapped in escape codes it may
//! not understand.

use std::io::{self, Write, stdout};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// Raw mode, the alternate screen, and bracketed paste.
///
/// With the mode on a paste arrives whole, as one `Event::Paste`, instead of as the keys
/// that would have typed it, each of which is a command in Normal mode. A terminal that
/// does not know the mode ignores the request and goes on sending keys, which is no worse
/// than before, so a failure to ask is not worth refusing to start over.
pub fn take() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = bracketed_paste(&mut stdout(), true);
    terminal
}

/// Undo [`take`]. Every way out of the event loop comes through here: quitting, SIGTERM
/// (which the loop turns into a quit), an error, and the pause while `$EDITOR` runs.
pub fn give_back() {
    let _ = bracketed_paste(&mut stdout(), false);
    ratatui::restore();
}

/// Give bracketed paste back on a panic as well.
///
/// `ratatui::init` installs a hook that leaves raw mode and the alternate screen, and knows
/// nothing of this mode. Called once, before the terminal is first taken: each
/// `ratatui::init` wraps the hook that was there, so this one stays in the chain however
/// many times the terminal is taken again after an editor.
pub fn give_back_on_panic() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = bracketed_paste(&mut stdout(), false);
        hook(info);
    }));
}

fn bracketed_paste(out: &mut impl Write, on: bool) -> io::Result<()> {
    if on {
        execute!(out, EnableBracketedPaste)
    } else {
        execute!(out, DisableBracketedPaste)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: &str = "\x1b[?2004h";
    const OFF: &str = "\x1b[?2004l";

    #[test]
    fn the_mode_is_asked_for_and_given_back_with_the_xterm_sequences() {
        let mut out = Vec::new();
        bracketed_paste(&mut out, true).unwrap();
        bracketed_paste(&mut out, false).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), format!("{ON}{OFF}"));
    }

    /// Set in the child this test starts, which is the process that panics.
    const CHILD: &str = "TMPRL_TEST_PANIC_IN_CHILD";

    #[test]
    fn a_panic_gives_bracketed_paste_back() {
        // The hook is process-wide, so the panic happens in a child: this same test, run
        // again with the variable set. Its stdout is what a terminal would have received.
        if std::env::var_os(CHILD).is_some() {
            give_back_on_panic();
            panic!("on purpose, to run the hook");
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "term::tests::a_panic_gives_bracketed_paste_back",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(!out.status.success(), "the child should have panicked");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains(OFF), "no disable sequence in {stdout:?}");
    }
}
