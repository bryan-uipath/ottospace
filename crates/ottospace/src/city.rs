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
use std::time::Duration;

use gpui::{
    AnyElement, Bounds, Context, Corners, Hsla, Pixels, Point, RenderImage, TextRun, Window, WindowId, canvas, div,
    fill, point, prelude::*, px, rgb,
};
use ottospace_core::atlas::{self, Atlas};
use ottospace_core::empty::{self, Backdrop, Surface};
use ottospace_core::model::Origin;
use ottospace_core::model::city::At;
use ottospace_core::scene::{self, Kind, Scene};
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
/// A hovered building's beacon: its line rises this many points above the building's top.
const BEACON: f32 = 36.;
/// Room kept between a name and the empty tab's hints, in points.
const CLEAR: f32 = 12.;
/// The hints' backing fades out over this many points, in this many layers: no edge to see.
const FEATHER: f32 = 40.;
const LAYERS: usize = 8;

/// What a folder's read was asked with: the folders git may run in, and the files lit then,
/// each with whether its call ran (a file an agent wrote may be new, once the call ends).
type Asked = (Vec<String>, Vec<(String, bool)>);

/// Reads checkouts' files on a thread of its own: a folder when it comes on show, or the files
/// its agents are at change. Every folder asked and not read yet goes in each job, so the
/// newest job (the thread takes only it) leaves none out. No timer of its own.
pub struct Reader {
    jobs: Sender<Vec<(String, Vec<String>)>>,
    reads: Receiver<Vec<(String, Option<Atlas>)>>,
    asked: HashMap<String, Asked>,
    pending: HashMap<String, Vec<String>>,
    held: HashMap<String, Arc<Atlas>>,
    /// How many reads came: for the state of a test run.
    came: usize,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, reads) = crate::child::newest(|dirs: Vec<(String, Vec<String>)>| {
            dirs.into_iter().map(|(dir, roots)| (dir.clone(), read(Path::new(&dir), &roots))).collect()
        });
        let (asked, pending, held) = (HashMap::new(), HashMap::new(), HashMap::new());
        Self { jobs, reads, asked, pending, held, came: 0 }
    }

    /// The folders on show, each with what it is asked with: one whose roots or lit files
    /// changed is read again. A folder no longer on show is forgotten, and read again when it
    /// comes back.
    fn ask(&mut self, on_show: HashMap<String, Asked>) {
        let mut changed = false;
        for (dir, asked) in &on_show {
            if self.asked.get(dir) != Some(asked) {
                self.pending.insert(dir.clone(), asked.0.clone());
                changed = true;
            }
        }
        self.pending.retain(|dir, _| on_show.contains_key(dir));
        self.asked = on_show;
        if changed {
            let _ = self.jobs.send(self.pending.iter().map(|(dir, roots)| (dir.clone(), roots.clone())).collect());
        }
    }

    /// Takes the reads that came. True if one did.
    fn take(&mut self) -> bool {
        let came: Vec<(String, Option<Atlas>)> = self.reads.try_iter().flatten().collect();
        let any = !came.is_empty();
        for (dir, read) in came {
            self.came += 1;
            self.pending.remove(&dir);
            if self.held.len() >= KEPT && !self.held.contains_key(&dir) {
                self.held.clear();
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
    let late = std::time::Instant::now() + TIMEOUT;
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
        if std::time::Instant::now() > late {
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

/// A picture asked for: the folder, its plan, the lit files (place ids, colours) and the size
/// in pixels. Empty places with one key share one picture.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    dir: String,
    plan: usize,
    lit: Vec<(String, u32)>,
    size: (u32, u32),
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
}

/// One empty place's city, as the last frame drew it.
#[derive(Default)]
struct Place {
    /// The picture it asks for, and the one it painted last (shown, stretched, until the asked
    /// one is made).
    asked: Option<Key>,
    shown: Option<Key>,
    /// Too small for a city (`empty::CITY_LEAST`): its lattice instead.
    small: bool,
    /// Of the last paint: where, in window points, in which window, and how many names.
    bounds: Bounds<Pixels>,
    window: Option<WindowId>,
    labels: usize,
    /// The box the user's pointer is over (an index into the scene's blocks), where that pointer
    /// was (found again in a new picture), and the beacon's label as the last paint drew it:
    /// whether a file's, whether lit, where.
    hovered: Option<usize>,
    pointer: Option<Point<Pixels>>,
    beacon: Option<(bool, bool, Bounds<Pixels>)>,
}

/// The pictures made and what each empty place drew: for the state of a test run, no name or
/// path in it.
#[derive(Default)]
pub struct Drawn {
    pictures: HashMap<Key, Made>,
    /// Sent to the picture's thread and not made yet, each with its plan.
    pending: HashMap<Key, Arc<Atlas>>,
    places: HashMap<Floor, Place>,
    /// Why there is no picture: no GPU, or the render failed. The lattice is drawn instead.
    failed: Option<String>,
    renders: usize,
    paints: usize,
    /// Pictures no place shows: GPUI keeps each in the window's atlas until it is dropped.
    retired: Vec<Arc<RenderImage>>,
}

pub type Shared = Rc<RefCell<Drawn>>;

/// Renders pictures on a thread of its own: each job is every picture asked and not made, and
/// the thread takes only the newest. The GPU is opened at the first and kept; none (or
/// `OTTOSPACE_CITY_NO_GPU` in a test run) and every picture fails.
pub struct Painter {
    jobs: Sender<Vec<(Key, Arc<Atlas>)>>,
    made: Receiver<Vec<(Key, Result<Made, String>)>>,
}

impl Painter {
    pub fn new() -> Self {
        let gpu: RefCell<Option<Result<Gpu, String>>> = RefCell::new(None);
        let (jobs, made) = crate::child::newest(move |wanted: Vec<(Key, Arc<Atlas>)>| {
            let mut made = Vec::new();
            for (key, plan) in wanted {
                let mut held = gpu.borrow_mut();
                let opened = held.get_or_insert_with(|| {
                    let refused = std::env::var_os("OTTOSPACE_E2E").and(std::env::var_os("OTTOSPACE_CITY_NO_GPU"));
                    if refused.is_some() { Err("no GPU (test)".into()) } else { Gpu::new() }
                });
                // A panic in the GPU's code fails this picture, and the GPU is opened again next time.
                let picture = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    opened.as_ref().map_err(Clone::clone).and_then(|gpu| make(gpu, &key, &plan))
                }));
                let picture = picture.unwrap_or_else(|_| {
                    *held = None;
                    Err("the render panicked".into())
                });
                made.push((key, picture));
            }
            made
        });
        Self { jobs, made }
    }
}

/// The scene of `plan` and its picture, on the picture's thread.
fn make(gpu: &Gpu, key: &Key, plan: &Arc<Atlas>) -> Result<Made, String> {
    let started = std::time::Instant::now();
    let lit: Vec<(&str, u32)> = key.lit.iter().map(|(id, colour)| (id.as_str(), *colour)).collect();
    let (w, h) = key.size;
    let scene = scene::scene(plan, &lit, w as f32 / h as f32);
    let Picture { width, height, bgra } = gpu.render(&scene, w, h)?;
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
    let (places, plan) = (plan.places.len(), plan.clone());
    Ok(Made { key: key.clone(), image, lit, names, scene, plan, places, groups, render_us })
}

impl Drawn {
    /// The picture `place` paints: the one it asks for once made, else the one it showed, if of
    /// the same folder. None with no GPU, or a place too small.
    fn picture(&self, place: &Place) -> Option<&Made> {
        let dir = &place.asked.as_ref()?.dir;
        let made = [&place.asked, &place.shown].into_iter().flatten().find_map(|key| self.pictures.get(key));
        made.filter(|made| self.failed.is_none() && !place.small && made.key.dir == *dir)
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
    fn snapshot(&self, floor: Floor, reads: usize, scale: f32) -> serde_json::Value {
        let place = self.places.get(&floor);
        let made = place.and_then(|place| self.picture(place));
        let [x, y, w, h] = rect(place.map(|place| place.bounds).unwrap_or_default());
        let lit: Vec<[f32; 4]> = (made.iter().flat_map(|made| &made.lit))
            .map(|(r, _)| [x + r[0] * w, y + r[1] * h, r[2] * w, r[3] * h])
            .collect();
        let tops: Vec<[f32; 2]> =
            (made.iter().flat_map(|made| &made.lit)).map(|(_, (tx, ty))| [x + tx * w, y + ty * h]).collect();
        let beacon = place.and_then(|place| place.beacon);
        let hovered = beacon.map(|(file, lit, at)| serde_json::json!({ "file": file, "lit": lit, "beacon": rect(at) }));
        let on = match floor {
            Floor::Pane(id) => serde_json::json!({ "pane": id }),
            Floor::Out(tab) => serde_json::json!({ "out": tab }),
            Floor::Main => serde_json::json!("tab"),
        };
        serde_json::json!({
            "on": on, "places": made.map(|made| made.places), "groups": made.map(|made| made.groups),
            "picture": made.map(|made| made.key.size), "bounds": [x, y, w, h], "scale": scale,
            "lit": lit, "labels": place.map_or(0, |place| place.labels), "failed": self.failed, "reads": reads,
            "renders": self.renders, "paints": self.paints, "render_us": made.map(|made| made.render_us),
            "hovered": hovered, "lit_tops": tops, "pictures": self.pictures.len(),
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
        let roots = self.model.checkout_roots();
        let mut on_show: HashMap<String, (Vec<String>, Vec<(String, bool)>)> = HashMap::new();
        let mut floors = HashSet::new();
        for at in self.model.empty_places() {
            let Some(city) = self.model.city_at(at) else { continue };
            floors.insert(self.floor_of(at));
            let lit = &mut on_show.entry(city.dir.to_string()).or_insert_with(|| (roots.clone(), Vec::new())).1;
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
        let made: Vec<(Key, Result<Made, String>)> = self.city_painter.made.try_iter().flatten().collect();
        for (key, picture) in made {
            drawn.renders += 1;
            drawn.pending.remove(&key);
            redraw = true;
            let asked = drawn.places.values().any(|place| place.asked.as_ref() == Some(&key));
            match picture {
                Ok(made) => {
                    drawn.failed = None;
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
                // A failure of a picture no longer asked for says nothing.
                Err(why) if asked => drawn.failed = Some(why),
                Err(_) => {}
            }
        }
        // Places gone are forgotten; a picture no place asks for or shows is dropped.
        drawn.places.retain(|floor, _| floors.contains(floor));
        let used: HashSet<Key> =
            drawn.places.values().flat_map(|place| [place.asked.clone(), place.shown.clone()]).flatten().collect();
        let unused: Vec<Key> = drawn.pictures.keys().filter(|key| !used.contains(*key)).cloned().collect();
        for key in unused {
            if let Some(old) = drawn.pictures.remove(&key) {
                drawn.retired.push(old.image);
            }
        }
        drawn.pending.retain(|key, _| used.contains(key));
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
    pub(crate) fn render_city(&self, floor: Floor, at: At, lattice: AnyElement) -> AnyElement {
        let Some(city) = self.model.city_at(at) else { return lattice };
        let Some(plan) = self.city.held.get(city.dir).cloned() else { return lattice };
        // The checkout's own name: a file in its root is labelled with it.
        let folder =
            Path::new(city.dir).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let lit: Vec<(String, u32)> = (city.lit.iter())
            .flat_map(|lit| {
                let colour = theme::identity(lit.colour);
                plan.labelled(lit.label, &folder).map(move |place| (place.id.clone(), colour)).collect::<Vec<_>>()
            })
            .collect();
        let surface = match floor {
            Floor::Pane(_) => Surface::Pane,
            Floor::Out(_) => Surface::OutTab,
            Floor::Main => Surface::Tab,
        };
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
                let key = Key { dir, plan: Arc::as_ptr(&plan) as usize, lit, size: (size(w), size(h)) };
                let place = held.places.entry(floor).or_default();
                (place.bounds, place.small) = (bounds, small);
                let wanted = !small && key.size.0 > 0 && key.size.1 > 0;
                place.asked = wanted.then(|| key.clone());
                if wanted && !held.pictures.contains_key(&key) && held.pending.insert(key, plan).is_none() {
                    let plans: Vec<(Key, Arc<Atlas>)> =
                        held.pending.iter().map(|(key, plan)| (key.clone(), plan.clone())).collect();
                    if jobs.send(plans).is_err() {
                        held.failed = Some("the picture's thread ended".into());
                    }
                }
                drop(held);
                drawn
            },
            // The hints as this frame laid them out.
            move |_, drawn: Shared, window, cx| paint(&drawn, floor, home.borrow().hints_at(floor), window, cx),
        );
        let picture = picture.absolute().size_full();
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
            let hovered = pointer.and_then(|at| drawn.under(floor, at, home.borrow().hints_at(floor)));
            if let Some(place) = drawn.places.get_mut(&floor) {
                place.pointer = pointer;
                changed |= std::mem::replace(&mut place.hovered, hovered) != hovered;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// For the state of a test run: the city of the first empty place on show that draws one
    /// (the main window's tab, else its empty panes by id, else a folded-out tab); null with none.
    pub(crate) fn city_snapshot(&self, scale: f32) -> serde_json::Value {
        let mut floors: Vec<Floor> = (self.model.empty_places().into_iter())
            .filter(|at| self.model.city_at(*at).is_some_and(|city| self.city.held.contains_key(city.dir)))
            .map(|at| self.floor_of(at))
            .collect();
        floors.sort_by_key(|floor| match floor {
            Floor::Main => (0, 0),
            Floor::Pane(id) => (1, *id),
            Floor::Out(tab) => (2, *tab),
        });
        let first = floors.first().filter(|floor| self.city_drawn.borrow().places.contains_key(floor));
        first.map_or(serde_json::Value::Null, |floor| self.city_drawn.borrow().snapshot(*floor, self.city.came, scale))
    }

    /// File lists read so far, a city on show or not: for the counts of a test run.
    pub(crate) fn city_reads(&self) -> usize {
        self.city.came
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
fn paint(drawn: &Shared, floor: Floor, hints: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
    let mut drawn = drawn.borrow_mut();
    let Some(place) = drawn.places.get_mut(&floor) else { return };
    // Nothing of an earlier paint is on show until this one draws it.
    (place.beacon, place.labels, place.window) = (None, 0, Some(window.window_handle().window_id()));
    let (bounds, hovered) = (place.bounds, place.hovered);
    let drawn = &mut *drawn;
    let Some(made) = drawn.places.get(&floor).and_then(|place| drawn.picture(place)) else { return };
    let shown = made.key.clone();
    let _ = window.paint_image(bounds, Corners::default(), made.image.clone(), 0, false);
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
    let shape = |text: &str, color: u32, size: f32, window: &mut Window| {
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: Hsla::from(rgb(color)),
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
        let line = shape(&name.text, theme::MUTED, theme::TEXT_2XS, window);
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
    let beacon = hovered.and_then(|i| beacon(made, i, bounds, clear, window, cx, &shape));
    drawn.paints += 1;
    if let Some(place) = drawn.places.get_mut(&floor) {
        (place.labels, place.beacon, place.shown) = (labels, beacon, Some(shown));
    }
}

/// The hovered box's beacon: a line rising from its top and a label at the line's end, the
/// file's name over its folder, or a district's folder. Drawn over the picture, never over the
/// hints: placed below the box if above would cover them, else not drawn. Returns whether it is
/// a file's, whether lit, and where the label is.
fn beacon(
    made: &Made,
    i: usize,
    bounds: Bounds<Pixels>,
    clear: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::App,
    shape: &dyn Fn(&str, u32, f32, &mut Window) -> gpui::ShapedLine,
) -> Option<(bool, bool, Bounds<Pixels>)> {
    let block = made.scene.blocks.get(i)?;
    let place = made.plan.places.get(block.place?)?;
    let (x, y) = made.scene.camera?.project(block.top())?;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let foot = bounds.origin + point(px(x * w), px(y * h));
    let file = !place.group;
    let (name, folder) = match place.id.rsplit_once('/') {
        Some((folder, name)) if file => (name, folder),
        _ => (place.id.as_str(), ""),
    };
    // One line each, as wide as the picture allows: a long path keeps its end (`…ui/views`).
    let most = bounds.size.width - px(16.);
    let fit = |text: &str, colour: u32, size: f32, window: &mut Window| {
        let mut text: String = text.chars().map(|c| if c.is_control() { '?' } else { c }).collect();
        loop {
            let line = shape(&text, colour, size, window);
            let count = text.chars().count();
            if line.width <= most || count <= 2 {
                return line;
            }
            let keep = ((count as f32 * f32::from(most) / f32::from(line.width)) as usize).clamp(1, count - 2);
            text = std::iter::once('…').chain(text.chars().skip(count - keep)).collect();
        }
    };
    let name = fit(name, theme::TEXT, theme::TEXT_XS, window);
    let under = (!folder.is_empty()).then(|| fit(folder, theme::MUTED, theme::TEXT_2XS, window));
    let width = name.width.max(under.as_ref().map_or(px(0.), |line| line.width)) + px(16.);
    let tall = px(if under.is_some() { 34. } else { 20. });
    // Centred over the box, kept inside the picture.
    let left = (foot.x - width / 2.).max(bounds.left()).min(bounds.right() - width);
    let label = |top: Pixels| Bounds { origin: point(left, top), size: gpui::size(width, tall) };
    let (up, down) = (label(foot.y - px(BEACON) - tall), label(foot.y + px(BEACON)));
    // The label, its line and its dot: none of them over the hints.
    let stem = |at: &Bounds<Pixels>| {
        let (from, to) = if at.top() < foot.y { (at.bottom(), foot.y) } else { (foot.y, at.top()) };
        Bounds::from_corners(point(foot.x - px(3.), from), point(foot.x + px(3.), to + px(3.)))
    };
    let fits = |b: &Bounds<Pixels>| {
        !b.intersects(&clear)
            && !stem(b).intersects(&clear)
            && bounds.contains(&b.origin)
            && bounds.contains(&b.bottom_right())
    };
    let at = [up, down].into_iter().find(fits)?;
    let line = stem(&at);
    let colour = Hsla::from(rgb(theme::PRIMARY));
    let line =
        Bounds::from_corners(point(foot.x - px(0.5), line.top()), point(foot.x + px(0.5), line.bottom() - px(3.)));
    window.paint_quad(fill(line, colour));
    window.paint_quad(fill(Bounds::centered_at(foot, gpui::size(px(5.), px(5.))), colour).corner_radii(px(2.5)));
    let pill = fill(at, Hsla::from(rgb(theme::BACKGROUND)).opacity(0.88)).corner_radii(px(theme::RADIUS_MD));
    window.paint_quad(pill.border_widths(px(1.)).border_color(colour.opacity(0.6)));
    let _ = name.paint(at.origin + point(px(8.), px(3.)), px(14.), window, cx);
    if let Some(under) = under {
        let _ = under.paint(at.origin + point(px(8.), px(18.)), px(14.), window, cx);
    }
    Some((file, matches!(block.kind, Kind::Lit(_)), at))
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
            let started = std::time::Instant::now();
            let places = read(&top, &roots).map(|atlas| atlas.places.len());
            println!("{dir}: {places:?} places in {:.1} ms", started.elapsed().as_secs_f64() * 1000.);
        }
    }
}
