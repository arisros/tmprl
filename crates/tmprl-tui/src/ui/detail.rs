//! The payload pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tmprl_core::history::{Failure, NormalizedEvent};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::{Payload, Rendered};
use tmprl_core::pending::{self, PendingActivity};

use crate::app::{App, DecodeState};
use crate::theme::Theme;
use crate::view::View;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    app: &App,
    t: &Theme,
    window_focused: bool,
) -> (usize, usize) {
    if area.height < 2 {
        return (0, 0);
    }
    let focused = window_focused && view.detail_focus;
    if let Some(piped) = view.piped.clone() {
        let max = render_piped(frame, area, view, &piped, t, focused);
        return (max, view.detail_scroll.min(max));
    }

    let Some(outline) = view.history.value() else {
        return (0, 0);
    };
    let (label, lines) = match &view.queried {
        Some(answer) => ("query", payload_lines(answer, app, t)),
        None => ("payloads", cursor_lines(outline, view, app, t)),
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

    let visible = area.height.saturating_sub(1) as usize;
    let (lines, starts) = fit(lines, area.width as usize, visible);
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = match first_match {
        Some(at) if view.detail_seek => starts[at].saturating_sub(1),
        _ => view.detail_scroll,
    }
    .min(max_scroll);

    let title = title(label, view, focused, scroll, max_scroll);
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(t.border(focused))
        .title(Span::styled(title, t.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = super::list_scrollbar(frame, inner, scroll, lines.len(), t);
    frame.render_widget(
        Paragraph::new(visible_rows(lines, scroll, inner.height)),
        inner,
    );
    record_text(frame, inner, view);
    (max_scroll, scroll)
}

fn cursor_lines<'a>(outline: &'a Outline, view: &'a View, app: &App, t: &Theme) -> Vec<Line<'a>> {
    match outline.row_at(view.cursor) {
        Some(Row::Event { event, .. }) => outline
            .event(event)
            .map(|e| event_lines(e, app, t))
            .unwrap_or_default(),
        Some(Row::Group { group, .. }) => group_lines(outline, group, view, app, t),
        None => Vec::new(),
    }
}

fn record_text(frame: &mut Frame, area: Rect, view: &View) {
    let buf = frame.buffer_mut();
    let cells = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        })
        .collect();
    *view.detail_text.borrow_mut() = crate::view::DrawnText {
        area: tmprl_ui::Rect::new(area.x, area.y, area.width, area.height),
        cells,
    };
}

fn title(label: &str, view: &View, focused: bool, scroll: usize, max_scroll: usize) -> String {
    let at = format!("{}/{}", scroll + 1, max_scroll + 1);
    match (max_scroll, focused) {
        (0, true) => format!(" {label} (<Esc> back) "),
        (0, false) => format!(" {label} (K to close) "),
        (_, true) => format!(" {label} (j/k <C-d>/<C-u> gg/G, {at}, <Esc> back) "),
        (_, false) => {
            let key = if view.detail_beside {
                "<C-w>l"
            } else {
                "<C-w>j"
            };
            format!(" {label} ({key} to scroll, {at}, K to close) ")
        }
    }
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
    let ids = g.ids(outline.events());
    if !ids.is_empty() {
        let mut spans = Vec::new();
        for (name, value) in ids {
            spans.push(Span::styled(format!("  {name} = "), t.faint));
            spans.push(Span::styled(value.to_string(), t.fg));
        }
        lines.push(Line::from(spans));
    }
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
        lines.extend(payload_lines(&e.payloads, app, t));
    }
    if lines.len() == before {
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

fn failure_lines<'a>(f: &'a Failure, indent: &str, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for (depth, link) in f.chain().enumerate() {
        let prefix = if depth == 0 { "" } else { "caused by " };
        let headline = link.headline();
        for (n, part) in headline.lines().enumerate() {
            let lead = if n == 0 { prefix } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{indent}{lead}{part}"),
                t.err,
            )));
        }

        let mut tags = Vec::new();
        if let Some(s) = &link.source {
            tags.push(s.clone());
        }
        if link.non_retryable {
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
    lines.extend(payload_lines(&e.payloads, app, t));
    if let Some(f) = &e.failure {
        lines.extend(stack_lines(f, "    ", t));
    }
    lines
}

fn payload_lines<'a>(payloads: &[(String, Payload)], app: &App, t: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for (label, p) in payloads {
        lines.push(Line::from(Span::styled(
            format!("  {label}"),
            t.accent.add_modifier(Modifier::BOLD),
        )));
        if let Some(shown) = app.shown_text(p) {
            for l in &shown.lines {
                lines.push(Line::from(Span::styled(format!("    {l}"), t.fg)));
            }
            if shown.hidden > 0 {
                lines.push(hidden_line(shown.hidden, t));
            }
            continue;
        }
        match p.render() {
            Rendered::Text(_) => {}
            Rendered::Null => lines.push(Line::from(Span::styled("    null", t.faint))),
            Rendered::Opaque { bytes, encoding } => lines.push(Line::from(Span::styled(
                format!("    {encoding}, {bytes} bytes, not shown"),
                t.faint,
            ))),
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

fn render_piped(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    piped: &Result<String, String>,
    t: &Theme,
    focused: bool,
) -> usize {
    let (body, style, label) = match piped {
        Ok(out) => (out, t.fg, "filtered"),
        Err(err) => (err, t.err, "filter failed"),
    };
    let lines: Vec<Line> = if body.trim().is_empty() {
        vec![Line::from(Span::styled("  (no output)", t.faint))]
    } else {
        let shown = crate::app::cut_for_pane(body);
        let mut lines: Vec<Line> = shown
            .lines
            .into_iter()
            .map(|l| Line::from(Span::styled(format!("  {l}"), style)))
            .collect();
        if shown.hidden > 0 {
            lines.push(hidden_line(shown.hidden, t));
        }
        lines
    };

    let visible = area.height.saturating_sub(1) as usize;
    let (lines, _) = fit(lines, area.width as usize, visible);
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = view.detail_scroll.min(max_scroll);

    let title = title(label, view, focused, scroll, max_scroll);
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(if piped.is_err() {
            t.err
        } else {
            t.border(focused)
        })
        .title(Span::styled(title, t.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = super::list_scrollbar(frame, inner, scroll, lines.len(), t);
    frame.render_widget(
        Paragraph::new(visible_rows(lines, scroll, inner.height)),
        inner,
    );
    record_text(frame, inner, view);
    max_scroll
}

fn hidden_line<'a>(hidden: usize, t: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        format!("    … {hidden} more lines, <leader>e opens the whole payload in $EDITOR"),
        t.faint,
    ))
}

// Paragraph::scroll takes a u16, which wraps past 65,535 rows.
fn visible_rows(lines: Vec<Line<'_>>, scroll: usize, height: u16) -> Vec<Line<'_>> {
    lines
        .into_iter()
        .skip(scroll)
        .take(height as usize)
        .collect()
}

fn fit(lines: Vec<Line<'_>>, width: usize, height: usize) -> (Vec<Line<'_>>, Vec<usize>) {
    let full = wrap(lines.clone(), width);
    if full.0.len() <= height || width < 4 {
        return full;
    }
    wrap(lines, width - 1)
}

fn wrap(lines: Vec<Line<'_>>, width: usize) -> (Vec<Line<'_>>, Vec<usize>) {
    let mut out = Vec::with_capacity(lines.len());
    let mut starts = Vec::with_capacity(lines.len());
    for line in lines {
        starts.push(out.len());
        let cells: Vec<(char, Style)> = line
            .spans
            .iter()
            .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
            .collect();
        if width == 0 || cells.len() <= width {
            out.push(line);
            continue;
        }
        let lead = cells.iter().take_while(|(c, _)| *c == ' ').count();
        let indent = (lead + 2).min(width / 2);
        let mut at = 0;
        let mut first = true;
        while at < cells.len() {
            let pad = if first { 0 } else { indent };
            let room = width - pad;
            let mut end = (at + room).min(cells.len());
            if end < cells.len()
                && let Some(space) = cells[at..end].iter().rposition(|(c, _)| *c == ' ')
                && space > 0
            {
                end = at + space + 1;
            }
            let mut spans = vec![Span::raw(" ".repeat(pad))];
            spans.extend(runs(&cells[at..end]));
            out.push(Line::from(spans).style(line.style));
            at = end;
            while at < cells.len() && cells[at].0 == ' ' {
                at += 1;
            }
            first = false;
        }
    }
    (out, starts)
}

fn runs(cells: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for &(c, style) in cells {
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), style)),
        }
    }
    spans
}
