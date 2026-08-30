//! Key representation and vim-style key-notation parsing.
//!
//! This is deliberately *not* crossterm's `KeyEvent`. Keeping our own type is what lets the
//! keymap be tested without a terminal, and what stops a crossterm major bump from reaching
//! into the keymap. `tmprl-tui` converts at the edge.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
    };

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

impl Chord {
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            mods: Mods::NONE,
        }
    }
    pub const fn ch(c: char) -> Self {
        Self::plain(Key::Char(c))
    }
    pub const fn ctrl(c: char) -> Self {
        Self {
            key: Key::Char(c),
            mods: Mods::CTRL,
        }
    }

    /// The character this chord would insert in Insert mode, if any.
    pub fn as_insertable(self) -> Option<char> {
        match self.key {
            Key::Char(c) if self.mods.is_none() => Some(c),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyParseError {
    #[error("unterminated `<` in key sequence `{0}`")]
    Unterminated(String),
    #[error("unknown key name `<{0}>`")]
    UnknownKey(String),
    #[error("empty key sequence")]
    Empty,
}

fn key_name(k: Key) -> String {
    match k {
        Key::Enter => "CR".into(),
        Key::Esc => "Esc".into(),
        Key::Tab => "Tab".into(),
        Key::BackTab => "S-Tab".into(),
        Key::Backspace => "BS".into(),
        Key::Delete => "Del".into(),
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::F(n) => format!("F{n}"),
        Key::Char(' ') => "Space".into(),
        Key::Char(c) => c.to_string(),
    }
}

fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "cr" | "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Esc,
        "tab" => Key::Tab,
        "s-tab" | "btab" => Key::BackTab,
        "bs" | "backspace" => Key::Backspace,
        "del" | "delete" => Key::Delete,
        "space" => Key::Char(' '),
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        _ => {
            let n = name.strip_prefix('f')?.parse::<u8>().ok()?;
            if (1..=12).contains(&n) {
                Key::F(n)
            } else {
                return None;
            }
        }
    })
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bare = matches!(self.key, Key::Char(c) if c != ' ');
        if self.mods.is_none() && bare {
            return write!(f, "{}", key_name(self.key));
        }
        let mut prefix = String::new();
        if self.mods.ctrl {
            prefix.push_str("C-");
        }
        if self.mods.alt {
            prefix.push_str("A-");
        }
        // BackTab already renders as S-Tab; don't double the prefix.
        if self.mods.shift && self.key != Key::BackTab {
            prefix.push_str("S-");
        }
        write!(f, "<{prefix}{}>", key_name(self.key))
    }
}

/// A sequence of chords, e.g. `<leader>ff` is three chords.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ChordSeq(pub Vec<Chord>);

impl ChordSeq {
    /// Parse vim key notation. `<leader>` expands to the supplied chord, so the leader is a
    /// configuration value rather than something baked into every binding.
    pub fn parse(s: &str, leader: Chord) -> Result<Self, KeyParseError> {
        let mut out = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] != '<' {
                out.push(Chord::ch(chars[i]));
                i += 1;
                continue;
            }
            let close = chars[i..]
                .iter()
                .position(|&c| c == '>')
                .ok_or_else(|| KeyParseError::Unterminated(s.to_string()))?
                + i;
            let token: String = chars[i + 1..close].iter().collect();
            out.push(parse_token(&token, leader)?);
            i = close + 1;
        }

        if out.is_empty() {
            return Err(KeyParseError::Empty);
        }
        Ok(Self(out))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn starts_with(&self, prefix: &[Chord]) -> bool {
        self.0.starts_with(prefix)
    }
}

fn parse_token(token: &str, leader: Chord) -> Result<Chord, KeyParseError> {
    let lower = token.to_ascii_lowercase();
    if lower == "leader" {
        return Ok(leader);
    }
    if let Some(k) = named_key(&lower) {
        return Ok(Chord::plain(k));
    }

    // Strip modifier prefixes in any order: <C-A-x>, <A-C-x>.
    let mut mods = Mods::NONE;
    let mut rest = token;
    loop {
        let head = rest.get(..2).map(str::to_ascii_lowercase);
        match head.as_deref() {
            Some("c-") => mods.ctrl = true,
            Some("a-") | Some("m-") => mods.alt = true,
            Some("s-") => mods.shift = true,
            _ => break,
        }
        rest = &rest[2..];
    }
    if mods.is_none() {
        return Err(KeyParseError::UnknownKey(token.to_string()));
    }

    let lower_rest = rest.to_ascii_lowercase();
    // `<S-Tab>` is BackTab, and its shift is already implied by the name.
    if mods.shift && lower_rest == "tab" {
        return Ok(Chord::plain(Key::BackTab));
    }
    if let Some(k) = named_key(&lower_rest) {
        return Ok(Chord { key: k, mods });
    }
    let mut it = rest.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Ok(Chord {
            key: Key::Char(c),
            mods,
        }),
        _ => Err(KeyParseError::UnknownKey(token.to_string())),
    }
}

impl fmt::Display for ChordSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.0 {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEADER: Chord = Chord::ch(' ');

    fn seq(s: &str) -> ChordSeq {
        ChordSeq::parse(s, LEADER).expect(s)
    }

    #[test]
    fn parses_plain_characters() {
        assert_eq!(seq("j").0, vec![Chord::ch('j')]);
        assert_eq!(seq("gg").0, vec![Chord::ch('g'), Chord::ch('g')]);
    }

    #[test]
    fn parses_control_chords() {
        assert_eq!(seq("<C-w>").0, vec![Chord::ctrl('w')]);
        assert_eq!(seq("<c-d>").0, vec![Chord::ctrl('d')]);
    }

    #[test]
    fn leader_expands_to_the_configured_chord() {
        assert_eq!(
            seq("<leader>ff").0,
            vec![Chord::ch(' '), Chord::ch('f'), Chord::ch('f')]
        );
        // A different leader changes every binding without editing them.
        let comma = ChordSeq::parse("<leader>x", Chord::ch(',')).unwrap();
        assert_eq!(comma.0[0], Chord::ch(','));
    }

    #[test]
    fn parses_named_keys() {
        assert_eq!(seq("<Esc>").0, vec![Chord::plain(Key::Esc)]);
        assert_eq!(seq("<CR>").0, vec![Chord::plain(Key::Enter)]);
        assert_eq!(seq("<Space>").0, vec![Chord::ch(' ')]);
        assert_eq!(seq("<F5>").0, vec![Chord::plain(Key::F(5))]);
        assert_eq!(seq("<S-Tab>").0, vec![Chord::plain(Key::BackTab)]);
    }

    #[test]
    fn parses_mixed_sequences() {
        assert_eq!(
            seq("<leader>s<C-w>").0,
            vec![Chord::ch(' '), Chord::ch('s'), Chord::ctrl('w')]
        );
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(
            ChordSeq::parse("<C-w", LEADER),
            Err(KeyParseError::Unterminated("<C-w".into()))
        );
        assert_eq!(
            ChordSeq::parse("<nope>", LEADER),
            Err(KeyParseError::UnknownKey("nope".into()))
        );
        assert_eq!(ChordSeq::parse("", LEADER), Err(KeyParseError::Empty));
    }

    #[test]
    fn round_trips_through_display() {
        for s in ["j", "gg", "<C-w>", "<Esc>", "<Space>", "<F5>", "<S-Tab>"] {
            assert_eq!(seq(s).to_string(), s, "round-trip failed for {s}");
        }
    }

    #[test]
    fn only_unmodified_characters_are_insertable() {
        assert_eq!(Chord::ch('j').as_insertable(), Some('j'));
        assert_eq!(Chord::ctrl('j').as_insertable(), None);
        assert_eq!(Chord::plain(Key::Esc).as_insertable(), None);
    }
}
