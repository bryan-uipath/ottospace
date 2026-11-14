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
    KeyDownEvent, ScrollHandle, ScrollWheelEvent, SharedString, StrikethroughStyle, StyledText, UnderlineStyle, Window,
    div, img, prelude::*, px, rgb,
};
use ottospace_agents::driver::Driver;
use ottospace_agents::{Answer, Event as Said, Input, Process, Ticket};
use ottospace_core::agents::{self, Start};
use ottospace_core::diff::{self, Change, Diff, Sign};
use ottospace_core::links::Target;
use ottospace_core::model::chat::{self, Key};
use ottospace_core::talk::Subject;
use ottospace_core::transcript::{Call, Entry, Event, Item, MAX_CARDS, Notice, Text, View, clean};
use ottospace_core::{Action, Origin, theme};

use crate::markdown::{self, Block, Span};
use crate::{Tooltip, Workspace};

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
}

/// What the drawer keeps of a conversation's look, not its state: where it is scrolled, the
/// rows opened, the replies laid out.
#[derive(Default)]
pub struct Look {
    scroll: ScrollHandle,
    /// The user scrolled up: new text does not pull the view down.
    away: bool,
    open: HashSet<u64>,
    /// Each reply's blocks, by entry, with the length of text they were made from.
    parsed: HashMap<u64, (usize, Rc<Vec<Block>>)>,
    /// The user opened the ended agent's stderr (`Drawer::details`), or the lines not understood.
    details: bool,
    unread: bool,
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
}

/// How long a hung-up TUI has to end before it is killed.
const HANG_UP_MS: u128 = 3000;

impl Stopping {
    /// Hangs up on the TUI, kills the structured process's group (its tools with it).
    pub fn new(terminal: Option<crate::terminal::Terminal>, mut native: Option<Native>) -> Self {
        if let Some(terminal) = &terminal {
            terminal.signal(libc::SIGHUP);
        }
        if let Some(native) = &mut native {
            let _ = native.driver.kill();
        }
        Self { terminal, native, since: Instant::now() }
    }

    /// True once every process of it has exited. A TUI that ignores the hang-up is killed.
    fn gone(&mut self) -> bool {
        let terminal = self.terminal.as_ref().is_none_or(|terminal| {
            if self.since.elapsed().as_millis() > HANG_UP_MS {
                terminal.signal(libc::SIGKILL);
            }
            terminal.exited()
        });
        let native = match &self.native {
            None => true,
            Some(native) => native.driver.events().try_iter().any(|said| matches!(said, Said::Exited { .. })),
        };
        if native {
            self.native = None;
        }
        terminal && self.native.is_none()
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
            let mut cursor = ottospace_sessions::Cursor::with_edits();
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(batch) = reader.read(&session, cursor.clone(), WATCH_BUDGET) else { return };
                cursor = batch.cursor;
                let events: Vec<Event> = batch.events.iter().flat_map(watched).collect();
                if !events.is_empty() && send.send(events).is_err() {
                    return;
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
        What::ToolStart { id, tool, summary, diff, .. } => {
            let (tool, label, diff) = (tool.clone(), summary.clone(), diff.clone());
            vec![Event::ToolStart { id: id.clone(), sub: false, tool, label, detail: Text::default(), diff }]
        }
        What::ToolEnd { id, failed, .. } => vec![Event::ToolEnd { id: id.clone(), failed: *failed }],
        What::Error { kind } => vec![Event::Error { kind: kind.clone(), retrying: false }],
        What::SubAgentStart { id, label } => {
            let (tool, label) = ("Agent".to_string(), label.clone());
            vec![Event::ToolStart { id: id.clone(), sub: true, tool, label, detail: Text::default(), diff: None }]
        }
        What::SubAgentEnd { id } => vec![Event::ToolEnd { id: id.clone(), failed: false }],
        What::Compaction { .. } => vec![Event::Compaction],
        What::ModelChange { to, .. } => vec![Event::Started { session: None, model: Some(to.clone()), mode: None }],
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
    ) -> Result<Option<String>, String> {
        let (runner, begin, session, model, prompt) = match start {
            Start::New(launch) => {
                let id = crate::talk::new_uuid().ok_or("no random bytes for a session id")?;
                let model = launch.model.clone();
                let prompt = agents::prompt(&launch.title, brief);
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
        let program = crate::talk::program(argv.program)?;
        // As the terminal runs it: the user's login shell finds the program, which gets its
        // arguments as positional parameters. The script is fixed text.
        let (shell, flags) = match std::env::var("OTTOSPACE_SHELL") {
            Ok(test) => (test, "-c"),
            // Interactive, as the terminal's: a `PATH` set in `.zshrc` finds the program too.
            Err(_) => (crate::terminal::login_shell(), "-lic"),
        };
        let mut command = Command::new(shell);
        command.args([flags, "exec \"$@\"", "ottospace-agent", &program]).args(&argv.args);
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        if let Some(task) = subject.task() {
            command.env("OTTOSPACE_TASK", task.to_string());
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
        let native = Native { driver, tickets, next: 0, since, model, session: given, cwd };
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
            NativeInput::Answer { ticket, allow, message } => {
                let Some(at) = native.tickets.iter().position(|(n, _)| *n == ticket) else { return };
                let (_, ticket) = native.tickets.remove(at);
                let answer = if allow { Answer::Allow } else { Answer::Deny { message } };
                Input::Answer { ticket, answer }
            }
        };
        if let Err(err) = native.driver.send(input) {
            self.error = Some(format!("claude: {err}"));
        }
    }

    /// Takes what each native agent said since the last tick to the core, and tells it of an
    /// agent that ended. Lays out the replies that changed.
    pub(crate) fn sync_native(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut news: Vec<(Subject, Vec<Event>)> = Vec::new();
        let mut ended = Vec::new();
        let mut details = Vec::new();
        for (subject, native) in &mut self.drawer.native {
            let mut events = Vec::new();
            while let Ok(said) = native.driver.events().try_recv() {
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
        let changed = !news.is_empty() || !ended.is_empty();
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
        self.lay_out();
        if changed {
            cx.notify();
        }
    }

    /// Parses the replies whose text changed, and reads the local images they show.
    fn lay_out(&mut self) {
        let mut wanted: Vec<(String, Option<String>)> = Vec::new();
        let subjects: Vec<Subject> = self.drawer.looks.keys().cloned().collect();
        for subject in subjects.iter().chain(self.model.talk.as_ref().map(|talk| &talk.task)) {
            let Some(chat) = self.model.chat(subject) else { continue };
            let repo = self.repo_of(subject);
            let look = self.drawer.looks.entry(subject.clone()).or_default();
            let ids: HashSet<u64> = chat.transcript.items().map(|item| item.id).collect();
            look.parsed.retain(|id, _| ids.contains(id));
            for item in chat.transcript.items() {
                let Entry::Assistant { text, .. } = &item.entry else { continue };
                if look.parsed.get(&item.id).is_some_and(|(len, _)| *len == text.len()) {
                    continue;
                }
                let blocks = markdown::parse(text.text(), repo.as_deref());
                images(&blocks, &mut |path| wanted.push((path.to_string(), repo.clone())));
                look.parsed.insert(item.id, (text.len(), Rc::new(blocks)));
            }
        }
        for (path, repo) in wanted {
            self.drawer.thumbs.entry(path.clone()).or_insert_with(|| read_thumb(path, repo));
        }
        for thumb in self.drawer.thumbs.values_mut() {
            if let Thumb::Reading(read) = thumb
                && let Ok(image) = read.try_recv()
            {
                *thumb = image.map_or(Thumb::None, Thumb::Ready);
            }
        }
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

    /// The repository a conversation's links and images may be in.
    fn repo_of(&self, subject: &Subject) -> Option<String> {
        match subject {
            Subject::Task(id) => self.model.tasks.get(*id)?.repo.clone(),
            Subject::Session { runner, id } => self.model.session_repo(runner, id),
        }
    }

    /// A key while a native conversation has the keyboard. Only the user's: a client's key
    /// reaches nothing here, as in the terminal.
    pub(crate) fn chat_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let Some(chat) = self.model.shown_chat() else { return };
        let card = chat.transcript.cards.front().map(|card| card.ticket).filter(|_| !chat.deny);
        let (key, mods) = (&event.keystroke.key, crate::mods(event.keystroke.modifiers));
        match chat::key(key, mods, card) {
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

    /// A click on a link: a web page in the default browser, a file of the repository revealed
    /// in Finder. Only the user's click, and only what `links::target` gave; a file is checked
    /// again with its links resolved, so a symlink cannot lead out of the repository.
    fn open_link(&mut self, subject: &Subject, target: &Target, cx: &mut Context<Self>) {
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
                let root = self.repo_of(subject).and_then(|repo| std::fs::canonicalize(repo).ok());
                match real.zip(root).filter(|(real, root)| real.starts_with(root)) {
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

    /// Copies what a model or a tool wrote: to the clipboard, never into the state reply's
    /// `copied` (a client must not read it there). A test run's clipboard is `copied` itself.
    fn copy_untrusted(&mut self, text: String, cx: &mut Context<Self>) {
        match self.system_clipboard {
            true => cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)),
            false => self.copied = Some(text),
        }
        cx.notify();
    }

    /// The wheel over the transcript: scrolled up, it stays; back at the bottom, it follows again.
    fn chat_wheel(&mut self, subject: &Subject, cx: &mut Context<Self>) {
        if let Some(look) = self.drawer.looks.get_mut(subject) {
            let (offset, max) = (look.scroll.offset().y, look.scroll.max_offset().height);
            look.away = -offset < max - px(4.);
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

    fn toggle_row(&mut self, subject: &Subject, id: u64, cx: &mut Context<Self>) {
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
            .children((t.unread_lines() > 0).then(|| self.chat_unread(subject, t, cx)))
            .on_scroll_wheel(cx.listener({
                let subject = subject.clone();
                move |this, _: &ScrollWheelEvent, _, cx| this.chat_wheel(&subject, cx)
            }));
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
            .children(self.chat_working(chat, cx))
            .children(self.chat_card(subject, chat, cx))
            .children(self.chat_ended(subject, chat, cx))
            .child(self.chat_composer(chat, cx))
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
        let line = format!(
            "{n} line{} from the agent not understood {}",
            if n == 1 { "" } else { "s" },
            if open { "▾" } else { "▸" }
        );
        let subject = subject.clone();
        let kinds = open.then(|| t.unread.iter().map(|(kind, n)| muted(format!("{} × {n}", clean(kind))).ml_4()));
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("chat-unread")
                    .relative()
                    .cursor_pointer()
                    .child(muted(line))
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
        Some(div().px_3().py_1().flex().flex_col().child(div().flex().child(button)).children(body))
    }

    /// Over a watched transcript: it is read-only, and Take over goes on with the session here
    /// once it is idle (the user's click: the same as on its row).
    fn watch_banner(&self, subject: &Subject, cx: &mut Context<Self>) -> Div {
        let ready = self.model.take_over_ready(subject);
        let tip = if ready { "Go on with this session here" } else { "Possible once it is idle in the other terminal" };
        let subject = subject.clone();
        let take = self.talk_button("chat-take-over", "Take over", tip).when(!ready, |b| b.opacity(0.4)).on_click(
            cx.listener(move |this, _, window, cx| {
                let action = match &subject {
                    Subject::Task(id) => Action::TaskTalk { id: *id },
                    Subject::Session { runner, id } => Action::SessionOpen { runner: runner.clone(), id: id.clone() },
                };
                if this.model.take_over_ready(&subject) {
                    this.dispatch(action, this.input, window, cx);
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
                let shown = clean(text.text());
                let copy = text.text().to_string();
                let body = div()
                    .rounded_md()
                    .bg(rgb(theme::SELECTED))
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(div().text_xs().text_color(rgb(theme::MUTED)).child("You"))
                    .child(shown);
                self.with_copy(key, id, body, copy.clone(), copy, cx)
            }
            Entry::Assistant { text, streaming } => {
                let blocks =
                    self.drawer.looks.get(subject).and_then(|look| look.parsed.get(&id)).map(|(_, b)| b.clone());
                let blocks = blocks.unwrap_or_else(|| Rc::new(markdown::parse(text.text(), None)));
                let plain = markdown::plain(&blocks);
                let at = format!("{key}:{id}");
                let body = div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .text_sm()
                    .children(blocks.iter().enumerate().map(|(n, block)| self.block(subject, &at, n, block, cx)))
                    .children(streaming.then(|| div().text_xs().text_color(rgb(theme::MUTED)).child("…")));
                self.with_copy(key, id, body, plain, text.text().to_string(), cx)
            }
            Entry::Tool { sub, tool, label, detail, state, diff, .. } => {
                let (glyph, color) = match state {
                    Call::Running => ("●", theme::PRIMARY),
                    Call::Done => ("✓", theme::PICK),
                    Call::Failed => ("✕", theme::ERROR),
                };
                let line = format!("{}{tool} · {label}", if *sub { "↳ " } else { "" });
                let row = self.row(subject, key, id, (glyph, color), line, detail, cx);
                match diff {
                    Some(diff) => {
                        div().flex().flex_col().gap_1().child(row).child(self.diff_card(subject, key, id, diff, cx))
                    }
                    None => return row,
                }
                .into_any_element()
            }
            Entry::AutoDenied { tool, label, reason, detail } => {
                let line = format!("Denied without asking ({reason}): {tool} · {label}");
                self.row(subject, key, id, ("✕", theme::ERROR), line, detail, cx)
            }
            Entry::Notice(notice) => muted(notice_text(notice)).into_any_element(),
        }
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
        detail: &Text,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.drawer.looks.get(subject).is_some_and(|look| look.open.contains(&id));
        let subject = subject.clone();
        let control = format!("chat-row:{key}:{id}");
        div()
            .id(ElementId::Name(control.clone().into()))
            .relative()
            .flex()
            .flex_col()
            .cursor_pointer()
            .child(self.controls.mark(control))
            .child(
                div()
                    .flex()
                    .gap_1p5()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(div().text_color(rgb(color)).child(glyph.to_string()))
                    .child(div().truncate().child(clean(&line))),
            )
            .children(open.then(|| code_text(&clean(detail.text())).ml_4().mt_0p5()))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_row(&subject, id, cx)))
            .into_any_element()
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
                    let subject = subject.clone();
                    path.cursor_pointer()
                        .hover(|s| s.text_color(rgb(theme::PRIMARY)))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_link(&subject, &target, cx)))
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
                .children(to.map(|to| div().min_w_0().truncate().child(to)))
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
            ("Copy", "Copy as plain text"),
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

    /// One block of a reply.
    fn block(&self, subject: &Subject, at: &str, n: usize, block: &Block, cx: &mut Context<Self>) -> AnyElement {
        let at = format!("{at}.{n}");
        match block {
            Block::Para(spans) => self.spans(subject, &at, spans, cx),
            Block::Heading(spans) => {
                div().font_weight(FontWeight::SEMIBOLD).child(self.spans(subject, &at, spans, cx)).into_any_element()
            }
            Block::Code { label, shown, copy, more } => {
                let copy = copy.clone();
                let button = self.word(
                    format!("chat-code-copy:{at}"),
                    ("Copy", "Copy the code"),
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
                    .child(label.clone().unwrap_or_else(|| "code".into()))
                    .child(button);
                let body = div()
                    .id(ElementId::Name(format!("chat-code:{at}").into()))
                    .overflow_x_scroll()
                    .child(code_text(shown).px_2().py_1());
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
            Block::Quote(inner) => div()
                .border_l_2()
                .border_color(rgb(theme::BORDER_STRONG))
                .pl_2()
                .text_color(rgb(theme::MUTED))
                .flex()
                .flex_col()
                .gap_1()
                .children(inner.iter().enumerate().map(|(i, block)| self.block(subject, &at, i, block, cx)))
                .into_any_element(),
            Block::List { start, items } => div()
                .flex()
                .flex_col()
                .gap_0p5()
                .children(items.iter().enumerate().map(|(i, item)| {
                    let mark = start.map_or("•".to_string(), |n| format!("{}.", n + i as u64));
                    let at = format!("{at}.{i}");
                    div().flex().gap_1p5().child(div().flex_none().text_color(rgb(theme::MUTED)).child(mark)).child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(item.iter().enumerate().map(|(j, b)| self.block(subject, &at, j, b, cx))),
                    )
                }))
                .into_any_element(),
            Block::Table(rows) => div()
                .flex()
                .flex_col()
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
                            div().flex_1().min_w_0().px_1p5().py_0p5().child(self.spans(subject, &at, cell, cx))
                        }))
                }))
                .into_any_element(),
            Block::Image { path } => match self.drawer.thumbs.get(path) {
                Some(Thumb::Ready(image)) => img(ImageSource::Image(image.clone()))
                    .max_w(px(320.))
                    .max_h(px(200.))
                    .rounded_md()
                    .into_any_element(),
                _ => muted("[image not shown]".into()).into_any_element(),
            },
            Block::Rule => div().h(px(1.)).bg(rgb(theme::BORDER)).into_any_element(),
            Block::Cut { blocks } => {
                muted(format!("… {blocks} more blocks not shown; Copy takes them")).into_any_element()
            }
        }
    }

    /// A run of spans as one text: styles by range, links clickable with their target on hover.
    fn spans(&self, subject: &Subject, at: &str, spans: &[Span], cx: &mut Context<Self>) -> AnyElement {
        let mut text = String::new();
        let (mut styles, mut links) = (Vec::new(), Vec::new());
        for span in spans {
            let range = text.len()..text.len() + span.text.len();
            text.push_str(&span.text);
            let link = span.link.is_some();
            let style = HighlightStyle {
                color: link.then(|| rgb(theme::PRIMARY).into()),
                font_weight: span.style.strong.then_some(FontWeight::BOLD),
                font_style: span.style.emphasis.then_some(FontStyle::Italic),
                background_color: span.style.code.then(|| rgb(theme::SELECTED).into()),
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
            if let Some(link) = &span.link {
                links.push((range, link.clone()));
            }
        }
        let styled = StyledText::new(SharedString::from(text)).with_highlights(styles);
        if links.is_empty() {
            return div().child(styled).into_any_element();
        }
        let ranges: Vec<_> = links.iter().map(|(range, _)| range.clone()).collect();
        let (targets, tips) = (Rc::new(links), ranges.clone());
        let (workspace, subject) = (cx.entity().downgrade(), subject.clone());
        let clicked = targets.clone();
        InteractiveText::new(ElementId::Name(format!("chat-text:{at}").into()), styled)
            .on_click(ranges, move |n, _, cx| {
                let Some((_, link)) = clicked.get(n) else { return };
                let target = link.target.clone();
                let _ = workspace.update(cx, |this, cx| this.open_link(&subject, &target, cx));
            })
            .tooltip(move |index, _, cx| {
                let n = tips.iter().position(|range| range.contains(&index))?;
                let tip = targets.get(n).map(|(_, link)| link.raw.clone())?;
                Some(AnyView::from(cx.new(|_| Tooltip(tip.into()))))
            })
            .into_any_element()
    }

    /// While a turn runs: what it waits on and for how long. Steady; Stop ends the turn.
    fn chat_working(&self, chat: &chat::Chat, cx: &mut Context<Self>) -> Option<Div> {
        let turn = chat.transcript.turn.as_ref()?;
        let secs = crate::now_ms().saturating_sub(turn.since) / 1000;
        let doing = turn.tool.clone().unwrap_or_else(|| "thinking".into());
        let hint = if chat.armed { "esc again stops" } else { "esc esc stops" };
        let stop = self
            .talk_button("chat-stop", "Stop", "Stop this turn; the session goes on")
            .text_color(rgb(theme::ERROR))
            .on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::TalkInterrupt, this.input, window, cx))),
            );
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
                .child(div().flex_1().min_w_0().truncate().child(format!(
                    "Working · {} · {}:{:02}",
                    clean(&doing),
                    secs / 60,
                    secs % 60
                )))
                .child(hint)
                .child(stop),
        )
    }

    /// The first permission request: what it would do, as the CLI asked it, one labelled line
    /// per field; the agent's own words apart; Allow and Deny. Allow waits for "Show all" on a
    /// cut request. ↵ does nothing here.
    fn chat_card(&self, subject: &Subject, chat: &chat::Chat, cx: &mut Context<Self>) -> Option<Div> {
        let card = chat.transcript.cards.front()?;
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
        let rows: usize = payload.lines().map(|line| line.chars().count() / 60 + 1).sum();
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
                        .child(div().flex_1().truncate().text_color(rgb(theme::MUTED)).child(card.label.clone()))
                        .children((queued > 0).then(|| muted(format!("{queued} more waiting")))),
                )
                .child(body)
                .children(cut)
                .children(claim)
                .child(div().flex().gap_1().child(allow).child(deny).child(note)),
        )
    }

    /// The composer: the draft, what ↵ does now.
    fn chat_composer(&self, chat: &chat::Chat, cx: &mut Context<Self>) -> Div {
        let card = !chat.transcript.cards.is_empty();
        let (placeholder, hint): (&'static str, &'static str) = match () {
            _ if card && chat.deny => ("Why deny it?", "↵ denies with this message · esc back to the request"),
            _ if card => ("A request waits above", "a allow · d deny · s show all · m deny with a message"),
            _ if chat.transcript.turn.is_some() => {
                ("Write the next message", "↵ sends once the turn ends · esc esc stops")
            }
            _ if !chat.transcript.live => ("The agent ended", "a click on its task goes on with its session"),
            _ => ("Message the agent", "↵ send · ⇧↵ new line · esc leave"),
        };
        let field = (!card || chat.deny).then(|| self.render_field_of("talk", placeholder, cx));
        div()
            .flex_none()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .flex()
            .flex_col()
            .gap_1()
            .children(field.map(|field| div().text_sm().child(field)))
            .child(div().text_xs().text_color(rgb(theme::MUTED)).child(hint))
    }

    /// The header's Chat | Terminal switch and the permission mode, for a Claude Code
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
        let mode = self.model.chat(subject).and_then(|chat| chat.transcript.mode.clone());
        Some(
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .children(mode.map(|mode| muted(format!("mode: {mode}")).pr_1()))
                .child(button("view-chat", "Chat", View::Native))
                .child(button("view-terminal", "Terminal", View::Terminal)),
        )
    }

    /// What the state reply says of native conversations: structure, counts and labels, never
    /// what the model or a tool wrote.
    pub(crate) fn chats_snapshot(&self) -> serde_json::Value {
        let subjects = self.model.rail().into_iter().map(|cell| cell.task);
        let subjects: Vec<Subject> = subjects.chain(self.model.talk.as_ref().map(|t| t.task.clone())).collect();
        let mut out = serde_json::Map::new();
        for subject in subjects {
            let Some(chat) = self.model.chat(&subject) else { continue };
            let look = self.drawer.looks.get(&subject);
            let blocks: Vec<serde_json::Value> = chat
                .transcript
                .items()
                .filter_map(|item| look?.parsed.get(&item.id).map(|(_, blocks)| shape(blocks)))
                .collect();
            out.insert(
                crate::talk::key(&subject),
                serde_json::json!({
                    "view": self.model.talk_view(&subject),
                    "native": self.drawer.native.contains_key(&subject),
                    "transcript": chat.transcript.summary(),
                    "replies": blocks,
                    "draft_chars": chat.draft.text().chars().count(),
                    "deny": chat.deny,
                    "armed": chat.armed,
                    "away": look.is_some_and(|look| look.away),
                    // That the ended agent left details, and whether they show: never what they say.
                    "details": self.drawer.details.contains_key(&subject),
                    "details_open": look.is_some_and(|look| look.details),
                    "unread_open": look.is_some_and(|look| look.unread),
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
    Interrupt,
}

impl Native {
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
        Said::ToolStart { id, parent, tool, label, detail, diff, .. } => {
            Event::ToolStart { id, sub: parent.is_some(), tool, label, detail: text(&detail), diff }
        }
        Said::ToolEnd { id, failed } => Event::ToolEnd { id, failed },
        // Past `MAX_CARDS` waiting, denied at once: no request is left waiting unseen.
        Said::Permission { ticket, tool, label, detail, .. } if native.tickets.len() >= MAX_CARDS => {
            let answer = Answer::Deny { message: Some("Too many requests at once: ask again.".into()) };
            let _ = native.driver.send(Input::Answer { ticket, answer });
            let reason = "too many requests at once".to_string();
            Event::AutoDenied { tool, label, reason, detail: text(&detail) }
        }
        Said::Permission { ticket, tool, label, detail, len, cut, claim, .. } => {
            native.next += 1;
            native.tickets.push((native.next, ticket));
            Event::Permission { ticket: native.next, tool, label, detail: text(&detail), len, cut, claim: text(&claim) }
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
        // Its stderr may hold tokens: the core hears only that there is some.
        Said::Exited { code, signal, .. } => Event::Exited { code, signal, details },
        Said::Unknown { kind } if ottospace_agents::QUIET.contains(&kind.as_str()) => return None,
        Said::Unknown { kind } => Event::Unknown { kind },
    })
}

/// Reads a local image for a thumbnail, off the main thread: inside the repository once its
/// links are resolved, a regular file of at most `MAX_THUMB`, a PNG, JPEG, GIF or WebP by its
/// first bytes. Never a URL: nothing is fetched.
fn read_thumb(path: String, repo: Option<String>) -> Thumb {
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
    });
    Thumb::Reading(read)
}

/// The local images among `blocks`.
fn images(blocks: &[Block], each: &mut impl FnMut(&str)) {
    for block in blocks {
        match block {
            Block::Image { path } => each(path),
            Block::Quote(inner) => images(inner, each),
            Block::List { items, .. } => items.iter().for_each(|item| images(item, each)),
            _ => {}
        }
    }
}

/// A reply's structure for the state: how many blocks of each kind, and links. No text.
fn shape(blocks: &[Block]) -> serde_json::Value {
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    fn count<'a>(blocks: &'a [Block], counts: &mut std::collections::BTreeMap<&'a str, usize>) {
        for block in blocks {
            let kind = match block {
                Block::Para(_) => "para",
                Block::Heading(_) => "heading",
                Block::Code { .. } => "code",
                Block::Quote(inner) => {
                    count(inner, counts);
                    "quote"
                }
                Block::List { items, .. } => {
                    items.iter().for_each(|item| count(item, counts));
                    "list"
                }
                Block::Table(_) => "table",
                Block::Image { .. } => "image",
                Block::Rule => "rule",
                Block::Cut { .. } => "cut",
            };
            *counts.entry(kind).or_default() += 1;
        }
    }
    count(blocks, &mut counts);
    let links = markdown::links_of(blocks);
    let web = links.iter().filter(|link| matches!(link.target, Target::Web(_))).count();
    serde_json::json!({ "blocks": counts, "links": { "web": web, "file": links.len() - web } })
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
        Notice::Resumed => "Session resumed. Earlier turns are in its file: Terminal shows them.".into(),
        Notice::MessageElsewhere => "A message, in the other terminal.".into(),
        Notice::Ended { code, signal, replied, .. } => {
            let how = match (code, signal) {
                (_, Some(signal)) => format!(" (signal {signal})"),
                (Some(code), None) => format!(" (exit {code})"),
                (None, None) => String::new(),
            };
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
    div().font_family(crate::terminal::font_family()).text_xs().whitespace_nowrap().child(text.to_string())
}
