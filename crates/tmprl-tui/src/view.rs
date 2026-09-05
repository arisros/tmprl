//! What one pane owns.
//!
//! Everything here is per-pane: two windows side by side each have their own cursor, their
//! own query, their own history and their own follow task. Everything that is *not* here —
//! the mode, the keymap, the prompt, the note line, the codec cache — belongs to the session
//! and is shared, because there is one keyboard and one status line however many panes are
//! open.
//!
//! Splitting this out of `App` is what makes a window tree possible at all. Before it, the
//! application held one cursor and one history, so a second pane had nothing of its own to
//! show.

use tmprl_client::NamespaceInfo;
use tmprl_core::history::NormalizedEvent;
use tmprl_core::outline::Outline;
use tmprl_core::{Loadable, ScheduleRow, StatusCounts, WorkflowList, WorkflowRow};

use crate::app::Screen;

pub struct View {
    pub screen: Screen,
    pub namespaces: Loadable<Vec<NamespaceInfo>>,
    pub workflows: Loadable<WorkflowList>,
    pub counts: Loadable<StatusCounts>,
    pub history: Loadable<Outline>,
    pub schedules: Loadable<Vec<ScheduleRow>>,

    /// The workflow whose history is on screen.
    pub viewing: Option<WorkflowRow>,
    /// Whether the history is being tailed. Shown in the statusline, because a view that
    /// silently changes under you is worse than one that does not update.
    pub following: bool,
    /// Whether the payload pane is open under the history list.
    pub show_detail: bool,
    /// Output of the last `!` filter, shown in the pane in place of the payloads. `Err` is
    /// the command's own stderr, which is the only useful thing to show when jq rejects a
    /// filter.
    pub piped: Option<Result<String, String>>,
    /// First visible line of the payload pane, and how far it can usefully go. A payload can
    /// be far taller than the pane — clipping a stack trace silently hides its end, which is
    /// the part worth reading.
    pub detail_scroll: usize,
    pub detail_max_scroll: usize,

    /// The visibility query, verbatim. This string is the interface: everything that
    /// filters the list compiles into it, and it is always on screen and always editable.
    pub query: String,
    /// Namespaces the workflow list is fanned out over.
    pub scope: Vec<String>,

    pub cursor: usize,
    /// Where a visual selection started, if one is active.
    pub anchor: Option<usize>,
    /// Rows this pane can show — set by the renderer, used by half-page motions. Per-pane
    /// because a half page in a split is half of *that* pane.
    pub page: usize,

    /// The row the cursor is on, by identity rather than by index. Rows arrive above the
    /// cursor on a live list, so an index silently drifts onto a different workflow.
    pub cursor_key: Option<(String, String)>,
    /// Cursor position on the namespace screen, restored by `-`.
    pub namespace_cursor: usize,
    /// Cursor position on the workflow list, restored by `-` from a history.
    pub workflow_cursor: usize,
    /// Bumped whenever the query or scope changes. Replies carrying an older generation
    /// belong to a query the user has already moved on from.
    ///
    /// Per-pane, and that matters: two panes fetching at once must not invalidate each
    /// other's replies.
    pub generation: u64,
    /// Every history event loaded so far, for re-grouping when a page arrives.
    pub history_events: Vec<NormalizedEvent>,
    /// Continuation token for the history being read. Empty means fully loaded.
    pub history_token: Vec<u8>,
    /// The last *non-empty* token seen. Follow resumes from here: an empty token restarts
    /// from event 1, and paging leaves the token empty once it has caught up.
    pub history_resume: Vec<u8>,
    /// The follow task, so toggling off — or leaving the screen — actually stops the poll.
    pub follow_task: Option<tokio::task::JoinHandle<()>>,
    /// A page request is in flight; scrolling must not queue a second one.
    pub loading_more: bool,
}

impl View {
    /// A fresh pane, scoped to one namespace.
    pub fn new(namespace: &str) -> Self {
        Self {
            screen: Screen::Namespaces,
            namespaces: Loadable::NotAsked,
            workflows: Loadable::NotAsked,
            counts: Loadable::NotAsked,
            history: Loadable::NotAsked,
            schedules: Loadable::NotAsked,
            viewing: None,
            following: false,
            show_detail: false,
            piped: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            query: String::new(),
            scope: vec![namespace.to_string()],
            cursor: 0,
            anchor: None,
            page: 10,
            cursor_key: None,
            namespace_cursor: 0,
            workflow_cursor: 0,
            generation: 0,
            history_events: Vec::new(),
            history_token: Vec::new(),
            history_resume: Vec::new(),
            follow_task: None,
            loading_more: false,
        }
    }

    /// A new pane looking at the same place as this one.
    ///
    /// The navigation is copied — screen, scope, query, which workflow — but none of the
    /// loaded data, tasks or tokens: the new pane fetches its own. Splitting is almost
    /// always "show me this again so I can take one of them somewhere else", and a split
    /// that dropped you back at the namespace list would make the diff case two navigations
    /// instead of one keystroke.
    pub fn fork(&self) -> Self {
        // Field-by-field rather than `..View::new(..)`: View has a Drop (it aborts a follow
        // poll), and struct update syntax would have to move out of the base value.
        let mut out = View::new(self.scope.first().map(String::as_str).unwrap_or_default());
        out.screen = self.screen;
        out.scope = self.scope.clone();
        out.query = self.query.clone();
        out.viewing = self.viewing.clone();
        out.show_detail = self.show_detail;
        out.namespace_cursor = self.namespace_cursor;
        out.workflow_cursor = self.workflow_cursor;
        out
    }

    /// Stop this pane's follow poll, if it has one.
    ///
    /// A poll left running holds a request open and keeps feeding a pane that may have been
    /// closed, so closing a window has to come through here.
    pub fn stop_following(&mut self) {
        self.following = false;
        if let Some(task) = self.follow_task.take() {
            task.abort();
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        // Closing a window must not leave its long poll running against a pane that no
        // longer exists.
        if let Some(task) = self.follow_task.take() {
            task.abort();
        }
    }
}

impl View {
    pub fn namespace_rows(&self) -> &[NamespaceInfo] {
        self.namespaces.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn workflow_rows(&self) -> &[WorkflowRow] {
        self.workflows
            .value()
            .map(WorkflowList::rows)
            .unwrap_or(&[])
    }

    pub fn schedule_rows(&self) -> &[ScheduleRow] {
        self.schedules.value().map(Vec::as_slice).unwrap_or(&[])
    }

    /// How many rows this pane's screen has.
    pub fn row_count(&self) -> usize {
        match self.screen {
            Screen::Namespaces => self.namespace_rows().len(),
            Screen::Workflows => self.workflow_rows().len(),
            Screen::History => self.history.value().map(Outline::len).unwrap_or(0),
            Screen::Schedules => self.schedule_rows().len(),
        }
    }

    /// Whether this pane is fanned out over more than one namespace, which is when rows
    /// need to say which namespace they came from.
    pub fn is_fanned_out(&self) -> bool {
        self.scope.len() > 1
    }

    /// The inclusive row range selected, if a visual mode is active in this pane.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.selection().is_some_and(|(lo, hi)| i >= lo && i <= hi)
    }
}
