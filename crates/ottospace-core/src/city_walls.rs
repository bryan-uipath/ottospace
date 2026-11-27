//! Where the code city names its folders and files in the picture (flag `city_wall_names`), as the
//! owner's web city settled it (`docs/ai/code-city/wall-names.md`). The top two folder tiers on the
//! side of their plinth that faces the camera: a name's em a share of its wall's height, grown
//! toward a size on screen while far (the top tier's more), shrunk to fit its wall. Deeper folders
//! and files on billboards over their box's top, facing the screen at one size a tier on a pill,
//! picked as GPUI's names were (`city_labels::place`): largest first, none over another, at most 50.
//! Sizes fall by tier: a file's name is never larger than a folder's. The renderer draws each where
//! this puts it: a wall's depth tested, a billboard on top, dimmed where its box is hidden.

use crate::city_labels::{self, Candidate, Tier};
use crate::scene::{Camera, Kind, Scene, cross, dot, sub, unit};
use crate::theme;

/// Folders of this tier and outer are named on their wall (2: a folder of the root and those on it).
pub const WALL_TIERS: usize = 2;
/// A wall's name's em is this share of its height, and keeps this many ems clear at each end.
const FILL: f32 = 0.8;
const END: f32 = 0.5;
/// It stands this share of an em off its wall: in front of it, for the depth test.
const OFF: f32 = 0.01;
/// Drawn while its width over its height on screen is at least this share of the name's own: a
/// wall seen nearly edge on squeezes it past reading.
const SQUEEZE: f32 = 0.5;
/// The font's capitals are this share of its em (SF Compact's, about).
const CAP: f32 = 0.7;
/// Far out, a top tier's capitals grow toward this many points tall, to at most this many times
/// its wall's own size; the second tier's toward less, less far (the web city's 15 px, 2.2×; 11, 1.6×).
const TOP_PT: f32 = 15.;
const TOP_GROW: f32 = 2.2;
const SUB_PT: f32 = 11.;
const SUB_GROW: f32 = 1.6;
/// A wall's name smaller than this em on screen (points) is not drawn.
pub const LEAST_PT: f32 = theme::TEXT_XS;
/// A billboard's em, points: a deeper folder's, a file's (GPUI's sizes for them).
pub const FOLDER_PT: f32 = theme::TEXT_XS;
pub const FILE_PT: f32 = theme::TEXT_2XS;
/// A billboard's pill stands this many points over its box's top.
const LIFT_PT: f32 = 2.;
/// Its pill reaches this many ems past its name's ends, and past its em box above and below (the
/// web city's 44 px text on a 68 px pill); `city_labels` keeps pills apart.
pub const PILL_END: f32 = 0.5;
pub const PILL_RISE: f32 = 0.275;
/// Names are measured for this many boxes at most, the largest: a few placed of each.
const SHAPED: usize = 4 * city_labels::MOST;

const UP: [f32; 3] = [0., 1., 0.];

/// A name in the picture: on a plinth's wall or a billboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sign {
    /// The box's block and the atlas place it names.
    pub block: usize,
    pub place: usize,
    /// Its em box's bottom left corner in the world; the way it reads (a unit vector, left to
    /// right as seen); the way its letters stand; its face's normal (a billboard's: to the camera).
    pub origin: [f32; 3],
    pub across: [f32; 3],
    pub up: [f32; 3],
    pub normal: [f32; 3],
    /// Its em and its width, in plan units.
    pub em: f32,
    pub width: f32,
    /// Its box on screen (a wall's em box, a billboard's pill): shares of the picture (x, y, w, h).
    pub rect: [f32; 4],
    /// The point it names (a wall's name's middle, a billboard's box's top): a box in front of it
    /// hides the name (`hidden`).
    pub anchor: [f32; 3],
}

fn at(o: [f32; 3], d: [f32; 3], by: f32) -> [f32; 3] {
    std::array::from_fn(|i| o[i] + d[i] * by)
}

/// The em box on screen of the box from `origin` along `across` and `up`, as shares; none if a
/// corner is behind the camera or it is wholly off the picture.
fn rect_of(camera: &Camera, m: &[[f32; 4]; 4], origin: [f32; 3], across: [f32; 3], up: [f32; 3]) -> Option<[f32; 4]> {
    let corners = [(0., 0.), (1., 0.), (0., 1.), (1., 1.)].map(|(a, u)| at(at(origin, across, a), up, u));
    let points = corners.into_iter().map(|p| camera.project_by(m, p)).collect::<Option<Vec<_>>>()?;
    let (x0, y0) = points.iter().fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
    let (x1, y1) = points.iter().fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.0), y.max(p.1)));
    (x1 > 0. && y1 > 0. && x0 < 1. && y0 < 1.).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// The names `scene`'s shown plinths of the top tiers carry on their walls. `wide`: a place's
/// name's width in ems (none: nothing to draw), asked only of a plinth with room on screen for a
/// name an em wide; `view`: the picture's size in pixels; `scale`: pixels a point.
pub fn signs(scene: &Scene, mut wide: impl FnMut(usize) -> Option<f32>, view: [f32; 2], scale: f32) -> Vec<Sign> {
    let Some(camera) = scene.camera else { return Vec::new() };
    let m = camera.matrix();
    let least = LEAST_PT * scale;
    let seen = |p: [f32; 3]| camera.project_by(&m, p).map(|(x, y)| [x * view[0], y * view[1]]);
    let apart = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let mut signs = Vec::new();
    for (block, b) in scene.blocks.iter().enumerate() {
        let Kind::Plate(tier) = b.kind else { continue };
        let Some(place) = b.place.filter(|_| tier <= WALL_TIERS && scene.is_shown(block)) else { continue };
        let (target, grow) = if tier <= 1 { (TOP_PT, TOP_GROW) } else { (SUB_PT, SUB_GROW) };
        let target = target / CAP * scale;
        // Each side: its outward normal, its middle, its length. The one facing the camera most
        // that holds the name (a long thin plinth's short end may not: its long side then).
        let mid = |i: usize| (b.min[i] + b.max[i]) / 2.;
        let (y, long, deep) = (mid(1), b.max[0] - b.min[0], b.max[2] - b.min[2]);
        let mut sides = [
            ([0., 0., -1.], [mid(0), y, b.min[2]], long),
            ([0., 0., 1.], [mid(0), y, b.max[2]], long),
            ([-1., 0., 0.], [b.min[0], y, mid(2)], deep),
            ([1., 0., 0.], [b.max[0], y, mid(2)], deep),
        ];
        let facing = |(n, c, _): &([f32; 3], [f32; 3], f32)| dot(*n, unit(sub(camera.eye, *c)));
        sides.sort_by(|a, b| facing(b).total_cmp(&facing(a)));
        let height = b.max[1] - b.min[1];
        let base = height * FILL;
        // Its em on screen per plan unit of em there.
        let per = |middle: [f32; 3]| Some(apart(seen(at(middle, UP, -0.5))?, seen(at(middle, UP, 0.5))?));
        let facing_sides = || sides.iter().filter(|side| facing(side) > 0.);
        // None with room on screen for a name an em wide, grown its most: its name is not measured.
        let room = |&(_, middle, length): &([f32; 3], [f32; 3], f32)| {
            let em = (base * grow).min(length / (1. + 2. * END));
            per(middle).is_some_and(|p| p * em >= least)
        };
        if !facing_sides().any(room) {
            continue;
        }
        let Some(ratio) = wide(place).filter(|r| *r > 0.) else { continue };
        let sign = |&(normal, middle, length): &([f32; 3], [f32; 3], f32)| -> Option<Sign> {
            // As large as the wall's height allows, grown toward its tier's size on screen while
            // that is less, then smaller to fit its length.
            let p = per(middle)?;
            let em = if p * base < target { (target / p).min(base * grow) } else { base };
            let em = em.min(length / (ratio + 2. * END));
            let width = em * ratio;
            let across = cross(UP, normal);
            // Grown past its wall, it stands on the wall's foot: never sunk into the plate below.
            let rise = if em > height { b.min[1] + em / 2. - y } else { 0. };
            let middle = at(at(middle, UP, rise), normal, OFF * em);
            let (l, r) = (seen(at(middle, across, -width / 2.))?, seen(at(middle, across, width / 2.))?);
            let (lo, hi) = (seen(at(middle, UP, -em / 2.))?, seen(at(middle, UP, em / 2.))?);
            let tall = apart(lo, hi);
            // Its width across its up on screen: a wall seen edge on runs its name along its up.
            let (d, u) = ([r[0] - l[0], r[1] - l[1]], [(hi[0] - lo[0]) / tall, (hi[1] - lo[1]) / tall]);
            let wide_on_screen = (d[0] * u[1] - d[1] * u[0]).abs();
            if tall < least || wide_on_screen < SQUEEZE * ratio * tall {
                return None;
            }
            let origin = at(at(middle, across, -width / 2.), UP, -em / 2.);
            let rect = rect_of(&camera, &m, origin, across.map(|v| v * width), UP.map(|v| v * em))?;
            Some(Sign { block, place, origin, across, up: UP, normal, em, width, rect, anchor: middle })
        };
        signs.extend(facing_sides().find_map(sign));
    }
    signs
}

/// The sign of `block`'s billboard over its top, facing the screen, its em `em_px` on screen,
/// `ratio` ems wide, its pill's bottom `lift_px` over the top; none behind the camera or off the
/// picture.
#[allow(clippy::too_many_arguments)] // A billboard's box, size and the picture's.
fn billboard(
    camera: &Camera,
    m: &[[f32; 4]; 4],
    (block, place): (usize, usize),
    top: [f32; 3],
    ratio: f32,
    em_px: f32,
    lift_px: f32,
    view: [f32; 2],
) -> Option<Sign> {
    let forward = unit(sub(camera.target, camera.eye));
    let across = unit(cross(forward, UP));
    let up = cross(across, forward);
    let depth = dot(sub(top, camera.eye), forward);
    if depth <= camera.near {
        return None;
    }
    // Plan units a pixel at its depth.
    let unit_px = 2. * depth * (camera.fov / 2.).tan() / view[1];
    let (em, width) = (em_px * unit_px, em_px * unit_px * ratio);
    let origin = at(at(top, up, lift_px * unit_px + PILL_RISE * em), across, -width / 2.);
    let pill = at(at(origin, across, -PILL_END * em), up, -PILL_RISE * em);
    let (wide, tall) = (width + 2. * PILL_END * em, (1. + 2. * PILL_RISE) * em);
    let rect = rect_of(camera, m, pill, across.map(|v| v * wide), up.map(|v| v * tall))?;
    Some(Sign { block, place, origin, across, up, normal: forward.map(|v| -v), em, width, rect, anchor: top })
}

/// The deeper folders' and files' names: billboards over their boxes' tops, at `FOLDER_PT` and
/// `FILE_PT`. Picked by `city_labels::place` (`kept`: the places named last); `moving`: none
/// picked again, the kept ones only moved (a drag re-picks when it ends). `wide`, `view` and
/// `scale` as `signs`.
pub fn billboards(
    scene: &Scene,
    mut wide: impl FnMut(usize) -> Option<f32>,
    view: [f32; 2],
    scale: f32,
    kept: &[usize],
    moving: bool,
) -> Vec<Sign> {
    let Some(camera) = scene.camera else { return Vec::new() };
    let m = camera.matrix();
    let points = [view[0] / scale, view[1] / scale];
    let tier = |kind: Kind| match kind {
        Kind::Plate(t) if t > WALL_TIERS => Some(Tier::Folder),
        Kind::Building | Kind::Lit(_) => Some(Tier::File),
        _ => None,
    };
    let em_pt = |tier: Tier| if tier == Tier::File { FILE_PT } else { FOLDER_PT };
    // Each box that may be named, its tier and its bounds in points.
    let named = |i: usize| -> Option<(usize, Tier, [f32; 4])> {
        let b = &scene.blocks[i];
        let (place, tier) = (b.place?, tier(b.kind)?);
        let [x, y, w, h] = camera.bounds(b)?;
        Some((place, tier, [x * points[0], y * points[1], w * points[0], h * points[1]]))
    };
    let mut picked: Vec<(usize, usize, Tier, f32)> = Vec::new();
    if moving {
        for (i, b) in scene.blocks.iter().enumerate() {
            let Some(place) = b.place.filter(|p| kept.contains(p) && scene.is_shown(i)) else { continue };
            let (Some(tier), Some(ratio)) = (tier(b.kind), wide(place)) else { continue };
            picked.push((i, place, tier, ratio));
        }
    } else {
        let mut boxes: Vec<(usize, usize, Tier, [f32; 4])> = (0..scene.blocks.len())
            .filter(|i| scene.is_shown(*i))
            .filter_map(|i| named(i).map(|(place, tier, bounds)| (i, place, tier, bounds)))
            .filter(|(.., bounds)| city_labels::sized(*bounds, points))
            .collect();
        let extent = |b: &[f32; 4]| b[2].max(b[3]);
        boxes.sort_by(|a, b| extent(&b.3).total_cmp(&extent(&a.3)).then(a.1.cmp(&b.1)));
        boxes.truncate(SHAPED);
        let mut ratios = Vec::new();
        let candidates: Vec<Candidate> = (boxes.iter())
            .filter_map(|&(i, place, tier, bounds)| {
                let ratio = wide(place).filter(|r| *r > 0.)?;
                let (x, y) = camera.project_by(&m, scene.blocks[i].top())?;
                let face = scene.outline(i)?.map(|(x, y)| [x * points[0], y * points[1]]);
                let em = em_pt(tier);
                // Its pill, centred where it stands: over the top, not on it.
                let size = [(ratio + 2. * PILL_END) * em, (1. + 2. * PILL_RISE) * em];
                let top = [x * points[0], y * points[1] - LIFT_PT - size[1] / 2.];
                ratios.push((place, i, ratio));
                Some(Candidate { key: place, tier, bounds, top, face, size })
            })
            .collect();
        for (place, _) in city_labels::place(&candidates, points, &[], kept) {
            if let Some(&(_, i, ratio)) = ratios.iter().find(|(p, ..)| *p == place) {
                picked.push((i, place, tier(scene.blocks[i].kind).unwrap_or(Tier::File), ratio));
            }
        }
    }
    (picked.into_iter())
        .filter_map(|(block, place, tier, ratio)| {
            let top = scene.blocks[block].top();
            billboard(&camera, &m, (block, place), top, ratio, em_pt(tier) * scale, LIFT_PT * scale, view)
        })
        .collect()
}

/// Whether a shown box, standing (not a frame of its rise), is between the camera and `sign`'s
/// anchor. A wall's name is depth tested: a solid box hides it, a glass one dims it. A billboard is
/// drawn on top: hidden, it is dimmed.
pub fn hidden(scene: &Scene, sign: &Sign) -> bool {
    let Some(camera) = scene.camera else { return false };
    (scene.blocks.iter().enumerate())
        .filter(|(i, b)| *i != sign.block && b.kind != Kind::Ground && scene.is_shown(*i))
        .any(|(_, b)| b.crossed(camera.eye, sign.anchor).is_some_and(|(t0, _)| t0 < 1. - 1e-4))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Block, Fit, View};

    const VIEW: [f32; 2] = [1600., 1000.];

    fn plinth(tier: usize, min: [f32; 3], max: [f32; 3]) -> Block {
        Block { min, max, kind: Kind::Plate(tier), place: Some(tier), district: Some(0) }
    }

    fn building(place: usize, min: [f32; 3], max: [f32; 3]) -> Block {
        Block { min, max, kind: Kind::Building, place: Some(place), district: Some(0) }
    }

    /// `blocks` seen from `view` about the middle of the plan.
    fn seen(blocks: Vec<Block>, view: View) -> Scene {
        let fit = Fit { distance: 3., target: [0.8, 0., 0.5], lower: 0. };
        Scene { blocks, camera: Some(fit.camera(&view, 1.6)), ..Scene::default() }
    }

    fn one() -> Block {
        plinth(1, [0.4, 0., 0.3], [1.2, 0.06, 0.7])
    }

    /// A sign's em on screen, pixels.
    fn tall(s: &Scene, sign: &Sign) -> f32 {
        let camera = s.camera.unwrap();
        let mid = at(at(sign.origin, sign.across, sign.width / 2.), sign.up, sign.em / 2.);
        let (a, b) = (
            camera.project(at(mid, sign.up, -sign.em / 2.)).unwrap(),
            camera.project(at(mid, sign.up, sign.em / 2.)).unwrap(),
        );
        (((a.0 - b.0) * VIEW[0]).powi(2) + ((a.1 - b.1) * VIEW[1]).powi(2)).sqrt()
    }

    #[test]
    fn a_plinth_is_named_on_the_wall_that_faces_the_camera_reading_left_to_right() {
        for (yaw, normal) in [(34., [0., 0., -1.]), (214., [0., 0., 1.]), (90., [-1., 0., 0.]), (300., [1., 0., 0.])] {
            let s = seen(vec![one()], View { yaw, ..View::default() });
            let signs = signs(&s, |_| Some(5.), VIEW, 1.);
            assert_eq!(signs.len(), 1, "{yaw}");
            let sign = signs[0];
            assert_eq!(sign.normal, normal, "{yaw}");
            // At least 0.8 of the wall's 0.06, at most 2.2 times that.
            assert!((0.048 - 1e-6..=0.048 * TOP_GROW + 1e-6).contains(&sign.em), "{yaw}: {sign:?}");
            assert!((sign.width - 5. * sign.em).abs() < 1e-6, "{yaw}: {sign:?}");
            let camera = s.camera.unwrap();
            let (l, r) =
                (camera.project(sign.origin).unwrap(), camera.project(at(sign.origin, sign.across, 1.)).unwrap());
            assert!(l.0 < r.0, "{yaw}: reads left to right on screen");
            let wall = if normal[2] < 0. {
                0.3
            } else if normal[2] > 0. {
                0.7
            } else if normal[0] < 0. {
                0.4
            } else {
                1.2
            };
            let axis = if normal[2] != 0. { 2 } else { 0 };
            assert!((sign.origin[axis] - wall) * normal[axis] > 0., "{yaw}: in front of its wall: {sign:?}");
        }
    }

    /// Far out the top tier's name grows toward its size on screen, at most 2.2 times its wall's,
    /// standing on the wall's foot; near, it is its wall's own. The second tier grows less.
    #[test]
    fn a_top_tiers_name_holds_its_size_far_out_and_shrinks_to_its_wall_near() {
        let far = seen(vec![one()], View { zoom: 2.5, ..View::default() });
        let sign = signs(&far, |_| Some(5.), VIEW, 1.)[0];
        assert!(sign.em > 0.048 * 1.05 && sign.em <= 0.048 * TOP_GROW + 1e-6, "{sign:?}");
        let px = tall(&far, &sign);
        assert!(px <= TOP_PT / CAP + 0.5, "{px}");
        if sign.em > 0.06 {
            assert!((sign.origin[1] - 0.).abs() < 1e-4, "on its foot: {sign:?}");
        }
        let near = seen(vec![one()], View { zoom: 0.3, ..View::default() });
        assert!((signs(&near, |_| Some(5.), VIEW, 1.)[0].em - 0.048).abs() < 1e-6);
        // The same plinth of the second tier: smaller far out.
        let sub = seen(vec![plinth(2, one().min, one().max)], View { zoom: 2.5, ..View::default() });
        let second = signs(&sub, |_| Some(5.), VIEW, 1.)[0];
        assert!(second.em < sign.em, "{second:?} {sign:?}");
        assert!(second.em <= 0.048 * SUB_GROW + 1e-6);
    }

    #[test]
    fn a_long_name_shrinks_to_fit_its_wall_and_too_small_is_not_drawn() {
        let s = seen(vec![one()], View::default());
        // Twenty ems and an end each side: the wall's 0.8 long, at most.
        let fit = signs(&s, |_| Some(20.), VIEW, 1.)[0];
        assert!(fit.em <= 0.8 / 21. + 1e-6 && fit.width + fit.em < 0.8 + 1e-6, "{fit:?}");
        // So long it would be smaller than the least: none.
        assert_eq!(signs(&s, |_| Some(100.), VIEW, 1.), []);
        // A far smaller picture, or a larger scale: none either.
        assert_eq!(signs(&s, |_| Some(5.), [160., 100.], 1.), []);
        assert!(signs(&s, |_| Some(5.), VIEW, 30.).is_empty());
    }

    /// Only the top two tiers are named on walls; deeper ones get billboards.
    #[test]
    fn only_the_top_two_tiers_shown_and_with_a_name_are_on_walls() {
        let deeper = plinth(3, [0.5, 0.06, 0.35], [1.1, 0.12, 0.65]);
        let s = seen(vec![one(), deeper], View::default());
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 1.).iter().map(|s| s.block).collect::<Vec<_>>(), [0]);
        assert_eq!(signs(&s, |_| None, VIEW, 1.), []);
        let hidden = Scene { shown: vec![false, true], ..s };
        assert_eq!(signs(&hidden, |_| Some(5.), VIEW, 1.), []);
    }

    /// Looking near straight down, or at a wall edge on: its name is not drawn.
    #[test]
    fn a_wall_seen_from_above_or_edge_on_is_not_named() {
        let s = seen(vec![one()], View { pitch: 80., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 1.), []);
        let thin = plinth(1, [0., 0., 0.48], [1.6, 0.06, 0.52]);
        let s = seen(vec![thin], View { yaw: 90., pitch: 12., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 1.), []);
        let s = seen(vec![thin], View { yaw: 60., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 1.).iter().map(|s| s.normal).collect::<Vec<_>>(), [[0., 0., -1.]]);
        let s = seen(vec![thin], View { yaw: 89., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 1.), []);
    }

    #[test]
    fn a_plinth_too_small_to_name_is_not_measured() {
        let small = plinth(1, [0.79, 0., 0.49], [0.8, 0.001, 0.5]);
        let mut asked = 0;
        let s = seen(vec![small], View::default());
        assert_eq!(signs(&s, |_| (asked += 1, Some(4.)).1, VIEW, 1.), []);
        assert_eq!(asked, 0);
    }

    #[test]
    fn a_tower_in_front_hides_a_name_and_none_does_not() {
        let s = seen(vec![one()], View::default());
        let sign = signs(&s, |_| Some(5.), VIEW, 1.)[0];
        assert!(!hidden(&s, &sign));
        let camera = s.camera.unwrap();
        let middle = at(at(sign.origin, sign.across, sign.width / 2.), UP, sign.em / 2.);
        let on = at(middle, unit(sub(camera.eye, middle)), 0.3);
        let tower = building(9, [on[0] - 0.02, 0., on[2] - 0.02], [on[0] + 0.02, 2., on[2] + 0.02]);
        let behind = seen(vec![one(), tower], View::default());
        assert!(hidden(&behind, &sign));
        assert!(!hidden(&Scene { shown: vec![true, false], ..behind }, &sign));
    }

    /// Sizes fall by tier: the top tier's wall, the second's, a deeper folder's billboard, a file's.
    #[test]
    fn sizes_strictly_decrease_by_tier() {
        let sizes = [TOP_PT / CAP, SUB_PT / CAP, FOLDER_PT, FILE_PT];
        assert!(sizes.windows(2).all(|w| w[0] > w[1]), "{sizes:?}");
        assert!(SUB_PT / CAP > LEAST_PT && TOP_GROW > SUB_GROW);
    }

    /// A billboard holds its tier's size on screen at any distance, standing over its box's top,
    /// facing the screen; a file's is never larger than a folder's.
    #[test]
    fn billboards_hold_their_tiers_size_and_a_files_is_smaller_than_its_folders() {
        let folder = plinth(3, [0.55, 0.06, 0.35], [1.05, 0.1, 0.65]);
        let file = building(10, [0.7, 0.06, 0.45], [0.9, 0.3, 0.55]);
        for scale in [1., 2.] {
            let mut sizes = Vec::new();
            for (b, pt) in [(folder, FOLDER_PT), (file, FILE_PT)] {
                for zoom in [0.9, 1.] {
                    let s = seen(vec![b], View { zoom, ..View::default() });
                    let board = billboards(&s, |_| Some(3.), VIEW, scale, &[], false)[0];
                    let px = tall(&s, &board);
                    assert!((px - pt * scale).abs() < 0.5, "{scale} {zoom}: {px}");
                    let eye = s.camera.unwrap().eye;
                    assert!(board.origin[1] > b.max[1] && dot(board.normal, unit(sub(eye, board.origin))) > 0.9);
                    assert_eq!(board.anchor, b.top());
                    sizes.push(px);
                }
            }
            assert!(sizes[2..].iter().all(|f| sizes[..2].iter().all(|d| f < d)), "{sizes:?}");
        }
    }

    /// Many files: the billboards picked never overlap on screen, at most `city_labels::MOST`;
    /// while the camera moves none is picked again, the kept only moved.
    #[test]
    fn billboards_picked_never_overlap_and_a_move_keeps_them() {
        let mut blocks = vec![plinth(1, [0., 0., 0.], [1.6, 0.06, 1.])];
        for i in 0..400 {
            let (x, z) = ((i % 20) as f32 * 0.08, (i / 20) as f32 * 0.05);
            let h = 0.05 + (i * 37 % 11) as f32 * 0.02;
            blocks.push(building(100 + i, [x, 0.06, z], [x + 0.06, 0.06 + h, z + 0.04]));
        }
        let s = seen(blocks.clone(), View { zoom: 0.5, ..View::default() });
        let boards = billboards(&s, |p| Some(1. + (p % 7) as f32 * 0.4), VIEW, 2., &[], false);
        assert!(boards.len() > 5 && boards.len() <= city_labels::MOST, "{}", boards.len());
        let px = |r: [f32; 4]| [r[0] * VIEW[0], r[1] * VIEW[1], r[2] * VIEW[0], r[3] * VIEW[1]];
        for (i, a) in boards.iter().enumerate() {
            for b in &boards[i + 1..] {
                let (a, b) = (px(a.rect), px(b.rect));
                let over = a[0] < b[0] + b[2] - 0.5
                    && b[0] < a[0] + a[2] - 0.5
                    && a[1] < b[1] + b[3] - 0.5
                    && b[1] < a[1] + a[3] - 0.5;
                assert!(!over, "{a:?} {b:?}");
            }
        }
        let kept: Vec<usize> = boards.iter().map(|b| b.place).collect();
        let turned = seen(blocks, View { zoom: 0.5, yaw: 40., ..View::default() });
        let moved = billboards(&turned, |p| Some(1. + (p % 7) as f32 * 0.4), VIEW, 2., &kept, true);
        assert!(moved.iter().all(|b| kept.contains(&b.place)) && !moved.is_empty());
    }
}
