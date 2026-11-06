//! Tabs: a list of window trees, one of them current.

use crate::{Rect, Tree, ViewId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabs {
    tabs: Vec<Tree>,
    current: usize,
}

impl Tabs {
    pub fn new(view: ViewId) -> Self {
        Self {
            tabs: vec![Tree::new(view)],
            current: 0,
        }
    }

    pub fn current(&self) -> &Tree {
        &self.tabs[self.current]
    }

    pub fn current_mut(&mut self) -> &mut Tree {
        &mut self.tabs[self.current]
    }

    pub fn index(&self) -> usize {
        self.current
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn open(&mut self, view: ViewId) {
        self.current += 1;
        self.tabs.insert(self.current, Tree::new(view));
    }

    pub fn close(&mut self) -> bool {
        if self.tabs.len() == 1 {
            return false;
        }
        self.tabs.remove(self.current);
        self.current = self.current.min(self.tabs.len() - 1);
        true
    }

    pub fn next(&mut self) {
        self.current = (self.current + 1) % self.tabs.len();
    }

    pub fn previous(&mut self) {
        self.current = (self.current + self.tabs.len() - 1) % self.tabs.len();
    }

    pub fn views(&self) -> Vec<ViewId> {
        self.tabs.iter().flat_map(|t| t.views()).collect()
    }

    pub fn layout(&self, area: Rect) -> Vec<crate::Pane> {
        self.current().layout(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;

    fn v(n: u64) -> ViewId {
        ViewId(n)
    }

    #[test]
    fn a_session_starts_with_one_tab() {
        let tabs = Tabs::new(v(1));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.index(), 0);
        assert_eq!(tabs.current().focused(), v(1));
    }

    #[test]
    fn opening_a_tab_switches_to_it_and_inserts_after_the_current_one() {
        let mut tabs = Tabs::new(v(1));
        tabs.open(v(2));
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.index(), 1);
        assert_eq!(tabs.current().focused(), v(2));

        tabs.previous();
        tabs.open(v(3));
        assert_eq!(tabs.index(), 1);
        assert_eq!(tabs.views(), [v(1), v(3), v(2)]);
    }

    #[test]
    fn tabs_wrap_in_both_directions() {
        let mut tabs = Tabs::new(v(1));
        tabs.open(v(2));
        tabs.open(v(3));
        assert_eq!(tabs.index(), 2);

        tabs.next();
        assert_eq!(tabs.index(), 0, "past the end comes back to the first");
        tabs.previous();
        assert_eq!(tabs.index(), 2, "and before the first is the last");
    }

    #[test]
    fn closing_a_tab_lands_on_the_one_that_took_its_place() {
        let mut tabs = Tabs::new(v(1));
        tabs.open(v(2));
        tabs.open(v(3));
        tabs.previous();

        assert!(tabs.close());
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.index(), 1);
        assert_eq!(tabs.current().focused(), v(3));
    }

    #[test]
    fn closing_the_last_tab_lands_on_the_new_last() {
        let mut tabs = Tabs::new(v(1));
        tabs.open(v(2));
        assert!(tabs.close());
        assert_eq!(tabs.index(), 0);
        assert_eq!(tabs.current().focused(), v(1));
    }

    #[test]
    fn the_final_tab_cannot_be_closed() {
        let mut tabs = Tabs::new(v(1));
        assert!(!tabs.close());
        assert_eq!(tabs.len(), 1);
    }

    #[test]
    fn each_tab_keeps_its_own_layout() {
        let mut tabs = Tabs::new(v(1));
        tabs.current_mut().split(Axis::Columns, v(2));
        assert_eq!(tabs.current().len(), 2);

        tabs.open(v(3));
        assert_eq!(tabs.current().len(), 1, "a new tab starts with one window");

        tabs.previous();
        assert_eq!(tabs.current().len(), 2, "the split is still there");
    }
}
