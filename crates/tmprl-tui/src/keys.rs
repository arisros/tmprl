//! Converting crossterm key events into `tmprl-core` chords.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tmprl_core::key::{Chord, Key, Mods};

use crate::app::Mouse;

pub fn to_mouse(ev: MouseEvent) -> Option<Mouse> {
    let (column, row) = (ev.column, ev.row);
    match ev.kind {
        MouseEventKind::ScrollDown => Some(Mouse::Wheel {
            down: true,
            column,
            row,
        }),
        MouseEventKind::ScrollUp => Some(Mouse::Wheel {
            down: false,
            column,
            row,
        }),
        MouseEventKind::Down(MouseButton::Left) => Some(Mouse::Click { column, row }),
        MouseEventKind::Drag(MouseButton::Left) => Some(Mouse::Drag { column, row }),
        MouseEventKind::Up(MouseButton::Left) => Some(Mouse::Release),
        _ => None,
    }
}

pub fn to_chord(ev: KeyEvent) -> Option<Chord> {
    // The kitty keyboard protocol sends release events too.
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

    #[test]
    fn a_held_left_button_drags_and_letting_go_releases() {
        let at = |kind| MouseEvent {
            kind,
            column: 4,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            to_mouse(at(MouseEventKind::Drag(MouseButton::Left))),
            Some(Mouse::Drag { column: 4, row: 2 })
        );
        assert_eq!(
            to_mouse(at(MouseEventKind::Up(MouseButton::Left))),
            Some(Mouse::Release)
        );
        assert_eq!(to_mouse(at(MouseEventKind::Drag(MouseButton::Right))), None);
    }
}
