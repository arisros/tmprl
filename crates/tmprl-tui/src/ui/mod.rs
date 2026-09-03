//! Rendering. Every function here is a pure projection of `&App` onto a frame — no state
//! is mutated except the viewport height, which the layout is what determines.

mod cmdline;
mod detail;
mod help;
mod history;
mod namespaces;
mod query;
mod statusline;
mod whichkey;
mod workflows;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::{App, PromptKind, Screen};
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    // The query bar is part of the workflow screen's chrome, not an overlay: it is always
    // on screen so the query is never something you have to go and open.
    let query_height = match app.screen {
        Screen::Workflows => 1,
        Screen::Namespaces | Screen::History => 0,
    };
    let [header, query_bar, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(query_height),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // The list needs to know how tall it is so that half-page motions mean something.
    app.page = body.height.saturating_sub(1) as usize;

    statusline::render_header(frame, header, app, &theme);
    match app.screen {
        Screen::Namespaces => namespaces::render(frame, body, app, &theme),
        Screen::Workflows => {
            query::render(frame, query_bar, app, &theme);
            workflows::render(frame, body, app, &theme);
        }
        Screen::History if app.show_detail => {
            // Roughly half each: enough list to keep your place, enough pane to read a
            // payload without scrolling for every value.
            let [list, pane] =
                Layout::vertical([Constraint::Min(3), Constraint::Percentage(50)]).areas(body);
            app.page = list.height.saturating_sub(1) as usize;
            history::render(frame, list, app, &theme);
            detail::render(frame, pane, app, &theme);
        }
        Screen::History => history::render(frame, body, app, &theme),
    }
    statusline::render_status(frame, status, app, &theme);

    // Overlays, outermost last.
    // Only `:` has completions to show; `!` takes a shell command and draws in the
    // statusline alone.
    if app
        .prompt
        .as_ref()
        .is_some_and(|p| p.kind == PromptKind::Command)
    {
        cmdline::render(frame, app, &theme);
    }
    if !app.which_key.is_empty() {
        whichkey::render(frame, app, &theme);
    }
    if app.show_help {
        help::render(frame, app, &theme);
    }
}

/// The hybrid relative/absolute gutter, matching `set relativenumber number`: the cursor
/// row shows its own 1-based index, every other row its distance. That is what makes a
/// count like `7j` something you read off the screen rather than estimate.
fn gutter(i: usize, cursor: usize) -> String {
    if i == cursor {
        format!("{:>4} ", i + 1)
    } else {
        format!("{:>4} ", i.abs_diff(cursor))
    }
}

/// Shorten to `max` characters, on a character boundary, with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// A centred box `w` x `h`, clamped to the frame.
fn centered(area: ratatui::layout::Rect, w: u16, h: u16) -> ratatui::layout::Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    ratatui::layout::Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tmprl_client::NamespaceInfo;
    use tmprl_core::{Chord, Loadable, StatusCounts, WorkflowList, WorkflowRow, WorkflowStatus};
    use tokio::sync::mpsc::unbounded_channel;

    fn ns(name: &str, days: i64) -> NamespaceInfo {
        NamespaceInfo {
            name: name.into(),
            state: "Registered".into(),
            retention_days: days,
            description: String::new(),
        }
    }

    /// An app with data but no connection. Rendering must be testable without a server —
    /// that is the whole point of keeping IO out of the render path.
    fn app_with_rows() -> App {
        let (tx, _rx) = unbounded_channel();
        let mut app = App::detached("prod", "default", tx);
        app.namespaces = Loadable::loaded(vec![
            ns("default", 24),
            ns("payments", 30),
            ns("temporal-system", 7),
        ]);
        app
    }

    fn draw(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn wf(ns: &str, id: &str, status: WorkflowStatus, start: i64) -> WorkflowRow {
        WorkflowRow {
            namespace: ns.into(),
            workflow_id: id.into(),
            run_id: format!("run-{id}"),
            workflow_type: "CheckoutWorkflow".into(),
            task_queue: "orders".into(),
            status,
            start_time: Some(start),
            close_time: None,
            history_length: 7,
        }
    }

    /// A workflow screen with rows, no connection. Start times are relative to now so the
    /// age column renders something stable.
    fn app_with_workflows(scope: &[&str]) -> App {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let mut app = app_with_rows();
        app.screen = crate::app::Screen::Workflows;
        app.scope = scope.iter().map(|s| s.to_string()).collect();

        let mut list = WorkflowList::default();
        list.reset(
            vec![
                wf(
                    "default",
                    "order-1001",
                    WorkflowStatus::Running,
                    now - 45_000,
                ),
                wf(
                    "payments",
                    "charge-77",
                    WorkflowStatus::Failed,
                    now - 7_200_000,
                ),
            ],
            vec![],
        );
        app.workflows = Loadable::loaded(list);
        app.counts = Loadable::loaded(StatusCounts::new(
            2,
            [(WorkflowStatus::Running, 1), (WorkflowStatus::Failed, 1)],
        ));
        app
    }

    /// A history screen with a workflow, a hidden workflow task, and two activities — the
    /// second of which failed after a retry.
    fn app_with_history() -> App {
        use tmprl_core::history::{
            Category as C, GroupRef as G, NormalizedEvent, Outcome as O, Role as R,
        };

        let e = |id: i64, g: G, r: R, c: C| {
            NormalizedEvent::new(id, "EVENT", c, g, r).with_time(Some(id * 1_000))
        };
        let mut started = e(5, G::Opened(4), R::Continues, C::Activity);
        started.attempt = Some(3);
        let mut failed = e(8, G::Opened(7), R::Closes, C::Activity).with_outcome(O::Failed);
        failed.failure = Some("card declined".into());

        let events = vec![
            e(1, G::Workflow, R::Opens, C::Workflow).with_subject("OrderWorkflow"),
            e(2, G::Opened(2), R::Opens, C::WorkflowTask),
            e(3, G::Opened(2), R::Closes, C::WorkflowTask),
            e(4, G::Opened(4), R::Opens, C::Activity).with_subject("ChargeCard"),
            started,
            e(6, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed),
            e(7, G::Opened(7), R::Opens, C::Activity).with_subject("ShipOrder"),
            failed,
        ];

        let mut app = app_with_workflows(&["default"]);
        app.screen = crate::app::Screen::History;
        app.viewing = Some(wf("default", "order-1001", WorkflowStatus::Running, 0));
        let groups = tmprl_core::history::group_events(&events);
        app.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));
        app
    }

    #[test]
    fn the_history_shows_one_row_per_group_not_per_event() {
        let mut app = app_with_history();
        let out = draw(&mut app, 110, 12);

        assert!(out.contains("ChargeCard"), "activity missing:\n{out}");
        assert!(out.contains("ShipOrder"), "activity missing:\n{out}");
        assert!(out.contains("activity"), "category missing:\n{out}");
        // Three groups on screen, not eight events: the body rows below the header.
        let rows = out
            .lines()
            .filter(|l| l.contains("activity") || l.contains("workflow "))
            .count();
        assert_eq!(rows, 3, "expected one row per group:\n{out}");
    }

    #[test]
    fn a_retried_group_shows_its_attempt_count_and_failure() {
        let mut app = app_with_history();
        let out = draw(&mut app, 110, 12);
        assert!(
            out.contains("×3"),
            "the retry count must be visible:\n{out}"
        );
        assert!(out.contains("card declined"), "failure missing:\n{out}");
    }

    #[test]
    fn folding_a_group_open_reveals_its_events_indented() {
        let mut app = app_with_history();
        app.run("motion.down", None); // the ChargeCard group
        app.run("history.fold", None);
        let out = draw(&mut app, 110, 14);

        // Event ids appear only once the group is unfolded.
        assert!(out.contains("EVENT"), "event rows missing:\n{out}");
        assert!(out.contains('▾'), "an open fold marker is expected:\n{out}");
    }

    #[test]
    fn the_history_header_names_the_workflow_and_counts_failures() {
        let mut app = app_with_history();
        let out = draw(&mut app, 110, 12);
        let header = out.lines().next().unwrap();
        assert!(header.contains("order-1001"), "workflow id missing:\n{out}");
        assert!(header.contains("failed"), "failure tally missing:\n{out}");
    }

    #[test]
    fn a_history_of_nothing_but_plumbing_says_so() {
        use tmprl_core::history::{Category as C, GroupRef as G, NormalizedEvent, Role as R};
        let events = vec![
            NormalizedEvent::new(1, "E", C::WorkflowTask, G::Opened(1), R::Opens),
            NormalizedEvent::new(2, "E", C::WorkflowTask, G::Opened(1), R::Closes),
        ];
        let groups = tmprl_core::history::group_events(&events);
        let mut app = app_with_history();
        app.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));

        let out = draw(&mut app, 110, 12);
        assert!(
            out.contains("nothing but workflow tasks"),
            "an empty pane would look broken:\n{out}"
        );
    }

    #[test]
    fn a_long_scope_never_runs_into_the_header_summary() {
        // A workflow id can be a UUID, and the history header shows it. Rendered at full
        // length it overwrites the right-hand tallies with no separator.
        let mut app = app_with_history();
        app.viewing = Some(wf(
            "default",
            "a24368a8-fcaf-4c19-bc07-0334f59ee9b1-and-then-some-more",
            WorkflowStatus::Running,
            0,
        ));
        for width in [60u16, 80, 110] {
            let out = draw(&mut app, width, 10);
            let header = out.lines().next().unwrap();
            assert!(
                header.contains("  ") || header.trim().is_empty(),
                "header should keep a gap at width {width}:\n{header}"
            );
            assert!(
                !header.contains("failed") || header.contains(" failed"),
                "the summary must not be run into by the scope at width {width}:\n{header}"
            );
        }
    }

    /// A history whose activity carries a JSON input and result.
    fn app_with_payloads() -> App {
        use tmprl_core::history::{
            Category as C, GroupRef as G, NormalizedEvent, Outcome as O, Role as R,
        };
        use tmprl_core::payload::Payload;

        let mut scheduled = NormalizedEvent::new(
            4,
            "ACTIVITY_TASK_SCHEDULED",
            C::Activity,
            G::Opened(4),
            R::Opens,
        )
        .with_time(Some(4_000))
        .with_subject("ChargeCard");
        scheduled.payloads.push((
            "input".into(),
            Payload::new("json/plain", br#"{"amount":100,"currency":"GBP"}"#.to_vec()),
        ));
        let mut completed = NormalizedEvent::new(
            6,
            "ACTIVITY_TASK_COMPLETED",
            C::Activity,
            G::Opened(4),
            R::Closes,
        )
        .with_time(Some(6_000))
        .with_outcome(O::Completed);
        completed.payloads.push((
            "result".into(),
            Payload::new("json/plain", b"\"charged\"".to_vec()),
        ));
        let mut secret = NormalizedEvent::new(
            7,
            "ACTIVITY_TASK_SCHEDULED",
            C::Activity,
            G::Opened(7),
            R::Opens,
        )
        .with_time(Some(7_000))
        .with_subject("Secret");
        secret.payloads.push((
            "input".into(),
            Payload::new("binary/encrypted", vec![0u8; 32]),
        ));

        let events = vec![
            NormalizedEvent::new(
                1,
                "WORKFLOW_EXECUTION_STARTED",
                C::Workflow,
                G::Workflow,
                R::Opens,
            )
            .with_time(Some(1_000))
            .with_subject("OrderWorkflow"),
            scheduled,
            completed,
            secret,
        ];
        let groups = tmprl_core::history::group_events(&events);
        let mut app = app_with_history();
        app.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));
        app
    }

    #[test]
    fn the_payload_pane_is_closed_until_asked_for() {
        let mut app = app_with_payloads();
        let out = draw(&mut app, 110, 20);
        assert!(
            !out.contains("payloads"),
            "the pane should start closed:\n{out}"
        );
        assert!(
            !out.contains("amount"),
            "no payload should be shown:\n{out}"
        );
    }

    #[test]
    fn the_payload_pane_shows_input_and_result_for_a_group() {
        // A group's arguments live on the event that opened it and its result on the event
        // that closed it, so the pane has to gather from both.
        let mut app = app_with_payloads();
        app.run("motion.down", None); // the ChargeCard group
        app.run("history.detail", None);

        let out = draw(&mut app, 110, 20);
        assert!(out.contains("payloads"), "pane missing:\n{out}");
        assert!(out.contains("input"), "input label missing:\n{out}");
        assert!(out.contains("result"), "result label missing:\n{out}");
        assert!(
            out.contains("\"amount\": 100"),
            "JSON should be pretty-printed:\n{out}"
        );
        assert!(out.contains("charged"), "result value missing:\n{out}");
    }

    #[test]
    fn an_encrypted_payload_says_it_needs_a_codec_rather_than_showing_bytes() {
        let mut app = app_with_payloads();
        app.run("motion.bottom", None); // the Secret group
        app.run("history.detail", None);

        let out = draw(&mut app, 110, 20);
        assert!(out.contains("encrypted"), "should be labelled:\n{out}");
        assert!(out.contains("codec"), "should say what is needed:\n{out}");
    }

    #[test]
    fn a_row_carrying_nothing_says_so() {
        // A pane showing only a title reads as broken; plenty of events carry no payload.
        let mut app = app_with_payloads();
        app.run("motion.top", None); // the workflow group, no payloads in this fixture
        app.run("history.detail", None);
        let out = draw(&mut app, 110, 20);
        assert!(out.contains("no payloads on this group"), "{out}");
    }

    #[test]
    fn a_tall_payload_scrolls_rather_than_clipping_silently() {
        use tmprl_core::payload::Payload;
        let mut app = app_with_payloads();
        // A deep value: taller than any pane on a normal terminal.
        let big: String = (0..60).map(|i| format!("\"k{i}\":{i},")).collect();
        let json = format!("{{{}\"last\":1}}", big);
        if let Some(o) = app.history.value_mut() {
            let mut events = o.events().to_vec();
            events[1].payloads = vec![(
                "input".into(),
                Payload::new("json/plain", json.into_bytes()),
            )];
            let groups = tmprl_core::history::group_events(&events);
            o.replace(events, groups);
        }
        app.run("motion.down", None);
        app.run("history.detail", None);

        let out = draw(&mut app, 110, 20);
        assert!(
            app.detail_max_scroll > 0,
            "the payload should overflow the pane"
        );
        assert!(
            out.contains("to scroll"),
            "an overflowing pane must say so:\n{out}"
        );

        // And it actually scrolls.
        let before = app.detail_scroll;
        app.run("history.detail-down", Some(5));
        assert!(app.detail_scroll > before);
    }

    #[test]
    fn moving_the_cursor_restarts_the_payload_pane_at_the_top() {
        use tmprl_core::payload::Payload;
        let mut app = app_with_payloads();
        // Only a payload taller than the pane can be scrolled at all.
        let big: String = (0..60).map(|i| format!("\"k{i}\":{i},")).collect();
        if let Some(o) = app.history.value_mut() {
            let mut events = o.events().to_vec();
            events[1].payloads = vec![(
                "input".into(),
                Payload::new("json/plain", format!("{{{big}\"last\":1}}").into_bytes()),
            )];
            let groups = tmprl_core::history::group_events(&events);
            o.replace(events, groups);
        }
        app.run("motion.down", None);
        app.run("history.detail", None);
        let _ = draw(&mut app, 110, 20); // the renderer is what learns how far it can scroll
        app.run("history.detail-down", Some(2));
        assert!(app.detail_scroll > 0);

        app.run("motion.down", None);
        assert_eq!(
            app.detail_scroll, 0,
            "a different value must be shown from its start"
        );
    }

    #[test]
    fn a_filter_result_replaces_the_payloads_in_the_pane() {
        // You asked to see the filtered value; showing it under the raw payloads would bury
        // the thing you asked for.
        let mut app = app_with_payloads();
        app.run("motion.down", None);
        app.run("history.detail", None);
        assert!(draw(&mut app, 110, 20).contains("amount"));

        app.handle(crate::app::Msg::Piped(Ok("\"charged\"".into())));
        let out = draw(&mut app, 110, 20);
        assert!(out.contains("filtered"), "pane should say so:\n{out}");
        assert!(out.contains("charged"), "output missing:\n{out}");
        assert!(
            !out.contains("amount"),
            "raw payloads should give way:\n{out}"
        );
    }

    #[test]
    fn a_failed_filter_shows_the_commands_own_message() {
        let mut app = app_with_payloads();
        app.run("motion.down", None);
        app.run("history.detail", None);
        app.handle(crate::app::Msg::Piped(Err(
            "jq: error: syntax error, unexpected INVALID_CHARACTER".into(),
        )));

        let out = draw(&mut app, 110, 20);
        assert!(out.contains("filter failed"), "{out}");
        assert!(out.contains("syntax error"), "jq's own diagnosis:\n{out}");
    }

    #[test]
    fn a_filter_with_no_output_says_so_rather_than_looking_broken() {
        let mut app = app_with_payloads();
        app.run("motion.down", None);
        app.run("history.detail", None);
        app.handle(crate::app::Msg::Piped(Ok(String::new())));
        assert!(draw(&mut app, 110, 20).contains("no output"));
    }

    #[test]
    fn the_pipe_prompt_is_drawn_with_its_own_sigil() {
        let mut app = app_with_payloads();
        app.run("motion.down", None);
        app.run("payload.pipe", None);

        let out = draw(&mut app, 110, 20);
        assert!(out.contains("!jq ."), "the ! prompt should show:\n{out}");
        // `:` completions have no business appearing over a shell command.
        assert!(
            !out.contains("app.quit"),
            "a pipe prompt must not offer command completions:\n{out}"
        );
    }

    #[test]
    fn following_is_announced_in_the_statusline() {
        // A view that rewrites itself while you read it must say so, or a changing screen
        // reads as a glitch.
        let mut app = app_with_history();
        assert!(!draw(&mut app, 110, 12).contains("FOLLOW"));

        app.following = true;
        let out = draw(&mut app, 110, 12);
        assert!(out.contains("FOLLOW"), "follow indicator missing:\n{out}");
        assert!(out.contains("NORMAL"), "the mode is still shown:\n{out}");
    }

    #[test]
    fn the_history_screen_renders_at_a_cramped_size() {
        let mut app = app_with_history();
        let _ = draw(&mut app, 20, 4);
        let _ = draw(&mut app, 8, 3);
        app.run("history.expand-all", None);
        let _ = draw(&mut app, 20, 4);
    }

    #[test]
    fn truncation_respects_character_boundaries() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
        // Multi-byte input must not be sliced mid-character.
        assert_eq!(truncate("日本語テスト", 3), "日本…");
    }

    #[test]
    fn the_gutter_is_absolute_on_the_cursor_and_relative_elsewhere() {
        assert_eq!(
            gutter(4, 4).trim(),
            "5",
            "cursor row shows its 1-based index"
        );
        assert_eq!(gutter(1, 4).trim(), "3");
        assert_eq!(gutter(7, 4).trim(), "3");
    }

    #[test]
    fn the_workflow_table_shows_status_id_type_and_age() {
        let mut app = app_with_workflows(&["default"]);
        let out = draw(&mut app, 110, 12);
        assert!(out.contains("order-1001"), "workflow id missing:\n{out}");
        assert!(out.contains("CheckoutWorkflow"), "type missing:\n{out}");
        assert!(out.contains("Running"), "status missing:\n{out}");
        assert!(out.contains("Failed"), "status missing:\n{out}");
        // Status is legible without colour: the glyph carries it too.
        assert!(
            out.contains(WorkflowStatus::Running.glyph()),
            "status glyph missing:\n{out}"
        );
        assert!(out.contains("45s"), "age column missing:\n{out}");
        assert!(out.contains("2h"), "age column missing:\n{out}");
    }

    #[test]
    fn the_query_bar_is_always_on_screen() {
        // The raw query is the interface; it is never behind a keystroke to reveal.
        let mut app = app_with_workflows(&["default"]);
        assert!(
            draw(&mut app, 110, 12).contains("query"),
            "query bar must be visible with an empty query"
        );

        app.query = "ExecutionStatus = 'Failed'".into();
        let out = draw(&mut app, 110, 12);
        assert!(
            out.contains("ExecutionStatus = 'Failed'"),
            "the raw query text must be shown verbatim:\n{out}"
        );
    }

    #[test]
    fn editing_the_query_shows_the_live_text_not_the_applied_one() {
        let mut app = app_with_workflows(&["default"]);
        app.query = "A = 1".into();
        app.run("mode.insert", None);
        for c in "23".chars() {
            app.handle(crate::app::Msg::Key(Chord::ch(c)));
        }
        let out = draw(&mut app, 110, 12);
        assert!(out.contains("A = 123"), "live edit missing:\n{out}");
        assert!(out.contains("INSERT"), "mode should be INSERT:\n{out}");
    }

    #[test]
    fn the_header_shows_per_status_counts() {
        let mut app = app_with_workflows(&["default"]);
        let out = draw(&mut app, 110, 12);
        let header = out.lines().next().unwrap();
        assert!(header.contains("total"), "count total missing:\n{out}");
        assert!(
            header.contains(WorkflowStatus::Failed.glyph()),
            "the header should tally failures with the table's glyph:\n{out}"
        );
    }

    #[test]
    fn the_namespace_column_appears_only_in_a_fan_out() {
        // On one namespace it would be the same value on every row.
        let mut single = app_with_workflows(&["default"]);
        let out = draw(&mut single, 110, 12);
        let body: String = out.lines().skip(2).collect::<Vec<_>>().join("\n");
        assert!(
            !body.contains("payments"),
            "a single-namespace list should not carry a namespace column:\n{out}"
        );

        let mut fanned = app_with_workflows(&["default", "payments"]);
        let out = draw(&mut fanned, 110, 12);
        let body: String = out.lines().skip(2).collect::<Vec<_>>().join("\n");
        assert!(
            body.contains("payments"),
            "a fan-out must tag rows with their namespace:\n{out}"
        );
    }

    #[test]
    fn an_empty_result_distinguishes_no_data_from_a_filter() {
        let mut app = app_with_workflows(&["default"]);
        app.workflows = Loadable::loaded(WorkflowList::default());

        let out = draw(&mut app, 110, 12);
        assert!(out.contains("no workflows in this namespace"), "{out}");

        app.query = "ExecutionStatus = 'Failed'".into();
        let out = draw(&mut app, 110, 12);
        assert!(
            out.contains("no workflows match this query"),
            "an empty filtered list must say the filter is why:\n{out}"
        );
    }

    #[test]
    fn the_workflow_screen_renders_at_a_cramped_size() {
        let mut app = app_with_workflows(&["default", "payments"]);
        let _ = draw(&mut app, 20, 4);
        let _ = draw(&mut app, 8, 3);
        app.show_help = true;
        let _ = draw(&mut app, 20, 4);
    }

    #[test]
    fn header_shows_profile_and_namespace() {
        let mut app = app_with_rows();
        let out = draw(&mut app, 90, 12);
        assert!(out.contains("prod"), "profile missing:\n{out}");
        assert!(out.contains("default"), "namespace missing:\n{out}");
        assert!(out.contains('3'), "row count missing:\n{out}");
    }

    #[test]
    fn statusline_shows_the_mode() {
        let mut app = app_with_rows();
        assert!(draw(&mut app, 90, 12).contains("NORMAL"));
        app.run("mode.insert", None);
        assert!(draw(&mut app, 90, 12).contains("INSERT"));
        app.run("mode.visual", None);
        assert!(draw(&mut app, 90, 12).contains("VISUAL"));
    }

    #[test]
    fn rows_are_listed_with_a_hybrid_relative_gutter() {
        let mut app = app_with_rows();
        app.cursor = 1;
        let out = draw(&mut app, 90, 12);
        assert!(out.contains("payments"), "rows missing:\n{out}");

        // Skip the header, which also mentions the namespace by name.
        let body: Vec<&str> = out.lines().skip(1).collect();
        let gutter = |needle: &str| -> String {
            body.iter()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("no row for {needle}:\n{out}"))
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect()
        };
        // The cursor row shows its own 1-based index; the others show distance.
        assert_eq!(
            gutter("payments"),
            "2",
            "absolute number on cursor row:\n{out}"
        );
        assert_eq!(gutter("default"), "1", "distance one row above:\n{out}");
        assert_eq!(
            gutter("temporal-system"),
            "1",
            "distance one row below:\n{out}"
        );
    }

    #[test]
    fn which_key_popup_appears_after_the_leader() {
        let mut app = app_with_rows();
        app.handle(crate::app::Msg::Key(Chord::ch(' ')));
        let out = draw(&mut app, 90, 14);
        assert!(
            !app.which_key.is_empty(),
            "leader should open a pending state"
        );
        assert!(
            out.contains("Quit"),
            "which-key should list <leader>q:\n{out}"
        );
    }

    #[test]
    fn help_overlay_lists_command_groups() {
        let mut app = app_with_rows();
        app.run("app.help", None);
        let out = draw(&mut app, 90, 24);
        assert!(out.contains("Motion"), "group heading missing:\n{out}");
        assert!(out.contains("Quit"), "command title missing:\n{out}");
        // Bindings are rendered from the keymap, not written by hand.
        assert!(out.contains("<Space>q"), "leader binding missing:\n{out}");
        assert!(out.contains("jk"), "insert-escape binding missing:\n{out}");
        // Ids must not be truncated — they are what `:` and keys.toml consume.
        assert!(out.contains("app.command-line"), "id truncated:\n{out}");
        assert!(out.contains("mode.visual-line"), "id truncated:\n{out}");
    }

    #[test]
    fn the_help_overlay_scrolls_instead_of_clipping_silently() {
        // Adding commands must never push a group off the bottom with no sign of it —
        // that is how a reader concludes a command does not exist.
        let mut app = app_with_rows();
        app.run("app.help", None);

        let out = draw(&mut app, 90, 16);
        assert!(
            app.help_max_scroll > 0,
            "the overlay should overflow at 16 rows"
        );
        assert!(
            out.contains("j/k to scroll"),
            "an overflowing overlay must say so:\n{out}"
        );
        assert!(out.contains("Application"), "first group missing:\n{out}");

        // The last group is reachable by scrolling. Whatever group is registered last, the
        // point is that adding commands must not push one off the bottom unreachably.
        app.run("motion.bottom", None);
        let out = draw(&mut app, 90, 16);
        let last_group = app.registry.groups().last().copied().unwrap();
        let last_id = app
            .registry
            .all()
            .iter()
            .rfind(|c| c.group == last_group)
            .unwrap()
            .id;
        assert!(
            out.contains(last_id),
            "the last command ({last_id}) must be reachable:\n{out}"
        );
    }

    #[test]
    fn a_long_binding_list_never_runs_into_the_title_column() {
        // Three bindings on one command is ordinary once keys.toml adds to the defaults.
        let mut app = app_with_rows();
        app.apply_config(Some("[normal]\n\"ZZ\" = \"app.quit\"\n"), None);
        app.run("app.help", None);

        let out = draw(&mut app, 90, 60);
        let line = out
            .lines()
            .find(|l| l.contains("app.quit"))
            .unwrap_or_else(|| panic!("no app.quit row:\n{out}"));
        assert!(
            line.contains("ZZ") && line.contains("Quit"),
            "both should render:\n{line}"
        );
        assert!(
            !line.contains("ZZQuit"),
            "the key column must not collide with the title:\n{line}"
        );
    }

    #[test]
    fn a_help_overlay_that_fits_says_nothing_about_scrolling() {
        let mut app = app_with_rows();
        app.run("app.help", None);
        let out = draw(&mut app, 90, 60);
        assert_eq!(app.help_max_scroll, 0);
        assert!(!out.contains("j/k to scroll"), "{out}");
        assert!(
            out.contains("nav.open") && out.contains("yank.record"),
            "{out}"
        );
    }

    #[test]
    fn motions_move_the_cursor_again_once_help_is_closed() {
        let mut app = app_with_rows();
        app.run("app.help", None);
        app.run("motion.down", None);
        assert_eq!(
            app.cursor, 0,
            "help was open; the list cursor must not move"
        );

        app.run("app.cancel", None);
        app.run("motion.down", None);
        assert_eq!(app.cursor, 1);
        assert_eq!(app.help_scroll, 0, "closing help resets its scroll");
    }

    #[test]
    fn command_line_shows_completions() {
        let mut app = app_with_rows();
        app.run("app.command-line", None);
        for c in "motion".chars() {
            app.handle(crate::app::Msg::Key(Chord::ch(c)));
        }
        let out = draw(&mut app, 90, 16);
        assert!(out.contains(":motion"), "command line missing:\n{out}");
        assert!(out.contains("motion.down"), "completions missing:\n{out}");
    }

    #[test]
    fn empty_state_renders_without_panicking() {
        let (tx, _rx) = unbounded_channel();
        let mut app = App::detached("p", "n", tx);
        let out = draw(&mut app, 80, 10);
        assert!(out.contains("NORMAL"));
    }

    #[test]
    fn renders_at_a_cramped_terminal_size() {
        // A layout that assumes room is a layout that panics on someone's split pane.
        let mut app = app_with_rows();
        app.show_help = true;
        let _ = draw(&mut app, 20, 4);
        app.show_help = false;
        app.handle(crate::app::Msg::Key(Chord::ch(' ')));
        let _ = draw(&mut app, 20, 4);
    }
}
