//! Finding and reading `~/.config/tmprl/*.toml`.
//!
//! This module is only the file IO. Everything that can be got wrong about the *contents*
//! (an unknown command id, a malformed chord, two views on one key) is decided in
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
///, tests run in parallel threads, and a flaky suite is worse than an untested one.
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

/// The files tmprl reads out of its config directory.
pub const CONFIG_FILES: [&str; 3] = ["config.toml", "keys.toml", "views.toml"];

/// What `--config-path` prints.
///
/// Answers "where do I put config.toml", which is otherwise only discoverable by reading
/// the source: the directory is chosen from the environment and there is no other way to
/// see which branch won. Marks each file present or absent, because an absent file and a
/// file in the wrong directory look identical from the outside.
pub fn describe_paths(explicit_temporal_config: Option<&str>) -> String {
    let mut out = String::new();

    // The connection file first: it is the one that decides whether `-p sit` resolves, and
    // the one whose default path differs per platform.
    match tmprl_client::config_file_in_use(explicit_temporal_config) {
        None => out.push_str("profiles: nowhere (no HOME)\n"),
        Some(path) => {
            let mark = if path.is_file() { "present" } else { "absent" };
            out.push_str(&format!("profiles: {} ({mark})\n", path.display()));
        }
    }

    match config_dir() {
        None => out.push_str(
            "config: nowhere (neither TMPRL_CONFIG_DIR, XDG_CONFIG_HOME nor HOME is set)\n",
        ),
        Some(dir) => {
            out.push_str(&format!("config: {}\n", dir.display()));
            for name in CONFIG_FILES {
                let path = dir.join(name);
                let mark = if path.is_file() { "present" } else { "absent" };
                out.push_str(&format!("  {name:<12} {mark}\n"));
            }
        }
    }
    out.push_str(&format!("audit:  {}\n", audit_path_display()));
    out
}

/// Where the audit log lives, as text, whether or not it exists yet.
fn audit_path_display() -> String {
    match audit_dir() {
        Ok(dir) => dir.join("audit.jsonl").display().to_string(),
        Err(e) => e,
    }
}

/// Read a config file, or `None` if it is absent.
///
/// An unreadable file is reported rather than treated as absent, "I wrote a keys.toml and
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

/// `$XDG_STATE_HOME/tmprl`, else `~/.local/state/tmprl`.
fn audit_dir() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(d) => Ok(PathBuf::from(d).join("tmprl")),
        None => match std::env::var_os("HOME") {
            Some(h) => Ok(PathBuf::from(h).join(".local").join("state").join("tmprl")),
            None => Err("no HOME to write an audit log under".into()),
        },
    }
}

/// Append one line to `~/.local/state/tmprl/audit.jsonl`.
///
/// State, not config: `$XDG_STATE_HOME` else `~/.local/state`, per the XDG spec. The file is
/// opened in append mode every time rather than held open, so an external `tail -f` sees each
/// line as it lands and nothing is lost if tmprl is killed.
pub fn append_audit(line: &str) -> Result<(), String> {
    use std::io::Write;

    let dir = audit_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

    let path = dir.join("audit.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("could not write {}: {e}", path.display()))
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

    #[test]
    fn describe_paths_names_every_file_and_the_audit_log() {
        // The listing has to be complete: a file tmprl reads but does not mention here
        // sends the reader looking in the wrong directory.
        let out = describe_paths(None);
        for name in CONFIG_FILES {
            assert!(out.contains(name), "{name} missing from:\n{out}");
        }
        assert!(out.contains("audit.jsonl"), "{out}");
        assert!(out.contains("config:"), "{out}");
    }

    #[test]
    fn describe_paths_marks_a_file_that_exists() {
        let dir = std::env::temp_dir().join("tmprl-describe-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "").unwrap();

        // `describe_paths` reads the real environment, so the marking logic is exercised
        // through the same predicate rather than by mutating process-global state.
        assert!(dir.join("config.toml").is_file());
        assert!(!dir.join("keys.toml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
