//! The code city behind a task's empty tab (flag `code_city`): the checkout's files read off
//! the main thread, and its picture rendered off it (`city_gpu`), painted where the lattice
//! was. Which folder and which files are lit is the core's (`Model::city`), and so are where
//! each file stands and the camera (`ottospace_core::atlas`, `ottospace_core::scene`).
//! Design: `docs/ai/code-city/plan.md`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use gpui::{
    AnyElement, Bounds, Context, Corners, Hsla, Pixels, RenderImage, TextRun, Window, canvas, div, fill, point,
    prelude::*, px, rgb,
};
use ottospace_core::atlas::{self, Atlas};
use ottospace_core::scene::{self, Kind};
use ottospace_core::{status_bar, theme};

use crate::city_gpu::{Gpu, Picture};
use crate::{Workspace, rect};

/// How long `git ls-files` may take, how much it may say, and how many files a city has. Past
/// any of them there is no city: half of one would put files where the whole one does not.
const TIMEOUT: Duration = Duration::from_secs(10);
const MOST_BYTES: u64 = 4 << 20;
const MOST_FILES: usize = 20_000;
/// Cities kept, by folder: a task's tab shown again draws at once.
const KEPT: usize = 8;
/// Names of the root's folders on their ground, at most.
const MOST_NAMES: usize = 24;
/// Room kept between a name and the empty tab's hints, and the hints' backing, in points.
const CLEAR: f32 = 12.;

/// What was asked last: the folder, the folders git may run in, and the files lit then, each
/// with whether its call ran (a file an agent wrote may be new, once the call ends).
type Asked = (String, Vec<String>, Vec<(String, bool)>);

/// Reads a checkout's files on a thread of its own: when the folder on show changes, or the
/// files its agents are at do. No timer of its own.
pub struct Reader {
    jobs: Sender<(String, Vec<String>)>,
    reads: Receiver<(String, Option<Atlas>)>,
    asked: Option<Asked>,
    held: HashMap<String, Arc<Atlas>>,
    /// How many reads came: for the state of a test run.
    came: usize,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, reads) = crate::child::newest(|(dir, roots): (String, Vec<String>)| {
            let read = read(Path::new(&dir), &roots);
            (dir, read)
        });
        Self { jobs, reads, asked: None, held: HashMap::new(), came: 0 }
    }

    /// Asks for `dir`'s city if it, `roots` or `lit` changed since the last ask. `None`: no
    /// city is on show, and the next one is read again.
    fn ask(&mut self, key: Option<Asked>) {
        if key != self.asked {
            if let Some((dir, roots, _)) = &key {
                let _ = self.jobs.send((dir.clone(), roots.clone()));
            }
            self.asked = key;
        }
    }

    /// Takes the reads that came. True if one did.
    fn take(&mut self) -> bool {
        let came: Vec<(String, Option<Atlas>)> = self.reads.try_iter().collect();
        let any = !came.is_empty();
        for (dir, read) in came {
            self.came += 1;
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

/// The plan of the checkout at `dir`: the files git lists there (tracked, and new ones it does
/// not ignore), each with its size. Hardened as the status bar's read (`child::git_read`), and
/// only in a folder that, resolved, is one of `roots` (`status_bar::reads_changes`). A link is
/// no file, and nothing under a folder that is a link is one: nothing outside the checkout is
/// looked at. `None`: no city.
fn read(dir: &Path, roots: &[String]) -> Option<Atlas> {
    let top =
        dir.canonicalize().ok().filter(|top| top.to_str().is_some_and(|top| status_bar::reads_changes(top, roots)))?;
    let mut list = crate::child::git_read(&top);
    list.args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
    let late = std::time::Instant::now() + TIMEOUT;
    let said = crate::child::run(list, TIMEOUT, MOST_BYTES).ok().filter(|said| said.ok)?;
    let names: Vec<&str> = said.out.split('\0').filter(|name| !name.is_empty()).collect();
    if said.out.len() as u64 >= MOST_BYTES || names.len() > MOST_FILES {
        return None;
    }
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
    (!files.is_empty()).then(|| atlas::atlas(&atlas::repo(&files)))
}

/// A picture asked for: the folder, its plan, the lit files (place ids, colours) and the size
/// in pixels.
#[derive(Clone, PartialEq)]
struct Key {
    dir: String,
    plan: usize,
    lit: Vec<(String, u32)>,
    size: (u32, u32),
}

/// A picture made: where, in shares of it, the lit buildings and the root's folders' names are.
struct Made {
    key: Key,
    image: Arc<RenderImage>,
    lit: Vec<[f32; 4]>,
    names: Vec<(String, (f32, f32))>,
    places: usize,
    groups: usize,
    render_us: u64,
}

/// What the picture's thread said last, and what the last frame drew: for the state of a test
/// run. No name or path is in it.
#[derive(Default)]
pub struct Drawn {
    asked: Option<Key>,
    made: Option<Made>,
    /// Why there is no picture: no GPU, or the render failed. The lattice is drawn instead.
    failed: Option<String>,
    renders: usize,
    paints: usize,
    /// Of the last paint: where, in window points, and how many names.
    bounds: Bounds<Pixels>,
    labels: usize,
    /// Pictures replaced: GPUI keeps each in the window's atlas until it is dropped.
    retired: Vec<Arc<RenderImage>>,
}

pub type Shared = Rc<RefCell<Drawn>>;

/// Renders pictures on a thread of its own, the newest asked only. The GPU is opened at the
/// first and kept; none (or `OTTOSPACE_CITY_NO_GPU` in a test run) and every picture fails.
pub struct Painter {
    jobs: Sender<(Key, Arc<Atlas>)>,
    made: Receiver<(Key, Result<Made, String>)>,
}

impl Painter {
    pub fn new() -> Self {
        let gpu: RefCell<Option<Result<Gpu, String>>> = RefCell::new(None);
        let (jobs, made) = crate::child::newest(move |(key, plan): (Key, Arc<Atlas>)| {
            let mut held = gpu.borrow_mut();
            let opened = held.get_or_insert_with(|| {
                let refused = std::env::var_os("OTTOSPACE_E2E").and(std::env::var_os("OTTOSPACE_CITY_NO_GPU"));
                if refused.is_some() { Err("no GPU (test)".into()) } else { Gpu::new() }
            });
            // A panic in the GPU's code fails this picture, and the GPU is opened again next time.
            let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                opened.as_ref().map_err(Clone::clone).and_then(|gpu| make(gpu, &key, &plan))
            }));
            let made = made.unwrap_or_else(|_| {
                *held = None;
                Err("the render panicked".into())
            });
            (key, made)
        });
        Self { jobs, made }
    }
}

/// The scene of `plan` and its picture, on the picture's thread.
fn make(gpu: &Gpu, key: &Key, plan: &Atlas) -> Result<Made, String> {
    let started = std::time::Instant::now();
    let lit: Vec<(&str, u32)> = key.lit.iter().map(|(id, colour)| (id.as_str(), *colour)).collect();
    let (w, h) = key.size;
    let scene = scene::scene(plan, &lit, w as f32 / h as f32);
    let Picture { width, height, bgra } = gpu.render(&scene, w, h)?;
    let camera = scene.camera.ok_or("no camera")?;
    let image = image::RgbaImage::from_raw(width, height, bgra).ok_or("a picture of the wrong size")?;
    let image = Arc::new(RenderImage::new([image::Frame::new(image)]));
    let lit = scene.blocks.iter().filter(|b| matches!(b.kind, Kind::Lit(_))).filter_map(|b| camera.bounds(b)).collect();
    // A folder of the root is named at the front edge of its ground.
    let plates = scene.blocks.iter().filter(|b| b.kind == Kind::Plate(1));
    let named = plan.places.iter().filter(|p| p.group && p.depth == 1);
    let names = named
        .zip(plates)
        .filter_map(|(place, b)| Some((place.label.clone(), camera.project([b.min[0], b.max[1], b.max[2]])?)))
        .take(MOST_NAMES)
        .collect();
    let groups = plan.places.iter().filter(|p| p.group).count();
    let render_us = started.elapsed().as_micros() as u64;
    Ok(Made { key: key.clone(), image, lit, names, places: plan.places.len(), groups, render_us })
}

impl Drawn {
    /// For the state of a test run: counts, the picture's size, where it and the lit buildings
    /// are, in window points.
    pub fn snapshot(&self, reads: usize, scale: f32) -> serde_json::Value {
        let [x, y, w, h] = rect(self.bounds);
        let lit: Vec<[f32; 4]> = (self.made.iter().flat_map(|made| &made.lit))
            .map(|r| [x + r[0] * w, y + r[1] * h, r[2] * w, r[3] * h])
            .collect();
        let made = self.made.as_ref();
        serde_json::json!({
            "places": made.map(|made| made.places), "groups": made.map(|made| made.groups),
            "picture": made.map(|made| made.key.size), "bounds": [x, y, w, h], "scale": scale,
            "lit": lit, "labels": self.labels, "failed": self.failed, "reads": reads,
            "renders": self.renders, "paints": self.paints, "render_us": made.map(|made| made.render_us),
        })
    }
}

impl Workspace {
    /// Asks for the city of the tab on show, on the titles' cadence; takes the pictures made.
    /// Redraws when either came.
    pub(crate) fn read_city(&mut self, cx: &mut Context<Self>) {
        let key = self.model.city().map(|city| {
            let lit = city.lit.iter().map(|lit| (lit.label.to_string(), lit.running)).collect();
            (city.dir.to_string(), self.model.checkout_roots(), lit)
        });
        self.city.ask(key);
        let mut redraw = self.city.take();
        for (key, made) in self.city_painter.made.try_iter() {
            let mut drawn = self.city_drawn.borrow_mut();
            drawn.renders += 1;
            redraw = true;
            match made {
                Ok(made) => {
                    let old = drawn.made.replace(made);
                    drawn.retired.extend(old.map(|old| old.image));
                    drawn.failed = None;
                }
                // A failure of a picture no longer asked for says nothing.
                Err(why) if drawn.asked.as_ref() == Some(&key) => {
                    let old = drawn.made.take();
                    drawn.retired.extend(old.map(|old| old.image));
                    drawn.failed = Some(why);
                }
                Err(_) => {}
            }
        }
        for image in std::mem::take(&mut self.city_drawn.borrow_mut().retired) {
            cx.drop_image(image, None);
        }
        if redraw {
            cx.notify();
        }
    }

    /// The floor of a task's empty tab: the city's picture once one is made for its folder,
    /// with the root's folders named and the hints backed; `lattice` until then, or with no
    /// city, or no GPU. Asks for a picture when the plan, the lit files or the size changed.
    pub(crate) fn render_city(&self, lattice: AnyElement) -> AnyElement {
        let Some(city) = self.model.city() else { return lattice };
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
        let (dir, drawn, jobs) = (city.dir.to_string(), self.city_drawn.clone(), self.city_painter.jobs.clone());
        let home = self.home.clone();
        let shows =
            drawn.borrow().made.as_ref().is_some_and(|made| made.key.dir == dir && drawn.borrow().failed.is_none());
        let picture = canvas(
            move |bounds, window, _| {
                // Pixels, as the window draws them: the picture is never scaled.
                let scale = window.scale_factor();
                let size = |v: Pixels| (f32::from(v) * scale).round() as u32;
                let key = Key {
                    dir,
                    plan: Arc::as_ptr(&plan) as usize,
                    lit,
                    size: (size(bounds.size.width), size(bounds.size.height)),
                };
                let mut held = drawn.borrow_mut();
                if held.asked.as_ref() != Some(&key) && key.size.0 > 0 && key.size.1 > 0 {
                    if jobs.send((key.clone(), plan)).is_err() {
                        held.failed = Some("the picture's thread ended".into());
                    }
                    held.asked = Some(key);
                }
                held.bounds = bounds;
                drop(held);
                drawn
            },
            // The hints as this frame laid them out.
            move |bounds, drawn: Shared, window, cx| paint(&drawn, bounds, home.borrow().hints(), window, cx),
        );
        let picture = picture.absolute().size_full();
        if shows {
            picture.into_any_element()
        } else {
            div().size_full().child(lattice).child(picture).into_any_element()
        }
    }

    /// For the state of a test run: what was drawn of the city on show; null with none.
    pub(crate) fn city_snapshot(&self, scale: f32) -> serde_json::Value {
        let on = self.model.city().is_some_and(|city| self.city.held.contains_key(city.dir));
        if on { self.city_drawn.borrow().snapshot(self.city.came, scale) } else { serde_json::Value::Null }
    }

    /// File lists read so far, a city on show or not: for the counts of a test run.
    pub(crate) fn city_reads(&self) -> usize {
        self.city.came
    }
}

/// The picture made for this folder, as large as the floor (a stale one stretched until the
/// new one comes); a backing behind the hints, so they read over any building; and the root's
/// folders' names, none behind the hints.
fn paint(drawn: &Shared, bounds: Bounds<Pixels>, hints: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
    let mut drawn = drawn.borrow_mut();
    let Some(made) = drawn.made.as_ref().filter(|made| Some(&made.key.dir) == drawn.asked.as_ref().map(|key| &key.dir))
    else {
        return;
    };
    if drawn.failed.is_some() {
        return;
    }
    let _ = window.paint_image(bounds, Corners::default(), made.image.clone(), 0, false);
    let clear = hints.dilate(px(CLEAR));
    if hints.size.width > px(0.) {
        window.paint_quad(fill(clear, Hsla::from(rgb(theme::BACKGROUND)).opacity(0.78)).corner_radii(px(CLEAR)));
    }
    let style = window.text_style();
    let mut labels = 0;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    // A name with a line break in it is not drawn: `shape_line` takes one line.
    for (name, (x, y)) in made.names.iter().filter(|(name, _)| !name.contains(char::is_control)) {
        let color = Hsla::from(rgb(theme::MUTED));
        let run = TextRun {
            len: name.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(name.clone().into(), px(theme::TEXT_2XS), &[run], None);
        let at = bounds.origin + point(px(x * w + 4.), px(y * h + 2.));
        let room = Bounds { origin: at, size: gpui::size(line.width, px(14.)) };
        if !room.intersects(&clear)
            && bounds.contains(&room.bottom_right())
            && line.paint(at, px(14.), window, cx).is_ok()
        {
            labels += 1;
        }
    }
    drawn.paints += 1;
    drawn.labels = labels;
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
