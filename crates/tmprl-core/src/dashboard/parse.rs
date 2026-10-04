//! `dashboard.toml`, read strictly.

use std::ops::RangeInclusive;

use super::layout::{
    DEFAULT_LIMIT, Layout, MAX_LIMIT, MAX_PANELS, PanelKind, PanelSpec, RUNNING, RowSpec, Show,
    Size,
};
use crate::config::ConfigError;
use crate::query;
use crate::timerange::parse_offset;

const FILE: &str = "dashboard.toml";

const TOP_KEYS: &str = "row";

const ROW_KEYS: &str = "height, lines or panel";

fn allowed(kind: &str) -> Option<(&'static [&'static str], &'static str)> {
    Some(match kind {
        "counts" => (
            &["kind", "title", "width", "namespaces", "query"],
            "kind, title, width, namespaces or query",
        ),
        "workflows" | "types" => (
            &[
                "kind",
                "title",
                "width",
                "namespaces",
                "limit",
                "query",
                "since",
            ],
            "kind, title, width, namespaces, limit, query or since",
        ),
        "queues" => (
            &[
                "kind",
                "title",
                "width",
                "namespaces",
                "limit",
                "query",
                "names",
            ],
            "kind, title, width, namespaces, limit, query or names",
        ),
        "schedules" => (
            &["kind", "title", "width", "namespaces", "limit", "show"],
            "kind, title, width, namespaces, limit or show",
        ),
        _ => return None,
    })
}

fn wrong(path: String, expected: &'static str) -> ConfigError {
    ConfigError::Type {
        file: FILE,
        path,
        expected,
    }
}

fn only(
    table: &toml::Table,
    keys: &[&str],
    place: String,
    expected: &'static str,
) -> Result<(), ConfigError> {
    match table.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(key) => Err(ConfigError::UnknownDashboardKey {
            place,
            key: key.clone(),
            expected,
        }),
        None => Ok(()),
    }
}

fn tables<'a>(
    table: &'a toml::Table,
    key: &str,
    path: &str,
    expected: &'static str,
) -> Result<Vec<&'a toml::Table>, ConfigError> {
    let Some(raw) = table.get(key) else {
        return Ok(Vec::new());
    };
    raw.as_array()
        .and_then(|a| a.iter().map(|v| v.as_table()).collect::<Option<Vec<_>>>())
        .ok_or_else(|| wrong(path.to_string(), expected))
}

fn text(table: &toml::Table, key: &str, path: &str) -> Result<Option<String>, ConfigError> {
    match table.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| wrong(format!("{path}.{key}"), "a string")),
    }
}

fn number(
    table: &toml::Table,
    key: &str,
    path: &str,
    range: RangeInclusive<i64>,
    expected: &'static str,
) -> Result<Option<u16>, ConfigError> {
    match table.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_integer()
            .filter(|n| range.contains(n))
            .and_then(|n| u16::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| wrong(format!("{path}.{key}"), expected)),
    }
}

fn strings(table: &toml::Table, key: &str, path: &str) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = table.get(key) else {
        return Ok(Vec::new());
    };
    raw.as_array()
        .and_then(|a| {
            a.iter()
                .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| wrong(format!("{path}.{key}"), "an array of names"))
}

fn filter(table: &toml::Table, path: &str, default: &str) -> Result<String, ConfigError> {
    let query = text(table, "query", path)?.unwrap_or_else(|| default.to_string());
    if query::orders(&query) {
        return Err(wrong(
            format!("{path}.query"),
            "a query without ORDER BY, a panel sorts its own rows",
        ));
    }
    Ok(query)
}

fn window(table: &toml::Table, path: &str) -> Result<Option<i64>, ConfigError> {
    match text(table, "since", path)? {
        None => Ok(None),
        Some(s) => parse_offset(&s)
            .filter(|ms| *ms > 0)
            .map(Some)
            .ok_or_else(|| wrong(format!("{path}.since"), "a duration such as 30m, 24h or 7d")),
    }
}

fn parse_panel(table: &toml::Table, path: &str) -> Result<PanelSpec, ConfigError> {
    let kind =
        text(table, "kind", path)?.ok_or_else(|| wrong(format!("{path}.kind"), "a panel kind"))?;
    let (keys, expected) = allowed(&kind).ok_or_else(|| ConfigError::BadPanelKind {
        path: format!("{path}.kind"),
        value: kind.clone(),
    })?;
    only(table, keys, format!("{path}, a {kind} panel"), expected)?;

    let kind = match kind.as_str() {
        "counts" => PanelKind::Counts {
            query: filter(table, path, "")?,
        },
        "workflows" => PanelKind::Workflows {
            query: filter(table, path, "")?,
            since_ms: window(table, path)?,
        },
        "types" => PanelKind::Types {
            query: filter(table, path, "")?,
            since_ms: window(table, path)?,
        },
        "queues" => PanelKind::Queues {
            query: filter(table, path, RUNNING)?,
            names: strings(table, "names", path)?,
        },
        _ => PanelKind::Schedules {
            show: match text(table, "show", path)?.as_deref() {
                None | Some("all") => Show::All,
                Some("paused") => Show::Paused,
                Some("upcoming") => Show::Upcoming,
                Some(_) => {
                    return Err(wrong(
                        format!("{path}.show"),
                        "`paused`, `upcoming` or `all`",
                    ));
                }
            },
        },
    };

    Ok(PanelSpec {
        kind,
        title: text(table, "title", path)?,
        width: number(table, "width", path, 1..=100, "an integer from 1 to 100")?.unwrap_or(1),
        limit: number(
            table,
            "limit",
            path,
            1..=MAX_LIMIT as i64,
            "an integer from 1 to 50",
        )?
        .map_or(DEFAULT_LIMIT, usize::from),
        namespaces: strings(table, "namespaces", path)?,
    })
}

/// Parse `dashboard.toml`:
///
/// ```toml
/// [[row]]
/// lines = 3
///
/// [[row.panel]]
/// kind = "counts"
///
/// [[row]]
///
/// [[row.panel]]
/// kind  = "workflows"
/// title = "Recent failures"
/// query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
/// since = "24h"
/// width = 2
/// ```
///
/// Strict: a key that does not exist, or one that belongs to another kind of panel, is an
/// error. A misspelt `query` would otherwise show every workflow under a "failures" title.
/// A file with no rows is an empty layout, which the caller reads as "not configured".
pub fn parse_dashboard(src: &str) -> Result<Layout, ConfigError> {
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;
    only(&table, &["row"], "the file".to_string(), TOP_KEYS)?;

    let mut rows = Vec::new();
    for (r, row) in tables(&table, "row", "row", "an array of [[row]] tables")?
        .into_iter()
        .enumerate()
    {
        let path = format!("row[{r}]");
        only(row, &["height", "lines", "panel"], path.clone(), ROW_KEYS)?;
        let height = number(row, "height", &path, 1..=100, "an integer from 1 to 100")?;
        let lines = number(row, "lines", &path, 3..=50, "an integer from 3 to 50")?;
        let size = match (height, lines) {
            (Some(_), Some(_)) => return Err(wrong(path, "given `height` or `lines`, not both")),
            (_, Some(n)) => Size::Lines(n),
            (weight, None) => Size::Weight(weight.unwrap_or(1)),
        };

        let panel_path = format!("{path}.panel");
        let specs = tables(
            row,
            "panel",
            &panel_path,
            "an array of [[row.panel]] tables",
        )?;
        if specs.is_empty() {
            return Err(wrong(panel_path, "at least one [[row.panel]] table"));
        }
        let panels = specs
            .into_iter()
            .enumerate()
            .map(|(p, panel)| parse_panel(panel, &format!("{panel_path}[{p}]")))
            .collect::<Result<Vec<_>, _>>()?;
        rows.push(RowSpec { size, panels });
    }

    let layout = Layout { rows };
    if layout.panels().count() > MAX_PANELS {
        return Err(wrong("row".to_string(), "at most 12 panels in all"));
    }
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::layout::DAY_MS;

    fn panel(src: &str) -> Result<Layout, ConfigError> {
        parse_dashboard(&format!("[[row]]\n[[row.panel]]\n{src}"))
    }

    fn unknown_key(result: Result<Layout, ConfigError>) -> String {
        match result {
            Err(ConfigError::UnknownDashboardKey { key, .. }) => key,
            other => panic!("expected an unknown key, got {other:?}"),
        }
    }

    fn wrong_path(result: Result<Layout, ConfigError>) -> String {
        match result {
            Err(ConfigError::Type { path, .. }) => path,
            other => panic!("expected a type error, got {other:?}"),
        }
    }

    #[test]
    fn a_file_with_no_rows_is_an_empty_layout() {
        assert!(parse_dashboard("").unwrap().is_empty());
        assert!(parse_dashboard("# nothing yet\n").unwrap().is_empty());
    }

    #[test]
    fn the_documented_example_parses() {
        let layout = parse_dashboard(
            r#"
            [[row]]
            lines = 3

            [[row.panel]]
            kind = "counts"

            [[row]]
            height = 3

            [[row.panel]]
            kind  = "workflows"
            title = "Recent failures"
            query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
            since = "24h"
            width = 2

            [[row.panel]]
            kind  = "types"
            query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
            since = "24h"
            "#,
        )
        .unwrap();

        assert_eq!(layout.rows[0].size, Size::Lines(3));
        assert_eq!(layout.rows[1].size, Size::Weight(3));
        let recent = &layout.rows[1].panels[0];
        assert_eq!(recent.title(), "Recent failures");
        assert_eq!(recent.width, 2);
        assert_eq!(
            recent.kind,
            PanelKind::Workflows {
                query: query::PROBLEMS.to_string(),
                since_ms: Some(DAY_MS),
            }
        );
        assert_eq!(layout.rows[1].panels[1].title(), "Workflow types");
    }

    #[test]
    fn what_a_panel_leaves_out_gets_a_default() {
        let layout = panel("kind = \"queues\"").unwrap();
        assert_eq!(layout.rows[0].size, Size::Weight(1));
        let spec = &layout.rows[0].panels[0];
        assert_eq!((spec.width, spec.limit), (1, DEFAULT_LIMIT));
        assert!(spec.namespaces.is_empty());
        assert_eq!(
            spec.kind,
            PanelKind::Queues {
                query: RUNNING.to_string(),
                names: Vec::new(),
            }
        );

        let layout = panel("kind = \"schedules\"\nshow = \"paused\"\nlimit = 5").unwrap();
        let spec = &layout.rows[0].panels[0];
        assert_eq!(spec.kind, PanelKind::Schedules { show: Show::Paused });
        assert_eq!(spec.limit, 5);
        assert_eq!(spec.title(), "Paused schedules");
    }

    #[test]
    fn a_key_that_does_not_exist_is_an_error_at_every_level() {
        assert_eq!(unknown_key(parse_dashboard("rows = 1")), "rows");
        assert_eq!(
            unknown_key(parse_dashboard(
                "[[row]]\nheigth = 2\n[[row.panel]]\nkind = \"counts\""
            )),
            "heigth"
        );
        assert_eq!(
            unknown_key(panel("kind = \"workflows\"\nqeury = \"x\"")),
            "qeury"
        );
    }

    #[test]
    fn a_key_of_another_kind_of_panel_is_an_error() {
        assert_eq!(
            unknown_key(panel("kind = \"counts\"\nsince = \"1h\"")),
            "since"
        );
        assert_eq!(unknown_key(panel("kind = \"counts\"\nlimit = 3")), "limit");
        assert_eq!(
            unknown_key(panel("kind = \"schedules\"\nquery = \"x\"")),
            "query"
        );
        let message = panel("kind = \"workflows\"\nshow = \"all\"")
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("row[0].panel[0], a workflows panel"),
            "{message}"
        );
    }

    #[test]
    fn a_kind_must_be_given_and_must_exist() {
        assert_eq!(wrong_path(panel("title = \"x\"")), "row[0].panel[0].kind");
        assert_eq!(
            panel("kind = \"gauge\""),
            Err(ConfigError::BadPanelKind {
                path: "row[0].panel[0].kind".into(),
                value: "gauge".into(),
            })
        );
    }

    #[test]
    fn a_row_has_one_size_and_at_least_one_panel() {
        assert_eq!(
            wrong_path(parse_dashboard(
                "[[row]]\nheight = 2\nlines = 4\n[[row.panel]]\nkind = \"counts\""
            )),
            "row[0]"
        );
        assert_eq!(
            wrong_path(parse_dashboard("[[row]]\nlines = 4")),
            "row[0].panel"
        );
        assert_eq!(wrong_path(parse_dashboard("row = 3")), "row");
    }

    #[test]
    fn numbers_outside_their_range_are_errors() {
        assert_eq!(
            wrong_path(panel("kind = \"counts\"\nwidth = 0")),
            "row[0].panel[0].width"
        );
        assert_eq!(
            wrong_path(panel("kind = \"workflows\"\nlimit = 51")),
            "row[0].panel[0].limit"
        );
        assert_eq!(
            wrong_path(parse_dashboard(
                "[[row]]\nlines = 2\n[[row.panel]]\nkind = \"counts\""
            )),
            "row[0].lines"
        );
    }

    #[test]
    fn a_query_may_not_order_and_a_window_must_be_a_duration() {
        assert_eq!(
            wrong_path(panel(
                "kind = \"workflows\"\nquery = \"A = 'b' ORDER BY StartTime\""
            )),
            "row[0].panel[0].query"
        );
        assert_eq!(
            wrong_path(panel("kind = \"workflows\"\nsince = \"yesterday\"")),
            "row[0].panel[0].since"
        );
        assert_eq!(
            wrong_path(panel("kind = \"workflows\"\nsince = \"0h\"")),
            "row[0].panel[0].since"
        );
        assert_eq!(
            wrong_path(panel("kind = \"schedules\"\nshow = \"soon\"")),
            "row[0].panel[0].show"
        );
        assert_eq!(
            wrong_path(panel("kind = \"queues\"\nnames = [\"a\", 3]")),
            "row[0].panel[0].names"
        );
    }

    #[test]
    fn a_layout_holds_a_bounded_number_of_panels() {
        let mut src = String::from("[[row]]\n");
        for _ in 0..=MAX_PANELS {
            src.push_str("[[row.panel]]\nkind = \"counts\"\n");
        }
        assert_eq!(wrong_path(parse_dashboard(&src)), "row");
    }
}
