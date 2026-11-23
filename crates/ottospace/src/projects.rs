//! The Projects section in the shell (`projects` flag): `projects_dir` read on a thread of its own
//! while the section is open, and its rows drawn. Read-only: nothing here writes, follows a link,
//! or reads more than the core's limits. What is listed, and in which order, is the core's.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::UNIX_EPOCH;

use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*, px, rgb};
use ottospace_core::features::Feature;
use ottospace_core::projects::{self, MAX_ENTRIES, MAX_NOTE_BYTES, MAX_PROJECTS, Project};
use ottospace_core::{Action, Origin, Section, theme};

use crate::{Mark, Workspace, config, cut_text, mark, now_ms, rect, row_meta, sidebar_row};

/// What one read of the folder found.
pub struct Read {
    dir: PathBuf,
    listed: Vec<Project>,
}

/// The reader thread, and when the folder was last asked for.
pub struct Reader {
    asks: Sender<PathBuf>,
    reads: Receiver<Read>,
    last: Option<u64>,
    open: bool,
    /// The folder the list is of: another one empties it.
    dir: Option<PathBuf>,
}

impl Reader {
    pub fn new() -> Self {
        let (asks, inbox) = channel::<PathBuf>();
        let (done, reads) = channel();
        std::thread::spawn(move || {
            while let Ok(mut dir) = inbox.recv() {
                // Only the latest ask: a slow folder is not read once per tick behind.
                while let Ok(newer) = inbox.try_recv() {
                    dir = newer;
                }
                let listed = read(&dir);
                if done.send(Read { dir, listed }).is_err() {
                    return;
                }
                ottospace_mac::wake::post();
            }
        });
        Self { asks, reads, last: None, open: false, dir: None }
    }
}

impl Workspace {
    /// The settings' folder to the core, and a read of it while the section is open
    /// (`projects::due`).
    pub(crate) fn tick_projects(&mut self, window: &gpui::Window, cx: &mut Context<Self>) {
        let dir = config::projects_dir();
        if dir != self.projects.dir || dir.is_some() != self.model.projects.on {
            let set = Action::SetProjects { on: dir.is_some(), listed: Vec::new() };
            self.dispatch(set, Origin::System, window, cx);
            (self.projects.last, self.projects.dir) = (None, dir.clone());
        }
        let open = self.model.sidebar == Some(Section::Projects);
        let opened = open && !self.projects.open;
        self.projects.open = open;
        let now = now_ms();
        if let Some(dir) = dir.clone().filter(|_| projects::due(open, opened, self.projects.last, now)) {
            self.projects.last = Some(now);
            let _ = self.projects.asks.send(dir);
        }
        let reads: Vec<Read> = self.projects.reads.try_iter().collect();
        // A read of a folder the settings no longer name is dropped.
        for read in reads.into_iter().filter(|read| Some(&read.dir) == dir.as_ref()) {
            self.dispatch(Action::SetProjects { on: true, listed: read.listed }, Origin::System, window, cx);
            cx.notify();
        }
    }

    /// The Projects section: each project's title and how long ago it changed.
    pub(crate) fn render_projects_section(&self) -> impl IntoElement {
        let now = now_ms();
        let ordered = self.model.projects.ordered();
        let empty = ordered.is_empty().then(|| {
            let hint = "No projects yet: each folder or .md file in projects_dir is one.";
            div().px_2().text_xs().text_color(rgb(theme::MUTED)).child(hint)
        });
        let rows = ordered.into_iter().enumerate().map(|(at, project)| {
            sidebar_row(("project", at), self.model.list == Some(at))
                .child(cut_text(project.title.clone()).flex_1())
                .child(row_meta(ottospace_core::tasks::age(project.modified, now)))
                .child(mark(&self.marks, Mark::Project(at)))
        });
        let list = div().id("projects").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.list_scroll).children(empty).children(rows);
        self.bars.tracked("projects", list, &self.list_scroll, theme::RAISED).min_h(px(0.))
    }

    /// The section as the state gives it: no title or path, only what each row is and where it
    /// was drawn (`row`).
    pub(crate) fn projects_bar(&self) -> serde_json::Value {
        let marks = self.marks.borrow();
        let row = |(at, row): (usize, projects::Row)| {
            let mut row = serde_json::json!(row);
            row["row"] = serde_json::json!(marks.get(&Mark::Project(at)).copied().map(rect));
            row
        };
        let on = self.on(Feature::Projects) && self.model.projects.on;
        let rows: Vec<serde_json::Value> =
            self.model.projects.rows().into_iter().enumerate().filter(|_| on).map(row).collect();
        serde_json::json!({ "on": on, "rows": rows })
    }
}

/// The projects in `dir`: each folder (its note `projects::note_of`) and each `.md` file, by
/// name, past hidden names and links. At most `MAX_PROJECTS`, of the first `4 * MAX_PROJECTS`
/// entries looked at. Nothing could be listed: none.
pub fn read(dir: &Path) -> Vec<Project> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let entries = entries.flatten().take(4 * MAX_PROJECTS);
    let projects = entries.filter_map(|entry| project(&entry.path()));
    projects.take(MAX_PROJECTS).collect()
}

/// One entry of the folder as a project, if it is one.
fn project(path: &Path) -> Option<Project> {
    let name = path.file_name()?.to_str()?.to_string();
    let meta = fs::symlink_metadata(path).ok()?;
    if name.starts_with('.') || meta.file_type().is_symlink() {
        return None;
    }
    let (note, modified) = if meta.is_dir() {
        let files = markdown_in(path);
        let at = projects::note_of(&files);
        let note = at.map(|at| path.join(&files[at].0));
        let note_at = at.map_or(0, |at| files[at].1);
        (note, ms(&meta).max(note_at))
    } else if meta.is_file() && projects::md_stem(&name).is_some() {
        (Some(path.to_path_buf()), ms(&meta))
    } else {
        return None;
    };
    let text = note.as_deref().and_then(|note| crate::collect::read_plain(note, MAX_NOTE_BYTES));
    let text = text.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    Some(Project {
        path: path.to_string_lossy().into_owned(),
        title: projects::title_of(text.as_deref(), &name),
        note: note.map(|note| note.to_string_lossy().into_owned()),
        modified,
    })
}

/// A folder's plain `.md` files (no link, no hidden name), by name and when each changed: of
/// its first `MAX_ENTRIES` entries.
fn markdown_in(dir: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let file = |entry: fs::DirEntry| {
        let name = entry.file_name().into_string().ok()?;
        let meta = fs::symlink_metadata(entry.path()).ok()?;
        let plain = meta.is_file() && !name.starts_with('.') && projects::md_stem(&name).is_some();
        plain.then(|| (name, ms(&meta)))
    };
    entries.flatten().take(MAX_ENTRIES).filter_map(file).collect()
}

/// When it last changed, ms since 1970 (0 if unknown).
fn ms(meta: &fs::Metadata) -> u64 {
    let since = meta.modified().ok().and_then(|at| at.duration_since(UNIX_EPOCH).ok());
    since.map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_folder_lists_its_folders_and_markdown_files_past_links_and_hidden_names() {
        let scratch = Scratch::dir(&format!("ottospace-projects-read-{}", std::process::id()));
        let dir: &Path = scratch.as_ref();
        fs::create_dir_all(dir.join("offline-sync")).unwrap();
        fs::write(dir.join("offline-sync/README.md"), "---\nk: v\n---\n# Offline sync\n").unwrap();
        fs::write(dir.join("offline-sync/spike.md"), "# Not the note").unwrap();
        fs::create_dir_all(dir.join("hiring-loop")).unwrap();
        fs::write(dir.join("launch-docs.md"), "notes, no heading").unwrap();
        fs::write(dir.join("mock.png"), "").unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        // A link out of the folder, to a folder and to a note, is never followed.
        let outside = Scratch::dir(&format!("ottospace-projects-outside-{}", std::process::id()));
        fs::write(outside.as_ref().join("secret.md"), "# Elsewhere").unwrap();
        std::os::unix::fs::symlink(outside.as_ref(), dir.join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.as_ref().join("secret.md"), dir.join("hiring-loop/README.md")).unwrap();
        let mut listed = read(dir);
        listed.sort_by(|a, b| a.path.cmp(&b.path));
        let titles: Vec<&str> = listed.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["hiring-loop", "launch-docs", "Offline sync"]);
        assert_eq!(listed[0].note, None);
        assert_eq!(listed[1].note.as_deref(), Some(listed[1].path.as_str()));
        assert!(listed[2].note.as_deref().is_some_and(|note| note.ends_with("offline-sync/README.md")));
        assert!(listed.iter().all(|p| p.modified > 0));
        assert_eq!(read(&dir.join("none")), []);
    }
}
