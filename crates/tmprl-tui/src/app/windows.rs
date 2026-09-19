//! Splits, tabs and moving focus between panes.

use super::*;

impl App {
    /// Focus a pane by id, switching tabs if it lives in another one.
    pub(super) fn focus_pane(&mut self, id: ViewId) {
        if self.tabs.current().focused() == id {
            return;
        }
        let previous = self.tabs.current().focused();
        if self.tabs.current_mut().focus_view(id) {
            self.refocus(previous);
            return;
        }
        // Not in this tab. `Tabs` exposes no way to look inside a tab that is not current,
        // so finding it means rotating; the starting index is kept so a miss can put the
        // rotation back. Without that, failing to find a pane silently leaves you on a
        // different tab from the one you were on.
        let start = self.tabs.index();
        for _ in 1..self.tabs.len() {
            self.tabs.next();
            if self.tabs.current().views().contains(&id) {
                self.tabs.current_mut().focus_view(id);
                self.refocus(previous);
                return;
            }
        }
        while self.tabs.index() != start {
            self.tabs.next();
        }
        self.note = Some(("that pane has gone".into(), Note::Warn));
    }

    /// Park the focused pane and take up whichever one the tree now points at.
    ///
    /// The reducer always acts on `self.view`, so every operation that can move focus ends
    /// here. Swapping rather than looking up is what lets the rest of the reducer stay
    /// unaware that there is more than one pane.
    pub(super) fn refocus(&mut self, previous: ViewId) {
        let now = self.tabs.current().focused();
        if now == previous {
            return;
        }
        let namespace = self.namespace.clone();
        let incoming = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
        let outgoing = std::mem::replace(&mut self.view, incoming);
        self.parked.insert(previous, outgoing);
    }

    pub(super) fn fresh_view_id(&mut self) -> ViewId {
        let id = ViewId(self.next_view_id);
        self.next_view_id += 1;
        id
    }

    /// Split the focused window. The new pane starts where this one is, which is almost
    /// always what you wanted it for, comparing two histories means opening the same place
    /// twice and then navigating one of them away.
    pub(super) fn split(&mut self, axis: Axis) {
        let previous = self.tabs.current().focused();
        let id = self.fresh_view_id();
        let forked = self.view.fork();
        self.parked.insert(id, forked);
        self.tabs.current_mut().split(axis, id);
        self.refocus(previous);
        // The new pane knows where it is but has fetched nothing yet.
        self.load_for_screen();
        self.note = Some((format!("{} windows", self.tabs.current().len()), Note::Info));
    }

    pub(super) fn close_window(&mut self) {
        let previous = self.tabs.current().focused();
        if !self.tabs.current_mut().close() {
            self.note = Some(("last window, <Space>q to quit tmprl".into(), Note::Warn));
            return;
        }
        // Its state goes with it, and View's Drop stops any follow poll it had running.
        self.parked.remove(&previous);
        let now = self.tabs.current().focused();
        let namespace = self.namespace.clone();
        let incoming = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
        self.view = incoming;
    }

    pub(super) fn focus_window(&mut self, dir: Direction) {
        let previous = self.tabs.current().focused();
        if self.tabs.current_mut().focus_direction(dir, self.frame) {
            self.refocus(previous);
        }
    }

    pub(super) fn resize_window(&mut self, dir: Direction) {
        // Ten cells' worth, as `<leader>r{hjkl}` promises. Weights are relative, so this is
        // a nudge rather than an exact cell count.
        self.tabs.current_mut().resize(dir, 10);
    }

    pub(super) fn new_tab(&mut self) {
        let previous = self.tabs.current().focused();
        let id = self.fresh_view_id();
        self.parked.insert(
            previous,
            std::mem::replace(&mut self.view, View::new(&self.namespace)),
        );
        self.tabs.open(id);
        // The new tab's view is the one we just made; nothing to take from `parked`.
        let _ = previous;
        self.load_for_screen();
    }

    pub(super) fn close_tab(&mut self) {
        if self.tabs.len() == 1 {
            self.note = Some(("last tab, <Space>q to quit tmprl".into(), Note::Warn));
            return;
        }
        // Every pane in the tab goes, along with whatever each was polling.
        for id in self.tabs.current().views() {
            self.parked.remove(&id);
        }
        self.tabs.close();
        let now = self.tabs.current().focused();
        let namespace = self.namespace.clone();
        self.view = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
    }

    pub(super) fn switch_tab(&mut self, forward: bool) {
        if self.tabs.len() == 1 {
            return;
        }
        let previous = self.tabs.current().focused();
        self.parked.insert(
            previous,
            std::mem::replace(&mut self.view, View::new(&self.namespace)),
        );
        if forward {
            self.tabs.next();
        } else {
            self.tabs.previous();
        }
        let now = self.tabs.current().focused();
        let namespace = self.namespace.clone();
        self.view = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
    }

    /// Record the area the panes were laid out in, so focus movement is geometric against
    /// what is actually on screen rather than against a guess.
    pub fn set_frame(&mut self, area: UiRect) {
        self.frame = area;
    }

    /// A non-focused pane's state, for rendering it.
    pub fn parked_view(&self, id: ViewId) -> Option<&View> {
        self.parked.get(&id)
    }
}
