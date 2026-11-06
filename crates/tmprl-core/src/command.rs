//! The command registry.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadPart {
    All,
    Input,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    ToggleHelp,
    ToggleMessages,
    OpenCommandLine,
    Cancel,
    Refresh,
    ToggleTimes,

    OpenItem,
    GoUp,
    GoSchedules,
    GoWorkflows,
    GoDashboard,
    NextPanel,
    PrevPanel,
    JumpBack,
    JumpForward,

    MoveDown,
    MoveUp,
    MoveLeft,
    MoveRight,
    MoveTop,
    MoveBottom,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,

    EnterInsert,
    LeaveInsert,
    EnterVisual,
    EnterVisualLine,

    OpenSearch,
    SearchNext,
    SearchPrev,
    FindWorkflow,
    FindEvent,
    FindPane,
    FindCommand,
    FindFilter,
    FindNamespace,
    FindProfile,
    ProblemList,

    YankField,
    YankRecord,
    YankPayloadAll,
    YankPayloadInput,
    YankPayloadResult,
    YankWorkflowId,
    YankRunId,
    LoadMore,

    ToggleFold,
    ExpandAll,
    CollapseAll,
    TogglePlumbing,
    ToggleTimeline,
    ToggleGaps,
    NextFailure,
    PrevFailure,
    ToggleFollow,
    ToggleDetail,
    DetailDown,
    DetailUp,
    OpenPipe,
    OpenEditor,
    OpenSource,
    OpenChild,
    OpenParent,

    SplitRight,
    SplitDown,
    CloseWindow,
    EqualizeWindows,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    GrowLeft,
    GrowRight,
    GrowUp,
    GrowDown,
    CancelWorkflow,
    TerminateWorkflow,
    SignalWorkflow,
    DeleteWorkflow,
    ResetWorkflow,
    UpdateWorkflow,
    PauseSchedule,
    TriggerSchedule,
    DeleteSchedule,
    BackfillSchedule,
    CreateSchedule,

    QueryWorkflow,
    StackTrace,

    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    SelectView(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub action: Action,
}

pub struct Registry {
    commands: Vec<Command>,
}

macro_rules! commands {
    ($( $id:literal, $group:literal, $title:literal => $action:ident );* $(;)?) => {
        vec![$(
            Command { id: $id, title: $title, group: $group, action: Action::$action },
        )*]
    };
}

impl Registry {
    pub fn builtin() -> Self {
        let commands = commands! {
            "app.quit",           "Application", "Quit"                      => Quit;
            "app.help",           "Application", "Toggle help"               => ToggleHelp;
            "app.messages",       "Application", "Everything the note line has said" => ToggleMessages;
            "app.command-line",   "Application", "Open the command line"     => OpenCommandLine;
            "app.cancel",         "Application", "Cancel pending input"      => Cancel;
            "app.refresh",        "Application", "Reload from the server"    => Refresh;
            "app.times",          "Application", "Clock times or ages"       => ToggleTimes;

            "motion.down",        "Motion",      "Move down"                 => MoveDown;
            "motion.up",          "Motion",      "Move up"                   => MoveUp;
            "motion.left",        "Motion",      "Move left"                 => MoveLeft;
            "motion.right",       "Motion",      "Move right"                => MoveRight;
            "motion.top",         "Motion",      "Go to first item"          => MoveTop;
            "motion.bottom",      "Motion",      "Go to last item"           => MoveBottom;
            "motion.half-down",   "Motion",      "Half page down"            => HalfPageDown;
            "motion.half-up",     "Motion",      "Half page up"              => HalfPageUp;
            "motion.page-down",   "Motion",      "Page down"                 => PageDown;
            "motion.page-up",     "Motion",      "Page up"                   => PageUp;

            "mode.insert",        "Mode",        "Enter Insert mode"         => EnterInsert;
            "mode.normal",        "Mode",        "Leave Insert mode"         => LeaveInsert;
            "mode.visual",        "Mode",        "Enter Visual mode"         => EnterVisual;
            "mode.visual-line",   "Mode",        "Enter Visual Line mode"    => EnterVisualLine;

            "nav.open",           "Navigation",  "Open the focused item"     => OpenItem;
            "nav.up",             "Navigation",  "Go up a level"             => GoUp;
            "nav.schedules",      "Navigation",  "Go to schedules"           => GoSchedules;
            "nav.workflows",      "Navigation",  "Go to workflows"           => GoWorkflows;
            "nav.dashboard",      "Navigation",  "Go to the dashboard"       => GoDashboard;
            "dashboard.next-panel","Navigation", "Next dashboard panel"      => NextPanel;
            "dashboard.prev-panel","Navigation", "Previous dashboard panel"  => PrevPanel;
            "nav.jump-back",      "Navigation",  "Jump back"                 => JumpBack;
            "nav.jump-forward",   "Navigation",  "Jump forward"              => JumpForward;

            "search.open",        "Find",        "Search within this view"   => OpenSearch;
            "search.next",        "Find",        "Next match"                => SearchNext;
            "search.previous",    "Find",        "Previous match"            => SearchPrev;
            "find.workflow",      "Find",        "Find a workflow"           => FindWorkflow;
            "find.event",         "Find",        "Find an event here"        => FindEvent;
            "find.pane",          "Find",        "Find an open pane"         => FindPane;
            "find.command",       "Find",        "Find a command"            => FindCommand;
            "find.filter",        "Find",        "Build a query filter"      => FindFilter;
            "find.namespace",     "Find",        "Switch namespace"          => FindNamespace;
            "find.profile",       "Find",        "Switch connection profile" => FindProfile;
            "list.problems",      "Find",        "Failed and stuck workflows" => ProblemList;

            "yank.field",         "Yank",        "Yank the focused value"    => YankField;
            "yank.record",        "Yank",        "Yank the row as JSON"      => YankRecord;
            "yank.payload",       "Yank",        "Yank every payload here"   => YankPayloadAll;
            "yank.payload-input", "Yank",        "Yank the input payloads"   => YankPayloadInput;
            "yank.payload-result","Yank",        "Yank the result payload"   => YankPayloadResult;
            "yank.workflow-id",   "Yank",        "Yank the workflow id"      => YankWorkflowId;
            "yank.run-id",        "Yank",        "Yank the run id"           => YankRunId;

            "list.more",          "List",        "Load the next page"        => LoadMore;

            "history.fold",       "History",     "Fold a group open or shut" => ToggleFold;
            "history.expand-all", "History",     "Expand every group"        => ExpandAll;
            "history.collapse-all","History",    "Collapse every group"      => CollapseAll;
            "history.plumbing",   "History",     "Show/hide workflow tasks"  => TogglePlumbing;
            "history.timeline",   "History",     "Timeline, like the web UI" => ToggleTimeline;
            "history.gaps",       "History",     "Fold/unfold idle time"     => ToggleGaps;
            "history.next-failure","History",    "Jump to the next failure"  => NextFailure;
            "history.prev-failure","History",    "Jump to the previous failure" => PrevFailure;
            "history.follow",     "History",     "Follow, tail a running workflow" => ToggleFollow;
            "history.detail",     "History",     "Show the payloads under the cursor" => ToggleDetail;
            "history.detail-down","History",     "Scroll the payload pane down" => DetailDown;
            "history.detail-up",  "History",     "Scroll the payload pane up"   => DetailUp;
            "payload.pipe",       "History",     "Pipe payloads through a command" => OpenPipe;
            "payload.edit",       "History",     "Open the payloads in $EDITOR" => OpenEditor;
            "history.source",     "History",     "Go to the source of this event" => OpenSource;
            "history.child",      "History",     "Open the child workflow"   => OpenChild;
            "history.parent",     "History",     "Open the parent workflow"  => OpenParent;

            "window.split-right", "Windows",     "Split side by side"        => SplitRight;
            "window.split-down",  "Windows",     "Split above and below"     => SplitDown;
            "window.close",       "Windows",     "Close this window"         => CloseWindow;
            "window.equalize",    "Windows",     "Give windows equal space"  => EqualizeWindows;
            "window.focus-left",  "Windows",     "Focus the window left"     => FocusLeft;
            "window.focus-right", "Windows",     "Focus the window right"    => FocusRight;
            "window.focus-up",    "Windows",     "Focus the window above"    => FocusUp;
            "window.focus-down",  "Windows",     "Focus the window below"    => FocusDown;
            "window.grow-left",   "Windows",     "Widen to the left"         => GrowLeft;
            "window.grow-right",  "Windows",     "Widen to the right"        => GrowRight;
            "window.grow-up",     "Windows",     "Grow upwards"              => GrowUp;
            "window.grow-down",   "Windows",     "Grow downwards"            => GrowDown;

            "workflow.cancel",    "Mutate",      "Cancel this workflow"      => CancelWorkflow;
            "workflow.terminate", "Mutate",      "Terminate this workflow"   => TerminateWorkflow;
            "workflow.signal",    "Mutate",      "Signal this workflow"      => SignalWorkflow;
            "workflow.delete",    "Mutate",      "Delete this workflow"      => DeleteWorkflow;
            "workflow.reset",     "Mutate",      "Reset to the event here"   => ResetWorkflow;
            "workflow.update",    "Mutate",      "Send an update"            => UpdateWorkflow;
            "schedule.pause",     "Mutate",      "Pause or resume a schedule" => PauseSchedule;
            "schedule.trigger",   "Mutate",      "Run a schedule now"        => TriggerSchedule;
            "schedule.delete",    "Mutate",      "Delete this schedule"      => DeleteSchedule;
            "schedule.backfill",  "Mutate",      "Backfill a time range"     => BackfillSchedule;
            "schedule.create",    "Mutate",      "Create a schedule"         => CreateSchedule;

            "workflow.query",     "Query",       "Ask the workflow a query"  => QueryWorkflow;
            "workflow.stack",     "Query",       "Where the workflow is blocked" => StackTrace;

            "tab.new",            "Tabs",        "Open a tab"                => NewTab;
            "tab.close",          "Tabs",        "Close this tab"            => CloseTab;
            "tab.next",           "Tabs",        "Next tab"                  => NextTab;
            "tab.previous",       "Tabs",        "Previous tab"              => PrevTab;
        };
        Self { commands }
    }

    pub fn add_views(&mut self, views: &[crate::config::SavedView]) {
        for v in views {
            let id: &'static str = Box::leak(format!("view.{}", v.key).into_boxed_str());
            let title: &'static str = Box::leak(v.name.clone().into_boxed_str());
            self.commands.retain(|c| c.id != id);
            self.commands.push(Command {
                id,
                title,
                group: "Views",
                action: Action::SelectView(v.key),
            });
        }
    }

    pub fn all(&self) -> &[Command] {
        &self.commands
    }

    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
    }

    pub fn search(&self, query: &str) -> Vec<&Command> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            let mut all: Vec<&Command> = self.commands.iter().collect();
            all.sort_by_key(|c| c.id);
            return all;
        }
        let mut hits: Vec<(&Command, i32)> = self
            .commands
            .iter()
            .filter_map(|c| {
                let by_id = crate::fuzzy::match_score(&q, c.id).map(|m| m.score);
                let by_title = crate::fuzzy::match_score(&q, c.title).map(|m| m.score);
                by_id.max(by_title).map(|s| (c, s))
            })
            .collect();
        hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(b.0.id)));
        hits.into_iter().map(|(c, _)| c).collect()
    }

    pub fn groups(&self) -> Vec<&'static str> {
        let mut seen = Vec::new();
        for c in &self.commands {
            if !seen.contains(&c.group) {
                seen.push(c.group);
            }
        }
        seen
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let r = Registry::builtin();
        let mut ids: Vec<_> = r.all().iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate command id in the registry");
    }

    #[test]
    fn every_action_is_registered() {
        let r = Registry::builtin();
        let actions: std::collections::HashSet<_> = r.all().iter().map(|c| c.action).collect();
        assert_eq!(
            actions.len(),
            r.all().len(),
            "two commands share an Action; each should be distinct"
        );
    }

    #[test]
    fn lookup_by_id() {
        let r = Registry::builtin();
        assert_eq!(r.get("motion.down").unwrap().action, Action::MoveDown);
        assert!(r.get("nope").is_none());
    }

    #[test]
    fn search_prefers_prefix_matches() {
        let r = Registry::builtin();
        let hits = r.search("motion.");
        assert!(hits.iter().all(|c| c.id.starts_with("motion.")));
        assert!(hits.len() >= 6);
    }

    #[test]
    fn search_matches_subsequences_and_titles() {
        let r = Registry::builtin();
        assert!(r.search("mdown").iter().any(|c| c.id == "motion.down"));
        assert!(r.search("quit").iter().any(|c| c.id == "app.quit"));
    }

    #[test]
    fn saved_views_become_real_commands() {
        use crate::config::SavedView;
        let mut r = Registry::builtin();
        r.add_views(&[
            SavedView {
                key: '1',
                name: "Running".into(),
                query: "ExecutionStatus = 'Running'".into(),
            },
            SavedView {
                key: '2',
                name: "Broken".into(),
                query: "ExecutionStatus = 'Failed'".into(),
            },
        ]);

        let one = r.get("view.1").expect("view.1 should be registered");
        assert_eq!(one.action, Action::SelectView('1'));
        assert_eq!(one.title, "Running", "the view's own name is its title");
        assert_eq!(one.group, "Views");
        assert_eq!(r.get("view.2").unwrap().action, Action::SelectView('2'));
        assert!(r.search("view.").iter().any(|c| c.id == "view.1"));
    }

    #[test]
    fn reloading_views_replaces_rather_than_duplicates() {
        use crate::config::SavedView;
        let mut r = Registry::builtin();
        let view = |name: &str| SavedView {
            key: '1',
            name: name.into(),
            query: String::new(),
        };
        r.add_views(&[view("First")]);
        r.add_views(&[view("Second")]);

        let hits: Vec<_> = r.all().iter().filter(|c| c.id == "view.1").collect();
        assert_eq!(hits.len(), 1, "a reloaded view must not register twice");
        assert_eq!(hits[0].title, "Second");
    }

    #[test]
    fn empty_search_returns_everything_sorted() {
        let r = Registry::builtin();
        let hits = r.search("  ");
        assert_eq!(hits.len(), r.all().len());
        assert!(hits.windows(2).all(|w| w[0].id <= w[1].id));
    }
}
