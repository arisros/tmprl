//! `theme.toml`, and what the terminal can show of it.

use crate::config::ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi16,
    Mono,
}

impl ColorDepth {
    pub fn parse_setting(s: &str) -> Option<Option<Self>> {
        match s {
            "auto" => Some(None),
            "truecolor" | "24bit" => Some(Some(Self::TrueColor)),
            "16" => Some(Some(Self::Ansi16)),
            "none" => Some(Some(Self::Mono)),
            _ => None,
        }
    }

    pub fn detect(no_color: Option<&str>, colorterm: Option<&str>) -> Self {
        if no_color.is_some_and(|v| !v.is_empty()) {
            return Self::Mono;
        }
        match colorterm {
            Some("truecolor" | "24bit") => Self::TrueColor,
            _ => Self::Ansi16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ansi {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

impl Ansi {
    const XTERM: [(Self, &'static str, Rgb); 16] = [
        (Self::Black, "black", Rgb(0x00, 0x00, 0x00)),
        (Self::Red, "red", Rgb(0xcd, 0x00, 0x00)),
        (Self::Green, "green", Rgb(0x00, 0xcd, 0x00)),
        (Self::Yellow, "yellow", Rgb(0xcd, 0xcd, 0x00)),
        (Self::Blue, "blue", Rgb(0x00, 0x00, 0xee)),
        (Self::Magenta, "magenta", Rgb(0xcd, 0x00, 0xcd)),
        (Self::Cyan, "cyan", Rgb(0x00, 0xcd, 0xcd)),
        (Self::White, "white", Rgb(0xe5, 0xe5, 0xe5)),
        (Self::BrightBlack, "bright-black", Rgb(0x7f, 0x7f, 0x7f)),
        (Self::BrightRed, "bright-red", Rgb(0xff, 0x00, 0x00)),
        (Self::BrightGreen, "bright-green", Rgb(0x00, 0xff, 0x00)),
        (Self::BrightYellow, "bright-yellow", Rgb(0xff, 0xff, 0x00)),
        (Self::BrightBlue, "bright-blue", Rgb(0x5c, 0x5c, 0xff)),
        (Self::BrightMagenta, "bright-magenta", Rgb(0xff, 0x00, 0xff)),
        (Self::BrightCyan, "bright-cyan", Rgb(0x00, 0xff, 0xff)),
        (Self::BrightWhite, "bright-white", Rgb(0xff, 0xff, 0xff)),
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Self::XTERM
            .iter()
            .find(|(_, name, _)| *name == s)
            .map(|(ansi, _, _)| *ansi)
    }

    pub fn is_grey(self) -> bool {
        matches!(
            self,
            Self::Black | Self::White | Self::BrightBlack | Self::BrightWhite
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self(byte(0)?, byte(2)?, byte(4)?))
    }

    pub fn nearest_ansi(self) -> Ansi {
        use Ansi::*;

        let Self(r, g, b) = self;
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let chroma = max - min;

        if chroma < Self::GREY_BELOW {
            return self.nearer(&[Black, BrightBlack, White, BrightWhite]);
        }

        let (rf, gf, bf, c) = (f32::from(r), f32::from(g), f32::from(b), f32::from(chroma));
        let hue = 60.0
            * if max == r {
                ((gf - bf) / c).rem_euclid(6.0)
            } else if max == g {
                (bf - rf) / c + 2.0
            } else {
                (rf - gf) / c + 4.0
            };
        self.nearer(&match hue {
            h if h < 20.0 => [Red, BrightRed],
            h if h < 70.0 => [Yellow, BrightYellow],
            h if h < 165.0 => [Green, BrightGreen],
            h if h < 200.0 => [Cyan, BrightCyan],
            h if h < 265.0 => [Blue, BrightBlue],
            h if h < 335.0 => [Magenta, BrightMagenta],
            _ => [Red, BrightRed],
        })
    }

    const GREY_BELOW: u8 = 0x30;

    fn nearer(self, candidates: &[Ansi]) -> Ansi {
        let distance = |Rgb(r, g, b): Rgb| {
            let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
            d(self.0, r) + d(self.1, g) + d(self.2, b)
        };
        Ansi::XTERM
            .iter()
            .filter(|(ansi, _, _)| candidates.contains(ansi))
            .min_by_key(|(_, _, rgb)| distance(*rgb))
            .map_or(Ansi::White, |(ansi, _, _)| *ansi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorValue {
    Rgb(Rgb),
    Ansi(Ansi),
}

impl ColorValue {
    pub const EXPECTED: &'static str = "expected #rrggbb, or black, red, green, yellow, blue, \
        magenta, cyan or white, optionally prefixed with bright-";

    pub fn parse(s: &str) -> Option<Self> {
        Rgb::parse(s)
            .map(Self::Rgb)
            .or_else(|| Ansi::parse(s).map(Self::Ansi))
    }

    pub fn to_ansi(self) -> Ansi {
        match self {
            Self::Rgb(rgb) => rgb.nearest_ansi(),
            Self::Ansi(ansi) => ansi,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Fg,
    Dim,
    Faint,
    Accent,
    Ok,
    Warn,
    Err,
    Sel,
    ModeNormal,
    ModeInsert,
    ModeVisual,
    ModeCommand,
}

impl Slot {
    const KEYS: [(&'static str, Self); 12] = [
        ("fg", Self::Fg),
        ("dim", Self::Dim),
        ("faint", Self::Faint),
        ("accent", Self::Accent),
        ("ok", Self::Ok),
        ("warn", Self::Warn),
        ("err", Self::Err),
        ("sel", Self::Sel),
        ("mode_normal", Self::ModeNormal),
        ("mode_insert", Self::ModeInsert),
        ("mode_visual", Self::ModeVisual),
        ("mode_command", Self::ModeCommand),
    ];

    pub const NAMES: &'static str = "fg, dim, faint, accent, ok, warn, err, sel, mode_normal, \
        mode_insert, mode_visual or mode_command";

    pub fn parse(s: &str) -> Option<Self> {
        Self::KEYS
            .iter()
            .find(|(key, _)| *key == s)
            .map(|(_, slot)| *slot)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeOverrides(pub Vec<(Slot, ColorValue)>);

pub fn parse_theme(src: &str) -> Result<ThemeOverrides, ConfigError> {
    const FILE: &str = "theme.toml";
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;

    let mut out = Vec::with_capacity(table.len());
    for (key, raw) in &table {
        let slot = Slot::parse(key).ok_or_else(|| ConfigError::UnknownThemeSlot {
            key: key.clone(),
            expected: Slot::NAMES,
        })?;
        let text = raw.as_str().ok_or_else(|| ConfigError::Type {
            file: FILE,
            path: key.clone(),
            expected: "a string",
        })?;
        let value = ColorValue::parse(text).ok_or_else(|| ConfigError::BadColor {
            key: key.clone(),
            value: text.to_string(),
            expected: ColorValue::EXPECTED,
        })?;
        out.push((slot, value));
    }
    Ok(ThemeOverrides(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_wins_over_everything() {
        assert_eq!(
            ColorDepth::detect(Some("1"), Some("truecolor")),
            ColorDepth::Mono
        );
    }

    #[test]
    fn an_empty_no_color_is_not_set() {
        assert_eq!(
            ColorDepth::detect(Some(""), Some("truecolor")),
            ColorDepth::TrueColor
        );
        assert_eq!(ColorDepth::detect(Some(""), None), ColorDepth::Ansi16);
    }

    #[test]
    fn colorterm_is_what_grants_truecolor() {
        assert_eq!(
            ColorDepth::detect(None, Some("truecolor")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::detect(None, Some("24bit")),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn anything_else_is_sixteen_colours() {
        assert_eq!(ColorDepth::detect(None, None), ColorDepth::Ansi16);
        assert_eq!(ColorDepth::detect(None, Some("")), ColorDepth::Ansi16);
        assert_eq!(
            ColorDepth::detect(None, Some("rxvt-xpm")),
            ColorDepth::Ansi16
        );
    }

    #[test]
    fn a_hex_colour_parses_in_either_case() {
        assert_eq!(Rgb::parse("#6b99e8"), Some(Rgb(0x6b, 0x99, 0xe8)));
        assert_eq!(Rgb::parse("#6B99E8"), Some(Rgb(0x6b, 0x99, 0xe8)));
    }

    #[test]
    fn a_malformed_hex_is_refused() {
        for bad in ["6b99e8", "#6b9", "#6b99e8ff", "#6b99eg", "#", "", "#+b99e8"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn every_ansi_name_round_trips() {
        for (ansi, name, _) in Ansi::XTERM {
            assert_eq!(Ansi::parse(name), Some(ansi), "{name}");
            assert_eq!(ColorValue::parse(name), Some(ColorValue::Ansi(ansi)));
        }
        assert_eq!(Ansi::parse("Red"), None);
        assert_eq!(Ansi::parse("purple"), None);
    }

    #[test]
    fn a_hex_degrades_to_the_nearest_named_colour() {
        assert_eq!(Rgb(0xf0, 0xc6, 0x74).nearest_ansi(), Ansi::Yellow);
        assert_eq!(Rgb(0x00, 0xd7, 0x5f).nearest_ansi(), Ansi::Green);
        assert_eq!(Rgb(0x6b, 0x99, 0xe8).nearest_ansi(), Ansi::BrightBlue);
        assert_eq!(Rgb(0xa7, 0xb8, 0x79).nearest_ansi(), Ansi::Green);
        assert_eq!(Rgb(0xff, 0x00, 0x7c).nearest_ansi(), Ansi::Magenta);
    }

    #[test]
    fn a_soft_colour_keeps_its_hue_rather_than_going_grey() {
        assert_eq!(Rgb(0xe0, 0x52, 0x52).nearest_ansi(), Ansi::Red);
        assert_eq!(Rgb(0x30, 0xa4, 0x6c).nearest_ansi(), Ansi::Green);
        assert_eq!(Rgb(0x00, 0x90, 0xff).nearest_ansi(), Ansi::BrightBlue);
    }

    #[test]
    fn a_colour_with_no_hue_is_a_grey() {
        assert_eq!(Rgb(0x10, 0x10, 0x10).nearest_ansi(), Ansi::Black);
        assert_eq!(Rgb(0x5a, 0x62, 0x70).nearest_ansi(), Ansi::BrightBlack);
        assert_eq!(Rgb(0xd8, 0xdc, 0xe4).nearest_ansi(), Ansi::White);
        assert_eq!(Rgb(0xff, 0xff, 0xff).nearest_ansi(), Ansi::BrightWhite);
        for grey in [Rgb(0, 0, 0), Rgb(0x80, 0x80, 0x80), Rgb(0xed, 0xf2, 0xfe)] {
            assert!(grey.nearest_ansi().is_grey(), "{grey:?}");
        }
    }

    #[test]
    fn every_colour_maps_to_something() {
        for r in [0u8, 0x7f, 0xff] {
            for g in [0u8, 0x7f, 0xff] {
                for b in [0u8, 0x7f, 0xff] {
                    let _ = Rgb(r, g, b).nearest_ansi();
                }
            }
        }
        assert_eq!(Rgb(0xff, 0x00, 0x00).nearest_ansi(), Ansi::BrightRed);
        assert_eq!(Rgb(0xff, 0x00, 0x01).nearest_ansi(), Ansi::BrightRed);
        assert_eq!(Rgb(0x00, 0x00, 0xee).nearest_ansi(), Ansi::Blue);
    }

    #[test]
    fn a_named_colour_degrades_to_itself() {
        assert_eq!(ColorValue::Ansi(Ansi::Cyan).to_ansi(), Ansi::Cyan);
        assert_eq!(
            ColorValue::Rgb(Rgb(0xcd, 0x00, 0x00)).to_ansi(),
            Ansi::Red,
            "an exact xterm value is that colour"
        );
    }

    #[test]
    fn a_theme_overrides_only_the_slots_it_names() {
        let theme = parse_theme("accent = \"#6b99e8\"\nerr = \"bright-red\"\n").unwrap();
        assert_eq!(
            theme.0,
            vec![
                (Slot::Accent, ColorValue::Rgb(Rgb(0x6b, 0x99, 0xe8))),
                (Slot::Err, ColorValue::Ansi(Ansi::BrightRed)),
            ]
        );
    }

    #[test]
    fn an_empty_theme_overrides_nothing() {
        assert_eq!(
            parse_theme("# just a comment\n"),
            Ok(ThemeOverrides::default())
        );
    }

    #[test]
    fn every_slot_is_a_key() {
        for (key, slot) in Slot::KEYS {
            let theme = parse_theme(&format!("{key} = \"red\"")).unwrap();
            assert_eq!(theme.0, vec![(slot, ColorValue::Ansi(Ansi::Red))]);
            assert!(Slot::NAMES.contains(key), "{key} missing from the message");
        }
    }

    #[test]
    fn an_unknown_slot_is_an_error() {
        let err = parse_theme("acent = \"red\"").unwrap_err();
        assert!(
            matches!(&err, ConfigError::UnknownThemeSlot { key, .. } if key == "acent"),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("theme.toml") && msg.contains("acent"), "{msg}");
    }

    #[test]
    fn a_table_is_not_a_slot() {
        assert!(parse_theme("[colors]\naccent = \"red\"").is_err());
    }

    #[test]
    fn an_unparseable_colour_is_an_error() {
        let err = parse_theme("accent = \"bluish\"").unwrap_err();
        assert!(
            matches!(&err, ConfigError::BadColor { key, value, .. }
                if key == "accent" && value == "bluish"),
            "{err:?}"
        );
        assert!(err.to_string().contains("#rrggbb"), "{err}");
    }

    #[test]
    fn a_colour_that_is_not_a_string_is_an_error() {
        assert_eq!(
            parse_theme("accent = 4"),
            Err(ConfigError::Type {
                file: "theme.toml",
                path: "accent".into(),
                expected: "a string",
            })
        );
    }

    #[test]
    fn malformed_toml_is_a_syntax_error() {
        assert!(matches!(
            parse_theme("accent = "),
            Err(ConfigError::Syntax {
                file: "theme.toml",
                ..
            })
        ));
    }
}
