//! Key resolution: chords in, command ids out.
//!
//! Three behaviours here are worth more than they look:
//!
//! * **Counts.** `7j` means seven, and it composes with any motion, because the count is
//!   accumulated by the resolver rather than by each command.
//! * **Prefixes.** An incomplete sequence resolves to [`Resolution::Pending`] carrying the
//!   keys that would complete it. That list is exactly what the which-key popup draws, so
//!   the popup can never disagree with the keymap.
//! * **Flushing.** An unmatched sequence returns the chords it swallowed. That is what lets
//!   `jk` leave Insert mode without eating a literal `j` typed before some other letter.

use crate::key::{Chord, ChordSeq, Key, KeyParseError};
use crate::mode::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub mode: Mode,
    pub seq: ChordSeq,
    pub command: &'static str,
}

/// A key that could come next, for the which-key popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEntry {
    pub next: Chord,
    /// `Some` when this key completes a binding, `None` when it only opens a deeper prefix.
    pub command: Option<&'static str>,
    /// How many bindings live under this key. `> 1` means it is a group.
    pub bindings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A digit was consumed into the count. Nothing to run yet.
    Count(u32),
    /// A prefix matched. `candidates` is what could come next.
    Pending { candidates: Vec<PendingEntry> },
    /// A binding matched.
    Run {
        id: &'static str,
        count: Option<u32>,
    },
    /// Nothing matched. `flushed` is every chord that was held, including this one, so the
    /// caller can treat them as literal input.
    Unbound { flushed: Vec<Chord> },
}

/// Keys held while waiting for a sequence to complete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pending {
    pub count: Option<u32>,
    pub chords: Vec<Chord>,
}

impl Pending {
    pub fn clear(&mut self) {
        self.count = None;
        self.chords.clear();
    }
    pub fn is_idle(&self) -> bool {
        self.count.is_none() && self.chords.is_empty()
    }
    /// What the statusline shows in the bottom right, like vim's pending-command indicator.
    pub fn display(&self) -> String {
        let mut s = String::new();
        if let Some(c) = self.count {
            s.push_str(&c.to_string());
        }
        for ch in &self.chords {
            s.push_str(&ch.to_string());
        }
        s
    }
}

pub struct Keymap {
    bindings: Vec<Binding>,
    leader: Chord,
}

/// Counts are capped so that a leaned-on digit key cannot ask for a motion of four billion.
const MAX_COUNT: u32 = 100_000;

impl Keymap {
    pub fn new(leader: Chord) -> Self {
        Self {
            bindings: Vec::new(),
            leader,
        }
    }

    pub fn bind(
        &mut self,
        mode: Mode,
        seq: &str,
        command: &'static str,
    ) -> Result<(), KeyParseError> {
        let seq = ChordSeq::parse(seq, self.leader)?;
        // Last binding wins, so a user keymap can override a default.
        self.bindings.retain(|b| !(b.mode == mode && b.seq == seq));
        self.bindings.push(Binding { mode, seq, command });
        Ok(())
    }

    pub fn leader(&self) -> Chord {
        self.leader
    }
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Every chord sequence bound to a command, for the help overlay.
    pub fn keys_for(&self, command: &str) -> Vec<&Binding> {
        self.bindings
            .iter()
            .filter(|b| b.command == command)
            .collect()
    }

    pub fn resolve(&self, mode: Mode, pending: &mut Pending, chord: Chord) -> Resolution {
        // A digit starts or extends a count, but only when no sequence is in flight —
        // otherwise `<leader>1` could never be bound.
        if mode.takes_counts()
            && pending.chords.is_empty()
            && let Key::Char(c) = chord.key
            && chord.mods.is_none()
            && let Some(d) = c.to_digit(10)
            && !(d == 0 && pending.count.is_none())
        {
            let next = pending
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(d);
            pending.count = Some(next.min(MAX_COUNT));
            return Resolution::Count(pending.count.unwrap());
        }

        pending.chords.push(chord);

        if let Some(b) = self
            .bindings
            .iter()
            .find(|b| b.mode == mode && b.seq.0 == pending.chords)
        {
            let count = pending.count;
            pending.clear();
            return Resolution::Run {
                id: b.command,
                count,
            };
        }

        let depth = pending.chords.len();
        let mut candidates: Vec<PendingEntry> = Vec::new();
        for b in &self.bindings {
            if b.mode != mode || b.seq.len() <= depth || !b.seq.starts_with(&pending.chords) {
                continue;
            }
            let next = b.seq.0[depth];
            let completes = b.seq.len() == depth + 1;
            match candidates.iter_mut().find(|e| e.next == next) {
                Some(e) => {
                    e.bindings += 1;
                    if completes {
                        e.command = Some(b.command);
                    }
                }
                None => candidates.push(PendingEntry {
                    next,
                    command: completes.then_some(b.command),
                    bindings: 1,
                }),
            }
        }

        if !candidates.is_empty() {
            candidates.sort_by_key(|e| e.next);
            return Resolution::Pending { candidates };
        }

        let flushed = std::mem::take(&mut pending.chords);
        pending.count = None;
        Resolution::Unbound { flushed }
    }
}

/// The default keymap.
///
/// Only bindings whose commands actually do something are registered. Binding a key to a
/// feature that is not built yet would make the which-key popup advertise things that do
/// nothing, which is worse than an empty keymap.
///
/// `C-h/j/k/l` are deliberately absent — see `docs/INTERFACE.md`.
pub fn default_keymap() -> Keymap {
    let mut m = Keymap::new(Chord::ch(' '));
    let mut bind = |mode, seq, cmd| {
        m.bind(mode, seq, cmd)
            .unwrap_or_else(|e| panic!("bad default binding `{seq}`: {e}"));
    };

    for mode in [Mode::Normal, Mode::Visual, Mode::VisualLine] {
        bind(mode, "j", "motion.down");
        bind(mode, "k", "motion.up");
        bind(mode, "<Down>", "motion.down");
        bind(mode, "<Up>", "motion.up");
        bind(mode, "gg", "motion.top");
        bind(mode, "G", "motion.bottom");
        bind(mode, "<C-d>", "motion.half-down");
        bind(mode, "<C-u>", "motion.half-up");
        bind(mode, "y", "yank.field");
        bind(mode, "Y", "yank.record");
        bind(mode, "<Esc>", "app.cancel");
        bind(mode, ":", "app.command-line");
        bind(mode, "?", "app.help");
        bind(mode, "R", "app.refresh");
        bind(mode, "<leader>q", "app.quit");
        bind(mode, "<C-c>", "app.quit");
    }

    // `<CR>` opens in the visual modes too, where it means "open the selection": that is
    // how several namespaces become one merged workflow list. `-` stays Normal-only —
    // walking up a level while selecting rows has no sensible meaning.
    for mode in [Mode::Normal, Mode::Visual, Mode::VisualLine] {
        bind(mode, "<CR>", "nav.open");
    }
    bind(Mode::Normal, "-", "nav.up");

    // Folds use vim's `z` family, so the which-key popup on `z` reads like vim's does.
    // `zp` is not a vim binding, but it sits in the same namespace as the folds it
    // resembles: it folds away the workflow-task plumbing.
    for mode in [Mode::Normal, Mode::Visual, Mode::VisualLine] {
        bind(mode, "za", "history.fold");
        bind(mode, "zR", "history.expand-all");
        bind(mode, "zM", "history.collapse-all");
        bind(mode, "zp", "history.plumbing");
        // vim-unimpaired's bracket motions: `]f` / `[f` for the next and previous failure.
        bind(mode, "]f", "history.next-failure");
        bind(mode, "[f", "history.prev-failure");
        bind(mode, "F", "history.follow");
        // `K` is vim's "look up what is under the cursor", which is exactly what the detail
        // pane does — it shows the payloads of the focused event or group.
        bind(mode, "K", "history.detail");
        // vim scrolls a window by a line with <C-e>/<C-y>; here they scroll the payload
        // pane, which is the only thing on screen tall enough to need it.
        bind(mode, "<C-e>", "history.detail-down");
        bind(mode, "<C-y>", "history.detail-up");
    }

    bind(Mode::Normal, "i", "mode.insert");
    bind(Mode::Normal, "v", "mode.visual");
    bind(Mode::Normal, "V", "mode.visual-line");

    // `jk` is the escape hatch; `<Esc>` works too.
    bind(Mode::Insert, "jk", "mode.normal");
    bind(Mode::Insert, "<Esc>", "mode.normal");

    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Keymap {
        default_keymap()
    }

    fn feed(m: &Keymap, mode: Mode, p: &mut Pending, keys: &[Chord]) -> Resolution {
        let mut last = Resolution::Unbound { flushed: vec![] };
        for &c in keys {
            last = m.resolve(mode, p, c);
        }
        last
    }

    #[test]
    fn resolves_a_single_key() {
        let (m, mut p) = (map(), Pending::default());
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('j')),
            Resolution::Run {
                id: "motion.down",
                count: None
            }
        );
        assert!(p.is_idle(), "pending state must reset after a match");
    }

    #[test]
    fn accumulates_multi_digit_counts() {
        let (m, mut p) = (map(), Pending::default());
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('1')),
            Resolution::Count(1)
        );
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('2')),
            Resolution::Count(12)
        );
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('j')),
            Resolution::Run {
                id: "motion.down",
                count: Some(12)
            }
        );
    }

    #[test]
    fn leading_zero_is_not_a_count() {
        // In vim `0` is a motion, not a count — it may only extend one already started.
        let (m, mut p) = (map(), Pending::default());
        assert!(matches!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('0')),
            Resolution::Unbound { .. }
        ));
        p.clear();
        m.resolve(Mode::Normal, &mut p, Chord::ch('1'));
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('0')),
            Resolution::Count(10)
        );
    }

    #[test]
    fn counts_are_capped() {
        let (m, mut p) = (map(), Pending::default());
        for _ in 0..12 {
            m.resolve(Mode::Normal, &mut p, Chord::ch('9'));
        }
        assert_eq!(p.count, Some(MAX_COUNT));
    }

    #[test]
    fn multi_key_sequences_report_pending_then_run() {
        let (m, mut p) = (map(), Pending::default());
        let r = m.resolve(Mode::Normal, &mut p, Chord::ch('g'));
        match r {
            Resolution::Pending { candidates } => {
                assert_eq!(candidates.len(), 1);
                assert_eq!(candidates[0].next, Chord::ch('g'));
                assert_eq!(candidates[0].command, Some("motion.top"));
            }
            other => panic!("expected Pending, got {other:?}"),
        }
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('g')),
            Resolution::Run {
                id: "motion.top",
                count: None
            }
        );
    }

    #[test]
    fn leader_lists_its_candidates() {
        let (m, mut p) = (map(), Pending::default());
        match m.resolve(Mode::Normal, &mut p, Chord::ch(' ')) {
            Resolution::Pending { candidates } => {
                assert!(candidates.iter().any(|c| c.command == Some("app.quit")));
            }
            other => panic!("expected Pending, got {other:?}"),
        }
    }

    #[test]
    fn counts_survive_a_multi_key_sequence() {
        let (m, mut p) = (map(), Pending::default());
        let r = feed(
            &m,
            Mode::Normal,
            &mut p,
            &[Chord::ch('5'), Chord::ch('g'), Chord::ch('g')],
        );
        assert_eq!(
            r,
            Resolution::Run {
                id: "motion.top",
                count: Some(5)
            }
        );
    }

    #[test]
    fn jk_leaves_insert_mode() {
        let (m, mut p) = (map(), Pending::default());
        assert!(matches!(
            m.resolve(Mode::Insert, &mut p, Chord::ch('j')),
            Resolution::Pending { .. }
        ));
        assert_eq!(
            m.resolve(Mode::Insert, &mut p, Chord::ch('k')),
            Resolution::Run {
                id: "mode.normal",
                count: None
            }
        );
    }

    #[test]
    fn a_held_j_is_flushed_when_the_sequence_fails() {
        // Typing "ja" in Insert must insert both characters, not swallow the `j`.
        let (m, mut p) = (map(), Pending::default());
        m.resolve(Mode::Insert, &mut p, Chord::ch('j'));
        match m.resolve(Mode::Insert, &mut p, Chord::ch('a')) {
            Resolution::Unbound { flushed } => {
                assert_eq!(flushed, vec![Chord::ch('j'), Chord::ch('a')]);
            }
            other => panic!("expected Unbound with both chords, got {other:?}"),
        }
        assert!(p.is_idle());
    }

    #[test]
    fn insert_mode_ignores_counts() {
        let (m, mut p) = (map(), Pending::default());
        match m.resolve(Mode::Insert, &mut p, Chord::ch('7')) {
            Resolution::Unbound { flushed } => assert_eq!(flushed, vec![Chord::ch('7')]),
            other => panic!("digits must be literal in Insert, got {other:?}"),
        }
    }

    #[test]
    fn ctrl_hjkl_is_never_bound() {
        // tmux's vim-tmux-navigator consumes these before any application sees them.
        let m = map();
        for c in ['h', 'j', 'k', 'l'] {
            let chord = Chord::ctrl(c);
            assert!(
                !m.bindings().iter().any(|b| b.seq.0 == vec![chord]),
                "<C-{c}> must not be bound; tmux eats it"
            );
        }
    }

    #[test]
    fn later_bindings_override_earlier_ones() {
        let mut m = Keymap::new(Chord::ch(' '));
        m.bind(Mode::Normal, "j", "motion.down").unwrap();
        m.bind(Mode::Normal, "j", "motion.up").unwrap();
        assert_eq!(m.bindings().len(), 1);
        let mut p = Pending::default();
        assert_eq!(
            m.resolve(Mode::Normal, &mut p, Chord::ch('j')),
            Resolution::Run {
                id: "motion.up",
                count: None
            }
        );
    }

    #[test]
    fn pending_display_matches_what_was_typed() {
        let (m, mut p) = (map(), Pending::default());
        feed(&m, Mode::Normal, &mut p, &[Chord::ch('2'), Chord::ch('g')]);
        assert_eq!(p.display(), "2g");
    }

    #[test]
    fn every_bound_command_exists_in_the_registry() {
        // A binding to a non-existent id would be a key that silently does nothing.
        let reg = crate::command::Registry::builtin();
        for b in map().bindings() {
            assert!(
                reg.get(b.command).is_some(),
                "binding {} points at unknown command `{}`",
                b.seq,
                b.command
            );
        }
    }
}
