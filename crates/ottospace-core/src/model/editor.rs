//! Compose in the user's editor (⌃G): a conversation's draft, or the new-task box's line, goes
//! to `$VISUAL`/`$EDITOR` in the drawer, and what it saved comes back to that same draft, never
//! sent. Only the user opens it or moves the keyboard to it; only the shell says what it saved.
//! The shell writes and reads the file and runs the editor.

use super::{Effect, Mode, Model};
use crate::talk::Subject;

/// Why a client's ⌃G, or its move of the keyboard to or from the editor, is refused.
pub const USERS_ALONE: &str = "only the user opens their editor on a message";
/// Why a client's report of what the editor saved is refused.
pub const SHELLS_ALONE: &str = "only ottospace reads what the editor saved";
/// Why a client's key reaches nothing while the editor has the keyboard.
pub const TYPES_ALONE: &str = "only the user types into their editor";

/// The fewest terminal columns the drawer has while it shows the editor: narrower, it widens.
pub const COLUMNS: usize = 60;

/// How long the status bar says how the editor's text came back.
pub const NOTE_MS: u64 = 6_000;

/// What the editor edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A conversation's composer draft.
    Chat(Subject),
    /// The new-task box's line, in the drawer.
    Compose,
}

/// The editor that runs, one at a time.
#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub target: Target,
    /// What the editor was given: the same text back changes nothing.
    pub given: String,
    /// The editor has the keyboard. False: the user clicked the card shown above it.
    pub keys: bool,
}

/// The text a saved file holds, less the one trailing newline an editor adds.
pub fn saved_text(saved: &str) -> &str {
    let text = saved.strip_suffix('\n').unwrap_or(saved);
    if text.len() < saved.len() { text.strip_suffix('\r').unwrap_or(text) } else { text }
}

impl Model {
    /// What ⌃G edits now: the new-task box that has the keyboard, else the shown composer that
    /// takes a message, while no card waits.
    fn editor_target(&self) -> Option<Target> {
        if self.compose_modal() && self.mode() == Mode::Compose {
            return Some(Target::Compose);
        }
        let chat = self.shown_draft_chat().filter(|chat| chat.transcript.cards.is_empty() && !chat.deny)?;
        Some(Target::Chat(chat.subject.clone()))
    }

    /// True if the editor runs on this conversation's draft: its composer takes no typing.
    pub fn edits(&self, subject: &Subject) -> bool {
        self.editing.as_ref().is_some_and(|editing| editing.target == Target::Chat(subject.clone()))
    }

    /// True if the editor runs on the new-task box's line, open in the drawer.
    pub(super) fn edits_compose(&self) -> bool {
        self.compose_modal() && self.editing.as_ref().is_some_and(|editing| editing.target == Target::Compose)
    }

    /// The conversation still keeps a draft: its transcript is here, and its task is not dismissed.
    fn keeps_draft(&self, subject: &Subject) -> bool {
        self.chat(subject).is_some() && subject.task().is_none_or(|id| self.tasks.live(id).is_some())
    }

    /// The drawer shows the editor in place of what it edits: the new-task box is open, or the
    /// conversation shown is the one it edits (or its own is gone, so it is not lost from view).
    pub fn editor_shown(&self) -> bool {
        let Some(editing) = &self.editing else { return false };
        match &editing.target {
            Target::Compose => self.compose_modal(),
            // The new-task box takes the drawer over any conversation.
            Target::Chat(_) if self.compose.is_some() || self.welcome() => false,
            Target::Chat(subject) => {
                let open = self.talk.as_ref().filter(|talk| talk.open);
                open.is_some_and(|talk| talk.task == *subject || !self.keeps_draft(subject))
            }
        }
    }

    /// The drawer's editor has the keyboard: keys go to it, never to the composer or a card.
    pub fn editor_keys(&self) -> bool {
        let Some(editing) = self.editing.as_ref().filter(|editing| editing.keys) else { return false };
        let mode = match editing.target {
            Target::Compose => Mode::Compose,
            Target::Chat(_) => Mode::Talk,
        };
        self.editor_shown() && self.mode() == mode
    }

    /// What the status bar says of the last text the editor gave back, for `NOTE_MS`.
    pub fn editor_note(&self) -> Option<&str> {
        let (text, at) = self.editor_note.as_ref()?;
        (self.now < at.saturating_add(NOTE_MS)).then_some(text.as_str())
    }

    /// ⌃G: the editor on what the user writes. Open already: the keyboard goes back to it, if
    /// the drawer shows it; else the user is told where it is.
    pub(super) fn editor_open(&mut self, effects: &mut Vec<Effect>) {
        if let Some(editing) = &self.editing {
            if !self.editor_shown() {
                let text = "your editor is open on another message: finish it there first".into();
                return effects.push(Effect::Notice { text });
            }
            let chat = matches!(editing.target, Target::Chat(_));
            return self.editor_keys_to(true, chat, effects);
        }
        let Some(target) = self.editor_target() else { return };
        let given = match &target {
            Target::Compose => self.compose_line().cloned().unwrap_or_default(),
            Target::Chat(subject) => self.chat(subject).map(|chat| chat.draft.text().to_string()).unwrap_or_default(),
        };
        let chat = matches!(target, Target::Chat(_));
        self.editing = Some(Editing { target, given: given.clone(), keys: true });
        self.editor_note = None;
        if chat {
            self.talk_keys(true, effects);
        }
        effects.push(Effect::OpenEditor { text: given });
    }

    /// The keyboard to the editor (`on`), or to the card above it. Either way it is in the drawer.
    pub(super) fn editor_keys_to(&mut self, on: bool, chat: bool, effects: &mut Vec<Effect>) {
        if !self.editor_shown() {
            return;
        }
        if let Some(editing) = &mut self.editing {
            editing.keys = on;
        }
        if chat {
            self.talk_keys(true, effects);
        }
    }

    /// The editor exited. What it saved (`None`: it failed, or exited with an error) replaces the
    /// draft it was opened on, never sent; the same text, or none, leaves the draft as it was.
    /// A draft that is gone (its task dismissed): the text goes to the clipboard, and is said.
    pub(super) fn editor_done(&mut self, saved: Option<String>, more: bool, effects: &mut Vec<Effect>) {
        let Some(Editing { target, given, .. }) = self.editing.take() else { return };
        let now = self.now;
        let said = |model: &mut Model, text: &str| model.editor_note = Some((text.to_string(), now));
        let Some(saved) = saved else {
            return said(self, "The editor exited with an error: the message is as it was");
        };
        let text = saved_text(&saved);
        if text == given && !more {
            return;
        }
        let (field, limit) = match &target {
            Target::Compose if self.capture.is_none() => (self.compose.as_mut(), crate::tasks::MAX_TITLE),
            Target::Chat(subject) if self.keeps_draft(subject) => {
                (self.chat_mut(subject).map(|chat| &mut chat.draft), super::chat::MAX_DRAFT)
            }
            _ => (None, 0),
        };
        match field.map(|field| field.replace_all(text) && !more) {
            Some(true) => {}
            Some(false) => said(
                self,
                &format!("The editor's text was longer than a message: its first {limit} characters are kept"),
            ),
            None => {
                effects.push(Effect::Clip { text: text.to_string() });
                said(self, "The message's draft is gone: what your editor saved is on the clipboard");
            }
        }
    }

    /// An adopted session's conversation is its task's now: so is the editor's draft.
    pub(super) fn editor_moves(&mut self, from: &Subject, to: &Subject) {
        if let Some(editing) = self.editing.as_mut().filter(|editing| editing.target == Target::Chat(from.clone())) {
            editing.target = Target::Chat(to.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Target, USERS_ALONE, saved_text};
    use crate::model::tests::started;
    use crate::model::{Action, Effect, Model, Origin};
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

    fn events(m: &mut Model, subject: &Subject, events: Vec<Event>) -> Vec<Effect> {
        m.update(Action::AgentEvents { task: subject.clone(), events, at: 5 }.sent_by(Origin::System))
    }

    fn typed(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Replace { range: None, text: text.into() } });
    }

    fn draft(m: &Model, subject: &Subject) -> String {
        m.chat(subject).map(|chat| chat.draft.text().to_string()).unwrap_or_default()
    }

    fn done(saved: Option<&str>, more: bool) -> Action {
        Action::EditorDone { saved: saved.map(str::to_string), more }
    }

    fn finished(m: &mut Model, saved: Option<&str>, more: bool) -> Vec<Effect> {
        m.update(done(saved, more).sent_by(Origin::System))
    }

    #[test]
    fn one_trailing_newline_goes_and_inner_ones_stay() {
        assert_eq!(saved_text("a\nb\n"), "a\nb");
        assert_eq!(saved_text("a\r\n"), "a");
        assert_eq!(saved_text("a\n\n"), "a\n");
        assert_eq!(saved_text("a"), "a");
        assert_eq!(saved_text("a\r"), "a\r");
    }

    /// ⌃G hands the draft to the editor; what it saved comes back to the composer, which has the
    /// keyboard again, and nothing is sent. Only the user opens it; only the shell reports it.
    #[test]
    fn the_editor_takes_the_draft_and_gives_it_back_unsent() {
        let (mut m, one) = live();
        typed(&mut m, "fix it");
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&Action::EditorOpen, origin), Some(USERS_ALONE));
            assert_eq!(m.update(Action::EditorOpen.sent_by(origin)), []);
        }
        assert_eq!(m.update(Action::EditorOpen), [Effect::OpenEditor { text: "fix it".into() }]);
        assert!(m.edits(&one) && m.editor_shown() && m.editor_keys());
        // The composer takes no typing meanwhile, and ↵ sends nothing.
        assert!(!m.talk_typing() && m.field().is_none());
        typed(&mut m, "more");
        assert_eq!((m.update(Action::TalkSend), draft(&m, &one)), (vec![], "fix it".to_string()));
        // A client never says what the editor saved.
        assert_eq!(m.update(done(Some("theirs\n"), false).sent_by(Origin::Socket)), []);
        assert!(m.edits(&one));
        // Saved: the text, its last newline gone, lines kept; the composer has the keyboard, cursor at the end.
        assert_eq!(finished(&mut m, Some("fix it\nand test it\n"), false), []);
        assert_eq!(draft(&m, &one), "fix it\nand test it");
        assert!(m.editing.is_none() && m.talk_typing());
        assert_eq!(m.field().map(|(of, field)| (of, field.cursor())), Some(("talk", "fix it\nand test it".len())));
        // ⌘Z takes back the whole of it.
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Undo });
        assert_eq!(draft(&m, &one), "fix it");
    }

    #[test]
    fn an_editor_that_fails_or_saves_nothing_new_leaves_the_draft() {
        let (mut m, one) = live();
        typed(&mut m, "keep me");
        m.update(Action::EditorOpen);
        assert_eq!(finished(&mut m, None, false), []);
        assert_eq!((draft(&m, &one), m.editor_note().is_some()), ("keep me".into(), true));
        // Unchanged: as it was, and nothing to say.
        m.update(Action::EditorOpen);
        assert!(m.editor_note().is_none());
        assert_eq!(finished(&mut m, Some("keep me\n"), false), []);
        assert_eq!((draft(&m, &one), m.editor_note()), ("keep me".into(), None));
        // Control characters go as the composer's own paste takes them; a tab is four spaces.
        m.update(Action::EditorOpen);
        finished(&mut m, Some("a\tb\u{1b}[31mc\u{7}\n"), false);
        assert_eq!(draft(&m, &one), "a    b[31mc");
    }

    /// Longer than a message: the first part is kept, and the status bar says so.
    #[test]
    fn a_text_too_long_is_cut_and_said() {
        let (mut m, one) = live();
        m.update(Action::EditorOpen);
        let long = "y".repeat(crate::model::chat::MAX_DRAFT + 5);
        finished(&mut m, Some(&long), false);
        assert_eq!(draft(&m, &one).len(), crate::model::chat::MAX_DRAFT);
        assert!(m.editor_note().is_some_and(|note| note.contains("100000")), "{:?}", m.editor_note());
        // The shell read only the start of a larger file: said too.
        m.update(Action::EditorOpen);
        finished(&mut m, Some("short"), true);
        assert_eq!((draft(&m, &one).as_str(), m.editor_note().is_some()), ("short", true));
        // It goes after a while.
        m.update(Action::Tick { now: m.now + super::NOTE_MS }.sent_by(Origin::System));
        assert_eq!(m.editor_note(), None);
    }

    /// The text goes back to the draft it was opened on, wherever the user went; a dismissed
    /// task's goes to the clipboard, said.
    #[test]
    fn the_text_returns_to_its_own_conversation_or_the_clipboard() {
        let (mut m, one) = live();
        let assign = crate::tasks::Assign::default();
        m.update(Action::TaskAdd { title: "write docs".into(), priority: 3, state: State::Triaged, tab: None, assign });
        let two = Subject::Task(m.tasks.list[1].id);
        typed(&mut m, "one's");
        m.update(Action::EditorOpen);
        // Another conversation shown: not the editor; ⌃G there says where it is.
        m.update(Action::TalkPick { id: two.clone() });
        assert!(!m.editor_shown());
        assert!(matches!(&m.update(Action::EditorOpen)[..], [Effect::Notice { .. }]));
        finished(&mut m, Some("one's, edited\n"), false);
        assert_eq!((draft(&m, &one), draft(&m, &two)), ("one's, edited".into(), String::new()));
        // Back on it, and its task dismissed meanwhile: to the clipboard, and said.
        m.update(Action::TalkPick { id: one.clone() });
        m.update(Action::TalkKeys { on: true });
        m.update(Action::EditorOpen);
        assert!(m.edits(&one));
        m.update(Action::TaskDismiss { id: one.task().unwrap_or_default() });
        m.update(Action::QuitConfirm);
        assert!(m.tasks.live(one.task().unwrap_or_default()).is_none());
        let effects = finished(&mut m, Some("rescued"), false);
        assert_eq!(effects, [Effect::Clip { text: "rescued".into() }]);
        assert!(m.editor_note().is_some_and(|note| note.contains("clipboard")));
        // Its agent ended meanwhile: still its draft.
        let (mut m, one) = live();
        m.update(Action::EditorOpen);
        events(&mut m, &one, vec![Event::Exited { code: Some(0), signal: None, details: false }]);
        m.update(Action::AgentExited { task: one.clone() }.sent_by(Origin::System));
        finished(&mut m, Some("after the end"), false);
        assert_eq!(draft(&m, &one), "after the end");
    }

    /// A card that arrives while the editor runs shows above it; the editor keeps the keyboard,
    /// so no keystroke meant for it answers the card. A click on the card gives it the keys
    /// (the user's alone); ⌃G goes back to the editor.
    #[test]
    fn a_card_while_editing_is_answered_only_once_the_user_moves_to_it() {
        let (mut m, one) = live();
        typed(&mut m, "go");
        m.update(Action::TalkSend);
        m.update(Action::EditorOpen);
        let card = Event::Permission {
            ticket: 1,
            tool: "Bash".into(),
            label: "git".into(),
            detail: Text::new("command: git"),
            len: 12,
            cut: false,
            claim: Text::default(),
        };
        events(&mut m, &one, vec![Event::TurnStarted, card]);
        assert!(m.editor_shown() && m.editor_keys(), "the editor keeps the keyboard");
        // The shell gives the editor its keys: `chat::key` is never asked, so `a` answers nothing.
        assert!(m.chat(&one).is_some_and(|chat| chat.transcript.cards.len() == 1));
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.update(Action::EditorKeys { on: false }.sent_by(origin)), []);
            assert!(m.editor_keys(), "{origin:?} moved the keyboard");
        }
        m.update(Action::EditorKeys { on: false });
        assert!(!m.editor_keys() && m.editor_shown(), "the card has the keys; the editor stays on show");
        assert_eq!(
            crate::model::chat::key("a", crate::keys::Mods::default(), Some(1)),
            crate::model::chat::Key::Act(Action::TalkAnswer { allow: true, ticket: 1 })
        );
        let effects = m.update(Action::TalkAnswer { allow: false, ticket: 1 });
        assert!(matches!(&effects[..], [Effect::AgentAnswer { allow: false, .. }]), "{effects:?}");
        // ⌃G: back to the editor, no second one.
        assert_eq!(m.update(Action::EditorOpen), []);
        assert!(m.editor_keys());
        assert_eq!(m.editing.as_ref().map(|e| e.target.clone()), Some(Target::Chat(one)));
    }

    /// The new-task box: its line in the editor, back as one line (breaks become spaces, as its paste does).
    #[test]
    fn the_new_task_box_edits_its_line_in_the_editor() {
        let mut m = Model::default();
        m.update(Action::OpenCompose { text: "fix zoom".into() });
        assert_eq!(m.update(Action::EditorOpen), [Effect::OpenEditor { text: "fix zoom".into() }]);
        assert!(m.editor_shown() && m.editor_keys() && m.field().is_none() && m.dropdown().is_none());
        // Nobody types in the box meanwhile.
        typed(&mut m, "x");
        assert_eq!(m.compose_line().map(String::as_str), Some("fix zoom"));
        finished(&mut m, Some("fix zoom\nin the canvas\n"), false);
        assert_eq!(m.compose_line().map(String::as_str), Some("fix zoom in the canvas"));
        assert_eq!(m.field().map(|(of, _)| of), Some("compose"));
        // Closed meanwhile: the clipboard.
        m.update(Action::EditorOpen);
        m.update(Action::CloseCompose);
        assert_eq!(finished(&mut m, Some("kept"), false), [Effect::Clip { text: "kept".into() }]);
    }

    /// A card waits: ⌃G opens nothing (the composer is not on show).
    #[test]
    fn no_editor_over_a_waiting_card() {
        let (mut m, one) = live();
        let card = Event::Permission {
            ticket: 3,
            tool: "Bash".into(),
            label: "ls".into(),
            detail: Text::new("command: ls"),
            len: 11,
            cut: false,
            claim: Text::default(),
        };
        events(&mut m, &one, vec![Event::TurnStarted, card]);
        assert_eq!(m.update(Action::EditorOpen), []);
        assert!(m.editing.is_none());
    }
}
