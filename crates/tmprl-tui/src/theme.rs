//! Colours.
//!
//! Derived from the `twilight256` palette so the interface sits comfortably beside an editor
//! themed the same way. `theme.toml` loading arrives with the rest of the configuration.

use ratatui::style::Color;
use tmprl_core::Mode;

pub struct Theme {
    pub fg: Color,
    pub dim: Color,
    pub faint: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
    pub sel: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            fg: Color::Rgb(0xd8, 0xdc, 0xe4),
            dim: Color::Rgb(0x8a, 0x93, 0xa3),
            faint: Color::Rgb(0x5a, 0x62, 0x70),
            accent: Color::Rgb(0x6b, 0x99, 0xe8), // Identifier
            ok: Color::Rgb(0xa7, 0xb8, 0x79),     // String
            warn: Color::Rgb(0xF0, 0xC6, 0x74),   // Function
            err: Color::Rgb(0xe0, 0x52, 0x52),
            sel: Color::Rgb(0x2c, 0x33, 0x40),
        }
    }
}

impl Theme {
    /// Mode indicator colour, matching the lualine convention of a distinct hue per mode.
    pub fn mode_color(&self, mode: Mode) -> Color {
        match mode {
            Mode::Normal => Color::Rgb(0xff, 0x00, 0x7c),
            Mode::Insert => Color::Rgb(0x00, 0xd7, 0x5f),
            Mode::Visual | Mode::VisualLine => Color::Rgb(0xff, 0xaf, 0x00),
            Mode::Command => Color::Rgb(0x8b, 0x5f, 0xff),
        }
    }
}
