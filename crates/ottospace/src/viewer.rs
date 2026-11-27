//! Shell half of the file viewer (`ottospace_core::viewer`, `model::viewer`): reads the file a
//! viewer pane names on a thread of its own (a regular file only, at most `MAX_BYTES`), looks for
//! changes twice a second and reads it again once a change has settled, draws it (markdown by
//! the conversation's renderer, lines with their numbers, or a picture), and tells the core what
//! it read and where the view is. Never its text: the state has the path, kind, line and count.

use std::cell::Cell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    AnyElement, Bounds, Context, ElementId, HighlightStyle, ImageSource, KeyDownEvent, Pixels, Point, ScrollHandle,
    SharedString, StyledText, UniformListScrollHandle, Window, canvas, div, img, point, prelude::*, px, rgb,
    uniform_list,
};
use ottospace_core::diff::Sign;
use ottospace_core::review;
use ottospace_core::terminal::CellPoint;
use ottospace_core::viewer::{self, MAX_BYTES, MAX_IMAGE_BYTES, Read, Shows, Viewer};
use ottospace_core::{Action, Axis, Origin, PaneId, theme};

use crate::Workspace;
use crate::conversation::Links;
use crate::highlight::{self, Mark};
use crate::markdown::{self, Block};
use crate::syntax::{self, Colours};
use crate::viewer_picture::{self, Picture};

/// One line of code or text, and one step of a markdown view (`Viewer::top`), in points.
pub const LINE: f32 = 18.;
/// The most characters of a line drawn; the file keeps the rest.
pub const MAX_DRAWN: usize = 2000;
/// How often a viewer's file is looked at, and how long a change must hold before it is read.
const LOOK: Duration = Duration::from_millis(500);
const SETTLE: Duration = Duration::from_millis(200);

/// What a file was when it was read: a change to any of these reads it again.
type Stamp = Option<(SystemTime, u64, u64)>;

/// What was read, ready to draw.
enum Body {
    None,
    Markdown(Rc<Vec<Block>>),
    /// Each line, and its coloured runs (`highlight`), by byte within it; or by its grammar.
    Lines(Rc<Vec<String>>, Rc<Vec<Vec<Mark>>>, Option<Rc<Colours>>),
    Picture(Rc<Picture>),
    /// A diff's rows (`review`), and each row's coloured runs by the lexer.
    Diff(Rc<review::Patch>, Rc<Vec<Vec<Mark>>>),
}

/// What a read thread hands back: the outcome, and the bytes a picture is drawn from or the text.
enum Loaded {
    Text(String),
    /// Markdown parsed, or text split into lines, off the main thread: a large file does not
    /// stall the window.
    Markdown(Vec<Block>),
    Lines(Vec<String>, Vec<Vec<Mark>>, Option<Colours>),
    Picture(Picture),
    Diff(review::Patch, Vec<Vec<Mark>>),
}

/// A viewer pane's file as the shell holds it.
pub struct FileView {
    path: String,
    /// Shows the file's change since the last commit (`Viewer::diff`), read from git.
    diff: bool,
    /// The checkouts git may be asked about (`status_bar::reads_changes`).
    roots: Vec<String>,
    body: Body,
    reading: Option<Receiver<(Read, Option<Loaded>, Stamp)>>,
    /// What the file was when last read; a change seen since, and when it was first seen.
    stamp: Stamp,
    seen: Option<(Stamp, Instant)>,
    looked: Instant,
    /// A look in progress, off the main thread: a stat on a share that stopped answering waits.
    looking: Option<Receiver<Stamp>>,
    scroll: ScrollHandle,
    lines: UniformListScrollHandle,
    /// The view goes to the core's line once the read is laid out (a restore, a reload).
    place: bool,
    /// What was last told the core: `[top, most, page]`.
    told: Option<[u32; 3]>,
    /// A picture: where it was last laid out, when its frames began to play, the frame on show,
    /// and where a drag that pans it was last.
    area: Rc<Cell<Bounds<Pixels>>>,
    began: Instant,
    frame: Cell<usize>,
    dragged: Cell<Option<Point<Pixels>>>,
}

impl FileView {
    fn new(path: String, shows: Shows, diff: bool, roots: Vec<String>) -> Self {
        let mut view = FileView {
            path,
            diff,
            roots,
            body: Body::None,
            reading: None,
            stamp: None,
            seen: None,
            looked: Instant::now(),
            looking: None,
            scroll: ScrollHandle::new(),
            lines: UniformListScrollHandle::new(),
            place: true,
            told: None,
            area: Rc::default(),
            began: Instant::now(),
            frame: Cell::new(0),
            dragged: Cell::new(None),
        };
        view.read(shows);
        view
    }

    fn read(&mut self, shows: Shows) {
        let (send, read) = channel();
        let path = self.path.clone();
        if self.diff {
            let roots = self.roots.clone();
            std::thread::spawn(move || {
                let at = diff_stamp(&path);
                let (read, loaded) = read_diff(&path, &roots);
                let _ = send.send((read, loaded, at));
                ottospace_mac::wake::post();
            });
            self.reading = Some(read);
            return;
        }
        std::thread::spawn(move || {
            // Before the read: a write during it changes the stamp, and the next look reads again.
            let at = stamp(&path);
            let (read, loaded) = match read_file(Path::new(&path), shows) {
                (read, Some(Loaded::Text(text))) if shows == Shows::Markdown => {
                    let dir = name_and_dir(&path).1;
                    (read, Some(Loaded::Markdown(markdown::parse_with(&text, Some(&dir), markdown::FILE))))
                }
                // A code file coloured by its grammar (`rs`, `Makefile`), else as a fence of its
                // extension would be (`jsonl`): the lexer, also for a parse too large or slow.
                (read, Some(Loaded::Text(text))) => {
                    let lines = text.lines().map(str::to_string).collect();
                    let lang = ottospace_core::viewer::lang_of(&path).filter(|_| shows == Shows::Code);
                    let tree = lang
                        .and_then(|lang| Some((lang, syntax::parse(lang, &text, syntax::deadline(syntax::BUDGET))?)));
                    match tree {
                        Some((lang, tree)) => {
                            (read, Some(Loaded::Lines(lines, Vec::new(), Some(Colours::new(lang, tree, text)))))
                        }
                        None => {
                            let marks = highlight::by_line(&text, &highlight::marks(extension(&path), &text));
                            (read, Some(Loaded::Lines(lines, marks, None)))
                        }
                    }
                }
                read => read,
            };
            let _ = send.send((read, loaded, at));
            ottospace_mac::wake::post();
        });
        self.reading = Some(read);
    }

    /// A picture's frames, to leave GPUI's atlas when it goes.
    fn frames(&self) -> Vec<Arc<gpui::RenderImage>> {
        match &self.body {
            Body::Picture(picture) => picture.frames.iter().map(|(frame, _)| frame.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// The scroll handle of what is drawn: a list keeps its offset in a plain one, shared.
    fn handle(&self) -> ScrollHandle {
        match self.body {
            Body::Lines(..) | Body::Diff(..) => self.lines.0.borrow().base_handle.clone(),
            _ => self.scroll.clone(),
        }
    }
}

/// The file at `path`: opened without waiting (a pipe would), then judged by what was opened, so
/// a link swapped meanwhile is judged as what it now is. Only a regular file of at most
/// `MAX_BYTES` is read.
fn read_file(path: &Path, shows: Shows) -> (Read, Option<Loaded>) {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY).open(path);
    let file = match file {
        Ok(file) => file,
        Err(err) if matches!(err.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {
            return (Read::Missing, None);
        }
        Err(_) => return (Read::Unreadable, None),
    };
    let Ok(meta) = file.metadata() else { return (Read::Unreadable, None) };
    if !meta.is_file() {
        return (Read::NotFile, None);
    }
    let most = if shows == Shows::Image { MAX_IMAGE_BYTES } else { MAX_BYTES };
    if meta.len() > most {
        return (Read::TooLarge { bytes: meta.len() }, None);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    if (&file as &File).take(most + 1).read_to_end(&mut bytes).is_err() {
        return (Read::Unreadable, None);
    }
    if bytes.len() as u64 > most {
        return (Read::TooLarge { bytes: bytes.len() as u64 }, None);
    }
    // Decoded here, on the read's thread, under the viewer's caps.
    if shows == Shows::Image {
        let read = bytes.len() as u64;
        return match viewer_picture::decode(&bytes) {
            Ok(picture) => {
                let (width, height, frames) = (picture.width, picture.height, picture.frames.len() as u32);
                (Read::Picture { width, height, frames, bytes: read }, Some(Loaded::Picture(picture)))
            }
            Err(said) => (said, None),
        };
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return (Read::Binary, None);
    }
    match String::from_utf8(bytes) {
        Ok(text) => (Read::Shown { lines: line_count(&text) }, Some(Loaded::Text(text))),
        Err(_) => (Read::Binary, None),
    }
}

/// How long git may take over one file's diff, and the most it may write.
const DIFF_TIME: Duration = Duration::from_secs(5);
const DIFF_BYTES: u64 = MAX_BYTES;

/// The change to the file at `path` since the last commit (`git diff HEAD`, hardened by
/// `child::git_read`), parsed, its lines coloured by the lexer: only in a checkout Repos lists.
fn read_diff(path: &str, roots: &[String]) -> (Read, Option<Loaded>) {
    let Some(real) = fs::canonicalize(path).ok() else { return (Read::Missing, None) };
    // Its checkout from git's own files, resolved, checked before any git runs.
    let top = real.parent().and_then(crate::repos::checkout_of).and_then(|(top, _)| top.canonicalize().ok());
    let top = top.and_then(|top| top.to_str().map(str::to_string));
    let Some(top) = top.filter(|top| ottospace_core::status_bar::reads_changes(top, roots)) else {
        return (Read::Unreadable, None);
    };
    let Some(rel) = real.strip_prefix(&top).ok().map(|rel| rel.to_string_lossy().into_owned()) else {
        return (Read::Unreadable, None);
    };
    // A fixed argv; the path a literal pathspec, never a pattern.
    let mut diff = crate::child::git_read(Path::new(&top));
    diff.env("GIT_LITERAL_PATHSPECS", "1");
    diff.args(["diff", "--no-color", "--no-ext-diff", "--no-textconv", "-U3", "HEAD", "--"]).arg(&rel);
    // Past the cap git is stopped: too large, never "no change". A git that failed: unread.
    let said = match crate::child::run(diff, DIFF_TIME, DIFF_BYTES) {
        Ok(said) if said.out.len() as u64 >= DIFF_BYTES => return (Read::TooLarge { bytes: DIFF_BYTES }, None),
        Ok(said) if said.ok => said,
        _ => return (Read::Unreadable, None),
    };
    let patch = review::parse(&said.out);
    if patch.binary {
        return (Read::Binary, None);
    }
    // No line changed: none at all, or only its mode, or a new empty file.
    if patch.rows.is_empty() {
        return (Read::NoChange, None);
    }
    // One line a row (a hunk's start an empty one), coloured as the file's kind.
    let text: Vec<&str> = (patch.rows.iter())
        .map(|row| match row {
            review::Row::Line { text, .. } => text.as_str(),
            review::Row::Hunk { .. } => "",
        })
        .collect();
    let text = text.join("\n");
    let marks = highlight::by_line(&text, &highlight::marks(extension(path), &text));
    let read = Read::Diff {
        rows: patch.rows.len() as u32,
        hunks: patch.hunks(),
        added: patch.added,
        removed: patch.removed,
        cut: patch.cut,
    };
    (read, Some(Loaded::Diff(patch, marks)))
}

/// The extension of the file at `path`: `rs` of `src/main.rs`, empty for none.
fn extension(path: &str) -> &str {
    Path::new(path).extension().and_then(|ext| ext.to_str()).unwrap_or("")
}

/// Lines as an editor numbers them: a last line with no newline counts, an empty file has none.
fn line_count(text: &str) -> u32 {
    let lines = text.split('\n').count() - usize::from(text.ends_with('\n') || text.is_empty());
    lines.min(u32::MAX as usize) as u32
}

/// A diff view's stamp: its file's, and its checkout's `HEAD` log (`repos::head_log`), so a
/// commit with the file untouched reads its diff again.
fn diff_stamp(path: &str) -> Stamp {
    let head =
        Path::new(path).parent().and_then(crate::repos::checkout_of).and_then(|(top, _)| crate::repos::head_log(&top));
    stamp(path).map(|(at, len, ino)| (at, len ^ head.unwrap_or(0).rotate_left(32), ino))
}

fn stamp(path: &str) -> Stamp {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len(), meta.ino()))
}

/// Why a client's `open_file` opens nothing: a path the viewer does not take, or no regular file
/// there now. `None`: it opens.
pub fn open_refusal(path: &str) -> Option<String> {
    if !viewer::sound(path) {
        return Some("open_file: the path must be absolute and one line".into());
    }
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => None,
        Ok(_) => Some(format!("{path} is not a regular file")),
        Err(_) => Some(format!("no file at {path}")),
    }
}

/// The file name a viewer's header shows, and the folder its links are judged in.
pub fn name_and_dir(path: &str) -> (String, String) {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let dir = Path::new(path).parent().map_or_else(|| "/".into(), |dir| dir.to_string_lossy().into_owned());
    (name, dir)
}

/// The keys a viewer takes, named as `vim::Keys::press` reads them: the character typed, else
/// the key's name, `ctrl-` before it with ⌃. `None`: a ⌘ or ⌥ shortcut, which is not the viewer's.
fn key_name(event: &KeyDownEvent) -> Option<String> {
    let (key, m) = (&event.keystroke, event.keystroke.modifiers);
    if m.platform || m.alt || m.function {
        return None;
    }
    if m.control {
        return Some(format!("ctrl-{}", key.key));
    }
    match key.key_char.as_deref() {
        Some(typed) if typed.chars().count() == 1 && !typed.chars().any(char::is_control) => Some(typed.to_string()),
        _ if key.key == "g" && m.shift => Some("G".into()),
        _ => Some(key.key.clone()),
    }
}

impl Workspace {
    /// When `sync_viewers` next looks for a change on disk: the main loop sleeps until then.
    pub(crate) fn viewers_due(&self) -> Option<Instant> {
        let due = |view: &FileView| {
            let every = if view.seen.is_some() { SETTLE / 2 } else { LOOK };
            (view.reading.is_none() && view.looking.is_none()).then(|| view.looked + every)
        };
        let disk = self.viewers.values().filter_map(due).min();
        // An animation on show: its next frame.
        let frames = self.animated_panes().into_iter().filter_map(|id| {
            let (view, core) = (self.viewers.get(&id)?, self.model.viewer(id)?);
            let Body::Picture(picture) = &view.body else { return None };
            playing(core).then(|| picture.next_change(view.began.elapsed()).map(|at| view.began + at))?
        });
        disk.into_iter().chain(frames).min()
    }

    /// Each tick: a view for each viewer pane, the files read handed to the core, changes on
    /// disk read again once they settle, and the core told where each view on show is.
    pub(crate) fn sync_viewers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let files: HashMap<PaneId, (String, Shows, bool)> = self
            .model
            .viewers
            .iter()
            .filter_map(|(id, view)| match &self.model.pane(*id)?.kind {
                ottospace_core::Kind::File { path } => Some((*id, (path.clone(), view.shows, view.diff))),
                _ => None,
            })
            .collect();
        // Pictures of viewers closed or reused since (`forget_viewer`), and those replaced below.
        let mut retired = std::mem::take(&mut self.retired_pictures);
        self.viewers.retain(|id, view| {
            // A file's view and its diff's are two: one going to the other is read again.
            let kept = files.get(id).is_some_and(|(path, _, diff)| *path == view.path && *diff == view.diff);
            if !kept {
                retired.extend(view.frames());
            }
            kept
        });
        let mut actions = Vec::new();
        // The checkouts Repos lists now: a diff read again asks git only of these.
        let roots = if files.values().any(|(.., diff)| *diff) { self.model.checkout_roots() } else { Vec::new() };
        for (id, (path, shows, diff)) in &files {
            let view =
                self.viewers.entry(*id).or_insert_with(|| FileView::new(path.clone(), *shows, *diff, roots.clone()));
            if *diff {
                view.roots.clone_from(&roots);
            }
            if let Some(Ok((read, loaded, at))) = view.reading.as_ref().map(Receiver::try_recv) {
                view.reading = None;
                view.stamp = at;
                let (_, dir) = name_and_dir(path);
                // The frames it drew before leave GPUI's atlas, whatever comes in their place.
                retired.extend(view.frames());
                view.body = match loaded {
                    Some(Loaded::Markdown(blocks)) => Body::Markdown(Rc::new(blocks)),
                    Some(Loaded::Lines(lines, marks, colours)) => {
                        Body::Lines(Rc::new(lines), Rc::new(marks), colours.map(Rc::new))
                    }
                    Some(Loaded::Picture(picture)) => {
                        view.began = Instant::now();
                        Body::Picture(Rc::new(picture))
                    }
                    Some(Loaded::Diff(patch, marks)) => Body::Diff(Rc::new(patch), Rc::new(marks)),
                    // The read thread made any text lines or markdown.
                    Some(Loaded::Text(_)) | None => Body::None,
                };
                if let Body::Markdown(blocks) = &view.body {
                    // Each picture once, however often it is named: one read thread each.
                    let mut wanted = std::collections::BTreeSet::new();
                    crate::conversation::images(blocks, &mut |image| drop(wanted.insert(image.to_string())));
                    for image in wanted {
                        // Read again with the file: a picture beside it may have changed too.
                        let thumb = crate::conversation::read_thumb(image.clone(), Some(dir.clone()));
                        self.drawer.thumbs.insert(image, thumb);
                    }
                }
                view.place = true;
                actions.push(Action::ViewerRead { pane: *id, path: path.clone(), read });
                cx.notify();
            }
            // A change on disk: read again once it has held still for `SETTLE` (an editor's save
            // may write twice). Looked at more often while a change settles.
            let every = if view.seen.is_some() { SETTLE / 2 } else { LOOK };
            if view.reading.is_none() && view.looking.is_none() && view.looked.elapsed() >= every {
                view.looked = Instant::now();
                let (send, looking) = channel();
                let (path, diff) = (path.clone(), view.diff);
                std::thread::spawn(move || {
                    let _ = send.send(if diff { diff_stamp(&path) } else { stamp(&path) });
                    ottospace_mac::wake::nudge();
                });
                view.looking = Some(looking);
            }
            let looked = view.looking.as_ref().map(Receiver::try_recv);
            if let Some(Ok(now)) = looked {
                view.looking = None;
                match &view.seen {
                    _ if now == view.stamp => view.seen = None,
                    Some((seen, since)) if *seen == now => {
                        if since.elapsed() >= SETTLE {
                            view.seen = None;
                            view.read(*shows);
                        }
                    }
                    _ => view.seen = Some((now, Instant::now())),
                }
            }
        }
        // Where each drawn view is, once laid out: put at the core's line after a read, then told.
        let drawn = self.model.shown_panes();
        for id in drawn {
            let (Some(view), Some(core)) = (self.viewers.get_mut(&id), self.model.viewer(id)) else { continue };
            if view.reading.is_some() || matches!(view.body, Body::None) {
                continue;
            }
            let handle = view.handle();
            let (height, max) = (f32::from(handle.bounds().size.height), f32::from(handle.max_offset().height));
            if height <= 0. {
                continue;
            }
            if view.place {
                view.place = false;
                handle.set_offset(point(px(0.), -px((core.top as f32 * LINE).min(max))));
                view.told = None;
                cx.notify();
            }
            let (most, page) = ((max / LINE).ceil() as u32, (height / LINE).floor() as u32);
            // At the end is at `most`, though the last step is less than a line.
            let offset = -f32::from(handle.offset().y);
            let top = if offset >= max - 1. && max > 0. { most } else { (offset / LINE).round().max(0.) as u32 };
            let now = [top.min(most), most, page];
            if view.told != Some(now) {
                view.told = Some(now);
                actions.push(Action::ViewerScrolled { pane: id, top: now[0], most, page });
            }
        }
        // An animation on show moves to its next frame when its time comes; none at rest, none
        // off show, none paused (or under Reduce Motion, until played).
        for id in self.animated_panes() {
            let (Some(view), Some(core)) = (self.viewers.get(&id), self.model.viewer(id)) else { continue };
            let Body::Picture(picture) = &view.body else { continue };
            // Paused, it stays on the frame on show (the first until it ever played).
            let at = if playing(core) { picture.frame_at(view.began.elapsed()) } else { view.frame.get() };
            if view.frame.replace(at) != at {
                self.picture_frames += 1;
                cx.notify();
            }
        }
        for image in retired {
            cx.drop_image(image, Some(window));
        }
        for action in actions {
            self.dispatch(action, Origin::System, window, cx);
        }
    }

    /// "File…" in the picker: the open panel, then the file the user picked in a viewer, in a new
    /// tab first with `new_tab`. A test run shows no panel; it counts the ask.
    pub(crate) fn choose_file(&mut self, axis: Axis, new_tab: bool, window: &Window, cx: &mut Context<Self>) {
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            self.file_prompts += 1;
            return;
        }
        let options =
            gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Open".into()) };
        let chosen = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Some(path) = paths.first().map(|path| path.to_string_lossy().into_owned()) else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if new_tab {
                    this.dispatch(Action::NewEmptyTab, Origin::User, window, cx);
                }
                let open = Action::OpenFile { path, axis, beside: None, reuse: false, at: None };
                this.dispatch(open, Origin::User, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// ⌘-click in terminal `id` at `at`: a path there to a text or markdown file (from the shell's
    /// folder, or `~`) opens in the viewer beside, reused for the next one, at the line written
    /// after it (`a.rs:12`, `a.rs:12-14`). False: no such file, and the click is the terminal's as before.
    pub(crate) fn terminal_path_click(
        &mut self,
        id: PaneId,
        at: CellPoint,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(terminal) = self.terminals.get(&id) else { return false };
        let Some((typed, lines)) = terminal.lines().get(at.row).and_then(|line| viewer::path_at(line, at.col)) else {
            return false;
        };
        let cwd = terminal.cwd().or_else(|| self.model.pane(id).and_then(|pane| pane.cwd.clone()));
        let home = std::env::var("HOME").ok();
        let Some(path) = viewer::absolute(&typed, cwd.as_deref(), home.as_deref()) else { return false };
        let Some(real) = fs::canonicalize(&path).ok().filter(|real| real.is_file()) else { return false };
        let real = real.to_string_lossy().into_owned();
        if !viewer::opens_on_click(&real) {
            return false;
        }
        let open = Action::OpenFile { path: real, axis: Axis::Right, beside: Some(id), reuse: true, at: lines };
        self.dispatch(open, self.input, window, cx);
        cx.notify();
        true
    }

    /// The core moved a view (a key): there it goes.
    pub(crate) fn viewer_scroll(&mut self, pane: PaneId, top: u32, cx: &mut Context<Self>) {
        if let Some(view) = self.viewers.get_mut(&pane) {
            let handle = view.handle();
            let max = f32::from(handle.max_offset().height);
            handle.set_offset(point(px(0.), -px((top as f32 * LINE).min(max.max(0.)))));
            view.told = None;
            cx.notify();
        }
    }

    /// A `#heading` or footnote link clicked in viewer `pane`: its block to the top, as a wheel
    /// would scroll it; the next frame tells the core where the view is.
    pub(crate) fn viewer_anchor(&mut self, pane: PaneId, anchor: &str, cx: &mut Context<Self>) {
        let Some(view) = self.viewers.get(&pane) else { return };
        let Body::Markdown(blocks) = &view.body else { return };
        if let Some(at) = markdown::anchor_at(blocks, anchor) {
            view.scroll.scroll_to_top_of_item(at);
            cx.notify();
        }
    }

    /// A key while the focused viewer has the keyboard. True: it was the viewer's. Esc with
    /// nothing typed and no lines marked is left for the toast, a shortcut for its binding.
    pub(crate) fn viewer_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(key) = key_name(event) else { return false };
        // Not the viewer's (a shortcut such as ⌃Tab): it goes on to its binding, and drops a count.
        let takes = self.model.viewer_takes(&key);
        if !takes && self.model.viewer_pending().is_none() {
            return false;
        }
        self.dispatch(Action::ViewerKey { key }, self.input, window, cx);
        cx.notify();
        takes
    }

    /// A viewer pane's body: what the file is, else why it is not shown.
    pub(crate) fn viewer_body(&self, id: PaneId, path: &str, cx: &mut Context<Self>) -> AnyElement {
        let read = self.model.viewer(id).map(|view| view.read.clone()).unwrap_or_default();
        let said = |text: String| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(text)
                .child(div().text_xs().child(path.to_string()))
                .into_any_element()
        };
        let view = self.viewers.get(&id);
        let shown = match read {
            Read::Reading => return said("Reading…".into()),
            Read::Missing => return said("File not found".into()),
            Read::TooLarge { bytes } => {
                let most = if self.model.viewer(id).is_some_and(|v| v.shows == Shows::Image) {
                    viewer::MAX_IMAGE_BYTES
                } else {
                    MAX_BYTES
                };
                let size = ottospace_core::calendar::fetch::size;
                return said(format!("Too large: {}; the viewer reads up to {}", size(bytes), size(most)));
            }
            Read::NotFile => return said("Not a regular file".into()),
            Read::Binary => return said("Not text: the viewer shows text, markdown and pictures".into()),
            Read::Unreadable => return said("Could not be read".into()),
            Read::TooManyPixels { width, height, frames } => {
                let of = if frames > 1 { format!(" × {frames} frames or more") } else { String::new() };
                let most = viewer::MAX_PIXELS / 1_000_000;
                return said(format!(
                    "Too large: {width} × {height} pixels{of}; the viewer decodes up to {most} million"
                ));
            }
            Read::Broken => return said("This picture could not be read".into()),
            Read::NoChange => return said("No line changed since the last commit".into()),
            Read::Shown { .. } | Read::Diff { .. } => view.map(|view| &view.body),
            Read::Picture { .. } => match view {
                Some(view) => return self.picture_body(id, view, cx),
                None => return said("Reading…".into()),
            },
        };
        let (_, dir) = name_and_dir(path);
        let body = match shown {
            Some(Body::Markdown(blocks)) => {
                let of = Links::File { pane: id, dir };
                let at = format!("viewer:{id}");
                let blocks = blocks.clone();
                // Each block a child of the scrolled list: a `#link` scrolls to its block.
                let drawn = blocks
                    .iter()
                    .enumerate()
                    .map(|(n, block)| div().max_w(px(796.)).child(self.block(&of, &at, n, block, cx)));
                div()
                    .id(ElementId::Name(format!("viewer-md:{id}").into()))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&view.map(|view| view.scroll.clone()).unwrap_or_default())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .text_sm()
                    .children(drawn)
                    .into_any_element()
            }
            Some(Body::Lines(lines, marks, colours)) => {
                let (lines, marks, colours) = (lines.clone(), marks.clone(), colours.clone());
                let digits = lines.len().max(1).to_string().len();
                // The marked lines, by place in `lines`; a test run notes where the first is drawn.
                let marked = self.model.viewer(id).and_then(|view| view.marked);
                let marked = marked.map(|at| at.first.saturating_sub(1) as usize..at.last as usize);
                let noted = std::env::var_os("OTTOSPACE_E2E").map(|_| self.marks.clone());
                if let Some(noted) = &noted {
                    noted.borrow_mut().remove(&crate::Mark::ViewerMarked(id));
                }
                let list = uniform_list(
                    ElementId::Name(format!("viewer-lines:{id}").into()),
                    lines.len(),
                    move |range, _, _| {
                        range
                            .map(|at| {
                                let line = &lines[at];
                                let line: String = line.chars().take(MAX_DRAWN).collect();
                                // Runs past what is drawn of a long line are cut with it.
                                let runs = match &colours {
                                    Some(colours) => colours.line(at),
                                    None => marks.get(at).map_or(Vec::new(), |marks| highlight::within(marks, &line)),
                                };
                                let styles: Vec<_> = runs
                                    .into_iter()
                                    .map(|(range, token)| {
                                        let colour = rgb(highlight::colour(token)).into();
                                        (range, HighlightStyle { color: Some(colour), ..Default::default() })
                                    })
                                    .collect();
                                let text = StyledText::new(SharedString::from(line)).with_highlights(styles);
                                let on = marked.as_ref().is_some_and(|marked| marked.contains(&at));
                                div()
                                    .h(px(LINE))
                                    // The pane's width at least: a mark's tint spans the row, not its text.
                                    .min_w(gpui::relative(1.))
                                    .relative()
                                    .when(on, |row| {
                                        // A tint and a bar at its left edge, in mark mode's colour.
                                        row.bg(gpui::Hsla::from(rgb(theme::MARK)).opacity(0.14)).child(
                                            div().absolute().left_0().top_0().bottom_0().w(px(2.)).bg(rgb(theme::MARK)),
                                        )
                                    })
                                    .when_some(
                                        noted.as_ref().filter(|_| marked.as_ref().is_some_and(|m| m.start == at)),
                                        |row, noted| row.child(crate::shown_mark(noted, crate::Mark::ViewerMarked(id))),
                                    )
                                    .flex()
                                    .gap_3()
                                    .whitespace_nowrap()
                                    .child(
                                        div()
                                            .flex_none()
                                            .w(px(digits as f32 * 7. + 8.))
                                            .text_right()
                                            .text_color(rgb(theme::MUTED))
                                            .child(SharedString::from((at + 1).to_string())),
                                    )
                                    .child(div().text_color(rgb(theme::TEXT)).child(text))
                            })
                            .collect()
                    },
                )
                .track_scroll(view.map(|view| view.lines.clone()).unwrap_or_default())
                .size_full()
                .px_2()
                .font_family(crate::terminal::font_family())
                .text_xs();
                list.into_any_element()
            }
            Some(Body::Diff(patch, marks)) => {
                let list = diff_rows(id, patch.clone(), marks.clone())
                    .track_scroll(view.map(|view| view.lines.clone()).unwrap_or_default())
                    .size_full()
                    .px_2()
                    .font_family(crate::terminal::font_family())
                    .text_xs();
                list.into_any_element()
            }
            _ => return said("Reading…".into()),
        };
        let handle = view.map(FileView::handle).unwrap_or_default();
        let body = self.bars.tracked(&format!("viewer:{id}"), body, &handle, theme::BACKGROUND).size_full();
        div().absolute().size_full().bg(rgb(theme::BACKGROUND)).child(body).into_any_element()
    }

    /// The viewers for the state: what is drawn of each, never its text. A picture's size, a
    /// markdown file's blocks by kind.
    pub(crate) fn viewers_snapshot(&self) -> serde_json::Value {
        let drawn_panes = self.model.shown_panes();
        let marked = |id: PaneId| {
            std::env::var_os("OTTOSPACE_E2E")?;
            // A viewer off show draws no row: what it last drew is not where it is.
            drawn_panes.contains(&id).then_some(())?;
            self.marks.borrow().get(&crate::Mark::ViewerMarked(id)).copied().map(crate::rect)
        };
        let drawn = |id: PaneId, view: &FileView| match &view.body {
            Body::None => serde_json::json!({ "body": null }),
            Body::Markdown(blocks) => {
                serde_json::json!({ "body": "markdown", "shape": crate::conversation::shape(blocks) })
            }
            // `syntax`: the grammar that coloured it, or null (the lexer, or plain). `marked_rect`, a
            // test run's: where the first marked line is drawn, if it shows.
            Body::Lines(lines, _, colours) => serde_json::json!({
                "body": "lines",
                "lines": lines.len(),
                "syntax": colours.as_ref().map(|colours| colours.lang().grammar().name),
                "marked_rect": marked(id),
            }),
            // A diff's rows and hunks, never a line.
            Body::Diff(patch, _) => serde_json::json!({
                "body": "diff",
                "rows": patch.rows.len(),
                "hunks": patch.hunks().len(),
                "added": patch.added,
                "removed": patch.removed,
            }),
            // `frame`: the frame on show (an animation's), never its pixels.
            Body::Picture(picture) => serde_json::json!({
                "body": "image",
                "frames": picture.frames.len(),
                "frame": view.frame.get(),
            }),
        };
        let all: serde_json::Map<String, serde_json::Value> =
            self.viewers.iter().map(|(id, view)| (id.to_string(), drawn(*id, view))).collect();
        serde_json::Value::Object(all)
    }
}

/// A diff's rows (`review`): a hunk's start says the unchanged lines folded before it; a line
/// its numbers on each side, its sign, and its text coloured, on a tint of added or removed.
fn diff_rows(id: PaneId, patch: Rc<review::Patch>, marks: Rc<Vec<Vec<Mark>>>) -> gpui::UniformList {
    let most = (patch.rows.iter())
        .filter_map(|row| match row {
            review::Row::Line { old, new, .. } => (*old).max(*new),
            review::Row::Hunk { .. } => None,
        })
        .max()
        .unwrap_or(1);
    let width = px(most.to_string().len() as f32 * 7. + 8.);
    // A cut patch ends with a row that says so.
    let count = patch.rows.len() + usize::from(patch.cut);
    uniform_list(ElementId::Name(format!("viewer-diff:{id}").into()), count, move |range, _, _| {
        range
            .map(|at| match patch.rows.get(at) {
                None => div()
                    .h(px(LINE))
                    .px_2()
                    .text_color(rgb(theme::MUTED))
                    .child(format!("Cut at {} rows: open the file in a terminal for the rest", review::MAX_ROWS)),
                Some(row) => match row {
                    review::Row::Hunk { skipped, old, new } => {
                        let said = match skipped {
                            0 => format!("@@ -{old} +{new}"),
                            1 => format!("⋯ 1 unchanged line · @@ -{old} +{new}"),
                            n => format!("⋯ {n} unchanged lines · @@ -{old} +{new}"),
                        };
                        div()
                            .h(px(LINE))
                            .min_w(gpui::relative(1.))
                            .flex()
                            .items_center()
                            .px_2()
                            .bg(rgb(theme::RAISED))
                            .text_color(rgb(theme::MUTED))
                            .whitespace_nowrap()
                            .child(SharedString::from(said))
                    }
                    review::Row::Line { sign, old, new, text } => {
                        let line: String = text.chars().take(MAX_DRAWN).collect();
                        let runs = marks.get(at).map_or(Vec::new(), |marks| highlight::within(marks, &line));
                        let styles: Vec<_> = runs
                            .into_iter()
                            .map(|(range, token)| {
                                let colour = rgb(highlight::colour(token)).into();
                                (range, HighlightStyle { color: Some(colour), ..Default::default() })
                            })
                            .collect();
                        // The drawer's diff cards' marks and colours.
                        let (mark, tint) = match sign {
                            Sign::Added => ("+", Some(theme::PICK)),
                            Sign::Removed => ("−", Some(theme::ERROR)),
                            _ => (" ", None),
                        };
                        let number = |n: &Option<u32>| {
                            div()
                                .flex_none()
                                .w(width)
                                .text_right()
                                .text_color(rgb(theme::MUTED))
                                .child(SharedString::from(n.map(|n| n.to_string()).unwrap_or_default()))
                        };
                        div()
                            .h(px(LINE))
                            .min_w(gpui::relative(1.))
                            .flex()
                            .gap_2()
                            .whitespace_nowrap()
                            .when_some(tint, |row, tint| row.bg(gpui::Hsla::from(rgb(tint)).opacity(0.12)))
                            .child(number(old))
                            .child(number(new))
                            .child(
                                div().flex_none().w(px(8.)).text_color(rgb(tint.unwrap_or(theme::MUTED))).child(mark),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT))
                                    .child(StyledText::new(SharedString::from(line)).with_highlights(styles)),
                            )
                    }
                },
            })
            .collect()
    })
}

/// The foot under a picture: its size and zoom, an animation's control.
const FOOT: f32 = 22.;

/// An animation plays as the user set it, else unless Reduce Motion is on.
fn playing(core: &Viewer) -> bool {
    core.playing.unwrap_or_else(|| !crate::dial::calm())
}

impl Workspace {
    /// The viewers whose animation may move on: on show, and not under the zoomed-out canvas.
    fn animated_panes(&self) -> Vec<PaneId> {
        if self.model.zoom.is_some() { Vec::new() } else { self.model.shown_panes() }
    }

    /// A viewer pane closed or reused: its view goes, its picture's frames with the next tick.
    pub(crate) fn forget_viewer(&mut self, id: PaneId) {
        if let Some(view) = self.viewers.remove(&id) {
            self.retired_pictures.extend(view.frames());
        }
    }

    /// A picture: fitted to the pane (never enlarged) or zoomed, moved by its pan, over a
    /// checker for what is transparent, its size in the foot; an animation's play control.
    fn picture_body(&self, id: PaneId, view: &FileView, cx: &mut Context<Self>) -> AnyElement {
        let (Body::Picture(picture), Some(core)) = (&view.body, self.model.viewer(id)) else {
            return div().into_any_element();
        };
        let area = view.area.get();
        // Above the foot: the picture never goes under it fitted.
        let room = (f32::from(area.size.width), (f32::from(area.size.height) - FOOT).max(0.));
        let fit = viewer_picture::fit(room, picture.width, picture.height);
        let scale = core.zoom.unwrap_or(fit);
        let (w, h) = (picture.width as f32 * scale, picture.height as f32 * scale);
        let (x, y) = ((room.0 - w) / 2. + core.pan[0], (room.1 - h) / 2. + core.pan[1]);
        let frame = picture.frames[view.frame.get().min(picture.frames.len() - 1)].0.clone();
        let laid = view.area.clone();
        // The checker under it: what is transparent shows as such.
        let checker = canvas(
            move |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                // Laid out anew (the first frame, a split, a resize): drawn again at its place.
                if laid.replace(bounds) != bounds {
                    window.refresh();
                }
                let shown = Bounds::new(bounds.origin + point(px(x), px(y)), gpui::size(px(w), px(h)));
                let shown = shown.intersect(&bounds);
                const SQUARE: f32 = 8.;
                let (left, top) = (f32::from(shown.origin.x), f32::from(shown.origin.y));
                let (right, bottom) = (left + f32::from(shown.size.width), top + f32::from(shown.size.height));
                let (ox, oy) = (f32::from(bounds.origin.x) + x, f32::from(bounds.origin.y) + y);
                let mut row = ((top - oy) / SQUARE).floor();
                while oy + row * SQUARE < bottom {
                    let mut col = ((left - ox) / SQUARE).floor();
                    while ox + col * SQUARE < right {
                        if (row + col) as i64 % 2 == 0 {
                            let square = Bounds::new(
                                point(px(ox + col * SQUARE), px(oy + row * SQUARE)),
                                gpui::size(px(SQUARE), px(SQUARE)),
                            )
                            .intersect(&shown);
                            window.paint_quad(gpui::fill(square, rgb(theme::RAISED)));
                        }
                        col += 1.;
                    }
                    row += 1.;
                }
            },
        )
        .absolute()
        .size_full();
        let animated = picture.frames.len() > 1;
        let plays = playing(core);
        let control = format!("viewer-play:{id}");
        let play = animated.then(|| {
            div()
                .id(ElementId::Name(control.clone().into()))
                .relative()
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(if plays { "Pause" } else { "Play" })
                .child(self.controls.mark(control))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.dispatch(Action::ViewerPlay { pane: Some(id), play: !plays }, this.input, window, cx);
                }))
        });
        let of = core.read.clone();
        let bytes = match of {
            Read::Picture { bytes, .. } => ottospace_core::calendar::fetch::size(bytes),
            _ => String::new(),
        };
        let frames = if animated { format!(" · {} frames", picture.frames.len()) } else { String::new() };
        let said = format!("{} × {} px{frames} · {bytes} · {:.0}%", picture.width, picture.height, scale * 100.);
        let foot = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(FOOT))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .bg(rgb(theme::BACKGROUND))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child(crate::cut_text(said))
            .children(play);
        let (zoomed, room_now) = (core.zoom.is_some(), (room.0, room.1, w, h));
        div()
            .id(ElementId::Name(format!("viewer-img:{id}").into()))
            .absolute()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::BACKGROUND))
            .child(checker)
            .child(img(ImageSource::Render(frame)).absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)))
            .child(foot)
            // Zoomed: the wheel and a drag move it, never past where it would leave the pane.
            .when(zoomed, |d| {
                d.on_scroll_wheel(cx.listener(move |this, e: &gpui::ScrollWheelEvent, window, cx| {
                    let delta = e.delta.pixel_delta(px(LINE));
                    let by = [f32::from(delta.x), f32::from(delta.y)];
                    this.picture_pan(id, by, room_now, window, cx);
                }))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, _| {
                        if let Some(view) = this.viewers.get(&id) {
                            view.dragged.set(Some(e.position));
                        }
                    }),
                )
                .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, window, cx| {
                    let Some(from) = this.viewers.get(&id).and_then(|view| view.dragged.get()) else { return };
                    if e.pressed_button != Some(gpui::MouseButton::Left) {
                        return this.viewers.get(&id).iter().for_each(|view| view.dragged.set(None));
                    }
                    let by = [f32::from(e.position.x - from.x), f32::from(e.position.y - from.y)];
                    if let Some(view) = this.viewers.get(&id) {
                        view.dragged.set(Some(e.position));
                    }
                    this.picture_pan(id, by, room_now, window, cx);
                }))
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _: &gpui::MouseUpEvent, _, _| {
                        if let Some(view) = this.viewers.get(&id) {
                            view.dragged.set(None);
                        }
                    }),
                )
            })
            .into_any_element()
    }

    /// Moves a zoomed picture by `by`, kept so a quarter of the pane (or all of the picture, if
    /// smaller) still shows it. `room`: the pane's width and height, then the picture's drawn.
    fn picture_pan(
        &mut self,
        id: PaneId,
        by: [f32; 2],
        room: (f32, f32, f32, f32),
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pan) = self.model.viewer(id).map(|view| view.pan) else { return };
        let (rw, rh, w, h) = room;
        let most = |side: f32, drawn: f32| ((side + drawn) / 2. - (side.min(drawn) / 4.)).max(0.);
        let to = [(pan[0] + by[0]).clamp(-most(rw, w), most(rw, w)), (pan[1] + by[1]).clamp(-most(rh, h), most(rh, h))];
        let by = [to[0] - pan[0], to[1] - pan[1]];
        if by != [0., 0.] {
            self.dispatch(Action::ViewerPan { pane: id, by }, self.input, window, cx);
            cx.notify();
        }
    }

    /// The fit of the picture in viewer `id` as last laid out, in points a pixel.
    pub(crate) fn picture_fit(&self, id: PaneId) -> Option<f32> {
        let view = self.viewers.get(&id)?;
        let Body::Picture(picture) = &view.body else { return None };
        let area = view.area.get();
        let room = (f32::from(area.size.width), (f32::from(area.size.height) - FOOT).max(0.));
        Some(viewer_picture::fit(room, picture.width, picture.height))
    }

    /// The viewer whose picture is under `at`, in the main window (the one pinches are read in).
    pub(crate) fn picture_at(&self, at: Point<Pixels>) -> Option<PaneId> {
        let main = self.model.tree_at(self.model.main_at());
        let shown = main.map(|tree| tree.leaves()).unwrap_or_default();
        self.viewers.iter().find_map(|(id, view)| {
            let over = matches!(view.body, Body::Picture(_)) && view.area.get().contains(&at);
            (over && shown.contains(id)).then_some(*id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir() -> Dir {
        let dir = std::env::temp_dir().join(format!("ottospace-viewer-{}-{:?}", std::process::id(), Instant::now()));
        fs::create_dir_all(&dir).expect("a folder");
        Dir(dir)
    }

    #[test]
    fn only_a_regular_file_within_the_cap_is_read() {
        let dir = dir();
        let text = dir.0.join("a.md");
        fs::write(&text, "# A\n\nb\n").expect("written");
        assert!(matches!(read_file(&text, Shows::Markdown), (Read::Shown { lines: 3 }, Some(Loaded::Text(_)))));
        assert_eq!(read_file(&dir.0.join("gone.md"), Shows::Markdown).0, Read::Missing);
        assert_eq!(read_file(&dir.0, Shows::Text).0, Read::NotFile);
        assert_eq!(read_file(Path::new("/dev/zero"), Shows::Text).0, Read::NotFile);
        let fifo = dir.0.join("pipe");
        let made = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).expect("a path");
        assert_eq!(unsafe { libc::mkfifo(made.as_ptr(), 0o600) }, 0);
        assert_eq!(read_file(&fifo, Shows::Text).0, Read::NotFile, "a pipe is never waited on");
        let big = dir.0.join("big.txt");
        fs::write(&big, vec![b'a'; MAX_BYTES as usize + 1]).expect("written");
        assert_eq!(read_file(&big, Shows::Text).0, Read::TooLarge { bytes: MAX_BYTES + 1 });
        let binary = dir.0.join("x.txt");
        fs::write(&binary, b"a\0b").expect("written");
        assert_eq!(read_file(&binary, Shows::Text).0, Read::Binary);
        let not_png = dir.0.join("x.png");
        fs::write(&not_png, b"text").expect("written");
        assert_eq!(read_file(&not_png, Shows::Image).0, Read::Binary);
        // Through a link to a regular file: read.
        let link = dir.0.join("link.md");
        std::os::unix::fs::symlink(&text, &link).expect("a link");
        assert!(matches!(read_file(&link, Shows::Markdown).0, Read::Shown { .. }));
    }

    #[test]
    fn lines_are_counted_as_an_editor_numbers_them() {
        assert_eq!(
            (line_count(""), line_count("a"), line_count("a\n"), line_count("a\nb"), line_count("\n\n")),
            (0, 1, 1, 2, 2)
        );
    }

    #[test]
    fn a_name_and_its_folder() {
        assert_eq!(name_and_dir("/w/app/README.md"), ("README.md".into(), "/w/app".into()));
        assert_eq!(name_and_dir("/x"), ("x".into(), "/".into()));
    }
}
