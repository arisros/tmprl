//! The history as a timeline, `<leader>G`.
//!
//! Drawn after the timeline in Temporal's web UI so the two read the same way: each group
//! is its events as dots on one shared time axis, joined by a line in the colour of how
//! the group ended. The queue time before an activity started is the faded first stretch,
//! a group still running trails a dashed line to the edge, and idle stretches are folded
//! (`≀`) rather than drawn to scale.
//!
//! It is the same outline as the list view, row for row, so folding, search, `]f`, `K` and
//! the cursor all behave the same; only the columns after the gutter differ.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tmprl_core::history::{Category, Group, NormalizedEvent, Outcome, Role};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::timeline::{Scale, Timeline, format_offset};

use super::history::category_label;
use super::{highlight, match_style};
use crate::app::{App, now_ms};

use crate::theme::Theme;
use crate::view::View;

/// The web UI's colours, from `temporalio/ui`: `lines-and-dots/colors.ts` and the dark
/// theme's `colorScales`. Kept as hex so a change there is a diff here.
mod web {
    use ratatui::style::Color;

    pub const GREEN_9: Color = Color::Rgb(0x30, 0xa4, 0x6c);
    pub const RED_9: Color = Color::Rgb(0xe5, 0x48, 0x4d);
    pub const RED_11: Color = Color::Rgb(0xce, 0x2c, 0x31);
    pub const PERSIMMON_8: Color = Color::Rgb(0xff, 0x99, 0x6c);
    pub const PERSIMMON_9: Color = Color::Rgb(0xec, 0x58, 0x00);
    pub const AMBER_9: Color = Color::Rgb(0xff, 0xc5, 0x3d);
    pub const TANGERINE_9: Color = Color::Rgb(0xff, 0xa8, 0x00);
    pub const BLUE_9: Color = Color::Rgb(0x00, 0x90, 0xff);
    pub const PINK_9: Color = Color::Rgb(0xff, 0x46, 0xa2);
    pub const PEACOCK_9: Color = Color::Rgb(0x35, 0x89, 0x89);
    pub const INDIGO_3: Color = Color::Rgb(0xed, 0xf2, 0xfe);
    pub const ZAFFRE_7: Color = Color::Rgb(0x84, 0xa7, 0xf0);
    /// The web UI draws activities in dark-magenta 9 (`#8b008b`) and workflows in zaffre 9
    /// (`#0014a8`). As a one-cell line on a dark terminal both all but vanish, so these are
    /// the same hues from shade 7.
    pub const MAGENTA_7: Color = Color::Rgb(0xdb, 0x9c, 0xd7);
}

/// What the faded colours are blended towards. The terminal's own background is not
/// knowable, so this is the dark it most likely is.
const BACKGROUND: Color = Color::Rgb(0x16, 0x18, 0x1d);
/// The web UI draws the queued stretch of a completed activity at this opacity.
const QUEUED_OPACITY: f32 = 0.35;

const LINE: char = '━';
const QUEUED: char = '┄';
const TAIL: char = '╍';
const DOT: char = '●';
const WORKFLOW_DOT: char = '◆';
const GRID: char = '┊';
const FOLD: char = '≀';

/// Gutter (`{:>4} `) plus the fold marker.
const PREFIX: usize = 7;

pub fn render(frame: &mut Frame, area: Rect, outline: &Outline, view: &View, app: &App, t: &Theme) {
    let message = |frame: &mut Frame, msg: &str| {
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), Style::new().fg(t.faint))),
            area,
        )
    };
    let width = (area.width as usize).saturating_sub(PREFIX + 1);
    if area.height < 2 || width < 8 {
        return message(frame, "too narrow for the timeline");
    }
    let Some(mut timeline) = Timeline::new(outline.groups(), now_ms()) else {
        return message(frame, "no event times to lay out");
    };
    if view.timeline_gaps_open {
        timeline.fold_gaps(false);
    }
    let scale = timeline.scale(width);
    let base = base_row(&scale, t);

    let height = area.height as usize - 1;
    let first = view
        .cursor
        .saturating_sub(height.saturating_sub(1) / 2)
        .min(outline.len().saturating_sub(height));

    let mut lines = vec![axis(&timeline, &scale, app, t)];
    lines.extend(
        outline
            .slice(first, height)
            .into_iter()
            .enumerate()
            .map(|(n, row)| {
                let index = first + n;
                let mut canvas = base.clone();
                let marker = match row {
                    Row::Group { group, expanded } => {
                        let Some(g) = outline.group(group) else {
                            return Line::default();
                        };
                        draw_group(&mut canvas, outline, g, &scale, index == view.cursor, t);
                        match (g.events.len() > 1, expanded) {
                            (false, _) => "  ",
                            (true, true) => "▾ ",
                            (true, false) => "▸ ",
                        }
                    }
                    Row::Event { group, event } => {
                        if let (Some(g), Some(e)) = (outline.group(group), outline.event(event)) {
                            draw_event(&mut canvas, g, e, &scale, t);
                        }
                        "  "
                    }
                };
                let focused = index == view.cursor;
                let mut spans = vec![
                    Span::styled(
                        super::gutter(index, view.cursor),
                        Style::new().fg(if focused { t.warn } else { t.faint }),
                    ),
                    Span::styled(marker, Style::new().fg(t.faint)),
                ];
                spans.extend(canvas.into_spans());
                let line = Line::from(spans);
                if focused || view.is_selected(index) {
                    line.style(Style::new().bg(t.sel))
                } else {
                    line
                }
            })
            .map(|l| highlight(l, &app.search, match_style())),
    );

    frame.render_widget(Paragraph::new(lines), area);
}

/// One row of cells, before anything is drawn on it.
#[derive(Clone)]
struct Canvas(Vec<(char, Style)>);

impl Canvas {
    fn width(&self) -> usize {
        self.0.len()
    }

    fn put(&mut self, column: usize, ch: char, style: Style) {
        if let Some(cell) = self.0.get_mut(column) {
            *cell = (ch, style);
        }
    }

    /// Write `text` from `column`, clipped at the right edge.
    fn text(&mut self, column: usize, text: &str, style: Style) {
        for (i, ch) in text.chars().enumerate() {
            self.put(column + i, ch, style);
        }
    }

    /// Runs of one style become one span, so a row is a handful of spans, not a hundred.
    fn into_spans(self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut run = String::new();
        let mut style = None;
        for (ch, s) in self.0 {
            if style.is_some_and(|cur| cur != s) {
                spans.push(Span::styled(
                    std::mem::take(&mut run),
                    style.unwrap_or_default(),
                ));
            }
            style = Some(s);
            run.push(ch);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, style.unwrap_or_default()));
        }
        spans
    }
}

/// The empty row: grid lines under each tick and the fold marks, which every row shares.
fn base_row(scale: &Scale, t: &Theme) -> Canvas {
    let mut c = Canvas(vec![(' ', Style::new()); scale.width()]);
    let grid = Style::new().fg(mix(t.faint, BACKGROUND, 0.5));
    for tick in scale.ticks() {
        c.put(tick.column, GRID, grid);
    }
    for fold in scale.folds() {
        for col in fold {
            c.put(col, FOLD, Style::new().fg(t.faint));
        }
    }
    c
}

/// The axis: offsets from the start of the run, or clock times under `<leader>T`.
fn axis(timeline: &Timeline, scale: &Scale, app: &App, t: &Theme) -> Line<'static> {
    let mut c = Canvas(vec![(' ', Style::new()); scale.width()]);
    let ticks = scale.ticks();
    let labels = |fine: bool| -> Vec<String> {
        ticks
            .iter()
            .map(|tick| {
                if app.times.is_absolute() {
                    app.clock.time_of_day(tick.at)
                } else {
                    format_offset(tick.at - timeline.start(), fine)
                }
            })
            .collect()
    };
    // Milliseconds only once whole seconds would print two neighbouring ticks the same.
    let mut text = labels(false);
    if text.windows(2).any(|w| w[0] == w[1]) {
        text = labels(true);
    }
    let label_style = Style::new().fg(t.dim);
    let mut free_from = 0;
    for (tick, label) in ticks.iter().zip(text) {
        let len = label.chars().count();
        // Centred on the tick, like the web axis, and dropped rather than overlapped.
        let from = tick.column.saturating_sub(len / 2);
        if from < free_from || from + len > c.width() {
            continue;
        }
        c.text(from, &label, label_style);
        free_from = from + len + 1;
    }
    for fold in scale.folds() {
        for col in fold {
            c.put(col, FOLD, Style::new().fg(t.faint));
        }
    }
    let mut spans = vec![Span::raw(" ".repeat(PREFIX))];
    spans.extend(c.into_spans());
    Line::from(spans)
}

/// A group: its events as dots, joined, and its name beside them.
fn draw_group(
    c: &mut Canvas,
    outline: &Outline,
    g: &Group,
    scale: &Scale,
    focused: bool,
    t: &Theme,
) {
    let events: Vec<&NormalizedEvent> = g
        .events
        .iter()
        .filter_map(|id| outline.event_by_id(*id))
        .filter(|e| e.time.is_some())
        .collect();
    let points: Vec<usize> = events
        .iter()
        .map(|e| scale.column(e.time.unwrap_or_default()))
        .collect();
    let Some(&last) = points.last() else {
        return;
    };
    let line = line_color(g);
    let pending = g.is_open();

    for (i, pair) in points.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let queued = i == 0 && queue_is_first(g, &events);
        let gradient = g.attempts > 1 && g.outcome == Outcome::Completed;
        for col in a..=b {
            let (ch, color) = if queued {
                (QUEUED, mix(line, BACKGROUND, 1.0 - QUEUED_OPACITY))
            } else if gradient {
                // A retried activity that got there in the end: the web UI fades each
                // stretch from failure on the left to success on the right.
                let at = if b > a {
                    (col - a) as f32 / (b - a) as f32
                } else {
                    1.0
                };
                (LINE, mix(web::RED_9, web::GREEN_9, at))
            } else {
                (LINE, line)
            };
            c.put(col, ch, Style::new().fg(color));
        }
    }
    if pending {
        let tail = Style::new().fg(line);
        for col in last..c.width() {
            c.put(col, TAIL, tail);
        }
    }
    let dot = if g.category == Category::Workflow {
        WORKFLOW_DOT
    } else {
        DOT
    };
    for (e, col) in events.iter().zip(&points) {
        c.put(*col, dot, Style::new().fg(dot_color(g, e)));
    }

    let label = label(g);
    let style = if focused {
        Style::new().fg(t.fg).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(t.fg)
    };
    let len = label.chars().count();
    let at = label_column(&points, c.width(), pending, len);
    // Padded with a space either side, the terminal's version of the web label's pill: it
    // keeps the name from running into the line it sits on.
    c.text(at.saturating_sub(1), &format!(" {label} "), style);
}

/// One event of an expanded group: a single dot on the same axis, and its name.
fn draw_event(c: &mut Canvas, g: &Group, e: &NormalizedEvent, scale: &Scale, t: &Theme) {
    let Some(time) = e.time else {
        return;
    };
    let col = scale.column(time);
    c.put(col, DOT, Style::new().fg(dot_color(g, e)));
    let at = label_column(&[col], c.width(), false, e.name.chars().count());
    c.text(
        at.saturating_sub(1),
        &format!(" {} ", e.name),
        Style::new().fg(t.dim),
    );
}

/// Where a label starts, by the web UI's rule (`timelineTextPosition`): before the dots
/// when they are in the right half, after them when they end early enough, and otherwise
/// on the line itself, after whichever of the first two stretches is shorter.
fn label_column(points: &[usize], width: usize, pending: bool, len: usize) -> usize {
    let (first, last) = (points[0], points[points.len() - 1]);
    if first > width / 2 {
        return first.saturating_sub(len + 1);
    }
    if last < width * 2 / 3 && !pending {
        return last + 2;
    }
    let mut at = first + 2;
    if points.len() == 2 && pending && points[1] - points[0] < width - points[1] {
        at = points[1] + 2;
    }
    if points.len() > 2 && points[2] - points[1] > points[1] - points[0] {
        at = points[1] + 2;
    }
    at
}

fn label(g: &Group) -> String {
    let name = if g.subject.is_empty() {
        category_label(g.category)
    } else {
        g.subject.as_str()
    };
    if g.attempts > 1 {
        format!("↻ {} • {name}", g.attempts)
    } else {
        name.to_string()
    }
}

/// Whether the first stretch is time spent queued: scheduled, then started, then closed.
/// That is the stretch the web UI fades, and only once the group completed.
fn queue_is_first(g: &Group, events: &[&NormalizedEvent]) -> bool {
    matches!(
        g.category,
        Category::Activity | Category::ChildWorkflow | Category::Nexus
    ) && g.outcome == Outcome::Completed
        && events.len() >= 3
        && events[1].role == Role::Continues
}

/// The line: how the group ended, or what kind of thing it is while it has not.
fn line_color(g: &Group) -> Color {
    match g.outcome {
        Outcome::Completed if g.category == Category::Timer => web::TANGERINE_9,
        Outcome::Completed => web::GREEN_9,
        Outcome::Failed | Outcome::Terminated | Outcome::Rejected => web::RED_11,
        Outcome::TimedOut => web::PERSIMMON_9,
        Outcome::Canceled => web::AMBER_9,
        Outcome::Pending if g.category == Category::Workflow => web::BLUE_9,
        Outcome::Pending | Outcome::ContinuedAsNew => {
            category_color(g.category).unwrap_or(web::INDIGO_3)
        }
    }
}

fn category_color(c: Category) -> Option<Color> {
    match c {
        Category::Workflow | Category::ChildWorkflow => Some(web::ZAFFRE_7),
        Category::Activity => Some(web::MAGENTA_7),
        Category::Timer => Some(web::PINK_9),
        Category::Nexus => Some(web::PEACOCK_9),
        Category::Update => Some(web::PERSIMMON_8),
        Category::WorkflowTask
        | Category::ExternalWorkflow
        | Category::Marker
        | Category::SearchAttributes => None,
    }
}

/// A dot is coloured by what its event did, the way the web UI classifies it.
fn dot_color(g: &Group, e: &NormalizedEvent) -> Color {
    match e.role {
        Role::Closes => match e.outcome {
            Outcome::Completed if g.category == Category::Timer => web::TANGERINE_9,
            Outcome::Completed => web::GREEN_9,
            Outcome::Failed | Outcome::Terminated | Outcome::Rejected => web::RED_9,
            Outcome::TimedOut => web::PERSIMMON_9,
            Outcome::Canceled => web::AMBER_9,
            Outcome::Pending | Outcome::ContinuedAsNew => web::INDIGO_3,
        },
        _ if e.name.ends_with("Signaled") => web::PINK_9,
        // Scheduled and initiated read as not yet started; everything else that opens or
        // continues a group is a start.
        Role::Opens
            if matches!(
                g.category,
                Category::Activity | Category::ChildWorkflow | Category::Nexus
            ) =>
        {
            web::INDIGO_3
        }
        _ => web::ZAFFRE_7,
    }
}

/// `a` blended towards `b`, `amount` of the way. Only RGB blends; anything else is
/// returned as is, which on a 16-colour terminal is the honest answer.
fn mix(a: Color, b: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * amount).round() as u8;
            Color::Rgb(m(ar, br), m(ag, bg), m(ab, bb))
        }
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_goes_before_dots_in_the_right_half() {
        assert_eq!(label_column(&[80, 90], 100, false, 10), 69);
    }

    #[test]
    fn a_label_goes_after_dots_that_end_early() {
        assert_eq!(label_column(&[10, 20, 30], 100, false, 10), 32);
    }

    #[test]
    fn a_long_or_running_group_carries_its_label_on_the_line() {
        // Ends past two thirds: written on the line after the first dot.
        assert_eq!(label_column(&[10, 90], 100, false, 10), 12);
        // Running with a short first stretch: after the second dot.
        assert_eq!(label_column(&[10, 20], 100, true, 10), 22);
        // Queued briefly, then ran long: the label sits after the start.
        assert_eq!(label_column(&[10, 12, 90], 100, false, 5), 14);
    }

    #[test]
    fn the_queued_stretch_is_faded_towards_the_background() {
        let faded = mix(web::GREEN_9, BACKGROUND, 1.0 - QUEUED_OPACITY);
        assert_ne!(faded, web::GREEN_9);
        assert_eq!(mix(web::GREEN_9, BACKGROUND, 0.0), web::GREEN_9);
        assert_eq!(mix(web::GREEN_9, BACKGROUND, 1.0), BACKGROUND);
    }

    #[test]
    fn runs_of_one_style_become_one_span() {
        let s = Style::new();
        let c = Canvas(vec![('a', s), ('b', s), ('c', s.fg(Color::Red))]);
        let spans = c.into_spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "ab");
    }
}
