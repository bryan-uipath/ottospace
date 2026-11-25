//! The code city in 3D: an atlas's plan as boxes on a ground (districts as stepped plinths,
//! files as buildings), and a perspective camera that frames all of it in a picture of a given
//! shape. The shell renders it; the core decides what stands where, how tall, and where the
//! camera is. Design: `docs/ai/code-city/plan.md`, "Drawing".

use crate::atlas::Atlas;

/// The plan is this much wider than deep, as the atlas cuts it.
const WIDE: f32 = 1.6;
/// A tower of lines in the owner's city at its tallest, in plan units (the plan is `WIDE` by
/// 1): the unit of a building's height.
const TALLEST: f32 = 0.12;
/// The owner's strata (in `TALLEST`s): the most levels a file stacks, each one commit; a level's
/// height, and its slab's (the rest a gap, so the strata read apart); the stub a file stands on.
pub(crate) const LEVELS: u32 = 120;
const LEVEL: f32 = 1.6 / 60.;
const SLAB: f32 = 1.15 / 60.;
const STUB: f32 = 0.4 / 60.;
/// The rise from a district to the one inside it, by depth, then `LIFT_MIN`: the owner's city's
/// terraces (`TIER_LIFT`, in its units where a tower is at most 60), in ours.
const LIFTS: [f32; 5] = [13., 8., 4.6, 2.6, 1.7];
const LIFT_MIN: f32 = 1.1;
/// Plan units per unit of the owner's city: its tallest tower (60) is our `TALLEST`, at three
/// quarters for terraces, so a backdrop's districts step without towering.
const UNIT: f32 = TALLEST / 60. * 0.75;
/// A district is a plinth this many times its terrace's lift (up to half as much again in a city
/// of few tall towers), so a folder reads as a block standing on its parent.
const PLINTH: f32 = 3.;
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
const PITCH: f32 = 38.;
const YAW: f32 = 34.;
const FOV: f32 = 30.;
/// Room kept round the city in the picture, a share of each side.
const MARGIN: f32 = 0.04;
/// The fitted city's middle sits this far below the picture's (a share of its height: 0.6 of the
/// way down), so the room over it holds the sky, beams, names and agents' icons.
pub const LOWER: f32 = 0.1;
/// A beam rises above a box's top as high as the city's highest top stands, and at least this
/// (plan units: the plan is 1 deep): in a city of few files towers are tall, and so are beams.
const BEAM: f32 = 0.2;
/// And on screen at least this share of the picture's height, where the picture's top allows.
const BEAM_SHARE: f32 = 0.22;

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
    /// How far below the picture's middle the target is drawn, a share of its height (a lens
    /// shift: the view's perspective is kept).
    pub shift: f32,
}

/// Where an agent's icon floats: over box `over` (its cone falls on that box's top) or, with
/// none, at the city's edge; at `at` in the world; `fan`: its place among the icons there
/// (`k` of `n`), spread side by side as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub over: Option<usize>,
    pub at: [f32; 3],
    pub fan: (usize, usize),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub blocks: Vec<Block>,
    pub camera: Option<Camera>,
    /// The fitted view's distance and target, for the camera of any other view (`Fit::camera`).
    pub fit: Option<Fit>,
    /// Each block's top and edge colours by its folder (`city_colours::tints`); none: its kind's.
    pub tints: Vec<Option<(u32, u32)>>,
    /// Which blocks are drawn (`lod::Tree::apply`): a box inside a closed one is not. Empty: all.
    pub shown: Vec<bool>,
    /// The colour a closed box glows in for a lit file it hides (`lod::Tree::apply`). Empty: none.
    pub glow: Vec<Option<u32>>,
    /// The strata a building is drawn as, in place of its box (`Layer`), by block.
    pub layers: Vec<Layer>,
}

/// A slice of building `block`'s box, drawn in place of it: from `y[0]` up to `y[1]` (no higher
/// than the box, which a lit building's sight line may have cut), in `colour`, or with none in the
/// box's own (its stub, in its folder's colour).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub block: usize,
    pub y: [f32; 2],
    pub colour: Option<u32>,
}

/// Where the user has turned, moved and zoomed the camera, from the fitted view: degrees round
/// the plan from its front (yaw) and down from level (pitch), the distance as a share of the
/// fitted one, the point looked at, moved over the plan from the city's middle (plan units), and
/// the picture slid up by `lift`, a share of its height (a vertical pan: the perspective kept).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub lift: f32,
}

/// Pitch is kept between these (degrees): never under the ground, never straight down.
pub const PITCH_RANGE: (f32, f32) = (12., 80.);
/// Zoom is kept between these: near enough that a building of a small city fills the tab, and
/// the fitted view (the whole city with its margin).
pub const ZOOM_RANGE: (f32, f32) = (0.05, 1.);
/// Lift is kept within this, either way: the point looked at stays in the picture.
pub const LIFT_MAX: f32 = 0.5;

impl Default for View {
    fn default() -> Self {
        View { yaw: YAW, pitch: PITCH, zoom: 1., pan: [0., 0.], lift: 0. }
    }
}

impl View {
    /// Inside its clamps: yaw from 0 to 360, pitch and zoom in their ranges, the point looked
    /// at over the plan. Not finite: the fitted view's.
    pub fn clamped(self) -> View {
        let fitted = View::default();
        let or = |v: f32, d: f32| if v.is_finite() { v } else { d };
        View {
            yaw: or(self.yaw, fitted.yaw).rem_euclid(360.),
            pitch: or(self.pitch, fitted.pitch).clamp(PITCH_RANGE.0, PITCH_RANGE.1),
            zoom: or(self.zoom, 1.).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1),
            pan: [or(self.pan[0], 0.).clamp(-WIDE / 2., WIDE / 2.), or(self.pan[1], 0.).clamp(-0.5, 0.5)],
            lift: or(self.lift, 0.).clamp(-LIFT_MAX, LIFT_MAX),
        }
    }

    /// Turned round the point looked at by `yaw` and tilted by `pitch` degrees.
    pub fn orbited(self, yaw: f32, pitch: f32) -> View {
        View { yaw: self.yaw + yaw, pitch: self.pitch + pitch, ..self }.clamped()
    }

    /// The point looked at moved by `dx`, `dz` over the plan, and the picture slid up by `up`.
    pub fn panned(self, dx: f32, dz: f32, up: f32) -> View {
        View { pan: [self.pan[0] + dx, self.pan[1] + dz], lift: self.lift + up, ..self }.clamped()
    }

    /// Nearer by `factor` (under 1: in), keeping `about` (a point of the plan, from the city's
    /// middle) where it is in the picture: the point looked at moves toward it as much.
    pub fn zoomed(self, factor: f32, about: [f32; 2]) -> View {
        let zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let f = zoom / self.zoom;
        let pan = [about[0] + (self.pan[0] - about[0]) * f, about[1] + (self.pan[1] - about[1]) * f];
        View { zoom, pan, ..self }.clamped()
    }

    /// Inside its clamps and finite: for the model's check.
    pub fn sound(&self) -> bool {
        let all = [self.yaw, self.pitch, self.zoom, self.pan[0], self.pan[1], self.lift];
        all.iter().all(|v| v.is_finite()) && *self == self.clamped()
    }
}

/// The fitted view of a scene: its distance, the point it looks at (the city's middle) and how
/// far below the picture's middle that point is drawn (`LOWER`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub distance: f32,
    pub target: [f32; 3],
    pub lower: f32,
}

impl Fit {
    /// The camera of `view` for a picture `aspect` wide to 1 tall.
    pub fn camera(&self, view: &View, aspect: f32) -> Camera {
        let (pitch, yaw) = (view.pitch.to_radians(), view.yaw.to_radians());
        // From the camera to the target: down by the pitch, into the plan turned by the yaw.
        let back = [-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()];
        let target = [self.target[0] + view.pan[0], self.target[1], self.target[2] + view.pan[1]];
        let distance = self.distance * view.zoom;
        let eye = std::array::from_fn(|i| target[i] + back[i] * distance);
        // The point looked at stays in the picture whatever `lower` is (`developer.city_frame`).
        let shift = (self.lower - view.lift).clamp(MARGIN - 0.5, 0.5 - MARGIN);
        let (near, far) = (distance / 50., distance + 4.);
        Camera { eye, target, fov: FOV.to_radians(), aspect, near, far, shift }
    }
}

/// A file's height, 0 to 1, from the commits that touched it: a share of `LEVELS`, the most the
/// owner's city stacks (its strata: one level a commit, `MAX_LEVELS`).
pub(crate) fn height(commits: u32) -> f32 {
    commits.min(LEVELS) as f32 / LEVELS as f32
}

/// How tall a place of `height` (0 to 1) stands, in `tallest`s: as the owner's city's strata,
/// its stub (0.4 of its units, where a tower of lines is at most 60) and a level (1.6) for each
/// of `LEVELS`.
fn rise(height: Option<f32>) -> f32 {
    STUB + height.unwrap_or(0.).clamp(0., 1.) * LEVELS as f32 * LEVEL
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
        let x = row(sx, r);
        let z = [a * f[0], a * f[1], a * f[2], -a * dot(f, e) + b];
        let w = [f[0], f[1], f[2], -dot(f, e)];
        // The lens shifted: the target `shift` of the height below the middle.
        let y: [f32; 4] = std::array::from_fn(|i| row(sy, u)[i] - 2. * self.shift * w[i]);
        let rows = [x, y, z, w];
        std::array::from_fn(|col| std::array::from_fn(|row| rows[row][col]))
    }

    /// Where a point is in the picture, as shares of it (0,0 top left; 1,1 bottom right).
    /// `None` behind the camera.
    pub fn project(&self, p: [f32; 3]) -> Option<(f32, f32)> {
        self.project_by(&self.matrix(), p)
    }

    /// `project`, by the camera's matrix made once.
    fn project_by(&self, m: &[[f32; 4]; 4], p: [f32; 3]) -> Option<(f32, f32)> {
        let clip = |row: usize| m[0][row] * p[0] + m[1][row] * p[1] + m[2][row] * p[2] + m[3][row];
        let w = clip(3);
        (w > self.near * 0.5).then(|| ((clip(0) / w + 1.) / 2., (1. - clip(1) / w) / 2.))
    }

    /// Where a box is in the picture: the bounds of its corners, as shares (x, y, w, h).
    pub fn bounds(&self, block: &Block) -> Option<[f32; 4]> {
        let m = self.matrix();
        let points: Option<Vec<(f32, f32)>> = block.corners().map(|p| self.project_by(&m, p)).collect();
        let points = points?;
        let (x0, y0) = points.iter().fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
        let (x1, y1) = points.iter().fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.0), y.max(p.1)));
        Some([x0, y0, x1 - x0, y1 - y0])
    }
}

/// The fitted view of `blocks` in a picture `aspect` wide to 1 tall: looking down `PITCH` from
/// `YAW` off the plan's front, as near as leaves `MARGIN` round every box.
fn frame(blocks: &[Block], aspect: f32, lower: f32) -> Option<Fit> {
    let lo = blocks.iter().fold([f32::MAX; 3], |lo, b| std::array::from_fn(|i| lo[i].min(b.min[i])));
    let hi = blocks.iter().fold([f32::MIN; 3], |hi, b| std::array::from_fn(|i| hi[i].max(b.max[i])));
    if blocks.is_empty() || aspect.is_nan() || aspect <= 0. {
        return None;
    }
    let all = Block { min: lo, max: hi, kind: Kind::Ground, place: None, district: None };
    let target = [(lo[0] + hi[0]) / 2., 0., (lo[2] + hi[2]) / 2.];
    let at = |distance: f32| Fit { distance, target, lower }.camera(&View::default(), aspect);
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
    Some(Fit { distance: far, target, lower })
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

/// The city of `atlas` for a picture `aspect` wide to 1 tall, seen from `view`. `lit`: places
/// (by id) an agent is at, each with its task's colour.
///
/// Districts: a folder of more than one child is a plate on its parent's, a tier higher, a
/// street in from its lot (siblings never touch). What stands in it is drawn a kerb in from its
/// edge, the plan's shares kept (scaled, never cut). A
/// building stands on its district, set back in its lot. A lit building stands above every
/// terrace, and from the fitted view any building between the camera and a lit one's top is cut
/// down to below the sight line: a lit building is never hidden. A view the user moved cuts
/// nothing: they look where they choose, and nothing changes height as they turn.
pub fn scene(atlas: &Atlas, lit: &[(&str, u32)], aspect: f32, view: &View) -> Scene {
    scene_lowered(atlas, lit, aspect, view, LOWER)
}

/// `scene`, its fitted middle `lower` of the picture's height below the picture's middle.
pub fn scene_lowered(atlas: &Atlas, lit: &[(&str, u32)], aspect: f32, view: &View, lower: f32) -> Scene {
    layout(atlas, lit, aspect, lower).scene(view)
}

/// A city laid out, before any camera: its boxes (none cut down), its fitted framing, and how
/// low a lit building may be cut. A camera move needs none of it again: `Layout::scene`.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub blocks: Vec<Block>,
    pub layers: Vec<Layer>,
    fit: Option<Fit>,
    aspect: f32,
    lit_floor: f32,
}

impl Layout {
    /// The city seen from `view`: its boxes as laid out, cut down only for the fitted view, lifted
    /// or not (a moved one cuts nothing: `clear_sight`). Nothing is laid out again.
    pub fn scene(&self, view: &View) -> Scene {
        let mut blocks = self.blocks.clone();
        let layers = self.layers.clone();
        let Some(fit) = self.fit else {
            return Scene {
                blocks,
                camera: None,
                fit: None,
                tints: Vec::new(),
                shown: Vec::new(),
                glow: Vec::new(),
                layers,
            };
        };
        let camera = fit.camera(view, self.aspect);
        // A lift moves no eye: the same lines of sight.
        if (View { lift: 0., ..*view }) == View::default() {
            clear_sight(&mut blocks, camera.eye, self.lit_floor);
        }
        Scene {
            blocks,
            camera: Some(camera),
            fit: Some(fit),
            tints: Vec::new(),
            shown: Vec::new(),
            glow: Vec::new(),
            layers,
        }
    }
}

/// `atlas` laid out as a city for a picture `aspect` wide to 1 tall, its fitted middle `lower`
/// below the picture's (`scene`): what a camera does not change.
pub fn layout(atlas: &Atlas, lit: &[(&str, u32)], aspect: f32, lower: f32) -> Layout {
    let ground = Block { min: [0., -0.01, 0.], max: [WIDE, 0., 1.], kind: Kind::Ground, place: None, district: None };
    let mut blocks = vec![ground];
    // A typical lot's side: e.g. 0.24 for 28 files, 0.018 for 5,000 (where the constants rule).
    let lot = (WIDE / atlas.places.iter().filter(|p| !p.group).count().max(1) as f32).sqrt();
    let (tallest, street) = (TALLEST.max(lot * TOWER), STREET.max(lot * LOT_STREET));
    // A plinth keeps its height beside a few tall towers.
    let lift = PLINTH * (tallest / TALLEST).min(1.5);
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
    let (mut buildings, mut layers) = (Vec::new(), Vec::new());
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
                let plate = lot.shrunk((street * DISTRICT_STREET).min(lot.short() / 4.));
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
        let top = floor + rise(place.height) * tallest;
        let kind = colour.map_or(Kind::Building, Kind::Lit);
        // A building of commits is its stub and a slab a level, the newest lowest, its roof the
        // top slab's; a lit one is whole.
        let mut top = top;
        if kind == Kind::Building && !place.strata.is_empty() {
            let at = |level: f32| floor + (STUB + level * LEVEL) * tallest;
            layers.push(Layer { block: buildings.len(), y: [floor, at(0.)], colour: None });
            for (level, colour) in place.strata.iter().enumerate() {
                let y = [at(level as f32), at(level as f32) + SLAB * tallest];
                layers.push(Layer { block: buildings.len(), y, colour: Some(*colour) });
                top = y[1];
            }
        }
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
    for layer in &mut layers {
        layer.block += blocks.len();
    }
    blocks.extend(buildings);
    let fit = frame(&blocks, aspect, lower);
    Layout { blocks, layers, fit, aspect, lit_floor: highest + LIT_ABOVE }
}

impl Camera {
    /// The ray from the eye through a point of the picture (shares of it, 0,0 top left): its
    /// direction, not of unit length.
    fn ray(&self, x: f32, y: f32) -> [f32; 3] {
        let (r, u, f) = self.axes();
        let tan = (self.fov / 2.).tan();
        let (dx, dy) = ((2. * x - 1.) * tan * self.aspect, (1. - 2. * y + 2. * self.shift) * tan);
        std::array::from_fn(|i| f[i] + r[i] * dx + u[i] * dy)
    }

    /// The point of the ground (y = 0) under a point of the picture, as plan x and z; none above
    /// the horizon.
    pub fn ground(&self, x: f32, y: f32) -> Option<[f32; 2]> {
        let d = self.ray(x, y);
        let t = -self.eye[1] / d[1];
        (d[1] < 0. && t.is_finite()).then(|| [self.eye[0] + d[0] * t, self.eye[2] + d[2] * t])
    }
}

impl Scene {
    /// Whether block `i` is drawn (`shown`; all when it is empty).
    pub fn is_shown(&self, i: usize) -> bool {
        self.shown.get(i).is_none_or(|shown| *shown)
    }

    /// The colour block `i` glows in, closed over a lit file (`glow`), if it does.
    pub fn glows(&self, i: usize) -> Option<u32> {
        self.glow.get(i).copied().flatten()
    }

    /// The box under a point of the picture (shares of it, 0,0 top left), by its index: the
    /// nearest building or district the camera's ray through it meets. Never the ground.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let camera = self.camera?;
        let d = camera.ray(x, y);
        let to: [f32; 3] = std::array::from_fn(|i| camera.eye[i] + d[i] * camera.far);
        let hits = self.blocks.iter().enumerate().filter(|(i, b)| b.kind != Kind::Ground && self.is_shown(*i));
        let near = hits.filter_map(|(i, b)| Some((b.crossed(camera.eye, to)?.0, i)));
        near.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, i)| i)
    }

    /// Box `i`'s beam: from the middle of its top straight up `beam_height` (vertical in the
    /// world, so under any camera), its end lowered to stay `MARGIN` inside the picture's top.
    /// Never the ground's.
    pub fn beam(&self, i: usize) -> Option<([f32; 3], [f32; 3])> {
        let (camera, block) = (self.camera?, self.blocks.get(i).filter(|b| b.kind != Kind::Ground)?);
        let foot = block.top();
        let up = |h: f32| [foot[0], foot[1] + h, foot[2]];
        let inside = |h: f32| camera.project(up(h)).is_some_and(|(_, y)| y >= MARGIN);
        // Long enough to see: at least `BEAM_SHARE` of the picture (a small city's was a stub).
        let (_, foot_y) = camera.project(foot)?;
        let short = |h: f32| camera.project(up(h)).is_none_or(|(_, y)| foot_y - y < BEAM_SHARE);
        let mut beam = self.beam_height();
        if short(beam) && inside(beam) {
            let (mut low, mut high) = (beam, beam * 16.);
            for _ in 0..24 {
                let mid = (low + high) / 2.;
                if short(mid) && inside(mid) { low = mid } else { high = mid }
            }
            beam = high;
        }
        Some((foot, up(highest_inside(&inside, beam))))
    }

    /// Each agent's mark, given the box it is at (none: no file known, or a file not in the
    /// city). Over a box: at its beam's end (`beam`), where a lit building's beam has its dot.
    /// At none (most of an agent's work is shell lines): hovering over the buildings, above the
    /// point the camera looks at (the city's middle, moved by a pan), as high as a beam rises. Several at one place are fanned in the order given.
    pub fn marks(&self, at: &[Option<usize>]) -> Vec<Mark> {
        let over: Vec<Option<usize>> = at.iter().map(|i| i.filter(|i| self.beam(*i).is_some())).collect();
        let hover = self.camera.filter(|_| over.contains(&None)).map(|camera| {
            let m = camera.target;
            let up = |h: f32| [m[0], m[1] + h, m[2]];
            let inside = |h: f32| camera.project(up(h)).is_some_and(|(_, y)| y >= MARGIN);
            up(highest_inside(&inside, self.beam_height()))
        });
        (over.iter().enumerate())
            .map(|(n, here)| {
                let point = here.and_then(|i| self.beam(i)).map(|(_, end)| end).or(hover).unwrap_or_default();
                let k = over[..n].iter().filter(|other| *other == here).count();
                Mark { over: *here, at: point, fan: (k, over.iter().filter(|other| *other == here).count()) }
            })
            .collect()
    }

    /// Box `i`'s top face on screen (shares of the picture), its corners in turn; none behind
    /// the camera or for the ground.
    pub fn outline(&self, i: usize) -> Option<[(f32, f32); 4]> {
        let (camera, b) = (self.camera?, self.blocks.get(i).filter(|b| b.kind != Kind::Ground)?);
        let y = b.max[1];
        let corners =
            [[b.min[0], y, b.min[2]], [b.max[0], y, b.min[2]], [b.max[0], y, b.max[2]], [b.min[0], y, b.max[2]]];
        let [a, b, c, d] = corners.map(|p| camera.project(p));
        Some([a?, b?, c?, d?])
    }

    /// The top edge of box `i`'s side that faces the camera most, on screen (shares of the
    /// picture), its left end first: where a terrace's name is written.
    pub fn front_edge(&self, i: usize) -> Option<[(f32, f32); 2]> {
        let (camera, b) = (self.camera?, self.blocks.get(i).filter(|b| b.kind != Kind::Ground)?);
        let (y, mid) = (b.max[1], b.top());
        // Each side: its outward direction and its top edge's ends.
        let sides = [
            ([0., 0., -1.], [b.min[0], y, b.min[2]], [b.max[0], y, b.min[2]]),
            ([0., 0., 1.], [b.min[0], y, b.max[2]], [b.max[0], y, b.max[2]]),
            ([-1., 0., 0.], [b.min[0], y, b.min[2]], [b.min[0], y, b.max[2]]),
            ([1., 0., 0.], [b.max[0], y, b.min[2]], [b.max[0], y, b.max[2]]),
        ];
        let facing = |n: [f32; 3]| dot(n, unit(sub(camera.eye, mid)));
        let (_, p, q) = sides.into_iter().max_by(|a, b| facing(a.0).total_cmp(&facing(b.0)))?;
        let (p, q) = (camera.project(p)?, camera.project(q)?);
        Some(if p.0 <= q.0 { [p, q] } else { [q, p] })
    }

    /// How high a beam rises: the city's highest top, at least `BEAM`.
    fn beam_height(&self) -> f32 {
        self.blocks.iter().map(|b| b.max[1]).fold(BEAM, f32::max)
    }
}

/// `h` if `inside(h)`, else the highest rise below it still inside (none: 0).
fn highest_inside(inside: &dyn Fn(f32) -> bool, h: f32) -> f32 {
    if inside(h) {
        return h;
    }
    let (mut low, mut high) = (0f32, h);
    for _ in 0..24 {
        let mid = (low + high) / 2.;
        if inside(mid) { low = mid } else { high = mid }
    }
    low
}

/// The share of a city's rise by which its last district's buildings start: the largest
/// district rises first, the others after it by size, the last this far in.
pub const STAGGER: f32 = 0.35;

/// How tall a building stands at share `t` of the city's rise, starting at `delay`, as a share
/// of its height: 0 to 1, eased out (cubic). The renderer's shader computes the same.
pub fn risen(t: f32, delay: f32) -> f32 {
    let at = ((t - delay) / (1. - STAGGER)).clamp(0., 1.);
    1. - (1. - at).powi(3)
}

impl Scene {
    /// When each box starts to rise, a share of the rise: buildings by their top-level
    /// district, the one with most buildings first (0), the one with fewest last (`STAGGER`),
    /// those on no district with the first; ground and districts never rise (-1: they are
    /// there from the start, the footprints the buildings grow on).
    pub fn rise_delays(&self) -> Vec<f32> {
        let mut sizes: Vec<(usize, usize)> = Vec::new();
        let rising = self.blocks.iter().filter(|b| matches!(b.kind, Kind::Building | Kind::Lit(_)));
        for d in rising.filter_map(|b| b.district) {
            match sizes.iter_mut().find(|(of, _)| *of == d) {
                Some((_, n)) => *n += 1,
                None => sizes.push((d, 1)),
            }
        }
        // Most buildings first; ties by number, so the order never depends on hashing.
        sizes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let last = sizes.len().saturating_sub(1).max(1) as f32;
        let rank = |d: usize| sizes.iter().position(|(of, _)| *of == d).unwrap_or(0) as f32;
        (self.blocks.iter())
            .map(|b| match b.kind {
                Kind::Building | Kind::Lit(_) => b.district.map_or(0., |d| rank(d) / last * STAGGER),
                _ => -1.,
            })
            .collect()
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
    use crate::atlas::{atlas, repo_with};

    fn city(files: &[(&str, u64)]) -> Atlas {
        let files: Vec<(String, u64)> = files.iter().map(|(p, b)| (p.to_string(), *b)).collect();
        plan(&files)
    }

    /// The plan of `files`, each with its number of lines and a commit for each thousand of them.
    fn plan(files: &[(String, u64)]) -> Atlas {
        let level = crate::strata::Level { at: 0, change: None };
        let levels = |n: &u64| vec![level; (n / 1000).min(200) as usize];
        let files_of = files.iter().map(|(path, n)| (path.clone(), levels(n))).collect();
        atlas(&repo_with(files, &crate::strata::History { files: files_of, span: (0, 0) }))
    }

    /// A repository with a tall file in front of a small one, as the camera sees them.
    fn many() -> Atlas {
        let mut files: Vec<(String, u64)> = Vec::new();
        for d in 0..6 {
            for f in 0..30 {
                files.push((format!("d{d}/s{}/f{f}.rs", f % 4), 200 + ((d * 31 + f * 17) % 50) as u64 * 900));
            }
        }
        plan(&files)
    }

    /// As the owner's strata: a stub, and a level a commit on it, up to 120.
    #[test]
    fn a_file_is_as_tall_as_its_commits_stack_in_the_owners_city() {
        assert_eq!((height(0), height(60), height(120), height(u32::MAX)), (0., 0.5, 1., 1.));
        assert_eq!((rise(None), rise(Some(0.))), (0.4 / 60., 0.4 / 60.));
        assert!((rise(Some(height(1))) - 2. / 60.).abs() < 1e-7);
        assert!((rise(Some(height(120))) - (0.4 + 120. * 1.6) / 60.).abs() < 1e-5);
    }

    /// A building of commits is drawn as its stub in its own colour and a slab a commit, the
    /// newest lowest, a gap between them, all inside its box; a lit one, and one of no commit,
    /// as its box.
    #[test]
    fn a_building_of_commits_is_drawn_as_its_strata() {
        let log = "\x012\tfeat: a\0\nsrc/a.rs\0\x011\tfix: b\0\nsrc/a.rs\0src/b.rs\0";
        let files: Vec<(String, u64)> = ["src/a.rs", "src/b.rs", "src/c.rs"].map(|p| (p.to_string(), 9)).into();
        let plan = atlas(&crate::atlas::repo_with(&files, &crate::strata::history(log)));
        let s = scene(&plan, &[("src/b.rs", 7)], 1.6, &View::default());
        let block = |id: &str| s.blocks.iter().position(|b| b.place.is_some_and(|p| plan.places[p].id == id)).unwrap();
        let of = |i: usize| s.layers.iter().filter(|l| l.block == i).copied().collect::<Vec<_>>();
        let (a, b, c) = (block("src/a.rs"), block("src/b.rs"), block("src/c.rs"));
        let layers = of(a);
        let colours: Vec<Option<u32>> = layers.iter().map(|l| l.colour).collect();
        let (feat, fix) = (crate::strata::Change::Feat, crate::strata::Change::Fix);
        let want = [None, Some(crate::strata::colour(Some(feat), 1.)), Some(crate::strata::colour(Some(fix), 0.))];
        assert_eq!(colours, want);
        let (bottom, top) = (s.blocks[a].min[1], s.blocks[a].max[1]);
        assert_eq!(layers[0].y[0], bottom);
        for pair in layers.windows(2) {
            assert!(pair[1].y[0] >= pair[0].y[1] && pair[0].y[1] > pair[0].y[0], "{pair:?}");
        }
        assert_eq!(layers[2].y[1], top, "its roof is its top slab's");
        assert!(of(b).is_empty() && of(c).is_empty(), "lit and no commits: whole");
    }

    /// A city of few files has large lots: its buildings are towers on streets, not slabs. A
    /// large one keeps the owner's heights.
    #[test]
    fn a_city_of_few_files_stands_as_towers_on_streets() {
        let files: Vec<(String, u64)> = (0..12).map(|i| (format!("d{}/f{i}.rs", i % 3), 40_000)).collect();
        let s = scene(&plan(&files), &[], 1.6, &View::default());
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
        let s = scene(&plan(&big), &[], 1.6, &View::default());
        let tallest = s.blocks.iter().filter(|b| b.kind == Kind::Building).map(|b| b.max[1] - b.min[1]);
        // 5,000 files of 120 levels are more than a city draws: 20 each, the roof the top slab's.
        assert!((tallest.fold(0., f32::max) - TALLEST * (STUB + 19. * LEVEL + SLAB)).abs() < 1e-5);
    }

    #[test]
    fn districts_step_up_and_buildings_stand_on_their_district() {
        let s = scene(
            &city(&[("src/ui/a.rs", 900), ("src/ui/d.rs", 900), ("src/b.rs", 900), ("c.rs", 900)]),
            &[],
            1.6,
            &View::default(),
        );
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
            trees.push(plan(&files));
        }
        for atlas in &trees {
            let s = scene(atlas, &[], 1.6, &View::default());
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
        let atlas = plan(&files);
        let s = scene(&atlas, &[], 1.6, &View::default());
        assert!(s.blocks.iter().filter(|b| matches!(b.kind, Kind::Plate(_))).count() > 3);
        for b in &s.blocks {
            assert!(b.max[0] > b.min[0] && b.max[2] > b.min[2], "{b:?}");
        }
    }

    /// A folder with one child is no district: `a` and `a/b` hold only `a/b/c`, which is the
    /// first tier. Its districts are a tier apart by the owner's lifts.
    #[test]
    fn a_folder_of_one_child_is_no_district() {
        let atlas = city(&[("a/b/c/x.rs", 900), ("a/b/c/y.rs", 900), ("a/b/c/z/w.rs", 900), ("a/b/c/z/v.rs", 900)]);
        let s = scene(&atlas, &[], 1.6, &View::default());
        assert!(plate(&s, &atlas, "a").is_none() && plate(&s, &atlas, "a/b").is_none());
        let (c, z) = (plate(&s, &atlas, "a/b/c").unwrap(), plate(&s, &atlas, "a/b/c/z").unwrap());
        assert_eq!((c.kind, z.kind), (Kind::Plate(1), Kind::Plate(2)));
        // Plinths: each tier 3 to 4.5 times the owner's lift (more beside this city's tall towers).
        let lift = c.max[1] / terrace(1, 1.);
        assert!((PLINTH..=PLINTH * 1.5).contains(&lift), "{lift}");
        assert_eq!(c.min[1], 0.);
        assert!((z.max[1] - terrace(2, lift)).abs() < 1e-6, "{} {}", z.max[1], terrace(2, lift));
    }

    /// The ray through a point of the picture meets what is drawn there: a building's top, a
    /// district's kerb, nothing off the city.
    #[test]
    fn a_point_of_the_picture_hits_what_is_drawn_there() {
        let atlas = many();
        for aspect in [0.7, 1.6, 3.] {
            let s = scene(&atlas, &[], aspect, &View::default());
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
        let s = scene(&atlas, &[], 1.6, &View::default());
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
            let s = scene(&atlas, &[], aspect, &View::default());
            let camera = s.camera.unwrap();
            let tall = s.blocks.iter().map(|b| b.max[1]).fold(BEAM, f32::max);
            let mut full = 0;
            for (i, b) in s.blocks.iter().enumerate().skip(1) {
                let (foot, end) = s.beam(i).unwrap();
                assert_eq!(foot, b.top());
                assert_eq!((end[0], end[2]), (foot[0], foot[2]), "vertical");
                assert!(end[1] >= foot[1] && end[1] <= foot[1] + 16. * tall + 1e-6, "{foot:?} {end:?}");
                let (_, y) = camera.project(end).unwrap();
                assert!(y >= MARGIN - 1e-4, "{aspect}: {y}");
                let foot_y = camera.project(foot).unwrap().1;
                assert!(y < foot_y || end[1] == foot[1], "up in the picture");
                // A share of the picture tall, unless the picture's top cuts it.
                assert!(foot_y - y >= BEAM_SHARE - 1e-3 || y < MARGIN + 1e-3, "{aspect}: {foot_y} to {y}");
                full += usize::from(end[1] >= foot[1] + tall - 1e-6);
            }
            assert!(full > 0, "{aspect}: some beam has its whole height");
        }
        let few = city(&[("a.rs", 90_000), ("b.rs", 90_000), ("c/d.rs", 9_000), ("c/e.rs", 9_000)]);
        let s = scene(&few, &[], 1.6, &View::default());
        let top = s.blocks.iter().map(|b| b.max[1]).fold(0., f32::max);
        assert!(top > BEAM, "towers taller than the least beam: {top}");
        let (foot, end) = s.beam(s.blocks.len() - 1).unwrap();
        assert!(end[1] - foot[1] > BEAM, "{foot:?} {end:?}");
        assert_eq!(scene(&atlas, &[], 1.6, &View::default()).beam(0), None, "the ground has none");
    }

    /// A district's outline is its top face on screen, and its front edge is the one of those
    /// four edges nearest the camera: lower in the picture than the edge behind it.
    #[test]
    fn a_districts_outline_and_front_edge_are_on_its_top_face() {
        let atlas = many();
        for yaw in [0., 34., 120., 250.] {
            let view = View { yaw, ..View::default() };
            let s = scene(&atlas, &[], 1.6, &view);
            let camera = s.camera.unwrap();
            let (i, b) = s.blocks.iter().enumerate().find(|(_, b)| b.kind == Kind::Plate(1)).unwrap();
            let outline = s.outline(i).unwrap();
            let [x, y, w, h] = camera.bounds(b).unwrap();
            for (px, py) in outline {
                assert!(px >= x - 1e-4 && px <= x + w + 1e-4 && py >= y - 1e-4 && py <= y + h + 1e-4, "{yaw}");
            }
            let [l, r] = s.front_edge(i).unwrap();
            assert!(l.0 <= r.0 && [l, r].iter().all(|p| outline.contains(p)), "{yaw}: {l:?} {r:?} {outline:?}");
            // The far edge: the two corners that are not on the front one.
            let far: Vec<_> = outline.iter().filter(|p| **p != l && **p != r).collect();
            assert!((l.1 + r.1) / 2. > (far[0].1 + far[1].1) / 2., "{yaw}: front {l:?} {r:?}, far {far:?}");
        }
        assert_eq!(scene(&atlas, &[], 1.6, &View::default()).outline(0), None, "the ground has none");
    }

    /// A city rises district by district, the one with most buildings first and the one with
    /// fewest last; ground and districts stand from the start; a building grows from nothing to
    /// its height, eased.
    #[test]
    fn the_largest_district_rises_first_on_standing_footprints() {
        let mut files: Vec<(String, u64)> = (0..9).map(|i| (format!("big/f{i}.rs"), 900)).collect();
        files.extend((0..4).map(|i| (format!("mid/f{i}.rs"), 900)));
        files.extend([("small/a.rs".to_string(), 900), ("small/b.rs".to_string(), 900), ("root.rs".to_string(), 9)]);
        let atlas = plan(&files);
        let s = scene(&atlas, &[], 1.6, &View::default());
        let delays = s.rise_delays();
        let of = |dir: &str| {
            let at = |b: &Block| b.place.is_some_and(|p| atlas.places[p].id.starts_with(dir) && !atlas.places[p].group);
            s.blocks.iter().zip(&delays).filter(|(b, _)| at(b)).map(|(_, d)| *d).collect::<Vec<_>>()
        };
        assert!(of("big/").iter().all(|d| *d == 0.), "{:?}", of("big/"));
        assert!(of("mid/").iter().all(|d| *d > 0. && *d < STAGGER));
        assert!(of("small/").iter().all(|d| *d == STAGGER));
        assert_eq!(of("root.rs"), [0.]);
        for (b, d) in s.blocks.iter().zip(&delays) {
            assert_eq!(*d < 0., matches!(b.kind, Kind::Ground | Kind::Plate(_)), "{b:?}");
        }
        assert_eq!((risen(0., 0.), risen(1., STAGGER), risen(STAGGER, STAGGER)), (0., 1., 0.));
        assert!(risen(0.3, 0.) > 0.3, "eased out: fast first");
    }

    /// An agent at a box floats at its beam's end; two at one box share it, fanned; one at
    /// none floats over the ground's leftmost corner in the picture, inside it.
    #[test]
    fn an_agent_floats_at_its_boxes_beam_and_one_at_none_hovers_over_the_city() {
        let s = scene(&many(), &[], 1.6, &View::default());
        let (a, b) = (s.blocks.len() - 1, s.blocks.len() - 5);
        let marks = s.marks(&[Some(a), None, Some(b), Some(a), None, Some(0)]);
        let end = |i: usize| s.beam(i).unwrap().1;
        assert_eq!((marks[0].over, marks[0].at, marks[0].fan), (Some(a), end(a), (0, 2)));
        assert_eq!((marks[3].over, marks[3].at, marks[3].fan), (Some(a), end(a), (1, 2)));
        assert_eq!((marks[2].at, marks[2].fan), (end(b), (0, 1)));
        // At none, and at the ground (no box of a file): hovering, three of them.
        let hover = marks[1].at;
        for (i, k) in [(1, 0), (4, 1), (5, 2)] {
            assert_eq!((marks[i].over, marks[i].at, marks[i].fan), (None, hover, (k, 3)), "{i}");
        }
        // Over the city's middle, as high as a beam, inside the picture.
        let camera = s.camera.unwrap();
        let (x, y) = camera.project(hover).unwrap();
        let (_, middle_y) = camera.project(camera.target).unwrap();
        assert!((x - 0.5).abs() < 0.01 && (MARGIN..middle_y).contains(&y), "{x} {y} {middle_y}");
        assert_eq!((hover[0], hover[2]), (camera.target[0], camera.target[2]));
    }

    #[test]
    fn a_hovering_agent_stays_in_the_picture_under_any_camera() {
        let lifts = [-LIFT_MAX, LIFT_MAX].map(|lift| View { lift, ..View::default() });
        let views =
            [View { zoom: 0.3, ..View::default() }, View { pitch: 80., ..View::default() }].into_iter().chain(lifts);
        let panned = View { pan: [3., -2.], yaw: 70., lift: 0.3, ..View::default() };
        for view in [View::default(), panned].into_iter().chain(views) {
            let s = scene(&many(), &[], 1.6, &view);
            let (x, y) = s.camera.unwrap().project(s.marks(&[None])[0].at).unwrap();
            assert!((x - 0.5).abs() < 0.01 && (MARGIN - 1e-3..=1.).contains(&y), "{view:?}: {x} {y}");
        }
    }

    #[test]
    fn every_box_is_in_the_picture_at_any_shape() {
        let atlas = many();
        for aspect in [0.02, 0.5, 1., 1.6, 3., 40.] {
            let s = scene(&atlas, &[], aspect, &View::default());
            let camera = s.camera.unwrap();
            for block in &s.blocks {
                let [x, y, w, h] = camera.bounds(block).unwrap();
                assert!(x >= 0. && y >= 0. && x + w <= 1. && y + h <= 1., "{aspect}: {block:?}");
            }
        }
    }

    #[test]
    fn the_projection_agrees_with_the_camera() {
        let s = scene(&many(), &[], 1.6, &View::default());
        let camera = s.camera.unwrap();
        // The target is `LOWER` below the picture's centre; a point nearer the camera is lower in it.
        let (x, y) = camera.project(camera.target).unwrap();
        assert!((x - 0.5).abs() < 1e-4 && (y - 0.5 - LOWER).abs() < 1e-4, "{x} {y}");
        let near: [f32; 3] = std::array::from_fn(|i| camera.target[i] + (camera.eye[i] - camera.target[i]) * 0.1);
        let near = [near[0], 0., near[2]];
        assert!(camera.project(near).unwrap().1 > 0.5);
        assert!(camera.project(sub(camera.eye, sub(camera.target, camera.eye))).is_none(), "behind it");
    }

    /// A camera move lays nothing out again: every view of one layout has its boxes, the
    /// fitted one cut down only where they hide a lit building; and it is the scene `scene`
    /// makes for that view.
    #[test]
    fn a_camera_move_rebuilds_no_blocks() {
        let atlas = many();
        let lit_id = atlas.places.iter().rev().find(|p| !p.group).unwrap().id.clone();
        let lit = [(lit_id.as_str(), 0xff0000)];
        let laid = layout(&atlas, &lit, 1.4, LOWER);
        let turned = View::default().orbited(50., -10.).zoomed(0.5, [0.1, 0.]);
        let (a, b) = (laid.scene(&turned), laid.scene(&View::default().panned(0.2, 0.1, 0.)));
        assert_eq!((&a.blocks, &b.blocks), (&laid.blocks, &laid.blocks));
        assert!(a.camera != b.camera);
        assert_eq!(laid.scene(&turned), scene(&atlas, &lit, 1.4, &turned));
        let fitted = laid.scene(&View::default());
        assert_eq!(fitted, scene(&atlas, &lit, 1.4, &View::default()));
        for (cut, whole) in fitted.blocks.iter().zip(&laid.blocks) {
            assert!(cut.max[1] <= whole.max[1] && cut.min == whole.min && cut.kind == whole.kind);
        }
    }

    /// The fitted city sits below the picture's middle and whole, at CI's window and a larger
    /// one, small and large; centred with no shift. The user's view keeps the shift.
    #[test]
    fn the_fitted_city_sits_below_the_middle_and_whole() {
        let small = city(&[("a/x.rs", 900), ("a/y.rs", 4000), ("b/z.rs", 300), ("r.md", 2000)]);
        // CI's 1024×674 window and 1400×900, less the sidebar and bars: about 1.1 and 1.4 wide.
        for (atlas, aspect) in [&small, &many()].into_iter().flat_map(|a| [(a, 1.1), (a, 1.4), (a, 1.6)]) {
            for lower in [0., LOWER, 0.16] {
                let s = scene_lowered(atlas, &[], aspect, &View::default(), lower);
                let camera = s.camera.unwrap();
                let (_, y) = camera.project(camera.target).unwrap();
                assert!((y - 0.5 - lower).abs() < 1e-4, "{aspect} {lower}: {y}");
                for b in &s.blocks {
                    let [x, y, w, h] = camera.bounds(b).unwrap();
                    let inside = |v: f32| (MARGIN - 1e-3..=1. - MARGIN + 1e-3).contains(&v);
                    assert!(inside(x) && inside(y) && inside(x + w) && inside(y + h), "{aspect} {lower}: {b:?}");
                }
            }
        }
        let turned = View::default().orbited(40., -10.);
        let camera = scene(&small, &[], 1.4, &turned).camera.unwrap();
        assert!((camera.project(camera.target).unwrap().1 - 0.5 - LOWER).abs() < 1e-4);
    }

    #[test]
    fn a_lit_building_is_coloured_and_stands_above_every_terrace() {
        let atlas = many();
        let id = atlas.places.iter().find(|p| !p.group && p.depth == 3).unwrap().id.clone();
        let s = scene(&atlas, &[(&id, 0xff6467)], 1.6, &View::default());
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
                    let plain = scene(&atlas, &[], aspect, &View::default());
                    let s = scene(&atlas, &lit, aspect, &View::default());
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
        assert!(scene(&Atlas::default(), &[], 1.6, &View::default()).camera.is_some(), "the ground alone");
        assert!(scene(&many(), &[], 0., &View::default()).camera.is_none());
    }

    /// A view stays in its clamps however far it is turned, moved or zoomed, and a view of no
    /// number is the fitted one's.
    #[test]
    fn a_view_stays_in_its_clamps() {
        let v = View::default().orbited(1000., 500.);
        assert_eq!((v.pitch, (v.yaw - (YAW + 1000.).rem_euclid(360.)).abs() < 1e-3), (PITCH_RANGE.1, true));
        assert_eq!(View::default().orbited(0., -500.).pitch, PITCH_RANGE.0);
        assert_eq!(View::default().zoomed(1e-6, [0., 0.]).zoom, ZOOM_RANGE.0);
        assert_eq!(View::default().zoomed(1e6, [0., 0.]).zoom, ZOOM_RANGE.1);
        assert_eq!(View::default().panned(9., -9., 9.).pan, [WIDE / 2., -0.5]);
        assert_eq!(View::default().panned(0., 0., -9.).lift, -LIFT_MAX);
        let nan = View { yaw: f32::NAN, pitch: f32::INFINITY, zoom: f32::NAN, pan: [f32::NAN, 1.], lift: f32::NAN };
        assert_eq!(nan.clamped(), View { pan: [0., 0.5], ..View::default() });
        for v in [View::default(), v, nan.clamped(), View::default().zoomed(0.3, [0.2, 0.1])] {
            assert!(v.sound(), "{v:?}");
        }
    }

    /// Zooming about a point keeps it where it was in the picture; panning by the ground's
    /// difference under two points moves the city as far.
    #[test]
    fn a_zoom_keeps_its_point_and_a_pan_follows_the_ground() {
        let s = scene(&many(), &[], 1.6, &View::default());
        let fit = s.fit.unwrap();
        let before = fit.camera(&View::default(), 1.6);
        let at = before.ground(0.3, 0.6).unwrap();
        let about = [at[0] - fit.target[0], at[1] - fit.target[2]];
        let near = View::default().zoomed(0.5, about);
        let after = fit.camera(&near, 1.6);
        let (x, y) = after.project([at[0], 0., at[1]]).unwrap();
        assert!((x - 0.3).abs() < 1e-3 && (y - 0.6).abs() < 1e-3, "{x} {y}");
        // The ground under 0.3 moved to 0.5: the city follows the pointer.
        let to = before.ground(0.5, 0.6).unwrap();
        let moved = View::default().panned(at[0] - to[0], at[1] - to[1], 0.);
        let (x, y) = fit.camera(&moved, 1.6).project([at[0], 0., at[1]]).unwrap();
        assert!((x - 0.5).abs() < 1e-3 && (y - 0.6).abs() < 1e-3, "{x} {y}");
    }

    /// A lift slides the whole picture straight up by its share of the height, the perspective
    /// kept; a zoom about a point keeps that point with a lift too, and the ground under the
    /// pointer is the lifted picture's.
    #[test]
    fn a_lift_slides_the_picture_up_and_keeps_zoom_and_ground() {
        let s = scene(&many(), &[], 1.6, &View::default());
        let fit = s.fit.unwrap();
        let lifted = View::default().panned(0., 0., 0.25);
        let (before, after) = (fit.camera(&View::default(), 1.6), fit.camera(&lifted, 1.6));
        for p in [[0.1, 0., 0.2], [1.2, 0.1, 0.8], fit.target] {
            let ((x0, y0), (x1, y1)) = (before.project(p).unwrap(), after.project(p).unwrap());
            assert!((x1 - x0).abs() < 1e-4 && (y0 - y1 - 0.25).abs() < 1e-4, "{p:?}: {y0} {y1}");
        }
        let at = after.ground(0.3, 0.4).unwrap();
        let (x, y) = after.project([at[0], 0., at[1]]).unwrap();
        assert!((x - 0.3).abs() < 1e-3 && (y - 0.4).abs() < 1e-3, "{x} {y}");
        let near = lifted.zoomed(0.5, [at[0] - fit.target[0], at[1] - fit.target[2]]);
        let (x, y) = fit.camera(&near, 1.6).project([at[0], 0., at[1]]).unwrap();
        assert!((x - 0.3).abs() < 1e-3 && (y - 0.4).abs() < 1e-3, "{x} {y}");
    }

    /// At either lift, with the city framed high or low (`developer.city_frame`), the point looked
    /// at stays in the picture.
    #[test]
    fn a_lift_keeps_the_point_looked_at_in_the_picture() {
        let fit = scene(&many(), &[], 1.6, &View::default()).fit.unwrap();
        for (lower, lift) in [(-0.2, LIFT_MAX), (0.3, -LIFT_MAX), (LOWER, LIFT_MAX), (LOWER, -LIFT_MAX)] {
            let camera = Fit { lower, ..fit }.camera(&View { lift, ..View::default() }, 1.6);
            let (_, y) = camera.project(camera.target).unwrap();
            assert!((MARGIN - 1e-4..=1. - MARGIN + 1e-4).contains(&y), "{lower} {lift}: {y}");
        }
    }

    /// The hit test holds from any view; a view the user moved cuts no building down.
    #[test]
    fn a_turned_view_hits_what_it_draws_and_cuts_nothing() {
        let atlas = many();
        let id = atlas.places.iter().find(|p| !p.group && p.depth == 3).unwrap().id.clone();
        let turned = View::default().orbited(120., 15.).zoomed(0.4, [0.1, 0.]);
        let s = scene(&atlas, &[(&id, 1)], 1.6, &turned);
        let camera = s.camera.unwrap();
        let mut hit = 0;
        for b in s.blocks.iter().filter(|b| b.kind == Kind::Building) {
            let Some((x, y)) = camera.project(b.top()).filter(|(x, y)| (0.0..1.).contains(x) && (0.0..1.).contains(y))
            else {
                continue;
            };
            hit += usize::from(s.hit(x, y).map(|i| &s.blocks[i]) == Some(b));
        }
        assert!(hit > 10, "{hit}");
        let plain = scene(&atlas, &[], 1.6, &turned);
        let cut = plain.blocks.iter().zip(&s.blocks).filter(|(a, b)| b.kind == Kind::Building && a.max[1] > b.max[1]);
        assert_eq!(cut.count(), 0);
        // A lift alone moves no eye: what the fitted view cuts for a lit file, it cuts.
        let (fitted, lifted) = (View::default(), View { lift: 0.3, ..View::default() });
        let cuts = |id: &str| {
            let fitted = scene(&atlas, &[(id, 1)], 1.6, &fitted).blocks;
            // Moved a hair, it cuts nothing: any difference is the fitted view's cut.
            let moved = View { pan: [1e-4, 0.], ..View::default() };
            (fitted != scene(&atlas, &[(id, 1)], 1.6, &moved).blocks).then_some(fitted)
        };
        let (id, cut) = (atlas.places.iter().filter(|p| !p.group))
            .find_map(|p| Some((p.id.clone(), cuts(&p.id)?)))
            .expect("a lit file the fitted view cuts for");
        assert_eq!(scene(&atlas, &[(&id, 1)], 1.6, &lifted).blocks, cut);
    }
}
