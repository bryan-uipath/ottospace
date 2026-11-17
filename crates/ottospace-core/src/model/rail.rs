//! The drawer's right rail (`docs/ai/spaces.md`, "The drawer's rail"): a narrow column at the
//! drawer's right edge, one item for each thing the drawer can show here. The running
//! transcript first while it runs, then the selected space's tasks, the most urgent first. Shown
//! only with two items or more. The selected item is what the drawer shows; only the user
//! changes it.

use serde::{Deserialize, Serialize};

use super::{Effect, Model};
use crate::tasks::{State, TaskId};

/// Why a client's choice on the rail is refused.
pub const RAIL_USERS: &str = "only the user switches what the drawer shows";

/// What an item of the rail stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RailItem {
    /// The transcript on (running or paused): one that ended is not on the rail.
    Transcript,
    Task {
        id: TaskId,
    },
}

impl Model {
    /// The rail's items, in order: the transcript while one is on, then the selected space's
    /// tasks by urgency (needs you, running, later, on hold; done ones left out but the one in
    /// front). Fewer than two: no rail.
    pub fn rail(&self) -> Vec<RailItem> {
        let of = self.space();
        let front = self.front(of);
        let done = |id: &TaskId| self.tasks.get(*id).is_some_and(|task| task.state == State::Done);
        let mut tasks: Vec<TaskId> =
            self.members(of).into_iter().filter(|id| Some(*id) == front || !done(id)).collect();
        tasks.sort_by_key(|id| self.urgency(*id));
        let transcript = self.listen.on.as_ref().map(|_| RailItem::Transcript);
        let items: Vec<RailItem> =
            transcript.into_iter().chain(tasks.into_iter().map(|id| RailItem::Task { id })).collect();
        if items.len() > 1 { items } else { Vec::new() }
    }

    /// The item the drawer shows, while there is a rail: the transcript while it is shown, else
    /// the task in front.
    pub fn rail_selected(&self) -> Option<RailItem> {
        let rail = self.rail();
        let shown = match self.listen.shown {
            true => Some(RailItem::Transcript),
            false => self.front(self.space()).map(|id| RailItem::Task { id }),
        };
        shown.filter(|item| rail.contains(item))
    }

    /// Whether the rail is drawn: it has items, and the drawer shows the transcript or a
    /// conversation, not the new-task box, a capture's draft or the inspector in its place.
    pub fn rail_drawn(&self) -> bool {
        let talk = self.talk.as_ref().is_some_and(|talk| talk.open);
        let boxed = self.compose.is_some() || self.welcome();
        let elsewhere = self.inspector.is_some() || self.capture.is_some();
        !elsewhere && !self.rail().is_empty() && (self.listen.shown || (talk && !boxed))
    }

    /// The user's choice of an item: the drawer shows it, open. The transcript is shown here,
    /// in this space's drawer (its source pane's tab is not brought up); a task is put in front.
    /// An item not on the rail: nothing.
    pub(super) fn rail_select(&mut self, item: RailItem, effects: &mut Vec<Effect>) {
        if !self.rail().contains(&item) {
            return;
        }
        match item {
            RailItem::Transcript => {
                // The conversation gives the keyboard back, or the transcript would be taken from under it.
                self.talk_keys(false, effects);
                self.listen.shown = true;
            }
            RailItem::Task { id } => {
                self.listen.shown = false;
                self.set_front(id);
                if let Some(talk) = &mut self.talk {
                    talk.open = true;
                }
            }
        }
    }

    /// The next (positive) or previous item of the rail, from the one selected. It wraps.
    pub(super) fn rail_cycle(&mut self, delta: i32, effects: &mut Vec<Effect>) {
        let rail = self.rail();
        let Some(at) = self.rail_selected().and_then(|item| rail.iter().position(|it| *it == item)) else { return };
        let to = (at as i64 + i64::from(delta)).rem_euclid(rail.len() as i64) as usize;
        self.rail_select(rail[to], effects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;
    use crate::listen::Source;
    use crate::model::{Action, Kind, Origin};
    use crate::tasks::State;

    /// A workspace of three tasks, the last in front, and a terminal pane (1) the fixture may hear.
    fn three() -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        for title in ["one", "two", "three"] {
            m.update(Action::OpenCompose { text: title.into() });
            m.update(Action::ComposeSubmit { open: true });
        }
        m
    }

    fn task(id: TaskId) -> RailItem {
        RailItem::Task { id }
    }

    fn set(m: &mut Model, id: TaskId, state: State) {
        m.update(Action::TaskSetState { id, state }.sent_by(Origin::System));
    }

    /// The user starts one, the keys out of the conversation (in it, the transcript would not show).
    fn listen(m: &mut Model) {
        m.update(Action::TalkKeys { on: false });
        let start = Action::ListenStart { source: Source::Pane(1), mic: false, id: "t".into(), at: 1_000 };
        m.update(start);
        m.update(Action::ListenBegan { id: "t".into(), at: 1_100 }.sent_by(Origin::System));
    }

    #[test]
    fn the_rail_is_the_transcript_then_the_tasks_by_urgency_and_none_with_one_item() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        m.update(Action::OpenCompose { text: "one".into() });
        m.update(Action::ComposeSubmit { open: true });
        // One task: nothing to switch.
        assert_eq!(m.rail(), []);
        // A transcript joins it: two items, the transcript first and shown.
        listen(&mut m);
        assert_eq!((m.rail(), m.rail_selected()), (vec![RailItem::Transcript, task(1)], Some(RailItem::Transcript)));
        // It stops: it leaves the rail, and with one item the rail goes.
        m.update(Action::ListenStop { at: 2_000 });
        assert_eq!((m.rail(), m.rail_selected()), (vec![], None));
        // Tasks by urgency: needs you, running, then the rest; done ones go, but the one in front.
        let mut m = three();
        set(&mut m, 1, State::Captured);
        set(&mut m, 2, State::NeedsYou);
        set(&mut m, 3, State::Running);
        assert_eq!(m.rail(), [task(2), task(3), task(1)]);
        set(&mut m, 2, State::Done);
        assert_eq!(m.rail(), [task(3), task(1)]);
    }

    #[test]
    fn a_click_or_the_keys_select_an_item_and_only_the_users() {
        let mut m = three();
        listen(&mut m);
        assert_eq!(m.rail_selected(), Some(RailItem::Transcript));
        // A task: in front, the drawer on its conversation, the transcript put away.
        m.update(Action::RailSelect { item: task(1) });
        assert_eq!((m.rail_selected(), m.listen.shown), (Some(task(1)), false));
        assert!(m.talk.as_ref().is_some_and(|talk| talk.open && talk.task.task() == Some(1)));
        // The transcript, from any space's drawer: its pane's tab is not brought up.
        let tab = m.active;
        m.update(Action::RailSelect { item: RailItem::Transcript });
        assert_eq!((m.rail_selected(), m.active), (Some(RailItem::Transcript), tab));
        // ⌘⌥] / ⌘⌥[ step through it in order, and wrap.
        let order = m.rail();
        m.update(Action::TalkCycle { delta: 1 });
        assert_eq!(m.rail_selected(), Some(order[1]));
        m.update(Action::TalkCycle { delta: -2 });
        assert_eq!(m.rail_selected(), order.last().copied());
        // A client's choice, or step, is refused.
        for action in
            [Action::RailSelect { item: task(2) }, Action::TalkCycle { delta: 1 }, Action::ListenShow { on: false }]
        {
            for origin in [Origin::Socket, Origin::System] {
                assert!(m.refusal(&action, origin).is_some(), "{action:?} {origin:?}");
                let before = m.rail_selected();
                m.update(action.clone().sent_by(origin));
                assert_eq!(m.rail_selected(), before, "{action:?} {origin:?}");
            }
        }
        // Nor does a client's ⌘⇧L or its keys into the conversation put the transcript away, the
        // conversation open behind it or closed (review: closed, the toggle gave it the keys).
        m.update(Action::RailSelect { item: RailItem::Transcript });
        for closed in [false, true] {
            if let Some(talk) = &mut m.talk {
                talk.open = !closed;
            }
            for theirs in [Action::TalkToggle, Action::TalkKeys { on: true }] {
                m.update(theirs.clone().sent_by(Origin::Socket));
                assert_eq!(m.rail_selected(), Some(RailItem::Transcript), "{theirs:?} closed: {closed}");
            }
        }
        // An item not on the rail: nothing.
        m.update(Action::RailSelect { item: task(9) });
        assert_eq!(m.rail_selected(), Some(RailItem::Transcript));
    }

    /// List mode reaches it: → past the panes (the transcript takes no keys), ↓ ↑, ↵ shows, ← back.
    #[test]
    fn the_keyboard_reaches_the_rail_right_of_the_drawer() {
        use crate::layout::Dir;
        use crate::model::Mode;
        use crate::model::zones::Target;
        let mut m = three();
        listen(&mut m);
        m.update(Action::Focus { pane: 1 });
        m.update(Action::FocusDir { dir: Dir::Right });
        assert_eq!((m.focus_target(), m.mode()), (Target::DrawerRail(RailItem::Transcript), Mode::List));
        let order = m.rail();
        m.update(Action::ListMove { delta: 1 });
        assert_eq!((m.focus_target(), m.rail_selected()), (Target::DrawerRail(order[1]), Some(RailItem::Transcript)));
        m.update(Action::ListChoose { axis: Axis::Right });
        assert_eq!((m.focus_target(), m.rail_selected()), (Target::DrawerRail(order[1]), Some(order[1])));
        // A client's key on it does nothing.
        assert!(m.refusal(&Action::ListMove { delta: 1 }, Origin::Socket).is_some());
        m.update(Action::FocusDir { dir: Dir::Left });
        assert_eq!(m.focus_target(), Target::Pane(1));
        // Review: → from the inspector put the keys on a rail it hides. Not drawn, not reached.
        m.update(Action::InspectorOpen { pane: 1, app: "Notes".into() });
        assert!(!m.rail_drawn());
        m.update(Action::FocusDir { dir: Dir::Right });
        m.update(Action::FocusDir { dir: Dir::Right });
        assert!(!matches!(m.focus_target(), Target::DrawerRail(_)), "{:?}", m.focus_target());
        m.update(Action::InspectorClose);
        m.update(Action::Focus { pane: 1 });
        // Its item gone (the transcript stopped, the rail with it): the keys leave it.
        m.update(Action::RailSelect { item: RailItem::Transcript });
        m.update(Action::FocusDir { dir: Dir::Right });
        m.update(Action::ListenStop { at: 5_000 });
        assert_ne!(m.focus_target(), Target::DrawerRail(RailItem::Transcript));
    }
}
