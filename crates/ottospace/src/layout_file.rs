//! `layout.json` in the shell: read and restored at start, written a second after a change and
//! at quit. What it holds and how it is restored are the core's (`ottospace_core::layout_file`).

use std::path::Path;
use std::time::{Duration, Instant};

use gpui::{Context, Window};
use ottospace_core::layout_file::{self, Unread};
use ottospace_core::{Action, Origin};

use crate::{Workspace, config};

/// How long after a change the file is written: a burst of changes makes one write.
const SETTLE: Duration = Duration::from_secs(1);

/// What `layout.json` and this run last said, kept between frames.
#[derive(Default)]
pub struct Kept {
    /// False: the file is not written this run (nothing is kept, a newer build's file, Start Fresh).
    writing: bool,
    /// When the next write is due.
    due: Option<Instant>,
    /// The text last read or written: the same text is not written again.
    stored: String,
    /// What was wrong with the file at start.
    pub notice: Option<String>,
    /// Why it could not be written, until a write succeeds.
    pub unsaved: Option<String>,
}

impl Workspace {
    /// Reads `layout.json` at start and builds what it holds. A file that does not read is set
    /// aside as `layout.json.bad` and the start is empty; a newer build's is left as it is and
    /// not written this run. True if a tab was restored.
    pub(crate) fn restore_layout(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        self.kept_layout.writing = config::keeps_layout();
        let text = match config::read_layout() {
            Ok(Some(text)) => text,
            Ok(None) => return false,
            Err(why) => return self.layout_unread(why),
        };
        let layout = match layout_file::parse(&text) {
            Ok(layout) => layout,
            Err(Unread::Newer(version)) => {
                self.kept_layout.writing = false;
                let why = format!("layout.json is a newer build's (version {version}): left as it is, not written");
                self.kept_layout.notice = Some(why);
                return false;
            }
            Err(Unread::Bad(why)) => return self.layout_unread(why),
        };
        let home = Some(config::user_home()).filter(|home| home.starts_with('/'));
        // Checked by the core to be absolute and one line; looked for once, here.
        let gone: Vec<String> = layout.dirs().into_iter().filter(|dir| !Path::new(dir).is_dir()).collect();
        let restored = !layout.tabs.is_empty();
        let launch = config::restores_apps();
        let action = Action::RestoreLayout { layout: Box::new(layout), gone: gone.clone(), home, launch };
        self.dispatch(action, Origin::System, window, cx);
        if !gone.is_empty() {
            let gone = format!("layout: {} is gone; its terminal starts in your home folder", gone.join(", "));
            self.kept_layout.notice = Some(gone);
        }
        self.kept_layout.stored = text;
        restored
    }

    /// The file does not read: it goes aside, and this run starts empty.
    fn layout_unread(&mut self, why: String) -> bool {
        self.kept_layout.notice = Some(match config::layout_aside(Some("bad")) {
            Ok(()) => format!("layout.json: {why}; set aside as layout.json.bad"),
            Err(err) => {
                self.kept_layout.writing = false;
                format!("layout.json: {why}; it could not be set aside ({err}) and is not written")
            }
        });
        false
    }

    /// After an action: a change to what the file holds is written a moment later.
    pub(crate) fn layout_changed(&mut self) {
        if self.kept_layout.writing && self.model.layout_moved() {
            self.kept_layout.due.get_or_insert_with(|| Instant::now() + SETTLE);
        }
    }

    /// Writes the file when a write is due, or with `quitting` at once: each terminal's directory
    /// and text change with no action, so the quit reads them again.
    pub(crate) fn store_layout(&mut self, quitting: bool) {
        let due = self.kept_layout.due.is_some_and(|at| at <= Instant::now());
        if !self.kept_layout.writing || !(due || quitting) {
            return;
        }
        self.kept_layout.due = None;
        let layout = self.model.layout_file(&self.cwds(), &self.histories(), config::workbook_lines());
        let text = layout_file::write(&layout);
        if text == self.kept_layout.stored {
            return;
        }
        match config::save_layout(&text) {
            Ok(()) => (self.kept_layout.stored, self.kept_layout.unsaved) = (text, None),
            Err(err) => self.kept_layout.unsaved = Some(format!("layout.json: could not save: {err}")),
        }
    }

    /// Start Fresh: the file goes, and none is written for the rest of the run.
    pub(crate) fn forget_layout(&mut self) {
        (self.kept_layout.writing, self.kept_layout.due) = (false, None);
        self.error = Some(match config::layout_aside(None) {
            Ok(()) => "Start Fresh: the next start is empty; the layout is not kept this run".into(),
            Err(err) => format!("layout.json: could not remove it: {err}"),
        });
    }
}
