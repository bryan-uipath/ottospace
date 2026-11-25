//! The native conversation in the drawer: transcripts folded from the runner's events, a
//! composer draft and a permission card per conversation, and the Chat | Terminal switch. Only
//! the user sends, answers, interrupts or switches, and only in the conversation shown.
//! Design: `docs/ai/agent-runners.md`.

use super::{Action, Effect, Model};
use crate::activity::Pulse;
use crate::agents;
use crate::ask::{self, Act, Form};
use crate::edit::Field;
use crate::runner::{RunnerId, Status, Support};
use crate::talk::{self, Chip, Subject};
use crate::tasks::{Group, Pasted, Queued, State};
use crate::transcript::{Event, Transcript, View};

/// Why a client's send, answer, interrupt or switch is refused.
pub const USERS_ALONE: &str = "only the user sends to, answers or interrupts a conversation's agent";
/// Why a client's report of an agent's events is refused: only ottospace's own runner reports.
pub const RUNNERS_ALONE: &str = "only ottospace's own runner reports an agent's events";
/// Why a client's collapse of a dial is refused: it is how the user keeps their drawer.
pub const DIAL_USERS: &str = "only the user collapses or opens a conversation's dial";

/// The most characters a composer draft holds.
pub const MAX_DRAFT: usize = 100_000;

/// The most characters a conversation's queued messages hold together: its task's file stays
/// small enough to read back (`config::load_tasks`).
pub const MAX_QUEUE: usize = 100_000;
/// Why ↵ queued nothing: the queue is full; the message stays in the composer.
pub const QUEUE_FULL: &str = "The queue is full: send or remove a queued message first. This one stays in the box";
/// Why Edit took nothing back: the composer has no room for it beside what is written.
pub const NO_ROOM: &str = "No room in the box for that message (or its images) beside what is written: it stays queued";

/// The most conversations whose transcript is kept: twice the sessions whose reads are kept.
const KEPT: usize = crate::talk::KEPT * 2;

/// One conversation drawn natively.
#[derive(Clone, Debug, PartialEq)]
pub struct Chat {
    pub subject: Subject,
    pub transcript: Transcript,
    /// The composer's text, kept while another conversation shows.
    pub draft: Field,
    /// The composer holds the user's words for a Deny: ↵ denies the card with them.
    pub deny: bool,
    /// Esc was pressed once while a turn ran: a second stops the turn.
    pub armed: bool,
    /// A message ↵ sent while its agent was not running: sent once the agent it started has.
    pub queued: Option<String>,
    /// Its btw column, while open (`btw`): side questions to its running agent.
    pub btw: Option<super::btw::Btw>,
    /// The draft holds a btw answer ("Use as my answer"): the model's words, never in the state.
    pub untrusted: bool,
    /// What ottospace added to its agent's run beside the user's words: its context dot.
    pub added: Option<agents::Added>,
    /// The first message's entry, and the user's line as text and tag pills (`compose::parts`).
    pub asked: Option<(u64, Vec<crate::compose::Part>)>,
    /// Images pasted in its composer, sent with its next message (or queued with it).
    pub images: Vec<Pasted>,
    /// Its task's queued messages wait for the user's ↵ (an empty composer sends the next): they
    /// stopped the turn, or the queue was read back after a restart. The queue is the task's
    /// (`Task::queued`, `Model::queue_of`).
    pub held: bool,
}

impl Chat {
    /// True if `ticket` is the card shown: an answer or Show all is for what the user saw.
    pub fn front_is(&self, ticket: u64) -> bool {
        self.transcript.cards.front().is_some_and(|card| card.ticket == ticket)
    }

    /// The question card shown, unless a Deny's message is written under it.
    pub fn form(&self) -> Option<&Form> {
        self.transcript.cards.front().and_then(|card| card.ask.as_ref()).filter(|_| !self.deny)
    }

    /// The cursor of the question card shown is on "Other…": typing goes there.
    pub fn answers_other(&self) -> bool {
        self.form().is_some_and(Form::on_other)
    }

    /// What `key` does in it: a question card's keys while one is shown, else `key`'s.
    pub fn key(&self, key: &str, mods: crate::keys::Mods) -> Key {
        if self.btw.as_ref().is_some_and(|btw| btw.keys) {
            return super::btw::Btw::key(key, mods);
        }
        if super::btw::is_key(key, mods) {
            return Key::Act(Action::BtwOpen);
        }
        let ticket = self.transcript.cards.front().map(|card| card.ticket).filter(|_| !self.deny);
        match (ticket, self.form()) {
            (Some(ticket), Some(form)) => ask_key(key, mods, ticket, form.on_other()),
            (ticket, _) => self::key(key, mods, ticket),
        }
    }

    fn new(subject: Subject) -> Self {
        let (transcript, draft) = (Transcript::default(), Field::lines(MAX_DRAFT));
        let (queued, btw, added, asked) = (None, None, None, None);
        let (held, images) = (false, Vec::new());
        Self {
            subject,
            transcript,
            draft,
            deny: false,
            armed: false,
            queued,
            btw,
            untrusted: false,
            added,
            asked,
            held,
            images,
        }
    }
}

/// The button at the composer's right (`Model::composer_button`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Primary {
    /// The turn: the session goes on (`TalkInterrupt`).
    Stop,
    /// `TalkStart`: go on with its session, or start its task's agent.
    Resume,
    Start,
}

/// A message that waited on a start back in the composer, unless the user wrote another since.
fn unqueue(chat: &mut Chat) {
    if let Some(text) = chat.queued.take().filter(|_| chat.draft.text().is_empty()) {
        chat.draft = Field::lines(MAX_DRAFT);
        chat.draft.apply(crate::edit::Edit::Paste(text));
    }
}

/// What a key does in a native conversation that has the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// Dispatch this action.
    Act(Action),
    /// ⇧↵ or ⌥↵: a new line in the draft.
    Newline,
    /// To the composer's field: an editing key, or text the input method types.
    Field,
    /// Nothing at all: a card has the keys, and this one is not among its answers.
    Swallow,
    /// Not the conversation's: a ⌘ shortcut goes on to its binding.
    Pass,
}

/// The key `key` with `mods`, while a card waits (`card`: its ticket, and no Deny's message is
/// written) or not.
/// On a card ↵ does nothing, so an Allow is never a reflex: `a` allows, `d` denies, `s` shows
/// all, `m` writes a message for the Deny. In the composer ↵ sends (or denies with the message),
/// ⇧↵ or ⌥↵ starts a new line, Esc leaves, or stops a turn when pressed twice. ⌃G, card or not: the
/// draft in the user's editor, or back to the editor that has it.
pub fn key(key: &str, mods: crate::keys::Mods, card: Option<u64>) -> Key {
    if mods.cmd {
        return Key::Pass;
    }
    let bare = !mods.ctrl && !mods.alt && !mods.shift;
    match key {
        "escape" => Key::Act(Action::TalkEscape),
        "g" if mods.ctrl && !mods.alt && !mods.shift => Key::Act(Action::EditorOpen),
        _ if let Some(ticket) = card => match key {
            "a" if bare => Key::Act(Action::TalkAnswer { allow: true, ticket }),
            "d" if bare => Key::Act(Action::TalkAnswer { allow: false, ticket }),
            "s" if bare => Key::Act(Action::TalkExpand { ticket }),
            "m" if bare => Key::Act(Action::TalkDenyNote),
            _ => Key::Swallow,
        },
        "enter" if mods.shift || (mods.alt && !mods.ctrl) => Key::Newline,
        "enter" if bare => Key::Act(Action::TalkSend),
        _ => Key::Field,
    }
}

/// A key on a question card (`typing`: the cursor is on an "Other…" row, whose words take the
/// letters, digits and space). ↑/↓ or j/k move, space picks, 1 to 9 pick that row, ⇥ / ⇧⇥ go
/// between questions, ↵ answers (and goes on, or sends), Esc declines.
pub fn ask_key(key: &str, mods: crate::keys::Mods, ticket: u64, typing: bool) -> Key {
    if mods.cmd {
        return Key::Pass;
    }
    let bare = !mods.ctrl && !mods.alt && !mods.shift;
    let act = |act| Key::Act(Action::TalkAsk { ticket, act });
    let digit = key.parse::<usize>().ok().filter(|n| key.len() == 1 && *n > 0);
    match key {
        "escape" => act(Act::Decline),
        "enter" if bare => act(Act::Submit),
        "up" => act(Act::Move { down: false }),
        "down" => act(Act::Move { down: true }),
        "tab" if !mods.ctrl && !mods.alt => act(Act::Tab { forward: !mods.shift }),
        _ if typing => Key::Field,
        "k" if bare => act(Act::Move { down: false }),
        "j" if bare => act(Act::Move { down: true }),
        "space" if bare => act(Act::Toggle),
        _ if let Some(n) = digit.filter(|_| bare) => act(Act::Number { n }),
        _ => Key::Swallow,
    }
}

impl Model {
    pub fn chat(&self, subject: &Subject) -> Option<&Chat> {
        self.chats.iter().find(|chat| chat.subject == *subject)
    }

    pub(super) fn chat_mut(&mut self, subject: &Subject) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|chat| chat.subject == *subject)
    }

    /// The conversation the open drawer shows, if it is drawn natively and its agent runs.
    pub fn shown_chat(&self) -> Option<&Chat> {
        let talk = self.talk.as_ref().filter(|talk| talk.open)?;
        self.chat(&talk.task).filter(|chat| chat.transcript.live)
    }

    pub(super) fn shown_chat_mut(&mut self) -> Option<&mut Chat> {
        let subject = self.talk.as_ref().filter(|talk| talk.open)?.task.clone();
        self.chat_mut(&subject).filter(|chat| chat.transcript.live)
    }

    /// The shown native conversation whose composer takes a message: its agent runs, or it
    /// ended and ↵ can go on with it (`talk_resumes`). Not a watched one.
    pub fn shown_draft_chat(&self) -> Option<&Chat> {
        let talk = self.talk.as_ref().filter(|talk| talk.open)?;
        let chat = self.chat(&talk.task)?;
        let native = self.talk_view(&talk.task) == View::Native;
        (chat.transcript.live || (native && !chat.transcript.watched && self.talk_resumes().is_some())).then_some(chat)
    }

    fn shown_draft_chat_mut(&mut self) -> Option<&mut Chat> {
        let subject = self.shown_draft_chat()?.subject.clone();
        self.chat_mut(&subject)
    }

    /// True if the keyboard may go into the conversation: its agent runs, or its native
    /// composer can go on with it.
    pub(super) fn talk_writable(&self, subject: &Subject) -> bool {
        let shown = self.talk.as_ref().is_some_and(|talk| talk.open && talk.task == *subject);
        self.talk_live(subject) || (shown && self.shown_draft_chat().is_some())
    }

    /// The card `ticket` first in its conversation's queue: the one shown. A Deny's message is
    /// for the card it was written under, so it ends.
    pub(super) fn card_first(&mut self, subject: &Subject, ticket: Option<u64>) {
        let Some(chat) = self.chat_mut(subject) else { return };
        let cards = &mut chat.transcript.cards;
        let at = ticket.and_then(|ticket| cards.iter().position(|card| card.ticket == ticket));
        if let Some(card) = at.filter(|at| *at > 0).and_then(|at| cards.remove(at)) {
            cards.push_front(card);
            chat.deny = false;
        }
    }

    /// The composer takes keys: the shown native conversation has the keyboard, no card waits
    /// (or the user writes a Deny's message), and the user's editor does not have its draft.
    /// Or a question card's "Other…" does: the editor has the draft, not it.
    pub fn talk_typing(&self) -> bool {
        let keys = self.talk.as_ref().is_some_and(|talk| talk.keys);
        let free = |chat: &Chat| {
            let btw = chat.btw.as_ref().is_some_and(|btw| btw.keys);
            !btw && (chat.answers_other()
                || ((chat.transcript.cards.is_empty() || chat.deny) && !self.edits(&chat.subject)))
        };
        keys && self.shown_draft_chat().is_some_and(free)
    }

    /// The open field's words are never in the state: a side question, the user's words to a
    /// question card, a draft that took a btw answer (the model's words).
    pub fn field_hidden(&self) -> bool {
        match self.field() {
            Some(("btw", _)) => true,
            // A repository's name may be a private one's: drawn, never in the state.
            Some(("clone", _)) => true,
            // A slot's title words are a window's title: drawn, never in the state.
            Some(("slot", _)) => true,
            // Its draft's chat: also an ended one the composer goes on with.
            Some(("talk", _)) => self.shown_draft_chat().is_some_and(|chat| chat.answers_other() || chat.untrusted),
            _ => false,
        }
    }

    /// The composer's draft; while a question card's cursor is on "Other…", its words.
    pub(super) fn talk_draft(&self) -> Option<&Field> {
        let chat = self.shown_draft_chat()?;
        match chat.form().filter(|form| form.on_other()) {
            Some(form) => form.other(form.cursor.0),
            None => Some(&chat.draft),
        }
    }

    /// An edit of `talk_draft`. Words typed on "Other…" pick it.
    pub(super) fn talk_edit(&mut self, edit: crate::edit::Edit) {
        let Some(chat) = self.shown_draft_chat_mut() else { return };
        let other = chat.answers_other();
        match chat.transcript.form_mut().filter(|_| other) {
            Some(form) => {
                if let Some(field) = form.typing() {
                    field.apply(edit);
                }
                form.typed();
            }
            None => drop(chat.draft.apply(edit)),
        }
    }

    /// The runner a conversation's agent is or would be: its session's, else its `@agent`'s.
    pub fn runner_of(&self, subject: &Subject) -> Option<RunnerId> {
        match subject {
            Subject::Session { runner, .. } => RunnerId::named(runner),
            Subject::Task(id) => {
                let task = self.tasks.get(*id)?;
                match &task.session {
                    Some(link) => RunnerId::named(&link.runner),
                    None => agents::read(task.agent.as_deref().unwrap_or(self.default_agent()), &self.models).runner,
                }
            }
        }
    }

    /// True if the conversation can be drawn natively: its runner's structured mode is verified
    /// (Claude Code's; Codex's app server is experimental and stays in a terminal).
    pub fn drawable(&self, subject: &Subject) -> bool {
        self.runner_of(subject).is_some_and(|runner| runner.spec().structured.status == Status::Verified)
    }

    /// How the drawer shows a conversation: the user's pick for it, else `conversation_view`
    /// if it can be drawn natively, else in a terminal.
    pub fn talk_view(&self, subject: &Subject) -> View {
        let picked = self.views.iter().find(|(of, _)| of == subject).map(|(_, view)| *view);
        let native = self.drawable(subject);
        picked.filter(|_| native).unwrap_or(if native { self.conversation_view } else { View::Terminal })
    }

    /// True if the shown conversation may switch views now: only one that can be drawn natively,
    /// and not while its native turn runs or a card waits (Stop ends the turn first). From the
    /// terminal at any time: the TUI is hung up on, as Stop does, whatever it was doing.
    pub fn talk_switchable(&self) -> bool {
        let Some(subject) = self.talk.as_ref().map(|talk| &talk.task) else { return false };
        let busy = self.chat(subject).is_some_and(|chat| chat.transcript.live && chat.transcript.busy());
        // Stopping: the stop stands; a switch now would start it again in the other view.
        self.drawable(subject) && !busy && !self.is_ending(subject)
    }

    /// An agent started: drawn natively, its conversation gets a live transcript (kept from
    /// before, if it had one), and the message that waited on it is sent. `resumed`: its
    /// session was gone on with.
    pub(super) fn chat_started(&mut self, subject: &Subject, resumed: bool, effects: &mut Vec<Effect>) {
        if self.talk_view(subject) != View::Native {
            // In a terminal now: a message that waited is not typed into its TUI; it goes back.
            if let Some(chat) = self.chat_mut(subject) {
                unqueue(chat);
            }
            return;
        }
        if self.chat(subject).is_none() {
            // Room for it: the oldest ended one goes.
            if self.chats.len() >= KEPT
                && let Some(at) = self.chats.iter().position(|chat| !chat.transcript.live)
            {
                self.chats.remove(at);
            }
            let chat = self.new_chat(subject);
            self.chats.push(chat);
        }
        // A new session is asked its task first: shown as the user's own line, its tags as pills
        // (`compose::parts`); what ottospace adds (`Task: `, the brief, the skills asked, its
        // guidance) is behind the context dot, as the start recorded it (`Model::adding`).
        let task = subject.task().and_then(|id| self.tasks.get(id));
        let line = task.map(|task| task.line.clone().unwrap_or_else(|| task.title.clone()));
        let parts = task.zip(line.as_deref()).map(|(task, line)| crate::compose::parts(line, task));
        let added = self.adding.iter().position(|(of, _)| of == subject).map(|at| self.adding.remove(at).1);
        let now = self.now;
        if let Some(chat) = self.chat_mut(subject) {
            chat.transcript.started(resumed);
            (chat.deny, chat.armed, chat.btw) = (false, false, None);
            if let Some(line) = line.filter(|_| !resumed) {
                chat.transcript.send(&line, now);
                chat.asked = chat.transcript.items().last().map(|item| item.id).zip(parts);
                chat.added = added;
            }
            if let Some(text) = chat.queued.take() {
                chat.transcript.send(&text, now);
                effects.push(sent(subject, text, std::mem::take(&mut chat.images)));
            }
        }
        self.send_queued(subject, effects);
    }

    /// The next queued message, if the conversation is idle: live, no turn, no card, not stopping,
    /// and not held by the user's stop. Each is a turn of its own; the next waits for its end.
    pub(super) fn send_queued(&mut self, subject: &Subject, effects: &mut Vec<Effect>) {
        let (now, ending, waiting) = (self.now, self.is_ending(subject), !self.queue_of(subject).is_empty());
        let Some(chat) = self.chat_mut(subject) else { return };
        if ending || chat.held || !chat.transcript.live || chat.transcript.busy() || !waiting {
            return;
        }
        let Some(next) = self.queue_mut(subject).map(|queue| queue.remove(0)) else { return };
        if let Some(chat) = self.chat_mut(subject) {
            chat.transcript.send(&next.text, now);
        }
        effects.push(sent(subject, next.text, next.images));
        self.chat_state(subject);
    }

    /// A conversation as it is first made: a task's queue read back (a restart) waits for the
    /// user's ↵, never sent unasked.
    fn new_chat(&self, subject: &Subject) -> Chat {
        let mut chat = Chat::new(subject.clone());
        chat.held = !self.queue_of(subject).is_empty();
        chat
    }

    /// The messages queued in `subject`'s conversation, oldest first: its task's. None for a
    /// session no task owns.
    pub fn queue_of(&self, subject: &Subject) -> &[Queued] {
        subject.task().and_then(|id| self.tasks.get(id)).map_or(&[], |task| &task.queued)
    }

    fn queue_mut(&mut self, subject: &Subject) -> Option<&mut Vec<Queued>> {
        let id = subject.task()?;
        self.tasks.list.iter_mut().find(|task| task.id == id).map(|task| &mut task.queued)
    }

    /// The composer emptied: what it held, its images too, was queued.
    fn clear_draft(&mut self, subject: &Subject) {
        if let Some(chat) = self.chat_mut(subject) {
            (chat.draft, chat.images) = (Field::lines(MAX_DRAFT), Vec::new());
        }
    }

    /// The images pasted in `subject`'s composer, for the message it sends.
    fn draft_images(&self, subject: &Subject) -> Vec<Pasted> {
        self.chat(subject).map(|chat| chat.images.clone()).unwrap_or_default()
    }

    /// Queues `text` in `subject`'s conversation, last. False: no task to keep it, or no room
    /// (the box says so); the words stay where they are.
    fn enqueue(&mut self, subject: &Subject, text: String, images: Vec<Pasted>, effects: &mut Vec<Effect>) -> bool {
        let used: usize = self.queue_of(subject).iter().map(|queued| queued.text.chars().count()).sum();
        if used + text.chars().count() > MAX_QUEUE {
            effects.push(Effect::Notice { text: QUEUE_FULL.into() });
            return false;
        }
        let queued = self.queued(text, images);
        self.queue_mut(subject).map(|queue| queue.push(queued)).is_some()
    }

    /// `text` as a queued message, with an id no other has had in this run, nor in the tasks read
    /// back.
    fn queued(&mut self, text: String, images: Vec<Pasted>) -> Queued {
        let kept = self.tasks.list.iter().flat_map(|task| task.queued.iter().map(|queued| queued.id)).max();
        let id = self.queue_next.max(kept.map_or(1, |id| id + 1));
        self.queue_next = id + 1;
        Queued { id, text, images }
    }

    /// What "send now" (⌘↵) does in `subject` while its turn runs: true, it stops the turn first
    /// (its runner takes no message mid-turn); false, the message goes into the turn.
    pub fn send_now_stops(&self, subject: &Subject) -> bool {
        !self.runner_of(subject).is_some_and(|runner| runner.spec().structured.mid_turn)
    }

    /// `text` now, in the shown conversation, by the user: into its turn where its runner takes
    /// that; else first in the queue, and the turn stopped (it goes once the turn has ended).
    /// Idle, it is simply sent.
    /// A card waits (a question first), or the agent stops: it goes first in the queue instead.
    fn send_now(&mut self, text: String, images: Vec<Pasted>, effects: &mut Vec<Effect>) {
        let Some(subject) = self.shown_chat().map(|chat| chat.subject.clone()) else { return };
        let (now, stops, ending) = (self.now, self.send_now_stops(&subject), self.is_ending(&subject));
        let Some(chat) = self.chat_mut(&subject) else { return };
        chat.held = false;
        let carded = !chat.transcript.cards.is_empty();
        let (running, owned) = (chat.transcript.turn.is_some(), subject.task().is_some());
        if !carded && !ending && ((running && !stops) || !owned) {
            chat.transcript.send(&text, now);
            effects.push(sent(&subject, text, images));
        } else if !owned {
            // No task to queue it: back in the composer, never lost.
            (chat.draft, chat.images) = (Field::lines_of(&text, MAX_DRAFT), images);
            return self.chat_state(&subject);
        } else {
            let queued = self.queued(text, images);
            if let Some(queue) = self.queue_mut(&subject) {
                queue.insert(0, queued);
            }
            if running && !carded && !ending {
                if let Some(chat) = self.chat_mut(&subject) {
                    chat.armed = false;
                }
                effects.push(Effect::AgentInterrupt { task: subject.clone() });
            }
        }
        self.send_queued(&subject, effects);
        self.chat_state(&subject);
    }

    /// Its agent is gone: its transcript stops being live. Returns whether the user switched it
    /// to the other view, and it is to be gone on with there.
    pub(super) fn chat_exited(&mut self, subject: &Subject) -> bool {
        if let Some(chat) = self.chat_mut(subject)
            && chat.transcript.live
        {
            chat.transcript.fold(crate::transcript::Event::Exited { code: None, signal: None, details: false }, 0);
        }
        // A btw is of its process: it ends with it.
        if let Some(chat) = self.chat_mut(subject) {
            (chat.deny, chat.armed, chat.btw) = (false, false, None);
        }
        let switching = self.switching.contains(subject);
        self.switching.retain(|of| of != subject);
        switching
    }

    /// A message written while its agent stopped: the session is gone on with, and it is sent
    /// once the agent runs (`chat_started`). Nothing to go on with: it is back in the composer.
    pub(super) fn chat_resume(&mut self, subject: &Subject, effects: &mut Vec<Effect>) {
        self.talk_to(subject.clone(), true, effects);
        if !self.talk_live(subject)
            && let Some(chat) = self.chat_mut(subject)
        {
            unqueue(chat);
        }
    }

    /// After a switch: the session is gone on with in the view picked, now that the other
    /// process is gone. Never two processes on one session.
    pub(super) fn chat_restart(&mut self, subject: Subject, effects: &mut Vec<Effect>) {
        if self.agent_runs(&subject) {
            return;
        }
        if let Some((start @ agents::Start::Resume { .. }, cwd)) = self.agent_start(&subject) {
            self.starting.push((subject.clone(), None));
            effects.push(self.start_effect(subject, start, cwd, None));
        }
    }

    /// An adopted session's conversation is its task's now: its transcript, view and switch too.
    pub(super) fn chat_moves(&mut self, from: &Subject, to: &Subject) {
        self.chats.retain(|chat| chat.subject != *to);
        self.views.retain(|(subject, _)| subject != to);
        self.dials.retain(|(subject, _)| subject != to);
        let rename = |subject: &mut Subject| {
            if subject == from {
                *subject = to.clone();
            }
        };
        self.chats.iter_mut().for_each(|chat| rename(&mut chat.subject));
        self.views.iter_mut().for_each(|(subject, _)| rename(subject));
        self.dials.iter_mut().for_each(|(subject, _)| rename(subject));
        self.switching.iter_mut().for_each(rename);
        self.editor_moves(from, to);
    }

    /// The repository a listed session is of: its links and images may be there.
    pub fn session_repo(&self, runner: &str, id: &str) -> Option<String> {
        self.session(runner, id).map(|found| found.repo.clone())
    }

    /// True if the conversation's agent was stopped for a switch of view and has not ended yet:
    /// the shell waits for its process to exit before it says so (`AgentExited`).
    pub fn is_switching(&self, subject: &Subject) -> bool {
        self.switching.contains(subject)
    }

    /// The one button at the right of the shown native conversation's composer: Stop while a
    /// turn runs, Resume (or Start) once its agent ended and it can be gone on with; none while
    /// it stops, while it idles, or in a terminal (its header has Stop and its body Start).
    pub fn composer_button(&self) -> Option<Primary> {
        let subject = &self.talk.as_ref().filter(|talk| talk.open)?.task;
        let chat = self.chat(subject).filter(|_| self.talk_view(subject) == View::Native)?;
        if self.is_ending(subject) || chat.transcript.watched {
            return None;
        }
        if chat.transcript.live {
            return chat.transcript.turn.is_some().then_some(Primary::Stop);
        }
        self.talk_resumes().map(|resumes| if resumes { Primary::Resume } else { Primary::Start })
    }

    /// True if the conversation's agent was asked to stop and its process has not exited yet:
    /// no Resume or Start until it has, and the composer says it is stopping.
    pub fn is_ending(&self, subject: &Subject) -> bool {
        self.ending.contains(subject)
    }

    /// The session to watch: the open drawer's conversation, when its agent does not run here,
    /// its session is at work or held elsewhere, and its runner's file can be followed live
    /// (`attach.watch`). The shell reads that file and sends what it says (`AgentEvents`):
    /// a read-only transcript, words included, as a chat shows it. Or a task's session nothing has, with
    /// none of it here (a restart since its agent ran): its past, read the same way, to go on with.
    pub fn watch_target(&self) -> Option<(Subject, RunnerId, String, String)> {
        let subject = self.talk.as_ref().filter(|talk| talk.open)?.task.clone();
        let runner = self.runner_of(&subject)?;
        if self.agent_runs(&subject) || runner.spec().attach.watch != Support::Yes {
            return None;
        }
        let (name, id) = self.session_of(&subject)?;
        if let Some(found) = self.facts(&name, &id).flatten().filter(|found| self.in_use(found)) {
            return Some((subject, runner, id, found.cwd.clone()));
        }
        let unread = self.chat(&subject).is_none_or(|chat| chat.transcript.past);
        let native = self.talk_view(&subject) == View::Native && !self.talk_live(&subject);
        let (start, cwd) = self.agent_start(&subject).filter(|_| subject.task().is_some() && unread && native)?;
        matches!(start, agents::Start::Resume { .. }).then_some((subject, runner, id, cwd?))
    }

    /// The folder a session no task has started in, as read (`Found::cwd`).
    pub fn session_dir(&self, runner: &str, id: &str) -> Option<String> {
        self.facts(runner, id).flatten().map(|found| found.cwd.clone())
    }

    /// The space on show is a session's (no task's) that no agent of ours runs: the sessions are
    /// read while it shows, whatever section is open (an overlay sidebar closes as its row opens),
    /// so its watch, its city and the counts hold.
    pub fn shows_session(&self) -> bool {
        let subject = self.talk.as_ref().map(|talk| &talk.task);
        subject.is_some_and(|subject| matches!(subject, Subject::Session { .. }) && !self.agent_runs(subject))
    }

    /// The conversation's transcript is followed from its session's file now (`watch_target`):
    /// "Watching" says so only while it is.
    pub fn watching(&self, subject: &Subject) -> bool {
        self.watch_target().is_some_and(|(watched, ..)| watched == *subject)
    }

    /// True if the session the drawer follows from its file (`watch_target`) is another
    /// program's now: read-only. Else it is a task's past, the user's to go on with.
    fn watched_elsewhere(&self, subject: &Subject) -> bool {
        let found = self.session_of(subject).and_then(|(runner, id)| self.facts(&runner, &id).flatten());
        found.is_some_and(|found| self.in_use(found))
    }

    /// True if the watched session may be taken over here now: idle, and its runner can go on
    /// with an idle session (`attach.take_over_when_idle`). Taking over is the user's click on
    /// it, which resumes it.
    pub fn take_over_ready(&self, subject: &Subject) -> bool {
        let idle =
            self.session_of(subject).and_then(|(r, i)| self.facts(&r, &i).flatten()).is_some_and(|s| !self.in_use(s));
        // Not listed nor read (the Tasks section closed): the click reads it first.
        let idle = idle || self.session_unread(subject);
        let can = self.runner_of(subject).is_some_and(|r| r.spec().attach.take_over_when_idle == Support::Yes);
        can && idle && !self.agent_runs(subject)
    }

    /// The views the user picked for tasks' conversations, for the layout file.
    pub(crate) fn saved_views(&self) -> Vec<(crate::tasks::TaskId, View)> {
        self.views.iter().filter_map(|(subject, view)| Some((subject.task()?, *view))).collect()
    }

    /// The views a layout file kept: each task's, as the user last picked it.
    pub(crate) fn restore_views(&mut self, views: Vec<(crate::tasks::TaskId, View)>) {
        for (task, view) in views {
            let subject = Subject::Task(task);
            self.views.retain(|(of, _)| *of != subject);
            self.views.push((subject, view));
        }
    }

    /// The agent chip: the selected space's conversation while it is closed and nothing else
    /// is in the drawer's place. What its agent does, with no clock in it.
    pub fn agent_chip(&self) -> Option<Chip> {
        let subject = &self.talk.as_ref().filter(|talk| !talk.open)?.task;
        if self.drawer_view().shows() || self.inspector.is_some() || self.listen.shown {
            return None;
        }
        let space = self.shown_spaces().iter().position(|space| space.of == *self.shown_space())?;
        let chat = self.chat(subject);
        // Its transcript says it while it is fed: drawn natively, or watched.
        let fed = chat.filter(|chat| chat.transcript.live || chat.transcript.watched);
        let pulse = match fed {
            Some(chat) => chat.transcript.pulse(),
            None if self.talk_working(subject) => Pulse::Running,
            None if self.agent_runs(subject) => Pulse::Idle,
            // Over: failed, if its transcript kept why.
            None => chat.map_or(Pulse::Ended, |chat| chat.transcript.pulse()),
        };
        let task = subject.task().and_then(|id| self.tasks.get(id));
        let needs = task.is_some_and(|task| task.group() == Some(Group::NeedsYou));
        // Alive and waiting, its task under Needs you: its turn is over, the next word is the user's.
        let state = if needs && pulse == Pulse::Idle { Pulse::NeedsYou } else { pulse };
        let label = fed.and_then(|chat| match pulse {
            Pulse::NeedsYou => chat.transcript.cards.front().map(|card| card.label.as_str()),
            Pulse::Running => chat.transcript.live_label(),
            _ => None,
        });
        Some(Chip { task: subject.clone(), space, state, line: talk::chip_line(state, label) })
    }

    /// The conversation's working line sweeps (`TaskRow::shimmer` for the drawer): a turn runs and
    /// nothing waits on the user, neither a card nor its task under Needs you.
    pub fn working_sweeps(&self, subject: &Subject) -> bool {
        let Some(chat) = self.chat(subject) else { return false };
        let task = subject.task().and_then(|id| self.tasks.get(id));
        let needs = task.is_some_and(|task| task.group() == Some(Group::NeedsYou));
        chat.transcript.turn.is_some() && chat.transcript.pulse() != Pulse::NeedsYou && !needs
    }

    /// True if the conversation's dial shows as its marker in the header: as the user left it,
    /// else open while its agent runs (or is watched) and collapsed once it has ended.
    pub fn dial_collapsed(&self, subject: &Subject) -> bool {
        let picked = self.dials.iter().find(|(of, _)| of == subject).map(|(_, collapsed)| *collapsed);
        let watched = self.chat(subject).is_some_and(|chat| chat.transcript.watched);
        picked.unwrap_or(!self.agent_runs(subject) && !watched)
    }

    /// The shown conversation's dial, the other way: the user's pick, kept for its task.
    pub(super) fn dial_toggle(&mut self) {
        let Some(subject) = self.talk.as_ref().filter(|talk| talk.open).map(|talk| talk.task.clone()) else { return };
        if self.chat(&subject).is_none() || self.talk_view(&subject) != View::Native {
            return;
        }
        let collapsed = !self.dial_collapsed(&subject);
        self.dials.retain(|(of, _)| *of != subject);
        self.dials.push((subject, collapsed));
    }

    /// The dials the user collapsed or opened, by task, for the layout file.
    pub(crate) fn saved_dials(&self) -> Vec<(crate::tasks::TaskId, bool)> {
        self.dials.iter().filter_map(|(subject, collapsed)| Some((subject.task()?, *collapsed))).collect()
    }

    pub(crate) fn restore_dials(&mut self, dials: Vec<(crate::tasks::TaskId, bool)>) {
        for (task, collapsed) in dials {
            let subject = Subject::Task(task);
            self.dials.retain(|(of, _)| *of != subject);
            self.dials.push((subject, collapsed));
        }
    }

    /// The conversations gone with their tasks take their transcripts with them. A message that
    /// waited on a start that did not come (it failed, was stopped, or its session was busy when
    /// read) goes back to its composer.
    pub(super) fn settle_chats(&mut self) {
        let waiting = self.chats.iter().filter(|c| c.queued.is_some() && !self.talk_live(&c.subject));
        let waiting: Vec<Subject> = waiting.map(|c| c.subject.clone()).collect();
        for subject in waiting {
            if let Some(chat) = self.chat_mut(&subject) {
                unqueue(chat);
            }
        }
        // A btw only where one is offered (its task's runner restored as one that takes none, say);
        // its input never has the keys while the user's editor has a draft.
        let btws: Vec<(Subject, bool)> = self
            .chats
            .iter()
            .filter(|c| c.btw.is_some())
            .map(|c| (c.subject.clone(), self.btw_offered(&c.subject)))
            .collect();
        let editing = self.editing.is_some();
        for (subject, offered) in btws {
            let Some(chat) = self.chat_mut(&subject) else { continue };
            match &mut chat.btw {
                Some(_) if !offered => chat.btw = None,
                Some(btw) if editing => btw.keys = false,
                _ => {}
            }
        }
        // A draft that took a btw answer keeps it out of the state until it is empty again, and
        // not waiting on a start (`unqueue` may put it back). A fresh field: ⌘Z brings none back.
        let emptied = |chat: &&mut Chat| chat.untrusted && chat.draft.text().is_empty() && chat.queued.is_none();
        for chat in self.chats.iter_mut().filter(emptied) {
            (chat.untrusted, chat.draft) = (false, Field::lines(MAX_DRAFT));
        }
        let tasks = &self.tasks;
        let gone = |subject: &Subject| subject.task().is_some_and(|id| tasks.get(id).is_none());
        self.chats.retain(|chat| !gone(&chat.subject));
        self.views.retain(|(subject, _)| !gone(subject));
        self.dials.retain(|(subject, _)| !gone(subject));
        self.switching.retain(|subject| !gone(subject));
    }

    pub(super) fn chat_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        let shown = self.talk.as_ref().filter(|talk| talk.open).map(|talk| talk.task.clone());
        match action {
            // The shell's runner alone reports events: a key or click of the user's is none.
            Action::AgentEvents { .. } if self.by_user => {}
            // A watched session's file: what it shows, read-only. No turn, no card: nothing here
            // can be answered or stopped, and nothing reaches its agent.
            Action::AgentEvents { task: subject, events, at }
                if !self.chat(&subject).is_some_and(|chat| chat.transcript.live) =>
            {
                if !self.watching(&subject) {
                    return;
                }
                if self.chat(&subject).is_none() {
                    self.chats.retain(|chat| chat.transcript.live || chat.subject != subject);
                    if self.chats.len() >= KEPT
                        && let Some(at) = self.chats.iter().position(|chat| !chat.transcript.live)
                    {
                        self.chats.remove(at);
                    }
                    let chat = self.new_chat(&subject);
                    self.chats.push(chat);
                }
                let elsewhere = self.watched_elsewhere(&subject);
                let Some(chat) = self.chat_mut(&subject) else { return };
                (chat.transcript.watched, chat.transcript.past) = (elsewhere, !elsewhere);
                for event in events.into_iter().filter(Event::shows_when_watched) {
                    chat.transcript.fold(event, at);
                }
            }
            Action::AgentEvents { task: subject, events, at } => {
                let Some(chat) = self.chat_mut(&subject).filter(|chat| chat.transcript.live) else { return };
                // A turn's start or end, or a request's, may cancel out within one batch: the task's
                // state is set again after any of them, not only when the end state differs.
                let turns = events.iter().any(|event| {
                    matches!(
                        event,
                        Event::TurnStarted
                            | Event::TurnFinished { .. }
                            | Event::Permission { .. }
                            | Event::PermissionClosed { .. }
                            | Event::Exited { .. }
                    )
                });
                let front = chat.transcript.cards.front().map(|card| card.ticket);
                let finished = events.iter().any(|event| matches!(event, Event::TurnFinished { .. }));
                for event in events {
                    // An agent that exited in this batch asks nothing and starts no turn: nothing to answer or stop.
                    if chat.transcript.live || !matches!(event, Event::Permission { .. } | Event::TurnStarted) {
                        chat.transcript.fold(event, at);
                    }
                }
                // A turn that ended in this batch takes its Esc with it, also when the next starts.
                chat.armed &= chat.transcript.turn.is_some() && !finished;
                // A Deny's message is for the card it was written under: another one in front ends it.
                chat.deny &= chat.transcript.cards.front().map(|card| card.ticket) == front;
                if turns {
                    self.chat_state(&subject);
                    self.send_queued(&subject, effects);
                }
            }
            // ↵ while the user's editor has the draft: nothing is sent.
            Action::TalkSend if shown.as_ref().is_some_and(|subject| self.edits(subject)) => {}
            // ↵ while its agent stops: the message waits; once the process is gone the session is
            // gone on with and it is sent (`AgentExited`). Never sent to the one stopping.
            Action::TalkSend if self.talk.as_ref().is_some_and(|talk| talk.open && self.is_ending(&talk.task)) => {
                let Some(chat) = self.shown_draft_chat_mut() else { return };
                let text = chat.draft.text().to_string();
                let subject = chat.subject.clone();
                if text.trim().is_empty() || chat.queued.is_some() {
                    return;
                }
                // Messages wait already: behind them, in order.
                if !self.queue_of(&subject).is_empty() {
                    if self.enqueue(&subject, text, self.draft_images(&subject), effects) {
                        self.clear_draft(&subject);
                    }
                    return;
                }
                let Some(chat) = self.chat_mut(&subject) else { return };
                (chat.queued, chat.draft) = (Some(text), Field::lines(MAX_DRAFT));
            }
            // ↵ while the user writes a Deny's message: the Deny, with it.
            Action::TalkSend if self.shown_chat().is_some_and(|chat| chat.deny) => {
                let front = self.shown_chat().and_then(|chat| chat.transcript.cards.front().map(|card| card.ticket));
                if let Some(ticket) = front {
                    self.chat_act(Action::TalkAnswer { allow: false, ticket }, effects)
                }
            }
            // ↵ in an ended conversation: its agent goes on (or starts), then gets the message.
            Action::TalkSend if self.shown_chat().is_none() => {
                let Some(chat) = self.shown_draft_chat_mut() else { return };
                let text = chat.draft.text().to_string();
                let subject = chat.subject.clone();
                if text.trim().is_empty() || chat.queued.is_some() {
                    return;
                }
                // Messages wait already: this one behind them; they go in order once it runs.
                if !self.queue_of(&subject).is_empty() {
                    if !self.enqueue(&subject, text, self.draft_images(&subject), effects) {
                        return;
                    }
                    self.clear_draft(&subject);
                    if let Some(chat) = self.chat_mut(&subject) {
                        chat.held = false;
                    }
                } else if let Some(chat) = self.chat_mut(&subject) {
                    (chat.queued, chat.draft) = (Some(text), Field::lines(MAX_DRAFT));
                }
                self.talk_to(subject, true, effects);
            }
            Action::TalkSend => {
                let now = self.now;
                let Some(chat) = self.shown_chat_mut() else { return };
                let text = chat.draft.text().to_string();
                let subject = chat.subject.clone();
                // An empty composer: the queue the user's stop held goes on.
                if text.trim().is_empty() {
                    chat.held = false;
                    return self.send_queued(&subject, effects);
                }
                // A turn runs, a card waits, or messages wait already: this one waits behind them
                // (a hold stays: only the empty ↵ lets them go). No task to keep it: sent now.
                let (busy, steers) =
                    (chat.transcript.busy(), chat.transcript.turn.is_some() && chat.transcript.cards.is_empty());
                if steers && subject.task().is_none() && !self.send_now_stops(&subject) {
                    return self.chat_act(Action::TalkSendNow, effects);
                }
                if busy || !self.queue_of(&subject).is_empty() {
                    if self.enqueue(&subject, text, self.draft_images(&subject), effects) {
                        self.clear_draft(&subject);
                    }
                    return self.send_queued(&subject, effects);
                }
                let Some(chat) = self.chat_mut(&subject) else { return };
                chat.transcript.send(&text, now);
                chat.draft = Field::lines(MAX_DRAFT);
                let subject = chat.subject.clone();
                effects.push(sent(&subject, text, std::mem::take(&mut chat.images)));
                self.chat_state(&subject);
            }
            Action::TalkSendNow if shown.as_ref().is_some_and(|subject| self.edits(subject)) => {}
            Action::TalkSendNow => {
                let Some(chat) = self.shown_chat_mut().filter(|chat| !chat.deny) else { return };
                let text = chat.draft.text().to_string();
                if text.trim().is_empty() {
                    return;
                }
                chat.draft = Field::lines(MAX_DRAFT);
                let images = std::mem::take(&mut chat.images);
                self.send_now(text, images, effects);
            }
            // Not while the user's editor has the draft: an edit would be written over by its save.
            Action::QueueEdit { .. } if shown.as_ref().is_some_and(|subject| self.edits(subject)) => {}
            // By the row's id: a row the queue has moved on from (sent meanwhile) is no other row.
            act @ (Action::QueueRemove { id } | Action::QueueEdit { id } | Action::QueueSendNow { id }) => {
                let Some(subject) = self.shown_chat().map(|chat| chat.subject.clone()) else { return };
                let Some(index) = self.queue_of(&subject).iter().position(|queued| queued.id == id) else { return };
                let Queued { text, images, .. } = self.queue_of(&subject)[index].clone();
                match act {
                    // Back in the composer, after what is written there; if all of it fits. Hidden
                    // from the state as a btw answer is: it may have been one.
                    Action::QueueEdit { .. } => {
                        let Some(chat) = self.chat_mut(&subject) else { return };
                        let before = chat.draft.text().trim_end().to_string();
                        let joined = if before.is_empty() { text } else { format!("{before}\n{text}") };
                        if joined.chars().count() > MAX_DRAFT || chat.images.len() + images.len() > MAX_IMAGES {
                            return effects.push(Effect::Notice { text: NO_ROOM.into() });
                        }
                        (chat.draft, chat.untrusted) = (Field::lines(MAX_DRAFT), true);
                        chat.draft.apply(crate::edit::Edit::Paste(joined));
                        chat.images.extend(images);
                        self.queue_mut(&subject).map(|queue| queue.remove(index));
                    }
                    Action::QueueRemove { .. } => drop(self.queue_mut(&subject).map(|queue| queue.remove(index))),
                    _ => {
                        self.queue_mut(&subject).map(|queue| queue.remove(index));
                        self.send_now(text, images, effects);
                    }
                }
            }
            Action::ImagePaste { id, kind } => self.paste_image(&id, &kind, effects),
            Action::ImageRemove { id } => {
                self.compose_images.retain(|image| image.id() != id);
                if let Some(chat) = self.shown_chat_mut() {
                    chat.images.retain(|image| image.id() != id);
                }
            }
            // Not delivered (its agent had ended): first in its queue again, held for the user's ↵
            // (its row says so; no notice: the shell's news interrupts nobody).
            Action::AgentSendFailed { .. } if self.by_user => {}
            Action::AgentSendFailed { task: subject, text, images } => {
                if subject.task().is_some() {
                    let queued = self.queued(text, images);
                    if let Some(queue) = self.queue_mut(&subject) {
                        queue.insert(0, queued);
                    }
                    if let Some(chat) = self.chat_mut(&subject) {
                        chat.held = true;
                    }
                } else if let Some(chat) = self.chat_mut(&subject) {
                    // No task to queue it: back in the composer, after what is written there.
                    let before = chat.draft.text().trim_end().to_string();
                    let joined = if before.is_empty() { text } else { format!("{before}\n{text}") };
                    chat.draft = Field::lines_of(&joined, MAX_DRAFT);
                    chat.images.extend(images);
                }
            }
            Action::TalkAnswer { allow, ticket } => {
                let Some(chat) = self.shown_chat_mut().filter(|chat| chat.front_is(ticket)) else { return };
                let words = chat.draft.text().trim().to_string();
                let message = (!allow && chat.deny && !words.is_empty()).then_some(words);
                let Some(answered) = chat.transcript.answer(allow) else { return };
                debug_assert_eq!(answered, ticket);
                if message.is_some() {
                    chat.draft = Field::lines(MAX_DRAFT);
                }
                chat.deny = false;
                let subject = chat.subject.clone();
                effects.push(Effect::AgentAnswer { task: subject.clone(), ticket, allow, message });
                self.chat_state(&subject);
            }
            Action::TalkExpand { ticket } => {
                if let Some(chat) = self.shown_chat_mut().filter(|chat| chat.front_is(ticket)) {
                    chat.transcript.expand();
                }
            }
            Action::TalkAsk { ticket, act } => {
                if let Some(subject) = self.shown_chat().map(|chat| chat.subject.clone()) {
                    self.ask_act(&subject, ticket, act, effects);
                }
            }
            // Not while the editor has the draft: the message would have nowhere to be typed.
            Action::TalkDenyNote if shown.as_ref().is_some_and(|subject| self.edits(subject)) => {}
            Action::TalkDenyNote => {
                if let Some(chat) = self.shown_chat_mut().filter(|chat| !chat.transcript.cards.is_empty()) {
                    chat.deny = true;
                }
            }
            Action::TalkEscape => {
                let Some(chat) = self.shown_chat_mut() else { return self.talk_keys(false, effects) };
                if std::mem::take(&mut chat.deny) {
                    return;
                }
                if chat.transcript.turn.is_none() {
                    return self.talk_keys(false, effects);
                }
                // Esc, Esc: the turn stops, and what is queued waits for the user. One Esc only says so.
                chat.armed = !chat.armed;
                if !chat.armed {
                    chat.held = true;
                    effects.push(Effect::AgentInterrupt { task: chat.subject.clone() });
                }
            }
            Action::TalkInterrupt => {
                if let Some(chat) = self.shown_chat_mut().filter(|chat| chat.transcript.turn.is_some()) {
                    (chat.armed, chat.held) = (false, true);
                    effects.push(Effect::AgentInterrupt { task: chat.subject.clone() });
                }
            }
            Action::TalkView { shows } => {
                let Some(subject) = shown.filter(|subject| self.talk_view(subject) != shows) else { return };
                if !self.talk_switchable() {
                    return;
                }
                self.views.retain(|(of, _)| *of != subject);
                self.views.push((subject.clone(), shows));
                // The process in the other view ends first; the session goes on in this one after.
                if self.agent_runs(&subject) && !self.switching.contains(&subject) {
                    self.switching.push(subject.clone());
                    effects.push(Effect::StopAgent { task: subject });
                }
            }
            _ => {}
        }
    }

    /// A key or click on `subject`'s question card `ticket`, while its agent runs: a move or a
    /// pick, ↵ (the answers go) or Esc (declined). The drawer's (`TalkAsk`) or Decisions' card's.
    pub(super) fn ask_act(&mut self, subject: &Subject, ticket: u64, act: Act, effects: &mut Vec<Effect>) {
        let waits = |chat: &&mut Chat| chat.transcript.live && chat.front_is(ticket) && !chat.deny;
        let Some(chat) = self.chat_mut(subject).filter(waits) else { return };
        match act {
            Act::Decline if chat.transcript.form_mut().is_some() => {
                chat.transcript.answer(false);
                let message = Some(ask::DECLINED.to_string());
                effects.push(Effect::AgentAnswer { task: subject.clone(), ticket, allow: false, message });
            }
            Act::Submit => {
                let Some((_, answers)) = chat.transcript.reply() else { return };
                effects.push(Effect::AgentAnswers { task: subject.clone(), ticket, answers });
            }
            act => {
                if let Some(form) = chat.transcript.form_mut() {
                    form.act(act);
                }
                return;
            }
        }
        self.chat_state(subject);
    }

    /// The task follows its native agent: running while a turn runs, needs you while a request
    /// waits or once the turn is over.
    fn chat_state(&mut self, subject: &Subject) {
        let (Some(task), Some(chat)) = (subject.task(), self.chat(subject)) else { return };
        // Done or put away by the user: what its stopping agent says last moves it nowhere.
        if self.put_away(task) {
            return;
        }
        let transcript = &chat.transcript;
        let state =
            if transcript.turn.is_some() && transcript.cards.is_empty() { State::Running } else { State::NeedsYou };
        if self.tasks.get(task).is_some_and(|task| task.state != state) {
            self.tasks.set_state(task, state);
        }
    }
}

/// The most images in one message.
pub const MAX_IMAGES: usize = 5;
/// Why a pasted image was not taken: where it would go, nothing takes images.
pub const IMAGES_NOT_HERE: &str =
    "This agent takes no images here: they go to Claude Code in Chat view. Nothing was attached";
/// Why: the message has as many as it may.
pub const TOO_MANY_IMAGES: &str = "At most 5 images in a message: nothing was attached";
/// Why: a capture's title in the bar is one line, with no attachments.
pub const NO_IMAGES_BAR: &str = "A capture's title takes no image: attach it in the new-task box (⌘N)";

impl Model {
    /// A pasted image, for the message being written: the new-task box's while it has the keys,
    /// else the shown composer's; only where its agent takes images and draws natively. Refused,
    /// its file goes at once and the box says why.
    fn paste_image(&mut self, id: &str, kind: &str, effects: &mut Vec<Effect>) {
        let Some(image) = Pasted::new(id, kind) else { return };
        let why = if self.composing() {
            let takes = self.box_takes_images();
            match () {
                _ if self.bar_draft().is_some() => Some(NO_IMAGES_BAR),
                _ if !takes => Some(IMAGES_NOT_HERE),
                _ if self.compose_images.len() >= MAX_IMAGES => Some(TOO_MANY_IMAGES),
                _ => return self.compose_images.push(image),
            }
        } else if let Some(subject) =
            self.shown_draft_chat().map(|chat| chat.subject.clone()).filter(|_| self.talk_typing())
        {
            let takes = self.runner_of(&subject).is_some_and(|runner| runner.spec().structured.images)
                && self.talk_view(&subject) == View::Native;
            let full = self.chat(&subject).is_some_and(|chat| chat.images.len() >= MAX_IMAGES);
            match () {
                _ if !takes => Some(IMAGES_NOT_HERE),
                _ if full => Some(TOO_MANY_IMAGES),
                _ => return drop(self.chat_mut(&subject).map(|chat| chat.images.push(image))),
            }
        } else {
            None
        };
        if let Some(why) = why {
            effects.push(Effect::Notice { text: why.into() });
        }
        effects.push(Effect::ImagesDrop { images: vec![image] });
    }

    /// The images pasted in the new-task box, for the task it files.
    pub fn compose_images(&self) -> &[Pasted] {
        &self.compose_images
    }

    /// The new-task box's agent, as its line has it now, takes images: it is drawn natively and
    /// its runner takes them. Asked at the paste, and again when the box files.
    pub(super) fn box_takes_images(&self) -> bool {
        let agent = self.compose_plan().map(|plan| plan.agent).unwrap_or_default();
        agents::read(&agent, &self.models).runner.is_some_and(|runner| {
            let structured = &runner.spec().structured;
            structured.images && structured.status == Status::Verified && self.conversation_view == View::Native
        })
    }

    /// True if something holds `image` now: a drop decided earlier in a step (before a failed
    /// send put it back) leaves its file.
    pub fn holds_image(&self, image: &Pasted) -> bool {
        self.images_held.contains(image)
    }

    /// Every pasted image something still holds: the box's, a composer's, a queued message's, a
    /// task's not done. One nothing holds any more is dropped (its file removed), once.
    pub(super) fn settle_images(&mut self, effects: &mut Vec<Effect>) {
        if self.compose.is_none() {
            self.compose_images.clear();
        }
        // A done task's may come back with it (undone): kept until it is cleaned away, or gone.
        let tasks = self.tasks.list.iter().filter(|task| task.state != State::Cleaned);
        let queued = self.tasks.list.iter().flat_map(|task| task.queued.iter().flat_map(|q| q.images.iter()));
        let held: std::collections::BTreeSet<Pasted> = self
            .compose_images
            .iter()
            .chain(self.compose_kept.iter().flat_map(|kept| kept.images.iter()))
            .chain(self.chats.iter().flat_map(|chat| chat.images.iter()))
            .chain(tasks.flat_map(|task| task.images.iter()))
            .chain(queued)
            .cloned()
            .collect();
        let gone: Vec<Pasted> = self.images_held.difference(&held).cloned().collect();
        if !gone.is_empty() {
            effects.push(Effect::ImagesDrop { images: gone });
        }
        self.images_held = held;
    }
}

/// The send effect for `text` and its images: `AgentSendWith` only when there are some.
fn sent(task: &Subject, text: String, images: Vec<Pasted>) -> Effect {
    match images.is_empty() {
        true => Effect::AgentSend { task: task.clone(), text },
        false => Effect::AgentSendWith { task: task.clone(), text, images },
    }
}

#[cfg(test)]
mod tests {
    use crate::model::chat::Key;
    use crate::model::tests::started;
    use crate::model::{Action, Effect, Model, Origin};
    use crate::talk::Subject;
    use crate::tasks::State;
    use crate::transcript::{Event, Outcome, Text, View};

    /// A model with two tasks whose Claude agents run natively, the first shown.
    fn add(m: &mut Model, title: &str) {
        let assign = crate::tasks::Assign::default();
        m.update(Action::TaskAdd { title: title.into(), priority: 3, state: State::Triaged, tab: None, assign });
    }

    fn two_live() -> Model {
        let mut m = Model::default();
        for title in ["fix login", "write docs"] {
            add(&mut m, title);
        }
        let ids: Vec<u32> = m.tasks.list.iter().map(|task| task.id).collect();
        for id in ids.iter().rev() {
            started(&mut m, Action::TaskTalk { id: *id });
            let started = Action::AgentStarted { task: Subject::Task(*id), session: None, at: 1 };
            m.update(started.sent_by(Origin::System));
        }
        // The first prompts' turns are over.
        for id in ids {
            events(&mut m, &Subject::Task(id), vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        }
        m
    }

    fn first(m: &Model) -> Subject {
        Subject::Task(m.tasks.list[0].id)
    }

    fn second(m: &Model) -> Subject {
        Subject::Task(m.tasks.list[1].id)
    }

    fn events(m: &mut Model, subject: &Subject, events: Vec<Event>) -> Vec<Effect> {
        m.update(Action::AgentEvents { task: subject.clone(), events, at: 5 }.sent_by(Origin::System))
    }

    /// The words of `subject`'s queued messages, in order.
    fn texts(m: &Model, subject: &Subject) -> Vec<String> {
        m.queue_of(subject).iter().map(|queued| queued.text.clone()).collect()
    }

    fn typed(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Replace { range: None, text: text.into() } });
    }

    fn card(ticket: u64, cut: bool) -> Event {
        let (tool, label, detail) = ("Bash".into(), "git".into(), Text::new("command: git"));
        Event::Permission { ticket, tool, label, detail, len: 12, cut, claim: Text::default(), questions: Vec::new() }
    }

    /// The dial follows its agent's calls and the user's message; it is open while the agent
    /// runs and collapsed once it ended, unless the user left it otherwise.
    #[test]
    fn a_dial_follows_calls_and_folds_away_once_its_agent_ended() {
        use crate::activity::{Pulse, Station};
        let mut m = two_live();
        let one = first(&m);
        let call = |id: &str, station| Event::ToolStart {
            id: id.into(),
            sub: false,
            tool: "Bash".into(),
            station,
            label: "cargo test".into(),
            detail: Text::default(),
            diff: None,
            gist: None,
        };
        let run = vec![
            Event::TurnStarted,
            call("a", Station::Shell),
            Event::ToolEnd { id: "a".into(), failed: true, output: None },
        ];
        events(&mut m, &one, [run, vec![Event::Hook, call("b", Station::Network)]].concat());
        let dial = m.chat(&one).map(|chat| chat.transcript.dial(6)).unwrap();
        let count = |s| dial.spot(s).map(|spot| (spot.calls, spot.failing, spot.lit));
        assert_eq!([count(Station::Shell), count(Station::Network)], [Some((1, 1, false)), Some((1, 0, true))]);
        assert_eq!((count(Station::Hooks), count(Station::You)), (Some((1, 0, false)), Some((1, 0, false))));
        assert_eq!((dial.pulse, dial.failed, dial.lit), (Pulse::Running, true, true));
        assert!(!m.dial_collapsed(&one), "open while it runs");
        // The user's next message clears the red.
        events(&mut m, &one, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        typed(&mut m, "again");
        m.update(Action::TalkSend);
        assert!(!m.chat(&one).map(|chat| chat.transcript.dial(7)).unwrap().failed);
        // Toggled: the user's alone, until toggled again; an ended one is collapsed by default.
        assert_eq!(m.refusal(&Action::DialToggle, Origin::Socket), Some(super::DIAL_USERS));
        m.update(Action::DialToggle);
        assert!(m.dial_collapsed(&one));
        m.update(Action::AgentExited { task: second(&m) }.sent_by(Origin::System));
        assert!(m.dial_collapsed(&second(&m)), "ended: collapsed");
        m.update(Action::DialToggle);
        assert!(!m.dial_collapsed(&one) && m.dial_collapsed(&second(&m)), "only the shown one");
    }

    /// The chip is there only while the selected space's conversation is closed and nothing
    /// else is in the drawer's place; it says what the agent does, and that it needs the user.
    #[test]
    fn the_agent_chip_says_what_the_agent_of_a_closed_conversation_does() {
        use crate::activity::{Pulse, Station};
        let mut m = two_live();
        let one = first(&m);
        let chip = |m: &Model| m.agent_chip().map(|chip| (chip.state, chip.line));
        let said = |state, line: &str| Some((state, line.to_string()));
        assert_eq!(chip(&m), None, "open: no chip");
        m.update(Action::TalkToggle);
        // Its turn over, its task under Needs you: the next word is the user's.
        assert_eq!(chip(&m), said(Pulse::NeedsYou, "Needs you"));
        assert_eq!(m.agent_chip().map(|chip| chip.task), Some(one.clone()));
        // A turn: the call it waits on, cleaned and cut; then the turn alone.
        let label = format!("cargo\u{1b}[31m test {}", "x".repeat(100));
        let call = Event::ToolStart {
            id: "a".into(),
            sub: false,
            tool: "Bash".into(),
            station: Station::Shell,
            label,
            detail: Text::default(),
            diff: None,
            gist: None,
        };
        events(&mut m, &one, vec![Event::TurnStarted, call]);
        let (state, line) = chip(&m).unwrap();
        assert!(state == Pulse::Running && line.starts_with("Working · cargo test xx"), "{line:?}");
        assert_eq!(line.chars().count(), "Working · ".chars().count() + crate::talk::CHIP_LABEL);
        events(&mut m, &one, vec![Event::ToolEnd { id: "a".into(), failed: false, output: None }]);
        assert_eq!(chip(&m), said(Pulse::Running, "Working"));
        // A request waits: needs you, with what it asks for.
        events(&mut m, &one, vec![card(1, false)]);
        assert_eq!(chip(&m), said(Pulse::NeedsYou, "Needs you · git"));
        // The new task in the drawer's place: no chip under it.
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!(chip(&m), None);
        m.update(Action::CloseCompose);
        assert_eq!(chip(&m), said(Pulse::NeedsYou, "Needs you · git"));
        // Its process gone: ended. Workspace has no conversation: no chip.
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        assert_eq!(chip(&m).map(|(_, line)| line), Some("Ended".to_string()));
        m.update(Action::SpaceSelect { index: 0 });
        assert_eq!((chip(&m), m.talk.as_ref()), (None, None));
    }

    /// A running task's row: while no call runs it thinks (its turn's start, for the verb), and
    /// its line sweeps; a call is not thinking; a task that needs you neither thinks nor sweeps.
    #[test]
    fn a_running_rows_line_thinks_and_sweeps_and_one_that_needs_you_does_not() {
        use crate::activity::Station;
        let mut m = two_live();
        let (one, id) = (first(&m), m.tasks.list[0].id);
        let row = |m: &Model| {
            let rows = m.task_sections(9).into_iter().flat_map(|section| section.rows);
            let row = rows.into_iter().find(|row| row.id == id);
            row.map(|row| (row.thinking, row.live, row.shimmer, m.working_sweeps(&one))).unwrap_or_default()
        };
        events(&mut m, &one, vec![Event::TurnStarted]);
        m.update(Action::TaskSetState { id, state: State::Running }.sent_by(Origin::System));
        assert_eq!(row(&m), (Some(5), None, true, true), "the turn's start");
        let call = |id: &str, label: &str| Event::ToolStart {
            id: id.into(),
            sub: false,
            tool: "Bash".into(),
            station: Station::Shell,
            label: label.into(),
            detail: Text::default(),
            diff: None,
            gist: None,
        };
        // Two calls; the newer ends first: the turn waits on the other, not on the model.
        events(&mut m, &one, vec![call("a", "cargo test"), call("b", "git status")]);
        assert_eq!(row(&m), (None, Some("git status".into()), true, true), "a call runs: not thinking");
        events(&mut m, &one, vec![Event::ToolEnd { id: "b".into(), failed: false, output: None }]);
        assert_eq!(row(&m), (None, Some("cargo test".into()), true, true));
        events(&mut m, &one, vec![Event::ToolEnd { id: "a".into(), failed: false, output: None }]);
        assert_eq!(row(&m), (Some(5), None, true, true));
        // A call with a blank label says nothing: thinking.
        events(&mut m, &one, vec![call("c", " \n ")]);
        assert_eq!(row(&m).0, Some(5));
        // A card waits while its task is still Running: neither sweeps.
        events(&mut m, &one, vec![card(1, false)]);
        m.update(Action::TaskSetState { id, state: State::Running }.sent_by(Origin::System));
        assert_eq!((row(&m).2, row(&m).3), (false, false));
        m.update(Action::TaskSetState { id, state: State::NeedsYou }.sent_by(Origin::System));
        assert_eq!(row(&m), (None, None, false, false), "needs you: it says what it needs");
    }

    /// With no transcript of its own (in a terminal), the chip goes by the task and its process.
    #[test]
    fn the_agent_chip_of_a_conversation_in_a_terminal_goes_by_its_task() {
        use crate::activity::Pulse;
        let mut m = Model::default();
        m.update(Action::SetAgents { claude: vec![], codex: vec![], default: None, shows: View::Terminal });
        add(&mut m, "fix login");
        let (one, id) = (first(&m), m.tasks.list[0].id);
        let chip = |m: &Model| m.agent_chip().map(|chip| (chip.state, chip.line));
        let said = |state, line: &str| Some((state, line.to_string()));
        m.update(Action::TaskTalk { id });
        m.update(Action::TalkToggle);
        assert_eq!(chip(&m), said(Pulse::Ended, "Ended"), "not started");
        m.update(Action::TalkToggle);
        m.update(Action::TalkStart);
        m.update(Action::AgentStarted { task: one.clone(), session: None, at: 1 }.sent_by(Origin::System));
        m.update(Action::TalkToggle);
        m.update(Action::TaskSetState { id, state: State::Triaged }.sent_by(Origin::System));
        assert_eq!(chip(&m), said(Pulse::Idle, "Idle"));
        m.update(Action::TaskSetState { id, state: State::Running }.sent_by(Origin::System));
        assert_eq!(chip(&m), said(Pulse::Running, "Working"));
        m.update(Action::TaskSetState { id, state: State::NeedsYou }.sent_by(Origin::System));
        assert_eq!(chip(&m), said(Pulse::NeedsYou, "Needs you"));
    }

    /// The transcript in the drawer's place: no chip, so the user's click on the selected space
    /// only selects it, and leaves the transcript shown (⌘⇧R would collapse it).
    #[test]
    fn no_agent_chip_while_the_transcript_is_in_the_drawer() {
        let mut m = two_live();
        m.update(Action::TalkToggle);
        let at = m.agent_chip().map(|chip| chip.space);
        assert!(at.is_some());
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        let source = crate::listen::Source::Pane(m.focused.unwrap_or_default());
        m.update(Action::ListenStart { source, mic: false, id: "t".into(), at: 1 });
        m.update(Action::ListenShow { on: true });
        assert!(m.listen.shown);
        assert_eq!(m.agent_chip(), None);
        m.update(Action::SpaceClick { index: at.unwrap_or_default() });
        assert!(m.listen.shown && m.talk.as_ref().is_some_and(|talk| !talk.open));
    }

    /// The user's click on the selected space (or the chip) opens its closed conversation, as
    /// ⌘⇧R does. A client's click only selects, and so does a click on another space.
    #[test]
    fn a_click_on_the_selected_space_opens_its_closed_conversation() {
        let mut m = two_live();
        let open = |m: &Model| m.talk.as_ref().map(|talk| (talk.task.clone(), talk.open, talk.keys));
        let (one, two) = (first(&m), second(&m));
        m.update(Action::TalkToggle);
        let at = m.agent_chip().map(|chip| chip.space).unwrap();
        assert_eq!(m.shown_spaces()[at].of, crate::model::spaces::Owner::of(&one));
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.update(Action::SpaceClick { index: at }.sent_by(origin)), []);
            assert_eq!(open(&m), Some((one.clone(), false, false)), "{origin:?}'s click opens nothing");
        }
        // The user's: open, the keyboard in it, exactly as the key.
        let mut key = two_live();
        key.update(Action::TalkToggle);
        key.update(Action::TalkToggle);
        m.update(Action::SpaceClick { index: at });
        assert_eq!(open(&m), Some((one.clone(), true, true)));
        assert_eq!(format!("{m:?}"), format!("{key:?}"));
        // Open already: a click changes nothing. It closes nothing.
        m.update(Action::SpaceClick { index: at });
        assert_eq!(format!("{m:?}"), format!("{key:?}"));
        // Another space: selected, the drawer as it was; closed, it stays closed.
        let other = m.shown_spaces().iter().position(|space| space.of == crate::model::spaces::Owner::of(&two));
        m.update(Action::TalkToggle);
        m.update(Action::SpaceClick { index: other.unwrap() });
        assert_eq!(open(&m), Some((two.clone(), false, false)));
        // Under the user's keys in the chat a client's click on another space waits, as its select.
        m.update(Action::SpaceClick { index: other.unwrap() });
        assert_eq!(open(&m), Some((two, true, true)));
        let theirs = Action::SpaceClick { index: at };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(crate::model::spaces::CHAT_TYPING));
        let before = format!("{m:?}");
        m.update(theirs.sent_by(Origin::Socket));
        assert_eq!(format!("{m:?}"), before);
    }

    #[test]
    fn with_no_setting_a_verified_runner_shows_natively_and_codex_in_a_terminal() {
        let read = |m: &mut Model, text: &str| {
            m.update(Action::SettingsRead { text: text.into(), written: false }.sent_by(Origin::System));
        };
        let mut m = Model::default();
        add(&mut m, "fix login");
        let one = first(&m);
        assert_eq!(m.talk_view(&one), View::Native, "no settings read yet");
        read(&mut m, "{}");
        assert_eq!(m.talk_view(&one), View::Native, "no key: Claude Code natively");
        read(&mut m, r#"{"default_agent": "codex"}"#);
        assert_eq!(m.talk_view(&one), View::Terminal, "Codex's structured mode is not verified");
        read(&mut m, r#"{"conversation_view": "terminal"}"#);
        assert_eq!(m.talk_view(&one), View::Terminal, "named: respected");
        read(&mut m, "{}");
        assert_eq!(m.talk_view(&one), View::Native, "the key removed: native again");
    }

    #[test]
    fn a_claude_conversation_is_native_unless_the_setting_or_the_user_says_terminal() {
        let mut m = two_live();
        let one = first(&m);
        assert_eq!(m.talk_view(&one), View::Native);
        assert!(m.chat(&one).is_some_and(|chat| chat.transcript.live));
        m.update(Action::SetAgents {
            claude: vec![],
            codex: vec![],
            default: Some("codex".into()),
            shows: View::Native,
        });
        add(&mut m, "codex one");
        let codex = Subject::Task(m.tasks.list.last().map(|task| task.id).unwrap_or_default());
        assert_eq!(m.talk_view(&codex), View::Terminal, "Codex has no native view yet");
        m.update(Action::SetAgents { claude: vec![], codex: vec![], default: None, shows: View::Terminal });
        assert_eq!(m.talk_view(&second(&m)), View::Terminal);
    }

    /// Stop is at the composer while a turn runs. The agent stopped: nothing can be started on
    /// its session until its process is gone (no Resume, no "cannot"), then Resume is the
    /// composer's button, and its end is the user's, not an error.
    #[test]
    fn a_stopped_agent_is_resumed_only_once_gone_and_its_end_is_no_error() {
        use super::Primary;
        let mut m = two_live();
        let one = first(&m);
        assert_eq!(m.composer_button(), None, "no turn: no Stop anywhere");
        typed(&mut m, "go on");
        m.update(Action::TalkSend);
        assert_eq!(m.composer_button(), Some(Primary::Stop));
        // The agent asked to stop: until its process exits, nothing to start and nothing to say.
        assert_eq!(m.update(Action::TaskStop { id: one.clone() }), [Effect::StopAgent { task: one.clone() }]);
        assert!(m.is_ending(&one));
        assert_eq!((m.composer_button(), m.talk_resumes(), m.talk_note()), (None, None, None));
        // While it stops: no switch of view (it would start it again there), nothing typed or sent.
        assert!(!m.talk_switchable());
        assert_eq!(m.update(Action::TalkView { shows: View::Terminal }), []);
        assert_eq!(m.update(Action::TalkSend), [], "an empty composer sends nothing");
        let exited = m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        assert!(!exited.iter().any(|e| matches!(e, Effect::StartAgent { .. })), "the stop stands: {exited:?}");
        assert!(!m.is_ending(&one));
        let link = m.tasks.get(one.task().unwrap_or_default()).and_then(|task| task.session.clone());
        let resumes = link.is_some();
        let primary = if resumes { Primary::Resume } else { Primary::Start };
        assert_eq!((m.composer_button(), m.talk_note()), (Some(primary), None));
        let end = m.chat(&one).and_then(|chat| chat.transcript.items().last()).map(|item| item.entry.clone());
        let by_user = matches!(
            end,
            Some(crate::transcript::Entry::Notice(crate::transcript::Notice::Ended { by_user: true, .. }))
        );
        assert!(by_user, "{end:?}");
    }

    /// The user's report: Stop, then at once a message and ↵. Nothing is refused or said
    /// against it; nothing reaches the process stopping; once it is gone the session is gone on
    /// with, once, and the message is sent to that agent, once.
    #[test]
    fn a_message_right_after_stop_resumes_once_with_no_refusal() {
        let mut m = Model::default();
        add(&mut m, "plan the week");
        let one = first(&m);
        started(&mut m, Action::TaskTalk { id: 1 });
        let id = "8f14e45f-ceea-467f-a8d3-0242ac130003";
        m.update(Action::AgentStarted { task: one.clone(), session: Some(id.into()), at: 1 }.sent_by(Origin::System));
        events(&mut m, &one, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        m.update(Action::TaskStop { id: one.clone() });
        typed(&mut m, "go on");
        assert_eq!(m.update(Action::TalkSend), [], "never to the process that stops");
        assert_eq!((m.talk_note(), m.talk_resumes()), (None, None), "no 'cannot', and no second start yet");
        let exited = m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        let starts = |effects: &[Effect]| {
            effects.iter().filter(|e| matches!(e, Effect::StartAgent { .. } | Effect::ReadSession { .. })).count()
        };
        assert_eq!(starts(&exited), 1, "gone on with once: {exited:?}");
        assert_eq!(m.talk_note(), None);
        // Not listed: where it started is read first; then it starts, and gets the message once.
        let read = Action::SessionRead { runner: "claude".into(), id: id.into(), found: None };
        let read = m.update(read.sent_by(Origin::System));
        let resumed = matches!(&exited[..], [Effect::StartAgent { start: crate::agents::Start::Resume { .. }, .. }])
            || matches!(&read[..], [Effect::StartAgent { start: crate::agents::Start::Resume { .. }, .. }]);
        assert!(resumed, "{exited:?} {read:?}");
        let again = m.update(Action::AgentStarted { task: one.clone(), session: None, at: 2 }.sent_by(Origin::System));
        assert_eq!(again, [Effect::AgentSend { task: one.clone(), text: "go on".into() }]);
        assert_eq!(m.update(Action::TalkSend), [], "sent once");
    }

    /// Files dropped on the conversation: their paths go into the message, quoted; a client's drop,
    /// or a name with a line break, puts nothing there.
    #[test]
    fn a_dropped_file_goes_into_the_message_as_its_path() {
        let mut m = two_live();
        let one = first(&m);
        let drop =
            |paths: &[&str]| Action::TalkDrop { paths: paths.iter().map(|p| p.to_string()).collect(), unreadable: 0 };
        m.update(drop(&["/tmp/a.png"]).sent_by(Origin::Socket));
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some(""));
        typed(&mut m, "look at ");
        m.update(drop(&["/tmp/a.png", "/tmp/it's"]));
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some("look at '/tmp/a.png' '/tmp/it'\\''s' "));
        m.update(drop(&["/tmp/a\nb"]));
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some("look at '/tmp/a.png' '/tmp/it'\\''s' "));
        assert_eq!(m.clip.as_ref().map(|clip| clip.text.as_str()), Some(crate::clip::DROP_REFUSED));
        // No room for the whole quoted path: nothing goes in, never part of it.
        let before = m.chat(&one).map(|chat| chat.draft.text().to_string());
        let long = format!("/tmp/{}", "x".repeat(super::MAX_DRAFT));
        m.update(Action::TalkDrop { paths: vec![long], unreadable: 0 });
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text().to_string()), before);
        assert_eq!(m.clip.as_ref().map(|clip| clip.text.as_str()), Some(crate::clip::DROP_NO_ROOM));
    }

    #[test]
    fn the_composer_sends_once_per_turn_and_only_the_users() {
        let mut m = two_live();
        let one = first(&m);
        assert!(m.talk_typing() && m.field().is_some_and(|(of, _)| of == "talk"));
        // A client writes nothing the user would send.
        let edit = Action::FieldEdit { edit: crate::edit::Edit::Replace { range: None, text: "rm -rf ~".into() } };
        assert_eq!(m.refusal(&edit, Origin::Socket), Some(super::USERS_ALONE));
        m.update(edit.sent_by(Origin::Socket));
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some(""));
        // A line break typed or pasted is kept: the composer holds lines.
        typed(&mut m, "two\nlines");
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Paste("\r\nand\tmore".into()) });
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some("two\nlines\nand    more"));
        m.update(Action::FieldEdit { edit: crate::edit::Edit::SelectAll });
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Cut });
        typed(&mut m, "hello");
        let sent = m.update(Action::TalkSend.sent_by(Origin::Socket));
        assert!(sent.is_empty() && m.refusal(&Action::TalkSend, Origin::Socket).is_some());
        assert_eq!(m.update(Action::TalkSend), [Effect::AgentSend { task: one.clone(), text: "hello".into() }]);
        assert_eq!(m.tasks.get(one.task().unwrap_or_default()).map(|t| t.state), Some(State::Running));
        typed(&mut m, "more");
        assert_eq!(m.update(Action::TalkSend), [], "a turn runs: ↵ queues it");
        assert_eq!(texts(&m, &one), ["more"]);
        let ended = events(&mut m, &one, vec![Event::TurnStarted, Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(ended, [Effect::AgentSend { task: one.clone(), text: "more".into() }], "sent once the turn ends");
        assert_eq!(m.tasks.get(one.task().unwrap_or_default()).map(|t| t.state), Some(State::Running));
    }

    /// While a turn runs ↵ queues; each queued message is a turn of its own, in order, once the
    /// conversation is idle; a card (a question) comes first; the user's Stop holds the queue
    /// until their ↵; ⌘↵ sends now, stopping the turn where the runner takes nothing mid-turn;
    /// the queue is kept with the task; only the user queues, edits, removes or sends now.
    #[test]
    fn messages_queue_while_a_turn_runs_and_go_in_order() {
        let mut m = two_live();
        let one = first(&m);
        let id = one.task().unwrap_or_default();
        let turn = |m: &mut Model, events| super::tests::events(m, &one, events);
        let done = || vec![Event::TurnFinished { outcome: Outcome::Completed }];
        let queue = |m: &Model| texts(m, &one);
        typed(&mut m, "first");
        m.update(Action::TalkSend);
        for text in ["second", "third"] {
            typed(&mut m, text);
            assert_eq!(m.update(Action::TalkSend), []);
        }
        assert_eq!(queue(&m), ["second", "third"]);
        // A task serialised (the state, the inbox, a socket reply) has their count, never their
        // words; only its file has them, and reads them back.
        let state = serde_json::to_string(&m.tasks).unwrap_or_default();
        let task = m.tasks.get(id).cloned().unwrap_or_else(|| panic!("task {id}"));
        let json = serde_json::json!(task).to_string();
        assert!(state.contains("\"queued\":2") && !state.contains("second") && !json.contains("second"), "{json}");
        let file = task.file_json();
        let words: Vec<&str> =
            file["queued"].as_array().into_iter().flatten().filter_map(|q| q["text"].as_str()).collect();
        assert_eq!(words, ["second", "third"]);
        let back: crate::tasks::Task = serde_json::from_value(file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.queued, task.queued);
        let counted: crate::tasks::Task = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}"));
        assert!(counted.queued.is_empty(), "a count brings no words back");
        // A client queues, edits, removes and sends nothing.
        for theirs in [
            Action::TalkSendNow,
            Action::QueueRemove { id: 1 },
            Action::QueueEdit { id: 1 },
            Action::QueueSendNow { id: 1 },
        ] {
            assert_eq!(m.refusal(&theirs, Origin::Socket), Some(super::USERS_ALONE));
        }
        // A card first: what is queued waits behind the question.
        assert_eq!(turn(&mut m, vec![card(4, false)]), []);
        assert_eq!(queue(&m).len(), 2);
        m.update(Action::TalkAnswer { allow: false, ticket: 4 });
        // The turn ends: the next goes, alone; then the next.
        assert_eq!(turn(&mut m, done()), [Effect::AgentSend { task: one.clone(), text: "second".into() }]);
        assert_eq!(turn(&mut m, done()), [Effect::AgentSend { task: one.clone(), text: "third".into() }]);
        assert!(queue(&m).is_empty());
        // The user's Stop holds the queue; their ↵ on an empty composer sends the next.
        typed(&mut m, "fourth");
        m.update(Action::TalkSend);
        m.update(Action::TalkInterrupt);
        assert_eq!(turn(&mut m, done()), [], "held after the user's stop");
        assert_eq!(m.update(Action::TalkSend), [Effect::AgentSend { task: one.clone(), text: "fourth".into() }]);
        // Edit puts it back in the composer; remove drops it.
        for text in ["fifth", "sixth"] {
            typed(&mut m, text);
            m.update(Action::TalkSend);
        }
        let ids: Vec<u64> = m.queue_of(&one).iter().map(|queued| queued.id).collect();
        m.update(Action::QueueEdit { id: ids[1] });
        m.update(Action::QueueRemove { id: ids[0] });
        assert_eq!((queue(&m), m.chat(&one).map(|chat| chat.draft.text().to_string())), (vec![], Some("sixth".into())));
        // ⌘↵ while a turn runs: Claude Code takes it into the turn, no stop.
        assert!(!m.send_now_stops(&one));
        assert_eq!(m.update(Action::TalkSendNow), [Effect::AgentSend { task: one.clone(), text: "sixth".into() }]);
        assert!(queue(&m).is_empty());
    }

    /// Found in review: a card waiting keeps "send now" behind the question (no stop); a full queue
    /// leaves the message in the box and says so; a queue read back waits for the user's ↵.
    #[test]
    fn the_queue_waits_for_a_question_has_a_limit_and_is_held_after_a_restart() {
        let mut m = two_live();
        let one = first(&m);
        let id = one.task().unwrap_or_default();
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        typed(&mut m, "now");
        events(&mut m, &one, vec![card(4, false)]);
        assert_eq!(m.update(Action::TalkSendNow), [], "no stop while a question waits");
        assert_eq!(texts(&m, &one), ["now"]);
        m.update(Action::TalkAnswer { allow: false, ticket: 4 });
        typed(&mut m, &"x".repeat(super::MAX_QUEUE));
        let full = m.update(Action::TalkSend);
        assert_eq!(full, [Effect::Notice { text: super::QUEUE_FULL.into() }]);
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text().chars().count()), Some(super::MAX_QUEUE));
        // Read back after a restart: held until the user's empty ↵.
        let queued = m.tasks.get(id).map(|task| task.queued.clone()).unwrap_or_default();
        let mut fresh = Model::default();
        add(&mut fresh, "fix login");
        let again = fresh.tasks.list[0].id;
        fresh.tasks.set_queued(again, queued);
        started(&mut fresh, Action::TaskTalk { id: again });
        let subject = Subject::Task(again);
        let up =
            fresh.update(Action::AgentStarted { task: subject.clone(), session: None, at: 1 }.sent_by(Origin::System));
        assert!(fresh.chat(&subject).is_some_and(|chat| chat.held), "{up:?}");
        assert_eq!(texts(&fresh, &subject), ["now"]);
    }

    /// A row names its message by id: the turn ending between the row's draw and the click sends
    /// the first; the click on the row drawn first then acts on nothing, never on the next one.
    /// A message whose write failed (its agent had ended) is back first in the queue, held.
    #[test]
    fn a_row_acts_on_its_own_message_and_an_undelivered_one_comes_back_first() {
        let mut m = two_live();
        let one = first(&m);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        for text in ["alpha", "beta"] {
            typed(&mut m, text);
            m.update(Action::TalkSend);
        }
        let drawn: Vec<u64> = m.queue_of(&one).iter().map(|queued| queued.id).collect();
        // The turn ends: alpha goes. The user's click was on alpha's row, drawn before.
        let sent = events(&mut m, &one, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(sent, [Effect::AgentSend { task: one.clone(), text: "alpha".into() }]);
        m.update(Action::QueueRemove { id: drawn[0] });
        assert_eq!(texts(&m, &one), ["beta"], "beta stays: the click was alpha's");
        // alpha was never written: first again, held for the user.
        let back = Action::AgentSendFailed { task: one.clone(), text: "alpha".into(), images: Vec::new() };
        assert_eq!(m.refusal(&back, Origin::Socket), Some(super::RUNNERS_ALONE));
        m.update(back.clone());
        assert_eq!(texts(&m, &one), ["beta"], "the user's report is nothing: only the runner's");
        m.update(back.sent_by(Origin::System));
        assert_eq!(texts(&m, &one), ["alpha", "beta"]);
        assert!(m.chat(&one).is_some_and(|chat| chat.held));
        let ids: Vec<u64> = m.queue_of(&one).iter().map(|queued| queued.id).collect();
        assert!(!drawn.contains(&ids[0]), "a new id: an old row's click is not this message's");
    }

    /// A pasted image joins the message being written: the composer's goes with its message; a
    /// client pastes nothing; one nothing holds any more has its file dropped, once; the state
    /// counts a task's, never names them.
    #[test]
    fn a_pasted_image_goes_with_its_message_and_its_file_goes_after() {
        use crate::tasks::Pasted;
        let mut m = two_live();
        let one = first(&m);
        let image = |n: u8| Pasted::new(&format!("{n:016x}"), "png").unwrap_or_else(|| panic!("{n}"));
        let paste = |n: u8| Action::ImagePaste { id: image(n).id().into(), kind: "png".into() };
        assert_eq!(m.refusal(&paste(1), Origin::Socket), Some(super::USERS_ALONE));
        // Not a name the shell makes: nothing.
        assert_eq!(m.update(Action::ImagePaste { id: "../../etc".into(), kind: "png".into() }), []);
        m.update(paste(1));
        typed(&mut m, "look");
        let sent = m.update(Action::TalkSend);
        let with = Effect::AgentSendWith { task: one.clone(), text: "look".into(), images: vec![image(1)] };
        // A task's conversation keeps nothing once sent: its file goes after the send reads it.
        assert_eq!(sent, [with, Effect::ImagesDrop { images: vec![image(1)] }]);
        // Too many: the sixth is refused, said, its file dropped.
        for n in 2..7 {
            m.update(paste(n));
        }
        let sixth = m.update(paste(7));
        assert_eq!(
            sixth,
            [Effect::Notice { text: super::TOO_MANY_IMAGES.into() }, Effect::ImagesDrop { images: vec![image(7)] }]
        );
        // × on a chip: gone, and its file.
        assert_eq!(
            m.update(Action::ImageRemove { id: image(2).id().into() }),
            [Effect::ImagesDrop { images: vec![image(2)] }]
        );
    }

    /// A runner that takes a message mid-turn gets "send now" in the turn: both do (probes).
    #[test]
    fn send_now_stops_only_where_the_runner_takes_nothing_mid_turn() {
        use crate::runner::RunnerId;
        assert!(RunnerId::Claude.spec().structured.mid_turn);
        assert!(RunnerId::Codex.spec().structured.mid_turn);
    }

    /// Only the shell's runner reports what an agent said: a user's or a client's report is nothing.
    #[test]
    fn events_come_from_the_runner_alone() {
        let mut m = two_live();
        let one = first(&m);
        for origin in [Origin::User, Origin::Socket] {
            m.update(Action::AgentEvents { task: one.clone(), events: vec![card(9, false)], at: 1 }.sent_by(origin));
            assert!(m.chat(&one).is_some_and(|chat| chat.transcript.cards.is_empty()), "{origin:?}");
        }
        // A turn that starts and ends in one report leaves the task waiting for the user.
        let turn = vec![Event::TurnStarted, Event::TurnFinished { outcome: Outcome::Completed }];
        m.tasks.set_state(one.task().unwrap_or_default(), State::Running);
        events(&mut m, &one, turn);
        assert_eq!(m.tasks.get(one.task().unwrap_or_default()).map(|t| t.state), Some(State::NeedsYou));
    }

    #[test]
    fn on_a_card_enter_does_nothing_and_letters_answer() {
        use super::{Key, key};
        use crate::keys::Mods;
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Mods::default() });
        let cmd = Mods { cmd: true, ..Mods::default() };
        let card = Some(4);
        assert_eq!(key("enter", none, card), Key::Swallow, "no reflex Allow");
        assert_eq!(key("a", none, card), Key::Act(Action::TalkAnswer { allow: true, ticket: 4 }));
        assert_eq!(key("A", shift, card), Key::Swallow);
        assert_eq!(key("d", none, card), Key::Act(Action::TalkAnswer { allow: false, ticket: 4 }));
        assert_eq!(key("s", none, card), Key::Act(Action::TalkExpand { ticket: 4 }));
        assert_eq!(key("m", none, card), Key::Act(Action::TalkDenyNote));
        assert_eq!(key("x", none, card), Key::Swallow);
        assert_eq!(key("escape", none, card), Key::Act(Action::TalkEscape));
        assert_eq!(key("enter", none, None), Key::Act(Action::TalkSend));
        assert_eq!(key("enter", shift, None), Key::Newline);
        assert_eq!(key("enter", Mods { alt: true, ..Mods::default() }, None), Key::Newline);
        assert_eq!((key("a", none, None), key("l", cmd, card)), (Key::Field, Key::Pass));
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        let edit = Key::Act(Action::EditorOpen);
        assert_eq!((key("g", ctrl, None), key("g", ctrl, card)), (edit.clone(), edit));
    }

    #[test]
    fn a_question_card_has_its_own_keys() {
        use super::{Key, ask_key};
        use crate::ask::Act;
        use crate::keys::Mods;
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Mods::default() });
        let act = |act| Key::Act(Action::TalkAsk { ticket: 4, act });
        let keys = |typing| {
            ["up", "k", "down", "j", "space", "3", "0", "enter", "escape", "a"].map(|k| ask_key(k, none, 4, typing))
        };
        let (down, up) = (act(Act::Move { down: true }), act(Act::Move { down: false }));
        let (submit, decline) = (act(Act::Submit), act(Act::Decline));
        assert_eq!(
            keys(false),
            [
                up.clone(),
                up.clone(),
                down.clone(),
                down.clone(),
                act(Act::Toggle),
                act(Act::Number { n: 3 }),
                Key::Swallow,
                submit.clone(),
                decline.clone(),
                Key::Swallow
            ]
        );
        // On "Other…": letters, digits and space are its words; arrows, ↵ and Esc are still the card's.
        let field = Key::Field;
        assert_eq!(
            keys(true),
            [
                up,
                field.clone(),
                down,
                field.clone(),
                field.clone(),
                field.clone(),
                field.clone(),
                submit,
                decline,
                field
            ]
        );
        assert_eq!(ask_key("enter", shift, 4, false), Key::Swallow);
        assert_eq!(ask_key("tab", none, 4, true), act(Act::Tab { forward: true }));
        assert_eq!(ask_key("tab", shift, 4, false), act(Act::Tab { forward: false }));
        assert_eq!(ask_key("q", Mods { cmd: true, ..none }, 4, false), Key::Pass);
    }

    fn ask(ticket: u64) -> Event {
        use crate::ask::tests::question;
        let (tool, label) = ("AskUserQuestion".to_string(), "AskUserQuestion".to_string());
        let questions = vec![question(2, false), question(2, true)];
        Event::Permission {
            ticket,
            tool,
            label,
            detail: Text::default(),
            len: 0,
            cut: false,
            claim: Text::default(),
            questions,
        }
    }

    /// The user picks, types under "Other…" and sends with ↵: one answer per question, by index
    /// and in their own words, never allowed as it came. A client moves, picks, types or sends
    /// nothing. The state has the card's shape alone.
    #[test]
    fn a_question_card_is_answered_by_the_user_alone() {
        use crate::ask::{Act, Chosen};
        let mut m = two_live();
        let one = first(&m);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        events(&mut m, &one, vec![Event::TurnStarted, ask(1)]);
        let shape = |m: &Model| m.chat(&one).map(|chat| serde_json::to_value(chat.transcript.summary().cards).unwrap());
        let ask_of = |m: &Model| shape(m).map(|cards| cards[0]["ask"].clone());
        assert_eq!(
            ask_of(&m),
            Some(serde_json::json!({"questions": [{"options": 2, "multi": false, "picked": [], "answered": false},
                {"options": 2, "multi": true, "picked": [], "answered": false}], "cursor": [0, 0], "tabs": 3}))
        );
        let act = |act| Action::TalkAsk { ticket: 1, act };
        assert_eq!(m.refusal(&act(Act::Submit), Origin::Socket), Some(super::USERS_ALONE));
        for act in [act(Act::Number { n: 2 }), act(Act::Submit), act(Act::Decline)] {
            assert_eq!(m.update(act.sent_by(Origin::Socket)), []);
        }
        assert_eq!(m.update(Action::TalkAnswer { allow: true, ticket: 1 }), [], "never allowed as it came");
        let none = crate::keys::Mods::default();
        assert_eq!(m.chat(&one).map(|chat| chat.key("enter", none)), Some(Key::Act(act(Act::Submit))));
        m.update(act(Act::Number { n: 2 }));
        // ↵ on the first of two: answered, nothing goes, the second shows.
        assert_eq!(m.update(act(Act::Submit)), []);
        assert_eq!(ask_of(&m).map(|a| a["cursor"].clone()), Some(serde_json::json!([1, 0])));
        m.update(act(Act::Toggle));
        m.update(act(Act::Move { down: true }));
        m.update(act(Act::Move { down: true }));
        assert!(m.talk_typing() && m.field().is_some_and(|(of, _)| of == "talk"), "typing on Other…");
        let edit = Action::FieldEdit { edit: crate::edit::Edit::Insert("big one".into()) };
        assert_eq!(m.refusal(&edit, Origin::Socket), Some(super::USERS_ALONE));
        m.update(edit.clone().sent_by(Origin::Socket));
        m.update(edit);
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some(""), "Other's words are not the draft");
        let said = shape(&m).map(|cards| cards.to_string()).unwrap_or_default();
        assert!(said.contains(r#""picked":[0,2]"#) && !said.contains("big"), "{said}");
        let answers =
            vec![Chosen { options: vec![1], other: None }, Chosen { options: vec![0], other: Some("big one".into()) }];
        // ↵ on the last question: Review; ↵ there sends.
        assert_eq!(m.update(act(Act::Submit)), []);
        assert_eq!(ask_of(&m).map(|a| a["cursor"].clone()), Some(serde_json::json!([2, 0])));
        assert_eq!(m.update(act(Act::Submit)), [Effect::AgentAnswers { task: one.clone(), ticket: 1, answers }]);
        let t = m.chat(&one).map(|chat| chat.transcript.summary());
        assert!(t.as_ref().is_some_and(|t| t.cards.is_empty()), "answered once");
        let last = t.and_then(|t| t.entries.last().cloned());
        let note = crate::transcript::NoteKind::Answered { answered: 2, of: 2 };
        assert_eq!(last, Some(crate::transcript::Shape::Note { note, bytes: "H: o1 · H: o0, big one".len() }));
        assert_eq!(m.tasks.get(one.task().unwrap_or_default()).map(|t| t.state), Some(State::Running));
        assert_eq!(m.update(act(Act::Submit)), []);
    }

    /// A task waiting on cards lists them under its row: a question and how far in, or a
    /// permission; past two, how many more. Kinds and counts only.
    #[test]
    fn a_tasks_row_lists_its_waiting_cards() {
        use crate::ask::Act;
        use crate::tasks::Pending;
        let mut m = two_live();
        let one = first(&m);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        events(&mut m, &one, vec![Event::TurnStarted, ask(1), card(2, false), card(3, false), card(4, false)]);
        let row = |m: &Model| {
            let sections = m.task_sections(10);
            let rows = sections.iter().flat_map(|s| s.rows.clone());
            rows.into_iter().find(|row| Some(row.id) == one.task()).map(|row| (row.pending, row.more))
        };
        let question = |shown| Pending::Question { shown, of: 2 };
        assert_eq!(row(&m), Some((vec![question(1), Pending::Permission], 2)));
        m.update(Action::TalkAsk { ticket: 1, act: Act::Tab { forward: true } });
        assert_eq!(row(&m), Some((vec![question(2), Pending::Permission], 2)));
        m.update(Action::TalkAsk { ticket: 1, act: Act::Tab { forward: true } });
        assert_eq!(row(&m).map(|(p, _)| p[0]), Some(question(2)), "on Review, the last");
        m.update(Action::TalkAsk { ticket: 1, act: Act::Decline });
        assert_eq!(row(&m), Some((vec![Pending::Permission, Pending::Permission], 1)));
        // A line's click: that card first, nothing answered; a client's moves nothing.
        assert_eq!(m.update(Action::TaskCard { id: 1, ticket: Some(3) }.sent_by(Origin::Socket)), []);
        let order =
            |m: &Model| m.chat(&one).map(|chat| chat.transcript.cards.iter().map(|c| c.ticket).collect::<Vec<_>>());
        assert_eq!(order(&m), Some(vec![2, 3, 4]));
        let effects = m.update(Action::TaskCard { id: one.task().unwrap_or_default(), ticket: Some(3) });
        assert!(!effects.iter().any(|e| matches!(e, Effect::AgentAnswer { .. } | Effect::AgentAnswers { .. })));
        assert_eq!(order(&m), Some(vec![3, 2, 4]));
        m.update(Action::TaskCard { id: one.task().unwrap_or_default(), ticket: None });
        assert_eq!(order(&m), Some(vec![3, 2, 4]));
        let said = serde_json::to_string(&m.task_sections(10)).unwrap();
        assert!(said.contains(r#""pending":[{"kind":"permission"},{"kind":"permission"}],"more":1"#), "{said}");
    }

    /// Esc declines: a Deny with fixed words, so the agent goes on.
    #[test]
    fn a_declined_question_card_tells_the_agent() {
        use crate::ask::Act;
        let mut m = two_live();
        let one = first(&m);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        events(&mut m, &one, vec![Event::TurnStarted, ask(2)]);
        let none = crate::keys::Mods::default();
        let Some(Key::Act(decline)) = m.chat(&one).map(|chat| chat.key("escape", none)) else { panic!() };
        let message = Some(crate::ask::DECLINED.to_string());
        assert_eq!(m.update(decline), [Effect::AgentAnswer { task: one.clone(), ticket: 2, allow: false, message }]);
        let last = m.chat(&one).and_then(|chat| chat.transcript.summary().entries.last().cloned());
        assert_eq!(last, Some(crate::transcript::Shape::Notice(crate::transcript::Notice::Declined)));
        // A permission card is not declined by it.
        events(&mut m, &one, vec![card(3, false)]);
        assert_eq!(m.update(Action::TalkAsk { ticket: 3, act: Act::Decline }), []);
    }

    #[test]
    fn a_new_session_shows_its_first_prompt() {
        let m = two_live();
        let shapes = m.chat(&first(&m)).map(|chat| chat.transcript.summary().entries);
        assert!(matches!(shapes.as_deref(), Some([crate::transcript::Shape::User { .. }])), "{shapes:?}");
    }

    #[test]
    fn esc_esc_interrupts_a_turn_and_esc_leaves_an_idle_one() {
        let mut m = two_live();
        let one = first(&m);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        assert_eq!(m.update(Action::TalkEscape), []);
        assert_eq!(m.update(Action::TalkEscape), [Effect::AgentInterrupt { task: one.clone() }]);
        assert_eq!(m.update(Action::TalkInterrupt.sent_by(Origin::Socket)), []);
        assert_eq!(m.update(Action::TalkInterrupt), [Effect::AgentInterrupt { task: one.clone() }]);
        events(&mut m, &one, vec![Event::TurnFinished { outcome: Outcome::Interrupted }]);
        m.update(Action::TalkEscape);
        assert!(m.talk.as_ref().is_some_and(|talk| !talk.keys), "idle: Esc leaves to the workspace");
    }

    /// A card blocks its own conversation's composer only; answers go to the one shown; a cut
    /// card is not allowed until opened; a Deny may carry the user's words.
    #[test]
    fn a_card_belongs_to_its_conversation() {
        let mut m = two_live();
        let (one, two) = (first(&m), second(&m));
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        events(&mut m, &one, vec![Event::TurnStarted, card(1, true)]);
        assert!(!m.talk_typing(), "the card has the keys");
        assert_eq!(m.tasks.get(one.task().unwrap_or_default()).map(|t| t.state), Some(State::NeedsYou));
        // The other conversation, shown mid-stream: its composer is free, and answers are its own.
        m.update(Action::TaskTalk { id: m.tasks.list[1].id });
        m.update(Action::TalkKeys { on: true });
        assert!(m.talk_typing());
        assert_eq!(m.update(Action::TalkAnswer { allow: false, ticket: 1 }), [], "no card here");
        typed(&mut m, "draft two");
        events(&mut m, &one, vec![Event::TextDelta { text: Text::new("more") }]);
        m.update(Action::TaskTalk { id: m.tasks.list[0].id });
        m.update(Action::TalkKeys { on: true });
        assert_eq!(m.chat(&two).map(|chat| chat.draft.text()), Some("draft two"), "its draft is kept");
        assert_eq!(m.update(Action::TalkAnswer { allow: true, ticket: 1 }), [], "cut: Show all first");
        assert_eq!(m.update(Action::TalkExpand { ticket: 1 }.sent_by(Origin::Socket)), []);
        assert_eq!(m.update(Action::TalkAnswer { allow: true, ticket: 1 }.sent_by(Origin::Socket)), []);
        // Show all and Allow for another card than the one shown do nothing.
        assert_eq!(m.update(Action::TalkExpand { ticket: 2 }), []);
        assert!(m.chat(&one).is_some_and(|chat| !chat.transcript.cards[0].expanded));
        m.update(Action::TalkExpand { ticket: 1 });
        let allowed = Effect::AgentAnswer { task: one.clone(), ticket: 1, allow: true, message: None };
        assert_eq!(m.update(Action::TalkAnswer { allow: true, ticket: 1 }), [allowed]);
        assert_eq!(m.update(Action::TalkAnswer { allow: true, ticket: 1 }), [], "answered once");
        events(&mut m, &one, vec![card(2, false)]);
        m.update(Action::TalkDenyNote);
        assert!(m.talk_typing());
        typed(&mut m, "use git diff");
        let denied =
            Effect::AgentAnswer { task: one.clone(), ticket: 2, allow: false, message: Some("use git diff".into()) };
        assert_eq!(m.update(Action::TalkAnswer { allow: false, ticket: 2 }), [denied]);
        assert_eq!(m.chat(&one).map(|chat| (chat.draft.text(), chat.deny)), Some(("", false)));
    }

    /// A card the CLI withdrew takes its answer and its Deny's message with it: the next card gets neither.
    #[test]
    fn an_answer_for_a_withdrawn_card_reaches_no_other() {
        let mut m = two_live();
        let one = first(&m);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        events(&mut m, &one, vec![card(1, false), card(2, false)]);
        m.update(Action::TalkDenyNote);
        typed(&mut m, "not that");
        events(&mut m, &one, vec![Event::PermissionClosed { ticket: 1 }]);
        assert!(m.chat(&one).is_some_and(|chat| !chat.deny), "the message was for the withdrawn card");
        assert_eq!(m.update(Action::TalkAnswer { allow: true, ticket: 1 }), []);
        assert_eq!(m.chat(&one).map(|chat| chat.transcript.cards.len()), Some(1));
    }

    /// The bug of 0.1.0: "plan my week ~/notes folder" was filed as "plan my week folder", and
    /// the agent was asked that alone. The `/path` stays in the title it is asked (#131).
    #[test]
    fn a_path_the_line_names_is_in_what_the_agent_is_asked() {
        let mut m = Model::default();
        m.update(Action::SetAgents { claude: vec![], codex: vec![], default: None, shows: View::Native });
        let found = crate::compose::PathFound { typed: "~/notes".into(), resolved: "/Users/me/notes".into() };
        let known = crate::compose::Known { paths: vec![found], ..Default::default() };
        let line = "plan my week ~/notes folder".to_string();
        m.update(Action::TaskFromLine { line, known, open: false });
        let task = m.tasks.list[0].id;
        assert_eq!(m.tasks.list[0].title, "plan my week ~/notes folder");
        let effects = started(&mut m, Action::TaskTalk { id: task });
        let Some(Effect::StartAgent { start: crate::agents::Start::New(launch), .. }) = effects.first() else {
            panic!("{effects:?}")
        };
        assert_eq!(launch.title, "plan my week ~/notes folder");
        m.update(Action::AgentStarted { task: Subject::Task(task), session: None, at: 1 }.sent_by(Origin::System));
        let shown = m.chat(&Subject::Task(task)).and_then(|chat| chat.transcript.items().next().cloned());
        let asked = shown.and_then(|item| match item.entry {
            crate::transcript::Entry::User { text } => Some(text.text().to_string()),
            _ => None,
        });
        assert_eq!(asked.as_deref(), Some("plan my week ~/notes folder"), "the user's own words");
        let added = m.chat(&Subject::Task(task)).and_then(|chat| chat.added.clone()).expect("what ottospace added");
        assert_eq!((added.via, added.tools.len(), added.briefed), (crate::agents::Via::Server, 7, false));
        assert!(added.guidance.is_some_and(|text| text.starts_with("You run inside ottospace")));
    }

    /// The bug of 0.1.0: each click on an ended task started it again, its prompt and "ended"
    /// added each time. A click shows it; Resume session (`TalkStart`) goes on with it once: a
    /// session never written starts afresh with its prompt, once; one written is resumed with none.
    #[test]
    fn selecting_an_ended_conversation_only_shows_it_and_resume_is_explicit() {
        let mut m = two_live();
        let one = first(&m);
        let id = one.task().unwrap_or_default();
        let link = crate::sessions::Link {
            runner: "claude".into(),
            id: "8f14e45f-ceea-467f-a8d3-0242ac130003".into(),
            state: None,
        };
        m.tasks.link(id, link.clone());
        events(&mut m, &one, vec![Event::Exited { code: Some(0), signal: None, details: false }]);
        m.update(Action::SessionMissing { task: id, id: link.id.clone() }.sent_by(Origin::System));
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        let entries = |m: &Model| m.chat(&one).map(|chat| chat.transcript.items().len());
        let before = entries(&m);
        for _ in 0..3 {
            assert_eq!(m.update(Action::TaskTalk { id }), []);
            assert_eq!(m.update(Action::TaskTalk { id: m.tasks.list[1].id }), []);
            assert_eq!(m.update(Action::TaskTalk { id: m.tasks.list[0].id }), []);
        }
        assert_eq!((entries(&m), m.agent_runs(&one), m.talk_resumes()), (before, false, Some(false)));
        // Never written: a new session, its prompt once.
        let effects = m.update(Action::TalkStart);
        assert!(
            matches!(&effects[..], [Effect::StartAgent { start: crate::agents::Start::New(_), .. }]),
            "{effects:?}"
        );
        assert_eq!(m.update(Action::TalkStart.sent_by(Origin::Socket)), []);
        m.update(Action::AgentStarted { task: one.clone(), session: None, at: 2 }.sent_by(Origin::System));
        assert_eq!(entries(&m), before.map(|n| n + 1), "the prompt, once");
        // Written: resumed, nothing sent again.
        let link_id = link.id.clone();
        m.tasks.link(id, link);
        events(&mut m, &one, vec![Event::Exited { code: Some(0), signal: None, details: false }]);
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        assert_eq!(m.talk_resumes(), Some(true));
        // Not listed: where it started is read first, then it is gone on with.
        assert!(matches!(&m.update(Action::TalkStart)[..], [Effect::ReadSession { .. }]));
        let read = Action::SessionRead { runner: "claude".into(), id: link_id.clone(), found: None };
        let effects = m.update(read.sent_by(Origin::System));
        assert!(matches!(&effects[..], [Effect::StartAgent { start: crate::agents::Start::Resume { .. }, .. }]));
        let shown = entries(&m);
        m.update(Action::AgentStarted { task: one.clone(), session: None, at: 3 }.sent_by(Origin::System));
        let users = m
            .chat(&one)
            .map(|c| c.transcript.items().filter(|i| matches!(i.entry, crate::transcript::Entry::User { .. })).count());
        // Its end gives way to one divider that it resumed, its turns above; no prompt again.
        assert_eq!((entries(&m), users), (shown, Some(2)), "one divider in place of the end, no prompt");
        let last = m.chat(&one).and_then(|c| c.transcript.items().last()).map(|i| i.entry.clone());
        let divider = crate::transcript::Notice::Resumed {
            here: true,
            ended: Some(crate::transcript::End { code: Some(0), signal: None, by_user: false }),
        };
        assert_eq!(last, Some(crate::transcript::Entry::Notice(divider)));
    }

    /// The bug of 0.1.0: "say hello" after the agent ended went nowhere (the composer took no
    /// keys). Now ↵ goes on with the session and sends the message once it runs; a start that
    /// fails gives the message back; only the user's ↵.
    #[test]
    fn enter_in_an_ended_conversation_resumes_and_sends() {
        let mut m = two_live();
        let one = first(&m);
        let id = one.task().unwrap_or_default();
        let link = crate::sessions::Link {
            runner: "claude".into(),
            id: "8f14e45f-ceea-467f-a8d3-0242ac130003".into(),
            state: None,
        };
        m.tasks.link(id, link);
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        m.update(Action::TalkKeys { on: true });
        assert!(m.talk_typing() && m.field().is_some_and(|(of, _)| of == "talk"), "the composer takes keys");
        typed(&mut m, "say hello");
        assert_eq!(m.update(Action::TalkSend.sent_by(Origin::Socket)), []);
        assert!(matches!(&m.update(Action::TalkSend)[..], [Effect::ReadSession { .. }]));
        let read = Action::SessionRead {
            runner: "claude".into(),
            id: "8f14e45f-ceea-467f-a8d3-0242ac130003".into(),
            found: None,
        };
        assert!(matches!(&m.update(read.sent_by(Origin::System))[..], [Effect::StartAgent { .. }]));
        // A start that failed: the words come back.
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        assert_eq!(m.chat(&one).map(|chat| chat.draft.text()), Some("say hello"));
        // The same when its read finds it at work elsewhere, or Stop comes first.
        m.update(Action::TalkSend);
        m.update(Action::TaskStop { id: one.clone() });
        assert_eq!(m.chat(&one).map(|chat| (chat.draft.text(), chat.queued.is_some())), Some(("say hello", false)));
        // Once it runs, the message goes.
        m.update(Action::TalkSend);
        let read = Action::SessionRead {
            runner: "claude".into(),
            id: "8f14e45f-ceea-467f-a8d3-0242ac130003".into(),
            found: None,
        };
        m.update(read.sent_by(Origin::System));
        let sent = m.update(Action::AgentStarted { task: one.clone(), session: None, at: 3 }.sent_by(Origin::System));
        assert_eq!(sent, [Effect::AgentSend { task: one.clone(), text: "say hello".into() }]);
        assert!(m.chat(&one).is_some_and(|chat| chat.draft.text().is_empty() && chat.transcript.turn.is_some()));
    }

    /// Stop while a switch waits for the process to end: nothing starts after.
    #[test]
    fn a_stop_during_a_switch_starts_nothing_after() {
        let mut m = two_live();
        let one = first(&m);
        let link = crate::sessions::Link {
            runner: "claude".into(),
            id: "8f14e45f-ceea-467f-a8d3-0242ac130003".into(),
            state: None,
        };
        m.tasks.link(one.task().unwrap_or_default(), link);
        assert_eq!(m.update(Action::TalkView { shows: View::Terminal }), [Effect::StopAgent { task: one.clone() }]);
        m.update(Action::TaskStop { id: one.clone() });
        assert_eq!(m.update(Action::AgentExited { task: one }.sent_by(Origin::System)), []);
    }

    /// Chat → Terminal ends the native process first; the session goes on in the terminal once it is gone.
    #[test]
    fn a_switch_stops_one_process_then_resumes_in_the_other() {
        let mut m = two_live();
        let one = first(&m);
        let id = "8f14e45f-ceea-467f-a8d3-0242ac130003";
        let task = one.task().unwrap_or_default();
        // A task's own session id: as `AgentStarted` of a new Claude session links it.
        let link = crate::sessions::Link { runner: "claude".into(), id: id.into(), state: None };
        m.tasks.link(task, link);
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        assert_eq!(m.update(Action::TalkView { shows: View::Terminal }), [], "not while a turn runs");
        events(&mut m, &one, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(m.update(Action::TalkView { shows: View::Terminal }.sent_by(Origin::Socket)), []);
        assert_eq!(m.update(Action::TalkView { shows: View::Terminal }), [Effect::StopAgent { task: one.clone() }]);
        let resumed = m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        assert!(
            matches!(&resumed[..], [Effect::StartAgent { start: crate::agents::Start::Resume { session, .. }, .. }] if session == id),
            "{resumed:?}"
        );
        assert_eq!(m.talk_view(&one), View::Terminal);
        m.update(Action::AgentStarted { task: one.clone(), session: None, at: 2 }.sent_by(Origin::System));
        assert!(m.chat(&one).is_some_and(|chat| !chat.transcript.live), "the terminal has it now");
        assert_eq!(m.update(Action::AgentExited { task: one }.sent_by(Origin::System)), [], "no switch: no restart");
    }
}
