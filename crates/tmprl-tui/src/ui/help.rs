//! The help overlay.

use ratatui::Frame;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::App;
use crate::theme::Theme;

const KEYS: usize = 22;
const TITLE: usize = 26;

pub fn render(frame: &mut Frame, app: &mut App, t: &Theme) {
    let area = super::centered(
        frame.area(),
        78,
        frame.area().height.saturating_sub(2).max(3),
    );
    if area.height < 3 || area.width < 20 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for group in app.registry.groups() {
        lines.push(Line::from(Span::styled(
            group.to_string(),
            t.accent.add_modifier(Modifier::BOLD),
        )));
        for cmd in app.registry.all().iter().filter(|c| c.group == group) {
            let keys = app.keymap.keys_for(cmd.id);
            let rendered = if keys.is_empty() {
                ",".to_string()
            } else {
                let mut seen: Vec<String> = Vec::new();
                for b in keys {
                    let s = b.seq.to_string();
                    if !seen.contains(&s) {
                        seen.push(s);
                    }
                }
                seen.join(" / ")
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<KEYS$}", super::truncate(&rendered, KEYS - 1)),
                    t.warn,
                ),
                Span::styled(
                    format!("{:<TITLE$}", super::truncate(cmd.title, TITLE - 1)),
                    t.fg,
                ),
                Span::styled(cmd.id.to_string(), t.faint),
            ]));
        }
        lines.push(Line::raw(""));
    }

    let visible = area.height.saturating_sub(2) as usize;
    app.overlay_max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.overlay_scroll.min(app.overlay_max_scroll);
    app.overlay_scroll = scroll;

    let title = if app.overlay_max_scroll == 0 {
        " help (Esc to close) ".to_string()
    } else {
        format!(
            " help (j/k to scroll, {}/{}, Esc to close) ",
            scroll + 1,
            app.overlay_max_scroll + 1
        )
    };

    let total = lines.len();
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
