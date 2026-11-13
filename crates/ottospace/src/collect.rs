//! The capture key, annotate mode and the right bar: the shell half. It freezes a pane's picture,
//! reads the text that goes with it, draws what the core says was drawn on it, and keeps the
//! files. What is captured, how it is quoted and what a brief says are the core's
//! (`ottospace_core::capture`). See `docs/ai/capture.md`.
//!
//! Only ottospace's own panes are ever captured: a terminal's grid, or the stream and the
//! accessibility tree of a window it hosts. The tree is read, never acted on. Only the user
//! starts a capture: a client of the socket cannot ask for a pane's picture or its text.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use core_video::pixel_buffer::{CVPixelBuffer, kCVPixelBufferLock_ReadOnly};
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, Div, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    ObjectFit, PathBuilder, Pixels, SharedString, Size, Stateful, Window, canvas, div, img, point, prelude::*, px,
    relative, rgb, size, surface,
};
use ottospace_core::capture::{self, Brief, Content, Facts, MAX_BRIEF_BYTES, Material, Shape, Shot as Lie, Tool};
use ottospace_core::rules;
use ottospace_core::tasks::{self, TaskId};
use ottospace_core::{Action, Kind, Mode, Origin, PaneId, theme};
use ottospace_mac::{ax, image, process};

use crate::{Frame, Hosted, Workspace, ax_debug, config, control, now_ms, sneak, terminal};

/// The capture key's name once it is registered, e.g. `CTRL-ALT-CMD-SPACE`. Unset: there is none.
pub static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// How wide the right bar is.
/// How long an app may take to answer for its tree before the capture goes without it.
const AX_LIMIT: Duration = Duration::from_secs(8);
/// A picture is taken once the pane is drawn beside the bar: this long after the key at least
/// (a terminal's program draws again at its new width), and at most this long (a window's
/// stream may never send a frame of the new size).
const SETTLE: Duration = Duration::from_millis(90);
const SETTLE_MOST: Duration = Duration::from_millis(1200);
/// A window whose stream has said nothing for this long is not being drawn: minimised, or on
/// another Space. Its last frame may be old.
const STALE_MS: u64 = 3_000;

/// A pane's picture as it was when it was taken.
pub enum Frozen {
    /// A hosted window's last stream frame, and the points of the window it shows.
    App { buffer: CVPixelBuffer, shown: Size<Pixels> },
    /// A terminal's rows, the size its pane had and that of one cell.
    Terminal { screen: Arc<terminal::Screen>, shown: Size<Pixels>, cell: [f64; 2] },
    /// The window had no frame yet.
    Blank,
}

/// What a picture is encoded from, as plain data for another thread.
enum Still {
    Frame(Frame),
    Rows(Arc<terminal::Screen>, [f64; 2], [f64; 2]),
}

impl Frozen {
    fn still(&self) -> Option<Still> {
        let points = |size: &Size<Pixels>| [f64::from(f32::from(size.width)), f64::from(f32::from(size.height))];
        match self {
            Frozen::App { buffer, .. } => Some(Still::Frame(Frame(buffer.clone()))),
            Frozen::Terminal { screen, shown, cell } => Some(Still::Rows(screen.clone(), points(shown), *cell)),
            Frozen::Blank => None,
        }
    }
}

fn encode(pixels: &Still, scale: f64) -> Option<Vec<u8>> {
    match pixels {
        Still::Frame(frame) => frame_png(frame),
        Still::Rows(screen, size, cell) => screen.picture(*size, *cell, scale),
    }
}

/// What a kept capture's pictures are made from: its number, its PNG if that is encoded, else
/// what was frozen, and the pixels a point.
type Job = (u32, Option<Arc<gpui::Image>>, Option<Still>, f64);

/// What the shell holds of one capture while its draft is open.
pub struct Shot {
    pub frozen: Frozen,
    /// The picture as a PNG, once it is encoded.
    pub png: Option<Arc<gpui::Image>>,
    /// Pixels a point in it.
    pub scale: f64,
}

/// A picture the core asked for, not taken yet: the bar opens first, so the pane is drawn at
/// the size it has beside the bar and the picture has no cut edge.
pub struct Pending {
    draft: String,
    n: u32,
    pane: PaneId,
    since: Instant,
    /// How often the workspace had been drawn when it was asked for.
    renders: u64,
}

/// A brief as the bar shows it: what it says, where its files are, its pictures.
pub struct Shown {
    pub brief: Brief,
    /// Its folder's name inside the materials folder, as its task has it.
    pub name: String,
    /// The folder, if anything is kept on disk.
    pub dir: Option<PathBuf>,
    pub text: String,
    /// Each capture's annotated picture, by its number, while this run still holds it.
    pub pictures: Vec<(u32, Arc<gpui::Image>)>,
}

/// The pixels of an NV12 frame (a full plane of Y, then a half-size plane of Cb and Cr
/// together) as RGBA. Full range, BT.709: what the stream of a window gives.
pub fn nv12_to_rgba(y: &[u8], y_stride: usize, c: &[u8], c_stride: usize, width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    for row in 0..height {
        for column in 0..width {
            let luma = f32::from(y.get(row * y_stride + column).copied().unwrap_or(0));
            let at = (row / 2) * c_stride + (column / 2) * 2;
            let (cb, cr) = (c.get(at).copied().unwrap_or(128), c.get(at + 1).copied().unwrap_or(128));
            let (cb, cr) = (f32::from(cb) - 128.0, f32::from(cr) - 128.0);
            let rgb = [luma + 1.5748 * cr, luma - 0.1873 * cb - 0.4681 * cr, luma + 1.8556 * cb];
            out.extend([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 255]);
        }
    }
    out
}

/// A stream frame as a PNG.
fn frame_png(frame: &Frame) -> Option<Vec<u8>> {
    let buffer = &frame.0;
    if buffer.get_plane_count() < 2 || buffer.lock_base_address(kCVPixelBufferLock_ReadOnly) != 0 {
        return None;
    }
    let (width, height) = (buffer.get_width(), buffer.get_height());
    let plane = |index: usize| {
        let (stride, rows) = (buffer.get_bytes_per_row_of_plane(index), buffer.get_height_of_plane(index));
        let base = unsafe { buffer.get_base_address_of_plane(index) } as *const u8;
        (!base.is_null()).then(|| (unsafe { std::slice::from_raw_parts(base, stride * rows) }, stride))
    };
    let rgba = match (plane(0), plane(1)) {
        (Some((y, y_stride)), Some((c, c_stride))) => Some(nv12_to_rgba(y, y_stride, c, c_stride, width, height)),
        _ => None,
    };
    buffer.unlock_base_address(kCVPixelBufferLock_ReadOnly);
    image::png_from_rgba(width, height, &rgba?)
}

/// The process that made a draft, if `draft` is a name the shell gives one:
/// `draft-<pid>-<time>`. Nothing else names a folder.
fn draft_pid(draft: &str) -> Option<i32> {
    let mut parts = draft.strip_prefix("draft-")?.split('-');
    let (pid, time, rest) = (parts.next()?, parts.next()?, parts.next());
    let digits = |part: &str| !part.is_empty() && part.len() <= 20 && part.bytes().all(|b| b.is_ascii_digit());
    if digits(pid) && digits(time) && rest.is_none() { pid.parse().ok() } else { None }
}

fn draft_dir(draft: &str) -> Option<PathBuf> {
    config::materials_dir().filter(|_| draft_pid(draft).is_some()).map(|dir| dir.join(draft))
}

/// A task's folder of captures, if `name` is one the core gives a task of this number.
pub(crate) fn task_dir(task: TaskId, name: &str) -> Option<PathBuf> {
    let word = name.strip_prefix(&format!("{task}-"))?;
    let own = tasks::materials_dir(task, word).is_some_and(|dir| dir == name);
    config::materials_dir().filter(|_| own).map(|dir| dir.join(name))
}

/// A word nobody can guess, 24 letters and digits: for a draft's task's folder and its brief's
/// fence. 96 bits from the system's random source. `None`: that could not be read.
fn new_word() -> Option<String> {
    use std::io::Read;
    let mut bytes = [0u8; 12];
    std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes)).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// A picture this build wrote is far smaller.
const MAX_PICTURE_BYTES: u64 = 64 * 1024 * 1024;

/// macOS's `O_NONBLOCK` and `O_NOFOLLOW`: opening a pipe does not wait, and a link is not followed.
const OPEN_PLAIN: i32 = 0x0004 | 0x0100;

/// What is in `file`, if it is a plain file of at most `most` bytes: opened once, and asked of
/// what was opened. What is on disk can be written by any program of the user's.
pub(crate) fn read_plain(file: &std::path::Path, most: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let opened = std::fs::OpenOptions::new().read(true).custom_flags(OPEN_PLAIN).open(file).ok()?;
    let meta = opened.metadata().ok()?;
    let mut bytes = Vec::new();
    let read = (meta.is_file() && meta.len() <= most).then(|| opened.take(most + 1).read_to_end(&mut bytes))?;
    (read.ok()? as u64 <= most).then_some(bytes)
}

fn pretty(what: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(what).unwrap_or_default() + "\n"
}

/// Removes the folders of drafts no run will come back to: those of a process that is gone (or
/// of this one's number, which a run before it had). Never a draft of another ottospace that
/// runs now. A crash is the only way one is left.
pub fn sweep_drafts() {
    let Some(entries) = config::materials_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else { return };
    let me = std::process::id() as i32;
    for path in entries.flatten().map(|entry| entry.path()) {
        let pid = path.file_name().and_then(|name| name.to_str()).and_then(draft_pid);
        let gone = pid.is_some_and(|pid| pid == me || process::parent(pid).is_none());
        // A folder itself, not a link to one somewhere else.
        if gone && std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn png_image(png: Vec<u8>) -> Arc<gpui::Image> {
    Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))
}

/// Paints the lines of `shapes` over `bounds`: ellipses, boxes and strokes. Notes are elements.
fn paint_shapes(shapes: &[Shape], selected: Option<usize>, bounds: Bounds<Pixels>, window: &mut Window) {
    let at = |p: [f64; 2]| bounds.origin + point(bounds.size.width * p[0] as f32, bounds.size.height * p[1] as f32);
    for (i, shape) in shapes.iter().enumerate() {
        let points: Vec<[f64; 2]> = match shape {
            Shape::Ellipse { rect: [x, y, w, h] } => {
                let turn = |i: usize| (i as f64 / 48.0) * std::f64::consts::TAU;
                (0..=48).map(|i| [x + w / 2.0 * (1.0 + turn(i).cos()), y + h / 2.0 * (1.0 + turn(i).sin())]).collect()
            }
            Shape::Box { rect: [x, y, w, h] } => vec![[*x, *y], [x + w, *y], [x + w, y + h], [*x, y + h], [*x, *y]],
            Shape::Stroke { points } => points.clone(),
            Shape::Note { .. } => continue,
        };
        let mut path = PathBuilder::stroke(px(if selected == Some(i) { 3.5 } else { 2.0 }));
        for (i, p) in points.iter().enumerate() {
            if i == 0 { path.move_to(at(*p)) } else { path.line_to(at(*p)) }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, rgb(theme::SNEAK));
        }
    }
}

/// A layer that paints `shapes` over its parent.
fn shapes_layer(shapes: Vec<Shape>, selected: Option<usize>) -> impl IntoElement {
    canvas(|_, _, _| {}, move |bounds, _, window, _| paint_shapes(&shapes, selected, bounds, window))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// True if the window rules say to leave this hosted window alone, by its title of now. The
/// app is asked with a short wait; if it does not answer, the title it was hosted with stands.
fn ignored(hosted: &Hosted) -> (Option<String>, bool) {
    hosted.window.quick(|window| {
        let (bundle, name) = ax::app_identity(hosted.pid);
        let title = window.title();
        let subrole = window.subrole().unwrap_or_else(|| "AXStandardWindow".into());
        let facts = rules::Facts {
            app: name.as_deref().unwrap_or(&hosted.app),
            bundle: bundle.as_deref().or(hosted.bundle.as_deref()),
            title: title.as_deref().unwrap_or(&hosted.title),
            subrole: &subrole,
        };
        let managed = rules::manages(&config::get().settings.windows, &facts);
        (title, !managed)
    })
}

impl Workspace {
    /// What the shell can read of the panes on show, for a capture and for "open windows".
    /// A window's title is asked of its app with a short wait: this runs on the main thread.
    pub(crate) fn pane_facts(&self) -> Vec<Facts> {
        let fact = |pane: &ottospace_core::Pane| {
            let terminal = self.terminals.get(&pane.id);
            let (title, ignored) = self.hosted.get(&pane.id).map(ignored).unwrap_or_default();
            Facts {
                pane: pane.id,
                cwd: terminal.and_then(terminal::Terminal::cwd),
                program: terminal.and_then(terminal::Terminal::running).and_then(|work| work.program),
                title,
                ignored,
            }
        };
        let shown = self.model.layout.as_ref().map(ottospace_core::Node::leaves).unwrap_or_default();
        self.model.panes.iter().filter(|pane| shown.contains(&pane.id)).map(fact).collect()
    }

    /// The user's capture key, the menu item, the bar's button, or their click on another pane
    /// while annotating: a picture of `pane` (default: the one in use) to draw on, in the draft
    /// in the right bar. `Err`: why nothing was captured. Only the user's: `origin` is theirs,
    /// or the core refuses.
    pub(crate) fn capture_pane(
        &mut self,
        pane: Option<PaneId>,
        origin: Origin,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let facts = self.pane_facts();
        let target = pane.or(self.model.live).or(self.model.focused);
        if facts.iter().any(|f| Some(f.pane) == target && f.ignored) {
            return Err("the window rules say to leave this window alone: it is not captured".into());
        }
        let action = Action::CaptureStart {
            pane,
            draft: format!("draft-{}-{}", std::process::id(), now_ms()),
            word: new_word().ok_or("capture: the system's random source could not be read")?,
            facts,
            images: config::get().settings.capture.keep_images && config::materials_dir().is_some(),
        };
        if let Some(why) = self.model.refusal(&action, origin) {
            return Err(why.into());
        }
        self.dispatch(action, origin, window, cx);
        Ok(())
    }

    /// `Effect::Snapshot`: the picture is taken once the pane is drawn beside the bar.
    pub(crate) fn ask_snapshot(&mut self, draft: &str, n: u32, pane: PaneId) {
        self.pending.push(Pending { draft: draft.to_string(), n, pane, since: Instant::now(), renders: self.renders });
    }

    /// True while a picture of `pane` is asked for and not taken.
    pub(crate) fn snapshot_pending(&self, pane: PaneId) -> bool {
        self.pending.iter().any(|pending| pending.pane == pane)
    }

    /// Each tick: takes the pictures whose pane is drawn at its size beside the bar. A terminal
    /// has had a moment to draw at its new width. A window's stream has sent a frame of the
    /// size the pane now shows, or it never will (a window that refused the size).
    pub(crate) fn take_snapshots(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.pending.is_empty() {
            return;
        }
        let settled = |pending: &Pending| {
            let waited = pending.since.elapsed();
            let drawn = self.renders > pending.renders && waited >= SETTLE;
            let framed = self.hosted.get(&pending.pane).is_none_or(|hosted| {
                let pane = crate::to_cg(hosted.bounds.get().size);
                let expected = hosted.latest.expected.load(Relaxed);
                let frame = hosted.frame().map(|frame| (frame.get_width() as u64) << 32 | frame.get_height() as u64);
                hosted.parked_size == pane && frame == Some(expected)
            });
            (drawn && framed) || waited >= SETTLE_MOST
        };
        let (ready, later): (Vec<Pending>, Vec<Pending>) =
            std::mem::take(&mut self.pending).into_iter().partition(settled);
        self.pending = later;
        for Pending { draft, n, pane, .. } in ready {
            self.snapshot_pane(&draft, n, pane, window, cx);
            // Visual mode's cursor goes now, after the picture: its going scrolls to the bottom.
            if let Some(at) = self.visual_after.iter().position(|waiting| *waiting == pane) {
                self.visual_after.remove(at);
                if let Some(terminal) = self.terminals.get(&pane) {
                    terminal.set_visual(false);
                }
            }
            cx.notify();
        }
    }

    /// Freezes what the pane shows now, tells the core what was taken, then encodes the picture
    /// and reads the text that goes with it. Encoding, writing and the accessibility read are
    /// off this thread.
    fn snapshot_pane(&mut self, draft: &str, n: u32, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        // A draft that is gone, or a capture taken out of it, has no picture to take.
        let wanted = self.model.capture.as_ref().is_some_and(|bar| bar.draft == draft && bar.material(n).is_some());
        if !wanted {
            return;
        }
        let scale = f64::from(window.scale_factor());
        let settings = config::get().settings.capture;
        let taken =
            |picture: bool, stale: bool| Action::CaptureTaken { draft: draft.to_string(), material: n, picture, stale };
        let content = |content| Action::CaptureContent { draft: draft.to_string(), material: n, content };
        // One look at a terminal: its picture and its rows are of the same instant.
        if let Some(screen) = self.terminals.get(&pane).map(|terminal| Arc::new(terminal.screen())) {
            let shown = self.cells.get(&pane).map(|cell| cell.get().size).unwrap_or_default();
            let cell = [f64::from(f32::from(self.cell.width)), f64::from(f32::from(self.cell.height))];
            let height = f64::from(f32::from(shown.height));
            let row = if height > 0.0 { cell[1] / height } else { 0.0 };
            let lines = screen.lines();
            self.shots.insert(n, Shot { frozen: Frozen::Terminal { screen, shown, cell }, png: None, scale });
            self.dispatch(taken(true, false), Origin::System, window, cx);
            self.dispatch(content(Content::Terminal { lines, row }), Origin::System, window, cx);
            return self.encode_picture(draft, n, cx);
        }
        let Some(hosted) = self.hosted.get(&pane) else {
            self.shots.insert(n, Shot { frozen: Frozen::Blank, png: None, scale });
            self.dispatch(taken(false, false), Origin::System, window, cx);
            return drop(self.dispatch(
                content(Content::Missing("the pane closed".into())),
                Origin::System,
                window,
                cx,
            ));
        };
        let lie = (hosted.shown_crop, [hosted.shown.width, hosted.shown.height]);
        let shown = size(px(hosted.shown.width as f32), px(hosted.shown.height as f32));
        let (pid, window_id) = (hosted.pid, hosted.window_id);
        // The stream says nothing while its window is not drawn: the frame held may be old.
        let stale = now_ms().saturating_sub(hosted.latest.seen.load(Relaxed)) > STALE_MS;
        let frozen = match hosted.frame() {
            Some(buffer) => Frozen::App { buffer, shown },
            None => Frozen::Blank,
        };
        let pictured = !matches!(frozen, Frozen::Blank);
        self.shots.insert(n, Shot { frozen, png: None, scale });
        self.dispatch(taken(pictured, pictured && stale), Origin::System, window, cx);
        self.encode_picture(draft, n, cx);
        if !settings.ax {
            let off = Content::Missing("accessibility capture is off (\"capture\": {\"ax\": false})".into());
            return drop(self.dispatch(content(off), Origin::System, window, cx));
        }
        // The tree is read once, now, with the reader's own limits. Nothing in it is pressed or set.
        let (started, draft) = (Instant::now(), draft.to_string());
        let read = cx.background_executor().spawn(async move { ax_debug::read_window(pid, window_id) });
        cx.spawn_in(window, async move |this, cx| {
            let read = read.await;
            let content = match read {
                _ if started.elapsed() > AX_LIMIT => Content::Missing("the app did not answer in time".into()),
                Err(why) => Content::Missing(why),
                Ok((frame, tree)) => {
                    let cut = tree.truncated;
                    let shot = Lie { window: frame, crop: lie.0, shown: lie.1 };
                    Content::App { root: tree.root, shot, truncated: cut.depth || cut.nodes || cut.timeout }
                }
            };
            let action = Action::CaptureContent { draft, material: n, content };
            let _ = this.update_in(cx, |this, window, cx| this.dispatch(action, Origin::System, window, cx));
        })
        .detach();
    }

    /// Encodes a capture's picture and, if pictures are kept, writes it to the draft's folder:
    /// both on another thread. Back here it is held for the bar; if its draft or the capture is
    /// gone by then, what was written goes again.
    fn encode_picture(&mut self, draft: &str, n: u32, cx: &mut Context<Self>) {
        let Some((pixels, scale)) = self.shots.get(&n).and_then(|shot| Some((shot.frozen.still()?, shot.scale))) else {
            return;
        };
        let images = self.model.capture.as_ref().is_some_and(|bar| bar.images);
        let (dir, draft) = (draft_dir(draft).filter(|_| images), draft.to_string());
        let work = cx.background_executor().spawn(async move {
            let png = encode(&pixels, scale)?;
            let written = dir.as_ref().map(|dir| config::write_private(dir, &format!("{n}.png"), &png));
            Some((png, dir, written.and_then(Result::err)))
        });
        cx.spawn(async move |this, cx| {
            let Some((png, dir, failed)) = work.await else { return };
            let _ = this.update(cx, |this, cx| {
                let current = this.model.capture.as_ref().is_some_and(|bar| bar.draft == draft);
                match this.shots.get_mut(&n).filter(|_| current) {
                    Some(shot) => {
                        shot.png = Some(png_image(png));
                        this.error = failed
                            .map(|err| format!("capture: could not keep the picture: {err}"))
                            .or(this.error.take());
                    }
                    // Too late: the draft was cancelled or added, or the capture taken out.
                    None => {
                        if let Some(dir) = dir {
                            let _ = std::fs::remove_file(dir.join(format!("{n}.png")));
                            let _ = std::fs::remove_dir(dir); // only if that left it empty
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `Effect::CaptureDropped`: what was kept of a draft, or of one capture of it, goes.
    pub(crate) fn drop_capture(&mut self, draft: &str, material: Option<u32>) {
        let dir = draft_dir(draft);
        match material {
            Some(n) => {
                self.shots.remove(&n);
                self.pending.retain(|pending| pending.n != n);
                if let Some(dir) = dir {
                    let _ = std::fs::remove_file(dir.join(format!("{n}.png")));
                }
            }
            None => {
                self.shots.clear();
                self.pending.clear();
                if let Some(dir) = dir {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
        }
    }

    /// `Effect::CaptureKept`: the draft's folder becomes the task's own, a new one, and each
    /// capture's pictures, its file and the brief are written into it on another thread. A
    /// picture that cannot be written is taken out of the brief before the brief is: the brief
    /// names only pictures that are there. A folder of that name that exists is never touched.
    /// With no folder to keep things in, the brief is held for this run only.
    pub(crate) fn keep_capture(&mut self, draft: &str, name: &str, brief: &Brief, cx: &mut Context<Self>) {
        let mut shots = std::mem::take(&mut self.shots);
        self.pending.clear();
        let task = brief.task;
        let mut dir = task_dir(task, name);
        if let Some(taken) = dir.as_ref().filter(|dir| std::fs::symlink_metadata(dir).is_ok()) {
            self.error = Some(format!("capture: {} is there already: the captures were not kept", taken.display()));
            dir = None;
        }
        // The draft's folder is the task's from now on: a picture written late goes nowhere.
        if let (Some(dir), Some(from)) = (&dir, draft_dir(draft).filter(|from| from.is_dir()))
            && let Err(err) = std::fs::rename(from, dir)
        {
            self.error = Some(format!("capture: could not keep the draft's pictures: {err}"));
        }
        // The pictures by their whole path: a reader opens them from wherever it runs.
        let at = dir.as_ref().map(|dir| dir.to_string_lossy().into_owned());
        let text = capture::markdown_in(brief, at.as_deref());
        let shown =
            Shown { brief: brief.clone(), name: name.to_string(), dir: dir.clone(), text, pictures: Vec::new() };
        self.briefs.insert(task, shown);
        // What each picture is drawn from: its PNG if it is encoded, else what was frozen.
        let jobs: Vec<Job> = brief
            .materials
            .iter()
            .filter_map(|kept| {
                shots.remove(&kept.n).map(|shot| (kept.n, shot.png.clone(), shot.frozen.still(), shot.scale))
            })
            .collect();
        let (mut brief, name) = (brief.clone(), name.to_string());
        let work = cx.background_executor().spawn(async move {
            let mut pictures = Vec::new();
            for kept in &mut brief.materials {
                let job = jobs.iter().find(|job| job.0 == kept.n);
                let png = job.and_then(|(_, png, pixels, scale)| {
                    let png =
                        png.as_ref().map(|image| image.bytes.clone()).or_else(|| encode(pixels.as_ref()?, *scale))?;
                    let drawn = image::annotate(&png, &kept.annotations, theme::SNEAK, *scale)?;
                    Some((png, drawn))
                });
                // Named in the brief: both files are written, or neither is named.
                if kept.image.is_some() {
                    let n = kept.n;
                    let written = dir.as_ref().zip(png.as_ref()).is_some_and(|(dir, (png, drawn))| {
                        let taken = config::write_private(dir, &format!("{n}.png"), png);
                        taken.and_then(|()| config::write_private(dir, &format!("{n}.annotated.png"), drawn)).is_ok()
                    });
                    if !written {
                        *kept = kept.clone().without_picture();
                    }
                }
                pictures.extend(png.map(|(_, drawn)| (kept.n, drawn)));
            }
            let text = capture::markdown_in(&brief, at.as_deref());
            let mut files = vec![("brief.md".to_string(), text.clone()), ("brief.json".to_string(), pretty(&brief))];
            files.extend(brief.materials.iter().map(|kept| (format!("{}.json", kept.n), pretty(kept))));
            let write = |dir: &PathBuf| {
                files.iter().try_for_each(|(name, text)| config::write_private(dir, name, text.as_bytes()))
            };
            let failed = dir.as_ref().and_then(|dir| write(dir).err());
            (brief, text, pictures, failed)
        });
        cx.spawn(async move |this, cx| {
            let (brief, text, pictures, failed) = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(err) = failed {
                    this.error = Some(format!("capture: could not keep the brief: {err}"));
                }
                // The brief as it was written, with the pictures to show beside it.
                if let Some(shown) = this.briefs.get_mut(&task).filter(|shown| shown.name == name) {
                    let pictures = pictures.into_iter().map(|(n, png)| (n, png_image(png))).collect();
                    (shown.brief, shown.text, shown.pictures) = (brief, text, pictures);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A task's brief: this run's, else read from the task's own folder. `None`: the task has
    /// no captures, or its brief is not there. What is on disk is input like any other: only a
    /// plain file of a size this build writes is read, and only if it is this task's.
    pub(crate) fn brief_of(&mut self, task: TaskId) -> Option<&Shown> {
        let name = self.model.tasks.get(task)?.materials_dir()?.to_string();
        if self.briefs.get(&task).is_none_or(|shown| shown.name != name) {
            self.briefs.remove(&task);
            // A brief that could not be read is not read again at every action.
            if self.no_brief.contains(&name) {
                return None;
            }
            let dir = task_dir(task, &name)?;
            // Not a link, a pipe or a device: reading one could wait for ever.
            let read = read_plain(&dir.join("brief.json"), MAX_BRIEF_BYTES);
            let brief = read.and_then(|bytes| serde_json::from_slice::<Brief>(&bytes).ok());
            let Some(brief) = brief.filter(|brief| brief.task == task) else {
                self.no_brief.insert(name);
                return None;
            };
            // Each annotated picture, read the same way, by the capture's number: never by a
            // name the brief's file holds.
            let picture = |kept: &capture::Kept| {
                let file = dir.join(format!("{}.annotated.png", kept.n));
                kept.annotated
                    .as_ref()
                    .and_then(|_| read_plain(&file, MAX_PICTURE_BYTES))
                    .map(|png| (kept.n, png_image(png)))
            };
            let pictures = brief.materials.iter().filter_map(picture).collect();
            let text = capture::markdown_in(&brief, Some(&dir.to_string_lossy()));
            self.briefs.insert(task, Shown { brief, name, dir: Some(dir), text, pictures });
        }
        self.briefs.get(&task)
    }

    /// The `task_brief` request.
    pub(crate) fn serve_brief(&mut self, id: Option<TaskId>, tab: Option<u32>) -> String {
        let of_tab = tab.and_then(|tab| self.model.tabs.iter().find(|t| t.id == tab)).and_then(|t| t.task);
        let talking = self.model.talk.map(|talk| talk.task);
        let Some(id) = id.or(of_tab).or(self.model.current_task).or(talking) else {
            return control::error("name a task: {\"cmd\":\"task_brief\",\"id\":7}");
        };
        match self.model.tasks.get(id).map(|task| task.materials.is_empty()) {
            None => return control::error(&format!("no task {id}")),
            Some(true) => return control::error(&format!("task {id} has no materials: nothing was captured for it")),
            Some(false) => {}
        }
        match self.brief_of(id) {
            Some(shown) => {
                // Each capture's pictures by their whole path, made from its number: the picture
                // is the capture, and a reader opens it from wherever it runs.
                let mut brief = serde_json::json!(shown.brief);
                let materials = brief["materials"].as_array_mut().into_iter().flatten().zip(&shown.brief.materials);
                for (material, kept) in materials {
                    let path = |name: String| shown.dir.as_ref().map(|dir| dir.join(name));
                    if kept.image.is_some() {
                        material["image"] = serde_json::json!(path(format!("{}.png", kept.n)));
                        material["annotated"] = serde_json::json!(path(format!("{}.annotated.png", kept.n)));
                    }
                }
                serde_json::json!({
                    "ok": true,
                    "task": id,
                    "dir": shown.dir,
                    "path": shown.dir.as_ref().map(|dir| dir.join("brief.md")),
                    "brief": brief,
                })
                .to_string()
            }
            None => control::error(&format!("task {id}'s brief is not in its folder any more")),
        }
    }

    /// ⌘Z or ⇧⌘Z in annotate mode with no note being typed: a step of what was drawn. Their
    /// bindings run before any key handler, so this is asked from there. False: not its key.
    pub(crate) fn annotate_step(&mut self, step: Action, window: &Window, cx: &mut Context<Self>) -> bool {
        let mine = self.model.mode() == Mode::Annotate && self.model.field().is_none();
        if mine {
            self.dispatch(step, self.input, window, cx);
        }
        mine
    }

    /// Annotate mode takes every key but ⌘ shortcuts. While a note is typed its field has them:
    /// ↵ keeps the note, Esc drops it. Else ⌫ removes the selected annotation, ↵ goes to the
    /// draft's title and Esc leaves. (⌘Z and ⇧⌘Z: `annotate_step`.)
    pub(crate) fn annotate_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let (key, m) = (event.keystroke.key.as_str(), event.keystroke.modifiers);
        let typing = self.model.field().is_some();
        let action = match key {
            _ if event.is_held && matches!(key, "enter" | "escape") => return cx.stop_propagation(),
            "enter" if typing => Action::AnnotateNote { keep: true },
            "escape" if typing => Action::AnnotateNote { keep: false },
            _ if typing => {
                if self.field_key(event, window, cx) {
                    cx.stop_propagation();
                }
                return;
            }
            "enter" => Action::AnnotateLeave { title: true },
            "escape" => Action::AnnotateLeave { title: false },
            "backspace" | "delete" => Action::AnnotateRemove,
            // The other ⌘ shortcuts reach their bindings; a plain key types nowhere.
            _ if m.platform => return,
            _ => return cx.stop_propagation(),
        };
        self.dispatch(action, self.input, window, cx);
        cx.stop_propagation();
    }

    /// Where `position` is on the picture being annotated, 0..1 of it, and the picture's size.
    fn on_picture(&self, position: gpui::Point<Pixels>) -> Option<([f64; 2], [f64; 2])> {
        let b = self.annotate_at.get();
        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
        let local = position - b.origin;
        let at = [f64::from(f32::from(local.x) / w), f64::from(f32::from(local.y) / h)];
        (w >= 1.0 && h >= 1.0).then_some((at, [f64::from(w), f64::from(h)]))
    }

    fn annotate_press(&mut self, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        let m = event.modifiers;
        if let Some((at, size)) = self.on_picture(event.position) {
            self.dispatch(Action::AnnotatePress { at, shift: m.shift, alt: m.alt, size }, self.input, window, cx);
        }
    }

    /// The pointer moved, anywhere in the window: a drag begun on the picture goes on.
    pub(crate) fn annotate_move(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let drawing = self.model.annotated().is_some_and(|material| material.marks.drawing());
        let Some((at, _)) = self.on_picture(event.position).filter(|_| drawing) else { return };
        let action = match event.pressed_button {
            Some(MouseButton::Left) => Action::AnnotateDrag { to: at },
            // The button came up outside the window.
            _ => Action::AnnotateRelease { at },
        };
        self.dispatch(action, self.input, window, cx);
    }

    pub(crate) fn annotate_release(&mut self, position: gpui::Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let drawing = self.model.annotated().is_some_and(|material| material.marks.drawing());
        if let Some((at, _)) = self.on_picture(position).filter(|_| drawing) {
            self.dispatch(Action::AnnotateRelease { at }, self.input, window, cx);
        }
    }

    /// What a control of the bar, the tool strip or the brief does, by its name: its click, and
    /// its sneak label.
    pub(crate) fn click_capture(&mut self, control: &str, window: &Window, cx: &mut Context<Self>) {
        let number = control.rsplit_once(':').and_then(|(_, n)| n.parse::<u32>().ok());
        let action = match (control.split(':').next().unwrap_or_default(), number) {
            ("tool-ellipse", _) => Action::AnnotateTool { tool: Tool::Ellipse },
            ("tool-box", _) => Action::AnnotateTool { tool: Tool::Box },
            ("tool-stroke", _) => Action::AnnotateTool { tool: Tool::Stroke },
            ("undo", _) => Action::AnnotateUndo,
            ("redo", _) => Action::AnnotateRedo,
            ("done", _) => Action::AnnotateLeave { title: true },
            ("cancel", _) => Action::CaptureCancel,
            ("discard", _) => Action::CaptureDiscard { yes: true },
            ("keep", _) => Action::CaptureDiscard { yes: false },
            ("add", _) => Action::ComposeSubmit { open: false },
            ("start", _) => Action::ComposeSubmit { open: true },
            ("back", _) => Action::CaptureBack,
            ("brief-close", _) => Action::BriefClose,
            ("edit", Some(n)) => Action::CaptureEdit { material: n },
            ("remove", Some(n)) => Action::CaptureRemove { material: n },
            ("more", _) => {
                self.error = self.capture_pane(None, self.input, window, cx).err();
                return cx.notify();
            }
            _ => return,
        };
        self.dispatch(action, self.input, window, cx);
        cx.notify();
    }

    /// A small button of the bar or the strip. `name` is its control: `draft:<name>`.
    fn capture_button(&self, name: &'static str, label: String, lit: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let control = name.to_string();
        div()
            .id(SharedString::from(format!("draft-{name}")))
            .relative()
            .flex_none()
            .px_1p5()
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .text_color(rgb(if lit { theme::BACKGROUND } else { theme::MUTED }))
            .when(lit, |d| d.bg(rgb(theme::SNEAK)))
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)).text_color(rgb(theme::TEXT)))
            .child(label)
            .child(self.controls.mark(format!("draft:{name}")))
            // The press must not reach what is under it: a pane's header would start a drag.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.click_capture(&control, window, cx)))
    }

    /// The tool strip for the header of the pane being annotated.
    pub(crate) fn annotate_strip(&self, material: &Material, cx: &mut Context<Self>) -> Div {
        let tool = material.marks.tool;
        let tools =
            [("tool-ellipse", "○", Tool::Ellipse), ("tool-box", "▢", Tool::Box), ("tool-stroke", "✎", Tool::Stroke)];
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .children(tools.map(|(name, label, of)| self.capture_button(name, label.into(), tool == of, cx)))
            .child(self.capture_button("undo", "↶".into(), false, cx))
            .child(self.capture_button("redo", "↷".into(), false, cx))
            .child(self.capture_button("done", "done".into(), false, cx))
    }

    /// A frozen picture, as large as its parent lets it be.
    fn frozen(&self, shot: Option<&Shot>, focused: bool) -> AnyElement {
        match shot.map(|shot| &shot.frozen) {
            Some(Frozen::App { buffer, .. }) => surface(buffer.clone()).size_full().into_any_element(),
            Some(Frozen::Terminal { screen, .. }) => {
                let (screen, cell) = (screen.clone(), self.cell);
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| terminal::paint(&screen, bounds, cell, focused, window, cx),
                )
                .size_full()
                .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    /// The pane being annotated: its frozen picture with what was drawn on it, taking the mouse.
    pub(crate) fn render_annotate(&self, material: &Material, cx: &mut Context<Self>) -> AnyElement {
        let shot = self.shots.get(&material.n);
        let marks = &material.marks;
        let shapes: Vec<Shape> = marks.shapes.iter().cloned().chain(marks.preview()).collect();
        let at = self.annotate_at.clone();
        let chip = |p: [f64; 2]| {
            sneak::chip().absolute().left(relative(p[0] as f32)).top(relative(p[1] as f32)).max_w(px(360.)).text_xs()
        };
        let notes = marks.shapes.iter().enumerate().filter_map(|(i, shape)| match shape {
            Shape::Note { at, text } => Some(
                chip(*at)
                    .when(marks.selected == Some(i), |d| d.border_2().border_color(rgb(theme::TEXT)))
                    .child(text.clone()),
            ),
            _ => None,
        });
        // The note being typed: the app's text field, where the click was.
        let typing = marks.note.as_ref().map(|(at, _)| {
            chip(*at).w(px(220.)).text_color(rgb(theme::BACKGROUND)).child(self.render_field_of("note", "note", cx))
        });
        let picture = div().absolute().top_0().left_0();
        // At the size it was taken, from the pane's top left, as the pane draws a window's
        // stream: what is drawn on it is then where it is in the picture that is kept.
        let picture = match shot.map(|shot| &shot.frozen) {
            Some(Frozen::App { shown, .. } | Frozen::Terminal { shown, .. }) => picture.w(shown.width).h(shown.height),
            _ => picture.size_full(),
        };
        picture
            .id(("annotate", material.n))
            .cursor(CursorStyle::Crosshair)
            .child(self.frozen(shot, false))
            .child(canvas(move |bounds, _, _| at.set(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
            .child(shapes_layer(shapes, marks.selected))
            .children(notes)
            .children(typing)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    // Not a click on the pane under it: that would focus it, or enter its app.
                    cx.stop_propagation();
                    this.annotate_press(event, window, cx);
                }),
            )
            .into_any_element()
    }

    /// A capture's row in the bar: a thumbnail with what was drawn, where it is from, its first
    /// note, and × to take it out. A click annotates it again.
    fn material_row(&self, material: &Material, annotating: bool, cx: &mut Context<Self>) -> AnyElement {
        let n = material.n;
        let shot = self.shots.get(&n);
        // A terminal's picture is its PNG once encoded; a window's is the frozen frame itself.
        let picture = match shot {
            Some(Shot { frozen: Frozen::App { buffer, .. }, .. }) => {
                surface(buffer.clone()).object_fit(ObjectFit::Fill).size_full().into_any_element()
            }
            Some(Shot { png: Some(png), .. }) => {
                img(png.clone()).size_full().object_fit(ObjectFit::Fill).into_any_element()
            }
            _ => div().into_any_element(),
        };
        let thumb = div()
            .relative()
            .flex_none()
            .w(px(72.))
            .h(px(46.))
            .rounded_sm()
            .overflow_hidden()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(if annotating { theme::SNEAK } else { theme::BORDER_STRONG }))
            .child(picture)
            .child(shapes_layer(material.marks.shapes.clone(), None));
        // Pictures are not kept and the pane gave no text: the task would get the notes alone.
        let bare = !self.model.capture.as_ref().is_some_and(|bar| bar.images) && material.text_exposed() == Some(false);
        let warning = bare.then(|| {
            let line = div().text_xs().text_color(rgb(theme::ERROR));
            line.child("no text exposed and pictures are off: only your notes will be kept")
        });
        let note = material.first_note().map_or_else(
            || match material.marks.shapes.len() {
                0 => "nothing drawn yet".to_string(),
                1 => "1 annotation".to_string(),
                n => format!("{n} annotations"),
            },
            str::to_string,
        );
        div()
            .id(("material", n))
            .relative()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .when(annotating, |d| d.bg(rgb(theme::SELECTED)))
            .child(thumb)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_xs().child(material.source.label()))
                    .child(div().truncate().text_xs().text_color(rgb(theme::MUTED)).child(note))
                    .children(warning),
            )
            .child(self.controls.mark(format!("draft:edit:{n}")))
            .on_click(cx.listener(move |this, _, window, cx| this.click_capture(&format!("edit:{n}"), window, cx)))
            .child(
                div()
                    .id(("material-remove", n))
                    .relative()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .text_color(rgb(theme::MUTED))
                    .hover(|s| s.text_color(rgb(theme::ERROR)))
                    .child("×")
                    .child(self.controls.mark(format!("draft:remove:{n}")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation(); // not a click on the row
                        this.click_capture(&format!("remove:{n}"), window, cx);
                    })),
            )
            .into_any_element()
    }

    /// The right bar's frame: the drawer, as high as the window and as wide as it is.
    fn right_bar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let at = self.bar_at.clone();
        div()
            .id("right-bar")
            .relative()
            .flex_none()
            .w(px(self.drawer.width))
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(theme::RAISED))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(canvas(move |bounds, _, _| at.set(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
            // The bar is ottospace's own: a click on it takes the keyboard back from a live app.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::Leave, this.input, window, cx))),
            )
    }

    /// The draft in the right bar: the new-task line as its title, then what was collected.
    /// The workspace stays in use beside it.
    pub(crate) fn render_draft(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let bar = self.model.capture.as_ref()?;
        let muted = |text: String| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let key = KEY.get().map(|key| format!(" ({})", key_label(key))).unwrap_or_default();
        let rows: Vec<AnyElement> =
            bar.materials.iter().map(|m| self.material_row(m, bar.annotating == Some(m.n), cx)).collect();
        let open = self.model.open_windows().unwrap_or_default();
        let panes: Vec<String> = open.panes.iter().map(ottospace_core::capture::Source::label).collect();
        let tabs = if open.tabs.len() == 1 { "1 tab".to_string() } else { format!("{} tabs", open.tabs.len()) };
        let open = [tabs].into_iter().chain(panes).collect::<Vec<_>>().join(" · ");
        let back = match self.model.back_to.and_then(|pane| self.model.pane(pane)).map(|pane| &pane.kind) {
            Some(Kind::App { name, .. }) => Some(self.capture_button("back", format!("↩ back to {name}"), false, cx)),
            _ => None,
        };
        let buttons = bar.asked.then(|| {
            let discard = self.capture_button("discard", "Discard (↵)".into(), false, cx);
            div().flex().gap_2().child(discard).child(self.capture_button("keep", "Keep (esc)".into(), false, cx))
        });
        let asks = buttons.map(|buttons| {
            let n = bar.materials.len();
            let what = if n == 1 { "its capture".to_string() } else { format!("its {n} captures") };
            div()
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::CONFIRM))
                .text_xs()
                .child(format!("Discard this draft and {what}?"))
                .child(buttons)
        });
        let hint = match self.model.mode() {
            Mode::Annotate => "drag: circle · ⇧ box · ⌥ pen · click: note · ⌘Z undo · ⌫ remove · ↵ title · esc done",
            _ => "click a capture to draw on it · esc cancels the draft",
        };
        let bar = self
            .right_bar(cx)
            .child(div().absolute().top(px(8.)).right(px(8.)).child(self.capture_button(
                "cancel",
                "×".into(),
                false,
                cx,
            )))
            .child(self.new_task_content(cx))
            .children(asks)
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(theme::SNEAK))
                    .child(format!("MATERIALS {}", bar.materials.len())),
            )
            // As high as its rows, and it scrolls under its bar when they do not fit.
            .child(
                self.bars
                    .area(
                        "materials",
                        div().id("materials").flex().flex_col().min_h_0().overflow_y_scroll().children(rows),
                    )
                    .flex_initial(),
            )
            .child(self.capture_button("more", format!("+ capture the pane in use{key}"), false, cx))
            .child(muted(hint.to_string()))
            .child(muted(format!("Open: {open}")))
            .children(back)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.capture_button("add", "Add task (↵)".into(), true, cx))
                    .child(self.capture_button("start", "Add and start (⌘↵)".into(), false, cx)),
            );
        Some(bar.into_any_element())
    }

    /// A task's brief in the right bar, read-only: how an agent gets it, each capture's picture,
    /// then the brief as it is written.
    pub(crate) fn render_brief(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = self.brief_content(cx)?;
        Some(self.right_bar(cx).p_0().child(content).into_any_element())
    }

    /// The brief itself, for the right bar or the drawer.
    pub(crate) fn brief_content(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.model.brief?;
        let shown = self.briefs.get(&id)?;
        let muted = |text: String| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let picture = |kept: &ottospace_core::capture::Kept| {
            // Held since it was drawn, or since the brief was read from the task's folder.
            let picture = shown.pictures.iter().find(|(n, _)| *n == kept.n).map(|(_, image)| img(image.clone()))?;
            let frame = div().h(px(170.)).rounded_sm().overflow_hidden().bg(rgb(theme::BACKGROUND));
            Some(
                frame
                    .border_1()
                    .border_color(rgb(theme::BORDER_STRONG))
                    .child(picture.size_full().object_fit(ObjectFit::Contain)),
            )
        };
        let materials = shown.brief.materials.iter().map(|kept| {
            let taken = &kept.captured.content;
            let found: usize = taken.under.iter().map(|under| under.nodes.len() + under.lines.len()).sum();
            let source = format!("{} · {}", kept.n, taken.source.label());
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().truncate().text_xs().child(source))
                .children(picture(kept))
                .child(muted(format!("{} annotations · {found} captured lines", kept.annotations.len())))
        });
        let fetch = format!("ottospace ctl '{{\"cmd\":\"task_brief\",\"id\":{id}}}'");
        let file = shown.dir.as_ref().map(|dir| muted(dir.join("brief.md").to_string_lossy().into_owned()));
        let lines = shown.text.lines().map(|line| div().min_h(px(8.)).child(line.to_string()));
        let bar = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(div().text_xs().text_color(rgb(theme::SNEAK)).child(format!("BRIEF · TASK {id}")))
                    .child(self.capture_button("brief-close", "×".into(), false, cx)),
            )
            .child(div().text_sm().child(shown.brief.ask.title.clone()))
            .child(muted("An agent in this task's terminals has OTTOSPACE_TASK set, and reads the brief with:".into()))
            .child(div().text_xs().font_family("Menlo").child(fetch))
            .children(file)
            .child(
                self.bars.area(
                    "brief",
                    div()
                        .id("brief-text")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(materials)
                        .child(
                            div().pt_2().text_xs().font_family("Menlo").text_color(rgb(theme::MUTED)).children(lines),
                        ),
                ),
            );
        Some(bar.into_any_element())
    }

    /// What was drawn of the draft and the brief, for the state: where the bar and the picture
    /// being annotated are, which pictures are encoded, and where the draft's files go.
    pub(crate) fn capture_snapshot(&self) -> serde_json::Value {
        let bar = self.model.capture.as_ref();
        let shown = self.model.brief.filter(|_| bar.is_none()).and_then(|id| self.briefs.get(&id));
        let open = bar.is_some() || shown.is_some();
        let mut encoded: Vec<u32> = self.shots.iter().filter(|(_, shot)| shot.png.is_some()).map(|(n, _)| *n).collect();
        encoded.sort_unstable();
        serde_json::json!({
            "key": KEY.get(),
            "bar": open.then(|| crate::rect(self.bar_at.get())),
            "picture": self.model.annotated().map(|_| crate::rect(self.annotate_at.get())),
            "encoded": encoded,
            // Pictures asked for and not taken yet: the pane is being drawn beside the bar.
            "pending": self.pending.len(),
            "dir": bar.and_then(|bar| draft_dir(&bar.draft)),
            "brief": shown.map(|shown| serde_json::json!({ "task": shown.brief.task, "dir": shown.dir })),
        })
    }
}

/// A key's name as the user reads it: `ctrl-alt-cmd-space` → `⌃⌥⌘Space`.
pub fn key_label(name: &str) -> String {
    let part = |part: &str| match part.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "⌃".to_string(),
        "alt" | "option" => "⌥".to_string(),
        "shift" => "⇧".to_string(),
        "cmd" => "⌘".to_string(),
        "space" => "Space".to_string(),
        other => other.to_uppercase(),
    };
    name.split('-').map(part).collect()
}

/// Registers the capture key: `$OTTOSPACE_CAPTURE_KEY`, else `capture_key` in the settings,
/// else the default. `off`: none. A test run registers one only if it names it: the key is
/// taken from every app while the run lasts.
/// Returns what went wrong, for the status bar: the key is another app's, or no key at all.
pub fn register_key(test_run: bool) -> Option<String> {
    let named = std::env::var("OTTOSPACE_CAPTURE_KEY").ok();
    if test_run && named.is_none() {
        return None;
    }
    let name = named.or(config::get().settings.capture_key.clone());
    let name = name.unwrap_or_else(|| ottospace_core::settings::CAPTURE_KEY.to_string());
    if name.eq_ignore_ascii_case("off") {
        return None;
    }
    let registered = ottospace_mac::hotkey::combo(&name)
        .ok_or_else(|| "it is not a key with a modifier, e.g. ctrl-alt-cmd-space".to_string())
        .and_then(|(code, modifiers)| ottospace_mac::hotkey::register_capture(code, modifiers));
    match registered {
        Ok(()) => {
            let _ = KEY.set(name.to_uppercase());
            None
        }
        Err(err) => Some(format!("capture key {} is off: {err}", key_label(&name))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_frame_becomes_rgb() {
        // 2×2: white, black over mid grey twice; no colour (Cb = Cr = 128).
        let rgba = nv12_to_rgba(&[255, 0, 128, 128], 2, &[128, 128], 2, 2, 2);
        assert_eq!(rgba, [255, 255, 255, 255, 0, 0, 0, 255, 128, 128, 128, 255, 128, 128, 128, 255]);
        // A row is `stride` bytes, of which the first `width` are pixels: red, with padding.
        let red = nv12_to_rgba(&[54, 9, 54, 9], 2, &[99, 255, 7, 7], 4, 1, 2);
        assert_eq!(red, [254, 0, 0, 255, 254, 0, 0, 255]);
        // Planes shorter than the size says read as black and grey, not past their end.
        assert_eq!(nv12_to_rgba(&[], 2, &[], 2, 1, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn a_draft_folder_is_named_only_by_the_shells_own_names() {
        assert_eq!(key_label("ctrl-alt-cmd-space"), "⌃⌥⌘Space");
        assert_eq!(key_label("CMD-SHIFT-2"), "⌘⇧2");
        assert_eq!((draft_pid("draft-123-456"), draft_pid("draft-0-1")), (Some(123), Some(0)));
        let wrong =
            ["draft-../x", "7", "draft-1/2", "", "draft-1", "draft-1-2-3", "draft--5", "draft-x-1", "Draft-1-2"];
        assert!(wrong.iter().all(|name| draft_pid(name).is_none()));
        // A word for a task's folder and a brief's fence: letters and digits, new each time.
        let (one, two) = (new_word().expect("a word"), new_word().expect("a word"));
        assert!(one != two && one.len() == 24 && one.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(ottospace_core::tasks::materials_dir(7, &one).is_some());
    }

    #[test]
    fn only_a_plain_file_of_a_size_is_read() {
        let dir = std::env::temp_dir().join(format!("otto-read-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (file, link) = (dir.join("a"), dir.join("link"));
        std::fs::write(&file, b"12345").unwrap();
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_plain(&file, 5).as_deref(), Some(&b"12345"[..]));
        // Too large, a link, a folder, nothing there.
        assert_eq!((read_plain(&file, 4), read_plain(&link, 9), read_plain(&dir, 9)), (None, None, None));
        assert_eq!(read_plain(&dir.join("none"), 9), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
