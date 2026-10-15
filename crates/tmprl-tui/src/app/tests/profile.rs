//! Switching connection profile: the list, the picker, and starting over on the new one.

use super::*;
use tmprl_core::fault::Code;

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

fn on_the_profile_list() -> App {
    let mut app = app();
    app.handle(Msg::Profiles(Ok(profiles())));
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Profiles);
    app
}

#[test]
fn only_a_profile_that_takes_the_environment_is_overridden_by_it() {
    use crate::app::profile::env_override;
    let none = |_: &str| false;
    let address = |v: &str| v == "TEMPORAL_ADDRESS";
    let namespace = |v: &str| v == "TEMPORAL_NAMESPACE";
    let both = |_: &str| true;
    assert_eq!(env_override(true, none), None);
    assert_eq!(env_override(true, address), Some("TEMPORAL_ADDRESS"));
    assert_eq!(env_override(true, namespace), Some("TEMPORAL_NAMESPACE"));
    assert_eq!(
        env_override(true, both),
        Some("TEMPORAL_ADDRESS"),
        "the address is the one that sends you to another cluster"
    );
    assert_eq!(env_override(false, both), None, "env = false turns it off");
}

#[test]
fn a_switch_drops_what_belonged_to_the_old_cluster() {
    let mut app = app();
    four(&mut app);
    app.view.screen = Screen::Workflows;
    app.run("find.command", None);
    assert!(app.picker.is_some());
    app.mode = Mode::Visual;
    app.attributes_for = Some("default".into());
    app.search_attributes = Loadable::Loading;

    adopt_dev(&mut app);
    assert!(app.picker.is_none());
    assert_eq!(app.mode, Mode::Normal);
    assert!(
        app.attributes_for.is_none(),
        "prod's search attributes are not dev's"
    );
    assert!(app.search_attributes.value().is_none());
    assert!(!app.search_attributes.is_loading());
}

#[test]
fn a_switch_keeps_the_profile_list_it_was_made_from() {
    let mut app = on_the_profile_list();
    adopt_dev(&mut app);
    assert_eq!(app.profile_rows().len(), 3);
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Profiles);
    assert_eq!(app.profile_rows()[app.view.cursor].name, "dev");
}

#[test]
fn a_profile_list_read_before_a_switch_is_dropped() {
    let mut app = app();
    let stale = app.epoch();
    adopt_dev(&mut app);
    app.handle(Msg::Reply {
        epoch: stale,
        msg: Box::new(Msg::Profiles(Ok(profiles()))),
    });
    assert!(app.view.profiles.value().is_none());
}

#[test]
fn a_mutation_on_the_current_connection_is_reported_as_usual() {
    let mut app = app();
    app.handle(Msg::Reply {
        epoch: app.epoch(),
        msg: Box::new(Msg::Mutated {
            mutation: Box::new(Mutation::Cancel {
                namespace: "default".into(),
                workflow_id: "order-r1".into(),
                run_id: "r1".into(),
            }),
            sent_to: ("prod".into(), "http://detached".into()),
            result: Ok(()),
            batch: None,
        }),
    });
    assert_eq!(app.note.as_ref().unwrap().0, "cancelled order-r1");
    let AuditSink::Captured(lines) = &app.audit_sink else {
        panic!("a detached app must never write the user's audit log");
    };
    assert_eq!(lines.len(), 1);
}

#[test]
fn a_connection_nobody_is_waiting_for_is_ignored() {
    let mut app = app();
    app.on_connected(7, Err("late".into()));
    assert!(app.note.is_none());
    assert_eq!(app.profile(), "prod");
}

#[test]
fn picking_the_connected_profile_says_so_and_connects_nothing() {
    let mut app = app();
    app.switch_profile("prod");
    assert_eq!(app.note.as_ref().unwrap().0, "already on prod");
    assert!(app.switching.is_none());
    assert_eq!(app.view.screen, Screen::Namespaces);
}

#[test]
fn a_second_switch_to_the_connected_profile_cancels_the_first() {
    let mut app = app();
    app.switch_profile("dev");
    app.switch_profile("prod");
    assert!(app.switching.is_none());
    app.on_connected(1, Err("unreachable".into()));
    assert_eq!(app.note.as_ref().unwrap().0, "already on prod");
}

#[test]
fn a_profile_list_that_cannot_be_read_is_reported_and_the_picker_given_up() {
    let mut app = app();
    app.run("find.profile", None);
    app.handle(Msg::Profiles(Err(Fault {
        operation: "",
        code: Code::Local,
        message: "expected `=`".into(),
    })));
    assert!(app.picker.is_none());
    assert!(!app.profile_picker_pending);
    assert!(app.view.profiles.error().is_some());
    let (note, level) = app.note.clone().unwrap();
    assert_eq!(level, Note::Error);
    assert!(note.contains("expected `=`"), "{note}");
}

#[test]
fn the_picker_marks_the_connected_profile_and_previews_readonly() {
    let mut app = app();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true\n"));
    app.handle(Msg::Profiles(Ok(profiles())));
    let items = app.profile_items();
    let prod = items.iter().find(|i| i.label == "prod").unwrap();
    assert_eq!(prod.note, "connected");
    assert!(prod.preview.contains("readonly    yes"), "{}", prod.preview);
    assert!(
        prod.preview.contains("prod.example:7233"),
        "{}",
        prod.preview
    );
    let dev = items.iter().find(|i| i.label == "dev").unwrap();
    assert_eq!(dev.note, "");
    assert!(dev.preview.contains("readonly    no"), "{}", dev.preview);
    assert!(
        dev.preview.contains("namespace   default"),
        "{}",
        dev.preview
    );
}

#[test]
fn the_picker_opens_at_once_when_the_list_is_already_loaded() {
    let mut app = app();
    app.handle(Msg::Profiles(Ok(profiles())));
    app.run("find.profile", None);
    assert!(app.picker.is_some());
    assert!(!app.profile_picker_pending);
}

#[test]
fn yank_on_the_profile_list_takes_the_name_or_the_record() {
    let app = on_the_profile_list();
    assert_eq!(app.field_under_cursor(), "prod");
    let record = app.records_selected();
    assert!(record.contains(r#""name":"prod""#), "{record}");
    assert!(
        record.contains(r#""address":"prod.example:7233""#),
        "{record}"
    );
}

#[test]
fn search_on_the_profile_list_matches_the_address() {
    let mut app = on_the_profile_list();
    search_for(&mut app, "staging.example");
    assert_eq!(app.profile_rows()[app.view.cursor].name, "staging");
}

#[test]
fn the_profile_list_has_nothing_to_mutate_or_draw_a_dashboard_of() {
    let mut app = on_the_profile_list();
    assert!(app.target_workflow().is_none());
    app.run("nav.dashboard", None);
    assert_eq!(app.view.screen, Screen::Profiles);
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn a_split_of_the_profile_list_is_another_profile_list() {
    let mut app = on_the_profile_list();
    app.run("window.split-right", None);
    assert_eq!(app.view.screen, Screen::Profiles);
    assert_eq!(app.tabs.current().len(), 2);
}
