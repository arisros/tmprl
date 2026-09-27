//! Rendering. Every function here is a pure projection of `&App` onto a frame, no state
//! is mutated except the viewport height, which the layout is what determines.

mod cmdline;
mod complete;
mod confirm;
mod detail;
mod form;
mod help;
mod highlight;
mod history;
mod namespaces;
mod picker;
mod query;
mod schedules;
mod statusline;
mod timeline;
mod whichkey;
mod workflows;

// The row labels the search matches against are built in `view`, and must read the same
// way the rows render, otherwise `/activity` finds rows that do not look like they say it.
pub(crate) use highlight::{highlight, match_style};
pub(crate) use history::category_label;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState};

use tmprl_core::config::PayloadPane;

use crate::app::{App, PromptKind, Screen};
use crate::theme::Theme;
use crate::view::View;

pub fn render(frame: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // Focus movement is geometric, so the tree has to know the area it was laid out in.
    app.set_frame(to_ui(body));
    let panes = app.tabs.layout(to_ui(body));

    // The focused pane's measurements are written back before anything is borrowed
    // immutably: a half page is half of *this* pane, not half the terminal.
    if let Some(pane) = panes.iter().find(|p| p.focused) {
        let inner = pane_body(
            from_ui(pane.rect),
            app.view.screen,
            app.view.show_detail,
            app.payload_pane(),
        );
        app.view.page = inner.list.height.saturating_sub(1) as usize;
    }

    statusline::render_header(frame, header, app, &theme);

    let mut focused_detail_max = None;
    for pane in &panes {
        // A non-focused pane draws from its parked state. Falling back to the focused view
        // would draw the same pane twice, which is worse than an empty rectangle.
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
    if let Some(max) = focused_detail_max {
        app.view.detail_max_scroll = max;
        app.view.detail_scroll = app.view.detail_scroll.min(max);
    }

    statusline::render_status(frame, status, app, &theme);

    // Overlays, outermost last. They belong to the session, not to a pane.
    // Only `:` has completions to show; `!` takes a shell command and draws in the
    // statusline alone.
    if app
        .prompt
        .as_ref()
        .is_some_and(|p| p.kind == PromptKind::Command)
    {
        cmdline::render(frame, app, &theme);
    }
    // The picker docks over the bottom of the panes. Above which-key and help, because
    // while it is open it owns the keyboard and they cannot be reached anyway.
    if let Some(p) = &app.picker {
        picker::render(frame, p, &theme);
    }
    if !app.which_key.is_empty() {
        whichkey::render(frame, app, &theme);
    }
    if app.show_help {
        help::render(frame, app, &theme);
    }
    if let Some(f) = &app.form {
        form::render(frame, f, &theme);
    }
    // Outermost of all: while this is up nothing else can be acted on, so nothing else
    // should be able to sit over it.
    if let Some(c) = &app.confirm {
        confirm::render(frame, c, &theme);
    }
}

/// How a pane divides its own rectangle.
struct PaneAreas {
    query: ratatui::layout::Rect,
    list: ratatui::layout::Rect,
    detail: Option<ratatui::layout::Rect>,
}

/// Below this width a side-by-side payload pane leaves neither half readable, so `right`
/// falls back to stacking.
const MIN_SIDE_BY_SIDE_WIDTH: u16 = 100;

fn pane_body(
    area: ratatui::layout::Rect,
    screen: Screen,
    show_detail: bool,
    payload: PayloadPane,
) -> PaneAreas {
    // The query bar is part of the workflow screen's chrome, not an overlay: it is always
    // on screen so the query is never something you have to go and open.
    let query_height = match screen {
        Screen::Workflows => 1,
        Screen::Namespaces | Screen::History | Screen::Schedules => 0,
    };
    let [query, rest] =
        Layout::vertical([Constraint::Length(query_height), Constraint::Min(1)]).areas(area);

    if screen == Screen::History && show_detail {
        // Roughly half each: enough list to keep your place, enough pane to read a payload
        // without scrolling for every value.
        let [list, detail] =
            if payload == PayloadPane::Right && rest.width >= MIN_SIDE_BY_SIDE_WIDTH {
                Layout::horizontal([Constraint::Min(40), Constraint::Percentage(50)]).areas(rest)
            } else {
                Layout::vertical([Constraint::Min(3), Constraint::Percentage(50)]).areas(rest)
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

/// Draw one window. Returns the payload pane's scroll extent, when it drew one.
fn render_pane(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    view: &View,
    app: &App,
    theme: &Theme,
    focused: bool,
    split: bool,
) -> Option<usize> {
    // With one window there is nothing to distinguish, and a border would only cost a row.
    let area = if split {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(if focused { theme.accent } else { theme.faint }));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let areas = pane_body(area, view.screen, view.show_detail, app.payload_pane());
    match view.screen {
        Screen::Namespaces => namespaces::render(frame, areas.list, view, app, theme),
        Screen::Workflows => {
            query::render(frame, areas.query, view, app, theme, focused);
            workflows::render(frame, areas.list, view, app, theme);
            // After the rows, because it sits over them.
            if focused && let Some(c) = &app.completion {
                complete::render(frame, areas.list, area, c, theme);
            }
        }
        Screen::Schedules => schedules::render(frame, areas.list, view, app, theme),
        Screen::History => {
            history::render(frame, areas.list, view, app, theme);
            if let Some(mut pane) = areas.detail {
                // Beside the list, the pane's own top rule does not separate it from the
                // rows to its left, so it gets a vertical one as well.
                if pane.x > areas.list.x {
                    let rule = Block::default()
                        .borders(Borders::LEFT)
                        .border_style(Style::new().fg(theme.faint));
                    let inner = rule.inner(pane);
                    frame.render_widget(rule, pane);
                    pane = inner;
                }
                return Some(detail::render(frame, pane, view, app, theme));
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

/// Draw a scrollbar for a list, and give back the width the rows may use.
///
/// The bar only appears when there is something to scroll, so a list that fits looks
/// exactly as it did before. A borderless pane has no spare column for it, so one is taken
/// out of the content: the alternative is drawing the thumb over the rightmost column,
/// which here holds the age, and a corrupted value is worse than a narrower one.
///
/// The decision is made from the row count alone, before any width is computed, so the
/// columns do not shift as the cursor moves.
fn list_scrollbar(frame: &mut Frame, area: Rect, cursor: usize, total: usize, t: &Theme) -> Rect {
    let height = area.height as usize;
    if total <= height || area.width < 4 {
        return area;
    }
    let body = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    // `width: 1` and not a struct update from `area`: a full-width rect would put the
    // right-hand bar off the end of it, where it is clipped away entirely.
    let bar = Rect {
        x: area.x + area.width - 1,
        y: area.y,
        width: 1,
        height: area.height,
    };
    draw_scrollbar(frame, bar, cursor, total, height, t);
    body
}

/// Draw a scrollbar over a bordered area's right edge.
///
/// For the overlays: the thumb rides the border it already has, so nothing gives up a
/// column and the pane is the same size whether or not it is scrolling.
fn border_scrollbar(frame: &mut Frame, area: Rect, position: usize, total: usize, t: &Theme) {
    let height = area.height.saturating_sub(2) as usize;
    if total <= height || height == 0 {
        return;
    }
    draw_scrollbar(frame, area, position, total, height, t);
}

/// No arrows, no track: an unscrolled pane should look untouched, and the thumb alone says
/// both how far down this is and how much of the whole it covers.
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
            .thumb_style(Style::new().fg(t.faint)),
        area,
        &mut state,
    );
}

/// The hybrid relative/absolute gutter, matching `set relativenumber number`: the cursor
/// row shows its own 1-based index, every other row its distance. That is what makes a
/// count like `7j` something you read off the screen rather than estimate.
fn gutter(i: usize, cursor: usize) -> String {
    if i == cursor {
        format!("{:>4} ", i + 1)
    } else {
        format!("{:>4} ", i.abs_diff(cursor))
    }
}

/// Shorten to `max` characters, on a character boundary, with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// A centred box `w` x `h`, clamped to the frame.
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
