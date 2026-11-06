//! Key resolution: chords in, command ids out.

use crate::key::{Chord, ChordSeq, Key, KeyParseError};
use crate::mode::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub mode: Mode,
    pub seq: ChordSeq,
    pub command: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEntry {
    pub next: Chord,
    pub command: Option<&'static str>,
    pub bindings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Count(u32),
    Pending {
        candidates: Vec<PendingEntry>,
    },
    Run {
        id: &'static str,
        count: Option<u32>,
    },
    Unbound {
        flushed: Vec<Chord>,
    },
}

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

    pub fn keys_for(&self, command: &str) -> Vec<&Binding> {
        self.bindings
            .iter()
            .filter(|b| b.command == command)
            .collect()
    }

    pub fn resolve(&self, mode: Mode, pending: &mut Pending, chord: Chord) -> Resolution {
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
        bind(mode, "h", "motion.left");
        bind(mode, "l", "motion.right");
        bind(mode, "<Left>", "motion.left");
        bind(mode, "<Right>", "motion.right");
        bind(mode, "gg", "motion.top");
        bind(mode, "G", "motion.bottom");
        bind(mode, "<C-d>", "motion.half-down");
        bind(mode, "<C-u>", "motion.half-up");
        bind(mode, "<C-f>", "motion.page-down");
        bind(mode, "<C-b>", "motion.page-up");
        bind(mode, "<PageDown>", "motion.page-down");
        bind(mode, "<PageUp>", "motion.page-up");
        bind(mode, "y", "yank.field");
        bind(mode, "Y", "yank.record");
        bind(mode, "<Esc>", "app.cancel");
        bind(mode, ":", "app.command-line");
        bind(mode, "?", "app.help");
        bind(mode, "R", "app.refresh");
        bind(mode, "<leader>q", "app.quit");
        bind(mode, "<C-c>", "app.quit");
    }

    for mode in [Mode::Normal, Mode::Visual, Mode::VisualLine] {
        bind(mode, "<CR>", "nav.open");
    }
    bind(Mode::Normal, "-", "nav.up");
    // <Tab> and <C-i> are the same key on a terminal.
    bind(Mode::Normal, "<C-o>", "nav.jump-back");
    bind(Mode::Normal, "<C-i>", "nav.jump-forward");
    bind(Mode::Normal, "<Tab>", "nav.jump-forward");
    bind(Mode::Normal, "gs", "nav.schedules");
    bind(Mode::Normal, "gw", "nav.workflows");
    bind(Mode::Normal, "gd", "nav.dashboard");
    bind(Mode::Normal, "]p", "dashboard.next-panel");
    bind(Mode::Normal, "[p", "dashboard.prev-panel");

    bind(Mode::Normal, "<leader>T", "app.times");
    bind(Mode::Normal, "<leader>G", "history.timeline");

    for mode in [Mode::Normal, Mode::Visual, Mode::VisualLine] {
        bind(mode, "za", "history.fold");
        bind(mode, "zR", "history.expand-all");
        bind(mode, "zM", "history.collapse-all");
        bind(mode, "zp", "history.plumbing");
        bind(mode, "zg", "history.gaps");
        bind(mode, "]f", "history.next-failure");
        bind(mode, "[f", "history.prev-failure");
        bind(mode, "F", "history.follow");
        bind(mode, "K", "history.detail");
        bind(mode, "<C-e>", "history.detail-down");
        bind(mode, "<C-y>", "history.detail-up");
        bind(mode, "!", "payload.pipe");
        bind(mode, "<leader>ya", "yank.payload");
        bind(mode, "<leader>yi", "yank.payload-input");
        bind(mode, "<leader>yr", "yank.payload-result");
        bind(mode, "<leader>yw", "yank.workflow-id");
        bind(mode, "<leader>yR", "yank.run-id");

        // <C-w> prefix, never bare <C-h/j/k/l>: tmux's vim-tmux-navigator takes those.
        bind(mode, "<leader>sv", "window.split-right");
        bind(mode, "<leader>sh", "window.split-down");
        bind(mode, "<leader>sx", "window.close");
        bind(mode, "<leader>se", "window.equalize");
        bind(mode, "<C-w>h", "window.focus-left");
        bind(mode, "<C-w>j", "window.focus-down");
        bind(mode, "<C-w>k", "window.focus-up");
        bind(mode, "<C-w>l", "window.focus-right");
        bind(mode, "<leader>rh", "window.grow-left");
        bind(mode, "<leader>rj", "window.grow-down");
        bind(mode, "<leader>rk", "window.grow-up");
        bind(mode, "<leader>rl", "window.grow-right");
        bind(mode, "<leader>mc", "workflow.cancel");
        bind(mode, "<leader>mt", "workflow.terminate");
        bind(mode, "<leader>ms", "workflow.signal");
        bind(mode, "<leader>md", "workflow.delete");
        bind(mode, "<leader>mr", "workflow.reset");
        bind(mode, "<leader>mu", "workflow.update");
        bind(mode, "<leader>mp", "schedule.pause");
        bind(mode, "<leader>mg", "schedule.trigger");
        bind(mode, "<leader>mD", "schedule.delete");
        bind(mode, "<leader>mb", "schedule.backfill");
        bind(mode, "<leader>mn", "schedule.create");
        bind(mode, "<leader>cq", "workflow.query");
        bind(mode, "<leader>cs", "workflow.stack");

        bind(mode, "<leader>to", "tab.new");
        bind(mode, "<leader>tx", "tab.close");
        bind(mode, "<leader>tn", "tab.next");
        bind(mode, "<leader>tp", "tab.previous");
    }

    bind(Mode::Normal, "/", "search.open");
    bind(Mode::Normal, "n", "search.next");
    bind(Mode::Normal, "N", "search.previous");

    bind(Mode::Normal, "<leader>ff", "find.workflow");
    bind(Mode::Normal, "<leader>fl", "find.event");
    bind(Mode::Normal, "<leader>fb", "find.pane");
    bind(Mode::Normal, "<leader>fh", "find.command");
    bind(Mode::Normal, "<leader>fg", "find.filter");
    bind(Mode::Normal, "<leader>N", "find.namespace");
    bind(Mode::Normal, "<leader>P", "find.profile");
    bind(Mode::Normal, "<leader>xx", "list.problems");
    bind(Mode::Normal, "<leader>xm", "app.messages");
    bind(Mode::Normal, "<leader>e", "payload.edit");
    bind(Mode::Normal, "gf", "history.source");
    bind(Mode::Normal, "gc", "history.child");
    bind(Mode::Normal, "gp", "history.parent");

    bind(Mode::Normal, "i", "mode.insert");
    bind(Mode::Normal, "v", "mode.visual");
    bind(Mode::Normal, "V", "mode.visual-line");

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
    fn every_default_binding_names_a_command_that_exists() {
        let registry = crate::command::Registry::builtin();
        let map = map();
        let missing: Vec<&str> = map
            .bindings()
            .iter()
            .map(|b| b.command)
            .filter(|id| registry.get(id).is_none())
            .collect();
        assert!(missing.is_empty(), "bound to nothing: {missing:?}");
    }

    #[test]
    fn no_two_default_bindings_claim_the_same_keys_in_one_mode() {
        let map = map();
        let mut seen: Vec<(Mode, &ChordSeq)> = Vec::new();
        let mut clashes = Vec::new();
        for b in map.bindings() {
            if seen.iter().any(|(m, s)| *m == b.mode && *s == &b.seq) {
                clashes.push(format!("{:?} {:?} -> {}", b.mode, b.seq, b.command));
            }
            seen.push((b.mode, &b.seq));
        }
        assert!(clashes.is_empty(), "duplicate bindings: {clashes:?}");
    }

    #[test]
    fn leader_capital_p_switches_profile_beside_leader_capital_n() {
        let m = map();
        for (key, id) in [('P', "find.profile"), ('N', "find.namespace")] {
            let mut p = Pending::default();
            assert!(matches!(
                m.resolve(Mode::Normal, &mut p, m.leader()),
                Resolution::Pending { .. }
            ));
            assert_eq!(
                m.resolve(Mode::Normal, &mut p, Chord::ch(key)),
                Resolution::Run { id, count: None }
            );
        }
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
                let gg = candidates
                    .iter()
                    .find(|c| c.next == Chord::ch('g'))
                    .expect("gg should be reachable from g");
                assert_eq!(gg.command, Some("motion.top"));
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
