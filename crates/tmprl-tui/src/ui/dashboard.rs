//! The dashboard: a board's panels, each a titled box of the items the cursor runs through.
//!
//! When the panels do not all fit, the ones that are drawn are the ones around the cursor,
//! so moving through the items scrolls the dashboard the way it scrolls a list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use tmprl_core::Loadable;
use tmprl_core::dashboard::{Board, Bucket, Item, PanelKind, QueueRef, Size, TimeField};
use tmprl_core::schedule::time_until;
use tmprl_core::workflow::humanize_age_ms;
use tmprl_ui::{Axis, Track, tracks};

use super::{from_ui, highlight, match_style, status_style, to_ui, truncate, wrap};
use crate::app::{App, now_ms};
use crate::theme::Theme;
use crate::view::View;

const MIN_WIDTH: u16 = 24;
const MIN_HEIGHT: u16 = 3;
const TYPE: usize = 20;
const AGE: usize = 4;
/// A workflow id beside a reason: wide enough for a UUID.
const ID: usize = 36;
/// The least room worth giving a reason.
const REASON: usize = 12;
/// An attempt count, `12/∞`.
const TRIES: usize = 7;
/// When the next try is, `next 45s`, or the state the activity is in.
const NEXT: usize = 10;
/// The lines a histogram is worth: four of columns and the axis under them.
const CHART: usize = 5;
/// The lines kept for a panel with nothing in it, whose message may wrap.
const EMPTY: usize = 2;

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme, focused: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(board) = &view.dashboard else {
        return;
    };
    let (cursor_panel, cursor_item) = board.locate(view.cursor).unwrap_or((0, 0));

    // Too small for one bordered panel: the panel under the cursor, without its box.
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let panel = Panel {
            board,
            index: cursor_panel,
            cursor: Some(cursor_item),
        };
        panel.body(frame, area, view, app, t);
        return;
    }

    let rows = &board.layout().rows;
    let firsts: Vec<usize> = rows
        .iter()
        .scan(0, |next, row| {
            let first = *next;
            *next += row.panels.len();
            Some(first)
        })
        .collect();
    let cursor_row = firsts
        .iter()
        .rposition(|first| *first <= cursor_panel)
        .unwrap_or(0);

    let heights: Vec<Track> = rows
        .iter()
        .zip(&firsts)
        .map(|(row, first)| match row.size {
            Size::Weight(w) => match content(board, *first, row.panels.len()) {
                Some(lines) => Track::Fit {
                    weight: w,
                    cells: lines.saturating_add(2),
                },
                None => Track::Weight(w),
            },
            Size::Lines(n) => Track::Cells(n),
        })
        .collect();
    let (first_row, row_rects) = window(to_ui(area), Axis::Rows, &heights, MIN_HEIGHT, cursor_row);

    let mut drawn = 0;
    let mut last = None;
    for (offset, row_rect) in row_rects.into_iter().enumerate() {
        let r = first_row + offset;
        let widths: Vec<Track> = rows[r]
            .panels
            .iter()
            .map(|p| Track::Weight(p.width))
            .collect();
        let must = if r == cursor_row {
            cursor_panel - firsts[r]
        } else {
            0
        };
        let (first_col, rects) = window(row_rect, Axis::Columns, &widths, MIN_WIDTH, must);
        for (offset, rect) in rects.into_iter().enumerate() {
            let index = firsts[r] + first_col + offset;
            let panel = Panel {
                board,
                index,
                cursor: (index == cursor_panel && !board.is_empty()).then_some(cursor_item),
            };
            panel.render(frame, from_ui(rect), view, app, t, focused);
            drawn += 1;
            last = Some(from_ui(rect));
        }
    }

    let hidden = board.panel_count() - drawn;
    if hidden > 0
        && let Some(rect) = last
    {
        let text = format!(" +{hidden} hidden ");
        let width = text.chars().count() as u16;
        if rect.width > width + 2 {
            let at = Rect::new(rect.right() - width - 1, rect.bottom() - 1, width, 1);
            frame.render_widget(Paragraph::new(Span::styled(text, t.warn)), at);
        }
    }
}

/// A queue's health in a few words, and how loudly to say it: a backlog nothing is polling
/// is the one state the panel exists to make impossible to miss.
///
/// `room` is what the line can spare. The age of the backlog is the first thing to go:
/// that there is one, and whether anything polls, matter more than how old it is.
fn queue_health(queue: &QueueRef, room: usize, t: &Theme) -> (String, Style) {
    let Some(health) = &queue.health else {
        return (String::new(), t.faint);
    };
    let pollers = match health.pollers {
        0 => "no pollers".to_string(),
        1 => "1 poller".to_string(),
        n => format!("{n} pollers"),
    };
    let text = match (health.backlog, health.backlog_age_ms) {
        (Some(0) | None, _) => pollers,
        (Some(n), Some(age)) if age > 0 => {
            let full = format!("backlog {n}, oldest {}  {pollers}", humanize_age_ms(age));
            if full.chars().count() <= room {
                full
            } else {
                format!("backlog {n}  {pollers}")
            }
        }
        (Some(n), _) => format!("backlog {n}  {pollers}"),
    };
    let style = if health.stuck() {
        t.err
    } else if health.pollers == 0 || health.backlog.is_some_and(|n| n > 0) {
        t.warn
    } else {
        t.dim
    };
    (text, style)
}

/// Cut `area` into as many of `list` as fit, starting far enough along that `must` is one
/// of them.
fn window(
    area: tmprl_ui::Rect,
    axis: Axis,
    list: &[Track],
    min: u16,
    must: usize,
) -> (usize, Vec<tmprl_ui::Rect>) {
    for start in 0..must {
        let rects = tracks(area, axis, &list[start..], min);
        if start + rects.len() > must {
            return (start, rects);
        }
    }
    (must, tracks(area, axis, &list[must.min(list.len())..], min))
}

/// The lines the tallest of a row's panels has to show, so the row can leave the rest to
/// rows with more. `None` until every panel has its answer: a row is not resized around a
/// panel that is still loading, or one that has an error to spell out.
fn content(board: &Board, first: usize, panels: usize) -> Option<u16> {
    (first..first + panels)
        .map(|panel| {
            board.state(panel)?.value()?;
            let lines = match board.spec(panel)?.kind {
                PanelKind::Counts { .. } => 1,
                PanelKind::Histogram { .. } => CHART,
                PanelKind::Workflows { .. }
                | PanelKind::Types { .. }
                | PanelKind::Queues { .. }
                | PanelKind::Retrying { .. }
                | PanelKind::Schedules { .. } => board.items(panel).len().max(EMPTY),
            };
            Some(u16::try_from(lines).unwrap_or(u16::MAX))
        })
        .try_fold(0, |tallest, lines| Some(lines?.max(tallest)))
}

struct Panel<'a> {
    board: &'a Board,
    index: usize,
    /// The item the cursor is on, when it is in this panel.
    cursor: Option<usize>,
}

impl Panel<'_> {
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        app: &App,
        t: &Theme,
        focused: bool,
    ) {
        let Some(spec) = self.board.spec(self.index) else {
            return;
        };
        let mut title = vec![Span::styled(format!(" {} ", spec.title()), t.accent)];
        if let Some(n) = self.board.discovered(self.index) {
            title.push(Span::styled(format!("names from {n} "), t.dim));
        } else if let Some(n) = self.board.sampled(self.index) {
            title.push(Span::styled(format!("of {n} sampled "), t.dim));
        }
        if let Some(peak) = self
            .board
            .buckets(self.index)
            .iter()
            .filter_map(|b| b.count)
            .max()
        {
            title.push(Span::styled(format!("peak {peak} "), t.dim));
        }
        if self.board.fault(self.index).is_some() && !self.board.items(self.index).is_empty() {
            let age = self
                .board
                .state(self.index)
                .and_then(Loadable::age)
                .map(|age| humanize_age_ms(age.as_millis() as i64))
                .unwrap_or_default();
            title.push(Span::styled(format!("stale {age} "), t.warn));
        }
        let block =
            Block::bordered()
                .title(Line::from(title))
                .border_style(if self.cursor.is_some() {
                    t.border(focused)
                } else {
                    t.faint
                });
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.body(frame, inner, view, app, t);
    }

    fn body(&self, frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let buckets = self.board.buckets(self.index);
        if !buckets.is_empty() {
            self.chart(frame, area, &buckets, app, t);
            return;
        }
        let items = self.board.items(self.index);
        if items.is_empty() {
            let lines: Vec<Line> = wrap(&self.empty(), area.width.saturating_sub(1) as usize)
                .into_iter()
                .map(|l| Line::from(Span::styled(format!(" {l}"), t.faint)))
                .collect();
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }

        let first_index = self.board.first_of(self.index);
        let style = |i: usize, base: Style| {
            if self.cursor == Some(i) {
                base.patch(t.sel).add_modifier(Modifier::BOLD)
            } else if view.is_selected(first_index + i) {
                base.patch(t.sel)
            } else {
                base
            }
        };

        // Counts read across, like the header of the workflow list, so the row they are
        // given can be three lines tall.
        if let Some(PanelKind::Counts { .. }) = self.board.spec(self.index).map(|s| &s.kind) {
            let mut spans = vec![Span::raw(" ")];
            for (i, item) in items.iter().enumerate() {
                if let Item::Status { status, count } = item {
                    spans.push(Span::styled(
                        format!("{} {count} {}", status.glyph(), status.query_name()),
                        style(i, status_style(*status, t)),
                    ));
                    spans.push(Span::raw("   "));
                }
            }
            if let Some(Loadable::Loaded(tmprl_core::dashboard::SourceData::Counts(c), _)) =
                self.board.state(self.index)
            {
                spans.push(Span::styled(format!("{} total", c.total), t.dim));
            }
            let line = highlight(Line::from(spans), &app.search, match_style());
            frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), area);
            return;
        }

        let height = area.height as usize;
        let first = self
            .cursor
            .unwrap_or(0)
            .saturating_sub(height.saturating_sub(1) / 2)
            .min(items.len().saturating_sub(height));
        let width = (area.width as usize).saturating_sub(1);
        let now = now_ms();
        let fanned_out = view.is_fanned_out();
        let longest_activity = items
            .iter()
            .filter_map(|item| match item {
                Item::Retry { activity, .. } => Some(activity.activity_type.chars().count()),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .min(TYPE + 8);
        // A column that says the same thing on every line says nothing.
        let mut types = items.iter().filter_map(|item| match item {
            Item::Workflow(w) => Some(w.workflow_type.as_str()),
            _ => None,
        });
        let one_type = types
            .next()
            .is_some_and(|first| types.all(|other| other == first))
            && items.len() > 1;
        let tally = |n: usize, exact: bool| {
            if exact {
                n.to_string()
            } else {
                format!("~{n}")
            }
        };
        let num = items
            .iter()
            .map(|item| match item {
                Item::Type { count, exact, .. } => tally(*count, *exact).len(),
                Item::Queue(q) => tally(q.running, q.exact).len(),
                Item::Status { .. }
                | Item::Workflow(_)
                | Item::Schedule(_)
                | Item::Retry { .. }
                | Item::Bucket { .. } => 0,
            })
            .max()
            .unwrap_or(0)
            .max(4);

        let lines: Vec<Line> = items
            .iter()
            .enumerate()
            .skip(first)
            .take(height)
            .map(|(i, item)| {
                let base = style(i, t.fg);
                let mut spans = vec![Span::styled(" ", base)];
                match item {
                    Item::Status { status, count } => spans.push(Span::styled(
                        format!("{} {count} {}", status.glyph(), status.query_name()),
                        style(i, status_style(*status, t)),
                    )),
                    Item::Workflow(w) => {
                        let show_type = !one_type && width >= 2 + 16 + TYPE + AGE + 2;
                        let fixed = 2 + AGE + 1 + if show_type { TYPE + 1 } else { 0 };
                        let room = width.saturating_sub(fixed).max(4);
                        // A reason takes what an id does not need, when that is worth having.
                        let reason = self
                            .board
                            .reason(w)
                            .filter(|_| room >= ID + 2 + REASON)
                            .map(|text| truncate(text, room - ID - 2));
                        let id_width = if reason.is_some() { ID } else { room };
                        spans.push(Span::styled(
                            format!("{} ", w.status.glyph()),
                            style(i, status_style(w.status, t)),
                        ));
                        spans.push(Span::styled(
                            format!("{:<id_width$} ", truncate(&w.workflow_id, id_width)),
                            base,
                        ));
                        if let Some(reason) = reason {
                            let reason_width = room - ID - 2;
                            spans.push(Span::styled(
                                format!(" {reason:<reason_width$} "),
                                style(i, t.dim),
                            ));
                        }
                        if show_type {
                            spans.push(Span::styled(
                                format!("{:<TYPE$} ", truncate(&w.workflow_type, TYPE)),
                                style(i, t.dim),
                            ));
                        }
                        let stamp = match self.board.spec(self.index).map(|s| &s.kind) {
                            Some(PanelKind::Workflows { window, .. })
                                if window.by == TimeField::Close =>
                            {
                                w.close_time
                            }
                            _ => w.start_time,
                        };
                        let age = stamp.map(|s| humanize_age_ms(now - s)).unwrap_or_default();
                        spans.push(Span::styled(format!("{age:>AGE$}"), style(i, t.faint)));
                    }
                    Item::Type { name, count, exact } => {
                        spans.push(Span::styled(
                            format!("{:>num$}  ", tally(*count, *exact)),
                            style(i, if *exact { t.err } else { t.dim }),
                        ));
                        spans.push(Span::styled(
                            truncate(name, width.saturating_sub(num + 2)),
                            base,
                        ));
                    }
                    Item::Queue(q) => {
                        // The count and its gap, two before the health, eight for a name.
                        let (health, health_style) =
                            queue_health(q, width.saturating_sub(num + 2 + 2 + 8), t);
                        let name_width = width.saturating_sub(num + 2 + health.chars().count() + 2);
                        spans.push(Span::styled(
                            format!("{:>num$}  ", tally(q.running, q.exact)),
                            style(i, if q.exact { t.accent } else { t.dim }),
                        ));
                        spans.push(Span::styled(truncate(&q.name, name_width.max(4)), base));
                        if !health.is_empty() {
                            spans.push(Span::styled(format!("  {health}"), style(i, health_style)));
                        }
                        if fanned_out {
                            spans.push(Span::styled(format!("  {}", q.namespace), style(i, t.dim)));
                        }
                    }
                    Item::Schedule(s) => {
                        let next = if s.paused {
                            "paused".to_string()
                        } else {
                            time_until(s.next_run, now).unwrap_or_default()
                        };
                        let id_width = width.saturating_sub(2 + 1 + 7).max(4);
                        spans.push(Span::styled(
                            format!("{} ", s.glyph()),
                            style(i, if s.paused { t.warn } else { t.accent }),
                        ));
                        spans.push(Span::styled(
                            format!("{:<id_width$} ", truncate(&s.schedule_id, id_width)),
                            base,
                        ));
                        spans.push(Span::styled(
                            format!("{next:>7}"),
                            style(i, if s.paused { t.warn } else { t.faint }),
                        ));
                    }
                    Item::Retry { row, activity } => {
                        let tries = if activity.maximum_attempts > 0 {
                            format!("{}/{}", activity.attempt, activity.maximum_attempts)
                        } else {
                            format!("{}/∞", activity.attempt)
                        };
                        let next = time_until(activity.next_attempt_at, now)
                            .map(|until| format!("next {until}"))
                            .unwrap_or_else(|| activity.state.label().to_string());
                        let why = activity
                            .last_failure
                            .as_ref()
                            .map(|failure| failure.root().headline())
                            .unwrap_or_default();
                        // The id, the activity and its tries always. The failure takes what
                        // is left, and when to expect the next try closes the line.
                        let fixed = 2 + TRIES + 1 + NEXT;
                        let room = width.saturating_sub(fixed).max(8);
                        let id_width = room.min(ID).max(room / 3).min(room);
                        let name_width = room.saturating_sub(id_width + 1).min(longest_activity);
                        let why_width = room.saturating_sub(id_width + 1 + name_width + 1);
                        spans.push(Span::styled("↻ ", style(i, t.warn)));
                        spans.push(Span::styled(
                            format!("{:<id_width$} ", truncate(&row.workflow_id, id_width)),
                            base,
                        ));
                        spans.push(Span::styled(
                            format!(
                                "{:<name_width$} ",
                                truncate(&activity.activity_type, name_width)
                            ),
                            style(i, t.accent),
                        ));
                        spans.push(Span::styled(format!("{tries:>TRIES$} "), style(i, t.err)));
                        spans.push(Span::styled(
                            format!("{:<why_width$}", truncate(&why, why_width)),
                            style(i, t.dim),
                        ));
                        spans.push(Span::styled(
                            format!("{:>NEXT$}", truncate(&next, NEXT)),
                            style(i, t.faint),
                        ));
                    }
                    // Drawn as columns by `chart`, never as a line.
                    Item::Bucket { .. } => {}
                }
                highlight(Line::from(spans), &app.search, match_style())
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// A histogram: one column for each stretch of time, tall as its count against the
    /// tallest, the newest on the right. When they do not all fit the oldest are left off.
    fn chart(&self, frame: &mut Frame, area: Rect, buckets: &[Bucket], app: &App, t: &Theme) {
        const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let width = (area.width as usize).saturating_sub(2);
        if width == 0 {
            return;
        }
        let step = (width / buckets.len()).clamp(1, 3);
        let bar = step.saturating_sub(1).max(1);
        let shown = &buckets[buckets.len().saturating_sub(width / step)..];
        let axis = area.height >= 3;
        let rows = area.height as usize - usize::from(axis);
        let peak = shown.iter().filter_map(|b| b.count).max().unwrap_or(0);
        // In eighths of a line, rounded up, so that one workflow still shows.
        let height = |count: i64| -> usize {
            if count <= 0 || peak <= 0 {
                return 0;
            }
            let full = (rows * 8) as i64;
            ((count * full + peak - 1) / peak).clamp(1, full) as usize
        };
        let under_cursor = self
            .cursor
            .and_then(|i| self.board.items(self.index).get(i))
            .and_then(|item| match item {
                Item::Bucket { from_ms, .. } => Some(*from_ms),
                _ => None,
            });

        let mut lines: Vec<Line> = (0..rows)
            .map(|row| {
                let below = (rows - 1 - row) * 8;
                let mut spans = vec![Span::raw(" ")];
                for bucket in shown {
                    let (symbol, style) = match bucket.count.map(height) {
                        None if below == 0 => ('░', t.faint),
                        Some(0) if below == 0 => ('·', t.faint),
                        Some(tall) if tall > below => (LEVELS[(tall - below).min(8) - 1], t.accent),
                        None | Some(_) => (' ', t.fg),
                    };
                    let style = if under_cursor == Some(bucket.from_ms) {
                        style.patch(t.sel).add_modifier(Modifier::BOLD)
                    } else {
                        style
                    };
                    spans.push(Span::styled(symbol.to_string().repeat(bar), style));
                    if step > bar {
                        spans.push(Span::raw(" "));
                    }
                }
                Line::from(spans)
            })
            .collect();

        if axis && let (Some(first), Some(last)) = (shown.first(), shown.last()) {
            let label = |ms: i64| -> String {
                // A time of day says nothing on an axis that runs over several days.
                if last.to_ms - first.from_ms > 2 * 86_400_000 {
                    app.clock.stamp(Some(ms)).chars().take(5).collect()
                } else {
                    app.clock.time_of_day(ms).chars().take(5).collect()
                }
            };
            let mut cells = vec![' '; shown.len() * step];
            let mut put = |at: usize, text: &str| {
                let text: Vec<char> = text.chars().collect();
                let free = cells
                    .get(at.saturating_sub(1)..(at + text.len() + 1).min(cells.len()))
                    .is_some_and(|span| span.iter().all(|c| *c == ' '));
                if free && at + text.len() <= cells.len() {
                    cells[at..at + text.len()].copy_from_slice(&text);
                }
            };
            let newest = if last.to_ms > now_ms() {
                "now".to_string()
            } else {
                label(last.to_ms)
            };
            put(0, &label(first.from_ms));
            put(
                (shown.len() * step).saturating_sub(newest.chars().count()),
                &newest,
            );
            let middle = shown.len() / 2;
            put(middle * step, &label(shown[middle].from_ms));
            lines.push(Line::from(Span::styled(
                format!(" {}", cells.into_iter().collect::<String>()),
                t.faint,
            )));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// What a panel with no items says: that it is waiting, why it failed, or what it
    /// would have shown.
    fn empty(&self) -> String {
        match self.board.state(self.index) {
            Some(Loadable::Loading) => "loading…".to_string(),
            Some(Loadable::Failed(fault)) => super::failed(fault),
            Some(Loadable::NotAsked) | None => String::new(),
            Some(Loadable::Loaded(..)) => match self.board.spec(self.index).map(|s| &s.kind) {
                Some(PanelKind::Counts { .. }) => "no workflows",
                Some(PanelKind::Workflows { .. }) | Some(PanelKind::Types { .. }) => "none",
                Some(PanelKind::Queues { .. }) => "no running workflows to find queues on",
                Some(PanelKind::Schedules { .. }) => "no schedules",
                Some(PanelKind::Histogram { .. }) => "nothing in this window",
                Some(PanelKind::Retrying { .. }) if self.board.looking() => "looking…",
                Some(PanelKind::Retrying { .. }) => "nothing retrying that often",
                None => "",
            }
            .to_string(),
        }
    }
}
