//! The wheel and the click, for a terminal that was asked to report them.

use super::*;

/// How far one notch of the wheel goes. Three is what terminals and editors scroll by.
const WHEEL: isize = 3;

/// The mouse events tmprl acts on. Everything else a terminal reports, movement and the
/// other buttons, is dropped before it gets here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Wheel { down: bool, column: u16, row: u16 },
    Click { column: u16, row: u16 },
}

impl App {
    pub(super) fn on_mouse(&mut self, mouse: Mouse) {
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
            }
            Mouse::Wheel { down, .. } if on_payload => {
                self.scroll_detail(if down { WHEEL } else { -WHEEL });
            }
            Mouse::Wheel { down, .. } if self.on_dashboard() => {
                let heading = if down { Heading::Down } else { Heading::Up };
                self.step_dashboard(heading, WHEEL as usize);
            }
            Mouse::Wheel { down, .. } => self.move_cursor(if down { WHEEL } else { -WHEEL }),
        }
    }
}

fn holds(rect: UiRect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
}
