//! The line editor: one line of text and a cursor, behind every input.
//!
//! The `:` line, the query bar, the picker prompt, a form field and the typed confirmation
//! all hold one of these, so a key that edits text does the same thing in each of them.
//! Before this each had its own append and backspace, which is how five inputs come to
//! disagree about what `<C-w>` does.
//!
//! The cursor is a byte offset that always sits on a `char` boundary, so the text is valid
//! UTF-8 after any edit. It moves by `char`, a Unicode scalar value, not by grapheme
//! cluster: segmenting needs a table this crate does not carry. An accent typed as a
//! combining mark (`e` then U+0301), a flag, or an emoji with a skin tone is therefore
//! several stops of the cursor, and one backspace can leave a base letter without its mark.

use std::ops::Deref;

use crate::key::{Chord, Key};

/// What a key did to a line, for the input that owns it to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// Not an editing key. The line is untouched and the key is the caller's to use.
    Ignored,
    /// An editing key that left the text as it was: a cursor move, or a delete with nothing
    /// to delete.
    Unchanged,
    /// The text changed.
    Changed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineEdit {
    text: String,
    cursor: usize,
}

impl LineEdit {
    /// A line holding `text`, with the cursor after it: an input opened on existing text is
    /// about to be added to far more often than rewritten from the front.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor, as a byte offset into [`LineEdit::text`].
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The text either side of the cursor, which is how a renderer draws it.
    pub fn split(&self) -> (&str, &str) {
        self.text.split_at(self.cursor)
    }

    /// Replace the text and put the cursor at `cursor`, moved back to a `char` boundary if
    /// it is not on one.
    pub fn set(&mut self, text: impl Into<String>, cursor: usize) {
        self.text = text.into();
        self.cursor = self.text.floor_char_boundary(cursor.min(self.text.len()));
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Delete the character before the cursor. Reports whether there was one, so a caller
    /// can decide what a backspace with nothing behind it means.
    pub fn backspace(&mut self) -> bool {
        let start = self.before(self.cursor);
        self.remove(start, self.cursor)
    }

    /// Delete the character under the cursor.
    pub fn delete(&mut self) -> bool {
        let end = self.after(self.cursor);
        self.remove(self.cursor, end)
    }

    pub fn left(&mut self) {
        self.cursor = self.before(self.cursor);
    }

    pub fn right(&mut self) {
        self.cursor = self.after(self.cursor);
    }

    pub fn start(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Back to the start of the word the cursor is in or after.
    ///
    /// A word here is letters, digits and `_`, as it is for readline's `<A-b>`: in
    /// `WorkflowId = 'order-17'` the stops are the pieces a reader would name.
    pub fn word_left(&mut self) {
        let gap = self.back_while(self.cursor, |c| !is_word(c));
        self.cursor = self.back_while(gap, is_word);
    }

    /// Forward to the end of the word the cursor is in or before.
    pub fn word_right(&mut self) {
        let gap = self.forward_while(self.cursor, |c| !is_word(c));
        self.cursor = self.forward_while(gap, is_word);
    }

    /// Delete back to the start of the word, where a word is anything between spaces.
    ///
    /// Wider than the word the cursor moves by, and deliberately: this is readline's
    /// `<C-w>`, which takes `'order-17'` in one stroke rather than four.
    pub fn delete_word_back(&mut self) -> bool {
        let gap = self.back_while(self.cursor, char::is_whitespace);
        let start = self.back_while(gap, |c| !c.is_whitespace());
        self.remove(start, self.cursor)
    }

    pub fn delete_to_start(&mut self) -> bool {
        self.remove(0, self.cursor)
    }

    pub fn delete_to_end(&mut self) -> bool {
        self.remove(self.cursor, self.text.len())
    }

    /// Apply an editing key: the one table of bindings every input shares.
    ///
    /// An input matches the keys that are its own first (Enter, Esc, whatever moves its
    /// list) and hands the rest here. Shift is not consulted: it is already part of a
    /// character, and on a named key it changes nothing in a line with no selection.
    pub fn key(&mut self, chord: Chord) -> Edit {
        let changed = match (chord.key, chord.mods.ctrl, chord.mods.alt) {
            (Key::Char(c), false, false) => {
                self.insert(c);
                true
            }
            (Key::Backspace, false, false) => self.backspace(),
            (Key::Delete, false, false) => self.delete(),
            (Key::Char('w'), true, false) | (Key::Backspace, false, true) => {
                self.delete_word_back()
            }
            (Key::Char('u'), true, false) => self.delete_to_start(),
            (Key::Char('k'), true, false) => self.delete_to_end(),
            (Key::Left, false, false) | (Key::Char('b'), true, false) => {
                self.left();
                false
            }
            (Key::Right, false, false) | (Key::Char('f'), true, false) => {
                self.right();
                false
            }
            (Key::Home, false, false) | (Key::Char('a'), true, false) => {
                self.start();
                false
            }
            (Key::End, false, false) | (Key::Char('e'), true, false) => {
                self.end();
                false
            }
            // Both spellings of a word move: terminals disagree about which of Alt and
            // Ctrl an arrow key carries, and `<A-b>` is what one with neither sends.
            (Key::Left, false, true) | (Key::Left, true, false) | (Key::Char('b'), false, true) => {
                self.word_left();
                false
            }
            (Key::Right, false, true)
            | (Key::Right, true, false)
            | (Key::Char('f'), false, true) => {
                self.word_right();
                false
            }
            _ => return Edit::Ignored,
        };
        if changed {
            Edit::Changed
        } else {
            Edit::Unchanged
        }
    }

    fn remove(&mut self, start: usize, end: usize) -> bool {
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.cursor = start;
        true
    }

    /// The boundary one `char` before `at`, or `at` itself at the start.
    fn before(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map_or(at, |(i, _)| i)
    }

    /// The boundary one `char` after `at`, or `at` itself at the end.
    fn after(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map_or(at, |c| at + c.len_utf8())
    }

    fn back_while(&self, from: usize, keep: impl Fn(char) -> bool) -> usize {
        let mut at = from;
        for (i, c) in self.text[..from].char_indices().rev() {
            if !keep(c) {
                break;
            }
            at = i;
        }
        at
    }

    fn forward_while(&self, from: usize, keep: impl Fn(char) -> bool) -> usize {
        let mut at = from;
        for c in self.text[from..].chars() {
            if !keep(c) {
                break;
            }
            at += c.len_utf8();
        }
        at
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Reading a line is reading its text. Changing it goes through the methods, which are
/// what keep the cursor on a boundary.
impl Deref for LineEdit {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl From<&str> for LineEdit {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for LineEdit {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl PartialEq<str> for LineEdit {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for LineEdit {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mods;

    const ALT: Mods = Mods {
        ctrl: false,
        alt: true,
        shift: false,
    };

    /// A line with the cursor where the `|` is.
    fn line(marked: &str) -> LineEdit {
        let cursor = marked.find('|').expect("mark the cursor with |");
        let mut l = LineEdit::default();
        l.set(marked.replace('|', ""), cursor);
        l
    }

    /// The line drawn back with its cursor, so an assertion reads as a picture.
    fn show(l: &LineEdit) -> String {
        let (before, after) = l.split();
        format!("{before}|{after}")
    }

    #[test]
    fn a_new_line_puts_the_cursor_after_its_text() {
        assert_eq!(show(&LineEdit::new("jq .")), "jq .|");
        assert_eq!(show(&LineEdit::default()), "|");
    }

    #[test]
    fn typing_inserts_at_the_cursor_not_at_the_end() {
        let mut l = line("ab|cd");
        l.insert('X');
        assert_eq!(show(&l), "abX|cd");
        l.insert_str("yz");
        assert_eq!(show(&l), "abXyz|cd");
    }

    #[test]
    fn backspace_and_delete_take_one_character_either_side() {
        let mut l = line("ab|cd");
        assert!(l.backspace());
        assert_eq!(show(&l), "a|cd");
        assert!(l.delete());
        assert_eq!(show(&l), "a|d");
    }

    #[test]
    fn deleting_at_an_edge_reports_that_nothing_went() {
        // A prompt closes on a backspace it had nothing to take, so the answer matters.
        let mut l = line("|ab");
        assert!(!l.backspace());
        let mut l = line("ab|");
        assert!(!l.delete());
        assert_eq!(show(&l), "ab|");
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut l = line("a|b");
        l.left();
        l.left();
        assert_eq!(show(&l), "|ab");
        l.right();
        l.right();
        l.right();
        assert_eq!(show(&l), "ab|");
    }

    #[test]
    fn start_and_end_jump() {
        let mut l = line("ab|cd");
        l.start();
        assert_eq!(show(&l), "|abcd");
        l.end();
        assert_eq!(show(&l), "abcd|");
    }

    #[test]
    fn the_cursor_never_splits_a_character() {
        // `é` is two bytes and `日` is three. Every stop must be a boundary, or the next
        // insert panics.
        let mut l = LineEdit::new("é日x");
        for _ in 0..4 {
            l.left();
            assert!(l.text().is_char_boundary(l.cursor()), "{}", l.cursor());
        }
        l.right();
        l.insert('!');
        assert_eq!(show(&l), "é!|日x");
        assert!(l.delete());
        assert_eq!(show(&l), "é!|x");
        l.left();
        assert!(l.backspace());
        assert_eq!(show(&l), "|!x");
    }

    #[test]
    fn a_combining_mark_is_a_stop_of_its_own() {
        // The limit the header admits to: `e` and U+0301 draw as one `é` and are two stops.
        let mut l = LineEdit::new("e\u{301}");
        assert!(l.backspace());
        assert_eq!(l.text(), "e", "the mark went, the letter stayed");
    }

    #[test]
    fn setting_a_cursor_inside_a_character_moves_it_back_to_a_boundary() {
        let mut l = LineEdit::default();
        l.set("é", 1);
        assert_eq!(l.cursor(), 0);
        l.set("ab", 99);
        assert_eq!(l.cursor(), 2, "past the end is the end");
    }

    #[test]
    fn words_move_over_punctuation_to_the_next_run_of_letters() {
        let mut l = line("WorkflowId = 'order-17'|");
        let mut stops = Vec::new();
        for _ in 0..4 {
            l.word_left();
            stops.push(show(&l));
        }
        assert_eq!(
            stops,
            [
                "WorkflowId = 'order-|17'",
                "WorkflowId = '|order-17'",
                "|WorkflowId = 'order-17'",
                "|WorkflowId = 'order-17'",
            ]
        );
        l.word_right();
        assert_eq!(show(&l), "WorkflowId| = 'order-17'");
        l.word_right();
        assert_eq!(show(&l), "WorkflowId = 'order|-17'");
    }

    #[test]
    fn deleting_a_word_back_takes_everything_up_to_the_space() {
        let mut l = line("WorkflowId = 'order-17'  |");
        assert!(l.delete_word_back());
        assert_eq!(show(&l), "WorkflowId = |");
        assert!(l.delete_word_back());
        assert_eq!(show(&l), "WorkflowId |");
        l.start();
        assert!(!l.delete_word_back(), "nothing behind the cursor");
    }

    #[test]
    fn deleting_a_word_back_keeps_what_is_after_the_cursor() {
        let mut l = line("a = 1 AND b|c = 2");
        assert!(l.delete_word_back());
        assert_eq!(show(&l), "a = 1 AND |c = 2");
    }

    #[test]
    fn deleting_to_either_end_stops_at_the_cursor() {
        let mut l = line("abc|def");
        assert!(l.delete_to_start());
        assert_eq!(show(&l), "|def");
        assert!(!l.delete_to_start());

        let mut l = line("abc|def");
        assert!(l.delete_to_end());
        assert_eq!(show(&l), "abc|");
        assert!(!l.delete_to_end());
    }

    #[test]
    fn the_readline_keys_reach_the_operations() {
        let cases: [(Chord, &str); 17] = [
            (Chord::ch('X'), "ab cX|d ef"),
            (Chord::plain(Key::Backspace), "ab |d ef"),
            (Chord::plain(Key::Delete), "ab c| ef"),
            (Chord::plain(Key::Left), "ab |cd ef"),
            (Chord::ctrl('b'), "ab |cd ef"),
            (Chord::plain(Key::Right), "ab cd| ef"),
            (Chord::ctrl('f'), "ab cd| ef"),
            (Chord::plain(Key::Home), "|ab cd ef"),
            (Chord::ctrl('a'), "|ab cd ef"),
            (Chord::plain(Key::End), "ab cd ef|"),
            (Chord::ctrl('e'), "ab cd ef|"),
            (
                Chord {
                    key: Key::Left,
                    mods: ALT,
                },
                "ab |cd ef",
            ),
            (
                Chord {
                    key: Key::Char('f'),
                    mods: ALT,
                },
                "ab cd| ef",
            ),
            (Chord::ctrl('w'), "ab |d ef"),
            (
                Chord {
                    key: Key::Backspace,
                    mods: ALT,
                },
                "ab |d ef",
            ),
            (Chord::ctrl('u'), "|d ef"),
            (Chord::ctrl('k'), "ab c|"),
        ];
        for (chord, want) in cases {
            let mut l = line("ab c|d ef");
            assert_ne!(l.key(chord), Edit::Ignored, "{chord}");
            assert_eq!(show(&l), want, "{chord}");
        }
    }

    #[test]
    fn a_key_says_whether_the_text_changed() {
        let mut l = line("a|b");
        assert_eq!(l.key(Chord::plain(Key::Left)), Edit::Unchanged);
        assert_eq!(l.key(Chord::plain(Key::Backspace)), Edit::Unchanged);
        assert_eq!(l.key(Chord::ch('x')), Edit::Changed);
        assert_eq!(l.key(Chord::ctrl('k')), Edit::Changed);
        assert_eq!(l.key(Chord::ctrl('k')), Edit::Unchanged, "nothing left");
    }

    #[test]
    fn keys_that_are_not_editing_are_left_for_the_input() {
        // Enter, Esc, Tab and the list keys mean something different in every input, so
        // the line must not have an opinion about them.
        let mut l = line("a|b");
        for chord in [
            Chord::plain(Key::Enter),
            Chord::plain(Key::Esc),
            Chord::plain(Key::Tab),
            Chord::plain(Key::Up),
            Chord::plain(Key::Down),
            Chord::ctrl('n'),
            Chord::ctrl('p'),
            Chord {
                key: Key::Char('x'),
                mods: ALT,
            },
        ] {
            assert_eq!(l.key(chord), Edit::Ignored, "{chord}");
        }
        assert_eq!(show(&l), "a|b");
    }

    #[test]
    fn a_line_reads_as_its_text() {
        let l = LineEdit::new("  delete ");
        assert_eq!(l.trim(), "delete");
        assert_eq!(l, "  delete ");
        assert_eq!(LineEdit::from("x"), LineEdit::new(String::from("x")));
    }
}
