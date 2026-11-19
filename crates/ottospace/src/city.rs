//! The code city behind a task's empty tab (flag `code_city`): the checkout's files read off
//! the main thread, and the plan drawn where the lattice was. Which folder and which files are
//! lit is the core's (`Model::city`), and so is where each file is (`ottospace_core::atlas`).
//! Design: `docs/ai/code-city/plan.md`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, Hsla, Pixels, Point, TextRun, Window, canvas, fill, outline, point, prelude::*,
    px, rgb, size,
};
use ottospace_core::atlas::{self, Atlas, Place};
use ottospace_core::{status_bar, theme};

use crate::{Workspace, rect};

/// How long `git ls-files` may take, how much it may say, and how many files a city has. Past
/// any of them there is no city: half of one would put files where the whole one does not.
const TIMEOUT: Duration = Duration::from_secs(10);
const MOST_BYTES: u64 = 4 << 20;
const MOST_FILES: usize = 20_000;
/// Cities kept, by folder: a task's tab shown again draws at once.
const KEPT: usize = 8;
/// A building narrower or lower than this, in points, is not drawn: its district's ground is.
const SMALLEST: f32 = 3.;
/// The gap round a building, in points: two of them are a street.
const STREET: f32 = 1.5;
/// A district of the root and a building this large, in points, have their names on them, if
/// the name fits; at most `MOST_NAMES` buildings' a frame.
const DISTRICT_NAMED: (f32, f32) = (90., 40.);
const BUILDING_NAMED: (f32, f32) = (64., 34.);
const MOST_NAMES: usize = 300;
/// A building's name: quieter than a district's.
const NAME: u32 = 0x52525c;

/// What was asked last: the folder, the folders git may run in, and the files lit then (a
/// file an agent wrote may be new).
type Asked = (String, Vec<String>, Vec<String>);

/// Reads a checkout's files on a thread of its own: when the folder on show changes, or the
/// files its agents are at do. No timer of its own.
pub struct Reader {
    jobs: Sender<(String, Vec<String>)>,
    reads: Receiver<(String, Option<Atlas>)>,
    asked: Option<Asked>,
    held: HashMap<String, Rc<Atlas>>,
    /// How many reads came: for the state of a test run.
    came: usize,
}

/// What the last frame drew of the city: for the state of a test run. No name is in it.
#[derive(Default)]
pub struct Drawn {
    paints: usize,
    places: usize,
    groups: usize,
    lit: Vec<[f32; 4]>,
    /// Of the last paint: the quads and names drawn, and how long it took.
    quads: usize,
    labels: usize,
    paint_us: u64,
}

pub type Shared = Rc<RefCell<Drawn>>;

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
                Some(atlas) => self.held.insert(dir, Rc::new(atlas)),
                None => self.held.remove(&dir),
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

impl Drawn {
    /// For the state of a test run: counts and where the lit buildings are, in window points.
    pub fn snapshot(&self, reads: usize) -> serde_json::Value {
        serde_json::json!({
            "places": self.places, "groups": self.groups, "lit": self.lit, "reads": reads, "paints": self.paints,
            "quads": self.quads, "labels": self.labels, "paint_us": self.paint_us,
        })
    }
}

/// `place` within `bounds`, in points.
fn within(place: &Place, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let origin = bounds.origin + point(px(place.rect.x * w), px(place.rect.y * h));
    Bounds { origin, size: size(px(place.rect.w * w), px(place.rect.h * h)) }
}

/// Paints the plan: a district's ground a little lighter than what it lies in, with a line
/// (stronger for a folder of the root); a building inside its lot, the gap the street; a lit
/// building tinted and outlined in its agent's colour. Then the names that fit: a large
/// district's, and a building's wide enough for its own.
fn paint(
    atlas: &Atlas,
    lit: &HashMap<&str, u32>,
    bounds: Bounds<Pixels>,
    drawn: &Shared,
    window: &mut Window,
    cx: &mut App,
) {
    let started = std::time::Instant::now();
    let (mut quads, mut groups, mut lit_at) = (0, 0, Vec::new());
    let mut names: Vec<(&str, Point<Pixels>, f32, u32)> = Vec::new();
    let ground = gpui::white().opacity(0.012);
    for place in &atlas.places {
        let at = within(place, bounds);
        let (w, h) = (f32::from(at.size.width), f32::from(at.size.height));
        if place.group {
            groups += 1;
            if w >= 1. && h >= 1. {
                let line = if place.depth == 1 { theme::BORDER_STRONG } else { theme::BORDER };
                window.paint_quad(fill(at, ground));
                window.paint_quad(outline(at, Hsla::from(rgb(line)).opacity(0.8), gpui::BorderStyle::Solid));
                quads += 2;
            }
            if place.depth == 1 && w >= DISTRICT_NAMED.0 && h >= DISTRICT_NAMED.1 {
                names.push((&place.label, at.origin + point(px(STREET + 4.), px(STREET + 2.)), w, theme::MUTED));
            }
            continue;
        }
        let colour = lit.get(place.id.as_str());
        // A lit building is drawn however small.
        if colour.is_none() && (w < SMALLEST || h < SMALLEST) {
            continue;
        }
        let inset = STREET.min(w / 4.).min(h / 4.);
        let lot = Bounds {
            origin: at.origin + point(px(inset), px(inset)),
            size: size(px((w - 2. * inset).max(1.)), px((h - 2. * inset).max(1.))),
        };
        match colour {
            // A tint and a line, not a slab: a large file lit must not shout.
            Some(colour) => {
                let colour = Hsla::from(rgb(*colour));
                window.paint_quad(fill(lot, colour.opacity(0.28)).corner_radii(px(2.)));
                window.paint_quad(outline(lot, colour, gpui::BorderStyle::Solid));
                quads += 2;
                lit_at.push(rect(lot));
            }
            None => {
                window.paint_quad(fill(lot, rgb(theme::RAISED)).corner_radii(px(2.)));
                quads += 1;
            }
        }
        // Under a district's own name there is no room for the first building's.
        if w >= BUILDING_NAMED.0 && h >= BUILDING_NAMED.1 && names.len() < MOST_NAMES {
            let colour = if colour.is_some() { theme::TEXT } else { NAME };
            names.push((&place.label, lot.origin + point(px(6.), px(h - 2. * inset - 18.)), w - 2. * inset, colour));
        }
    }
    let style = window.text_style();
    let mut labels = 0;
    for (name, origin, room, colour) in names {
        let color = Hsla::from(rgb(colour));
        let run = TextRun {
            len: name.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(name.to_string().into(), px(theme::TEXT_2XS), &[run], None);
        // A name that does not fit its building is not cut: it is left out.
        if f32::from(line.width) + 12. <= room && line.paint(origin, px(14.), window, cx).is_ok() {
            labels += 1;
        }
    }
    let mut drawn = drawn.borrow_mut();
    *drawn = Drawn {
        paints: drawn.paints + 1,
        places: atlas.places.len(),
        groups,
        lit: lit_at,
        quads,
        labels,
        paint_us: started.elapsed().as_micros() as u64,
    };
}

impl Workspace {
    /// Asks for the city of the tab on show, on the titles' cadence; redraws when one came.
    pub(crate) fn read_city(&mut self, cx: &mut Context<Self>) {
        let key = self.model.city().map(|city| {
            let lit = city.lit.iter().map(|lit| lit.label.to_string()).collect();
            (city.dir.to_string(), self.model.checkout_roots(), lit)
        });
        self.city.ask(key);
        if self.city.take() {
            cx.notify();
        }
    }

    /// The city to draw in place of an empty tab's lattice: `None` with none on show, or none
    /// read yet.
    pub(crate) fn render_city(&self) -> Option<AnyElement> {
        let city = self.model.city()?;
        let atlas = self.city.held.get(city.dir)?.clone();
        let lit: Vec<(String, u32)> =
            city.lit.iter().map(|lit| (lit.label.to_string(), theme::identity(lit.colour))).collect();
        // The checkout's own name: a file in its root is labelled with it.
        let folder =
            Path::new(city.dir).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let drawn = self.city_drawn.clone();
        let plan = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let colours: HashMap<&str, u32> = (lit.iter())
                    .flat_map(|(label, colour)| {
                        atlas.labelled(label, &folder).map(move |place| (place.id.as_str(), *colour))
                    })
                    .collect();
                paint(&atlas, &colours, bounds, &drawn, window, cx);
            },
        );
        Some(plan.absolute().size_full().into_any_element())
    }

    /// For the state of a test run: what was drawn of the city on show; null with none.
    pub(crate) fn city_snapshot(&self) -> serde_json::Value {
        let on = self.model.city().is_some_and(|city| self.city.held.contains_key(city.dir));
        if on { self.city_drawn.borrow().snapshot(self.city.came) } else { serde_json::Value::Null }
    }

    /// File lists read so far, a city on show or not: for the counts of a test run.
    pub(crate) fn city_reads(&self) -> usize {
        self.city.came
    }
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
