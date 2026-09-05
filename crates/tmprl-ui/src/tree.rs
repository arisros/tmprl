//! One tab's window tree.

use crate::{Axis, Direction, Rect, ViewId};

/// A window, laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pane {
    pub view: ViewId,
    pub rect: Rect,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Leaf(ViewId),
    Split {
        axis: Axis,
        children: Vec<Node>,
        /// Relative sizes, not absolute cells. A tree laid out at one terminal size and then
        /// at another keeps its proportions, which is what makes a resize of the terminal
        /// not scramble a layout the reader arranged.
        weights: Vec<u16>,
    },
}

/// A tree of windows, with one of them focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    root: Node,
    /// Child indices from the root down to the focused leaf. Empty means the root is a leaf.
    focus: Vec<usize>,
}

/// The weight a freshly split window gets. Any constant works (only ratios matter) but a
/// round number keeps `resize` arithmetic legible when debugging a layout.
const DEFAULT_WEIGHT: u16 = 100;

impl Tree {
    pub fn new(view: ViewId) -> Self {
        Self {
            root: Node::Leaf(view),
            focus: Vec::new(),
        }
    }

    pub fn focused(&self) -> ViewId {
        match Self::at(&self.root, &self.focus) {
            Some(Node::Leaf(v)) => *v,
            // Unreachable while `focus` always points at a leaf, which every mutation
            // maintains. Falling back to the first view beats panicking in a renderer.
            _ => self.views().first().copied().unwrap_or(ViewId(0)),
        }
    }

    /// Every view in the tree, left to right, top to bottom.
    pub fn views(&self) -> Vec<ViewId> {
        let mut out = Vec::new();
        Self::walk(&self.root, &mut |v| out.push(v));
        out
    }

    pub fn len(&self) -> usize {
        self.views().len()
    }

    pub fn is_empty(&self) -> bool {
        false // a tree always has at least one leaf
    }

    /// Split the focused window, and focus the new one, as vim does.
    pub fn split(&mut self, axis: Axis, view: ViewId) {
        let path = self.focus.clone();
        let Some(node) = Self::at_mut(&mut self.root, &path) else {
            return;
        };

        match node {
            // Splitting a leaf turns it into a two-child split.
            Node::Leaf(existing) => {
                *node = Node::Split {
                    axis,
                    children: vec![Node::Leaf(*existing), Node::Leaf(view)],
                    weights: vec![DEFAULT_WEIGHT, DEFAULT_WEIGHT],
                };
                self.focus.push(1);
            }
            Node::Split { .. } => {}
        }

        // If the new window's parent splits on the same axis as its grandparent, the nesting
        // is redundant, vim flattens it, and so does this, or `<C-w>l` would have to step
        // through invisible levels.
        self.flatten();
    }

    /// Close the focused window. Returns false when it is the only one, since a tab with no
    /// windows has nothing to draw.
    pub fn close(&mut self) -> bool {
        if self.focus.is_empty() {
            return false;
        }
        let (parent_path, index) = {
            let mut p = self.focus.clone();
            let i = p.pop().expect("focus is non-empty");
            (p, i)
        };

        let Some(Node::Split {
            children, weights, ..
        }) = Self::at_mut(&mut self.root, &parent_path)
        else {
            return false;
        };
        children.remove(index);
        weights.remove(index);

        // A split with one child left is not a split any more.
        if children.len() == 1 {
            let only = children.remove(0);
            let Some(parent) = Self::at_mut(&mut self.root, &parent_path) else {
                return false;
            };
            *parent = only;
            self.focus = parent_path;
            // Focus must land on a leaf, not on whatever the collapsed child happened to be.
            self.descend_to_leaf();
        } else {
            // Focus the neighbour that took its place, or the last one if it was the last.
            self.focus = parent_path;
            self.focus.push(index.min(children.len() - 1));
            self.descend_to_leaf();
        }
        self.flatten();
        true
    }

    /// Move focus geometrically, the way `<C-w>hjkl` does.
    ///
    /// Geometric rather than tree-structural: the window to the right is the one that *looks*
    /// to the right, which is not always a sibling. Candidates must overlap this window
    /// across the direction's axis, so pressing `<C-w>j` in a tall left-hand pane does not
    /// jump to something in a different column that happens to sit lower.
    ///
    /// Returns false when there is nothing that way.
    pub fn focus_direction(&mut self, dir: Direction, area: Rect) -> bool {
        let panes = self.layout_with_paths(area);
        let Some((_, from)) = panes.iter().find(|(_, p)| p.focused).map(|(a, b)| (a, *b)) else {
            return false;
        };

        let axis = dir.axis();
        let best = panes
            .iter()
            .filter(|(_, p)| !p.focused)
            .filter(|(_, p)| from.rect.overlaps_across(p.rect, axis))
            .filter(|(_, p)| match dir {
                Direction::Left => p.rect.right() <= from.rect.x,
                Direction::Right => p.rect.x >= from.rect.right(),
                Direction::Up => p.rect.bottom() <= from.rect.y,
                Direction::Down => p.rect.y >= from.rect.bottom(),
            })
            // Nearest edge first, then nearest along the other axis, so a column of
            // candidates resolves to the one closest to where the cursor already is.
            .min_by_key(|(_, p)| {
                let gap = match dir {
                    Direction::Left => from.rect.x.saturating_sub(p.rect.right()),
                    Direction::Right => p.rect.x.saturating_sub(from.rect.right()),
                    Direction::Up => from.rect.y.saturating_sub(p.rect.bottom()),
                    Direction::Down => p.rect.y.saturating_sub(from.rect.bottom()),
                };
                let offset = match axis {
                    Axis::Columns => p.rect.y.abs_diff(from.rect.y),
                    Axis::Rows => p.rect.x.abs_diff(from.rect.x),
                };
                (gap, offset)
            });

        match best {
            Some((path, _)) => {
                self.focus = path.clone();
                true
            }
            None => false,
        }
    }

    /// Grow or shrink the focused window along `dir` by `cells` worth of weight.
    ///
    /// Applied to the nearest ancestor that actually splits on that axis: asking a
    /// side-by-side split to get taller is meaningless, and silently doing nothing there
    /// would look like a broken key.
    pub fn resize(&mut self, dir: Direction, delta: i32) -> bool {
        let axis = dir.axis();
        let mut path = self.focus.clone();

        while !path.is_empty() {
            let index = *path.last().expect("non-empty");
            let parent_path = &path[..path.len() - 1];
            let is_match = matches!(
                Self::at(&self.root, parent_path),
                Some(Node::Split { axis: a, .. }) if *a == axis
            );
            if is_match {
                let Some(Node::Split { weights, .. }) = Self::at_mut(&mut self.root, parent_path)
                else {
                    return false;
                };
                if weights.len() < 2 {
                    return false;
                }
                // Growing one window shrinks its neighbour: the parent's total is fixed, so
                // weight has to come from somewhere rather than being conjured.
                let neighbour = if index + 1 < weights.len() {
                    index + 1
                } else {
                    index - 1
                };
                let signed = if dir.is_forward() { delta } else { -delta };
                let taken =
                    signed.clamp(-(weights[index] as i32 - 1), weights[neighbour] as i32 - 1);
                if taken == 0 {
                    return false;
                }
                weights[index] = (weights[index] as i32 + taken) as u16;
                weights[neighbour] = (weights[neighbour] as i32 - taken) as u16;
                return true;
            }
            path.pop();
        }
        false
    }

    /// Give every window in every split the same share, as `<C-w>=` does.
    pub fn equalize(&mut self) {
        Self::equalize_node(&mut self.root);
    }

    /// Where everything goes, given the space available.
    pub fn layout(&self, area: Rect) -> Vec<Pane> {
        self.layout_with_paths(area)
            .into_iter()
            .map(|(_, pane)| pane)
            .collect()
    }

    fn layout_with_paths(&self, area: Rect) -> Vec<(Vec<usize>, Pane)> {
        let mut out = Vec::new();
        Self::place(&self.root, area, &mut Vec::new(), &self.focus, &mut out);
        out
    }

    fn place(
        node: &Node,
        area: Rect,
        path: &mut Vec<usize>,
        focus: &[usize],
        out: &mut Vec<(Vec<usize>, Pane)>,
    ) {
        match node {
            Node::Leaf(view) => out.push((
                path.clone(),
                Pane {
                    view: *view,
                    rect: area,
                    focused: path.as_slice() == focus,
                },
            )),
            Node::Split {
                axis,
                children,
                weights,
            } => {
                for (i, (child, rect)) in children
                    .iter()
                    .zip(divide(area, *axis, weights))
                    .enumerate()
                {
                    path.push(i);
                    Self::place(child, rect, path, focus, out);
                    path.pop();
                }
            }
        }
    }

    fn at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
        match path.split_first() {
            None => Some(node),
            Some((i, rest)) => match node {
                Node::Split { children, .. } => Self::at(children.get(*i)?, rest),
                Node::Leaf(_) => None,
            },
        }
    }

    fn at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
        match path.split_first() {
            None => Some(node),
            Some((i, rest)) => match node {
                Node::Split { children, .. } => Self::at_mut(children.get_mut(*i)?, rest),
                Node::Leaf(_) => None,
            },
        }
    }

    fn walk(node: &Node, f: &mut impl FnMut(ViewId)) {
        match node {
            Node::Leaf(v) => f(*v),
            Node::Split { children, .. } => {
                for c in children {
                    Self::walk(c, f);
                }
            }
        }
    }

    /// Push focus down to a leaf, taking the first child at each level.
    fn descend_to_leaf(&mut self) {
        loop {
            match Self::at(&self.root, &self.focus) {
                Some(Node::Split { children, .. }) if !children.is_empty() => self.focus.push(0),
                _ => return,
            }
        }
    }

    /// Merge a split into its parent when both divide on the same axis.
    fn flatten(&mut self) {
        let focused = self.focused();
        Self::flatten_node(&mut self.root);
        // Flattening renumbers children, so the old path may point elsewhere. Re-find the
        // view that was focused rather than trusting the indices.
        if let Some(path) = Self::path_of(&self.root, focused, &mut Vec::new()) {
            self.focus = path;
        }
    }

    fn flatten_node(node: &mut Node) {
        let Node::Split {
            axis,
            children,
            weights,
        } = node
        else {
            return;
        };
        for c in children.iter_mut() {
            Self::flatten_node(c);
        }

        let mut new_children = Vec::new();
        let mut new_weights = Vec::new();
        for (child, weight) in std::mem::take(children)
            .into_iter()
            .zip(std::mem::take(weights))
        {
            match child {
                Node::Split {
                    axis: inner_axis,
                    children: inner,
                    weights: inner_weights,
                } if inner_axis == *axis => {
                    // Redistribute the parent's share of this slot across the children that
                    // are being promoted into it, so the layout does not visibly jump.
                    let total: u32 = inner_weights.iter().map(|w| *w as u32).sum::<u32>().max(1);
                    for (c, w) in inner.into_iter().zip(inner_weights) {
                        new_children.push(c);
                        new_weights.push(
                            ((w as u32 * weight as u32) / total)
                                .max(1)
                                .min(u16::MAX as u32) as u16,
                        );
                    }
                }
                other => {
                    new_children.push(other);
                    new_weights.push(weight);
                }
            }
        }
        *children = new_children;
        *weights = new_weights;
    }

    fn path_of(node: &Node, view: ViewId, path: &mut Vec<usize>) -> Option<Vec<usize>> {
        match node {
            Node::Leaf(v) if *v == view => Some(path.clone()),
            Node::Leaf(_) => None,
            Node::Split { children, .. } => {
                for (i, c) in children.iter().enumerate() {
                    path.push(i);
                    if let Some(found) = Self::path_of(c, view, path) {
                        return Some(found);
                    }
                    path.pop();
                }
                None
            }
        }
    }

    fn equalize_node(node: &mut Node) {
        if let Node::Split {
            children, weights, ..
        } = node
        {
            for w in weights.iter_mut() {
                *w = DEFAULT_WEIGHT;
            }
            for c in children.iter_mut() {
                Self::equalize_node(c);
            }
        }
    }
}

/// Divide `area` along `axis` in proportion to `weights`.
///
/// Every pane gets at least one cell, and the remainder from integer division goes to the
/// earliest panes. Without the floor, a heavily lopsided split renders a zero-width window
/// that cannot be seen or focused out of.
fn divide(area: Rect, axis: Axis, weights: &[u16]) -> Vec<Rect> {
    let n = weights.len();
    if n == 0 {
        return Vec::new();
    }
    let total_extent = area.extent(axis);
    // Not enough room for one cell each: hand out what there is and let the rest be empty,
    // rather than overlapping panes on top of each other.
    if (total_extent as usize) < n {
        return (0..n)
            .map(|i| {
                let mut r = area;
                let at = area_start(area, axis) + i as u16;
                set_span(
                    &mut r,
                    axis,
                    at,
                    if (i as u16) < total_extent { 1 } else { 0 },
                );
                r
            })
            .collect();
    }

    let sum: u32 = weights
        .iter()
        .map(|w| (*w).max(1) as u32)
        .sum::<u32>()
        .max(1);
    let spare = total_extent as u32 - n as u32; // one cell already reserved per pane
    let mut spans: Vec<u16> = weights
        .iter()
        .map(|w| 1 + ((*w).max(1) as u32 * spare / sum) as u16)
        .collect();

    // Integer division leaves cells over; give them to the earliest panes so the total is
    // exactly the space available and no column goes unpainted.
    let assigned: u32 = spans.iter().map(|s| *s as u32).sum();
    let mut leftover = total_extent as u32 - assigned;
    let mut i = 0;
    while leftover > 0 {
        spans[i % n] += 1;
        leftover -= 1;
        i += 1;
    }

    let mut out = Vec::with_capacity(n);
    let mut at = area_start(area, axis);
    for span in spans {
        let mut r = area;
        set_span(&mut r, axis, at, span);
        out.push(r);
        at += span;
    }
    out
}

fn area_start(area: Rect, axis: Axis) -> u16 {
    match axis {
        Axis::Columns => area.x,
        Axis::Rows => area.y,
    }
}

fn set_span(r: &mut Rect, axis: Axis, at: u16, span: u16) {
    match axis {
        Axis::Columns => {
            r.x = at;
            r.width = span;
        }
        Axis::Rows => {
            r.y = at;
            r.height = span;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(n: u64) -> ViewId {
        ViewId(n)
    }

    /// A generous area so proportional division lands on round numbers.
    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };

    fn rects(tree: &Tree) -> Vec<(u64, Rect)> {
        tree.layout(AREA)
            .into_iter()
            .map(|p| (p.view.0, p.rect))
            .collect()
    }

    #[test]
    fn one_window_fills_the_area() {
        let tree = Tree::new(v(1));
        assert_eq!(rects(&tree), [(1, AREA)]);
        assert_eq!(tree.focused(), v(1));
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn a_column_split_divides_the_width_and_focuses_the_new_window() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));

        assert_eq!(tree.focused(), v(2), "vim focuses the window it just made");
        assert_eq!(
            rects(&tree),
            [(1, Rect::new(0, 0, 50, 40)), (2, Rect::new(50, 0, 50, 40))]
        );
    }

    #[test]
    fn a_row_split_divides_the_height() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Rows, v(2));
        assert_eq!(
            rects(&tree),
            [
                (1, Rect::new(0, 0, 100, 20)),
                (2, Rect::new(0, 20, 100, 20))
            ]
        );
    }

    #[test]
    fn panes_tile_the_area_exactly_with_no_gap_or_overlap() {
        // Integer division leaves cells over; unassigned they show as an unpainted column.
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Columns, v(3));

        let area = Rect::new(0, 0, 100, 40); // 100 / 3 does not divide
        let mut panes = tree.layout(area);
        panes.sort_by_key(|p| p.rect.x);

        assert_eq!(panes[0].rect.x, 0);
        for pair in panes.windows(2) {
            assert_eq!(
                pair[0].rect.right(),
                pair[1].rect.x,
                "panes must abut exactly"
            );
        }
        assert_eq!(panes.last().unwrap().rect.right(), area.right());
    }

    #[test]
    fn splitting_on_the_same_axis_flattens_instead_of_nesting() {
        // Nested same-axis splits look identical but make focus movement step through
        // invisible levels.
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Columns, v(3));

        assert_eq!(tree.len(), 3);
        assert_eq!(tree.views(), [v(1), v(2), v(3)]);
        assert_eq!(tree.focused(), v(3), "focus survives the flattening");

        // Flattening must not move anything on screen. vim's `:vsplit` halves the *current*
        // window, so splitting twice gives one half and two quarters, not three thirds.
        // The nesting is what goes away, not the proportions.
        let widths: Vec<u16> = tree.layout(AREA).iter().map(|p| p.rect.width).collect();
        assert_eq!(widths, [50, 25, 25], "flattening must preserve the layout");

        // Evening them up is `<C-w>=`, a separate decision.
        tree.equalize();
        let widths: Vec<u16> = tree.layout(AREA).iter().map(|p| p.rect.width).collect();
        assert!(
            widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1,
            "equalize should even them out, got {widths:?}"
        );
    }

    #[test]
    fn splitting_on_the_other_axis_does_nest() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Rows, v(3));

        // 2 was the right half; it is now split top and bottom.
        let layout = rects(&tree);
        assert_eq!(layout.len(), 3);
        assert!(layout.contains(&(1, Rect::new(0, 0, 50, 40))));
        assert!(layout.contains(&(2, Rect::new(50, 0, 50, 20))));
        assert!(layout.contains(&(3, Rect::new(50, 20, 50, 20))));
    }

    #[test]
    fn focus_moves_to_what_looks_that_way() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2)); // 1 | 2, focus on 2

        assert!(tree.focus_direction(Direction::Left, AREA));
        assert_eq!(tree.focused(), v(1));
        assert!(tree.focus_direction(Direction::Right, AREA));
        assert_eq!(tree.focused(), v(2));
    }

    #[test]
    fn focus_does_not_move_off_the_edge() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.focus_direction(Direction::Left, AREA); // on 1, the leftmost

        assert!(
            !tree.focus_direction(Direction::Left, AREA),
            "nothing there"
        );
        assert_eq!(tree.focused(), v(1), "and focus stays put");
        assert!(!tree.focus_direction(Direction::Up, AREA));
    }

    #[test]
    fn focus_skips_windows_that_do_not_share_any_rows() {
        // 1 fills the left; 2 over 3 on the right. From 1, `l` must reach 2, the one it
        // shares rows with, not 3.

        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Rows, v(3));
        tree.focus_direction(Direction::Left, AREA);
        assert_eq!(tree.focused(), v(1));

        assert!(tree.focus_direction(Direction::Right, AREA));
        assert_eq!(tree.focused(), v(2), "the top-right shares row 0 with 1");

        // And from 2, down reaches 3 but right reaches nothing.
        assert!(tree.focus_direction(Direction::Down, AREA));
        assert_eq!(tree.focused(), v(3));
        assert!(!tree.focus_direction(Direction::Right, AREA));
    }

    #[test]
    fn closing_a_window_gives_its_space_to_the_survivor() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));

        assert!(tree.close());
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.focused(), v(1));
        assert_eq!(rects(&tree), [(1, AREA)], "the split collapsed entirely");
    }

    #[test]
    fn closing_the_last_window_is_refused() {
        // A tab with no windows has nothing to draw; quitting is a separate decision.
        let mut tree = Tree::new(v(1));
        assert!(!tree.close());
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn closing_focuses_a_neighbour_and_never_a_split() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Columns, v(3)); // 1 | 2 | 3, focus 3

        assert!(tree.close());
        assert_eq!(tree.views(), [v(1), v(2)]);
        assert_eq!(tree.focused(), v(2), "focus falls back to the neighbour");

        // Focus must be a real window afterwards, not an interior node.
        assert!(tree.layout(AREA).iter().any(|p| p.focused));
    }

    #[test]
    fn closing_a_nested_window_collapses_its_parent_onto_a_leaf() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Rows, v(3)); // right half split; focus 3

        assert!(tree.close());
        assert_eq!(tree.len(), 2);
        assert_eq!(
            rects(&tree),
            [(1, Rect::new(0, 0, 50, 40)), (2, Rect::new(50, 0, 50, 40))],
            "2 should reclaim the whole right half"
        );
        assert_eq!(tree.focused(), v(2));
        assert_eq!(
            tree.layout(AREA).iter().filter(|p| p.focused).count(),
            1,
            "exactly one window is focused"
        );
    }

    #[test]
    fn resizing_takes_from_the_neighbour_rather_than_conjuring_space() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2)); // focus 2, the right half

        let before: u16 = tree.layout(AREA)[0].rect.width;
        assert!(tree.resize(Direction::Right, 50));

        let after = tree.layout(AREA);
        assert!(after[1].rect.width > before, "the focused window grew");
        assert!(after[0].rect.width < before, "its neighbour gave the space");
        assert_eq!(
            after[0].rect.width + after[1].rect.width,
            AREA.width,
            "the total is unchanged"
        );
    }

    #[test]
    fn resizing_along_an_axis_with_no_split_does_nothing() {
        // Asking a side-by-side split to get taller is meaningless.
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        assert!(!tree.resize(Direction::Down, 10));
        assert!(
            !Tree::new(v(1)).resize(Direction::Right, 10),
            "one window alone"
        );
    }

    #[test]
    fn a_window_can_never_be_resized_out_of_existence() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        for _ in 0..50 {
            tree.resize(Direction::Right, 1_000);
        }
        for pane in tree.layout(AREA) {
            assert!(pane.rect.width >= 1, "a zero-width window cannot be seen");
        }
    }

    #[test]
    fn equalize_undoes_a_resize() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.resize(Direction::Right, 60);
        tree.equalize();

        let widths: Vec<u16> = tree.layout(AREA).iter().map(|p| p.rect.width).collect();
        assert!(
            widths[0].abs_diff(widths[1]) <= 1,
            "expected even columns, got {widths:?}"
        );
    }

    #[test]
    fn a_layout_keeps_its_proportions_at_a_different_terminal_size() {
        // Weights are relative, so resizing the terminal must not scramble an arrangement
        // the reader set up.
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.resize(Direction::Right, 50);

        let wide = tree.layout(Rect::new(0, 0, 200, 40));
        let narrow = tree.layout(Rect::new(0, 0, 100, 40));
        let ratio = |p: &[Pane]| p[1].rect.width as f32 / p[0].rect.width as f32;
        assert!(
            (ratio(&wide) - ratio(&narrow)).abs() < 0.15,
            "proportions drifted: {} vs {}",
            ratio(&wide),
            ratio(&narrow)
        );
    }

    #[test]
    fn a_tiny_area_does_not_produce_overlapping_windows() {
        // Three windows in two columns cannot all be seen; they must still not be drawn on
        // top of each other.
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Columns, v(3));

        let panes = tree.layout(Rect::new(0, 0, 2, 1));
        for pair in panes.windows(2) {
            assert!(
                pair[0].rect.right() <= pair[1].rect.x,
                "windows overlap: {:?}",
                panes.iter().map(|p| p.rect).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_zero_sized_area_does_not_panic() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Rows, v(2));
        let panes = tree.layout(Rect::new(0, 0, 0, 0));
        assert_eq!(panes.len(), 2);
        assert!(panes.iter().all(|p| p.rect.is_empty()));
    }

    #[test]
    fn exactly_one_window_is_focused_however_the_tree_was_built() {
        let mut tree = Tree::new(v(1));
        tree.split(Axis::Columns, v(2));
        tree.split(Axis::Rows, v(3));
        tree.split(Axis::Columns, v(4));
        tree.focus_direction(Direction::Left, AREA);
        tree.close();

        assert_eq!(
            tree.layout(AREA).iter().filter(|p| p.focused).count(),
            1,
            "focus is a single window at all times"
        );
    }
}
