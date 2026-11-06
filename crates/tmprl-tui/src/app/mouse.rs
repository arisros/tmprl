//! The wheel and the click, for a terminal that was asked to report them.

use super::*;
use crate::view::TextSelection;

const WHEEL: isize = 3;

const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Wheel { down: bool, column: u16, row: u16 },
    Click { column: u16, row: u16 },
    Drag { column: u16, row: u16 },
    Release,
}

impl App {
    pub(super) fn on_mouse(&mut self, mouse: Mouse) {
        if mouse == Mouse::Release {
            self.dragging = false;
            self.copy_selection();
            return;
        }
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
                } else if let Some(selection) = &mut self.text_selection {
                    let area = self.view.detail_text.borrow().area;
                    selection.to = (
                        column.clamp(area.x, area.right().saturating_sub(1)),
                        row.clamp(area.y, area.bottom().saturating_sub(1)),
                    );
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
                self.text_selection = None;
                if on_payload {
                    self.dragging = row == self.view.detail_area.y;
                    if !self.dragging && holds(self.view.detail_text.borrow().area, column, row) {
                        self.text_selection = Some(TextSelection {
                            from: (column, row),
                            to: (column, row),
                        });
                    }
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

    fn copy_selection(&mut self) {
        let Some(selection) = self.text_selection.take() else {
            return;
        };
        if selection.from == selection.to {
            return;
        }
        let text = self.view.detail_text.borrow().text(&selection);
        if !text.trim().is_empty() {
            self.yank(text);
        }
    }

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
