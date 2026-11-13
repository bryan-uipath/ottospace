//! Marks in the shell: the Marks section, the keys a mark takes, `marks.json`, and what the core
//! asks of this machine (is the directory there, is the app installed, which windows has it).
//! What a mark is and what going to one does are the core's (`ottospace_core::marks`).

use std::hash::{BuildHasher, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

use gpui::{AnyView, ClickEvent, Context, Div, KeyDownEvent, Stateful, Window, div, prelude::*, px, rgb};
use ottospace_core::marks::{self, Field, MarkId, MarkInput, MarkOf, MarkRef, MarkRow, Need, Pick};
use ottospace_core::settings::Installed;
use ottospace_core::tasks::Moved;
use ottospace_core::{Action, Axis, Kind, Origin, PaneId, theme};
use ottospace_mac::{ax, capture, icon};

use crate::{Hosted, Tooltip, Workspace, config, list_button, rect, running_apps, window_managed};

/// An app window the core asked for (`Effect::HostWindow`).
#[derive(Clone)]
pub struct Wanted {
    pub mark: Option<MarkId>,
    pub bundle: Option<String>,
    pub name: String,
    pub title: String,
    pub into: Option<PaneId>,
    pub axis: Axis,
}

/// How long what was found missing on this machine is believed before it is looked for again.
const CHECK_EVERY: Duration = Duration::from_secs(2);

/// What `marks.json` and this machine last said, kept between frames.
#[derive(Default)]
pub struct Kept {
    /// False: the file is left as it is this run (it does not read, or holds a mark that does not).
    pub writing: bool,
    /// The marks as last written or read, to write only what changes.
    stored: String,
    /// The file's text as this run last read or wrote it: another ottospace wrote since if it differs.
    on_disk: Option<String>,
    /// Why the marks could not be written, until a write succeeds.
    pub unsaved: Option<String>,
    /// What is wrong with the file, for as long as the run lasts. Its own line: no other notice hides it.
    pub notice: Option<String>,
    /// A look at this machine is under way, off the UI thread.
    checking: bool,
    /// When the last look ended, and what it looked for.
    checked: Option<(Instant, Vec<Need>)>,
}

/// A name for good for a new mark: 16 hex digits nobody else has.
fn new_uid() -> String {
    format!("{:016x}", std::collections::hash_map::RandomState::new().build_hasher().finish())
}

/// The letter a key stands for, whatever the layout types with it: `key` is the letter the key
/// has under ⌘, as sneak mode reads it. Else what it types.
fn lettered(event: &KeyDownEvent) -> Option<String> {
    let key = &event.keystroke;
    let named = Some(key.key.clone()).filter(|name| name.chars().count() == 1);
    named.or_else(|| key.key_char.clone()).filter(|text| !text.chars().any(char::is_control))
}

/// A small control at the end of a mark's row: dim until the pointer is on the row.
fn row_button(id: (&'static str, usize), group: &str, label: &'static str, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(16.))
        .h(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .text_xs()
        .text_color(rgb(theme::BORDER_STRONG))
        .group_hover(group.to_string(), |s| s.text_color(rgb(theme::MUTED)))
        .hover(move |s| s.bg(rgb(theme::BACKGROUND)).text_color(rgb(hover)))
        .child(label)
}

impl Workspace {
    /// Each terminal's directory now: the core records it when a pane or tab is marked.
    fn cwds(&self) -> Vec<(PaneId, String)> {
        self.terminals.iter().filter_map(|(id, terminal)| Some((*id, terminal.cwd()?))).collect()
    }

    /// Tells the core each hosted window's title as it is now, where it changed since the window
    /// was hosted: a mark records, and looks for, the title of now.
    fn sync_titles(&mut self) {
        let now = |(id, hosted): (&PaneId, &Hosted)| Some((*id, hosted.window.title().filter(|t| !t.is_empty())?));
        let known = |id: PaneId| match self.model.pane(id).map(|pane| &pane.kind) {
            Some(Kind::App { title, .. }) => Some(title.clone()),
            _ => None,
        };
        let titles: Vec<(PaneId, String)> =
            self.hosted.iter().filter_map(now).filter(|(id, title)| known(*id).as_ref() != Some(title)).collect();
        if !titles.is_empty() {
            self.model.update(Action::Titles { titles }.sent_by(Origin::System));
        }
    }

    /// The file is left as it is for the rest of the run, and the status bar says why.
    fn leave_marks_alone(&mut self, why: String) {
        self.kept_marks.writing = false;
        self.kept_marks.notice = Some(format!("marks.json: {why}; marks are not saved this run"));
    }

    /// Reads `marks.json` at start. A file that does not read, or holds a mark that does not, is
    /// left alone: nothing is written this run, and the status bar says why.
    pub(crate) fn restore_marks(&mut self, cx: &mut Context<Self>) {
        self.kept_marks.writing = true;
        let text = match config::read_marks() {
            Ok(Some(text)) => text,
            Ok(None) => return,
            Err(err) => return self.leave_marks_alone(err),
        };
        let loaded = match marks::parse(&text) {
            Ok(loaded) => loaded,
            Err(err) => return self.leave_marks_alone(err),
        };
        if !loaded.skipped.is_empty() {
            self.leave_marks_alone(loaded.skipped.join("; "));
        }
        // Said once: the next write leaves the commands out.
        let commands = loaded.commands.then(|| marks::NO_COMMANDS.to_string());
        self.kept_marks.notice = self.kept_marks.notice.take().or(commands);
        self.model.update(Action::RestoreMarks { marks: loaded.marks, extra: loaded.extra }.sent_by(Origin::System));
        self.kept_marks.stored = marks::write(&self.model.marks.list, &self.model.marks.extra);
        self.kept_marks.on_disk = Some(text);
        self.check_marks(cx);
    }

    /// Writes `marks.json` when the marks changed. If another ottospace on this folder wrote it
    /// since this one last read or wrote it, its marks are taken in first, mark by mark.
    pub(crate) fn store_marks(&mut self) {
        let mut text = marks::write(&self.model.marks.list, &self.model.marks.extra);
        if !self.kept_marks.writing || text == self.kept_marks.stored {
            return;
        }
        let theirs = match config::read_marks() {
            Ok(now) => now.filter(|now| self.kept_marks.on_disk.as_ref() != Some(now)),
            Err(err) => return self.kept_marks.unsaved = Some(format!("marks: could not read marks.json: {err}")),
        };
        if let Some(theirs) = theirs {
            let read = |text: &str| marks::parse(text).ok().filter(|loaded| loaded.skipped.is_empty());
            let Some(theirs) = read(&theirs) else {
                return self.leave_marks_alone("it was changed while ottospace ran, and does not read".into());
            };
            let base = self.kept_marks.on_disk.as_deref().and_then(read).unwrap_or_default();
            let merged = marks::merge(&base.marks, &self.model.marks.list, &theirs.marks);
            self.model.update(Action::RestoreMarks { marks: merged, extra: theirs.extra }.sent_by(Origin::System));
            text = marks::write(&self.model.marks.list, &self.model.marks.extra);
        }
        match config::save_marks(&text) {
            Ok(()) => {
                self.kept_marks.on_disk = config::keeps_marks().then(|| text.clone());
                (self.kept_marks.stored, self.kept_marks.unsaved) = (text, None);
            }
            Err(err) => self.kept_marks.unsaved = Some(format!("marks: could not save marks.json: {err}")),
        }
    }

    /// Looks for what the marks need on this machine, off the UI thread: a directory on a mount
    /// that hangs must not freeze the window. The answer reaches the core as an action. What was
    /// found is believed for `CHECK_EVERY`.
    pub(crate) fn check_marks(&mut self, cx: &mut Context<Self>) {
        let needs = self.model.marks.needs();
        let fresh = |(at, checked): &(Instant, Vec<Need>)| *checked == needs && at.elapsed() < CHECK_EVERY;
        let nothing = needs.is_empty() && self.model.marks.missing().is_empty();
        if self.kept_marks.checking || nothing || self.kept_marks.checked.as_ref().is_some_and(fresh) {
            return;
        }
        self.kept_marks.checking = true;
        // A bare binary has no bundle to look up: it counts as there while it runs.
        let running: Vec<String> = self.apps.iter().map(|app| app.name.clone()).collect();
        let checked = needs.clone();
        cx.spawn(async move |this, cx| {
            let gone = move |need: &Need| match need {
                Need::Dir(path) => !Path::new(path).is_dir(),
                Need::App { bundle, name } => {
                    icon::installed(bundle.as_deref(), name).is_none() && !running.contains(name)
                }
            };
            let look = async move { needs.into_iter().filter(|need| gone(need)).collect::<Vec<Need>>() };
            let missing = cx.background_executor().spawn(look).await;
            let _ = this.update(cx, |this, cx| {
                this.kept_marks.checking = false;
                this.kept_marks.checked = Some((Instant::now(), checked));
                if missing != this.model.marks.missing() {
                    this.model.update(Action::MarksChecked { missing }.sent_by(Origin::System));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The mark a request names by its `name` or its `key`, or why there is none.
    pub(crate) fn mark_ref(&self, name: Option<String>, key: Option<char>) -> Result<MarkRef, String> {
        let (mark, missing) = match (name, key) {
            (Some(name), None) => (MarkRef::Name(name.clone()), format!("no mark named {name:?}")),
            (None, Some(key)) => (MarkRef::Key(key), format!("no mark with the letter {key}")),
            _ => return Err("name the mark by `name` or by `key`: one of them".into()),
        };
        self.model.marks.find(&mark).map(MarkRef::Id).ok_or(missing)
    }

    /// The name of the mark the Marks section offers to put back, while it offers.
    pub(crate) fn mark_offered(&self) -> Option<&str> {
        let offered = self.model.undo_offer.is_some_and(|offer| offer.moved == Moved::MarkRemoved);
        self.model.marks.removed().filter(|_| offered).map(|mark| mark.name.as_str())
    }

    /// The marks as the state lists them: each row, and where it and its buttons were drawn.
    pub(crate) fn marks_json(&self) -> Vec<serde_json::Value> {
        let drawn = |id: String| self.controls.bounds(&id).map(rect);
        let row = |row: MarkRow| {
            let id = row.id;
            let mut value = serde_json::json!(row);
            value["row"] = serde_json::json!(drawn(format!("mark:{id}")));
            value["buttons"] = serde_json::json!({
                "key": drawn(format!("mark-key:{id}")),
                "rename": drawn(format!("mark-rename:{id}")),
                "remove": drawn(format!("mark-remove:{id}")),
            });
            value
        };
        self.model.mark_rows().into_iter().map(row).collect()
    }

    /// ⌘B, ⌘⇧B and the menu: marks the focused pane or the tab on show, or takes its mark off.
    pub(crate) fn toggle_mark(&mut self, of: MarkOf, window: &Window, cx: &mut Context<Self>) {
        self.error = self.model.mark_refusal(of).filter(|_| self.model.mark_of(of).is_none());
        self.sync_titles();
        let (uid, cwds) = (new_uid(), self.cwds());
        self.dispatch(Action::MarkToggle { of, uid, cwds }, self.input, window, cx);
    }

    /// The `mark_add` request.
    pub(crate) fn add_mark(
        &mut self,
        of: MarkOf,
        (name, key): (Option<String>, Option<char>),
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_titles();
        let (uid, cwds) = (new_uid(), self.cwds());
        self.dispatch(Action::MarkAdd { of, name, key, uid, cwds }, self.input, window, cx);
    }

    /// Goes to a mark: a click or ↵ on its row, ⌘⌥1…9, the `mark_go` request. The error is why
    /// nothing happened, or what of it could not be opened; it also shows in the status bar.
    pub(crate) fn go_to_mark(
        &mut self,
        mark: MarkRef,
        axis: Axis,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let id = self.model.marks.find(&mark).ok_or("no such mark")?;
        let action = Action::MarkGo { mark: MarkRef::Id(id), axis };
        // The core would drop it for who sent it, or behind what has the keyboard: say which.
        if let Some(why) = self.model.refusal(&action, self.input).or(self.model.waits(&action)) {
            return Err(why.into());
        }
        self.sync_titles();
        let unavailable = self.mark_unavailable(id);
        self.error = unavailable.clone();
        self.dispatch(action, self.input, window, cx);
        // A window that could not be hosted: `carry_out` left the reason in `error`.
        unavailable.or(self.error.clone()).map_or(Ok(()), Err)
    }

    /// Why nothing of a mark that is not open can be opened, e.g. "notes: its directory is gone: /x".
    fn mark_unavailable(&self, id: MarkId) -> Option<String> {
        let row = self.model.mark_rows().into_iter().find(|row| row.id == id)?;
        row.reason.filter(|_| !row.available).map(|reason| format!("{}: {reason}", row.name))
    }

    /// A click on a mark's row, or its label: goes to it.
    pub(crate) fn choose_mark(&mut self, id: MarkId, axis: Axis, window: &Window, cx: &mut Context<Self>) {
        if let Err(why) = self.go_to_mark(MarkRef::Id(id), axis, window, cx) {
            self.error = Some(why);
        }
    }

    /// A click on a row's pencil or letter, or its label: the row takes the keyboard.
    pub(crate) fn edit_mark(&mut self, id: MarkId, field: Field, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::MarkEdit { mark: MarkRef::Id(id), field }, self.input, window, cx);
    }

    /// A click on a row's ×, or its label.
    pub(crate) fn remove_mark(&mut self, id: MarkId, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::MarkRemove { mark: MarkRef::Id(id) }, self.input, window, cx);
    }

    /// A click on "Mark removed · Undo", or its label.
    pub(crate) fn undo_mark(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::MarkUndo, self.input, window, cx);
    }

    /// While a mark waits it takes every key but ⌘ shortcuts: a letter (to jump to, or to give
    /// the mark), text for its name, ⌫, ↵ and Esc.
    pub(crate) fn mark_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let renaming = matches!(self.model.mark_input, Some(MarkInput::Rename { .. }));
        if key.modifiers.platform && !renaming {
            return; // ⌘ shortcuts still reach their bindings, and end the wait
        }
        // A name is a text field: it edits as the others do, and what types text is left to
        // the input method.
        if renaming && !matches!(key.key.as_str(), "escape" | "enter") {
            if self.field_key(event, window, cx) {
                cx.stop_propagation();
            }
            return;
        }
        cx.stop_propagation();
        // A held key types only into a name: one press, one jump.
        if event.is_held && !renaming {
            return;
        }
        // A letter to jump to or to give is the key's own, whatever the layout types with it.
        let text = lettered(event);
        let action = match key.key.as_str() {
            "escape" => Action::MarkCancel,
            "enter" if !event.is_held => Action::MarkSubmit,
            "enter" => return,
            "backspace" => Action::MarkBackspace,
            _ => match text.filter(|_| !key.modifiers.control) {
                Some(text) => Action::MarkType { text },
                None => return,
            },
        };
        let jumping = self.model.mark_input == Some(MarkInput::Jump);
        if let (true, Action::MarkType { text }) = (jumping, &action) {
            let to = text.chars().next().and_then(|key| self.model.marks.find(&MarkRef::Key(key)));
            self.error = to.and_then(|id| self.mark_unavailable(id));
            self.sync_titles();
        }
        self.dispatch(action, self.input, window, cx);
        // The key that ended the wait is still down: its repeats are for nothing else. A held ↵
        // after a rename must not run what sits at the shell's prompt, nor a held letter type.
        self.sneak_spent = self.model.mark_input.is_none();
    }

    /// Carries out `Effect::HostWindow`: hosts the app's window, or starts the app first.
    pub(crate) fn host_window(
        &mut self,
        wanted: Wanted,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.hosting {
            return Err("window hosting is off in e2e runs".into());
        }
        let app = (wanted.bundle.as_deref(), wanted.name.as_str());
        let same = |running: &capture::App| {
            let (bundle, _) = ax::app_identity(running.pid);
            marks::same_app(app, (bundle.as_deref(), &running.name))
        };
        if let Some(running) = running_apps(self.hosting, self.host_only).into_iter().find(same) {
            return self.host_window_of(running.pid, &wanted, window, cx);
        }
        if self.host_only.is_some() {
            return Err("this test run may host only the app it started".into());
        }
        // Where the system has it: nothing written in `marks.json` is ever run.
        let path = icon::installed(wanted.bundle.as_deref(), &wanted.name);
        let path = path.ok_or_else(|| format!("{} is not installed", wanted.name))?;
        let app = Installed { name: wanted.name.clone(), path: path.to_string_lossy().into_owned() };
        if let Some(pane) = wanted.into {
            let why = format!("{} is starting", wanted.name);
            self.model.update(Action::Unfilled { pane, why }.sent_by(Origin::System));
        }
        self.launch(app, wanted.axis, false, Some(wanted))
    }

    /// Hosts the window of the running app `pid` that the core picks for `wanted`.
    pub(crate) fn host_window_of(
        &mut self,
        pid: i32,
        wanted: &Wanted,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let main = ax::Window::main_id(pid);
        let windows: Vec<ax::Window> = ax::Window::standard_of(pid).into_iter().filter(window_managed).collect();
        let held = |id: u32| self.hosted.iter().find(|(_, hosted)| hosted.window_id == id).map(|(pane, _)| *pane);
        let fact = |window: &ax::Window| {
            let id = window.id()?;
            let title = window.title().unwrap_or_default();
            Some(marks::Window { id, title, main: Some(id) == main, pane: held(id) })
        };
        let facts: Vec<marks::Window> = windows.iter().filter_map(fact).collect();
        let pane = match (marks::pick_window(&wanted.title, &facts), wanted.into) {
            (Pick::Host(id), into) => {
                // The pane to fill was closed while the app started: nothing to do.
                if into.is_some_and(|pane| self.model.pane(pane).is_none_or(|p| p.kind != Kind::Empty)) {
                    return Ok(());
                }
                let found = windows.into_iter().find(|window| window.id() == Some(id));
                let hosted = capture::target(found.ok_or("the window is gone")?).and_then(Hosted::new)?;
                let action = match into {
                    Some(pane) => Action::Fill { pane, kind: hosted.kind() },
                    None => Action::Open { kind: hosted.kind(), axis: wanted.axis, beside: None },
                };
                // No pane: the window is back where it was, and a launch tries again.
                let opened = self.attach(hosted, action, window, cx);
                Some(opened.ok_or_else(|| format!("{}: no pane could be opened for its window", wanted.name))?)
            }
            (Pick::Focus(pane), None) => {
                self.dispatch(Action::Focus { pane }, self.input, window, cx);
                Some(pane)
            }
            (Pick::Focus(pane), Some(_)) => {
                return Err(format!("{}: its window is in pane {pane} already", wanted.name));
            }
            (Pick::None, _) if facts.is_empty() => return Err(format!("{} has no window yet", wanted.name)),
            (Pick::None, _) => return Err(format!("{}: no window titled {:?}", wanted.name, wanted.title)),
        };
        if let (Some(mark), Some(pane)) = (wanted.mark, pane) {
            self.dispatch(Action::MarkBind { mark, pane }, Origin::System, window, cx);
        }
        Ok(())
    }

    /// Why an empty pane is empty, if a mark's tab left it so: drawn in the pane.
    pub(crate) fn unfilled_why(&self, pane: PaneId) -> Option<String> {
        let empty = self.model.marks.unfilled().iter().find(|empty| empty.pane == pane)?;
        Some(format!("{} · ⌘E to pick something else", empty.why))
    }

    /// The Marks section: each mark's kind, name, letter, whether it is open, and its buttons.
    /// A click goes to the mark.
    pub(crate) fn render_marks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.model.mark_input.as_ref();
        let rows = self.model.mark_rows();
        let empty = rows.is_empty().then(|| {
            let hint = "No marks. ⌘B marks the focused pane, ⌘⇧B the tab on show.";
            div().px_2().text_xs().text_color(rgb(theme::MUTED)).child(hint)
        });
        let hint = (!rows.is_empty()).then(|| {
            let hint = "⌘' then its letter · ⌘⌥1…9";
            div().px_2().pt_2().text_xs().text_color(rgb(theme::BORDER_STRONG)).child(hint)
        });
        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            let (id, at) = (row.id, row.id as usize);
            let group = format!("mark-{id}");
            let (tag, color) = match row.kind {
                "terminal" => ("TERM", theme::PRIMARY),
                "app" => ("APP", theme::APP),
                _ => ("TAB", theme::TEXT),
            };
            let renaming = matches!(input, Some(MarkInput::Rename { mark, .. }) if *mark == id);
            let lettering = input == Some(&MarkInput::SetKey { mark: id });
            let name = match renaming {
                // The name being typed: the text field.
                true => {
                    div().flex_1().min_w_0().flex().text_color(rgb(theme::MARK)).child(self.render_field("name", cx))
                }
                false => div().when(!row.available, |d| d.text_color(rgb(theme::MUTED))).child(row.name.clone()),
            };
            let letter = match (lettering, row.key) {
                (true, _) => "?".to_string(),
                (false, Some(key)) => key.to_uppercase().to_string(),
                (false, None) => "·".to_string(),
            };
            let lit = row.key.is_some() || lettering;
            let letter = div()
                .id(("mark-key", at))
                .flex_none()
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if lettering { theme::MARK } else { theme::BORDER }))
                .text_xs()
                .text_color(rgb(if lit { theme::MARK } else { theme::BORDER_STRONG }))
                .hover(|s| s.border_color(rgb(theme::BORDER_STRONG)))
                .child(letter)
                .child(self.controls.mark(format!("mark-key:{id}")))
                // None of the three also goes to the mark, as a click on the row does.
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.edit_mark(id, Field::Key, window, cx);
                }));
            let rename = row_button(("mark-rename", at), &group, "✎", theme::TEXT)
                .child(self.controls.mark(format!("mark-rename:{id}")))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.edit_mark(id, Field::Name, window, cx);
                }));
            let remove = row_button(("mark-remove", at), &group, "×", theme::ERROR)
                .child(self.controls.mark(format!("mark-remove:{id}")))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.remove_mark(id, window, cx);
                }));
            // A dot while its pane or tab is open; the same room when it is not, so rows line up.
            let dot = div().flex_none().size(px(6.)).rounded_full().when(row.open, |d| d.bg(rgb(theme::PICK)));
            let line = div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_none().w(px(34.)).text_xs().text_color(rgb(color)).child(tag))
                .child(name.flex_1().min_w_0().truncate())
                .child(dot)
                .child(letter)
                .child(rename)
                .child(remove);
            // One line: a long path is cut, and the pointer shows all of it.
            let reason = row.reason.map(|reason| {
                let all = reason.clone();
                div()
                    .id(("mark-reason", at))
                    .w_full()
                    .truncate()
                    .text_xs()
                    .text_color(rgb(theme::ERROR))
                    .child(reason)
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(all.clone().into()))))
            });
            list_button(("mark", at), self.model.list == Some(index))
                .group(group)
                .flex_col()
                .items_start()
                .gap_0p5()
                .child(line)
                .children(reason)
                .child(self.controls.mark(format!("mark:{id}")))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                    this.choose_mark(id, axis, window, cx);
                }))
        });
        let rows: Vec<_> = rows.collect();
        // After a mark is taken off, a line at the top offers to put it back, as Tasks does.
        let notice = self.mark_offered().map(|_| {
            self.undo_line("mark-undo", Moved::MarkRemoved.title())
                .on_click(cx.listener(|this, _, window, cx| this.undo_mark(window, cx)))
        });
        let list = div().id("marks").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.list_scroll).children(notice).children(empty).children(rows);
        self.bars.tracked("marks", list.children(hint), &self.list_scroll)
    }
}
