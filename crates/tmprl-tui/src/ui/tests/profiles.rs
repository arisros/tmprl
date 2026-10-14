//! The profile list and the address in the header.

use super::*;
use tmprl_client::ProfileSummary;

#[test]
fn the_profile_list_marks_the_connected_one() {
    let mut app = app_with_rows();
    app.view.profiles = Loadable::loaded(vec![
        ProfileSummary {
            name: "dev".into(),
            address: Some("localhost:7233".into()),
            namespace: None,
        },
        ProfileSummary {
            name: "prod".into(),
            address: Some("prod.example:7233".into()),
            namespace: Some("orders".into()),
        },
    ]);
    app.view.screen = crate::app::Screen::Profiles;
    let out = draw(&mut app, 100, 8);
    let prod = out
        .lines()
        .find(|l| l.contains("prod.example:7233"))
        .expect(&out);
    assert!(
        prod.contains("orders") && prod.contains("connected"),
        "{out}"
    );
    let dev = out
        .lines()
        .find(|l| l.contains("localhost:7233"))
        .expect(&out);
    assert!(
        dev.contains("default") && !dev.contains("connected"),
        "{out}"
    );
    assert!(out.contains("2 profiles"), "{out}");
}

#[test]
fn the_header_shows_the_address_when_there_is_room() {
    let mut app = app_with_rows();
    let wide = draw(&mut app, 100, 6);
    assert!(
        wide.lines().next().unwrap().contains("(detached)"),
        "{wide}"
    );
    let narrow = draw(&mut app, 40, 6);
    assert!(
        !narrow.lines().next().unwrap().contains("detached"),
        "{narrow}"
    );
}
