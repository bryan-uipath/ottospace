//! Shell half of the right drawer: a task's agent in a terminal of its own, beside the
//! workspace and in no tab, and the 48 pt rail at the window's right edge. The core says which
//! conversation shows and who has the keyboard (`Model::talk`); this file starts the agent the
//! core asks for (`Effect::StartAgent`) and draws it. See `docs/ai/task-conversation.md`.

use std::collections::HashMap;
use std::time::Instant;

use gpui::{
    AnyElement, AnyView, Bounds, Context, Div, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, ScrollWheelEvent,
    Stateful, Window, canvas, div, prelude::*, px, rgb, svg,
};
use ottospace_core::agents::{self, Command, Start};
use ottospace_core::keys::{self, Mods};
use ottospace_core::tasks::{Group, TaskId};
use ottospace_core::terminal::scroll_key;
use ottospace_core::{Action, Axis, Kind, Mode, Origin, theme};

use crate::terminal::{self, Terminal};
use crate::{Tooltip, Workspace, collect, home};

/// The right rail's width, in points.
pub const RAIL: f32 = 48.;

/// A task's agent, running in a terminal of the drawer's or ended there.
pub struct Agent {
    pub terminal: Terminal,
    since: Instant,
    /// What the header names: the model, else the runner.
    pub model: String,
    /// The core was told it ended.
    told: bool,
}

/// What the drawer holds, as last drawn.
#[derive(Default)]
pub struct Drawer {
    pub agents: HashMap<TaskId, Agent>,
    /// Where the conversation's terminal was last drawn.
    pub body: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// Its width at the last render: the draft's collecting state takes the same.
    pub width: f32,
    /// The working dots are lit, in their blink; `still`: the system asks for less motion.
    pub blink: bool,
    pub still: bool,
}

/// A new session's id: a random (version 4) UUID. `None`: no random bytes could be read.
fn new_uuid() -> Option<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut b)).ok()?;
    (b[6], b[8]) = ((b[6] & 0x0f) | 0x40, (b[8] & 0x3f) | 0x80);
    let hex = |range: std::ops::Range<usize>| b[range].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    Some(format!("{}-{}-{}-{}-{}", hex(0..4), hex(4..6), hex(6..8), hex(8..10), hex(10..16)))
}

/// Where a runner's program is found. A test run may name a folder of stand-ins for it
/// (`OTTOSPACE_AGENTS_DIR`): never outside one, so no test starts a real agent by mistake.
/// A test run that names no such folder starts no agent at all.
fn program(name: &str) -> Result<String, String> {
    if std::env::var_os("OTTOSPACE_E2E").is_none() {
        return Ok(name.to_string());
    }
    let dir =
        std::env::var_os("OTTOSPACE_AGENTS_DIR").ok_or("a test run starts no agent without OTTOSPACE_AGENTS_DIR")?;
    Ok(std::path::Path::new(&dir).join(name).to_string_lossy().into_owned())
}

impl Workspace {
    /// Starts a task's agent: the command comes from the core's fixed table, with a new session's
    /// id. Returns that id, if the runner takes one, or why nothing started.
    pub(crate) fn start_agent(
        &mut self,
        task: TaskId,
        start: &Start,
        cwd: Option<&str>,
        brief: Option<&str>,
    ) -> Result<Option<String>, String> {
        let brief = brief.and_then(|dir| collect::task_dir(task, dir)).map(|dir| dir.join("brief.md"));
        let brief = brief.as_ref().and_then(|path| path.to_str());
        let (command, session, model) = match start {
            Start::New(launch) => {
                let session = new_uuid().ok_or("no random bytes for a session id")?;
                let command = launch.command(&session, brief).ok_or("no command for this agent")?;
                let model = launch.model.clone().unwrap_or_else(|| launch.runner.to_string());
                (command, agents::names_session(launch.runner).then_some(session), model)
            }
            Start::Resume { runner, session } => {
                (agents::resume(runner, session).ok_or("no command for this session")?, None, runner.clone())
            }
        };
        let Command { program: name, args } = command;
        // The agent can ask for its task's brief over the socket, as in a task's own terminals.
        let vars = [("OTTOSPACE_TASK", task.to_string())];
        let terminal = Terminal::run(&program(name)?, &args, &vars, cwd).map_err(|err| format!("{name}: {err}"))?;
        let agent = Agent { terminal, since: Instant::now(), model, told: false };
        self.drawer.agents.insert(task, agent);
        Ok(session)
    }

    /// Ends a task's agent: dropping its terminal hangs up on it.
    pub(crate) fn stop_agent(&mut self, task: TaskId) {
        self.drawer.agents.remove(&task);
    }

    /// Fits the conversation's terminal to the drawer, redraws on its output, and tells the
    /// core of an agent whose program ended. Its last screen stays on show.
    pub(crate) fn sync_agents(&mut self, window: &Window, cx: &mut Context<Self>) {
        let shown = self.model.talk.filter(|talk| talk.open).map(|talk| talk.task);
        let (mut redraw, mut ended) = (false, Vec::new());
        for (task, agent) in &mut self.drawer.agents {
            if shown == Some(*task) {
                agent.terminal.fit(self.drawer.body.get().size, self.cell);
            }
            redraw |= agent.terminal.take_dirty();
            if agent.terminal.exited() && !agent.told {
                agent.told = true;
                ended.push(*task);
            }
        }
        for task in ended {
            self.dispatch(Action::AgentExited { task }, Origin::System, window, cx);
        }
        if redraw {
            cx.notify();
        }
    }

    /// A key while the conversation has the keyboard: to the agent, as a terminal pane's keys go
    /// to its program. Esc too: it is the agent's. ⌘ shortcuts fall through to their bindings.
    pub(crate) fn talk_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(agent) = self.model.talk.and_then(|talk| self.drawer.agents.get(&talk.task)) else { return };
        let terminal = &agent.terminal;
        let (key, m) = (&event.keystroke, event.keystroke.modifiers);
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, cmd: m.platform };
        if let Some(scroll) = scroll_key(&key.key, mods, terminal.alt_screen()) {
            terminal.scroll(scroll);
            cx.notify();
            return cx.stop_propagation();
        }
        if let Some(bytes) = keys::encode(&key.key, key.key_char.as_deref(), mods, terminal.app_cursor()) {
            terminal.write(bytes);
            cx.stop_propagation();
        }
    }

    /// A label typed in sneak mode on one of the drawer's, the right rail's or the empty
    /// workspace's controls: what a click on it does. False: not one of theirs.
    pub(crate) fn activate_drawer(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let task = self.model.talk.map(|talk| talk.task);
        let action = match (kind, name, task) {
            ("rail", "new-task", _) | ("drawer", "new_task", _) => {
                self.new_task(window, cx);
                return true;
            }
            ("talk", "talk-go", _) => {
                self.talk_go(window, cx);
                return true;
            }
            ("talk", "talk-brief", Some(id)) if self.model.brief == Some(id) => Action::BriefClose,
            ("talk", "talk-brief", Some(id)) => Action::BriefShow { id },
            ("talk", "talk-stop", Some(id)) => Action::TaskStop { id },
            ("talk", "talk-close", _) => Action::TalkToggle,
            ("rail-talk", id, _) => match id.parse() {
                Ok(id) => Action::TalkPick { id },
                Err(_) => return true,
            },
            ("talk", "", _) => Action::TalkKeys { on: true },
            ("drawer", "close", _) => Action::CloseCompose,
            ("home", "open_app", _) => {
                self.open_picker(Axis::Right, false, window, cx);
                return true;
            }
            ("home", "terminal", _) => Action::Split { axis: Axis::Right, kind: Kind::Terminal },
            ("pick", _, _) => {
                let mut chars = name.chars();
                let Some(mark) = chars.next() else { return true };
                let rest = chars.as_str();
                if self.model.compose.is_none() {
                    self.new_task(window, cx);
                }
                Action::ComposePick { mark, name: (!rest.is_empty()).then(|| rest.to_string()) }
            }
            _ => return false,
        };
        self.dispatch(action, self.input, window, cx);
        true
    }

    /// True if the open drawer is on this task: its header names it.
    pub(crate) fn talk_names(&self, task: TaskId) -> bool {
        self.model.talk.is_some_and(|talk| talk.open && talk.task == task)
    }

    /// ⌘↵ with a conversation open: shows its task's tabs, or leaves them if they show.
    pub(crate) fn talk_go(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(task) = self.model.talk.filter(|talk| talk.open).map(|talk| talk.task) else { return };
        let action = match self.model.talk_goes_to() {
            Some(_) => Action::TaskOpen { id: Some(task) },
            None if self.model.current_task == Some(task) => Action::TaskLeave,
            None => return,
        };
        self.dispatch(action, self.input, window, cx);
    }

    /// A small button of the drawer's header or the rail, named for sneak mode and tests.
    fn talk_button(&self, name: &'static str, label: &str, tip: &'static str) -> Stateful<Div> {
        div()
            .id(name)
            .relative()
            .flex_none()
            .px_1p5()
            .h(px(22.))
            .flex()
            .items_center()
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
            .child(label.to_string())
            .child(self.controls.mark(format!("talk:{name}")))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
            // The press must not reach the drawer, which takes the keyboard on a press.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    /// The conversation's header: state, title, model and time, then what can be done.
    fn talk_header(&self, task: TaskId, cx: &mut Context<Self>) -> Option<Div> {
        let shown = self.model.tasks.get(task)?;
        let group = shown.group().unwrap_or(Group::Captured);
        let agent = self.drawer.agents.get(&task);
        let runs = self.model.agent_runs(task);
        let muted = |text: String| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let elapsed = agent.filter(|_| runs).map(|agent| {
            let secs = agent.since.elapsed().as_secs();
            if secs < 3600 { format!("{}:{:02}", secs / 60, secs % 60) } else { format!("{}h", secs / 3600) }
        });
        // ⌘↵: the task's tabs into the workspace, or, while they are on show, back out of them.
        let go = match (self.model.talk_goes_to(), self.model.current_task == Some(task)) {
            (Some(name), _) => Some(self.talk_button("talk-go", &format!("Go to {name} ⌘↵"), "Show its tabs")),
            (None, true) => Some(self.talk_button("talk-go", "Leave ⌘↵", "Back to the tabs of no task")),
            (None, false) => None,
        };
        let go = go.map(|button| button.on_click(cx.listener(|this, _, window, cx| this.talk_go(window, cx))));
        let briefed = !shown.materials.is_empty();
        let brief = briefed.then(|| {
            let on = self.model.brief == Some(task);
            self.talk_button("talk-brief", if on { "Agent" } else { "Brief" }, "The task's brief, or its agent")
                .on_click(cx.listener(move |this, _, window, cx| {
                    let action = if on { Action::BriefClose } else { Action::BriefShow { id: task } };
                    this.dispatch(action, this.input, window, cx);
                }))
        });
        let stop = runs.then(|| {
            self.talk_button("talk-stop", "Stop", "End this task's agent").text_color(rgb(theme::ERROR)).on_click(
                cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::TaskStop { id: task }, this.input, window, cx);
                }),
            )
        });
        let close = self.talk_button("talk-close", "×", "Close to the rail (⌘⇧L); the agent goes on").on_click(
            cx.listener(|this, _, window, cx| drop(this.dispatch(Action::TalkToggle, this.input, window, cx))),
        );
        Some(
            // As high as the tab strip: one title strip runs across the window.
            div()
                .h(px(crate::TOP_BAR))
                .flex_none()
                .pl_3()
                .pr_1()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .child(home::dot(theme::task(group)))
                .child(div().flex_1().min_w_0().truncate().text_sm().child(shown.title.clone()))
                .children(agent.map(|agent| muted(agent.model.clone())))
                .children(elapsed.map(muted))
                .children(go)
                .children(brief)
                .children(stop)
                .child(close),
        )
    }

    /// The conversation: the agent's terminal as it is drawn in a pane. Stage 1 of the one seam
    /// (`docs/ai/task-conversation.md`): later stages draw the session natively here.
    fn talk_body(&self, task: TaskId, cx: &mut Context<Self>) -> AnyElement {
        let at = self.drawer.body.clone();
        let record = canvas(move |bounds, _, _| at.set(bounds), |_, _, _, _| {}).absolute().size_full();
        let muted = |text: String| div().p_3().text_sm().text_color(rgb(theme::MUTED)).child(text);
        let Some(agent) = self.drawer.agents.get(&task) else {
            let why = self.model.talk_note().unwrap_or_else(|| "no agent runs for this task".into());
            return div().relative().flex_1().child(record).child(muted(why)).into_any_element();
        };
        let (screen, cell) = (agent.terminal.screen(), self.cell);
        let keys = self.model.mode() == Mode::Talk;
        let grid =
            canvas(|_, _, _| {}, move |bounds, _, window, cx| terminal::paint(&screen, bounds, cell, keys, window, cx))
                .size_full();
        let ended = agent.terminal.exited().then(|| {
            div()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .border_t_1()
                .border_color(rgb(theme::BORDER))
                .child("The agent ended. A click on its task goes on with its session.")
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().relative().flex_1().min_h_0().m_1().child(record).child(grid).on_scroll_wheel(cx.listener(
                move |this, e: &ScrollWheelEvent, _, cx| {
                    let height = this.cell.height;
                    if let Some(agent) = this.drawer.agents.get_mut(&task) {
                        agent.terminal.wheel(e.delta.pixel_delta(height).y.into(), height.into());
                        cx.notify();
                    }
                },
            )))
            .children(ended)
            .into_any_element()
    }

    /// The drawer on a task's conversation, `width` points wide. A press in it gives it the
    /// keyboard. The brief shows in place of the agent while it is asked for.
    pub(crate) fn render_talk(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let talk = self.model.talk.filter(|talk| talk.open)?;
        let header = self.talk_header(talk.task, cx)?;
        let body = match self.model.brief == Some(talk.task) {
            true => self.brief_content(cx).unwrap_or_else(|| div().into_any_element()),
            false => self.talk_body(talk.task, cx),
        };
        let drawer = div()
            .id("talk")
            .relative()
            .flex_none()
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BACKGROUND))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(self.controls.mark("talk"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.dispatch(Action::Leave, this.input, window, cx);
                    this.dispatch(Action::TalkKeys { on: true }, this.input, window, cx);
                }),
            )
            .child(header)
            .children(
                self.model
                    .talk_note()
                    .map(|note| div().px_3().py_1().text_xs().text_color(rgb(theme::ERROR)).child(note)),
            )
            .child(body);
        Some(drawer.into_any_element())
    }

    /// The drawer, if it shows: the new task being written (or, with nothing open yet, the one
    /// to write), else the conversation, else a task's brief beside its tabs.
    pub(crate) fn render_drawer(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.model.compose.is_some() || self.model.welcome() {
            return Some(self.render_new_task(width, cx).into_any_element());
        }
        self.render_talk(width, cx).or_else(|| self.render_brief(cx))
    }

    /// The rail at the right edge: a cell per open conversation (`Model::rail`), then `+`.
    /// A click on a cell shows its conversation; on the one shown, closes the drawer.
    pub(crate) fn render_right_rail(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let cells = self.model.rail().into_iter().map(|cell| {
            let (task, tip) = (cell.task, gpui::SharedString::from(cell.tip.clone()));
            let ring = cell.group.map_or(theme::MUTED, theme::task);
            let badge = div()
                .relative()
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .border_2()
                .border_color(rgb(ring))
                .bg(rgb(theme::SELECTED))
                .text_xs()
                .text_color(rgb(theme::TEXT))
                .child(cell.mono.clone())
                // At work: a dot at the corner that blinks, unless the system asks for less motion.
                .children(cell.working.then(|| {
                    let lit = self.drawer.blink || self.drawer.still;
                    home::dot(theme::PICK)
                        .absolute()
                        .top(px(-3.))
                        .right(px(-3.))
                        .border_1()
                        .border_color(rgb(theme::BACKGROUND))
                        .when(!lit, |d| d.opacity(0.25))
                }));
            div()
                .id(("rail-talk", task as usize))
                .relative()
                .flex_none()
                .size(px(RAIL))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .when(cell.shown, |d| crate::raised_cell(d, ring, true))
                .child(badge)
                .child(self.controls.mark(format!("rail-talk:{task}")))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::TalkPick { id: task }, this.input, window, cx);
                }))
        });
        let cells: Vec<_> = cells.collect();
        let divider = (!cells.is_empty()).then(|| div().mx_3().my_1().h(px(1.)).bg(rgb(theme::BORDER_STRONG)));
        let new = div()
            .id("new-task")
            .relative()
            .flex_none()
            .size(px(RAIL))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .tooltip(|_, cx| AnyView::from(cx.new(|_| Tooltip("New task (⌘N)".into()))))
            .child(self.controls.mark("rail:new-task"))
            .child(svg().path("icons/plus.svg").size(px(16.)).text_color(rgb(theme::MUTED)))
            .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)));
        div()
            .id("right-rail")
            // The raised cell's shadow falls on its neighbours, never over the drawer.
            .overflow_hidden()
            .flex_none()
            .w(px(RAIL))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            // The title strip runs on over the rail.
            .child(div().h(px(crate::TOP_BAR)).flex_none().border_b_1().border_color(rgb(theme::BORDER)))
            .children(cells)
            .children(divider)
            .child(new)
    }

    /// ⌘⌃1 … ⌘⌃9: the nth conversation of the right rail, as a click on its cell.
    pub(crate) fn rail_key(&mut self, n: usize, window: &Window, cx: &mut Context<Self>) {
        if let Some(cell) = self.model.rail().get(n) {
            self.dispatch(Action::TalkPick { id: cell.task }, self.input, window, cx);
        }
    }

    /// Every half second while a rail cell's agent is at work: its dot blinks.
    pub(crate) fn blink(&mut self, cx: &mut Context<Self>) {
        if !self.drawer.still && self.model.rail().iter().any(|cell| cell.working) {
            self.drawer.blink = !self.drawer.blink;
            cx.notify();
        }
    }

    /// What the state reply says of the drawer: where it is drawn, and each agent's terminal.
    pub(crate) fn talk_snapshot(&self) -> serde_json::Value {
        let agents: serde_json::Map<String, serde_json::Value> = self
            .drawer
            .agents
            .iter()
            .map(|(task, agent)| {
                let (cols, rows) = agent.terminal.grid_size();
                let terminal = serde_json::json!({
                    "model": agent.model, "ended": agent.terminal.exited(),
                    "cols": cols, "rows": rows, "lines": agent.terminal.lines(),
                });
                (task.to_string(), terminal)
            })
            .collect();
        let shown = self.model.talk.is_some_and(|talk| talk.open);
        serde_json::json!({
            "body": shown.then(|| crate::rect(self.drawer.body.get())),
            "agents": agents,
            "goes_to": self.model.talk_goes_to(),
            // The right rail's conversations, in order: task, badge, state, at work, shown, tooltip.
            "rail": self.model.rail(),
            "note": self.model.talk_note(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_id_is_a_version_4_uuid() {
        let (one, two) = (new_uuid().expect("an id"), new_uuid().expect("an id"));
        assert!(one != two && agents::is_uuid(&one) && agents::is_uuid(&two));
        assert_eq!(&one[14..15], "4");
        assert!(matches!(one.as_bytes()[19], b'8' | b'9' | b'a' | b'b'), "{one}");
    }
}
