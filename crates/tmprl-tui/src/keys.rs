//! Converting crossterm key events into `tmprl-core` chords.
//!
//! This is the only place in the program that knows crossterm's key types. Everything above
//! works in `tmprl_core::Chord`, which is what lets the keymap be tested without a terminal.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tmprl_core::key::{Chord, Key, Mods};

pub fn to_chord(ev: KeyEvent) -> Option<Chord> {
    // Terminals with the kitty keyboard protocol send press *and* release; acting on both
    // would run every command twice.
    if ev.kind == KeyEventKind::Release {
        return None;
    }

    let key = match ev.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::F(n) => Key::F(n),
        _ => return None,
    };

    let m = ev.modifiers;
    let mut mods = Mods {
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
    };

    // For a character, shift is already expressed by the character itself — `G` arrives as
    // Char('G'), often with SHIFT also set. Keeping the flag would stop it matching a
    // binding parsed from the string "G", which carries no modifiers.
    if matches!(key, Key::Char(_)) {
        mods.shift = false;
    }

    Some(Chord { key, mods })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_characters_carry_no_modifiers() {
        let c = to_chord(ev(KeyCode::Char('j'), KeyModifiers::NONE)).unwrap();
        assert_eq!(c, Chord::ch('j'));
    }

    #[test]
    fn shifted_characters_match_bindings_written_as_uppercase() {
        // This is the bug this conversion exists to prevent: `G` with SHIFT set must still
        // equal the chord parsed from the string "G".
        let c = to_chord(ev(KeyCode::Char('G'), KeyModifiers::SHIFT)).unwrap();
        assert_eq!(c, Chord::ch('G'));
    }

    #[test]
    fn control_chords_are_preserved() {
        let c = to_chord(ev(KeyCode::Char('d'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(c, Chord::ctrl('d'));
    }

    #[test]
    fn named_keys_map_across() {
        assert_eq!(
            to_chord(ev(KeyCode::Esc, KeyModifiers::NONE)).unwrap(),
            Chord::plain(Key::Esc)
        );
        assert_eq!(
            to_chord(ev(KeyCode::F(5), KeyModifiers::NONE)).unwrap(),
            Chord::plain(Key::F(5))
        );
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut e = ev(KeyCode::Char('j'), KeyModifiers::NONE);
        e.kind = KeyEventKind::Release;
        assert!(
            to_chord(e).is_none(),
            "release events would double every command"
        );
    }
}
