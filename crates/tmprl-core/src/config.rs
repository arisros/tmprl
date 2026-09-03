//! `keys.toml` and `views.toml`.
//!
//! Parsing lives here, in the crate with no IO, so a malformed config is a unit test rather
//! than something you discover by launching the application. `tmprl-tui` reads the bytes off
//! disk and hands them to these functions.
//!
//! Both loaders are *strict and additive*: an unknown command id or an unparseable chord is
//! reported, not skipped silently. A keymap that quietly drops the line you just wrote is
//! considerably worse than one that tells you the line is wrong.

use crate::command::Registry;
use crate::key::KeyParseError;
use crate::keymap::Keymap;
use crate::mode::Mode;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{file} is not valid TOML: {message}")]
    Syntax { file: &'static str, message: String },
    #[error("{file}: `{path}` should be {expected}")]
    Type {
        file: &'static str,
        path: String,
        expected: &'static str,
    },
    #[error("keys.toml: `{0}` is not a mode (expected normal, insert, visual, v-line or command)")]
    UnknownMode(String),
    #[error("keys.toml: `{chord}` is bound to `{command}`, which is not a command")]
    UnknownCommand { chord: String, command: String },
    #[error("keys.toml: `{chord}` is not a key sequence: {source}")]
    BadChord {
        chord: String,
        #[source]
        source: KeyParseError,
    },
    #[error("views.toml: view `{name}` has key `{key}`; keys must be a single digit 1-9")]
    BadViewKey { name: String, key: String },
    #[error("views.toml: two views claim key `{0}`")]
    DuplicateViewKey(char),
}

// ── views.toml ───────────────────────────────────────────────────────────────

/// A saved visibility query, reachable from a key.
///
/// The query is stored verbatim. A saved view sets the query bar's contents and nothing
/// else — it is a bookmark, not a mode, so after selecting one the text is still right there
/// to edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedView {
    /// `1`–`9`. Views are reached with the leader key, because a bare digit in Normal mode
    /// is the start of a count.
    pub key: char,
    pub name: String,
    pub query: String,
}

/// Parse `views.toml`:
///
/// ```toml
/// [[view]]
/// key   = "1"
/// name  = "Running"
/// query = "ExecutionStatus = 'Running'"
/// ```
pub fn parse_views(src: &str) -> Result<Vec<SavedView>, ConfigError> {
    const FILE: &str = "views.toml";
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;

    let Some(raw) = table.get("view") else {
        return Ok(Vec::new());
    };
    let entries = raw.as_array().ok_or(ConfigError::Type {
        file: FILE,
        path: "view".into(),
        expected: "an array of [[view]] tables",
    })?;

    let mut views: Vec<SavedView> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let t = entry.as_table().ok_or_else(|| ConfigError::Type {
            file: FILE,
            path: format!("view[{i}]"),
            expected: "a table",
        })?;
        let field = |name: &str| -> Result<String, ConfigError> {
            t.get(name)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| ConfigError::Type {
                    file: FILE,
                    path: format!("view[{i}].{name}"),
                    expected: "a string",
                })
        };

        let name = field("name")?;
        let key = field("key")?;
        let mut chars = key.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(c @ '1'..='9'), None) => c,
            _ => return Err(ConfigError::BadViewKey { name, key }),
        };
        if views.iter().any(|v| v.key == key) {
            return Err(ConfigError::DuplicateViewKey(key));
        }
        views.push(SavedView {
            key,
            name,
            query: field("query")?,
        });
    }

    views.sort_by_key(|v| v.key);
    Ok(views)
}

// ── config.toml ──────────────────────────────────────────────────────────────

/// Where the codec server lives, if the cluster uses one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecConfig {
    /// Base URL. `/decode` is appended to it, per Temporal's contract.
    pub endpoint: String,
    /// Sent verbatim as `Authorization`. Optional, and deliberately *not* defaulted from
    /// anything: a codec server is a service the user runs, and quietly forwarding a
    /// credential they did not ask us to send would be a surprise.
    pub auth: Option<String>,
}

/// `config.toml`. Everything in it is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub codec: Option<CodecConfig>,
}

/// Parse `config.toml`:
///
/// ```toml
/// [codec]
/// endpoint = "http://localhost:8081"
/// auth     = "Bearer …"          # optional
/// ```
pub fn parse_config(src: &str) -> Result<Config, ConfigError> {
    const FILE: &str = "config.toml";
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;

    let Some(raw) = table.get("codec") else {
        return Ok(Config::default());
    };
    let codec = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "codec".into(),
        expected: "a table",
    })?;

    let endpoint = codec
        .get("endpoint")
        .and_then(|v| v.as_str())
        .ok_or(ConfigError::Type {
            file: FILE,
            path: "codec.endpoint".into(),
            expected: "a string",
        })?
        .trim_end_matches('/')
        .to_string();
    if endpoint.is_empty() {
        return Err(ConfigError::Type {
            file: FILE,
            path: "codec.endpoint".into(),
            expected: "a non-empty URL",
        });
    }

    let auth = match codec.get("auth") {
        None => None,
        Some(v) => Some(
            v.as_str()
                .ok_or(ConfigError::Type {
                    file: FILE,
                    path: "codec.auth".into(),
                    expected: "a string",
                })?
                .to_string(),
        ),
    };

    Ok(Config {
        codec: Some(CodecConfig { endpoint, auth }),
    })
}

// ── keys.toml ────────────────────────────────────────────────────────────────

/// Apply `keys.toml` on top of a keymap:
///
/// ```toml
/// [normal]
/// "<leader>w" = "nav.open"
/// "ZZ"        = "app.quit"
///
/// [insert]
/// "jj" = "mode.normal"
/// ```
///
/// Later bindings win, and this runs after the defaults, so a user binding overrides the
/// built-in one for the same chord in the same mode.
///
/// Command ids are resolved against `registry`, which is what lets a `String` from a config
/// file become the `&'static str` the keymap stores — and what makes a typo an error at
/// startup instead of a key that silently does nothing.
pub fn apply_keys(src: &str, registry: &Registry, keymap: &mut Keymap) -> Result<(), ConfigError> {
    const FILE: &str = "keys.toml";
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;

    for (mode_name, bindings) in &table {
        let mode = parse_mode(mode_name)?;
        let bindings = bindings.as_table().ok_or_else(|| ConfigError::Type {
            file: FILE,
            path: mode_name.clone(),
            expected: "a table of \"chord\" = \"command.id\"",
        })?;

        for (chord, command) in bindings {
            let command = command.as_str().ok_or_else(|| ConfigError::Type {
                file: FILE,
                path: format!("{mode_name}.{chord}"),
                expected: "a command id string",
            })?;
            // Resolving through the registry is what turns the config's String into the
            // 'static id the keymap holds.
            let id = registry
                .get(command)
                .ok_or_else(|| ConfigError::UnknownCommand {
                    chord: chord.clone(),
                    command: command.to_string(),
                })?
                .id;
            keymap
                .bind(mode, chord, id)
                .map_err(|source| ConfigError::BadChord {
                    chord: chord.clone(),
                    source,
                })?;
        }
    }
    Ok(())
}

/// Bind each saved view to `<leader>{digit}`.
///
/// Not to the bare digit the interface design originally called for: a leading digit in
/// Normal mode is a count (`7j`), and counts are load-bearing. `<leader>1` keeps both, and
/// puts the views in the which-key popup under the leader where they are discoverable.
///
/// Only views that actually exist get a binding, so the popup never advertises an empty
/// slot. Call [`Registry::add_views`] first — the commands must exist to be bound.
pub fn bind_views(views: &[SavedView], keymap: &mut Keymap) -> Result<(), ConfigError> {
    for v in views {
        let seq = format!("<leader>{}", v.key);
        let id: &'static str = Box::leak(format!("view.{}", v.key).into_boxed_str());
        keymap
            .bind(Mode::Normal, &seq, id)
            .map_err(|source| ConfigError::BadChord { chord: seq, source })?;
    }
    Ok(())
}

fn parse_mode(name: &str) -> Result<Mode, ConfigError> {
    Ok(match name.trim().to_ascii_lowercase().as_str() {
        "normal" => Mode::Normal,
        "insert" => Mode::Insert,
        "visual" => Mode::Visual,
        "v-line" | "visual-line" | "visualline" => Mode::VisualLine,
        "command" => Mode::Command,
        _ => return Err(ConfigError::UnknownMode(name.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Chord;
    use crate::keymap::{Pending, Resolution, default_keymap};

    #[test]
    fn views_parse_in_key_order() {
        let views = parse_views(
            r#"
            [[view]]
            key = "3"
            name = "Failed"
            query = "ExecutionStatus = 'Failed'"

            [[view]]
            key = "1"
            name = "Running"
            query = "ExecutionStatus = 'Running'"
            "#,
        )
        .unwrap();

        assert_eq!(views.len(), 2);
        assert_eq!(views[0].key, '1');
        assert_eq!(views[0].name, "Running");
        assert_eq!(views[1].key, '3');
        assert_eq!(views[1].query, "ExecutionStatus = 'Failed'");
    }

    #[test]
    fn an_absent_or_empty_views_file_is_not_an_error() {
        assert_eq!(parse_views("").unwrap(), Vec::new());
        assert_eq!(parse_views("# nothing here\n").unwrap(), Vec::new());
    }

    #[test]
    fn a_view_key_must_be_a_single_digit() {
        for key in ["0", "10", "a", ""] {
            let src = format!("[[view]]\nkey = \"{key}\"\nname = \"N\"\nquery = \"\"\n");
            assert!(
                matches!(parse_views(&src), Err(ConfigError::BadViewKey { .. })),
                "key {key:?} should be rejected"
            );
        }
    }

    #[test]
    fn two_views_cannot_claim_the_same_key() {
        let src = r#"
            [[view]]
            key = "1"
            name = "A"
            query = ""
            [[view]]
            key = "1"
            name = "B"
            query = ""
        "#;
        assert_eq!(parse_views(src), Err(ConfigError::DuplicateViewKey('1')));
    }

    #[test]
    fn a_view_missing_a_field_says_which_one() {
        let err = parse_views("[[view]]\nkey = \"1\"\n").unwrap_err();
        assert!(
            err.to_string().contains("view[0].name"),
            "error should name the missing field, got: {err}"
        );
    }

    #[test]
    fn malformed_toml_is_reported_not_ignored() {
        assert!(matches!(
            parse_views("[[view]\nkey =").unwrap_err(),
            ConfigError::Syntax { .. }
        ));
    }

    #[test]
    fn saved_views_bind_under_the_leader_not_the_bare_digit() {
        // A bare `1` starts a count, and counts compose with every motion. Binding views
        // to bare digits would break `7j`, which is not a trade worth making.
        let mut registry = Registry::builtin();
        let views = vec![SavedView {
            key: '1',
            name: "Running".into(),
            query: "ExecutionStatus = 'Running'".into(),
        }];
        registry.add_views(&views);
        let mut keymap = default_keymap();
        bind_views(&views, &mut keymap).unwrap();

        let mut p = Pending::default();
        assert_eq!(
            keymap.resolve(Mode::Normal, &mut p, Chord::ch('1')),
            Resolution::Count(1),
            "a bare digit must still start a count"
        );
        p.clear();

        assert!(matches!(
            keymap.resolve(Mode::Normal, &mut p, Chord::ch(' ')),
            Resolution::Pending { .. }
        ));
        assert_eq!(
            keymap.resolve(Mode::Normal, &mut p, Chord::ch('1')),
            Resolution::Run {
                id: "view.1",
                count: None
            }
        );
    }

    #[test]
    fn an_unconfigured_view_slot_is_left_unbound() {
        // Which-key and the help overlay are generated from the keymap, so a binding for a
        // view that does not exist would be a lie rendered on screen.
        let mut keymap = default_keymap();
        bind_views(&[], &mut keymap).unwrap();
        let mut p = Pending::default();
        keymap.resolve(Mode::Normal, &mut p, Chord::ch(' '));
        match keymap.resolve(Mode::Normal, &mut p, Chord::ch('4')) {
            Resolution::Unbound { .. } => {}
            other => panic!("<leader>4 should be unbound, got {other:?}"),
        }
    }

    #[test]
    fn a_codec_endpoint_is_read_and_normalised() {
        let c = parse_config(
            r#"
            [codec]
            endpoint = "http://localhost:8081/"
            auth = "Bearer abc"
            "#,
        )
        .unwrap();
        let codec = c.codec.unwrap();
        // The trailing slash goes, because `/decode` is appended and `//decode` is not the
        // same path to every server.
        assert_eq!(codec.endpoint, "http://localhost:8081");
        assert_eq!(codec.auth.as_deref(), Some("Bearer abc"));
    }

    #[test]
    fn auth_is_optional_and_never_invented() {
        let c = parse_config("[codec]\nendpoint = \"http://x\"\n").unwrap();
        assert_eq!(c.codec.unwrap().auth, None);
    }

    #[test]
    fn no_codec_section_means_no_codec() {
        assert_eq!(parse_config("").unwrap(), Config::default());
        assert_eq!(parse_config("# nothing\n").unwrap().codec, None);
    }

    #[test]
    fn a_codec_section_without_an_endpoint_is_an_error() {
        // Silently ignoring it would leave encrypted payloads unreadable with no clue why.
        let err = parse_config("[codec]\nauth = \"x\"\n").unwrap_err();
        assert!(err.to_string().contains("codec.endpoint"), "got {err}");

        let err = parse_config("[codec]\nendpoint = \"\"\n").unwrap_err();
        assert!(err.to_string().contains("codec.endpoint"), "got {err}");
    }

    #[test]
    fn keys_toml_overrides_a_default_binding() {
        let registry = Registry::builtin();
        let mut keymap = default_keymap();

        apply_keys("[normal]\n\"j\" = \"motion.up\"\n", &registry, &mut keymap).unwrap();

        let mut p = Pending::default();
        assert_eq!(
            keymap.resolve(Mode::Normal, &mut p, Chord::ch('j')),
            Resolution::Run {
                id: "motion.up",
                count: None
            },
            "a user binding must win over the built-in one"
        );
    }

    #[test]
    fn keys_toml_adds_a_new_sequence() {
        let registry = Registry::builtin();
        let mut keymap = default_keymap();
        apply_keys("[normal]\n\"ZZ\" = \"app.quit\"\n", &registry, &mut keymap).unwrap();

        let mut p = Pending::default();
        assert!(matches!(
            keymap.resolve(Mode::Normal, &mut p, Chord::ch('Z')),
            Resolution::Pending { .. }
        ));
        assert_eq!(
            keymap.resolve(Mode::Normal, &mut p, Chord::ch('Z')),
            Resolution::Run {
                id: "app.quit",
                count: None
            }
        );
    }

    #[test]
    fn every_mode_name_is_accepted() {
        let registry = Registry::builtin();
        let mut keymap = default_keymap();
        let src = r#"
            [normal]
            "<F5>" = "app.refresh"
            [insert]
            "<F5>" = "mode.normal"
            [visual]
            "<F5>" = "app.cancel"
            [v-line]
            "<F5>" = "app.cancel"
            [command]
            "<F5>" = "app.cancel"
        "#;
        assert_eq!(apply_keys(src, &registry, &mut keymap), Ok(()));
    }

    #[test]
    fn an_unknown_command_is_an_error_rather_than_a_dead_key() {
        // This is the whole reason the loader resolves through the registry. A silently
        // dropped binding is a key that does nothing, with no way to find out why.
        let registry = Registry::builtin();
        let mut keymap = default_keymap();
        let err = apply_keys(
            "[normal]\n\"x\" = \"motion.sideways\"\n",
            &registry,
            &mut keymap,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ConfigError::UnknownCommand {
                chord: "x".into(),
                command: "motion.sideways".into()
            }
        );
        assert!(err.to_string().contains("motion.sideways"));
    }

    #[test]
    fn an_unknown_mode_is_an_error() {
        let registry = Registry::builtin();
        let mut keymap = default_keymap();
        assert_eq!(
            apply_keys("[sideways]\n\"x\" = \"app.quit\"\n", &registry, &mut keymap),
            Err(ConfigError::UnknownMode("sideways".into()))
        );
    }

    #[test]
    fn an_unparseable_chord_names_itself() {
        let registry = Registry::builtin();
        let mut keymap = default_keymap();
        let err = apply_keys(
            "[normal]\n\"<Nope>\" = \"app.quit\"\n",
            &registry,
            &mut keymap,
        )
        .unwrap_err();
        assert!(
            matches!(err, ConfigError::BadChord { ref chord, .. } if chord == "<Nope>"),
            "got {err}"
        );
    }

    #[test]
    fn an_empty_keys_file_leaves_the_defaults_alone() {
        let registry = Registry::builtin();
        let mut keymap = default_keymap();
        let before = keymap.bindings().len();
        apply_keys("", &registry, &mut keymap).unwrap();
        assert_eq!(keymap.bindings().len(), before);
    }
}
