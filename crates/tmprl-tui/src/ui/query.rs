//! The visibility query bar.
//!
//! Always visible, always the raw query. Anything that filters the list, a saved view
//! today, a filter builder later, writes into this string rather than replacing it with a
//! structure you cannot see or edit. That the web UI hides the query behind a filter widget
//! is the single most irritating thing about using it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme::Theme;
use crate::view::View;

const LABEL: &str = " query ";
const HINT: &str = "  ⏎ apply   Esc cancel";

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme, focused: bool) {
    if area.height == 0 {
        return;
    }
    // Only the focused pane can be in Insert mode, so only it shows a live edit; the others
    // show the query they have applied.
    let editing = focused && app.is_editing_query();
    let text = if focused {
        app.query_display()
    } else {
        view.query.as_str()
    };

    let label = if editing { t.fg } else { t.faint }.add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(
        LABEL,
        if editing { label.patch(t.sel) } else { label },
    )];

    if editing {
        // The line being typed gets the room first, and the hint only what is left over:
        // it is the same two keys every time, and the query is not.
        let room = (area.width as usize).saturating_sub(LABEL.len() + 1);
        let hinted = room.saturating_sub(HINT.chars().count());
        let fits = app.insert_buf.chars().count() < hinted;
        spans.push(Span::raw(" "));
        spans.extend(super::input::spans(
            &app.insert_buf,
            t.fg,
            if fits { hinted } else { room },
        ));
        if fits {
            spans.push(Span::styled(HINT, t.faint));
        }
    } else if text.is_empty() {
        // An empty query means "everything", which is worth saying rather than leaving the
        // bar blank and ambiguous.
        spans.push(Span::styled(" all workflows (i to filter)", t.faint));
    } else {
        spans.push(Span::styled(format!(" {text}"), t.dim));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
