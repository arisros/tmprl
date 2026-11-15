//! The header and status lines.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tmprl_core::config::Accent;

use super::status_style;
use crate::app::{App, Note, Screen};
use crate::theme::Theme;

const BRAND_WIDTH: usize = 8;
const CRUMB: &str = " › ";
const MIN_SCOPE: usize = 8;

pub fn render_header(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let screen = app.view.screen;
    let mut right = match screen {
        Screen::Profiles => profile_summary(app, t),
        Screen::Namespaces => namespace_summary(app, t),
        Screen::Workflows => workflow_summary(app, t),
        Screen::History => Vec::new(),
        Screen::Schedules => schedule_summary(app, t),
        Screen::Dashboard => dashboard_summary(app, t),
    };
    let pill = pill_spans(app, t);
    let pill_width = spans_width(&pill);
    let scope = scope_label(app);
    let mut address = format!(" {}", short_address(app.address()));
    let mut place = screen_name(screen, false);

    let width = area.width as usize;
    let need = |address: &str, place: &str, right: &[Span]| {
        BRAND_WIDTH
            + pill_width
            + chars(address)
            + 2 * CRUMB.chars().count()
            + chars(&scope).min(MIN_SCOPE)
            + chars(place)
            + spans_width(right)
            + 2
    };
    if need(&address, place, &right) > width {
        address.clear();
    }
    if need(&address, place, &right) > width {
        right.clear();
    }
    if need(&address, place, &right) > width {
        place = screen_name(screen, true);
    }
    let fixed = need(&address, place, &right) - chars(&scope).min(MIN_SCOPE);
    let scope = elide_middle(&scope, width.saturating_sub(fixed).max(MIN_SCOPE));

    let mut left = vec![Span::raw(" ")];
    left.extend(super::brand::wordmark(t));
    left.push(Span::raw(" "));
    left.extend(pill);
    left.extend([
        Span::styled(address, t.faint),
        Span::styled(CRUMB, t.faint),
        Span::styled(scope, t.fg),
        Span::styled(CRUMB, t.faint),
        Span::styled(place, t.accent),
    ]);
    frame.render_widget(Paragraph::new(Line::from(left)), area);

    let right_width = spans_width(&right);
    if right_width > 0 && width > right_width + 2 {
        let r = Rect {
            x: area.x + area.width - right_width as u16 - 1,
            y: area.y,
            width: right_width as u16,
            height: 1,
        };
        frame.render_widget(Paragraph::new(Line::from(right)), r);
    }
}

pub(super) fn pill_accent(app: &App) -> Accent {
    app.accent()
        .unwrap_or_else(|| Accent::guess(app.profile(), app.address()))
}

fn pill_spans<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    let name = app.profile().to_uppercase();
    let profile = t.pill(pill_accent(app));
    match (t.is_mono(), app.readonly()) {
        (true, false) => vec![Span::styled(format!("[{name}]"), profile)],
        (true, true) => vec![Span::styled(format!("[{name} RO]"), profile)],
        (false, false) => vec![Span::styled(format!(" {name} "), profile)],
        (false, true) => vec![
            Span::styled(format!(" {name} "), profile),
            Span::styled(" RO ", t.lock_pill()),
        ],
    }
}

fn screen_name(screen: Screen, short: bool) -> &'static str {
    match (screen, short) {
        (Screen::Profiles, false) => "profiles",
        (Screen::Profiles, true) => "prof",
        (Screen::Namespaces, false) => "namespaces",
        (Screen::Namespaces, true) => "ns",
        (Screen::Workflows, false) => "workflows",
        (Screen::Workflows, true) => "wf",
        (Screen::History, false) => "history",
        (Screen::History, true) => "hist",
        (Screen::Schedules, false) => "schedules",
        (Screen::Schedules, true) => "sched",
        (Screen::Dashboard, false) => "dashboard",
        (Screen::Dashboard, true) => "dash",
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| chars(&s.content)).sum()
}

pub(super) fn elide_middle(s: &str, max: usize) -> String {
    let n = chars(s);
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep.div_ceil(2);
    let tail = keep - head;
    let start: String = s.chars().take(head).collect();
    let end: String = s.chars().skip(n - tail).collect();
    format!("{start}…{end}")
}

pub(super) fn short_address(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.strip_suffix('/').unwrap_or(rest)
}

fn scope_label(app: &App) -> String {
    if let Some(w) = &app.view.viewing {
        return w.namespace.clone();
    }
    match app.view.scope.len() {
        0 | 1 => app.namespace().to_string(),
        n => format!("{} +{}", app.view.scope[0], n - 1),
    }
}

fn profile_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    let text = if app.view.profiles.is_loading() {
        "loading…".to_string()
    } else {
        format!("{} profiles", app.profile_rows().len())
    };
    vec![Span::styled(text, t.dim)]
}

fn namespace_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    let text = if app.view.namespaces.is_loading() {
        "loading…".to_string()
    } else {
        format!("{} namespaces", app.namespace_rows().len())
    };
    vec![Span::styled(text, t.dim)]
}

fn dashboard_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    let Some(board) = &app.view.dashboard else {
        return Vec::new();
    };
    let panels = board.panel_count();
    let loading = (0..panels).any(|p| board.state(p).is_some_and(|s| s.is_loading()));
    vec![Span::styled(
        if loading {
            "loading…".to_string()
        } else {
            match app.dashboard_refresh().interval_ms() {
                Some(ms) => format!(
                    "dashboard  {panels} panels  auto {}",
                    tmprl_core::workflow::humanize_age_ms(ms)
                ),
                None => format!("dashboard  {panels} panels"),
            }
        },
        t.dim,
    )]
}

fn workflow_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    if let Some(e) = app.view.counts.error() {
        return vec![Span::styled(format!("counts: {e}"), t.err)];
    }
    let Some(counts) = app.view.counts.value() else {
        return vec![Span::styled(
            if app.view.counts.is_loading() {
                "counting…"
            } else {
                ""
            },
            t.faint,
        )];
    };

    let mut spans = Vec::new();
    for (status, n) in counts.iter() {
        spans.push(Span::styled(
            format!("{} {n}  ", status.glyph()),
            status_style(status, t),
        ));
    }
    spans.push(Span::styled(format!("{} total", counts.total), t.dim));
    spans
}

fn schedule_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    let rows = app.view.schedule_rows();
    if rows.is_empty() {
        return vec![Span::styled(
            if app.view.schedules.is_loading() {
                "loading schedules…"
            } else {
                "no schedules"
            },
            t.faint,
        )];
    }
    let paused = rows.iter().filter(|s| s.paused).count();
    let mut spans = Vec::new();
    if paused > 0 {
        spans.push(Span::styled(
            format!("‖ {paused} paused  "),
            t.warn.add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(format!("{} schedules", rows.len()), t.dim));
    spans
}

pub fn render_status(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    if let Some(prompt) = &app.prompt {
        let sigil = prompt.sigil();
        let mut spans = vec![Span::styled(sigil, t.accent)];
        spans.extend(super::edit_spans(
            &prompt.buf,
            (area.width as usize).saturating_sub(sigil.chars().count()),
            t.fg,
            t.accent.add_modifier(Modifier::REVERSED),
        ));
        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mode = app.mode;
    let mut spans = vec![
        Span::styled(format!(" {} ", mode.label()), t.mode_badge(mode)),
        Span::raw(" "),
    ];
    if app.view.following {
        spans.push(Span::styled(" FOLLOW ", t.badge(t.ok)));
        spans.push(Span::raw(" "));
    }

    match &app.note {
        Some((msg, kind)) => {
            let c = match kind {
                Note::Info => t.ok,
                Note::Warn => t.warn,
                Note::Error => t.err,
            };
            spans.push(Span::styled(msg.clone(), c));
        }
        None => {
            if let Some(sel) = app.selection() {
                let n = sel.1 - sel.0 + 1;
                spans.push(Span::styled(format!("{n} selected"), t.warn));
            } else if let Some(err) = app.view.namespaces.error() {
                spans.push(Span::styled(err.to_string(), t.err));
            } else {
                spans.push(Span::styled("? help   : commands".to_string(), t.faint));
            }
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    let pend = app.pending.display();
    if !pend.is_empty() {
        let w = pend.len() as u16;
        if area.width > w + 2 {
            let r = Rect {
                x: area.x + area.width - w - 1,
                y: area.y,
                width: w,
                height: 1,
            };
            frame.render_widget(Paragraph::new(Span::styled(pend, t.warn)), r);
        }
    }
}
