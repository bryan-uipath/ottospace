//! The code city behind an empty place (flag `code_city`): an empty tab or pane draws the
//! checkout it is of (`Model::city_at`). Each folder on show is read once, off the main thread;
//! each picture (a folder, its lit files, a size) is rendered once off it (`city_gpu`), and every
//! empty place of that folder and size paints it. Where each file stands and the camera are the
//! core's (`ottospace_core::atlas`, `ottospace_core::scene`). Design: `docs/ai/code-city/plan.md`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Bounds, Context, Corners, Hsla, PathBuilder, Pixels, Point, RenderImage, TextRun, Window, WindowId,
    canvas, div, fill, point, prelude::*, px, rgb,
};
use ottospace_core::atlas::{self, Atlas};
use ottospace_core::empty::{self, Backdrop};
use ottospace_core::model::Origin;
use ottospace_core::model::city::{At, Who};
use ottospace_core::scene::{self, Kind, Scene};
use ottospace_core::tasks::{Group, TaskId};
use ottospace_core::{status_bar, theme};

use crate::city_gpu::{Gpu, Picture};
use crate::home::Floor;
use crate::{Workspace, rect};

/// How long `git ls-files` may take, how much it may say, and how many files a city has. Past
/// any of them there is no city: half of one would put files where the whole one does not.
const TIMEOUT: Duration = Duration::from_secs(10);
const MOST_BYTES: u64 = 4 << 20;
const MOST_FILES: usize = 20_000;
/// Cities kept, by folder: a task's tab shown again draws at once.
const KEPT: usize = 8;
/// Names of districts on their plates, at most, and their tier at most (2: a folder of the root
/// and the districts on it).
const MOST_NAMES: usize = 24;
const NAMED_TIERS: usize = 2;
/// A beam's label sits this many points beside its dot; the hovered beam fades in over this long.
const BESIDE: f32 = 8.;
const FADE: Duration = Duration::from_millis(120);
/// A place's first city, after its lattice waited for it, rises over this long (its buildings
/// grow, district by district); where a frame of the rise renders slower than `RISE_BUDGET_US`,
/// its standing picture fades in over `FADE_IN` instead.
const RISE: Duration = Duration::from_millis(400);
const RISE_BUDGET_US: u64 = 12_000;
const FADE_IN: Duration = Duration::from_millis(250);
/// Pictures no place shows, kept so a tab come back shows its city at once: the newest this many.
const KEPT_PICTURES: usize = 2;
/// An agent's icon's side and the gap between icons fanned side by side, in points; an icon
/// moving to another file glides over this long.
const ICON: f32 = 22.;
const FAN_GAP: f32 = 4.;
const GLIDE: Duration = Duration::from_millis(280);
/// Room kept between a name and the empty tab's hints, in points.
const CLEAR: f32 = 12.;
/// The hints' backing fades out over this many points, in this many layers: no edge to see.
const FEATHER: f32 = 40.;
const LAYERS: usize = 8;

/// What a folder's read was asked with: the folders git may run in, and the files lit then,
/// each with whether its call ran (a file an agent wrote may be new, once the call ends).
type Asked = (Vec<String>, Vec<(String, bool)>);

/// A thread of its own that answers each job with `work`, in order: of the jobs waiting, only
/// the last of each `key` (a newer ask for the same thing replaces the older), the others answered
/// `None`. Each job comes back with its answer. Wakes the app for each.
fn latest_each<J, K, R>(
    key: fn(&J) -> K,
    mut work: impl FnMut(&J) -> R + Send + 'static,
) -> (Sender<J>, Receiver<(J, Option<R>)>)
where
    J: Send + 'static,
    K: PartialEq + 'static,
    R: Send + 'static,
{
    let (jobs, inbox) = std::sync::mpsc::channel::<J>();
    let (done, answers) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(job) = inbox.recv() {
            let waiting: Vec<J> = std::iter::once(job).chain(inbox.try_iter()).collect();
            let keys: Vec<K> = waiting.iter().map(key).collect();
            for (i, job) in waiting.into_iter().enumerate() {
                let newer = keys[i + 1..].contains(&keys[i]);
                let answer = (!newer).then(|| work(&job));
                if done.send((job, answer)).is_err() {
                    return;
                }
            }
            ottospace_mac::wake::post();
        }
    });
    (jobs, answers)
}

/// A folder's read: the folder, and the folders git may run in.
type ReadJob = (String, Vec<String>);

/// Reads checkouts' files on a thread of its own: a folder when it comes on show, or the files
/// its agents are at change, once each (`latest_each`). No timer of its own.
pub struct Reader {
    jobs: Sender<ReadJob>,
    reads: Receiver<(ReadJob, Option<(Option<Atlas>, u64)>)>,
    asked: HashMap<String, Asked>,
    held: HashMap<String, Arc<Atlas>>,
    /// How many reads came, and how long the last took (µs): for the state of a test run.
    came: usize,
    read_us: Option<u64>,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, reads) = latest_each(
            |job: &ReadJob| job.0.clone(),
            |(dir, roots): &ReadJob| {
                let started = Instant::now();
                (read(Path::new(dir), roots), started.elapsed().as_micros() as u64)
            },
        );
        Self { jobs, reads, asked: HashMap::new(), held: HashMap::new(), came: 0, read_us: None }
    }

    /// The folders on show, each with what it is asked with: one new, or whose roots or lit
    /// files changed, is read (again). A folder no longer on show is forgotten, and read again
    /// when it comes back.
    fn ask(&mut self, on_show: HashMap<String, Asked>) {
        for (dir, asked) in &on_show {
            if self.asked.get(dir) != Some(asked) {
                let _ = self.jobs.send((dir.clone(), asked.0.clone()));
            }
        }
        self.asked = on_show;
    }

    /// Takes the reads that came. True if one did.
    fn take(&mut self) -> bool {
        let came: Vec<(String, (Option<Atlas>, u64))> =
            self.reads.try_iter().filter_map(|((dir, _), read)| Some((dir, read?))).collect();
        let any = !came.is_empty();
        for (dir, (read, us)) in came {
            (self.came, self.read_us) = (self.came + 1, Some(us));
            // Too many kept: those of folders not on show go, never one on show.
            if self.held.len() >= KEPT && !self.held.contains_key(&dir) {
                self.held.retain(|kept, _| self.asked.contains_key(kept));
            }
            match read {
                // The same plan read again keeps its `Arc`: its picture is not made again.
                Some(atlas) if self.held.get(&dir).is_some_and(|held| **held == atlas) => {}
                Some(atlas) => drop(self.held.insert(dir, Arc::new(atlas))),
                None => drop(self.held.remove(&dir)),
            };
        }
        any
    }
}

/// The plan of the checkout at `dir` (`checkout_files`). `None`: no city.
fn read(dir: &Path, roots: &[String]) -> Option<Atlas> {
    let (files, _) = checkout_files(dir, roots, MOST_FILES, MOST_BYTES, true)?;
    (!files.is_empty()).then(|| atlas::atlas(&atlas::repo(&files)))
}

/// The files git lists in the checkout at `dir` (tracked, and new ones it does not ignore), each
/// with its size: at most `most`, and true if there were more (or more than `most_bytes` of
/// names); `whole`: a list that was cut is none. Hardened as the status bar's read
/// (`child::git_read`), and only in a folder that, resolved, is one of `roots`
/// (`status_bar::reads_changes`). A link is no file, and nothing under a folder that is a link is
/// one: nothing outside the checkout is looked at. `None`: not such a checkout, or git failed or
/// took too long.
pub(crate) fn checkout_files(
    dir: &Path,
    roots: &[String],
    most: usize,
    most_bytes: u64,
    whole: bool,
) -> Option<(Vec<(String, u64)>, bool)> {
    let top =
        dir.canonicalize().ok().filter(|top| top.to_str().is_some_and(|top| status_bar::reads_changes(top, roots)))?;
    let mut list = crate::child::git_read(&top);
    list.args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
    let late = Instant::now() + TIMEOUT;
    let said = crate::child::run(list, TIMEOUT, most_bytes).ok()?;
    // Cut at `most_bytes`: git stops on the closed pipe, and the last name may be half of one.
    let full = said.out.len() as u64 >= most_bytes;
    if !said.ok && !full {
        return None;
    }
    let mut names: Vec<&str> = said.out.split('\0').filter(|name| !name.is_empty()).collect();
    if full {
        names.pop();
    }
    let cut = full || names.len() > most;
    if cut && whole {
        return None;
    }
    names.truncate(most);
    // Each folder on the way, asked once: a real folder, not a link to one elsewhere.
    let mut folders: HashMap<&Path, bool> = HashMap::new();
    let real = |folder: &Path| std::fs::symlink_metadata(top.join(folder)).is_ok_and(|at| at.is_dir());
    let mut files: Vec<(String, u64)> = Vec::new();
    for name in names {
        // A slow disk: the limit is the whole read's, not git's alone.
        if Instant::now() > late {
            return None;
        }
        let mut over = Path::new(name).ancestors().skip(1).filter(|folder| !folder.as_os_str().is_empty());
        if !over.all(|folder| *folders.entry(folder).or_insert_with(|| real(folder))) {
            continue;
        }
        if let Some(file) = std::fs::symlink_metadata(top.join(name)).ok().filter(|file| file.is_file()) {
            files.push((name.to_string(), file.len()));
        }
    }
    Some((files, cut))
}

/// A picture asked for: the folder, its plan, the lit files (place ids, colours), the size in
/// pixels and whether its buildings are glass. Empty places with one key share one picture.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    dir: String,
    plan: usize,
    lit: Vec<(String, u32)>,
    size: (u32, u32),
    glass: bool,
    /// How far through its rise, in thousandths: 1000 standing.
    rise: u16,
}

impl Key {
    /// The same picture standing (not a frame of its rise).
    fn standing(&self) -> Key {
        Key { rise: 1000, ..self.clone() }
    }
}

/// A district's name on its plate: where it starts and how wide the plate's edge is there, in
/// shares of the picture.
struct Name {
    text: String,
    at: (f32, f32),
    room: f32,
}

/// A picture made: where, in shares of it, the lit buildings and the districts' names are; the
/// scene and plan it shows, for what the pointer is over.
struct Made {
    key: Key,
    image: Arc<RenderImage>,
    lit: Vec<([f32; 4], (f32, f32))>,
    names: Vec<Name>,
    scene: Scene,
    plan: Arc<Atlas>,
    places: usize,
    groups: usize,
    render_us: u64,
    /// When a place last painted it: the newest unused are kept (`KEPT_PICTURES`).
    used: Instant,
}

/// One empty place's city, as the last frame drew it.
#[derive(Default)]
struct Place {
    /// The picture it asks for, and the one it painted last (shown, stretched, until the asked
    /// one is made).
    asked: Option<Key>,
    shown: Option<Key>,
    /// Too small for a city (`empty::CITY_LEAST`): its lattice instead. Measured: a frame laid
    /// it out (a folder is read only for a place measured and not too small).
    small: bool,
    measured: bool,
    /// The scale of the window it is in.
    scale: f32,
    /// Of the last paint: where, in window points, in which window, and how many names.
    bounds: Bounds<Pixels>,
    window: Option<WindowId>,
    labels: usize,
    /// The box the user's pointer is over (an index into the scene's blocks), where that pointer
    /// was (found again in a new picture), and its beam as the last paint drew it.
    hovered: Option<usize>,
    pointer: Option<Point<Pixels>>,
    beacon: Option<Beacon>,
    /// The box whose beam fades in, and since when.
    fading: Option<(usize, Instant)>,
    /// The lit buildings' beams the last paint drew: foot and end.
    lit_beams: Vec<[f32; 4]>,
    /// When it was first laid out; how long after that its first picture painted (ms); whether
    /// it painted without one first (its lattice waited); since when its first picture fades in.
    since: Option<Instant>,
    /// The newest picture made for it: painted until the one asked for comes.
    newest: Option<Key>,
    first_ms: Option<u64>,
    waited: bool,
    fade: Option<Instant>,
    /// Since when its first city rises; whether a frame of it was too slow (then it fades).
    rise: Option<Instant>,
    rise_off: bool,
    /// The agents' icons the last paint drew, and the one under the user's pointer (its key).
    icons: Vec<Icon>,
    hovered_agent: Option<String>,
}

/// An agent working in the city, to draw: its task (none: a session no task has) and a key
/// that tells it from the others, its colour, badge letters (a session's: its runner's glyph),
/// title (its hover's) and the place it is at (an atlas id), if any.
#[derive(Clone)]
struct Floating {
    task: Option<TaskId>,
    key: String,
    colour: u32,
    mono: String,
    title: String,
    at: Option<String>,
}

/// An agent's icon as a paint drew it: its centre, where it glides to and since when (window
/// points), its box, whether its cone shines on a building, its place in a fan (`k` of `n`).
#[derive(Clone)]
struct Icon {
    task: Option<TaskId>,
    key: String,
    shown: Point<Pixels>,
    from: Point<Pixels>,
    to: Point<Pixels>,
    since: Instant,
    rect: Bounds<Pixels>,
    cone: bool,
    fan: (usize, usize),
}

/// A hovered box's beam as drawn: whether a file's, whether lit, its foot and end, its label.
#[derive(Clone, Copy)]
struct Beacon {
    file: bool,
    lit: bool,
    beam: [f32; 4],
    label: Bounds<Pixels>,
}

/// The pictures made and what each empty place drew: for the state of a test run, no name or
/// path in it.
#[derive(Default)]
pub struct Drawn {
    pictures: HashMap<Key, Made>,
    /// Sent to the picture's thread and not answered yet: never sent twice.
    sent: HashSet<Key>,
    /// Why a picture could not be made (no GPU, a failed render): never asked for again. Its
    /// places draw the lattice.
    failed: HashMap<Key, String>,
    places: HashMap<Floor, Place>,
    renders: usize,
    paints: usize,
    /// Frames the hovered beam's fade asked for: none under Reduce Motion.
    beam_frames: u64,
    /// Frames a first picture's fade-in asked for: none at rest, none under Reduce Motion.
    fade_frames: u64,
    /// Frames a first city's rise painted: none at rest, none under Reduce Motion.
    rise_frames: u64,
    /// Frames an agent's icon gliding to another file asked for: none at rest or under Reduce Motion.
    agent_frames: u64,
    /// Pictures no place shows: GPUI keeps each in the window's atlas until it is dropped.
    retired: Vec<Arc<RenderImage>>,
}

pub type Shared = Rc<RefCell<Drawn>>;

/// A picture asked for by a place.
type PaintJob = (Floor, Key, Arc<Atlas>);

/// Renders pictures on a thread of its own, the newest each place asked for only (a camera or a
/// size that moved on is skipped: `latest_each`). The GPU is opened at the first and kept; none
/// (or `OTTOSPACE_CITY_NO_GPU` in a test run) and every picture fails.
pub struct Painter {
    jobs: Sender<PaintJob>,
    made: Receiver<(PaintJob, Option<Result<Made, String>>)>,
}

impl Painter {
    pub fn new() -> Self {
        let mut gpu: Option<Result<Gpu, String>> = None;
        // The last scene made, by its key standing: a rise's frames build it once.
        let mut last: Option<(Key, Scene)> = None;
        let (jobs, made) = latest_each(
            |job: &PaintJob| job.0,
            move |(_, key, plan): &PaintJob| {
                let opened = gpu.get_or_insert_with(|| {
                    let refused = std::env::var_os("OTTOSPACE_E2E").and(std::env::var_os("OTTOSPACE_CITY_NO_GPU"));
                    if refused.is_some() { Err("no GPU (test)".into()) } else { Gpu::new() }
                });
                // A panic in the GPU's code fails this picture, and the GPU is opened again next time.
                let picture = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    opened.as_ref().map_err(Clone::clone).and_then(|gpu| make(gpu, key, plan, &mut last))
                }));
                picture.unwrap_or_else(|_| {
                    gpu = None;
                    Err("the render panicked".into())
                })
            },
        );
        Self { jobs, made }
    }
}

/// The scene of `plan` and its picture, on the picture's thread.
fn make(gpu: &Gpu, key: &Key, plan: &Arc<Atlas>, last: &mut Option<(Key, Scene)>) -> Result<Made, String> {
    let started = Instant::now();
    let lit: Vec<(&str, u32)> = key.lit.iter().map(|(id, colour)| (id.as_str(), *colour)).collect();
    let (w, h) = key.size;
    let standing = key.standing();
    let scene = match last.take() {
        Some((was, scene)) if was == standing => scene,
        _ => scene::scene(plan, &lit, w as f32 / h as f32),
    };
    *last = Some((standing, scene.clone()));
    let Picture { width, height, bgra } = gpu.render(&scene, w, h, f32::from(key.rise) / 1000., key.glass)?;
    let camera = scene.camera.ok_or("no camera")?;
    let image = image::RgbaImage::from_raw(width, height, bgra).ok_or("a picture of the wrong size")?;
    let image = Arc::new(RenderImage::new([image::Frame::new(image)]));
    // Each lit building's bounds, and the middle of its top (never hidden).
    let lit = (scene.blocks.iter().filter(|b| matches!(b.kind, Kind::Lit(_))))
        .filter_map(|b| {
            let [x, y, w, h] = camera.bounds(b)?;
            let (tx, ty) = camera.project(b.top())?;
            Some(([x, y, w, h], (tx, ty)))
        })
        .collect();
    // A district is named along the back edge of its plate, from its left end on screen, outer
    // tiers first; a district on another is named from it (`ui` on `src`, not `src/ui`).
    let mut plates: Vec<(usize, usize, &scene::Block)> = (scene.blocks.iter())
        .filter_map(|b| match (b.kind, b.place) {
            (Kind::Plate(tier), Some(place)) if tier <= NAMED_TIERS => Some((tier, place, b)),
            _ => None,
        })
        .collect();
    plates.sort_by_key(|(tier, ..)| *tier);
    let ids: Vec<&str> =
        plates.iter().filter(|(tier, ..)| *tier < NAMED_TIERS).map(|(_, p, _)| plan.places[*p].id.as_str()).collect();
    let names = (plates.iter())
        .filter_map(|(_, place, b)| {
            let id = plan.places[*place].id.as_str();
            let outer =
                ids.iter().filter_map(|outer| id.strip_prefix(outer)?.strip_prefix('/')).min_by_key(|rest| rest.len());
            let one = camera.project([b.min[0], b.max[1], b.max[2]])?;
            let other = camera.project([b.max[0], b.max[1], b.max[2]])?;
            let at = if one.0 <= other.0 { one } else { other };
            Some(Name { text: outer.unwrap_or(id).to_string(), at, room: (other.0 - one.0).abs() })
        })
        .take(MOST_NAMES)
        .collect();
    let groups = plan.places.iter().filter(|p| p.group).count();
    let render_us = started.elapsed().as_micros() as u64;
    let (places, plan, used) = (plan.places.len(), plan.clone(), Instant::now());
    Ok(Made { key: key.clone(), image, lit, names, scene, plan, places, groups, render_us, used })
}

impl Drawn {
    /// The picture `place` paints: the one it asks for once made, else the one it showed, if of
    /// the same folder. None with no GPU, or a place too small.
    fn picture(&self, place: &Place) -> Option<&Made> {
        let dir = &place.asked.as_ref()?.dir;
        // The one asked for, else the newest made for it (a rise's frame, while a newer is on its
        // way), else the one shown.
        let made = [&place.asked, &place.newest, &place.shown].into_iter().flatten();
        let made = made.filter_map(|key| self.pictures.get(key)).find(|made| made.key.dir == *dir);
        made.filter(|_| !place.small && self.failure(place).is_none())
    }

    /// The pictures a place asks for or shows: not those kept for a place to come back.
    fn in_use(&self) -> usize {
        let used = |key: &Key| {
            self.places.values().any(|p| [&p.asked, &p.newest, &p.shown].iter().any(|k| k.as_ref() == Some(key)))
        };
        self.pictures.keys().filter(|key| used(key)).count()
    }

    /// Why `place`'s picture could not be made, if it could not.
    fn failure(&self, place: &Place) -> Option<&String> {
        self.failed.get(&place.asked.as_ref()?.standing())
    }

    /// The box under `at` (window points) in `floor`'s picture, by its index; none over the
    /// hints, off the picture, or with no picture.
    fn under(&self, floor: Floor, at: Point<Pixels>, hints: Bounds<Pixels>) -> Option<usize> {
        let place = self.places.get(&floor)?;
        let (hints, bounds) = (hints.dilate(px(CLEAR)), place.bounds);
        if !bounds.contains(&at) || (hints.size.width > px(0.) && hints.contains(&at)) {
            return None;
        }
        let made = self.picture(place)?;
        let share = |v: Pixels, from: Pixels, size: Pixels| f32::from(v - from) / f32::from(size);
        made.scene.hit(share(at.x, bounds.left(), bounds.size.width), share(at.y, bounds.top(), bounds.size.height))
    }

    /// For the state of a test run: `floor`'s counts, its picture's size, where it and the lit
    /// buildings are, in window points.
    fn snapshot(&self, floor: Floor, reads: usize) -> serde_json::Value {
        let place = self.places.get(&floor);
        let made = place.and_then(|place| self.picture(place));
        let (scale, failed) = (place.map(|place| place.scale), place.and_then(|place| self.failure(place)));
        let [x, y, w, h] = rect(place.map(|place| place.bounds).unwrap_or_default());
        let lit: Vec<[f32; 4]> = (made.iter().flat_map(|made| &made.lit))
            .map(|(r, _)| [x + r[0] * w, y + r[1] * h, r[2] * w, r[3] * h])
            .collect();
        let tops: Vec<[f32; 2]> =
            (made.iter().flat_map(|made| &made.lit)).map(|(_, (tx, ty))| [x + tx * w, y + ty * h]).collect();
        let beacon = place.and_then(|place| place.beacon);
        let hovered = beacon
            .map(|b| serde_json::json!({ "file": b.file, "lit": b.lit, "beam": b.beam, "beacon": rect(b.label) }));
        let lit_beams = place.map(|place| place.lit_beams.clone()).unwrap_or_default();
        let agents: Vec<serde_json::Value> = (place.iter().flat_map(|place| &place.icons))
            .map(|i| {
                let hovered = place.is_some_and(|place| place.hovered_agent.as_ref() == Some(&i.key));
                serde_json::json!({
                    "task": i.task, "rect": rect(i.rect), "cone": i.cone, "fan": [i.fan.0, i.fan.1], "hovered": hovered,
                })
            })
            .collect();
        serde_json::json!({
            "on": on(floor), "places": made.map(|made| made.places), "groups": made.map(|made| made.groups),
            "picture": made.map(|made| made.key.size), "bounds": [x, y, w, h], "scale": scale,
            "lit": lit, "labels": place.map_or(0, |place| place.labels), "failed": failed, "reads": reads,
            "renders": self.renders, "paints": self.paints, "render_us": made.map(|made| made.render_us),
            "hovered": hovered, "lit_tops": tops, "lit_beams": lit_beams, "pictures": self.in_use(),
            "first_ms": place.and_then(|place| place.first_ms),
            "glass": made.map(|made| made.key.glass),
            "agents": agents,
        })
    }
}

impl Workspace {
    /// Where an empty place is drawn.
    fn floor_of(&self, at: At) -> Floor {
        match at {
            At::Pane(id) => Floor::Pane(id),
            At::Tab(id) if self.model.is_out(id) => Floor::Out(id),
            At::Tab(_) => Floor::Main,
        }
    }

    /// Asks for the folders of the empty places on show, on the titles' cadence; takes the
    /// pictures made; forgets the places gone and drops the pictures none shows. Redraws when
    /// a read or a picture came.
    pub(crate) fn read_city(&mut self, cx: &mut Context<Self>) {
        // The flag off and nothing left to forget: nothing to look for, each step.
        let on = self.model.features.on(ottospace_core::features::Feature::CodeCity);
        if !on && self.city_drawn.borrow().places.is_empty() && self.city_drawn.borrow().pictures.is_empty() {
            return;
        }
        let mut roots = None;
        let mut on_show: HashMap<String, (Vec<String>, Vec<(String, bool)>)> = HashMap::new();
        let mut floors = HashSet::new();
        for at in self.model.empty_places() {
            let Some(city) = self.model.city_at(at) else { continue };
            let floor = self.floor_of(at);
            floors.insert(floor);
            // Read only for a place a frame laid out large enough for a city; a place not laid
            // out yet (a tab that became a pane) keeps its folder's read, not asked again.
            let drawn = self.city_drawn.borrow();
            let place = drawn.places.get(&floor).filter(|place| place.measured);
            let kept = place.is_none() && self.city.asked.contains_key(city.dir);
            if !(place.is_some_and(|place| !place.small) || kept) {
                continue;
            }
            drop(drawn);
            let roots = roots.get_or_insert_with(|| self.model.checkout_roots()).clone();
            let lit = &mut on_show.entry(city.dir.to_string()).or_insert_with(|| (roots, Vec::new())).1;
            for one in &city.lit {
                let one = (one.label.to_string(), one.running);
                if !lit.contains(&one) {
                    lit.push(one);
                }
            }
        }
        self.city.ask(on_show);
        let mut redraw = self.city.take();
        let hints = self.home.clone();
        let mut drawn = self.city_drawn.borrow_mut();
        let made: Vec<(PaintJob, Option<Result<Made, String>>)> = self.city_painter.made.try_iter().collect();
        for ((floor, key, _), picture) in made {
            drawn.sent.remove(&key);
            redraw = true;
            // Skipped: a newer picture of that place was asked for.
            let Some(picture) = picture else { continue };
            if let (Ok(_), Some(place)) = (&picture, drawn.places.get_mut(&floor)) {
                // Not a frame of a rise turned off (too slow, or failed): its lattice waits instead.
                if key.rise == 1000 || !place.rise_off {
                    place.newest = Some(key.clone());
                }
            }
            drawn.renders += 1;
            match picture {
                Ok(made) => {
                    if let Some(old) = drawn.pictures.insert(key, made) {
                        drawn.retired.push(old.image);
                    }
                    // Another scene: the box under each pointer, found again in it.
                    let found: Vec<(Floor, Option<usize>)> = (drawn.places.iter())
                        .map(|(floor, place)| {
                            (
                                *floor,
                                place.pointer.and_then(|at| drawn.under(*floor, at, hints.borrow().hints_at(*floor))),
                            )
                        })
                        .collect();
                    for (floor, hovered) in found {
                        if let Some(place) = drawn.places.get_mut(&floor) {
                            place.hovered = hovered;
                        }
                    }
                }
                // A frame of a rise failed: no rise, its standing picture is tried instead.
                Err(_) if key.rise < 1000 => {
                    if let Some(place) = drawn.places.get_mut(&floor) {
                        (place.rise, place.rise_off, place.newest) = (None, true, None);
                    }
                }
                // The standing picture failed: the place says why, and none of its frames is asked again.
                Err(why) => drop(drawn.failed.insert(key, why)),
            }
        }
        // Places gone are forgotten; a picture or failure no place asks for or shows is dropped.
        drawn.places.retain(|floor, _| floors.contains(floor));
        // A place whose standing picture is made needs no older one (two panes split to one size
        // share a picture made for the other: the first's full-size one is not kept). Not in a
        // rise: its frame shows until the next comes.
        let Drawn { places, pictures, .. } = &mut *drawn;
        let made = |k: &Key| k.rise == 1000 && pictures.contains_key(k);
        // One asking for none (too small) shows none.
        for place in places.values_mut().filter(|p| p.asked.as_ref().is_none_or(made)) {
            place.newest = None;
        }
        let used: HashSet<Key> = (drawn.places.values())
            .flat_map(|place| [place.asked.clone(), place.newest.clone(), place.shown.clone()])
            .flatten()
            .collect();
        // A frame of a rise is never kept.
        let mut unused: Vec<(Instant, Key)> = (drawn.pictures.iter())
            .filter(|(key, _)| !used.contains(*key) && key.rise == 1000)
            .map(|(key, made)| (made.used, key.clone()))
            .collect();
        // The newest few stay: a tab shown again paints its city at once, with no fade.
        unused.sort_by(|a, b| b.0.cmp(&a.0));
        let rises: Vec<Key> =
            drawn.pictures.keys().filter(|key| key.rise < 1000 && !used.contains(*key)).cloned().collect();
        for key in unused.into_iter().skip(KEPT_PICTURES).map(|(_, key)| key).chain(rises) {
            if let Some(old) = drawn.pictures.remove(&key) {
                drawn.retired.push(old.image);
            }
        }
        drawn.failed.retain(|key, _| used.iter().any(|used| used.standing() == *key));
        let retired = std::mem::take(&mut drawn.retired);
        drop(drawn);
        for image in retired {
            cx.drop_image(image, None);
        }
        if redraw {
            cx.notify();
        }
    }

    /// The backdrop of an empty place: the city of its folder once its picture is made, with
    /// districts named and the hints backed; `lattice` until then, or with no city, no GPU, or a
    /// pane too small for one (`empty::CITY_LEAST`). Asks for a picture when the plan, the lit
    /// files or the size changed; one of a folder, lit files and size already made, for another
    /// place, is painted at once.
    pub(crate) fn render_city(&self, floor: Floor, at: At, lattice: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let Some(city) = self.model.city_at(at) else { return lattice };
        // Not read yet (or too large): the place is still laid out, so its size is known.
        let plan = self.city.held.get(city.dir).cloned();
        // The checkout's own name: a file in its root is labelled with it.
        let folder =
            Path::new(city.dir).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let lit: Vec<(String, u32)> = (city.lit.iter())
            .flat_map(|lit| {
                let colour = theme::identity(lit.colour);
                let places = plan.iter().flat_map(|plan| plan.labelled(lit.label, &folder));
                places.map(move |place| (place.id.clone(), colour)).collect::<Vec<_>>()
            })
            .collect();
        let badges = self.badges.borrow();
        let floating: Vec<Floating> = (city.agents.iter())
            .filter_map(|agent| {
                // A label two files share (`a/src/main.rs`, `b/src/main.rs`): at no known file.
                let at = agent.at.and_then(|label| {
                    let mut places = plan.as_ref()?.labelled(label, &folder);
                    let one = places.next()?.id.clone();
                    places.next().is_none().then_some(one)
                });
                Some(match agent.who {
                    Who::Task(task) => {
                        let subject = ottospace_core::talk::Subject::Task(task);
                        let mono = badges.iter().find(|(of, _)| *of == subject)?.1.clone();
                        let title = self.model.tasks.live(task)?.title.clone();
                        let (key, colour) = (format!("task:{task}"), theme::identity(agent.colour));
                        Floating { task: Some(task), key, colour, mono, title, at }
                    }
                    // A session no task has: its runner's glyph, in the colour of work going on.
                    Who::Session { runner, id, title } => {
                        let spec = ottospace_core::runner::RunnerId::named(runner)?.spec();
                        let title = title.unwrap_or(spec.display).to_string();
                        let (key, colour) = (format!("session:{runner}:{id}"), theme::task(Group::Running));
                        Floating { task: None, key, colour, mono: spec.glyph.to_string(), title, at }
                    }
                })
            })
            .collect();
        drop(badges);
        let surface = floor.surface(&self.model);
        let (dir, drawn, jobs) = (city.dir.to_string(), self.city_drawn.clone(), self.city_painter.jobs.clone());
        let (home, hints) = (self.home.clone(), self.home.clone());
        let shows = {
            let drawn = drawn.borrow();
            drawn.places.get(&floor).and_then(|place| drawn.picture(place)).is_some_and(|made| made.key.dir == dir)
        };
        let picture = canvas(
            move |floor_bounds, window, _| {
                let bounds = area(floor_bounds, hints.borrow().hints_at(floor));
                let mut held = drawn.borrow_mut();
                // Too small for a city: the place's whole floor, not the part under the hints.
                let room = (f32::from(floor_bounds.size.width), f32::from(floor_bounds.size.height));
                let small = empty::empty(surface, room, true).backdrop != Backdrop::City;
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                // Pixels, as the window draws them: the picture is never scaled.
                let scale = window.scale_factor();
                let size = |v: f32| (v * scale).round() as u32;
                let place = held.places.entry(floor).or_default();
                // A city that became too small, or large enough: the frame after shows its lattice.
                if place.measured && place.small != small {
                    window.refresh();
                }
                (place.bounds, place.small, place.measured, place.scale) = (bounds, small, true, scale);
                place.since.get_or_insert_with(Instant::now);
                let mut key = plan.as_ref().map(|plan| Key {
                    dir,
                    plan: Arc::as_ptr(plan) as usize,
                    lit,
                    size: (size(w), size(h)),
                    rise: 1000,
                    glass: !crate::config::live().developer.city_opaque,
                });
                key = key.filter(|key| !small && key.size.0 > 0 && key.size.1 > 0);
                // Another folder in the same place (the main window's tab switched): its own first
                // picture, timed, risen or faded, again.
                let other = |a: &Key, b: &Key| a.dir != b.dir;
                if place.shown.as_ref().zip(key.as_ref()).is_some_and(|(a, b)| other(a, b)) && place.first_ms.is_some()
                {
                    (place.first_ms, place.fade, place.rise, place.rise_off, place.waited) =
                        (None, None, None, false, false);
                    place.since = Some(Instant::now());
                }
                // Its plan come after its lattice waited: its city rises, once (none under Reduce
                // Motion, nor after a frame of it was too slow).
                let first = place.waited && place.first_ms.is_none() && !place.rise_off;
                if key.is_some() && first && place.rise.is_none() && !crate::dial::calm() {
                    place.rise = Some(Instant::now());
                }
                let share = place.rise.map(|at| at.elapsed().as_secs_f32() / RISE.as_secs_f32());
                match (share, key.as_mut()) {
                    (Some(share), Some(key)) if share < 1. => key.rise = ((share * 100.) as u16) * 10,
                    _ => place.rise = None,
                }
                place.asked = key.clone();
                // Each picture asked for once: not made, not on its way, not failed.
                if let (Some(key), Some(plan)) = (key, plan) {
                    let new = !held.pictures.contains_key(&key) && !held.failed.contains_key(&key.standing());
                    if new && held.sent.insert(key.clone()) && jobs.send((floor, key, plan)).is_err() {
                        held.sent.clear();
                    }
                }
                drop(held);
                drawn
            },
            // The hints as this frame laid them out.
            move |_, drawn: Shared, window, cx| {
                paint(&drawn, floor, home.borrow().hints_at(floor), &floating, window, cx)
            },
        );
        // A press on an agent's icon: the user's own goes to its task (`Model::city_agent_click`).
        let picture = div().absolute().size_full().child(picture.absolute().size_full()).on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                this.city_click(floor, e.position, window, cx)
            }),
        );
        if shows {
            picture.into_any_element()
        } else {
            div().size_full().child(lattice).child(picture).into_any_element()
        }
    }

    /// The pointer moved: the box under it, if it is the user's, in the empty place of this
    /// window it is over, not over the hints (they win). Redraws when that changed; nothing is
    /// rendered again.
    pub(crate) fn city_hover(&mut self, at: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let id = window.window_handle().window_id();
        let user = self.input == Origin::User;
        let home = self.home.clone();
        let mut drawn = self.city_drawn.borrow_mut();
        let floors: Vec<Floor> = drawn.places.keys().copied().collect();
        let mut changed = false;
        for floor in floors {
            let Some(place) = drawn.places.get(&floor) else { continue };
            let pointer = (user && place.window == Some(id) && place.bounds.contains(&at)).then_some(at);
            // An icon over the sky is no box: its name comes and goes by the icon under the pointer.
            let icon = |at: Option<Point<Pixels>>| {
                at.and_then(|at| place.icons.iter().rev().position(|icon| icon.rect.contains(&at)))
            };
            let icon_changed = icon(place.pointer) != icon(pointer);
            let hovered = pointer.and_then(|at| drawn.under(floor, at, home.borrow().hints_at(floor)));
            if let Some(place) = drawn.places.get_mut(&floor) {
                place.pointer = pointer;
                changed |= std::mem::replace(&mut place.hovered, hovered) != hovered || icon_changed;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// A press at `at` in `floor`'s city: on an agent's icon, what the core says a click there
    /// does (the user's own goes to its task).
    fn city_click(&mut self, floor: Floor, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let icon = (self.city_drawn.borrow().places.get(&floor))
            .and_then(|place| place.icons.iter().rev().find(|icon| icon.rect.contains(&at)).map(|icon| icon.task));
        let Some(task) = icon else { return };
        cx.stop_propagation();
        // A session's icon: which pane it runs in is not known, so the core says nothing.
        let who = task.map_or(Who::Session { runner: "", id: "", title: None }, Who::Task);
        if let Some(action) = self.model.city_agent_click(who, self.input) {
            self.dispatch(action, self.input, window, cx);
        }
        cx.notify();
    }

    /// For the state of a test run: the city of the first empty place on show that draws one
    /// (the main window's tab, else its empty panes by id, else a folded-out tab); null with none.
    pub(crate) fn city_snapshot(&self) -> serde_json::Value {
        let mut floors: Vec<Floor> = (self.model.empty_places().into_iter())
            .filter(|at| self.model.city_at(*at).is_some_and(|city| self.city.held.contains_key(city.dir)))
            .map(|at| self.floor_of(at))
            .collect();
        floors.sort_by_key(|floor| match floor {
            Floor::Main => (0, 0),
            Floor::Pane(id) => (1, *id),
            Floor::Out(tab) => (2, *tab),
        });
        // The first that drew a picture, else the first laid out.
        let drawn = self.city_drawn.borrow();
        let painted = |floor: &&Floor| drawn.places.get(*floor).is_some_and(|place| drawn.picture(place).is_some());
        let first =
            floors.iter().find(painted).or_else(|| floors.iter().find(|floor| drawn.places.contains_key(*floor)));
        let Some(first) = first else { return serde_json::Value::Null };
        let mut snapshot = drawn.snapshot(*first, self.city.came);
        snapshot["read_us"] = self.city.read_us.into();
        // The places too small for a city, drawing their lattice.
        let small = floors.iter().filter(|floor| drawn.places.get(*floor).is_some_and(|place| place.small));
        snapshot["small"] = small.map(|floor| on(*floor)).collect::<Vec<_>>().into();
        snapshot
    }

    /// File lists read so far, a city on show or not: for the counts of a test run.
    pub(crate) fn city_reads(&self) -> usize {
        self.city.came
    }

    /// Frames the hovered beam's fade asked for: for the counts of a test run.
    pub(crate) fn city_beam_frames(&self) -> u64 {
        self.city_drawn.borrow().beam_frames
    }

    /// Frames a first picture's fade-in asked for: for the counts of a test run.
    pub(crate) fn city_fade_frames(&self) -> u64 {
        self.city_drawn.borrow().fade_frames
    }

    /// Frames a first city's rise painted: for the counts of a test run.
    pub(crate) fn city_rise_frames(&self) -> u64 {
        self.city_drawn.borrow().rise_frames
    }

    /// Frames an agent's icon gliding asked for: for the counts of a test run.
    pub(crate) fn city_agent_frames(&self) -> u64 {
        self.city_drawn.borrow().agent_frames
    }
}

/// Where a place is, for the state of a test run: `"tab"`, `{"pane": id}` or `{"out": tab}`.
fn on(floor: Floor) -> serde_json::Value {
    match floor {
        Floor::Pane(id) => serde_json::json!({ "pane": id }),
        Floor::Out(tab) => serde_json::json!({ "out": tab }),
        Floor::Main => serde_json::json!("tab"),
    }
}

/// The part of the floor the picture takes: below the hints when that band is no wider than the
/// plan's shape (a tall tab: the city is as large there, and no building is behind them); else
/// all of it.
fn area(floor: Bounds<Pixels>, hints: Bounds<Pixels>) -> Bounds<Pixels> {
    let below = Bounds::from_corners(point(floor.left(), hints.bottom() + px(CLEAR)), floor.bottom_right());
    let tall = below.size.height > px(0.) && below.size.width <= below.size.height * 1.6;
    if hints.size.width > px(0.) && tall { below } else { floor }
}

/// The picture made for this folder, as large as its area (a stale one stretched until the new
/// one comes); a backing behind the hints, so they read over any building; and the root's
/// folders' names, none behind the hints.
fn paint(
    drawn: &Shared,
    floor: Floor,
    hints: Bounds<Pixels>,
    floating: &[Floating],
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let mut drawn = drawn.borrow_mut();
    let Some(place) = drawn.places.get_mut(&floor) else { return };
    // Nothing of an earlier paint is on show until this one draws it.
    (place.beacon, place.labels, place.window) = (None, 0, Some(window.window_handle().window_id()));
    place.lit_beams.clear();
    let (bounds, hovered, pointer) = (place.bounds, place.hovered, place.pointer);
    let was = std::mem::take(&mut place.icons);
    let (waited, fade, first_ms, laid) = (place.waited, place.fade, place.first_ms, place.since);
    let drawn = &mut *drawn;
    let made = drawn.places.get(&floor).and_then(|place| drawn.picture(place));
    // A frame of the rise rendered too slowly: no rise for this city, its standing picture fades in.
    let slow = made.is_some_and(|made| made.key.rise < 1000 && made.render_us > RISE_BUDGET_US);
    let Some(made) = made.filter(|_| !slow) else {
        // Its lattice shows while it waits: its first picture will rise, or fade in.
        if let Some(place) = drawn.places.get_mut(&floor) {
            place.waited = true;
            if slow {
                (place.rise, place.rise_off, place.newest) = (None, true, None);
                window.refresh();
            }
        }
        return;
    };
    let rising = made.key.rise < 1000;
    if rising {
        window.request_animation_frame();
    }
    let shown = made.key.clone();
    let _ = window.paint_image(bounds, Corners::default(), made.image.clone(), 0, false);
    // A first picture after the lattice waited fades in from the background; one made already
    // (a tab come back) shows at once, as under Reduce Motion.
    let fade = fade.or_else(|| (waited && first_ms.is_none() && !rising).then(Instant::now));
    let faded = match fade {
        Some(at) if !crate::dial::calm() => (at.elapsed().as_secs_f32() / FADE_IN.as_secs_f32()).min(1.),
        _ => 1.,
    };
    let fading_in = faded < 1.;
    if fading_in {
        window.paint_quad(fill(bounds, Hsla::from(rgb(theme::BACKGROUND)).opacity(1. - faded * faded)));
        window.request_animation_frame();
    }
    let first_ms = first_ms.or_else(|| laid.map(|laid| laid.elapsed().as_millis() as u64));
    let clear = hints.dilate(px(CLEAR));
    if hints.size.width > px(0.) {
        // Faint wide pills under tighter ones: about 0.75 dark over the hints, nothing at the edge.
        for i in 0..=LAYERS {
            let d = FEATHER * (1. - i as f32 / LAYERS as f32);
            let pill = px(d) + hints.size.height / 2.;
            window.paint_quad(
                fill(hints.dilate(px(d)), Hsla::from(rgb(theme::BACKGROUND)).opacity(0.14)).corner_radii(pill),
            );
        }
    }
    let style = window.text_style();
    let shape = |text: &str, color: Hsla, size: f32, window: &mut Window| {
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window.text_system().shape_line(text.to_string().into(), px(size), &[run], None)
    };
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let mut drawn_names: Vec<Bounds<Pixels>> = Vec::new();
    // A name with a line break in it is not drawn: `shape_line` takes one line. Nor one wider
    // than its plate's edge, or over the hints or another name.
    for name in made.names.iter().filter(|name| !name.text.contains(char::is_control)) {
        let line = shape(&name.text, Hsla::from(rgb(theme::MUTED)), theme::TEXT_2XS, window);
        let at = bounds.origin + point(px(name.at.0 * w + 4.), px(name.at.1 * h + 2.));
        let room = Bounds { origin: at, size: gpui::size(line.width, px(14.)) };
        if f32::from(line.width) + 8. <= name.room * w
            && !room.intersects(&clear)
            && bounds.contains(&room.bottom_right())
            && !drawn_names.iter().any(|other| other.intersects(&room))
            && line.paint(at, px(14.), window, cx).is_ok()
        {
            drawn_names.push(room);
        }
    }
    let labels = drawn_names.len();
    let to_window = |p: [f32; 3]| {
        let (x, y) = made.scene.camera?.project(p)?;
        Some(bounds.origin + point(px(x * w), px(y * h)))
    };
    let beam_of = |i: usize| {
        let (foot, end) = made.scene.beam(i)?;
        Some((to_window(foot)?, to_window(end)?))
    };
    // Each lit building's beam, steady and thin in its task's colour (a hovered one's over it).
    let mut lit_beams = Vec::new();
    for (i, block) in made.scene.blocks.iter().enumerate() {
        let Kind::Lit(colour) = block.kind else { continue };
        let Some((foot, end)) = beam_of(i) else { continue };
        if let Some((end, _)) = fit_beam(foot, end, None, bounds, clear) {
            paint_beam(window, foot, end, Hsla::from(rgb(colour)).opacity(0.85), 1., 2.);
            lit_beams.push([foot.x, foot.y, end.x, end.y].map(f32::from));
        }
    }
    let (icons, gliding) = agents(made, floating, &was, bounds, clear, window, cx, &shape);
    // The last painted is on top.
    let over_icon = pointer.and_then(|at| icons.iter().rev().find(|icon| icon.rect.contains(&at)));
    let hovered_agent = over_icon.map(|icon| icon.key.clone());
    if let Some(icon) = over_icon {
        let title = floating.iter().find(|f| f.key == icon.key).map(|f| f.title.as_str()).unwrap_or_default();
        name_agent(icon.rect, title, bounds, window, cx, &shape);
    }
    // An icon under the pointer wins over the box under it.
    let hovered = hovered.filter(|_| hovered_agent.is_none());
    // The hovered box's beam fades in, at once under Reduce Motion.
    let since = match (hovered, drawn.places.get(&floor).and_then(|place| place.fading)) {
        (Some(i), Some((was, since))) if was == i => Some((i, since)),
        (Some(i), _) => Some((i, Instant::now())),
        (None, _) => None,
    };
    let shown_for = since.map_or(FADE, |(_, since)| since.elapsed());
    let alpha = if crate::dial::calm() { 1. } else { (shown_for.as_secs_f32() / FADE.as_secs_f32()).min(1.) };
    let beacon = hovered.and_then(|i| {
        let (foot, end) = beam_of(i)?;
        beacon(made, i, (foot, end), alpha, bounds, clear, window, cx, &shape)
    });
    let fades = beacon.is_some() && alpha < 1.;
    if fades {
        window.request_animation_frame();
    }
    if gliding {
        window.request_animation_frame();
    }
    drawn.paints += 1;
    drawn.beam_frames += u64::from(fades);
    drawn.fade_frames += u64::from(fading_in);
    drawn.rise_frames += u64::from(rising);
    drawn.agent_frames += u64::from(gliding);
    if let Some(made) = drawn.pictures.get_mut(&shown) {
        made.used = Instant::now();
    }
    if let Some(place) = drawn.places.get_mut(&floor) {
        (place.labels, place.beacon, place.shown) = (labels, beacon, Some(shown));
        (place.fading, place.lit_beams) = (since, lit_beams);
        (place.fade, place.first_ms) = (fade, first_ms);
        (place.icons, place.hovered_agent) = (icons, hovered_agent);
    }
}

/// The hovered icon's name: its task's title beside it, right of it or else left, on a backing.
fn name_agent(
    icon: Bounds<Pixels>,
    title: &str,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::App,
    shape: &dyn Fn(&str, Hsla, f32, &mut Window) -> gpui::ShapedLine,
) {
    let title: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).take(60).collect();
    let line = shape(&title, Hsla::from(rgb(theme::TEXT)), theme::TEXT_XS, window);
    let size = gpui::size(line.width + px(12.), px(18.));
    let y = icon.center().y - size.height / 2.;
    let right = Bounds { origin: point(icon.right() + px(BESIDE / 2.), y), size };
    let left = Bounds { origin: point(icon.left() - px(BESIDE / 2.) - size.width, y), size };
    let label = if bounds.contains(&right.bottom_right()) { right } else { left };
    window.paint_quad(fill(label, Hsla::from(rgb(theme::BACKGROUND)).opacity(0.8)).corner_radii(px(theme::RADIUS_MD)));
    let _ = line.paint(label.origin + point(px(6.), px(2.)), px(14.), window, cx);
}

/// Each agent's icon (`Scene::marks`): over the building it is at, at its beam's end, a cone of
/// its colour from the icon down onto the building's top; fanned beside others there; at no file,
/// over the city's edge (under the hints if they cover it) with no cone. Lowered along the beam
/// until clear of the hints and inside the picture, else not drawn. One that moves glides there (at once under Reduce Motion).
/// Returns the icons drawn, and whether one still glides.
fn agents(
    made: &Made,
    floating: &[Floating],
    was: &[Icon],
    bounds: Bounds<Pixels>,
    clear: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::App,
    shape: &dyn Fn(&str, Hsla, f32, &mut Window) -> gpui::ShapedLine,
) -> (Vec<Icon>, bool) {
    let Some(camera) = made.scene.camera else { return (Vec::new(), false) };
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let to_window = |p: [f32; 3]| camera.project(p).map(|(x, y)| bounds.origin + point(px(x * w), px(y * h)));
    let place = |id: &str| made.plan.places.iter().position(|place| place.id == id);
    let boxes: Vec<Option<usize>> = (floating.iter())
        .map(|agent| {
            let at = place(agent.at.as_deref()?)?;
            made.scene.blocks.iter().position(|block| block.place == Some(at))
        })
        .collect();
    let (calm, now) = (crate::dial::calm(), Instant::now());
    let side = gpui::size(px(ICON), px(ICON));
    let mut gliding = false;
    let mut icons = Vec::new();
    for (agent, mark) in floating.iter().zip(made.scene.marks(&boxes)) {
        let Some(end) = to_window(mark.at) else { continue };
        let block = mark.over.and_then(|i| made.scene.blocks.get(i));
        let foot = block.and_then(|block| to_window(block.top()));
        let offset = point(px((mark.fan.0 as f32 - (mark.fan.1 as f32 - 1.) / 2.) * (ICON + FAN_GAP)), px(0.));
        let fits = |at: Point<Pixels>| {
            let rect = Bounds::centered_at(at, side);
            (!rect.intersects(&clear) && bounds.contains(&rect.origin) && bounds.contains(&rect.bottom_right()))
                .then_some(at)
        };
        // At no file: under the hints if they cover the city's corner (a workspace's empty tab
        // keeps 560 points clear for them).
        let under = point(end.x, end.y.max(clear.bottom() + px(ICON / 2. + 1.)));
        let to = match foot {
            Some(f) => [1., 0.75, 0.5, 0.25].into_iter().find_map(|t: f32| fits(f + (end - f) * t + offset)),
            None => [end, under].into_iter().find_map(|at| fits(at + offset)),
        };
        let Some(to) = to else { continue };
        let before = was.iter().find(|icon| icon.key == agent.key);
        let (from, since) = match before {
            Some(icon) if icon.to == to => (icon.from, icon.since),
            Some(icon) => (icon.shown, now),
            None => (to, now),
        };
        let t = if calm { 1. } else { (since.elapsed().as_secs_f32() / GLIDE.as_secs_f32()).min(1.) };
        gliding |= t < 1. && from != to;
        let shown = from + (to - from) * (1. - (1. - t).powi(3));
        let rect = Bounds::centered_at(shown, side);
        let colour = Hsla::from(rgb(agent.colour));
        // The cone: from the icon's foot over the building's top, its widest corners on screen.
        let top =
            block.map(|b| [[b.min[0], b.min[2]], [b.max[0], b.min[2]], [b.max[0], b.max[2]], [b.min[0], b.max[2]]]);
        let corners: Option<Vec<Point<Pixels>>> =
            top.and_then(|top| top.iter().map(|[x, z]| to_window([*x, block?.max[1], *z])).collect());
        if let Some(corners) = &corners {
            let apex = point(shown.x, rect.bottom());
            let by_x =
                |a: &&Point<Pixels>, b: &&Point<Pixels>| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal);
            if let (Some(left), Some(right)) = (corners.iter().min_by(by_x), corners.iter().max_by(by_x)) {
                let mut cone = PathBuilder::fill();
                cone.move_to(apex);
                cone.line_to(*left);
                cone.line_to(*right);
                cone.close();
                crate::dial::stroke(window, cone, colour.opacity(0.16));
            }
            let mut pool = PathBuilder::fill();
            pool.move_to(corners[0]);
            for corner in &corners[1..] {
                pool.line_to(*corner);
            }
            pool.close();
            crate::dial::stroke(window, pool, colour.opacity(0.3));
        }
        // The task's badge, as drawn on its row: its letters in a square bordered in its colour.
        window.paint_quad(
            fill(rect, rgb(theme::SELECTED)).corner_radii(px(ICON / 4.)).border_widths(px(2.)).border_color(colour),
        );
        let mono = shape(&agent.mono, Hsla::from(rgb(theme::TEXT)), (ICON * 0.4).max(9.), window);
        let _ = mono.paint(point(shown.x - mono.width / 2., shown.y - px(7.)), px(14.), window, cx);
        let (task, key, cone) = (agent.task, agent.key.clone(), corners.is_some());
        icons.push(Icon { task, key, shown, from, to, since, rect, cone, fan: mark.fan });
    }
    (icons, gliding)
}

/// Where the end of a beam from `foot` to `end` is drawn (window points), and its label's box (`label`: its
/// size) beside the dot, right of it or else left: the beam lowered (to three quarters, a half, a
/// quarter) until none of it is over the hints or off the picture; none if it never fits.
fn fit_beam(
    foot: Point<Pixels>,
    end: Point<Pixels>,
    label: Option<gpui::Size<Pixels>>,
    bounds: Bounds<Pixels>,
    clear: Bounds<Pixels>,
) -> Option<(Point<Pixels>, Option<Bounds<Pixels>>)> {
    let clear_of = |b: &Bounds<Pixels>| !b.intersects(&clear);
    let inside = |b: &Bounds<Pixels>| bounds.contains(&b.origin) && bounds.contains(&b.bottom_right());
    [1., 0.75, 0.5, 0.25].into_iter().find_map(|t: f32| {
        let end = foot + (end - foot) * t;
        let line = Bounds::from_corners(
            point(foot.x.min(end.x) - px(4.), end.y - px(4.)),
            point(foot.x.max(end.x) + px(4.), foot.y),
        );
        if !(clear_of(&line) && inside(&line)) {
            return None;
        }
        let Some(size) = label else { return Some((end, None)) };
        let at = |left: Pixels| Bounds { origin: point(left, end.y - size.height / 2.), size };
        let sides = [at(end.x + px(BESIDE)), at(end.x - px(BESIDE) - size.width)];
        let label = sides.into_iter().find(|b| clear_of(b) && inside(b))?;
        Some((end, Some(label)))
    })
}

/// A beam: a line from `foot` up to `end`, faint at the box and brightest at its dot (as light
/// falling from it), and the dot.
fn paint_beam(window: &mut Window, foot: Point<Pixels>, end: Point<Pixels>, colour: Hsla, width: f32, dot: f32) {
    const STEPS: usize = 6;
    for step in 0..STEPS {
        let (a, b) = (step as f32 / STEPS as f32, (step + 1) as f32 / STEPS as f32);
        let mut line = PathBuilder::stroke(px(width));
        line.move_to(foot + (end - foot) * a);
        line.line_to(foot + (end - foot) * b);
        crate::dial::stroke(window, line, colour.opacity(0.15 + 0.85 * b * b));
    }
    crate::dial::disc(window, end, dot, colour);
}

/// The hovered box's beacon: its beam (`foot` to `end`, window points) in the text's colour, and
/// beside its dot a label: the file's path in the checkout, or the district's, cut in the middle
/// if long. Never over the hints (`fit_beam`). `alpha`: how far it has faded in.
#[allow(clippy::too_many_arguments)]
fn beacon(
    made: &Made,
    i: usize,
    (foot, end): (Point<Pixels>, Point<Pixels>),
    alpha: f32,
    bounds: Bounds<Pixels>,
    clear: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::App,
    shape: &dyn Fn(&str, Hsla, f32, &mut Window) -> gpui::ShapedLine,
) -> Option<Beacon> {
    let block = made.scene.blocks.get(i)?;
    let place = made.plan.places.get(block.place?)?;
    let colour = Hsla::from(rgb(theme::TEXT)).opacity(alpha);
    // One line, as wide as the wider side of the dot allows: `src/…/views/list.rs`.
    let side = (bounds.right() - foot.x.max(end.x)).max(foot.x.min(end.x) - bounds.left());
    let most = (side - px(BESIDE + 12.)).max(px(40.));
    let mut text: String = place.id.chars().map(|c| if c.is_control() { '?' } else { c }).collect();
    let line = loop {
        let line = shape(&text, colour, theme::TEXT_XS, window);
        let count = text.chars().count();
        if line.width <= most || count <= 3 {
            break line;
        }
        let keep = ((count as f32 * f32::from(most) / f32::from(line.width)) as usize).clamp(2, count - 2);
        let (head, tail) = (keep / 2, keep - keep / 2);
        let chars: Vec<char> = text.chars().collect();
        text = chars[..head].iter().chain(['…'].iter()).chain(&chars[count - tail..]).collect();
    };
    let size = gpui::size(line.width + px(12.), px(18.));
    let (end, label) = fit_beam(foot, end, Some(size), bounds, clear)?;
    let label = label?;
    paint_beam(window, foot, end, colour, 1.5, 3.);
    let backing = Hsla::from(rgb(theme::BACKGROUND)).opacity(0.8 * alpha);
    window.paint_quad(fill(label, backing).corner_radii(px(theme::RADIUS_MD)));
    let _ = line.paint(label.origin + point(px(6.), px(2.)), px(14.), window, cx);
    let beam = [foot.x, foot.y, end.x, end.y].map(f32::from);
    Some(Beacon { file: !place.group, lit: matches!(block.kind, Kind::Lit(_)), beam, label })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long a checkout's read takes, for the design note's costs. `CITY_BENCH`: folders,
    /// `:` between them. `cargo test -p ottospace --release -- --ignored a_read_is_timed --nocapture`.
    #[test]
    #[ignore = "prints timings of the folders CITY_BENCH names; run it by name, in release"]
    fn a_read_is_timed() {
        for dir in std::env::var("CITY_BENCH").unwrap_or_default().split(':').filter(|dir| !dir.is_empty()) {
            let top = Path::new(dir).canonicalize().unwrap();
            let roots = [top.to_string_lossy().into_owned()];
            let started = Instant::now();
            let places = read(&top, &roots).map(|atlas| atlas.places.len());
            println!("{dir}: {places:?} places in {:.1} ms", started.elapsed().as_secs_f64() * 1000.);
        }
    }
}
