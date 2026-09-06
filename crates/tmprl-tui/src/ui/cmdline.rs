//! Completions for the `:` command line.
//!
//! Candidates come from the command registry, so anything runnable is discoverable by
//! typing part of its name, there is no second list to keep in sync.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::App;
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &App, t: &Theme) {
    let matches = app.cmdline_matches();
    if matches.is_empty() {
        return;
    }
    let area = frame.area();
    let height = (matches.len() as u16 + 2).min(area.height.saturating_sub(1));
    if height < 3 {
        return;
    }

    let width = 56u16.min(area.width);
    let popup = Rect {
        x: area.x,
        // Sits directly above the status row, which is where the command line itself is.
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };

    let lines: Vec<Line> = matches
        .iter()
        .map(|c| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<20}", c.id),
                    Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(c.title.to_string(), Style::new().fg(t.dim)),
            ])
        })
        .collect();

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Span::styled(" commands ", Style::new().fg(t.faint)))
                .border_style(Style::new().fg(t.faint)),
        ),
        popup,
    );
}
