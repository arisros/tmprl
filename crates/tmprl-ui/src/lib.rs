//! The window tree: splits, tabs and focus, expressed as rectangles.

mod tabs;
mod tracks;
mod tree;

pub use tabs::Tabs;
pub use tracks::{Track, tracks};
pub use tree::{Pane, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    Columns,
    Rows,
}

impl Axis {
    pub fn other(self) -> Self {
        match self {
            Axis::Columns => Axis::Rows,
            Axis::Rows => Axis::Columns,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Columns,
            Direction::Up | Direction::Down => Axis::Rows,
        }
    }

    pub fn is_forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn extent(self, axis: Axis) -> u16 {
        match axis {
            Axis::Columns => self.width,
            Axis::Rows => self.height,
        }
    }

    pub fn overlaps_across(self, other: Rect, axis: Axis) -> bool {
        match axis {
            Axis::Columns => self.y < other.bottom() && other.y < self.bottom(),
            Axis::Rows => self.x < other.right() && other.x < self.right(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_axis_has_an_opposite() {
        assert_eq!(Axis::Columns.other(), Axis::Rows);
        assert_eq!(Axis::Rows.other(), Axis::Columns);
    }

    #[test]
    fn a_direction_knows_which_split_it_can_move_within() {
        assert_eq!(Direction::Left.axis(), Axis::Columns);
        assert_eq!(Direction::Right.axis(), Axis::Columns);
        assert_eq!(Direction::Up.axis(), Axis::Rows);
        assert_eq!(Direction::Down.axis(), Axis::Rows);
        assert!(Direction::Right.is_forward() && Direction::Down.is_forward());
        assert!(!Direction::Left.is_forward() && !Direction::Up.is_forward());
    }

    #[test]
    fn rect_edges_saturate_rather_than_wrapping() {
        let r = Rect::new(u16::MAX - 1, u16::MAX - 1, 10, 10);
        assert_eq!(r.right(), u16::MAX);
        assert_eq!(r.bottom(), u16::MAX);
    }

    #[test]
    fn overlap_is_measured_across_the_axis_not_along_it() {
        let left = Rect::new(0, 0, 10, 20);
        let right = Rect::new(10, 0, 10, 20);
        assert!(left.overlaps_across(right, Axis::Columns));

        assert!(!left.overlaps_across(right, Axis::Rows));
    }

    #[test]
    fn touching_edges_do_not_count_as_overlapping() {
        let top = Rect::new(0, 0, 10, 10);
        let bottom = Rect::new(0, 10, 10, 10);
        assert!(!top.overlaps_across(bottom, Axis::Columns));
        assert!(
            top.overlaps_across(bottom, Axis::Rows),
            "they share columns"
        );
    }

    #[test]
    fn an_empty_rect_is_one_with_no_area() {
        assert!(Rect::new(0, 0, 0, 5).is_empty());
        assert!(Rect::new(0, 0, 5, 0).is_empty());
        assert!(!Rect::new(0, 0, 1, 1).is_empty());
    }

    #[test]
    fn extent_reads_the_axis_it_is_asked_for() {
        let r = Rect::new(0, 0, 30, 10);
        assert_eq!(r.extent(Axis::Columns), 30);
        assert_eq!(r.extent(Axis::Rows), 10);
    }
}
