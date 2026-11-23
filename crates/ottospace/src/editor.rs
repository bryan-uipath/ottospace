//! Shell half of ⌃G: the draft in a private file (`<home>/drafts/`, 0700; the file 0600, a
//! random name), the user's editor on it in a terminal the drawer hosts, and what it saved read
//! back once it exits. The core decides where the text goes (`model::editor`); this writes,
//! runs, reads and draws. A file is removed when its editor exits, and at start a crashed
//! run's are deleted, not restored.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use gpui::{AnyElement, Context, KeyDownEvent, MouseButton, MouseDownEvent, Window, canvas, div, prelude::*, rgb};
use ottospace_core::model::editor::Target;
use ottospace_core::{Action, Origin, theme};

use crate::terminal::{self, Terminal};
use crate::{Workspace, config};

/// The user's editor on a draft, and its file: removed when this goes.
pub struct Editor {
    pub terminal: Terminal,
    file: PathBuf,
    /// The program it runs, once the shell has exec'd it.
    name: Option<String>,
}

impl Drop for Editor {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
    }
}

/// The most bytes read back: every character a draft may hold, at four bytes each.
const MOST: u64 = 4 * ottospace_core::model::chat::MAX_DRAFT as u64;

/// `<home>/drafts`, made private (0700). `Err`: a test run that named no folder, or a link or
/// a file where the folder should be.
fn drafts_dir() -> io::Result<PathBuf> {
    let home = config::get().home.as_ref().ok_or_else(|| io::Error::other("no ottospace folder in this run"))?;
    let dir = home.join("drafts");
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?,
        Ok(_) => return Err(io::Error::other(format!("{} is not a folder", dir.display()))),
        Err(_) => fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?,
    }
    Ok(dir)
}

/// Writes `text` to a new file of its own in the drafts folder, 0600: `<pid>-<uuid>.md`, so a
/// sweep knows a running ottospace's from a crashed one's, and editors highlight Markdown.
fn write_draft(text: &str) -> io::Result<PathBuf> {
    let id = crate::talk::new_uuid().ok_or_else(|| io::Error::other("no random bytes for a file name"))?;
    let file = drafts_dir()?.join(format!("{}-{id}.md", std::process::id()));
    let mut made = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file)?;
    made.write_all(text.as_bytes())?;
    Ok(file)
}

/// What the editor saved: at most `MOST` bytes, invalid UTF-8 replaced, and whether there was
/// more. Never through a link: an editor that saves by rename leaves a plain file.
fn read_back(file: &Path) -> io::Result<(String, bool)> {
    let opened = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(file)?;
    if !opened.metadata()?.is_file() {
        return Err(io::Error::other("the draft is not a plain file"));
    }
    let mut bytes = Vec::new();
    opened.take(MOST + 1).read_to_end(&mut bytes)?;
    let more = bytes.len() as u64 > MOST;
    bytes.truncate(MOST as usize);
    Ok((String::from_utf8_lossy(&bytes).into_owned(), more))
}

/// At start: a draft whose ottospace is gone (a crash) is deleted, not restored. A running
/// one's is left alone.
pub fn sweep() {
    let Some(dir) = config::get().home.as_ref().map(|home| home.join("drafts")) else { return };
    let Ok(entries) = fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name.to_str().and_then(|name| name.split('-').next()).and_then(|pid| pid.parse::<i32>().ok());
        // SAFETY: a signal 0 only asks whether the process is there.
        let alive = pid.is_some_and(|pid| pid > 0 && unsafe { libc::kill(pid, 0) } == 0);
        if !alive && entry.file_type().is_ok_and(|kind| !kind.is_dir()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The editor's name for the composer: the program running, else the one the variables name.
fn shown_name(name: Option<&str>) -> String {
    let named = || {
        let var = std::env::var("VISUAL").ok().filter(|v| !v.is_empty()).or_else(|| std::env::var("EDITOR").ok())?;
        let first = var.split_whitespace().next()?;
        Path::new(first).file_name().map(|name| name.to_string_lossy().into_owned())
    };
    name.map(str::to_string).or_else(named).unwrap_or_else(|| "your editor".into())
}

impl Workspace {
    /// The core asked for the editor on `text`: the file, then the editor in the drawer's terminal.
    pub(crate) fn open_editor(&mut self, text: &str) -> Result<(), String> {
        let file = write_draft(text).map_err(|err| format!("editor: {err}"))?;
        let started = Terminal::edit(&file.to_string_lossy(), &[]);
        let editor = Editor { terminal: started.map_err(|err| format!("editor: {err}"))?, file, name: None };
        self.drawer.editor = Some(editor);
        Ok(())
    }

    /// Fits the editor to the drawer while it shows, redraws on its output, and once it exits
    /// reads what it saved (only on status 0) and tells the core; its file goes.
    pub(crate) fn sync_editor(&mut self, window: &Window, cx: &mut Context<Self>) {
        let shown = self.model.editor_shown();
        let Some(editor) = &mut self.drawer.editor else { return };
        if shown {
            editor.terminal.fit(self.drawer.body.get().size, self.cell);
        }
        let shells = ["sh", "bash", "zsh", "dash", "fish", "login"];
        if let Some(name) = editor.terminal.program().filter(|name| !shells.contains(&name.as_str())) {
            editor.name = Some(name);
        }
        let dirty = editor.terminal.take_dirty();
        if !editor.terminal.exited() {
            if dirty {
                cx.notify();
            }
            return;
        }
        let read = editor.terminal.exited_ok().then(|| read_back(&editor.file));
        let (saved, more) = match read {
            Some(Ok((text, more))) => (Some(text), more),
            Some(Err(err)) => {
                self.error = Some(format!("editor: {err}"));
                (None, false)
            }
            None => (None, false),
        };
        self.drawer.editor = None;
        self.dispatch(Action::EditorDone { saved, more }, Origin::System, window, cx);
        cx.notify();
    }

    /// A key while the editor has the keyboard: to it, as a terminal pane's keys go to its
    /// program. Only the user's: a client types nothing into the user's editor.
    pub(crate) fn editor_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.input != Origin::User {
            self.error = Some(ottospace_core::model::editor::TYPES_ALONE.into());
            return cx.stop_propagation();
        }
        if let Some(editor) = &self.drawer.editor {
            terminal::send_key(&editor.terminal, event, cx);
        }
    }

    /// ⌘V while the editor has the keyboard: the clipboard's text into it. False: it has not.
    pub(crate) fn editor_paste(&self, text: Option<&str>) -> bool {
        let Some(editor) = self.drawer.editor.as_ref().filter(|_| self.model.editor_keys()) else { return false };
        if let Some(text) = text.filter(|_| self.input == Origin::User) {
            editor.terminal.paste(text);
        }
        true
    }

    /// What the composer says while the editor has its draft: "Editing in nvim…".
    pub(crate) fn editing_words(&self) -> String {
        let name = self.drawer.editor.as_ref().and_then(|editor| editor.name.as_deref());
        format!("Editing in {}…", shown_name(name))
    }

    /// The drawer's body while the editor runs: what it edits, the editor's terminal (a click
    /// gives it the keyboard), then as in the conversation the card that waits (a click gives it
    /// the keyboard) and the composer, which says the editor has its draft.
    pub(crate) fn editor_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let at = self.drawer.body.clone();
        let record = canvas(move |bounds, _, _| at.set(bounds), |_, _, _, _| {}).absolute().size_full();
        let keys = self.model.editor_keys();
        let chat = match self.model.editing.as_ref().map(|editing| &editing.target) {
            Some(Target::Chat(subject)) => self.model.chat(subject).map(|chat| (subject.clone(), chat)),
            _ => None,
        };
        let compose = self.model.editing.as_ref().is_some_and(|editing| editing.target == Target::Compose);
        let what = if compose { "Editing the new task" } else { "Editing message" };
        let note = div()
            .flex_none()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .truncate()
            .child(format!("{what} · save and quit puts it back, unsent"));
        // A card of the conversation edited: shown, and answered only once the user moves to it.
        let card = chat.as_ref().and_then(|(subject, chat)| {
            let card = self.chat_card(subject, chat, cx)?;
            let words = if keys { "Click the request to answer it (a, d)" } else { "⌃G back to the editor" };
            Some(
                div()
                    .id("editor-card")
                    .relative()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .child(self.controls.mark("talk:editor-card".to_string()))
                    .child(card)
                    .child(div().px_3().pb_1().text_xs().text_color(rgb(theme::SNEAK)).child(words))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.editor_click(false, window, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
        });
        let composer = chat.as_ref().map(|(subject, chat)| self.chat_composer(subject, chat, cx));
        let grid = self.drawer.editor.as_ref().map(|editor| {
            let (screen, cell) = (editor.terminal.screen(), self.cell);
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| terminal::paint(&screen, bounds, cell, keys, &[], window, cx),
            )
            .size_full()
        });
        let opening = grid.is_none().then(|| div().p_3().text_sm().text_color(rgb(theme::MUTED)).child("Opening…"));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(note)
            .child(
                div()
                    .id("editor-terminal")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .m_1()
                    .child(record)
                    .children(grid)
                    .children(opening)
                    .child(self.controls.mark("talk:editor-terminal".to_string()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.editor_click(true, window, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
            .children(card)
            .children(composer)
            .into_any_element()
    }

    /// A click on the editor (`on`) or on the card under it: the keyboard goes there, in the drawer.
    pub(crate) fn editor_click(&mut self, on: bool, window: &Window, cx: &mut Context<Self>) {
        let chat = self.model.editing.as_ref().is_some_and(|editing| matches!(editing.target, Target::Chat(_)));
        if chat {
            self.dispatch(Action::Leave, self.input, window, cx);
            self.dispatch(Action::TalkKeys { on: true }, self.input, window, cx);
        }
        self.dispatch(Action::EditorKeys { on }, self.input, window, cx);
        cx.notify();
    }

    /// The drawer while the editor has the new-task box's line: its header, then the editor.
    pub(crate) fn render_compose_editor(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .h(gpui::px(crate::TOP_BAR))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(div().text_xs().text_color(rgb(theme::COMPOSE)).child("NEW TASK · EDITING"));
        div()
            .id("new-task-drawer")
            .flex_none()
            .w(gpui::px(width))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::RAISED))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(header)
            .child(self.editor_body(cx))
            .into_any_element()
    }

    /// What the state reply says of the editor: what it edits and whether it has the keyboard.
    /// Never the text.
    pub(crate) fn editor_snapshot(&self) -> serde_json::Value {
        let Some(editing) = &self.model.editing else { return serde_json::Value::Null };
        let target = match &editing.target {
            Target::Chat(subject) => crate::talk::key(subject),
            Target::Compose => "compose".into(),
        };
        serde_json::json!({
            "target": target,
            "keys": self.model.editor_keys(),
            "shown": self.model.editor_shown(),
            "running": self.drawer.editor.as_ref().is_some_and(|editor| !editor.terminal.exited()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_editor_saved_is_read_whole_up_to_the_limit_and_never_through_a_link() {
        let dir = crate::scratch::Scratch::dir(&format!("ottospace-editor-read-{}", std::process::id()));
        let file = dir.join("draft.md");
        fs::write(&file, b"two\nlines \xff\n").unwrap();
        assert_eq!(read_back(&file).unwrap(), ("two\nlines \u{fffd}\n".to_string(), false));
        fs::write(&file, vec![b'y'; MOST as usize + 3]).unwrap();
        let (text, more) = read_back(&file).unwrap();
        assert_eq!((text.len() as u64, more), (MOST, true));
        let link = dir.join("link.md");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read_back(&link).is_err(), "a link is not followed");
    }

    #[test]
    fn the_editor_is_named_by_what_runs_else_the_variables() {
        assert_eq!(shown_name(Some("nvim")), "nvim");
        assert!(!shown_name(None).is_empty());
    }
}
