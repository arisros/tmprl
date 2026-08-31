//! Finding and reading `~/.config/tmprl/*.toml`.
//!
//! This module is only the file IO. Everything that can be got wrong about the *contents*
//! — an unknown command id, a malformed chord, two views on one key — is decided in
//! `tmprl_core::config`, where it is unit tested without touching a disk.

use std::path::PathBuf;

/// `$TMPRL_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tmprl`, else `~/.config/tmprl`.
///
/// `HOME` is read rather than pulled from a crate: this is the only path lookup tmprl does,
/// and it is not worth a dependency.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TMPRL_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("tmprl"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("tmprl"))
}

/// Read a config file, or `None` if it is absent.
///
/// An unreadable file is reported rather than treated as absent — "I wrote a keys.toml and
/// nothing happened" is the failure this exists to prevent.
pub fn read(name: &str) -> Result<Option<String>, String> {
    let Some(path) = config_dir().map(|d| d.join(name)) else {
        return Ok(None);
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The env vars are process-global, so the precedence cases run in one test rather than
    /// racing each other across threads.
    #[test]
    fn config_dir_follows_the_documented_precedence() {
        // SAFETY: single-threaded within this test; no other test reads these vars.
        unsafe {
            std::env::set_var("HOME", "/home/someone");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("TMPRL_CONFIG_DIR");
        }
        assert_eq!(
            config_dir(),
            Some(PathBuf::from("/home/someone/.config/tmprl"))
        );

        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/xdg") };
        assert_eq!(config_dir(), Some(PathBuf::from("/xdg/tmprl")));

        unsafe { std::env::set_var("TMPRL_CONFIG_DIR", "/explicit") };
        assert_eq!(
            config_dir(),
            Some(PathBuf::from("/explicit")),
            "an explicit override wins over XDG"
        );

        unsafe {
            std::env::remove_var("TMPRL_CONFIG_DIR");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn an_absent_file_is_not_an_error() {
        unsafe { std::env::set_var("TMPRL_CONFIG_DIR", "/nonexistent-tmprl-config-dir") };
        assert_eq!(read("keys.toml"), Ok(None));
        unsafe { std::env::remove_var("TMPRL_CONFIG_DIR") };
    }
}
