//! Switching connection profile: the list, the picker, and starting over on the new one.

use super::*;

fn profiles() -> Vec<ProfileSummary> {
    ["dev", "prod", "staging"]
        .into_iter()
        .map(|name| ProfileSummary {
            name: name.into(),
            address: Some(format!("{name}.example:7233")),
            namespace: None,
        })
        .collect()
}

fn adopt_dev(app: &mut App) {
    app.adopt(
        "dev".into(),
        "http://dev.example:7233/".into(),
        "orders".into(),
        None,
    );
}

#[test]
fn a_reply_from_the_connection_switched_away_from_is_dropped() {
    let mut app = app();
    let stale = app.epoch();
    adopt_dev(&mut app);
    app.handle(Msg::Reply {
        epoch: stale,
        msg: Box::new(Msg::Namespaces(Ok(vec![NamespaceInfo {
            name: "prod-only".into(),
            state: "Registered".into(),
            retention_days: 3,
            description: String::new(),
        }]))),
    });
    assert!(
        app.namespace_rows().is_empty(),
        "prod's namespaces landed on dev"
    );

    app.handle(Msg::Reply {
        epoch: app.epoch(),
        msg: Box::new(Msg::Namespaces(Ok(Vec::new()))),
    });
    assert!(
        app.view.namespaces.value().is_some(),
        "a current reply is applied"
    );
}

#[test]
fn a_mutation_that_lands_after_a_switch_is_audited_where_it_was_sent() {
    let mut app = app();
    let stale = app.epoch();
    adopt_dev(&mut app);
    app.handle(Msg::Reply {
        epoch: stale,
        msg: Box::new(Msg::Mutated {
            mutation: Box::new(Mutation::Cancel {
                namespace: "default".into(),
                workflow_id: "order-r1".into(),
                run_id: "r1".into(),
            }),
            sent_to: ("prod".into(), "http://prod.example:7233/".into()),
            result: Ok(()),
            batch: None,
        }),
    });
    let AuditSink::Captured(lines) = &app.audit_sink else {
        panic!("a detached app must never write the user's audit log");
    };
    assert_eq!(lines.len(), 1, "a stale mutation is still recorded");
    assert!(lines[0].contains("\"profile\":\"prod\""), "{}", lines[0]);
    assert!(!lines[0].contains("\"profile\":\"dev\""), "{}", lines[0]);
}

#[test]
fn a_switch_starts_the_session_over_on_one_pane() {
    let mut app = app();
    four(&mut app);
    app.view.screen = Screen::Workflows;
    app.run("window.split-right", None);
    assert_eq!(app.tabs.current().len(), 2);

    adopt_dev(&mut app);
    assert_eq!(app.profile(), "dev");
    assert_eq!(app.namespace(), "orders");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs.current().len(), 1);
    assert!(app.parked.is_empty());
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert!(app.workflow_rows().is_empty());
    assert_eq!(
        app.jumps.len(),
        0,
        "every jump pointed into the old cluster"
    );
}

#[test]
fn a_switch_takes_the_new_profiles_settings_and_keeps_the_readonly_flag() {
    let mut app = app();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true\n"));
    assert!(app.readonly());

    adopt_dev(&mut app);
    assert!(!app.readonly(), "dev has no readonly of its own");

    app.adopt("prod".into(), "x".into(), "default".into(), None);
    assert!(app.readonly());

    let mut flagged = super::app();
    flagged.readonly_flag = true;
    adopt_dev(&mut flagged);
    assert!(flagged.readonly(), "--readonly holds for the whole session");
}

#[test]
fn only_the_last_switch_asked_for_is_taken() {
    let mut app = app();
    app.switch_profile("dev");
    app.switch_profile("staging");
    app.on_connected(1, Err("unreachable".into()));
    assert_eq!(app.note.as_ref().unwrap().0, "connecting to staging…");

    app.on_connected(2, Err("unreachable".into()));
    let (note, level) = app.note.clone().unwrap();
    assert_eq!(level, Note::Error);
    assert!(note.contains("still on prod"), "{note}");
    assert_eq!(
        app.profile(),
        "prod",
        "a failed connect leaves you where you were"
    );
}

#[test]
fn enter_on_the_connected_profile_goes_back_to_its_namespaces() {
    let mut app = app();
    app.handle(Msg::Profiles(Ok(profiles())));
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Profiles);
    assert_eq!(app.profile_rows()[app.view.cursor].name, "prod");

    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert!(app.switching.is_none());
}

#[test]
fn the_cursor_lands_on_the_connected_profile_when_the_list_arrives_late() {
    let mut app = app();
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Profiles);
    app.handle(Msg::Profiles(Ok(profiles())));
    assert_eq!(app.profile_rows()[app.view.cursor].name, "prod");
}

#[test]
fn the_connected_profile_is_listed_even_when_no_file_names_it() {
    let mut app = app();
    app.handle(Msg::Profiles(Ok(Vec::new())));
    let rows = app.profile_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "prod");
}

#[test]
fn the_profile_picker_waits_for_the_list_then_opens() {
    let mut app = app();
    app.run("find.profile", None);
    assert!(app.picker.is_none());

    app.handle(Msg::Profiles(Ok(profiles())));
    let picker = app.picker.as_mut().expect("opened once the list arrived");
    assert_eq!(picker.kind.title(), "profiles");
    assert_eq!(picker.total(), 3);

    for c in "stag".chars() {
        picker.push(c);
    }
    app.accept_picker();
    assert_eq!(app.note.as_ref().unwrap().0, "connecting to staging…");
}
