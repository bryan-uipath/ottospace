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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        let after = self.nudge(pane, axis, step, true);
        after.or_else(|| self.nudge(pane, axis, step, false)) == Some(true)
    }

    /// Steps the nearest split on `on` that holds `pane` in `a` (`in_a`) or in `b`.
    /// `None`: there is no such split. Else whether it moved: not if it was at its limit.
    fn nudge(&mut self, pane: u32, on: Axis, step: f32, in_a: bool) -> Option<bool> {
        let mut moved = None;
        self.climb(pane, on, &mut |ratio, first| {
            if first != in_a {
                return false;
            }
            let to = (*ratio + step).clamp(MIN_SHARE, 1.0 - MIN_SHARE);
            moved = Some((to - *ratio).abs() > f32::EPSILON);
            *ratio = to;
            true
        });
        moved
    }

    /// Scales `pane`'s width (`on` is `Right`) or height by `by`, e.g. 1.2 for a fifth more, by
    /// moving the nearest split on that axis above it. Each side keeps `MIN_SHARE`; what a split
    /// at that limit could not give is asked of the next split above on the axis. False if no
    /// such split holds the pane, or none moved.
    pub fn scale(&mut self, pane: u32, on: Axis, by: f32) -> bool {
        // Not a number (a measure of nothing): no ratio may become one.
        if !by.is_finite() {
            return false;
        }
        let (mut left, mut moved) = (by, false);
        self.climb(pane, on, &mut |ratio, first| {
            // The pane's side of the split.
            let share = if first { *ratio } else { 1.0 - *ratio };
            let wanted = (share * left).clamp(MIN_SHARE, 1.0 - MIN_SHARE);
            let to = if first { wanted } else { 1.0 - wanted };
            moved |= (to - *ratio).abs() > f32::EPSILON;
            *ratio = to;
            left *= share / wanted;
            (left - 1.0).abs() < 1e-4
        });
        moved
    }

    /// What `scale` would do. `None`: no split on that axis holds the pane. Else whether one
    /// would move.
    pub fn scalable(&self, pane: u32, on: Axis, by: f32) -> Option<bool> {
        let (mut copy, mut splits) = (self.clone(), 0);
        copy.climb(pane, on, &mut |_, _| {
            splits += 1;
            false
        });
        (splits > 0).then(|| copy.scale(pane, on, by))
    }

    /// Visits the splits on `on` that hold `pane`, the nearest first, until `visit` says it is
    /// done. `visit` gets the split's ratio and whether the pane is in its first half.
    /// `None`: `pane` is not under this node. Else whether `visit` was done.
    fn climb(&mut self, pane: u32, on: Axis, visit: &mut dyn FnMut(&mut f32, bool) -> bool) -> Option<bool> {
        match self {
            Node::Leaf(id) => (*id == pane).then_some(false),
            Node::Split { axis, ratio, a, b } => {
                let (done, first) = match a.climb(pane, on, visit) {
                    Some(done) => (done, true),
                    None => (b.climb(pane, on, visit)?, false),
                };
                Some(done || (*axis == on && visit(ratio, first)))
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
    use super::{Axis, Axis::*, Dir, Node, Node::Leaf, Rect, Zone};

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

    /// Each leaf's width and height when the tree fills 1000 by 800.
    fn sizes(tree: &Node) -> Vec<(u32, f32, f32)> {
        let rects = tree.rects(Rect { x: 0.0, y: 0.0, w: 1000.0, h: 800.0 });
        rects.into_iter().map(|(id, r)| (id, r.w.round(), r.h.round())).collect()
    }

    #[test]
    fn scale_moves_the_nearest_split_on_each_axis() {
        // 1 | (2 | 3): the inner split moves, and 1 keeps its half.
        let mut row = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Right);
        assert!(row.scale(3, Right, 1.4));
        assert_eq!(sizes(&row), [(1, 500., 800.), (2, 150., 800.), (3, 350., 800.)]);
        assert!(row.scale(2, Right, 2.0));
        assert_eq!(sizes(&row), [(1, 500., 800.), (2, 300., 800.), (3, 200., 800.)]);
        // (1 over 3) | 2: 3's width is the column's; its height is its own.
        let mut nested = Leaf(1).insert(Some(1), 2, Right).insert(Some(1), 3, Down);
        assert!(nested.scale(3, Right, 1.2));
        assert!(nested.scale(3, Down, 0.5));
        assert_eq!(sizes(&nested), [(1, 600., 600.), (3, 600., 200.), (2, 400., 800.)]);
    }

    #[test]
    fn scale_asks_the_next_split_above_for_what_the_nearest_could_not_give() {
        // 1 | (2 | 3) in 1000 pt: pane 3 has 250 and wants 600. The inner split stops at 90%
        // of its half; the outer one gives the rest.
        let mut row = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Right);
        assert!(row.scale(3, Right, 600. / 250.));
        assert_eq!(sizes(&row), [(1, 333., 800.), (2, 67., 800.), (3, 600., 800.)]);
        // Shrinking carries too: pane 2 wants 20 of its 67, gets the inner minimum, and the
        // outer split takes the rest from the pair.
        assert!(row.scale(2, Right, 20. / 67.));
        let [(_, one, _), (_, two, _), (_, three, _)] = sizes(&row)[..] else { panic!() };
        assert!((one + two + three - 1000.).abs() <= 1. && two < 67. && one > 333., "{one} {two} {three}");
        // A split on the other axis between them is passed over.
        let mut nested = Leaf(1).insert(Some(1), 2, Right).insert(Some(2), 3, Down).insert(Some(3), 4, Right);
        assert!(nested.scale(4, Right, 3.0));
        let width = |tree: &Node, id| sizes(tree).iter().find(|(pane, ..)| *pane == id).map(|s| s.1);
        assert_eq!(width(&nested, 4), Some(750.), "250 × 3: inner to 90%, the outer split to 833");
        assert_eq!(width(&nested, 2), Some(833.));
    }

    #[test]
    fn scalable_says_what_scale_would_do_and_moves_nothing() {
        let mut tree = Leaf(1).insert(Some(1), 2, Right);
        assert_eq!(tree.scalable(2, Right, 1.5), Some(true));
        assert_eq!(tree.scalable(2, Down, 1.5), None, "the pane spans the tab's height");
        assert_eq!(tree.scalable(9, Right, 1.5), None, "no such pane");
        assert_eq!(Leaf(1).scalable(1, Right, 1.5), None, "a single pane");
        assert_eq!(sizes(&tree), [(1, 500., 800.), (2, 500., 800.)]);
        tree.scale(2, Right, 5.0);
        assert_eq!(tree.scalable(2, Right, 1.5), Some(false), "at the limit");
        assert_eq!(tree.scalable(2, Right, 0.5), Some(true));
    }

    #[test]
    fn scale_stops_at_the_minimum_share_and_needs_a_split_on_the_axis() {
        let mut tree = Leaf(1).insert(Some(1), 2, Right);
        assert!(tree.scale(2, Right, 5.0));
        assert_eq!(sizes(&tree), [(1, 100., 800.), (2, 900., 800.)]);
        assert!(!tree.scale(2, Right, 5.0), "at the limit");
        assert!(tree.scale(2, Right, 0.01));
        assert_eq!(sizes(&tree), [(1, 900., 800.), (2, 100., 800.)]);
        // Both panes span the tab's height: nothing to move.
        assert!(!tree.scale(2, Down, 0.5));
        assert!(!tree.scale(9, Right, 0.5), "no such pane");
        assert_eq!(sizes(&tree), [(1, 900., 800.), (2, 100., 800.)]);
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

    /// One change to a tree. A new leaf's id is the next unused one.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Op {
        Insert(Option<u32>, Axis),
        Split(u32, Axis, bool),
        Remove(u32),
        Resize(u32, Dir),
        Swap(u32, u32),
        Equalize,
        /// As the model drops a pane on another: an edge splits the target, the centre swaps.
        Drop(u32, u32, Zone),
        /// A fit: the pane's extent on the axis times this.
        Scale(u32, Axis, f32),
    }

    /// The ratios of the splits on `on` above `pane`, nearest first.
    fn above(tree: &Node, pane: u32, on: Axis) -> Option<Vec<f32>> {
        match tree {
            Node::Leaf(id) => (*id == pane).then(Vec::new),
            Node::Split { axis, ratio, a, b } => {
                let mut found = above(a, pane, on).or_else(|| above(b, pane, on))?;
                found.extend((*axis == on).then_some(*ratio));
                Some(found)
            }
        }
    }

    fn extent(tree: &Node, pane: u32, on: Axis) -> Option<f32> {
        let rects = tree.rects(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        rects.iter().find(|(id, _)| *id == pane).map(|(_, r)| if on == Right { r.w } else { r.h })
    }

    fn ratios(tree: &Node) -> Vec<f32> {
        match tree {
            Node::Leaf(_) => Vec::new(),
            Node::Split { ratio, a, b, .. } => [vec![*ratio], ratios(a), ratios(b)].concat(),
        }
    }

    /// The ops from an empty tree, with what must hold after each. The error names the op.
    fn run(ops: &[Op]) -> Result<(), String> {
        use super::MIN_SHARE;
        let (mut tree, mut next): (Option<Node>, u32) = (None, 0);
        // The leaves the tree must hold.
        let mut held: Vec<u32> = Vec::new();
        for (at, op) in ops.iter().enumerate() {
            let fail = |why: String| Err(format!("op {at} {op:?}: {why}"));
            let before = tree.clone();
            match *op {
                Op::Insert(beside, axis) => {
                    next += 1;
                    tree = Some(tree.take().map_or(Leaf(next), |root| root.insert(beside, next, axis)));
                    let split = beside.is_some_and(|beside| held.contains(&beside));
                    if split && !tree.as_ref().is_some_and(|t| t.is_split(beside.unwrap_or(0), next, axis, false)) {
                        return fail("the new leaf is not beside the one named".into());
                    }
                    held.push(next);
                }
                Op::Split(target, axis, first) => {
                    let Some(root) = &mut tree else { continue };
                    let done = root.split(target, next + 1, axis, first);
                    if done != held.contains(&target) || (done && !root.is_split(target, next + 1, axis, first)) {
                        return fail(format!("split said {done}"));
                    }
                    if done {
                        next += 1;
                        held.push(next);
                    }
                }
                Op::Remove(id) => {
                    tree = tree.take().and_then(|root| root.remove(id));
                    held.retain(|leaf| *leaf != id);
                }
                Op::Resize(pane, dir) => {
                    let Some(root) = &mut tree else { continue };
                    let moved = root.resize(pane, dir);
                    // A border at its limit may settle on it by a rounding's worth: that is no move.
                    let was = before.as_ref().map(ratios).unwrap_or_default();
                    if moved == ratios(root).iter().zip(&was).all(|(now, was)| (now - was).abs() < 1e-6) {
                        return fail(format!("resize said {moved}"));
                    }
                }
                Op::Swap(x, y) => {
                    // Only ids given out: a leaf must not take one a later leaf will get.
                    let Some(root) = tree.as_mut().filter(|_| x.max(y) <= next) else { continue };
                    let was = root.leaves();
                    root.swap(x, y);
                    let trade = |id: &u32| {
                        if *id == x {
                            y
                        } else if *id == y {
                            x
                        } else {
                            *id
                        }
                    };
                    // A tree that holds one of the two gets the other.
                    if root.leaves() != was.iter().map(trade).collect::<Vec<_>>() {
                        return fail(format!("{was:?} became {:?}", root.leaves()));
                    }
                    held = root.leaves();
                }
                Op::Equalize => {
                    let Some(root) = &mut tree else { continue };
                    root.equalize();
                    let once = root.clone();
                    root.equalize();
                    if *root != once {
                        return fail("a second equalize moved a split".into());
                    }
                }
                Op::Drop(source, target, zone) => {
                    let Some(root) = tree.take() else { continue };
                    if source == target || !held.contains(&source) || !held.contains(&target) {
                        tree = Some(root);
                        continue;
                    }
                    let (axis, first) = match zone {
                        Zone::Left => (Right, true),
                        Zone::Right => (Right, false),
                        Zone::Top => (Down, true),
                        Zone::Bottom => (Down, false),
                        Zone::Center => {
                            let mut root = root;
                            root.swap(source, target);
                            tree = Some(root);
                            continue;
                        }
                    };
                    let mut root =
                        if root.is_split(target, source, axis, first) { Some(root) } else { root.remove(source) };
                    let there = root.as_mut().is_some_and(|root| {
                        root.is_split(target, source, axis, first) || root.split(target, source, axis, first)
                    });
                    if !there || !root.as_ref().is_some_and(|root| root.is_split(target, source, axis, first)) {
                        return fail("the dropped pane is not where it was dropped".into());
                    }
                    tree = root;
                }
                Op::Scale(pane, on, by) => {
                    let Some(root) = &mut tree else { continue };
                    let (said, was) = (root.scalable(pane, on, by), extent(root, pane, on));
                    let moved = root.scale(pane, on, by);
                    if said.unwrap_or(false) != moved || (said.is_none() && Some(&*root) != before.as_ref()) {
                        return fail(format!("scalable said {said:?}, scale {moved}"));
                    }
                    // The pane has the size asked for, or every split that could give it more
                    // (or take more) is at its limit.
                    let (Some(was), Some(now), true) = (was, extent(root, pane, on), by.is_finite() && by > 0.0) else {
                        continue;
                    };
                    let splits = above(root, pane, on).unwrap_or_default();
                    let exact = (now / was / by - 1.0).abs() < 2e-3;
                    let limit =
                        |ratio: &f32| (ratio - MIN_SHARE).abs() < 1e-6 || (ratio - (1.0 - MIN_SHARE)).abs() < 1e-6;
                    if !splits.is_empty() && !exact && !splits.iter().all(limit) {
                        return fail(format!("the pane is {} times its size, with splits {splits:?}", now / was));
                    }
                    if splits.is_empty() && now != was {
                        return fail("a pane with no split on the axis changed size".into());
                    }
                }
            }
            // Every leaf once, and no other.
            let mut leaves = tree.as_ref().map(Node::leaves).unwrap_or_default();
            leaves.sort_unstable();
            held.sort_unstable();
            if leaves != held || leaves.windows(2).any(|pair| pair[0] == pair[1]) {
                return fail(format!("the tree holds {leaves:?}, not {held:?}"));
            }
            let Some(root) = &tree else { continue };
            // Every ratio is a number and leaves each side its least share: `MIN_SHARE`, or
            // with more than ten panes in a row what `equalize` gives one.
            let least = MIN_SHARE.min(1.0 / leaves.len() as f32) - 1e-6;
            if let Some(ratio) = ratios(root).into_iter().find(|r| !(least..=1.0 - least).contains(r)) {
                return fail(format!("a ratio is {ratio}"));
            }
            // The panes tile the area, and a neighbour is another pane across a shared edge.
            let rects = root.rects(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
            let area: f32 = rects.iter().map(|(_, r)| r.w * r.h).sum();
            if rects.iter().any(|(_, r)| r.w <= 0.0 || r.h <= 0.0 || r.x < -1e-4 || r.y < -1e-4)
                || (area - 1.0).abs() > 1e-3
            {
                return fail(format!("the panes do not tile the area: {rects:?}"));
            }
            for (pane, mine) in &rects {
                for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
                    let Some(other) = root.neighbor(*pane, dir) else { continue };
                    let Some((_, theirs)) = rects.iter().find(|(id, _)| *id == other && id != pane) else {
                        return fail(format!("pane {pane}'s neighbour {dir:?} is {other}"));
                    };
                    let gap = match dir {
                        Dir::Left => mine.x - (theirs.x + theirs.w),
                        Dir::Right => theirs.x - (mine.x + mine.w),
                        Dir::Up => mine.y - (theirs.y + theirs.h),
                        Dir::Down => theirs.y - (mine.y + mine.h),
                    };
                    if gap.abs() > 1e-3 {
                        return fail(format!("pane {pane}'s neighbour {dir:?}, {other}, does not touch it"));
                    }
                }
            }
        }
        Ok(())
    }

    /// An op as Rust, for a failure's report.
    fn code(op: &Op) -> String {
        match op {
            Op::Insert(beside, axis) => format!("Op::Insert({beside:?}, {axis:?})"),
            Op::Split(at, axis, first) => format!("Op::Split({at}, {axis:?}, {first})"),
            Op::Resize(pane, dir) => format!("Op::Resize({pane}, Dir::{dir:?})"),
            Op::Drop(source, target, zone) => format!("Op::Drop({source}, {target}, Zone::{zone:?})"),
            Op::Scale(pane, axis, by) if by.is_nan() => format!("Op::Scale({pane}, {axis:?}, f32::NAN)"),
            Op::Scale(pane, axis, by) if by.is_infinite() => format!("Op::Scale({pane}, {axis:?}, f32::INFINITY)"),
            op => format!("Op::{op:?}"),
        }
    }

    /// Random splits, closes, resizes, swaps, drops, fits and equalizes, in any order. A failure
    /// prints the least ops that still fail, as the argument of `run`.
    #[test]
    fn any_changes_in_any_order_leave_a_sound_tree() {
        use crate::fuzz::{self, Rng};
        let one = |seed: u64| {
            let mut rng = Rng::new(seed);
            // Ids near the ones in use: most name a leaf, some name none.
            let mut ops: Vec<Op> = Vec::new();
            for _ in 0..80 {
                let grown = ops.iter().filter(|op| matches!(op, Op::Insert(..) | Op::Split(..))).count() as u32;
                let leaf = |rng: &mut Rng| rng.below(grown as usize + 2) as u32;
                let axis = rng.pick(&[Right, Down]);
                let dir = rng.pick(&[Dir::Left, Dir::Right, Dir::Up, Dir::Down]);
                let by = rng.pick(&[1.2, 0.8, 2.0, 0.5, 1.0, 1.0001, 10.0, 0.01, 0.0, -1.0, f32::NAN, f32::INFINITY]);
                ops.push(match rng.below(12) {
                    0 | 1 => Op::Insert(rng.coin().then(|| leaf(&mut rng)), axis),
                    2 | 3 => Op::Split(leaf(&mut rng), axis, rng.coin()),
                    4 => Op::Remove(leaf(&mut rng)),
                    5 | 6 => Op::Resize(leaf(&mut rng), dir),
                    7 => Op::Swap(leaf(&mut rng), leaf(&mut rng)),
                    8 => Op::Equalize,
                    9 => {
                        let zone = rng.pick(&[Zone::Left, Zone::Right, Zone::Top, Zone::Bottom, Zone::Center]);
                        Op::Drop(leaf(&mut rng), leaf(&mut rng), zone)
                    }
                    _ => Op::Scale(leaf(&mut rng), axis, by),
                });
            }
            let fails = |ops: &[Op]| fuzz::caught(|| run(ops)).unwrap_or_else(Err).is_err();
            fails(&ops).then(|| {
                let least = fuzz::shrink(ops, fails);
                let why = fuzz::caught(|| run(&least)).unwrap_or_else(Err).err().unwrap_or_default();
                let ops: Vec<String> = least.iter().map(code).collect();
                format!("seed {seed}: {why}\n    run(&[{}]).unwrap();", ops.join(", "))
            })
        };
        if let Some(report) = fuzz::each_seed(0, 4_000, one) {
            panic!("{report}");
        }
    }
}
