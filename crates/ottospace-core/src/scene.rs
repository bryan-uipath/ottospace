//! The code city in 3D: an atlas's plan as boxes on a ground (districts as stepped terraces,
//! files as buildings), and a perspective camera that frames all of it in a picture of a given
//! shape. The shell renders it; the core decides what stands where, how tall, and where the
//! camera is. Design: `docs/ai/code-city/plan.md`, "Drawing".

use serde::{Deserialize, Serialize};

use crate::atlas::Atlas;

/// The plan is this much wider than deep, as the atlas cuts it.
const WIDE: f32 = 1.6;
/// The tallest building, in plan units (the plan is `WIDE` by 1).
const TALLEST: f32 = 0.12;
/// The rise from a district to the one inside it, by depth, then `LIFT_MIN`: the owner's city's
/// terraces (`TIER_LIFT`, in its units where a tower is at most 60), in ours.
const LIFTS: [f32; 5] = [13., 8., 4.6, 2.6, 1.7];
const LIFT_MIN: f32 = 1.1;
/// Plan units per unit of the owner's city: its tallest tower (60) is our `TALLEST`, at three
/// quarters for terraces, so a backdrop's districts step without towering.
const UNIT: f32 = TALLEST / 60. * 0.75;
/// The gap round a district and round a building, at most, in plan units.
const STREET: f32 = 0.0035;
/// A folder whose lot's shorter side is under this (plan units, far below a pixel) is no
/// district: its insets would run a deep chain of folders down to nothing.
const LEAST_DISTRICT: f32 = 1e-3;
/// Districts are this many streets apart, so a folder reads as a block of its own.
const DISTRICT_STREET: f32 = 1.5;
/// The ground a district keeps free inside its own edge, a share of its shorter side at most
/// (and at most a street): a district inside it never reaches it.
const KERB: f32 = 0.125;
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
const PITCH: f32 = 52.;
const YAW: f32 = 34.;
const FOV: f32 = 30.;
/// Room kept round the city in the picture, a share of each side.
const MARGIN: f32 = 0.04;
/// A beam rises above a box's top as high as the city's highest top stands, and at least this
/// (plan units: the plan is 1 deep): in a city of few files towers are tall, and so are beams.
const BEAM: f32 = 0.2;

/// How folders are drawn: an experiment for the owner to pick from (`developer.city_look`, not a
/// user's setting); the pick becomes the only look and the rest go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Look {
    /// As built: terraces a step lighter per tier, the outer two tiers named.
    #[default]
    Terraces,
    /// Each district a slab three times as high (more in a city of tall towers), its top edge
    /// lit, its sides' edges dim.
    Plinths,
    /// Each top-level district and all on it a hue of its own, outlined stronger, wider streets.
    Tint,
    /// Every district to the fourth tier named, larger and brighter the outer it is.
    Labels,
    /// Plinths two thirds as high, tint, and names to the third tier.
    Blend,
}

impl Look {
    /// A tier's lift, times the terraces' (a plinth's also times its city's towers: `scene_as`).
    fn lift(self) -> f32 {
        match self {
            Look::Plinths => 3.,
            Look::Blend => 2.,
            _ => 1.,
        }
    }

    /// Districts are this many streets apart.
    fn district_street(self) -> f32 {
        match self {
            Look::Tint => 3.,
            Look::Blend => 2.5,
            _ => DISTRICT_STREET,
        }
    }

    /// Districts named on their plates: to this tier.
    pub fn named_tiers(self) -> usize {
        match self {
            Look::Labels => 4,
            Look::Blend => 3,
            _ => 2,
        }
    }

    /// Each top-level district in a hue of its own.
    pub fn tinted(self) -> bool {
        matches!(self, Look::Tint | Look::Blend)
    }

    /// A district's top edge lit, its sides' edges dim.
    pub fn rims(self) -> bool {
        matches!(self, Look::Plinths | Look::Blend)
    }

    /// Names sized and lit by tier.
    pub fn graded_names(self) -> bool {
        matches!(self, Look::Labels | Look::Blend)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The ground under everything.
    Ground,
    /// A district: a folder, its tier (1 for the outermost). A folder with one child is no
    /// district of its own: its child stands where it would, at its tier.
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
    /// The atlas place it stands for (an index into `Atlas::places`); none for the ground.
    pub place: Option<usize>,
    /// The top-level district it is (or stands in), counted from 0; none outside every district.
    pub district: Option<usize>,
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
    pub look: Look,
}

/// A building's height, 0 to 1, from a file's size: the owner's city's `buildingHeight`
/// (`2 + 6·log2(1 + lines/10)`, from 2 to 60), its lines guessed as bytes / 32.
pub(crate) fn height(bytes: u64) -> f32 {
    let lines = bytes as f32 / 32.;
    (2. + 6. * (1. + lines / 10.).log2()).clamp(2., 60.) / 60.
}

/// The top of a district at `depth` (0: the ground), each lift `lift` times the terraces'.
fn terrace(depth: usize, lift: f32) -> f32 {
    (0..depth).map(|at| LIFTS.get(at).copied().unwrap_or(LIFT_MIN) * UNIT * lift).sum()
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
    /// The middle of its top: what of a lit building must be seen, where a beacon stands.
    pub fn top(&self) -> [f32; 3] {
        [(self.min[0] + self.max[0]) / 2., self.max[1], (self.min[2] + self.max[2]) / 2.]
    }

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
    let all = Block { min: lo, max: hi, kind: Kind::Ground, place: None, district: None };
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

/// An area of the ground: x from `x0` to `x1`, z from `z0` to `z1`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Area {
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
}

impl Area {
    fn shrunk(self, by: f32) -> Area {
        Area { x0: self.x0 + by, z0: self.z0 + by, x1: self.x1 - by, z1: self.z1 - by }
    }

    fn short(self) -> f32 {
        (self.x1 - self.x0).min(self.z1 - self.z0)
    }

    /// `a`, a part of `self`, as the same part of `to`.
    fn map(self, a: Area, to: Area) -> Area {
        let x = |v: f32| to.x0 + (v - self.x0) / (self.x1 - self.x0) * (to.x1 - to.x0);
        let z = |v: f32| to.z0 + (v - self.z0) / (self.z1 - self.z0) * (to.z1 - to.z0);
        Area { x0: x(a.x0), z0: z(a.z0), x1: x(a.x1), z1: z(a.z1) }
    }
}

/// What a group of the plan leaves for what stands in it: its lot on the plan (`from`), drawn
/// in `to` (its district less a kerb), on which tier.
#[derive(Clone, Copy)]
struct Room {
    depth: usize,
    from: Area,
    to: Area,
    tier: usize,
    district: Option<usize>,
}

/// The city of `atlas` for a picture `aspect` wide to 1 tall. `lit`: places (by id) an agent is
/// at, each with its task's colour.
///
/// Districts: a folder of more than one child is a plate on its parent's, a tier higher, a
/// street in from its lot (siblings never touch). What stands in it is drawn a kerb in from its
/// edge, the plan's shares kept (scaled, never cut). A
/// building stands on its district, set back in its lot. A lit building stands above every
/// terrace, and any building between the camera and a lit one's top is cut down to below the
/// sight line: a lit building is never hidden.
pub fn scene(atlas: &Atlas, lit: &[(&str, u32)], aspect: f32) -> Scene {
    scene_as(atlas, lit, aspect, Look::default())
}

/// `scene`, its folders drawn in `look`.
pub fn scene_as(atlas: &Atlas, lit: &[(&str, u32)], aspect: f32, look: Look) -> Scene {
    let ground = Block { min: [0., -0.01, 0.], max: [WIDE, 0., 1.], kind: Kind::Ground, place: None, district: None };
    let mut blocks = vec![ground];
    // A typical lot's side: e.g. 0.24 for 28 files, 0.018 for 5,000 (where the constants rule).
    let lot = (WIDE / atlas.places.iter().filter(|p| !p.group).count().max(1) as f32).sqrt();
    let (tallest, street) = (TALLEST.max(lot * TOWER), STREET.max(lot * LOT_STREET));
    // A plinth keeps its height beside a few tall towers.
    let lift = look.lift() * if look.rims() { (tallest / TALLEST).min(1.5) } else { 1. };
    let terrace = |tier: usize| terrace(tier, lift);
    // How many children each group has: a group's are the places after it one deeper, up to
    // the next place no deeper than it.
    let mut children = vec![0usize; atlas.places.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, place) in atlas.places.iter().enumerate() {
        while open.last().is_some_and(|&g| atlas.places[g].depth >= place.depth) {
            open.pop();
        }
        if let Some(&g) = open.last() {
            children[g] += 1;
        }
        if place.group {
            open.push(i);
        }
    }
    let whole = Area { x0: 0., z0: 0., x1: WIDE, z1: 1. };
    let root = Room { depth: 0, from: whole, to: whole.shrunk(street), tier: 0, district: None };
    let mut districts = 0;
    let mut rooms = vec![root];
    let mut highest = 0f32;
    let mut buildings = Vec::new();
    for (i, place) in atlas.places.iter().enumerate() {
        while rooms.len() > 1 && rooms.last().is_some_and(|room| room.depth >= place.depth) {
            rooms.pop();
        }
        let Some(&parent) = rooms.last() else { continue };
        let r = place.rect;
        let from = Area { x0: r.x * WIDE, z0: r.y, x1: (r.x + r.w) * WIDE, z1: r.y + r.h };
        let lot = parent.from.map(from, parent.to);
        let floor = terrace(parent.tier);
        if place.group {
            let mut room = Room { depth: place.depth, from, to: lot, tier: parent.tier, district: parent.district };
            if children[i] != 1 && lot.short() >= LEAST_DISTRICT {
                let plate = lot.shrunk((street * look.district_street()).min(lot.short() / 4.));
                let to = plate.shrunk(street.min(plate.short() * KERB));
                // A district on the ground is a top-level one: the next number.
                let district = parent.district.or_else(|| {
                    districts += 1;
                    Some(districts - 1)
                });
                room = Room { depth: place.depth, from, to, tier: parent.tier + 1, district };
                let top = terrace(room.tier);
                highest = highest.max(top);
                let (min, max) = ([plate.x0, floor, plate.z0], [plate.x1, top, plate.z1]);
                blocks.push(Block { min, max, kind: Kind::Plate(room.tier), place: Some(i), district });
            }
            rooms.push(room);
            continue;
        }
        let set = |side: f32| (side * SETBACK).min(street);
        let (sx, sz) = (set(lot.x1 - lot.x0), set(lot.z1 - lot.z0));
        let colour = lit.iter().find(|(id, _)| *id == place.id).map(|(_, colour)| *colour);
        let top = floor + place.height.unwrap_or(0.).clamp(0., 1.).max(1. / 30.) * tallest;
        let kind = colour.map_or(Kind::Building, Kind::Lit);
        buildings.push(Block {
            min: [lot.x0 + sx, floor, lot.z0 + sz],
            max: [lot.x1 - sx, top, lot.z1 - sz],
            kind,
            place: Some(i),
            district: parent.district,
        });
    }
    for block in buildings.iter_mut().filter(|b| matches!(b.kind, Kind::Lit(_))) {
        block.max[1] = block.max[1].max(highest + LIT_ABOVE);
    }
    blocks.extend(buildings);
    let Some(camera) = frame(&blocks, aspect) else { return Scene { blocks, camera: None, look } };
    clear_sight(&mut blocks, camera.eye, highest + LIT_ABOVE);
    Scene { blocks, camera: Some(camera), look }
}

impl Scene {
    /// The box under a point of the picture (shares of it, 0,0 top left), by its index: the
    /// nearest building or district the camera's ray through it meets. Never the ground.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let camera = self.camera?;
        let (r, u, f) = camera.axes();
        let tan = (camera.fov / 2.).tan();
        let (dx, dy) = ((2. * x - 1.) * tan * camera.aspect, (1. - 2. * y) * tan);
        let to: [f32; 3] = std::array::from_fn(|i| camera.eye[i] + (f[i] + r[i] * dx + u[i] * dy) * camera.far);
        let hits = self.blocks.iter().enumerate().filter(|(_, b)| b.kind != Kind::Ground);
        let near = hits.filter_map(|(i, b)| Some((b.crossed(camera.eye, to)?.0, i)));
        near.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, i)| i)
    }

    /// Box `i`'s beam: from the middle of its top straight up `beam_height` (vertical in the
    /// world, so under any camera), its end lowered to stay `MARGIN` inside the picture's top.
    /// Never the ground's.
    pub fn beam(&self, i: usize) -> Option<([f32; 3], [f32; 3])> {
        let (camera, block) = (self.camera?, self.blocks.get(i).filter(|b| b.kind != Kind::Ground)?);
        let foot = block.top();
        let beam = self.beam_height();
        let up = |h: f32| [foot[0], foot[1] + h, foot[2]];
        let inside = |h: f32| camera.project(up(h)).is_some_and(|(_, y)| y >= MARGIN);
        if inside(beam) {
            return Some((foot, up(beam)));
        }
        // The highest end still inside: none higher than the top itself.
        let (mut low, mut high) = (0f32, beam);
        for _ in 0..24 {
            let mid = (low + high) / 2.;
            if inside(mid) { low = mid } else { high = mid }
        }
        Some((foot, up(low)))
    }

    /// How high a beam rises: the city's highest top, at least `BEAM`.
    fn beam_height(&self) -> f32 {
        self.blocks.iter().map(|b| b.max[1]).fold(BEAM, f32::max)
    }
}

/// Cuts down every building that stands in the line from `eye` to a lit building's top, to just
/// below that line; a lit one no lower than `lit_floor`, still above every terrace. A lit one cut
/// has a new line of its own, so again until nothing is cut.
fn clear_sight(blocks: &mut [Block], eye: [f32; 3], lit_floor: f32) {
    let lit = |b: &Block| matches!(b.kind, Kind::Lit(_));
    for _ in 0..8 {
        let mut cut = false;
        for to in blocks.iter().filter(|b| lit(b)).map(Block::top).collect::<Vec<_>>() {
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
        let s = scene(&city(&[("src/ui/a.rs", 900), ("src/ui/d.rs", 900), ("src/b.rs", 900), ("c.rs", 900)]), &[], 1.6);
        let plate = |depth| s.blocks.iter().find(|b| b.kind == Kind::Plate(depth)).unwrap();
        assert_eq!(plate(1).min[1], 0.);
        assert_eq!(plate(2).min[1], plate(1).max[1]);
        assert!(plate(2).max[1] > plate(1).max[1]);
        let floors: Vec<f32> = s.blocks.iter().filter(|b| b.kind == Kind::Building).map(|b| b.min[1]).collect();
        // c.rs on the ground, b.rs on src, a.rs and d.rs on src/ui.
        assert_eq!(floors.len(), 4);
        assert!(floors.contains(&0.) && floors.contains(&plate(1).max[1]) && floors.contains(&plate(2).max[1]));
    }

    /// The plate of a place, by its id.
    fn plate<'a>(s: &'a Scene, atlas: &Atlas, id: &str) -> Option<&'a Block> {
        s.blocks.iter().find(|b| matches!(b.kind, Kind::Plate(_)) && b.place.is_some_and(|p| atlas.places[p].id == id))
    }

    /// Every district lies inside the one it stands on, a kerb in from its edge; no two
    /// districts on one, and no two buildings, touch; a building stands inside its district.
    #[test]
    fn districts_nest_with_a_kerb_and_siblings_never_touch() {
        let mut trees = vec![many(), city(&[("a.rs", 9_000), ("b/c.rs", 10), ("b/d.rs", 10)])];
        for seed in 0..40u64 {
            let files: Vec<(String, u64)> = (0..(5 + seed * 7))
                .map(|i| {
                    let h = (i * 2_654_435_761 + seed * 40_503) % 1_000_003;
                    (format!("d{}/e{}/f{}/g{i}.rs", h % 3, h % 5, h % 2), 100 + h % 60_000)
                })
                .collect();
            trees.push(atlas(&repo(&files)));
        }
        let looks = [Look::Terraces, Look::Plinths, Look::Tint, Look::Labels, Look::Blend];
        for (atlas, look) in trees.iter().flat_map(|atlas| looks.map(|look| (atlas, look))) {
            let s = scene_as(atlas, &[], 1.6, look);
            // The district each place stands in: its nearest group ancestor with a plate.
            let parent_plate = |i: usize| {
                let depth = atlas.places[i].depth;
                let mut want = depth;
                atlas.places[..i].iter().rev().find_map(|p| {
                    if p.depth < want {
                        want = p.depth;
                        return plate(&s, atlas, &p.id).filter(|_| p.group);
                    }
                    None
                })
            };
            let inside = |a: &Block, b: &Block, by: f32| {
                a.min[0] >= b.min[0] + by
                    && a.min[2] >= b.min[2] + by
                    && a.max[0] <= b.max[0] - by
                    && a.max[2] <= b.max[2] - by
            };
            let shown: Vec<&Block> = s.blocks.iter().filter(|b| b.kind != Kind::Ground).collect();
            for b in &shown {
                let Some(i) = b.place else { continue };
                if let Some(parent) = parent_plate(i) {
                    assert!(inside(b, parent, 1e-5), "{} in {:?}", atlas.places[i].id, parent);
                    // On it: a district a tier up, a building on its top.
                    assert_eq!(b.min[1], parent.max[1], "{}", atlas.places[i].id);
                    if let (Kind::Plate(t), Kind::Plate(p)) = (b.kind, parent.kind) {
                        assert_eq!(t, p + 1);
                    }
                }
            }
            let is_plate = |b: &&&Block| matches!(b.kind, Kind::Plate(_));
            for kind in [true, false] {
                let some: Vec<&&Block> = shown.iter().filter(|b| is_plate(b) == kind).collect();
                for (n, a) in some.iter().enumerate() {
                    for b in &some[n + 1..] {
                        let apart = |k: usize| (b.min[k] - a.max[k]).max(a.min[k] - b.max[k]);
                        let nested = inside(a, b, 0.) || inside(b, a, 0.);
                        assert!(nested || apart(0).max(apart(2)) > 1e-6, "{a:?} touches {b:?}");
                    }
                }
            }
        }
    }

    /// A chain of 80 folders, each with a file: the outer ones are districts, each inside the
    /// last; the deep ones, too small for a district, are none, and every box keeps a footprint.
    #[test]
    fn a_deep_chain_of_folders_keeps_every_footprint() {
        let files: Vec<(String, u64)> = (1..=80)
            .flat_map(|depth| {
                let dir = vec!["d"; depth].join("/");
                [(format!("{dir}/a.rs"), 1), (format!("{dir}/b.rs"), 1)]
            })
            .collect();
        let atlas = atlas(&repo(&files));
        let s = scene(&atlas, &[], 1.6);
        assert!(s.blocks.iter().filter(|b| matches!(b.kind, Kind::Plate(_))).count() > 3);
        for b in &s.blocks {
            assert!(b.max[0] > b.min[0] && b.max[2] > b.min[2], "{b:?}");
        }
    }

    /// Every look keeps the plan: the same boxes over the same places. Plinths stand higher, a
    /// tint's districts further apart; each box of a top-level district carries its number, a
    /// file in the root none.
    #[test]
    fn a_look_draws_the_same_places_its_own_way() {
        let atlas = city(&[
            ("a/x.rs", 900),
            ("a/b/y.rs", 900),
            ("a/b/z.rs", 900),
            ("c/w.rs", 900),
            ("c/v.rs", 900),
            ("r.rs", 9),
        ]);
        let plain = scene(&atlas, &[], 1.6);
        let top = |s: &Scene| {
            s.blocks.iter().filter(|b| matches!(b.kind, Kind::Plate(_))).map(|b| b.max[1]).fold(0., f32::max)
        };
        let gap = |s: &Scene| {
            let (a, c) = (plate(s, &atlas, "a").unwrap(), plate(s, &atlas, "c").unwrap());
            (c.min[0] - a.max[0]).max(a.min[0] - c.max[0]).max(c.min[2] - a.max[2]).max(a.min[2] - c.max[2])
        };
        for look in [Look::Plinths, Look::Tint, Look::Labels, Look::Blend] {
            let s = scene_as(&atlas, &[], 1.6, look);
            let places = |s: &Scene| s.blocks.iter().map(|b| (b.place, b.district)).collect::<Vec<_>>();
            assert_eq!(places(&s), places(&plain), "{look:?}");
            assert_eq!(s.look, look);
            assert_eq!(top(&s) > top(&plain), look.rims(), "{look:?}");
            assert_eq!(gap(&s) > gap(&plain), look.tinted(), "{look:?}");
        }
        let district = |id: &str| {
            let place = atlas.places.iter().position(|p| p.id == id);
            plain.blocks.iter().find(|b| b.place == place).unwrap().district
        };
        assert_eq!((district("a"), district("a/b"), district("a/b/y.rs")), (Some(0), Some(0), Some(0)));
        assert_eq!((district("c"), district("c/v.rs"), district("r.rs")), (Some(1), Some(1), None));
    }

    /// A folder with one child is no district: `a` and `a/b` hold only `a/b/c`, which is the
    /// first tier. Its districts are a tier apart by the owner's lifts.
    #[test]
    fn a_folder_of_one_child_is_no_district() {
        let atlas = city(&[("a/b/c/x.rs", 900), ("a/b/c/y.rs", 900), ("a/b/c/z/w.rs", 900), ("a/b/c/z/v.rs", 900)]);
        let s = scene(&atlas, &[], 1.6);
        assert!(plate(&s, &atlas, "a").is_none() && plate(&s, &atlas, "a/b").is_none());
        let (c, z) = (plate(&s, &atlas, "a/b/c").unwrap(), plate(&s, &atlas, "a/b/c/z").unwrap());
        assert_eq!((c.kind, z.kind), (Kind::Plate(1), Kind::Plate(2)));
        assert_eq!((c.min[1], c.max[1], z.max[1]), (0., terrace(1, 1.), terrace(2, 1.)));
    }

    /// The ray through a point of the picture meets what is drawn there: a building's top, a
    /// district's kerb, nothing off the city.
    #[test]
    fn a_point_of_the_picture_hits_what_is_drawn_there() {
        let atlas = many();
        for aspect in [0.7, 1.6, 3.] {
            let s = scene(&atlas, &[], aspect);
            let camera = s.camera.unwrap();
            let mut hit = 0;
            for b in s.blocks.iter().filter(|b| b.kind == Kind::Building) {
                let (x, y) = camera.project(b.top()).unwrap();
                // Another building in front may hide it: then the hit is that one, nearer.
                let got = &s.blocks[s.hit(x, y).unwrap()];
                if got == b {
                    hit += 1;
                } else {
                    let d = |b: &Block| (0..3).map(|i| (b.top()[i] - camera.eye[i]).powi(2)).sum::<f32>();
                    assert!(got.crossed(camera.eye, b.top()).is_some() || d(got) <= d(b), "{got:?} for {b:?}");
                }
            }
            assert!(hit > s.blocks.len() / 2, "{hit}");
            assert_eq!(s.hit(0.001, 0.001), None, "off the city");
        }
        // A district's front kerb, where nothing stands.
        let atlas = city(&[("a/x.rs", 900), ("a/y.rs", 900), ("b.rs", 900)]);
        let s = scene(&atlas, &[], 1.6);
        let a = plate(&s, &atlas, "a").unwrap();
        let kerb = [(a.min[0] + a.max[0]) / 2., a.max[1], a.min[2] + 1e-4];
        let (x, y) = s.camera.unwrap().project(kerb).unwrap();
        assert_eq!(s.hit(x, y).map(|i| &s.blocks[i]), Some(a));
    }

    /// A beam stands straight up from the middle of a box's top, as tall as the city's highest
    /// top (at least `BEAM`) where the picture has room, lower where it has not, its end never
    /// above the picture's margin. A city of few, tall towers has tall beams.
    #[test]
    fn a_beam_rises_straight_up_and_stays_in_the_picture() {
        let atlas = many();
        for aspect in [0.5, 1.6, 4.] {
            let s = scene(&atlas, &[], aspect);
            let camera = s.camera.unwrap();
            let tall = s.blocks.iter().map(|b| b.max[1]).fold(BEAM, f32::max);
            let mut full = 0;
            for (i, b) in s.blocks.iter().enumerate().skip(1) {
                let (foot, end) = s.beam(i).unwrap();
                assert_eq!(foot, b.top());
                assert_eq!((end[0], end[2]), (foot[0], foot[2]), "vertical");
                assert!(end[1] >= foot[1] && end[1] <= foot[1] + tall + 1e-6, "{foot:?} {end:?}");
                let (_, y) = camera.project(end).unwrap();
                assert!(y >= MARGIN - 1e-4, "{aspect}: {y}");
                assert!(y < camera.project(foot).unwrap().1 || end[1] == foot[1], "up in the picture");
                full += usize::from(end[1] == foot[1] + tall);
            }
            assert!(full > 0, "{aspect}: some beam has its whole height");
        }
        let few = city(&[("a.rs", 90_000), ("b.rs", 90_000), ("c/d.rs", 9_000), ("c/e.rs", 9_000)]);
        let s = scene(&few, &[], 1.6);
        let top = s.blocks.iter().map(|b| b.max[1]).fold(0., f32::max);
        assert!(top > BEAM, "towers taller than the least beam: {top}");
        let (foot, end) = s.beam(s.blocks.len() - 1).unwrap();
        assert!(end[1] - foot[1] > BEAM, "{foot:?} {end:?}");
        assert_eq!(scene(&atlas, &[], 1.6).beam(0), None, "the ground has none");
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
                    for to in s.blocks.iter().filter(|b| matches!(b.kind, Kind::Lit(_))).map(Block::top) {
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
