//! Where the command line opens: `-n`, `-q`, `-w` and `--readonly`.

use super::*;

fn started(startup: Startup) -> App {
    let mut app = app();
    app.start(startup);
    app
}

/// An app started with `-w order-r1`, and the lookup still out.
fn looking_up() -> App {
    started(Startup {
        workflow: Some("order-r1".into()),
        ..Default::default()
    })
}

fn found(app: &mut App, result: Result<Vec<WorkflowRow>, Fault>) {
    app.handle(Msg::StartupWorkflow {
        generation: app.view.generation,
        id: "order-r1".into(),
        result,
    });
}

/// A run of `order-r1`, as the lookup returns several of for one workflow id.
fn run_of(run: &str, start: i64) -> WorkflowRow {
    let mut row = wf("default", run, start);
    row.workflow_id = "order-r1".into();
    row
}

#[test]
fn no_flags_opens_on_the_namespace_list() {
    let app = started(Startup::default());
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert!(!app.readonly());
}

#[test]
fn a_namespace_opens_on_its_workflow_list() {
    let app = started(Startup {
        namespace: Some("orders".into()),
        ..Default::default()
    });
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(app.view.scope, ["orders"]);
    assert_eq!(app.namespace(), "orders", "it replaces the profile's");
    assert!(app.view.workflows.is_loading(), "the list was asked for");
}

#[test]
fn a_query_opens_applied_in_the_profiles_namespace() {
    let app = started(Startup {
        query: Some("ExecutionStatus = 'Failed'".into()),
        ..Default::default()
    });
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(app.view.scope, ["default"]);
    assert_eq!(app.query_display(), "ExecutionStatus = 'Failed'");
    assert_eq!(app.mode, Mode::Normal, "applied, not left being edited");
    assert!(app.view.workflows.is_loading());
}

#[test]
fn dash_from_a_startup_list_goes_up_to_the_namespaces() {
    let mut app = started(Startup {
        namespace: Some("orders".into()),
        ..Default::default()
    });
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
}

#[test]
fn a_workflow_opens_its_history() {
    let mut app = looking_up();
    assert_eq!(app.view.screen, Screen::Workflows);
    assert!(app.view.workflows.is_loading(), "not an empty namespace");

    found(&mut app, Ok(vec![run_of("r1", 100)]));
    assert_eq!(app.view.screen, Screen::History);
    assert_eq!(app.view.viewing.as_ref().unwrap().run_id, "r1");
    assert!(app.note.is_none(), "one match needs no comment");
}

#[test]
fn several_runs_open_the_newest_and_say_so() {
    let mut app = looking_up();
    // Out of order, as a fan-out across visibility pages can be.
    found(
        &mut app,
        Ok(vec![
            run_of("r1", 100),
            run_of("r3", 300),
            run_of("r2", 200),
        ]),
    );
    assert_eq!(app.view.viewing.as_ref().unwrap().run_id, "r3");
    let (text, level) = app.note.clone().expect("the choice should be reported");
    assert!(text.contains("3 runs"), "{text}");
    assert_eq!(level, Note::Info);
}

#[test]
fn a_workflow_is_looked_up_in_the_namespace_given_with_it() {
    let app = started(Startup {
        namespace: Some("orders".into()),
        workflow: Some("order-r1".into()),
        ..Default::default()
    });
    assert_eq!(app.view.scope, ["orders"]);
}

#[test]
fn a_workflow_nobody_has_lands_on_the_list_and_says_so() {
    let mut app = looking_up();
    let before = app.view.generation;
    found(&mut app, Ok(Vec::new()));

    assert_eq!(app.view.screen, Screen::Workflows);
    assert_ne!(app.view.generation, before, "the list was asked for");
    let (text, level) = app.note.clone().expect("a miss should be reported");
    assert!(
        text.contains("order-r1"),
        "names what was asked for: {text}"
    );
    assert!(text.contains("default"), "and where it looked: {text}");
    assert_eq!(level, Note::Warn);
}

#[test]
fn a_failed_lookup_lands_on_the_list_with_the_error() {
    let mut app = looking_up();
    found(&mut app, Err("deadline exceeded".into()));

    assert_eq!(app.view.screen, Screen::Workflows);
    let (text, level) = app.note.clone().unwrap();
    assert!(text.contains("deadline exceeded"), "{text}");
    assert_eq!(level, Note::Error);
}

#[test]
fn a_query_given_with_a_workflow_is_what_the_fallback_list_shows() {
    let mut app = started(Startup {
        query: Some("WorkflowType = 'Checkout'".into()),
        workflow: Some("order-r1".into()),
        ..Default::default()
    });
    found(&mut app, Ok(Vec::new()));
    assert_eq!(app.query_display(), "WorkflowType = 'Checkout'");
}

#[test]
fn a_lookup_that_arrives_after_the_reader_moved_on_is_dropped() {
    // Went up to the namespaces while it was out.
    let mut app = looking_up();
    app.run("nav.up", None);
    found(&mut app, Ok(vec![run_of("r1", 100)]));
    assert_eq!(app.view.screen, Screen::Namespaces);

    // Asked for something else: the reply carries the generation it was issued under.
    let mut app = looking_up();
    let issued = app.view.generation;
    app.run("list.problems", None);
    app.handle(Msg::StartupWorkflow {
        generation: issued,
        id: "order-r1".into(),
        result: Ok(vec![run_of("r1", 100)]),
    });
    assert_eq!(app.view.screen, Screen::Workflows);
    assert!(app.view.viewing.is_none());
}

#[test]
fn dash_from_a_startup_history_fetches_the_list_it_never_had() {
    let mut app = looking_up();
    found(&mut app, Ok(vec![run_of("r1", 100)]));
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((history_events(), Vec::new())),
    });

    let before = app.view.generation;
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_ne!(app.view.generation, before, "the list was asked for");
    assert!(app.view.workflows.is_loading());
}

#[test]
fn dash_from_a_history_opened_off_a_list_does_not_refetch_it() {
    let mut app = viewing_history();
    let before = app.view.generation;
    app.run("nav.up", None);
    assert_eq!(app.view.generation, before);
    assert_eq!(app.workflow_rows().len(), 1);
}

#[test]
fn the_readonly_flag_refuses_and_names_the_flag() {
    let mut app = on_a_workflow();
    app.start(Startup {
        readonly: true,
        ..Default::default()
    });
    assert!(app.readonly());

    app.run("workflow.terminate", None);
    assert!(app.confirm.is_none(), "no confirmation may open");
    let (text, level) = app.note.clone().expect("a refusal should be reported");
    assert!(text.contains("--readonly"), "names the cause: {text}");
    assert!(
        !text.contains("profile"),
        "the profile did not ask for this: {text}"
    );
    assert_eq!(level, Note::Warn);
}

#[test]
fn the_readonly_flag_holds_whatever_the_config_says() {
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = false"));
    app.start(Startup {
        readonly: true,
        ..Default::default()
    });
    assert!(app.readonly());

    app.run_mutations(vec![Mutation::Terminate {
        namespace: "default".into(),
        workflow_id: "order-r1".into(),
        run_id: "r1".into(),
        reason: "x".into(),
    }]);
    let (text, _) = app.note.clone().expect("refused at the wire too");
    assert!(text.contains("read-only"), "{text}");
}

#[test]
fn a_readonly_profile_is_named_even_when_the_flag_is_also_given() {
    // Dropping the flag would not make this run writable, so the flag is not the answer.
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));
    app.start(Startup {
        readonly: true,
        ..Default::default()
    });

    app.run("workflow.terminate", None);
    let (text, _) = app.note.clone().unwrap();
    assert!(text.contains("profile prod"), "{text}");
}

#[test]
fn the_dashboard_flag_opens_on_the_dashboard() {
    let app = started(Startup {
        dashboard: true,
        ..Default::default()
    });
    assert_eq!(app.view.screen, Screen::Dashboard);
    assert_eq!(app.view.scope, ["default"]);
    assert!(app.view.dashboard.is_some());
}

#[test]
fn the_dashboard_flag_takes_the_namespace_it_is_given() {
    let mut app = started(Startup {
        dashboard: true,
        namespace: Some("orders".into()),
        ..Default::default()
    });
    assert_eq!(app.view.scope, ["orders"]);
    assert_eq!(app.view.dashboard.as_ref().unwrap().scope(), ["orders"]);

    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
}
