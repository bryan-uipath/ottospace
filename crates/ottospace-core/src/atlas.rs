//! The atlas: a tree of things as a plan seen from above, where a thing is where it was the
//! last time. The same tree gives the same plan, and a change to one thing moves as little else
//! as it can. It knows nothing of what the things are: a source makes the tree (`repo`: a
//! repository's folders and files, the code city). Design: `docs/ai/code-city/plan.md`.
//!
//! A group's children share it in the order the source gives, cut in two near the middle, each
//! half again. Weights are coarse on purpose. A leaf's is its source's size class, so a small
//! change keeps it. A group's is its children's sum rounded up a step (`step`): the room up to
//! the step is ground kept free at the group's end. A new last child lands there and moves at
//! most the few last children beside it; any new child that fits the ground leaves the plan
//! outside its group as it was.

use std::collections::BTreeMap;

use crate::layout::Rect;

/// The plan is cut as if drawn this much wider than tall, whatever the window: a resize
/// stretches the plan and never rearranges it.
const ASPECT: f32 = 1.6;
/// Weights are cut in halves as whole numbers: this many times before a half rounds down.
const HALVINGS: u32 = 24;

/// One thing, or a group of things, as a source gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// Its own for good, and no other node's: what a place, a link or an activity names.
    pub id: String,
    pub label: String,
    /// What sizes a leaf on the plan, already coarse (a source's size classes). A change to it
    /// moves things, so it is a measure that changes slowly (a file's size, a channel's
    /// members). A group's is not read: it weighs its children.
    pub footprint: u64,
    /// A second measure, 0 to 1, that is only drawn and moves nothing, so it may change often
    /// (churn, a month's messages). Not drawn yet.
    pub height: Option<f32>,
    /// In the order they are laid out: one that keeps old things before new ones keeps them
    /// in place.
    pub children: Vec<Node>,
}

/// Two nodes that go together (files changed together, an issue that blocks another), by id.
/// A source gives them beside its tree; the layout does not read them. Not drawn yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub from: String,
    pub to: String,
    pub weight: u32,
}

/// Where a node is, in fractions of the plan (0,0 top left; 1,1 bottom right).
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub id: String,
    pub label: String,
    /// 1 for a child of the root.
    pub depth: usize,
    pub rect: Rect,
    /// It has children: a district, not a building.
    pub group: bool,
    pub height: Option<f32>,
}

/// Every node but the root, a group before what is in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Atlas {
    pub places: Vec<Place>,
}

/// A group's weight: its children's sum, rounded up to 4, 5, 6 or 7 times a power of two
/// (100 → 112). At most a fifth of a group is free ground.
fn step(sum: u64) -> u64 {
    if sum < 8 {
        return sum;
    }
    let shift = sum.ilog2() - 2;
    (sum.div_ceil(1 << shift)).saturating_mul(1 << shift)
}

/// `round`: groups are rounded up a step. Without it (a test's baseline) a group weighs
/// exactly its children.
fn weigh(node: &Node, round: bool) -> u64 {
    if node.children.is_empty() {
        return node.footprint.max(1);
    }
    let sum = node.children.iter().map(|child| weigh(child, round)).fold(0, u64::saturating_add);
    if round { step(sum) } else { sum }
}

/// Edges, so two parts of a cut share one number and no gap opens between them.
#[derive(Clone, Copy)]
struct Edges {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Edges {
    fn rect(self) -> Rect {
        Rect { x: self.x0, y: self.y0, w: self.x1 - self.x0, h: self.y1 - self.y0 }
    }

    /// The first `share` of the longer side, and the rest.
    fn cut(self, share: f32) -> (Edges, Edges) {
        if (self.x1 - self.x0) * ASPECT >= self.y1 - self.y0 {
            let at = self.x0 + (self.x1 - self.x0) * share;
            (Edges { x1: at, ..self }, Edges { x0: at, ..self })
        } else {
            let at = self.y0 + (self.y1 - self.y0) * share;
            (Edges { y1: at, ..self }, Edges { y0: at, ..self })
        }
    }
}

struct Layout {
    round: bool,
    atlas: Atlas,
}

impl Layout {
    /// `children`, each with where its weight ends (the first starts at `from`), share `area`,
    /// which stands for the weights `from` to `to`; past the last child's end is free ground.
    /// The cut is at the child's end nearest the middle (the earlier on a tie). While the
    /// children end before the middle the cut is the middle itself: ground is halved, not
    /// stretched, so a child added at the end moves the others only when it crosses a middle.
    fn tile(&mut self, children: &[(&Node, u64)], (from, to): (u64, u64), area: Edges, depth: usize) {
        let (Some((first, _)), Some((_, end))) = (children.first(), children.last()) else { return };
        if children.len() == 1 && *end >= to {
            let group = !first.children.is_empty();
            let (id, label) = (first.id.clone(), first.label.clone());
            self.atlas.places.push(Place { id, label, depth, rect: area.rect(), group, height: first.height });
            return self.group(first, area, depth + 1);
        }
        let middle = from + (to - from) / 2;
        let (mut at, mut cut) = (children.len(), middle);
        if *end > middle && middle > from {
            let inner = children.iter().enumerate().filter(|(_, (_, end))| *end < to);
            let nearest = inner.min_by_key(|(_, (_, end))| end.abs_diff(middle));
            (at, cut) = nearest.map_or((at, *end), |(i, (_, end))| (i + 1, *end));
        } else if *end > middle {
            // Too fine to halve again: the children take all of it.
            (at, cut) = (children.len(), to);
        }
        let (head, rest) = area.cut((cut - from) as f32 / (to - from) as f32);
        self.tile(&children[..at], (from, cut), head, depth);
        self.tile(&children[at..], (cut, to), rest, depth);
    }

    fn group(&mut self, node: &Node, area: Edges, depth: usize) {
        let mut end = 0u64;
        let ends = node.children.iter().map(|child| {
            end = end.saturating_add(weigh(child, self.round) << HALVINGS);
            (child, end)
        });
        let children: Vec<(&Node, u64)> = ends.collect();
        let whole = if self.round { step(end >> HALVINGS) << HALVINGS } else { end };
        self.tile(&children, (0, whole), area, depth);
    }
}

fn layout(root: &Node, round: bool) -> Atlas {
    let mut layout = Layout { round, atlas: Atlas::default() };
    layout.group(root, Edges { x0: 0., y0: 0., x1: 1., y1: 1. }, 1);
    layout.atlas
}

/// The plan of `root`'s tree: the root is the whole plan.
pub fn atlas(root: &Node) -> Atlas {
    layout(root, true)
}

/// A file's weight: 1 up to 2 KB, doubling with the size, 64 from 128 KB. An edit that keeps
/// the size class changes nothing.
fn file_weight(bytes: u64) -> u64 {
    bytes.div_ceil(2048).clamp(1, 64).next_power_of_two()
}

#[derive(Default)]
struct Folder<'a> {
    folders: BTreeMap<&'a str, Folder<'a>>,
    files: BTreeMap<&'a str, u64>,
}

impl Folder<'_> {
    fn node(&self, path: &str, label: &str) -> Node {
        let join = |name: &str| if path.is_empty() { name.to_string() } else { format!("{path}/{name}") };
        let folders = self.folders.iter().map(|(name, folder)| folder.node(&join(name), name));
        // A file and a folder may share a name in a list that is not git's: the file's id says so.
        let file = |(name, bytes): (&&str, &u64)| {
            let id = if self.folders.contains_key(name) { join(name) + "/" } else { join(name) };
            Node { id, label: name.to_string(), footprint: file_weight(*bytes), ..Node::default() }
        };
        let children = folders.chain(self.files.iter().map(file)).collect();
        Node { id: path.to_string(), label: label.to_string(), children, ..Node::default() }
    }
}

/// The source of the code city. `files`: each a path from the repository's root (parts between
/// `/`) and its size in bytes; their order does not matter, and of two with one path the later
/// counts. A folder's children are its folders by name, then its files by name, as a file
/// explorer lists them. An id is the path.
pub fn repo(files: &[(String, u64)]) -> Node {
    let mut root = Folder::default();
    for (path, bytes) in files {
        let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let Some(name) = parts.pop() else { continue };
        let folder = parts.into_iter().fold(&mut root, |folder, part| folder.folders.entry(part).or_default());
        folder.files.insert(name, *bytes);
    }
    root.node("", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(of: &[(&str, u64)]) -> Vec<(String, u64)> {
        of.iter().map(|(path, bytes)| (path.to_string(), *bytes)).collect()
    }

    fn city(of: &[(&str, u64)]) -> Atlas {
        atlas(&repo(&files(of)))
    }

    fn rect_of(atlas: &Atlas, id: &str) -> Rect {
        atlas.places.iter().find(|place| place.id == id).map(|place| place.rect).expect(id)
    }

    fn leaf(id: &str, weight: u64) -> Node {
        Node { id: id.into(), label: id.into(), footprint: weight, ..Node::default() }
    }

    fn group(id: &str, children: Vec<Node>) -> Node {
        Node { id: id.into(), label: id.into(), children, ..Node::default() }
    }

    #[test]
    fn a_files_weight_is_its_size_class_and_a_groups_is_rounded_up_a_step() {
        let classes = [(0, 1), (2048, 1), (2049, 2), (4096, 2), (4097, 4), (100_000, 64), (u64::MAX, 64)];
        for (bytes, weight) in classes {
            assert_eq!(file_weight(bytes), weight, "{bytes}");
        }
        for (sum, weight) in [(0, 0), (1, 1), (7, 7), (8, 8), (9, 10), (100, 112), (112, 112), (113, 128)] {
            assert_eq!(step(sum), weight, "{sum}");
            assert_eq!(step(weight), weight, "a step of a step is itself");
        }
        assert_eq!(step(u64::MAX), u64::MAX);
    }

    #[test]
    fn folders_come_before_files_each_by_name_and_share_the_plan_by_weight() {
        // src weighs 2 + 1 + 1, the two files in the root 2 and 2: the first cut is in half.
        let city = city(&[("b.md", 4000), ("src/main.rs", 4000), ("a.md", 4000), ("src/a.rs", 10), ("src/z.rs", 10)]);
        let ids: Vec<(&str, &str, bool)> =
            city.places.iter().map(|p| (p.id.as_str(), p.label.as_str(), p.group)).collect();
        let expected = [
            ("src", "src", true),
            ("src/a.rs", "a.rs", false),
            ("src/main.rs", "main.rs", false),
            ("src/z.rs", "z.rs", false),
            ("a.md", "a.md", false),
            ("b.md", "b.md", false),
        ];
        assert_eq!(ids, expected);
        assert_eq!(city.places[0].depth, 1);
        assert_eq!(city.places[1].depth, 2);
        assert_eq!(rect_of(&city, "src"), Rect { x: 0., y: 0., w: 0.5, h: 1. });
        // The halves are taller than wide as drawn (0.5 × 1.6 < 1): cut across.
        assert_eq!(rect_of(&city, "a.md"), Rect { x: 0.5, y: 0., w: 0.5, h: 0.5 });
        assert_eq!(rect_of(&city, "b.md"), Rect { x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
        // src is cut across too: a.rs over the other two, which are then side by side.
        assert_eq!(rect_of(&city, "src/a.rs"), Rect { x: 0., y: 0., w: 0.5, h: 0.25 });
        assert_eq!(rect_of(&city, "src/z.rs"), Rect { x: 1. / 3., y: 0.25, w: 0.5 - 1. / 3., h: 0.75 });
    }

    #[test]
    fn the_order_given_and_odd_paths_do_not_matter() {
        let one = city(&[("a/b.rs", 1), ("a/c.rs", 9000), ("d.rs", 1)]);
        let other = city(&[("d.rs", 5), ("/a//c.rs", 9000), ("", 7), ("///", 7), ("a/b.rs", 1), ("d.rs", 1)]);
        assert_eq!(one, other);
        assert_eq!(city(&[]), Atlas::default());
        // A file and a folder of one name are both there, each with its own id.
        let both = city(&[("a", 1), ("a/b", 1)]);
        let ids: Vec<&str> = both.places.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["a", "a/b", "a/"]);
    }

    #[test]
    fn an_edit_within_a_size_class_moves_nothing_and_a_new_file_stays_in_its_folder() {
        let docs: Vec<String> = (1..=9).map(|i| format!("docs/{i}.md")).collect();
        let mut before: Vec<(&str, u64)> = docs.iter().map(|path| (path.as_str(), 100)).collect();
        before.extend([("src/x.rs", 30_000), ("src/y.rs", 30_000)]);
        let mut edited = before.clone();
        edited[9].1 = 31_000;
        assert_eq!(city(&before), city(&edited));
        // docs weighs 9, a step up 10: a tenth file fits, and nothing but it is new.
        let mut added = before.clone();
        added.push(("docs/a.md", 100));
        let (was, mut is) = (city(&before), city(&added));
        is.places.retain(|place| place.id != "docs/a.md");
        assert_eq!(was, is);
        // An eleventh makes docs a step larger: src, after it, moves over. docs/1.md stays.
        added.push(("docs/b.md", 100));
        let more = city(&added);
        assert_ne!(rect_of(&was, "src"), rect_of(&more, "src"));
        assert_eq!(rect_of(&was, "docs/1.md").x, rect_of(&more, "docs/1.md").x);
    }

    #[test]
    fn a_group_keeps_ground_at_its_end_and_a_new_last_child_lands_there() {
        // Nine issues weigh 9, a step up is 10: a tenth of the plan is free.
        let issues = |n: usize| group("", (1..=n).map(|i| leaf(&format!("P-{i}"), 1)).collect());
        let (nine, ten) = (atlas(&issues(9)), atlas(&issues(10)));
        let covered = |atlas: &Atlas| atlas.places.iter().map(|p| p.rect.w * p.rect.h).sum::<f32>();
        assert!((covered(&nine) - 0.9).abs() < 1e-5 && (covered(&ten) - 1.).abs() < 1e-5);
        // The tenth lands in it: the nine are where they were.
        assert_eq!(nine.places[..], ten.places[..9]);
        // The eleventh makes the group a step larger: now all move, once.
        assert_ne!(atlas(&issues(11)).places[..9], nine.places[..]);
    }

    /// Seeded random trees: what must hold of every plan, and how much one change moves.
    mod prop {
        use super::*;
        use crate::fuzz::{self, Rng};

        const NAMES: [&str; 12] = ["a", "b", "core", "src", "ui", "test", "docs", "x", "y", "lib", "mod", "z"];

        fn path(rng: &mut Rng) -> String {
            let folders: Vec<&str> = (0..rng.below(5)).map(|_| rng.pick(&NAMES[..6])).collect();
            let file = format!("{}{}.rs", rng.pick(&NAMES), rng.below(40));
            folders.into_iter().chain([file.as_str()]).collect::<Vec<_>>().join("/")
        }

        /// Sizes spread evenly over the classes: 1 byte to 512 KB.
        fn bytes(rng: &mut Rng) -> u64 {
            (1 << rng.below(20)) + rng.below(1000) as u64
        }

        fn files(rng: &mut Rng, most: usize) -> Vec<(String, u64)> {
            let files: BTreeMap<String, u64> = (0..1 + rng.below(most)).map(|_| (path(rng), bytes(rng))).collect();
            files.into_iter().collect()
        }

        #[derive(Clone, Debug)]
        enum Change {
            Add(String, u64),
            Remove(usize),
            Resize(usize, u64),
        }

        fn change(rng: &mut Rng) -> Change {
            match rng.below(3) {
                0 => Change::Add(path(rng), bytes(rng)),
                1 => Change::Remove(rng.below(1000)),
                _ => Change::Resize(rng.below(1000), bytes(rng)),
            }
        }

        /// The files after `change`, and the path it touched.
        fn apply(files: &[(String, u64)], change: &Change) -> (Vec<(String, u64)>, String) {
            let mut after: BTreeMap<String, u64> = files.iter().cloned().collect();
            let at = |i: &usize| files[i % files.len()].0.clone();
            let path = match change {
                Change::Add(path, bytes) => {
                    after.insert(path.clone(), *bytes);
                    path.clone()
                }
                Change::Remove(i) => {
                    after.remove(&at(i));
                    at(i)
                }
                Change::Resize(i, bytes) => {
                    after.insert(at(i), *bytes);
                    at(i)
                }
            };
            (after.into_iter().collect(), path)
        }

        /// Each group's weight by id ("" is the root).
        fn weights(node: &Node, into: &mut BTreeMap<String, u64>) {
            if !node.children.is_empty() {
                into.insert(node.id.clone(), weigh(node, true));
                node.children.iter().for_each(|child| weights(child, into));
            }
        }

        /// The deepest folder over `path` that weighs the same in both trees: nothing outside
        /// it may move, nor may its own district. With none, the root: the whole plan.
        fn anchor(was: &Node, is: &Node, path: &str) -> String {
            let (mut before, mut after) = (BTreeMap::new(), BTreeMap::new());
            weights(was, &mut before);
            weights(is, &mut after);
            let mut folder = path;
            while let Some((parent, _)) = folder.rsplit_once('/') {
                if before.get(parent).is_some_and(|weight| Some(weight) == after.get(parent)) {
                    return parent.to_string();
                }
                folder = parent;
            }
            String::new()
        }

        fn inside(id: &str, folder: &str) -> bool {
            folder.is_empty() || id.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
        }

        fn over(a: Rect, b: Rect) -> bool {
            let (x, y) = ((a.x + a.w).min(b.x + b.w) - a.x.max(b.x), (a.y + a.h).min(b.y + b.h) - a.y.max(b.y));
            x > 1e-6 && y > 1e-6
        }

        fn within(a: Rect, b: Rect) -> bool {
            let (right, bottom) = (b.x + b.w - (a.x + a.w), b.y + b.h - (a.y + a.h));
            a.x >= b.x - 1e-6 && a.y >= b.y - 1e-6 && right >= -1e-6 && bottom >= -1e-6
        }

        /// Every plan: each node once, inside the plan and its group, no two of a group over
        /// each other, and each child's share of its group is its weight's.
        fn sound(files: &[(String, u64)]) -> Result<Atlas, String> {
            let root = repo(files);
            let plan = atlas(&root);
            let mut shuffled = files.to_vec();
            shuffled.reverse();
            if plan != atlas(&repo(&shuffled)) || plan != atlas(&root) {
                return Err("the same files gave another plan".into());
            }
            let leaves: Vec<&Place> = plan.places.iter().filter(|place| !place.group).collect();
            let mut given: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
            let mut placed: Vec<&str> = leaves.iter().map(|place| place.id.as_str()).collect();
            given.sort_unstable();
            placed.sort_unstable();
            if given != placed {
                return Err(format!("placed {placed:?} of {given:?}"));
            }
            let whole = Rect { x: 0., y: 0., w: 1., h: 1. };
            if let Some(out) = plan.places.iter().find(|p| !(p.rect.w > 0. && p.rect.h > 0. && within(p.rect, whole))) {
                return Err(format!("{} is at {:?}", out.id, out.rect));
            }
            for (i, a) in leaves.iter().enumerate() {
                if let Some(b) = leaves[i + 1..].iter().find(|b| over(a.rect, b.rect)) {
                    return Err(format!("{} {:?} lies over {} {:?}", a.id, a.rect, b.id, b.rect));
                }
            }
            for district in plan.places.iter().filter(|place| place.group) {
                if let Some(out) =
                    plan.places.iter().find(|p| inside(&p.id, &district.id) && !within(p.rect, district.rect))
                {
                    return Err(format!("{} {:?} is outside {} {:?}", out.id, out.rect, district.id, district.rect));
                }
            }
            shares(&root, whole, &plan)?;
            Ok(plan)
        }

        /// Each child's area is its weight's share of its group's.
        fn shares(node: &Node, rect: Rect, plan: &Atlas) -> Result<(), String> {
            let whole = weigh(node, true) as f32;
            for child in &node.children {
                let at = rect_of(plan, &child.id);
                let (is, should) = (at.w * at.h, rect.w * rect.h * weigh(child, true) as f32 / whole);
                if (is - should).abs() > should * 1e-3 + 1e-9 {
                    return Err(format!("{} covers {is}, its weight says {should}", child.id));
                }
                shares(child, at, plan)?;
            }
            Ok(())
        }

        /// One change moves only what is in the anchor folder; the rest is bit for bit the same.
        fn step_holds(files: &[(String, u64)], change: &Change) -> Result<(), String> {
            let was = sound(files)?;
            let (after, touched) = apply(files, change);
            let is = sound(&after)?;
            let anchor = anchor(&repo(files), &repo(&after), &touched);
            let kept = |place: &&Place| !inside(&place.id, &anchor);
            let (was_kept, is_kept): (Vec<_>, Vec<_>) =
                (was.places.iter().filter(kept).collect(), is.places.iter().filter(kept).collect());
            if was_kept != is_kept {
                return Err(format!("{change:?} moved a place outside {anchor:?}"));
            }
            Ok(())
        }

        fn run(seed: u64) -> Option<String> {
            let mut rng = Rng::new(seed);
            let files = files(&mut rng, 60);
            let changes: Vec<Change> = (0..8).map(|_| change(&mut rng)).collect();
            let run = |changes: &[Change]| {
                let mut files = files.clone();
                for change in changes {
                    step_holds(&files, change)?;
                    files = apply(&files, change).0;
                    if files.is_empty() {
                        break;
                    }
                }
                Ok(())
            };
            let fails = |changes: &[Change]| fuzz::caught(|| run(changes)).unwrap_or_else(Err).is_err();
            fails(&changes).then(|| {
                let least = fuzz::shrink(changes, fails);
                let why = fuzz::caught(|| run(&least)).unwrap_or_else(Err).err().unwrap_or_default();
                format!("seed {seed}: {why}\nfiles: {files:?}\nchanges: {least:?}")
            })
        }

        #[test]
        fn random_repositories_keep_the_rules_and_a_change_stays_in_its_anchor() {
            if let Some(report) = fuzz::each_seed(0, 2_000, run) {
                panic!("{report}");
            }
        }

        /// A source that churns: groups whose things only come at the end, as issues or
        /// threads do. A new last thing that fits its group's free ground moves nothing outside
        /// the group, and in it only the few last things beside the ground it lands in.
        #[test]
        fn a_thing_added_at_the_end_of_its_group_moves_only_a_few_of_the_last_things() {
            let tree = |sizes: &[Vec<u64>]| {
                let lane = |(i, sizes): (usize, &Vec<u64>)| {
                    let things = sizes.iter().enumerate().map(|(j, w)| leaf(&format!("{i}/{j}"), *w));
                    group(&i.to_string(), things.collect())
                };
                group("", sizes.iter().enumerate().map(lane).collect())
            };
            let (mut fits, mut still, mut share) = (0, 0, 0.);
            for seed in 0..2_000 {
                let mut rng = Rng::new(seed);
                let lanes = (0..2 + rng.below(6)).map(|_| (0..10 + rng.below(40)).map(|_| 1 << rng.below(4)).collect());
                let mut sizes: Vec<Vec<u64>> = lanes.collect();
                let was = tree(&sizes);
                let (lane, weight) = (rng.below(sizes.len()), 1 << rng.below(3));
                let sum: u64 = sizes[lane].iter().sum();
                sizes[lane].push(weight);
                if step(sum) != step(sum + weight) {
                    continue;
                }
                let (before, after) = (atlas(&was), atlas(&tree(&sizes)));
                let moved: Vec<&Place> = before.places.iter().filter(|p| rect_of(&after, &p.id) != p.rect).collect();
                let own = format!("{lane}/");
                assert!(moved.iter().all(|p| p.id.starts_with(&own)), "seed {seed}: moved outside the group");
                // Those that moved are the last of the group, with no still thing among them.
                let first = moved.iter().filter_map(|p| p.id[own.len()..].parse::<usize>().ok()).min();
                assert!(first.is_none_or(|first| first + moved.len() == sizes[lane].len() - 1), "seed {seed}");
                fits += 1;
                still += usize::from(moved.is_empty());
                share += moved.len() as f32 / (sizes[lane].len() - 1) as f32;
            }
            let (still, share) = (still as f32 / fits as f32, share / fits as f32);
            println!("a new last thing moves none of its group {still} of the time; on average {share} of the group");
            assert!(fits > 500 && still > 0.5 && share < 0.1, "{fits} {still} {share}");
        }

        /// The share of the other leaves one change moves, over `seeds` repositories of about
        /// 400 files: (mean, the share of changes that move none).
        fn moved(seeds: u64, round: bool, change: impl Fn(&mut Rng, &[(String, u64)]) -> Change) -> (f32, f32) {
            let (mut sum, mut still) = (0., 0);
            for seed in 0..seeds {
                let mut rng = Rng::new(seed);
                let files = files(&mut rng, 800);
                let (after, _) = apply(&files, &change(&mut rng, &files));
                let (was, is) = (layout(&repo(&files), round), layout(&repo(&after), round));
                let was_at: BTreeMap<&str, Rect> = was.places.iter().map(|p| (p.id.as_str(), p.rect)).collect();
                let both =
                    is.places.iter().filter(|p| !p.group).filter_map(|p| Some((was_at.get(p.id.as_str())?, &p.rect)));
                let (all, moved) = both.fold((0, 0), |(all, moved), (a, b)| (all + 1, moved + usize::from(a != b)));
                sum += moved as f32 / all.max(1) as f32;
                still += usize::from(moved == 0);
            }
            (sum / seeds as f32, still as f32 / seeds as f32)
        }

        /// The numbers in the design note: `cargo test -p ottospace-core atlas -- --nocapture`.
        #[test]
        fn one_change_moves_a_small_share_of_the_city_and_far_less_than_exact_weights() {
            let add = |rng: &mut Rng, _: &[(String, u64)]| Change::Add(path(rng) + "x", bytes(rng));
            // An edit: a file a tenth larger.
            let edit = |rng: &mut Rng, files: &[(String, u64)]| {
                let at = rng.below(files.len());
                Change::Resize(at, files[at].1 + files[at].1 / 10 + 1)
            };
            let (add_coarse, add_exact) = (moved(300, true, add), moved(300, false, add));
            let (edit_coarse, edit_exact) = (moved(300, true, edit), moved(300, false, edit));
            println!("a new file moves {add_coarse:?} of the others (mean, share of changes that move none)");
            println!("  with exact group weights {add_exact:?}");
            println!("an edit of a tenth moves {edit_coarse:?}; with exact group weights {edit_exact:?}");
            assert!(add_coarse.0 < 0.2 && add_coarse.0 < add_exact.0 / 3., "{add_coarse:?} {add_exact:?}");
            assert!(edit_coarse.1 > 0.8 && edit_coarse.0 < 0.05, "{edit_coarse:?}");
        }
    }
}
