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

#[test]
fn the_profile_list_says_when_it_is_loading_or_could_not_be_read() {
    let mut app = app_with_rows();
    app.view.screen = crate::app::Screen::Profiles;
    app.view.profiles = Loadable::Loading;
    assert!(draw(&mut app, 80, 6).contains("loading profiles…"));

    app.view.profiles = Loadable::Failed(tmprl_core::fault::Fault {
        operation: "",
        code: tmprl_core::fault::Code::Local,
        message: "bad toml".into(),
    });
    assert!(draw(&mut app, 80, 6).contains("could not read temporal.toml (R to retry)"));
}

#[test]
fn a_profile_with_no_address_or_namespace_shows_the_defaults() {
    let mut app = app_with_rows();
    app.view.screen = crate::app::Screen::Profiles;
    app.view.profiles = Loadable::loaded(vec![ProfileSummary {
        name: "bare".into(),
        address: None,
        namespace: None,
    }]);
    let out = draw(&mut app, 100, 6);
    let row = out.lines().find(|l| l.contains("bare")).expect(&out);
    assert!(
        row.contains("localhost:7233") && row.contains("default"),
        "{out}"
    );
}

#[test]
fn an_address_is_shown_without_its_scheme_or_slash() {
    use crate::ui::statusline::short_address;
    assert_eq!(
        short_address("https://prod.example:7233/"),
        "prod.example:7233"
    );
    assert_eq!(short_address("http://127.0.0.1:7233"), "127.0.0.1:7233");
    assert_eq!(short_address("localhost:7233"), "localhost:7233");
    assert_eq!(short_address(""), "");
}
