//! One line of editable text: the prompts, the `:` line and the query bar.

use std::collections::VecDeque;

use crate::key::{Chord, Key};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineEdit {
    text: String,
    /// A byte offset, always on a char boundary.
    at: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Changed,
    Moved,
    Emptied,
    Unhandled,
}

impl From<&str> for LineEdit {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_string(),
            at: text.len(),
        }
    }
}

impl From<String> for LineEdit {
    fn from(text: String) -> Self {
        let at = text.len();
        Self { text, at }
    }
}

impl PartialEq<&str> for LineEdit {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<str> for LineEdit {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl std::fmt::Display for LineEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl LineEdit {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn before(&self) -> &str {
        &self.text[..self.at]
    }

    pub fn after(&self) -> &str {
        &self.text[self.at..]
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set(&mut self, text: impl Into<String>) {
        *self = Self::from(text.into());
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.at, c);
        self.at += c.len_utf8();
    }

    pub fn paste(&mut self, text: &str) {
        let clean: String = text
            .trim_end_matches(['\n', '\r'])
            .chars()
            .map(|c| {
                if c == '\n' || c == '\r' || c == '\t' {
                    ' '
                } else {
                    c
                }
            })
            .filter(|c| !c.is_control())
            .collect();
        self.text.insert_str(self.at, &clean);
        self.at += clean.len();
    }

    pub fn key(&mut self, chord: Chord) -> Edit {
        let (ctrl, alt) = (chord.mods.ctrl, chord.mods.alt);
        match chord.key {
            Key::Char(c) if chord.mods.is_none() => {
                self.insert(c);
                Edit::Changed
            }
            Key::Backspace if alt => self.delete_word_back(),
            Key::Backspace => self.backspace(),
            Key::Char('h') if ctrl => self.backspace(),
            Key::Delete => self.delete(),
            Key::Char('d') if ctrl => self.delete(),
            Key::Char('w') if ctrl => self.delete_word_back(),
            Key::Char('u') if ctrl => self.kill(0..self.at),
            Key::Char('k') if ctrl => self.kill(self.at..self.text.len()),
            Key::Left if ctrl || alt => self.move_to(self.word_left()),
            Key::Right if ctrl || alt => self.move_to(self.word_right()),
            Key::Char('b') if alt => self.move_to(self.word_left()),
            Key::Char('f') if alt => self.move_to(self.word_right()),
            Key::Left => self.move_to(self.char_left()),
            Key::Char('b') if ctrl => self.move_to(self.char_left()),
            Key::Right => self.move_to(self.char_right()),
            Key::Char('f') if ctrl => self.move_to(self.char_right()),
            Key::Home => self.move_to(0),
            Key::Char('a') if ctrl => self.move_to(0),
            Key::End => self.move_to(self.text.len()),
            Key::Char('e') if ctrl => self.move_to(self.text.len()),
            _ => Edit::Unhandled,
        }
    }

    fn backspace(&mut self) -> Edit {
        if self.text.is_empty() {
            return Edit::Emptied;
        }
        let from = self.char_left();
        self.kill(from..self.at)
    }

    fn delete(&mut self) -> Edit {
        let to = self.char_right();
        self.kill(self.at..to)
    }

    fn delete_word_back(&mut self) -> Edit {
        if self.text.is_empty() {
            return Edit::Emptied;
        }
        let from = self.word_left();
        self.kill(from..self.at)
    }

    fn kill(&mut self, range: std::ops::Range<usize>) -> Edit {
        if range.is_empty() {
            return Edit::Moved;
        }
        self.at = range.start;
        self.text.replace_range(range, "");
        Edit::Changed
    }

    fn move_to(&mut self, at: usize) -> Edit {
        self.at = at;
        Edit::Moved
    }

    fn char_left(&self) -> usize {
        self.before()
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn char_right(&self) -> usize {
        self.after()
            .chars()
            .next()
            .map_or(self.at, |c| self.at + c.len_utf8())
    }

    fn word_left(&self) -> usize {
        let before = self.before();
        let trimmed = before.trim_end_matches(|c: char| !is_word(c));
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| !is_word(*c))
            .map_or(0, |(i, c)| i + c.len_utf8())
    }

    fn word_right(&self) -> usize {
        let after = self.after();
        let skipped = after.len() - after.trim_start_matches(|c: char| !is_word(c)).len();
        let rest = &after[skipped..];
        let word = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
        self.at + skipped + word
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

#[derive(Debug, Clone, Default)]
pub struct LineHistory {
    entries: VecDeque<String>,
    at: Option<usize>,
    draft: String,
}

const HISTORY_CAP: usize = 200;

impl LineHistory {
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        self.at = None;
        if line.is_empty() {
            return;
        }
        self.entries.retain(|e| e != line);
        self.entries.push_back(line.to_string());
        if self.entries.len() > HISTORY_CAP {
            self.entries.pop_front();
        }
    }

    pub fn older(&mut self, current: &str) -> Option<&str> {
        let next = match self.at {
            None if self.entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.at = Some(next);
        self.entries.get(next).map(String::as_str)
    }

    pub fn newer(&mut self) -> Option<&str> {
        let i = self.at?;
        if i + 1 < self.entries.len() {
            self.at = Some(i + 1);
            return self.entries.get(i + 1).map(String::as_str);
        }
        self.at = None;
        Some(&self.draft)
    }

    pub fn reset(&mut self) {
        self.at = None;
        self.draft.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mods;

    fn alt(c: char) -> Chord {
        Chord {
            key: Key::Char(c),
            mods: Mods {
                alt: true,
                ..Mods::NONE
            },
        }
    }

    fn typed(text: &str) -> LineEdit {
        let mut line = LineEdit::default();
        for c in text.chars() {
            line.key(Chord::ch(c));
        }
        line
    }

    #[test]
    fn typing_goes_in_at_the_cursor() {
        let mut line = typed("WorkflowId = 'a'");
        line.key(Chord::plain(Key::Home));
        for c in "x ".chars() {
            line.key(Chord::ch(c));
        }
        assert_eq!(line.as_str(), "x WorkflowId = 'a'");
        assert_eq!(line.before(), "x ");
    }

    #[test]
    fn left_and_right_move_by_a_character_even_a_wide_one() {
        let mut line = LineEdit::from("añb");
        line.key(Chord::plain(Key::Left));
        line.key(Chord::plain(Key::Left));
        assert_eq!(line.before(), "a");
        line.key(Chord::plain(Key::Right));
        assert_eq!(line.before(), "añ");
    }

    #[test]
    fn the_shell_keys_move_and_cut() {
        let mut line = LineEdit::from("ExecutionStatus = 'Running' AND WorkflowType = 'Order'");
        line.key(alt('b'));
        assert_eq!(line.after(), "Order'");
        line.key(Chord::ctrl('w'));
        assert_eq!(line.as_str(), "ExecutionStatus = 'Running' AND Order'");
        line.key(Chord::ctrl('a'));
        line.key(alt('f'));
        assert_eq!(line.before(), "ExecutionStatus");
        line.key(Chord::ctrl('k'));
        assert_eq!(line.as_str(), "ExecutionStatus");
        line.key(Chord::ctrl('u'));
        assert!(line.is_empty());
    }

    #[test]
    fn ctrl_w_deletes_the_word_before_the_cursor() {
        let mut line = LineEdit::from("jq .items | length");
        line.key(Chord::ctrl('w'));
        assert_eq!(line.as_str(), "jq .items | ");
        line.key(Chord::ctrl('w'));
        assert_eq!(line.as_str(), "jq .");
    }

    #[test]
    fn backspace_on_an_empty_line_says_so() {
        let mut line = typed("a");
        assert_eq!(line.key(Chord::plain(Key::Backspace)), Edit::Changed);
        assert_eq!(line.key(Chord::plain(Key::Backspace)), Edit::Emptied);
    }

    #[test]
    fn delete_takes_the_character_under_the_cursor() {
        let mut line = LineEdit::from("abc");
        line.key(Chord::plain(Key::Home));
        line.key(Chord::plain(Key::Delete));
        assert_eq!(line.as_str(), "bc");
    }

    #[test]
    fn a_paste_lands_on_one_line_at_the_cursor() {
        let mut line = LineEdit::from("WorkflowId = ''");
        line.key(Chord::plain(Key::Left));
        line.paste("order-42\n");
        assert_eq!(line.as_str(), "WorkflowId = 'order-42'");
        line.paste("a\nb\tc\u{7}");
        assert_eq!(line.before(), "WorkflowId = 'order-42a b c");
    }

    #[test]
    fn keys_a_line_has_no_use_for_are_left_to_the_caller() {
        let mut line = LineEdit::default();
        assert_eq!(line.key(Chord::plain(Key::Up)), Edit::Unhandled);
        assert_eq!(line.key(Chord::ctrl('n')), Edit::Unhandled);
    }

    #[test]
    fn history_walks_back_and_returns_to_the_draft() {
        let mut h = LineHistory::default();
        h.push("one");
        h.push("two");
        assert_eq!(h.older("typing"), Some("two"));
        assert_eq!(h.older("two"), Some("one"));
        assert_eq!(h.older("one"), None, "the oldest stays");
        assert_eq!(h.newer(), Some("two"));
        assert_eq!(h.newer(), Some("typing"));
        assert_eq!(h.newer(), None);
    }

    #[test]
    fn entering_a_line_again_moves_it_to_the_newest() {
        let mut h = LineHistory::default();
        h.push("one");
        h.push("two");
        h.push("one");
        h.push("  ");
        assert_eq!(h.older(""), Some("one"));
        assert_eq!(h.older(""), Some("two"));
        assert_eq!(h.older(""), None);
    }
}
