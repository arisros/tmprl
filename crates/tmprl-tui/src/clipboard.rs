//! Yanking to the clipboard over OSC 52.

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

pub fn yank(text: &str, max: usize) -> Result<(), YankError> {
    if text.len() > max {
        return Err(YankError::TooLarge {
            len: text.len(),
            max,
        });
    }
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
