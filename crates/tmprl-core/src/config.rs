//! `config.toml`, `keys.toml` and `views.toml`. `theme.toml` is parsed in [`crate::theme`]

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
    #[error("config.toml: `timezone` is `{value}`, which is not a zone: {message}")]
    BadTimezone { value: String, message: String },
    #[error("config.toml: `{path}` is `{value}`, which is not a colour ({expected})")]
    BadAccent {
        path: String,
        value: String,
        expected: &'static str,
    },
    #[error("theme.toml: `{key}` is not a colour slot (expected {expected})")]
    UnknownThemeSlot { key: String, expected: &'static str },
    #[error("theme.toml: `{key}` is `{value}`, which is not a colour ({expected})")]
    BadColor {
        key: String,
        value: String,
        expected: &'static str,
    },
    #[error("dashboard.toml: `{key}` is not a key of {place} (expected {expected})")]
    UnknownDashboardKey {
        place: String,
        key: String,
        expected: &'static str,
    },
    #[error(
        "dashboard.toml: `{path}` is `{value}`, which is not a panel kind (expected counts, workflows, types, queues, histogram, retrying or schedules)"
    )]
    BadPanelKind { path: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedView {
    pub key: char,
    pub name: String,
    pub query: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecConfig {
    pub endpoint: String,
    pub auth: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKey {
    Literal(String),
    Command(String),
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileConfig {
    pub accent: Option<Accent>,
    pub readonly: bool,
    pub codec: Option<CodecConfig>,
    pub api_key: Option<ApiKey>,
    pub env: bool,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            accent: None,
            readonly: false,
            codec: None,
            api_key: None,
            env: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub accent: Option<Accent>,
    pub readonly: bool,
    pub codec: Option<CodecConfig>,
    pub api_key: Option<ApiKey>,
    pub env: bool,
}

impl Default for Resolved {
    fn default() -> Self {
        Self {
            accent: None,
            readonly: false,
            codec: None,
            api_key: None,
            env: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PayloadPane {
    Bottom,
    #[default]
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

pub const PAYLOAD_SIZE: u16 = 50;
pub const PAYLOAD_SIZE_MIN: u16 = 20;
pub const PAYLOAD_SIZE_MAX: u16 = 80;

pub fn clamp_payload_size(percent: i32) -> u16 {
    percent.clamp(PAYLOAD_SIZE_MIN as i32, PAYLOAD_SIZE_MAX as i32) as u16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Off,
    Every(i64),
}

impl Refresh {
    pub const FLOOR_MS: i64 = 5_000;

    pub fn interval_ms(self) -> Option<i64> {
        match self {
            Refresh::Off => None,
            Refresh::Every(ms) => Some(ms),
        }
    }
}

impl Default for Refresh {
    fn default() -> Self {
        Refresh::Every(30_000)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub codec: Option<CodecConfig>,
    pub timezone: Option<String>,
    pub profiles: Vec<(String, ProfileConfig)>,
    pub payload_pane: PayloadPane,
    pub payload_size: u16,
    pub dashboard_refresh: Refresh,
    pub yank_max: usize,
    pub source: Option<String>,
    pub mouse: bool,
    pub colors: Option<crate::theme::ColorDepth>,
}

// Terminals commonly cap OSC 52 around 100 KB and truncate silently.
pub const DEFAULT_YANK_MAX: usize = 64 * 1024;

impl Default for Config {
    fn default() -> Self {
        Self {
            codec: None,
            timezone: None,
            profiles: Vec::new(),
            payload_pane: PayloadPane::default(),
            payload_size: PAYLOAD_SIZE,
            dashboard_refresh: Refresh::default(),
            yank_max: DEFAULT_YANK_MAX,
            mouse: false,
            source: None,
            colors: None,
        }
    }
}

impl Config {
    pub fn resolve(&self, profile: &str) -> Resolved {
        let found = self.profiles.iter().find(|(name, _)| name == profile);
        match found {
            None => Resolved {
                codec: self.codec.clone(),
                ..Resolved::default()
            },
            Some((_, p)) => Resolved {
                accent: p.accent,
                readonly: p.readonly,
                codec: p.codec.clone().or_else(|| self.codec.clone()),
                api_key: p.api_key.clone(),
                env: p.env,
            },
        }
    }
}

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

    let timezone = match table.get("timezone") {
        None => None,
        Some(v) => {
            let name = v.as_str().ok_or(ConfigError::Type {
                file: FILE,
                path: "timezone".into(),
                expected: "a string",
            })?;
            crate::clock::Clock::named(name).map_err(|message| ConfigError::BadTimezone {
                value: name.to_string(),
                message,
            })?;
            Some(name.to_string())
        }
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

    let (payload_pane, payload_size) = match table.get("layout") {
        None => (PayloadPane::default(), PAYLOAD_SIZE),
        Some(raw) => parse_layout(raw)?,
    };

    let dashboard_refresh = match table.get("refresh") {
        None => Refresh::default(),
        Some(raw) => parse_refresh(raw)?,
    };

    let yank_max = match table.get("yank") {
        None => DEFAULT_YANK_MAX,
        Some(raw) => parse_yank(raw)?,
    };

    let source = match table.get("source") {
        None => None,
        Some(raw) => parse_source(raw)?,
    };

    let mouse = match table.get("input") {
        None => false,
        Some(raw) => parse_input(raw)?,
    };

    let colors = match table.get("display") {
        None => None,
        Some(raw) => parse_display(raw)?,
    };

    Ok(Config {
        codec,
        timezone,
        profiles,
        payload_pane,
        payload_size,
        dashboard_refresh,
        yank_max,
        source,
        mouse,
        colors,
    })
}

fn parse_source(raw: &toml::Value) -> Result<Option<String>, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "source".into(),
        expected: "a table",
    })?;
    match table.get("command") {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Some(s.to_string()))
            .ok_or(ConfigError::Type {
                file: FILE,
                path: "source.command".into(),
                expected: "a program and its arguments, such as \"~/bin/find-source\"",
            }),
    }
}

fn parse_refresh(raw: &toml::Value) -> Result<Refresh, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "refresh".into(),
        expected: "a table",
    })?;
    let Some(value) = table.get("dashboard") else {
        return Ok(Refresh::default());
    };
    match value.as_str() {
        Some("off") => Some(Refresh::Off),
        Some(text) => crate::timerange::parse_offset(text)
            .filter(|ms| *ms >= Refresh::FLOOR_MS)
            .map(Refresh::Every),
        None => None,
    }
    .ok_or(ConfigError::Type {
        file: FILE,
        path: "refresh.dashboard".into(),
        expected: "\"off\" or a duration of at least 5s, such as \"30s\" or \"2m\"",
    })
}

fn parse_yank(raw: &toml::Value) -> Result<usize, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "yank".into(),
        expected: "a table",
    })?;
    match table.get("max_bytes") {
        None => Ok(DEFAULT_YANK_MAX),
        Some(v) => v
            .as_integer()
            .filter(|n| *n > 0)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(ConfigError::Type {
                file: FILE,
                path: "yank.max_bytes".into(),
                expected: "a number of bytes above zero",
            }),
    }
}

fn parse_input(raw: &toml::Value) -> Result<bool, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "input".into(),
        expected: "a table",
    })?;
    match table.get("mouse") {
        None => Ok(false),
        Some(v) => v.as_bool().ok_or(ConfigError::Type {
            file: FILE,
            path: "input.mouse".into(),
            expected: "true or false",
        }),
    }
}

fn parse_display(raw: &toml::Value) -> Result<Option<crate::theme::ColorDepth>, ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "display".into(),
        expected: "a table",
    })?;
    match table.get("colors") {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(crate::theme::ColorDepth::parse_setting)
            .ok_or(ConfigError::Type {
                file: FILE,
                path: "display.colors".into(),
                expected: "\"auto\", \"truecolor\", \"16\" or \"none\"",
            }),
    }
}

fn parse_layout(raw: &toml::Value) -> Result<(PayloadPane, u16), ConfigError> {
    const FILE: &str = "config.toml";
    let table = raw.as_table().ok_or(ConfigError::Type {
        file: FILE,
        path: "layout".into(),
        expected: "a table",
    })?;
    let pane = match table.get("payload") {
        None => PayloadPane::default(),
        Some(v) => v
            .as_str()
            .and_then(PayloadPane::parse)
            .ok_or(ConfigError::Type {
                file: FILE,
                path: "layout.payload".into(),
                expected: "\"bottom\" or \"right\"",
            })?,
    };
    let size = match table.get("payload_size") {
        None => PAYLOAD_SIZE,
        Some(v) => v
            .as_integer()
            .filter(|n| (PAYLOAD_SIZE_MIN as i64..=PAYLOAD_SIZE_MAX as i64).contains(n))
            .map(|n| n as u16)
            .ok_or(ConfigError::Type {
                file: FILE,
                path: "layout.payload_size".into(),
                expected: "a percentage from 20 to 80",
            })?,
    };
    Ok((pane, size))
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

    let text = |key: &str, expected: &'static str| match table.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| ConfigError::Type {
                file: FILE,
                path: format!("profile.{name}.{key}"),
                expected,
            }),
    };
    let api_key = match (
        text("api_key", "a non-empty key")?,
        text("api_key_cmd", "a non-empty command")?,
    ) {
        (Some(_), Some(_)) => {
            return Err(ConfigError::Type {
                file: FILE,
                path: format!("profile.{name}.api_key"),
                expected: "absent when api_key_cmd is set",
            });
        }
        (Some(key), None) => Some(ApiKey::Literal(key)),
        (None, Some(cmd)) => Some(ApiKey::Command(cmd)),
        (None, None) => None,
    };

    let env = match table.get("env") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| ConfigError::Type {
            file: FILE,
            path: format!("profile.{name}.env"),
            expected: "true or false",
        })?,
    };

    Ok(ProfileConfig {
        accent,
        readonly,
        codec,
        api_key,
        env,
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
        assert_eq!(
            cfg.resolve("sit").codec.unwrap().endpoint,
            "http://localhost:8081"
        );
    }

    #[test]
    fn a_profile_carries_its_key_command_and_ignores_the_environment() {
        let cfg = parse_config(
            r#"
[profile.prod]
api_key_cmd = "pass show temporal/prod"
env         = false
"#,
        )
        .unwrap();
        let r = cfg.resolve("prod");
        assert_eq!(
            r.api_key,
            Some(ApiKey::Command("pass show temporal/prod".into()))
        );
        assert!(!r.env);
        let other = cfg.resolve("sit");
        assert_eq!(other.api_key, None);
        assert!(
            other.env,
            "the environment applies unless a profile says otherwise"
        );
    }

    #[test]
    fn a_profile_can_carry_its_key_as_written() {
        let cfg = parse_config("[profile.sit]\napi_key = \"secret\"").unwrap();
        assert_eq!(
            cfg.resolve("sit").api_key,
            Some(ApiKey::Literal("secret".into()))
        );
    }

    #[test]
    fn a_key_and_a_key_command_together_are_refused() {
        let err =
            parse_config("[profile.sit]\napi_key = \"secret\"\napi_key_cmd = \"pass show x\"")
                .unwrap_err();
        assert!(
            matches!(err, ConfigError::Type { ref path, .. } if path == "profile.sit.api_key"),
            "got {err}"
        );
    }

    #[test]
    fn an_empty_key_command_is_refused() {
        let err = parse_config("[profile.prod]\napi_key_cmd = \"  \"").unwrap_err();
        assert!(
            matches!(err, ConfigError::Type { ref path, .. } if path == "profile.prod.api_key_cmd"),
            "got {err}"
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
    fn the_payload_pane_defaults_to_the_right_at_half() {
        let cfg = parse_config("").unwrap();
        assert_eq!(cfg.payload_pane, PayloadPane::Right);
        assert_eq!(cfg.payload_size, PAYLOAD_SIZE);
        assert_eq!(
            parse_config("[layout]\n").unwrap().payload_pane,
            PayloadPane::Right
        );
    }

    #[test]
    fn the_payload_pane_size_is_a_bounded_percentage() {
        let cfg = parse_config("[layout]\npayload = \"bottom\"\npayload_size = 30").unwrap();
        assert_eq!(
            (cfg.payload_pane, cfg.payload_size),
            (PayloadPane::Bottom, 30)
        );
        for src in [
            "[layout]\npayload_size = 10",
            "[layout]\npayload_size = 90",
            "[layout]\npayload_size = \"half\"",
        ] {
            let err = parse_config(src).unwrap_err();
            assert!(matches!(err, ConfigError::Type { .. }), "{src}: {err:?}");
        }
        assert_eq!(clamp_payload_size(5), PAYLOAD_SIZE_MIN);
        assert_eq!(clamp_payload_size(95), PAYLOAD_SIZE_MAX);
        assert_eq!(clamp_payload_size(55), 55);
    }

    #[test]
    fn the_yank_limit_defaults_to_64_kb() {
        assert_eq!(parse_config("").unwrap().source, None);
        assert_eq!(
            parse_config("[source]\ncommand = \" ~/bin/find-source -x \"")
                .unwrap()
                .source
                .as_deref(),
            Some("~/bin/find-source -x")
        );
        assert!(parse_config("[source]\ncommand = 3").is_err());
        assert!(parse_config("[source]\ncommand = \"\"").is_err());
        assert_eq!(parse_config("").unwrap().yank_max, DEFAULT_YANK_MAX);
        assert_eq!(parse_config("[yank]\n").unwrap().yank_max, 64 * 1024);
    }

    #[test]
    fn colours_can_be_forced_where_the_environment_does_not_say() {
        use crate::theme::ColorDepth;
        let colors = |src: &str| parse_config(src).unwrap().colors;
        assert_eq!(colors(""), None);
        assert_eq!(colors("[display]\ncolors = \"auto\""), None);
        assert_eq!(
            colors("[display]\ncolors = \"truecolor\""),
            Some(ColorDepth::TrueColor)
        );
        assert_eq!(
            colors("[display]\ncolors = \"16\""),
            Some(ColorDepth::Ansi16)
        );
        assert_eq!(
            colors("[display]\ncolors = \"none\""),
            Some(ColorDepth::Mono)
        );
        assert!(matches!(
            parse_config("[display]\ncolors = \"256\""),
            Err(ConfigError::Type { path, .. }) if path == "display.colors"
        ));
    }

    #[test]
    fn the_mouse_is_off_until_asked_for() {
        assert!(!parse_config("").unwrap().mouse);
        assert!(!parse_config("[input]\n").unwrap().mouse);
        assert!(parse_config("[input]\nmouse = true").unwrap().mouse);
        assert!(parse_config("[input]\nmouse = \"yes\"").is_err());
    }

    #[test]
    fn the_yank_limit_can_be_raised() {
        let cfg = parse_config("[yank]\nmax_bytes = 1048576").unwrap();
        assert_eq!(cfg.yank_max, 1_048_576);
    }

    #[test]
    fn a_yank_limit_that_is_not_a_positive_number_is_reported() {
        for src in [
            "[yank]\nmax_bytes = 0",
            "[yank]\nmax_bytes = -1",
            "[yank]\nmax_bytes = \"1MB\"",
            "yank = 4096",
        ] {
            let err = parse_config(src).unwrap_err();
            assert!(matches!(err, ConfigError::Type { .. }), "{src}: {err:?}");
        }
    }

    #[test]
    fn the_payload_pane_can_open_at_the_bottom() {
        let cfg = parse_config("[layout]\npayload = \"bottom\"").unwrap();
        assert_eq!(cfg.payload_pane, PayloadPane::Bottom);
    }

    #[test]
    fn an_unknown_payload_position_is_reported() {
        for src in [
            "[layout]\npayload = \"popup\"",
            "[layout]\npayload = 1",
            "layout = 1",
        ] {
            let err = parse_config(src).unwrap_err();
            assert!(matches!(err, ConfigError::Type { .. }), "{src}: {err:?}");
        }
    }

    #[test]
    fn a_zone_name_survives_the_round_trip_into_the_config() {
        let c = parse_config("timezone = \"Asia/Jakarta\"\n").unwrap();
        assert_eq!(c.timezone.as_deref(), Some("Asia/Jakarta"));
        assert_eq!(parse_config("").unwrap().timezone, None);
    }

    #[test]
    fn a_zone_that_does_not_exist_is_rejected_at_parse_time() {
        let err = parse_config("timezone = \"Asia/Jakata\"\n").unwrap_err();
        assert!(
            matches!(err, ConfigError::BadTimezone { ref value, .. } if value == "Asia/Jakata"),
            "{err}"
        );
        assert!(err.to_string().contains("Asia/Jakata"), "{err}");
    }

    #[test]
    fn the_dashboard_refreshes_every_thirty_seconds_unless_told_otherwise() {
        assert_eq!(
            parse_config("").unwrap().dashboard_refresh,
            Refresh::Every(30_000)
        );
        assert_eq!(
            parse_config("[refresh]\ndashboard = \"2m\"")
                .unwrap()
                .dashboard_refresh,
            Refresh::Every(120_000)
        );
        assert_eq!(
            parse_config("[refresh]\ndashboard = \"off\"")
                .unwrap()
                .dashboard_refresh
                .interval_ms(),
            None
        );
    }

    #[test]
    fn a_refresh_interval_must_be_a_duration_of_five_seconds_or_more() {
        for value in ["\"2s\"", "\"soon\"", "30", "\"\""] {
            let result = parse_config(&format!("[refresh]\ndashboard = {value}"));
            assert!(
                matches!(&result, Err(ConfigError::Type { path, .. }) if path == "refresh.dashboard"),
                "{value}: {result:?}"
            );
        }
        assert_eq!(
            parse_config("[refresh]\ndashboard = \"5s\"")
                .unwrap()
                .dashboard_refresh,
            Refresh::Every(5_000)
        );
    }
}
