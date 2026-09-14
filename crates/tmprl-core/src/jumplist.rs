//! The jumplist: `<C-o>` back, `<C-i>` forward.
//!
//! Generic over what a position *is*, which is what keeps this crate free of screens. The
//! application decides that a position is "this pane, on this screen, scoped here, looking
//! at that workflow, cursor on row N"; this module only knows how vim's list behaves.
//!
//! The behaviour is worth stating because it is not a plain undo stack:
//!
//! * Going back from the newest position **records that position first**, so `<C-i>` can
//!   return to it. Without that, one `<C-o>` is a one-way trip.
//! * A new jump **discards the forward entries**, exactly as vim does. Having gone back
//!   two and then jumped somewhere new, `<C-i>` would otherwise return to a future that no
//!   longer follows from where you are.
//! * The list is bounded. It holds navigation history for a session that might run all day,
//!   and an unbounded one is a slow leak of whole workflow rows.

/// A bounded list of positions with a cursor into it.
#[derive(Debug, Clone)]
pub struct Jumplist<T> {
    entries: Vec<T>,
    /// Where we are in `entries`. `entries.len()` means "at the live position", which is
    /// not itself in the list until something needs it to be.
    at: usize,
    limit: usize,
}

/// vim's default `'jumpoptions'` depth. No reason to differ, and a round number people
/// already have an intuition for beats a new one.
pub const DEFAULT_LIMIT: usize = 100;

impl<T> Default for Jumplist<T> {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl<T> Jumplist<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            at: 0,
            limit: limit.max(1),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Record that we are leaving `from`.
    ///
    /// Called *before* the navigation happens, with the position being left behind: that is
    /// what `<C-o>` should return to.
    pub fn push(&mut self, from: T) {
        // Anything ahead of the cursor is a future that no longer follows from here.
        self.entries.truncate(self.at);
        self.entries.push(from);
        if self.entries.len() > self.limit {
            // Drop the oldest. The cursor is about to be set to the end anyway, but doing
            // this by subtraction rather than assignment keeps the two in step if this ever
            // stops being true.
            let excess = self.entries.len() - self.limit;
            self.entries.drain(..excess);
        }
        self.at = self.entries.len();
    }

    /// `<C-o>`. `current` is where we are now, so that `<C-i>` has somewhere to return to.
    pub fn back(&mut self, current: T) -> Option<&T> {
        if self.at == 0 {
            return None;
        }
        // Leaving the live position for the first time: park it at the end so forward
        // motion can find it again.
        if self.at == self.entries.len() {
            self.entries.push(current);
        }
        self.at -= 1;
        self.entries.get(self.at)
    }

    /// `<C-i>`.
    pub fn forward(&mut self) -> Option<&T> {
        if self.at + 1 >= self.entries.len() {
            return None;
        }
        self.at += 1;
        self.entries.get(self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Jumplist<&'static str> {
        Jumplist::new(DEFAULT_LIMIT)
    }

    #[test]
    fn there_is_nowhere_to_go_back_to_at_first() {
        let mut j = list();
        assert_eq!(j.back("here"), None);
        assert_eq!(j.forward(), None);
    }

    #[test]
    fn back_returns_the_position_that_was_left() {
        let mut j = list();
        j.push("a"); // left a, now at b
        assert_eq!(j.back("b"), Some(&"a"));
    }

    #[test]
    fn forward_returns_to_where_back_was_pressed_from() {
        // The reason `back` takes the current position: without parking it, one `<C-o>` is
        // a one-way trip and `<C-i>` has nothing to return to.
        let mut j = list();
        j.push("a");
        assert_eq!(j.back("b"), Some(&"a"));
        assert_eq!(j.forward(), Some(&"b"));
    }

    #[test]
    fn back_walks_further_each_time() {
        let mut j = list();
        j.push("a");
        j.push("b");
        assert_eq!(j.back("c"), Some(&"b"));
        assert_eq!(j.back("c"), Some(&"a"));
        assert_eq!(j.back("c"), None, "nothing older than a");
    }

    #[test]
    fn forward_stops_at_the_newest_position() {
        let mut j = list();
        j.push("a");
        j.back("b");
        assert_eq!(j.forward(), Some(&"b"));
        assert_eq!(j.forward(), None);
    }

    #[test]
    fn a_new_jump_discards_the_forward_entries() {
        // Gone back to `a`, then jumped somewhere new: `<C-i>` must not offer `b`, which is
        // a future that no longer follows from here.
        let mut j = list();
        j.push("a");
        j.push("b");
        assert_eq!(j.back("c"), Some(&"b"));
        assert_eq!(j.back("c"), Some(&"a"));

        j.push("a");
        assert_eq!(j.forward(), None, "the old future is gone");
        assert_eq!(j.back("z"), Some(&"a"));
    }

    #[test]
    fn the_list_is_bounded_and_drops_the_oldest() {
        let mut j: Jumplist<usize> = Jumplist::new(3);
        for i in 0..10 {
            j.push(i);
        }
        assert_eq!(j.len(), 3);
        assert_eq!(j.back(99), Some(&9));
        assert_eq!(j.back(99), Some(&8));
        assert_eq!(j.back(99), Some(&7));
        assert_eq!(j.back(99), None, "6 and older were dropped");
    }

    #[test]
    fn a_limit_of_zero_is_treated_as_one_rather_than_dividing_by_it() {
        let mut j: Jumplist<usize> = Jumplist::new(0);
        j.push(1);
        assert_eq!(j.len(), 1);
    }
}
