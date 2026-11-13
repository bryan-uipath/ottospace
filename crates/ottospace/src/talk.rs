//! Shell half of the right drawer: a task's agent in a terminal of its own, beside the
//! workspace and in no tab, and the 48 pt rail at the window's right edge. The core says which
//! conversation shows and who has the keyboard (`Model::talk`); this file starts the agent the
//! core asks for (`Effect::StartAgent`) and draws it. See `docs/ai/task-conversation.md`.

use std::collections::HashMap;
use std::time::Instant;

use gpui::{
    AnyElement, AnyView, Bounds, Context, CursorStyle, Div, EmptyView, KeyDownEvent, MouseButton, MouseDownEvent,
    Pixels, ScrollWheelEvent, Stateful, Window, canvas, div, prelude::*, px, rgb, svg,
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
    /// The id it was started with (Claude's), and where it runs: to see if its session was written.
    session: Option<String>,
    cwd: Option<String>,
    /// The core was told it ended.
    told: bool,
}

/// What a drag of the drawer's left edge carries.
pub(crate) struct DraggedEdge;

/// What the drawer holds, as last drawn.
#[derive(Default)]
pub struct Drawer {
    pub agents: HashMap<TaskId, Agent>,
    /// Where the conversation's terminal was last drawn.
    pub body: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// Its width at the last render: the draft's collecting state takes the same.
    pub width: f32,
    /// A read of new Codex sessions under way, for agents whose session is not known yet.
    reading: Option<std::sync::mpsc::Receiver<Vec<agents::Started>>>,
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
        // No repository: the home folder, as the core says, not wherever ottospace was started.
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        let cwd = cwd.or(home.as_deref());
        let terminal = Terminal::run(&program(name)?, &args, &vars, cwd).map_err(|err| format!("{name}: {err}"))?;
        let (given, cwd) = (session.clone(), cwd.map(str::to_string));
        let agent = Agent { terminal, since: Instant::now(), model, told: false, session: given, cwd };
        self.drawer.agents.insert(task, agent);
        Ok(session)
    }

    /// Ends a task's agent: dropping its terminal hangs up on it. Returns what the core is told.
    pub(crate) fn stop_agent(&mut self, task: TaskId) -> Vec<Action> {
        let missing = self.session_missing(task);
        self.drawer.agents.remove(&task);
        missing.into_iter().chain([Action::AgentExited { task }]).collect()
    }

    /// The session id a gone agent was started with, if no Claude folder holds its file
    /// (`projects/<folder>/<id>.jsonl`): it was never written. `None` too when sessions are not
    /// read (`"sessions": false`): nothing is known.
    fn session_missing(&self, task: TaskId) -> Option<Action> {
        let agent = self.drawer.agents.get(&task)?;
        let (id, cwd) = (agent.session.clone()?, agent.cwd.as_ref()?);
        let dirs = crate::config::session_dirs()?;
        let real = std::fs::canonicalize(cwd).ok()?;
        let folder = ottospace_sessions::claude::slug(&real);
        let file = format!("{id}.jsonl");
        let written = dirs.claude.iter().any(|dir| dir.join("projects").join(&folder).join(&file).is_file());
        (!written).then_some(Action::SessionMissing { task, id })
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
            let news = self.session_missing(task).into_iter().chain([Action::AgentExited { task }]);
            for action in news.collect::<Vec<_>>() {
                self.dispatch(action, Origin::System, window, cx);
            }
        }
        if redraw {
            cx.notify();
        }
    }

    /// A key while the conversation has the keyboard: to the agent, as a terminal pane's keys go
    /// to its program. Esc too: it is the agent's. ⌘ shortcuts fall through to their bindings.
    pub(crate) fn talk_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        // An agent runs with the user's rights: a client types nothing into it (⌃C, ⌃D included).
        if self.input != Origin::User {
            self.error = Some(agents::USERS_ALONE.into());
            return cx.stop_propagation();
        }
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
    /// to write), else the conversation, else a task's brief beside its tabs. Its left edge
    /// drags to set its width.
    pub(crate) fn render_drawer(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let drawer = if self.model.compose.is_some() || self.model.welcome() {
            self.render_new_task(width, cx).into_any_element()
        } else {
            self.render_talk(width, cx).or_else(|| self.render_brief(cx))?
        };
        let edge = div().id("drawer-edge").absolute().top_0().bottom_0().left(px(-3.)).w(px(6.));
        let edge = edge.cursor(CursorStyle::ResizeLeftRight).on_drag(DraggedEdge, |_, _, _, cx| cx.new(|_| EmptyView));
        Some(div().relative().flex_none().h_full().flex().child(drawer).child(edge).into_any_element())
    }

    /// The drawer's edge dragged to `x` in the window: its width is what lies right of it, less the rail.
    pub(crate) fn drawer_edge_moved(&mut self, x: f32, window: &mut Window, cx: &mut Context<Self>) {
        let width = f32::from(window.viewport_size().width) - x - RAIL;
        self.dispatch(Action::TalkResize { width }, self.input, window, cx);
    }

    /// The rail at the right edge: a cell per open conversation (`Model::rail`), then `+`.
    /// A click on a cell shows its conversation; on the one shown, closes the drawer.
    pub(crate) fn render_right_rail(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        // The cell the keyboard is on, when the right rail has it (`zones`).
        let focused = match self.model.rail_focus {
            Some(ottospace_core::model::zones::RailFocus::Right(at)) => Some(at),
            _ => None,
        };
        let cells = self.model.rail().into_iter().enumerate().map(|(at, cell)| {
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
                // At work: a steady dot at the corner. Nothing on the rail moves.
                .children(cell.working.then(|| {
                    let dot = home::dot(theme::PICK).absolute().top(px(-3.)).right(px(-3.));
                    dot.border_1().border_color(rgb(theme::BACKGROUND))
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
                .when(focused == Some(at), crate::focus_ring)
                .child(badge)
                .child(self.controls.mark(format!("rail-talk:{task}")))
                // The task's hover card; the short tip only if the task is gone.
                .map(|d| match crate::progress::card_tip(self, task) {
                    Some(card) => d.tooltip(card),
                    None => d.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone())))),
                })
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
        let new = new.when(focused.is_some_and(|at| at == cells.len()), crate::focus_ring);
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

    /// About once a second while an agent's session is not known (`Model::seeking`): reads, on a
    /// thread of its own, the sessions Codex wrote in those folders, and tells the core
    /// (`AgentSessions`), which links each agent to its own.
    pub(crate) fn seek_sessions(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(found) = self.drawer.reading.as_ref().and_then(|read| read.try_recv().ok()) {
            self.drawer.reading = None;
            let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
            let news = Action::AgentSessions { found, home, now: crate::now_ms() };
            self.dispatch(news, Origin::System, window, cx);
        }
        let seeking = self.model.seeking();
        // Read only where sessions are read: not with `"sessions": false`, nor in a test run
        // whose settings name no folder.
        let Some(codex_home) = crate::config::session_dirs().and_then(|dirs| dirs.codex) else { return };
        if seeking.is_empty() || self.drawer.reading.is_some() {
            return;
        }
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        let dirs: Vec<String> = seeking.into_iter().filter_map(|(_, cwd)| cwd.or_else(|| home.clone())).collect();
        let (send, read) = std::sync::mpsc::channel();
        self.drawer.reading = Some(read);
        std::thread::spawn(move || {
            use ottospace_sessions::Runner;
            let codex = ottospace_sessions::codex::Codex::new(codex_home);
            // A folder as the core names it: Codex writes where it ran with links resolved.
            let named = |cwd: &std::path::Path| {
                dirs.iter().find(|dir| std::fs::canonicalize(dir).is_ok_and(|real| real == cwd)).cloned()
            };
            let roots: Vec<std::path::PathBuf> =
                dirs.iter().filter_map(|dir| std::fs::canonicalize(dir).ok()).collect();
            let sessions = codex.sessions(&roots).unwrap_or_default();
            let found = sessions.into_iter().filter_map(|session| {
                let (cwd, started) = (named(&session.cwd)?, session.started?);
                Some(agents::Started { runner: "codex".into(), id: session.id, cwd, started })
            });
            let _ = send.send(found.collect());
        });
    }

    /// ⌘V while the conversation has the keyboard: the clipboard's text to the agent, bracketed
    /// if it asked for that. False: the conversation does not have the keyboard.
    pub(crate) fn talk_paste(&mut self, text: Option<String>) -> bool {
        let shown = self.model.talk.filter(|_| self.model.mode() == Mode::Talk).map(|talk| talk.task);
        let Some(agent) = shown.and_then(|task| self.drawer.agents.get(&task)) else { return false };
        if let Some(text) = text.filter(|_| self.input == Origin::User) {
            agent.terminal.paste(&text);
        }
        true
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
            "width": self.drawer.width,
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
