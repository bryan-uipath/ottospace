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
    AnyElement, Bounds, Context, Corners, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels,
    Point, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, TextRun, Window, WindowId, canvas, div, fill,
    point, prelude::*, px, rgb,
};
use ottospace_core::Action;
use ottospace_core::atlas::{self, Atlas};
use ottospace_core::city_colours::{self, FolderColours};
use ottospace_core::city_filter;
use ottospace_core::city_labels::{self, Candidate, Tier};
use ottospace_core::empty::{self, Backdrop};
use ottospace_core::model::Origin;
use ottospace_core::model::city::{At, CameraStep, Glimpse, SPIN_STEPS, Who};
use ottospace_core::scene::{self, Camera, Kind, Layout, Scene, View};
use ottospace_core::settings::expand_home;
use ottospace_core::stats::rank;
use ottospace_core::tasks::{Group, TaskId};
use ottospace_core::{status_bar, theme};

use crate::city_gpu::{Boxes, Gpu, Picture};
use crate::home::Floor;
use crate::{Workspace, rect};

/// A drag of a city's camera: the place, the button, where it began and was last, and whether
/// it moved past a click (a left press is a click until then).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CityDrag {
    floor: Floor,
    at: At,
    button: MouseButton,
    from: Point<Pixels>,
    last: Point<Pixels>,
    moving: bool,
}

/// A press that moves less than this (points) is a click: it focuses, never turns the camera.
const CLICK_SLOP: f32 = 4.;
/// Degrees the camera turns per point of a left drag, or of a trackpad's ⌥ scroll.
const ORBIT_PER_POINT: f32 = 0.4;
/// A mouse wheel's line zooms by this; a trackpad's ⌘ scroll by the second, per point.
const ZOOM_PER_LINE: f32 = 1.15;
const ZOOM_PER_POINT: f32 = 1.006;
/// A reset eases back over this long; at once under Reduce Motion.
const EASE: Duration = Duration::from_millis(200);

/// How long `git ls-files` may take, how much it may say, and how many files a city has. Past
/// any of them there is no city: half of one would put files where the whole one does not.
const TIMEOUT: Duration = Duration::from_secs(60);
const MOST_BYTES: u64 = 64 << 20;
const MOST_FILES: usize = 600_000;
/// Cities kept, by folder: a task's tab shown again draws at once.
const KEPT: usize = 8;
/// Folders of this tier and outer are named on their front wall (2: a folder of the root and
/// the districts on it).
const WALL_TIERS: usize = 2;
/// Names are shaped for this many things at most, the largest: a few placed of each.
const SHAPED: usize = 4 * city_labels::MOST;
/// A beam's label sits this many points beside its dot; the hovered beam fades in over this long.
const BESIDE: f32 = 6.;
const FADE: Duration = Duration::from_millis(120);
/// A place's first city, after its lattice waited for it, rises over this long (its buildings
/// grow, district by district); where a frame of the rise renders slower than `RISE_BUDGET_US`,
/// its standing picture fades in over `FADE_IN` instead.
const RISE: Duration = Duration::from_millis(400);
const RISE_BUDGET_US: u64 = 16_000;
const FADE_IN: Duration = Duration::from_millis(250);
/// Pictures no place shows, kept so a tab come back shows its city at once: the newest this many.
const KEPT_PICTURES: usize = 2;
/// An agent's icon's side and the gap between icons fanned side by side, in points; an icon
/// moving to another file glides over this long.
const ICON: f32 = 22.;
const FAN_GAP: f32 = 4.;
const GLIDE: Duration = Duration::from_millis(280);
/// The most sub-agents drawn under an agent's icon, each a square this wide, this far apart.
const SUBS: usize = 8;
const SUB: f32 = 5.;
const SUB_GAP: f32 = 2.;
/// A busy agent's pulse: once this long.
const PULSE: Duration = Duration::from_millis(1600);
/// Room kept between a name and the empty tab's hints, in points.
const CLEAR: f32 = 12.;
/// The hints' backing fades out over this many points, in this many layers: no edge to see.
const FEATHER: f32 = 40.;
const LAYERS: usize = 8;

/// What a folder's read was asked with: the folders git may run in, `city_exclude` as written
/// (its keys resolved on the read's thread), and the files lit then, each with whether its call
/// ran (a file an agent wrote may be new, once the call ends).
type Asked = (Vec<String>, Exclude, Vec<(String, bool)>);

/// `city_exclude`: by checkout key (`*`, or a path), its entries.
type Exclude = Vec<(String, Vec<String>)>;

/// A thread of its own that answers each job with `work`, in order: of the jobs waiting, only
/// the last of each `key` (a newer ask for the same thing replaces the older), the others answered
/// `None`. Each job comes back with its answer, and `answered` is called after each.
fn latest_each<J, K, R>(
    key: fn(&J) -> K,
    mut work: impl FnMut(&J) -> R + Send + 'static,
    answered: impl Fn() + Send + 'static,
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
                answered();
            }
        }
    });
    (jobs, answers)
}

/// A folder's read: the folder, the folders git may run in, and `city_exclude`.
type ReadJob = (String, Vec<String>, Exclude);

/// Reads checkouts' files on a thread of its own: a folder when it comes on show, or the files
/// its agents are at change, once each (`latest_each`). No timer of its own.
pub struct Reader {
    jobs: Sender<ReadJob>,
    reads: Receiver<(ReadJob, Option<Option<(Option<Atlas>, u64)>>)>,
    asked: HashMap<String, Asked>,
    /// The folders on show, as the read's thread sees them: a read of one no longer on show
    /// (its tab closed) stops.
    wanted: Arc<std::sync::Mutex<HashSet<String>>>,
    held: HashMap<String, Arc<Atlas>>,
    /// How many reads came, and how long the last took (µs): for the state of a test run.
    came: usize,
    read_us: Option<u64>,
}

impl Reader {
    pub fn new() -> Self {
        let wanted: Arc<std::sync::Mutex<HashSet<String>>> = Arc::default();
        let on_show = wanted.clone();
        let (jobs, reads) = latest_each(
            |job: &ReadJob| job.0.clone(),
            move |(dir, roots, exclude): &ReadJob| {
                let started = Instant::now();
                let gone = || on_show.lock().is_ok_and(|wanted| !wanted.contains(dir));
                let atlas = read(Path::new(dir), roots, exclude, &gone);
                // Stopped (its place closed): no answer, so a city held for it is kept.
                (atlas.is_some() || !gone()).then(|| (atlas, started.elapsed().as_micros() as u64))
            },
            ottospace_mac::wake::post,
        );
        Self { jobs, reads, asked: HashMap::new(), wanted, held: HashMap::new(), came: 0, read_us: None }
    }

    /// The folders on show, each with what it is asked with: one new, or whose roots or lit
    /// files changed, is read (again). A folder no longer on show is forgotten, and read again
    /// when it comes back.
    fn ask(&mut self, on_show: HashMap<String, Asked>) {
        // Published first: a read sent below must find its folder wanted.
        if let Ok(mut wanted) = self.wanted.lock() {
            *wanted = on_show.keys().cloned().collect();
        }
        for (dir, asked) in &on_show {
            if self.asked.get(dir) != Some(asked) {
                let _ = self.jobs.send((dir.clone(), asked.0.clone(), asked.1.clone()));
            }
        }
        self.asked = on_show;
    }

    /// Takes the reads that came. True if one did.
    fn take(&mut self) -> bool {
        let came: Vec<(String, (Option<Atlas>, u64))> =
            self.reads.try_iter().filter_map(|((dir, ..), read)| Some((dir, read.flatten()?))).collect();
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

/// `city_exclude`'s entries for the checkout at `dir`: every checkout's and its own (a key is its
/// path as written, `~` for the home folder, matched resolved). On the read's thread: a key on a
/// slow disk never holds up a frame.
fn exclude_for(dir: &Path, setting: &[(String, Vec<String>)]) -> Vec<String> {
    let setting: std::collections::BTreeMap<String, Vec<String>> = setting.iter().cloned().collect();
    let home = crate::config::user_home();
    let resolved = |path: &str| std::fs::canonicalize(expand_home(path, &home)).ok();
    let here = std::fs::canonicalize(dir).ok();
    city_filter::entries_for(&setting, |key| here.is_some() && resolved(key) == here)
}

/// The plan of the checkout at `dir` (`checkout_files`), less the folders `exclude` leaves out
/// for it (`city_filter`): dropped before the count and the layout, an untracked one never
/// listed by git. `None`: no city, or `gone` (its place closed) said to stop.
fn read(dir: &Path, roots: &[String], exclude: &[(String, Vec<String>)], gone: &dyn Fn() -> bool) -> Option<Atlas> {
    let rule = city_filter::Exclude::new(&exclude_for(dir, exclude));
    let keep = |name: &str| !rule.excludes(name);
    let limits = (MOST_FILES, MOST_BYTES, TIMEOUT);
    let (files, _) = checkout_files(dir, roots, limits, true, &rule.git_patterns(), &keep, gone)?;
    (!files.is_empty()).then(|| atlas::atlas(&atlas::repo(&files)))
}

/// The files git lists in the checkout at `dir` (tracked, and new ones it does not ignore), each
/// with its size, within `(most, most_bytes, timeout)`: at most `most` files, and true if there
/// were more (or more than `most_bytes` of names), all of it within `timeout`; `whole`: a list that was cut is none. Hardened as the status bar's read
/// (`child::git_read`), and only in a folder that, resolved, is one of `roots`
/// (`status_bar::reads_changes`). A link is no file, and nothing under a folder that is a link is
/// one: nothing outside the checkout is looked at. Git lists no untracked file under `skip`
/// (its `--exclude` patterns), and only names `keep` keeps are counted or looked at. `None`: not
/// such a checkout, git failed or took too long, or `gone` said to stop (looked at every 1,024 files).
pub(crate) fn checkout_files(
    dir: &Path,
    roots: &[String],
    (most, most_bytes, timeout): (usize, u64, Duration),
    whole: bool,
    skip: &[String],
    keep: &dyn Fn(&str) -> bool,
    gone: &dyn Fn() -> bool,
) -> Option<(Vec<(String, u64)>, bool)> {
    let top =
        dir.canonicalize().ok().filter(|top| top.to_str().is_some_and(|top| status_bar::reads_changes(top, roots)))?;
    let mut list = crate::child::git_read(&top);
    list.args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
    list.args(skip.iter().map(|pattern| format!("--exclude={pattern}")));
    if gone() {
        return None;
    }
    let late = Instant::now() + timeout;
    let said = crate::child::run_until(list, timeout, most_bytes, gone).ok()?;
    // Cut at `most_bytes`: git stops on the closed pipe, and the last name may be half of one.
    let full = said.out.len() as u64 >= most_bytes;
    if !said.ok && !full {
        return None;
    }
    let mut names: Vec<&str> = said.out.split('\0').filter(|name| !name.is_empty()).collect();
    if full {
        names.pop();
    }
    names.retain(|name| keep(name));
    let cut = full || names.len() > most;
    if cut && whole {
        return None;
    }
    names.truncate(most);
    // Each folder on the way, asked once: a real folder, not a link to one elsewhere.
    let mut folders: HashMap<&Path, bool> = HashMap::new();
    let real = |folder: &Path| std::fs::symlink_metadata(top.join(folder)).is_ok_and(|at| at.is_dir());
    let mut files: Vec<(String, u64)> = Vec::new();
    for (i, name) in names.into_iter().enumerate() {
        // A slow disk: the limit is the whole read's, not git's alone.
        if Instant::now() > late || (i % 1024 == 0 && gone()) {
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
    /// The camera's view, as bits: yaw, pitch, zoom, pan.
    view: [u32; 5],
    /// How far below the picture's middle the fitted city's is, as bits (`scene::LOWER`).
    lower: u32,
    /// `city_folder_colors`: folder name to colour name.
    folders: Vec<(String, String)>,
    /// The user's drag (or a reset's ease) moves its camera: rendered at half size, unnamed.
    moving: bool,
}

impl Key {
    fn view(&self) -> View {
        let [yaw, pitch, zoom, x, z] = self.view.map(f32::from_bits);
        View { yaw, pitch, zoom, pan: [x, z] }
    }
}

impl Key {
    /// The same picture standing (not a frame of its rise).
    fn standing(&self) -> Key {
        Key { rise: 1000, ..self.clone() }
    }

    /// What its layout depends on (`scene::Layout`): not its camera, rise or motion. A camera
    /// move lays nothing out again.
    fn laid(&self) -> Key {
        Key { rise: 1000, view: [0; 5], moving: false, ..self.clone() }
    }

    /// Rendered at half size while it moves or rises: a quarter of the GPU's work, the copy out
    /// and the upload, stretched for a moment; standing, at full size.
    fn render_size(&self) -> (u32, u32) {
        let (w, h) = self.size;
        if self.moving || self.rise < 1000 { ((w / 2).max(1), (h / 2).max(1)) } else { (w, h) }
    }
}

/// A thing the picture may name (`city_labels`): its place, how it is named, its name, its
/// bounds and the middle of its top (where its name is centred), in shares of the picture.
struct Mark {
    place: usize,
    tier: Tier,
    text: String,
    bounds: [f32; 4],
    top: (f32, f32),
    face: [(f32, f32); 4],
}

/// A picture made: where, in shares of it, the lit buildings and the things it may name are;
/// the scene and plan it shows, for what the pointer is over.
struct Made {
    key: Key,
    image: Arc<RenderImage>,
    lit: Vec<([f32; 4], (f32, f32))>,
    marks: Vec<Mark>,
    scene: Scene,
    plan: Arc<Atlas>,
    places: usize,
    groups: usize,
    render_us: u64,
    /// The GPU's part of it: what a rise's frame costs once the city is laid out.
    gpu_us: u64,
    /// When a place last painted it: the newest unused are kept (`KEPT_PICTURES`).
    used: Instant,
    /// When it was made, and whether it was painted yet: its wait to be painted is timed once
    /// (`Drawn::waits`).
    made_at: Instant,
    painted: bool,
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
    /// Of the last paint: where, in window points, in which window, and its names (by place,
    /// where); kept first by the next (`city_labels::place`).
    bounds: Bounds<Pixels>,
    window: Option<WindowId>,
    names: Vec<(usize, Tier, Bounds<Pixels>)>,
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
    /// Its agents' reads and searches playing (`glimmer`).
    glimmer: Glimmer,
}

/// A read's flash, a search's sweep, and the files a search's result named, lit: this long each.
const FLASH: Duration = Duration::from_millis(450);
const SWEEP: Duration = Duration::from_millis(800);
const FOUND: Duration = Duration::from_millis(1500);
/// Glimpses this close together play as one: a burst of 20 reads is one ripple, not 20.
const TOGETHER: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq)]
enum Play {
    Flash,
    Sweep,
    Found,
}

impl Play {
    fn length(self) -> Duration {
        match self {
            Play::Flash => FLASH,
            Play::Sweep => SWEEP,
            Play::Found => FOUND,
        }
    }
}

/// One effect playing: over these boxes (a sweep over `whole`: the ground), in this colour.
struct Effect {
    play: Play,
    blocks: Vec<usize>,
    whole: bool,
    colour: u32,
    since: Instant,
}

/// The glimpses a place has played (by id; a search's result as `<id>#found`), and what plays.
/// Those there at its first paint are taken as played: opening a city replays nothing.
#[derive(Default)]
struct Glimmer {
    seen: HashSet<String>,
    order: std::collections::VecDeque<String>,
    began: bool,
    effects: Vec<Effect>,
}

/// The most glimpses a place remembers as played: the oldest are forgotten first.
const SEEN: usize = 1024;

impl Glimmer {
    /// Whether `key` is new: then it is played, and remembered.
    fn see(&mut self, key: String) -> bool {
        if self.seen.contains(&key) {
            return false;
        }
        if self.order.len() == SEEN
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        self.order.push_back(key.clone());
        self.seen.insert(key)
    }
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
    /// A call of it runs: its building, or the icon at no file, pulses (`PULSE`).
    busy: bool,
    /// The sub-agents it runs: small squares under its icon (`SUBS` at most, then `+n`).
    subagents: usize,
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
    pulsing: bool,
    subagents: usize,
}

/// A hovered box's beacon as drawn: whether a file's, whether lit; a file's beam (foot and end)
/// or a folder's outline (its top face's corners); its label.
#[derive(Clone, Copy)]
struct Beacon {
    file: bool,
    lit: bool,
    beam: Option<[f32; 4]>,
    outline: Option<[[f32; 2]; 4]>,
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
    /// Frames a busy agent's pulse asked for: none once its calls end, none under Reduce Motion.
    pulse_frames: u64,
    /// Frames a read's flash, a search's sweep or its result's light asked for: none once they
    /// end, none under Reduce Motion.
    glimmer_frames: u64,
    /// Pictures no place shows: GPUI keeps each in the window's atlas until it is dropped.
    retired: Vec<Arc<RenderImage>>,
    /// While a spin runs: its place, and each new picture's wait there from made to painted, in ms.
    waits: Option<(Floor, Vec<f32>)>,
}

pub type Shared = Rc<RefCell<Drawn>>;

/// A picture asked for by a place.
type PaintJob = (Floor, Key, Arc<Atlas>);

/// Renders pictures on a thread of its own, the newest each place asked for only (a camera or a
/// size that moved on is skipped: `latest_each`). The GPU is opened at the first, or before it
/// (`warm`), and kept; none (or `OTTOSPACE_CITY_NO_GPU` in a test run) and every picture fails.
pub struct Painter {
    /// Rung for each picture made: the main thread takes it at once (`Bell::rung`), not at the
    /// loop's next tick, so a turn's picture is painted on the next frame.
    pub(crate) bell: Arc<Bell>,
    jobs: Sender<PaintJob>,
    made: Receiver<(PaintJob, Option<Result<Made, String>>)>,
    gpu: Locked<Option<Result<Gpu, String>>>,
    /// The GPU's early opening was started (`warm`): once, never waited on.
    warming: Arc<std::sync::atomic::AtomicBool>,
}

/// Owned by the picture's thread and, while it opens the GPU early, the warming one.
type Locked<T> = Arc<std::sync::Mutex<T>>;

/// The GPU, opened and warmed (`Gpu::new`, `Gpu::warm`); or why not. A panic opening it is a
/// failure, never a poisoned lock.
fn open_gpu() -> Result<Gpu, String> {
    let refused = std::env::var_os("OTTOSPACE_E2E").and(std::env::var_os("OTTOSPACE_CITY_NO_GPU"));
    if refused.is_some() {
        return Err("no GPU (test)".into());
    }
    let gpu = std::panic::catch_unwind(Gpu::new).unwrap_or_else(|_| Err("opening the GPU panicked".into()))?;
    gpu.warm();
    Ok(gpu)
}

/// The GPU's lock, whatever panicked while another held it.
fn lock_gpu(gpu: &Locked<Option<Result<Gpu, String>>>) -> std::sync::MutexGuard<'_, Option<Result<Gpu, String>>> {
    gpu.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Painter {
    pub fn new() -> Self {
        let gpu: Locked<Option<Result<Gpu, String>>> = Arc::default();
        let held = gpu.clone();
        // The layouts made and their colours, by what they depend on, the newest last: a rise's
        // frames and a camera's moves lay one out once (two places of one checkout, two sizes).
        let mut last: Layouts = Vec::new();
        let bell = Arc::new(Bell::default());
        let ring = bell.clone();
        let (jobs, made) = latest_each(
            |job: &PaintJob| job.0,
            move |(_, key, plan): &PaintJob| {
                let mut gpu = lock_gpu(&held);
                let opened = gpu.get_or_insert_with(open_gpu);
                // A panic in the GPU's code fails this picture, and the GPU is opened again next time.
                let picture = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    opened.as_ref().map_err(Clone::clone).and_then(|gpu| make(gpu, key, plan, &mut last))
                }));
                picture.unwrap_or_else(|_| {
                    *gpu = None;
                    Err("the render panicked".into())
                })
            },
            move || {
                ottospace_mac::wake::post();
                ring.ring();
            },
        );
        Self { bell, jobs, made, gpu, warming: Arc::default() }
    }

    /// Opens the GPU on a thread of its own, if not yet: a city on show asks before its first
    /// picture, so that picture waits for no device nor first draw.
    /// Never waits: the main thread takes no lock the picture's thread holds while it renders.
    pub fn warm(&self) {
        if !self.warming.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let gpu = self.gpu.clone();
            std::thread::spawn(move || drop(lock_gpu(&gpu).get_or_insert_with(open_gpu)));
        }
    }
}

/// A signal from the picture's thread to the main thread: rung, and awaited (`rung`).
#[derive(Default)]
pub(crate) struct Bell {
    rung: std::sync::atomic::AtomicBool,
    waiter: std::sync::Mutex<Option<std::task::Waker>>,
}

impl Bell {
    fn ring(&self) {
        self.rung.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(waker) = self.waiter.lock().ok().and_then(|mut waiter| waiter.take()) {
            waker.wake();
        }
    }

    /// Ready once rung since the last time it was.
    pub(crate) fn rung(&self) -> impl std::future::Future<Output = ()> + '_ {
        std::future::poll_fn(move |cx| {
            // The waker first, then the flag: a ring between the two is seen by the flag.
            if let Ok(mut waiter) = self.waiter.lock() {
                *waiter = Some(cx.waker().clone());
            }
            if self.rung.swap(false, std::sync::atomic::Ordering::SeqCst) {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
    }
}

/// Layouts kept, newest last, with their folders' colours and boxes: by `Key::laid`.
type Layouts = Vec<(Key, Layout, Vec<Option<(u32, u32)>>, Arc<Boxes>)>;

/// Layouts kept for camera moves: a place or two of one checkout, at their sizes.
const KEPT_LAYOUTS: usize = 4;

/// A wheel or trackpad step moves its city's camera this long: its pictures are small and
/// unnamed until the steps stop.
pub(crate) const WHEELING: Duration = Duration::from_millis(150);

/// The scene of `plan` and its picture, on the picture's thread.
fn make(gpu: &Gpu, key: &Key, plan: &Arc<Atlas>, last: &mut Layouts) -> Result<Made, String> {
    let started = Instant::now();
    let (w, h) = key.size;
    let laid = key.laid();
    let kept = last.iter().position(|(was, ..)| *was == laid).map(|at| last.remove(at));
    let (layout, tints, boxes) = match kept {
        Some((_, layout, tints, boxes)) => (layout, tints, Some(boxes)),
        None => {
            let lit: Vec<(&str, u32)> = key.lit.iter().map(|(id, colour)| (id.as_str(), *colour)).collect();
            let layout = scene::layout(plan, &lit, w as f32 / h as f32, f32::from_bits(key.lower));
            // Each building in its folder's colour (`city_colours`).
            let user = key.folders.iter().cloned().collect();
            let rule = FolderColours::new(&user).unwrap_or_else(|_| FolderColours::built_in());
            let tints = city_colours::tints(plan, &layout.blocks, &rule);
            (layout, tints, None)
        }
    };
    let mut scene = layout.scene(&key.view());
    // Its boxes' colours and delays, once a layout: a camera move only places them.
    let boxes = boxes.unwrap_or_else(|| {
        scene.tints = tints.clone();
        Arc::new(Boxes::new(&scene, key.glass))
    });
    last.push((laid, layout, tints, boxes.clone()));
    if last.len() > KEPT_LAYOUTS {
        last.remove(0);
    }
    let scene_us = started.elapsed().as_micros() as u64;
    let (rw, rh) = key.render_size();
    let Picture { width, height, bgra, gpu_us, copy_us, prep_us, boxes_us, alloc_us } =
        gpu.render_boxes(&scene, &boxes, rw, rh, f32::from(key.rise) / 1000.)?;
    let camera = scene.camera.ok_or("no camera")?;
    let made_at = Instant::now();
    let image = image::RgbaImage::from_raw(width, height, bgra).ok_or("a picture of the wrong size")?;
    let image = Arc::new(RenderImage::new([image::Frame::new(image)]));
    // Where a picture's time goes, for the bench (`scripts/bench`'s city motion): numbers only.
    let image_us = made_at.elapsed().as_micros() as u64;
    let fields = serde_json::json!({ "scene_us": scene_us, "prep_us": prep_us, "boxes_us": boxes_us, "alloc_us": alloc_us, "gpu_us": gpu_us, "copy_us": copy_us, "image_us": image_us, "rise": key.rise, "w": rw, "h": rh });
    crate::trace::record("city_picture", fields);
    // Each lit building's bounds, and the middle of its top (never hidden).
    let lit = (scene.blocks.iter().filter(|b| matches!(b.kind, Kind::Lit(_))))
        .filter_map(|b| {
            let [x, y, w, h] = camera.bounds(b)?;
            let (tx, ty) = camera.project(b.top())?;
            Some(([x, y, w, h], (tx, ty)))
        })
        .collect();
    // What may be named: the top two folder tiers on their front wall, deeper folders and files
    // over their top (folders in capitals). None while the city rises or its camera moves.
    let rising = key.rise < 1000 || key.moving;
    let marks_at = Instant::now();
    let marks: Vec<Mark> = (scene.blocks.iter().enumerate())
        .filter(|_| !rising)
        .filter_map(|(i, b)| {
            let place = b.place?;
            let tier = match b.kind {
                Kind::Plate(tier) if tier <= WALL_TIERS => Tier::Wall,
                Kind::Plate(_) => Tier::Folder,
                Kind::Building | Kind::Lit(_) => Tier::File,
                Kind::Ground => return None,
            };
            let top = camera.project(b.top())?;
            let label = &plan.places[place].label;
            let text = if tier == Tier::File { label.clone() } else { label.to_uppercase() };
            Some(Mark { place, tier, text, bounds: camera.bounds(b)?, top, face: scene.outline(i)? })
        })
        .collect();
    if !rising {
        crate::trace::record("city_marks", serde_json::json!({ "us": marks_at.elapsed().as_micros() as u64 }));
    }
    let groups = plan.places.iter().filter(|p| p.group).count();
    let render_us = started.elapsed().as_micros() as u64;
    let (places, plan, used) = (plan.places.len(), plan.clone(), Instant::now());
    Ok(Made {
        key: key.clone(),
        image,
        lit,
        marks,
        scene,
        plan,
        places,
        groups,
        render_us,
        gpu_us,
        used,
        made_at: used,
        painted: false,
    })
}

impl Drawn {
    /// The picture `place` paints: the one it asks for once made, else the one it showed, if of
    /// the same folder. None with no GPU, or a place too small.
    fn picture(&self, place: &Place) -> Option<&Made> {
        let dir = &place.asked.as_ref()?.dir;
        // The one asked for, else the newest made for it (a rise's or a drag's frame, while a
        // newer is on its way), else the one shown.
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
            .map(|b| serde_json::json!({ "file": b.file, "lit": b.lit, "beam": b.beam, "outline": b.outline, "beacon": rect(b.label) }));
        let lit_beams = place.map(|place| place.lit_beams.clone()).unwrap_or_default();
        let agents: Vec<serde_json::Value> = (place.iter().flat_map(|place| &place.icons))
            .map(|i| {
                let hovered = place.is_some_and(|place| place.hovered_agent.as_ref() == Some(&i.key));
                serde_json::json!({
                    "task": i.task, "rect": rect(i.rect), "cone": i.cone, "fan": [i.fan.0, i.fan.1], "hovered": hovered,
                    "pulsing": i.pulsing, "subagents": i.subagents,
                })
            })
            .collect();
        let tier = |t: Tier| match t {
            Tier::Wall => "wall",
            Tier::Folder => "folder",
            Tier::File => "file",
        };
        let names: Vec<_> = (place.iter().flat_map(|place| &place.names))
            .map(|(_, t, r)| serde_json::json!({ "tier": tier(*t), "rect": rect(*r) }))
            .collect();
        serde_json::json!({
            "on": on(floor), "places": made.map(|made| made.places), "groups": made.map(|made| made.groups),
            "picture": made.map(|made| made.key.render_size()), "bounds": [x, y, w, h], "scale": scale,
            "lit": lit, "labels": names.len(), "names": names, "spots": made.map(|made| spots(made, [x, y, w, h])), "failed": failed, "reads": reads,
            "renders": self.renders, "paints": self.paints, "render_us": made.map(|made| made.render_us),
            "hovered": hovered, "lit_tops": tops, "lit_beams": lit_beams, "pictures": self.in_use(),
            "first_ms": place.and_then(|place| place.first_ms),
            "glass": made.map(|made| made.key.glass),
            "agents": agents,
            "glimmer": place.map(|place| {
                let n = |play: Play| place.glimmer.effects.iter().filter(|e| e.play == play).count();
                serde_json::json!({ "flash": n(Play::Flash), "sweep": n(Play::Sweep), "found": n(Play::Found) })
            }),
            "lower": made.map(|made| f32::from_bits(made.key.lower)),
            "folder_colors": made.map(|made| made.key.folders.len()),
        })
    }
}

impl Workspace {
    /// The empty place drawn on `floor`: a pane's, or a tab's (the main window's, or one out).
    pub(crate) fn at_of(&self, floor: Floor) -> At {
        match floor {
            Floor::Pane(id) => At::Pane(id),
            Floor::Out(tab) => At::Tab(tab),
            Floor::Main => At::Tab(self.model.tabs[self.model.main_at()].id),
        }
    }

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
        let exclude: Exclude = crate::config::live().city_exclude.clone().unwrap_or_default().into_iter().collect();
        let mut on_show: HashMap<String, Asked> = HashMap::new();
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
            let exclude = exclude.clone();
            let lit = &mut on_show.entry(city.dir.to_string()).or_insert_with(|| (roots, exclude, Vec::new())).2;
            for one in &city.lit {
                let one = (one.label.to_string(), one.running);
                if !lit.contains(&one) {
                    lit.push(one);
                }
            }
        }
        // A checkout read with files to show: its GPU opens before its first picture is asked.
        if on_show.keys().any(|dir| self.city.held.contains_key(dir)) {
            self.city_painter.warm();
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
        let Some(mut city) = self.model.city_at(at) else { return lattice };
        let view = self.eased_view(at, city.view);
        let easing = self.city_ease.get().is_some_and(|(of, ..)| of == at);
        // Its camera moving: the user's drag of it that has moved, a reset easing back, or a
        // wheel or trackpad step under `WHEELING` ago (`city_wheel` draws again after it).
        let wheeled = self.city_wheeled.get().is_some_and(|(of, when)| of == at && when.elapsed() < WHEELING);
        let moving = easing || wheeled || self.model.camera_moving(at);
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
        let glimpses = std::mem::take(&mut city.glimpses);
        let floating: Vec<Floating> = (city.agents.iter())
            .filter_map(|agent| {
                // A label two files share (`a/src/main.rs`, `b/src/main.rs`): at no known file.
                let at = agent.at.and_then(|label| {
                    let mut places = plan.as_ref()?.labelled(label, &folder);
                    let one = places.next()?.id.clone();
                    places.next().is_none().then_some(one)
                });
                // At no file, the first path its shell line names that is a building or a plinth.
                let at = at.or_else(|| {
                    let places = &plan.as_ref()?.places;
                    agent
                        .near
                        .iter()
                        .find_map(|path| places.iter().find(|place| place.id == *path))
                        .map(|p| p.id.clone())
                });
                let (busy, subagents) = (agent.busy, agent.subagents);
                Some(match agent.who {
                    Who::Task(task) => {
                        let subject = ottospace_core::talk::Subject::Task(task);
                        let mono = badges.iter().find(|(of, _)| *of == subject)?.1.clone();
                        let title = self.model.tasks.live(task)?.title.clone();
                        let (key, colour) = (format!("task:{task}"), theme::identity(agent.colour));
                        Floating { task: Some(task), key, colour, mono, title, at, busy, subagents }
                    }
                    // An agent CLI in a pane, by its process: its runner's glyph, no title of its own.
                    Who::Pane { pane, runner } => {
                        let spec = ottospace_core::runner::RunnerId::named(runner)?.spec();
                        let (key, colour) = (format!("pane:{pane}"), theme::task(Group::Running));
                        let (mono, title) = (spec.glyph.to_string(), spec.display.to_string());
                        Floating { task: None, key, colour, mono, title, at, busy, subagents }
                    }
                    // A session no task has: its runner's glyph, in the colour of work going on.
                    Who::Session { runner, id, title } => {
                        let spec = ottospace_core::runner::RunnerId::named(runner)?.spec();
                        let title = title.unwrap_or(spec.display).to_string();
                        let (key, colour) = (format!("session:{runner}:{id}"), theme::task(Group::Running));
                        Floating { task: None, key, colour, mono: spec.glyph.to_string(), title, at, busy, subagents }
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
                if easing {
                    window.request_animation_frame();
                }
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
                let view = [view.yaw, view.pitch, view.zoom, view.pan[0], view.pan[1]].map(f32::to_bits);
                // `developer.city_frame`: where the fitted city's middle sits, a share of the height.
                let frame = crate::config::live().developer.city_frame;
                let lower = frame.filter(|f| (0.3..=0.8).contains(f)).map_or(scene::LOWER, |f| f - 0.5);
                let mut key = plan.as_ref().map(|plan| Key {
                    dir,
                    plan: Arc::as_ptr(plan) as usize,
                    lit,
                    size: (size(w), size(h)),
                    rise: 1000,
                    view,
                    lower: lower.to_bits(),
                    folders: crate::config::live().city_folder_colors.clone().unwrap_or_default().into_iter().collect(),
                    glass: !crate::config::live().developer.city_opaque,
                    moving,
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
                // Each picture asked for once: not made, not on its way, not failed. A camera
                // that moved on leaves the picture of where it was to be skipped (`latest_each`).
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
                paint(&drawn, floor, home.borrow().hints_at(floor), &floating, &glimpses, window, cx)
            },
        );
        // A press on an agent's icon: the user's own goes to its task (`Model::city_agent_click`).
        let picture = div().absolute().size_full().child(picture.absolute().size_full()).on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                this.city_click(floor, e.position, window, cx)
            }),
        );
        let backdrop = if shows {
            picture.into_any_element()
        } else {
            div().size_full().child(lattice).child(picture).into_any_element()
        };
        // The user's pointer moves its camera: a middle drag pans, a left drag past a click
        // turns it, the wheel zooms. A moved one has a quiet way back at its corner (a double
        // click is kept for going into a file).
        let id = match floor {
            Floor::Main => "tab".to_string(),
            Floor::Pane(id) => id.to_string(),
            Floor::Out(tab) => format!("out{tab}"),
        };
        let moved = self.model.city_view(at) != View::default();
        let reset = moved.then(|| {
            let button = crate::ui::button(
                SharedString::from(format!("city-reset-{floor:?}")),
                crate::ui::Button::Ghost(crate::ui::Face::Label("Reset view".into())),
                crate::ui::State::Rest,
            );
            div().absolute().bottom(px(8.)).right(px(8.)).child(
                button.child(self.controls.mark(format!("city:reset:{id}"))).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.city_ease.set(Some((at, this.model.city_view(at), std::time::Instant::now())));
                        this.city_step(at, CameraStep::Reset, window, cx);
                    }),
                ),
            )
        });
        div()
            .id(SharedString::from(format!("city-{floor:?}")))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.city_press(floor, at, e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.city_press(floor, at, e, window, cx)),
            )
            .on_scroll_wheel(
                cx.listener(move |this, e: &ScrollWheelEvent, window, cx| this.city_wheel(floor, at, e, window, cx)),
            )
            .child(backdrop)
            .children(reset)
            .into_any_element()
    }

    /// The view a place's picture is made from: the model's, or on the way back to it after a
    /// reset (eased, frames asked and counted; none under Reduce Motion).
    fn eased_view(&self, at: At, to: View) -> View {
        let Some((_, from, since)) = self.city_ease.get().filter(|(of, ..)| *of == at) else { return to };
        let t = since.elapsed().as_secs_f32() / EASE.as_secs_f32();
        if t >= 1. || crate::dial::calm() {
            self.city_ease.set(None);
            return to;
        }
        self.city_frames.set(self.city_frames.get() + 1);
        // Ease out, the short way round.
        let k = 1. - (1. - t).powi(3);
        let turn = (to.yaw - from.yaw + 540.).rem_euclid(360.) - 180.;
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        View {
            yaw: from.yaw + turn * k,
            pitch: lerp(from.pitch, to.pitch),
            zoom: lerp(from.zoom, to.zoom),
            pan: [lerp(from.pan[0], to.pan[0]), lerp(from.pan[1], to.pan[1])],
        }
        .clamped()
    }

    /// The camera a place's picture shows, and where it is: for the ground under the pointer.
    fn city_camera(&self, floor: Floor, at: At) -> Option<(Camera, Bounds<Pixels>, [f32; 3])> {
        let drawn = self.city_drawn.borrow();
        let place = drawn.places.get(&floor)?;
        let fit = drawn.picture(place)?.scene.fit?;
        let b = place.bounds;
        let aspect = f32::from(b.size.width) / f32::from(b.size.height);
        Some((fit.camera(&self.model.city_view(at), aspect), b, fit.target))
    }

    /// The point of the plan under `p` (window points), as the camera now sees it.
    fn city_ground(&self, floor: Floor, at: At, p: Point<Pixels>) -> Option<[f32; 2]> {
        let (camera, b, _) = self.city_camera(floor, at)?;
        let share = |v: Pixels, from: Pixels, size: Pixels| f32::from(v - from) / f32::from(size);
        camera.ground(share(p.x, b.left(), b.size.width), share(p.y, b.top(), b.size.height))
    }

    fn city_step(&mut self, at: At, step: CameraStep, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::CityCamera { at, step }, self.input, window, cx);
    }

    /// Whether `at` is over `floor`'s hints (and the room kept around them).
    fn over_hints(&self, floor: Floor, at: Point<Pixels>) -> bool {
        let hints = self.home.borrow().hints_at(floor).dilate(px(CLEAR));
        hints.size.width > px(0.) && hints.contains(&at)
    }

    /// A press on a city: over the hints, theirs. A middle press grabs it to pan; a left press is
    /// a click (the pane takes focus) until it moves past `CLICK_SLOP`.
    fn city_press(&mut self, floor: Floor, at: At, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.input != Origin::User || self.over_hints(floor, e.position) || self.city_spin.is_some() {
            return;
        }
        // Over its lattice (no picture: no GPU, too small, not read yet) there is no camera.
        let on = self.city_camera(floor, at).is_some_and(|(_, bounds, _)| bounds.contains(&e.position));
        if !on {
            return;
        }
        let moving = e.button == MouseButton::Middle;
        self.city_drag = Some(CityDrag { floor, at, button: e.button, from: e.position, last: e.position, moving });
        if moving {
            self.city_step(at, CameraStep::Grab, window, cx);
        }
    }

    /// The pointer moved with a city's drag held: a left drag turns the camera (past a click),
    /// a middle one moves the ground under the pointer with it.
    pub(crate) fn city_drag_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.city_drag else { return };
        if e.pressed_button != Some(drag.button) {
            return self.city_release(window, cx);
        }
        let (dx, dy) = (f32::from(e.position.x - drag.last.x), f32::from(e.position.y - drag.last.y));
        let far = (e.position - drag.from).magnitude() as f32;
        if !drag.moving && far > CLICK_SLOP {
            drag.moving = true;
            self.city_step(drag.at, CameraStep::Grab, window, cx);
        }
        if drag.moving {
            let step = match drag.button {
                MouseButton::Middle => {
                    let (Some(a), Some(b)) = (
                        self.city_ground(drag.floor, drag.at, drag.last),
                        self.city_ground(drag.floor, drag.at, e.position),
                    ) else {
                        drag.last = e.position;
                        self.city_drag = Some(drag);
                        return;
                    };
                    CameraStep::Pan { dx: a[0] - b[0], dz: a[1] - b[1] }
                }
                _ => CameraStep::Orbit { yaw: -dx * ORBIT_PER_POINT, pitch: dy * ORBIT_PER_POINT },
            };
            self.city_step(drag.at, step, window, cx);
        }
        drag.last = e.position;
        self.city_drag = Some(drag);
    }

    /// The button went up: the drag ends where it is.
    pub(crate) fn city_release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(drag) = self.city_drag.take().filter(|drag| drag.moving) {
            self.city_step(drag.at, CameraStep::Let, window, cx);
        }
    }

    /// Esc during a drag of a city: the camera back where the drag began. The user's key only.
    pub(crate) fn city_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.city_drag.filter(|drag| drag.moving && self.input == Origin::User) else { return false };
        self.city_drag = None;
        self.city_step(drag.at, CameraStep::Cancel, window, cx);
        true
    }

    /// The wheel over a city: a mouse wheel's lines zoom about the point under the pointer; a
    /// trackpad's scroll pans (the city follows the fingers), with ⌥ turns, with ⌘ zooms (GPUI
    /// 0.2.2 delivers no pinch or rotate gesture).
    fn city_wheel(&mut self, floor: Floor, at: At, e: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Over the hints, theirs, as a press.
        if self.input != Origin::User || self.over_hints(floor, e.position) {
            return;
        }
        let about = |this: &Self| this.city_about(floor, at, e.position);
        let step = match e.delta {
            ScrollDelta::Lines(d) => {
                about(self).map(|about| CameraStep::Zoom { factor: ZOOM_PER_LINE.powf(-d.y), about })
            }
            ScrollDelta::Pixels(d) => {
                let (dx, dy) = (f32::from(d.x), f32::from(d.y));
                if e.modifiers.alt {
                    Some(CameraStep::Orbit { yaw: -dx * ORBIT_PER_POINT, pitch: dy * ORBIT_PER_POINT })
                } else if e.modifiers.platform {
                    about(self).map(|about| CameraStep::Zoom { factor: ZOOM_PER_POINT.powf(-dy), about })
                } else {
                    let a = self.city_ground(floor, at, e.position);
                    let b = self.city_ground(floor, at, e.position + d);
                    a.zip(b).map(|(a, b)| CameraStep::Pan { dx: a[0] - b[0], dz: a[1] - b[1] })
                }
            }
        };
        if let Some(step) = step {
            self.city_stepped(at, step, window, cx);
            cx.stop_propagation();
        }
    }

    /// The point a zoom keeps where it is: under `position`, in plan units from the city's middle;
    /// above the horizon (a low pitch), the point looked at (the view's pan, kept).
    fn city_about(&self, floor: Floor, at: At, position: Point<Pixels>) -> Option<[f32; 2]> {
        let (_, _, target) = self.city_camera(floor, at)?;
        let Some(p) = self.city_ground(floor, at, position) else { return Some(self.model.city_view(at).pan) };
        Some([p[0] - target[0], p[1] - target[2]])
    }

    /// A step of the wheel or a pinch: moving until the steps stop, then drawn once more, full size
    /// and named.
    fn city_stepped(&mut self, at: At, step: CameraStep, window: &Window, cx: &mut Context<Self>) {
        self.city_wheeled.set(Some((at, Instant::now())));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(WHEELING + Duration::from_millis(20)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        self.city_step(at, step, window, cx);
    }

    /// A pinch at `position`, `nearer` times nearer (the native hook's: GPUI has none), over the
    /// city of this window it is on, not over its hints. The user's alone.
    pub(crate) fn city_pinch(&mut self, position: Point<Pixels>, nearer: f32, window: &Window, cx: &mut Context<Self>) {
        let id = window.window_handle().window_id();
        let found = {
            let drawn = self.city_drawn.borrow();
            let over = |(_, place): &(&Floor, &Place)| place.window == Some(id) && place.bounds.contains(&position);
            drawn.places.iter().find(over).map(|(floor, _)| *floor)
        };
        let Some(floor) = found.filter(|floor| self.input == Origin::User && !self.over_hints(*floor, position)) else {
            return;
        };
        let at = self.at_of(floor);
        // The city's zoom is a share of the distance: under 1, nearer.
        if let Some(about) = self.city_about(floor, at, position) {
            self.city_stepped(at, CameraStep::Zoom { factor: nearer.recip(), about }, window, cx);
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
        let icon = (self.city_drawn.borrow().places.get(&floor)).and_then(|place| {
            place.icons.iter().rev().find(|icon| icon.rect.contains(&at)).map(|icon| (icon.task, icon.key.clone()))
        });
        let Some((task, key)) = icon else { return };
        cx.stop_propagation();
        // A session's icon is keyed `session:<runner>:<id>`; a pane's agent has none of these.
        let session = key.strip_prefix("session:").and_then(|rest| rest.split_once(':'));
        let who = match (task, session) {
            (Some(task), _) => Who::Task(task),
            (None, Some((runner, id))) => Who::Session { runner, id, title: None },
            (None, None) => Who::Pane { pane: 0, runner: "" },
        };
        if let Some(action) = self.model.city_agent_click(who, self.input) {
            self.dispatch(action, self.input, window, cx);
        }
        cx.notify();
    }

    /// For the state of a test run: the city of the first empty place on show that draws one
    /// (the main window's tab, else its empty panes by id, else a folded-out tab); null with none.
    pub(crate) fn city_snapshot(&self) -> serde_json::Value {
        let mut floors: Vec<(Floor, At)> = (self.model.empty_places().into_iter())
            .filter(|at| self.model.city_at(*at).is_some_and(|city| self.city.held.contains_key(city.dir)))
            .map(|at| (self.floor_of(at), at))
            .collect();
        floors.sort_by_key(|(floor, _)| match floor {
            Floor::Main => (0, 0),
            Floor::Pane(id) => (1, *id),
            Floor::Out(tab) => (2, *tab),
        });
        // The first that drew a picture, else the first laid out.
        let drawn = self.city_drawn.borrow();
        let painted =
            |(floor, _): &&(Floor, At)| drawn.places.get(floor).is_some_and(|place| drawn.picture(place).is_some());
        let first =
            floors.iter().find(painted).or_else(|| floors.iter().find(|(floor, _)| drawn.places.contains_key(floor)));
        let Some((floor, at)) = first else { return serde_json::Value::Null };
        let mut snapshot = drawn.snapshot(*floor, self.city.came);
        snapshot["read_us"] = self.city.read_us.into();
        // The places too small for a city, drawing their lattice.
        let small = floors.iter().filter(|(floor, _)| drawn.places.get(floor).is_some_and(|place| place.small));
        snapshot["small"] = small.map(|(floor, _)| on(*floor)).collect::<Vec<_>>().into();
        // Its camera, as the core keeps it, and whether a drag of one is in flight.
        let v = self.model.city_view(*at);
        snapshot["view"] = serde_json::json!({ "yaw": v.yaw, "pitch": v.pitch, "zoom": v.zoom, "pan": v.pan });
        snapshot["grabbed"] = self.model.camera_grabbed().into();
        snapshot["spin"] = serde_json::json!({ "running": self.city_spin.is_some(), "result": self.city_spun });
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

    /// Frames a busy agent's pulse asked for: for the counts of a test run.
    pub(crate) fn city_pulse_frames(&self) -> u64 {
        self.city_drawn.borrow().pulse_frames
    }

    /// Frames the reads' and searches' effects asked for: for the counts of a test run.
    pub(crate) fn city_glimmer_frames(&self) -> u64 {
        self.city_drawn.borrow().glimmer_frames
    }
}

/// A spin in flight (the developer's timed turn, `CameraStep::Spin`): its place, the steps made,
/// when the last frame began, each frame's time from the one before (ms), how many frames showed
/// the picture the frame before showed, and the picture shown then.
pub(crate) struct Spin {
    floor: Floor,
    at: At,
    step: usize,
    /// Its frames are asked for (`city_spin_frames`), from the frame after it began.
    asked: bool,
    last: Option<Instant>,
    frames: Vec<f32>,
    repeats: usize,
    shown: Option<Key>,
}

/// A spin's numbers, as the toast, the event log and a test run's state give them: no name or path.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct Spun {
    pub(crate) steps: usize,
    pub(crate) done: bool,
    pub(crate) frames: usize,
    pub(crate) mean: f32,
    pub(crate) p50: f32,
    pub(crate) p95: f32,
    pub(crate) p99: f32,
    pub(crate) max: f32,
    pub(crate) over_16: usize,
    pub(crate) over_33: usize,
    /// Frames that showed the same picture as the frame before, while the camera turned.
    pub(crate) repeats: usize,
    /// A new picture's wait from made to painted (ms): median, p95 and the longest.
    pub(crate) wait_p50: f32,
    pub(crate) wait_p95: f32,
    pub(crate) wait_max: f32,
    pub(crate) boxes: usize,
}

impl Workspace {
    /// Spins the city on show once round, a step a frame, timing each frame: the user's own
    /// (the palette's Developer command; a test run's `city_spin`). Esc stops it.
    pub(crate) fn city_spin_start(&mut self, origin: Origin, window: &Window, cx: &mut Context<Self>) {
        if origin != Origin::User || self.city_spin.is_some() || self.city_drag.is_some() {
            return;
        }
        // In the main window: its frames time the spin.
        let places = self.model.empty_places().into_iter().map(|at| (self.floor_of(at), at));
        let first = places.filter(|(floor, _)| !matches!(floor, Floor::Out(_))).find(|(floor, _)| {
            let drawn = self.city_drawn.borrow();
            drawn.places.get(floor).is_some_and(|place| drawn.picture(place).is_some())
        });
        let Some((floor, at)) = first else {
            self.told = Some(("No city on show to spin".into(), Instant::now()));
            return cx.notify();
        };
        self.dispatch(Action::CityCamera { at, step: CameraStep::Grab }, origin, window, cx);
        self.city_drawn.borrow_mut().waits = Some((floor, Vec::new()));
        let spin = Spin { floor, at, step: 0, asked: false, last: None, frames: Vec::new(), repeats: 0, shown: None };
        self.city_spin = Some(spin);
        cx.notify();
    }

    /// A spin begun: its first frame asked for, from a frame's render (which has the window).
    pub(crate) fn city_spin_frames(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(spin) = self.city_spin.as_mut().filter(|spin| !spin.asked) {
            spin.asked = true;
            cx.on_next_frame(window, |this, window, cx| this.city_spin_frame(window, cx));
        }
    }

    /// A frame of a spin: its time from the last, whether the last showed a new picture, then
    /// the next step; after the last, the camera back where it was and the numbers told.
    fn city_spin_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut spin) = self.city_spin.take() else { return };
        // A frame asked for by a spin Esc ended: the new one asks its own.
        if !spin.asked {
            self.city_spin = Some(spin);
            return;
        }
        let now = Instant::now();
        if let Some(last) = spin.last {
            spin.frames.push((now - last).as_secs_f32() * 1e3);
        }
        spin.last = Some(now);
        let shown = self.city_drawn.borrow().places.get(&spin.floor).and_then(|place| place.shown.clone());
        // The frame before showed what the one before it did: the turn stood still on screen.
        if spin.step > 1 && shown.is_some() && shown == spin.shown {
            spin.repeats += 1;
        }
        spin.shown = shown;
        // Its city covered (a pane opened over it) or gone: stopped, not done.
        if self.model.city_at(spin.at).is_none() {
            return self.city_spin_end(spin, false, window, cx);
        }
        if spin.step == SPIN_STEPS {
            return self.city_spin_end(spin, true, window, cx);
        }
        spin.step += 1;
        self.dispatch(Action::CityCamera { at: spin.at, step: CameraStep::Spin }, Origin::User, window, cx);
        self.city_spin = Some(spin);
        cx.on_next_frame(window, |this, window, cx| this.city_spin_frame(window, cx));
        cx.notify();
    }

    /// A spin ends (all its steps, or Esc): its camera back, its numbers in a toast, the event
    /// log and a test run's state.
    fn city_spin_end(&mut self, spin: Spin, done: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Action::CityCamera { at: spin.at, step: CameraStep::Cancel }, Origin::User, window, cx);
        let mut frames = spin.frames;
        frames.sort_by(f32::total_cmp);
        let mut waits = self.city_drawn.borrow_mut().waits.take().map(|(_, waits)| waits).unwrap_or_default();
        waits.sort_by(f32::total_cmp);
        let boxes = {
            let drawn = self.city_drawn.borrow();
            drawn
                .places
                .get(&spin.floor)
                .and_then(|place| drawn.picture(place))
                .map_or(0, |made| made.scene.blocks.len())
        };
        let n = frames.len();
        let spun = Spun {
            steps: spin.step,
            done,
            frames: n,
            mean: if n == 0 { 0. } else { frames.iter().sum::<f32>() / n as f32 },
            p50: rank(&frames, 50),
            p95: rank(&frames, 95),
            p99: rank(&frames, 99),
            max: frames.last().copied().unwrap_or(0.),
            over_16: frames.iter().filter(|f| **f > 16.7).count(),
            over_33: frames.iter().filter(|f| **f > 33.).count(),
            repeats: spin.repeats,
            wait_p50: rank(&waits, 50),
            wait_p95: rank(&waits, 95),
            wait_max: waits.last().copied().unwrap_or(0.),
            boxes,
        };
        let text = format!(
            "Spin{}: {n} frames, mean {:.1} ms, p95 {:.1}, p99 {:.1}, max {:.1}; over 16.7 ms {}, over 33 ms {}; {} repeated",
            if done { "" } else { " stopped" },
            spun.mean,
            spun.p95,
            spun.p99,
            spun.max,
            spun.over_16,
            spun.over_33,
            spun.repeats
        );
        crate::event_log::event("city_spin", serde_json::to_value(&spun).unwrap_or_default());
        self.told = Some((text, Instant::now()));
        self.city_spun = Some(spun);
        cx.notify();
    }

    /// Esc during a spin stops it, the camera back where it was.
    pub(crate) fn city_spin_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(spin) = self.city_spin.take_if(|_| self.input == Origin::User) else { return false };
        self.city_spin_end(spin, false, window, cx);
        true
    }
}

/// For the state of a test run: a point of the picture (window points) over a folder's own
/// ground (an outer district's), and one over a building's top, for a test's pointer; each null
/// with none. A few of each are tried: the state is asked often.
fn spots(made: &Made, [x, y, w, h]: [f32; 4]) -> serde_json::Value {
    let s = &made.scene;
    let hits = |i: usize, (px, py): (f32, f32)| (s.hit(px, py) == Some(i)).then(|| [x + px * w, y + py * h]);
    let folder = (s.blocks.iter().enumerate()).filter(|(_, b)| b.kind == Kind::Plate(1)).take(8).find_map(|(i, _)| {
        // From the middle of its near edge toward its middle: the kerb first.
        let [l, r] = s.front_edge(i)?;
        let o = s.outline(i)?;
        let mid = (o.iter().map(|p| p.0).sum::<f32>() / 4., o.iter().map(|p| p.1).sum::<f32>() / 4.);
        let near = ((l.0 + r.0) / 2., (l.1 + r.1) / 2.);
        (1..10).find_map(|k| {
            let t = k as f32 / 20.;
            hits(i, (near.0 + (mid.0 - near.0) * t, near.1 + (mid.1 - near.1) * t))
        })
    });
    let camera = s.camera;
    let file = (s.blocks.iter().enumerate()).filter(|(_, b)| b.kind == Kind::Building).take(64).find_map(|(i, b)| {
        let (tx, ty) = camera?.project(b.top())?;
        hits(i, (tx, ty + 1. / h))
    });
    serde_json::json!({ "folder": folder, "file": file })
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
    glimpses: &[Glimpse],
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let mut drawn = drawn.borrow_mut();
    let Some(place) = drawn.places.get_mut(&floor) else { return };
    // Nothing of an earlier paint is on show until this one draws it.
    let kept: Vec<usize> = place.names.drain(..).map(|(key, ..)| key).collect();
    (place.beacon, place.window) = (None, Some(window.window_handle().window_id()));
    place.lit_beams.clear();
    let (bounds, hovered, pointer) = (place.bounds, place.hovered, place.pointer);
    let was = std::mem::take(&mut place.icons);
    let mut glimmer = std::mem::take(&mut place.glimmer);
    let (waited, fade, first_ms, laid) = (place.waited, place.fade, place.first_ms, place.since);
    let drawn = &mut *drawn;
    let made = drawn.places.get(&floor).and_then(|place| drawn.picture(place));
    // A frame of the rise rendered too slowly: no rise for this city, its standing picture fades in.
    // The GPU's time: the layout (made once, before the first frame) and names (none while it
    // rises) are no frame's cost.
    let slow = made.is_some_and(|made| made.key.rise < 1000 && made.gpu_us > RISE_BUDGET_US);
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
    // Its first paint: a picture shown again (kept, a tab come back) is no new one.
    let new = !made.painted;
    let wait = made.made_at.elapsed().as_secs_f32() * 1e3;
    let painted = Instant::now();
    let _ = window.paint_image(bounds, Corners::default(), made.image.clone(), 0, false);
    let us = painted.elapsed().as_micros() as u64;
    crate::trace::record("city_paint", serde_json::json!({ "us": us, "new": new, "wait_us": (wait * 1e3) as u64 }));

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
    // Names by `city_labels`: the things large enough on screen, the largest shaped, placed with
    // no overlap and clear of the hints. One with a line break is not named: `shape_line` takes one.
    let at = |(x, y): (f32, f32)| [x * w, y * h];
    let local = |b: Bounds<Pixels>| {
        let o = b.origin - bounds.origin;
        [f32::from(o.x), f32::from(o.y), f32::from(b.size.width), f32::from(b.size.height)]
    };
    let avoid: Vec<[f32; 4]> = (hints.size.width > px(0.)).then(|| local(clear)).into_iter().collect();
    let mut marks: Vec<(&Mark, [f32; 4])> = (made.marks.iter())
        .filter(|m| !m.text.contains(char::is_control))
        .map(|m| (m, [m.bounds[0] * w, m.bounds[1] * h, m.bounds[2] * w, m.bounds[3] * h]))
        .filter(|(_, b)| city_labels::sized(*b, [w, h]))
        .collect();
    let extent = |b: &[f32; 4]| b[2].max(b[3]);
    // In `city_labels::place`'s order: the largest first, whatever its tier.
    marks.sort_by(|a, b| extent(&b.1).total_cmp(&extent(&a.1)).then(a.0.place.cmp(&b.0.place)));
    marks.truncate(SHAPED);
    let style_of = |tier: Tier| match tier {
        Tier::Wall => (theme::TEXT_XS, Hsla::from(rgb(theme::MUTED))),
        Tier::Folder => (theme::TEXT_2XS, Hsla::from(rgb(theme::MUTED))),
        Tier::File => (theme::TEXT_2XS, Hsla::from(rgb(theme::MUTED)).opacity(0.8)),
    };
    let lines: Vec<gpui::ShapedLine> =
        marks.iter().map(|(m, _)| shape(&m.text, style_of(m.tier).1, style_of(m.tier).0, window)).collect();
    let candidates: Vec<Candidate> = (marks.iter().zip(&lines))
        .map(|((m, b), line)| Candidate {
            key: m.place,
            tier: m.tier,
            bounds: *b,
            top: at(m.top),
            face: m.face.map(at),
            size: [f32::from(line.width), style_of(m.tier).0 + 4.],
        })
        .collect();
    let mut names = Vec::new();
    for (key, r) in city_labels::place(&candidates, [w, h], &avoid, &kept) {
        let Some(i) = candidates.iter().position(|c| c.key == key) else { continue };
        let origin = bounds.origin + point(px(r[0]), px(r[1]));
        if lines[i].paint(origin, px(r[3]), window, cx).is_ok() {
            names.push((key, candidates[i].tier, Bounds { origin, size: gpui::size(px(r[2]), px(r[3])) }));
        }
    }
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
            paint_beam(window, foot, end, Hsla::from(rgb(colour)).opacity(0.85), 1., 2., false);
            lit_beams.push([foot.x, foot.y, end.x, end.y].map(f32::from));
        }
    }
    let glimmering = glance(&mut glimmer, made, glimpses, bounds, clear, window);
    let (icons, gliding, pulsing) = agents(made, floating, &was, bounds, clear, window, cx, &shape);
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
    // A folder's has no beam: none worked out for it.
    let folder = |i: usize| made.scene.blocks.get(i).is_some_and(|b| matches!(b.kind, Kind::Plate(_)));
    let beacon = hovered.and_then(|i| {
        let beam = if folder(i) { None } else { beam_of(i) };
        beacon(made, i, beam, pointer, alpha, bounds, clear, window, cx, &shape)
    });
    let fades = beacon.is_some() && alpha < 1.;
    if fades {
        window.request_animation_frame();
    }
    if gliding || pulsing || glimmering {
        window.request_animation_frame();
    }
    drawn.paints += 1;
    if let Some((_, waits)) = drawn.waits.as_mut().filter(|(of, _)| new && *of == floor) {
        waits.push(wait);
    }
    drawn.beam_frames += u64::from(fades);
    drawn.fade_frames += u64::from(fading_in);
    drawn.rise_frames += u64::from(rising);
    drawn.agent_frames += u64::from(gliding);
    drawn.pulse_frames += u64::from(pulsing);
    drawn.glimmer_frames += u64::from(glimmering);
    if let Some(made) = drawn.pictures.get_mut(&shown) {
        (made.used, made.painted) = (Instant::now(), true);
    }
    if let Some(place) = drawn.places.get_mut(&floor) {
        (place.names, place.beacon, place.shown) = (names, beacon, Some(shown));
        (place.fading, place.lit_beams) = (since, lit_beams);
        (place.fade, place.first_ms) = (fade, first_ms);
        (place.icons, place.hovered_agent, place.glimmer) = (icons, hovered_agent, glimmer);
    }
}

/// The box of the atlas place `id` in a picture's scene: a building, or a folder's plinth.
fn block_of(made: &Made, id: &str) -> Option<usize> {
    let at = made.plan.places.iter().position(|place| place.id == id)?;
    made.scene.blocks.iter().position(|block| block.place == Some(at))
}

/// The agents' reads and searches (`Glimpse`), each played once, on the picture as it is (no
/// render): a read flashes its building's top; a search sweeps a band of light across what it
/// covers (its folders' plinths, the whole ground when unscoped), and the files its result names
/// stay lit a moment. Glimpses `TOGETHER` play as one; one sweep at a time; none under Reduce
/// Motion. Returns whether one still plays.
fn glance(
    g: &mut Glimmer,
    made: &Made,
    glimpses: &[Glimpse],
    bounds: Bounds<Pixels>,
    clear: Bounds<Pixels>,
    window: &mut Window,
) -> bool {
    let (now, calm) = (Instant::now(), crate::dial::calm());
    // A folder with no plinth of its own (its one child stands for it): its buildings'.
    let blocks_of = |path: &str| -> Vec<usize> {
        let one = block_of(made, path);
        let under = |id: &str| id.strip_prefix(path).is_some_and(|rest| rest.starts_with('/'));
        let inside = (made.plan.places.iter().filter(|place| !place.group && under(&place.id)))
            .filter_map(|place| block_of(made, &place.id));
        one.map_or_else(|| inside.collect(), |one| vec![one])
    };
    g.effects.retain(|e| now.checked_duration_since(e.since).is_none_or(|d| d < e.play.length()));
    for glimpse in glimpses {
        let search = if glimpse.search { Play::Sweep } else { Play::Flash };
        let whole = glimpse.search && glimpse.scope.is_empty();
        let found = format!("{}#found", glimpse.id);
        for (key, play, paths, whole) in
            [(glimpse.id.clone(), search, &glimpse.scope, whole), (found, Play::Found, &glimpse.found, false)]
        {
            // A result not known yet is not played yet.
            if play == Play::Found && paths.is_empty() {
                continue;
            }
            if !g.see(key) || !g.began || calm {
                continue;
            }
            // A result names files: its paths exactly (a folder's buildings only for a scope).
            let mut blocks: Vec<usize> = match play {
                Play::Found => paths.iter().filter_map(|path| block_of(made, path)).collect(),
                _ => paths.iter().flat_map(|path| blocks_of(path)).collect(),
            };
            blocks.sort_unstable();
            blocks.dedup();
            if blocks.is_empty() && !whole {
                continue;
            }
            let colour = theme::identity(glimpse.colour);
            // One ripple: the same play, in the same colour, begun `TOGETHER` ago at most.
            let together = |e: &Effect| {
                e.play == play
                    && e.colour == colour
                    && now.checked_duration_since(e.since).is_some_and(|d| d < TOGETHER)
            };
            if let Some(e) = g.effects.iter_mut().find(|e| together(e)) {
                e.blocks.extend(blocks);
                e.blocks.sort_unstable();
                e.blocks.dedup();
                e.whole |= whole;
                continue;
            }
            // One sweep at a time: the next begins as the last ends (at most one waits).
            let last = g.effects.iter().filter(|e| e.play == Play::Sweep).map(|e| e.since + SWEEP).max();
            let since = if play == Play::Sweep { last.map_or(now, |end| end.max(now)) } else { now };
            if play == Play::Sweep && g.effects.iter().filter(|e| e.play == Play::Sweep).count() >= 2 {
                continue;
            }
            g.effects.push(Effect { play, blocks, whole, colour, since });
        }
    }
    g.began = true;
    // Reduce Motion turned on mid-play: nothing more plays.
    if calm {
        g.effects.clear();
    }
    let Some(camera) = made.scene.camera else { return false };
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let at = |(x, y): (f32, f32)| bounds.origin + point(px(x * w), px(y * h));
    let face = |i: usize| made.scene.outline(i).map(|corners| corners.map(at));
    let ground = || {
        let b = made.scene.blocks.iter().find(|b| b.kind == Kind::Ground)?;
        let corners =
            [[b.min[0], 0., b.min[2]], [b.max[0], 0., b.min[2]], [b.max[0], 0., b.max[2]], [b.min[0], 0., b.max[2]]];
        let [a, b, c, d] = corners.map(|p| camera.project(p).map(at));
        Some([a?, b?, c?, d?])
    };
    let fill_face = |face: &[Point<Pixels>; 4], colour: Hsla, window: &mut Window| {
        let mut path = PathBuilder::fill();
        path.move_to(face[0]);
        for corner in &face[1..] {
            path.line_to(*corner);
        }
        path.close();
        crate::dial::stroke(window, path, colour);
    };
    // A found file's light: full, then fading over its last 40%.
    let fading = |t: f32| 1. - ((t - 0.6) / 0.4).clamp(0., 1.);
    let to_window = |p: [f32; 3]| camera.project(p).map(at);
    for e in &g.effects {
        // A sweep waiting its turn draws nothing yet.
        let Some(run) = now.checked_duration_since(e.since) else { continue };
        let t = (run.as_secs_f32() / e.play.length().as_secs_f32()).min(1.);
        let colour = Hsla::from(rgb(e.colour));
        let faces = e.blocks.iter().filter_map(|i| face(*i)).chain(ground().filter(|_| e.whole));
        for f in faces {
            match e.play {
                Play::Flash => fill_face(&f, colour.opacity(0.6 * (1. - t)), window),
                Play::Found => fill_face(&f, colour.opacity(0.45 * fading(t)), window),
                // A band across the face, from one side to the other, brightest midway.
                Play::Sweep => {
                    let (s0, s1) = ((t * 1.2 - 0.2).clamp(0., 1.), (t * 1.2).clamp(0., 1.));
                    let lerp = |p: Point<Pixels>, q: Point<Pixels>, s: f32| p + (q - p) * s;
                    let band = [lerp(f[0], f[1], s0), lerp(f[0], f[1], s1), lerp(f[3], f[2], s1), lerp(f[3], f[2], s0)];
                    fill_face(&band, colour.opacity(0.4 * (t * std::f32::consts::PI).sin()), window);
                }
            }
        }
        // A found file's top alone is a few pixels at most zooms: it gets a lit file's steady beam.
        if e.play == Play::Found {
            // Buildings only: a lit one has its beam already, a folder none.
            let building = |i: usize| made.scene.blocks.get(i).is_some_and(|b| b.kind == Kind::Building);
            for &i in e.blocks.iter().filter(|i| building(**i)) {
                let Some((foot, end)) = made.scene.beam(i) else { continue };
                let (Some(foot), Some(end)) = (to_window(foot), to_window(end)) else { continue };
                if let Some((end, _)) = fit_beam(foot, end, None, bounds, clear) {
                    paint_beam(window, foot, end, colour.opacity(0.85 * fading(t)), 1., 2., false);
                }
            }
        }
    }
    !g.effects.is_empty()
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
/// hovering over the city's middle (above the hints if they cover it, else under) with no cone. Lowered along the beam
/// until clear of the hints and inside the picture, else not drawn. One that moves glides there (at once under Reduce Motion).
/// A busy agent pulses (none under Reduce Motion): the pool on its building, or at no file a
/// halo round its icon, swelling and fading once every `PULSE`.
/// Returns the icons drawn, whether one still glides, and whether one pulses.
fn agents(
    made: &Made,
    floating: &[Floating],
    was: &[Icon],
    bounds: Bounds<Pixels>,
    clear: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::App,
    shape: &dyn Fn(&str, Hsla, f32, &mut Window) -> gpui::ShapedLine,
) -> (Vec<Icon>, bool, bool) {
    let Some(camera) = made.scene.camera else { return (Vec::new(), false, false) };
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let to_window = |p: [f32; 3]| camera.project(p).map(|(x, y)| bounds.origin + point(px(x * w), px(y * h)));
    let boxes: Vec<Option<usize>> = floating.iter().map(|agent| block_of(made, agent.at.as_deref()?)).collect();
    let (calm, now) = (crate::dial::calm(), Instant::now());
    let side = gpui::size(px(ICON), px(ICON));
    let (mut gliding, mut pulsing) = (false, false);
    let mut icons = Vec::new();
    // 0 to 1 and back, one clock for every icon so two busy ones pulse together.
    let beat = {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let t = START.get_or_init(Instant::now).elapsed().as_secs_f32() / PULSE.as_secs_f32();
        0.5 - 0.5 * (t * std::f32::consts::TAU).cos()
    };
    for (agent, mark) in floating.iter().zip(made.scene.marks(&boxes)) {
        let Some(end) = to_window(mark.at) else { continue };
        let block = mark.over.and_then(|i| made.scene.blocks.get(i));
        let foot = block.and_then(|block| to_window(block.top()));
        let offset = point(px((mark.fan.0 as f32 - (mark.fan.1 as f32 - 1.) / 2.) * (ICON + FAN_GAP)), px(0.));
        // Its sub-agents' row (`SUBS` at most; fanned, as many as its room in the fan) under it.
        let most = if mark.fan.1 > 1 { ((ICON + FAN_GAP + SUB_GAP) / (SUB + SUB_GAP)) as usize } else { SUBS };
        let below = if agent.subagents > 0 { px(3. + SUB) } else { px(0.) };
        let fits = |at: Point<Pixels>| {
            let rect = Bounds::centered_at(at, side);
            let rect = Bounds { size: gpui::size(rect.size.width, rect.size.height + below), ..rect };
            (!rect.intersects(&clear) && bounds.contains(&rect.origin) && bounds.contains(&rect.bottom_right()))
                .then_some(at)
        };
        // At no file: whole inside the picture's top, then above the hints if they cover it, else
        // under (a workspace's empty tab keeps 560 points clear for them).
        let hover = point(end.x, end.y.max(bounds.top() + px(ICON / 2. + 1.)));
        let above = point(hover.x, hover.y.min(clear.top() - px(ICON / 2. + 1.)));
        let under = point(hover.x, hover.y.max(clear.bottom() + px(ICON / 2. + 1.)));
        let to = match foot {
            Some(f) => [1., 0.75, 0.5, 0.25].into_iter().find_map(|t: f32| fits(f + (end - f) * t + offset)),
            None => [hover, above, under].into_iter().find_map(|at| fits(at + offset)),
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
        let pulse = (agent.busy && !calm).then_some(beat);
        pulsing |= pulse.is_some();
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
            crate::dial::stroke(window, pool, colour.opacity(0.3 + 0.3 * pulse.unwrap_or(0.)));
        } else if let Some(p) = pulse {
            let halo = rect.dilate(px(2. + 6. * p));
            let ring = fill(halo, gpui::transparent_black()).corner_radii(px(ICON / 4. + 4.));
            window.paint_quad(ring.border_widths(px(2.)).border_color(colour.opacity(0.6 * (1. - p))));
        }
        // The task's badge, as drawn on its row: its letters in a square bordered in its colour.
        window.paint_quad(
            fill(rect, rgb(theme::SELECTED)).corner_radii(px(ICON / 4.)).border_widths(px(2.)).border_color(colour),
        );
        let mono = shape(&agent.mono, Hsla::from(rgb(theme::TEXT)), (ICON * 0.4).max(9.), window);
        let _ = mono.paint(point(shown.x - mono.width / 2., shown.y - px(7.)), px(14.), window, cx);
        let (task, key, cone) = (agent.task, agent.key.clone(), corners.is_some());
        // Its sub-agents: small squares in its colour in a row under it, then `+n`.
        let shown_subs = agent.subagents.min(most);
        let row = shown_subs as f32 * (SUB + SUB_GAP) - SUB_GAP;
        for k in 0..shown_subs {
            let x = shown.x - px(row / 2.) + px(k as f32 * (SUB + SUB_GAP));
            let sub = Bounds { origin: point(x, rect.bottom() + px(3.)), size: gpui::size(px(SUB), px(SUB)) };
            window.paint_quad(fill(sub, colour.opacity(0.85)).corner_radii(px(2.)));
        }
        if agent.subagents > most {
            let more =
                shape(&format!("+{}", agent.subagents - most), Hsla::from(rgb(theme::TEXT)), theme::TEXT_XS, window);
            let at = point(shown.x + px(row / 2. + 3.), rect.bottom() + px(3. + SUB / 2. - 7.));
            let _ = more.paint(at, px(14.), window, cx);
        }
        let subagents = agent.subagents;
        icons.push(Icon {
            task,
            key,
            shown,
            from,
            to,
            since,
            rect,
            cone,
            fan: mark.fan,
            pulsing: pulse.is_some(),
            subagents,
        });
    }
    (icons, gliding, pulsing)
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
        let o = bounds.origin;
        let (dot, view) = ([end.x - o.x, end.y - o.y], [bounds.size.width, bounds.size.height]);
        let avoid = [[clear.origin.x - o.x, clear.origin.y - o.y, clear.size.width, clear.size.height].map(f32::from)];
        let size_of = [f32::from(size.width), f32::from(size.height)];
        let r = city_labels::beside(dot.map(f32::from), size_of, BESIDE, view.map(f32::from), &avoid)?;
        Some((end, Some(Bounds { origin: o + point(px(r[0]), px(r[1])), size })))
    })
}

/// A beam: a line from `foot` up to `end`, faint at the box and brightest at its dot (as light
/// falling from it), and the dot; with `glow`, a soft halo round both (the hovered one).
#[allow(clippy::too_many_arguments)]
fn paint_beam(
    window: &mut Window,
    foot: Point<Pixels>,
    end: Point<Pixels>,
    colour: Hsla,
    width: f32,
    dot: f32,
    glow: bool,
) {
    const STEPS: usize = 6;
    if glow {
        let mut halo = PathBuilder::stroke(px(width * 4.));
        halo.move_to(foot);
        halo.line_to(end);
        crate::dial::stroke(window, halo, colour.opacity(colour.a * 0.18));
        crate::dial::disc(window, end, dot * 2.6, colour.opacity(colour.a * 0.22));
    }
    for step in 0..STEPS {
        let (a, b) = (step as f32 / STEPS as f32, (step + 1) as f32 / STEPS as f32);
        let mut line = PathBuilder::stroke(px(width));
        line.move_to(foot + (end - foot) * a);
        line.line_to(foot + (end - foot) * b);
        crate::dial::stroke(window, line, colour.opacity(0.15 + 0.85 * b * b));
    }
    crate::dial::disc(window, end, dot, colour);
}

/// The hovered box's beacon, and beside it a label: the file's path in the checkout, or the
/// district's, cut in the middle if long. A file's is its beam (`beam`: foot to end, window
/// points) in the text's colour with the label by its dot, never over the hints (`fit_beam`); a
/// folder's is the outline of its top face, its label under its near edge or at the `pointer`
/// (`city_labels::callout`). `alpha`: how far it has faded in.
#[allow(clippy::too_many_arguments)]
fn beacon(
    made: &Made,
    i: usize,
    beam: Option<(Point<Pixels>, Point<Pixels>)>,
    pointer: Option<Point<Pixels>>,
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
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let to_window = |(x, y): (f32, f32)| bounds.origin + point(px(x * w), px(y * h));
    // A folder: its outline, none if the camera cuts its top face (then its label by the pointer).
    let folder = matches!(block.kind, Kind::Plate(_));
    let outline = folder.then(|| made.scene.outline(i)).flatten().map(|o| o.map(to_window));
    let beam = if folder { None } else { Some(beam?) };
    // One line, as wide as the wider side of the dot allows (a folder's: two thirds of the city):
    // `src/…/views/list.rs`.
    let most = match beam {
        Some((foot, end)) => {
            (bounds.right() - foot.x.max(end.x)).max(foot.x.min(end.x) - bounds.left()) - px(BESIDE + 12.)
        }
        None => bounds.size.width * (2. / 3.),
    };
    let most = most.max(px(40.));
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
    let (label, beam, outline) = match beam {
        Some((foot, end)) => {
            let (end, label) = fit_beam(foot, end, Some(size), bounds, clear)?;
            paint_beam(window, foot, end, colour, 2., 3.5, true);
            (label?, Some([foot.x, foot.y, end.x, end.y].map(f32::from)), None)
        }
        None => {
            // Its outline in the accent colour, a soft halo under it.
            let cyan = Hsla::from(rgb(theme::PRIMARY)).opacity(alpha);
            for (width, colour) in [(5., cyan.opacity(0.2 * alpha)), (1.5, cyan)] {
                let Some(corners) = outline else { break };
                let mut path = PathBuilder::stroke(px(width));
                path.move_to(corners[3]);
                corners.iter().for_each(|c| path.line_to(*c));
                crate::dial::stroke(window, path, colour);
            }
            let local = |p: Point<Pixels>| [f32::from(p.x - bounds.origin.x), f32::from(p.y - bounds.origin.y)];
            let avoid = [
                local(clear.origin)[0],
                local(clear.origin)[1],
                f32::from(clear.size.width),
                f32::from(clear.size.height),
            ];
            let at = pointer.map_or_else(|| local(bounds.center()), local);
            let size_of = [f32::from(size.width), f32::from(size.height)];
            let r = city_labels::callout(outline.map(|o| o.map(local)), size_of, at, [w, h], &[avoid]);
            let label = Bounds { origin: bounds.origin + point(px(r[0]), px(r[1])), size };
            (label, None, outline.map(|o| o.map(|c| [f32::from(c.x), f32::from(c.y)])))
        }
    };
    let backing = Hsla::from(rgb(theme::BACKGROUND)).opacity(0.8 * alpha);
    window.paint_quad(fill(label, backing).corner_radii(px(theme::RADIUS_MD)));
    let _ = line.paint(label.origin + point(px(6.), px(2.)), px(14.), window, cx);
    Some(Beacon { file: !place.group, lit: matches!(block.kind, Kind::Lit(_)), beam, outline, label })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checkout's folders left out (built in, and the setting's) are not in its plan, nor
    /// counted: the plan is as if they were never there.
    #[test]
    fn folders_left_out_are_not_in_the_plan() {
        let scratch = crate::scratch::Scratch::dir(&format!("ottospace-city-exclude-{}", std::process::id()));
        let top = scratch.as_ref().join("repo");
        for (file, text) in [("src/a.rs", "x"), ("notes/archive/old.md", "y"), ("node_modules/m/i.js", "z")] {
            std::fs::create_dir_all(top.join(file).parent().unwrap()).unwrap();
            std::fs::write(top.join(file), text).unwrap();
        }
        // Tracked: a global ignore file of the user's leaves none out.
        for args in [&["init", "-q"][..], &["add", "-f", "."]] {
            let git = std::process::Command::new("git").arg("-C").arg(&top).args(args).status().unwrap();
            assert!(git.success());
        }
        let top = top.canonicalize().unwrap();
        let roots = [top.to_string_lossy().into_owned()];
        let ids = |exclude: &[&str]| {
            let exclude: Exclude = vec![("*".into(), exclude.iter().map(|e| e.to_string()).collect())];
            read(&top, &roots, &exclude, &|| false)
                .map(|atlas| atlas.places.into_iter().map(|p| p.id).collect::<Vec<_>>())
        };
        assert_eq!(ids(&["notes/archive"]), Some(vec!["src".into(), "src/a.rs".into()]));
        // A key that is this checkout's path: its own entries too.
        let own: Exclude = vec![(top.to_string_lossy().into_owned(), vec!["src".into()])];
        let left = read(&top, &roots, &own, &|| false).map(|a| a.places.into_iter().map(|p| p.id).collect::<Vec<_>>());
        assert!(left.is_some_and(|ids| !ids.iter().any(|id| id.starts_with("src"))));
        // Its place closed while it read: no plan.
        assert!(read(&top, &roots, &[], &|| true).is_none());
        let all = ids(&["!node_modules"]).unwrap();
        assert!(
            all.contains(&"node_modules/m/i.js".to_string()) && all.contains(&"notes/archive/old.md".into()),
            "{all:?}"
        );
    }

    /// A made-up repository the same every run: `files` files in about `folders` folders up to
    /// `deepest` deep, most near the top; sizes most small, a few large. `scripts/shot`'s
    /// `made_up_repo` makes the same tree on disk.
    pub(crate) fn made_up_repo(files: usize, folders: usize, deepest: usize) -> Vec<(String, u64)> {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut dirs: Vec<(String, usize)> = vec![(String::new(), 0)];
        for k in 1..folders {
            let parent = loop {
                let u = next();
                let p = (dirs.len() as f64 * u * u) as usize;
                if dirs[p].1 < deepest {
                    break p;
                }
            };
            let (path, depth) = dirs[parent].clone();
            dirs.push((format!("{path}d{k}/"), depth + 1));
        }
        (0..files)
            .map(|i| {
                let dir = &dirs[(dirs.len() as f64 * next().powf(1.5)) as usize].0;
                (format!("{dir}f{i}.rs"), 200 + (next().powf(3.) * 60_000.) as u64)
            })
            .collect()
    }

    /// The pictures of a turn of the city, as the picture's thread makes them while the user's drag
    /// turns it, at a Retina tab's size: each stage's time per picture, as a distribution, and the
    /// one-off costs (plan, layout, boxes, first picture, names, hit test, memory). Sizes: the
    /// owner's repository's (9,500 files) and a config repository's (50,000), or `CITY_SIZES`
    /// (`files:folders:deepest`, `,` between). Needs a GPU; no window.
    /// `cargo test -p ottospace --release -- --ignored a_turn_is_timed --nocapture`.
    #[test]
    #[ignore = "needs a GPU; prints timings"]
    fn a_turn_is_timed() {
        let gpu = Gpu::new().expect("a GPU");
        gpu.warm();
        let sizes = std::env::var("CITY_SIZES").unwrap_or_else(|_| "9500:900:8,50000:6800:9".into());
        for size in sizes.split(',') {
            let n: Vec<usize> = size.split(':').map(|n| n.parse().expect("files:folders:deepest")).collect();
            let rss_before = rss_mb();
            let started = Instant::now();
            let files = made_up_repo(n[0], n[1], n[2]);
            let listed_ms = started.elapsed().as_secs_f64() * 1e3;
            let folders: HashSet<&str> = files.iter().filter_map(|(p, _)| p.rsplit_once('/').map(|(d, _)| d)).collect();
            let bytes: usize = files.iter().map(|(p, _)| p.len() + 1).sum();
            let started = Instant::now();
            let plan = Arc::new(atlas::atlas(&atlas::repo(&files)));
            let atlas_ms = started.elapsed().as_secs_f64() * 1e3;
            println!(
                "\n{} files, {} folders, {bytes} bytes of names; made up in {listed_ms:.0} ms; plan (atlas) {atlas_ms:.1} ms",
                files.len(),
                folders.len()
            );
            drop(files);
            // On the main thread: a lit file's lookup each frame, a read again compared with the
            // plan held, and the old plan let go.
            let t = Instant::now();
            let found = plan.labelled("d3/f12.rs", "made-up").count();
            let lookup_ms = t.elapsed().as_secs_f64() * 1e3;
            let again = Arc::new((*plan).clone());
            let t = Instant::now();
            let same = *again == *plan;
            let compare_ms = t.elapsed().as_secs_f64() * 1e3;
            let t = Instant::now();
            drop(again);
            let drop_ms = t.elapsed().as_secs_f64() * 1e3;
            println!(
                "main thread (ms): a lit file's lookup {lookup_ms:.2} ({found} found), a read again compared {compare_ms:.1} ({same}), a plan let go {drop_ms:.1}"
            );
            turn(&gpu, &plan, rss_before);
        }
    }

    /// This process's resident size, in MB.
    fn rss_mb() -> f64 {
        let pid = std::process::id().to_string();
        let out = std::process::Command::new("/bin/ps").args(["-o", "rss=", "-p", &pid]).output();
        let out = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        out.parse::<f64>().unwrap_or(0.) / 1024.
    }

    fn print_stage(name: &str, v: &mut [f64]) {
        if v.is_empty() {
            return;
        }
        v.sort_by(f64::total_cmp);
        let p = |q: f64| v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1];
        let over = |ms: f64| v.iter().filter(|t| **t > ms).count();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        println!(
            "| {name} | {mean:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {} | {} |",
            p(0.5),
            p(0.95),
            p(0.99),
            v[v.len() - 1],
            over(16.7),
            over(33.)
        );
    }

    fn turn(gpu: &Gpu, plan: &Arc<Atlas>, rss_before: f64) {
        let (w, h) = (2560u32, 1500u32);
        let mut key = Key {
            dir: "made-up".into(),
            plan: Arc::as_ptr(plan) as usize,
            lit: Vec::new(),
            size: (w, h),
            glass: true,
            rise: 1000,
            view: [0; 5],
            lower: scene::LOWER.to_bits(),
            folders: Vec::new(),
            moving: false,
        };
        // The one-off stages apart: layout, folder colours, boxes (as `make` does them once).
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let layout = scene::layout(plan, &[], w as f32 / h as f32, scene::LOWER);
        let layout_ms = ms(t);
        let t = Instant::now();
        let tints = city_colours::tints(plan, &layout.blocks, &FolderColours::built_in());
        let tints_ms = ms(t);
        let mut fitted = layout.scene(&View::default());
        fitted.tints = tints;
        let t = Instant::now();
        drop(Boxes::new(&fitted, true));
        let boxes_ms = ms(t);
        drop(layout);
        let set = |key: &mut Key, v: View| key.view = [v.yaw, v.pitch, v.zoom, v.pan[0], v.pan[1]].map(f32::to_bits);
        let mut last = Layouts::new();
        let mut view = View::default();
        set(&mut key, view);
        let skip = crate::trace::jsonl().lines().count();
        let at_rest = Instant::now();
        let first = make(gpu, &key, plan, &mut last).unwrap();
        let rest_ms = ms(at_rest);
        let blocks = first.scene.blocks.len();
        let districts = first.scene.blocks.iter().filter(|b| matches!(b.kind, Kind::Plate(_))).count();
        // What a fitted picture shows: buildings under a pixel wide or deep on screen.
        let camera = first.scene.camera.unwrap();
        let px = |b: &scene::Block| camera.bounds(b).map(|r| (r[2] * w as f32, r[3] * h as f32));
        let buildings: Vec<(f32, f32)> =
            first.scene.blocks.iter().filter(|b| b.kind == Kind::Building).filter_map(px).collect();
        let under = |n: f32| buildings.iter().filter(|(bw, bh)| bw.min(*bh) < n).count();
        // Names at rest: the things that may be named, then placed (as `paint`, names ~6 px a letter).
        let t = Instant::now();
        let mut marks: Vec<(&Mark, [f32; 4])> = (first.marks.iter())
            .map(|m| {
                (m, [m.bounds[0] * w as f32, m.bounds[1] * h as f32, m.bounds[2] * w as f32, m.bounds[3] * h as f32])
            })
            .filter(|(_, b)| city_labels::sized(*b, [w as f32, h as f32]))
            .collect();
        marks.sort_by(|a, b| b.1[2].max(b.1[3]).total_cmp(&a.1[2].max(a.1[3])));
        marks.truncate(SHAPED);
        let at = |(x, y): (f32, f32)| [x * w as f32, y * h as f32];
        let candidates: Vec<Candidate> = (marks.iter())
            .map(|(m, b)| Candidate {
                key: m.place,
                tier: m.tier,
                bounds: *b,
                top: at(m.top),
                face: m.face.map(at),
                size: [6. * m.text.len() as f32, 14.],
            })
            .collect();
        let placed = city_labels::place(&candidates, [w as f32, h as f32], &[], &[]).len();
        let names_ms = ms(t);
        // Hover: the box under the pointer, at 200 points of the picture.
        let t = Instant::now();
        let hits = (0..200)
            .filter(|i| first.scene.hit((i % 20) as f32 / 20. + 0.02, (i / 20) as f32 / 10. + 0.05).is_some())
            .count();
        let hit_ms = ms(t) / 200.;
        let rss_city = rss_mb();
        // GPU bytes kept: instances, and the targets of the half-size picture (4× colour and depth,
        // resolve, read buffer).
        let (hw, hh) = (u64::from(w / 2), u64::from(h / 2));
        let gpu_mb = (blocks as u64 * 64 + hw * hh * 4 * (4 + 4 + 1 + 1)) as f64 / 1048576.;
        let mut stages: HashMap<&str, Vec<f64>> = HashMap::new();
        let mut kept = Some(first);
        key.moving = true;
        for _ in 0..240 {
            view = view.orbited(1.5, 0.);
            set(&mut key, view);
            let started = Instant::now();
            let made = make(gpu, &key, plan, &mut last).unwrap();
            let total = ms(started);
            // The picture it replaces is let go, as `read_city` retires it.
            let dropped = Instant::now();
            drop(kept.replace(made));
            stages.entry("drop old").or_default().push(ms(dropped));
            stages.entry("picture total").or_default().push(total);
        }
        let trace = crate::trace::jsonl();
        let events = |skip: usize, w: u64| -> Vec<serde_json::Value> {
            (trace.lines().skip(skip))
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter(|e| e["ev"] == "city_picture" && e["w"] == w)
                .collect()
        };
        let pictures = events(skip, u64::from(w / 2));
        let marks_ms: f64 = (trace.lines().skip(skip))
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|e| e["ev"] == "city_marks")
            .and_then(|e| e["us"].as_f64())
            .unwrap_or(0.)
            / 1e3;
        for field in ["scene_us", "boxes_us", "alloc_us", "gpu_us", "copy_us", "image_us"] {
            let v = pictures.iter().filter_map(|e| e[field].as_f64()).map(|us| us / 1e3).collect();
            stages.insert(field.trim_end_matches("_us"), v);
        }
        // The same turn with opaque buildings: the GPU's time without glass.
        key.glass = false;
        let skip = crate::trace::jsonl().lines().count();
        for _ in 0..60 {
            view = view.orbited(1.5, 0.);
            set(&mut key, view);
            drop(make(gpu, &key, plan, &mut last).unwrap());
        }
        let trace = crate::trace::jsonl();
        let opaque: Vec<f64> = (trace.lines().skip(skip))
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|e| e["ev"] == "city_picture" && e["w"] == u64::from(w / 2) && e["rise"] == 1000)
            .skip(1)
            .filter_map(|e| e["gpu_us"].as_f64().map(|us| us / 1e3))
            .collect();
        stages.insert("gpu, no glass", opaque);
        println!("{blocks} boxes ({districts} districts, {} buildings)", buildings.len());
        println!(
            "once (ms): layout {layout_ms:.1}, folder colours {tints_ms:.1}, boxes {boxes_ms:.1}; first picture at rest {rest_ms:.1} \
             (of it, what may be named {marks_ms:.1}); names placed {names_ms:.1} ({placed} of {} candidates)",
            candidates.len()
        );
        println!(
            "hover: {hit_ms:.2} ms a hit test ({hits} of 200 on a box); memory: RSS +{:.0} MB, GPU ~{gpu_mb:.0} MB",
            rss_city - rss_before
        );
        println!(
            "fitted picture {w}×{h}: buildings under 1 px {} ({:.0}%), under 2 px {}, under 4 px {}",
            under(1.),
            100. * under(1.) as f64 / buildings.len().max(1) as f64,
            under(2.),
            under(4.)
        );
        println!("turn: 240 pictures at {}×{} (half size), ms", w / 2, h / 2);
        println!("| stage | mean | p50 | p95 | p99 | max | over 16.7 | over 33 |");
        println!("|---|---|---|---|---|---|---|---|");
        for name in ["scene", "boxes", "alloc", "gpu", "gpu, no glass", "copy", "image", "drop old", "picture total"] {
            if let Some(v) = stages.get_mut(name) {
                print_stage(name, v);
            }
        }
    }

    /// How long a checkout's read takes, for the design note's costs. `CITY_BENCH`: folders,
    /// `:` between them. `cargo test -p ottospace --release -- --ignored a_read_is_timed --nocapture`.
    #[test]
    #[ignore = "prints timings of the folders CITY_BENCH names; run it by name, in release"]
    fn a_read_is_timed() {
        for dir in std::env::var("CITY_BENCH").unwrap_or_default().split(':').filter(|dir| !dir.is_empty()) {
            let top = Path::new(dir).canonicalize().unwrap();
            let roots = [top.to_string_lossy().into_owned()];
            let started = Instant::now();
            let places = read(&top, &roots, &[], &|| false).map(|atlas| atlas.places.len());
            let took = started.elapsed();
            // Its tab closed as it read: how long until the read lets go.
            let at = Instant::now();
            drop(read(&top, &roots, &[], &|| true));
            println!("{dir}: a read whose place closed ends in {:.0} ms", at.elapsed().as_secs_f64() * 1e3);
            println!("{dir}: {places:?} places in {:.1} ms", took.as_secs_f64() * 1000.);
        }
    }
}
