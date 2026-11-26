//! Shell half of the file explorer (`ottospace_core::explorer`, `model::explorer`): lists one
//! folder at a time on a thread of its own, never through a link nor outside the root, reads
//! which files git ignores (`city::checkout_files`, the one reader) and git's marks (`git status`,
//! hardened, only in a checkout Repos lists), and draws the rows. Never a name in the state
//! outside a test run.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use gpui::{
    AnyElement, Context, ElementId, MouseButton, MouseDownEvent, UniformListScrollHandle, Window, div, prelude::*, px,
    rgb, uniform_list,
};
use ottospace_core::explorer::{Entry, Listing, MAX_ENTRIES, Mark, listable};
use ottospace_core::row_menu::MenuOf;
use ottospace_core::{Action, Origin, PaneId, theme};

use crate::Workspace;

/// How long `git status` may take, and how much of what it says is read.
const STATUS_TIME: Duration = Duration::from_secs(5);
const STATUS_BYTES: u64 = 4 << 20;
/// One row, in points.
const ROW: f32 = 22.;

enum Job {
    List { pane: PaneId, root: String, dir: String, roots: Vec<String> },
    Marks { pane: PaneId, root: String, roots: Vec<String> },
}

enum Read {
    Listed { pane: PaneId, dir: String, entries: Option<Vec<Entry>>, cut: bool },
    Marks { pane: PaneId, marks: Vec<(String, Mark)> },
}

/// What git ignores in a checkout, as it says it (`dir/` for a whole folder). `None`: not read
/// (not a listed checkout, or git said too much).
type Ignored = Option<HashSet<String>>;

/// Reads each folder asked, in turn, on a thread of its own.
pub struct Reader {
    jobs: Sender<Job>,
    reads: Receiver<Read>,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (done, reads) = channel();
        std::thread::spawn(move || {
            // Each root's ignored list: read before its first listing, read again with its marks.
            let mut ignored: HashMap<String, Ignored> = HashMap::new();
            for job in inbox {
                let read = match job {
                    Job::Marks { pane, root, roots } => {
                        ignored.insert(root.clone(), git_ignored(&root, &roots));
                        Read::Marks { pane, marks: marks(&root, &roots) }
                    }
                    Job::List { pane, root, dir, roots } => {
                        let kept = ignored.entry(root.clone()).or_insert_with(|| git_ignored(&root, &roots));
                        // Only a checkout Repos lists: a restored pane or a stale ask reads nothing else.
                        let listed = listed_top(&root, &roots).and_then(|top| list(&top, &dir, kept.as_ref()));
                        let cut = listed.as_ref().is_some_and(|(_, cut)| *cut);
                        Read::Listed { pane, dir, entries: listed.map(|(entries, _)| entries), cut }
                    }
                };
                if done.send(read).is_err() {
                    return;
                }
                ottospace_mac::wake::post();
            }
        });
        Self { jobs, reads }
    }
}

/// `root` resolved, if it is a checkout Repos lists (`status_bar::reads_changes`).
fn listed_top(root: &str, roots: &[String]) -> Option<PathBuf> {
    let top = Path::new(root).canonicalize().ok()?;
    ottospace_core::status_bar::reads_changes(top.to_str()?, roots).then_some(top)
}

/// A folder opened by descriptor, never by its path again: each part from the root opened with
/// `O_NOFOLLOW | O_DIRECTORY` under the one before, so a link swapped in at any moment is refused,
/// never followed, and nothing outside the root is reached.
struct Folder(std::os::fd::OwnedFd);

impl Folder {
    fn open(top: &Path, dir: &str) -> Option<Folder> {
        use std::os::fd::{FromRawFd, OwnedFd};
        use std::os::unix::ffi::OsStrExt;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let top = std::ffi::CString::new(top.as_os_str().as_bytes()).ok()?;
        // SAFETY: a NUL-ended path; the descriptor is owned from here.
        let fd = unsafe { libc::open(top.as_ptr(), flags) };
        let mut at = (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })?;
        if !Path::new(dir).components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        for part in dir.split('/').filter(|part| !part.is_empty()) {
            if !listable(part) {
                return None;
            }
            let name = std::ffi::CString::new(part).ok()?;
            // SAFETY: `at` is an open folder; `name` one plain part, NUL-ended.
            let fd = unsafe { libc::openat(std::os::fd::AsRawFd::as_raw_fd(&at), name.as_ptr(), flags) };
            at = (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })?;
        }
        Some(Folder(at))
    }

    /// Its names and their own types (a link is not followed): at most `most`, `true` if more.
    fn entries(self, most: usize) -> Option<(Vec<(String, bool, bool)>, bool)> {
        use std::os::fd::IntoRawFd;
        let fd = self.0.into_raw_fd();
        // SAFETY: `fd` is an open folder, handed to the stream, which closes it.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            // SAFETY: not taken by the stream: closed here.
            unsafe { libc::close(fd) };
            return None;
        }
        let (mut names, mut cut) = (Vec::new(), false);
        loop {
            // SAFETY: `stream` is open until `closedir` below.
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            // SAFETY: `readdir` gave a valid entry; its name is NUL-ended.
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
            let Ok(name) = name.to_str() else { continue };
            if !listable(name) {
                continue;
            }
            if names.len() == most {
                cut = true;
                break;
            }
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            let c = std::ffi::CString::new(name).ok()?;
            // SAFETY: the stream's own folder; the name inside it; the link itself, not followed.
            let ok = unsafe { libc::fstatat(libc::dirfd(stream), c.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) };
            if ok != 0 {
                continue;
            }
            let kind = stat.st_mode & libc::S_IFMT;
            names.push((name.to_string(), kind == libc::S_IFDIR, kind == libc::S_IFLNK));
        }
        // SAFETY: opened above, closed once.
        unsafe { libc::closedir(stream) };
        Some((names, cut))
    }
}

/// Folder `dir` of the resolved checkout `top`, at most `MAX_ENTRIES` names (`true`: there were
/// more). A link is listed as one, never followed; a name git ignores (or under a folder it
/// ignores) says so.
fn list(top: &Path, dir: &str, ignored: Option<&HashSet<String>>) -> Option<(Vec<Entry>, bool)> {
    let (names, cut) = Folder::open(top, dir)?.entries(MAX_ENTRIES)?;
    let under_ignored = ignored.is_some_and(|ignored| {
        let mut at = String::new();
        dir.split('/').filter(|p| !p.is_empty()).any(|part| {
            at.push_str(part);
            at.push('/');
            ignored.contains(&at)
        })
    });
    let entries = names
        .into_iter()
        .map(|(name, folder, link)| {
            let path = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
            let says = |path: &str| ignored.is_some_and(|ignored| ignored.contains(path));
            let ignored = under_ignored || says(&path) || (folder && says(&format!("{path}/")));
            Entry { name, dir: folder, link, ignored }
        })
        .collect();
    Some((entries, cut))
}

/// What git ignores in the checkout at `root` (`git ls-files --others --ignored --directory`,
/// hardened as the status bar's git), only where Repos lists it.
fn git_ignored(root: &str, roots: &[String]) -> Ignored {
    let top = listed_top(root, roots)?;
    let mut ask = crate::child::git_read(&top);
    ask.args(["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"]);
    let said = crate::child::run(ask, STATUS_TIME, STATUS_BYTES).ok()?;
    if !said.ok || said.out.len() as u64 >= STATUS_BYTES {
        return None;
    }
    Some(said.out.split('\0').filter(|path| !path.is_empty()).map(str::to_string).collect())
}

/// Git's marks of the checkout at `root`: `git status`, hardened (`child::git_read`), only where
/// its top folder is one Repos lists (`status_bar::reads_changes`).
fn marks(root: &str, roots: &[String]) -> Vec<(String, Mark)> {
    let Some(top) = Path::new(root).canonicalize().ok().and_then(|top| top.to_str().map(str::to_string)) else {
        return Vec::new();
    };
    if !ottospace_core::status_bar::reads_changes(&top, roots) {
        return Vec::new();
    }
    let mut status = crate::child::git_read(Path::new(&top));
    status.args(["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=dirty"]);
    let Ok(said) = crate::child::run(status, STATUS_TIME, STATUS_BYTES) else { return Vec::new() };
    parse_status(&said.out)
}

/// `git status --porcelain=v1 -z`: `XY path`, a rename's old path after it.
fn parse_status(out: &str) -> Vec<(String, Mark)> {
    let mut marks = Vec::new();
    let mut parts = out.split('\0');
    while let Some(record) = parts.next() {
        let (Some(code), Some(path)) = (record.get(..2), record.get(3..)) else { continue };
        let mark = match code {
            "??" => Mark::Untracked,
            "DD" | "AA" => Mark::Conflicted,
            code if code.contains('U') => Mark::Conflicted,
            code if code.starts_with('A') => Mark::Added,
            _ => Mark::Modified,
        };
        if code.starts_with('R') || code.starts_with('C') {
            parts.next();
        }
        if path.split('/').all(listable) {
            marks.push((path.trim_end_matches('/').to_string(), mark));
        }
    }
    marks
}

impl Workspace {
    /// `ListDir`: read folder `dir` of explorer `pane`.
    pub(crate) fn list_dir(&self, pane: PaneId, root: &str, dir: &str) {
        let roots = self.model.checkout_roots();
        let _ = self.explorer_reader.jobs.send(Job::List { pane, root: root.into(), dir: dir.into(), roots });
    }

    /// `ExplorerMarks`: git's list and marks of explorer `pane`'s checkout.
    pub(crate) fn read_explorer_marks(&self, pane: PaneId, root: &str) {
        let roots = self.model.checkout_roots();
        let _ = self.explorer_reader.jobs.send(Job::Marks { pane, root: root.into(), roots });
    }

    /// What the reader read since, to the core.
    pub(crate) fn take_explorer_reads(&mut self, window: &Window, cx: &mut Context<Self>) {
        let came: Vec<Read> = self.explorer_reader.reads.try_iter().collect();
        for read in came {
            let action = match read {
                Read::Listed { pane, dir, entries, cut } => Action::ExplorerListed { pane, dir, entries, cut },
                Read::Marks { pane, marks } => Action::ExplorerMarks { pane, marks },
            };
            self.dispatch(action, Origin::System, window, cx);
            cx.notify();
        }
    }

    /// A key while the focused explorer has the keyboard. True: it was the explorer's.
    pub(crate) fn explorer_key(&mut self, event: &gpui::KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let (key, m) = (&event.keystroke, event.keystroke.modifiers);
        if m.platform || m.control || m.alt || m.function {
            return false;
        }
        let name = match key.key.as_str() {
            "up" | "down" | "left" | "right" | "enter" => key.key.clone(),
            _ => match key.key_char.as_deref() {
                Some(typed) if typed.chars().count() == 1 => typed.to_string(),
                _ => return false,
            },
        };
        if !self.model.explorer_takes(&name) {
            return false;
        }
        self.dispatch(Action::ExplorerKey { key: name }, self.input, window, cx);
        cx.notify();
        true
    }

    /// An explorer pane's body: its folder's rows, a header with the root and Show ignored.
    pub(crate) fn explorer_body(&self, id: PaneId, root: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(explorer) = self.model.explorer(id) else { return div().into_any_element() };
        let rows = std::rc::Rc::new(explorer.rows());
        let selected = explorer.selected_at(&rows);
        let shows_ignored = explorer.show_ignored;
        let said = match explorer.listings.get("") {
            Some(Listing::Reading) => Some("Reading…"),
            Some(Listing::Unread) | None => Some("This folder could not be read"),
            Some(Listing::Unlisted) => Some("Not a checkout Repos lists: add it to Repos to see its files"),
            Some(Listing::Read { entries, .. }) if entries.is_empty() => Some("Empty folder"),
            Some(Listing::Read { .. }) => None,
        };
        let cut = matches!(explorer.listings.get(""), Some(Listing::Read { cut: true, .. }));
        let handle: UniformListScrollHandle = self.explorer_scroll.borrow_mut().entry(id).or_default().clone();
        // The selected row in view, once each time it changes.
        if let Some(at) = selected
            && self.explorer_shown.borrow_mut().insert(id, at) != Some(at)
        {
            handle.scroll_to_item(at, gpui::ScrollStrategy::Top);
        }
        let name = root.trim_end_matches('/').rsplit('/').next().unwrap_or(root).to_string();
        let toggle = format!("explorer-ignored:{id}");
        let header = div()
            .flex_none()
            .h(px(ROW))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(crate::cut_text(if cut { format!("{name} · first {MAX_ENTRIES}") } else { name }))
            .child(
                div()
                    .id(ElementId::Name(toggle.clone().into()))
                    .relative()
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                    .child(if shows_ignored { "Hide ignored" } else { "Show ignored" })
                    .child(self.controls.mark(toggle))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(Action::ExplorerIgnored { pane: id }, this.input, window, cx);
                    })),
            );
        let focused = self.model.focused == Some(id);
        let list = uniform_list(
            ElementId::Name(format!("explorer:{id}").into()),
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|at| {
                        let row = &rows[at];
                        let on = selected == Some(at);
                        let path = row.path.clone();
                        let chevron = match (row.dir, row.open) {
                            (true, true) => "▾",
                            (true, false) => "▸",
                            _ => "",
                        };
                        let icon = icon(row.name(), row.dir, row.link);
                        let mark = row.mark.map(|mark| {
                            let (letter, colour) = match mark {
                                Mark::Modified => ("M", theme::SNEAK),
                                Mark::Added => ("A", theme::PICK),
                                Mark::Untracked => ("U", theme::PICK),
                                Mark::Conflicted => ("!", theme::ERROR),
                            };
                            div().flex_none().text_color(rgb(colour)).child(letter)
                        });
                        let control = format!("explorer-row:{id}:{at}");
                        div()
                            .id(ElementId::Name(control.clone().into()))
                            .relative()
                            .h(px(ROW))
                            .pl(px(8. + 14. * row.depth as f32))
                            .pr_2()
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .when(on && focused, |d| d.bg(rgb(theme::SELECTED)))
                            .when(on && !focused, |d| d.bg(rgb(theme::RAISED)))
                            .hover(|s| s.bg(rgb(theme::RAISED)))
                            .text_color(rgb(if row.ignored || row.link { theme::MUTED } else { theme::TEXT }))
                            .child(div().flex_none().w(px(10.)).text_color(rgb(theme::MUTED)).child(chevron))
                            .child(div().flex_none().w(px(14.)).text_color(rgb(theme::MUTED)).child(icon))
                            .child(crate::cut_text(row.name().to_string()).flex_1())
                            .children(row.cut.then(|| {
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child(format!("first {MAX_ENTRIES}"))
                            }))
                            .children(mark)
                            .child(this.controls.mark(control))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.dispatch(Action::ExplorerClick { pane: id, row: at }, this.input, window, cx);
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.open_row_menu_at(MenuOf::Explorer(id, path.clone()), e.position, window, cx);
                                }),
                            )
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(handle)
        .flex_1()
        .text_sm();
        let body = match said {
            Some(said) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(said)
                .into_any_element(),
            None => list.into_any_element(),
        };
        div()
            .absolute()
            .size_full()
            .flex()
            .flex_col()
            // Under 1, the city behind its tab shows through (`city_behind`).
            .when(self.model.pane_opacity(id) == 1., |d| d.bg(rgb(theme::BACKGROUND)))
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The explorers for the state: their shape, never a name. A test run's adds each row's path.
    pub(crate) fn explorers_snapshot(&self) -> serde_json::Value {
        let test = std::env::var_os("OTTOSPACE_E2E").is_some();
        let shape = |explorer: &ottospace_core::explorer::Explorer| {
            let rows = explorer.rows();
            let reading = explorer.listings.values().filter(|l| matches!(l, Listing::Reading)).count();
            serde_json::json!({
                "rows": rows.len(),
                "selected": explorer.selected_at(&rows),
                "open": explorer.open.len(),
                "reading": reading,
                "marked": rows.iter().filter(|row| row.mark.is_some()).count(),
                "show_ignored": explorer.show_ignored,
                "paths": test.then(|| rows.iter().map(|row| row.path.clone()).collect::<Vec<_>>()),
            })
        };
        let all: serde_json::Map<String, serde_json::Value> = self
            .model
            .panes
            .iter()
            .filter_map(|pane| Some((pane.id.to_string(), shape(self.model.explorer(pane.id)?))))
            .collect();
        serde_json::Value::Object(all)
    }
}

/// A row's glyph by what it is: a folder, a link, else the kind of file its name says.
fn icon(name: &str, dir: bool, link: bool) -> &'static str {
    if link {
        return "↗";
    }
    if dir {
        return "▤";
    }
    match ottospace_core::viewer::shows(&format!("/{name}")) {
        ottospace_core::viewer::Shows::Markdown => "¶",
        ottospace_core::viewer::Shows::Code => "‹›",
        ottospace_core::viewer::Shows::Image => "◩",
        ottospace_core::viewer::Shows::Text => "≡",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_folder_is_listed_inside_the_root_only_never_through_a_link() {
        let dir = Dir(std::env::temp_dir().join(format!("ottospace-explorer-{}", std::process::id())));
        std::fs::create_dir_all(dir.0.join("app/src")).expect("folders");
        let root = dir.0.join("app").canonicalize().expect("the root");
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("a file");
        std::fs::create_dir_all(dir.0.join("outside")).expect("a folder outside");
        std::fs::write(dir.0.join("outside/secret.txt"), "made-up").expect("a file outside");
        std::os::unix::fs::symlink(dir.0.join("outside"), root.join("out")).expect("a link");
        let names = |dir: &str| {
            let (entries, _) = list(&root, dir, None)?;
            let mut names: Vec<(String, bool, bool)> = entries.into_iter().map(|e| (e.name, e.dir, e.link)).collect();
            names.sort();
            Some(names)
        };
        assert_eq!(names(""), Some(vec![("out".into(), false, true), ("src".into(), true, false)]));
        assert_eq!(names("src"), Some(vec![("main.rs".into(), false, false)]));
        // Through the link, up and out, or a hostile name: nothing.
        for bad in ["out", "..", "src/..", "../outside", "src/../../outside", "src/main.rs", ".git"] {
            assert_eq!(names(bad), None, "{bad}");
        }
        // What git ignores, a file or a whole folder (and what is under it): said so; a link not.
        std::fs::create_dir_all(root.join("target/debug")).expect("a folder");
        std::fs::write(root.join("build.log"), "made-up").expect("a file");
        let ignored: HashSet<String> = ["build.log".to_string(), "target/".to_string()].into();
        let said = |dir: &str| {
            let (entries, _) = list(&root, dir, Some(&ignored)).expect("listed");
            let mut said: Vec<(String, bool)> = entries.into_iter().map(|e| (e.name, e.ignored)).collect();
            said.sort();
            said
        };
        assert_eq!(
            said(""),
            [("build.log".into(), true), ("out".into(), false), ("src".into(), false), ("target".into(), true)]
        );
        assert_eq!(said("target"), [("debug".to_string(), true)]);
    }

    #[test]
    fn git_status_says_each_files_mark() {
        let out = " M src/a.rs\0?? new.md\0A  added.rs\0UU both.rs\0R  to.rs\0from.rs\0 M ../escape\0";
        assert_eq!(
            parse_status(out),
            [
                ("src/a.rs".to_string(), Mark::Modified),
                ("new.md".into(), Mark::Untracked),
                ("added.rs".into(), Mark::Added),
                ("both.rs".into(), Mark::Conflicted),
                ("to.rs".into(), Mark::Modified),
            ]
        );
    }
}
