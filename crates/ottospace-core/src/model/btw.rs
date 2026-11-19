//! "btw": side questions to a conversation's running agent, in a column beside it. Answered from
//! the agent's context by its runner (`RunnerSpec::structured.side_question`), never in the
//! main conversation: no message, no answer to a card, no interrupt. A thread lives with the
//! agent's process, its conversation and until the user closes it. Only the user asks, uses or
//! closes; only the shell's runner says what came back. Design: `docs/ai/agent-runners.md`.

use super::chat::{Key, MAX_DRAFT};
use super::{Action, Effect, Model};
use crate::edit::{Edit, Field};
use crate::talk::Subject;
use crate::transcript::Text;

/// Why a client's open, ask, use or close of a btw is refused.
pub const USERS_ALONE: &str = "only the user asks, uses or closes a side question";
/// Why a client's report of a side answer is refused.
pub const RUNNERS_ALONE: &str = "only ottospace's own runner reports a side answer";

/// The most questions a thread keeps: past it the oldest go, and leave its history.
pub const MAX_PAIRS: usize = 40;

/// One conversation's side thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Btw {
    pub pairs: Vec<Pair>,
    /// Its input: the user's next question.
    pub field: Field,
    /// Its input has the drawer's keyboard, not the composer.
    pub keys: bool,
}

/// A question and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub id: u64,
    /// The user's own words.
    pub question: String,
    pub reply: Reply,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Waiting,
    /// Untrusted: drawn, never in the state. `synthetic`: the CLI's own words (an API error).
    Answer {
        text: Text,
        synthetic: bool,
    },
    Failed,
}

impl Btw {
    fn new() -> Self {
        Self { pairs: Vec::new(), field: Field::lines(MAX_DRAFT), keys: true }
    }

    /// The question in flight: one at a time.
    pub fn asking(&self) -> Option<u64> {
        self.pairs.iter().find(|pair| pair.reply == Reply::Waiting).map(|pair| pair.id)
    }

    /// The answered pairs, the model's own, as the runner takes them back: question, answer.
    pub fn history(&self) -> Vec<(String, String)> {
        let answered = self.pairs.iter().filter_map(|pair| match &pair.reply {
            Reply::Answer { text, synthetic: false } => Some((pair.question.clone(), text.text().to_string())),
            _ => None,
        });
        answered.collect()
    }

    /// What a key does in its input: ↵ asks, ⇧↵ a new line, Esc or ⌘⇧K back to the composer.
    pub fn key(key: &str, mods: crate::keys::Mods) -> Key {
        let bare = !mods.ctrl && !mods.alt && !mods.shift && !mods.cmd;
        match key {
            _ if is_key(key, mods) => Key::Act(Action::BtwKeys { on: false }),
            _ if mods.cmd => Key::Pass,
            "escape" => Key::Act(Action::BtwKeys { on: false }),
            "enter" if mods.shift && !mods.ctrl && !mods.alt => Key::Newline,
            "enter" if bare => Key::Act(Action::BtwAsk),
            _ => Key::Field,
        }
    }
}

/// ⌘⇧K: the btw column, from the conversation (`chat::key`) or back from it.
pub fn is_key(key: &str, mods: crate::keys::Mods) -> bool {
    key == "k" && mods.cmd && mods.shift && !mods.ctrl && !mods.alt
}

impl Model {
    /// True if the conversation offers a btw: its agent runs here, natively, and its runner takes
    /// side questions. Never for a watched one, nor a runner whose spec says no (Codex).
    pub fn btw_offered(&self, subject: &Subject) -> bool {
        let runs = self.chat(subject).is_some_and(|chat| chat.transcript.live && !chat.transcript.watched);
        runs && self.runner_of(subject).is_some_and(|runner| runner.spec().structured.side_question)
    }

    /// The shown conversation's btw, if its column is open.
    pub fn shown_btw(&self) -> Option<&Btw> {
        self.shown_chat().and_then(|chat| chat.btw.as_ref())
    }

    /// The btw's input has the keyboard: typing goes there, not to the composer.
    pub fn btw_typing(&self) -> bool {
        let keys = self.talk.as_ref().is_some_and(|talk| talk.keys);
        keys && self.shown_btw().is_some_and(|btw| btw.keys)
    }

    pub(super) fn btw_field(&self) -> Option<&Field> {
        self.shown_btw().map(|btw| &btw.field)
    }

    pub(super) fn btw_edit(&mut self, edit: Edit) {
        if let Some(btw) = self.shown_btw_mut() {
            btw.field.apply(edit);
        }
    }

    fn shown_btw_mut(&mut self) -> Option<&mut Btw> {
        self.shown_chat_mut()?.btw.as_mut()
    }

    pub(super) fn btw_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        let shown = self.shown_chat().map(|chat| chat.subject.clone());
        match action {
            // Not while the user's editor has a draft: it has the keyboard.
            Action::BtwOpen | Action::BtwKeys { on: true } if self.editing.is_some() => {}
            Action::BtwOpen | Action::BtwKeys { on: true } => {
                let Some(subject) = shown.filter(|subject| self.btw_offered(subject)) else { return };
                let open = matches!(action, Action::BtwOpen);
                let Some(chat) = self.chat_mut(&subject) else { return };
                match &mut chat.btw {
                    Some(btw) => btw.keys = true,
                    None if open => chat.btw = Some(Btw::new()),
                    None => return,
                }
                self.talk_keys(true, effects);
            }
            Action::BtwKeys { on: false } => {
                if let Some(btw) = self.shown_btw_mut() {
                    btw.keys = false;
                }
            }
            Action::BtwAsk => {
                let id = self.btw_next + 1;
                let Some(subject) = shown else { return };
                let Some(btw) = self.shown_btw_mut().filter(|btw| btw.asking().is_none()) else { return };
                let question = btw.field.text().trim().to_string();
                if question.is_empty() {
                    return;
                }
                let history = btw.history();
                btw.field = Field::lines(MAX_DRAFT);
                btw.pairs.push(Pair { id, question: question.clone(), reply: Reply::Waiting });
                if btw.pairs.len() > MAX_PAIRS {
                    btw.pairs.remove(0);
                }
                self.btw_next = id;
                effects.push(Effect::AgentSideQuestion { task: subject, id, question, history });
            }
            // The shell's runner alone reports: a key or click of the user's is none.
            Action::BtwAnswered { .. } | Action::BtwFailed { .. } if self.by_user => {}
            Action::BtwAnswered { task, id, text, synthetic } => {
                self.btw_reply(&task, id, Reply::Answer { text, synthetic });
            }
            Action::BtwFailed { task, id } => self.btw_reply(&task, id, Reply::Failed),
            Action::BtwUse { id } => {
                let Some(chat) = self.shown_chat_mut() else { return };
                let answer = chat.btw.as_ref().and_then(|btw| btw.pairs.iter().find(|pair| pair.id == id));
                let Some(Reply::Answer { text, .. }) = answer.map(|pair| pair.reply.clone()) else { return };
                // An open question card's "Other…", else the composer; never sent.
                let card = !chat.deny;
                match chat.transcript.form_mut().filter(|_| card) {
                    Some(form) => form.fill_other(text.text()),
                    None => {
                        chat.draft.apply(Edit::Paste(text.text().to_string()));
                        chat.untrusted = true;
                    }
                }
                if let Some(btw) = &mut chat.btw {
                    btw.keys = false;
                }
            }
            Action::BtwClose => {
                if let Some(chat) = self.shown_chat_mut() {
                    chat.btw = None;
                }
            }
            _ => {}
        }
    }

    /// What came of question `id`: only the one waiting, of a thread still open.
    fn btw_reply(&mut self, task: &Subject, id: u64, reply: Reply) {
        let Some(btw) = self.chat_mut(task).and_then(|chat| chat.btw.as_mut()) else { return };
        if let Some(pair) = btw.pairs.iter_mut().find(|pair| pair.id == id && pair.reply == Reply::Waiting) {
            pair.reply = reply;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::started;
    use crate::model::{Envelope, Origin};
    use crate::tasks::State;
    use crate::transcript::{Event, Outcome};

    /// A task whose Claude agent runs natively, shown, its first turn running.
    fn live(agent: Option<&str>) -> (Model, Subject) {
        let mut m = Model::default();
        let assign = crate::tasks::Assign { agent: agent.map(str::to_string), ..Default::default() };
        m.update(Action::TaskAdd { title: "fix login".into(), priority: 3, state: State::Triaged, tab: None, assign });
        let id = m.tasks.list[0].id;
        started(&mut m, Action::TaskTalk { id });
        let subject = Subject::Task(id);
        m.update(Action::AgentStarted { task: subject.clone(), session: None, at: 1 }.sent_by(Origin::System));
        m.update(
            Action::AgentEvents { task: subject.clone(), events: vec![Event::TurnStarted], at: 2 }
                .sent_by(Origin::System),
        );
        (m, subject)
    }

    fn typed(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: Edit::Replace { range: None, text: text.into() } });
    }

    fn answered(task: &Subject, id: u64, text: &str) -> Envelope {
        Action::BtwAnswered { task: task.clone(), id, text: Text::new(text), synthetic: false }.sent_by(Origin::System)
    }

    #[test]
    fn a_side_question_mid_turn_goes_beside_it_and_its_history_after() {
        let (mut m, one) = live(None);
        let before = m.chat(&one).unwrap().transcript.clone();
        assert!(m.btw_offered(&one));
        m.update(Action::BtwOpen);
        assert!(m.btw_typing() && m.field().is_some_and(|(of, _)| of == "btw"));
        typed(&mut m, "what is it doing?");
        let effects = m.update(Action::BtwAsk);
        let asked = Effect::AgentSideQuestion {
            task: one.clone(),
            id: 1,
            question: "what is it doing?".into(),
            history: vec![],
        };
        assert_eq!(effects, [asked]);
        // One at a time.
        typed(&mut m, "and?");
        assert_eq!(m.update(Action::BtwAsk), []);
        m.update(answered(&one, 1, "it runs `cargo test`"));
        assert_eq!(m.shown_btw().unwrap().asking(), None);
        let effects = m.update(Action::BtwAsk);
        let history = vec![("what is it doing?".to_string(), "it runs `cargo test`".to_string())];
        assert!(matches!(&effects[..], [Effect::AgentSideQuestion { id: 2, history: h, .. }] if *h == history));
        // A synthetic answer is shown, never taken back as history.
        m.update(
            Action::BtwAnswered { task: one.clone(), id: 2, text: Text::new("(API error)"), synthetic: true }
                .sent_by(Origin::System),
        );
        assert_eq!(m.shown_btw().unwrap().history(), history);
        // The main conversation is as it was; the turn still runs.
        assert_eq!(m.chat(&one).unwrap().transcript, before);
        // Interrupting the main turn leaves the thread.
        m.update(Action::TalkInterrupt);
        m.update(
            Action::AgentEvents {
                task: one.clone(),
                events: vec![Event::TurnFinished { outcome: Outcome::Interrupted }],
                at: 3,
            }
            .sent_by(Origin::System),
        );
        assert_eq!(m.shown_btw().map(|btw| btw.pairs.len()), Some(2));
        // Its agent ends: so does the thread.
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        assert!(m.chat(&one).unwrap().btw.is_none());
    }

    #[test]
    fn only_the_user_asks_uses_or_closes_and_only_the_runner_answers() {
        let (mut m, one) = live(None);
        // What takes the keyboard says first that the user is typing.
        for action in [Action::BtwOpen, Action::BtwKeys { on: true }] {
            assert!(m.refusal(&action, Origin::Socket).is_some(), "{action:?}");
        }
        for action in [Action::BtwAsk, Action::BtwUse { id: 1 }, Action::BtwClose, Action::BtwKeys { on: false }] {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(USERS_ALONE), "{action:?}");
            assert_eq!(m.refusal(&action, Origin::System), Some(USERS_ALONE), "{action:?}");
        }
        assert_eq!(m.update(Action::BtwOpen.sent_by(Origin::Socket)), []);
        assert!(m.shown_btw().is_none());
        m.update(Action::BtwOpen);
        typed(&mut m, "why?");
        assert_eq!(m.refusal(&Action::FieldEdit { edit: Edit::Paste("x".into()) }, Origin::Socket), Some(USERS_ALONE));
        m.update(Action::BtwAsk);
        let report = Action::BtwAnswered { task: one.clone(), id: 1, text: Text::new("x"), synthetic: false };
        assert_eq!(m.refusal(&report, Origin::Socket), Some(RUNNERS_ALONE));
        m.update(report.clone());
        assert_eq!(m.shown_btw().unwrap().asking(), Some(1), "a user's report is none");
        m.update(Action::BtwFailed { task: one.clone(), id: 1 }.sent_by(Origin::System));
        assert_eq!(m.shown_btw().unwrap().pairs[0].reply, Reply::Failed);
        // An answer to a question asked before a close goes nowhere.
        m.update(Action::BtwClose);
        m.update(answered(&one, 1, "late"));
        assert!(m.shown_btw().is_none());
    }

    #[test]
    fn use_as_my_answer_fills_the_composer_or_a_cards_other_and_sends_nothing() {
        let (mut m, one) = live(None);
        m.update(Action::BtwOpen);
        typed(&mut m, "which one?");
        m.update(Action::BtwAsk);
        m.update(answered(&one, 1, "Redis"));
        assert_eq!(m.update(Action::BtwUse { id: 1 }), []);
        assert_eq!(m.chat(&one).unwrap().draft.text(), "Redis");
        assert!(m.chat(&one).unwrap().untrusted, "the model's words: out of the state");
        assert!(!m.btw_typing() && m.talk_typing(), "the keyboard back to the composer");
        // A question card: its "Other…".
        let q = crate::ask::Question {
            question: Text::new("Which cache?"),
            header: Text::new("Cache"),
            options: vec![crate::ask::Choice { label: Text::new("SQLite"), description: Text::new("a file") }],
            multi: false,
        };
        let card = Event::Permission {
            ticket: 1,
            tool: "AskUserQuestion".into(),
            label: "ask".into(),
            detail: Text::new("q"),
            len: 1,
            cut: false,
            claim: Text::default(),
            questions: vec![q],
        };
        m.update(Action::AgentEvents { task: one.clone(), events: vec![card], at: 4 }.sent_by(Origin::System));
        assert_eq!(m.update(Action::BtwUse { id: 1 }), []);
        let form = m.shown_chat().unwrap().form().unwrap();
        assert_eq!((form.other(0).map(Field::text), form.on_other(), form.answered(0)), (Some("Redis"), true, true));
        assert_eq!(m.shown_chat().unwrap().transcript.cards.len(), 1, "never answered");
    }

    /// The user's editor and the btw's input never both have the keyboard.
    #[test]
    fn the_editor_takes_the_keys_from_the_btw() {
        let (mut m, _) = live(None);
        m.update(Action::BtwOpen);
        assert!(m.btw_typing());
        m.update(Action::EditorOpen);
        assert!(m.editing.is_some() && !m.btw_typing());
        m.update(Action::BtwKeys { on: true });
        assert!(!m.btw_typing());
    }

    #[test]
    fn nothing_is_offered_for_a_codex_task() {
        let (mut m, one) = live(Some("codex"));
        assert!(!m.btw_offered(&one));
        m.update(Action::BtwOpen);
        assert!(m.shown_btw().is_none());
    }
}
