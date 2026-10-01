//! The payload pane.
//!
//! Shows the full detail of whatever the cursor is on: the event's fields, and its payloads
//! decoded and pretty-printed. It is a pane under the list rather than an overlay, because
//! the value you are reading usually only makes sense next to the row it belongs to.
//!
//! For a *group* row the interesting payloads are its input and its result, the arguments it
//! was called with and what came back. Those live on the events that opened and closed the
//! group, so the pane gathers from both rather than showing only the row you happen to be on.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tmprl_core::history::{Failure, NormalizedEvent};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::Rendered;
use tmprl_core::pending::{self, PendingActivity};

use crate::app::{App, DecodeState};
use crate::theme::Theme;
use crate::view::View;

/// Draw the pane. Returns how far it can scroll and where it ended up, which moves when a
/// search asked it to find the match.
pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) -> (usize, usize) {
    if area.height < 2 {
        return (0, 0);
    }
    // A filter result replaces the payloads: you asked to see the filtered value, and
    // showing both would bury it.
    if let Some(piped) = view.piped.clone() {
        let max = render_piped(frame, area, view, &piped, t);
        return (max, view.detail_scroll.min(max));
    }

    let Some(outline) = view.history.value() else {
        return (0, 0);
    };
    let lines = match outline.row_at(view.cursor) {
        Some(Row::Event { event, .. }) => outline
            .event(event)
            .map(|e| event_lines(e, app, t))
            .unwrap_or_default(),
        Some(Row::Group { group, .. }) => group_lines(outline, group, view, app, t),
        None => Vec::new(),
    };

    let lines = if lines.is_empty() {
        vec![Line::from(Span::styled("  nothing carried here", t.faint))]
    } else {
        lines
    };

    let search = app.search.for_pane();
    let first_match = lines
        .iter()
        .position(|l| l.spans.iter().any(|s| search.matches(&s.content)));
    let lines: Vec<Line> = lines
        .into_iter()
        .map(|l| super::highlight(l, &search, super::match_style()))
        .collect();

    // A payload can be far taller than the pane. Clipping it silently would hide the end of
    // a stack trace, which is the part worth reading, so the pane scrolls and says so.
    let visible = area.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    // One line of context above the match, its label or the key it sits under.
    let scroll = match first_match {
        Some(at) if view.detail_seek => at.saturating_sub(1),
        _ => view.detail_scroll,
    }
    .min(max_scroll);

    let title = if max_scroll == 0 {
        " payloads (K to close) ".to_string()
    } else {
        format!(
            " payloads (<C-e>/<C-y> to scroll, {}/{}, K to close) ",
            scroll + 1,
            max_scroll + 1
        )
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(t.faint)
        .title(Span::styled(title, t.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // This pane's only border is the rule along its top, so the bar takes a column rather
    // than being drawn over the payload.
    let inner = super::list_scrollbar(frame, inner, scroll, lines.len(), t);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
    (max_scroll, scroll)
}

fn group_lines<'a>(
    outline: &'a Outline,
    group: usize,
    view: &'a View,
    app: &App,
    t: &Theme,
) -> Vec<Line<'a>> {
    let Some(g) = outline.group(group) else {
        return Vec::new();
    };
    let live = pending::for_group(&view.pending, g, outline.events());
    let mut title = vec![
        Span::styled(
            format!("  {} ", g.subject),
            t.fg.add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  {} event(s)", g.outcome.label(), g.events.len()),
            t.dim,
        ),
    ];
    // On the title line rather than its own: this pane's height is the payloads' budget,
    // and a line spent here is a line of JSON that scrolls out of sight.
    if g.started_at.is_some() {
        let ended = match g.ended_at {
            Some(e) => format!(" → {}", app.clock.stamp(Some(e))),
            None => String::new(),
        };
        title.push(Span::styled(
            format!("  {}{ended}", app.clock.stamp(g.started_at)),
            t.faint,
        ));
    }
    let mut lines = vec![Line::from(title)];
    if let Some(p) = live {
        lines.push(pending_line(p, app, t));
    }
    let failure = g
        .failure
        .as_ref()
        .or(live.and_then(|p| p.last_failure.as_ref()));
    if let Some(f) = failure {
        lines.extend(failure_lines(f, "  ", t));
    }

    let before = lines.len();
    for id in g.payload_ends() {
        let Some(e) = outline.events().iter().find(|e| e.id == id) else {
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
            t.faint,
        )));
    }
    if let Some(f) = failure {
        lines.extend(stack_lines(f, "  ", t));
    }
    lines
}

/// Where a still-open activity is in its retries, as the server last described it.
fn pending_line<'a>(p: &PendingActivity, app: &App, t: &Theme) -> Line<'a> {
    let mut parts = vec![
        format!("attempt {}", p.attempts_label()),
        p.state.label().to_string(),
    ];
    if let Some(at) = p.next_attempt_at {
        parts.push(format!("next attempt {}", app.clock.stamp(Some(at))));
    }
    if let Some(w) = &p.last_worker {
        parts.push(format!("worker {w}"));
    }
    Line::from(Span::styled(format!("  {}", parts.join(" · ")), t.warn))
}

/// The failure chain: every link's message, then what it was raised from.
///
/// The outermost link is usually the least specific thing anyone could say about the
/// failure, "activity task failed", so the chain is the point: the link that names the
/// class and the sentence a human wrote is normally two down from it.
fn failure_lines<'a>(f: &'a Failure, indent: &str, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for (depth, link) in f.chain().enumerate() {
        let prefix = if depth == 0 { "" } else { "caused by " };
        lines.push(Line::from(Span::styled(
            format!("{indent}{prefix}{}", link.headline()),
            t.err,
        )));

        let mut tags = Vec::new();
        if let Some(s) = &link.source {
            tags.push(s.clone());
        }
        if link.non_retryable {
            // The reason a failed activity never came back, and not otherwise on screen.
            tags.push("not retryable".to_string());
        }
        if !tags.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("{indent}  {}", tags.join(" · ")),
                t.faint,
            )));
        }
    }
    lines
}

/// Stack traces, last in the pane and after the payloads.
///
/// A Java trace is fifty lines that push the input out of sight, and the input is what you
/// read first. Everything is still here, one `<C-e>` away, rather than truncated.
fn stack_lines<'a>(f: &'a Failure, indent: &str, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for link in f.chain() {
        let Some(trace) = &link.stack_trace else {
            continue;
        };
        lines.push(Line::from(Span::styled(
            format!("{indent}stack trace  {}", link.headline()),
            t.warn.add_modifier(Modifier::BOLD),
        )));
        for l in trace.lines() {
            lines.push(Line::from(Span::styled(format!("{indent}  {l}"), t.dim)));
        }
    }
    lines
}

fn event_lines<'a>(e: &'a NormalizedEvent, app: &App, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("  {} ", e.name), t.fg.add_modifier(Modifier::BOLD)),
        Span::styled(format!("event {}", e.id), t.dim),
    ])];
    if e.time.is_some() {
        lines.push(Line::from(Span::styled(
            format!("    at {}", app.clock.full(e.time)),
            t.faint,
        )));
    }
    for (k, v) in &e.fields {
        lines.push(Line::from(vec![
            Span::styled(format!("    {k} = "), t.faint),
            Span::styled(v.clone(), t.fg),
        ]));
    }
    if let Some(f) = &e.failure {
        lines.extend(failure_lines(f, "    ", t));
    }
    lines.extend(payload_lines(e, app, t));
    if let Some(f) = &e.failure {
        lines.extend(stack_lines(f, "    ", t));
    }
    lines
}

fn payload_lines<'a>(e: &'a NormalizedEvent, app: &App, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for (label, p) in &e.payloads {
        lines.push(Line::from(Span::styled(
            format!("  {label}"),
            t.accent.add_modifier(Modifier::BOLD),
        )));
        match p.render() {
            Rendered::Text(text) => {
                for l in text.lines() {
                    lines.push(Line::from(Span::styled(format!("    {l}"), t.fg)));
                }
            }
            Rendered::Null => lines.push(Line::from(Span::styled("    null", t.faint))),
            // Say what it is and how big rather than showing bytes. The value is not lost,
            // it is just not something a terminal should be asked to print.
            Rendered::Opaque { bytes, encoding } => lines.push(Line::from(Span::styled(
                format!("    {encoding}, {bytes} bytes, not shown"),
                t.faint,
            ))),
            // The encoding is named rather than called "encrypted": a codec chooses its own
            // encoding, so seeing the actual one is what tells you whether the codec server
            // you are running is the right one for this payload.
            Rendered::Encrypted { bytes, encoding } => {
                let (what, style) = match app.decode_state(p) {
                    DecodeState::NoCodec => (
                        format!(
                            "    🔒 {encoding}, {bytes} bytes, set a codec endpoint in config.toml"
                        ),
                        t.warn,
                    ),
                    DecodeState::InFlight => (
                        format!("    🔒 {encoding}, {bytes} bytes, decoding…"),
                        t.dim,
                    ),
                    DecodeState::Idle => (format!("    🔒 {encoding}, {bytes} bytes"), t.warn),

                    DecodeState::Failed(why) => (
                        format!("    🔒 {encoding}, {bytes} bytes, codec: {why} (R to retry)"),
                        t.err,
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
    view: &View,
    piped: &Result<String, String>,
    t: &Theme,
) -> usize {
    let (body, style, label) = match piped {
        Ok(out) => (out, t.fg, "filtered"),
        Err(err) => (err, t.err, "filter failed"),
    };
    let lines: Vec<Line> = if body.trim().is_empty() {
        vec![Line::from(Span::styled("  (no output)", t.faint))]
    } else {
        body.lines()
            .map(|l| Line::from(Span::styled(format!("  {l}"), style)))
            .collect()
    };

    let visible = area.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = view.detail_scroll.min(max_scroll);

    let title = if max_scroll == 0 {
        format!(" {label} (K to close) ")
    } else {
        format!(
            " {label} (<C-e>/<C-y> to scroll, {}/{}, K to close) ",
            scroll + 1,
            max_scroll + 1
        )
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(if piped.is_err() { t.err } else { t.faint })
        .title(Span::styled(title, t.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
    max_scroll
}
