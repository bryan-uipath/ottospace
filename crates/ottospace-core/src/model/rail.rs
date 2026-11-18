//! The drawer's right rail (`docs/ai/spaces.md`, "The drawer's rail"): a narrow column at the
//! drawer's right edge, one item for each thing the drawer can show here, in two sections. On
//! top this workspace's tasks, the most urgent first; below a divider what is the same in every
//! workspace: the transcript, idle or recording. Always shown: with the drawer open, or the
//! drawer collapsed into it. The selected item is what the drawer shows; only the user changes it.

use serde::{Deserialize, Serialize};

use super::{Effect, Model};
use crate::tasks::{State, TaskId};

/// Why a client's choice on the rail is refused.
pub const RAIL_USERS: &str = "only the user switches what the drawer shows";
/// Why a client's ⌘⇧R, or its click on the drawer's →, is refused.
pub const DRAWER_USERS: &str = "only the user collapses or opens the drawer";

/// What the user's collapse folded away, for ⌘⇧R to bring back (`Model::folded`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folded {
    /// The transcript was shown.
    Transcript,
    /// The new-task box of a fresh start (`Model::welcome`).
    Welcome,
    /// A conversation, a brief, the inspector or the box.
    Rest,
}

/// What an item of the rail stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RailItem {
    /// The transcript: idle, or one on (running or paused).
    Transcript,
    Task {
        id: TaskId,
    },
}

/// The rail's two parts: this workspace's, then (below the divider) what every workspace shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RailSection {
    Workspace,
    Everywhere,
}

impl RailItem {
    pub fn section(self) -> RailSection {
        match self {
            RailItem::Task { .. } => RailSection::Workspace,
            RailItem::Transcript => RailSection::Everywhere,
        }
    }
}

impl Model {
    /// The rail's items, top to bottom: the selected space's tasks by urgency (needs you,
    /// running, later, on hold; done ones left out but the one in front), then the transcript
    /// while it can be had (`listen_item`).
    pub fn rail(&self) -> Vec<RailItem> {
        let of = self.space();
        let front = self.front(of);
        let done = |id: &TaskId| self.tasks.get(*id).is_some_and(|task| task.state == State::Done);
        let tasks = self.space_tasks(of).into_iter().filter(|id| Some(*id) == front || !done(id));
        let transcript = self.listen_item().then_some(RailItem::Transcript);
        tasks.map(|id| RailItem::Task { id }).chain(transcript).collect()
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

    /// The drawer is collapsed into its rail: nothing is in its place, neither the transcript, a
    /// conversation, the new-task box, a brief, a capture's draft nor the inspector.
    pub fn drawer_collapsed(&self) -> bool {
        let talk = self.talk.as_ref().is_some_and(|talk| talk.open);
        let boxed = self.compose.is_some() || self.welcome_shown();
        let elsewhere = self.inspector.is_some() || self.capture.is_some() || self.brief.is_some();
        !(self.listen.shown || talk || boxed || elsewhere)
    }

    /// The selected item while the drawer's place shows it: not while collapsed, nor while the
    /// inspector, a capture's draft or the new-task box is there. What the rail lifts.
    pub fn rail_shown(&self) -> Option<RailItem> {
        let taken = self.inspector.is_some() || self.capture.is_some();
        let boxed = self.compose.is_some() || self.welcome_shown();
        let talk = self.talk.as_ref().is_some_and(|talk| talk.open);
        self.rail_selected().filter(|item| match item {
            RailItem::Transcript => self.listen.shown && !taken,
            RailItem::Task { .. } => !self.listen.shown && !taken && !boxed && talk,
        })
    }

    /// The fresh start's new-task box is in the drawer: nothing is open yet and the user did not collapse it.
    pub fn welcome_shown(&self) -> bool {
        self.welcome() && self.folded.is_none()
    }

    /// The user's choice of an item: the drawer shows it, open. The transcript is shown here,
    /// in this space's drawer (its source pane's tab is not brought up); a task is put in front.
    /// The inspector closes and the new-task box's line is kept: they were in its place. A
    /// capture's draft stays until kept or discarded. An item not on the rail: nothing.
    pub(super) fn rail_select(&mut self, item: RailItem, effects: &mut Vec<Effect>) {
        if !self.rail().contains(&item) {
            return;
        }
        self.inspector = None;
        if self.compose_host() == Some(super::Host::Drawer) {
            self.put_compose_aside();
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

    /// The next (positive) or previous item of the rail, from the one selected. It wraps. None
    /// selected (a session's space: no task in front): from the top, or the foot.
    pub(super) fn rail_cycle(&mut self, delta: i32, effects: &mut Vec<Effect>) {
        let rail = self.rail();
        let from = match self.rail_selected().and_then(|item| rail.iter().position(|it| *it == item)) {
            Some(at) => at as i64,
            // Just above the top, or on it: a step lands on the first, or wraps to the last.
            None if !rail.is_empty() => {
                if delta > 0 {
                    -1
                } else {
                    0
                }
            }
            None => return,
        };
        let to = (from + i64::from(delta)).rem_euclid(rail.len() as i64) as usize;
        self.rail_select(rail[to], effects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;
    use crate::features::{Feature, Features};
    use crate::listen::{Source, Status};
    use crate::model::{Action, Kind, Origin, Section};
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
    fn the_rail_is_the_tasks_by_urgency_then_the_transcript_always() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        // Nothing here could be heard: no transcript, and no task yet. One task: shown alone.
        assert_eq!(m.rail(), []);
        m.update(Action::OpenCompose { text: "one".into() });
        m.update(Action::ComposeSubmit { open: true });
        assert_eq!(m.rail(), [task(1)]);
        assert!(!m.drawer_collapsed(), "the drawer on it");
        // Something can be heard: the transcript below, idle, in the other section.
        m.update(Action::ListenFixture.sent_by(Origin::System));
        assert_eq!(m.rail(), [task(1), RailItem::Transcript]);
        let sections: Vec<RailSection> = m.rail().into_iter().map(RailItem::section).collect();
        assert_eq!(sections, [RailSection::Workspace, RailSection::Everywhere]);
        // On, it is still last and shown; stopped, it stays, idle, and so does the drawer on it.
        listen(&mut m);
        assert_eq!((m.rail(), m.rail_selected()), (vec![task(1), RailItem::Transcript], Some(RailItem::Transcript)));
        m.update(Action::ListenStop { at: 2_000 });
        assert_eq!((m.rail(), m.rail_selected()), (vec![task(1), RailItem::Transcript], Some(RailItem::Transcript)));
        // The flag off: the transcript goes, and the drawer leaves it.
        m.update(Action::SetFeatures { features: Features::all().with(Feature::Transcripts, false) });
        assert_eq!((m.rail(), m.listen.shown), (vec![task(1)], false));
        // Tasks by urgency: needs you, running, then the rest; done ones go, but the one in front.
        let mut m = three();
        set(&mut m, 1, State::Captured);
        set(&mut m, 2, State::NeedsYou);
        set(&mut m, 3, State::Running);
        assert_eq!(m.rail(), [task(2), task(3), task(1), RailItem::Transcript]);
        set(&mut m, 2, State::Done);
        assert_eq!(m.rail(), [task(3), task(1), RailItem::Transcript]);
    }

    /// The owner: "the transcript is still one click away". With no task, the user's ⌘⇧R opens
    /// the drawer on the transcript idle, the rail beside it; a client's does not.
    #[test]
    fn with_no_task_the_users_toggle_opens_the_idle_transcript() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        m.update(Action::TalkToggle.sent_by(Origin::Socket));
        assert!(!m.listen.shown);
        m.update(Action::TalkToggle);
        assert_eq!(
            (m.rail(), m.rail_selected(), m.drawer_collapsed()),
            (vec![RailItem::Transcript], Some(RailItem::Transcript), false)
        );
        assert!(m.listen.on.is_none());
        m.update(Action::TalkToggle);
        assert!(!m.listen.shown && m.drawer_collapsed());
        // Nothing could be heard: nothing opens.
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::TalkToggle);
        assert!(!m.listen.shown);
    }

    /// The owner: with no conversation, ⌘⇧R opens the transcript in this drawer, as a click on
    /// the rail does; it does not jump to the tab of the pane being heard, in another workspace.
    #[test]
    fn the_toggle_opens_the_transcript_here_not_in_its_panes_tab() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        listen(&mut m);
        m.update(Action::TalkToggle);
        assert!(!m.listen.shown && m.listen.on.is_some());
        // Another workspace, no task in it: the drawer opens on the transcript, the tab stays.
        m.update(Action::NewTab);
        let (tab, space) = (m.active, m.space().clone());
        assert!(m.talk.is_none());
        m.update(Action::TalkToggle);
        assert_eq!((m.rail_selected(), m.active, m.space().clone()), (Some(RailItem::Transcript), tab, space));
    }

    /// The owner: "how do i close the 'transcript' pane?". → (⌘⇧R) folds the drawer into its
    /// rail: the transcript records on, the agents work on, the items stay. A rail click, ⌘⇧R or
    /// ⌘⌥] opens it again. Only the user's.
    #[test]
    fn the_drawer_collapses_into_the_rail_and_only_the_user_opens_it() {
        let mut m = three();
        listen(&mut m);
        let items = m.rail();
        m.update(Action::TalkToggle);
        assert!(m.drawer_collapsed());
        assert_eq!((m.rail(), m.listen.on.as_ref().map(|on| on.status)), (items.clone(), Some(Status::On)));
        // A client's ⌘⇧R, its keys into the conversation or its click on an item: refused.
        for theirs in [Action::TalkToggle, Action::TalkKeys { on: true }, Action::RailSelect { item: task(1) }] {
            assert!(m.refusal(&theirs, Origin::Socket).is_some(), "{theirs:?}");
            m.update(theirs.clone().sent_by(Origin::Socket));
            assert!(m.drawer_collapsed(), "{theirs:?}");
        }
        assert_eq!(m.refusal(&Action::TalkToggle, Origin::Socket), Some(DRAWER_USERS));
        // The user's click on a task: open on its conversation. → again: collapsed, the agent on.
        m.update(Action::RailSelect { item: task(1) });
        assert!(!m.drawer_collapsed() && m.talk.as_ref().is_some_and(|talk| talk.open && talk.task.task() == Some(1)));
        m.update(Action::TalkToggle);
        assert!(m.drawer_collapsed() && m.talk.as_ref().is_some_and(|talk| talk.task.task() == Some(1)));
        // ⌘⇧R opens it on the same; ⌘⌥] on the next item.
        m.update(Action::TalkToggle);
        assert_eq!((m.drawer_collapsed(), m.rail_selected()), (false, Some(task(1))));
        m.update(Action::TalkToggle);
        m.update(Action::TalkCycle { delta: 1 });
        assert!(!m.drawer_collapsed());
        // A client's ⌘⇧R does not collapse it either.
        m.update(Action::TalkToggle.sent_by(Origin::Socket));
        assert!(!m.drawer_collapsed());
    }

    /// The owner: reopening shows what was collapsed. The transcript collapsed beside a
    /// conversation: ⌘⇧R brings the transcript back, not the conversation.
    #[test]
    fn the_toggle_reopens_the_transcript_it_collapsed() {
        let mut m = three();
        listen(&mut m);
        m.update(Action::TalkToggle);
        assert_eq!((m.drawer_collapsed(), m.folded), (true, Some(Folded::Transcript)));
        m.update(Action::TalkToggle);
        assert_eq!((m.drawer_collapsed(), m.rail_shown(), m.folded), (false, Some(RailItem::Transcript), None));
        // A conversation collapsed: the conversation.
        m.update(Action::RailSelect { item: task(2) });
        m.update(Action::TalkToggle);
        m.update(Action::TalkToggle);
        assert_eq!(m.rail_shown(), Some(task(2)));
    }

    /// The owner: ⌘⇧R collapses the fresh start's box too, and opens it again. While the user
    /// collapsed the drawer, a client's ⌘N (the bar's +, the rail's +) is refused: only the user
    /// opens it. The user's own ⌘N opens the box.
    #[test]
    fn the_fresh_start_box_collapses_and_only_the_user_opens_the_drawer_again() {
        let mut m = Model::default();
        assert!(m.welcome_shown() && !m.drawer_collapsed());
        m.update(Action::TalkToggle);
        assert!(m.drawer_collapsed() && !m.welcome_shown());
        let theirs = Action::OpenCompose { text: String::new() };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(DRAWER_USERS));
        m.update(theirs.clone().sent_by(Origin::Socket));
        assert!(m.drawer_collapsed() && m.compose.is_none());
        m.update(Action::TalkToggle);
        assert!(!m.drawer_collapsed() && m.compose.is_some() && m.folded.is_none());
        // The box collapsed in a workspace: the client's ⌘N refused, the user's opens it.
        m.update(Action::CloseCompose);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(theirs.clone());
        m.update(Action::TalkToggle);
        assert!(m.drawer_collapsed() && m.compose.is_none());
        m.update(theirs.clone().sent_by(Origin::Socket));
        assert!(m.compose.is_none());
        m.update(theirs.clone());
        assert!(!m.drawer_collapsed() && m.compose.is_some());
        // Not collapsed by the user: a client's ⌘N opens the box as before.
        m.update(Action::CloseCompose);
        assert_eq!(
            (m.drawer_collapsed(), m.folded, m.refusal(&theirs, Origin::Socket), m.compose.is_some()),
            (true, None, None, false)
        );
    }

    /// The rail shows beside whatever is in the drawer's place: a click on an item there closes
    /// the inspector, and puts the new-task box's line aside.
    #[test]
    fn a_rail_click_beside_the_inspector_or_the_box_shows_the_item() {
        let mut m = three();
        listen(&mut m);
        m.update(Action::InspectorOpen { pane: 1, app: "Notes".into() });
        m.update(Action::RailSelect { item: task(2) });
        assert_eq!((m.inspector.is_none(), m.rail_selected()), (true, Some(task(2))));
        m.update(Action::OpenCompose { text: "later".into() });
        m.update(Action::RailSelect { item: RailItem::Transcript });
        assert_eq!((m.compose.is_none(), m.rail_selected()), (true, Some(RailItem::Transcript)));
        assert!(m.compose_kept.as_ref().is_some_and(|kept| kept.line.text() == "later"));
        // Review: ⌘⇧R collapses the drawer whatever is in its place (the box, typed in, keeps it), and lifts nothing.
        m.tasks.list.iter_mut().for_each(|task| task.materials = vec!["shot.png".into()]);
        for open in [Action::InspectorOpen { pane: 1, app: "Notes".into() }, Action::BriefShow { id: 1 }] {
            m.update(Action::RailSelect { item: task(1) });
            m.update(Action::TalkToggle);
            m.update(open.clone());
            assert!(!m.drawer_collapsed() && m.talk.as_ref().is_some_and(|talk| !talk.open), "{open:?}");
            m.update(Action::TalkToggle);
            assert!(m.drawer_collapsed() && m.rail_shown().is_none(), "{open:?}");
        }
        m.update(Action::OpenCompose { text: "typed".into() });
        m.update(Action::TalkToggle);
        assert!(m.drawer_collapsed() && m.compose_kept.as_ref().is_some_and(|kept| kept.line.text() == "typed"));
        // Beside the box, the front task is selected but not shown.
        m.update(Action::RailSelect { item: task(1) });
        assert_eq!(m.rail_shown(), Some(task(1)));
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!((m.rail_selected(), m.rail_shown()), (Some(task(1)), None));
    }

    /// ⌘⇧L and the left rail's → open the sidebar on the section open last; ← and ⌘⇧L close it.
    #[test]
    fn the_sidebar_toggle_opens_the_section_open_last() {
        let mut m = Model::default();
        assert_eq!((m.sidebar, m.sidebar_toggled()), (None, Section::Repos));
        m.update(Action::ToggleSection { section: Section::Tasks });
        assert_eq!(m.sidebar_toggled(), Section::Tasks);
        m.update(Action::ToggleSection { section: m.sidebar_toggled() });
        assert_eq!((m.sidebar, m.sidebar_toggled()), (None, Section::Tasks));
        m.update(Action::ToggleSection { section: m.sidebar_toggled() });
        assert_eq!(m.sidebar, Some(Section::Tasks));
        m.update(Action::CloseSidebar);
        assert_eq!(m.sidebar_toggled(), Section::Tasks);
        // A section no longer on the rail: its first.
        m.update(Action::ToggleSection { section: Section::Marks });
        m.update(Action::CloseSidebar);
        m.update(Action::SetFeatures { features: Features::all().with(Feature::Marks, false) });
        assert_eq!(m.sidebar_toggled(), Section::Repos);
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
        // ⌘⌥] / ⌘⌥[ step through it in order, and wrap: from the transcript, last, to the top.
        let order = m.rail();
        m.update(Action::TalkCycle { delta: 1 });
        assert_eq!(m.rail_selected(), Some(order[0]));
        m.update(Action::TalkCycle { delta: -2 });
        assert_eq!(m.rail_selected(), Some(order[order.len() - 2]));
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
        // Nor does a client's ⌘⇧R or its keys into the conversation put the transcript away, the
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
        assert_eq!((m.focus_target(), m.rail_selected()), (Target::DrawerRail(order[0]), Some(RailItem::Transcript)));
        m.update(Action::ListChoose { axis: Axis::Right });
        assert_eq!((m.focus_target(), m.rail_selected()), (Target::DrawerRail(order[0]), Some(order[0])));
        // A client's key on it does nothing.
        assert!(m.refusal(&Action::ListMove { delta: 1 }, Origin::Socket).is_some());
        m.update(Action::FocusDir { dir: Dir::Left });
        assert_eq!(m.focus_target(), Target::Pane(1));
        // The owner: the rail always shows, beside the inspector too: → past its rows reaches it.
        m.update(Action::InspectorOpen { pane: 1, app: "Notes".into() });
        m.update(Action::FocusDir { dir: Dir::Right });
        assert_eq!(m.focus_target(), Target::Inspector);
        m.update(Action::FocusDir { dir: Dir::Right });
        assert!(matches!(m.focus_target(), Target::DrawerRail(_)), "{:?}", m.focus_target());
        m.update(Action::InspectorClose);
        m.update(Action::Focus { pane: 1 });
        // Its item gone (the flag off, the transcript with it): the keys leave it.
        m.update(Action::RailSelect { item: RailItem::Transcript });
        m.update(Action::FocusDir { dir: Dir::Right });
        m.update(Action::SetFeatures { features: Features::all().with(Feature::Transcripts, false) });
        assert_ne!(m.focus_target(), Target::DrawerRail(RailItem::Transcript));
    }

    /// Review: a conversation whose agent does not run takes no keys, so → past the panes went
    /// nowhere and the rail was out of the keyboard's reach. It goes on to the rail.
    #[test]
    fn past_a_conversation_that_takes_no_keys_the_keyboard_reaches_the_rail() {
        use crate::layout::Dir;
        use crate::model::zones::Target;
        let mut m = three();
        for id in m.tasks.list.iter().map(|task| task.id).collect::<Vec<_>>() {
            m.update(Action::AgentExited { task: crate::talk::Subject::Task(id) }.sent_by(Origin::System));
        }
        let front = m.rail_selected();
        assert!(!m.drawer_collapsed() && front.is_some() && m.talk.as_ref().is_some_and(|talk| !talk.keys));
        m.update(Action::Focus { pane: 1 });
        m.update(Action::FocusDir { dir: Dir::Right });
        assert_eq!(Some(m.focus_target()), front.map(Target::DrawerRail));
    }
}
