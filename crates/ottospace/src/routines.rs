//! The Routines section: carries the core's routine effects to the ottobot client, answers the
//! `routines` requests and draws the list. What is listed, in what order and in what words is the
//! core's (`ottospace_core::routines`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use gpui::{AnyElement, ClickEvent, Context, Div, Window, div, prelude::*, px, rgb};
use ottospace_core::routines::{Group, Row, Tone, report_allowed};
use ottospace_core::{Action, Axis, Effect, Section, theme};

use crate::ottobot::{self, Event, Poller};
use crate::{Workspace, control, list_button, now_ms};

/// The shell's side of the section: the daemon's socket and the thread that talks to it.
pub(crate) struct Link {
    /// `None`: a test run that named no socket. Nothing connects anywhere.
    socket: Option<PathBuf>,
    poller: Option<Poller>,
    /// What the poller was last told: the section is open.
    watching: bool,
    /// Socket callers waiting for the daemon's answer to their run request, by routine.
    waiting: Vec<(String, Sender<String>)>,
    /// A test run opens no report: it keeps the path `o` would have opened.
    opened: Option<String>,
}

impl Link {
    pub(crate) fn start() -> Self {
        let socket = ottobot::socket_path();
        let poller = socket.clone().map(Poller::start);
        Self { socket, poller, watching: false, waiting: Vec::new(), opened: None }
    }
}

/// What the section's list holds, top to bottom. Rows are numbered as list mode numbers them.
enum Item {
    Line(String, u32),
    Heading(String, u32),
    Row(usize, Row),
}

/// A path as one word for a POSIX shell: `/a/it's` → `'/a/it'\''s'`. `None` if it has a
/// character that would act on the terminal (a newline would send the line early).
fn quote(path: &Path) -> Option<String> {
    let text = path.to_str().filter(|text| !text.chars().any(char::is_control))?;
    Some(format!("'{}'", text.replace('\'', "'\\''")))
}

impl Workspace {
    /// Each tick: tells the poll thread whether the section is open, and hands what it read to
    /// the core.
    pub(crate) fn sync_routines(&mut self, window: &Window, cx: &mut Context<Self>) {
        let open = self.model.sidebar == Some(Section::Routines);
        let Some(poller) = &self.routines.poller else { return };
        if open != self.routines.watching {
            poller.watch(open);
        }
        let events: Vec<Event> = poller.events.try_iter().collect();
        self.routines.watching = open;
        for event in events {
            match event {
                Event::Snapshot(routines) => {
                    self.dispatch(Action::RoutinesUpdated { routines, now: now_ms() }, window, cx);
                }
                Event::Ran { name, error } => {
                    self.dispatch(Action::RoutineRunResult { name: name.clone(), error: error.clone() }, window, cx);
                    // The socket caller that asked for this run, if one did.
                    if let Some(at) = self.routines.waiting.iter().position(|(asked, _)| *asked == name) {
                        let (_, reply) = self.routines.waiting.remove(at);
                        let _ = reply.send(error.map_or_else(|| self.routines_reply(), |why| control::error(&why)));
                    }
                }
            }
        }
    }

    /// Carries out the core's routine effects.
    pub(crate) fn routine_effect(&mut self, effect: &Effect) {
        match effect {
            Effect::RoutineRun { name } => {
                if let Some(poller) = &self.routines.poller {
                    poller.run(name.clone());
                }
            }
            // Typed into the new pane's shell, so the output stays when the run ends. The core
            // let only a plain run id through; the two paths are quoted.
            Effect::RoutineFollow { pane, run } => {
                let exe = std::env::current_exe().ok();
                let words = (self.routines.socket.as_deref().and_then(quote), exe.as_deref().and_then(quote));
                match (self.terminals.get(pane), words) {
                    (Some(terminal), (Some(socket), Some(exe))) => {
                        terminal.write(format!("OTTOBOT_SOCKET={socket} {exe} routine-tail {run}\r").into_bytes());
                    }
                    _ => self.error = Some("routines: could not start routine-tail in the pane".into()),
                }
            }
            // The path came from a socket reply, and the file is an agent's own: only a report
            // file under the daemon's store, and never drawn by ottospace itself.
            Effect::OpenReport { path, store_dir } => {
                let real =
                    |path: &str| std::fs::canonicalize(path).ok().and_then(|path| path.to_str().map(String::from));
                let report = real(path).zip(real(store_dir)).filter(|(path, store)| report_allowed(path, store));
                match report.map(|(path, _)| path).filter(|path| Path::new(path).is_file()) {
                    Some(path) if std::env::var_os("OTTOSPACE_E2E").is_some() => self.routines.opened = Some(path),
                    Some(path) => {
                        let opened = std::process::Command::new("/usr/bin/open").arg(&path).spawn();
                        self.error = opened.err().map(|err| format!("routines: open: {err}"));
                    }
                    None => self.error = Some("routines: not a report under the daemon's store; not opened".into()),
                }
            }
            _ => {}
        }
    }

    /// The `routines` reply: the section as it is listed, not the state.
    pub(crate) fn routines_reply(&self) -> String {
        if let Some(poller) = &self.routines.poller {
            poller.poll(); // so a caller that asks again reads fresh rows, with the section closed too
        }
        let routines = &self.model.routines;
        let list: Vec<serde_json::Value> = routines
            .list
            .iter()
            .map(|routine| {
                serde_json::json!({
                    "name": routine.name,
                    "description": routine.description,
                    // The trigger's summary; null if it could not be read.
                    "trigger": routine.trigger.as_ref().ok(),
                    "next_fire": routine.next_fire,
                    "last_fired": routine.last_fired,
                    "active_run": routine.active_run,
                    "note": routine.note,
                })
            })
            .collect();
        serde_json::json!({
            "ok": true,
            "routines": {
                // The daemon's socket as resolved, or null: nothing is connected to.
                "socket": self.routines.socket,
                "daemon": routines.daemon,
                // What the last run, follow or report request came to.
                "note": self.model.routines_note,
                "needs_you": routines.needs_you(self.model.routines_at),
                "rows": self.model.routine_rows(),
                "list": list,
                "quota": routines.quota_lines().into_iter().map(|(line, _)| line).collect::<Vec<_>>(),
                // Test runs: the report `o` would have opened.
                "opened_report": self.routines.opened,
            },
        })
        .to_string()
    }

    /// `routine_run`: replies once the daemon has answered, or at once if the core refuses.
    pub(crate) fn routine_run(&mut self, name: String, reply: Sender<String>, window: &Window, cx: &mut Context<Self>) {
        let effects = self.dispatch(Action::RoutineRun { name: name.clone() }, window, cx);
        if effects.iter().any(|effect| matches!(effect, Effect::RoutineRun { .. })) && self.routines.poller.is_some() {
            self.routines.waiting.push((name, reply));
        } else {
            let _ = reply.send(control::error(self.model.routines_note.as_deref().unwrap_or("not run")));
        }
    }

    fn routine_items(&self) -> Vec<Item> {
        let routines = &self.model.routines;
        let mut items = Vec::new();
        items.extend(routines.daemon_line().map(|line| Item::Line(line, theme::MUTED)));
        items.extend(self.model.routines_note.clone().map(|note| Item::Line(note, theme::PRIMARY)));
        let mut heading = None;
        for (index, row) in self.model.routine_rows().into_iter().enumerate() {
            if heading != Some(row.group) {
                heading = Some(row.group);
                let color = if row.group == Group::Alerts { theme::SNEAK } else { theme::MUTED };
                items.extend(Some(row.group.title()).filter(|t| !t.is_empty()).map(|t| Item::Heading(t.into(), color)));
            }
            items.push(Item::Row(index, row));
        }
        items.extend(routines.hint().map(|hint| Item::Line(hint, theme::MUTED)));
        let quota = routines.quota_lines();
        items.extend((!quota.is_empty()).then(|| Item::Heading("Quota".into(), theme::MUTED)));
        let color = |tone| if tone == Tone::Warning { theme::SNEAK } else { theme::MUTED };
        items.extend(quota.into_iter().map(|(line, tone)| Item::Line(line, color(tone))));
        items
    }

    /// Where row `at` is among the children of the list, to scroll it into view.
    pub(crate) fn routine_child(&self, at: usize) -> Option<usize> {
        self.routine_items().iter().position(|item| matches!(item, Item::Row(index, _) if *index == at))
    }

    /// The count on the rail's icon: alerts and failures. Shown with the section closed too.
    pub(crate) fn routine_badge(&self) -> Option<Div> {
        let count = Some(self.model.routines.needs_you(self.model.routines_at)).filter(|count| *count > 0)?;
        let pill = div().absolute().top(px(3.)).right(px(3.)).h(px(14.)).min_w(px(14.)).px(px(3.)).rounded_full();
        let pill = pill.flex().items_center().justify_center().bg(rgb(theme::SNEAK)).text_color(rgb(theme::BACKGROUND));
        Some(pill.text_size(px(9.)).child(count.to_string()))
    }

    /// Alerts, running runs, failures, routines by next fire, then quota. A click follows the
    /// row's run in a terminal pane: right, or down with shift.
    pub(crate) fn render_routines(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let line = |text: String, color: u32| div().px_2().text_xs().text_color(rgb(color)).child(text);
        let items = self.routine_items().into_iter().map(|item| match item {
            Item::Line(text, color) => line(text, color).into_any_element(),
            Item::Heading(title, color) => line(title, color).pt_2().into_any_element(),
            Item::Row(index, row) => self.render_routine(index, row, cx),
        });
        let list = div().id("routines").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        list.track_scroll(&self.list_scroll).children(items.collect::<Vec<_>>())
    }

    fn render_routine(&self, index: usize, row: Row, cx: &mut Context<Self>) -> AnyElement {
        let tone = match row.tone {
            Tone::Urgent => Some(theme::ERROR),
            Tone::Warning => Some(theme::SNEAK),
            Tone::Running => Some(theme::PRIMARY),
            Tone::Plain => None,
        };
        let top = div()
            .flex()
            .items_center()
            .gap_2()
            .children(tone.map(|color| div().flex_none().size(px(6.)).rounded_full().bg(rgb(color))))
            .child(div().flex_1().min_w_0().truncate().child(row.title))
            .child(div().flex_none().text_xs().text_color(rgb(tone.unwrap_or(theme::MUTED))).child(row.badge));
        let lines =
            row.lines.into_iter().map(|line| div().truncate().text_xs().text_color(rgb(theme::MUTED)).child(line));
        let run = row.run;
        list_button(("routine", index), self.model.list == Some(index))
            .child(div().flex_1().min_w_0().flex().flex_col().child(top).children(lines))
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if let Some(run) = run.clone() {
                    let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                    this.dispatch(Action::RoutineFollow { run, axis }, window, cx);
                }
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_typed_as_one_word_or_not_at_all() {
        assert_eq!(quote(Path::new("/tmp/otto.sock")).as_deref(), Some("'/tmp/otto.sock'"));
        assert_eq!(quote(Path::new("/a b/it's $(x)")).as_deref(), Some(r"'/a b/it'\''s $(x)'"));
        assert_eq!(quote(Path::new("/a\nrm -rf")), None);
    }
}
