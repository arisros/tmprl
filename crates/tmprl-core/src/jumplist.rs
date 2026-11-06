//! The jumplist: `<C-o>` back, `<C-i>` forward.

#[derive(Debug, Clone)]
pub struct Jumplist<T> {
    entries: Vec<T>,
    at: usize,
    limit: usize,
}

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

    pub fn push(&mut self, from: T) {
        self.entries.truncate(self.at);
        self.entries.push(from);
        if self.entries.len() > self.limit {
            let excess = self.entries.len() - self.limit;
            self.entries.drain(..excess);
        }
        self.at = self.entries.len();
    }

    pub fn back(&mut self, current: T) -> Option<&T> {
        if self.at == 0 {
            return None;
        }
        if self.at == self.entries.len() {
            self.entries.push(current);
        }
        self.at -= 1;
        self.entries.get(self.at)
    }

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
        j.push("a");
        assert_eq!(j.back("b"), Some(&"a"));
    }

    #[test]
    fn forward_returns_to_where_back_was_pressed_from() {
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
