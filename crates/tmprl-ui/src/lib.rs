//! The window tree: splits, tabs and focus, expressed as rectangles.
//!
//! There are no ratatui types here and no dependencies at all. A layout is a tree plus some
//! arithmetic, and keeping the terminal out of it is what lets the rules, where focus goes
//! when you press `<C-w>l`, what happens to a split's siblings when you close it, be tested
//! as plain functions.
//!
//! The model is vim's, because the people who want a Temporal client in their terminal
//! mostly have vim's window commands in their fingers already. It also means the diff
//! feature is not a feature: two workflow histories in a side-by-side split *is* the
//! comparison, and it works for any two views rather than a pair somebody anticipated.

mod tabs;
mod tree;

pub use tabs::Tabs;
pub use tree::{Pane, Tree};

/// Which way a split divides its children.
///
/// Named for what you *see* rather than for the cut, because "horizontal split" is ambiguous
/// in exactly the way that produces transposed layouts: vim's `:split` is called horizontal
/// and stacks windows vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Side by side, divided along x. vim's `:vsplit`.
    Columns,
    /// One above another, divided along y. vim's `:split`.
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

/// Where to move focus, or which edge to drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The axis a split must divide on for this direction to mean anything within it.
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Columns,
            Direction::Up | Direction::Down => Axis::Rows,
        }
    }

    /// Whether this direction increases the coordinate.
    pub fn is_forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

/// What a pane is showing. Opaque here: this crate arranges rectangles and does not care
/// what is drawn in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub u64);

/// A rectangle in terminal cells.
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

    /// Length along an axis.
    pub fn extent(self, axis: Axis) -> u16 {
        match axis {
            Axis::Columns => self.width,
            Axis::Rows => self.height,
        }
    }

    /// Whether the two overlap on the axis *perpendicular* to `axis`.
    ///
    /// This is what makes `<C-w>l` land somewhere sensible: of the windows to the right,
    /// the ones worth considering are those that share some rows with this one.
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
        // Two panes side by side, sharing every row: they overlap across Columns.
        let left = Rect::new(0, 0, 10, 20);
        let right = Rect::new(10, 0, 10, 20);
        assert!(left.overlaps_across(right, Axis::Columns));

        // The same pair share no columns, so moving up or down between them is meaningless.
        assert!(!left.overlaps_across(right, Axis::Rows));
    }

    #[test]
    fn touching_edges_do_not_count_as_overlapping() {
        // A pane ending at y=10 and one starting at y=10 share no row.
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
