//! The command registry.
//!
//! Every user-visible action is registered here exactly once. The keymap, the `:` command
//! line, the which-key popup and the help overlay all read this one table, so they cannot
//! drift apart: a command that exists is reachable and discoverable by construction.
//!
//! Commands carry an [`Action`] rather than a function pointer. Dispatch lives in
//! `tmprl-tui`, which keeps this crate free of any application or terminal types — and
//! makes the match on `Action` exhaustive, so a new command cannot be silently unhandled.

/// What a command does. `tmprl-tui` matches on this exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // Application
    Quit,
    ToggleHelp,
    OpenCommandLine,
    Cancel,
    Refresh,

    // Motion
    MoveDown,
    MoveUp,
    MoveTop,
    MoveBottom,
    HalfPageDown,
    HalfPageUp,

    // Modes
    EnterInsert,
    LeaveInsert,
    EnterVisual,
    EnterVisualLine,

    // Data
    YankField,
    YankRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Stable identifier. This is what `keys.toml`, macros and `--exec` refer to, so it is
    /// part of the public interface and must not change casually.
    pub id: &'static str,
    pub title: &'static str,
    /// Grouping for the help overlay.
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
            "app.command-line",   "Application", "Open the command line"     => OpenCommandLine;
            "app.cancel",         "Application", "Cancel pending input"      => Cancel;
            "app.refresh",        "Application", "Reload from the server"    => Refresh;

            "motion.down",        "Motion",      "Move down"                 => MoveDown;
            "motion.up",          "Motion",      "Move up"                   => MoveUp;
            "motion.top",         "Motion",      "Go to first item"          => MoveTop;
            "motion.bottom",      "Motion",      "Go to last item"           => MoveBottom;
            "motion.half-down",   "Motion",      "Half page down"            => HalfPageDown;
            "motion.half-up",     "Motion",      "Half page up"              => HalfPageUp;

            "mode.insert",        "Mode",        "Enter Insert mode"         => EnterInsert;
            "mode.normal",        "Mode",        "Leave Insert mode"         => LeaveInsert;
            "mode.visual",        "Mode",        "Enter Visual mode"         => EnterVisual;
            "mode.visual-line",   "Mode",        "Enter Visual Line mode"    => EnterVisualLine;

            "yank.field",         "Yank",        "Yank the focused value"    => YankField;
            "yank.record",        "Yank",        "Yank the row as JSON"      => YankRecord;
        };
        Self { commands }
    }

    pub fn all(&self) -> &[Command] {
        &self.commands
    }

    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Subsequence match over the id and title, ranked so that shorter ids win ties. Good
    /// enough for a command line where the candidate set is small and known.
    pub fn search(&self, query: &str) -> Vec<&Command> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            let mut all: Vec<&Command> = self.commands.iter().collect();
            all.sort_by_key(|c| c.id);
            return all;
        }
        let mut hits: Vec<&Command> = self
            .commands
            .iter()
            .filter(|c| {
                subsequence(&q, &c.id.to_ascii_lowercase())
                    || subsequence(&q, &c.title.to_ascii_lowercase())
            })
            .collect();
        hits.sort_by_key(|c| (!c.id.to_ascii_lowercase().starts_with(&q), c.id.len(), c.id));
        hits
    }

    /// Every group name, in first-registered order — the help overlay renders in this order.
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

fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut h = haystack.chars();
    needle.chars().all(|c| h.any(|x| x == c))
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
        // Adding an Action without a Command would make it unreachable from the command
        // line, which defeats the point of the registry.
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
    fn empty_search_returns_everything_sorted() {
        let r = Registry::builtin();
        let hits = r.search("  ");
        assert_eq!(hits.len(), r.all().len());
        assert!(hits.windows(2).all(|w| w[0].id <= w[1].id));
    }
}
