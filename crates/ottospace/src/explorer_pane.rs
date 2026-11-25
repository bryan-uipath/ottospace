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
    List { pane: PaneId, root: String, dir: String },
    Marks { pane: PaneId, root: String, roots: Vec<String> },
}

enum Read {
    Listed { pane: PaneId, dir: String, entries: Option<Vec<Entry>>, cut: bool },
    Marks { pane: PaneId, marks: Vec<(String, Mark)> },
}

/// What git lists of a checkout (tracked, and new ones it does not ignore), and each folder on the
/// way: a name in neither is ignored. `None`: no git list (not a listed checkout, or cut).
type Kept = Option<(HashSet<String>, HashSet<String>)>;

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
            // Each root's git list, read with its marks (once an explorer opens).
            let mut kept: HashMap<String, Kept> = HashMap::new();
            for job in inbox {
                let read = match job {
                    Job::Marks { pane, root, roots } => {
                        kept.insert(root.clone(), git_list(&root, &roots));
                        Read::Marks { pane, marks: marks(&root, &roots) }
                    }
                    Job::List { pane, root, dir } => {
                        let ignored = kept.get(&root).and_then(Option::as_ref);
                        let listed = list(Path::new(&root), &dir, ignored);
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

/// The real folder `dir` (from the root) of `root`: each part one plain name, no part a link, and
/// what it resolves to inside the root. `None`: it is not one.
fn folder(root: &Path, dir: &str) -> Option<(PathBuf, PathBuf)> {
    let top = root.canonicalize().ok()?;
    let mut at = top.clone();
    for part in dir.split('/').filter(|part| !part.is_empty()) {
        if !listable(part) {
            return None;
        }
        at.push(part);
        // Never through a link: one swapped in is refused, not followed.
        let meta = std::fs::symlink_metadata(&at).ok()?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return None;
        }
    }
    let real = at.canonicalize().ok()?;
    (real.starts_with(&top) && Path::new(dir).components().all(|c| matches!(c, Component::Normal(_))))
        .then_some((top, real))
}

/// Folder `dir` of `root`, at most `MAX_ENTRIES` names (`true`: there were more). A link is
/// listed as one, never followed; a name git ignores says so.
fn list(root: &Path, dir: &str, kept: Option<&(HashSet<String>, HashSet<String>)>) -> Option<(Vec<Entry>, bool)> {
    let (_, real) = folder(root, dir)?;
    let mut entries = Vec::new();
    let mut cut = false;
    for item in std::fs::read_dir(real).ok()? {
        let Ok(item) = item else { continue };
        let Some(name) = item.file_name().to_str().map(str::to_string) else { continue };
        if !listable(&name) {
            continue;
        }
        if entries.len() == MAX_ENTRIES {
            cut = true;
            break;
        }
        // The entry's own type: a link is not followed to learn what it points at.
        let Ok(kind) = item.file_type() else { continue };
        let path = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        let ignored = kept.is_some_and(|(files, folders)| !files.contains(&path) && !folders.contains(&path));
        entries.push(Entry { name, dir: kind.is_dir(), link: kind.is_symlink(), ignored });
    }
    Some((entries, cut))
}

/// What git lists of the checkout at `root`, and its folders; `None` past the cap or not a listed one.
fn git_list(root: &str, roots: &[String]) -> Kept {
    let (files, cut) = crate::city::checkout_files(
        Path::new(root),
        roots,
        crate::goto::MOST_FILES,
        crate::goto::MOST_BYTES,
        false,
        &[],
        &|_| true,
    )?;
    if cut {
        return None;
    }
    let mut folders = HashSet::new();
    for (path, _) in &files {
        let mut at = path.as_str();
        while let Some((parent, _)) = at.rsplit_once('/') {
            if !folders.insert(parent.to_string()) {
                break;
            }
            at = parent;
        }
    }
    Some((files.into_iter().map(|(path, _)| path).collect(), folders))
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
        let _ = self.explorer_reader.jobs.send(Job::List { pane, root: root.into(), dir: dir.into() });
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
            .bg(rgb(theme::BACKGROUND))
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
        let root = dir.0.join("app");
        std::fs::create_dir_all(root.join("src")).expect("folders");
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
