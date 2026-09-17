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
    #[error("config.toml: `{path}` is `{value}`, which is not a colour ({expected})")]
    BadAccent {
        path: String,
        value: String,
        expected: &'static str,
    },
}

/// A saved visibility query, reachable from a key.
///
/// The query is stored verbatim. A saved view sets the query bar's contents and nothing
/// else, it is a bookmark, not a mode, so after selecting one the text is still right there
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

/// A colour name for a profile's accent.
///
/// Named rather than a hex triple: this has to read on a 16-colour terminal, and the point
/// is that production is unmistakable, not that it matches anyone's palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
}

impl Accent {
    pub const NAMES: &'static str = "red, green, yellow, blue, magenta or cyan";

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "red" => Self::Red,
            "green" => Self::Green,
            "yellow" => Self::Yellow,
            "blue" => Self::Blue,
            "magenta" => Self::Magenta,
            "cyan" => Self::Cyan,
            _ => return None,
        })
    }
}

/// Per-profile settings, keyed by the profile name in `temporal.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileConfig {
    /// Colour for the profile name in the statusline.
    pub accent: Option<Accent>,
    /// Refuse every mutation on this profile.
    pub readonly: bool,
    /// Overrides the top-level codec for this profile.
    pub codec: Option<CodecConfig>,
}

/// What applies to the profile actually connected, after falling back to the globals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub accent: Option<Accent>,
    pub readonly: bool,
    pub codec: Option<CodecConfig>,
}

/// Where the payload pane (`K`) opens, relative to the history list.
///
/// Both keep the list on screen, so `j` / `k` still move the row the pane is showing. There
/// is no popup on purpose: it would cover the rows you are stepping through.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PayloadPane {
    #[default]
    Bottom,
    Right,
}

impl PayloadPane {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bottom" => Some(Self::Bottom),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// `config.toml`. Everything in it is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// The codec used by any profile that does not name its own.
    pub codec: Option<CodecConfig>,
    pub profiles: Vec<(String, ProfileConfig)>,
    /// `[layout] payload`. Global rather than per profile: it is about your terminal, not
    /// the cluster.
    pub payload_pane: PayloadPane,
}

impl Config {
    /// Settings for one profile. A profile with no section is not an error: it simply gets
    /// the globals, which is what every single-cluster user has today.
    pub fn resolve(&self, profile: &str) -> Resolved {
        let found = self.profiles.iter().find(|(name, _)| name == profile);
        match found {
            None => Resolved {
                accent: None,
                readonly: false,
                codec: self.codec.clone(),
            },
            Some((_, p)) => Resolved {
                accent: p.accent,
                readonly: p.readonly,
                // A profile that names no codec uses the global one; pointing production at
                // the codec you set up for SIT is exactly the mistake worth preventing, but
                // a single-cluster config must keep working unchanged.
                codec: p.codec.clone().or_else(|| self.codec.clone()),
            },
        }
    }
}

/// Parse `config.toml`:
///
/// ```toml
/// [layout]
/// payload = "right"              # or "bottom", the default
///
/// [codec]
/// endpoint = "http://localhost:8081"
/// auth     = "Bearer …"          # optional
///
/// [profile.prod]                 # keyed by the profile in temporal.toml
/// accent   = "red"
/// readonly = true
///
/// [profile.prod.codec]           # overrides the codec above, for this profile only
/// endpoint = "https://codec.internal"
/// ```
pub fn parse_config(src: &str) -> Result<Config, ConfigError> {
    const FILE: &str = "config.toml";
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;

    let codec = match table.get("codec") {
        None => None,
        Some(raw) => Some(parse_codec(raw, "codec")?),
    };

    let profiles = match table.get("profile") {
        None => Vec::new(),
        Some(raw) => {
            let table = raw.as_table().ok_or(ConfigError::Type {
                file: FILE,
                path: "profile".into(),
                expected: "a table",
            })?;
            let mut out = Vec::with_capacity(table.len());
            for (name, raw) in table {
                out.push((name.clone(), parse_profile(raw, name)?));
            }
            out
        }
    };

    let payload_pane = match table.get("layout") {
        None => PayloadPane::default(),
        Some(raw) => parse_layout(raw)?,
    };

    Ok(Config {
        codec,
        profiles,
        payload_pane,
    })
}

fn parse_layout(raw: &toml::Value) -> Result<PayloadPane, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "layout".into(),
        expected: "a table",
    })?;
    match table.get("payload") {
        None => Ok(PayloadPane::default()),
        Some(v) => v
            .as_str()
            .and_then(PayloadPane::parse)
            .ok_or(ConfigError::Type {
                file: FILE,
                path: "layout.payload".into(),
                expected: "\"bottom\" or \"right\"",
            }),
    }
}

fn parse_profile(raw: &toml::Value, name: &str) -> Result<ProfileConfig, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or_else(|| ConfigError::Type {
        file: FILE,
        path: format!("profile.{name}"),
        expected: "a table",
    })?;

    let accent = match table.get("accent") {
        None => None,
        Some(v) => {
            let text = v.as_str().ok_or_else(|| ConfigError::Type {
                file: FILE,
                path: format!("profile.{name}.accent"),
                expected: "a string",
            })?;
            Some(Accent::parse(text).ok_or_else(|| ConfigError::BadAccent {
                path: format!("profile.{name}.accent"),
                value: text.to_string(),
                expected: Accent::NAMES,
            })?)
        }
    };

    let readonly = match table.get("readonly") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| ConfigError::Type {
            file: FILE,
            path: format!("profile.{name}.readonly"),
            expected: "true or false",
        })?,
    };

    let codec = match table.get("codec") {
        None => None,
        Some(raw) => Some(parse_codec(raw, &format!("profile.{name}.codec"))?),
    };

    Ok(ProfileConfig {
        accent,
        readonly,
        codec,
    })
}

fn parse_codec(raw: &toml::Value, path: &str) -> Result<CodecConfig, ConfigError> {
    const FILE: &str = "config.toml";
    let codec = raw.as_table().ok_or_else(|| ConfigError::Type {
        file: FILE,
        path: path.to_string(),
        expected: "a table",
    })?;

    let endpoint = codec
        .get("endpoint")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ConfigError::Type {
            file: FILE,
            path: format!("{path}.endpoint"),
            expected: "a string",
        })?
        .trim_end_matches('/')
        .to_string();
    if endpoint.is_empty() {
        return Err(ConfigError::Type {
            file: FILE,
            path: format!("{path}.endpoint"),
            expected: "a non-empty URL",
        });
    }

    let auth = match codec.get("auth") {
        None => None,
        Some(v) => Some(
            v.as_str()
                .ok_or_else(|| ConfigError::Type {
                    file: FILE,
                    path: format!("{path}.auth"),
                    expected: "a string",
                })?
                .to_string(),
        ),
    };

    Ok(CodecConfig { endpoint, auth })
}

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
/// file become the `&'static str` the keymap stores, and what makes a typo an error at
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
/// slot. Call [`Registry::add_views`] first, the commands must exist to be bound.
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

    #[test]
    fn a_config_with_no_profile_section_gives_every_profile_the_globals() {
        // The single-cluster config that exists today must keep working untouched.
        let cfg = parse_config("[codec]\nendpoint = \"http://localhost:8081\"").unwrap();
        let r = cfg.resolve("anything");
        assert_eq!(r.codec.unwrap().endpoint, "http://localhost:8081");
        assert!(!r.readonly);
        assert_eq!(r.accent, None);
    }

    #[test]
    fn a_profile_codec_overrides_the_global_one() {
        let cfg = parse_config(
            r#"
[codec]
endpoint = "http://localhost:8081"

[profile.prod.codec]
endpoint = "https://codec.internal"
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.resolve("prod").codec.unwrap().endpoint,
            "https://codec.internal"
        );
        // A profile that names no codec still falls back, rather than losing decoding.
        assert_eq!(
            cfg.resolve("sit").codec.unwrap().endpoint,
            "http://localhost:8081"
        );
    }

    #[test]
    fn a_profile_carries_its_accent_and_readonly_flag() {
        let cfg = parse_config(
            r#"
[profile.prod]
accent   = "red"
readonly = true

[profile.sit]
accent = "green"
"#,
        )
        .unwrap();
        let prod = cfg.resolve("prod");
        assert_eq!(prod.accent, Some(Accent::Red));
        assert!(prod.readonly);

        let sit = cfg.resolve("sit");
        assert_eq!(sit.accent, Some(Accent::Green));
        assert!(!sit.readonly, "readonly must not leak between profiles");
    }

    #[test]
    fn an_unknown_accent_is_reported_rather_than_ignored() {
        // Silently dropping it would leave production painted like everything else, which
        // is the exact failure the accent exists to prevent.
        let err = parse_config("[profile.prod]\naccent = \"crimson\"").unwrap_err();
        assert!(matches!(err, ConfigError::BadAccent { .. }), "{err:?}");
        assert!(err.to_string().contains("crimson"), "{err}");
    }

    #[test]
    fn readonly_must_be_a_boolean() {
        let err = parse_config("[profile.prod]\nreadonly = \"yes\"").unwrap_err();
        assert!(matches!(err, ConfigError::Type { .. }), "{err:?}");
    }

    #[test]
    fn the_payload_pane_defaults_to_bottom() {
        assert_eq!(parse_config("").unwrap().payload_pane, PayloadPane::Bottom);
        assert_eq!(
            parse_config("[layout]\n").unwrap().payload_pane,
            PayloadPane::Bottom
        );
    }

    #[test]
    fn the_payload_pane_can_open_on_the_right() {
        let cfg = parse_config("[layout]\npayload = \"right\"").unwrap();
        assert_eq!(cfg.payload_pane, PayloadPane::Right);
    }

    #[test]
    fn an_unknown_payload_position_is_reported() {
        // "popup" is the one people will try; it is refused rather than quietly ignored.
        for src in [
            "[layout]\npayload = \"popup\"",
            "[layout]\npayload = 1",
            "layout = 1",
        ] {
            let err = parse_config(src).unwrap_err();
            assert!(matches!(err, ConfigError::Type { .. }), "{src}: {err:?}");
        }
    }
}
