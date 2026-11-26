//! A native conversation's transcript: what the agent runner's events built, bounded. The
//! shell translates `ottospace_agents::Event` into `Event` and the core folds it. Everything a
//! model or tool wrote is `Text`: drawn in the drawer, never serialised into the socket's state.
//! Design: `docs/ai/agent-runners.md`.

use std::collections::VecDeque;
use std::fmt;

use serde::Serialize;

use crate::activity::{Activity, Dial, Is, Pulse, Station};
use crate::ask::{Chosen, Form, FormShape, Question};
use crate::diff::{Diff, DiffShape};
use crate::gist::{self, Gist};
use crate::glance::Glances;

/// Text a model, a tool or the CLI wrote. Never `Serialize`; `Debug` prints its length only.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Text(String);

/// The most of one entry's text kept.
pub const MAX_TEXT: usize = 256 << 10;
/// The most entries kept, and the most bytes of text: past either the oldest go from memory.
pub const MAX_ITEMS: usize = 400;
pub const MAX_BYTES: usize = 4 << 20;
/// The most requests waiting at once. The shell denies one past it before the core sees it.
pub const MAX_CARDS: usize = 32;

impl Text {
    /// At most `MAX_TEXT` bytes of `text`, cut at a character.
    pub fn new(text: &str) -> Self {
        Self(cut(text, MAX_TEXT).to_string())
    }

    pub fn text(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn push(&mut self, more: &str) {
        let room = MAX_TEXT.saturating_sub(self.0.len());
        self.0.push_str(cut(more, room));
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Text({} bytes)", self.0.len())
    }
}

/// `text`'s first `max` bytes, at a character boundary.
pub fn cut(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Text as drawn: terminal escape sequences and other control characters out (a tab is four
/// spaces), marks that hide or reorder text written out (`\u{202e}`).
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => crate::workbook::skip_escape(&mut chars),
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            // Hidden or reordering marks (`capture::shows`) and line separators, written out.
            c if !crate::capture::shows(c) || matches!(c, '\u{2028}' | '\u{2029}') => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    out
}

/// The end of an agent's stderr as the drawer shows it on the user's click: cleaned as any
/// text (`clean`), then each line with its credential-shaped parts hidden (`secrets`). Empty:
/// nothing worth showing. Never kept in the model or the state.
pub fn details(stderr: &str) -> String {
    let lines = clean(stderr).lines().map(str::to_string).collect();
    crate::secrets::hide_lines(lines).join("\n").trim().to_string()
}

/// How the drawer shows a conversation: drawn natively, or the agent's own TUI in a terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    Native,
    Terminal,
}

/// How a turn ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Interrupted,
    /// `kind`: a code from the runner's table.
    Failed {
        kind: String,
    },
}

/// One runner event, as the core keeps it. Names (`tool`, `label`, `kind`, `model`, `mode`,
/// `session`) are labels by `docs/ai/sessions.md`'s rules; `Text` is for the drawer alone.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `mode`: the CLI's permission mode (`default`, `auto`, …), the user's own.
    Started {
        session: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    },
    TurnStarted,
    TextDelta {
        text: Text,
    },
    Text {
        text: Text,
    },
    /// `diff`: an edit's change, from the call's input (`diff::claude`, `diff::codex_*`).
    /// `station`: where it goes (`ottospace_sessions::station::classify`). `gist`: what its row
    /// shows (`gist::claude`).
    ToolStart {
        id: String,
        /// The sub-agent call it runs under (`parent_tool_use_id`); `Some("")`: a sub-agent's, its
        /// parent unknown (a watched session's file).
        parent: Option<String>,
        tool: String,
        station: Station,
        label: String,
        detail: Text,
        diff: Option<Diff>,
        gist: Option<Gist>,
    },
    /// `output`: what the call gave back (`gist::output`), kept only for a command's row.
    ToolEnd {
        id: String,
        failed: bool,
        output: Option<Text>,
    },
    /// A sub-agent or a background task started (Claude's `system/task_started`). `id`: the call
    /// that started it, whose row then says so; with none, a row of its own.
    Subtask {
        id: Option<String>,
        description: Text,
    },
    /// A sub-agent or background task ended, and what it said of it (`system/task_notification`).
    SubtaskEnded {
        status: Status,
        summary: Text,
    },
    /// The CLI's own notification (`system/notification`): its text.
    Note {
        text: Text,
    },
    /// A call denied without asking the user (`system/permission_denied`): `by` what.
    DeniedBy {
        tool: String,
        by: By,
    },
    /// The CLI went on with another model (`system/model_fallback` and its kin).
    ModelFallback {
        from: Option<String>,
        to: String,
    },
    /// `ticket`: the shell's number for the runner's request. `cut`: `detail` is the first part.
    /// `questions`: the agent asks the user (`RunnerSpec::structured.asks`): a question card.
    Permission {
        ticket: u64,
        tool: String,
        label: String,
        detail: Text,
        len: usize,
        cut: bool,
        claim: Text,
        questions: Vec<Question>,
    },
    PermissionClosed {
        ticket: u64,
    },
    AutoDenied {
        tool: String,
        label: String,
        reason: String,
        detail: Text,
    },
    Usage {
        context: Option<u64>,
        window: Option<u64>,
        cost_micro_usd: Option<u64>,
    },
    Compaction,
    TurnFinished {
        outcome: Outcome,
    },
    Error {
        kind: String,
        retrying: bool,
    },
    /// Its process ended: by its code, or a signal. `details`: the shell keeps its stderr's end,
    /// shown on the user's click (never here: it may hold tokens).
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        details: bool,
    },
    /// A line the runner sent that no rule reads, and not one known to be quiet: its kind, a
    /// label. Counted, never silent.
    Unknown {
        kind: String,
    },
    /// One of the user's hooks ran (`system.hook_started`): the dial's Hooks station.
    Hook,
    /// A watched session's file says a message was sent there: its words stay in the file.
    MessageElsewhere,
    /// A watched session's file says what the user wrote there: shown as a message of theirs.
    UserText {
        text: Text,
    },
    /// A watched session's file is read again from its start: what was built from it goes.
    WatchFromStart,
}

impl Event {
    /// True if a watched session's transcript shows it: what happened, never a turn to stop or
    /// a request to answer here.
    pub fn shows_when_watched(&self) -> bool {
        matches!(
            self,
            Event::Started { .. }
                | Event::ToolStart { .. }
                | Event::ToolEnd { .. }
                | Event::Usage { .. }
                | Event::Compaction
                | Event::Error { .. }
                | Event::MessageElsewhere
                | Event::UserText { .. }
                | Event::Text { .. }
                | Event::WatchFromStart
        )
    }
}

/// A tool call's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Call {
    Running,
    Done,
    Failed,
}

/// Fixed words a transcript says of itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "notice")]
pub enum Notice {
    Interrupted,
    Failed {
        kind: String,
    },
    Error {
        kind: String,
        retrying: bool,
    },
    Compacted,
    /// A permission answered: by the user, as shown.
    Allowed {
        tool: String,
        label: String,
    },
    Denied {
        tool: String,
        label: String,
    },
    /// A question card declined by the user.
    Declined,
    /// A call the CLI denied without asking the user, `by` what (its settings, its mode, …).
    DeniedBy {
        tool: String,
        by: By,
    },
    /// The CLI went on with another model.
    ModelFallback {
        from: Option<String>,
        to: String,
    },
    /// The session was gone on with. `here`: its earlier turns are on show above, and this is a
    /// quiet divider, which takes the place of the end before it (`ended`); else they are in its
    /// file, not here, and it says so.
    Resumed {
        here: bool,
        ended: Option<End>,
    },
    /// A message was sent in the terminal that runs the watched session.
    MessageElsewhere,
    /// A message of before, as its session's file has it (`Transcript::past`).
    MessageBefore,
    /// ottospace served (or refused) the task's read of an app pane (agent mode): its record,
    /// not the agent's account. `app`: the app's name, cleaned.
    AppUsed {
        what: crate::agent_mode::AppUse,
        app: String,
        pane: crate::model::PaneId,
        refused: Option<String>,
    },
    /// Its process ended. `replied`: anything came back from it since it started; `details`:
    /// its stderr's end can be shown.
    /// `by_user`: the user stopped it; no error, a quiet divider.
    Ended {
        code: Option<i32>,
        signal: Option<i32>,
        replied: bool,
        details: bool,
        by_user: bool,
    },
}

/// What denied a call without asking the user: from Claude's `decision_reason_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    /// A rule in the user's settings (`rule`, `subcommandResults`).
    Settings,
    /// The permission mode (`mode`).
    Mode,
    /// Auto mode's classifier (`classifier`).
    AutoMode,
    /// One of the user's hooks (`hook`).
    Hook,
    /// Outside the folders the session may touch (`workingDir`).
    Folder,
    /// A safety check (`safetyCheck`).
    Safety,
    /// The sandbox (`sandboxOverride`).
    Sandbox,
    /// Nobody could be asked (`asyncAgent`).
    Nobody,
    /// Anything else.
    Other,
}

impl By {
    /// Claude's `decision_reason_type`. `None`: `permissionPromptTool`, the user's own answer
    /// here, which the card's notice already says.
    pub fn claude(reason: Option<&str>) -> Option<Self> {
        Some(match reason {
            Some("rule" | "subcommandResults") => By::Settings,
            Some("mode") => By::Mode,
            Some("classifier") => By::AutoMode,
            Some("hook") => By::Hook,
            Some("workingDir") => By::Folder,
            Some("safetyCheck") => By::Safety,
            Some("sandboxOverride") => By::Sandbox,
            Some("asyncAgent") => By::Nobody,
            Some("permissionPromptTool") => return None,
            _ => By::Other,
        })
    }
}

/// How a subtask ended (`task_notification`'s `status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Completed,
    Failed,
    Stopped,
    Other,
}

impl Status {
    pub fn named(status: Option<&str>) -> Self {
        match status {
            Some("completed") => Status::Completed,
            Some("failed") => Status::Failed,
            Some("stopped" | "killed") => Status::Stopped,
            _ => Status::Other,
        }
    }
}

/// What a note row says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "note")]
pub enum NoteKind {
    /// "Started a subtask: <description>".
    Subtask,
    /// "Subtask done: <summary>", and the like.
    Ended { status: Status },
    /// The CLI's notification.
    Notification,
    /// "You answered 2 of 3 · <the answers>": a question card, sent.
    Answered { answered: usize, of: usize },
}

/// How an agent ended, kept on the divider of the resume that followed: for its hover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct End {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub by_user: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    User {
        text: Text,
    },
    /// `streaming`: deltas still arrive; the whole block replaces them.
    Assistant {
        text: Text,
        streaming: bool,
    },
    /// `parent`: the sub-agent call it ran under, if a sub-agent made it. `gist`: what its row
    /// shows; `output`: a command's.
    Tool {
        id: String,
        parent: Option<String>,
        tool: String,
        label: String,
        detail: Text,
        state: Call,
        diff: Option<Diff>,
        gist: Option<Gist>,
        output: Option<Text>,
    },
    /// A compact muted row of what the CLI said: a subtask, how it ended, a notification.
    Note {
        note: NoteKind,
        text: Text,
    },
    /// A request denied without asking, with what it asked.
    AutoDenied {
        tool: String,
        label: String,
        reason: String,
        detail: Text,
    },
    Notice(Notice),
}

impl Entry {
    fn bytes(&self) -> usize {
        match self {
            Entry::User { text } | Entry::Assistant { text, .. } => text.len(),
            Entry::Tool { detail, diff, gist, output, .. } => {
                let more = gist.as_ref().map_or(0, Gist::bytes) + output.as_ref().map_or(0, Text::len);
                detail.len() + 64 + diff.as_ref().map_or(0, Diff::bytes) + more
            }
            Entry::Note { text, .. } => text.len() + 32,
            Entry::AutoDenied { detail, .. } => detail.len() + 64,
            Entry::Notice(_) => 32,
        }
    }
}

/// An entry and its number: the shell's key for what it laid out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: u64,
    pub entry: Entry,
}

/// A permission request waiting for the user. Allow is refused on a `cut` card until it was
/// `expanded`: what cannot be shown in full is not approvable in one step. A question card
/// (`ask`) is never allowed as it came: only answered (`Transcript::reply`) or declined.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub ticket: u64,
    pub tool: String,
    pub label: String,
    pub detail: Text,
    pub len: usize,
    pub cut: bool,
    pub claim: Text,
    pub expanded: bool,
    pub ask: Option<Form>,
}

impl Card {
    /// True if Allow may be given now.
    pub fn allowable(&self) -> bool {
        (!self.cut || self.expanded) && self.ask.is_none()
    }
}

/// A turn under way: since when (ms since 1970), and the call it waits on.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub since: u64,
    pub tool: Option<String>,
}

/// One conversation as the drawer draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    items: VecDeque<Item>,
    next: u64,
    bytes: usize,
    /// Entries dropped from memory, oldest first: the session's file still has them.
    pub dropped: usize,
    /// Requests waiting: the first is shown, the rest queue behind it.
    pub cards: VecDeque<Card>,
    pub turn: Option<Turn>,
    pub session: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub context: Option<u64>,
    pub window: Option<u64>,
    pub cost_micro_usd: Option<u64>,
    /// Its process runs.
    pub live: bool,
    /// Built from a session another program runs (`Model::watch_target`): read-only.
    pub watched: bool,
    /// Built from the file of a session nothing has (a restart since its agent ran): its calls
    /// and edits, to go on with. Not read-only.
    pub past: bool,
    /// Something came back from its process since it started: a turn, text, a call, a request.
    pub replied: bool,
    /// Lines not understood, by kind, in the order first seen: at most `MAX_KINDS` kinds.
    pub unread: Vec<(String, usize)>,
    /// The user asked its process to stop: its end is theirs, not an error.
    pub stopping: bool,
    /// What its dial draws (`dial`).
    pub activity: Activity,
    /// Its shell calls that run, by id, their command lines: kept apart from the rows, which a
    /// long transcript drops (at most `MOST_SHELLS`, the newest).
    shells: Vec<(String, Text)>,
    /// Its sub-agent calls (`Agent`, `Task`), running or ended: at most `MAX_SUBS`, the newest.
    subs: Vec<Face>,
    /// What its agent read and searched lately: the code city's brief effects.
    pub glances: Glances,
}

/// The most sub-agent calls kept.
const MAX_SUBS: usize = 64;

/// A sub-agent its agent started, as the header's faces show it: `kind` its type (`Explore`),
/// `about` its description in one line, when it started and ended. Untrusted: never in the state.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub id: String,
    /// The sub-agent that started it; `None`: the agent itself.
    pub parent: Option<String>,
    pub kind: String,
    pub about: String,
    pub since: u64,
    pub ended: Option<u64>,
}

/// The most running shell calls whose lines are kept.
const MOST_SHELLS: usize = 8;

/// The most kinds of line not understood kept apart; past it they count under the last.
pub const MAX_KINDS: usize = 8;

impl Transcript {
    pub fn items(&self) -> impl DoubleEndedIterator<Item = &Item> + ExactSizeIterator {
        self.items.iter()
    }

    /// A turn runs or a request waits: nothing is sent, and the view does not switch.
    pub fn busy(&self) -> bool {
        self.turn.is_some() || !self.cards.is_empty()
    }

    /// The call a running turn waits on, for the rail's live line.
    pub fn live_label(&self) -> Option<&str> {
        self.turn.as_ref()?.tool.as_deref()
    }

    fn push(&mut self, entry: Entry) {
        self.bytes += entry.bytes();
        self.items.push_back(Item { id: self.next, entry });
        self.next += 1;
        self.trim();
    }

    /// Drops the oldest entries past `MAX_ITEMS` or `MAX_BYTES`; the newest always stays.
    fn trim(&mut self) {
        while self.items.len() > 1 && (self.items.len() > MAX_ITEMS || self.bytes > MAX_BYTES) {
            if let Some(gone) = self.items.pop_front() {
                self.bytes = self.bytes.saturating_sub(gone.entry.bytes());
                self.dropped += 1;
            }
        }
    }

    /// The live reply block, if one is being written.
    fn streaming(&mut self) -> Option<&mut Text> {
        match self.items.back_mut().map(|item| &mut item.entry) {
            Some(Entry::Assistant { text, streaming: true }) => Some(text),
            _ => None,
        }
    }

    /// The reply block being written is whole as it stands.
    /// A row the CLI says on its own time. Said while a reply streams, it goes before it, so the
    /// reply goes on: `Hel`, a note, `Hello` is one reply.
    fn aside(&mut self, entry: Entry) {
        self.push(entry);
        let n = self.items.len();
        if n >= 2 && matches!(self.items[n - 2].entry, Entry::Assistant { streaming: true, .. }) {
            self.items.swap(n - 2, n - 1);
        }
    }

    fn settle(&mut self) {
        if let Some(Item { entry: Entry::Assistant { streaming, .. }, .. }) = self.items.back_mut() {
            *streaming = false;
        }
    }

    /// The user's message, sent at `at`: a turn runs from now.
    pub fn send(&mut self, text: &str, at: u64) {
        if self.turn.is_some() {
            // Into the running turn: its reply streams on (`Hel`, the message, `Hello` is one) and its call stays.
            self.aside(Entry::User { text: Text::new(text) });
        } else {
            self.settle();
            self.push(Entry::User { text: Text::new(text) });
            self.turn = Some(Turn { since: at, tool: None });
        }
        self.activity.message(at);
    }

    /// Its dial at `now` (ms since 1970). A watched one steps a message at a time: no clocks.
    pub fn dial(&self, now: u64) -> Dial {
        self.activity.dial(now, &self.is())
    }

    /// Its dial's state alone: what the agent chip says while the conversation is closed.
    pub fn pulse(&self) -> Pulse {
        self.activity.pulse(&self.is())
    }

    /// A call of its agent runs now.
    pub fn calling(&self) -> bool {
        self.activity.running_label().is_some()
    }

    /// How many sub-agents its agent runs now: its `Agent` or `Task` calls that still run (one
    /// whose end never came is over once its calls are: an interrupt, an exit).
    pub fn subagents(&self) -> usize {
        self.subs.iter().filter(|face| face.parent.is_none() && self.activity.runs(&face.id)).count()
    }

    /// The sub-agents `of` started (`None`: the agent's own) for the header's faces: the running
    /// first, then the most recently active (started or ended last). Empty while none of them
    /// runs: the faces show only then.
    pub fn faces(&self, of: Option<&str>) -> Vec<Face> {
        let mine = || self.subs.iter().filter(|face| face.parent.as_deref() == of);
        if !mine().any(|face| self.activity.runs(&face.id)) {
            return Vec::new();
        }
        // Newest first, so calls of one batch (one time) keep that order through the stable sort.
        let mut faces: Vec<Face> = mine().rev().cloned().collect();
        for face in &mut faces {
            // One whose end never came is over once its calls are (an interrupt, an exit).
            if face.ended.is_none() && !self.activity.runs(&face.id) {
                face.ended = Some(face.since);
            }
        }
        faces.sort_by_key(|face| (face.ended.is_some(), std::cmp::Reverse(face.ended.unwrap_or(face.since))));
        faces
    }

    /// How many sub-agents it keeps.
    pub fn faces_kept(&self) -> usize {
        self.subs.len()
    }

    /// A sub-agent it started, at any depth, by its call's id.
    pub fn face(&self, id: &str) -> Option<&Face> {
        self.subs.iter().find(|face| face.id == id)
    }

    /// What a sub-agent does now, in one line: its newest call's label, else whether it ended.
    pub fn sub_status(&self, id: &str) -> Option<String> {
        let face = self.face(id)?;
        let last = self.items.iter().rev().find_map(|item| match &item.entry {
            Entry::Tool { parent: Some(of), label, state, .. } if of == id => Some((label, *state)),
            _ => None,
        });
        // As the chip's line: a label is the model's, cleaned and on one line.
        let last = last.map(|(label, state)| (clean(label).replace('\n', " "), state));
        let running = self.activity.runs(id);
        Some(match last {
            Some((label, Call::Running)) => format!("{} · {label}", face.kind),
            Some((label, _)) if running => format!("{} · last {label}", face.kind),
            _ if running => format!("{} · starting", face.kind),
            _ => format!("{} · done", face.kind),
        })
    }

    /// The command line of its newest shell call that still runs.
    pub fn shell_line(&self) -> Option<&str> {
        let id = self.activity.running_on(Station::Shell)?;
        self.shells.iter().find(|(of, _)| of == id).map(|(_, line)| line.text())
    }

    /// The one file its agent is at now (`Activity::at`).
    pub fn file_at(&self) -> Option<&str> {
        self.activity.at(&self.is())
    }

    /// The labels of the files its agent is at now (`Activity::files`).
    pub fn files(&self) -> Vec<(&str, bool)> {
        self.activity.files(&self.is())
    }

    /// `files`, and those held a while after their calls ended (`Activity::files_held`).
    pub fn files_held(&self, now: u64) -> Vec<(&str, bool)> {
        self.activity.files_held(&self.is(), now)
    }

    /// The newest file held at `now` (`Activity::file_held`).
    pub fn file_held(&self, now: u64) -> Option<&str> {
        self.activity.file_held(&self.is(), now)
    }

    /// When each of its files' holds ends (`Activity::holds_end`).
    pub fn holds_end(&self) -> Vec<u64> {
        self.activity.holds_end(&self.is()).collect()
    }

    fn is(&self) -> Is<'_> {
        let asking = self.cards.front().map(|card| card.label.as_str());
        let (live, watched, turn) = (self.live, self.watched && !self.live, self.turn.is_some());
        // A file's records came all at once: no clock says when its calls ran.
        let timed = !watched && (live || !self.past);
        Is { live, watched, turn, asking, context: self.context, window: self.window, timed }
    }

    /// A process for this conversation started (again). A resume with its earlier turns on show
    /// is one quiet divider, in place of the end just before it: one conversation, not two.
    pub fn started(&mut self, resumed: bool) {
        (self.live, self.watched, self.past, self.replied, self.stopping) = (true, false, false, false, false);
        // Calls a watched file never ended are not this process's.
        self.activity.settle(None, 0);
        if !resumed {
            return;
        }
        let here = self.items.iter().any(|item| !matches!(item.entry, Entry::Notice(_)));
        // With its turns on show, the end just before gives way; else it stays on the record.
        let ended = match self.items.back().map(|item| &item.entry).filter(|_| here) {
            Some(&Entry::Notice(Notice::Ended { code, signal, by_user, .. })) => Some(End { code, signal, by_user }),
            _ => None,
        };
        if ended.is_some() {
            self.items.pop_back();
        }
        self.push(Entry::Notice(Notice::Resumed { here, ended }));
    }

    /// A use of an app pane by this conversation's task, as ottospace saw it: a row, and one use
    /// of the dial's Apps station (failed, if refused).
    pub(crate) fn app_used(&mut self, notice: Notice, label: &str, failed: bool, at: u64) {
        self.push(Entry::Notice(notice));
        self.activity.app(label, failed, at);
    }

    /// Opens the first card's whole payload: Allow is possible from now on.
    pub fn expand(&mut self) {
        if let Some(card) = self.cards.front_mut() {
            card.expanded = true;
        }
    }

    /// The user's answer to the first card: its ticket, or `None` (no card; Allow on a cut card
    /// not yet expanded). A card is answered once: it is gone after.
    pub fn answer(&mut self, allow: bool) -> Option<u64> {
        let card = self.cards.front().filter(|card| !allow || card.allowable())?.clone();
        self.cards.pop_front();
        let (tool, label) = (card.tool, card.label);
        let notice = match (allow, card.ask.is_some()) {
            (_, true) => Notice::Declined,
            (true, false) => Notice::Allowed { tool, label },
            (false, false) => Notice::Denied { tool, label },
        };
        self.push(Entry::Notice(notice));
        Some(card.ticket)
    }

    /// The first card's question form, to move or pick in.
    pub fn form_mut(&mut self) -> Option<&mut Form> {
        self.cards.front_mut()?.ask.as_mut()
    }

    /// ↵ on the first card, a question card: its ticket and the answers, once each question has
    /// one; it is gone after. Else `None`, and the cursor on the question unanswered.
    pub fn reply(&mut self) -> Option<(u64, Vec<Chosen>)> {
        let chosen = self.form_mut()?.submit()?;
        let card = self.cards.pop_front()?;
        let form = card.ask.as_ref()?;
        // The card folds to a line: how many were answered, and the answers (drawn only).
        let note = NoteKind::Answered { answered: form.answers(), of: form.questions.len() };
        self.push(Entry::Note { note, text: Text::new(&form.summary()) });
        Some((card.ticket, chosen))
    }

    /// How many lines were not understood.
    pub fn unread_lines(&self) -> usize {
        self.unread.iter().map(|(_, n)| n).sum()
    }

    /// Folds one event in, at `at` (ms since 1970).
    pub fn fold(&mut self, event: Event, at: u64) {
        if matches!(
            event,
            Event::TurnStarted
                | Event::TextDelta { .. }
                | Event::Text { .. }
                | Event::ToolStart { .. }
                | Event::Permission { .. }
                | Event::AutoDenied { .. }
                | Event::TurnFinished { .. }
                | Event::Error { .. }
                | Event::Subtask { .. }
                | Event::SubtaskEnded { .. }
                | Event::Note { .. }
                | Event::DeniedBy { .. }
        ) {
            self.replied = true;
        }
        // The model writes: one request on the dial.
        if matches!(event, Event::TextDelta { .. } | Event::Text { .. }) {
            self.activity.step(at);
        }
        match event {
            Event::Started { session, model, mode } => {
                self.session = session.or(self.session.take());
                self.model = model.or(self.model.take());
                self.mode = mode.or(self.mode.take());
            }
            Event::TurnStarted => {
                if self.turn.is_none() {
                    self.activity.turn(at);
                    self.glances.clear();
                }
                self.turn.get_or_insert(Turn { since: at, tool: None });
            }
            Event::TextDelta { text } => match self.streaming() {
                Some(live) => {
                    let before = live.len();
                    live.push(text.text());
                    self.bytes += live.len() - before;
                    self.trim();
                }
                None => self.push(Entry::Assistant { text, streaming: true }),
            },
            Event::Text { text } => match self.items.back_mut() {
                Some(Item { entry: Entry::Assistant { text: live, streaming: streaming @ true }, .. }) => {
                    self.bytes = (self.bytes + text.len()).saturating_sub(live.len());
                    (*live, *streaming) = (text, false);
                    self.trim();
                }
                _ => self.push(Entry::Assistant { text, streaming: false }),
            },
            Event::ToolStart { id, parent, tool, station, label, detail, diff, gist } => {
                self.settle();
                self.activity.start(&id, station, &label, at);
                self.glances.start(&id, &tool, station, detail.text());
                // A watched file's sub-agent start names no parent (`Some("")`): no face. One id is
                // one face; a call its own parent has none (no loop to walk).
                let known = self.subs.iter().any(|face| face.id == id);
                if matches!(tool.as_str(), "Agent" | "Task") && parent.as_deref() != Some("") && !known {
                    if self.subs.len() == MAX_SUBS {
                        // The oldest that no longer runs goes first: a long one outlives many short.
                        let activity = &self.activity;
                        let gone = self.subs.iter().position(|face| !activity.runs(&face.id)).unwrap_or(0);
                        self.subs.remove(gone);
                    }
                    let about = gist.as_ref().map(|gist| gist.line(None)).unwrap_or_default();
                    let (kind, mine) = (label.clone(), parent.clone().filter(|of| *of != id));
                    self.subs.push(Face { id: id.clone(), parent: mine, kind, about, since: at, ended: None });
                }
                if station == Station::Shell {
                    if self.shells.len() == MOST_SHELLS {
                        self.shells.remove(0);
                    }
                    self.shells.push((id.clone(), detail.clone()));
                }
                if let Some(turn) = &mut self.turn {
                    turn.tool = Some(label.clone());
                }
                let state = Call::Running;
                self.push(Entry::Tool { id, parent, tool, label, detail, state, diff, gist, output: None });
            }
            Event::ToolEnd { id, failed, output } => {
                self.activity.end(&id, failed, at);
                self.glances.end(&id, output.as_ref().map(Text::text));
                self.shells.retain(|(of, _)| *of != id);
                if let Some(face) = self.subs.iter_mut().find(|face| face.id == id) {
                    face.ended = Some(at);
                }
                let call = self.items.iter_mut().rev().find_map(|item| match &mut item.entry {
                    Entry::Tool { id: of, state, gist, output, .. } if *of == id => Some((state, gist, output)),
                    _ => None,
                });
                let mut grew = 0;
                if let Some((state, gist, kept)) = call {
                    *state = if failed { Call::Failed } else { Call::Done };
                    // Only a command's row opens on what it printed.
                    let command = gist.as_ref().is_some_and(|gist| gist.kind == gist::Kind::Command);
                    if let Some(output) = output.filter(|output| command && !output.is_empty() && kept.is_none()) {
                        grew = output.len();
                        *kept = Some(output);
                    }
                }
                if grew > 0 {
                    self.bytes += grew;
                    self.trim();
                }
                // Another call that still runs is what the turn waits on now.
                let still = self.activity.running_label().map(str::to_string);
                if let Some(turn) = &mut self.turn {
                    turn.tool = still;
                }
            }
            // The call that started it says so (its gist); else a row of its own.
            Event::Subtask { id, description } => {
                // Only a sub-agent's row says it started one: a background shell's call does not.
                let started = |item: &Item| {
                    matches!(&item.entry, Entry::Tool { id: of, gist: Some(gist), .. }
                        if Some(of) == id.as_ref() && gist.kind == gist::Kind::Subtask)
                };
                if !self.items.iter().any(started) {
                    self.aside(Entry::Note { note: NoteKind::Subtask, text: description });
                }
            }
            Event::SubtaskEnded { status, summary } => {
                self.aside(Entry::Note { note: NoteKind::Ended { status }, text: summary })
            }
            Event::Note { text } => self.aside(Entry::Note { note: NoteKind::Notification, text }),
            Event::DeniedBy { tool, by } => self.aside(Entry::Notice(Notice::DeniedBy { tool, by })),
            Event::ModelFallback { from, to } => {
                self.model = Some(to.clone());
                self.aside(Entry::Notice(Notice::ModelFallback { from, to }));
            }
            Event::Permission { ticket, tool, label, detail, len, cut, claim, questions } => {
                if !self.cards.iter().any(|card| card.ticket == ticket) && self.cards.len() < MAX_CARDS {
                    self.settle();
                    let ask = Form::new(questions);
                    self.cards.push_back(Card { ticket, tool, label, detail, len, cut, claim, expanded: false, ask });
                }
            }
            Event::PermissionClosed { ticket } => self.cards.retain(|card| card.ticket != ticket),
            Event::AutoDenied { tool, label, reason, detail } => {
                self.push(Entry::AutoDenied { tool, label, reason, detail })
            }
            Event::Usage { context, window, cost_micro_usd } => {
                // A watched model request: its file says no more of it. Live, its replies and calls say it.
                if context.is_some() && (self.watched || self.past) {
                    self.activity.step(at);
                }
                self.context = context.or(self.context);
                self.window = window.or(self.window);
                self.cost_micro_usd = cost_micro_usd.or(self.cost_micro_usd);
            }
            Event::Compaction => self.push(Entry::Notice(Notice::Compacted)),
            Event::TurnFinished { outcome } => {
                self.settle();
                self.end_calls();
                (self.turn, self.cards) = (None, VecDeque::new());
                let failed = match &outcome {
                    Outcome::Failed { kind } => Some(kind.clone()),
                    _ => None,
                };
                self.activity.settle(failed, at);
                match outcome {
                    Outcome::Completed => {}
                    Outcome::Interrupted => self.push(Entry::Notice(Notice::Interrupted)),
                    Outcome::Failed { kind } => self.push(Entry::Notice(Notice::Failed { kind })),
                }
            }
            Event::Error { kind, retrying } => {
                if !retrying {
                    self.activity.failed(&kind);
                }
                self.push(Entry::Notice(Notice::Error { kind, retrying }))
            }
            Event::Exited { code, signal, details } => {
                self.settle();
                self.end_calls();
                (self.turn, self.cards, self.live) = (None, VecDeque::new(), false);
                let by_user = std::mem::take(&mut self.stopping);
                // An end the user did not ask for, by a code or a signal, is the dial's error.
                let fault = match (code, signal) {
                    (_, Some(signal)) => Some(format!("signal {signal}")),
                    (Some(code), None) if code != 0 => Some(format!("exit {code}")),
                    _ => None,
                };
                self.activity.settle(fault.filter(|_| !by_user), at);
                self.push(Entry::Notice(Notice::Ended { code, signal, replied: self.replied, details, by_user }));
            }
            Event::Unknown { kind } => {
                let seen = self.unread.iter().position(|(of, _)| *of == kind);
                match seen.or((self.unread.len() >= MAX_KINDS).then_some(MAX_KINDS - 1)) {
                    Some(at) => self.unread[at].1 += 1,
                    None => self.unread.push((kind, 1)),
                }
            }
            // Only a watched transcript is rebuilt; a live one is the stream's.
            Event::WatchFromStart if !self.live => {
                *self = Transcript { watched: !self.past, past: self.past, ..Transcript::default() }
            }
            Event::WatchFromStart => {}
            Event::Hook => self.activity.hook(at),
            Event::MessageElsewhere => {
                self.message_there(at);
                self.push(Entry::Notice(if self.past { Notice::MessageBefore } else { Notice::MessageElsewhere }));
            }
            Event::UserText { text } => {
                self.message_there(at);
                self.push(Entry::User { text });
            }
        }
    }

    /// A new message in a watched session's file: its calls before it are over.
    fn message_there(&mut self, at: u64) {
        self.activity.settle(None, at);
        self.activity.message(at);
        self.settle();
    }

    /// Calls still running when their turn ends never finished: shown as failed.
    fn end_calls(&mut self) {
        for item in &mut self.items {
            if let Entry::Tool { state: state @ Call::Running, .. } = &mut item.entry {
                *state = Call::Failed;
            }
        }
    }

    /// What the socket's state says of it: structure, counts and labels, never its text.
    pub fn summary(&self) -> Summary {
        let entries = self.items.iter().map(|item| Shape::of(&item.entry)).collect();
        let cards = self.cards.iter().map(|card| CardShape {
            tool: card.tool.clone(),
            label: card.label.clone(),
            cut: card.cut,
            expanded: card.expanded,
            len: card.len,
            ask: card.ask.as_ref().map(Form::shape),
        });
        Summary {
            entries,
            dropped: self.dropped,
            cards: cards.collect(),
            turn: self.turn.as_ref().map(|turn| turn.tool.clone().unwrap_or_default()),
            model: self.model.clone(),
            mode: self.mode.clone(),
            session: self.session.clone(),
            context: self.context,
            window: self.window,
            live: self.live,
            watched: self.watched,
            not_understood: self.unread_lines(),
        }
    }
}

/// A transcript as the state reply has it. No text of the model's or a tool's.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Summary {
    pub entries: Vec<Shape>,
    pub dropped: usize,
    pub cards: Vec<CardShape>,
    /// A turn runs: the label of the call it waits on, else empty.
    pub turn: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub session: Option<String>,
    pub context: Option<u64>,
    pub window: Option<u64>,
    pub live: bool,
    pub watched: bool,
    /// Lines the runner sent that no rule reads: a count, never what they said.
    pub not_understood: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Shape {
    User {
        bytes: usize,
    },
    Assistant {
        bytes: usize,
        streaming: bool,
    },
    /// `summary`: what its row shows, by kind and lines (`gist::Shape`), never its words.
    Tool {
        tool: String,
        label: String,
        state: Call,
        sub: bool,
        diff: Option<DiffShape>,
        summary: Option<gist::Shape>,
    },
    Note {
        #[serde(flatten)]
        note: NoteKind,
        bytes: usize,
    },
    AutoDenied {
        tool: String,
        label: String,
        reason: String,
    },
    Notice(Notice),
}

impl Shape {
    fn of(entry: &Entry) -> Self {
        match entry {
            Entry::User { text } => Shape::User { bytes: text.len() },
            Entry::Assistant { text, streaming } => Shape::Assistant { bytes: text.len(), streaming: *streaming },
            Entry::Tool { tool, label, state, parent, diff, gist, .. } => Shape::Tool {
                tool: tool.clone(),
                label: label.clone(),
                state: *state,
                sub: parent.is_some(),
                diff: diff.as_ref().map(Diff::shape),
                summary: gist.as_ref().map(Gist::shape),
            },
            Entry::Note { note, text } => Shape::Note { note: *note, bytes: text.len() },
            Entry::AutoDenied { tool, label, reason, .. } => {
                Shape::AutoDenied { tool: tool.clone(), label: label.clone(), reason: reason.clone() }
            }
            Entry::Notice(notice) => Shape::Notice(notice.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CardShape {
    pub tool: String,
    pub label: String,
    pub cut: bool,
    pub expanded: bool,
    pub len: usize,
    /// A question card's shape: counts, picks and the cursor. Null for a permission.
    pub ask: Option<FormShape>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Text {
        Text::new(s)
    }

    fn texts(t: &Transcript) -> Vec<String> {
        t.items()
            .map(|item| match &item.entry {
                Entry::User { text } => format!("user:{}", text.text()),
                Entry::Assistant { text, streaming } => {
                    format!("{}:{}", if *streaming { "live" } else { "done" }, text.text())
                }
                Entry::Tool { label, state, .. } => format!("tool:{label}:{state:?}"),
                Entry::AutoDenied { label, .. } => format!("auto:{label}"),
                Entry::Note { note, text } => format!("note:{note:?}:{}", text.text()),
                Entry::Notice(notice) => format!("{notice:?}"),
            })
            .collect()
    }

    fn permission(ticket: u64, cut: bool) -> Event {
        let (tool, label) = ("Bash".into(), "git".into());
        let (detail, claim) = (text("command: git"), Text::default());
        Event::Permission { ticket, tool, label, detail, len: 12, cut, claim, questions: Vec::new() }
    }

    /// Found in review: a message sent into a turn mid-reply split it in two (`Hel`, `Hello`).
    #[test]
    fn a_message_mid_reply_leaves_the_reply_whole() {
        let mut t = Transcript::default();
        t.send("hi", 10);
        t.fold(Event::TurnStarted, 11);
        t.fold(Event::TextDelta { text: text("Hel") }, 12);
        t.send("steer", 13);
        t.fold(Event::TextDelta { text: text("lo") }, 14);
        t.fold(Event::Text { text: text("Hello") }, 15);
        assert_eq!(texts(&t), ["user:hi", "user:steer", "done:Hello"]);
        assert_eq!(t.turn.as_ref().map(|turn| turn.since), Some(10), "the same turn");
    }

    #[test]
    fn deltas_build_a_block_the_whole_text_replaces() {
        let mut t = Transcript::default();
        t.send("hi", 10);
        t.fold(Event::TurnStarted, 11);
        t.fold(Event::TextDelta { text: text("Let me ") }, 12);
        t.fold(Event::TextDelta { text: text("look") }, 13);
        assert_eq!(texts(&t), ["user:hi", "live:Let me look"]);
        t.fold(Event::Text { text: text("Let me look.") }, 14);
        let tool = Event::ToolStart {
            id: "a".into(),
            parent: None,
            tool: "Bash".into(),
            station: crate::activity::Station::Shell,
            label: "git".into(),
            detail: text("git status"),
            diff: None,
            gist: None,
        };
        t.fold(tool, 15);
        assert_eq!(t.live_label(), Some("git"));
        t.fold(Event::ToolEnd { id: "a".into(), failed: false, output: None }, 16);
        t.fold(Event::TextDelta { text: text("Done") }, 17);
        t.fold(Event::TurnFinished { outcome: Outcome::Completed }, 18);
        assert_eq!(texts(&t), ["user:hi", "done:Let me look.", "tool:git:Done", "done:Done"]);
        assert!(!t.busy() && t.turn.is_none());
        // A whole block with no deltas before it is a block of its own.
        t.fold(Event::Text { text: text("Again") }, 19);
        assert_eq!(texts(&t).last().map(String::as_str), Some("done:Again"));
    }

    #[test]
    fn a_turn_cut_short_says_so_and_ends_its_calls() {
        let mut t = Transcript::default();
        t.send("go", 1);
        let tool = Event::ToolStart {
            id: "a".into(),
            parent: Some("p".into()),
            tool: "Read".into(),
            station: crate::activity::Station::Files,
            label: "a.rs".into(),
            detail: text("/a.rs"),
            diff: None,
            gist: None,
        };
        t.fold(tool, 2);
        t.fold(permission(1, false), 3);
        t.fold(Event::TurnFinished { outcome: Outcome::Interrupted }, 4);
        assert_eq!(texts(&t), ["user:go", "tool:a.rs:Failed", "Interrupted"]);
        assert!(t.cards.is_empty(), "no request outlives its turn");
        t.fold(Event::Exited { code: Some(0), signal: None, details: false }, 5);
        assert!(!t.live && matches!(t.items().last().map(|i| &i.entry), Some(Entry::Notice(Notice::Ended { .. }))));
    }

    /// The user's stop and the resume after it, with the turns on show: one quiet divider in
    /// place of the end; "earlier turns are in its file" only when none are on show.
    #[test]
    fn a_stop_then_a_resume_is_one_divider_in_one_conversation() {
        let mut t = Transcript::default();
        t.started(false);
        t.send("Task: plan", 1);
        t.fold(Event::TurnFinished { outcome: Outcome::Completed }, 2);
        t.stopping = true;
        t.fold(Event::Exited { code: None, signal: Some(9), details: false }, 3);
        let stopped = Notice::Ended { code: None, signal: Some(9), replied: true, details: false, by_user: true };
        assert_eq!(texts(&t).last(), Some(&format!("{stopped:?}")));
        t.started(true);
        let end = End { code: None, signal: Some(9), by_user: true };
        let divider = Notice::Resumed { here: true, ended: Some(end) };
        assert_eq!(texts(&t)[1..], [format!("{divider:?}")], "the end gives way: one line, not two");
        assert!(!t.stopping, "a later end is not the user's unless asked again");
        // A resume of a session not shown before: its turns are in its file, and it says so.
        let mut fresh = Transcript::default();
        fresh.started(true);
        let elsewhere = format!("{:?}", Notice::Resumed { here: false, ended: None });
        assert_eq!(texts(&fresh), [elsewhere.clone()]);
        // Stopped and resumed again with still no turns on show: the stop stays on the record.
        fresh.stopping = true;
        fresh.fold(Event::Exited { code: None, signal: None, details: false }, 4);
        fresh.started(true);
        assert_eq!(fresh.items().len(), 3, "{:?}", texts(&fresh));
    }

    /// A running shell call's line outlasts its row: a long transcript drops old rows, never the
    /// line of a call that still runs. Ended, it is gone.
    #[test]
    fn a_running_shell_line_outlasts_its_row() {
        let mut t = Transcript::default();
        t.started(false);
        let bash = Event::ToolStart {
            id: "b".into(),
            parent: None,
            tool: "Bash".into(),
            station: Station::Shell,
            label: "cat".into(),
            detail: Text::new("cat src/main.rs"),
            diff: None,
            gist: None,
        };
        t.fold(Event::TurnStarted, 1);
        t.fold(bash, 2);
        for n in 0..(MAX_ITEMS + 10) {
            t.fold(Event::Text { text: Text::new(&format!("note {n}")) }, 3);
        }
        assert!(!t.items().any(|item| matches!(&item.entry, Entry::Tool { .. })), "its row is gone");
        assert_eq!(t.shell_line(), Some("cat src/main.rs"));
        t.fold(Event::ToolEnd { id: "b".into(), failed: false, output: None }, 4);
        assert_eq!(t.shell_line(), None);
    }

    /// Live, a usage line after a call is no second model request; a watched call left open is
    /// over at the next message there, and at a start here.
    #[test]
    fn the_dial_counts_a_step_once_and_drops_calls_left_open() {
        let call = |id: &str| Event::ToolStart {
            id: id.into(),
            parent: None,
            tool: "Bash".into(),
            station: Station::Shell,
            label: "ls".into(),
            detail: Text::default(),
            diff: None,
            gist: None,
        };
        let usage = Event::Usage { context: Some(5), window: None, cost_micro_usd: None };
        let mut t = Transcript::default();
        t.started(false);
        t.send("go", 0);
        t.fold(call("a"), 1);
        t.fold(Event::ToolEnd { id: "a".into(), failed: false, output: None }, 2);
        t.fold(usage.clone(), 3);
        let model = |t: &Transcript| t.dial(4).spot(Station::Model).map(|spot| spot.calls);
        assert_eq!(model(&t), Some(1));
        let lit = |t: &Transcript| t.dial(9).spot(Station::Shell).is_some_and(|spot| spot.lit);
        let mut w = Transcript { watched: true, ..Transcript::default() };
        w.fold(usage, 1);
        w.fold(call("b"), 2);
        assert!(model(&w) == Some(1) && lit(&w));
        w.fold(Event::MessageElsewhere, 3);
        assert!(!lit(&w), "a new message there");
        w.fold(call("d"), 4);
        w.fold(Event::UserText { text: Text::new("made-up words") }, 5);
        assert!(!lit(&w), "its words too");
        assert!(matches!(&w.items.back().unwrap().entry, Entry::User { text } if text.text() == "made-up words"));
        w.fold(call("c"), 4);
        w.started(true);
        assert!(!lit(&w), "taken over here");
    }

    #[test]
    fn details_are_cleaned_and_tokens_hidden() {
        let stderr = "\u{1b}[31mError:\u{1b}[0m bad key sk-ant-api03-abcdefghijklmnop\nAuthorization: Bearer x\n";
        assert_eq!(details(stderr), "Error: bad key [hidden]\nAuthorization: [hidden]");
        assert_eq!(details("  \n"), "");
    }

    /// An end says whether anything came back since the process started; lines not understood
    /// are counted by kind, and the summary has their count alone.
    #[test]
    fn an_end_says_if_it_replied_and_unread_lines_are_counted() {
        let mut t = Transcript::default();
        t.started(false);
        t.send("Task: plan", 1);
        for kind in ["made_up", "made_up", "system.other"] {
            t.fold(Event::Unknown { kind: kind.into() }, 2);
        }
        t.fold(Event::Exited { code: Some(0), signal: None, details: true }, 3);
        let ended = Notice::Ended { code: Some(0), signal: None, replied: false, details: true, by_user: false };
        assert_eq!(texts(&t).last(), Some(&format!("{ended:?}")));
        assert_eq!(t.unread, [("made_up".to_string(), 2), ("system.other".to_string(), 1)]);
        assert_eq!(t.summary().not_understood, 3);
        // Started again, it replies: the next end says so.
        t.started(true);
        t.fold(Event::TurnStarted, 4);
        t.fold(Event::Exited { code: None, signal: Some(9), details: false }, 5);
        let ended = Notice::Ended { code: None, signal: Some(9), replied: true, details: false, by_user: false };
        assert_eq!(texts(&t).last(), Some(&format!("{ended:?}")));
        for n in 0..MAX_KINDS + 3 {
            t.fold(Event::Unknown { kind: format!("k{n}") }, 6);
        }
        assert_eq!((t.unread.len(), t.unread_lines()), (MAX_KINDS, 3 + MAX_KINDS + 3));
    }

    /// A cut request is not allowed until opened in full; each is answered once; a second
    /// request queues behind the first; a withdrawn one goes.
    #[test]
    fn cards_are_answered_once_in_order_and_a_cut_one_only_once_expanded() {
        let mut t = Transcript::default();
        t.fold(permission(7, true), 1);
        t.fold(permission(8, false), 2);
        t.fold(permission(7, true), 3);
        assert_eq!(t.cards.len(), 2, "the same request twice is one card");
        assert_eq!(t.answer(true), None, "cut and not opened: no Allow");
        t.expand();
        assert_eq!(t.answer(true), Some(7));
        assert_eq!(t.cards.front().map(|c| c.ticket), Some(8));
        assert_eq!(t.answer(false), Some(8));
        assert_eq!((t.answer(false), t.answer(true)), (None, None), "nothing left to answer");
        t.fold(permission(9, false), 4);
        t.fold(Event::PermissionClosed { ticket: 9 }, 5);
        assert!(t.cards.is_empty());
        let notices: Vec<String> = texts(&t);
        assert_eq!(notices, [r#"Allowed { tool: "Bash", label: "git" }"#, r#"Denied { tool: "Bash", label: "git" }"#]);
    }

    #[test]
    fn the_oldest_entries_go_past_the_caps_and_the_count_says_so() {
        let mut t = Transcript::default();
        for n in 0..MAX_ITEMS + 5 {
            t.send(&n.to_string(), n as u64);
        }
        assert_eq!((t.items().len(), t.dropped), (MAX_ITEMS, 5));
        assert_eq!(t.items().next().map(|item| item.id), Some(5));
        let mut t = Transcript::default();
        let big = "x".repeat(MAX_TEXT * 2);
        for _ in 0..(MAX_BYTES / MAX_TEXT) + 3 {
            t.fold(Event::Text { text: text(&big) }, 0);
        }
        assert!(t.bytes <= MAX_BYTES && t.dropped > 0, "{} bytes", t.bytes);
        // One streamed block grows to its cap and no further.
        let mut t = Transcript::default();
        for _ in 0..3 {
            t.fold(Event::TextDelta { text: text(&big) }, 0);
        }
        assert_eq!(t.items().map(|item| item.entry.bytes()).sum::<usize>(), MAX_TEXT);
        assert_eq!(t.bytes, MAX_TEXT);
    }

    #[test]
    fn text_is_cut_at_a_character_and_never_printed() {
        let t = Text::new(&"é".repeat(MAX_TEXT));
        assert!(t.len() <= MAX_TEXT && t.text().chars().all(|c| c == 'é'));
        assert_eq!(format!("{:?}", Text::new("secret")), "Text(6 bytes)");
        let mut transcript = Transcript::default();
        transcript.send("secret words", 0);
        let summary = serde_json::to_string(&transcript.summary()).unwrap();
        assert!(!summary.contains("secret"), "{summary}");
    }

    /// An edit's card stays with its call; the state has its counts, never a line or its path.
    #[test]
    fn an_edit_call_keeps_its_card_and_the_summary_only_its_counts() {
        let input = serde_json::json!({"file_path": "/w/secret.rs", "old_string": "secret a", "new_string": "b\nc"});
        let diff = crate::diff::claude("Edit", &input);
        let mut t = Transcript { watched: true, ..Transcript::default() };
        let start = |diff| Event::ToolStart {
            id: "e".into(),
            parent: None,
            tool: "Edit".into(),
            station: crate::activity::Station::Files,
            label: "secret.rs".into(),
            detail: Text::default(),
            diff,
            gist: None,
        };
        assert!(start(diff.clone()).shows_when_watched());
        t.fold(start(diff.clone()), 1);
        t.fold(start(None), 2);
        let kept: Vec<Option<&Diff>> = t
            .items()
            .map(|item| match &item.entry {
                Entry::Tool { diff, .. } => diff.as_ref(),
                _ => None,
            })
            .collect();
        assert_eq!(kept, [diff.as_ref(), None]);
        let summary = serde_json::to_value(t.summary()).unwrap();
        let shape = &summary["entries"][0]["diff"];
        assert_eq!(shape["changes"], serde_json::json!(["edit"]));
        assert_eq!((&shape["added"], &shape["removed"], &shape["lines"]), (&2.into(), &1.into(), &3.into()));
        assert!(summary["entries"][1]["diff"].is_null(), "{summary}");
        assert!(!summary.to_string().contains("w/secret"), "{summary}");
    }
    /// A command's row keeps what it printed; a row of another kind does not. Its summary in the
    /// state is a shape: no command, no output.
    #[test]
    fn a_command_row_keeps_its_output_and_the_state_only_its_shape() {
        let call = |id: &str, tool: &str, input: serde_json::Value| Event::ToolStart {
            id: id.into(),
            parent: None,
            tool: tool.into(),
            station: Station::Shell,
            label: "x".into(),
            detail: Text::default(),
            diff: None,
            gist: crate::gist::claude(tool, &input),
        };
        let end = |id: &str| Event::ToolEnd { id: id.into(), failed: false, output: Some(text("secret out")) };
        let mut t = Transcript::default();
        t.fold(call("b", "Bash", serde_json::json!({"command": "echo secret", "description": "Say"})), 1);
        t.fold(end("b"), 2);
        t.fold(call("r", "Read", serde_json::json!({"file_path": "/w/secret.rs"})), 3);
        t.fold(end("r"), 4);
        let outputs: Vec<Option<&str>> = t
            .items()
            .map(|item| match &item.entry {
                Entry::Tool { output, .. } => output.as_ref().map(Text::text),
                _ => None,
            })
            .collect();
        assert_eq!(outputs, [Some("secret out"), None]);
        let summary = serde_json::to_value(t.summary()).unwrap();
        assert_eq!(summary["entries"][0]["summary"], serde_json::json!({"kind": "command", "lines": 1, "note": true}));
        assert_eq!(summary["entries"][1]["summary"]["kind"], "path");
        assert!(!summary.to_string().contains("secret"), "{summary}");
    }

    /// The faces: none while no sub-agent runs; the running first, newest first, then the ended,
    /// the last to end first; each with its type and description.
    #[test]
    fn faces_put_running_sub_agents_first_then_the_most_recent() {
        let mut t = Transcript::default();
        let start = |t: &mut Transcript, id: &str, kind: &str, about: &str, at| {
            let input = serde_json::json!({"description": about, "subagent_type": kind});
            let gist = crate::gist::claude("Agent", &input);
            let (id, tool, label, detail) = (id.into(), "Agent".into(), kind.into(), Text::default());
            let station = Station::Agents;
            t.fold(Event::ToolStart { id, parent: None, tool, station, label, detail, diff: None, gist }, at);
        };
        let end = |t: &mut Transcript, id: &str, at| {
            t.fold(Event::ToolEnd { id: id.into(), failed: false, output: None }, at)
        };
        start(&mut t, "a", "Explore", "Find every caller", 1);
        start(&mut t, "b", "tests", "Retry backoff tests", 2);
        start(&mut t, "c", "review", "Review queue.ts", 3);
        end(&mut t, "a", 4);
        end(&mut t, "b", 5);
        let ids = |t: &Transcript| t.faces(None).into_iter().map(|f| f.id).collect::<Vec<_>>();
        assert_eq!(ids(&t), ["c", "b", "a"], "running first, then the last to end");
        let c = &t.faces(None)[0];
        assert_eq!((c.kind.as_str(), c.about.as_str(), c.ended), ("review", "Review queue.ts", None));
        // A sub-agent's own sub-agent is its face, not the agent's.
        t.fold(
            Event::ToolStart {
                id: "s".into(),
                parent: Some("c".into()),
                tool: "Agent".into(),
                station: Station::Agents,
                label: "x".into(),
                detail: Text::default(),
                diff: None,
                gist: None,
            },
            6,
        );
        assert_eq!(t.faces(None).len(), 3);
        let nested = t.faces(Some("c"));
        assert_eq!((nested.len(), nested[0].parent.as_deref()), (1, Some("c")));
        assert_eq!(t.subagents(), 1, "the agent's own only");
        end(&mut t, "c", 7);
        assert!(t.faces(None).is_empty(), "none runs: no faces");
        // One long sub-agent outlives `MAX_SUBS` short ones: it is still counted.
        start(&mut t, "long", "Explore", "Long", 8);
        for n in 0..MAX_SUBS as u64 {
            start(&mut t, &format!("short{n}"), "Explore", "Short", 9 + n);
            end(&mut t, &format!("short{n}"), 9 + n);
        }
        assert_eq!(t.subagents(), 1);
        assert_eq!(t.faces(None)[0].id, "long");
        // Started in one batch, at one time: the newest first.
        start(&mut t, "x", "Explore", "X", 100);
        start(&mut t, "y", "Explore", "Y", 100);
        assert_eq!(ids(&t)[..3], ["y", "x", "long"]);
    }

    /// A subtask's start is its call's row when there is one, else a row of its own; its end,
    /// a notification, a denial by settings and a fallback are rows; the state has their kinds.
    #[test]
    fn subtasks_notes_denials_and_fallbacks_are_rows() {
        let mut t = Transcript::default();
        let agent = serde_json::json!({"description": "Find it", "subagent_type": "Explore"});
        t.fold(
            Event::ToolStart {
                id: "a".into(),
                parent: None,
                tool: "Agent".into(),
                station: Station::Agents,
                label: "Explore".into(),
                detail: Text::default(),
                diff: None,
                gist: crate::gist::claude("Agent", &agent),
            },
            1,
        );
        t.fold(Event::Subtask { id: Some("a".into()), description: text("Find it") }, 2);
        t.fold(Event::Subtask { id: Some("z".into()), description: text("Watch the build") }, 3);
        t.fold(Event::SubtaskEnded { status: Status::Completed, summary: text("Found it") }, 4);
        t.fold(Event::Note { text: text("Fast mode is off") }, 5);
        t.fold(Event::DeniedBy { tool: "Bash".into(), by: By::Settings }, 6);
        t.fold(Event::ModelFallback { from: Some("m-1".into()), to: "m-2".into() }, 7);
        assert_eq!(
            texts(&t),
            [
                "tool:Explore:Running",
                "note:Subtask:Watch the build",
                "note:Ended { status: Completed }:Found it",
                "note:Notification:Fast mode is off",
                r#"DeniedBy { tool: "Bash", by: Settings }"#,
                r#"ModelFallback { from: Some("m-1"), to: "m-2" }"#,
            ]
        );
        assert_eq!(t.model.as_deref(), Some("m-2"));
        // A background shell's start, though its call has a row: a row of its own.
        let shell = serde_json::json!({"command": "cargo build", "run_in_background": true});
        let mut b = Transcript::default();
        b.fold(
            Event::ToolStart {
                id: "s".into(),
                parent: None,
                tool: "Bash".into(),
                station: Station::Shell,
                label: "cargo build".into(),
                detail: Text::default(),
                diff: None,
                gist: crate::gist::claude("Bash", &shell),
            },
            1,
        );
        b.fold(Event::Subtask { id: Some("s".into()), description: text("Build") }, 2);
        assert_eq!(texts(&b), ["tool:cargo build:Running", "note:Subtask:Build"]);
        // A fallback while a reply streams: the reply goes on, whole, after it.
        let mut r = Transcript::default();
        r.fold(Event::TextDelta { text: text("Look") }, 1);
        r.fold(Event::ModelFallback { from: None, to: "m-2".into() }, 2);
        r.fold(Event::Text { text: text("Looked.") }, 3);
        assert_eq!(texts(&r), [r#"ModelFallback { from: None, to: "m-2" }"#, "done:Looked."]);
        // So does a note, a subtask's end or a denial.
        let mut n = Transcript::default();
        n.fold(Event::TextDelta { text: text("Hel") }, 1);
        n.fold(Event::Note { text: text("ready") }, 2);
        n.fold(Event::SubtaskEnded { status: Status::Failed, summary: text("x") }, 3);
        n.fold(Event::DeniedBy { tool: "Bash".into(), by: By::Hook }, 4);
        n.fold(Event::Text { text: text("Hello") }, 5);
        assert_eq!(texts(&n).last().map(String::as_str), Some("done:Hello"));
        assert_eq!(texts(&n).iter().filter(|t| t.starts_with("done:")).count(), 1, "{:?}", texts(&n));
        let summary = serde_json::to_value(t.summary()).unwrap();
        let ended = &summary["entries"][2];
        assert_eq!(ended, &serde_json::json!({"kind": "note", "note": "ended", "status": "completed", "bytes": 8}));
        assert_eq!(
            summary["entries"][4],
            serde_json::json!({"kind": "notice", "notice": "denied_by", "tool": "Bash", "by": "settings"})
        );
        assert!(!summary.to_string().contains("Found"), "{summary}");
    }

    #[test]
    fn denials_and_ends_are_named_from_fixed_tables() {
        assert_eq!(By::claude(Some("rule")), Some(By::Settings));
        assert_eq!(By::claude(Some("classifier")), Some(By::AutoMode));
        assert_eq!(By::claude(Some("permissionPromptTool")), None, "the user's own answer: its notice says it");
        assert_eq!(By::claude(Some("a private reason")), Some(By::Other));
        assert_eq!(By::claude(None), Some(By::Other));
        assert_eq!(Status::named(Some("killed")), Status::Stopped);
        assert_eq!(Status::named(Some("anything")), Status::Other);
    }
}
