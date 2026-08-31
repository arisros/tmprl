//! The visibility query bar.
//!
//! Always visible, always the raw query. Anything that filters the list — a saved view
//! today, a filter builder later — writes into this string rather than replacing it with a
//! structure you cannot see or edit. That the web UI hides the query behind a filter widget
//! is the single most irritating thing about using it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let editing = app.is_editing_query();
    let text = app.query_display();

    let label = Style::new()
        .fg(if editing { t.fg } else { t.faint })
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(
        " query ",
        if editing { label.bg(t.sel) } else { label },
    )];

    if text.is_empty() && !editing {
        // An empty query means "everything", which is worth saying rather than leaving the
        // bar blank and ambiguous.
        spans.push(Span::styled(
            " all workflows — i to filter",
            Style::new().fg(t.faint),
        ));
    } else {
        spans.push(Span::styled(
            format!(" {text}"),
            Style::new().fg(if editing { t.fg } else { t.dim }),
        ));
    }
    if editing {
        spans.push(Span::styled("█", Style::new().fg(t.accent)));
        spans.push(Span::styled(
            "  ⏎ apply   Esc cancel",
            Style::new().fg(t.faint),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
