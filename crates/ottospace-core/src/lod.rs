//! Level of detail for the code city: which boxes stand for their contents and which open to
//! show them. One ladder, any depth: a box with children (a folder's plinth today; a file's
//! building once its insides are parts) opens when its children would be large enough on
//! screen, and closes when they would not. Nothing is laid out again: a level only shows or
//! hides boxes inside an outline that was there already, and raises a closed box to stand for
//! what it hides. Design: `docs/ai/code-city/zoom-detail.md`.

use crate::atlas::Atlas;
use crate::scene::{Block, Camera, Kind};

/// A box opens when its children's lots would be this many pixels a side on screen, and closes
/// again below `CLOSE_PX`: between the two it keeps its level (hysteresis).
pub const OPEN_PX: f32 = 3.;
pub const CLOSE_PX: f32 = 2.;
/// A box's level never changes again within this long of its last change: no flicker.
pub const HOLD_MS: u64 = 300;
/// A level change grows children out of (or sinks them into) their box over this long; at once
/// under Reduce Motion.
pub const FADE_MS: u64 = 200;

/// The ladder of a laid-out city: each box's parent (the box it stands in), and for each box
/// with children what it needs to open and to stand for them closed.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    parent: Vec<Option<usize>>,
    /// Plan units a side of its children's lots, on average; 0: no children (a leaf).
    child_side: Vec<f32>,
    centre: Vec<[f32; 2]>,
    /// Closed, how far above its own top it stands: its leaves' heights above it, mean by area.
    merged: Vec<f32>,
    /// Closed, the colour of a lit leaf in it (the first), if any.
    lit: Vec<Option<u32>>,
}

/// Each box's level: open or not, and when it last changed (ms; none: settled long ago).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub open: Vec<bool>,
    pub since: Vec<Option<u64>>,
}

impl Tree {
    /// The ladder of `blocks` laid out from `atlas`: a box stands in the nearest plinth whose
    /// folder holds it (a folder of one child has none: its child stands in the next one out).
    pub fn new(atlas: &Atlas, blocks: &[Block]) -> Tree {
        let id = |b: &Block| b.place.and_then(|p| atlas.places.get(p)).map(|p| p.id.as_str());
        let plates: std::collections::HashMap<&str, usize> = (blocks.iter().enumerate())
            .filter(|(_, b)| matches!(b.kind, Kind::Plate(_)))
            .filter_map(|(i, b)| Some((id(b)?, i)))
            .collect();
        let parent: Vec<Option<usize>> = (blocks.iter())
            .map(|b| {
                let mut path = id(b)?;
                while let Some((up, _)) = path.rsplit_once('/') {
                    if let Some(&p) = plates.get(up) {
                        return Some(p);
                    }
                    path = up;
                }
                None
            })
            .collect();
        let n = blocks.len();
        let area = |b: &Block| (b.max[0] - b.min[0]) * (b.max[2] - b.min[2]);
        let mut children = vec![0usize; n];
        for p in parent.iter().flatten() {
            children[*p] += 1;
        }
        let child_side = (0..n)
            .map(|i| if children[i] == 0 { 0. } else { (area(&blocks[i]) / children[i] as f32).sqrt() })
            .collect();
        let centre = blocks.iter().map(|b| [(b.min[0] + b.max[0]) / 2., (b.min[2] + b.max[2]) / 2.]).collect();
        // Each leaf counted in every box over it: its height above that box's top, by its area.
        let (mut weighed, mut weight) = (vec![0f32; n], vec![0f32; n]);
        let mut lit = vec![None; n];
        for (i, b) in blocks.iter().enumerate().filter(|(i, _)| children[*i] == 0) {
            let mut up = parent[i];
            while let Some(p) = up {
                weighed[p] += (b.max[1] - blocks[p].max[1]).max(0.) * area(b);
                weight[p] += area(b);
                if let (Kind::Lit(colour), None) = (b.kind, lit[p]) {
                    lit[p] = Some(colour);
                }
                up = parent[p];
            }
        }
        let merged = (0..n).map(|i| if weight[i] > 0. { weighed[i] / weight[i] } else { 0. }).collect();
        Tree { parent, child_side, centre, merged, lit }
    }

    /// The box `i` stands in, if any.
    pub fn parent(&self, i: usize) -> Option<usize> {
        self.parent.get(i).copied().flatten()
    }

    /// How many pixels a side box `i`'s children's lots would be, `height_px` the picture's
    /// height. By the distance from the eye to the point looked at, widened by how far the box is
    /// from that point on the plan: turning the camera round that point changes no level.
    pub fn child_px(&self, i: usize, camera: &Camera, height_px: f32) -> f32 {
        let d2: f32 = (0..3).map(|k| (camera.eye[k] - camera.target[k]).powi(2)).sum();
        let c = self.centre[i];
        let r2 = (c[0] - camera.target[0]).powi(2) + (c[1] - camera.target[2]).powi(2);
        self.child_side[i] * height_px / (2. * (camera.fov / 2.).tan() * (d2 + r2).sqrt())
    }

    /// The levels for `camera` after `was` (none: the first picture, settled at once): a box
    /// opens past `OPEN_PX`, closes under `CLOSE_PX`, and changes at most once in `HOLD_MS`.
    /// Leaves have no level. Also whether a change waits on its hold (ask again later).
    pub fn select(&self, camera: &Camera, height_px: f32, now_ms: u64, was: Option<&Detail>) -> (Detail, bool) {
        let n = self.parent.len();
        let was = was.filter(|d| d.open.len() == n);
        let mut detail = was.cloned().unwrap_or_else(|| Detail { open: vec![false; n], since: vec![None; n] });
        let mut waits = false;
        for i in (0..n).filter(|i| self.child_side[*i] > 0.) {
            let px = self.child_px(i, camera, height_px);
            let Some(was) = was else {
                detail.open[i] = px >= (OPEN_PX + CLOSE_PX) / 2.;
                continue;
            };
            let want = if was.open[i] { px >= CLOSE_PX } else { px >= OPEN_PX };
            if want == was.open[i] {
                continue;
            }
            if was.since[i].is_some_and(|at| now_ms.saturating_sub(at) < HOLD_MS) {
                waits = true;
                continue;
            }
            (detail.open[i], detail.since[i]) = (want, Some(now_ms));
        }
        (detail, waits)
    }

    /// How open box `i` is at `now_ms`: 1 open, 0 closed, between while it changes over `fade_ms`.
    pub fn openness(&self, detail: &Detail, i: usize, now_ms: u64, fade_ms: u64) -> f32 {
        if self.child_side[i] == 0. {
            return 1.;
        }
        let t = match detail.since[i] {
            Some(at) if fade_ms > 0 => (now_ms.saturating_sub(at) as f32 / fade_ms as f32).min(1.),
            _ => 1.,
        };
        if detail.open[i] { t } else { 1. - t }
    }

    /// Whether any box still changes at `now_ms` (more pictures to make): from the moment it
    /// changed (openness still 0 or 1 then) until its fade is over.
    pub fn changing(&self, detail: &Detail, now_ms: u64, fade_ms: u64) -> bool {
        detail.since.iter().flatten().any(|at| now_ms.saturating_sub(*at) < fade_ms)
    }

    /// `blocks` as `detail` shows them at `now_ms`: which are drawn, and the colour a closed box
    /// glows in for a lit leaf it hides; a closed box raised to stand for what it hides, its
    /// children grown out of it as it opens. Only heights change: every box keeps its outline on
    /// the plan, and its kind.
    pub fn apply(
        &self,
        blocks: &mut [Block],
        detail: &Detail,
        now_ms: u64,
        fade_ms: u64,
    ) -> (Vec<bool>, Vec<Option<u32>>) {
        let n = blocks.len().min(self.parent.len());
        let (mut shown, mut glow) = (vec![true; n], vec![None; n]);
        let open: Vec<f32> = (0..n).map(|i| self.openness(detail, i, now_ms, fade_ms)).collect();
        let tops: Vec<f32> = blocks.iter().map(|b| b.max[1]).collect();
        // Each box's own height first: a closed one raised to stand for what it hides.
        let own: Vec<f32> = (0..n).map(|i| tops[i] + self.merged[i] * (1. - open[i])).collect();
        // Parents come before their children (the layout's order): each sees its parent done.
        for i in 0..n {
            let mut top = own[i];
            if let Some(p) = self.parent[i] {
                shown[i] = shown[p] && open[p] > 0.;
                // Grown out of the parent's closed top as it opens: inside it until then.
                let low = top.min(tops[p] + self.merged[p]);
                top = low + (top - low) * open[p];
            }
            blocks[i].max[1] = top;
            if open[i] < 1. {
                glow[i] = self.lit[i];
            }
        }
        (shown, glow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas;
    use crate::scene::{self, View};

    fn city(files: usize) -> (Atlas, scene::Layout) {
        let files: Vec<(String, u64)> = (0..files)
            .map(|i| (format!("d{}/e{}/g{}/f{i}.rs", i % 7, i % 5, i % 3), 300 + (i as u64 * 7919) % 40_000))
            .collect();
        let plan = atlas::atlas(&atlas::repo(&files));
        let layout = scene::layout(&plan, &[], 1.6, scene::LOWER);
        (plan, layout)
    }

    fn camera(layout: &scene::Layout, view: View) -> Camera {
        layout.scene(&view).camera.unwrap()
    }

    /// Every box stands inside its parent's outline, and no level moves or resizes any box on
    /// the plan: only heights change, and a hidden box is inside its open ancestor's column.
    #[test]
    fn a_level_moves_nothing_on_the_plan() {
        let (plan, layout) = city(3000);
        let tree = Tree::new(&plan, &layout.blocks);
        for (i, b) in layout.blocks.iter().enumerate() {
            if let Some(p) = tree.parent(i) {
                let q = &layout.blocks[p];
                let inside =
                    b.min[0] >= q.min[0] && b.max[0] <= q.max[0] && b.min[2] >= q.min[2] && b.max[2] <= q.max[2];
                assert!(inside, "box {i} outside its parent {p}");
            }
        }
        for zoom in [1., 0.5, 0.2, 0.08, 0.05] {
            let view = View { zoom, ..View::default() };
            let (detail, _) = tree.select(&camera(&layout, view), 200., 0, None);
            for now in [0, 50, 100, 400] {
                let mut blocks = layout.blocks.clone();
                tree.apply(&mut blocks, &detail, now, FADE_MS);
                for (a, b) in layout.blocks.iter().zip(&blocks) {
                    assert_eq!(
                        [a.min[0], a.min[2], a.max[0], a.max[2], a.min[1]],
                        [b.min[0], b.min[2], b.max[0], b.max[2], b.min[1]]
                    );
                }
            }
        }
    }

    /// Far out, most folders stand closed; near in, they open: fewer boxes drawn far away.
    #[test]
    fn far_out_folders_close_and_near_in_they_open() {
        let (plan, layout) = city(6000);
        let tree = Tree::new(&plan, &layout.blocks);
        let drawn = |zoom: f32| {
            let view = View { zoom, ..View::default() };
            let (detail, _) = tree.select(&camera(&layout, view), 200., 0, None);
            tree.apply(&mut layout.blocks.clone(), &detail, 0, 0).0.iter().filter(|s| **s).count()
        };
        let (far, near) = (drawn(1.), drawn(0.05));
        assert!(far < layout.blocks.len() && near > far, "far {far}, near {near} of {}", layout.blocks.len());
    }

    /// A full turn round the point looked at changes no level: a spin is level-neutral.
    #[test]
    fn a_turn_changes_no_level() {
        let (plan, layout) = city(4000);
        let tree = Tree::new(&plan, &layout.blocks);
        for zoom in [1., 0.3, 0.1] {
            let mut view = View { zoom, ..View::default() };
            let (mut detail, _) = tree.select(&camera(&layout, view), 200., 0, None);
            let open = detail.open.iter().filter(|o| **o).count();
            assert!(zoom < 1. || open < tree.child_side.iter().filter(|s| **s > 0.).count(), "some closed far out");
            for step in 1..=240u64 {
                view = view.orbited(1.5, 0.);
                let (next, _) = tree.select(&camera(&layout, view), 200., step * 16 + 1000, Some(&detail));
                assert_eq!(next.open, detail.open, "zoom {zoom}, step {step}");
                detail = next;
            }
        }
    }

    /// Between the two thresholds a box keeps its level: a camera that wobbles in and out flips
    /// nothing; a box changes at most once within `HOLD_MS`.
    #[test]
    fn a_wobbling_camera_flips_nothing() {
        let (plan, layout) = city(4000);
        let tree = Tree::new(&plan, &layout.blocks);
        let mut view = View::default();
        let (mut detail, _) = tree.select(&camera(&layout, view), 200., 0, None);
        let mut changes: Vec<(usize, u64)> = Vec::new();
        // In slowly to a fifth, wobbling 2% each frame, then back out.
        let steps: Vec<f32> =
            (0..400).map(|k| if k < 200 { 0.992f32.powi(k) } else { 0.992f32.powi(400 - k) }).collect();
        for (k, z) in steps.iter().enumerate() {
            let wobble = if k % 2 == 0 { 1.02 } else { 0.98 };
            view.zoom = z * wobble;
            let now = k as u64 * 16 + 1000;
            let (next, _) = tree.select(&camera(&layout, view), 200., now, Some(&detail));
            changes.extend((0..next.open.len()).filter(|i| next.open[*i] != detail.open[*i]).map(|i| (i, now)));
            detail = next;
        }
        let mut by_box: std::collections::HashMap<usize, Vec<u64>> = Default::default();
        for (i, at) in changes {
            by_box.entry(i).or_default().push(at);
        }
        for (i, at) in by_box {
            // In once and out once at most, never back within the hold.
            assert!(at.len() <= 2, "box {i} changed {} times", at.len());
            assert!(at.windows(2).all(|w| w[1] - w[0] >= HOLD_MS), "box {i}: {at:?}");
        }
    }

    /// A box opening grows its children out of its closed top: at the start they are inside it
    /// (none above it), at the end as laid out; at once with no fade (Reduce Motion).
    #[test]
    fn children_grow_out_of_their_box() {
        let (plan, layout) = city(3000);
        let tree = Tree::new(&plan, &layout.blocks);
        let far = tree.select(&camera(&layout, View::default()), 200., 0, None).0;
        let near_view = View { zoom: 0.05, ..View::default() };
        let (near, _) = tree.select(&camera(&layout, near_view), 200., 1000, Some(&far));
        let opened: Vec<usize> = (0..near.open.len()).filter(|i| near.open[*i] && !far.open[*i]).collect();
        assert!(!opened.is_empty());
        let mut start = layout.blocks.clone();
        tree.apply(&mut start, &near, 1000, FADE_MS);
        let mut end = layout.blocks.clone();
        tree.apply(&mut end, &near, 1000 + FADE_MS, FADE_MS);
        let mut calm = layout.blocks.clone();
        tree.apply(&mut calm, &near, 1000, 0);
        // Still changing at the moment it changed (openness 0 then): more pictures are asked.
        assert!(tree.changing(&near, 1000, FADE_MS) && !tree.changing(&near, 1000 + FADE_MS, FADE_MS));
        // A few ms in, nothing pops out: each child, a closed folder in it too, barely above it.
        let mut soon = layout.blocks.clone();
        tree.apply(&mut soon, &near, 1005, FADE_MS);
        let tallest = layout.blocks.iter().map(|b| b.max[1]).fold(0., f32::max);
        for &p in &opened {
            for i in (0..start.len()).filter(|i| tree.parent(*i) == Some(p)) {
                assert!(soon[i].max[1] <= start[p].max[1] + 0.03 * tallest, "child {i} popped out of {p}");
                assert!(start[i].max[1] <= start[p].max[1] + 1e-6, "child {i} above its closed box {p}");
                assert_eq!(end[i].max[1], layout.blocks[i].max[1]);
                assert_eq!(calm[i].max[1], layout.blocks[i].max[1]);
            }
        }
    }

    /// A closed box hiding a lit file is lit in its colour.
    #[test]
    fn a_closed_box_hiding_a_lit_file_is_lit() {
        let files: Vec<(String, u64)> = (0..400).map(|i| (format!("a/b{}/f{i}.rs", i % 4), 2000)).collect();
        let plan = atlas::atlas(&atlas::repo(&files));
        let layout = scene::layout(&plan, &[("a/b1/f5.rs", 0xff6467)], 1.6, scene::LOWER);
        let tree = Tree::new(&plan, &layout.blocks);
        let closed = Detail { open: vec![false; layout.blocks.len()], since: vec![None; layout.blocks.len()] };
        let mut blocks = layout.blocks.clone();
        let (shown, glow) = tree.apply(&mut blocks, &closed, 0, 0);
        let lit = (0..blocks.len()).filter(|i| shown[*i] && glow[*i] == Some(0xff6467));
        assert!(lit.count() >= 1);
        // Still a folder: its label and hover are a folder's.
        assert!((0..blocks.len()).all(|i| blocks[i].kind == layout.blocks[i].kind));
    }
}
