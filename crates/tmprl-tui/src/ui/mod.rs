//! Rendering. Every function here is a pure projection of `&App` onto a frame, no state

mod cmdline;
mod complete;
mod confirm;
mod dashboard;
mod detail;
mod form;
mod help;
mod highlight;
mod history;
mod messages;
mod namespaces;
mod picker;
mod profiles;
mod query;
mod schedules;
mod statusline;
mod timeline;
mod whichkey;
mod workflows;

pub(crate) use highlight::{highlight, match_style};
pub(crate) use history::category_label;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState};

use tmprl_core::WorkflowStatus;
use tmprl_core::config::PayloadPane;

use crate::app::{App, PromptKind, Screen};
use crate::theme::Theme;
use crate::view::View;

pub fn render(frame: &mut Frame, app: &mut App) {
    let theme = app.theme;
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    app.set_frame(to_ui(body));
    let panes = app.tabs.layout(to_ui(body));

    if let Some(pane) = panes.iter().find(|p| p.focused) {
        let rect = from_ui(pane.rect);
        let rect = if panes.len() > 1 {
            Block::default().borders(Borders::ALL).inner(rect)
        } else {
            rect
        };
        let inner = pane_body(
            rect,
            app.view.screen,
            app.view.show_detail,
            app.payload_pane(),
            app.payload_size(),
        );
        app.view.page = inner.list.height.saturating_sub(1) as usize;
        match inner.detail {
            Some(detail) => {
                app.view.detail_beside = detail.x > inner.list.x;
                app.view.detail_page = detail.height.saturating_sub(1) as usize;
                app.view.detail_area = to_ui(detail);
                app.view.body_area = to_ui(inner.list.union(detail));
            }
            None => {
                app.view.detail_focus = false;
                app.view.detail_area = tmprl_ui::Rect::default();
                app.view.body_area = tmprl_ui::Rect::default();
            }
        }
    }

    statusline::render_header(frame, header, app, &theme);

    let mut focused_detail_max = None;
    for pane in &panes {
        let Some(view) = (if pane.focused {
            Some(&app.view)
        } else {
            app.parked_view(pane.view)
        }) else {
            continue;
        };
        let max = render_pane(
            frame,
            from_ui(pane.rect),
            view,
            app,
            &theme,
            pane.focused,
            panes.len() > 1,
        );
        if pane.focused {
            focused_detail_max = max;
        }
    }
    if let Some((max, scroll)) = focused_detail_max {
        app.view.detail_max_scroll = max;
        app.view.detail_scroll = scroll;
    }
    if let Some(selection) = app.text_selection() {
        let area = app.view.detail_text.borrow().area;
        let buf = frame.buffer_mut();
        for (y, first, last) in selection.lines(area) {
            for x in first..=last {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.modifier.insert(ratatui::style::Modifier::REVERSED);
                }
            }
        }
    }
    app.view.detail_seek = false;

    statusline::render_status(frame, status, app, &theme);

    if app
        .prompt
        .as_ref()
        .is_some_and(|p| p.kind == PromptKind::Command)
    {
        cmdline::render(frame, app, &theme);
    }
    if let Some(p) = &app.picker {
        picker::render(frame, p, &theme);
    }
    if !app.which_key.is_empty() {
        whichkey::render(frame, app, &theme);
    }
    if app.show_help {
        help::render(frame, app, &theme);
    }
    if app.show_messages {
        messages::render(frame, app, &theme);
    }
    if let Some(f) = &app.form {
        form::render(frame, f, &theme);
    }
    if let Some(c) = &app.confirm {
        confirm::render(frame, c, &theme);
    }
}

struct PaneAreas {
    query: ratatui::layout::Rect,
    list: ratatui::layout::Rect,
    detail: Option<ratatui::layout::Rect>,
}

const MIN_SIDE_BY_SIDE_WIDTH: u16 = 100;

fn pane_body(
    area: ratatui::layout::Rect,
    screen: Screen,
    show_detail: bool,
    payload: PayloadPane,
    size: u16,
) -> PaneAreas {
    let query_height = match screen {
        Screen::Workflows => 1,
        Screen::Profiles
        | Screen::Namespaces
        | Screen::History
        | Screen::Schedules
        | Screen::Dashboard => 0,
    };
    let [query, rest] =
        Layout::vertical([Constraint::Length(query_height), Constraint::Min(1)]).areas(area);

    if screen == Screen::History && show_detail {
        let [list, detail] =
            if payload == PayloadPane::Right && rest.width >= MIN_SIDE_BY_SIDE_WIDTH {
                Layout::horizontal([Constraint::Min(40), Constraint::Percentage(size)]).areas(rest)
            } else {
                Layout::vertical([Constraint::Min(3), Constraint::Percentage(size)]).areas(rest)
            };
        PaneAreas {
            query,
            list,
            detail: Some(detail),
        }
    } else {
        PaneAreas {
            query,
            list: rest,
            detail: None,
        }
    }
}

fn render_pane(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    view: &View,
    app: &App,
    theme: &Theme,
    focused: bool,
    split: bool,
) -> Option<(usize, usize)> {
    let area = if split {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border(focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };
    if area.width == 0 || area.height == 0 {
        return None;
    }

    view.rows_drawn.set(None);
    let areas = pane_body(
        area,
        view.screen,
        view.show_detail,
        app.payload_pane(),
        app.payload_size(),
    );
    match view.screen {
        Screen::Profiles => profiles::render(frame, areas.list, view, app, theme),
        Screen::Namespaces => namespaces::render(frame, areas.list, view, app, theme),
        Screen::Workflows => {
            query::render(frame, areas.query, view, app, theme, focused);
            workflows::render(frame, areas.list, view, app, theme);
            if focused && let Some(c) = &app.completion {
                complete::render(frame, areas.list, area, c, theme);
            }
        }
        Screen::Schedules => schedules::render(frame, areas.list, view, app, theme),
        Screen::Dashboard => dashboard::render(frame, areas.list, view, app, theme, focused),
        Screen::History => {
            history::render(frame, areas.list, view, app, theme);
            if let Some(mut pane) = areas.detail {
                if pane.x > areas.list.x {
                    let rule = Block::default()
                        .borders(Borders::LEFT)
                        .border_style(theme.faint);
                    let inner = rule.inner(pane);
                    frame.render_widget(rule, pane);
                    pane = inner;
                }
                return Some(detail::render(frame, pane, view, app, theme, focused));
            }
        }
    }
    None
}

fn to_ui(r: ratatui::layout::Rect) -> tmprl_ui::Rect {
    tmprl_ui::Rect::new(r.x, r.y, r.width, r.height)
}

fn from_ui(r: tmprl_ui::Rect) -> ratatui::layout::Rect {
    ratatui::layout::Rect::new(r.x, r.y, r.width, r.height)
}

fn list_scrollbar(frame: &mut Frame, area: Rect, cursor: usize, total: usize, t: &Theme) -> Rect {
    let height = area.height as usize;
    if total <= height || area.width < 4 {
        return area;
    }
    let body = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    let bar = Rect {
        x: area.x + area.width - 1,
        y: area.y,
        width: 1,
        height: area.height,
    };
    draw_scrollbar(frame, bar, cursor, total, height, t);
    body
}

fn border_scrollbar(frame: &mut Frame, area: Rect, position: usize, total: usize, t: &Theme) {
    let height = area.height.saturating_sub(2) as usize;
    if total <= height || height == 0 {
        return;
    }
    draw_scrollbar(frame, area, position, total, height, t);
}

fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    position: usize,
    total: usize,
    viewport: usize,
    t: &Theme,
) {
    let mut state = ScrollbarState::new(total)
        .position(position)
        .viewport_content_length(viewport);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_style(t.faint),
        area,
        &mut state,
    );
}

fn status_style(s: WorkflowStatus, t: &Theme) -> Style {
    match s {
        WorkflowStatus::Running => t.accent,
        WorkflowStatus::Completed => t.ok,
        WorkflowStatus::Failed | WorkflowStatus::TimedOut => t.err,
        WorkflowStatus::Terminated | WorkflowStatus::Canceled => t.warn,
        WorkflowStatus::ContinuedAsNew | WorkflowStatus::Paused => t.dim,
        WorkflowStatus::Unspecified => t.faint,
    }
}

fn gutter(i: usize, cursor: usize) -> String {
    if i == cursor {
        format!("{:>4} ", i + 1)
    } else {
        format!("{:>4} ", i.abs_diff(cursor))
    }
}

pub(crate) fn edit_spans(
    line: &tmprl_core::line::LineEdit,
    width: usize,
    text: Style,
    cursor: Style,
) -> Vec<ratatui::text::Span<'static>> {
    use ratatui::text::Span;
    let before: Vec<char> = line.before().chars().collect();
    let mut after = line.after().chars();
    let under = after.next();
    let room = width.saturating_sub(1).max(1);
    let skip = before.len().saturating_sub(room);
    let shown_before: String = before[skip..].iter().collect();
    let left = room.saturating_sub(before.len() - skip);
    let shown_after: String = after.take(left).collect();
    vec![
        Span::styled(shown_before, text),
        Span::styled(under.map_or("█".to_string(), String::from), cursor),
        Span::styled(shown_after, text),
    ]
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

fn failed(fault: &tmprl_core::fault::Fault) -> String {
    match fault.hint() {
        Some(hint) if hint.contains(" R") => format!("{fault}. {hint}"),
        Some(hint) => format!("{fault}. {hint} (R to retry)"),
        None => format!("{fault} (R to retry)"),
    }
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn centered(area: ratatui::layout::Rect, w: u16, h: u16) -> ratatui::layout::Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    ratatui::layout::Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests;
