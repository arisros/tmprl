//! The schedule list.

use super::*;

#[test]
fn the_schedule_list_shows_state_spec_and_next_run() {
    use tmprl_core::ScheduleRow;
    let mut app = app_with_rows();
    app.view.screen = crate::app::Screen::Schedules;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    app.view.schedules = Loadable::loaded(vec![
        ScheduleRow {
            namespace: "default".into(),
            schedule_id: "nightly-recon".into(),
            workflow_type: "Reconcile".into(),
            paused: false,
            notes: String::new(),
            spec: "0 2 * * *".into(),
            next_run: Some(now + 3_600_000),
            recent_runs: 2,
        },
        ScheduleRow {
            namespace: "default".into(),
            schedule_id: "held".into(),
            workflow_type: "Reconcile".into(),
            paused: true,
            notes: String::new(),
            spec: "every 1h".into(),
            next_run: Some(now + 60_000),
            recent_runs: 0,
        },
    ]);

    let out = draw(&mut app, 110, 12);
    assert!(out.contains("nightly-recon"), "{out}");
    assert!(
        out.contains("0 2 * * *"),
        "the spec should read as cron:\n{out}"
    );
    assert!(out.contains("1h"), "next run missing:\n{out}");
    assert!(out.contains("paused"), "{out}");
    assert!(out.lines().next().unwrap().contains("1 paused"), "{out}");

    // A paused schedule still has future times, since the server computes them from the
    // spec. Showing one would suggest it is about to run.
    let held = out.lines().find(|l| l.contains("held")).unwrap();
    assert!(
        !held.contains("1m"),
        "a paused row should not count down:\n{held}"
    );
}

#[test]
fn an_empty_schedule_list_points_at_the_other_list() {
    let mut app = app_with_rows();
    app.view.screen = crate::app::Screen::Schedules;
    app.view.schedules = Loadable::loaded(Vec::new());
    assert!(draw(&mut app, 110, 12).contains("no schedules in this namespace"));
}
