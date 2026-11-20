//! The quit question's rows: what quitting ends (`Confirm::running`, `Confirm::agents`) and the
//! drafts it loses (`Confirm::unsent`). A draft is named by its kind and place, never its words.

use serde::Serialize;

use super::editor::Target;
use super::{Action, Confirm, Model, Origin};
use crate::capture::Source;
use crate::talk::Subject;

/// What kind of draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A conversation's composer.
    Message,
    /// A question card's typed "Other…".
    Answer,
    /// The btw column's next side question.
    Btw,
    /// The new-task box's line, open or put aside.
    NewTask,
    /// The capture draft in the right bar: a title or captures.
    Capture,
    /// ⌃G's editor on a message or on the new-task line.
    Editor,
}

/// A draft quitting would lose.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Unsent {
    pub kind: Kind,
    /// Where it is: a conversation's name, a capture's app or pane. Empty: the new-task box.
    pub place: String,
    /// The conversation it is in: going to it opens that.
    #[serde(skip)]
    pub of: Option<Subject>,
}

impl Unsent {
    /// Its row, e.g. "Unsent message · Fix the login".
    pub fn line(&self) -> String {
        let what = match self.kind {
            Kind::Message => "Unsent message",
            Kind::Answer => "Unsent answer",
            Kind::Btw => "Unsent side question",
            Kind::NewTask => "New task, not started",
            Kind::Capture => "Capture draft",
            Kind::Editor => "Message open in your editor",
        };
        match self.place.is_empty() {
            true => what.into(),
            false => format!("{what} · {}", self.place),
        }
    }
}

/// The quit question's section of what quitting ends, and of the drafts it loses.
pub const ENDS: &str = "Quitting ends these:";
pub const NOT_SENT: &str = "Not sent yet";

impl Confirm {
    /// The quit question's sections, each with its rows. Never one with no rows.
    pub fn sections(&self) -> Vec<(&'static str, Vec<String>)> {
        let unsent = self.unsent.iter().map(Unsent::line).collect();
        [(ENDS, self.lines()), (NOT_SENT, unsent)].into_iter().filter(|(_, rows)| !rows.is_empty()).collect()
    }

    /// It is the quit question, not one of the others the dialog asks.
    pub fn quits(&self) -> bool {
        self.dismiss.is_none()
            && self.paste.is_none()
            && self.unlist.is_none()
            && self.close.is_none()
            && self.later.is_none()
            && self.edited.is_none()
            && self.delete.is_none()
    }
}

/// More than spaces: a draft of only spaces or line breaks loses nothing.
fn words(text: &str) -> bool {
    !text.trim().is_empty()
}

impl Model {
    /// A conversation's name: its task's title, else its session's.
    fn named(&self, subject: &Subject) -> String {
        match subject {
            Subject::Task(id) => self.tasks.get(*id).map(|task| task.title.clone()).unwrap_or_default(),
            Subject::Session { runner, id } => self.session_title(runner, id),
        }
    }

    /// The agents quitting stops, by their conversation's name: drawn in a terminal or natively.
    pub fn quit_agents(&self) -> Vec<String> {
        self.agents.iter().map(|subject| self.named(subject)).collect()
    }

    /// The drafts quitting now loses, one row each, wherever they are: the conversation on show
    /// or another, kept for a task put away or done.
    pub fn unsent(&self) -> Vec<Unsent> {
        let editing = self.editing.as_ref().map(|editing| &editing.target);
        let mut rows = Vec::new();
        let mut row = |kind, place: String, of: Option<Subject>| rows.push(Unsent { kind, place, of });
        for chat in &self.chats {
            let (subject, place) = (&chat.subject, self.named(&chat.subject));
            // The editor's row stands for the message it holds.
            // A message waiting on its agent's start is unsent too.
            let message = words(chat.draft.text()) || chat.queued.is_some();
            if message && editing != Some(&Target::Chat(subject.clone())) {
                row(Kind::Message, place.clone(), Some(subject.clone()));
            }
            let forms = chat.transcript.cards.iter().filter_map(|card| card.ask.as_ref());
            let other = |form: &crate::ask::Form| {
                (0..form.questions.len()).any(|q| form.other(q).is_some_and(|field| words(field.text())))
            };
            if forms.into_iter().any(other) {
                row(Kind::Answer, place.clone(), Some(subject.clone()));
            }
            if chat.btw.as_ref().is_some_and(|btw| words(btw.field.text())) {
                row(Kind::Btw, place, Some(subject.clone()));
            }
        }
        // While a capture draft is in the bar, `compose` is its title.
        // The editor's row stands for the open line, not one put aside.
        let open = self.compose.as_ref().filter(|_| self.capture.is_none() && editing != Some(&Target::Compose));
        let kept = self.compose_kept.as_ref().map(|kept| &kept.line);
        if open.into_iter().chain(kept).any(|line| words(line.text())) {
            row(Kind::NewTask, String::new(), None);
        }
        if let Some(bar) = &self.capture {
            let titled = self.compose.as_ref().is_some_and(|title| words(title.text()));
            if titled || !bar.materials.is_empty() {
                let from = |n: usize| match &bar.materials[n].source {
                    Source::App { .. } => bar.materials[n].source.name(),
                    Source::Terminal { .. } => format!("pane {}", bar.materials[n].pane),
                };
                let place = match bar.materials.len() {
                    0 => String::new(),
                    1 => from(0),
                    n => format!("{} +{}", from(0), n - 1),
                };
                row(Kind::Capture, place, None);
            }
        }
        match editing {
            Some(Target::Chat(subject)) => row(Kind::Editor, self.named(subject), Some(subject.clone())),
            Some(Target::Compose) => row(Kind::Editor, "New task".into(), None),
            None => {}
        }
        rows
    }

    /// The user's `QuitGo`, as what the user would do, in turn: cancel the quit, then go to the
    /// draft at `row` of its list, e.g. its conversation with the keyboard in it. `None`: no
    /// such row, or not the user's.
    pub(super) fn quit_go(&self, sent: &super::Envelope) -> Option<Vec<Action>> {
        let Action::QuitGo { row } = sent.action else { return None };
        let question = self.confirm.as_ref().filter(|confirm| sent.origin == Origin::User && confirm.quits());
        let go = question.and_then(|confirm| confirm.unsent.get(row))?;
        // An editor's row gives the keyboard to the editor, not the composer under it.
        let keys = match go.kind {
            Kind::Editor => Action::EditorKeys { on: true },
            Kind::Btw => Action::BtwKeys { on: true },
            _ => Action::TalkKeys { on: true },
        };
        let open = match &go.of {
            Some(Subject::Task(id)) => vec![Action::TaskTalk { id: *id }, keys],
            Some(Subject::Session { runner, id }) => {
                vec![Action::SessionOpen { runner: runner.clone(), id: id.clone() }, keys]
            }
            None if go.kind == Kind::Capture => vec![Action::CaptureTitle],
            // One draft at a time: while a capture is in the bar, the box waits (⌘N is its title).
            None if self.capture.is_some() => Vec::new(),
            // The box, still open, has the keyboard again once the question is gone.
            None if self.compose.is_some() => Vec::new(),
            None => vec![Action::OpenCompose { text: String::new() }],
        };
        // What lies over the workspace gives way first, or the draft's place would wait under it.
        let clear =
            [Action::QuitCancel, Action::ClosePicker, Action::ClosePalette, Action::CloseKeys, Action::LeaveVisual];
        Some([clear.to_vec(), open].concat())
    }
}

#[cfg(test)]
mod tests {
    use super::{ENDS, Kind, NOT_SENT};
    use crate::edit::Edit;
    use crate::model::tests::started;
    use crate::model::{Action, Confirm, Effect, Model, Origin, Running};
    use crate::talk::Subject;
    use crate::tasks::State;
    use crate::transcript::{Event, Outcome, Text};

    /// A task whose Claude agent runs natively, shown, its first turn over.
    fn live() -> (Model, Subject) {
        let mut m = Model::default();
        let assign = crate::tasks::Assign::default();
        m.update(Action::TaskAdd { title: "fix login".into(), priority: 3, state: State::Triaged, tab: None, assign });
        let id = m.tasks.list[0].id;
        let one = Subject::Task(id);
        started(&mut m, Action::TaskTalk { id });
        m.update(Action::AgentStarted { task: one.clone(), session: None, at: 1 }.sent_by(Origin::System));
        events(&mut m, &one, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        (m, one)
    }

    /// The same, its agent ended since: nothing runs.
    fn ended() -> (Model, Subject) {
        let (mut m, one) = live();
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        (m, one)
    }

    fn events(m: &mut Model, subject: &Subject, events: Vec<Event>) {
        m.update(Action::AgentEvents { task: subject.clone(), events, at: 5 }.sent_by(Origin::System));
    }

    /// The composer's words, as the user typed them.
    fn draft(m: &mut Model, subject: &Subject, text: &str) {
        let chat = m.chat_mut(subject).expect("its chat");
        chat.draft.apply(Edit::Replace { range: None, text: text.into() });
    }

    fn ask(m: &mut Model) -> Vec<Effect> {
        m.update(Action::QuitAsked { running: Vec::new() })
    }

    fn rows(m: &Model) -> Vec<String> {
        m.unsent().iter().map(super::Unsent::line).collect()
    }

    fn sections(m: &Model) -> Vec<(&'static str, Vec<String>)> {
        m.confirm.as_ref().map(Confirm::sections).unwrap_or_default()
    }

    /// The owner's "A draft is open" over an empty list: no draft, an agent drawn natively. The
    /// shell listed only the drawer's terminals' agents. Its row now names it; no drafts section.
    #[test]
    fn a_native_agent_is_named_with_what_quitting_ends_and_is_no_draft() {
        let (mut m, _) = live();
        assert_eq!(ask(&mut m), []);
        assert_eq!(sections(&m), [(ENDS, vec!["agent · fix login".to_string()])]);
        assert_eq!(m.confirm.as_ref().map(Confirm::title).as_deref(), Some("Quit?"));
    }

    /// A message in a conversation's composer: its row names the task, never the words.
    #[test]
    fn an_unsent_message_is_named_by_its_task() {
        let (mut m, one) = ended();
        draft(&mut m, &one, "secret plan");
        assert_eq!(rows(&m), ["Unsent message · fix login"]);
        assert_eq!(ask(&mut m), []);
        assert_eq!(sections(&m), [(NOT_SENT, vec!["Unsent message · fix login".to_string()])]);
        let state = serde_json::to_string(&m.confirm).unwrap_or_default();
        assert!(state.contains(r#""kind":"message""#) && !state.contains("secret"), "{state}");
    }

    /// Spaces or line breaks alone lose nothing: no question.
    #[test]
    fn a_draft_of_only_spaces_is_no_draft() {
        let (mut m, one) = ended();
        draft(&mut m, &one, "  \n\t ");
        assert_eq!(ask(&mut m), [Effect::Quit]);
        m.update(Action::OpenCompose { text: "   ".into() });
        assert_eq!(ask(&mut m), [Effect::Quit]);
    }

    /// A running program and a draft: both sections, what ends first.
    #[test]
    fn a_running_program_and_a_draft_show_both_sections() {
        let (mut m, one) = ended();
        draft(&mut m, &one, "fix it");
        m.update(Action::Split { axis: crate::layout::Axis::Right, kind: crate::model::Kind::Terminal });
        let sleep = Running { pane: 1, program: Some("sleep".into()), background: false };
        m.update(Action::QuitAsked { running: vec![sleep] });
        let sections = sections(&m);
        assert_eq!(sections.iter().map(|(head, _)| *head).collect::<Vec<_>>(), [ENDS, NOT_SENT]);
        assert_eq!(sections[0].1, ["pane 1 · sleep"]);
    }

    /// The new-task box's line, open or put aside when a conversation opened.
    #[test]
    fn the_new_task_line_is_a_draft_open_or_put_aside() {
        let (mut m, one) = ended();
        m.update(Action::OpenCompose { text: "plan the week".into() });
        assert_eq!(rows(&m), ["New task, not started"]);
        m.update(Action::TaskTalk { id: one.task().unwrap_or_default() });
        assert!(m.compose.is_none(), "put aside");
        assert_eq!(rows(&m), ["New task, not started"]);
    }

    /// A question card's typed "Other…" answer.
    #[test]
    fn a_typed_other_answer_is_a_draft() {
        let (mut m, one) = live();
        let questions = vec![crate::ask::tests::question(2, false)];
        let (tool, label) = ("AskUserQuestion".to_string(), "AskUserQuestion".to_string());
        let (detail, claim) = (Text::default(), Text::default());
        let card = Event::Permission { ticket: 1, tool, label, detail, len: 0, cut: false, claim, questions };
        events(&mut m, &one, vec![Event::TurnStarted, card]);
        assert_eq!(rows(&m), Vec::<String>::new());
        let chat = m.chat_mut(&one).expect("its chat");
        let form = chat.transcript.cards.front_mut().and_then(|card| card.ask.as_mut()).expect("a form");
        form.cursor = (0, form.rows(0) - 1);
        form.typing().expect("Other…").apply(Edit::Insert("  ".into()));
        assert_eq!(rows(&m), Vec::<String>::new(), "spaces alone");
        let chat = m.chat_mut(&one).expect("its chat");
        let form = chat.transcript.cards.front_mut().and_then(|card| card.ask.as_mut()).expect("a form");
        form.typing().expect("Other…").apply(Edit::Insert("mine".into()));
        assert_eq!(rows(&m), ["Unsent answer · fix login"]);
    }

    /// ⌃G's editor: one row for the message it holds, not one more for the composer's copy.
    #[test]
    fn a_message_in_the_editor_is_one_row() {
        let (mut m, _) = live();
        m.update(Action::FieldEdit { edit: Edit::Replace { range: None, text: "fix it".into() } });
        m.update(Action::EditorOpen);
        assert!(m.editing.is_some());
        assert_eq!(rows(&m), ["Message open in your editor · fix login"]);
    }

    /// A draft's row goes to it from under the picker: the picker gives way.
    #[test]
    fn a_drafts_row_goes_to_it_from_under_the_picker() {
        let (mut m, one) = ended();
        draft(&mut m, &one, "fix it");
        let assign = crate::tasks::Assign::default();
        m.update(Action::TaskAdd { title: "other".into(), priority: 3, state: State::Triaged, tab: None, assign });
        m.update(Action::TaskTalk { id: m.tasks.list[1].id });
        m.update(Action::OpenPicker { axis: crate::layout::Axis::Right, new_tab: false, items: Vec::new() });
        ask(&mut m);
        m.update(Action::QuitGo { row: 0 });
        assert!(m.picker.is_none() && m.confirm.is_none());
        assert!(m.talk.as_ref().is_some_and(|talk| talk.task == one));
        assert_eq!(m.field().map(|(of, _)| of), Some("talk"));
    }

    /// The btw column's typed question: a row, and its row gives that input the keyboard.
    #[test]
    fn a_typed_side_question_is_a_draft_and_its_row_goes_to_it() {
        let (mut m, _) = live();
        m.update(Action::BtwOpen);
        m.update(Action::FieldEdit { edit: Edit::Replace { range: None, text: "  ".into() } });
        assert_eq!(rows(&m), Vec::<String>::new(), "spaces alone");
        m.update(Action::FieldEdit { edit: Edit::Replace { range: None, text: "what now?".into() } });
        assert_eq!(rows(&m), ["Unsent side question · fix login"]);
        m.update(Action::BtwKeys { on: false });
        ask(&mut m);
        m.update(Action::QuitGo { row: 0 });
        assert!(m.confirm.is_none() && m.btw_typing());
    }

    /// A message ↵ sent while its agent was not running waits on its start: unsent too.
    #[test]
    fn a_message_waiting_on_its_agents_start_is_unsent() {
        let (mut m, one) = ended();
        m.chat_mut(&one).expect("its chat").queued = Some("go on".into());
        assert_eq!(rows(&m), ["Unsent message · fix login"]);
    }

    /// ⌃G on the new-task line: its row stands for that line, and a line put aside still has its own.
    #[test]
    fn a_line_put_aside_is_listed_beside_the_editor_on_another() {
        let (mut m, one) = ended();
        m.update(Action::OpenCompose { text: "mine".into() });
        m.update(Action::TaskTalk { id: one.task().unwrap_or_default() });
        m.update(Action::OpenCompose { text: "another".into() });
        m.update(Action::EditorOpen);
        assert!(m.editing.is_some());
        assert_eq!(rows(&m), ["New task, not started", "Message open in your editor · New task"]);
    }

    /// The editor's row gives the keyboard to the editor, not to a card that took it.
    #[test]
    fn the_editors_row_gives_the_editor_the_keyboard() {
        let (mut m, _) = live();
        m.update(Action::FieldEdit { edit: Edit::Replace { range: None, text: "fix it".into() } });
        m.update(Action::EditorOpen);
        m.update(Action::EditorKeys { on: false });
        assert!(!m.editor_keys());
        ask(&mut m);
        m.update(Action::QuitGo { row: 0 });
        assert!(m.confirm.is_none() && m.editor_keys());
    }

    /// A draft kept for a task put away still counts, named so the user finds it. Its row cancels
    /// the quit and opens it with the keyboard in the composer. Only the user's click does.
    #[test]
    fn a_drafts_row_goes_to_it_even_for_a_task_put_away() {
        let (mut m, one) = live();
        let id = one.task().unwrap_or_default();
        m.update(Action::TaskLater { id });
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        draft(&mut m, &one, "fix it");
        assert!(m.put_away(id));
        assert_eq!(ask(&mut m), []);
        assert_eq!(m.confirm.as_ref().map(|c| c.unsent[0].kind), Some(Kind::Message));
        assert_eq!(m.refusal(&Action::QuitGo { row: 0 }, Origin::Socket), Some(crate::model::CONFIRM_UP));
        assert_eq!(m.update(Action::QuitGo { row: 0 }.sent_by(Origin::Socket)), []);
        assert!(m.confirm.is_some());
        let effects = m.update(Action::QuitGo { row: 0 });
        assert!(!effects.contains(&Effect::Quit), "{effects:?}");
        assert!(m.confirm.is_none());
        assert!(m.talk.as_ref().is_some_and(|talk| talk.open && talk.task == one));
        assert_eq!(m.field().map(|(of, _)| of), Some("talk"));
    }
}
