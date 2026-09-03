//! The payload pane.
//!
//! Shows the full detail of whatever the cursor is on: the event's fields, and its payloads
//! decoded and pretty-printed. It is a pane under the list rather than an overlay, because
//! the value you are reading usually only makes sense next to the row it belongs to.
//!
//! For a *group* row the interesting payloads are its input and its result — the arguments it
//! was called with and what came back. Those live on the events that opened and closed the
//! group, so the pane gathers from both rather than showing only the row you happen to be on.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tmprl_core::history::NormalizedEvent;
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::Rendered;

use crate::app::{App, DecodeState};
use crate::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.height < 2 {
        return;
    }
    // A filter result replaces the payloads: you asked to see the filtered value, and
    // showing both would bury it.
    if let Some(piped) = app.piped.clone() {
        return render_piped(frame, area, app, &piped, t);
    }

    let Some(outline) = app.history.value() else {
        return;
    };
    let lines = match outline.row_at(app.cursor) {
        Some(Row::Event { event, .. }) => outline
            .event(event)
            .map(|e| event_lines(e, app, t))
            .unwrap_or_default(),
        Some(Row::Group { group, .. }) => group_lines(outline, group, app, t),
        None => Vec::new(),
    };

    let lines = if lines.is_empty() {
        vec![Line::from(Span::styled(
            "  nothing carried here",
            Style::new().fg(t.faint),
        ))]
    } else {
        lines
    };

    // A payload can be far taller than the pane. Clipping it silently would hide the end of
    // a stack trace, which is the part worth reading, so the pane scrolls and says so.
    let visible = area.height.saturating_sub(1) as usize;
    app.detail_max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.detail_scroll.min(app.detail_max_scroll);
    app.detail_scroll = scroll;

    let title = if app.detail_max_scroll == 0 {
        " payloads — K to close ".to_string()
    } else {
        format!(
            " payloads — <C-e>/<C-y> to scroll ({}/{}) — K to close ",
            scroll + 1,
            app.detail_max_scroll + 1
        )
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(t.faint))
        .title(Span::styled(title, Style::new().fg(t.accent)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

/// The events whose payloads a group cares about: the one that opened it and the one that
/// closed it. The middle of a group is task plumbing and carries nothing.
fn group_lines<'a>(outline: &'a Outline, group: usize, app: &App, t: &Theme) -> Vec<Line<'a>> {
    let Some(g) = outline.group(group) else {
        return Vec::new();
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("  {} ", g.subject),
            Style::new().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  {} event(s)", g.outcome.label(), g.events.len()),
            Style::new().fg(t.dim),
        ),
    ])];
    if let Some(f) = &g.failure {
        lines.push(Line::from(Span::styled(
            format!("  {f}"),
            Style::new().fg(t.err),
        )));
    }

    let before = lines.len();
    for id in [g.events.first(), g.events.last()].into_iter().flatten() {
        let Some(e) = outline.events().iter().find(|e| e.id == *id) else {
            continue;
        };
        if e.payloads.is_empty() {
            continue;
        }
        lines.extend(payload_lines(e, app, t));
    }
    if lines.len() == before {
        // A pane showing nothing but a title reads as broken. Plenty of events genuinely
        // carry no payload, and saying so is the answer to "where is the input".
        lines.push(Line::from(Span::styled(
            "  no payloads on this group",
            Style::new().fg(t.faint),
        )));
    }
    lines
}

fn event_lines<'a>(e: &'a NormalizedEvent, app: &App, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("  {} ", e.name),
            Style::new().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("event {}", e.id), Style::new().fg(t.dim)),
    ])];
    for (k, v) in &e.fields {
        lines.push(Line::from(vec![
            Span::styled(format!("    {k} = "), Style::new().fg(t.faint)),
            Span::styled(v.clone(), Style::new().fg(t.fg)),
        ]));
    }
    if let Some(f) = &e.failure {
        lines.push(Line::from(Span::styled(
            format!("    {f}"),
            Style::new().fg(t.err),
        )));
    }
    lines.extend(payload_lines(e, app, t));
    lines
}

fn payload_lines<'a>(e: &'a NormalizedEvent, app: &App, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for (label, p) in &e.payloads {
        lines.push(Line::from(Span::styled(
            format!("  {label}"),
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        )));
        match p.render() {
            Rendered::Text(text) => {
                for l in text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("    {l}"),
                        Style::new().fg(t.fg),
                    )));
                }
            }
            Rendered::Null => lines.push(Line::from(Span::styled(
                "    null",
                Style::new().fg(t.faint),
            ))),
            // Say what it is and how big rather than showing bytes. The value is not lost,
            // it is just not something a terminal should be asked to print.
            Rendered::Opaque { bytes, encoding } => lines.push(Line::from(Span::styled(
                format!("    {encoding}, {bytes} bytes — not shown"),
                Style::new().fg(t.faint),
            ))),
            Rendered::Encrypted { bytes } => {
                let (what, style) = match app.decode_state(p) {
                    DecodeState::NoCodec => (
                        format!(
                            "    🔒 encrypted, {bytes} bytes — set [codec] endpoint in config.toml"
                        ),
                        Style::new().fg(t.warn),
                    ),
                    DecodeState::InFlight => (
                        format!("    🔒 encrypted, {bytes} bytes — decoding…"),
                        Style::new().fg(t.dim),
                    ),
                    DecodeState::Idle => (
                        format!("    🔒 encrypted, {bytes} bytes"),
                        Style::new().fg(t.warn),
                    ),
                };
                lines.push(Line::from(Span::styled(what, style)));
            }
        }
    }
    lines
}

/// The output of a `!` filter.
///
/// Failure is rendered as the command's own stderr rather than a message of ours: when a jq
/// expression is wrong, jq's diagnosis is the entire answer and paraphrasing it loses the
/// line and column.
fn render_piped(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    piped: &Result<String, String>,
    t: &Theme,
) {
    let (body, style, label) = match piped {
        Ok(out) => (out, Style::new().fg(t.fg), "filtered"),
        Err(err) => (err, Style::new().fg(t.err), "filter failed"),
    };
    let lines: Vec<Line> = if body.trim().is_empty() {
        vec![Line::from(Span::styled(
            "  (no output)",
            Style::new().fg(t.faint),
        ))]
    } else {
        body.lines()
            .map(|l| Line::from(Span::styled(format!("  {l}"), style)))
            .collect()
    };

    let visible = area.height.saturating_sub(1) as usize;
    app.detail_max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.detail_scroll.min(app.detail_max_scroll);
    app.detail_scroll = scroll;

    let title = if app.detail_max_scroll == 0 {
        format!(" {label} — K to close ")
    } else {
        format!(
            " {label} — <C-e>/<C-y> to scroll ({}/{}) — K to close ",
            scroll + 1,
            app.detail_max_scroll + 1
        )
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(if piped.is_err() { t.err } else { t.faint }))
        .title(Span::styled(title, Style::new().fg(t.accent)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}
