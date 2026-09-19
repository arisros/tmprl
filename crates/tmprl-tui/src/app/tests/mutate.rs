//! Mutations and schedules: targets, confirmations, the readonly guard.

use super::*;

#[test]
fn a_mutation_key_only_opens_a_confirmation() {
    // Nothing reaches the cluster until the reader says yes.
    let mut app = on_a_workflow();
    app.run("workflow.terminate", None);

    let c = app.confirm.clone().expect("a confirmation should open");
    assert_eq!(c.first().verb(), "Terminate");
    assert_eq!(c.first().workflow_id(), "order-r1");
    assert_eq!(c.first().namespace(), "default");
}

#[test]
fn a_key_that_cannot_list_namespaces_still_lands_somewhere_usable() {
    // Temporal Cloud: the key is scoped to one namespace, so ListNamespaces is refused
    // while everything inside that namespace works. Stranding the reader on the opening
    // screen would make tmprl unusable against Cloud for no good reason.
    let mut app = App::detached("sit", "lora-sit.ixing", unbounded_channel().0);
    app.handle(Msg::Namespaces(Err(
        "code: 'The caller does not have permission to execute the specified operation', \
         message: \"Request unauthorized.\""
            .into(),
    )));

    let rows = app.namespace_rows();
    assert_eq!(rows.len(), 1, "the profile's own namespace is the fallback");
    assert_eq!(rows[0].name, "lora-sit.ixing");
    let (note, level) = app.note.clone().expect("the reader must be told why");
    assert!(note.contains("cannot list namespaces"), "{note}");
    assert!(note.contains("lora-sit.ixing"), "{note}");
    assert_eq!(
        level,
        Note::Info,
        "this is not an error, it is a scoped key"
    );
}

#[test]
fn a_real_namespace_failure_is_still_an_error() {
    // Only permission is special-cased; a transport failure must not be dressed up as a
    // one-row list, which would look like a cluster with one namespace.
    let mut app = App::detached("sit", "lora-sit.ixing", unbounded_channel().0);
    app.handle(Msg::Namespaces(Err("transport error".into())));

    assert!(app.namespace_rows().is_empty());
    assert_eq!(app.note.clone().unwrap().1, Note::Error);
}

#[test]
fn a_readonly_profile_refuses_before_a_confirmation_opens() {
    // The refusal costs one keystroke: a signal payload typed in full and then rejected
    // teaches the reader nothing useful.
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));
    assert!(
        app.readonly(),
        "config.toml should have marked prod read-only"
    );

    app.run("workflow.terminate", None);
    assert!(app.confirm.is_none(), "no confirmation may open");
    let (text, level) = app.note.clone().expect("a refusal should be reported");
    assert!(
        text.contains("prod"),
        "the refusal names the profile: {text}"
    );
    assert!(text.contains("read-only"), "{text}");
    assert!(matches!(level, Note::Warn));
}

#[test]
fn a_readonly_profile_refuses_at_the_wire_too() {
    // The guard that matters: whatever route a mutation took to get here, this is the
    // only path to the cluster.
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));

    // Batch and single share this path, so guarding it covers both.
    app.run_mutations(vec![Mutation::Terminate {
        namespace: "default".into(),
        workflow_id: "order-r1".into(),
        run_id: "r1".into(),
        reason: "x".into(),
    }]);
    let (text, _) = app.note.clone().expect("a refusal should be reported");
    assert!(text.contains("read-only"), "{text}");
}

#[test]
fn a_profile_without_a_readonly_flag_still_mutates() {
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.sit]\nreadonly = true"));
    assert!(
        !app.readonly(),
        "another profile's flag must not apply here"
    );

    app.run("workflow.terminate", None);
    assert!(app.confirm.is_some(), "a confirmation should still open");
}

#[test]
fn a_confirmation_owns_every_key_while_it_is_up() {
    // Nothing bound elsewhere may fire while a destructive action is pending.
    let mut app = on_a_workflow();
    let before = app.view.cursor;
    app.run("workflow.terminate", None);

    app.handle(Msg::Key(Chord::ch('j')));
    assert_eq!(app.view.cursor, before, "j must not move the cursor");
    assert!(
        app.confirm.is_some(),
        "and must not dismiss the confirmation"
    );

    app.handle(Msg::Key(Chord::ch(' ')));
    assert!(
        app.which_key.is_empty(),
        "the leader must not open which-key"
    );
}

#[test]
fn escape_always_backs_out() {
    let mut app = on_a_workflow();
    app.run("workflow.terminate", None);
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Esc)));

    assert!(app.confirm.is_none());
    let (msg, _) = app.note.clone().unwrap();
    assert_eq!(msg, "cancelled");
}

#[test]
fn deleting_costs_a_word_and_nearly_is_not_enough() {
    let mut app = on_a_workflow();
    app.run("workflow.delete", None);
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word.as_deref(), Some("delete"));

    // Enter with the word unfinished is not a refusal, just not yet.
    for ch in "delet".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));
    assert!(app.confirm.is_some(), "still waiting for the word");

    app.handle(Msg::Key(Chord::ch('e')));
    assert!(app.confirm.clone().unwrap().is_satisfied());
}

#[test]
fn a_mutation_needs_something_under_the_cursor() {
    let mut app = app(); // namespace screen, nothing selected
    app.run("workflow.terminate", None);
    assert!(app.confirm.is_none());
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn a_history_screen_mutates_the_workflow_it_is_showing() {
    let mut app = on_a_workflow();
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);

    app.run("workflow.cancel", None);
    let c = app
        .confirm
        .clone()
        .expect("the open workflow is the target");
    assert_eq!(c.first().workflow_id(), "order-r1");
}

#[test]
fn a_mutation_over_a_selection_covers_every_selected_row() {
    let mut app = on_four_workflows();
    select(&mut app, 3);
    app.run("workflow.cancel", None);

    let c = app.confirm.clone().expect("confirmed");
    assert_eq!(c.len(), 3);
    assert!(c.is_batch());
    let ids: Vec<&str> = c.mutations.iter().map(|m| m.workflow_id()).collect();
    assert_eq!(ids, ["order-r1", "order-r2", "order-r3"]);
    assert_eq!(c.first().verb(), "Cancel");
}

#[test]
fn without_a_selection_a_mutation_still_covers_one_row() {
    // The batch path and the single path are the same code, so they cannot drift.
    let mut app = on_four_workflows();
    app.run("workflow.cancel", None);
    let c = app.confirm.clone().expect("confirmed");
    assert_eq!(c.len(), 1);
    assert!(!c.is_batch());
    assert_eq!(c.first().workflow_id(), "order-r1");
}

#[test]
fn a_selection_upwards_covers_the_same_rows_as_one_downwards() {
    let mut app = on_four_workflows();
    app.handle(Msg::Key(Chord::ch('j')));
    app.handle(Msg::Key(Chord::ch('j')));
    app.run("mode.visual-line", None);
    app.handle(Msg::Key(Chord::ch('k')));
    app.run("workflow.cancel", None);

    let c = app.confirm.clone().unwrap();
    let ids: Vec<&str> = c.mutations.iter().map(|m| m.workflow_id()).collect();
    assert_eq!(ids, ["order-r2", "order-r3"]);
}

#[test]
fn a_destructive_batch_costs_the_count_rather_than_a_keypress() {
    // One key is too cheap to end three workflows at once.
    let mut app = on_four_workflows();
    select(&mut app, 3);
    app.run("workflow.terminate", None);

    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word.as_deref(), Some("3"));
    assert!(!c.is_satisfied(), "Enter alone does not go ahead");

    for ch in "3".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    assert!(app.confirm.clone().unwrap().is_satisfied());
}

#[test]
fn a_batch_that_destroys_histories_still_costs_the_word() {
    // Delete outranks the count: it destroys the record itself.
    let mut app = on_four_workflows();
    select(&mut app, 2);
    app.run("workflow.delete", None);
    assert_eq!(
        app.confirm.clone().unwrap().typed_word.as_deref(),
        Some("delete")
    );
}

#[test]
fn a_single_non_destructive_action_still_costs_nothing() {
    let mut app = on_four_workflows();
    app.run("workflow.cancel", None);
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word, None);
    assert!(c.is_satisfied());
}

#[test]
fn running_a_batch_spends_the_selection() {
    // The rows it covered are about to change, so a second batch must not be one
    // keypress away from a range that no longer means what it did.
    let mut app = on_four_workflows();
    select(&mut app, 2);
    assert!(app.view.selection().is_some());
    app.run("workflow.cancel", None);
    // A cancel over two rows is destructive, so it owes the count first.
    app.handle(Msg::Key(Chord::ch('2')));
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.view.selection().is_none());
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn a_signal_over_a_selection_sends_the_same_name_to_each() {
    let mut app = on_four_workflows();
    select(&mut app, 2);
    app.run("workflow.signal", None);
    for ch in "retry".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let c = app.confirm.clone().expect("confirmed");
    assert_eq!(c.len(), 2);
    assert!(c.mutations.iter().all(|m| m.cli().contains("--name retry")));
    assert_eq!(c.typed_word, None, "a signal is not a loss");
}

#[test]
fn a_batch_reports_progress_rather_than_each_row_in_turn() {
    let mut app = on_four_workflows();
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::Cancel {
            namespace: "default".into(),
            workflow_id: "order-r1".into(),
            run_id: "r1".into(),
        }),
        result: Ok(()),
        batch: Some((2, 3)),
    });
    let (msg, _) = app.note.clone().unwrap();
    assert_eq!(msg, "cancelled 2/3");
}

#[test]
fn a_signal_asks_for_its_name_before_confirming() {
    let mut app = on_a_workflow();
    app.run("workflow.signal", None);
    assert!(app.confirm.is_none(), "a signal needs a name first");
    assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Signal);

    for ch in "retry".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let c = app.confirm.clone().expect("now it can be confirmed");
    assert!(
        c.first().cli().contains("--name retry"),
        "{}",
        c.first().cli()
    );
    assert!(!c.first().is_destructive(), "a signal is not a loss");
}

#[test]
fn a_reset_resolves_to_a_workflow_task_the_cursor_is_not_on() {
    // The rows the server needs are exactly the ones the outline folds away, so "reset
    // to here" walks back, and the confirmation shows which id it landed on.
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
    let mut app = on_a_workflow();
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((
            vec![
                hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("W"),
                hev(2, G::Opened(2), R::Opens, C::WorkflowTask),
                hev(3, G::Opened(2), R::Closes, C::WorkflowTask).with_outcome(O::Completed),
                hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("A"),
                hev(5, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed),
            ],
            Vec::new(),
        )),
    });
    app.run("motion.bottom", None); // the activity group, not a workflow task

    app.run("workflow.reset", None);
    let c = app.confirm.clone().expect("a confirmation");
    assert!(
        c.first().cli().contains("--event-id 3"),
        "should resolve back to the completed workflow task: {}",
        c.first().cli()
    );
    assert!(c.first().is_destructive(), "a reset abandons work");
}

#[test]
fn a_reset_needs_a_history_and_says_so() {
    let mut app = on_a_workflow(); // still on the workflow list
    app.run("workflow.reset", None);
    assert!(app.confirm.is_none());
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("history"), "got {msg}");
}

#[test]
fn a_history_with_no_completed_task_cannot_be_reset() {
    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    let mut app = on_a_workflow();
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((
            vec![hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("W")],
            Vec::new(),
        )),
    });
    app.run("workflow.reset", None);
    assert!(app.confirm.is_none(), "there is nowhere valid to reset to");
}

#[test]
fn an_update_asks_for_its_name_and_is_not_destructive() {
    let mut app = on_a_workflow();
    app.run("workflow.update", None);
    assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Update);
    assert_eq!(app.prompt.clone().unwrap().sigil(), "update:");

    for ch in "setLimit".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let c = app.confirm.clone().expect("a confirmation");
    assert!(
        c.first().cli().contains("update execute"),
        "{}",
        c.first().cli()
    );
    assert!(c.first().cli().contains("--name setLimit"));
    assert!(
        !c.first().is_destructive(),
        "an update adds, it does not end"
    );
}

#[test]
fn a_signal_and_an_update_do_not_get_confused() {
    let mut app = on_a_workflow();
    app.run("workflow.signal", None);
    for ch in "ping".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));
    let cli = app.confirm.clone().unwrap().first().cli();
    assert!(cli.contains("workflow signal"), "{cli}");
    assert!(!cli.contains("update"), "{cli}");
}

#[test]
fn a_finished_mutation_reports_and_refreshes() {
    let mut app = on_a_workflow();
    let m = Mutation::Cancel {
        namespace: "default".into(),
        workflow_id: "order-r1".into(),
        run_id: "r1".into(),
    };
    app.handle(Msg::Mutated {
        mutation: Box::new(m),
        result: Ok(()),
        batch: None,
    });
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Info);
    assert!(msg.contains("order-r1"), "got {msg}");
}

#[test]
fn a_failed_mutation_shows_the_servers_reason() {
    let mut app = on_a_workflow();
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::Cancel {
            namespace: "default".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
        }),
        result: Err("PermissionDenied: not allowed".into()),
        batch: None,
    });
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("PermissionDenied"), "got {msg}");
}

#[test]
fn creating_a_schedule_collects_every_field_then_confirms() {
    let mut app = on_schedules();
    app.run("schedule.create", None);
    let form = app.form.clone().expect("a form opens");
    assert_eq!(form.cursor, 0);
    assert!(app.confirm.is_none(), "nothing is proposed yet");

    for (i, v) in ["nightly", "recon", "OrderWorkflow", "demo-tq", "0 2 * * *"]
        .iter()
        .enumerate()
    {
        for ch in v.chars() {
            app.handle(Msg::Key(Chord::ch(ch)));
        }
        if i < 4 {
            app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Tab)));
        }
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.form.is_none(), "the form closes once it is complete");
    let cli = app.confirm.clone().expect("confirmed").first().cli();
    assert!(cli.contains("--schedule-id nightly"), "{cli}");
    assert!(cli.contains("--workflow-id recon"), "{cli}");
    assert!(cli.contains("--type OrderWorkflow"), "{cli}");
    assert!(cli.contains("--task-queue demo-tq"), "{cli}");
    assert!(cli.contains("--cron '0 2 * * *'"), "{cli}");
    assert!(!cli.contains("--input"), "input was left empty: {cli}");
}

#[test]
fn an_incomplete_schedule_sends_the_caret_to_the_field_that_is_missing() {
    // Naming the field without going to it leaves the reader hunting for it.
    let mut app = on_schedules();
    app.run("schedule.create", None);
    for ch in "nightly".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.confirm.is_none(), "nothing is proposed");
    let form = app.form.clone().expect("the form stays open");
    assert_eq!(form.fields[form.cursor].label, "workflow id");
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("workflow id"), "got {msg}");
}

#[test]
fn backspace_on_an_empty_field_steps_back_rather_than_closing_the_form() {
    // A form is several fields deep, so losing all of them to one key would be a trap.
    let mut app = on_schedules();
    app.run("schedule.create", None);
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Tab)));
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Backspace)));

    let form = app.form.clone().expect("still open");
    assert_eq!(form.fields[form.cursor].label, "schedule id");
}

#[test]
fn a_schedule_form_takes_literal_keys_rather_than_running_commands() {
    // `j` and `q` are bound in Normal mode; inside a field they are text.
    let mut app = on_schedules();
    app.run("schedule.create", None);
    for ch in "jq".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    assert_eq!(app.form.clone().unwrap().get("schedule id"), "jq");
    assert!(!app.should_quit);
}

#[test]
fn a_backfill_asks_for_its_window_before_confirming() {
    let mut app = on_schedules();
    app.run("schedule.backfill", None);
    assert!(app.confirm.is_none(), "a backfill needs a window first");
    assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Backfill);

    for ch in "-1d..now".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let m = app
        .confirm
        .clone()
        .expect("now it can be confirmed")
        .first()
        .clone();
    assert_eq!(m.verb(), "Backfill");
    assert_eq!(m.schedule_id(), Some("nightly"));
    let cli = m.cli();
    assert!(cli.contains("--overlap-policy BufferAll"), "{cli}");
    assert!(!m.is_destructive(), "it starts runs, it destroys nothing");
}

#[test]
fn an_unreadable_backfill_window_stops_at_the_prompt() {
    // The server's answer to a bad range is less specific than the parser's, and
    // confirming first would put a nonsense command in front of the reader.
    let mut app = on_schedules();
    app.run("schedule.backfill", None);
    for ch in "yesterday".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.confirm.is_none(), "nothing to confirm");
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("START..END"), "got {msg}");
}

#[test]
fn pausing_toggles_towards_the_opposite_of_now() {
    // One key does both, so the target state is whatever the schedule is not.
    let mut app = on_schedules();
    app.run("schedule.pause", None);
    let m = app.confirm.clone().unwrap().first().clone();
    assert_eq!(m.verb(), "Pause");
    assert!(m.cli().ends_with("--pause"));

    app.confirm = None;
    app.view.schedules.value_mut().unwrap()[0].paused = true;
    app.run("schedule.pause", None);
    let m = app.confirm.clone().unwrap().first().clone();
    assert_eq!(m.verb(), "Resume");
    assert!(m.cli().ends_with("--unpause"));
}

#[test]
fn a_paused_schedule_shows_the_new_state_before_the_list_catches_up() {
    // ListSchedules is eventually consistent, so a refresh straight after the patch can
    // return the old state and contradict the message beside it.
    let mut app = on_schedules();
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::PauseSchedule {
            namespace: "default".into(),
            schedule_id: "nightly".into(),
            paused: true,
        }),
        result: Ok(()),
        batch: None,
    });
    assert!(app.view.schedule_rows()[0].paused);
}

#[test]
fn deleting_a_schedule_costs_the_typed_word() {
    let mut app = on_schedules();
    app.run("schedule.delete", None);
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word.as_deref(), Some("delete"));
    assert!(c.first().cli().starts_with("temporal schedule delete "));
}

#[test]
fn schedule_keys_need_a_schedule_under_the_cursor() {
    let mut app = app(); // namespace screen
    app.run("schedule.trigger", None);
    assert!(app.confirm.is_none());
    assert!(matches!(app.note, Some((_, Note::Warn))));
}
