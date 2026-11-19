//! How a conversation's task ends (the canvas "Later vs Done"): ✓ Done, the task is finished;
//! Later, it is put away on hold, to be resumed. While a turn runs the header has no Done: the
//! composer's Stop stops that turn (the owner: one stop button). Both Done and Later stop its
//! agent and take it off the drawer's rail.
//! Only the user's own click or key does any of it. Design: `docs/ai/done.md`.

use serde::Serialize;

use super::{Action, Confirm, Effect, Model};
use crate::notice::{self, Notice};
use crate::talk::Subject;
use crate::tasks::{State, TaskId};
use crate::transcript::{Entry, View};

/// Why a client's Done, Later, Undo or answer to "Stop it and put it away?" is refused.
pub const USERS_ALONE: &str = "only the user marks a task done or puts it away";
/// Why Done does nothing while a turn runs: the composer's Stop is for that.
pub const TURN_RUNS: &str = "its turn runs: Stop it first";
/// How long "Done · <task>" offers its Undo, in milliseconds: as every other Undo.
pub const DONE_MS: u64 = crate::tasks::NOTICE_MS;

/// The tasks the user put away and the Done that may be taken back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Finishing {
    /// Done or Later by the user: off the rail, never in front, until they open it again.
    pub(crate) away: Vec<TaskId>,
    pub offer: Option<DoneOffer>,
}

/// "Done · <task>" on the notice bar, with Undo, until `until` (ms since 1970).
#[derive(Clone, Debug, PartialEq)]
pub struct DoneOffer {
    pub task: TaskId,
    pub title: String,
    /// Its state before: what Undo gives back.
    pub(crate) before: State,
    pub until: u64,
}

/// What a task's conversation header offers at its right end, given its agent (`Model::finish`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Buttons {
    /// Later: always, but while its agent stops.
    pub later: bool,
    /// ✓ Done: once its agent idles. Mid-turn the composer's Stop is the one stop.
    pub done: bool,
}

impl Model {
    /// True if the user put the task away (Done or Later) and has not opened it since.
    pub fn put_away(&self, id: TaskId) -> bool {
        self.finish.away.contains(&id)
    }

    /// A native turn runs.
    fn native_turn(&self, subject: &Subject) -> bool {
        self.chat(subject).is_some_and(|chat| chat.transcript.live && chat.transcript.turn.is_some())
    }

    /// True if its agent is at work: a native turn runs, or in a terminal its task is Running.
    pub(super) fn works(&self, subject: &Subject) -> bool {
        self.native_turn(subject) || (self.talk_view(subject) != View::Native && self.talk_working(subject))
    }

    /// The header's Later and Done, for a live task that is not done. No Done while its agent
    /// works (a turn runs, or in a terminal it runs) or stops.
    pub fn finish(&self, subject: &Subject) -> Option<Buttons> {
        let task = self.tasks.live(subject.task()?)?;
        if task.state == State::Done {
            return None;
        }
        let ending = self.is_ending(subject);
        Some(Buttons { later: !ending, done: !ending && !self.works(subject) })
    }

    /// "Got what you needed? ✓ Done · Later" under the last reply: the shown conversation's agent
    /// answered and idles, nothing waits on the user, and they have not begun to type.
    pub fn done_offered(&self, subject: &Subject) -> bool {
        let Some(chat) = self.chat(subject).filter(|_| self.finish(subject).is_some_and(|b| b.done)) else {
            return false;
        };
        let t = &chat.transcript;
        let quiet = t.live && t.turn.is_none() && t.cards.is_empty() && chat.draft.text().is_empty();
        let said = t.items().rev().find(|item| matches!(item.entry, Entry::User { .. } | Entry::Assistant { .. }));
        quiet && chat.queued.is_none() && said.is_some_and(|item| matches!(item.entry, Entry::Assistant { .. }))
    }

    /// ⌘W: Later on the drawer's task while the keyboard is in the drawer; else the pane closes.
    pub fn close_key(&self) -> Action {
        let talk = self.talk.as_ref().filter(|talk| talk.open && talk.keys);
        match talk.and_then(|talk| talk.task.task()).filter(|id| self.finish(&Subject::Task(*id)).is_some()) {
            Some(id) => Action::TaskLater { id },
            None => Action::Close { pane: None },
        }
    }

    /// ⌘↵ in the conversation: its Done, while Done is offered.
    pub fn done_key(&self) -> Option<Action> {
        let talk = self.talk.as_ref().filter(|talk| talk.open && talk.keys)?;
        let id = talk.task.task()?;
        self.finish(&talk.task)?.done.then_some(Action::TaskDone { id })
    }

    /// "Done · <task>" while its Undo is offered.
    pub fn done_notice(&self) -> Option<Notice> {
        let offer = self.finish.offer.as_ref()?;
        Some(Notice { kind: notice::Kind::Undo, text: format!("Done · {}", offer.title) })
    }

    pub(super) fn finish_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::TaskDone { id } => {
                let subject = Subject::Task(id);
                let Some(task) = self.tasks.live(id).filter(|t| t.state != State::Done) else { return };
                if self.works(&subject) {
                    return effects.push(Effect::Notice { text: TURN_RUNS.into() });
                }
                let (before, title) = (task.state, task.title.clone());
                // The seal plays where it was pressed: the drawer on this task.
                let shown = self.talk.as_ref().is_some_and(|talk| talk.open && talk.task == subject);
                self.put_task_away(id, State::Done, true, effects);
                self.finish.offer =
                    Some(DoneOffer { task: id, title, before, until: self.now.saturating_add(DONE_MS) });
                if shown {
                    effects.push(Effect::Seal { task: id });
                }
            }
            Action::TaskLater { id } => {
                let Some(title) = self.tasks.live(id).filter(|t| t.state != State::Done).map(|t| t.title.clone())
                else {
                    return;
                };
                if self.works(&Subject::Task(id)) {
                    // The question is ottospace's own UI: a live window would cover it.
                    self.leave(effects);
                    self.confirm = Some(Confirm { later: Some((id, title)), ..Confirm::default() });
                    return;
                }
                self.put_task_away(id, State::Parked, false, effects);
            }
            Action::DoneUndo => {
                let Some(offer) = self.finish.offer.take() else { return };
                if self.tasks.live(offer.task).is_none_or(|t| t.state != State::Done) {
                    return;
                }
                self.tasks.set_state(offer.task, offer.before);
                self.finish.away.retain(|id| *id != offer.task);
                self.set_front(offer.task);
                // Back where it was: the drawer on it, if its workspace is the one on show.
                if self.task_space(offer.task) == *self.space() {
                    if let Some(talk) = &mut self.talk {
                        talk.open = true;
                    }
                    self.drawer_open = true;
                }
            }
            _ => {}
        }
    }

    /// Yes to "Stop it and put it away?": as Later on an idle one.
    pub(super) fn later_confirmed(&mut self, id: TaskId, effects: &mut Vec<Effect>) {
        if self.tasks.live(id).is_some_and(|t| t.state != State::Done) {
            self.put_task_away(id, State::Parked, false, effects);
        }
    }

    /// The user's ↵ or click on a task put away, or done: it is a task like any other again.
    /// True if it was done: its session is gone on with.
    pub(super) fn revive(&mut self, id: TaskId) -> bool {
        let away = self.put_away(id);
        self.finish.away.retain(|task| *task != id);
        let Some(state) = self.tasks.live(id).map(|t| t.state) else { return false };
        let done = state == State::Done;
        if done || (away && state == State::Parked) {
            self.tasks.set_state(id, State::NeedsYou);
        }
        if self.finish.offer.as_ref().is_some_and(|offer| offer.task == id) {
            self.finish.offer = None;
        }
        done
    }

    /// Its agent stops (as Stop stops it), it is `state`, off the rail, and the next task of its
    /// workspace is in front. The drawer folds into the rail on `fold`, or when none is left.
    fn put_task_away(&mut self, id: TaskId, state: State, fold: bool, effects: &mut Vec<Effect>) {
        let subject = Subject::Task(id);
        let of = self.task_space(id);
        let was_front = self.front(&of) == Some(id);
        self.act(Action::TaskStop { id: subject.clone() }, effects);
        self.tasks.set_state(id, state);
        if !self.finish.away.contains(&id) {
            self.finish.away.push(id);
        }
        // Another task's rail ×: the one in front stays.
        let next = self.members(&of).into_iter().filter(|t| !self.put_away(*t)).min_by_key(|t| self.urgency(*t));
        if let Some(next) = next.filter(|_| was_front) {
            self.set_front(next);
        }
        let shown = self.talk.as_ref().is_some_and(|talk| talk.task == subject);
        if shown && (fold || next.is_none()) {
            self.talk_keys(false, effects);
            if let Some(talk) = &mut self.talk {
                talk.open = false;
            }
            self.drawer_open = false;
        }
    }

    /// After every step: a task put away is one still there, in the state it was put away in
    /// or whatever its runner said since. The offer ends with its task.
    pub(super) fn settle_finish(&mut self) {
        // One whose state anyone changed since (a runner's report, a client's) is news: back on the rail.
        let tasks = &self.tasks;
        self.finish.away.retain(|id| tasks.live(*id).is_some_and(|t| matches!(t.state, State::Done | State::Parked)));
        let gone = self
            .finish
            .offer
            .as_ref()
            .is_some_and(|offer| tasks.live(offer.task).is_none_or(|t| t.state != State::Done));
        if gone {
            self.finish.offer = None;
        }
    }

    /// × on the bar over "Done · <task>": the offer ends.
    pub(super) fn done_dismiss(&mut self, text: &str) {
        if self.done_notice().is_some_and(|said| said.text == text) {
            self.finish.offer = None;
        }
    }

    pub(super) fn finish_tick(&mut self, now: u64) {
        self.finish.offer = self.finish.offer.take().filter(|offer| offer.until > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Origin;
    use crate::model::chat::Primary;
    use crate::model::rail::RailItem;
    use crate::model::tests::started;
    use crate::transcript::{Event, Outcome, Text};

    /// One task whose native agent answered and idles, its conversation shown with the keyboard.
    fn idle() -> Model {
        let mut m = Model::default();
        let assign = crate::tasks::Assign::default();
        m.update(Action::TaskAdd { title: "fix login".into(), priority: 3, state: State::Triaged, tab: None, assign });
        started(&mut m, Action::TaskTalk { id: 1 });
        let session = Some("8f14e45f-ceea-467f-a8d3-0242ac130003".into());
        m.update(Action::AgentStarted { task: Subject::Task(1), session, at: 1 }.sent_by(Origin::System));
        replied(&mut m);
        m
    }

    fn events(m: &mut Model, events: Vec<Event>) -> Vec<Effect> {
        m.update(Action::AgentEvents { task: Subject::Task(1), events, at: 5 }.sent_by(Origin::System))
    }

    fn replied(m: &mut Model) {
        let text = Event::Text { text: Text::new("Fixed: the redirect keeps the cookie.") };
        events(m, vec![Event::TurnStarted, text, Event::TurnFinished { outcome: Outcome::Completed }]);
    }

    fn state(m: &Model, id: TaskId) -> Option<State> {
        m.tasks.get(id).map(|t| t.state)
    }

    const ONE: Subject = Subject::Task(1);

    #[test]
    fn done_on_an_idle_task_stops_its_agent_seals_it_and_undo_brings_it_back() {
        let mut m = idle();
        assert_eq!(m.finish(&ONE), Some(Buttons { later: true, done: true }));
        assert!(m.done_offered(&ONE), "the quiet offer under the reply");
        assert_eq!((m.rail(), m.done_key()), (vec![RailItem::Task { id: 1 }], Some(Action::TaskDone { id: 1 })));
        let effects = m.update(Action::TaskDone { id: 1 });
        assert_eq!(effects, [Effect::StopAgent { task: ONE }, Effect::Seal { task: 1 }]);
        assert_eq!((state(&m, 1), m.put_away(1), m.rail()), (Some(State::Done), true, vec![]));
        // The drawer folded into the rail; the workspace stays on the row, with no task in front.
        assert!(m.drawer_collapsed());
        assert_eq!((m.front(m.space()), m.space().clone()), (None, crate::model::Owner::Task { id: 1 }));
        assert_eq!(m.done_notice().map(|n| (n.kind, n.text)), Some((notice::Kind::Undo, "Done · fix login".into())));
        // Its stopping agent's last word moves it nowhere.
        events(&mut m, vec![Event::Exited { code: Some(0), signal: None, details: false }]);
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        assert_eq!(state(&m, 1), Some(State::Done));
        // Undo: as it was, in front, the drawer on it; its agent is not started again.
        assert_eq!(m.update(Action::DoneUndo), []);
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail()),
            (Some(State::NeedsYou), false, vec![RailItem::Task { id: 1 }])
        );
        assert_eq!((m.drawer_collapsed(), m.done_notice()), (false, None));
        assert_eq!(m.composer_button(), Some(Primary::Resume));
    }

    #[test]
    fn the_offer_to_take_done_back_lasts_six_seconds_or_until_dismissed() {
        let mut m = idle();
        m.update(Action::Tick { now: 10_000 }.sent_by(Origin::System));
        m.update(Action::TaskDone { id: 1 });
        assert_eq!(m.next_due(), Some(10_000 + DONE_MS));
        m.update(Action::Tick { now: 10_000 + DONE_MS - 1 }.sent_by(Origin::System));
        assert!(m.done_notice().is_some());
        m.update(Action::Tick { now: 10_000 + DONE_MS }.sent_by(Origin::System));
        assert_eq!(m.done_notice(), None);
        m.update(Action::DoneUndo);
        assert_eq!(state(&m, 1), Some(State::Done), "its time is up");
        // A second one, dismissed by its ×.
        let mut m = idle();
        m.update(Action::TaskDone { id: 1 });
        let text = m.done_notice().map(|n| n.text).unwrap_or_default();
        m.update(Action::NoticeDismiss { notice: notice::Kind::Undo, text });
        assert_eq!(m.done_notice(), None);
    }

    #[test]
    fn while_a_turn_runs_the_header_has_no_done_and_the_composers_stop_stops_only_the_turn() {
        let mut m = idle();
        events(&mut m, vec![Event::TurnStarted]);
        assert_eq!(m.finish(&ONE), Some(Buttons { later: true, done: false }));
        assert_eq!(m.composer_button(), Some(Primary::Stop), "the one stop: the composer's");
        assert!(!m.done_offered(&ONE));
        assert_eq!(m.done_key(), None, "⌘↵ marks nothing done mid-turn");
        // Done sent anyway: it says why, and changes nothing.
        assert_eq!(m.update(Action::TaskDone { id: 1 }), [Effect::Notice { text: TURN_RUNS.into() }]);
        assert_eq!(state(&m, 1), Some(State::Running));
        assert_eq!(m.update(Action::TalkInterrupt), [Effect::AgentInterrupt { task: ONE }]);
        events(&mut m, vec![Event::TurnFinished { outcome: Outcome::Interrupted }]);
        assert_eq!((m.agent_runs(&ONE), m.put_away(1)), (true, false), "the conversation goes on");
        assert_eq!(m.finish(&ONE).map(|b| b.done), Some(true), "the turn over: ✓ Done again");
    }

    #[test]
    fn later_on_an_idle_task_puts_it_on_hold_and_its_row_brings_it_back() {
        let mut m = idle();
        assert_eq!(m.close_key(), Action::TaskLater { id: 1 }, "⌘W in the drawer");
        assert_eq!(m.update(Action::TaskLater { id: 1 }), [Effect::StopAgent { task: ONE }]);
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail(), m.confirm.is_none()),
            (Some(State::Parked), true, vec![], true)
        );
        assert_eq!(m.done_notice(), None, "Later offers no Undo: Tasks has it");
        assert_eq!(m.close_key(), Action::Close { pane: None }, "the drawer folded: ⌘W closes the pane");
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        // Its row: a task again, in front, its conversation open; nothing starts by the click.
        let effects = m.update(Action::TaskTalk { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::StartAgent { .. })), "{effects:?}");
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail()),
            (Some(State::NeedsYou), false, vec![RailItem::Task { id: 1 }])
        );
    }

    #[test]
    fn later_asks_first_while_a_turn_runs() {
        let mut m = idle();
        events(&mut m, vec![Event::TurnStarted]);
        assert_eq!(m.update(Action::TaskLater { id: 1 }), []);
        let asked = m.confirm.as_ref().map(|c| (c.later.clone(), c.title()));
        assert_eq!(asked, Some((Some((1, "fix login".into())), "Stop \"fix login\" and put it away?".into())));
        // Only the user answers; Keep running leaves it as it was.
        assert_eq!(m.refusal(&Action::QuitConfirm, Origin::Socket), Some(USERS_ALONE));
        m.update(Action::QuitCancel);
        assert_eq!((m.confirm.is_none(), state(&m, 1), m.put_away(1)), (true, Some(State::Running), false));
        m.update(Action::TaskLater { id: 1 });
        assert_eq!(m.update(Action::QuitConfirm), [Effect::StopAgent { task: ONE }]);
        assert_eq!((state(&m, 1), m.put_away(1)), (Some(State::Parked), true));
        // The turn ending by itself while it asks: the question goes, nothing is put away.
        let mut m = idle();
        events(&mut m, vec![Event::TurnStarted]);
        m.update(Action::TaskLater { id: 1 });
        events(&mut m, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!((m.confirm.is_none(), m.put_away(1)), (true, false));
    }

    #[test]
    fn only_the_user_marks_done_puts_away_or_takes_back() {
        let m = idle();
        for origin in [Origin::Socket, Origin::System] {
            for action in
                [Action::TaskDone { id: 1 }, Action::TaskLater { id: 1 }, Action::DoneUndo, Action::TalkInterrupt]
            {
                assert!(m.refusal(&action, origin).is_some(), "{action:?} from {origin:?}");
            }
            assert_eq!(m.refusal(&Action::TaskDone { id: 1 }, origin), Some(USERS_ALONE));
        }
        assert_eq!(m.refusal(&Action::TaskDone { id: 1 }, Origin::User), None);
    }

    #[test]
    fn the_offer_line_goes_once_the_user_types_and_comes_back_after_the_next_reply() {
        let mut m = idle();
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Replace { range: None, text: "and".into() } });
        assert!(!m.done_offered(&ONE));
        m.update(Action::TalkSend);
        assert!(!m.done_offered(&ONE), "the agent at work");
        replied(&mut m);
        assert!(m.done_offered(&ONE));
    }

    /// Three tasks in one workspace: Later on the one in front shows the next, the drawer
    /// open; Done folds it into the rail, the next in front.
    #[test]
    fn in_a_workspace_of_several_the_next_task_comes_in_front() {
        let mut m = Model::default();
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        for title in ["one", "two", "three"] {
            m.update(Action::OpenCompose { text: title.into() });
            m.update(Action::ComposeSubmit { open: true });
        }
        let front = |m: &Model| m.front(m.space());
        assert_eq!(front(&m), Some(3));
        m.update(Action::TaskLater { id: 3 });
        assert_eq!((front(&m), m.drawer_collapsed(), m.put_away(3)), (Some(1), false, true));
        m.update(Action::TaskDone { id: 1 });
        assert_eq!((front(&m), m.drawer_collapsed()), (Some(2), true));
        let ids: Vec<RailItem> = m.rail();
        assert_eq!(ids, [RailItem::Task { id: 2 }]);
    }

    /// ↵ on a task's row in list mode brings one put away back, as its click does.
    #[test]
    fn the_keyboard_on_its_row_brings_a_task_put_away_back() {
        let mut m = idle();
        m.update(Action::TaskLater { id: 1 });
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        m.update(Action::ToggleSection { section: crate::model::Section::Tasks });
        m.update(Action::FocusList);
        let at = m.list_rows().iter().position(|row| *row == crate::model::ListRow::Task(1));
        m.list = at;
        m.update(Action::ListChoose { axis: crate::Axis::Right });
        assert_eq!((state(&m, 1), m.put_away(1), m.front(m.space())), (Some(State::NeedsYou), false, Some(1)));
        assert!(m.talk.as_ref().is_some_and(|talk| talk.open && talk.task == ONE));
    }

    /// The × on a task's rail item that is not in front: the one in front stays.
    #[test]
    fn later_on_another_tasks_rail_item_leaves_the_one_in_front() {
        let mut m = Model::default();
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        for title in ["one", "two", "three"] {
            m.update(Action::OpenCompose { text: title.into() });
            m.update(Action::ComposeSubmit { open: true });
        }
        m.update(Action::TaskSetState { id: 1, state: State::NeedsYou }.sent_by(Origin::System));
        assert_eq!(m.front(m.space()), Some(3));
        m.update(Action::TaskLater { id: 2 });
        assert_eq!((m.front(m.space()), m.put_away(2), m.drawer_collapsed()), (Some(3), true, false));
    }

    /// A state anyone reports since is news: the task is back on the rail.
    #[test]
    fn a_task_put_away_whose_state_is_reported_comes_back() {
        let mut m = idle();
        m.update(Action::TaskLater { id: 1 });
        m.update(Action::TaskSetState { id: 1, state: State::NeedsYou }.sent_by(Origin::System));
        assert_eq!((m.put_away(1), m.rail()), (false, vec![RailItem::Task { id: 1 }]));
    }

    /// Later's question is no quit: only ↵ stops the agent; Q and ⌘Q keep it running.
    #[test]
    fn only_return_answers_yes_to_later() {
        use crate::keys::Mods;
        let cmd = Mods { cmd: true, ..Mods::default() };
        assert_eq!(Confirm::answer_later("enter", Mods::default(), Origin::User), Some(Action::QuitConfirm));
        for (key, mods) in [("q", Mods::default()), ("q", cmd), ("escape", Mods::default())] {
            assert_eq!(Confirm::answer_later(key, mods, Origin::User), Some(Action::QuitCancel), "{key}");
        }
        assert_eq!(Confirm::answer_later("q", Mods::default(), Origin::Socket), None);
    }
}
