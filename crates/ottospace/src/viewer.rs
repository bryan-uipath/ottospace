//! Shell half of the file viewer (`ottospace_core::viewer`, `model::viewer`): reads the file a
//! viewer pane names on a thread of its own (a regular file only, at most `MAX_BYTES`), looks for
//! changes twice a second and reads it again once a change has settled, draws it (markdown by
//! the conversation's renderer, lines with their numbers, or a picture), and tells the core what
//! it read and where the view is. Never its text: the state has the path, kind, line and count.

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
    AnyElement, Context, ElementId, HighlightStyle, ImageSource, KeyDownEvent, ScrollHandle, SharedString, StyledText,
    UniformListScrollHandle, Window, div, img, point, prelude::*, px, rgb, uniform_list,
};
use ottospace_core::terminal::CellPoint;
use ottospace_core::viewer::{self, MAX_BYTES, Read, Shows};
use ottospace_core::{Action, Axis, Origin, PaneId, theme};

use crate::Workspace;
use crate::conversation::Links;
use crate::highlight::{self, Mark};
use crate::markdown::{self, Block};

/// One line of code or text, and one step of a markdown view (`Viewer::top`), in points.
pub const LINE: f32 = 18.;
/// The most characters of a line drawn; the file keeps the rest.
const MAX_DRAWN: usize = 2000;
/// How often a viewer's file is looked at, and how long a change must hold before it is read.
const LOOK: Duration = Duration::from_millis(500);
const SETTLE: Duration = Duration::from_millis(200);

/// What a file was when it was read: a change to any of these reads it again.
type Stamp = Option<(SystemTime, u64, u64)>;

/// What was read, ready to draw.
enum Body {
    None,
    Markdown(Rc<Vec<Block>>),
    /// Each line, and its coloured runs (`highlight`), by byte within it.
    Lines(Rc<Vec<String>>, Rc<Vec<Vec<Mark>>>),
    Image(Arc<gpui::Image>),
}

/// What a read thread hands back: the outcome, and the bytes a picture is drawn from or the text.
enum Loaded {
    Text(String),
    /// Markdown parsed, or text split into lines, off the main thread: a large file does not
    /// stall the window.
    Markdown(Vec<Block>),
    Lines(Vec<String>, Vec<Vec<Mark>>),
    Image(gpui::ImageFormat, Vec<u8>),
}

/// A viewer pane's file as the shell holds it.
pub struct FileView {
    path: String,
    /// What it was read as: a markdown file opened at a line is read again as its source.
    shows: Shows,
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
}

impl FileView {
    fn new(path: String, shows: Shows) -> Self {
        let mut view = FileView {
            path,
            shows,
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
        };
        view.read(shows);
        view
    }

    fn read(&mut self, shows: Shows) {
        let (send, read) = channel();
        let path = self.path.clone();
        std::thread::spawn(move || {
            // Before the read: a write during it changes the stamp, and the next look reads again.
            let at = stamp(&path);
            let (read, loaded) = match read_file(Path::new(&path), shows) {
                (read, Some(Loaded::Text(text))) if shows == Shows::Markdown => {
                    let dir = name_and_dir(&path).1;
                    (read, Some(Loaded::Markdown(markdown::parse_with(&text, Some(&dir), markdown::FILE))))
                }
                // A code file coloured as a fence of its extension would be (`rs`, `py`).
                (read, Some(Loaded::Text(text))) => {
                    let marks = highlight::by_line(&text, &highlight::marks(extension(&path), &text));
                    (read, Some(Loaded::Lines(text.lines().map(str::to_string).collect(), marks)))
                }
                read => read,
            };
            let _ = send.send((read, loaded, at));
            ottospace_mac::wake::post();
        });
        self.reading = Some(read);
    }

    /// The scroll handle of what is drawn: a list keeps its offset in a plain one, shared.
    fn handle(&self) -> ScrollHandle {
        match self.body {
            Body::Lines(..) => self.lines.0.borrow().base_handle.clone(),
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
    if meta.len() > MAX_BYTES {
        return (Read::TooLarge { bytes: meta.len() }, None);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    if (&file as &File).take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() {
        return (Read::Unreadable, None);
    }
    if bytes.len() as u64 > MAX_BYTES {
        return (Read::TooLarge { bytes: bytes.len() as u64 }, None);
    }
    if shows == Shows::Image {
        let format = match ottospace_agents::Media::sniff(&bytes) {
            Some(ottospace_agents::Media::Png) => gpui::ImageFormat::Png,
            Some(ottospace_agents::Media::Jpeg) => gpui::ImageFormat::Jpeg,
            Some(ottospace_agents::Media::Gif) => gpui::ImageFormat::Gif,
            Some(ottospace_agents::Media::Webp) => gpui::ImageFormat::Webp,
            None => return (Read::Binary, None),
        };
        return (Read::Shown { lines: 0 }, Some(Loaded::Image(format, bytes)));
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return (Read::Binary, None);
    }
    match String::from_utf8(bytes) {
        Ok(text) => (Read::Shown { lines: line_count(&text) }, Some(Loaded::Text(text))),
        Err(_) => (Read::Binary, None),
    }
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
        self.viewers.values().filter_map(due).min()
    }

    /// Each tick: a view for each viewer pane, the files read handed to the core, changes on
    /// disk read again once they settle, and the core told where each view on show is.
    pub(crate) fn sync_viewers(&mut self, window: &Window, cx: &mut Context<Self>) {
        let files: HashMap<PaneId, (String, Shows)> = self
            .model
            .viewers
            .iter()
            .filter_map(|(id, view)| match &self.model.pane(*id)?.kind {
                ottospace_core::Kind::File { path } => Some((*id, (path.clone(), view.shows))),
                _ => None,
            })
            .collect();
        self.viewers
            .retain(|id, view| files.get(id).is_some_and(|(path, shows)| (path, shows) == (&view.path, &view.shows)));
        let mut actions = Vec::new();
        for (id, (path, shows)) in &files {
            let view = self.viewers.entry(*id).or_insert_with(|| FileView::new(path.clone(), *shows));
            if let Some(Ok((read, loaded, at))) = view.reading.as_ref().map(Receiver::try_recv) {
                view.reading = None;
                view.stamp = at;
                let (_, dir) = name_and_dir(path);
                view.body = match loaded {
                    Some(Loaded::Markdown(blocks)) => Body::Markdown(Rc::new(blocks)),
                    Some(Loaded::Lines(lines, marks)) => Body::Lines(Rc::new(lines), Rc::new(marks)),
                    Some(Loaded::Image(format, bytes)) => Body::Image(Arc::new(gpui::Image::from_bytes(format, bytes))),
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
                let path = path.clone();
                std::thread::spawn(move || {
                    let _ = send.send(stamp(&path));
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
    /// nothing typed is left for the notice bar, a shortcut for its binding.
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
                let most = MAX_BYTES / (1024 * 1024);
                return said(format!("Too large: {:.1} MB; the viewer reads up to {most} MB", bytes as f64 / 1e6));
            }
            Read::NotFile => return said("Not a regular file".into()),
            Read::Binary => return said("Not text: the viewer shows text, markdown and pictures".into()),
            Read::Unreadable => return said("Could not be read".into()),
            Read::Shown { .. } => view.map(|view| &view.body),
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
            Some(Body::Lines(lines, marks)) => {
                let (lines, marks) = (lines.clone(), marks.clone());
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
                                let runs = marks.get(at).map_or(Vec::new(), |marks| highlight::within(marks, &line));
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
            Some(Body::Image(image)) => div()
                .id(ElementId::Name(format!("viewer-img:{id}").into()))
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&view.map(|view| view.scroll.clone()).unwrap_or_default())
                .p_3()
                .child(img(ImageSource::Image(image.clone())).max_w_full())
                .into_any_element(),
            _ => return said("Reading…".into()),
        };
        let handle = view.map(FileView::handle).unwrap_or_default();
        let body = self.bars.tracked(&format!("viewer:{id}"), body, &handle, theme::BACKGROUND).size_full();
        div().absolute().size_full().bg(rgb(theme::BACKGROUND)).child(body).into_any_element()
    }

    /// The viewers for the state: what is drawn of each, never its text. A picture's size, a
    /// markdown file's blocks by kind.
    pub(crate) fn viewers_snapshot(&self) -> serde_json::Value {
        let marked = |id: PaneId| {
            std::env::var_os("OTTOSPACE_E2E")?;
            self.marks.borrow().get(&crate::Mark::ViewerMarked(id)).copied().map(crate::rect)
        };
        let drawn = |id: PaneId, view: &FileView| match &view.body {
            Body::None => serde_json::json!({ "body": null }),
            Body::Markdown(blocks) => {
                serde_json::json!({ "body": "markdown", "shape": crate::conversation::shape(blocks) })
            }
            // A test run's: where the first marked line is drawn, if it shows.
            Body::Lines(lines, _) => {
                serde_json::json!({ "body": "lines", "lines": lines.len(), "marked_rect": marked(id) })
            }
            Body::Image(_) => serde_json::json!({ "body": "image" }),
        };
        let all: serde_json::Map<String, serde_json::Value> =
            self.viewers.iter().map(|(id, view)| (id.to_string(), drawn(*id, view))).collect();
        serde_json::Value::Object(all)
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
