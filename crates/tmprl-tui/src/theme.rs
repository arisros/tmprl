//! Colours.
//!
//! One palette of named slots, built once at startup for the colour depth the terminal was
//! found to have, then repainted by `theme.toml`. A slot is a whole [`Style`] rather than a
//! colour, because below truecolor some slots have no colour to give: `faint` is the
//! terminal's own foreground dimmed, and the cursor row is reverse video. Renderers ask for
//! the slot and never learn which depth they are drawing at.
//!
//! The truecolor defaults are derived from the `twilight256` palette so the interface sits
//! comfortably beside an editor themed the same way. They assume a dark background. The
//! 16-colour defaults assume nothing: they are the terminal's own named colours and its own
//! foreground, so they follow whatever theme the terminal has, light ones included.

use ratatui::style::{Color, Modifier, Style};
use tmprl_core::Mode;
use tmprl_core::config::Accent;
use tmprl_core::theme::{Ansi, ColorDepth, ColorValue, Rgb, Slot, ThemeOverrides};

/// What the terminal tmprl was started in can show. Read once: the rule itself is
/// [`ColorDepth::detect`], which takes the two variables as arguments so it can be tested
/// without touching the process environment.
pub fn depth_from_env() -> ColorDepth {
    // Lossy rather than `env::var`: a `NO_COLOR` that is not valid UTF-8 is still set.
    let var = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    ColorDepth::detect(var("NO_COLOR").as_deref(), var("COLORTERM").as_deref())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    depth: ColorDepth,
    pub fg: Style,
    pub dim: Style,
    pub faint: Style,
    pub accent: Style,
    pub ok: Style,
    pub warn: Style,
    pub err: Style,
    /// Patched over a row's style to mark the cursor row and a selection.
    pub sel: Style,
    /// The mode indicator's background, a distinct hue per mode as lualine does it.
    /// `None` without colour, where the label alone says which mode it is.
    mode_normal: Option<Color>,
    mode_insert: Option<Color>,
    mode_visual: Option<Color>,
    mode_command: Option<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ColorDepth::TrueColor, &ThemeOverrides::default())
    }
}

impl Theme {
    /// The defaults for `depth`, with the slots `overrides` names repainted.
    ///
    /// Without colour the overrides are dropped: `NO_COLOR` is the user saying so outright,
    /// and it outranks a file they wrote for some other terminal.
    pub fn new(depth: ColorDepth, overrides: &ThemeOverrides) -> Self {
        let mut theme = match depth {
            ColorDepth::TrueColor => Self::truecolor(),
            ColorDepth::Ansi16 => Self::ansi16(),
            ColorDepth::Mono => return Self::mono(),
        };
        for (slot, value) in &overrides.0 {
            let color = match depth {
                ColorDepth::TrueColor => color(*value),
                _ => ansi(value.to_ansi()),
            };
            match slot {
                Slot::Fg => theme.fg = Style::new().fg(color),
                Slot::Dim => theme.dim = Style::new().fg(color),
                Slot::Faint => theme.faint = Style::new().fg(color),
                Slot::Accent => theme.accent = Style::new().fg(color),
                Slot::Ok => theme.ok = Style::new().fg(color),
                Slot::Warn => theme.warn = Style::new().fg(color),
                Slot::Err => theme.err = Style::new().fg(color),
                Slot::Sel => theme.sel = Style::new().bg(color),
                Slot::ModeNormal => theme.mode_normal = Some(color),
                Slot::ModeInsert => theme.mode_insert = Some(color),
                Slot::ModeVisual => theme.mode_visual = Some(color),
                Slot::ModeCommand => theme.mode_command = Some(color),
            }
        }
        theme
    }

    fn truecolor() -> Self {
        let fg = |r, g, b| Style::new().fg(Color::Rgb(r, g, b));
        Self {
            depth: ColorDepth::TrueColor,
            fg: fg(0xd8, 0xdc, 0xe4),
            dim: fg(0x8a, 0x93, 0xa3),
            faint: fg(0x5a, 0x62, 0x70),
            accent: fg(0x6b, 0x99, 0xe8), // Identifier
            ok: fg(0xa7, 0xb8, 0x79),     // String
            warn: fg(0xf0, 0xc6, 0x74),   // Function
            err: fg(0xe0, 0x52, 0x52),
            sel: Style::new().bg(Color::Rgb(0x2c, 0x33, 0x40)),
            mode_normal: Some(Color::Rgb(0xff, 0x00, 0x7c)),
            mode_insert: Some(Color::Rgb(0x00, 0xd7, 0x5f)),
            mode_visual: Some(Color::Rgb(0xff, 0xaf, 0x00)),
            mode_command: Some(Color::Rgb(0x8b, 0x5f, 0xff)),
        }
    }

    /// Only colours with a hue are named. The greys are where terminal themes disagree
    /// most, bright black is the background itself under Solarized, so text stays the
    /// terminal's own foreground and the cursor row is reverse video: both are readable
    /// on any background by construction.
    fn ansi16() -> Self {
        Self {
            depth: ColorDepth::Ansi16,
            fg: Style::new(),
            dim: Style::new(),
            faint: Style::new().add_modifier(Modifier::DIM),
            accent: Style::new().fg(Color::Blue),
            ok: Style::new().fg(Color::Green),
            warn: Style::new().fg(Color::Yellow),
            err: Style::new().fg(Color::Red),
            sel: Style::new().add_modifier(Modifier::REVERSED),
            mode_normal: Some(Color::Magenta),
            mode_insert: Some(Color::Green),
            mode_visual: Some(Color::Yellow),
            mode_command: Some(Color::Cyan),
        }
    }

    fn mono() -> Self {
        Self {
            depth: ColorDepth::Mono,
            fg: Style::new(),
            dim: Style::new(),
            faint: Style::new().add_modifier(Modifier::DIM),
            accent: Style::new(),
            ok: Style::new(),
            warn: Style::new(),
            // The one slot whose meaning cannot be left to the glyph beside it: an error
            // in the statusline is only its text.
            err: Style::new().add_modifier(Modifier::BOLD),
            sel: Style::new().add_modifier(Modifier::REVERSED),
            mode_normal: None,
            mode_insert: None,
            mode_visual: None,
            mode_command: None,
        }
    }

    /// The colour a profile's `accent` names.
    ///
    /// The basic ANSI colours rather than the palette's RGB: this has to survive a
    /// 16-colour terminal, since the whole point is that production cannot be mistaken
    /// for SIT.
    pub fn profile_accent(&self, accent: Accent) -> Style {
        if self.depth == ColorDepth::Mono {
            return Style::new();
        }
        Style::new().fg(match accent {
            Accent::Red => Color::Red,
            Accent::Green => Color::Green,
            Accent::Yellow => Color::Yellow,
            Accent::Blue => Color::Blue,
            Accent::Magenta => Color::Magenta,
            Accent::Cyan => Color::Cyan,
        })
    }

    /// The mode indicator: the mode's hue as a block behind its label.
    pub fn mode_badge(&self, mode: Mode) -> Style {
        badge(match mode {
            Mode::Normal => self.mode_normal,
            Mode::Insert => self.mode_insert,
            Mode::Visual | Mode::VisualLine => self.mode_visual,
            Mode::Command => self.mode_command,
        })
    }

    /// A block in a slot's colour, for a label that has to be seen from across the room.
    pub fn badge(&self, slot: Style) -> Style {
        badge(slot.fg)
    }

    /// A pane's border. Focus is the accent, and where the accent has no colour to show it
    /// with, weight: the focused border is bold against the others' dim.
    pub fn border(&self, focused: bool) -> Style {
        match (focused, self.accent.fg) {
            (false, _) => self.faint,
            (true, Some(_)) => self.accent,
            (true, None) => self.accent.add_modifier(Modifier::BOLD),
        }
    }

    /// The characters a fuzzy match landed on. Underlined as well where there is no accent
    /// colour, since the row they sit in may already be bold.
    pub fn hit(&self, base: Style) -> Style {
        let hit = base.patch(self.accent).add_modifier(Modifier::BOLD);
        match self.accent.fg {
            Some(_) => hit,
            None => hit.add_modifier(Modifier::UNDERLINED),
        }
    }

    /// A colour that is not a slot, the timeline's, brought down to what this terminal has.
    ///
    /// Sixteen colours get the nearest hue. A colour that degrades to a grey is left as the
    /// terminal's foreground instead: black and white are each invisible on one of the two
    /// backgrounds, and a grey carried no hue worth keeping.
    pub fn paint(&self, color: Color) -> Style {
        match (self.depth, color) {
            (ColorDepth::Mono, _) => Style::new(),
            (ColorDepth::Ansi16, Color::Rgb(r, g, b)) => match Rgb(r, g, b).nearest_ansi() {
                grey if grey.is_grey() => Style::new(),
                hue => Style::new().fg(ansi(hue)),
            },
            _ => Style::new().fg(color),
        }
    }
}

fn badge(color: Option<Color>) -> Style {
    match color {
        Some(c) => Style::new().fg(Color::Black).bg(c),
        None => Style::new().add_modifier(Modifier::REVERSED),
    }
    .add_modifier(Modifier::BOLD)
}

fn color(value: ColorValue) -> Color {
    match value {
        ColorValue::Rgb(Rgb(r, g, b)) => Color::Rgb(r, g, b),
        ColorValue::Ansi(a) => ansi(a),
    }
}

/// ratatui calls ANSI 7 `Gray` and ANSI 15 `White`; the terminal's names are the ones
/// `theme.toml` uses.
fn ansi(a: Ansi) -> Color {
    match a {
        Ansi::Black => Color::Black,
        Ansi::Red => Color::Red,
        Ansi::Green => Color::Green,
        Ansi::Yellow => Color::Yellow,
        Ansi::Blue => Color::Blue,
        Ansi::Magenta => Color::Magenta,
        Ansi::Cyan => Color::Cyan,
        Ansi::White => Color::Gray,
        Ansi::BrightBlack => Color::DarkGray,
        Ansi::BrightRed => Color::LightRed,
        Ansi::BrightGreen => Color::LightGreen,
        Ansi::BrightYellow => Color::LightYellow,
        Ansi::BrightBlue => Color::LightBlue,
        Ansi::BrightMagenta => Color::LightMagenta,
        Ansi::BrightCyan => Color::LightCyan,
        Ansi::BrightWhite => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmprl_core::theme::parse_theme;

    fn themed(depth: ColorDepth, src: &str) -> Theme {
        Theme::new(depth, &parse_theme(src).unwrap())
    }

    fn slots(t: &Theme) -> [Style; 8] {
        [t.fg, t.dim, t.faint, t.accent, t.ok, t.warn, t.err, t.sel]
    }

    const MODES: [Mode; 5] = [
        Mode::Normal,
        Mode::Insert,
        Mode::Visual,
        Mode::VisualLine,
        Mode::Command,
    ];

    #[test]
    fn without_colour_no_slot_carries_one() {
        let t = Theme::new(ColorDepth::Mono, &ThemeOverrides::default());
        for style in slots(&t) {
            assert_eq!((style.fg, style.bg), (None, None), "{style:?}");
        }
        for mode in MODES {
            let badge = t.mode_badge(mode);
            assert_eq!((badge.fg, badge.bg), (None, None), "{mode:?}");
        }
        assert_eq!(t.profile_accent(Accent::Red), Style::new());
        assert_eq!(t.paint(Color::Rgb(0xce, 0x2c, 0x31)), Style::new());
        assert_eq!(t.badge(t.ok).bg, None);
    }

    #[test]
    fn without_colour_the_things_that_must_be_told_apart_still_are() {
        let t = Theme::new(ColorDepth::Mono, &ThemeOverrides::default());
        // The cursor row and a selection, against an ordinary row.
        assert!(t.sel.add_modifier.contains(Modifier::REVERSED));
        assert_ne!(t.fg.patch(t.sel), t.fg);
        // The focused pane, against the others.
        assert_ne!(t.border(true), t.border(false));
        // Chrome recedes, an error does not.
        assert!(t.faint.add_modifier.contains(Modifier::DIM));
        assert!(t.err.add_modifier.contains(Modifier::BOLD));
        // A fuzzy hit shows on a row that is already bold.
        let row = t.fg.add_modifier(Modifier::BOLD);
        assert_ne!(t.hit(row), row);
        // The mode indicator is still a block.
        assert!(
            t.mode_badge(Mode::Normal)
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn no_color_outranks_theme_toml() {
        let t = themed(ColorDepth::Mono, "accent = \"#ff0000\"\nsel = \"blue\"");
        assert_eq!(t, Theme::new(ColorDepth::Mono, &ThemeOverrides::default()));
    }

    #[test]
    fn sixteen_colours_never_emit_rgb() {
        let t = themed(
            ColorDepth::Ansi16,
            "fg = \"#d8dce4\"\nsel = \"#2c3340\"\nmode_normal = \"#ff007c\"",
        );
        let mut colors: Vec<Option<Color>> = slots(&t).iter().flat_map(|s| [s.fg, s.bg]).collect();
        colors.extend(MODES.iter().map(|m| t.mode_badge(*m).bg));
        colors.push(t.paint(Color::Rgb(0xce, 0x2c, 0x31)).fg);
        colors.push(t.profile_accent(Accent::Red).fg);
        for c in colors {
            assert!(!matches!(c, Some(Color::Rgb(..))), "{c:?}");
        }
    }

    #[test]
    fn sixteen_colours_leave_text_to_the_terminal() {
        // No named grey anywhere in the defaults: that is what makes a light background
        // readable without tmprl knowing it is light.
        let t = Theme::new(ColorDepth::Ansi16, &ThemeOverrides::default());
        assert_eq!(t.fg, Style::new());
        assert_eq!(t.faint.fg, None);
        assert_eq!((t.sel.fg, t.sel.bg), (None, None));
        assert!(t.sel.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(t.err.fg, Some(Color::Red));
    }

    #[test]
    fn a_hex_in_theme_toml_is_the_nearest_named_colour_on_sixteen() {
        let t = themed(ColorDepth::Ansi16, "accent = \"#e05252\"");
        assert_eq!(t.accent, Style::new().fg(Color::Red));
        let t = themed(ColorDepth::TrueColor, "accent = \"#e05252\"");
        assert_eq!(t.accent, Style::new().fg(Color::Rgb(0xe0, 0x52, 0x52)));
    }

    #[test]
    fn an_override_touches_only_its_slot() {
        let plain = Theme::default();
        let t = themed(ColorDepth::TrueColor, "warn = \"bright-yellow\"");
        assert_eq!(t.warn, Style::new().fg(Color::LightYellow));
        assert_eq!(
            Theme {
                warn: plain.warn,
                ..t
            },
            plain
        );
    }

    #[test]
    fn an_override_replaces_the_slots_modifier_too() {
        // `faint` is dimmed foreground on sixteen colours. Naming a colour for it means
        // that colour, not that colour dimmed.
        let t = themed(ColorDepth::Ansi16, "faint = \"bright-black\"");
        assert_eq!(t.faint, Style::new().fg(Color::DarkGray));
    }

    #[test]
    fn sel_and_the_modes_are_backgrounds() {
        let t = themed(ColorDepth::Ansi16, "sel = \"blue\"\nmode_insert = \"cyan\"");
        assert_eq!(t.sel, Style::new().bg(Color::Blue));
        assert_eq!(t.mode_badge(Mode::Insert).bg, Some(Color::Cyan));
        assert_eq!(t.mode_badge(Mode::Normal).bg, Some(Color::Magenta));
    }

    #[test]
    fn white_means_the_terminals_white() {
        let t = themed(
            ColorDepth::TrueColor,
            "fg = \"white\"\ndim = \"bright-white\"",
        );
        assert_eq!(t.fg.fg, Some(Color::Gray));
        assert_eq!(t.dim.fg, Some(Color::White));
    }

    #[test]
    fn a_timeline_grey_is_left_to_the_terminal_on_sixteen_colours() {
        let t = Theme::new(ColorDepth::Ansi16, &ThemeOverrides::default());
        assert_eq!(t.paint(Color::Rgb(0xed, 0xf2, 0xfe)), Style::new());
        assert_eq!(
            t.paint(Color::Rgb(0x30, 0xa4, 0x6c)),
            Style::new().fg(Color::Green)
        );
    }

    #[test]
    fn a_profile_accent_is_a_named_colour_at_every_depth_with_colour() {
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi16] {
            let t = Theme::new(depth, &ThemeOverrides::default());
            assert_eq!(t.profile_accent(Accent::Red).fg, Some(Color::Red));
        }
    }
}
