//! The native conversation in the drawer: transcripts folded from the runner's events, a
//! composer draft and a permission card per conversation, and the Chat | Terminal switch. Only
//! the user sends, answers, interrupts or switches, and only in the conversation shown.
//! Design: `docs/ai/agent-runners.md`.

use super::{Action, Effect, Model};
use crate::agents;
use crate::edit::Field;
use crate::runner::{RunnerId, Status, Support};
use crate::talk::Subject;
use crate::tasks::State;
use crate::transcript::{Event, Transcript, View};

/// Why a client's send, answer, interrupt or switch is refused.
pub const USERS_ALONE: &str = "only the user sends to, answers or interrupts a conversation's agent";
/// Why a client's report of an agent's events is refused: only ottospace's own runner reports.
pub const RUNNERS_ALONE: &str = "only ottospace's own runner reports an agent's events";

/// The most characters a composer draft holds.
pub const MAX_DRAFT: usize = 100_000;

/// The most conversations whose transcript is kept: those of the rail, and as many ended.
const KEPT: usize = crate::talk::RAIL * 2;

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
}

impl Chat {
    /// True if `ticket` is the card shown: an answer or Show all is for what the user saw.
    pub fn front_is(&self, ticket: u64) -> bool {
        self.transcript.cards.front().is_some_and(|card| card.ticket == ticket)
    }

    fn new(subject: Subject) -> Self {
        let (transcript, draft) = (Transcript::default(), Field::lines(MAX_DRAFT));
        Self { subject, transcript, draft, deny: false, armed: false }
    }
}

/// What a key does in a native conversation that has the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// Dispatch this action.
    Act(Action),
    /// ⇧↵: a new line in the draft.
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
/// ⇧↵ starts a new line, Esc leaves, or stops a turn when pressed twice.
pub fn key(key: &str, mods: crate::keys::Mods, card: Option<u64>) -> Key {
    if mods.cmd {
        return Key::Pass;
    }
    let bare = !mods.ctrl && !mods.alt && !mods.shift;
    match key {
        "escape" => Key::Act(Action::TalkEscape),
        _ if let Some(ticket) = card => match key {
            "a" if bare => Key::Act(Action::TalkAnswer { allow: true, ticket }),
            "d" if bare => Key::Act(Action::TalkAnswer { allow: false, ticket }),
            "s" if bare => Key::Act(Action::TalkExpand { ticket }),
            "m" if bare => Key::Act(Action::TalkDenyNote),
            _ => Key::Swallow,
        },
        "enter" if mods.shift => Key::Newline,
        "enter" if bare => Key::Act(Action::TalkSend),
        _ => Key::Field,
    }
}

impl Model {
    pub fn chat(&self, subject: &Subject) -> Option<&Chat> {
        self.chats.iter().find(|chat| chat.subject == *subject)
    }

    fn chat_mut(&mut self, subject: &Subject) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|chat| chat.subject == *subject)
    }

    /// The conversation the open drawer shows, if it is drawn natively and its agent runs.
    pub fn shown_chat(&self) -> Option<&Chat> {
        let talk = self.talk.as_ref().filter(|talk| talk.open)?;
        self.chat(&talk.task).filter(|chat| chat.transcript.live)
    }

    fn shown_chat_mut(&mut self) -> Option<&mut Chat> {
        let subject = self.talk.as_ref().filter(|talk| talk.open)?.task.clone();
        self.chat_mut(&subject).filter(|chat| chat.transcript.live)
    }

    /// The composer takes keys: the shown native conversation has the keyboard, and no card
    /// waits (or the user writes a Deny's message).
    pub fn talk_typing(&self) -> bool {
        let keys = self.talk.as_ref().is_some_and(|talk| talk.keys);
        keys && self.shown_chat().is_some_and(|chat| chat.transcript.cards.is_empty() || chat.deny)
    }

    pub(super) fn talk_draft(&self) -> Option<&Field> {
        self.shown_chat().map(|chat| &chat.draft)
    }

    pub(super) fn talk_draft_mut(&mut self) -> Option<&mut Field> {
        self.shown_chat_mut().map(|chat| &mut chat.draft)
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
        self.drawable(subject) && !busy
    }

    /// An agent started: drawn natively, its conversation gets a live transcript (kept from
    /// before, if it had one). `resumed`: its session was gone on with.
    pub(super) fn chat_started(&mut self, subject: &Subject, resumed: bool) {
        if self.talk_view(subject) != View::Native {
            return;
        }
        if self.chat(subject).is_none() {
            // Room for it: the oldest ended one goes.
            if self.chats.len() >= KEPT
                && let Some(at) = self.chats.iter().position(|chat| !chat.transcript.live)
            {
                self.chats.remove(at);
            }
            self.chats.push(Chat::new(subject.clone()));
        }
        // A new session is asked its task first: shown as the user's message, the brief's path aside.
        let title = subject.task().and_then(|id| self.tasks.get(id)).map(|task| task.title.clone());
        let now = self.now;
        if let Some(chat) = self.chat_mut(subject) {
            chat.transcript.started(resumed);
            (chat.deny, chat.armed) = (false, false);
            if let Some(title) = title.filter(|_| !resumed) {
                chat.transcript.send(&agents::prompt(&title, None), now);
            }
        }
    }

    /// Its agent is gone: its transcript stops being live. Returns whether the user switched it
    /// to the other view, and it is to be gone on with there.
    pub(super) fn chat_exited(&mut self, subject: &Subject) -> bool {
        if let Some(chat) = self.chat_mut(subject)
            && chat.transcript.live
        {
            chat.transcript.fold(crate::transcript::Event::Exited { code: None }, 0);
        }
        if let Some(chat) = self.chat_mut(subject) {
            (chat.deny, chat.armed) = (false, false);
        }
        let switching = self.switching.contains(subject);
        self.switching.retain(|of| of != subject);
        switching
    }

    /// After a switch: the session is gone on with in the view picked, now that the other
    /// process is gone. Never two processes on one session.
    pub(super) fn chat_restart(&mut self, subject: Subject, effects: &mut Vec<Effect>) {
        if self.agent_runs(&subject) {
            return;
        }
        if let Some((start @ agents::Start::Resume { .. }, cwd)) = self.agent_start(&subject) {
            self.starting.push((subject.clone(), None));
            effects.push(Effect::StartAgent { task: subject, start, cwd, brief: None });
        }
    }

    /// An adopted session's conversation is its task's now: its transcript, view and switch too.
    pub(super) fn chat_moves(&mut self, from: &Subject, to: &Subject) {
        self.chats.retain(|chat| chat.subject != *to);
        self.views.retain(|(subject, _)| subject != to);
        let rename = |subject: &mut Subject| {
            if subject == from {
                *subject = to.clone();
            }
        };
        self.chats.iter_mut().for_each(|chat| rename(&mut chat.subject));
        self.views.iter_mut().for_each(|(subject, _)| rename(subject));
        self.switching.iter_mut().for_each(rename);
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

    /// The session to watch: the open drawer's conversation, when its agent does not run here,
    /// its session is at work or held elsewhere, and its runner's file can be followed live
    /// (`attach.watch`). The shell reads that file and sends what it says (`AgentEvents`):
    /// a read-only transcript of labels, never the text.
    pub fn watch_target(&self) -> Option<(Subject, RunnerId, String, String)> {
        let subject = self.talk.as_ref().filter(|talk| talk.open)?.task.clone();
        let runner = self.runner_of(&subject)?;
        if self.agent_runs(&subject) || runner.spec().attach.watch != Support::Yes {
            return None;
        }
        let (name, id) = self.session_of(&subject)?;
        let found = self.facts(&name, &id).flatten()?;
        self.in_use(found).then(|| (subject, runner, id, found.cwd.clone()))
    }

    /// True if the watched session may be taken over here now: idle, and its runner can go on
    /// with an idle session (`attach.take_over_when_idle`). Taking over is the user's click on
    /// it, which resumes it.
    pub fn take_over_ready(&self, subject: &Subject) -> bool {
        let idle =
            self.session_of(subject).and_then(|(r, i)| self.facts(&r, &i).flatten()).is_some_and(|s| !self.in_use(s));
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

    /// The conversations gone with their tasks take their transcripts with them.
    pub(super) fn settle_chats(&mut self) {
        let tasks = &self.tasks;
        let gone = |subject: &Subject| subject.task().is_some_and(|id| tasks.get(id).is_none());
        self.chats.retain(|chat| !gone(&chat.subject));
        self.views.retain(|(subject, _)| !gone(subject));
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
                if self.watch_target().is_none_or(|(watched, ..)| watched != subject) {
                    return;
                }
                if self.chat(&subject).is_none() {
                    self.chats.retain(|chat| chat.transcript.live || chat.subject != subject);
                    if self.chats.len() >= KEPT
                        && let Some(at) = self.chats.iter().position(|chat| !chat.transcript.live)
                    {
                        self.chats.remove(at);
                    }
                    self.chats.push(Chat::new(subject.clone()));
                }
                let Some(chat) = self.chat_mut(&subject) else { return };
                chat.transcript.watched = true;
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
                for event in events {
                    chat.transcript.fold(event, at);
                }
                chat.armed &= chat.transcript.turn.is_some();
                // A Deny's message is for the card it was written under: another one in front ends it.
                chat.deny &= chat.transcript.cards.front().map(|card| card.ticket) == front;
                if turns {
                    self.chat_state(&subject);
                }
            }
            // ↵ while the user writes a Deny's message: the Deny, with it.
            Action::TalkSend if self.shown_chat().is_some_and(|chat| chat.deny) => {
                let front = self.shown_chat().and_then(|chat| chat.transcript.cards.front().map(|card| card.ticket));
                if let Some(ticket) = front {
                    self.chat_act(Action::TalkAnswer { allow: false, ticket }, effects)
                }
            }
            Action::TalkSend => {
                let now = self.now;
                let Some(chat) = self.shown_chat_mut().filter(|chat| !chat.transcript.busy()) else { return };
                let text = chat.draft.text().to_string();
                if text.trim().is_empty() {
                    return;
                }
                chat.transcript.send(&text, now);
                chat.draft = Field::lines(MAX_DRAFT);
                let subject = chat.subject.clone();
                effects.push(Effect::AgentSend { task: subject.clone(), text });
                self.chat_state(&subject);
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
                // Esc, Esc: the turn stops. One Esc only says so.
                chat.armed = !chat.armed;
                if !chat.armed {
                    effects.push(Effect::AgentInterrupt { task: chat.subject.clone() });
                }
            }
            Action::TalkInterrupt => {
                if let Some(chat) = self.shown_chat_mut().filter(|chat| chat.transcript.turn.is_some()) {
                    chat.armed = false;
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

    /// The task follows its native agent: running while a turn runs, needs you while a request
    /// waits or once the turn is over.
    fn chat_state(&mut self, subject: &Subject) {
        let (Some(task), Some(chat)) = (subject.task(), self.chat(subject)) else { return };
        let transcript = &chat.transcript;
        let state =
            if transcript.turn.is_some() && transcript.cards.is_empty() { State::Running } else { State::NeedsYou };
        if self.tasks.get(task).is_some_and(|task| task.state != state) {
            self.tasks.set_state(task, state);
        }
    }
}

#[cfg(test)]
mod tests {
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
            m.update(Action::TaskTalk { id: *id });
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

    fn typed(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Replace { range: None, text: text.into() } });
    }

    fn card(ticket: u64, cut: bool) -> Event {
        let (tool, label, detail) = ("Bash".into(), "git".into(), Text::new("command: git"));
        Event::Permission { ticket, tool, label, detail, len: 12, cut, claim: Text::default() }
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
        assert_eq!(m.update(Action::TalkSend), [], "a turn runs: ↵ sends nothing");
        events(&mut m, &one, vec![Event::TurnStarted, Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(m.tasks.get(one.task().unwrap_or_default()).map(|t| t.state), Some(State::NeedsYou));
        assert_eq!(m.update(Action::TalkSend), [Effect::AgentSend { task: one, text: "more".into() }]);
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
        assert_eq!((key("a", none, None), key("l", cmd, card)), (Key::Field, Key::Pass));
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
        m.update(Action::TalkPick { id: two.clone() });
        m.update(Action::TalkKeys { on: true });
        assert!(m.talk_typing());
        assert_eq!(m.update(Action::TalkAnswer { allow: false, ticket: 1 }), [], "no card here");
        typed(&mut m, "draft two");
        events(&mut m, &one, vec![Event::TextDelta { text: Text::new("more") }]);
        m.update(Action::TalkPick { id: one.clone() });
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
