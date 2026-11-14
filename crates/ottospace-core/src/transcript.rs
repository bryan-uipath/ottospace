//! A native conversation's transcript: what the agent runner's events built, bounded. The
//! shell translates `ottospace_agents::Event` into `Event` and the core folds it. Everything a
//! model or tool wrote is `Text`: drawn in the drawer, never serialised into the socket's state.
//! Design: `docs/ai/agent-runners.md`.

use std::collections::VecDeque;
use std::fmt;

use serde::Serialize;

use crate::diff::{Diff, DiffShape};

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
pub(crate) fn cut(text: &str, max: usize) -> &str {
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
    ToolStart {
        id: String,
        sub: bool,
        tool: String,
        label: String,
        detail: Text,
        diff: Option<Diff>,
    },
    ToolEnd {
        id: String,
        failed: bool,
    },
    /// `ticket`: the shell's number for the runner's request. `cut`: `detail` is the first part.
    Permission {
        ticket: u64,
        tool: String,
        label: String,
        detail: Text,
        len: usize,
        cut: bool,
        claim: Text,
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
    Exited {
        code: Option<i32>,
    },
    /// A watched session's file says a message was sent there: its words stay in the file.
    MessageElsewhere,
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
    /// The session was gone on with: earlier turns are in its file, not here.
    Resumed,
    /// A message was sent in the terminal that runs the watched session.
    MessageElsewhere,
    Ended {
        code: Option<i32>,
    },
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
    /// `sub`: a call a sub-agent made.
    Tool {
        id: String,
        sub: bool,
        tool: String,
        label: String,
        detail: Text,
        state: Call,
        diff: Option<Diff>,
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
            Entry::Tool { detail, diff, .. } => detail.len() + 64 + diff.as_ref().map_or(0, Diff::bytes),
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
/// `expanded`: what cannot be shown in full is not approvable in one step.
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
}

impl Card {
    /// True if Allow may be given now.
    pub fn allowable(&self) -> bool {
        !self.cut || self.expanded
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
}

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
    fn settle(&mut self) {
        if let Some(Item { entry: Entry::Assistant { streaming, .. }, .. }) = self.items.back_mut() {
            *streaming = false;
        }
    }

    /// The user's message, sent at `at`: a turn runs from now.
    pub fn send(&mut self, text: &str, at: u64) {
        self.settle();
        self.push(Entry::User { text: Text::new(text) });
        self.turn = Some(Turn { since: at, tool: None });
    }

    /// A process for this conversation started (again).
    pub fn started(&mut self, resumed: bool) {
        (self.live, self.watched) = (true, false);
        if resumed {
            self.push(Entry::Notice(Notice::Resumed));
        }
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
        self.push(Entry::Notice(if allow { Notice::Allowed { tool, label } } else { Notice::Denied { tool, label } }));
        Some(card.ticket)
    }

    /// Folds one event in, at `at` (ms since 1970).
    pub fn fold(&mut self, event: Event, at: u64) {
        match event {
            Event::Started { session, model, mode } => {
                self.session = session.or(self.session.take());
                self.model = model.or(self.model.take());
                self.mode = mode.or(self.mode.take());
            }
            Event::TurnStarted => {
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
            Event::ToolStart { id, sub, tool, label, detail, diff } => {
                self.settle();
                if let Some(turn) = &mut self.turn {
                    turn.tool = Some(label.clone());
                }
                self.push(Entry::Tool { id, sub, tool, label, detail, state: Call::Running, diff });
            }
            Event::ToolEnd { id, failed } => {
                let call = self.items.iter_mut().rev().find_map(|item| match &mut item.entry {
                    Entry::Tool { id: of, state, .. } if *of == id => Some(state),
                    _ => None,
                });
                if let Some(state) = call {
                    *state = if failed { Call::Failed } else { Call::Done };
                }
                if let Some(turn) = &mut self.turn {
                    turn.tool = None;
                }
            }
            Event::Permission { ticket, tool, label, detail, len, cut, claim } => {
                if !self.cards.iter().any(|card| card.ticket == ticket) && self.cards.len() < MAX_CARDS {
                    self.settle();
                    self.cards.push_back(Card { ticket, tool, label, detail, len, cut, claim, expanded: false });
                }
            }
            Event::PermissionClosed { ticket } => self.cards.retain(|card| card.ticket != ticket),
            Event::AutoDenied { tool, label, reason, detail } => {
                self.push(Entry::AutoDenied { tool, label, reason, detail })
            }
            Event::Usage { context, window, cost_micro_usd } => {
                self.context = context.or(self.context);
                self.window = window.or(self.window);
                self.cost_micro_usd = cost_micro_usd.or(self.cost_micro_usd);
            }
            Event::Compaction => self.push(Entry::Notice(Notice::Compacted)),
            Event::TurnFinished { outcome } => {
                self.settle();
                self.end_calls();
                (self.turn, self.cards) = (None, VecDeque::new());
                match outcome {
                    Outcome::Completed => {}
                    Outcome::Interrupted => self.push(Entry::Notice(Notice::Interrupted)),
                    Outcome::Failed { kind } => self.push(Entry::Notice(Notice::Failed { kind })),
                }
            }
            Event::Error { kind, retrying } => self.push(Entry::Notice(Notice::Error { kind, retrying })),
            Event::Exited { code } => {
                self.settle();
                self.end_calls();
                (self.turn, self.cards, self.live) = (None, VecDeque::new(), false);
                self.push(Entry::Notice(Notice::Ended { code }));
            }
            // Only a watched transcript is rebuilt; a live one is the stream's.
            Event::WatchFromStart if !self.live => *self = Transcript { watched: true, ..Transcript::default() },
            Event::WatchFromStart => {}
            Event::MessageElsewhere => {
                self.settle();
                self.push(Entry::Notice(Notice::MessageElsewhere));
            }
        }
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Shape {
    User { bytes: usize },
    Assistant { bytes: usize, streaming: bool },
    Tool { tool: String, label: String, state: Call, sub: bool, diff: Option<DiffShape> },
    AutoDenied { tool: String, label: String, reason: String },
    Notice(Notice),
}

impl Shape {
    fn of(entry: &Entry) -> Self {
        match entry {
            Entry::User { text } => Shape::User { bytes: text.len() },
            Entry::Assistant { text, streaming } => Shape::Assistant { bytes: text.len(), streaming: *streaming },
            Entry::Tool { tool, label, state, sub, diff, .. } => Shape::Tool {
                tool: tool.clone(),
                label: label.clone(),
                state: *state,
                sub: *sub,
                diff: diff.as_ref().map(Diff::shape),
            },
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
                Entry::Notice(notice) => format!("{notice:?}"),
            })
            .collect()
    }

    fn permission(ticket: u64, cut: bool) -> Event {
        let (tool, label) = ("Bash".into(), "git".into());
        Event::Permission { ticket, tool, label, detail: text("command: git"), len: 12, cut, claim: Text::default() }
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
            sub: false,
            tool: "Bash".into(),
            label: "git".into(),
            detail: text("git status"),
            diff: None,
        };
        t.fold(tool, 15);
        assert_eq!(t.live_label(), Some("git"));
        t.fold(Event::ToolEnd { id: "a".into(), failed: false }, 16);
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
            sub: true,
            tool: "Read".into(),
            label: "a.rs".into(),
            detail: text("/a.rs"),
            diff: None,
        };
        t.fold(tool, 2);
        t.fold(permission(1, false), 3);
        t.fold(Event::TurnFinished { outcome: Outcome::Interrupted }, 4);
        assert_eq!(texts(&t), ["user:go", "tool:a.rs:Failed", "Interrupted"]);
        assert!(t.cards.is_empty(), "no request outlives its turn");
        t.fold(Event::Exited { code: Some(0) }, 5);
        assert!(!t.live && matches!(t.items().last().map(|i| &i.entry), Some(Entry::Notice(Notice::Ended { .. }))));
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
            sub: false,
            tool: "Edit".into(),
            label: "secret.rs".into(),
            detail: Text::default(),
            diff,
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
}
