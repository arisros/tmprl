//! Finding and reading `~/.config/tmprl/*.toml`.
//!
//! This module is only the file IO. Everything that can be got wrong about the *contents*
//! — an unknown command id, a malformed chord, two views on one key — is decided in
//! `tmprl_core::config`, where it is unit tested without touching a disk.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `$TMPRL_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tmprl`, else `~/.config/tmprl`.
///
/// `HOME` is read rather than pulled from a crate: this is the only path lookup tmprl does,
/// and it is not worth a dependency.
pub fn config_dir() -> Option<PathBuf> {
    resolve_dir(
        std::env::var_os("TMPRL_CONFIG_DIR"),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// The precedence rule, with the environment passed in.
///
/// Split out so it can be tested as a function. Reading the real environment in a test
/// means mutating process-global state, which races against every other test in the binary
/// — tests run in parallel threads, and a flaky suite is worse than an untested one.
fn resolve_dir(
    explicit: Option<OsString>,
    xdg: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = explicit {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = xdg {
        return Some(PathBuf::from(dir).join("tmprl"));
    }
    home.map(|h| PathBuf::from(h).join(".config").join("tmprl"))
}

/// Read a config file, or `None` if it is absent.
///
/// An unreadable file is reported rather than treated as absent — "I wrote a keys.toml and
/// nothing happened" is the failure this exists to prevent.
pub fn read(name: &str) -> Result<Option<String>, String> {
    match config_dir() {
        Some(dir) => read_from(&dir, name),
        None => Ok(None),
    }
}

fn read_from(dir: &Path, name: &str) -> Result<Option<String>, String> {
    let path = dir.join(name);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn an_explicit_directory_wins() {
        assert_eq!(
            resolve_dir(os("/explicit"), os("/xdg"), os("/home/someone")),
            Some(PathBuf::from("/explicit"))
        );
    }

    #[test]
    fn xdg_is_used_when_there_is_no_explicit_override() {
        assert_eq!(
            resolve_dir(None, os("/xdg"), os("/home/someone")),
            Some(PathBuf::from("/xdg/tmprl"))
        );
    }

    #[test]
    fn home_is_the_fallback() {
        assert_eq!(
            resolve_dir(None, None, os("/home/someone")),
            Some(PathBuf::from("/home/someone/.config/tmprl"))
        );
    }

    #[test]
    fn with_no_environment_at_all_there_is_no_config_dir() {
        assert_eq!(resolve_dir(None, None, None), None);
    }

    #[test]
    fn an_absent_file_is_not_an_error() {
        let dir = PathBuf::from("/nonexistent-tmprl-config-dir");
        assert_eq!(read_from(&dir, "keys.toml"), Ok(None));
    }

    #[test]
    fn a_file_that_exists_is_read() {
        let dir = std::env::temp_dir().join("tmprl-config-read-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("views.toml"), "# hello\n").unwrap();

        assert_eq!(read_from(&dir, "views.toml"), Ok(Some("# hello\n".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
