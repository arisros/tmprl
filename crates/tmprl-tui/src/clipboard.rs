//! Yanking to the clipboard over OSC 52.
//!
//! Deliberately *not* xclip/xsel/wl-copy. The common deployment is SSH into a remote,
//! often headless host, where those either fail or copy into a clipboard on the server,
//! which helps nobody, silently. OSC 52 hands the text back over the terminal connection to
//! the machine the human is actually sitting at.
//!
//! Through tmux this needs `set -g set-clipboard on`, and many terminfo entries need an `Ms`
//! override before tmux will emit the sequence at all. See `docs/INTERFACE.md`.

use std::io::{self, Write, stdout};

use crossterm::{clipboard::CopyToClipboard, execute};

/// Longest payload we will attempt. Terminals commonly cap OSC 52 around 100 KB and a
/// silently truncated clipboard is worse than a refusal.
pub const MAX_YANK: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum YankError {
    #[error("{0} bytes is too large to yank (limit {MAX_YANK})")]
    TooLarge(usize),
    #[error("terminal write failed: {0}")]
    Io(#[from] io::Error),
}

pub fn yank(text: &str) -> Result<(), YankError> {
    if text.len() > MAX_YANK {
        return Err(YankError::TooLarge(text.len()));
    }
    let mut out = stdout();
    execute!(out, CopyToClipboard::to_clipboard_from(text))?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_yanks_are_refused_rather_than_truncated() {
        let big = "x".repeat(MAX_YANK + 1);
        assert!(matches!(yank(&big), Err(YankError::TooLarge(_))));
    }
}
