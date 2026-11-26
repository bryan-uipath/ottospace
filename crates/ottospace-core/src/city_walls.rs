//! Where the code city's top two folder tiers are named on their plinths (flag `city_wall_names`):
//! on the side of each plinth that faces the camera, as the owner's web city signs its terraces
//! (`docs/ai/code-city/original.md`, "Labels"). A name's em is a share of its wall's height, less
//! where the name must shrink to fit the wall; one smaller on screen than the least size, or seen
//! squeezed (its wall edge on), is not drawn. The renderer draws each where this puts it, depth
//! tested: a building in front hides it. Design: `docs/ai/code-city/wall-names.md`.

use crate::scene::{Kind, Scene, cross, dot, sub, unit};

/// Folders of this tier and outer are named on a wall (2: a folder of the root and those on it).
pub const WALL_TIERS: usize = 2;
/// A name's em is this share of its wall's height, and keeps this many ems clear at each end.
const FILL: f32 = 0.8;
const END: f32 = 0.5;
/// It stands this share of an em off its wall: in front of it, for the depth test.
const OFF: f32 = 0.01;
/// Drawn while its width over its height on screen is at least this share of the name's own: a
/// wall seen nearly edge on squeezes it past reading.
const SQUEEZE: f32 = 0.5;

const UP: [f32; 3] = [0., 1., 0.];

/// A name on a plinth's wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sign {
    /// The plinth's block and the atlas place it names.
    pub block: usize,
    pub place: usize,
    /// Its em box's bottom left corner in the world, just off the wall; the way it reads (a unit
    /// vector, left to right as seen); the wall's outward normal.
    pub origin: [f32; 3],
    pub across: [f32; 3],
    pub normal: [f32; 3],
    /// Its em and its width, in plan units.
    pub em: f32,
    pub width: f32,
    /// Its em box on screen: shares of the picture (x, y, w, h).
    pub rect: [f32; 4],
}

fn at(o: [f32; 3], d: [f32; 3], by: f32) -> [f32; 3] {
    std::array::from_fn(|i| o[i] + d[i] * by)
}

/// The names `scene`'s shown plinths of the top tiers carry. `wide`: a place's name's width in ems
/// (none: nothing to draw); `view`: the picture's size, in the unit of `least`, the least em drawn.
pub fn signs(scene: &Scene, wide: impl Fn(usize) -> Option<f32>, view: [f32; 2], least: f32) -> Vec<Sign> {
    let Some(camera) = scene.camera else { return Vec::new() };
    let mut signs = Vec::new();
    for (block, b) in scene.blocks.iter().enumerate() {
        let (Kind::Plate(tier), Some(place)) = (b.kind, b.place) else { continue };
        if tier > WALL_TIERS || !scene.is_shown(block) {
            continue;
        }
        let Some(ratio) = wide(place).filter(|r| *r > 0.) else { continue };
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
        let sign = |&(normal, middle, length): &([f32; 3], [f32; 3], f32)| -> Option<Sign> {
            // As large as the wall's height allows, smaller to fit its length.
            let em = (height * FILL).min(length / (ratio + 2. * END));
            let width = em * ratio;
            let across = cross(normal.map(|v| -v), UP);
            let middle = at(middle, normal, OFF * em);
            let seen = |p: [f32; 3]| camera.project(p).map(|(x, y)| [x * view[0], y * view[1]]);
            let (l, r) = (seen(at(middle, across, -width / 2.))?, seen(at(middle, across, width / 2.))?);
            let (lo, hi) = (seen(at(middle, UP, -em / 2.))?, seen(at(middle, UP, em / 2.))?);
            let apart = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            let tall = apart(lo, hi);
            // Its width across its up on screen: a wall seen edge on runs its name along its up.
            let (d, u) = ([r[0] - l[0], r[1] - l[1]], [(hi[0] - lo[0]) / tall, (hi[1] - lo[1]) / tall]);
            let wide_on_screen = (d[0] * u[1] - d[1] * u[0]).abs();
            if tall < least || wide_on_screen < SQUEEZE * ratio * tall {
                return None;
            }
            let origin = at(at(middle, across, -width / 2.), UP, -em / 2.);
            let corners = [(0., 0.), (width, 0.), (0., em), (width, em)].map(|(a, u)| at(at(origin, across, a), UP, u));
            let points = corners.into_iter().map(|p| camera.project(p)).collect::<Option<Vec<_>>>()?;
            let (x0, y0) = points.iter().fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
            let (x1, y1) = points.iter().fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.0), y.max(p.1)));
            let rect = [x0, y0, x1 - x0, y1 - y0];
            let on = x1 > 0. && y1 > 0. && x0 < 1. && y0 < 1.;
            on.then_some(Sign { block, place, origin, across, normal, em, width, rect })
        };
        signs.extend(sides.iter().filter(|side| facing(side) > 0.).find_map(sign));
    }
    signs
}

/// Whether a shown box, standing (not a frame of its rise), is between the camera and the middle
/// of `sign`: a solid one hides it there (the depth test), a glass one dims it.
pub fn hidden(scene: &Scene, sign: &Sign) -> bool {
    let Some(camera) = scene.camera else { return false };
    let middle = at(at(sign.origin, sign.across, sign.width / 2.), UP, sign.em / 2.);
    (scene.blocks.iter().enumerate())
        .filter(|(i, b)| *i != sign.block && b.kind != Kind::Ground && scene.is_shown(*i))
        .any(|(_, b)| b.crossed(camera.eye, middle).is_some_and(|(t0, _)| t0 < 1. - 1e-4))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Block, Fit, View};

    const VIEW: [f32; 2] = [1600., 1000.];

    fn plinth(tier: usize, min: [f32; 3], max: [f32; 3]) -> Block {
        Block { min, max, kind: Kind::Plate(tier), place: Some(tier), district: Some(0) }
    }

    /// `blocks` seen from `view` about the middle of the plan.
    fn seen(blocks: Vec<Block>, view: View) -> Scene {
        let fit = Fit { distance: 3., target: [0.8, 0., 0.5], lower: 0. };
        Scene { blocks, camera: Some(fit.camera(&view, 1.6)), ..Scene::default() }
    }

    fn one() -> Block {
        plinth(1, [0.4, 0., 0.3], [1.2, 0.06, 0.7])
    }

    #[test]
    fn a_plinth_is_named_on_the_wall_that_faces_the_camera_reading_left_to_right() {
        for (yaw, normal) in [(34., [0., 0., -1.]), (214., [0., 0., 1.]), (90., [-1., 0., 0.]), (300., [1., 0., 0.])] {
            let s = seen(vec![one()], View { yaw, ..View::default() });
            let signs = signs(&s, |_| Some(5.), VIEW, 12.);
            assert_eq!(signs.len(), 1, "{yaw}");
            let sign = signs[0];
            assert_eq!(sign.normal, normal, "{yaw}");
            // Its full height: 0.8 of the wall's 0.06.
            assert!((sign.em - 0.048).abs() < 1e-6 && (sign.width - 0.24).abs() < 1e-6, "{yaw}: {sign:?}");
            let camera = s.camera.unwrap();
            let (l, r) =
                (camera.project(sign.origin).unwrap(), camera.project(at(sign.origin, sign.across, 1.)).unwrap());
            assert!(l.0 < r.0, "{yaw}: reads left to right on screen");
            // Just off its wall, in front of it.
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
            assert!((sign.origin[axis] - wall) * normal[axis] > 0., "{yaw}: {sign:?}");
        }
    }

    #[test]
    fn a_long_name_shrinks_to_fit_its_wall_and_too_small_is_not_drawn() {
        let s = seen(vec![one()], View::default());
        // Twenty ems and an end each side: the wall's 0.8 long.
        let fit = signs(&s, |_| Some(20.), VIEW, 12.)[0];
        assert!((fit.em - 0.8 / 21.).abs() < 1e-6 && fit.width + fit.em < 0.8 + 1e-6, "{fit:?}");
        // So long it would be smaller than the least: none.
        assert_eq!(signs(&s, |_| Some(100.), VIEW, 12.), []);
        // A smaller picture, or a larger least: none either.
        assert_eq!(signs(&s, |_| Some(5.), [320., 200.], 12.), []);
        assert!(signs(&s, |_| Some(5.), VIEW, 400.).is_empty());
        // Zoomed in, a name too small at the fitted view is drawn.
        let near = seen(vec![one()], View { zoom: 0.3, ..View::default() });
        assert_eq!(signs(&near, |_| Some(5.), [320., 200.], 12.).len(), 1);
    }

    #[test]
    fn only_the_top_two_tiers_shown_and_with_a_name() {
        let deeper = plinth(3, [0.5, 0.06, 0.35], [1.1, 0.1, 0.65]);
        let s = seen(vec![one(), deeper], View::default());
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 12.).iter().map(|s| s.block).collect::<Vec<_>>(), [0]);
        assert_eq!(signs(&s, |_| None, VIEW, 12.), []);
        let hidden = Scene { shown: vec![false, true], ..s };
        assert_eq!(signs(&hidden, |_| Some(5.), VIEW, 12.), []);
    }

    /// Looking near straight down, or at a wall edge on: its name is not drawn.
    #[test]
    fn a_wall_seen_from_above_or_edge_on_is_not_named() {
        let s = seen(vec![one()], View { pitch: 80., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 12.), []);
        // A long thin plinth seen down its length: its short end faces the camera, too short for
        // the name, and its long side edge on.
        let thin = plinth(1, [0., 0., 0.48], [1.6, 0.06, 0.52]);
        let s = seen(vec![thin], View { yaw: 90., pitch: 12., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 12.), []);
        // Seen from a corner: its short end faces it most, its long side holds the name.
        let s = seen(vec![thin], View { yaw: 60., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 12.).iter().map(|s| s.normal).collect::<Vec<_>>(), [[0., 0., -1.]]);
        // Nearly down its length: the long side's name runs along its up on screen, a sliver.
        let s = seen(vec![thin], View { yaw: 89., ..View::default() });
        assert_eq!(signs(&s, |_| Some(5.), VIEW, 12.), []);
    }

    #[test]
    fn a_tower_in_front_hides_a_name_and_none_does_not() {
        let s = seen(vec![one()], View::default());
        let sign = signs(&s, |_| Some(5.), VIEW, 12.)[0];
        assert!(!hidden(&s, &sign));
        // Between the camera (in front, to the left) and the wall's middle.
        let camera = s.camera.unwrap();
        let middle = at(at(sign.origin, sign.across, sign.width / 2.), UP, sign.em / 2.);
        let on = at(middle, unit(sub(camera.eye, middle)), 0.3);
        let tower = Block {
            min: [on[0] - 0.02, 0., on[2] - 0.02],
            max: [on[0] + 0.02, 2., on[2] + 0.02],
            kind: Kind::Building,
            place: Some(9),
            district: None,
        };
        let behind = seen(vec![one(), tower], View::default());
        assert!(hidden(&behind, &sign));
        // Not drawn (`lod`): it hides nothing.
        assert!(!hidden(&Scene { shown: vec![true, false], ..behind }, &sign));
    }
}
