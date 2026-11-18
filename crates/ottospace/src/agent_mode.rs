//! Agent mode in the shell (`ottospace_core::agent_mode`): the robot dropdown in an app pane's
//! header and its menu, ⌘⌃V, the palette's commands, and `app_read`, `app_look` and `ax_tree`
//! served to the task whose pass asks. What a level grants, and to whom, is the core's.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use gpui::{AnyElement, AnyView, Context, MouseDownEvent, Window, actions, div, prelude::*, px, rgb, svg};
use ottospace_core::agent_mode::{self, AppUse, Level, menu_key};
use ottospace_core::features::Feature;
use ottospace_core::tasks::{Caller, TaskId};
use ottospace_core::{Action, Origin, PaneId, theme};

use crate::pass::Pass;
use crate::{Tooltip, Workspace, control, dialog, now_ms, rect};

// ⌘⌃V opens the focused app pane's menu; the palette sets a level on it.
actions!(ottospace, [AgentMenuKey, AgentOff, AgentSee, AgentAct]);

/// The menu's width, and an item's height, in points.
const WIDTH: f32 = 250.;
const ITEM: f32 = 30.;
/// The longest side of a look's picture, in pixels.
const LOOK_MOST: usize = 1600;
/// The pictures a pane's folder keeps: the oldest goes when another comes.
const LOOKS_KEPT: u32 = 10;

/// At start: the look folders of a run that is gone (it quit or crashed: `agent-<pid>-…`), or of
/// this one's pid, go. Only folders, never through a link.
pub(crate) fn sweep_looks() {
    let Some(entries) = crate::config::materials_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else { return };
    let me = std::process::id() as i32;
    for path in entries.flatten().map(|entry| entry.path()) {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let pid = name.strip_prefix("agent-").and_then(|rest| rest.split('-').next()).and_then(|pid| pid.parse().ok());
        let gone = pid.is_some_and(|pid: i32| pid == me || ottospace_mac::process::parent(pid).is_none());
        if gone && std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// A look's folder: `<materials>/agent-<pid>-<task>-<word>/`, made for one pane and task, removed
/// when the pane's grant is no longer See or more for that task, and at the next start.
/// Also where the open menu's robot was last painted: a frame's render comes before its paint.
/// Each folder counts its own pictures.
pub(crate) struct Looks {
    folders: HashMap<PaneId, (TaskId, PathBuf, u32)>,
    robot: Option<[f32; 4]>,
}

impl Default for Looks {
    fn default() -> Self {
        Self { folders: HashMap::new(), robot: None }
    }
}

/// The control id of the robot of `pane`, and of an item of the open menu.
fn robot_control(pane: PaneId) -> String {
    format!("agent:{pane}")
}

/// The control id of a restored pane's offer.
fn offer_control(pane: PaneId) -> String {
    format!("agent-offer:{pane}")
}

fn item_control(index: usize) -> String {
    format!("agent-item:{index}")
}

/// What the robot says on hover, at a level, for a task or none.
fn tip(level: Level, task: Option<TaskId>) -> String {
    match (task, level) {
        (None, _) => agent_mode::NO_TASK.into(),
        (Some(task), Level::Off) => format!("Agent mode off: task {task} cannot see this window"),
        (Some(task), Level::See) => format!("Task {task} can see this window"),
        (Some(task), Level::Act) => format!("Task {task} can see this window. {}", agent_mode::ACT_LATER),
    }
}

fn colour(level: Level) -> u32 {
    match level {
        Level::Off => theme::MUTED,
        Level::See => theme::AGENT_SEE,
        Level::Act => theme::AGENT_ACT,
    }
}

impl Workspace {
    /// The robot in an app pane's header: the glyph, the level's word and a caret, in the level's
    /// colour; dimmed where no task works. None with the flag off, and on any other pane.
    pub(crate) fn agent_header(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        // Grants are kept while the flag is off; the robot is not drawn.
        let grant = self.model.agent.panes.get(&pane).copied().filter(|_| self.on(Feature::AgentMode))?;
        let colour = if grant.task.is_some() { colour(grant.level) } else { theme::BORDER_STRONG };
        let open = self.model.agent.menu.is_some_and(|menu| menu.pane == pane);
        let tip = tip(grant.level, grant.task);
        let robot = div()
            .id(("agent", pane as usize))
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .text_color(rgb(colour))
            .when(open, |d| d.bg(rgb(theme::BORDER_STRONG)))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(svg().path("icons/robot.svg").size(px(13.)).text_color(rgb(colour)))
            .child(grant.level.word())
            .child("▾")
            .child(self.controls.mark(robot_control(pane)))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into()))))
            // The press must not reach the header: an unsteady click would start a drag.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.agent_open(Some(pane), false, window, cx)));
        // A level kept from a run before is only offered: the user's click gives it.
        let offer = self.model.agent.offers.get(&pane).map(|kept| {
            let task = self.model.tasks.get(kept.task).map(|task| task.title.lines().next().unwrap_or_default());
            let task: String = task.unwrap_or_default().chars().take(40).collect();
            let text = format!("Restore {} for {task}?", kept.level.label());
            div()
                .id(("agent-offer", pane as usize))
                .flex_none()
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(self::colour(kept.level)))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(text)
                .child(self.controls.mark(offer_control(pane)))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.agent_send(Action::AgentOfferAccept { pane }, window, cx)
                }))
        });
        Some(div().flex().items_center().gap_1().child(robot).children(offer).into_any_element())
    }

    /// The colour of the pane's header line where a task may use it: See's blue, See and act's
    /// amber. None at Off, with no task, or with the flag off.
    pub(crate) fn agent_header_line(&self, pane: PaneId) -> Option<u32> {
        let grant = self.model.agent.panes.get(&pane).filter(|_| self.on(Feature::AgentMode))?;
        (grant.task.is_some() && grant.level != Level::Off).then(|| colour(grant.level))
    }

    /// Opens (or shuts) the menu of `pane`, default the focused one. `keys`: by a key or a label.
    pub(crate) fn agent_open(&mut self, pane: Option<PaneId>, keys: bool, window: &Window, cx: &mut Context<Self>) {
        self.agent_send(Action::AgentMenu { pane, keys }, window, cx);
        let robot = self.model.agent.menu.and_then(|menu| self.controls.bounds(&robot_control(menu.pane)));
        self.looks.robot = robot.map(rect);
    }

    /// The palette's level, on the focused app pane.
    pub(crate) fn agent_level(&mut self, level: Level, window: &Window, cx: &mut Context<Self>) {
        self.agent_send(Action::AgentLevel { pane: None, level }, window, cx);
    }

    /// Sends an agent mode action as whoever is acting; refused, the status bar says why.
    fn agent_send(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) {
        if let Some(why) = self.model.refusal(&action, self.input) {
            self.error = Some(why.into());
            return cx.notify();
        }
        self.dispatch(action, self.input, window, cx);
        cx.notify();
    }

    /// A key of the user's while the menu is open: the menu's. `false`: none is open, or the key
    /// is a client's, which goes where it went before.
    pub(crate) fn agent_menu_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.input != Origin::User || self.model.agent.menu.is_none() {
            return false;
        }
        self.agent_send(menu_key(key), window, cx);
        true
    }

    /// A press of the user's while the menu is open: on an item or the robot, theirs; anywhere
    /// else it shuts the menu and is no press on what is under it (`true`).
    pub(crate) fn agent_menu_press(&mut self, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.model.agent.menu.filter(|_| self.input == Origin::User) else { return false };
        let own = (0..Level::ALL.len()).map(item_control).chain([robot_control(menu.pane)]);
        let inside = |control: String| self.controls.bounds(&control).is_some_and(|b| b.contains(&event.position));
        if own.into_iter().any(inside) {
            return false;
        }
        self.agent_send(Action::AgentMenuClose, window, cx);
        true
    }

    /// A sneak label (or a click) on the robot (`agent:<pane>`), a menu item (`agent-item:<n>`) or
    /// an offer (`agent-offer:<pane>`).
    /// False: not one of these.
    pub(crate) fn click_agent(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        match (kind, name.parse::<usize>()) {
            ("agent", Ok(pane)) => self.agent_open(Some(pane as PaneId), true, window, cx),
            ("agent-item", Ok(index)) => self.agent_send(Action::AgentMenuChoose { index: Some(index) }, window, cx),
            ("agent-offer", Ok(pane)) => self.agent_send(Action::AgentOfferAccept { pane: pane as PaneId }, window, cx),
            _ => return false,
        }
        true
    }

    /// The open menu, under the robot that opened it: Off, See, See and act, the level on show
    /// ticked. Where no task works, it says so and offers nothing.
    pub(crate) fn render_agent_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let menu = self.model.agent.menu?;
        let grant = self.model.agent.panes.get(&menu.pane).copied()?;
        let [x, y, _, h] = self.looks.robot?;
        let view = window.viewport_size();
        let left = (x as f32).min(f32::from(view.width) - WIDTH - 4.).max(4.);
        let list =
            dialog::floating(WIDTH).absolute().left(px(left)).top(px((y + h) as f32 + 2.)).p_1().text_sm().occlude();
        let Some(task) = grant.task else {
            let none = div().h(px(ITEM)).px_2().flex().items_center().text_color(rgb(theme::MUTED));
            return Some(list.child(none.child(agent_mode::NO_TASK)));
        };
        let rows = Level::ALL.into_iter().enumerate().map(|(index, level)| {
            let ticked = level == grant.level;
            let later = (level == Level::Act)
                .then(|| div().text_xs().text_color(rgb(theme::MUTED)).child(agent_mode::ACT_LATER));
            div()
                .id(("agent-item", index))
                .relative()
                .min_h(px(ITEM))
                .px_2()
                .py_1()
                .flex()
                .flex_col()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .when(menu.selected == Some(index), |d| d.bg(rgb(theme::SELECTED)))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().w(px(12.)).text_color(rgb(colour(level))).child(if ticked { "✓" } else { "" }))
                        .child(level.label()),
                )
                .children(later)
                .child(self.controls.mark(item_control(index)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.agent_send(Action::AgentMenuChoose { index: Some(index) }, window, cx);
                }))
        });
        let head =
            div().px_2().py_1().text_xs().text_color(rgb(theme::MUTED)).child(format!("What task {task} may do here"));
        Some(list.child(head).children(rows))
    }

    /// `app_read`, `app_look` and `ax_tree`: answered only to the task the pane is granted to, at
    /// See or more (`Model::app_access`). Each one served or refused for a task is a row in its
    /// conversation. A level gone below See while it is read: the answer is dropped.
    pub(crate) fn serve_app(
        &mut self,
        pane: PaneId,
        what: AppUse,
        pass: Option<Pass>,
        limits: (Option<usize>, Option<usize>),
        reply: Sender<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let at = now_ms();
        let caller = self.caller(pass.as_ref());
        let access = self.model.app_access(pane, caller, what, at);
        if let Caller::Task(task) = caller {
            let refused = access.as_ref().err().cloned();
            self.dispatch(Action::AppUsed { task, pane, what, refused, at }, Origin::System, window, cx);
        }
        let task = match access {
            Ok(task) => task,
            Err(why) => return drop(reply.send(control::error(&why))),
        };
        match what {
            AppUse::Read => self.serve_read(pane, (task, pass), limits, reply, cx),
            AppUse::Look => self.serve_look(pane, (task, pass), reply, cx),
        }
    }

    /// `ax_tree` in an app-window test run (`OTTOSPACE_HOST_ONLY`: it hosts only the canary, and
    /// no user is there to grant a level): `app_read`'s reply, with no task.
    pub(crate) fn serve_canary_tree(
        &mut self,
        pane: PaneId,
        (depth, nodes): (Option<usize>, Option<usize>),
        reply: Sender<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.ax_source(Some(pane)) else {
            return drop(reply.send(control::error("not an app pane")));
        };
        let read = cx.background_executor().spawn(async move { crate::ax_debug::read_hosted(source, depth, nodes) });
        cx.background_executor()
            .spawn(async move {
                let answer = match read.await {
                    Err(why) => control::error(&why),
                    Ok((window, millis, tree)) => {
                        let captured = serde_json::json!({
                            "untrusted": true, "warning": ottospace_core::capture::WARNING,
                            "count": tree.count, "truncated": tree.truncated, "tree": agent_mode::seen(tree.root),
                        });
                        serde_json::json!({ "ok": true, "pane": pane, "task": null, "window": window, "millis": millis, "captured": captured })
                            .to_string()
                    }
                };
                let _ = reply.send(answer);
            })
            .detach();
    }

    /// Still granted to `task` at See or more, the flag on: an answer read before an Off is dropped.
    fn still_sees(&self, pane: PaneId, task: TaskId) -> bool {
        let granted = |grant: &ottospace_core::agent_mode::Grant| grant.task == Some(task) && grant.level >= Level::See;
        self.on(Feature::AgentMode) && self.model.agent.panes.get(&pane).is_some_and(granted)
    }

    /// As `still_sees`, and the asker's pass still `task`'s: an agent stopped while it waits gets nothing.
    fn still_answers(&self, pane: PaneId, (task, pass): (TaskId, Option<&Pass>)) -> bool {
        self.still_sees(pane, task) && self.caller(pass) == Caller::Task(task)
    }

    fn serve_read(
        &mut self,
        pane: PaneId,
        (task, pass): (TaskId, Option<Pass>),
        (depth, nodes): (Option<usize>, Option<usize>),
        reply: Sender<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.ax_source(Some(pane)) else {
            return drop(reply.send(control::error("no window in this pane yet")));
        };
        let read = cx.background_executor().spawn(async move { crate::ax_debug::read_hosted(source, depth, nodes) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let seeing = this.update(cx, |this, _| this.still_answers(pane, (task, pass.as_ref()))).unwrap_or(false);
            let answer = match read {
                _ if !seeing => control::error("agent mode went off on this pane while it was read"),
                Err(why) => control::error(&why),
                Ok((window, millis, tree)) => {
                    let root = agent_mode::seen(tree.root);
                    let captured = serde_json::json!({
                        "untrusted": true, "warning": ottospace_core::capture::WARNING,
                        "count": tree.count, "truncated": tree.truncated, "tree": root,
                    });
                    serde_json::json!({ "ok": true, "pane": pane, "task": task, "window": window, "millis": millis, "captured": captured })
                        .to_string()
                }
            };
            let _ = reply.send(answer);
        })
        .detach();
    }

    /// The pane's last frame as a PNG, at most 1600 px on its long side, written 0600 in the
    /// look folder of this pane and task. The reply gives its path.
    fn serve_look(
        &mut self,
        pane: PaneId,
        (task, pass): (TaskId, Option<Pass>),
        reply: Sender<String>,
        cx: &mut Context<Self>,
    ) {
        // Live or peeked: the stream is stopped, and the frame held is from before.
        if self.hosted.get(&pane).is_some_and(|hosted| hosted.paused.is_some()) {
            return drop(reply.send(control::error("the pane is live: no picture until it is parked")));
        }
        let Some(frame) = self.hosted.get(&pane).and_then(|hosted| hosted.frame()) else {
            return drop(reply.send(control::error("no picture of this pane yet")));
        };
        let Some((dir, count)) = self.look_dir(pane, task) else {
            return drop(reply.send(control::error("no folder to keep the picture in")));
        };
        let name = format!("look-{count}.png");
        // At most `LOOKS_KEPT` a folder: the one that many before goes.
        if let Some(old) = count.checked_sub(LOOKS_KEPT) {
            let _ = std::fs::remove_file(dir.join(format!("look-{old}.png")));
        }
        let frame = crate::Frame(frame);
        let folder = dir.clone();
        let work = cx.background_executor().spawn(async move {
            let png = crate::collect::frame_png(&frame, Some(LOOK_MOST)).ok_or("the picture could not be made")?;
            crate::config::write_private(&dir, &name, &png).map_err(|_| "the picture could not be kept")?;
            Ok::<PathBuf, &str>(dir.join(name))
        });
        cx.spawn(async move |this, cx| {
            let written = work.await;
            // The folder still this pane's for `task`: one swept while it was written (an Off and
            // See again) is no longer tracked, so it goes, picture and all.
            let seeing = this
                .update(cx, |this, _| {
                    let tracked = this.looks.folders.get(&pane).is_some_and(|(of, at, _)| *of == task && *at == folder);
                    if !tracked {
                        let _ = std::fs::remove_dir_all(&folder);
                    }
                    tracked && this.still_answers(pane, (task, pass.as_ref()))
                })
                .unwrap_or(false);
            let answer = match written {
                Ok(path) if seeing => {
                    let path = path.to_string_lossy().into_owned();
                    let captured = serde_json::json!({ "untrusted": true, "warning": ottospace_core::capture::WARNING, "path": path });
                    serde_json::json!({ "ok": true, "pane": pane, "task": task, "captured": captured }).to_string()
                }
                Ok(path) => {
                    let _ = std::fs::remove_file(path);
                    control::error("agent mode went off on this pane while it was looked at")
                }
                Err(why) => control::error(why),
            };
            let _ = reply.send(answer);
        })
        .detach();
    }

    /// The folder of looks for `pane` and `task`, under the materials folder: one name a pane, a
    /// word nobody can guess in it; and the next picture's number in it. The first picture
    /// written makes it, 0700. Another task's folder of the pane goes.
    fn look_dir(&mut self, pane: PaneId, task: TaskId) -> Option<(PathBuf, u32)> {
        if let Some((_, dir, count)) = self.looks.folders.get_mut(&pane).filter(|(of, _, _)| *of == task) {
            *count += 1;
            return Some((dir.clone(), *count));
        }
        let word = crate::collect::new_word()?;
        let dir = crate::config::materials_dir()?.join(format!("agent-{}-{task}-{word}", std::process::id()));
        if let Some((_, old, _)) = self.looks.folders.insert(pane, (task, dir.clone(), 1)) {
            let _ = std::fs::remove_dir_all(old);
        }
        Some((dir, 1))
    }

    /// Each tick: where the open menu's robot is; a look folder whose pane is no longer seen by
    /// its task goes, pictures and all.
    pub(crate) fn sync_agent_looks(&mut self) {
        let robot = self.model.agent.menu.and_then(|menu| self.controls.bounds(&robot_control(menu.pane)));
        if let Some(robot) = robot {
            self.looks.robot = Some(rect(robot));
        }
        let gone: Vec<PaneId> = self
            .looks
            .folders
            .iter()
            .filter(|(pane, (task, _, _))| !self.still_sees(**pane, *task))
            .map(|(p, _)| *p)
            .collect();
        for pane in gone {
            if let Some((_, dir, _)) = self.looks.folders.remove(&pane) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}
