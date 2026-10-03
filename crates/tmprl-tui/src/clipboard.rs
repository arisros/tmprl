//! Yanking to the clipboard over OSC 52.
//!
//! Deliberately *not* xclip/xsel/wl-copy. The common deployment is SSH into a remote,
//! often headless host, where those either fail or copy into a clipboard on the server,
//! which helps nobody, silently. OSC 52 hands the text back over the terminal connection to
//! the machine the human is actually sitting at.
//!
//! Inside tmux the text goes through `tmux load-buffer -w` instead of a raw OSC 52. tmux
//! drops OSC 52 from applications unless `set-clipboard` is `on` (the default is
//! `external`), while `-w` has tmux emit the sequence to the client terminal itself. That
//! still crosses SSH, since the client is where the human sits.

use std::io::{self, Write, stdout};
use std::process::{Command, Stdio};

use crossterm::{clipboard::CopyToClipboard, execute};

#[derive(Debug, thiserror::Error)]
pub enum YankError {
    #[error("{len} bytes is too large to yank (limit {max})")]
    TooLarge { len: usize, max: usize },
    #[error("terminal write failed: {0}")]
    Io(#[from] io::Error),
    #[error("tmux load-buffer failed: {0}")]
    Tmux(String),
}

/// `max` is `[yank] max_bytes`. A silently truncated clipboard is worse than a refusal, so
/// the caller is expected to have sent anything longer to a file already.
pub fn yank(text: &str, max: usize) -> Result<(), YankError> {
    if text.len() > max {
        return Err(YankError::TooLarge {
            len: text.len(),
            max,
        });
    }
    // Tests run inside the developer's tmux, where a real yank would clobber their clipboard.
    if cfg!(test) {
        return Ok(());
    }
    if std::env::var_os("TMUX").is_some_and(|v| !v.is_empty()) {
        return yank_via_tmux(text);
    }
    let mut out = stdout();
    execute!(out, CopyToClipboard::to_clipboard_from(text))?;
    out.flush()?;
    Ok(())
}

fn yank_via_tmux(text: &str) -> Result<(), YankError> {
    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| YankError::Tmux(e.to_string()))?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(text.as_bytes())
        .map_err(|e| YankError::Tmux(e.to_string()))?;
    let output = child
        .wait_with_output()
        .map_err(|e| YankError::Tmux(e.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(YankError::Tmux(if stderr.is_empty() {
        output.status.to_string()
    } else {
        stderr
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_yanks_are_refused_rather_than_truncated() {
        let big = "x".repeat(101);
        assert!(matches!(
            yank(&big, 100),
            Err(YankError::TooLarge { len: 101, max: 100 })
        ));
        assert!(yank(&big[..100], 100).is_ok());
    }
}
