//! The visibility query bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme::Theme;
use crate::view::View;

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme, focused: bool) {
    if area.height == 0 {
        return;
    }
    let editing = focused && app.is_editing_query();
    let text = if focused {
        app.query_display()
    } else {
        view.query.as_str()
    };

    let label = if editing { t.fg } else { t.faint }.add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(
        " query ",
        if editing { label.patch(t.sel) } else { label },
    )];

    if text.is_empty() && !editing {
        spans.push(Span::styled(" all workflows (i to filter)", t.faint));
    } else if editing {
        const HINT: &str = "  ⏎ apply   Esc cancel";
        let room = (area.width as usize).saturating_sub(" query  ".len() + HINT.chars().count());
        spans.push(Span::raw(" "));
        spans.extend(super::edit_spans(
            &app.insert_buf,
            room,
            t.fg,
            t.accent.add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(HINT, t.faint));
    } else {
        spans.push(Span::styled(format!(" {text}"), t.dim));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
