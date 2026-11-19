//! The workflow bar at the top of a history, and the info card `gi` opens under it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tmprl_core::Loadable;
use tmprl_core::info::{self, Card, Wait};

use super::{status_style, truncate};
use crate::app::{App, Screen, now_ms};
use crate::theme::Theme;
use crate::view::View;

const INDENT: usize = 2;
const HINT: &str = " <Space>yw id · <Space>yR run ";
const WAITING: &str = "waiting on";
const TWO_COLUMNS: usize = 100;

pub fn height(view: &View, width: u16) -> u16 {
    if view.screen != Screen::History || view.viewing.is_none() {
        return 0;
    }
    if !view.show_info {
        return 1;
    }
    let body = match card(view, &|_| String::new()) {
        Some(card) => card_lines(&card, width as usize),
        None => 1,
    };
    1 + 2 + body as u16
}

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let bar = Rect { height: 1, ..area };
    frame.render_widget(Paragraph::new(bar_line(view, t, area.width as usize)), bar);
    if !view.show_info || area.height < 3 {
        return;
    }
    let block_area = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let mut block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(t.faint)
        .title(Span::styled(" info ", t.accent));
    if area.width as usize > HINT.len() + 12 {
        block = block.title_top(Line::from(Span::styled(HINT, t.faint)).right_aligned());
    }
    let inner = block.inner(block_area);
    frame.render_widget(block, block_area);
    frame.render_widget(
        Paragraph::new(styled_card(view, app, t, area.width as usize)),
        inner,
    );
}

fn bar_line<'a>(view: &View, t: &Theme, width: usize) -> Line<'a> {
    let Some(row) = &view.viewing else {
        return Line::default();
    };
    let described = view.info.value();
    let status = described.map_or(row.status, |i| i.status);
    let workflow_type = described.map_or(row.workflow_type.as_str(), |i| &i.workflow_type);
    let queue = described.map_or(row.task_queue.as_str(), |i| &i.task_queue);
    let (start, close) = described.map_or((row.start_time, row.close_time), |i| {
        (i.start_time, i.close_time)
    });

    let mut segments: Vec<(u8, Vec<Span<'a>>)> = vec![
        (
            0,
            vec![Span::styled(
                format!(" {} {} ", status.glyph(), status.query_name()),
                status_style(status, t).add_modifier(Modifier::BOLD),
            )],
        ),
        (
            4,
            vec![Span::styled(
                format!(" {workflow_type} "),
                t.fg.add_modifier(Modifier::BOLD),
            )],
        ),
        (
            0,
            vec![Span::styled(format!(" {} ", row.workflow_id), t.fg)],
        ),
    ];
    if queue != workflow_type {
        segments.push((5, vec![Span::styled(format!(" {queue} "), t.dim)]));
    }
    if let Some(took) = info::elapsed(start, close, now_ms()) {
        segments.push((3, vec![Span::styled(format!(" {took} "), t.dim)]));
    }
    if let Some(outline) = view.history.value() {
        let s = tmprl_core::outline::summarize(outline.groups());
        if s.failures > 0 {
            segments.push((
                1,
                vec![Span::styled(
                    format!(" ✗ {} failed ", s.failures),
                    t.err.add_modifier(Modifier::BOLD),
                )],
            ));
        }
        if s.in_flight > 0 {
            segments.push((
                6,
                vec![Span::styled(format!(" {} open ", s.in_flight), t.dim)],
            ));
        }
        segments.push((
            7,
            vec![Span::styled(
                format!(" {} events ", outline.events().len()),
                t.dim,
            )],
        ));
    }

    let hint = if view.show_info {
        "gi hide "
    } else {
        "gi info "
    };
    let fits = |segments: &[(u8, Vec<Span>)], with_hint: bool| {
        let used: usize = segments
            .iter()
            .flat_map(|(_, s)| s)
            .map(|s| s.content.chars().count())
            .sum();
        used + if with_hint { hint.len() + 1 } else { 0 } <= width
    };
    let mut with_hint = true;
    while !fits(&segments, with_hint) {
        if with_hint {
            with_hint = false;
            continue;
        }
        let Some(drop) = segments
            .iter()
            .enumerate()
            .filter(|(_, (p, _))| *p > 0)
            .max_by_key(|(_, (p, _))| *p)
            .map(|(i, _)| i)
        else {
            break;
        };
        segments.remove(drop);
    }

    let mut spans: Vec<Span<'a>> = segments.into_iter().flat_map(|(_, s)| s).collect();
    if with_hint {
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        spans.push(Span::raw(" ".repeat(width - used - hint.len())));
        spans.push(Span::styled(hint, t.faint));
    }
    Line::from(spans)
}

fn card(view: &View, stamp: &dyn Fn(i64) -> String) -> Option<Card> {
    let info = view.info.value()?;
    Some(info.card(now_ms(), &view.pending, stamp))
}

/// The glyph, its colour and the text's colour for one state, wherever pending work is shown.
pub(super) fn wait_look(wait: Wait, t: &Theme) -> (Option<char>, Style, Style) {
    match wait {
        Wait::Running => (Some('●'), t.accent, t.accent),
        Wait::Queued => (Some('○'), t.dim, t.fg),
        Wait::Retrying => (Some('↻'), t.warn, t.warn),
        Wait::Held => (Some('‖'), t.warn, t.warn),
        Wait::Quiet => (None, t.faint, t.faint),
    }
}

fn columns(width: usize) -> usize {
    if width >= TWO_COLUMNS { 2 } else { 1 }
}

fn card_lines(card: &Card, width: usize) -> usize {
    card.waiting.len()
        + card.facts.len().div_ceil(columns(width))
        + card.wide.len()
        + usize::from(!card.empty.is_empty())
}

fn label_width<'a>(labels: impl Iterator<Item = &'a str>) -> usize {
    labels.map(|l| l.chars().count()).max().unwrap_or(0)
}

fn cell<'a>(
    label: &str,
    label_width: usize,
    value: &str,
    width: usize,
    label_style: Style,
    value_style: Style,
) -> Vec<Span<'a>> {
    let value = truncate(value, width.saturating_sub(label_width + 3));
    let pad = width.saturating_sub(label_width + 2 + value.chars().count());
    vec![
        Span::styled(format!("{label:<label_width$}  "), label_style),
        Span::styled(value, value_style),
        Span::raw(" ".repeat(pad)),
    ]
}

fn styled_card<'a>(view: &View, app: &App, t: &Theme, width: usize) -> Vec<Line<'a>> {
    let card = match &view.info {
        Loadable::Failed(fault) => {
            return vec![Line::from(Span::styled(
                truncate(&format!("  {fault}"), width),
                t.err,
            ))];
        }
        Loadable::NotAsked | Loadable::Loading => {
            return vec![Line::from(Span::styled("  describing…", t.faint))];
        }
        Loadable::Loaded(..) => match card(view, &|ms| app.clock.moment(ms)) {
            Some(card) => card,
            None => return Vec::new(),
        },
    };
    let room = width.saturating_sub(INDENT);
    let indent = || Span::raw(" ".repeat(INDENT));
    let cols = columns(width);
    let left_labels = card
        .facts
        .iter()
        .step_by(cols)
        .map(|(l, _)| *l)
        .chain(card.wide.iter().map(|(l, _)| *l))
        .chain([WAITING]);
    let left = label_width(left_labels);
    let right = label_width(card.facts.iter().skip(1).step_by(2).map(|(l, _)| *l));

    let mut lines = Vec::new();
    for (i, item) in card.waiting.iter().enumerate() {
        let label = if i == 0 { WAITING } else { "" };
        let (glyph, glyph_style, style) =
            wait_look(card.states.get(i).copied().unwrap_or(Wait::Quiet), t);
        let text = match glyph {
            Some(g) => format!("{g} {item}"),
            None => item.clone(),
        };
        let mut spans = vec![indent()];
        let mut cell = cell(
            label,
            left,
            &text,
            room,
            t.accent.add_modifier(Modifier::BOLD),
            style,
        );
        let value = cell[1].content.to_string();
        if let Some(g) = glyph {
            let body = value.strip_prefix(&format!("{g} ")).unwrap_or(&value);
            let mut parts = vec![Span::styled(format!("{g} "), glyph_style)];
            match body.split_once(" · ") {
                Some((what, rest)) => {
                    parts.push(Span::styled(
                        what.to_string(),
                        style.add_modifier(Modifier::BOLD),
                    ));
                    parts.push(Span::styled(format!(" · {rest}"), t.dim));
                }
                None => parts.push(Span::styled(
                    body.to_string(),
                    style.add_modifier(Modifier::BOLD),
                )),
            }
            cell.splice(1..2, parts);
        }
        spans.extend(cell);
        lines.push(Line::from(spans));
    }

    let column = room / cols;
    for pair in card.facts.chunks(cols) {
        let mut spans = vec![indent()];
        for (i, (label, value)) in pair.iter().enumerate() {
            let label_width = if i == 0 { left } else { right };
            spans.extend(cell(label, label_width, value, column, t.dim, t.fg));
        }
        lines.push(Line::from(spans));
    }

    for (label, value) in &card.wide {
        let style = if value.starts_with('+') {
            t.faint
        } else {
            t.fg
        };
        let mut spans = vec![indent()];
        spans.extend(cell(label, left, value, room, t.dim, style));
        lines.push(Line::from(spans));
    }

    if !card.empty.is_empty() {
        lines.push(Line::from(vec![
            indent(),
            Span::styled(truncate(&card.empty.join(" · "), room), t.faint),
        ]));
    }
    lines
}
