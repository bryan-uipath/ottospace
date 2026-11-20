//! The code city in 3D: an atlas's plan as boxes on a ground (districts as stepped terraces,
//! files as buildings), and a perspective camera that frames all of it in a picture of a given
//! shape. The shell renders it; the core decides what stands where, how tall, and where the
//! camera is. Design: `docs/ai/code-city/plan.md`, "Drawing".

use crate::atlas::Atlas;

/// The plan is this much wider than deep, as the atlas cuts it.
const WIDE: f32 = 1.6;
/// The tallest building, in plan units (the plan is `WIDE` by 1).
const TALLEST: f32 = 0.16;
/// The rise from a district to the one inside it, by depth, then `LIFT_MIN`: the owner's city's
/// terraces (`TIER_LIFT`, in its units where a tower is at most 60), in ours.
const LIFTS: [f32; 5] = [13., 8., 4.6, 2.6, 1.7];
const LIFT_MIN: f32 = 1.1;
/// Plan units per unit of the owner's city: its tallest tower (60) is our `TALLEST`, halved for
/// terraces, so a backdrop's districts step without towering.
const UNIT: f32 = TALLEST / 60. * 0.5;
/// The gap round a district and round a building, at most, in plan units.
const STREET: f32 = 0.0035;
/// A building's footprint is its lot less this share on each side (at most `STREET`).
const SETBACK: f32 = 0.12;
/// In a city of few files (large lots), the tallest building is at least this many typical lot
/// sides tall, and a street at least this share of one: towers on streets, not slabs.
const TOWER: f32 = 2.5;
const LOT_STREET: f32 = 0.12;
/// A lit building's top stands at least this far above the highest terrace, so no terrace in
/// front of it hides it.
const LIT_ABOVE: f32 = 0.02;
/// The camera looks down this much (degrees) and from this side of the plan (degrees from its
/// front, toward its left), with this vertical field of view.
const PITCH: f32 = 38.;
const YAW: f32 = 34.;
const FOV: f32 = 30.;
/// Room kept round the city in the picture, a share of each side.
const MARGIN: f32 = 0.04;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The ground under everything.
    Ground,
    /// A district: a folder, its depth from the root (1 for a folder of the root).
    Plate(usize),
    Building,
    /// A building an agent is at, in its task's colour.
    Lit(u32),
}

/// A box standing on the ground plane: x across the plan, z into it, y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    /// Vertical, in radians.
    pub fov: f32,
    /// The picture's width over its height.
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub blocks: Vec<Block>,
    pub camera: Option<Camera>,
}

/// A building's height, 0 to 1, from a file's size: the owner's city's `buildingHeight`
/// (`2 + 6·log2(1 + lines/10)`, from 2 to 60), its lines guessed as bytes / 32.
pub(crate) fn height(bytes: u64) -> f32 {
    let lines = bytes as f32 / 32.;
    (2. + 6. * (1. + lines / 10.).log2()).clamp(2., 60.) / 60.
}

/// The top of a district at `depth` (0: the ground).
fn terrace(depth: usize) -> f32 {
    (0..depth).map(|at| LIFTS.get(at).copied().unwrap_or(LIFT_MIN) * UNIT).sum()
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}

impl Block {
    fn corners(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        (0..8).map(|i| {
            let pick = |axis: usize| if i >> axis & 1 == 0 { self.min[axis] } else { self.max[axis] };
            [pick(0), pick(1), pick(2)]
        })
    }

    /// Where the segment from `a` to `b` is inside this box, as shares of it (slab method).
    fn crossed(&self, a: [f32; 3], b: [f32; 3]) -> Option<(f32, f32)> {
        let (mut t0, mut t1) = (0f32, 1f32);
        for axis in 0..3 {
            let d = b[axis] - a[axis];
            if d.abs() < 1e-9 {
                if a[axis] < self.min[axis] || a[axis] > self.max[axis] {
                    return None;
                }
                continue;
            }
            let (s, e) = ((self.min[axis] - a[axis]) / d, (self.max[axis] - a[axis]) / d);
            (t0, t1) = (t0.max(s.min(e)), t1.min(s.max(e)));
        }
        (t0 < t1).then_some((t0, t1))
    }
}

impl Camera {
    /// The camera's axes: right, up, forward.
    fn axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let forward = unit(sub(self.target, self.eye));
        let right = unit(cross(forward, [0., 1., 0.]));
        (right, cross(right, forward), forward)
    }

    /// World to clip space, column-major, for the renderer: x and y from -1 to 1 across the
    /// picture, depth from 0 (near) to 1 (far).
    pub fn matrix(&self) -> [[f32; 4]; 4] {
        let (r, u, f) = self.axes();
        let e = self.eye;
        let sy = 1. / (self.fov / 2.).tan();
        let sx = sy / self.aspect;
        let (a, b) = (self.far / (self.far - self.near), -self.near * self.far / (self.far - self.near));
        // Rows of projection × view, written out.
        let row = |s: f32, axis: [f32; 3]| [s * axis[0], s * axis[1], s * axis[2], -s * dot(axis, e)];
        let (x, y) = (row(sx, r), row(sy, u));
        let z = [a * f[0], a * f[1], a * f[2], -a * dot(f, e) + b];
        let w = [f[0], f[1], f[2], -dot(f, e)];
        let rows = [x, y, z, w];
        std::array::from_fn(|col| std::array::from_fn(|row| rows[row][col]))
    }

    /// Where a point is in the picture, as shares of it (0,0 top left; 1,1 bottom right).
    /// `None` behind the camera.
    pub fn project(&self, p: [f32; 3]) -> Option<(f32, f32)> {
        let m = self.matrix();
        let clip = |row: usize| m[0][row] * p[0] + m[1][row] * p[1] + m[2][row] * p[2] + m[3][row];
        let w = clip(3);
        (w > self.near * 0.5).then(|| ((clip(0) / w + 1.) / 2., (1. - clip(1) / w) / 2.))
    }

    /// Where a box is in the picture: the bounds of its corners, as shares (x, y, w, h).
    pub fn bounds(&self, block: &Block) -> Option<[f32; 4]> {
        let points: Option<Vec<(f32, f32)>> = block.corners().map(|p| self.project(p)).collect();
        let points = points?;
        let (x0, y0) = points.iter().fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
        let (x1, y1) = points.iter().fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.0), y.max(p.1)));
        Some([x0, y0, x1 - x0, y1 - y0])
    }
}

/// The camera that frames `blocks` in a picture `aspect` wide to 1 tall: looking down `PITCH`
/// from `YAW` off the plan's front, as near as leaves `MARGIN` round every box.
fn frame(blocks: &[Block], aspect: f32) -> Option<Camera> {
    let lo = blocks.iter().fold([f32::MAX; 3], |lo, b| std::array::from_fn(|i| lo[i].min(b.min[i])));
    let hi = blocks.iter().fold([f32::MIN; 3], |hi, b| std::array::from_fn(|i| hi[i].max(b.max[i])));
    if blocks.is_empty() || aspect.is_nan() || aspect <= 0. {
        return None;
    }
    let all = Block { min: lo, max: hi, kind: Kind::Ground };
    let target = [(lo[0] + hi[0]) / 2., 0., (lo[2] + hi[2]) / 2.];
    let (pitch, yaw) = (PITCH.to_radians(), YAW.to_radians());
    // From the camera to the target: down by the pitch, into the plan turned by the yaw.
    let back = [-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()];
    let at = |distance: f32| {
        let eye = std::array::from_fn(|i| target[i] + back[i] * distance);
        let span = distance + 4.;
        Camera { eye, target, fov: FOV.to_radians(), aspect, near: distance / 50., far: span }
    };
    let fits = |camera: &Camera| {
        all.corners().all(|p| {
            camera
                .project(p)
                .is_some_and(|(x, y)| (MARGIN..=1. - MARGIN).contains(&x) && (MARGIN..=1. - MARGIN).contains(&y))
        })
    };
    // Nearer frames tighter: the nearest distance that still fits.
    // Far enough for any shape: a very narrow picture needs the camera far back.
    let (mut near, mut far) = (0.01f32, 100f32);
    while !fits(&at(far)) {
        if far > 1e6 {
            return None;
        }
        (near, far) = (far, far * 4.);
    }
    for _ in 0..40 {
        let mid = (near + far) / 2.;
        if fits(&at(mid)) { far = mid } else { near = mid }
    }
    Some(at(far))
}

/// The city of `atlas` for a picture `aspect` wide to 1 tall. `lit`: places (by id) an agent is
/// at, each with its task's colour. A lit building stands above every terrace, and any building
/// between the camera and a lit one's top is cut down to below the sight line: a lit building is
/// never hidden.
pub fn scene(atlas: &Atlas, lit: &[(&str, u32)], aspect: f32) -> Scene {
    let ground = Block { min: [0., -0.01, 0.], max: [WIDE, 0., 1.], kind: Kind::Ground };
    let mut blocks = vec![ground];
    let highest = atlas.places.iter().filter(|p| p.group).map(|p| terrace(p.depth)).fold(0f32, f32::max);
    // A typical lot's side: e.g. 0.24 for 28 files, 0.018 for 5,000 (where the constants rule).
    let lot = (WIDE / atlas.places.iter().filter(|p| !p.group).count().max(1) as f32).sqrt();
    let (tallest, street) = (TALLEST.max(lot * TOWER), STREET.max(lot * LOT_STREET));
    for place in &atlas.places {
        let r = place.rect;
        let (x0, z0, x1, z1) = (r.x * WIDE, r.y, (r.x + r.w) * WIDE, r.y + r.h);
        let floor = terrace(place.depth - 1);
        if place.group {
            let street = street.min((x1 - x0) / 4.).min((z1 - z0) / 4.);
            let (min, max) = ([x0 + street, floor, z0 + street], [x1 - street, terrace(place.depth), z1 - street]);
            blocks.push(Block { min, max, kind: Kind::Plate(place.depth) });
            continue;
        }
        let set = |side: f32| (side * SETBACK).min(street);
        let (sx, sz) = (set(x1 - x0), set(z1 - z0));
        let colour = lit.iter().find(|(id, _)| *id == place.id).map(|(_, colour)| *colour);
        let mut top = floor + place.height.unwrap_or(0.).clamp(0., 1.).max(1. / 30.) * tallest;
        if colour.is_some() {
            top = top.max(highest + LIT_ABOVE);
        }
        let kind = colour.map_or(Kind::Building, Kind::Lit);
        blocks.push(Block { min: [x0 + sx, floor, z0 + sz], max: [x1 - sx, top, z1 - sz], kind });
    }
    let Some(camera) = frame(&blocks, aspect) else { return Scene { blocks, camera: None } };
    clear_sight(&mut blocks, camera.eye, highest + LIT_ABOVE);
    Scene { blocks, camera: Some(camera) }
}

/// The top centre of a box: what of a lit building must be seen.
fn top(block: &Block) -> [f32; 3] {
    [(block.min[0] + block.max[0]) / 2., block.max[1], (block.min[2] + block.max[2]) / 2.]
}

/// Cuts down every building that stands in the line from `eye` to a lit building's top, to just
/// below that line; a lit one no lower than `lit_floor`, still above every terrace. A lit one cut
/// has a new line of its own, so again until nothing is cut.
fn clear_sight(blocks: &mut [Block], eye: [f32; 3], lit_floor: f32) {
    let lit = |b: &Block| matches!(b.kind, Kind::Lit(_));
    for _ in 0..8 {
        let mut cut = false;
        for to in blocks.iter().filter(|b| lit(b)).map(top).collect::<Vec<_>>() {
            for block in blocks.iter_mut().filter(|b| b.kind == Kind::Building || lit(b)) {
                // Only where the line comes down to it: the line is lowest where it leaves the box.
                let Some((_, leaves)) = block.crossed(eye, to).filter(|(_, leaves)| *leaves < 0.999) else { continue };
                let floor = if lit(block) { lit_floor } else { block.min[1] };
                let below = (eye[1] + (to[1] - eye[1]) * leaves - 1e-4).max(floor);
                cut |= below < block.max[1];
                block.max[1] = below.min(block.max[1]);
            }
        }
        if !cut {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{atlas, repo};

    fn city(files: &[(&str, u64)]) -> Atlas {
        let files: Vec<(String, u64)> = files.iter().map(|(p, b)| (p.to_string(), *b)).collect();
        atlas(&repo(&files))
    }

    /// A repository with a tall file in front of a small one, as the camera sees them.
    fn many() -> Atlas {
        let mut files: Vec<(String, u64)> = Vec::new();
        for d in 0..6 {
            for f in 0..30 {
                files.push((format!("d{d}/s{}/f{f}.rs", f % 4), 200 + ((d * 31 + f * 17) % 50) as u64 * 900));
            }
        }
        atlas(&repo(&files))
    }

    #[test]
    fn a_file_is_as_tall_as_the_owners_city_makes_it() {
        assert_eq!(height(0), 2. / 60.);
        // 3,200 bytes: 100 lines, 2 + 6·log2(11).
        assert!((height(3_200) - (2. + 6. * 11f32.log2()) / 60.).abs() < 1e-6);
        assert_eq!(height(u64::MAX), 1.);
    }

    /// A city of few files has large lots: its buildings are towers on streets, not slabs. A
    /// large one keeps the owner's heights.
    #[test]
    fn a_city_of_few_files_stands_as_towers_on_streets() {
        let files: Vec<(String, u64)> = (0..12).map(|i| (format!("d{}/f{i}.rs", i % 3), 9_000)).collect();
        let s = scene(&atlas(&repo(&files)), &[], 1.6);
        let lot = (WIDE / 12.).sqrt();
        let buildings: Vec<&Block> = s.blocks.iter().filter(|b| b.kind == Kind::Building).collect();
        let tallest = buildings.iter().map(|b| b.max[1] - b.min[1]).fold(0., f32::max);
        assert!(tallest >= lot, "{tallest} < {lot}");
        // No two buildings closer than twice a large city's street.
        for (i, a) in buildings.iter().enumerate() {
            for b in &buildings[i + 1..] {
                let gap = |k: usize| (b.min[k] - a.max[k]).max(a.min[k] - b.max[k]);
                assert!(gap(0).max(gap(2)) >= 2. * STREET, "{a:?} {b:?}");
            }
        }
        let big: Vec<(String, u64)> = (0..5_000).map(|i| (format!("d{}/f{i}.rs", i % 7), u64::MAX)).collect();
        let s = scene(&atlas(&repo(&big)), &[], 1.6);
        let tallest = s.blocks.iter().filter(|b| b.kind == Kind::Building).map(|b| b.max[1] - b.min[1]);
        assert!((tallest.fold(0., f32::max) - TALLEST).abs() < 1e-6);
    }

    #[test]
    fn districts_step_up_and_buildings_stand_on_their_district() {
        let s = scene(&city(&[("src/ui/a.rs", 900), ("src/b.rs", 900), ("c.rs", 900)]), &[], 1.6);
        let plate = |depth| s.blocks.iter().find(|b| b.kind == Kind::Plate(depth)).unwrap();
        assert_eq!(plate(1).min[1], 0.);
        assert_eq!(plate(2).min[1], plate(1).max[1]);
        assert!(plate(2).max[1] > plate(1).max[1]);
        let floors: Vec<f32> = s.blocks.iter().filter(|b| b.kind == Kind::Building).map(|b| b.min[1]).collect();
        // c.rs on the ground, b.rs on src, a.rs on src/ui.
        assert_eq!(floors.len(), 3);
        assert!(floors.contains(&0.) && floors.contains(&plate(1).max[1]) && floors.contains(&plate(2).max[1]));
    }

    #[test]
    fn every_box_is_in_the_picture_at_any_shape() {
        let atlas = many();
        for aspect in [0.02, 0.5, 1., 1.6, 3., 40.] {
            let s = scene(&atlas, &[], aspect);
            let camera = s.camera.unwrap();
            for block in &s.blocks {
                let [x, y, w, h] = camera.bounds(block).unwrap();
                assert!(x >= 0. && y >= 0. && x + w <= 1. && y + h <= 1., "{aspect}: {block:?}");
            }
        }
    }

    #[test]
    fn the_projection_agrees_with_the_camera() {
        let s = scene(&many(), &[], 1.6);
        let camera = s.camera.unwrap();
        // The target is the picture's centre; a point nearer the camera is lower in it.
        let (x, y) = camera.project(camera.target).unwrap();
        assert!((x - 0.5).abs() < 1e-4 && (y - 0.5).abs() < 1e-4, "{x} {y}");
        let near: [f32; 3] = std::array::from_fn(|i| camera.target[i] + (camera.eye[i] - camera.target[i]) * 0.1);
        let near = [near[0], 0., near[2]];
        assert!(camera.project(near).unwrap().1 > 0.5);
        assert!(camera.project(sub(camera.eye, sub(camera.target, camera.eye))).is_none(), "behind it");
    }

    #[test]
    fn a_lit_building_is_coloured_and_stands_above_every_terrace() {
        let atlas = many();
        let id = atlas.places.iter().find(|p| !p.group && p.depth == 3).unwrap().id.clone();
        let s = scene(&atlas, &[(&id, 0xff6467)], 1.6);
        let lit: Vec<&Block> = s.blocks.iter().filter(|b| b.kind == Kind::Lit(0xff6467)).collect();
        assert_eq!(lit.len(), 1);
        let highest = s.blocks.iter().filter(|b| matches!(b.kind, Kind::Plate(_))).map(|b| b.max[1]).fold(0., f32::max);
        assert!(lit[0].max[1] >= highest + LIT_ABOVE - 1e-6);
        assert_eq!(s.blocks.iter().filter(|b| matches!(b.kind, Kind::Lit(_))).count(), 1);
    }

    /// The camera sees every lit building's top: nothing else stands in the line to it.
    #[test]
    fn a_lit_building_is_never_hidden() {
        let atlas = many();
        let leaves: Vec<&str> = atlas.places.iter().filter(|p| !p.group).map(|p| p.id.as_str()).collect();
        let mut cut = 0;
        for aspect in [0.6, 1.6] {
            for (i, id) in leaves.iter().enumerate().step_by(7) {
                // One lit, then two: far apart, and neighbours (one may stand in front of the other).
                let (far, next) = (leaves[(i + 40) % leaves.len()], leaves[(i + 1) % leaves.len()]);
                for lit in [vec![(*id, 1)], vec![(*id, 1), (far, 2)], vec![(*id, 1), (next, 2)]] {
                    let plain = scene(&atlas, &[], aspect);
                    let s = scene(&atlas, &lit, aspect);
                    let eye = s.camera.unwrap().eye;
                    for to in s.blocks.iter().filter(|b| matches!(b.kind, Kind::Lit(_))).map(top) {
                        let over: Vec<&Block> =
                            s.blocks.iter().filter(|b| b.crossed(eye, to).is_some_and(|(_, t)| t < 0.999)).collect();
                        assert!(over.is_empty(), "{id}: {over:?}");
                    }
                    cut += plain.blocks.iter().zip(&s.blocks).filter(|(a, b)| a.max[1] > b.max[1] + 1e-6).count();
                }
            }
        }
        // Some building somewhere was in the way and cut down: the test reaches the rule.
        assert!(cut > 0);
    }

    #[test]
    fn no_camera_for_a_picture_of_no_width() {
        assert!(scene(&Atlas::default(), &[], 1.6).camera.is_some(), "the ground alone");
        assert!(scene(&many(), &[], 0.).camera.is_none());
    }
}
