//! Shell half of the native conversation: runs Claude Code through `ottospace-agents`
//! (`claude -p`, stream-json in and out), turns its events into the core's (`AgentEvents`),
//! carries out sends, answers and interrupts, and draws the transcript, the permission card
//! and the composer in the drawer. The core decides; this translates and draws. Everything the
//! model or a tool wrote is untrusted: drawn as text, never fetched, never run; a link opens
//! only on the user's click. See `docs/ai/agent-runners.md`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use gpui::{
    AnyElement, AnyView, Context, Div, ElementId, FontStyle, FontWeight, HighlightStyle, ImageSource, InteractiveText,
    KeyDownEvent, MouseButton, MouseDownEvent, ScrollHandle, ScrollWheelEvent, StrikethroughStyle, UnderlineStyle,
    Window, div, img, prelude::*, px, rgb,
};
use ottospace_agents::driver::Driver;
use ottospace_agents::{Answer, Event as Said, Input, Process, Ticket};
use ottospace_core::activity::Station;
use ottospace_core::agents::{self, Start};
use ottospace_core::ask::{Act, Form};
use ottospace_core::diff::{self, Change, Diff, Sign};
use ottospace_core::gist;
use ottospace_core::links::Target;
use ottospace_core::model::chat::{self, Key, Primary};
use ottospace_core::scrollbar::LATEST;
use ottospace_core::talk::Subject;
use ottospace_core::tasks::TaskId;
use ottospace_core::transcript::{
    By, Call, Entry, Event, Item, MAX_CARDS, NoteKind, Notice, Status, Text, View, clean,
};
use ottospace_core::{Action, Origin, theme, working};

use crate::highlight;
use crate::markdown::{self, Align, Block, Callout, Span};
use crate::scrollbar::{Axis, Fade, extent};
use crate::{Tooltip, Workspace};

/// Whose markdown is drawn, for its links and pictures: a conversation's (inside its task's
/// repository), or a file viewer's (inside the file's folder).
#[derive(Clone)]
pub(crate) enum Links {
    Chat(Subject),
    File { pane: ottospace_core::PaneId, dir: String },
}

impl Links {
    /// The conversation whose text this is: selectable there (`chat_select`). None for a file's.
    pub(crate) fn subject(&self) -> Option<&Subject> {
        match self {
            Links::Chat(subject) => Some(subject),
            Links::File { .. } => None,
        }
    }
}

/// A call's part shown open: what it was given, or what it gave back.
#[derive(Clone, Copy)]
enum Part {
    Input,
    Output,
}

/// A native agent: its process, the requests it waits on by our number, and when it started.
pub struct Native {
    driver: Driver,
    tickets: Vec<(u64, Ticket)>,
    next: u64,
    pub since: Instant,
    /// What the header names: the model, else the runner.
    pub model: String,
    /// The runner and the id it was started with, if the id was chosen here, and where it runs:
    /// to see, once it ends, whether its session was written (`session_missing`).
    pub session: Option<(ottospace_core::runner::RunnerId, String)>,
    pub cwd: Option<String>,
    /// Its task's pass, while it runs (`pass`): none for a session's agent, nor once it moved.
    pub(crate) pass: Option<crate::pass::Live>,
}

/// What the drawer keeps of a conversation's look, not its state: where it is scrolled, the
/// rows opened, the replies laid out.
#[derive(Default)]
pub struct Look {
    scroll: ScrollHandle,
    /// The user scrolled up: new text does not pull the view down.
    away: bool,
    open: HashSet<u64>,
    /// Each reply's blocks, by entry, with the length of text they were made from and the
    /// grammars' queries built by then (`syntax::built`): a fence's lexer colours, made again.
    parsed: HashMap<u64, ((usize, usize), Rc<Vec<Block>>)>,
    /// The user opened the ended agent's stderr (`Drawer::details`), or the lines not understood.
    details: bool,
    unread: bool,
    /// A question card's options, scrolled; the cursor last brought into view (ticket, tab, row);
    /// the contexts opened in full (ticket, question).
    ask_scroll: ScrollHandle,
    ask_seen: std::cell::Cell<Option<(u64, usize, usize)>>,
    ask_open: HashSet<(u64, usize)>,
    /// Text selected in it (`chat_select`).
    pub(crate) select: crate::chat_select::Selecting,
}

/// A question's context past this many lines folds behind "Show all".
const FOLD_LINES: usize = 6;

/// A question's markdown, parsed whole: the blocks before the question (context), and the
/// question itself, its last block. `None`: drawn as plain text, as it came (a line past what
/// markdown draws of one, so no word is cut; or no block at all).
pub(crate) fn question_blocks(text: &str) -> (Vec<Block>, Option<Block>) {
    let whole = text.lines().all(|line| line.chars().count() <= markdown::MAX_LINE);
    let mut blocks = if whole { markdown::parse(text, None) } else { Vec::new() };
    // Footnotes are drawn last but are not the question: they stay with its context.
    let at = blocks.iter().rposition(|block| !matches!(block, Block::Footnotes(_)));
    let asked = at.map(|at| blocks.remove(at));
    (blocks, asked)
}

/// About how many rows `text` takes as drawn in a card: a long line wraps.
fn rows_drawn(text: &str) -> usize {
    text.lines().map(|line| line.chars().count() / 60 + 1).sum()
}

/// A local image's thumbnail: being read, read, or not to be drawn.
pub enum Thumb {
    Reading(Receiver<Option<Arc<gpui::Image>>>),
    Ready(Arc<gpui::Image>),
    None,
}

/// An agent stopped for a switch of view: its process, until it is gone. Never two processes
/// on one session: the other view starts only after.
pub struct Stopping {
    terminal: Option<crate::terminal::Terminal>,
    native: Option<Native>,
    since: Instant,
    /// The TUI's job when its shell runs it (`Terminal::job`): it is gone only once that is.
    job: Option<i32>,
}

/// Sends `signal` to process group `group`. False: no process is in it.
fn signal_group(group: i32, signal: i32) -> bool {
    // SAFETY: plain integers.
    unsafe { libc::killpg(group, signal) == 0 }
}

/// How long a hung-up TUI has to end before it is killed.
const HANG_UP_MS: u128 = 3000;

impl Stopping {
    /// Hangs up on the TUI, kills the structured process's group (its tools with it).
    pub fn new(terminal: Option<crate::terminal::Terminal>, mut native: Option<Native>) -> Self {
        let job = terminal.as_ref().and_then(crate::terminal::Terminal::job);
        if let Some(terminal) = &terminal {
            terminal.signal(libc::SIGHUP);
        }
        job.map(|group| signal_group(group, libc::SIGHUP));
        if let Some(native) = &mut native {
            // Stopped, its pass goes now: its file too, though its stdout may stay open.
            native.pass = None;
            let _ = native.driver.kill();
        }
        Self { terminal, native, since: Instant::now(), job }
    }

    /// True once every process of it has exited. A TUI that ignores the hang-up is killed.
    fn gone(&mut self) -> bool {
        let late = self.since.elapsed().as_millis() > HANG_UP_MS;
        let terminal = self.terminal.as_ref().is_none_or(|terminal| {
            if late {
                terminal.signal(libc::SIGKILL);
            }
            terminal.exited()
        });
        let job = self.job.is_none_or(|group| !signal_group(group, if late { libc::SIGKILL } else { 0 }));
        let native = match &self.native {
            None => true,
            Some(native) => native.driver.events().try_iter().any(|said| matches!(said, Said::Exited { .. })),
        };
        if native {
            self.native = None;
        }
        terminal && job && self.native.is_none()
    }
}

/// A watched session's file, followed on a thread of its own: what it says, as the core's events.
pub struct Watch {
    subject: Subject,
    read: Receiver<Vec<Event>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

/// How often a watched session's file is read again, and how much of it at once.
const WATCH_MS: u64 = 1000;
const WATCH_BUDGET: u64 = 4 << 20;

impl Watch {
    /// Finds the session in its folder, then reads its file from the start and on as it grows.
    fn start(
        subject: Subject,
        runner: ottospace_core::runner::RunnerId,
        id: String,
        cwd: String,
        dirs: ottospace_sessions::ConfigDirs,
    ) -> Self {
        let (send, read) = std::sync::mpsc::channel();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let Some(reader) = ottospace_sessions::reader(runner, &dirs) else { return };
            let Ok(sessions) = reader.sessions(&[std::path::PathBuf::from(&cwd)]) else { return };
            let Some(session) = sessions.into_iter().find(|session| session.id == id) else { return };
            // Read from the start again: what an earlier watch built goes first.
            if send.send(vec![Event::WatchFromStart]).is_err() {
                return;
            }
            ottospace_mac::wake::post();
            let mut cursor = ottospace_sessions::Cursor::with_edits();
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(batch) = reader.read(&session, cursor.clone(), WATCH_BUDGET) else { return };
                cursor = batch.cursor;
                let events: Vec<Event> = batch.events.iter().flat_map(watched).collect();
                if !events.is_empty() {
                    if send.send(events).is_err() {
                        return;
                    }
                    ottospace_mac::wake::post();
                }
                if !batch.more {
                    std::thread::sleep(std::time::Duration::from_millis(WATCH_MS));
                }
            }
        });
        Self { subject, read, stop }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// What a session file's event shows in a watched transcript: labels only, never text.
fn watched(event: &ottospace_sessions::Event) -> Vec<Event> {
    use ottospace_sessions::What;
    match &event.what {
        What::UserMessage => vec![Event::MessageElsewhere],
        What::ModelStep { model, usage, window } => {
            let context = usage.map(|usage| usage.input.saturating_add(usage.output));
            let usage = Event::Usage { context, window: *window, cost_micro_usd: None };
            let started = model.clone().map(|model| Event::Started { session: None, model: Some(model), mode: None });
            started.into_iter().chain([usage]).collect()
        }
        What::ToolStart { id, tool, station, summary, diff, gist } => {
            let (tool, station, label, diff, gist) =
                (tool.clone(), *station, summary.clone(), diff.clone(), gist.clone());
            let detail = Text::default();
            vec![Event::ToolStart { id: id.clone(), sub: false, tool, station, label, detail, diff, gist }]
        }
        What::ToolEnd { id, failed, output, .. } => {
            vec![Event::ToolEnd { id: id.clone(), failed: *failed, output: output.clone() }]
        }
        What::Error { kind } => vec![Event::Error { kind: kind.clone(), retrying: false }],
        What::SubAgentStart { id, label } => {
            let (tool, label, station) = ("Agent".to_string(), label.clone(), Station::Agents);
            vec![Event::ToolStart {
                id: id.clone(),
                sub: true,
                tool,
                station,
                label,
                detail: Text::default(),
                diff: None,
                gist: None,
            }]
        }
        What::SubAgentEnd { id } => vec![Event::ToolEnd { id: id.clone(), failed: false, output: None }],
        What::Compaction { .. } => vec![Event::Compaction],
        What::ModelChange { to, .. } => vec![Event::Started { session: None, model: Some(to.clone()), mode: None }],
        What::Cwd { .. } => Vec::new(),
    }
}

/// The largest image file drawn as a thumbnail.
const MAX_THUMB: u64 = 8 << 20;
/// The most events taken from one agent per tick.
const BATCH: usize = 500;

impl Workspace {
    /// Starts a conversation's Claude Code in its structured mode: the argv from the agents
    /// crate's fixed table, through the login shell for its `PATH`, the prompt on stdin.
    /// Returns the new session's id, or why nothing started.
    pub(crate) fn start_native(
        &mut self,
        subject: &Subject,
        start: &Start,
        cwd: Option<&str>,
        brief: Option<&str>,
        command_text: Option<&str>,
    ) -> Result<Option<String>, String> {
        let (runner, begin, session, model, prompt) = match start {
            Start::New(launch) => {
                let id = crate::talk::new_uuid().ok_or("no random bytes for a session id")?;
                let model = launch.model.clone();
                let prompt = agents::prompt(launch.runner, &launch.title, brief);
                let chosen = launch.runner.spec().chooses_session().then(|| id.clone());
                (launch.runner, ottospace_agents::Start::New { id }, chosen, model, Some(prompt))
            }
            Start::Resume { runner, session } => {
                let own = subject.task().and_then(|id| self.model.tasks.get(id)).and_then(|task| task.repo.as_deref());
                if let Some(dir) = cwd.filter(|dir| Some(*dir) != own) {
                    self.in_checkout(dir)?;
                }
                (*runner, ottospace_agents::Start::Resume { id: session.clone() }, None, None, None)
            }
        };
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        let dir = cwd.or(home.as_deref()).filter(|dir| Path::new(dir).is_dir());
        let (argv, wire) =
            ottospace_agents::structured(runner, Path::new(dir.unwrap_or("/")), model.as_deref(), &begin)
                .ok_or("no structured command for this agent")?;
        let (script, argv, path) = crate::talk::shell_run(argv.program, &argv.args, command_text)?;
        // As the terminal runs it: the user's login shell finds the program, which gets its
        // arguments as positional parameters. The script is fixed text.
        let (shell, flags) = match std::env::var("OTTOSPACE_SHELL") {
            Ok(test) => (test, "-c"),
            // Interactive, as the terminal's: a `PATH` set in `.zshrc` finds the program too.
            Err(_) => (crate::terminal::login_shell(), "-lic"),
        };
        let mut command = Command::new(shell);
        command.args([flags, script, "ottospace-agent"]).args(&argv);
        command.envs(path);
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let pass = subject.task().map(|_| crate::pass::Live::mint()).transpose()?;
        if let Some(task) = subject.task() {
            command.env("OTTOSPACE_TASK", task.to_string());
        }
        // Its own, for its tool shells' `ottospace ctl`: the user's shells in the task's tab have none.
        if let Some(pass) = &pass {
            command.env(crate::pass::VAR, pass.file());
        }
        // An empty key is no key: passed on, the CLI could take it for one and fail.
        if std::env::var_os("ANTHROPIC_API_KEY").is_some_and(|key| key.is_empty()) {
            command.env_remove("ANTHROPIC_API_KEY");
        }
        let name = runner.name();
        let driver = Driver::spawn(command, wire).map_err(|err| format!("{name}: {err}"))?;
        if let Some(text) = prompt {
            driver.send(Input::Send { text, images: Vec::new() }).map_err(|err| format!("{name}: {err}"))?;
        }
        let model = model.unwrap_or_else(|| name.into());
        let given = session.clone().map(|id| (runner, id));
        let (tickets, since, cwd) = (Vec::new(), Instant::now(), dir.map(str::to_string));
        let native = Native { driver, tickets, next: 0, since, model, session: given, cwd, pass };
        self.drawer.native.insert(subject.clone(), native);
        Ok(session)
    }

    /// True if the conversation is drawn natively: its agent runs so; or no terminal of it does
    /// and its transcript is left, natively or watched.
    pub(crate) fn is_native(&self, subject: &Subject) -> bool {
        let kept = self
            .model
            .chat(subject)
            .is_some_and(|chat| chat.transcript.watched || self.model.talk_view(subject) == View::Native);
        self.drawer.native.contains_key(subject) || (kept && !self.drawer.agents.contains_key(subject))
    }

    /// A message, an answer or an interrupt the core asked for, to the agent.
    pub(crate) fn native_input(&mut self, subject: &Subject, input: NativeInput) {
        let Some(native) = self.drawer.native.get_mut(subject) else { return };
        let input = match input {
            NativeInput::Send(text) => Input::Send { text, images: Vec::new() },
            NativeInput::Interrupt => Input::Interrupt,
            NativeInput::Answer { ticket: n, allow, message } => {
                let answer = if allow { Answer::Allow } else { Answer::Deny { message } };
                let Some(ticket) = native.take(n) else { return };
                Input::Answer { ticket, answer }
            }
            NativeInput::Answers { ticket: n, answers } => {
                let Some(ticket) = native.take(n) else { return };
                Input::Answer { ticket, answer: Answer::Questions(answers) }
            }
        };
        if let Err(err) = native.driver.send(input) {
            self.broke(format!("claude: {err}"));
        }
    }

    /// A btw's question to the agent, beside its turn. `BtwFailed` if it could not be sent.
    pub(crate) fn side_question(
        &mut self,
        subject: &Subject,
        id: u64,
        question: &str,
        history: &[(String, String)],
    ) -> Option<Action> {
        let input = Input::SideQuestion { id, question: question.to_string(), history: history.to_vec() };
        let sent = self.drawer.native.get(subject).map(|native| native.driver.send(input));
        (!matches!(sent, Some(Ok(())))).then(|| Action::BtwFailed { task: subject.clone(), id })
    }

    /// Takes what each native agent said since the last tick to the core, and tells it of an
    /// agent that ended. Lays out the replies that changed.
    pub(crate) fn sync_native(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut news: Vec<(Subject, Vec<Event>)> = Vec::new();
        // What came of side questions: beside the transcript, never in it.
        let mut sides: Vec<Action> = Vec::new();
        let mut ended = Vec::new();
        let mut details = Vec::new();
        for (subject, native) in &mut self.drawer.native {
            // Its process gone, its pass goes: the file too, and the pass is no longer hidden.
            if native.reaped() {
                native.pass = None;
            }
            let mut events = Vec::new();
            while let Ok(said) = native.driver.events().try_recv() {
                let task = subject.clone();
                match said {
                    Said::SideAnswer { id, text, synthetic } => {
                        let plain = Text::new(&crate::markdown::plain_line(text.text()));
                        let text = Text::new(text.text());
                        sides.push(Action::BtwAnswered { task, id, text, plain, synthetic });
                        continue;
                    }
                    Said::SideFailed { id } => {
                        sides.push(Action::BtwFailed { task, id });
                        continue;
                    }
                    _ => {}
                }
                let mut kept = false;
                if let Said::Exited { stderr, .. } = &said {
                    ended.push(subject.clone());
                    // Its stderr may hold tokens: kept here for the user's click, never in the core.
                    let shown = ottospace_core::transcript::details(stderr.text());
                    kept = !shown.is_empty();
                    details.push((subject.clone(), shown));
                }
                events.extend(translate(native, said, kept));
                if events.len() >= BATCH {
                    break;
                }
            }
            if !events.is_empty() {
                news.push((subject.clone(), events));
            }
        }
        let changed = !news.is_empty() || !ended.is_empty() || !sides.is_empty();
        for (subject, shown) in details {
            self.drawer.looks.entry(subject.clone()).or_default().details = false;
            match shown.is_empty() {
                true => self.drawer.details.remove(&subject),
                false => self.drawer.details.insert(subject, shown),
            };
        }
        for (task, events) in news {
            self.dispatch(Action::AgentEvents { task, events, at: crate::now_ms() }, Origin::System, window, cx);
        }
        for side in sides {
            self.dispatch(side, Origin::System, window, cx);
        }
        for task in ended {
            // A session id it never wrote is not followed: the next click starts a new one.
            let missing = self.session_missing(&task);
            self.drawer.native.remove(&task);
            for action in missing.into_iter().chain([Action::AgentExited { task }]) {
                self.dispatch(action, Origin::System, window, cx);
            }
        }
        // A process stopped for a switch: the core hears it ended only once it has.
        let mut gone = Vec::new();
        self.drawer.stopping.retain_mut(|(subject, stopping)| {
            let done = stopping.gone();
            if done {
                gone.push(subject.clone());
            }
            !done
        });
        for task in gone {
            self.dispatch(Action::AgentExited { task }, Origin::System, window, cx);
        }
        let changed = self.sync_watch(window, cx) || changed;
        let changed = self.lay_out() || changed;
        if changed {
            cx.notify();
        }
    }

    /// Parses the replies whose text changed, and reads the local images they show. True if a
    /// picture was read, or a reply's fences gained their grammar's colours: drawn now.
    fn lay_out(&mut self) -> bool {
        let mut wanted: Vec<(String, Option<String>)> = Vec::new();
        let mut recoloured = false;
        let subjects: Vec<Subject> = self.drawer.looks.keys().cloned().collect();
        for subject in subjects.iter().chain(self.model.talk.as_ref().map(|talk| &talk.task)) {
            let Some(chat) = self.model.chat(subject) else { continue };
            let repo = self.repo_of(subject);
            let look = self.drawer.looks.entry(subject.clone()).or_default();
            let ids: HashSet<u64> = chat.transcript.items().map(|item| item.id).collect();
            look.parsed.retain(|id, _| ids.contains(id));
            for item in chat.transcript.items() {
                let Entry::Assistant { text, .. } = &item.entry else { continue };
                let made = (text.len(), crate::syntax::built());
                if look.parsed.get(&item.id).is_some_and(|(was, _)| *was == made) {
                    continue;
                }
                recoloured |= look.parsed.get(&item.id).is_some_and(|((len, _), _)| *len == text.len());
                let blocks = markdown::parse(text.text(), repo.as_deref());
                images(&blocks, &mut |path| wanted.push((path.to_string(), repo.clone())));
                look.parsed.insert(item.id, (made, Rc::new(blocks)));
            }
        }
        for (path, repo) in wanted {
            self.drawer.thumbs.entry(path.clone()).or_insert_with(|| read_thumb(path, repo));
        }
        let mut read_one = false;
        for thumb in self.drawer.thumbs.values_mut() {
            if let Thumb::Reading(read) = thumb
                && let Ok(image) = read.try_recv()
            {
                *thumb = image.map_or(Thumb::None, Thumb::Ready);
                read_one = true;
            }
        }
        read_one || recoloured
    }

    /// Follows the session the core says to watch, read-only, from its file: starts or ends the
    /// reader as the target changes, and gives the core what it read. True if it read anything.
    fn sync_watch(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let target = self.model.watch_target();
        let same = |watch: &Watch| target.as_ref().is_some_and(|(subject, ..)| *subject == watch.subject);
        if self.drawer.watch.as_ref().is_some_and(|watch| !same(watch)) {
            self.drawer.watch = None;
        }
        if let (None, Some((subject, runner, id, cwd))) = (&self.drawer.watch, target) {
            self.drawer.watch = crate::config::session_dirs().map(|dirs| Watch::start(subject, runner, id, cwd, dirs));
        }
        let Some(watch) = &self.drawer.watch else { return false };
        let events: Vec<Event> = watch.read.try_iter().flatten().collect();
        if events.is_empty() {
            return false;
        }
        let task = watch.subject.clone();
        self.dispatch(Action::AgentEvents { task, events, at: crate::now_ms() }, Origin::System, window, cx);
        true
    }

    /// The repository a conversation's links and images may be in, and its agent runs in.
    pub(crate) fn repo_of(&self, subject: &Subject) -> Option<String> {
        match subject {
            Subject::Task(id) => self.model.tasks.get(*id)?.repo.clone(),
            Subject::Session { runner, id } => self.model.session_repo(runner, id),
        }
    }

    /// A key while a native conversation has the keyboard. Only the user's: a client's key
    /// reaches nothing here, as in the terminal.
    pub(crate) fn chat_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let Some(chat) = self.model.shown_draft_chat() else { return };
        let subject = chat.subject.clone();
        let (key, mods) = (&event.keystroke.key, crate::mods(event.keystroke.modifiers));
        // ↵ held on a question card: one answer, not the next tab's and Send's too.
        if event.is_held && key == "enter" && chat.form().is_some() {
            return cx.stop_propagation();
        }
        let what = chat.key(key, mods);
        if self.transcript_escape(key, mods) {
            cx.notify();
            return cx.stop_propagation();
        }
        let key = what;
        // Typing goes to the composer: the keys are no longer the transcript's.
        if matches!(key, Key::Field | Key::Newline | Key::Act(Action::BtwOpen))
            && let Some(look) = self.drawer.looks.get_mut(&subject)
        {
            look.select.keys = false;
        }
        match key {
            Key::Pass => return,
            Key::Act(action) => drop(self.dispatch(action, self.input, window, cx)),
            Key::Newline => {
                let edit = ottospace_core::edit::Edit::Replace { range: None, text: "\n".into() };
                self.dispatch(Action::FieldEdit { edit }, self.input, window, cx);
            }
            Key::Field => {
                // Typed text comes by the input method; an editing key is the field's.
                if !self.field_key(event, window, cx) {
                    return;
                }
            }
            Key::Swallow => {}
        }
        cx.stop_propagation();
    }

    /// A click on a link: a web page in the default browser, a file of the repository (or of the
    /// viewed file's folder) revealed in Finder; with ⌘ (`cmd`), a text or markdown file opens in
    /// the viewer instead (`viewer::opens_on_click`). Only the user's click, and only what
    /// `links::target` gave; a file is checked again with its links resolved, so a symlink cannot
    /// lead out of the folder.
    fn open_link(&mut self, of: &Links, target: &Target, cmd: bool, window: &Window, cx: &mut Context<Self>) {
        if self.input != Origin::User {
            self.error = Some("only the user's click opens a link".into());
            return;
        }
        let test = std::env::var_os("OTTOSPACE_E2E").is_some();
        match target {
            Target::Web(url) => {
                self.drawer.opened.push("web");
                if !test {
                    cx.open_url(url);
                }
            }
            Target::File(path) => {
                let real = std::fs::canonicalize(path).ok();
                let root = match of {
                    Links::Chat(subject) => self.repo_of(subject),
                    Links::File { dir, .. } => Some(dir.clone()),
                };
                let root = root.and_then(|root| std::fs::canonicalize(root).ok());
                match real.zip(root).filter(|(real, root)| real.starts_with(root)) {
                    Some((real, _)) if cmd && ottospace_core::viewer::opens_on_click(&real.to_string_lossy()) => {
                        self.drawer.opened.push("viewer");
                        let path = real.to_string_lossy().into_owned();
                        // A viewer's link goes to the viewer it opened, beside it.
                        let (beside, reuse) = match of {
                            Links::File { pane, .. } => (Some(*pane), true),
                            Links::Chat(_) => (None, false),
                        };
                        let open =
                            Action::OpenFile { path, axis: ottospace_core::Axis::Right, beside, reuse, at: None };
                        self.dispatch(open, Origin::User, window, cx);
                    }
                    Some((real, _)) => {
                        self.drawer.opened.push("file");
                        if !test {
                            cx.reveal_path(&real);
                        }
                    }
                    None => self.error = Some("that file is not in the task's repository".into()),
                }
            }
        }
        cx.notify();
    }

    /// The wheel over the transcript: scrolled up, it stays; back at the bottom, it follows again.
    fn chat_wheel(&mut self, subject: &Subject, cx: &mut Context<Self>) {
        if let Some(look) = self.drawer.looks.get_mut(subject) {
            look.away = extent(&look.scroll, Axis::Down).beyond(LATEST).1;
        }
        cx.notify();
    }

    fn chat_latest(&mut self, subject: &Subject, cx: &mut Context<Self>) {
        if let Some(look) = self.drawer.looks.get_mut(subject) {
            look.away = false;
            look.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub(crate) fn toggle_row(&mut self, subject: &Subject, id: u64, cx: &mut Context<Self>) {
        let open = &mut self.drawer.looks.entry(subject.clone()).or_default().open;
        if !open.remove(&id) {
            open.insert(id);
        }
        cx.notify();
    }

    /// The conversation drawn natively: the transcript, the working row, the first card, the composer.
    pub(crate) fn chat_body(&self, subject: &Subject, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.model.chat(subject) else { return div().flex_1().into_any_element() };
        let t = &chat.transcript;
        let key = crate::talk::key(subject);
        self.drawer.texts.begin(subject);
        let look = self.drawer.looks.get(subject);
        let scroll = look.map(|look| look.scroll.clone()).unwrap_or_default();
        let away = look.is_some_and(|look| look.away);
        if !away {
            scroll.scroll_to_bottom();
        }
        let dropped = (t.dropped > 0)
            .then(|| muted(format!("{} earlier entries are not kept here; the session's file has them.", t.dropped)));
        let items = t.items().map(|item| self.chat_item(subject, &key, item, cx));
        let list = div()
            .id(ElementId::Name(format!("chat-scroll:{key}").into()))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .children(dropped)
            .children(items)
            .children(self.done_offer_line(subject, cx))
            .children((t.unread_lines() > 0).then(|| self.chat_unread(subject, t, cx)))
            .on_scroll_wheel(cx.listener({
                let subject = subject.clone();
                move |this, _: &ScrollWheelEvent, _, cx| this.chat_wheel(&subject, cx)
            }))
            // A press selects text, as in a text field; one elsewhere takes the keys from it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let subject = subject.clone();
                    move |this, event: &MouseDownEvent, _, cx| this.transcript_press(&subject, event, cx)
                }),
            )
            .on_mouse_down_out(cx.listener({
                let subject = subject.clone();
                move |this, _: &MouseDownEvent, _, _| this.transcript_leave(&subject)
            }))
            .child(self.transcript_listener(subject, cx));
        // A drag or a page of the bar leaves the latest, as a wheel does.
        let (this, who) = (cx.entity().downgrade(), subject.clone());
        let moved = move |cx: &mut gpui::App| drop(this.update(cx, |this, cx| this.chat_wheel(&who, cx)));
        let bar = self.bars.follow(
            &format!("chat:{key}"),
            Axis::Down,
            &scroll,
            // On its latest within `LATEST`, as `chat_wheel` says: no fade at its foot then.
            Fade { within: LATEST, ..Fade::to(theme::BACKGROUND) },
            !away,
            moved,
        );
        let list = div().relative().flex_1().min_h_0().min_w_0().flex().flex_col().child(list).child(bar);
        let latest = away.then(|| {
            let subject = subject.clone();
            self.talk_button("chat-latest", "↓ Jump to latest", "Back to the newest message")
                .on_click(cx.listener(move |this, _, _, cx| this.chat_latest(&subject, cx)))
        });
        let watching = (t.watched && !t.live).then(|| self.watch_banner(subject, cx));
        // As wide as the drawer, never wider: text wraps, code scrolls sideways.
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .children(watching)
            .child(list)
            .children(latest.map(|button| div().flex().justify_center().py_0p5().child(button)))
            .children(self.chat_working(subject, chat))
            .children(self.chat_card(subject, chat, cx))
            .children(self.chat_ended(subject, chat, cx))
            .child(self.chat_composer(subject, chat, cx))
            .into_any_element()
    }

    /// Lines the runner sent that no rule reads: counted, their kinds (labels) on a click.
    fn chat_unread(
        &self,
        subject: &Subject,
        t: &ottospace_core::transcript::Transcript,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.drawer.looks.get(subject).is_some_and(|look| look.unread);
        let n = t.unread_lines();
        // Diagnostics, not conversation: a small mark at the end, its words on hover and click.
        let tip: gpui::SharedString =
            format!("{n} line{} from the agent not understood", if n == 1 { "" } else { "s" }).into();
        let subject = subject.clone();
        let kinds = open.then(|| t.unread.iter().map(|(kind, n)| muted(format!("{} × {n}", clean(kind)))));
        div()
            .flex()
            .flex_col()
            .items_end()
            .child(
                div()
                    .id("chat-unread")
                    .relative()
                    .cursor_pointer()
                    .px_1()
                    .rounded_sm()
                    .text_color(gpui::Hsla::from(rgb(theme::MUTED)).opacity(0.7))
                    .hover(|s| s.bg(rgb(theme::SELECTED)))
                    .child(div().text_xs().child(format!("? {n}")))
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
                    .child(self.controls.mark("talk:chat-unread".to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let look = this.drawer.looks.entry(subject.clone()).or_default();
                        look.unread = !look.unread;
                        cx.notify();
                    })),
            )
            .children(kinds.into_iter().flatten())
    }

    /// Under a transcript whose agent ended: Show details, which opens the end of its stderr,
    /// cleaned and its tokens hidden. Only on the user's click: it may say what the model must not.
    fn chat_ended(&self, subject: &Subject, chat: &chat::Chat, cx: &mut Context<Self>) -> Option<Div> {
        let t = &chat.transcript;
        if t.live || t.watched {
            return None;
        }
        // Resume and Start are the composer's (`composer_button`): here only its details.
        let details = self.drawer.details.get(subject)?;
        let open = self.drawer.looks.get(subject).is_some_and(|look| look.details);
        let label = if open { "Hide details" } else { "Show details" };
        let who = subject.clone();
        let button = self
            .talk_button("chat-details", label, "What the agent's program printed last (stderr)")
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.input != Origin::User {
                    this.error = Some("only the user's click shows an agent's details".into());
                    return cx.notify();
                }
                let look = this.drawer.looks.entry(who.clone()).or_default();
                look.details = !look.details;
                cx.notify();
            }));
        let body = open.then(|| {
            div()
                .id("chat-details-text")
                .max_h(px(160.))
                .overflow_y_scroll()
                .mt_1()
                .p_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::RAISED))
                .children(details.lines().map(code_text))
        });
        let row = div().flex().items_center().gap_1().child(button);
        Some(div().px_3().py_1().flex().flex_col().child(row).children(body))
    }

    /// Over a watched transcript: it is read-only, and Take over goes on with the session here
    /// once it is idle (the user's click: the same as on its row).
    fn watch_banner(&self, subject: &Subject, cx: &mut Context<Self>) -> Div {
        let ready = self.model.take_over_ready(subject);
        let tip = if ready { "Go on with this session here" } else { "Possible once it is idle in the other terminal" };
        let subject = subject.clone();
        let take = self.talk_button("chat-take-over", "Take over", tip).when(!ready, |b| b.opacity(0.4)).on_click(
            cx.listener(move |this, _, window, cx| {
                if this.model.take_over_ready(&subject) {
                    this.dispatch(Action::TalkStart, this.input, window, cx);
                }
            }),
        );
        div()
            .px_3()
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child(div().flex_1().child("Another terminal runs it. Read-only: its calls and edits, never its words."))
            .child(take)
    }

    /// The header's chip on a watched conversation: it follows another process's file, a
    /// message at a time, not streamed. None on one ottospace runs.
    pub(crate) fn watch_chip(&self, subject: &Subject) -> Option<gpui::Stateful<Div>> {
        let t = &self.model.chat(subject)?.transcript;
        // Taken over in a terminal, its transcript stays as watched: the agent is ours now.
        if !t.watched || t.live || self.model.agent_runs(subject) {
            return None;
        }
        let colour = subject.task().and_then(|id| self.model.tasks.get(id)).map(|task| task.colour);
        let tip = "Another process holds this session: each reply shows here once that message is complete";
        Some(
            div()
                .id("talk-watching")
                .relative()
                .flex_none()
                .px_1p5()
                .h(px(18.))
                .flex()
                .items_center()
                .rounded_full()
                .border_1()
                .border_color(rgb(theme::identity(colour.flatten())))
                .bg(rgb(theme::SELECTED))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child("Watching · updates per message")
                .child(self.controls.mark("talk:talk-watching".to_string()))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into())))),
        )
    }

    fn chat_item(&self, subject: &Subject, key: &str, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let id = item.id;
        match &item.entry {
            Entry::User { text } => {
                let copy = text.text().to_string();
                let body = div()
                    .rounded_md()
                    .bg(rgb(theme::SELECTED))
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(div().text_xs().text_color(rgb(theme::MUTED)).child("You"))
                    .child(self.picked_text(Some(subject), &format!("msg:{id}"), text.text()));
                self.with_copy(key, id, body, copy.clone(), copy, cx)
            }
            Entry::Assistant { text, streaming } => {
                let blocks =
                    self.drawer.looks.get(subject).and_then(|look| look.parsed.get(&id)).map(|(_, b)| b.clone());
                let blocks = blocks.unwrap_or_else(|| Rc::new(markdown::parse(text.text(), None)));
                let plain = markdown::plain(&blocks);
                let (at, of) = (format!("{key}:{id}"), Links::Chat(subject.clone()));
                let body = div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .text_sm()
                    .children(blocks.iter().enumerate().map(|(n, block)| self.block(&of, &at, n, block, cx)))
                    .children(streaming.then(|| div().text_xs().text_color(rgb(theme::MUTED)).child("…")));
                self.with_copy(key, id, body, plain, text.text().to_string(), cx)
            }
            Entry::Tool { sub, tool, label, detail, state, diff, gist, output, .. } => {
                let (glyph, color) = match state {
                    Call::Running => ("●", theme::PRIMARY),
                    Call::Done => ("✓", theme::PICK),
                    Call::Failed => ("✕", theme::ERROR),
                };
                let open = self.drawer.looks.get(subject).is_some_and(|look| look.open.contains(&id));
                let row = match gist {
                    Some(gist) => {
                        let repo = self.repo_of(subject);
                        let line = gist.line(repo.as_deref());
                        let words = match gist.kind {
                            gist::Kind::Command => code_text(&line).truncate().text_color(rgb(theme::TEXT)),
                            _ => div().truncate().child(line),
                        };
                        let head = div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .gap_1p5()
                                    .min_w_0()
                                    .child(div().flex_none().child(gist.lead(tool)))
                                    .child(words.min_w_0()),
                            )
                            .children(gist.note().map(|note| div().truncate().opacity(0.75).child(note)));
                        // Open: the command whole and what it printed; another call, its detail.
                        let body = open.then(|| match gist.command() {
                            Some(command) => {
                                // As an agent's stderr is shown: cleaned, credential-shaped parts hidden.
                                let printed = output.as_ref().map(|output| {
                                    let printed = ottospace_core::transcript::details(output.text());
                                    // A tab drawn as four spaces: cut again to what was kept.
                                    ottospace_core::transcript::cut(&printed, gist::MAX_OUTPUT).to_string()
                                });
                                div()
                                    .id(ElementId::Name(format!("chat-row-body:{key}:{id}").into()))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(self.tool_part(subject, key, id, Part::Input, &command, cx))
                                    .children(
                                        printed.map(|out| self.tool_part(subject, key, id, Part::Output, &out, cx)),
                                    )
                            }
                            None => div()
                                .id(ElementId::Name(format!("chat-row-body:{key}:{id}").into()))
                                .child(self.tool_part(subject, key, id, Part::Input, detail.text(), cx)),
                        });
                        self.gist_row(subject, key, id, (glyph, color), head, body, cx)
                    }
                    None => {
                        let line = format!("{tool} · {label}");
                        self.row(subject, key, id, (glyph, color), line, detail.text(), cx)
                    }
                };
                // A sub-agent's calls sit under the row that started it.
                let row = if *sub { div().ml_4().child(row).into_any_element() } else { row };
                match diff {
                    Some(diff) => {
                        div().flex().flex_col().gap_1().child(row).child(self.diff_card(subject, key, id, diff, cx))
                    }
                    None => return row,
                }
                .into_any_element()
            }
            Entry::Note { note, text } => {
                // Untrusted words on one line, after fixed ones.
                let words = clean(text.text()).split_whitespace().collect::<Vec<_>>().join(" ");
                let lead = match note {
                    NoteKind::Subtask => "Started a subtask:".into(),
                    NoteKind::Ended { status: Status::Completed } => "Subtask done:".into(),
                    NoteKind::Ended { status: Status::Failed } => "Subtask failed:".into(),
                    NoteKind::Ended { status: Status::Stopped } => "Subtask stopped:".into(),
                    NoteKind::Ended { status: Status::Other } => "Subtask ended:".into(),
                    NoteKind::Notification => "Claude Code:".into(),
                    NoteKind::Answered { answered, of } if words.is_empty() => {
                        format!("You answered {answered} of {of}.")
                    }
                    NoteKind::Answered { answered, of } => format!("You answered {answered} of {of} ·"),
                };
                div()
                    .id(ElementId::Name(format!("chat-note:{key}:{id}").into()))
                    .flex()
                    .gap_1p5()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(div().flex_none().child(lead))
                    .child(self.picked_text(Some(subject), &format!("note:{id}"), &words).truncate().min_w_0())
                    .into_any_element()
            }
            Entry::AutoDenied { tool, label, reason, detail } => {
                let line = format!("Denied without asking ({reason}): {tool} · {label}");
                self.row(subject, key, id, ("✕", theme::ERROR), line, detail.text(), cx)
            }
            Entry::Notice(notice) => match divider(notice) {
                Some((word, tip)) => {
                    let rule = || div().flex_1().h(px(1.)).bg(rgb(theme::BORDER));
                    let line = div().id(ElementId::Name(format!("chat-divider:{key}:{id}").into()));
                    let line = line.flex().items_center().gap_2().text_xs().text_color(rgb(theme::MUTED));
                    let line = line.child(rule()).child(word).child(rule());
                    match tip {
                        Some(tip) => line
                            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into()))))
                            .into_any_element(),
                        None => line.into_any_element(),
                    }
                }
                None => self
                    .picked_text(Some(subject), &format!("notice:{id}"), &notice_text(notice))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .into_any_element(),
            },
        }
    }

    /// A call whose gist shows: its words as the core gave them, its body below while open. A
    /// click (or its sneak label) opens and closes it; nothing in it is a link.
    #[allow(clippy::too_many_arguments)]
    fn gist_row(
        &self,
        subject: &Subject,
        key: &str,
        id: u64,
        (glyph, color): (&str, u32),
        head: Div,
        body: Option<gpui::Stateful<Div>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let subject = subject.clone();
        let control = format!("chat-row:{key}:{id}");
        div()
            .id(ElementId::Name(control.clone().into()))
            .relative()
            .flex()
            .flex_col()
            .child(self.controls.mark(control.clone()))
            // Its head opens and closes it: a click in what it printed leaves it open.
            .child(
                div()
                    .id(ElementId::Name(format!("{control}:head").into()))
                    .flex()
                    .gap_1p5()
                    .min_w_0()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(div().flex_none().text_color(rgb(color)).child(glyph.to_string()))
                    .child(head.flex_1())
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                        if !crate::chat_select::dragged(e) {
                            this.toggle_row(&subject, id, cx)
                        }
                    })),
            )
            .children(body.map(|body| body.ml_4().mt_0p5()))
            .into_any_element()
    }

    /// A tool call as one compact row; a click opens its detail as plain text.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        subject: &Subject,
        key: &str,
        id: u64,
        (glyph, color): (&str, u32),
        line: String,
        detail: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.drawer.looks.get(subject).is_some_and(|look| look.open.contains(&id));
        let body = open.then(|| self.tool_part(subject, key, id, Part::Input, detail, cx).ml_4().mt_0p5());
        let who = subject.clone();
        let control = format!("chat-row:{key}:{id}");
        div()
            .id(ElementId::Name(control.clone().into()))
            .relative()
            .flex()
            .flex_col()
            .child(self.controls.mark(control.clone()))
            // Its head opens and closes it: a click or a drag in its input leaves it open.
            .child(
                div()
                    .id(ElementId::Name(format!("{control}:head").into()))
                    .flex()
                    .gap_1p5()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(div().text_color(rgb(color)).child(glyph.to_string()))
                    .child(div().truncate().child(clean(&line)))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                        if !crate::chat_select::dragged(e) {
                            this.toggle_row(&who, id, cx)
                        }
                    })),
            )
            .children(body)
            .into_any_element()
    }

    /// An open call's input (wrapped: JSON comes pretty-printed) or output (scrolls sideways, as
    /// code does), selectable, with Copy on hover.
    fn tool_part(&self, subject: &Subject, key: &str, id: u64, part: Part, text: &str, cx: &mut Context<Self>) -> Div {
        let (name, tip) = match part {
            Part::Input => ("in", "Copy the call's input (or select part of it and ⌘C)"),
            Part::Output => ("out", "Copy what it printed (or select part of it and ⌘C)"),
        };
        let shown = self.picked_text(Some(subject), &format!("{name}:{id}"), text);
        let shown = match part {
            Part::Input => shown
                .font_family(crate::terminal::font_family())
                .text_xs()
                .min_w_0()
                .text_color(rgb(theme::TEXT))
                .into_any_element(),
            Part::Output => div()
                .id(ElementId::Name(format!("chat-out:{key}:{id}").into()))
                .overflow_x_scroll()
                .child(code_style(shown))
                .into_any_element(),
        };
        let copy = clean(text);
        let word = self.word(
            format!("chat-io-copy:{key}:{id}:{name}"),
            ("Copy", tip),
            move |this, _, cx| this.copy_untrusted(copy.clone(), cx),
            cx,
        );
        let words = crate::words::words("chat-io", false).top(px(-2.)).child(word);
        div().group("chat-io").relative().min_w_0().child(shown).child(words)
    }

    /// An edit's change under its call: each file's path (a click reveals it, if `File::target`
    /// allows), then its lines, −/+ coloured. Plain monospaced text, as the core cut and cleaned it.
    fn diff_card(&self, subject: &Subject, key: &str, id: u64, diff: &Diff, cx: &mut Context<Self>) -> Div {
        let repo = self.repo_of(subject);
        let files = diff.files.iter().enumerate().map(|(n, file)| {
            let target = file.target(repo.as_deref());
            let path = div()
                .id(ElementId::Name(format!("chat-diff-path:{key}:{id}:{n}").into()))
                .min_w_0()
                .truncate()
                .text_color(rgb(theme::TEXT))
                .child(file.shown(repo.as_deref()));
            let path = match target {
                Some(target) => {
                    let of = Links::Chat(subject.clone());
                    path.cursor_pointer().hover(|s| s.text_color(rgb(theme::PRIMARY))).on_click(cx.listener(
                        move |this, e: &gpui::ClickEvent, window, cx| {
                            this.open_link(&of, &target, e.modifiers().platform, window, cx)
                        },
                    ))
                }
                None => path,
            };
            let what = match file.change {
                Change::Add => Some("new file"),
                Change::Delete => Some("deleted"),
                Change::Write => Some("written whole"),
                Change::Edit => None,
            };
            let to = file.to.as_ref().map(|to| format!("→ {}", diff::shown_path(to.text(), repo.as_deref())));
            let header = div()
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .py_0p5()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(path)
                .children(to.map(crate::cut_text))
                .children(what.or(file.all.then_some("every occurrence")).map(|what| div().flex_none().child(what)))
                .child(div().flex_1())
                .child(div().flex_none().text_color(rgb(theme::PICK)).child(format!("+{}", file.added)))
                .child(div().flex_none().text_color(rgb(theme::ERROR)).child(format!("−{}", file.removed)));
            let lines = file.lines.iter().map(|line| {
                let (sign, color) = match line.sign {
                    Sign::Added => ("+", theme::PICK),
                    Sign::Removed => ("−", theme::ERROR),
                    Sign::Context => (" ", theme::MUTED),
                    Sign::Hunk => ("", theme::BORDER_STRONG),
                };
                code_text(&format!("{sign} {}", line.text.text())).text_color(rgb(color))
            });
            let body = div()
                .id(ElementId::Name(format!("chat-diff:{key}:{id}:{n}").into()))
                .overflow_x_scroll()
                .px_2()
                .py_1()
                .children(lines);
            let more = (file.more > 0).then(|| muted(format!("… {} more lines", file.more)).px_2().pb_1());
            div()
                .flex()
                .flex_col()
                .when(n > 0, |d| d.border_t_1().border_color(rgb(theme::BORDER)))
                .child(header)
                .child(body)
                .children(more)
        });
        let more_files =
            (diff.more_files > 0).then(|| muted(format!("… {} more files", diff.more_files)).px_2().py_0p5());
        div()
            .ml_4()
            .rounded_md()
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::RAISED))
            .flex()
            .flex_col()
            .children(files)
            .children(more_files)
    }

    /// A message with its hover words: Copy (plain text) and Copy as Markdown.
    fn with_copy(
        &self,
        key: &str,
        id: u64,
        body: Div,
        plain: String,
        raw: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = self.word(
            format!("chat-copy:{key}:{id}"),
            ("Copy", "Copy as plain text (or select part of it and ⌘C)"),
            move |this, _, cx| this.copy_untrusted(plain.clone(), cx),
            cx,
        );
        let markdown = self.word(
            format!("chat-copy-md:{key}:{id}"),
            ("Copy as Markdown", "Copy the message as its author wrote it"),
            move |this, _, cx| this.copy_untrusted(raw.clone(), cx),
            cx,
        );
        let words = crate::words::words("chat-msg", false).top(px(-2.)).child(copy).child(markdown);
        div().group("chat-msg").relative().child(body).child(words).into_any_element()
    }

    /// One block of a reply, or of a viewed markdown file.
    pub(crate) fn block(&self, of: &Links, at: &str, n: usize, block: &Block, cx: &mut Context<Self>) -> AnyElement {
        let at = format!("{at}.{n}");
        let blocks = |blocks: &[Block], at: &str, cx: &mut Context<Self>| {
            let drawn: Vec<AnyElement> =
                blocks.iter().enumerate().map(|(i, block)| self.block(of, at, i, block, cx)).collect();
            div().flex_1().min_w_0().flex().flex_col().gap_1().children(drawn)
        };
        match block {
            Block::Para(spans) => self.spans(of, &at, spans, cx),
            Block::Heading { level, spans, .. } => {
                // h1 and h2 ruled under, as GitHub draws them; h6 quieter than text.
                let size = [22., 19., 16.5, 15., 14., 13.][usize::from(level.clamp(&1, &6) - 1)];
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(size))
                    .when(*level <= 2, |d| d.pb_1().border_b_1().border_color(rgb(theme::BORDER)))
                    .when(*level == 6, |d| d.text_color(rgb(theme::MUTED)))
                    .child(self.spans(of, &at, spans, cx))
                    .into_any_element()
            }
            Block::Code { label, shown, copy, more, marks } => {
                let copy = copy.clone();
                let button = self.word(
                    format!("chat-code-copy:{at}"),
                    match of.subject() {
                        Some(_) => ("Copy", "Copy the code (or select part of it and ⌘C)"),
                        None => ("Copy", "Copy the code"),
                    },
                    move |this, _, cx| this.copy_untrusted(copy.clone(), cx),
                    cx,
                );
                let header = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_0p5()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(label.clone().unwrap_or_default())
                    .child(button);
                let id = format!("chat-code:{at}");
                // Selectable as `picked_text` draws it, coloured where cleaning left the text as marked.
                let cleaned = clean(shown);
                let styles = if cleaned == *shown {
                    let style = |token| HighlightStyle {
                        color: Some(rgb(highlight::colour(token)).into()),
                        ..Default::default()
                    };
                    marks.iter().map(|(range, token)| (range.clone(), style(*token))).collect()
                } else {
                    Vec::new()
                };
                let (styled, note) = self.picked(of.subject(), &format!("code:{at}"), cleaned, styles);
                let code = code_style(div().relative().cursor_text().child(styled).children(note));
                let body = div()
                    .id(ElementId::Name(id.clone().into()))
                    .overflow_x_scroll()
                    .flex()
                    .child(code.flex_none().px_2().py_1());
                // A line past the edge fades there: the cue that it scrolls across.
                let body = self.bars.across(&id, body, theme::RAISED);
                let more = (*more > 0).then(|| muted(format!("… {more} more lines; Copy takes them all")).px_2());
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .bg(rgb(theme::RAISED))
                    .child(header)
                    .child(body)
                    .children(more)
                    .into_any_element()
            }
            Block::Quote { callout: None, blocks: inner } => div()
                .border_l_2()
                .border_color(rgb(theme::BORDER_STRONG))
                .pl_2()
                .text_color(rgb(theme::MUTED))
                .child(blocks(inner, &at, cx))
                .into_any_element(),
            Block::Quote { callout: Some(callout), blocks: inner } => {
                let (word, colour) = match callout {
                    Callout::Note => ("Note", theme::MARK),
                    Callout::Tip => ("Tip", theme::PICK),
                    Callout::Important => ("Important", theme::VISUAL),
                    Callout::Warning => ("Warning", theme::SNEAK),
                    Callout::Caution => ("Caution", theme::ERROR),
                };
                div()
                    .border_l_2()
                    .border_color(rgb(colour))
                    .pl_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(rgb(colour)).child(word))
                    .child(blocks(inner, &at, cx))
                    .into_any_element()
            }
            Block::List { start, items } => div()
                .flex()
                .flex_col()
                .gap_0p5()
                .children(items.iter().enumerate().map(|(i, item)| {
                    let mark = match (item.task, start) {
                        (Some(done), _) => task_box(done),
                        (None, Some(n)) => div().child(format!("{}.", n + i as u64)),
                        (None, None) => div().child("•"),
                    };
                    let at = format!("{at}.{i}");
                    div()
                        .flex()
                        .gap_1p5()
                        .child(div().flex_none().text_color(rgb(theme::MUTED)).child(mark))
                        .child(blocks(&item.blocks, &at, cx))
                }))
                .into_any_element(),
            Block::Table { align, widths, rows } => {
                let table = div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .rounded_md()
                    .text_xs()
                    .children(rows.iter().enumerate().map(|(r, row)| {
                        div()
                            .flex()
                            .when(r == 0, |d| d.font_weight(FontWeight::SEMIBOLD).bg(rgb(theme::RAISED)))
                            .when(r > 0, |d| d.border_t_1().border_color(rgb(theme::BORDER)))
                            .children(row.iter().enumerate().map(|(c, cell)| {
                                let at = format!("{at}.{r}.{c}");
                                // Its widest cell's characters, at about a character's width each.
                                let chars = widths.get(c).copied().unwrap_or(3) as f32;
                                let cell = div()
                                    .flex_none()
                                    .w(px(chars * TABLE_CHAR + 12.))
                                    .px_1p5()
                                    .py_0p5()
                                    .child(self.spans(of, &at, cell, cx));
                                match align.get(c).copied().unwrap_or_default() {
                                    Align::Left => cell,
                                    Align::Center => cell.text_center(),
                                    Align::Right => cell.text_right(),
                                }
                            }))
                    }));
                let id = format!("chat-table:{at}");
                let row = div().id(ElementId::Name(id.clone().into())).overflow_x_scroll().flex().child(table);
                self.bars.across(&id, row, theme::BACKGROUND).into_any_element()
            }
            Block::Image { path, alt } => match self.drawer.thumbs.get(path) {
                Some(Thumb::Ready(image)) => {
                    let (w, h) = crate::picture::size(&image.bytes)
                        .map_or(crate::picture::MAX, |(w, h)| crate::picture::shown(w, h));
                    img(ImageSource::Image(image.clone())).w(px(w)).h(px(h)).rounded_md().into_any_element()
                }
                _ => muted(format!("[image: {alt}]")).into_any_element(),
            },
            Block::Rule => div().h(px(1.)).bg(rgb(theme::BORDER)).into_any_element(),
            Block::Footnotes(notes) => div()
                .mt_2()
                .pt_2()
                .border_t_1()
                .border_color(rgb(theme::BORDER))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .flex()
                .flex_col()
                .gap_1()
                .children(notes.iter().map(|note| {
                    let at = format!("{at}.{}", note.n);
                    div().flex().gap_1p5().child(div().flex_none().child(format!("{}.", note.n))).child(blocks(
                        &note.blocks,
                        &at,
                        cx,
                    ))
                }))
                .into_any_element(),
            Block::Cut { blocks } => {
                muted(format!("… {blocks} more blocks not shown; Copy takes them")).into_any_element()
            }
        }
    }

    /// A run of spans as one text: styles by range, links clickable with their target on hover.
    /// In a viewer, a `#heading` or footnote link scrolls to it.
    fn spans(&self, of: &Links, at: &str, spans: &[Span], cx: &mut Context<Self>) -> AnyElement {
        let mut text = String::new();
        let (mut styles, mut links) = (Vec::new(), Vec::new());
        let viewer = matches!(of, Links::File { .. });
        for span in spans {
            // Cleaned as a selectable text is drawn (`picked_text`), each span on its own.
            let shown = clean(&span.text);
            let range = text.len()..text.len() + shown.len();
            text.push_str(&shown);
            let anchor = span.anchor.clone().filter(|_| viewer);
            let link = span.link.is_some() || anchor.is_some();
            let style = HighlightStyle {
                color: (link || span.style.mention).then(|| rgb(theme::PRIMARY).into()),
                font_weight: (span.style.strong || span.style.mention).then_some(FontWeight::BOLD),
                font_style: span.style.emphasis.then_some(FontStyle::Italic),
                background_color: (span.style.code || span.style.mention).then(|| rgb(theme::SELECTED).into()),
                underline: link.then(|| UnderlineStyle { thickness: px(1.), ..Default::default() }),
                strikethrough: span
                    .style
                    .strike
                    .then(|| StrikethroughStyle { thickness: px(1.), ..Default::default() }),
                ..Default::default()
            };
            if style != HighlightStyle::default() {
                styles.push((range.clone(), style));
            }
            match (&span.link, anchor) {
                (Some(link), _) => links.push((range, Goes::Link(link.clone()))),
                (None, Some(anchor)) => links.push((range, Goes::Anchor(anchor))),
                (None, None) => {}
            }
        }
        let (styled, note) = self.picked(of.subject(), at, text, styles);
        let host = div().relative().when(note.is_some(), |d| d.cursor_text());
        if links.is_empty() {
            return host.child(styled).children(note).into_any_element();
        }
        let ranges: Vec<_> = links.iter().map(|(range, _)| range.clone()).collect();
        let (targets, tips) = (Rc::new(links), ranges.clone());
        let (workspace, of) = (cx.entity().downgrade(), of.clone());
        let clicked = targets.clone();
        let text = InteractiveText::new(ElementId::Name(format!("chat-text:{at}").into()), styled)
            .on_click(ranges, move |n, window, cx| {
                let Some((_, goes)) = clicked.get(n) else { return };
                let cmd = window.modifiers().platform;
                let _ = workspace.update(cx, |this, cx| match (goes, &of) {
                    // A drag that selected the link's words opens nothing.
                    _ if of.subject().is_some_and(|subject| this.transcript_has_selection(subject)) => {}
                    (Goes::Link(link), _) => this.open_link(&of, &link.target, cmd, window, cx),
                    (Goes::Anchor(anchor), Links::File { pane, .. }) => this.viewer_anchor(*pane, anchor, cx),
                    (Goes::Anchor(_), Links::Chat(_)) => {}
                });
            })
            .tooltip(move |index, _, cx| {
                let n = tips.iter().position(|range| range.contains(&index))?;
                let tip = match targets.get(n).map(|(_, goes)| goes)? {
                    Goes::Link(link) => link.raw.clone(),
                    Goes::Anchor(anchor) => format!("#{anchor}"),
                };
                Some(AnyView::from(cx.new(|_| Tooltip(tip.into()))))
            });
        host.child(text).children(note).into_any_element()
    }

    /// While a turn runs: what it waits on, else a verb, swept while it works (`ui::shimmer`); and
    /// for how long. Stop is at the composer. Waiting on the user: still, as the card says why.
    fn chat_working(&self, subject: &Subject, chat: &chat::Chat) -> Option<Div> {
        let turn = chat.transcript.turn.as_ref()?;
        let (now, still) = (crate::now_ms(), self.working_still(subject));
        let secs = now.saturating_sub(turn.since) / 1000;
        let said: gpui::SharedString = working::line(turn.tool.as_deref(), turn.since, now, crate::dial::calm()).into();
        let (phase, next) = self.sweep.now(still).unzip();
        Some(
            div()
                .px_3()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .border_t_1()
                .border_color(rgb(theme::BORDER))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(crate::home::dot(theme::task(ottospace_core::tasks::Group::Running)))
                .child(crate::cut_over(said.clone(), crate::ui::shimmer(said, phase)))
                .child(div().flex_1().child(format!("· {}:{:02}", secs / 60, secs % 60)))
                .children(next),
        )
    }

    /// What the state says of the working line: the verb it shows (null: what it waits on, the
    /// turn's), and whether it sweeps. Null with no turn.
    fn working_snapshot(&self, subject: &Subject, chat: &chat::Chat) -> serde_json::Value {
        let Some(turn) = chat.transcript.turn.as_ref() else { return serde_json::Value::Null };
        let verb = working::waits_on(turn.tool.as_deref())
            .is_none()
            .then(|| working::verb(turn.since, crate::now_ms(), crate::dial::calm()));
        serde_json::json!({ "verb": verb, "shimmer": !self.working_still(subject) })
    }

    /// A working line drawn still: Reduce Motion, or something waits on the user (`working_sweeps`).
    fn working_still(&self, subject: &Subject) -> bool {
        crate::dial::calm() || !self.model.working_sweeps(subject)
    }

    /// The first permission request: what it would do, as the CLI asked it, one labelled line
    /// per field; the agent's own words apart; Allow and Deny. Allow waits for "Show all" on a
    /// cut request. ↵ does nothing here.
    pub(crate) fn chat_card(&self, subject: &Subject, chat: &chat::Chat, cx: &mut Context<Self>) -> Option<Div> {
        let card = chat.transcript.cards.front()?;
        if let Some(form) = chat.form() {
            return Some(self.ask_card(subject, chat, form, card.ticket, None, cx));
        }
        let full = card.expanded.then(|| self.drawer.native.get(subject).and_then(|n| n.full(card.ticket))).flatten();
        let payload = full.unwrap_or_else(|| card.detail.text().to_string());
        let lines = payload.lines().map(|line| {
            let (label, value) = line.split_once(": ").unwrap_or(("", line));
            div()
                .flex()
                .gap_1p5()
                .child(div().flex_none().text_color(rgb(theme::MUTED)).child(format!("{label}:")))
                .child(div().flex_1().min_w_0().child(clean(value)))
        });
        // A few lines as they are; more scroll in a box of fixed height (a scroll box grows to its cap).
        // Rows as drawn, about: a long line wraps.
        let rows = rows_drawn(&payload);
        let most = if card.expanded { 16 } else { 8 };
        let body = div()
            .id("chat-card-payload")
            .when(rows > most, |d| d.h(px(18. * most as f32)).overflow_y_scroll())
            .font_family(crate::terminal::font_family())
            .text_xs()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(lines);
        // Each button names the card drawn: one the CLI withdrew meanwhile takes its click along.
        let ticket = card.ticket;
        let cut = (card.cut && !card.expanded).then(|| {
            let show = self.talk_button("chat-show-all", "Show all (s)", "See all of it before you allow it").on_click(
                cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(Action::TalkExpand { ticket }, this.input, window, cx))
                }),
            );
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(theme::SNEAK))
                .child(format!("Cut: {} bytes in all. Allow opens once you have seen all of it.", card.len))
                .child(show)
        });
        let claim = (!card.claim.is_empty()).then(|| {
            div()
                .text_xs()
                .italic()
                .text_color(rgb(theme::MUTED))
                .child(format!("The agent says: {}", clean(card.claim.text())))
        });
        let allowable = card.allowable();
        let allow = self
            .talk_button("chat-allow", "Allow (a)", "Allow this one request, as shown")
            .when(allowable, |b| b.text_color(rgb(theme::PICK)))
            .when(!allowable, |b| b.opacity(0.4))
            .on_click(cx.listener(move |this, _, window, cx| {
                drop(this.dispatch(Action::TalkAnswer { allow: true, ticket }, this.input, window, cx))
            }));
        let deny = self
            .talk_button("chat-deny", "Deny (d)", "Deny it; the agent goes on without it")
            .text_color(rgb(theme::ERROR))
            .on_click(cx.listener(move |this, _, window, cx| {
                drop(this.dispatch(Action::TalkAnswer { allow: false, ticket }, this.input, window, cx))
            }));
        let note = self.talk_button("chat-deny-note", "Deny with a message (m)", "Tell the agent why").on_click(
            cx.listener(|this, _, window, cx| drop(this.dispatch(Action::TalkDenyNote, this.input, window, cx))),
        );
        let queued = chat.transcript.cards.len() - 1;
        Some(
            div()
                .mx_3()
                .mb_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::SNEAK))
                .bg(rgb(theme::RAISED))
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .text_sm()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Allow {}?", card.tool)))
                        .child(crate::cut_text(card.label.clone()).flex_1().text_color(rgb(theme::MUTED)))
                        .children((queued > 0).then(|| muted(format!("{queued} more waiting")))),
                )
                .child(body)
                .children(cut)
                .children(claim)
                .child(div().flex().gap_1().child(allow).child(deny).child(note)),
        )
    }

    /// A question card. Several questions show one at a time, each header a tab (✓ once
    /// answered), a thin bar a segment per question, and Review last: every answer, a row back to
    /// its question, Send. A question is markdown: its last paragraph the question, what comes
    /// before it context, folded past `FOLD_LINES`. The options scroll with the cursor's row in
    /// view; the foot stays. The card grows to the drawer's height.
    /// `host`: the Decisions card's task, when the card is drawn there (`decision_body`): its keys
    /// and clicks are `DecisionsAsk`, its controls `decision:…`. Else the drawer's.
    pub(crate) fn ask_card(
        &self,
        subject: &Subject,
        chat: &chat::Chat,
        form: &Form,
        ticket: u64,
        host: Option<TaskId>,
        cx: &mut Context<Self>,
    ) -> Div {
        let look = self.drawer.looks.get(subject);
        let (tab, row) = form.cursor;
        let n = form.questions.len();
        let tabs = (form.tabs() > 0).then(|| {
            let mut buttons: Vec<gpui::Stateful<Div>> = Vec::new();
            for at in 0..=n {
                buttons.push(self.ask_tab(form, at, ticket, host, cx));
            }
            let review = buttons.pop();
            // Many tabs scroll sideways: Review stays reachable.
            div()
                .id(ElementId::Name(format!("chat-ask-tabs:{ticket}").into()))
                .overflow_x_scroll()
                .flex_none()
                .flex()
                .gap_0p5()
                .px_1p5()
                .pt_1p5()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .children(buttons)
                .child(div().flex_1())
                .children(review)
        });
        // A segment per question: green answered, the accent on show, amber on Review if left, grey to come.
        let bar = (form.tabs() > 0).then(|| {
            let segment = |q: usize| {
                let (color, faint) = match () {
                    _ if form.answered(q) => (theme::PICK, false),
                    _ if q == tab => (theme::PRIMARY, true),
                    _ if form.review() => (theme::SNEAK, false),
                    _ => (theme::BORDER_STRONG, false),
                };
                div().flex_1().h(px(3.)).rounded_sm().bg(rgb(color)).when(faint, |d| d.opacity(0.55))
            };
            div().flex_none().flex().gap(px(3.)).px_3().pt_2().children((0..n).map(segment))
        });
        let scroll = look.map(|look| look.ask_scroll.clone()).unwrap_or_default();
        // The cursor's row in view, once each time it moves.
        if let Some(look) = look
            && look.ask_seen.replace(Some((ticket, tab, row))) != Some((ticket, tab, row))
        {
            scroll.scroll_to_item(row);
        }
        let rows: Vec<AnyElement> = match form.review() {
            true => (0..n).map(|q| self.review_row(form, q, ticket, host, cx)).collect(),
            false => (0..form.rows(tab)).map(|r| self.ask_row(form, tab, r, ticket, host, cx)).collect(),
        };
        let rows = div()
            .id(ElementId::Name(format!("chat-ask-rows:{ticket}:{tab}").into()))
            .flex_shrink()
            .min_h(px(40.))
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .flex()
            .flex_col()
            .pb_1()
            .children(rows);
        let question = (!form.review()).then(|| self.ask_question(subject, form, tab, ticket, host, cx));
        let primary = |label: &'static str, tip: &'static str, act: Act| {
            self.button_of(ask_controls(host), "chat-ask-send", label, tip)
                .bg(rgb(theme::PRIMARY))
                .text_color(rgb(theme::BACKGROUND))
                .font_weight(FontWeight::SEMIBOLD)
                .on_click(cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(ask_action(host, ticket, act), this.input, window, cx))
                }))
        };
        let (hint, send) = match () {
            _ if form.tabs() == 0 => {
                ("↵ send · Esc decline".to_string(), primary("Send", "Send the answer to the agent", Act::Submit))
            }
            _ if form.review() => (
                "a row goes back to it · ↵ send · Esc decline".into(),
                primary("Send", "Send these answers; one left goes as no answer", Act::Submit),
            ),
            _ => (
                format!("{} of {n} · ↵ next · ⇥ / ⇧⇥ between · Esc decline", tab + 1),
                primary("Next", "Answer this one and go on", Act::Submit),
            ),
        };
        // In Decisions Esc closes Decisions (its foot says so): the card's hint leaves it out.
        let hint = if host.is_some() { hint.replace(" · Esc decline", "") } else { hint };
        let decline = self
            .button_of(
                ask_controls(host),
                "chat-ask-decline",
                if form.tabs() > 0 { "Decline all" } else { "Decline" },
                "Answer nothing; the agent goes on",
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                drop(this.dispatch(ask_action(host, ticket, Act::Decline), this.input, window, cx))
            }));
        let queued = chat.transcript.cards.len() - 1;
        let foot = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .child(muted(hint).flex_1().min_w_0())
            .children((queued > 0).then(|| muted(format!("{queued} more waiting"))))
            .child(decline)
            .child(send);
        div()
            .mx_3()
            .mb_2()
            .flex_shrink()
            .min_h_0()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(rgb(theme::PRIMARY))
            .bg(rgb(theme::RAISED))
            .flex()
            .flex_col()
            .children(tabs)
            .children(bar)
            .children(question.into_iter().flatten())
            .child(rows)
            .child(foot)
    }

    /// A question's tab, or Review's (`at` = the questions' count): ✓ once answered, amber on
    /// Review if left, underlined while on show. A click shows it.
    fn ask_tab(
        &self,
        form: &Form,
        at: usize,
        ticket: u64,
        host: Option<TaskId>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let n = form.questions.len();
        let answered = form.answered(at);
        let label = if at < n { clean(&form.name(at)) } else { "Review".to_string() };
        let shown = form.cursor.0 == at;
        let color = match () {
            _ if shown => theme::TEXT,
            _ if answered => theme::PICK,
            _ if form.review() && at < n => theme::SNEAK,
            _ => theme::MUTED,
        };
        div()
            .id(ElementId::Name(format!("chat-ask-tab:{at}").into()))
            .relative()
            .px_2p5()
            .py_1p5()
            .text_xs()
            .cursor_pointer()
            .text_color(rgb(color))
            .when(shown, |d| d.border_b_2().border_color(rgb(theme::PRIMARY)))
            .child(if answered { format!("✓ {label}") } else { label })
            .child(self.controls.mark(format!("{}:chat-ask-tab:{at}", ask_controls(host))))
            .on_click(cx.listener(move |this, _, window, cx| {
                drop(this.dispatch(ask_action(host, ticket, Act::Show { tab: at }), this.input, window, cx))
            }))
    }

    /// A question as markdown: its last paragraph the question, in bold; what comes before it
    /// context, folded behind "Show all · N lines" past `FOLD_LINES`. With no tabs, its header too.
    /// Its parts are the card's own children: in a short drawer the shown context alone shrinks
    /// (with the options), so the question, the options and the foot keep their room.
    fn ask_question(
        &self,
        subject: &Subject,
        form: &Form,
        q: usize,
        ticket: u64,
        host: Option<TaskId>,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let question = &form.questions[q];
        let of = Links::Chat(subject.clone());
        let at = format!("ask:{ticket}:{q}");
        let text = question.question.text();
        let (blocks, asked) = question_blocks(text);
        let draw = |blocks: &[Block], from: usize, cx: &mut Context<Self>| {
            let drawn: Vec<AnyElement> =
                blocks.iter().enumerate().map(|(i, b)| self.block(&of, &at, from + i, b, cx)).collect();
            div().flex().flex_col().gap_1p5().children(drawn)
        };
        let header = (form.tabs() == 0 && !question.header.is_empty()).then(|| {
            div()
                .px_1p5()
                .rounded_sm()
                .bg(rgb(theme::SELECTED))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(clean(question.header.text()))
        });
        let open = self.drawer.looks.get(subject).is_some_and(|look| look.ask_open.contains(&(ticket, q)));
        let context = (!blocks.is_empty()).then(|| {
            let lines = rows_drawn(&markdown::plain(&blocks));
            let folds = lines > FOLD_LINES && !open;
            let body = draw(&blocks, 0, cx)
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .when(folds, |d| d.max_h(px(18. * FOLD_LINES as f32)).overflow_hidden());
            // Shown in full it scrolls in a box of its own: the question under it stays in view.
            let body = div()
                .id(ElementId::Name(format!("chat-ask-context:{ticket}:{q}").into()))
                .when(open, |d| d.max_h(px(18. * 2. * FOLD_LINES as f32)).overflow_y_scroll())
                .child(body);
            let who = subject.clone();
            let show = (lines > FOLD_LINES).then(|| {
                let label = if open { "Show less".to_string() } else { format!("Show all · {lines} lines") };
                div()
                    .id("chat-ask-show-all")
                    .relative()
                    .text_xs()
                    .cursor_pointer()
                    .text_color(rgb(theme::PRIMARY))
                    .child(label)
                    .child(self.controls.mark(format!("{}:chat-ask-show-all", ask_controls(host))))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_ask_context(&who, ticket, q, cx)))
            });
            (body, show)
        });
        let asked = match asked {
            Some(asked) => draw(std::slice::from_ref(&asked), blocks.len(), cx),
            None => div().child(clean(text)),
        };
        let asked = asked.text_sm().font_weight(FontWeight::SEMIBOLD);
        // The question in view whatever the context: a long one scrolls in a box of its own,
        // so the options and the foot stay in the card.
        let asked = div()
            .id(ElementId::Name(format!("chat-ask-asked:{ticket}:{q}").into()))
            .max_h(px(18. * (FOLD_LINES as f32 + 2.)))
            .overflow_y_scroll()
            .child(asked);
        // Spaced as a column of `gap_2` (Show all `gap_1` under its context), padded `px_3 pt_2 pb_1p5`.
        let part = |d: AnyElement| div().flex_none().px_3().pt_2().child(d);
        let mut parts: Vec<Div> =
            header.map(|chip| part(div().flex().child(chip).into_any_element())).into_iter().collect();
        if let Some((body, show)) = context {
            parts.push(match open {
                true => div().flex_shrink().min_h_0().flex().flex_col().px_3().pt_2().child(body),
                false => part(body.into_any_element()),
            });
            parts.extend(show.map(|show| div().flex_none().px_3().pt_1().child(show)));
        }
        parts.push(part(asked.into_any_element()));
        parts.extend(question.multi.then(|| part(muted("pick any".into()).into_any_element())));
        if let Some(last) = parts.pop() {
            parts.push(last.pb_1p5());
        }
        parts
    }

    /// Review's row for a question: its header and its answer, or "Not answered" in amber. A
    /// click goes back to it.
    fn review_row(
        &self,
        form: &Form,
        q: usize,
        ticket: u64,
        host: Option<TaskId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = clean(&form.name(q));
        let here = form.cursor.1 == q;
        let parts: Vec<String> = form.parts(q).iter().map(|part| clean(part)).collect();
        let answered = form.answered(q);
        let answer = match answered {
            true => div().flex_1().min_w_0().text_sm().child(parts.join(", ")),
            false => div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(rgb(theme::SNEAK))
                .child("Not answered · goes as “no answer”"),
        };
        let n = form.questions.len();
        let id = format!("chat-ask:{n}:{q}");
        div()
            .id(ElementId::Name(id.clone().into()))
            .relative()
            .flex()
            .gap_2p5()
            .px_3()
            .py_1p5()
            .cursor_pointer()
            .when(here, |d| d.bg(rgb(theme::SELECTED)))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(div().flex_none().w(px(80.)).truncate().text_sm().text_color(rgb(theme::MUTED)).child(name))
            .child(answer)
            .child(self.controls.mark(format!("{}:{id}", ask_controls(host))))
            .on_click(cx.listener(move |this, _, window, cx| {
                let act = Act::Row { question: n, row: q };
                drop(this.dispatch(ask_action(host, ticket, act), this.input, window, cx))
            }))
            .into_any_element()
    }

    /// "Show all" on a question's context, or less again. The drawer's look, not the card's state.
    pub(crate) fn toggle_ask_context(&mut self, subject: &Subject, ticket: u64, q: usize, cx: &mut Context<Self>) {
        // The user's card: a client's click changes nothing of it.
        if self.input != Origin::User {
            self.error = Some(chat::USERS_ALONE.into());
            return cx.notify();
        }
        let look = self.drawer.looks.entry(subject.clone()).or_default();
        if !look.ask_open.remove(&(ticket, q)) {
            look.ask_open.insert((ticket, q));
        }
        // The options' room changed: the cursor's row is brought into view again.
        look.ask_seen.set(None);
        cx.notify();
    }

    /// One row of a question: its mark, its number key, its label and description; "Other…"
    /// last, its words in a field while the cursor is on it. A click picks it.
    fn ask_row(
        &self,
        form: &Form,
        q: usize,
        row: usize,
        ticket: u64,
        host: Option<TaskId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let question = &form.questions[q];
        let (here, picked) = (form.cursor == (q, row), form.picked(q, row));
        let mark = match (question.multi, picked) {
            (true, true) => "☑",
            (true, false) => "☐",
            (false, true) => "●",
            (false, false) => "○",
        };
        let other = row == question.options.len();
        let words = form.other(q).map(|field| field.text()).unwrap_or_default();
        let body = match question.options.get(row) {
            Some(choice) => div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_sm().child(clean(choice.label.text())))
                .children((!choice.description.is_empty()).then(|| muted(clean(choice.description.text())))),
            None if here => {
                div().flex_1().min_w_0().text_sm().child(self.render_field_of("talk", "Other: type your answer", cx))
            }
            None if words.is_empty() => div().flex_1().text_sm().text_color(rgb(theme::MUTED)).child("Other…"),
            None => div().flex_1().min_w_0().text_sm().child(clean(words)),
        };
        let number = (row < 9).then(|| (row + 1).to_string()).unwrap_or_default();
        let id = format!("chat-ask:{q}:{row}");
        div()
            .id(ElementId::Name(id.clone().into()))
            .relative()
            .flex()
            .flex_none()
            .items_start()
            .gap_2()
            .px_3()
            .py_1p5()
            .border_l_2()
            .border_color(rgb(theme::RAISED))
            .cursor_pointer()
            .when(here, |d| d.bg(rgb(theme::SELECTED)).border_color(rgb(theme::PRIMARY)))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(
                div()
                    .flex_none()
                    .w(px(12.))
                    .text_sm()
                    .text_color(rgb(if picked { theme::PICK } else { theme::MUTED }))
                    .child(mark),
            )
            .child(div().flex_none().w(px(10.)).text_xs().text_color(rgb(theme::MUTED)).child(number))
            .child(body)
            // The press goes on to the drawer, which takes the keyboard: "Other…" is typed in next.
            .child(self.controls.mark(format!("{}:{id}", ask_controls(host))))
            // The field takes clicks in its own row: a click there places the cursor.
            .when(!(other && here), |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    let act = Act::Row { question: q, row };
                    drop(this.dispatch(ask_action(host, ticket, act), this.input, window, cx))
                }))
            })
            .into_any_element()
    }

    /// The composer: the draft, what ↵ does now, and at its right the one button the core
    /// names (`composer_button`): Stop while a turn runs, Resume or Start once the agent ended.
    pub(crate) fn chat_composer(&self, subject: &Subject, chat: &chat::Chat, cx: &mut Context<Self>) -> Div {
        let card = !chat.transcript.cards.is_empty();
        let ending = self.model.is_ending(subject);
        // The user's editor has the draft: the composer says so, and takes no typing.
        let editing = self.model.edits(subject).then(|| muted(self.editing_words()).text_sm().py_0p5());
        // Only what is not how every text box works: the card's buttons show their own keys.
        let (placeholder, hint): (&'static str, &'static str) = match () {
            _ if editing.is_some() && self.model.editor_keys() => ("", "save and quit puts it back here, unsent"),
            _ if editing.is_some() => ("", "⌃G back to the editor"),
            // ↵ meanwhile: sent once the session is gone on with, after the process is gone.
            _ if ending => ("Stopping…", ""),
            _ if card && chat.deny => ("Why deny it?", "↵ denies with this message · esc back to the request"),
            _ if card => ("", ""),
            _ if chat.transcript.turn.is_some() => {
                ("Write the next message", if chat.armed { "esc again stops" } else { "esc esc stops" })
            }
            _ if chat.queued.is_some() => ("Starting the agent…", "your message goes once it runs"),
            // Sending resumes the session.
            _ if !chat.transcript.live && self.model.shown_draft_chat().is_some() => ("Resume…", ""),
            _ if !chat.transcript.live => ("The agent ended", "it cannot be gone on with from here"),
            _ => ("Message", ""),
        };
        let open = chat.transcript.live || self.model.shown_draft_chat().is_some();
        let field =
            (open && (!card || chat.deny) && editing.is_none()).then(|| self.render_field_of("talk", placeholder, cx));
        let button = self.model.composer_button().and_then(|primary| match primary {
            Primary::Stop => Some(
                self.talk_button("chat-stop", "Stop", "Stop this turn; the session goes on (esc esc)")
                    .text_color(rgb(theme::ERROR))
                    .on_click(cx.listener(|this, _, window, cx| {
                        drop(this.dispatch(Action::TalkInterrupt, this.input, window, cx))
                    })),
            ),
            Primary::Resume | Primary::Start => self.start_button(cx),
        });
        let btw = self.btw_button(subject, cx);
        // A card waits and no turn to stop: nothing to write and nothing to say, so no empty strip.
        if field.is_none() && editing.is_none() && hint.is_empty() && button.is_none() && btw.is_none() {
            return div();
        }
        // The foot: the hint, and "btw" at its end.
        let foot = (!hint.is_empty() || btw.is_some()).then(|| {
            let hint = div().flex_1().min_w_0().text_xs().text_color(rgb(theme::MUTED)).child(hint);
            div().flex().items_center().gap_2().child(hint).children(btw)
        });
        let takes_drops = field.is_some();
        div()
            .flex_none()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .flex()
            .flex_col()
            .gap_1()
            // Files from Finder, while the message can be written: their paths go into it, quoted.
            .when(takes_drops, |composer| {
                composer.drag_over::<gpui::ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::SELECTED))).on_drop(
                    cx.listener(|this, dropped: &gpui::ExternalPaths, window, cx| {
                        let (paths, unreadable) = crate::dropped_names(dropped.paths());
                        drop(this.dispatch(Action::TalkDrop { paths, unreadable }, this.input, window, cx));
                    }),
                )
            })
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_sm().children(field).children(editing))
                    .children(button.map(|button| button.flex_none())),
            )
            .children(foot)
    }

    /// The header's Chat | Terminal switch, for a Claude Code
    /// conversation. Greyed while it cannot switch (a turn runs, a card waits).
    pub(crate) fn view_switch(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Div> {
        if !self.model.drawable(subject) {
            return None;
        }
        let shows = self.model.talk_view(subject);
        let able = self.model.talk_switchable();
        let button = |name: &'static str, label: &'static str, view: View| {
            let on = shows == view;
            self.talk_button(name, label, "Show this conversation natively, or in the CLI's own terminal")
                .when(on, |b| b.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                .when(!able && !on, |b| b.opacity(0.4))
                .on_click(cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(Action::TalkView { shows: view }, this.input, window, cx))
                }))
        };
        // The permission mode is on the title's hover, and at the foot when it acts unasked.
        Some(div().flex().items_center().gap_0p5().child(button("view-chat", "Chat", View::Native)).child(button(
            "view-terminal",
            "Terminal",
            View::Terminal,
        )))
    }

    /// What the state reply says of native conversations: structure, counts and labels, never
    /// what the model or a tool wrote.
    pub(crate) fn chats_snapshot(&self) -> serde_json::Value {
        // The conversations of the spaces on the row, hidden ones too, and the drawer's.
        // Every conversation of a space: a session's, each task of a workspace.
        let of = |space: &ottospace_core::model::Space| -> Vec<Subject> {
            let tasks = self.model.members(&space.of).into_iter().map(Subject::Task);
            let own = self.model.space_subject(&space.of).filter(|subject| subject.task().is_none());
            tasks.chain(own).collect()
        };
        let subjects = self.model.spaces().iter().flat_map(of);
        let mut subjects: Vec<Subject> = subjects.chain(self.model.talk.as_ref().map(|t| t.task.clone())).collect();
        subjects.dedup();
        let mut out = serde_json::Map::new();
        for subject in subjects {
            let Some(chat) = self.model.chat(&subject) else { continue };
            let look = self.drawer.looks.get(&subject);
            let blocks: Vec<serde_json::Value> = chat
                .transcript
                .items()
                .filter_map(|item| look?.parsed.get(&item.id).map(|(_, blocks)| shape(blocks)))
                .collect();
            // Each call's row says whether the user opened it.
            let mut transcript = serde_json::to_value(chat.transcript.summary()).unwrap_or_default();
            if let Some(entries) = transcript["entries"].as_array_mut() {
                for (entry, item) in entries.iter_mut().zip(chat.transcript.items()) {
                    if entry["kind"] == "tool" {
                        entry["expanded"] = look.is_some_and(|look| look.open.contains(&item.id)).into();
                    }
                }
            }
            out.insert(
                crate::talk::key(&subject),
                serde_json::json!({
                    "view": self.model.talk_view(&subject),
                    "native": self.drawer.native.contains_key(&subject),
                    "transcript": transcript,
                    "replies": blocks,
                    "draft_chars": chat.draft.text().chars().count(),
                    // The user's editor has its draft (⌃G).
                    "editing": self.model.edits(&subject),
                    "deny": chat.deny,
                    "armed": chat.armed,
                    "away": look.is_some_and(|look| look.away),
                    // That the ended agent left details, and whether they show: never what they say.
                    "details": self.drawer.details.contains_key(&subject),
                    "details_open": look.is_some_and(|look| look.details),
                    "unread_open": look.is_some_and(|look| look.unread),
                    "dial": self.dial_snapshot(&subject),
                    "working": self.working_snapshot(&subject, chat),
                    // How much of the transcript is selected, never its text; whether it has the keys.
                    "selection": self.selection_snapshot(&subject),
                    "btw": crate::btw::snapshot(&self.model, &subject),
                }),
            );
        }
        serde_json::json!({ "chats": out, "opened": self.drawer.opened })
    }
}

/// What the core asked to reach an agent.
pub enum NativeInput {
    Send(String),
    Answer { ticket: u64, allow: bool, message: Option<String> },
    Answers { ticket: u64, answers: Vec<ottospace_core::ask::Chosen> },
    Interrupt,
}

impl Native {
    /// Its process has exited.
    pub(crate) fn reaped(&self) -> bool {
        self.driver.reaped()
    }

    /// The request numbered `n`, to answer once.
    fn take(&mut self, n: u64) -> Option<Ticket> {
        let at = self.tickets.iter().position(|(of, _)| *of == n)?;
        Some(self.tickets.remove(at).1)
    }

    /// A request's whole payload, for "Show all".
    fn full(&self, ticket: u64) -> Option<String> {
        self.tickets.iter().find(|(n, _)| *n == ticket).map(|(_, t)| t.full().text().to_string())
    }
}

/// A runner's event as the core's, its requests numbered. `None`: nothing the drawer shows.
fn translate(native: &mut Native, said: Said, details: bool) -> Option<Event> {
    let text = |u: &ottospace_agents::Untrusted| Text::new(u.text());
    Some(match said {
        Said::Started { session, model, mode } => {
            if let Some(model) = &model {
                native.model = model.clone();
            }
            Event::Started { session, model, mode }
        }
        Said::TurnStarted => Event::TurnStarted,
        Said::TextDelta { text: t } => Event::TextDelta { text: text(&t) },
        Said::Text { text: t } => Event::Text { text: text(&t) },
        Said::ToolStart { id, parent, tool, station, label, detail, diff, gist } => {
            Event::ToolStart { id, sub: parent.is_some(), tool, station, label, detail: text(&detail), diff, gist }
        }
        Said::ToolEnd { id, failed, output } => {
            Event::ToolEnd { id, failed, output: Some(text(&output)).filter(|output| !output.is_empty()) }
        }
        Said::Subtask { id, description } => Event::Subtask { id, description: text(&description) },
        Said::SubtaskEnded { status, summary } => Event::SubtaskEnded { status, summary: text(&summary) },
        Said::Note { text: t } => Event::Note { text: text(&t) },
        Said::DeniedBy { tool, by } => Event::DeniedBy { tool, by },
        Said::ModelFallback { from, to } => Event::ModelFallback { from, to },
        // Past `MAX_CARDS` waiting, denied at once: no request is left waiting unseen.
        Said::Permission { ticket, tool, label, detail, .. } if native.tickets.len() >= MAX_CARDS => {
            let answer = Answer::Deny { message: Some("Too many requests at once: ask again.".into()) };
            let _ = native.driver.send(Input::Answer { ticket, answer });
            let reason = "too many requests at once".to_string();
            Event::AutoDenied { tool, label, reason, detail: text(&detail) }
        }
        Said::Permission { ticket, tool, label, detail, len, cut, claim, questions, .. } => {
            native.next += 1;
            native.tickets.push((native.next, ticket));
            let (detail, claim) = (text(&detail), text(&claim));
            Event::Permission { ticket: native.next, tool, label, detail, len, cut, claim, questions }
        }
        Said::PermissionClosed { ticket } => {
            let at = native.tickets.iter().position(|(_, t)| *t == ticket)?;
            let (n, _) = native.tickets.remove(at);
            Event::PermissionClosed { ticket: n }
        }
        Said::AutoDenied { tool, label, reason, detail } => {
            Event::AutoDenied { tool, label, reason: reason.into(), detail: text(&detail) }
        }
        Said::Usage { context, window, cost_micro_usd } => Event::Usage { context, window, cost_micro_usd },
        Said::Compaction => Event::Compaction,
        Said::TurnFinished { outcome } => Event::TurnFinished { outcome },
        Said::Error { kind, retrying } => Event::Error { kind, retrying },
        // A side question's: `sync_native` takes them aside; one started is nothing to show.
        Said::SideStarted { .. } | Said::SideAnswer { .. } | Said::SideFailed { .. } => return None,
        // Its stderr may hold tokens: the core hears only that there is some.
        Said::Exited { code, signal, .. } => Event::Exited { code, signal, details },
        // A hook of the user's that starts: the dial's Hooks station. Its other lines are quiet.
        Said::Unknown { kind } if kind == "system.hook_started" => Event::Hook,
        Said::Unknown { kind } if ottospace_agents::is_quiet(&kind) => return None,
        Said::Unknown { kind } => Event::Unknown { kind },
    })
}

/// Reads a local image for a thumbnail, off the main thread: inside the repository once its
/// links are resolved, a regular file of at most `MAX_THUMB`, a PNG, JPEG, GIF or WebP by its
/// first bytes. Never a URL: nothing is fetched.
pub(crate) fn read_thumb(path: String, repo: Option<String>) -> Thumb {
    let (send, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let image = (|| {
            let real = std::fs::canonicalize(&path).ok()?;
            let root = std::fs::canonicalize(repo?).ok()?;
            if !real.starts_with(&root) || std::fs::metadata(&real).ok()?.len() > MAX_THUMB {
                return None;
            }
            let image = ottospace_agents::Image::load(&real).ok()?;
            let format = match image.media {
                ottospace_agents::Media::Png => gpui::ImageFormat::Png,
                ottospace_agents::Media::Jpeg => gpui::ImageFormat::Jpeg,
                ottospace_agents::Media::Gif => gpui::ImageFormat::Gif,
                ottospace_agents::Media::Webp => gpui::ImageFormat::Webp,
            };
            Some(Arc::new(gpui::Image::from_bytes(format, image.bytes)))
        })();
        let _ = send.send(image);
        ottospace_mac::wake::post();
    });
    Thumb::Reading(read)
}

/// The local images among `blocks`.
pub(crate) fn images(blocks: &[Block], each: &mut impl FnMut(&str)) {
    for block in blocks {
        if let Block::Image { path, .. } = block {
            each(path)
        }
        markdown::inner(block).into_iter().for_each(|blocks| images(blocks, each));
    }
}

/// A reply's structure for the state: how many blocks of each kind (and list items with a task
/// box, `task`), and links. No text.
pub(crate) fn shape(blocks: &[Block]) -> serde_json::Value {
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    fn count(blocks: &[Block], counts: &mut std::collections::BTreeMap<&str, usize>) {
        for block in blocks {
            if let Block::List { items, .. } = block {
                let tasks = items.iter().filter(|item| item.task.is_some()).count();
                if tasks > 0 {
                    *counts.entry("task").or_default() += tasks;
                }
            }
            markdown::inner(block).into_iter().for_each(|blocks| count(blocks, counts));
            *counts.entry(markdown::kind(block)).or_default() += 1;
        }
    }
    count(blocks, &mut counts);
    let links = markdown::links_of(blocks);
    let web = links.iter().filter(|link| matches!(link.target, Target::Web(_))).count();
    serde_json::json!({ "blocks": counts, "links": { "web": web, "file": links.len() - web } })
}

/// A notice drawn as a thin rule with a word, not a sentence: the user's stop, and a resume
/// with the turns above it. The word, and how it ended, for the hover.
fn divider(notice: &Notice) -> Option<(&'static str, Option<String>)> {
    match notice {
        Notice::Ended { by_user: true, code, signal, .. } => Some(("stopped", how(*code, *signal))),
        Notice::Resumed { here: true, ended: Some(end) } if end.by_user => {
            Some(("stopped · resumed", how(end.code, end.signal)))
        }
        Notice::Resumed { here: true, ended: Some(end) } => {
            let how = how(end.code, end.signal).map_or(String::new(), |how| format!(" ({how})"));
            Some(("ended · resumed", Some(format!("The agent ended{how}"))))
        }
        Notice::Resumed { here: true, ended: None } => Some(("resumed", None)),
        _ => None,
    }
}

/// How an agent ended, when its program said: `exit 1`, `signal 9`.
fn how(code: Option<i32>, signal: Option<i32>) -> Option<String> {
    match (code, signal) {
        (_, Some(signal)) => Some(format!("signal {signal}")),
        (Some(code), None) => Some(format!("exit {code}")),
        (None, None) => None,
    }
}

/// The fixed words a notice says.
fn notice_text(notice: &Notice) -> String {
    match notice {
        Notice::Interrupted => "Stopped. The session goes on.".into(),
        Notice::Failed { kind } => format!("The agent reported: {kind}. The turn failed."),
        Notice::Error { kind, retrying: true } => format!("{kind}: trying again."),
        Notice::Error { kind, .. } => format!("The agent reported: {kind}."),
        Notice::Compacted => "The context was compacted.".into(),
        Notice::Allowed { tool, label } => format!("You allowed {tool} · {label}."),
        Notice::Denied { tool, label } => format!("You denied {tool} · {label}."),
        Notice::Declined => "You declined to answer.".into(),
        Notice::DeniedBy { tool, by } => {
            let by = match by {
                By::Settings => " by your settings",
                By::Mode => " by the permission mode",
                By::AutoMode => " by auto mode",
                By::Hook => " by a hook",
                By::Folder => ": outside the folders it may use",
                By::Safety => " by a safety check",
                By::Sandbox => " by the sandbox",
                By::Nobody => ": nobody could be asked",
                By::Other => "",
            };
            format!("Denied: {tool}{by}.")
        }
        Notice::ModelFallback { from: Some(from), to } => format!("The model changed: {from} → {to}."),
        Notice::ModelFallback { from: None, to } => format!("The model changed to {to}."),
        Notice::Resumed { here: true, .. } => "Resumed.".into(),
        Notice::Resumed { here: false, .. } => {
            "Session resumed. Earlier turns are in its file: Terminal shows them.".into()
        }
        Notice::MessageElsewhere => "A message, in the other terminal.".into(),
        Notice::MessageBefore => "A message. Its words are in the session's file: Terminal shows them.".into(),
        // ottospace's own record of what the task did in an app pane (agent mode).
        Notice::AppUsed { what, app, pane, refused: None } => format!("The task {} {app} (pane {pane}).", what.verb()),
        Notice::AppUsed { what, app, pane, refused: Some(why) } => {
            format!("Refused: the task {} {app} (pane {pane}): {why}", what.verb())
        }
        Notice::Ended { by_user: true, .. } => "Stopped.".into(),
        Notice::Ended { code, signal, replied, .. } => {
            let how = how(*code, *signal).map_or(String::new(), |how| format!(" ({how})"));
            match replied {
                true => format!("The agent ended{how}."),
                false => format!("The agent ended before replying{how}. Terminal view may show why."),
            }
        }
    }
}

fn muted(text: String) -> Div {
    div().text_xs().text_color(rgb(theme::MUTED)).child(text)
}

/// Plain text in the terminal's font, lines kept, not wrapped.
fn code_text(text: &str) -> Div {
    code_style(div()).child(text.to_string())
}

/// `code_text`'s look on a div of its own (a selectable text's).
fn code_style(d: Div) -> Div {
    d.font_family(crate::terminal::font_family()).text_xs().whitespace_nowrap()
}

/// A key or click on a question card: the drawer's `TalkAsk`, or the Decisions card's.
pub(crate) fn ask_action(host: Option<TaskId>, ticket: u64, act: Act) -> Action {
    match host {
        Some(task) => Action::DecisionsAsk { task, ticket, act },
        None => Action::TalkAsk { ticket, act },
    }
}

/// Whose a question card's controls are, for their ids: the drawer's or the Decisions card's.
fn ask_controls(host: Option<TaskId>) -> &'static str {
    if host.is_some() { "decision" } else { "talk" }
}

/// A task item's box, ticked when done.
fn task_box(done: bool) -> Div {
    let tick = div().text_size(px(9.)).text_color(rgb(theme::BACKGROUND)).child("✓");
    div()
        .mt(px(4.))
        .size(px(12.))
        .rounded_sm()
        .border_1()
        .border_color(rgb(if done { theme::PRIMARY } else { theme::BORDER_STRONG }))
        .when(done, |d| d.bg(rgb(theme::PRIMARY)))
        .flex()
        .items_center()
        .justify_center()
        .children(done.then_some(tick))
}

/// What a link in a run of text does: opens, or (in a viewer) scrolls to a heading or footnote.
enum Goes {
    Link(markdown::Link),
    Anchor(String),
}

/// About how wide a table's character is drawn, in points (`text_xs`).
const TABLE_CHAR: f32 = 7.6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_with_a_footnote_is_still_its_last_block() {
        let (context, asked) = question_blocks("Some context.\n\nUse option A?[^1]\n\n[^1]: A keeps the cache.");
        assert_eq!(asked.map(|asked| markdown::plain(&[asked])).as_deref(), Some("Use option A?[1]"));
        assert!(matches!(&context[..], [Block::Para(_), Block::Footnotes(_)]), "{context:#?}");
    }
}
