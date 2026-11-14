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
use ottospace_core::keys;
use ottospace_core::runner::RunnerId;
use ottospace_core::talk::{self, Subject};
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
    /// The runner and id it was started with, if the id was chosen here, and where it runs: to
    /// see if its session was written.
    session: Option<(RunnerId, String)>,
    cwd: Option<String>,
    /// The core was told it ended.
    told: bool,
}

/// What a drag of the drawer's left edge carries.
pub(crate) struct DraggedEdge;

/// What the drawer holds, as last drawn.
#[derive(Default)]
pub struct Drawer {
    pub agents: HashMap<Subject, Agent>,
    /// Where the conversation's terminal was last drawn.
    pub body: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// Its width at the last render: the draft's collecting state takes the same.
    pub width: f32,
    /// A read of new sessions under way, for agents whose runner names its own (Codex).
    reading: Option<std::sync::mpsc::Receiver<Vec<agents::Started>>>,
    /// Reads of one session each under way, for resumes that wait on them (`SessionRead`).
    session_reads: Vec<(String, String, std::sync::mpsc::Receiver<Action>)>,
    /// Native conversations' agents, and what the drawer keeps of each one's look (`conversation`).
    pub native: HashMap<Subject, crate::conversation::Native>,
    pub looks: HashMap<Subject, crate::conversation::Look>,
    pub thumbs: HashMap<String, crate::conversation::Thumb>,
    /// The kinds of link the user opened (`web`, `file`): a test run opens nothing and reads these.
    pub opened: Vec<&'static str>,
    /// Agents stopped for a switch of view, until their process is gone (`Stopping`).
    pub stopping: Vec<(Subject, crate::conversation::Stopping)>,
    /// The watched session's file being followed (`Model::watch_target`).
    pub watch: Option<crate::conversation::Watch>,
    /// The end of each ended native agent's stderr, cleaned and its tokens hidden
    /// (`transcript::details`): drawn on the user's click only, never in the state or a log.
    pub details: HashMap<Subject, String>,
}

/// A new session's id: a random (version 4) UUID. `None`: no random bytes could be read.
pub(crate) fn new_uuid() -> Option<String> {
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
pub(crate) fn program(name: &str) -> Result<String, String> {
    if std::env::var_os("OTTOSPACE_E2E").is_none() {
        return Ok(name.to_string());
    }
    let dir =
        std::env::var_os("OTTOSPACE_AGENTS_DIR").ok_or("a test run starts no agent without OTTOSPACE_AGENTS_DIR")?;
    Ok(std::path::Path::new(&dir).join(name).to_string_lossy().into_owned())
}

/// How a conversation is named in the state reply and on its rail cell's control: a task's id,
/// or `<runner>:<session id>`.
pub fn key(subject: &Subject) -> String {
    match subject {
        Subject::Task(id) => id.to_string(),
        Subject::Session { runner, id } => format!("{runner}:{id}"),
    }
}

/// The conversation a `key` names.
fn subject(key: &str) -> Option<Subject> {
    match key.split_once(':') {
        Some((runner, id)) => Some(Subject::Session { runner: runner.into(), id: id.into() }),
        None => key.parse().ok().map(Subject::Task),
    }
}

impl Workspace {
    /// Starts a conversation's agent: the command comes from the core's fixed table, with a new
    /// session's id. Returns that id, if the runner takes one, or why nothing started.
    pub(crate) fn start_agent(
        &mut self,
        subject: &Subject,
        start: &Start,
        cwd: Option<&str>,
        brief: Option<&str>,
    ) -> Result<Option<String>, String> {
        let task = subject.task();
        let brief =
            brief.zip(task).and_then(|(dir, task)| collect::task_dir(task, dir)).map(|dir| dir.join("brief.md"));
        let brief = brief.as_ref().and_then(|path| path.to_str());
        // Drawn natively: the runner's structured mode, not its TUI.
        if self.model.talk_view(subject) == ottospace_core::transcript::View::Native {
            return self.start_native(subject, start, cwd, brief);
        }
        let (command, session, model) = match start {
            Start::New(launch) => {
                let session = new_uuid().ok_or("no random bytes for a session id")?;
                let command = launch.command(&session, brief).ok_or("no command for this agent")?;
                let model = launch.model.clone().unwrap_or_else(|| launch.runner.name().to_string());
                (command, launch.runner.spec().chooses_session().then_some((launch.runner, session)), model)
            }
            Start::Resume { runner, session } => {
                // Where the session started: the core checked its words, this checks where it leads.
                let own = task.and_then(|id| self.model.tasks.get(id)).and_then(|task| task.repo.as_deref());
                if let Some(dir) = cwd.filter(|dir| Some(*dir) != own) {
                    self.in_checkout(dir)?;
                }
                (
                    agents::resume(*runner, session).ok_or("no command for this session")?,
                    None,
                    runner.name().to_string(),
                )
            }
        };
        let Command { program: name, args } = command;
        // The agent can ask for its task's brief over the socket, as in a task's own terminals.
        let vars: Vec<(&str, String)> = task.map(|id| ("OTTOSPACE_TASK", id.to_string())).into_iter().collect();
        // No repository: the home folder, as the core says, not wherever ottospace was started.
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        let cwd = cwd.or(home.as_deref());
        let terminal = Terminal::run(&program(name)?, &args, &vars, cwd).map_err(|err| format!("{name}: {err}"))?;
        let (given, cwd) = (session.clone(), cwd.map(str::to_string));
        let session = session.map(|(_, id)| id);
        let agent = Agent { terminal, since: Instant::now(), model, told: false, session: given, cwd };
        self.drawer.agents.insert(subject.clone(), agent);
        Ok(session)
    }

    /// `dir`, its links resolved, is in a listed repository or worktree, also resolved.
    pub(crate) fn in_checkout(&self, dir: &str) -> Result<(), String> {
        let real = std::fs::canonicalize(dir).map_err(|err| format!("{dir}: {err}"))?;
        let roots = self.model.repo_names().filter_map(|(_, path)| std::fs::canonicalize(path).ok());
        roots
            .into_iter()
            .any(|root| real.starts_with(root))
            .then_some(())
            .ok_or(format!("{dir} is in no listed repository"))
    }

    /// Ends a conversation's agent: dropping its terminal hangs up on it. Returns what the core is told.
    /// Stopped for a switch of view, it is told only once its process is gone, so the other
    /// view never starts on a session still held.
    pub(crate) fn stop_agent(&mut self, subject: &Subject) -> Vec<Action> {
        // Already ending for a switch: it says it ended once its process has, not before.
        if self.drawer.stopping.iter().any(|(of, _)| of == subject) {
            return Vec::new();
        }
        let missing = self.session_missing(subject);
        let terminal = self.drawer.agents.remove(subject).map(|agent| agent.terminal);
        let native = self.drawer.native.remove(subject);
        if self.model.is_switching(subject) && (terminal.is_some() || native.is_some()) {
            let stopping = crate::conversation::Stopping::new(terminal, native);
            self.drawer.stopping.push((subject.clone(), stopping));
            return missing.into_iter().collect();
        }
        missing.into_iter().chain([Action::AgentExited { task: subject.clone() }]).collect()
    }

    /// Reads one session on a thread of its own, for a resume waiting on it. Sessions not read
    /// at all (`"sessions": false`): answered at once, with nothing known.
    pub(crate) fn read_session(&mut self, runner: &str, id: &str) -> Option<Action> {
        let (runner, id) = (runner.to_string(), id.to_string());
        if crate::config::session_dirs().is_none() {
            return Some(Action::SessionRead { runner, id, found: None });
        }
        let (send, read) = std::sync::mpsc::channel();
        self.drawer.session_reads.push((runner.clone(), id.clone(), read));
        std::thread::spawn(move || {
            let found = crate::sessions::read_one(&runner, &id, crate::now_ms());
            let _ = send.send(Action::SessionRead { runner, id, found });
        });
        None
    }

    /// An adopted session's agent is its task's now: its terminal goes on under the task.
    pub(crate) fn move_agent(&mut self, from: &Subject, to: &Subject) {
        if let Some(agent) = self.drawer.agents.remove(from) {
            self.drawer.agents.insert(to.clone(), agent);
        }
        if let Some(native) = self.drawer.native.remove(from) {
            self.drawer.native.insert(to.clone(), native);
        }
        if let Some(look) = self.drawer.looks.remove(from) {
            self.drawer.looks.insert(to.clone(), look);
        }
        // One ending for a switch ends under its new name: the core knows it by that one now.
        for (subject, _) in &mut self.drawer.stopping {
            if subject == from {
                *subject = to.clone();
            }
        }
    }

    /// The session id a task's gone agent was started with, if its runner's folders hold no file
    /// for it (Claude: `projects/<folder>/<id>.jsonl`): it was never written. `None` too when
    /// sessions are not read (`"sessions": false`): nothing is known.
    pub(crate) fn session_missing(&self, subject: &Subject) -> Option<Action> {
        let task = subject.task()?;
        // Its terminal's, or its native process's.
        let native = self.drawer.native.get(subject).map(|native| (&native.session, &native.cwd));
        let (session, cwd) = self.drawer.agents.get(subject).map(|agent| (&agent.session, &agent.cwd)).or(native)?;
        let ((runner, id), cwd) = (session.clone()?, cwd.as_ref()?);
        let dirs = crate::config::session_dirs()?;
        std::fs::canonicalize(cwd).ok()?;
        let written = ottospace_sessions::reader(runner, &dirs).and_then(|reader| reader.path_of(cwd.as_ref(), &id));
        written.is_none().then_some(Action::SessionMissing { task, id })
    }

    /// Fits the conversation's terminal to the drawer, redraws on its output, and tells the
    /// core of an agent whose program ended. Its last screen stays on show.
    pub(crate) fn sync_agents(&mut self, window: &Window, cx: &mut Context<Self>) {
        let shown = self.model.talk.as_ref().filter(|talk| talk.open).map(|talk| &talk.task);
        let (mut redraw, mut ended) = (false, Vec::new());
        for (subject, agent) in &mut self.drawer.agents {
            if shown == Some(subject) {
                agent.terminal.fit(self.drawer.body.get().size, self.cell);
            }
            redraw |= agent.terminal.take_dirty();
            if agent.terminal.exited() && !agent.told {
                agent.told = true;
                ended.push(subject.clone());
            }
        }
        for task in ended {
            let news = self.session_missing(&task).into_iter().chain([Action::AgentExited { task }]);
            for action in news.collect::<Vec<_>>() {
                self.dispatch(action, Origin::System, window, cx);
            }
        }
        // Sessions read for a resume. A thread gone with no answer: nothing is known.
        let mut read = Vec::new();
        self.drawer.session_reads.retain(|(runner, id, answer)| match answer.try_recv() {
            Ok(action) => {
                read.push(action);
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                read.push(Action::SessionRead { runner: runner.clone(), id: id.clone(), found: None });
                false
            }
        });
        for action in read {
            self.dispatch(action, Origin::System, window, cx);
        }
        self.sync_native(window, cx);
        if redraw {
            cx.notify();
        }
    }

    /// A key while the conversation has the keyboard: to the agent, as a terminal pane's keys go
    /// to its program. Esc too: it is the agent's. ⌘ shortcuts fall through to their bindings.
    pub(crate) fn talk_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        // An agent runs with the user's rights: a client types nothing into it (⌃C, ⌃D included).
        if self.input != Origin::User {
            self.error = Some(agents::USERS_ALONE.into());
            return cx.stop_propagation();
        }
        if self.model.shown_chat().is_some() {
            return self.chat_key(event, window, cx);
        }
        let Some(agent) = self.model.talk.as_ref().and_then(|talk| self.drawer.agents.get(&talk.task)) else { return };
        let terminal = &agent.terminal;
        let (key, m) = (&event.keystroke, event.keystroke.modifiers);
        let mods = crate::mods(m);
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
        let shown = self.model.talk.as_ref().map(|talk| talk.task.clone());
        let task = shown.as_ref().and_then(Subject::task);
        let action = match (kind, name, task) {
            ("rail", "new-task", _) | ("drawer", "new_task", _) => {
                self.new_task(window, cx);
                return true;
            }
            ("talk", "talk-go", _) => Action::TalkGo,
            ("talk", "talk-brief", Some(id)) if self.model.brief == Some(id) => Action::BriefClose,
            ("talk", "talk-brief", Some(id)) => Action::BriefShow { id },
            ("talk", "talk-stop", _) => match shown {
                Some(id) => Action::TaskStop { id },
                None => return true,
            },
            ("talk", "talk-adopt", _) => match shown {
                Some(Subject::Session { runner, id }) => Action::SessionAdopt { runner, id },
                _ => return true,
            },
            ("talk", "talk-close", _) => Action::TalkToggle,
            // The native conversation's buttons: the core says who may.
            ("talk", "chat-allow" | "chat-deny" | "chat-show-all", _) => {
                let shown = self.model.shown_chat().and_then(|chat| chat.transcript.cards.front());
                let Some(ticket) = shown.map(|card| card.ticket) else { return true };
                match name {
                    "chat-allow" => Action::TalkAnswer { allow: true, ticket },
                    "chat-deny" => Action::TalkAnswer { allow: false, ticket },
                    _ => Action::TalkExpand { ticket },
                }
            }
            ("talk", "chat-deny-note", _) => Action::TalkDenyNote,
            ("talk", "chat-stop", _) => Action::TalkInterrupt,
            ("talk", "view-chat", _) => Action::TalkView { shows: ottospace_core::transcript::View::Native },
            ("talk", "view-terminal", _) => Action::TalkView { shows: ottospace_core::transcript::View::Terminal },
            ("rail-talk", id, _) => match subject(id) {
                Some(id) => Action::TalkPick { id },
                None => return true,
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
        self.model.talk.as_ref().is_some_and(|talk| talk.open && talk.task == Subject::Task(task))
    }

    /// A task's badge as drawn everywhere: its letters (`Model::badges`), its identity colour,
    /// its state's dot. `None`: no such task, or it is dismissed.
    pub(crate) fn task_badge(&self, id: TaskId, size: f32) -> Option<Div> {
        let task = self.model.tasks.live(id)?;
        let badges = self.badges.borrow();
        let mono = badges.iter().find(|(of, _)| *of == Subject::Task(id)).map(|(_, mono)| mono.clone())?;
        Some(badge(&mono, theme::identity(task.colour), task.group().map(theme::task), size))
    }
}

/// A badge: `mono` in a rounded square bordered in an identity colour, and a state's dot at
/// its corner. The one look of a task on the tab bar, its panes, its row, the rail and the drawer.
pub(crate) fn badge(mono: &str, colour: u32, state: Option<u32>, size: f32) -> Div {
    let dot = (size * 0.36).max(7.);
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size / 4.))
        .border_2()
        .border_color(rgb(colour))
        .bg(rgb(theme::SELECTED))
        .text_size(px((size * 0.4).max(9.)))
        .text_color(rgb(theme::TEXT))
        .child(mono.to_string())
        .children(state.map(|state| {
            let at = px(-dot / 2.);
            div()
                .absolute()
                .top(at)
                .right(at)
                .size(px(dot))
                .rounded_full()
                .bg(rgb(state))
                .border_1()
                .border_color(rgb(theme::BACKGROUND))
        }))
}

impl Workspace {
    /// A small button of the drawer's header or the rail, named for sneak mode and tests.
    pub(crate) fn talk_button(&self, name: &'static str, label: &str, tip: &'static str) -> Stateful<Div> {
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
    fn talk_header(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Div> {
        let shown = match subject {
            Subject::Task(id) => Some(self.model.tasks.get(*id)?),
            Subject::Session { .. } => None,
        };
        // A session's dot is lit while it is at work, as on its row.
        let (title, color) = match (shown, subject) {
            (Some(task), _) => (task.title.clone(), theme::task(task.group().unwrap_or(Group::Captured))),
            (None, Subject::Session { runner, id }) => {
                let at_work = self.model.rail().iter().any(|cell| cell.task == *subject && cell.working);
                let color = if at_work { theme::task(Group::Running) } else { theme::BORDER_STRONG };
                (self.model.session_title(runner, id), color)
            }
            (None, Subject::Task(_)) => return None,
        };
        // The model and since when: of the agent's terminal, or of its native process.
        let native = self.drawer.native.get(subject).map(|native| (native.model.clone(), native.since));
        let agent = self.drawer.agents.get(subject).map(|agent| (agent.model.clone(), agent.since)).or(native);
        let runs = self.model.agent_runs(subject);
        let muted = |text: String| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let elapsed = agent.as_ref().filter(|_| runs).map(|(_, since)| {
            let secs = since.elapsed().as_secs();
            if secs < 3600 { format!("{}:{:02}", secs / 60, secs % 60) } else { format!("{}h", secs / 3600) }
        });
        // ⌘↵: to the task's tabs; with none, claim the tab on show or make it a new one.
        let go = self.model.talk_go().map(|go| {
            let tip = match go {
                talk::Go::Tabs => "Show the tab of this task shown last",
                talk::Go::Claim => "Make the tab on show this task's",
                talk::Go::New => "A new tab of this task, with a terminal in its repository",
            };
            self.talk_button("talk-go", &format!("{} ⌘↵", go.label()), tip).on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::TalkGo, this.input, window, cx))),
            )
        });
        let brief = shown.filter(|task| !task.materials.is_empty()).map(|task| {
            let (task, on) = (task.id, self.model.brief == Some(task.id));
            self.talk_button("talk-brief", if on { "Agent" } else { "Brief" }, "The task's brief, or its agent")
                .on_click(cx.listener(move |this, _, window, cx| {
                    let action = if on { Action::BriefClose } else { Action::BriefShow { id: task } };
                    this.dispatch(action, this.input, window, cx);
                }))
        });
        // A session's: Adopt makes it a task, its conversation and all.
        let adopt = match subject {
            // Only a listed session can be adopted: not with the Tasks section closed, nor hidden.
            Subject::Session { runner, id } if self.model.session_listed(runner, id) => {
                let (runner, id) = (runner.clone(), id.clone());
                Some(self.talk_button("talk-adopt", "Adopt", "Make a task of this session").on_click(cx.listener(
                    move |this, _, window, cx| {
                        let adopt = Action::SessionAdopt { runner: runner.clone(), id: id.clone() };
                        this.dispatch(adopt, this.input, window, cx);
                    },
                )))
            }
            _ => None,
        };
        let stop = runs.then(|| {
            let id = subject.clone();
            self.talk_button("talk-stop", "Stop", "End this conversation's agent")
                .text_color(rgb(theme::ERROR))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::TaskStop { id: id.clone() }, this.input, window, cx);
                }))
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
                .child(match shown.and_then(|task| self.task_badge(task.id, 18.)) {
                    Some(badge) => badge,
                    None => home::dot(color),
                })
                .child(div().flex_1().min_w(px(72.)).truncate().text_sm().child(title))
                // The model gives way before the title does.
                .children(agent.map(|(model, _)| muted(model).flex_shrink().min_w_0().truncate()))
                .children(elapsed.map(muted))
                .children(self.watch_chip(subject))
                .children(self.view_switch(subject, cx))
                .children(go)
                .children(brief)
                .children(adopt)
                .children(stop)
                .child(close),
        )
    }

    /// The conversation: the agent's terminal as it is drawn in a pane. Stage 1 of the one seam
    /// (`docs/ai/task-conversation.md`): later stages draw the session natively here.
    fn talk_body(&self, subject: &Subject, cx: &mut Context<Self>) -> AnyElement {
        let at = self.drawer.body.clone();
        let record = canvas(move |bounds, _, _| at.set(bounds), |_, _, _, _| {}).absolute().size_full();
        let muted = |text: String| div().p_3().text_sm().text_color(rgb(theme::MUTED)).child(text);
        // Drawn natively, or watched read-only: the transcript.
        if self.is_native(subject) {
            return div()
                .relative()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .child(record)
                .child(self.chat_body(subject, cx))
                .into_any_element();
        }
        if self.drawer.stopping.iter().any(|(of, _)| of == subject) {
            let wait = muted("Ending the other view's process before this one starts…".into());
            return div().relative().flex_1().child(record).child(wait).into_any_element();
        }
        let Some(agent) = self.drawer.agents.get(subject) else {
            // The note, if there is one, is the line under the header (`render_talk`): once.
            // Nothing either while its session is read for a resume.
            let none = self.model.talk_note().is_none() && !self.model.talk_live(subject);
            // A task with a session (a conversation shown again at start, say) is gone on with by the user only.
            let session = subject.task().and_then(|id| self.model.tasks.get(id)).is_some_and(|t| t.session.is_some());
            let why = match session {
                true => "Not running. A click on its task goes on with its session.",
                false => "no agent runs for this conversation",
            };
            let why = none.then(|| muted(why.into()));
            return div().relative().flex_1().child(record).children(why).into_any_element();
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
                .child(match subject {
                    Subject::Task(_) => "The agent ended. A click on its task goes on with its session.",
                    Subject::Session { .. } => "The agent ended. A click on its row goes on with its session.",
                })
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().relative().flex_1().min_h_0().m_1().child(record).child(grid).on_scroll_wheel(cx.listener({
                let subject = subject.clone();
                move |this, e: &ScrollWheelEvent, _, cx| {
                    let height = this.cell.height;
                    if let Some(agent) = this.drawer.agents.get_mut(&subject) {
                        agent.terminal.wheel(e.delta.pixel_delta(height).y.into(), height.into());
                        cx.notify();
                    }
                }
            })))
            .children(ended)
            .into_any_element()
    }

    /// The drawer on a task's conversation, `width` points wide. A press in it gives it the
    /// keyboard. The brief shows in place of the agent while it is asked for.
    pub(crate) fn render_talk(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let talk = self.model.talk.as_ref().filter(|talk| talk.open)?;
        let header = self.talk_header(&talk.task, cx)?;
        let body = match self.model.brief.is_some() && self.model.brief == talk.task.task() {
            true => self.brief_content(cx).unwrap_or_else(|| div().into_any_element()),
            false => self.talk_body(&talk.task, cx),
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
            let (name, tip) = (key(&cell.task), gpui::SharedString::from(cell.tip.clone()));
            // Its identity colour (a session's grey) and its state's dot, as on the bar and the rows.
            // Not `task_badge`: a session's cell has no task, and its dot is lit while it works.
            let task_colour = cell.task.task().and_then(|id| self.model.tasks.get(id)).and_then(|task| task.colour);
            let ring = theme::identity(task_colour);
            let state = cell.group.map(theme::task).or(cell.working.then(|| theme::task(Group::Running)));
            let badge = badge(&cell.mono, ring, state, 30.);
            let (task, cell_task) = (cell.task.clone(), cell.task.task());
            div()
                .id(gpui::ElementId::Name(format!("rail-talk:{name}").into()))
                .relative()
                .flex_none()
                .w(px(RAIL))
                .h(px(crate::RAIL_CELL))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .when(cell.shown, |d| crate::raised_cell(d, ring, true, at == 0))
                .when(focused == Some(at), crate::focus_ring)
                .child(badge)
                .child(self.controls.mark(format!("rail-talk:{name}")))
                // The task's hover card; the short tip only if the task is gone.
                .map(|d| match cell_task.and_then(|task| crate::progress::card_tip(self, task)) {
                    Some(card) => d.tooltip(card),
                    None => d.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone())))),
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::TalkPick { id: task.clone() }, this.input, window, cx);
                }))
        });
        let cells: Vec<_> = cells.collect();
        let divider = (!cells.is_empty()).then(|| div().mx_3().my_1().h(px(1.)).bg(rgb(theme::BORDER_STRONG)));
        let new = div()
            .id("new-task")
            .relative()
            .flex_none()
            .w(px(RAIL))
            .h(px(crate::RAIL_CELL))
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
        if let Some(cell) = self.model.rail().into_iter().nth(n) {
            self.dispatch(Action::TalkPick { id: cell.task }, self.input, window, cx);
        }
    }

    /// About once a second while an agent's session is not known (`Model::seeking`): reads, on a
    /// thread of its own, the sessions their runners wrote in those folders, and tells the core
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
        let Some(session_dirs) = crate::config::session_dirs() else { return };
        // Each runner sought, with its reader: one whose folder is not read finds nothing.
        let mut readers: Vec<(RunnerId, Box<dyn ottospace_sessions::SessionReader>)> = Vec::new();
        for (runner, _) in &seeking {
            if !readers.iter().any(|(seen, _)| seen == runner)
                && let Some(reader) = ottospace_sessions::reader(*runner, &session_dirs)
            {
                readers.push((*runner, reader));
            }
        }
        if readers.is_empty() || self.drawer.reading.is_some() {
            return;
        }
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        let dirs: Vec<(RunnerId, String)> =
            seeking.into_iter().filter_map(|(runner, cwd)| Some((runner, cwd.or_else(|| home.clone())?))).collect();
        let (send, read) = std::sync::mpsc::channel();
        self.drawer.reading = Some(read);
        std::thread::spawn(move || {
            let mut found = Vec::new();
            for (runner, reader) in readers {
                let dirs: Vec<&String> = dirs.iter().filter(|(of, _)| *of == runner).map(|(_, dir)| dir).collect();
                // A folder as the core names it: the runner writes where it ran with links resolved.
                let named = |cwd: &std::path::Path| {
                    dirs.iter()
                        .find(|dir| std::fs::canonicalize(dir).is_ok_and(|real| real == cwd))
                        .map(|dir| dir.to_string())
                };
                let roots: Vec<std::path::PathBuf> =
                    dirs.iter().filter_map(|dir| std::fs::canonicalize(dir).ok()).collect();
                let sessions = reader.sessions(&roots).unwrap_or_default();
                found.extend(sessions.into_iter().filter_map(|session| {
                    let (cwd, started) = (named(&session.cwd)?, session.started?);
                    Some(agents::Started { runner, id: session.id, cwd, started })
                }));
            }
            let _ = send.send(found);
        });
    }

    /// ⌘V while the conversation has the keyboard: the clipboard's text to the agent, bracketed
    /// if it asked for that. False: the conversation does not have the keyboard.
    pub(crate) fn talk_paste(&mut self, text: Option<String>) -> bool {
        let shown = self.model.talk.as_ref().filter(|_| self.model.mode() == Mode::Talk).map(|talk| &talk.task);
        let Some(agent) = shown.and_then(|task| self.drawer.agents.get(task)) else { return false };
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
            .map(|(subject, agent)| {
                let (cols, rows) = agent.terminal.grid_size();
                let terminal = serde_json::json!({
                    "model": agent.model, "ended": agent.terminal.exited(),
                    "cols": cols, "rows": rows, "lines": agent.terminal.lines(),
                });
                (key(subject), terminal)
            })
            .collect();
        let shown = self.model.talk.as_ref().is_some_and(|talk| talk.open);
        serde_json::json!({
            "body": shown.then(|| crate::rect(self.drawer.body.get())),
            "width": self.drawer.width,
            "agents": agents,
            "native": self.chats_snapshot(),
            "goes_to": self.model.talk_go(),
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
