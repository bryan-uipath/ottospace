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

use std::collections::{BTreeMap, HashMap};

use crate::layout::Rect;
use crate::strata::{self, History};

/// The plan is cut as if drawn this much wider than tall, whatever the window: a resize
/// stretches the plan and never rearranges it.
const ASPECT: f32 = 1.6;
/// Weights are cut in halves as whole numbers: this many times before a half rounds down.
const HALVINGS: u32 = 24;
/// The most a leaf weighs: a size class, not a count. Rectangles are `f32`: a leaf a millionth
/// of its group still has a width.
pub const MOST: u64 = 1 << 16;

/// One thing, or a group of things, as a source gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// Its own for good, and no other node's: what a place or an activity names.
    pub id: String,
    pub label: String,
    /// What sizes a leaf on the plan, already coarse (a source's size classes). A change to it
    /// moves things, so it is a measure that changes slowly (a file's size, a channel's
    /// members). 1 to `MOST`. A group's is not read: it weighs its children.
    pub footprint: u64,
    /// A second measure, 0 to 1, that is only drawn and moves nothing, so it may change often
    /// (churn, a month's messages). The code city's: a building's height (`scene::height`).
    pub height: Option<f32>,
    /// Its levels' colours, the base first (drawn only, as `height`). The code city's: a file's
    /// commits (`strata`).
    pub strata: Vec<u32>,
    /// In the order they are laid out: one that keeps old things before new ones keeps them
    /// in place.
    pub children: Vec<Node>,
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
    pub strata: Vec<u32>,
}

/// Every node but the root, a group before what is in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Atlas {
    pub places: Vec<Place>,
}

impl Atlas {
    /// The leaves of a `repo` plan that a file's label names (`station::path_label`: the last
    /// two parts of its path, or its name): each whose id is the label or ends with it after
    /// a `/`. `folder` is the repository's own folder's name: a file in the root is labelled
    /// `folder/name`.
    pub fn labelled<'a>(&'a self, label: &'a str, folder: &'a str) -> impl Iterator<Item = &'a Place> {
        let root = label.strip_prefix(folder).and_then(|rest| rest.strip_prefix('/')).filter(|_| !folder.is_empty());
        let ends =
            move |id: &str| id.strip_suffix(label).is_some_and(|before| before.is_empty() || before.ends_with('/'));
        let named = move |id: &str| ends(id) || Some(id) == root;
        self.places.iter().filter(move |place| !place.group && named(&place.id))
    }
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
        return node.footprint.clamp(1, MOST);
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
    fn tile(&mut self, children: &[(&Node, u128)], (from, to): (u128, u128), area: Edges, depth: usize) {
        let (Some((first, _)), Some((_, end))) = (children.first(), children.last()) else { return };
        if children.len() == 1 && *end >= to {
            let group = !first.children.is_empty();
            let (id, label) = (first.id.clone(), first.label.clone());
            let (height, strata) = (first.height, first.strata.clone());
            self.atlas.places.push(Place { id, label, depth, rect: area.rect(), group, height, strata });
            return self.group(first, area, depth + 1);
        }
        let middle = from + (to - from) / 2;
        let (mut at, mut cut) = (children.len(), middle);
        if *end > middle && middle > from {
            let inner = children.iter().enumerate().filter(|(_, (_, end))| *end < to);
            let nearest = inner.min_by_key(|(_, (_, end))| end.abs_diff(middle));
            (at, cut) = nearest.map_or((at, *end), |(i, (_, end))| (i + 1, *end));
        } else if *end > middle {
            // Too fine to halve again (no weight is this small): nothing more is placed.
            return;
        }
        let (head, rest) = area.cut(((cut - from) as f64 / (to - from) as f64) as f32);
        self.tile(&children[..at], (from, cut), head, depth);
        self.tile(&children[at..], (cut, to), rest, depth);
    }

    fn group(&mut self, node: &Node, area: Edges, depth: usize) {
        // Wider than a weight, so a shift drops nothing: the cuts stay whole numbers.
        let (mut end, mut sum) = (0u128, 0u64);
        let ends = node.children.iter().map(|child| {
            let weight = weigh(child, self.round);
            (end, sum) = (end + (u128::from(weight) << HALVINGS), sum.saturating_add(weight));
            (child, end)
        });
        let children: Vec<(&Node, u128)> = ends.collect();
        let whole = if self.round { (u128::from(step(sum)) << HALVINGS).max(end) } else { end };
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

/// A file's weight: its lines, as the owner's city sizes a file (area by lines, at least 1),
/// rounded up a step as a group is (100 → 112), so an edit seldom changes it; at most `MOST`.
fn file_weight(lines: u64) -> u64 {
    step(lines.max(1)).min(MOST)
}

#[derive(Default)]
struct Folder<'a> {
    folders: BTreeMap<&'a str, Folder<'a>>,
    files: BTreeMap<&'a str, u64>,
}

impl Folder<'_> {
    fn node(&self, path: &str, label: &str, strata: &dyn Fn(&str) -> Vec<u32>) -> Node {
        let join = |name: &str| if path.is_empty() { name.to_string() } else { format!("{path}/{name}") };
        let folders = self.folders.iter().map(|(name, folder)| folder.node(&join(name), name, strata));
        // A file and a folder may share a name in a list that is not git's: the file's id says so.
        let file = |(name, lines): (&&str, &u64)| {
            let id = if self.folders.contains_key(name) { join(name) + "/" } else { join(name) };
            let strata = strata(&join(name));
            let height = Some(crate::scene::height(strata.len() as u32));
            Node { id, label: name.to_string(), footprint: file_weight(*lines), height, strata, ..Node::default() }
        };
        let children = folders.chain(self.files.iter().map(file)).collect();
        Node { id: path.to_string(), label: label.to_string(), children, ..Node::default() }
    }
}

/// The source of the code city. `files`: each a path from the repository's root (parts between
/// `/`) and its lines; their order does not matter, and of two with one path the later counts.
/// A folder's children are its folders by name, then its files by name, as a file explorer lists
/// them. An id is the path. No file has commits: each is as low as one with none.
pub fn repo(files: &[(String, u64)]) -> Node {
    repo_with(files, &History::default())
}

/// `repo`, each file a level for each commit of `history` that touched it, the newest first, in
/// its colour (`strata::colour`, its age through the history), as tall (`scene::height`): at most
/// `scene::LEVELS`, and fewer for all if the city's would be more than `MOST_LEVELS`.
pub fn repo_with(files: &[(String, u64)], history: &History) -> Node {
    let mut root = Folder::default();
    for (path, lines) in files {
        let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let Some(name) = parts.pop() else { continue };
        let folder = parts.into_iter().fold(&mut root, |folder, part| folder.folders.entry(part).or_default());
        folder.files.insert(name, *lines);
    }
    let levels = |path: &str| history.files.get(path).map_or(&[][..], Vec::as_slice);
    let most = most_levels(&files.iter().map(|(path, _)| levels(path).len()).collect::<Vec<_>>());
    let (oldest, span) = (history.span.0, history.span.1.saturating_sub(history.span.0).max(1));
    let age = |at: u64| at.saturating_sub(oldest) as f32 / span as f32;
    root.node("", "", &|id| levels(id).iter().take(most).map(|l| strata::colour(l.change, age(l.at))).collect())
}

/// The levels drawn in a city at most: boxes for the GPU, one each.
const MOST_LEVELS: usize = 100_000;

/// The most levels each file stands, its `counts` levels given: `scene::LEVELS`, less while the
/// whole city's would be more than `MOST_LEVELS` (the newest of each kept). A file of commits
/// has a stub too: one more box.
fn most_levels(counts: &[usize]) -> usize {
    let total = |most: usize| counts.iter().map(|n| (*n).min(most) + usize::from(*n > 0)).sum::<usize>();
    (0..=crate::scene::LEVELS as usize).rev().find(|most| total(*most) <= MOST_LEVELS).unwrap_or(0)
}

/// Lines added and removed by each path, from `git diff --numstat -z --no-renames`: each record
/// `added\tremoved\tpath` and a NUL; a binary file's numbers are `-` (none).
pub fn numstat(said: &str) -> HashMap<String, Option<(u64, u64)>> {
    let record = |r: &str| {
        let mut parts = r.splitn(3, '\t');
        let (added, removed, path) = (parts.next()?, parts.next()?, parts.next()?);
        Some((path.to_string(), added.parse().ok().zip(removed.parse().ok())))
    };
    said.split('\0').filter(|r| !r.is_empty()).filter_map(record).collect()
}

/// Each blob's size in bytes, from `git ls-tree -r -l -z`: each record `mode type object size`, a
/// tab, the path and a NUL (a submodule's size is `-`: none).
pub fn tree_sizes(said: &str) -> HashMap<String, u64> {
    let record = |r: &str| {
        let (meta, path) = r.split_once('\t')?;
        Some((path.to_string(), meta.split_whitespace().nth(3)?.parse().ok()?))
    };
    said.split('\0').filter_map(record).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(of: &[(&str, u64)]) -> Vec<(String, u64)> {
        of.iter().map(|(path, lines)| (path.to_string(), *lines)).collect()
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
    fn a_files_weight_is_its_lines_and_a_groups_is_rounded_up_a_step() {
        let lines = [(0, 1), (1, 1), (7, 7), (9, 10), (100, 112), (112, 112), (113, 128), (1 << 20, MOST)];
        for (lines, weight) in lines.into_iter().chain([(u64::MAX, MOST)]) {
            assert_eq!(file_weight(lines), weight, "{lines}");
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
        let city = city(&[("b.md", 2), ("src/main.rs", 2), ("a.md", 2), ("src/a.rs", 1), ("src/z.rs", 1)]);
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
    fn a_label_names_the_files_whose_path_ends_with_it() {
        let files =
            [("src/ui/mod.rs", 1), ("src/mod.rs", 1), ("ui/mod.rs", 1), ("gui/mod.rs", 1), ("ui/mod.rs.bak", 1)];
        let city = city(&[&files[..], &[("top.rs", 1)]].concat());
        fn named<'a>(city: &'a Atlas, label: &'a str) -> Vec<&'a str> {
            city.labelled(label, "app").map(|place| place.id.as_str()).collect()
        }
        assert_eq!(named(&city, "ui/mod.rs"), ["src/ui/mod.rs", "ui/mod.rs"]);
        assert_eq!(named(&city, "src/mod.rs"), ["src/mod.rs"]);
        assert_eq!(named(&city, "mod.rs").len(), 4);
        // A file in the root is labelled with the repository's folder: `/code/app/top.rs`.
        assert_eq!(named(&city, "app/top.rs"), ["top.rs"]);
        // A folder is never lit, nor a label that names nothing here (`file`: what is no path).
        let none: Vec<&str> = Vec::new();
        assert_eq!(
            (named(&city, "ui"), named(&city, "file"), named(&city, "")),
            (none.clone(), none.clone(), none.clone())
        );
        assert_eq!((named(&city, "app/mod.rs"), named(&city, "other/top.rs")), (none.clone(), none));
    }

    /// A file is as tall as the commits git says touched it, a level each, the newest first in the
    /// colour of its kind and age; one it names no commit of is a stub.
    #[test]
    fn a_files_height_and_strata_are_the_commits_that_touched_it() {
        let log = "\x01300\tfeat: x\0\nsrc/a.rs\0b.md\0\x01200\tfix: y\0\nsrc/a.rs\0\x01100\tz\0\ngone.rs\0";
        let plan = atlas(&repo_with(&files(&[("src/a.rs", 9), ("b.md", 9), ("c.md", 9)]), &strata::history(log)));
        let at = |id: &str| plan.places.iter().find(|p| p.id == id).map(|p| (p.height, p.strata.clone()));
        let level = crate::scene::height;
        let (feat, fix) =
            (strata::colour(Some(strata::Change::Feat), 1.), strata::colour(Some(strata::Change::Fix), 0.5));
        assert_eq!(at("src/a.rs"), Some((Some(level(2)), vec![feat, fix])));
        assert_eq!(at("b.md"), Some((Some(level(1)), vec![feat])));
        assert_eq!(at("c.md"), Some((Some(0.), vec![])));
        // Heights and strata move nothing.
        let rects = |a: Atlas| a.places.into_iter().map(|p| p.rect).collect::<Vec<_>>();
        assert_eq!(rects(plan), rects(city(&[("src/a.rs", 9), ("b.md", 9), ("c.md", 9)])));
    }

    /// git's count of a file's lines: what each path added and removed, none for a binary one.
    #[test]
    fn numstat_gives_each_paths_lines_added_and_removed() {
        let said = numstat("12\t0\tsrc/a.rs\0-\t-\tlogo.png\0003\t1\ta b/c\td.md\0\0bad\0");
        let want: HashMap<String, Option<(u64, u64)>> =
            [("src/a.rs".into(), Some((12, 0))), ("logo.png".into(), None), ("a b/c\td.md".into(), Some((3, 1)))]
                .into();
        assert_eq!(said, want);
    }

    #[test]
    fn ls_tree_gives_each_blobs_size() {
        let said = "100644 blob 6d86ca5     639\tsrc/a b.rs\0160000 commit 24d8dc9       -\tvendor\0bad\0";
        assert_eq!(tree_sizes(said), [("src/a b.rs".to_string(), 639)].into());
    }

    /// A city of more levels than the GPU is given draws fewer of each file's: the newest.
    #[test]
    fn a_city_of_too_many_levels_draws_fewer_of_each() {
        assert_eq!(most_levels(&[3, 500, 0]), crate::scene::LEVELS as usize);
        let many = vec![200; 2_000];
        let most = most_levels(&many);
        assert_eq!(most, MOST_LEVELS / 2_000 - 1, "a stub each");
        assert!(many.iter().map(|n| (*n).min(most + 1) + 1).sum::<usize>() > MOST_LEVELS);
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
    fn an_edit_within_a_step_moves_nothing_and_a_new_file_stays_in_its_folder() {
        let docs: Vec<String> = (1..=9).map(|i| format!("docs/{i}.md")).collect();
        let mut before: Vec<(&str, u64)> = docs.iter().map(|path| (path.as_str(), 1)).collect();
        before.extend([("src/x.rs", 100), ("src/y.rs", 100)]);
        let mut edited = before.clone();
        // 100 lines and 112 weigh the same.
        edited[9].1 = 112;
        let rects = |atlas: Atlas| atlas.places.into_iter().map(|p| (p.id, p.rect)).collect::<Vec<_>>();
        assert_eq!(rects(city(&before)), rects(city(&edited)));
        // docs weighs 9, a step up 10: a tenth file fits, and nothing but it is new.
        let mut added = before.clone();
        added.push(("docs/a.md", 1));
        let (was, mut is) = (city(&before), city(&added));
        is.places.retain(|place| place.id != "docs/a.md");
        assert_eq!(was, is);
        // An eleventh makes docs a step larger: src, after it, moves over. docs/1.md stays.
        added.push(("docs/b.md", 1));
        let more = city(&added);
        assert_ne!(rect_of(&was, "src"), rect_of(&more, "src"));
        assert_eq!(rect_of(&was, "docs/1.md").x, rect_of(&more, "docs/1.md").x);
    }

    /// Weights no source should give still end: none is lost in a shift, and the huge one is
    /// a leaf of the most a leaf weighs.
    #[test]
    fn huge_weights_and_deep_trees_still_give_a_plan() {
        let plan = atlas(&group("", vec![leaf("a", 1 << 40), leaf("b", u64::MAX), leaf("c", 1)]));
        assert_eq!(plan.places.len(), 3);
        assert!(rect_of(&plan, "a").w > 0.3 && rect_of(&plan, "c").w > 0. && rect_of(&plan, "c").h > 0.);
        // 400 folders, one in the other, each with a file beside the next.
        let nest = |inner: Node, i: u32| group(&format!("g{i}"), vec![inner, leaf(&format!("f{i}"), MOST)]);
        let plan = atlas(&group("", vec![(0..400).fold(leaf("f", 8), nest)]));
        assert_eq!(plan.places.len(), 801);
        assert!(plan.places.iter().all(|place| place.rect.w >= 0. && place.rect.h >= 0. && place.rect.x.is_finite()));
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

        /// Lines spread evenly over the powers of two: 1 to about 4,000.
        fn lines(rng: &mut Rng) -> u64 {
            (1 << rng.below(12)) + rng.below(100) as u64
        }

        fn files(rng: &mut Rng, most: usize) -> Vec<(String, u64)> {
            let files: BTreeMap<String, u64> = (0..1 + rng.below(most)).map(|_| (path(rng), lines(rng))).collect();
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
                0 => Change::Add(path(rng), lines(rng)),
                1 => Change::Remove(rng.below(1000)),
                _ => Change::Resize(rng.below(1000), lines(rng)),
            }
        }

        /// The files after `change`, and the path it touched.
        fn apply(files: &[(String, u64)], change: &Change) -> (Vec<(String, u64)>, String) {
            let mut after: BTreeMap<String, u64> = files.iter().cloned().collect();
            let at = |i: &usize| files[i % files.len()].0.clone();
            let path = match change {
                Change::Add(path, lines) => {
                    after.insert(path.clone(), *lines);
                    path.clone()
                }
                Change::Remove(i) => {
                    after.remove(&at(i));
                    at(i)
                }
                Change::Resize(i, lines) => {
                    after.insert(at(i), *lines);
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
            // Every node but the root has one place, a group's before its children's.
            fn nodes<'a>(node: &'a Node, into: &mut Vec<(&'a str, bool)>) {
                for child in &node.children {
                    into.push((&child.id, !child.children.is_empty()));
                    nodes(child, into);
                }
            }
            let mut every = Vec::new();
            nodes(&root, &mut every);
            if every != plan.places.iter().map(|place| (place.id.as_str(), place.group)).collect::<Vec<_>>() {
                return Err(format!("placed {:?} of {every:?}", plan.places));
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

        /// How long a plan takes, for the design note's costs:
        /// `cargo test -p ottospace-core --release -- --ignored a_plan_is_timed --nocapture`.
        /// There is no shorter path for one change: a change is the whole plan again.
        #[test]
        #[ignore = "prints timings; run it by name, in release"]
        fn a_plan_is_timed() {
            for count in [500, 5_000, 20_000, 50_000] {
                let mut rng = Rng::new(count as u64);
                let folder = |rng: &mut Rng| format!("d{}", rng.below(12));
                let mut files: Vec<(String, u64)> = (0..count)
                    .map(|i| {
                        let folders: Vec<String> = (0..1 + rng.below(5)).map(|_| folder(&mut rng)).collect();
                        (format!("{}/f{i}.rs", folders.join("/")), lines(&mut rng))
                    })
                    .collect();
                let timed = |files: &[(String, u64)]| {
                    let started = std::time::Instant::now();
                    let (mut tree, mut plan) = (0., 0.);
                    for _ in 0..5 {
                        let before = std::time::Instant::now();
                        let root = repo(files);
                        tree += before.elapsed().as_secs_f64();
                        let before = std::time::Instant::now();
                        let places = atlas(&root).places.len();
                        plan += before.elapsed().as_secs_f64();
                        assert!(places > files.len());
                    }
                    (tree * 200., plan * 200., started.elapsed().as_secs_f64() * 200.)
                };
                let whole = timed(&files);
                files.push(("d0/d1/new.rs".into(), 5_000));
                let after = timed(&files);
                println!(
                    "{count} files: tree {:.2} ms, plan {:.2} ms, both {:.2} ms; after one more file {:.2} ms",
                    whole.0, whole.1, whole.2, after.2
                );
            }
        }

        /// The numbers in the design note: `cargo test -p ottospace-core atlas -- --nocapture`.
        #[test]
        fn one_change_moves_a_small_share_of_the_city_and_far_less_than_exact_weights() {
            let add = |rng: &mut Rng, _: &[(String, u64)]| Change::Add(path(rng) + "x", lines(rng));
            // An edit: `more` lines added to a file, or a tenth more.
            let edit = |more: Option<u64>| {
                move |rng: &mut Rng, files: &[(String, u64)]| {
                    let at = rng.below(files.len());
                    Change::Resize(at, files[at].1 + more.unwrap_or(files[at].1 / 10 + 1))
                }
            };
            let (add_coarse, add_exact) = (moved(300, true, add), moved(300, false, add));
            let (line, ten) = (moved(300, true, edit(Some(1))), moved(300, true, edit(Some(10))));
            let (tenth, tenth_exact) = (moved(300, true, edit(None)), moved(300, false, edit(None)));
            println!("a new file moves {add_coarse:?} of the others (mean, share of changes that move none)");
            println!("  with exact group weights {add_exact:?}");
            println!("a line added moves {line:?}; ten lines {ten:?}");
            println!("a tenth more moves {tenth:?}; with exact group weights {tenth_exact:?}");
            assert!(add_coarse.0 < 0.3 && add_coarse.0 < add_exact.0 / 3., "{add_coarse:?} {add_exact:?}");
            assert!(line.1 > 0.8 && line.0 < 0.02, "{line:?}");
            assert!(tenth.0 < tenth_exact.0 / 3., "{tenth:?} {tenth_exact:?}");
        }
    }
}
