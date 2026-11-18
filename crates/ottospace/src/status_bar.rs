//! The status bar's segments around the mode and messages: where you are, what's next, who's
//! working. What they say and which shrink first are the core's (`ottospace_core::status_bar`);
//! here the focused folder's branch and changes are read, off the main thread, and drawn.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use gpui::{AnyView, Context, Div, FontWeight, Stateful, TextRun, Window, div, prelude::*, px, rems, rgb, svg};
use ottospace_core::status_bar::{self, Agents, Fit, Meeting, Place, Widths};
use ottospace_core::tasks::Group;
use ottospace_core::{Action, Kind, theme};

use crate::{Tooltip, Workspace, cut_text, now_ms};

/// How long `git status` may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Between segments (`gap_3`), and an icon with its gap.
const GAP: f32 = 12.;
const ICON: f32 = 12. + 6.;

/// A folder's checkout: the branch, and whether it has changes not committed.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkout {
    pub branch: Option<String>,
    pub dirty: bool,
}

/// Reads the focused folder's checkout on a thread of its own: when the folder changes, or the
/// program in front of its shell does (a `git commit` that ended). No timer of its own.
pub struct Reader {
    jobs: Sender<(String, bool)>,
    reads: Receiver<(String, Option<Checkout>)>,
    asked: Option<(String, bool, Option<String>)>,
    held: Option<(String, Option<Checkout>)>,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, inbox) = channel::<(String, bool)>();
        let (done, reads) = channel();
        std::thread::spawn(move || {
            for (dir, changes) in inbox {
                let read = read(Path::new(&dir), changes);
                if done.send((dir, read)).is_err() {
                    return;
                }
                ottospace_mac::wake::post();
            }
        });
        Self { jobs, reads, asked: None, held: None }
    }

    /// Asks for `dir`'s checkout if it, or `program`, changed since the last ask. `changes`: run
    /// `git status` there (`Model::reads_changes`); else the branch alone.
    pub fn ask(&mut self, dir: Option<&str>, changes: bool, program: Option<String>) {
        let key = dir.map(|dir| (dir.to_string(), changes, program));
        if key != self.asked {
            if let Some((dir, changes, _)) = &key {
                let _ = self.jobs.send((dir.clone(), *changes));
            }
            self.asked = key;
        }
    }

    /// The next `ask` reads again: the lists were read again.
    pub fn forget(&mut self) {
        self.asked = None;
    }

    /// Takes the reads that came. True if what is held changed.
    pub fn take(&mut self) -> bool {
        let before = self.held.clone();
        self.held = self.reads.try_iter().last().or(self.held.take());
        self.held != before
    }

    /// `dir`'s checkout, as last read. `None`: not read yet, or no checkout.
    pub fn of(&self, dir: &str) -> Option<&Checkout> {
        self.held.as_ref().filter(|(at, _)| at == dir).and_then(|(_, checkout)| checkout.as_ref())
    }
}

/// `dir`'s checkout: the branch from git's own files (as the Repos section reads it), and with
/// `changes`, `git status` (hardened: `child::git_read`). `None`: in no checkout.
fn read(dir: &Path, changes: bool) -> Option<Checkout> {
    let (top, branch) = crate::repos::checkout_of(dir)?;
    if !changes {
        return Some(Checkout { branch, dirty: false });
    }
    let mut status = crate::child::git_read(&top);
    status.args(["status", "--porcelain", "--untracked-files=normal", "--ignore-submodules"]);
    // One line is enough to know: what follows is not read.
    let said = crate::child::run(status, TIMEOUT, 256).ok();
    Some(Checkout { branch, dirty: said.is_some_and(|said| !said.out.trim().is_empty()) })
}

/// A line's width in points at the bar's size, as drawn.
pub(crate) fn width(window: &Window, text: &str) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let style = window.text_style();
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let size = rems(0.75).to_pixels(window.rem_size());
    f32::from(window.text_system().shape_line(text.to_string().into(), size, &[run], None).width)
}

/// The segments to draw, shortened to fit.
pub struct Segments {
    pub place: Option<Stateful<Div>>,
    pub meeting: Option<Stateful<Div>>,
    pub agents: Stateful<Div>,
    pub fit: Fit,
}

impl Workspace {
    /// Where the focused pane is (`status_bar::place`).
    pub(crate) fn status_place(&self) -> Option<Place> {
        let pane = self.model.status_pane();
        let cwd = pane.filter(|p| p.kind == Kind::Terminal).and_then(|p| self.terminals.get(&p.id)?.cwd());
        status_bar::place(pane, cwd.as_deref())
    }

    /// Asks for the focused folder's checkout, on the titles' cadence; redraws when one came.
    pub(crate) fn read_status_checkout(&mut self, cx: &mut Context<Self>) {
        let folder = match self.status_place() {
            Some(Place::Folder(dir)) => Some(dir),
            _ => None,
        };
        let program = self.model.status_pane().and_then(|p| self.terminals.get(&p.id)?.foreground_command());
        let changes = folder.as_deref().is_some_and(|dir| self.model.reads_changes(dir));
        self.status_git.ask(folder.as_deref(), changes, program);
        if self.status_git.take() {
            cx.notify();
        }
    }

    /// The meeting the bar names now.
    pub(crate) fn status_meeting(&self) -> Option<Meeting> {
        status_bar::meeting(&self.model.calendar, now_ms())
    }

    /// The folder's text, branch and changes: `(text, branch, dirty)`.
    fn folder_parts(&self, dir: &str, name_only: bool) -> (String, Option<String>, bool) {
        let home = std::env::var("HOME").ok();
        let text = status_bar::folder_text(dir, home.as_deref(), name_only);
        let checkout = self.status_git.of(dir);
        (text, checkout.and_then(|c| c.branch.clone()), checkout.is_some_and(|c| c.dirty))
    }

    /// The place as one line, for the state and its width: `~/code/app · main ●`.
    fn place_line(&self, place: &Place, name_only: bool) -> String {
        match place {
            Place::App(name) => name.clone(),
            Place::Folder(dir) => {
                let (text, branch, dirty) = self.folder_parts(dir, name_only);
                let branch = branch.map(|b| format!(" · {b}")).unwrap_or_default();
                format!("{text}{branch}{}", if dirty { " ●" } else { "" })
            }
        }
    }

    /// The meeting's line with its Join: `Team sync now · Join`.
    fn meeting_line(meeting: &Meeting, short: bool) -> String {
        meeting.text(short) + if meeting.join { " · Join" } else { "" }
    }

    /// The segments, shortened to the room the bar leaves them beside `middle` points of its own.
    pub(crate) fn status_segments(&self, middle: f32, window: &Window, cx: &mut Context<Self>) -> Segments {
        let (place, meeting, agents) = (self.status_place(), self.status_meeting(), self.model.agents());
        let icon = |on: bool| if on { ICON } else { 0. };
        let place_width = |name_only: bool| {
            place.as_ref().map_or(0., |p| {
                let folder = matches!(p, Place::Folder(_));
                // The folder's icon, and the branch's when there is one.
                let branch =
                    matches!(p, Place::Folder(dir) if self.status_git.of(dir).is_some_and(|c| c.branch.is_some()));
                width(window, &self.place_line(p, name_only)) + icon(folder) + icon(branch)
            })
        };
        let meeting_width =
            |short: bool| meeting.as_ref().map_or(0., |m| width(window, &Self::meeting_line(m, short)) + ICON);
        let widths = Widths {
            folder: place_width(false),
            folder_name: place_width(true),
            meeting: meeting_width(false),
            meeting_short: meeting_width(true),
            agents: width(window, &agents.text(false)),
            agents_short: width(window, &agents.text(true)),
        };
        // The bar's padding, the gaps between what it holds and the line before the agents.
        let shown = 3 + usize::from(place.is_some()) + usize::from(meeting.is_some());
        let room = f32::from(window.viewport_size().width) - 16. - GAP * (shown - 1) as f32 - 1. - middle;
        let fit = status_bar::fit(room, widths);
        Segments {
            place: place.map(|place| self.place_segment(&place, fit.folder_name, cx)),
            meeting: meeting.map(|meeting| self.meeting_segment(&meeting, fit.meeting_short, cx)),
            agents: self.agents_segment(agents, fit.agents_short, cx),
            fit,
        }
    }

    fn place_segment(&self, place: &Place, name_only: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let segment = div().id("status-place").relative().flex().items_center().gap(px(6.)).min_w_0();
        let segment = match place {
            Place::App(name) => segment.child(cut_text(name.clone())),
            Place::Folder(dir) => {
                let (text, branch, dirty) = self.folder_parts(dir, name_only);
                let tip = |_: &mut Window, cx: &mut gpui::App| {
                    AnyView::from(cx.new(|_| Tooltip("Copy the folder's path".into())))
                };
                segment
                    .cursor_pointer()
                    .child(icon("icons/folder.svg"))
                    .child(cut_text(text))
                    .children(branch.map(|branch| {
                        let part = div().flex_none().flex().items_center().gap(px(6.));
                        part.child(div().text_color(rgb(theme::BORDER_STRONG)).child("·"))
                            .child(icon("icons/repos.svg"))
                            .child(branch)
                    }))
                    .when(dirty, |d| d.child(div().flex_none().text_color(rgb(theme::SNEAK)).child("●")))
                    .tooltip(tip)
                    .on_click(cx.listener(|this, _, window, cx| this.click_status("place", window, cx)))
            }
        };
        segment.child(self.controls.mark("status:place"))
    }

    fn meeting_segment(&self, meeting: &Meeting, short: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let running = meeting.in_min.is_none();
        let join = meeting.join.then(|| {
            div()
                .id("status-join")
                .relative()
                .cursor_pointer()
                .text_color(rgb(theme::TEXT))
                .font_weight(FontWeight::MEDIUM)
                .child("Join")
                .child(self.controls.mark("status:join"))
                .on_click(cx.listener(|this, _, window, cx| this.click_status("join", window, cx)))
        });
        div()
            .id("status-meeting")
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .when(running, |d| d.text_color(rgb(theme::TEXT)).font_weight(FontWeight::MEDIUM))
            .child(icon("icons/calendar.svg"))
            .child(meeting.text(short))
            .when(join.is_some(), |d| d.child(div().text_color(rgb(theme::BORDER_STRONG)).child("·")))
            .children(join)
            .child(self.controls.mark("status:meeting"))
    }

    fn agents_segment(&self, agents: Agents, short: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let parts = agents.parts(short).into_iter().map(|(text, needs)| {
            let color = match needs {
                Some(true) => theme::task(Group::NeedsYou),
                Some(false) => theme::task(Group::Running),
                None => theme::MUTED,
            };
            div().flex_none().text_color(rgb(color)).child(text)
        });
        let tip = format!("{} · ⌘⇧N goes to the next that needs you", agents.text(false));
        let segment = div()
            .id("status-agents")
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(if short { 6. } else { 10. }))
            .children(parts)
            .child(self.controls.mark("status:agents"));
        match agents == Agents::default() {
            true => segment,
            false => segment
                .cursor_pointer()
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into()))))
                .on_click(cx.listener(|this, _, window, cx| this.click_status("agents", window, cx))),
        }
    }

    /// A click on a segment, or its sneak label: the folder's path is copied (the user's alone),
    /// the meeting joined (the user's alone), the next task that needs you shown (as ⌘⇧N).
    pub(crate) fn click_status(&mut self, name: &str, window: &Window, cx: &mut Context<Self>) {
        match name {
            "place" => {
                if let Some(Place::Folder(path)) = self.status_place() {
                    self.dispatch(Action::CopyPath { path }, self.input, window, cx);
                }
            }
            "join" => {
                if let Some(id) = self.status_meeting().filter(|m| m.join).map(|m| m.id) {
                    self.dispatch(Action::CalendarJoin { id }, self.input, window, cx);
                }
            }
            "agents" => self.next_task(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The bar's segments in the state: where each is drawn; the place's and the agents' text.
    /// The meeting's title is never here: only whether it shows, and where.
    pub(crate) fn status_snapshot(&self) -> serde_json::Value {
        let at = |id: &str| self.controls.bounds(id).map(crate::rect);
        let place = self.status_place();
        let fit = self.status_fit.get();
        serde_json::json!({
            "place": place.map(|p| serde_json::json!({ "text": self.place_line(&p, fit.folder_name), "rect": at("status:place") })),
            "meeting": self.status_meeting().map(|_| serde_json::json!({ "rect": at("status:meeting"), "join": at("status:join") })),
            "agents": { "text": self.model.agents().text(fit.agents_short), "rect": at("status:agents") },
            "shrink": fit.steps(),
        })
    }
}

fn icon(path: &'static str) -> gpui::Svg {
    svg().path(path).size(px(12.)).flex_none().text_color(rgb(theme::MUTED))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;
    use std::process::{Command, Stdio};

    /// A repository with a file not committed, whose own config names an fsmonitor that leaves
    /// `marker` when it runs.
    fn hostile(at: &Path, marker: &Path) {
        std::fs::create_dir_all(at).unwrap();
        let git = |args: &[&str]| {
            let mut git = Command::new("git");
            git.arg("-C").arg(at).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
            assert!(git.stdin(Stdio::null()).status().unwrap().success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "develop"]);
        let script = at.join("monitor.sh");
        std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(&["config", "core.fsmonitor", script.to_str().unwrap()]);
    }

    #[test]
    fn a_status_runs_none_of_the_repositorys_programs_and_none_runs_outside_repos() {
        let scratch = Scratch::dir(&format!("ottospace-status-git-{}", std::process::id()));
        let (repo, marker) = (scratch.as_ref().join("repo"), scratch.as_ref().join("ran"));
        hostile(&repo, &marker);
        // A plain status runs it: what the test guards against is there.
        let plain = Command::new("git").arg("-C").arg(&repo).arg("status").stdout(Stdio::null()).status().unwrap();
        assert!(plain.success() && marker.exists(), "git ran no fsmonitor: the test proves nothing");
        std::fs::remove_file(&marker).unwrap();
        // Outside Repos: the branch, from HEAD, and no git at all.
        assert_eq!(read(&repo, false), Some(Checkout { branch: Some("develop".into()), dirty: false }));
        // Inside: the status, hardened. It sees the changes and runs nothing of the repository's.
        assert_eq!(read(&repo, true), Some(Checkout { branch: Some("develop".into()), dirty: true }));
        assert!(!marker.exists(), "the repository's fsmonitor ran");
    }
}
