//! The wheel and the click, for a terminal that was asked to report them.

use super::*;

/// How far one notch of the wheel goes. Three is what terminals and editors scroll by.
const WHEEL: isize = 3;

/// Two clicks on one row within this long are a double click, which opens it.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// The mouse events tmprl acts on. Everything else a terminal reports, movement and the
/// other buttons, is dropped before it gets here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Wheel {
        down: bool,
        column: u16,
        row: u16,
    },
    Click {
        column: u16,
        row: u16,
    },
    /// The pointer moved with the button held.
    Drag {
        column: u16,
        row: u16,
    },
    Release,
}

impl App {
    pub(super) fn on_mouse(&mut self, mouse: Mouse) {
        if mouse == Mouse::Release {
            self.dragging = false;
            return;
        }
        // Whatever owns the keyboard owns the screen too. Acting on a window behind a
        // confirmation would be acting on something that cannot be seen.
        if self.confirm.is_some()
            || self.prompt.is_some()
            || self.picker.is_some()
            || self.form.is_some()
            || self.mode == Mode::Insert
        {
            return;
        }
        let (column, row) = match mouse {
            Mouse::Wheel { column, row, .. } | Mouse::Click { column, row } => (column, row),
            Mouse::Drag { column, row } => {
                if self.dragging {
                    self.drag_detail(column, row);
                }
                return;
            }
            Mouse::Release => return,
        };
        if self.overlay_open() {
            if let Mouse::Wheel { down, .. } = mouse {
                self.scroll_overlay(if down { WHEEL } else { -WHEEL });
            }
            return;
        }

        let Some(window) = self
            .tabs
            .layout(self.frame)
            .into_iter()
            .find(|p| holds(p.rect, column, row))
        else {
            return;
        };
        if !window.focused {
            self.view.detail_focus = false;
            self.focus_pane(window.view);
        }
        let on_payload = self.view.show_detail
            && self.view.screen == Screen::History
            && holds(self.view.detail_area, column, row);

        match mouse {
            Mouse::Click { .. } => {
                self.view.detail_focus = on_payload && self.view.detail_max_scroll > 0;
                if on_payload {
                    // The top rule is the pane's handle: held, it resizes the pane.
                    self.dragging = row == self.view.detail_area.y;
                } else {
                    self.click_row(column, row);
                }
            }
            Mouse::Wheel { down, .. } if on_payload => {
                self.scroll_detail(if down { WHEEL } else { -WHEEL });
            }
            Mouse::Wheel { down, .. } if self.on_dashboard() => {
                let heading = if down { Heading::Down } else { Heading::Up };
                self.step_dashboard(heading, WHEEL as usize);
            }
            Mouse::Wheel { down, .. } => self.move_cursor(if down { WHEEL } else { -WHEEL }),
            Mouse::Drag { .. } | Mouse::Release => {}
        }
    }

    /// Put the cursor on the row clicked, and open it when that is the second click there.
    fn click_row(&mut self, column: u16, row: u16) {
        let Some(at) = self
            .view
            .rows_drawn
            .get()
            .and_then(|drawn| drawn.row_at(column, row))
            .filter(|at| *at < self.row_count())
        else {
            self.last_click = None;
            return;
        };
        let now = std::time::Instant::now();
        let again = self.last_click.is_some_and(|(when, _, line)| {
            line == row && at == self.view.cursor && now.duration_since(when) < DOUBLE_CLICK
        });
        self.set_cursor(at);
        if again {
            self.last_click = None;
            self.open_focused();
        } else {
            self.last_click = Some((now, column, row));
        }
    }
}

fn holds(rect: UiRect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
}
