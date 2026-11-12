//! Binary split tree of panes. Leaves hold pane ids.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    #[default]
    Right,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// Where on a pane another pane is dropped: an edge splits it there, the centre swaps the two.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl Zone {
    /// The zone under a point of a pane, as fractions of its width and height from the top-left:
    /// the middle (30% to 70% both ways) is the centre, else the nearest edge.
    pub fn at(x: f32, y: f32) -> Zone {
        let middle = 0.3..=0.7;
        if middle.contains(&x) && middle.contains(&y) {
            return Zone::Center;
        }
        let edges = [(x, Zone::Left), (1.0 - x, Zone::Right), (y, Zone::Top), (1.0 - y, Zone::Bottom)];
        edges.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).map_or(Zone::Center, |(_, zone)| zone)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    Leaf(u32),
    /// `ratio`: the share of the split that `a` takes, e.g. 0.5 for equal halves.
    Split {
        axis: Axis,
        #[serde(serialize_with = "share")]
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// Writes a ratio to three places, e.g. 0.45: as an `f64`, the `f32` would read 0.44999998807907104.
fn share<S: serde::Serializer>(ratio: &f32, to: S) -> Result<S::Ok, S::Error> {
    to.serialize_f64((f64::from(*ratio) * 1000.0).round() / 1000.0)
}

/// One resize step, as a share of the split it moves, and the least share either side keeps.
pub const RESIZE_STEP: f32 = 0.05;
pub const MIN_SHARE: f32 = 0.1;

impl Node {
    fn pair(axis: Axis, a: Node, b: Node) -> Node {
        Node::Split { axis, ratio: 0.5, a: Box::new(a), b: Box::new(b) }
    }

    /// Adds `new` after leaf `beside`, splitting it along `axis`. With no such leaf, splits the root.
    pub fn insert(self, beside: Option<u32>, new: u32, axis: Axis) -> Node {
        let mut root = self;
        if beside.is_some_and(|at| root.split(at, new, axis, false)) {
            return root;
        }
        Node::pair(axis, root, Node::Leaf(new))
    }

    /// Splits leaf `at` along `axis`; `new` takes the half after it, or with `before` the half
    /// before it (left or top). False if there is no such leaf.
    pub fn split(&mut self, at: u32, new: u32, axis: Axis, before: bool) -> bool {
        match self {
            Node::Leaf(id) if *id == at => {
                let (first, second) = if before { (new, at) } else { (at, new) };
                *self = Node::pair(axis, Node::Leaf(first), Node::Leaf(second));
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(at, new, axis, before) || b.split(at, new, axis, before),
        }
    }

    /// True if leaf `new` already sits beside leaf `at` as `split(at, new, axis, before)` would
    /// put it: the two are the halves of one split on `axis`, in that order.
    pub fn is_split(&self, at: u32, new: u32, axis: Axis, before: bool) -> bool {
        let (first, second) = if before { (new, at) } else { (at, new) };
        match self {
            Node::Leaf(_) => false,
            Node::Split { axis: on, a, b, .. } => {
                (*on == axis && **a == Node::Leaf(first) && **b == Node::Leaf(second))
                    || a.is_split(at, new, axis, before)
                    || b.is_split(at, new, axis, before)
            }
        }
    }

    /// Removes leaf `id`; its sibling takes the parent's place. `None` once the tree is empty.
    pub fn remove(self, id: u32) -> Option<Node> {
        match self {
            Node::Leaf(leaf) => (leaf != id).then_some(Node::Leaf(leaf)),
            Node::Split { axis, ratio, a, b } => match (a.remove(id), b.remove(id)) {
                (Some(a), Some(b)) => Some(Node::Split { axis, ratio, a: Box::new(a), b: Box::new(b) }),
                (a, b) => a.or(b),
            },
        }
    }

    /// Leaves in reading order: left to right, top to bottom within each split.
    pub fn leaves(&self) -> Vec<u32> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { a, b, .. } => [a.leaves(), b.leaves()].concat(),
        }
    }

    /// Each leaf's rect when the tree fills `area`.
    pub fn rects(&self, area: Rect) -> Vec<(u32, Rect)> {
        match self {
            Node::Leaf(id) => vec![(*id, area)],
            Node::Split { axis, ratio, a, b } => {
                let (first, second) = match axis {
                    Axis::Right => {
                        let w = area.w * ratio;
                        (Rect { w, ..area }, Rect { x: area.x + w, w: area.w - w, ..area })
                    }
                    Axis::Down => {
                        let h = area.h * ratio;
                        (Rect { h, ..area }, Rect { y: area.y + h, h: area.h - h, ..area })
                    }
                };
                let mut out = a.rects(first);
                out.extend(b.rects(second));
                out
            }
        }
    }

    /// The pane that touches `from` on its `dir` side. With several, the one nearest `from`'s
    /// top (for left/right) or left (for up/down) edge.
    pub fn neighbor(&self, from: u32, dir: Dir) -> Option<u32> {
        let rects = self.rects(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        let (_, me) = *rects.iter().find(|(id, _)| *id == from)?;
        // (`from`'s edge, the candidate's facing edge, both spans across the direction of travel)
        let facing = |r: &Rect| match dir {
            Dir::Left => (me.x, r.x + r.w, (me.y, me.h), (r.y, r.h)),
            Dir::Right => (me.x + me.w, r.x, (me.y, me.h), (r.y, r.h)),
            Dir::Up => (me.y, r.y + r.h, (me.x, me.w), (r.x, r.w)),
            Dir::Down => (me.y + me.h, r.y, (me.x, me.w), (r.x, r.w)),
        };
        const EPS: f32 = 1e-4;
        rects
            .iter()
            .filter(|(id, _)| *id != from)
            .filter_map(|(id, r)| {
                let (mine, theirs, (a, a_len), (b, b_len)) = facing(r);
                let touches = (mine - theirs).abs() < EPS;
                let overlaps = a < b + b_len - EPS && b < a + a_len - EPS;
                (touches && overlaps).then_some((*id, (b - a).abs()))
            })
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(id, _)| id)
    }

    /// Moves one boundary of `pane` on `dir`'s axis a step toward `dir`: its right or bottom
    /// border (the nearest split that holds it in `a`), else its left or top one. So the border
    /// that moves does not depend on how the tree is nested. Each side keeps `MIN_SHARE` of the
    /// split; a border at that limit stays, and no other moves instead. False if nothing moved.
    pub fn resize(&mut self, pane: u32, dir: Dir) -> bool {
        let (axis, step) = match dir {
            Dir::Left => (Axis::Right, -RESIZE_STEP),
            Dir::Right => (Axis::Right, RESIZE_STEP),
            Dir::Up => (Axis::Down, -RESIZE_STEP),
            Dir::Down => (Axis::Down, RESIZE_STEP),
        };
        let after = self.nudge(pane, axis, step, true).flatten();
        after.or_else(|| self.nudge(pane, axis, step, false).flatten()) == Some(true)
    }

    /// Steps the nearest split on `on` that holds `pane` in `a` (`in_a`) or in `b`.
    /// `None`: `pane` is not under this node. `Some(None)`: it is, with no such split so far.
    /// `Some(Some(moved))`: the split was found; `moved` is false if it was at its limit.
    fn nudge(&mut self, pane: u32, on: Axis, step: f32, in_a: bool) -> Option<Option<bool>> {
        match self {
            Node::Leaf(id) => (*id == pane).then_some(None),
            Node::Split { axis, ratio, a, b } => {
                let (found, first) = match a.nudge(pane, on, step, in_a) {
                    Some(found) => (found, true),
                    None => (b.nudge(pane, on, step, in_a)?, false),
                };
                if found.is_some() || *axis != on || first != in_a {
                    return Some(found);
                }
                let to = (*ratio + step).clamp(MIN_SHARE, 1.0 - MIN_SHARE);
                let moved = (to - *ratio).abs() > f32::EPSILON;
                *ratio = to;
                Some(Some(moved))
            }
        }
    }

    /// Leaves `x` and `y` trade places. A tree that holds only one of them gets the other.
    pub fn swap(&mut self, x: u32, y: u32) {
        match self {
            Node::Leaf(id) if *id == x => *id = y,
            Node::Leaf(id) if *id == y => *id = x,
            Node::Leaf(_) => {}
            Node::Split { a, b, .. } => {
                a.swap(x, y);
                b.swap(x, y);
            }
        }
    }

    /// Evens out every split: panes side by side in a row (or stacked in a column) get equal
    /// shares, e.g. three in a row a third each.
    pub fn equalize(&mut self) {
        if let Node::Split { axis, ratio, a, b } = self {
            let (first, second) = (a.run(*axis), b.run(*axis));
            *ratio = first / (first + second);
            a.equalize();
            b.equalize();
        }
    }

    /// How many equal shares this node takes in a split along `along`: a split on the same axis
    /// is a run of its children; anything else is one.
    fn run(&self, along: Axis) -> f32 {
        match self {
            Node::Split { axis, a, b, .. } if *axis == along => a.run(along) + b.run(along),
            _ => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Axis::*, Dir, Node, Node::Leaf, Rect, Zone};

    #[test]
    fn a_drop_zone_is_the_centre_or_the_nearest_edge() {
        assert_eq!(Zone::at(0.5, 0.5), Zone::Center);
        assert_eq!(Zone::at(0.35, 0.65), Zone::Center);
        assert_eq!(Zone::at(0.1, 0.5), Zone::Left);
        assert_eq!(Zone::at(0.9, 0.5), Zone::Right);
        assert_eq!(Zone::at(0.5, 0.05), Zone::Top);
        assert_eq!(Zone::at(0.5, 0.95), Zone::Bottom);
        // A corner goes to the nearer edge.
        assert_eq!(Zone::at(0.1, 0.2), Zone::Left);
        assert_eq!(Zone::at(0.8, 0.9), Zone::Bottom);
    }

    #[test]
    fn split_can_put_the_new_leaf_first() {
        let mut tree = Leaf(1).insert(Some(1), 2, Right);
        assert!(tree.split(2, 3, Down, true));
        assert_eq!(tree, Node::pair(Right, Leaf(1), Node::pair(Down, Leaf(3), Leaf(2))));
        assert!(!tree.split(9, 4, Down, true));
    }

    #[test]
    fn insert_splits_the_given_leaf() {
        let tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(1), 3, Down);
        // 1 is split down into 1/3; 2 stays to the right.
        assert_eq!(tree, Node::pair(Right, Node::pair(Down, Leaf(1), Leaf(3)), Leaf(2)));
    }

    #[test]
    fn insert_without_target_splits_the_root() {
        let tree = Leaf(1).insert(Some(1), 2, Right).insert(None, 3, Down).insert(Some(9), 4, Right);
        let top = Node::pair(Right, Leaf(1), Leaf(2));
        assert_eq!(tree, Node::pair(Right, Node::pair(Down, top, Leaf(3)), Leaf(4)));
    }

    #[test]
    fn remove_collapses_to_the_sibling() {
        let tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(1), 3, Down);
        let tree = tree.remove(1).unwrap();
        assert_eq!(tree, Node::pair(Right, Leaf(3), Leaf(2)));
        assert_eq!(tree.remove(3).unwrap().remove(2), None);
    }

    #[test]
    fn neighbor_follows_the_shared_edge() {
        // 1 | 2
        //   | 3
        let tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Down);
        assert_eq!(tree.neighbor(1, Dir::Right), Some(2)); // nearest 1's top edge
        assert_eq!(tree.neighbor(3, Dir::Left), Some(1));
        assert_eq!(tree.neighbor(2, Dir::Down), Some(3));
        assert_eq!(tree.neighbor(3, Dir::Up), Some(2));
        assert_eq!(tree.neighbor(1, Dir::Left), None);
        assert_eq!(tree.neighbor(9, Dir::Left), None);
    }

    fn widths(tree: &Node) -> Vec<(u32, f32, f32)> {
        let unit = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
        tree.rects(unit).into_iter().map(|(id, r)| (id, (r.w * 100.0).round(), (r.h * 100.0).round())).collect()
    }

    #[test]
    fn resize_moves_the_nearest_split_on_that_axis_and_stops_at_the_minimum() {
        // 1 | 2
        //   | 3
        let mut tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Down);
        // From 3, the nearest left/right split is the root: its boundary moves left, 1 shrinks.
        assert!(tree.resize(3, Dir::Left));
        assert_eq!(widths(&tree), [(1, 45.0, 100.0), (2, 55.0, 50.0), (3, 55.0, 50.0)]);
        // Up/down moves the 2/3 split only.
        assert!(tree.resize(3, Dir::Down));
        assert_eq!(widths(&tree), [(1, 45.0, 100.0), (2, 55.0, 55.0), (3, 55.0, 45.0)]);
        // 1 has no up/down split around it; an unknown pane has none at all.
        assert!(!tree.resize(1, Dir::Up));
        assert!(!tree.resize(9, Dir::Left));
        for _ in 0..30 {
            tree.resize(1, Dir::Left);
        }
        assert_eq!(widths(&tree)[0], (1, 10.0, 100.0));
        assert!(!tree.resize(1, Dir::Left));
    }

    #[test]
    fn a_split_at_its_limit_stops_and_its_parent_does_not_move_instead() {
        // 1 | 2 | 3, nested as 1 | (2 | 3).
        let mut tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Right);
        // From 3, only the 2|3 border moves left, until 2 is at its minimum.
        for _ in 0..8 {
            assert!(tree.resize(3, Dir::Left));
        }
        assert_eq!(widths(&tree), [(1, 50.0, 100.0), (2, 5.0, 100.0), (3, 45.0, 100.0)]);
        // More presses do nothing: the 1|2 border is not 3's to move.
        assert!(!tree.resize(3, Dir::Left));
        assert!(!tree.resize(3, Dir::Left));
        assert_eq!(widths(&tree), [(1, 50.0, 100.0), (2, 5.0, 100.0), (3, 45.0, 100.0)]);
    }

    #[test]
    fn a_middle_pane_moves_its_right_border_however_the_tree_is_nested() {
        // 1 | 2 | 3 nested as 1 | (2 | 3), and 1 | 3 | 2 nested as (1 | 3) | 2.
        let right_heavy = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Right);
        let left_heavy = Leaf(1).insert(Some(1), 2, Right).insert(Some(1), 3, Right);
        for (mut tree, middle) in [(right_heavy, 2), (left_heavy, 3)] {
            let right = |tree: &Node| {
                let unit = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
                tree.rects(unit).into_iter().map(|(_, r)| r.x + r.w).collect::<Vec<_>>()[1]
            };
            let before = right(&tree);
            assert!(tree.resize(middle, Dir::Right));
            assert!(right(&tree) > before, "the middle pane's right border moves right");
            assert!(tree.resize(middle, Dir::Left));
            assert!(tree.resize(middle, Dir::Left));
            assert!(right(&tree) < before, "and left");
        }
        // The last pane of a row has no right border: its left one moves.
        let mut tree = Leaf(1).insert(Some(1), 2, Right);
        assert!(tree.resize(2, Dir::Left));
        assert_eq!(widths(&tree), [(1, 45.0, 100.0), (2, 55.0, 100.0)]);
    }

    #[test]
    fn swap_trades_two_leaves_and_keeps_the_splits() {
        let mut tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Down);
        tree.resize(1, Dir::Left);
        tree.swap(1, 3);
        assert_eq!(tree.leaves(), [3, 2, 1]);
        assert_eq!(widths(&tree)[0], (3, 45.0, 100.0));
    }

    #[test]
    fn equalize_gives_panes_in_a_row_equal_shares() {
        // 1 | 2 | 3, with 3 split down into 3/4.
        let mut tree = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Right).insert(Some(3), 4, Down);
        tree.resize(4, Dir::Up);
        tree.equalize();
        assert_eq!(widths(&tree), [(1, 33.0, 100.0), (2, 33.0, 100.0), (3, 33.0, 50.0), (4, 33.0, 50.0)]);
    }
}
