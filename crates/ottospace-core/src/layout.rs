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
    Split { axis: Axis, a: Box<Node>, b: Box<Node> },
}

impl Node {
    fn pair(axis: Axis, a: Node, b: Node) -> Node {
        Node::Split { axis, a: Box::new(a), b: Box::new(b) }
    }

    /// Adds `new` after leaf `beside`, splitting it along `axis`. With no such leaf, splits the root.
    pub fn insert(self, beside: Option<u32>, new: u32, axis: Axis) -> Node {
        let mut root = self;
        if beside.is_some_and(|at| root.split(at, new, axis)) {
            return root;
        }
        Node::pair(axis, root, Node::Leaf(new))
    }

    fn split(&mut self, at: u32, new: u32, axis: Axis) -> bool {
        match self {
            Node::Leaf(id) if *id == at => {
                *self = Node::pair(axis, Node::Leaf(at), Node::Leaf(new));
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(at, new, axis) || b.split(at, new, axis),
        }
    }

    /// Removes leaf `id`; its sibling takes the parent's place. `None` once the tree is empty.
    pub fn remove(self, id: u32) -> Option<Node> {
        match self {
            Node::Leaf(leaf) => (leaf != id).then_some(Node::Leaf(leaf)),
            Node::Split { axis, a, b } => match (a.remove(id), b.remove(id)) {
                (Some(a), Some(b)) => Some(Node::pair(axis, a, b)),
                (a, b) => a.or(b),
            },
        }
    }

    /// Each leaf's rect when the tree fills `area`. Splits are equal halves.
    pub fn rects(&self, area: Rect) -> Vec<(u32, Rect)> {
        match self {
            Node::Leaf(id) => vec![(*id, area)],
            Node::Split { axis, a, b } => {
                let (first, second) = match axis {
                    Axis::Right => {
                        let w = area.w / 2.0;
                        (Rect { w, ..area }, Rect { x: area.x + w, w, ..area })
                    }
                    Axis::Down => {
                        let h = area.h / 2.0;
                        (Rect { h, ..area }, Rect { y: area.y + h, h, ..area })
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
}

#[cfg(test)]
mod tests {
    use super::{Axis::*, Dir, Node, Node::Leaf};

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
}
