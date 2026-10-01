//! The `:messages` overlay: everything the note line has said, newest last.

use ratatui::Frame;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::{App, Logged, Note};
use crate::theme::Theme;

/// Width of the time and level columns, so continuation and detail lines sit under the
/// text. Two of margin, `14:03:22`, two, the level padded to five, two.
const INDENT: usize = 19;

pub fn render(frame: &mut Frame, app: &mut App, t: &Theme) {
    let area = super::centered(
        frame.area(),
        100,
        frame.area().height.saturating_sub(2).max(3),
    );
    if area.height < 3 || area.width < 20 {
        return;
    }

    // Wrapped by hand, because a server's message is as long as the server likes and the
    // end of it is often the useful part, and the scroll limit needs the row count.
    let width = (area.width as usize).saturating_sub(2 + INDENT).max(10);
    let mut lines: Vec<Line> = Vec::new();
    for m in &app.messages {
        entry(&mut lines, m, width, app, t);
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  nothing has been said yet",
            t.faint,
        )));
    }

    let visible = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    app.overlay_max_scroll = total.saturating_sub(visible);
    let scroll = app.overlay_scroll.min(app.overlay_max_scroll);
    app.overlay_scroll = scroll;

    let title = if app.overlay_max_scroll == 0 {
        " messages (Esc to close) ".to_string()
    } else {
        format!(
            " messages (j/k to scroll, {}/{}, Esc to close) ",
            scroll + 1,
            app.overlay_max_scroll + 1
        )
    };

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)).block(
            Block::bordered()
                .title(Span::styled(title, t.accent))
                .border_style(t.faint),
        ),
        area,
    );
    super::border_scrollbar(frame, area, scroll, total, t);
}

fn entry(lines: &mut Vec<Line>, m: &Logged, width: usize, app: &App, t: &Theme) {
    let (label, colour) = match m.level {
        Note::Info => ("info", t.ok),
        Note::Warn => ("warn", t.warn),
        Note::Error => ("error", t.err),
    };
    let pad = " ".repeat(INDENT);
    let mut text = m.text.lines().flat_map(|l| super::wrap(l, width));
    lines.push(Line::from(vec![
        Span::styled(format!("  {}  ", app.clock.time_of_day(m.at_ms)), t.faint),
        Span::styled(format!("{label:<5}"), colour.add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {}", text.next().unwrap_or_default()), t.fg),
    ]));
    for rest in text {
        lines.push(Line::from(Span::styled(format!("{pad}{rest}"), t.fg)));
    }

    let Some(fault) = &m.fault else {
        return;
    };
    // The name gRPC uses, because that is what a server log or a bug report will say.
    let origin = if fault.operation.is_empty() {
        fault.code.name().to_string()
    } else {
        format!("{} · {}", fault.operation, fault.code.name())
    };
    for detail in std::iter::once(origin).chain(fault.hint().map(str::to_string)) {
        lines.push(Line::from(Span::styled(format!("{pad}{detail}"), t.faint)));
    }
}
