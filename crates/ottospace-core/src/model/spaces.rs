//! The top row: Home, then each open task, session and workbook, as a space holding its own
//! tabs. One space is selected: the tab on show is its, and the drawer is on its agent. A space
//! hidden leaves the row; nothing in it ends. Home holds no pane: what lands there goes to a
//! workspace (`settle_home`). See `docs/ai/spaces.md`.

use serde::{Deserialize, Serialize};

use super::{Action, Confirm, Effect, Envelope, Host, Kind, Model, Origin, PaneId, Running, Tab};
use crate::talk::{Subject, Talk};
use crate::tasks::{Task, TaskId};
use crate::workbook::WorkbookId;

/// Whose a space is.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Owner {
    /// Home: the first item of the top row, never hidden. It holds no pane, and one empty tab
    /// of its own (`HOME_TAB`), its screen. `workspace` is what a build before Home called it.
    #[serde(alias = "workspace")]
    Home,
    Task {
        id: TaskId,
    },
    /// A session's that is no task's, opened from Other sessions.
    Session {
        runner: String,
        id: String,
    },
    Workbook {
        id: WorkbookId,
    },
    /// Decisions (`decisions`): ephemeral, one at most, never kept. Its conversation is its card's task's.
    Decisions,
}

impl Owner {
    /// A conversation's own space: a session's, or the one a task has when it is in no other
    /// (`Model::task_space` says where a task is).
    pub fn of(subject: &Subject) -> Owner {
        match subject {
            Subject::Task(id) => Owner::Task { id: *id },
            Subject::Session { runner, id } => Owner::Session { runner: runner.clone(), id: id.clone() },
        }
    }

    pub fn task(&self) -> Option<TaskId> {
        match self {
            Owner::Task { id } => Some(*id),
            _ => None,
        }
    }
}

/// A space of the top row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Space {
    pub of: Owner,
    /// Off the top row: its tabs, panes and agent go on. Its row in Tasks or Workbook opens it again.
    pub hidden: bool,
    /// The tab it showed last, by id: selecting the space shows that one.
    #[serde(skip)]
    pub(crate) last: Option<u32>,
    /// The other tasks in a task's space: each started there by the user's ↵, in that order.
    /// A task is in one space at most.
    #[serde(skip)]
    pub(crate) joined: Vec<TaskId>,
    /// The task whose conversation is in front: the drawer's, the dial's, the chip's. `None`:
    /// the space's own.
    #[serde(skip)]
    pub(crate) front: Option<TaskId>,
    /// It has its place on the row: false until the step that made it ends (`settle_space_order`).
    #[serde(skip)]
    pub(crate) placed: bool,
}

impl Space {
    pub(crate) fn new(of: Owner) -> Self {
        Space { of, hidden: false, last: None, joined: Vec::new(), front: None, placed: false }
    }
}

/// A space as the state shows it: its tabs by place in `tabs`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpaceRow {
    pub of: Owner,
    pub hidden: bool,
    pub selected: bool,
    pub tabs: Vec<usize>,
    /// Its tasks, the most urgent first: needs you, running, later, on hold, done. Its badges: `space_badges`.
    pub tasks: Vec<TaskId>,
    /// The task whose conversation is in front, if it has a task.
    pub front: Option<TaskId>,
    /// Its default worktree, while listed: where its new terminals and tasks start.
    pub worktree: Option<String>,
    /// What its item on the top row shows before its name.
    pub glyph: Glyph,
    /// What its item says of the work in it (`space_activity`): idle, working or needs you.
    pub indicator: super::repo_dots::Activity,
    /// False for a temporary workspace: closing it discards it. Every other space is true.
    pub kept: bool,
}

/// What a space's item on the top row shows before its name (`Model::glyph`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Glyph {
    /// Home's house.
    Home,
    /// Its tasks' badges.
    Tasks,
    /// A workspace of no task holding one app pane and nothing else: that app's icon, by name.
    App { name: String },
    /// No icon of its own: a workspace's quiet dot (lit by `space_activity`), a session's monogram while small.
    Name,
}

/// What `settle_home` asks of the step: how it began.
pub(crate) struct HomeStep {
    /// The step asked for a tab.
    pub(crate) new_tab: bool,
    /// Home was on show.
    pub(crate) on_show: bool,
    /// The tabs there were, by id.
    pub(crate) tabs: Vec<u32>,
    /// It is not the user's, and their keys are in the chat or the new-task box.
    pub(crate) typing: bool,
}

/// The first item of the top row, and its screen.
pub const HOME: &str = "Home";
/// A task's workspace bound to no workspace of the bar, once its tasks are done: its name.
pub const NO_TASK: &str = "Workspace";
/// Home's one tab, always empty: no other tab has this id.
pub const HOME_TAB: u32 = 0;
/// Why a client's change of space waits: the user's keys are in the chat.
pub const CHAT_TYPING: &str = "the user is typing in the chat";
/// Why a client's Esc or close of the new-task box is refused: the user is writing in it.
pub const BOX_TYPING: &str = "the user is writing a new task";
/// Why a client may not say which of a workspace's conversations is in front.
pub const FRONT_USERS: &str = "only the user says which conversation is in front";
/// Why a client may not move the new task the user writes to another workspace.
pub const PIN_USERS: &str = "only the user moves the new task to another workspace";

impl Model {
    /// The selected space: the one of the tab on show.
    pub fn space(&self) -> &Owner {
        &self.tabs[self.active].space
    }

    /// The spaces in row order, hidden ones too.
    pub fn spaces(&self) -> &[Space] {
        &self.spaces
    }

    /// The spaces on the top row, in order.
    pub fn shown_spaces(&self) -> Vec<&Space> {
        self.spaces.iter().filter(|space| !space.hidden).collect()
    }

    /// How many tabs the bars hold: all but Home's own, which is after them in `tabs`.
    pub fn bar_tabs(&self) -> usize {
        self.tabs.iter().filter(|tab| tab.id != HOME_TAB).count()
    }

    /// Every space, with its tabs, for the state: Home has none to list.
    pub fn space_rows(&self) -> Vec<SpaceRow> {
        let row = |space: &Space| SpaceRow {
            of: space.of.clone(),
            hidden: space.hidden,
            selected: *self.shown_space() == space.of,
            tabs: self.space_tabs(&space.of).into_iter().filter(|at| self.tabs[*at].id != HOME_TAB).collect(),
            tasks: self.space_tasks(&space.of),
            front: self.front(&space.of),
            worktree: self.space_worktree(&space.of).map(String::from),
            glyph: self.glyph(&space.of),
            indicator: self.space_activity(&space.of),
            kept: self.space_kept(&space.of),
        };
        self.spaces.iter().map(row).collect()
    }

    /// Whether a space is kept: all but a temporary workspace's and Decisions.
    pub fn space_kept(&self, of: &Owner) -> bool {
        match of {
            Owner::Workbook { id } => self.workbooks.is_kept(*id),
            Owner::Decisions => false,
            _ => true,
        }
    }

    /// Whether a space is a temporary workspace's the user closed, its Undo offered: no tab goes there.
    pub(crate) fn closing(&self, of: &Owner) -> bool {
        matches!(of, Owner::Workbook { id } if self.workbooks.closing(*id))
    }

    /// The tasks in a space, in the order they came: its own, then each that joined it.
    pub fn members(&self, of: &Owner) -> Vec<TaskId> {
        let Owner::Task { id } = of else { return Vec::new() };
        let joined = self.spaces.iter().find(|space| space.of == *of).map(|space| space.joined.clone());
        std::iter::once(*id).chain(joined.unwrap_or_default()).filter(|id| self.tasks.live(*id).is_some()).collect()
    }

    /// How soon a task wants the user: needs you, running, later, on hold, done; then its
    /// priority, then the older.
    pub(crate) fn urgency(&self, id: TaskId) -> (u8, u8, TaskId) {
        let task = self.tasks.get(id);
        let group = task.and_then(|task| task.state.group()).map_or(u8::MAX, |group| group as u8);
        (group, task.map_or(u8::MAX, |task| task.priority), id)
    }

    /// What a space's item on the top row shows: a workspace whose tabs hold one pane, an
    /// app's, shows that app (Slack alone: Slack's icon); with tasks not done, their badges.
    pub fn glyph(&self, of: &Owner) -> Glyph {
        match of {
            Owner::Home => Glyph::Home,
            // Decisions: its card's task's badge.
            Owner::Task { .. } | Owner::Decisions if !self.space_badges(of).is_empty() => Glyph::Tasks,
            Owner::Task { .. } | Owner::Session { .. } | Owner::Decisions => Glyph::Name,
            Owner::Workbook { .. } => {
                let panes = self.tab_panes();
                let leaves: Vec<_> = self.space_tabs(of).into_iter().flat_map(|at| panes[at].clone()).collect();
                match (leaves.as_slice(), leaves.first().and_then(|id| self.pane(*id)).map(|p| &p.kind)) {
                    ([_], Some(Kind::App { name, .. })) => Glyph::App { name: name.clone() },
                    _ => Glyph::Name,
                }
            }
        }
    }

    /// A space's tasks, the most urgent first.
    pub fn space_tasks(&self, of: &Owner) -> Vec<TaskId> {
        if *of == Owner::Decisions {
            return self.front(of).into_iter().collect();
        }
        let mut tasks = self.members(of);
        tasks.sort_by_key(|id| self.urgency(*id));
        tasks
    }

    /// The badges of a space's item on the top row: its tasks not done, the most urgent first.
    pub fn space_badges(&self, of: &Owner) -> Vec<TaskId> {
        let undone = |id: &TaskId| self.tasks.get(*id).is_some_and(Task::undone);
        self.space_tasks(of).into_iter().filter(undone).collect()
    }

    /// The task a space's item stands for, by its title and colour: its own while not done,
    /// else its most urgent one not done. None once all are done: a done task shows only in Tasks.
    pub fn space_task(&self, of: &Owner) -> Option<TaskId> {
        let own = of.task().filter(|id| self.tasks.get(*id).is_some_and(Task::undone));
        own.or_else(|| self.space_badges(of).first().copied())
    }

    /// The task in front in a space: the one set, while it is in it, else the space's own.
    pub fn front(&self, of: &Owner) -> Option<TaskId> {
        // Decisions holds no task: its card's is in front.
        if *of == Owner::Decisions {
            return self.decision_card().map(|(task, _)| task);
        }
        let members = self.members(of);
        let set = self.spaces.iter().find(|space| space.of == *of).and_then(|space| space.front);
        // A task the user put away (Done, Later) is never in front: with none else, none is.
        let away = |id: &TaskId| self.put_away(*id);
        set.filter(|id| members.contains(id) && !away(id)).or(members.into_iter().find(|id| !away(id)))
    }

    /// The space a task is in: the one it joined, else its own.
    pub fn task_space(&self, task: TaskId) -> Owner {
        let joined = self.spaces.iter().find(|space| space.joined.contains(&task));
        joined.map_or(Owner::Task { id: task }, |space| space.of.clone())
    }

    /// The space a conversation is in.
    pub fn subject_space(&self, subject: &Subject) -> Owner {
        match subject {
            Subject::Task(id) => self.task_space(*id),
            Subject::Session { .. } => Owner::of(subject),
        }
    }

    /// The conversation of a space: a session's own, a workspace's front task's. The drawer,
    /// the dial and the agent chip are on the selected space's.
    pub fn space_subject(&self, of: &Owner) -> Option<Subject> {
        match of {
            Owner::Session { runner, id } => Some(Subject::Session { runner: runner.clone(), id: id.clone() }),
            // Decisions: its card's task (`front`): the drawer talks it over, the right rail shows it.
            _ => self.front(of).map(Subject::Task),
        }
    }

    /// Puts a task in front in its space. The space is not selected by it.
    pub(crate) fn set_front(&mut self, task: TaskId) {
        let of = self.task_space(task);
        if let Some(space) = self.spaces.iter_mut().find(|space| space.of == of) {
            space.front = Some(task);
        }
    }

    /// A space's name: its task's title (`space_task`), its session's, its workbook's.
    pub fn space_title(&self, of: &Owner) -> String {
        match of {
            Owner::Home => HOME.into(),
            // Several tasks in a kept workspace, or none not done: the workspace's name, else `NO_TASK`.
            Owner::Task { id } => {
                let task = self.space_task(of);
                let book = self.workbooks.bound_to(*id).filter(|_| self.members(of).len() > 1 || task.is_none());
                let name = book.and_then(|book| self.workbooks.get(book)).map(|book| book.name.clone());
                let title = task.and_then(|id| self.tasks.get(id)).map(|task| task.title.clone());
                name.or(title).unwrap_or_else(|| NO_TASK.into())
            }
            Owner::Session { runner, id } => self.session_title(runner, id),
            Owner::Workbook { id } => self.workbooks.get(*id).map(|book| book.name.clone()).unwrap_or_default(),
            Owner::Decisions => super::decisions::NAME.into(),
        }
    }

    /// The places in `tabs` of a space's tabs.
    pub fn space_tabs(&self, of: &Owner) -> Vec<usize> {
        (0..self.tabs.len()).filter(|at| self.tabs[*at].space == *of).collect()
    }

    /// The place in `tabs` of the main window's space's `n`th tab, as its tab row draws them (⌘1…).
    pub fn visible_tab(&self, n: usize) -> Option<usize> {
        self.space_tabs(self.shown_space()).get(n).copied()
    }

    /// The place in `tabs` of the tab a space shows when selected: its last, else its first; one
    /// that is not out on another display before one that is.
    fn space_tab(&self, of: &Owner) -> Option<usize> {
        let all = self.space_tabs(of);
        let here: Vec<usize> = all.iter().copied().filter(|at| !self.is_out(self.tabs[*at].id)).collect();
        let tabs = if here.is_empty() { all } else { here };
        let last = self.spaces.iter().find(|space| space.of == *of).and_then(|space| space.last);
        let last = last.and_then(|id| tabs.iter().copied().find(|at| self.tabs[*at].id == id));
        last.or(tabs.first().copied())
    }

    /// Whether a space may hold tabs: its task is live, its workbook kept.
    pub(crate) fn owner_valid(&self, of: &Owner) -> bool {
        match of {
            Owner::Home | Owner::Session { .. } => true,
            Owner::Task { id } => self.tasks.live(*id).is_some(),
            Owner::Workbook { id } => self.workbooks.get(*id).is_some(),
            Owner::Decisions => self.decisions.is_some(),
        }
    }

    /// Whether a space may open: one a tab may be of, and a session's only while it is listed,
    /// its agent runs, or it has its space already.
    fn may_open(&self, of: &Owner) -> bool {
        let session = |runner: &str, id: &str| {
            let subject = Subject::Session { runner: runner.into(), id: id.into() };
            self.session(runner, id).is_some() || self.agent_runs(&subject) || self.spaces.iter().any(|s| s.of == *of)
        };
        match of {
            Owner::Session { runner, id } => session(runner, id),
            _ => self.owner_valid(of),
        }
    }

    /// A space on the row: added at its end if it is not there, shown if it was hidden.
    pub(crate) fn add_space(&mut self, of: &Owner) {
        match self.spaces.iter_mut().find(|space| space.of == *of) {
            Some(space) => space.hidden = false,
            None => self.spaces.push(Space::new(of.clone())),
        }
    }

    /// Opens a space and selects it: one it lacks is made, with one empty tab, at the row's end;
    /// a hidden one comes back where it was, its tabs as they were.
    pub(crate) fn open_space(&mut self, of: Owner, effects: &mut Vec<Effect>) {
        if !self.may_open(&of) {
            return;
        }
        self.add_space(&of);
        if let Some(at) = self.space_tab(&of) {
            return self.select_tab(at, effects);
        }
        let title = match &of {
            Owner::Task { id } => self.tasks.get(*id).map(|task| {
                let place = task.repo.as_deref().and_then(|path| self.repo_name(path));
                place.unwrap_or_else(|| super::short_title(&task.title))
            }),
            Owner::Session { runner, id } => Some(super::short_title(&self.session_title(runner, id))),
            Owner::Workbook { id } => self.workbooks.get(*id).map(|book| book.name.clone()),
            Owner::Home | Owner::Decisions => None,
        };
        self.leave(effects);
        self.leave_visual(effects);
        self.park_active();
        // A session's tab is in its checkout: its city, and a terminal opened there starts in it.
        let folder = match &of {
            Owner::Session { runner, id } => self.facts(runner, id).flatten().map(|found| found.repo.clone()),
            _ => None,
        };
        self.push_tab_of(of);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if let Some(title) = title {
                tab.title = title;
            }
            tab.folder = folder;
        }
    }

    /// Selects the space at `place` on the top row.
    pub(crate) fn select_space(&mut self, place: usize, effects: &mut Vec<Effect>) {
        let Some(of) = self.shown_spaces().get(place).map(|space| space.of.clone()) else { return };
        if let Some(at) = self.space_tab(&of) {
            self.select_tab(at, effects);
        }
    }

    /// A click on a space of the top row. The user's on the selected one, its conversation
    /// closed, opens it, as ⌘⇧R does; any other click only selects.
    pub(crate) fn space_click(&self, sent: Envelope) -> Envelope {
        let Action::SpaceClick { index } = sent.action else { return sent };
        let opens = sent.origin == Origin::User && self.agent_chip().is_some_and(|chip| chip.space == index);
        let action = if opens { Action::TalkToggle } else { Action::SpaceSelect { index } };
        action.sent_by(sent.origin)
    }

    /// The user's close of the last tab of a temporary workspace whose terminals there run
    /// something asks first, as its × does (`SpaceHideAsked`): yes closes the workspace, with its
    /// Undo. Any other `CloseTabAsked` is `CloseTab`.
    pub(crate) fn close_tab_asked(&self, sent: Envelope) -> Envelope {
        let Action::CloseTabAsked { index, running } = sent.action else { return sent };
        let at = index.unwrap_or(self.active);
        let panes = self.tab_panes().get(at).cloned().unwrap_or_default();
        let asks = |tab: &Tab| match tab.space {
            Owner::Workbook { id } => {
                self.closable(id)
                    && self.space_tabs(&tab.space).len() == 1
                    && running.iter().any(|r| panes.contains(&r.pane))
            }
            _ => false,
        };
        let tab = self.tabs.get(at).filter(|tab| sent.origin == Origin::User && asks(tab));
        let place = tab.and_then(|tab| self.shown_spaces().iter().position(|space| space.of == tab.space));
        let action = match place {
            Some(place) => Action::SpaceHideAsked { index: Some(place), running },
            None => Action::CloseTab { index },
        };
        action.sent_by(sent.origin)
    }

    /// The next (positive) or previous space on the row. It wraps.
    pub(crate) fn cycle_space(&mut self, delta: i32, effects: &mut Vec<Effect>) {
        let shown = self.shown_spaces();
        let at = shown.iter().position(|space| space.of == *self.shown_space()).unwrap_or(0);
        let to = (at as i64 + i64::from(delta)).rem_euclid(shown.len() as i64) as usize;
        self.select_space(to, effects);
    }

    /// Takes the space at `place` (default: the selected one) off the row. Home stays. A
    /// selected one gives way to its right neighbour, else its left.
    pub(crate) fn hide_space(&mut self, place: Option<usize>, effects: &mut Vec<Effect>) {
        let Some(of) = self.space_to_hide(place) else { return };
        if *self.shown_space() == of
            && let Some(tab) = self.space_after(&of)
        {
            self.select_tab(tab, effects);
        }
        if let Some(space) = self.spaces.iter_mut().find(|space| space.of == of) {
            space.hidden = true;
        }
    }

    /// The space `hide_space` takes off the row: at `place`, else the selected one. Never Home.
    fn space_to_hide(&self, place: Option<usize>) -> Option<Owner> {
        let shown = self.shown_spaces();
        let at = place.unwrap_or_else(|| shown.iter().position(|space| space.of == *self.shown_space()).unwrap_or(0));
        shown.get(at).map(|space| space.of.clone()).filter(|of| *of != Owner::Home)
    }

    /// The user's × or ⌘⌃W. A temporary workspace whose terminals run something (`running`) is
    /// asked about first, as quit is: closed, they end once its Undo runs out. Else `hide_space`.
    pub(crate) fn hide_space_asked(
        &mut self,
        place: Option<usize>,
        mut running: Vec<Running>,
        effects: &mut Vec<Effect>,
    ) {
        let id = match self.space_to_hide(place) {
            Some(Owner::Workbook { id }) if self.closable(id) => id,
            _ => return self.hide_space(place, effects),
        };
        let of = Owner::Workbook { id };
        let panes: Vec<PaneId> = (self.tabs.iter().zip(self.tab_panes()))
            .filter_map(|(tab, panes)| (tab.space == of).then_some(panes))
            .flatten()
            .collect();
        running.retain(|r| panes.contains(&r.pane));
        if running.is_empty() {
            return self.hide_space(place, effects);
        }
        // The question is ottospace's own UI: a live window would cover it.
        self.leave(effects);
        self.confirm = Some(Confirm { running, close: Some(id), ..Confirm::default() });
    }

    /// A space's place on the row: the order tabs keep. One not on the row goes last, and
    /// Home's own tab after all: a tab's place counts the workspaces' tabs alone.
    pub(crate) fn space_rank(&self, of: &Owner) -> usize {
        let at = self.spaces.iter().position(|space| space.of == *of).unwrap_or(usize::MAX - 1);
        if *of == Owner::Home { usize::MAX } else { at }
    }

    /// The tab to show instead of space `of`, hidden or left with no tab: its right neighbour's
    /// on the row, else its left one's. One that is out on another display: Home's.
    pub(crate) fn space_after(&self, of: &Owner) -> Option<usize> {
        let shown = self.shown_spaces();
        let at = shown.iter().position(|space| space.of == *of)?;
        let next = shown.get(at + 1).or(shown.get(at.wrapping_sub(1)))?;
        let tab = self.space_tab(&next.of)?;
        match self.is_out(self.tabs[tab].id) {
            true => self.tabs.iter().position(|tab| tab.id == HOME_TAB),
            false => Some(tab),
        }
    }

    /// Whether a selection would change the selected space: what a client may not do while the
    /// user types in the chat.
    pub(crate) fn changes_space(&self, action: &Action) -> bool {
        let shown = self.shown_spaces();
        let selected = shown.iter().position(|space| space.of == *self.shown_space());
        let other = |of: Option<&Owner>| of.is_some_and(|of| of != self.shown_space());
        match action {
            Action::SelectTab { index } => other(self.tabs.get(*index).map(|tab| &tab.space)),
            Action::SpaceSelect { index } | Action::SpaceClick { index } => {
                other(shown.get(*index).map(|space| &space.of))
            }
            Action::SpaceCycle { .. } => shown.len() > 1,
            Action::SpaceHide { index } => index.or(selected) == selected && *self.shown_space() != Owner::Home,
            Action::SpaceOpen { space: of } => other(Some(of)),
            Action::TaskGo { id } => id.is_none_or(|id| other(Some(&self.task_space(id)))),
            Action::OpenRepo { .. } | Action::RepoClick { .. } => *self.shown_space() != Owner::Home,
            // The selected space's last tab closed (its tabs out on other displays do not count): the space goes.
            Action::CloseTab { index } => {
                let at = index.unwrap_or(self.active);
                let here = self.space_tabs(self.shown_space()).into_iter().filter(|at| !self.is_out(self.tabs[*at].id));
                at == self.active && here.count() == 1
            }
            _ => false,
        }
    }

    /// Shows tab `id` again if a step selected another space under the user's typing: what it
    /// made stays, in the background. Nothing else changes: the keys stay where they were.
    pub(crate) fn keep_shown(&mut self, id: u32, effects: &mut Vec<Effect>) {
        let Some(at) = self.tabs.iter().position(|tab| tab.id == id) else { return };
        if self.tabs[at].space != *self.space() {
            // An app it made live there would stay up over the tab shown again.
            self.leave(effects);
            self.park_active();
            self.active = at;
            self.unpark_active();
        }
    }

    /// Why a client's change of space waits, if it does.
    pub(crate) fn space_refusal(&self, action: &Action, origin: Origin) -> Option<&'static str> {
        let typing = self.talk.as_ref().is_some_and(|talk| talk.keys);
        (origin != Origin::User && typing && self.changes_space(action)).then_some(CHAT_TYPING)
    }

    /// After every step: Home first and shown, with its one empty tab; each tab's space on the
    /// row, its tabs side by side in row order (Home's last); a space with no tab gone, as one
    /// whose workbook is.
    pub(crate) fn settle_spaces(&mut self, origin: Origin, step: &HomeStep) {
        let gone: Vec<Owner> = self.spaces.iter().map(|s| s.of.clone()).filter(|of| !self.owner_valid(of)).collect();
        for tab in self.tabs.iter_mut().filter(|tab| gone.contains(&tab.space)) {
            tab.space = Owner::Home;
        }
        self.settle_home(origin, step);
        let owners: Vec<Owner> = self.tabs.iter().map(|tab| tab.space.clone()).collect();
        self.spaces.retain(|space| space.of == Owner::Home || owners.contains(&space.of));
        for of in owners {
            if !self.spaces.iter().any(|space| space.of == of) {
                self.spaces.push(Space::new(of));
            }
        }
        let workspace = self.spaces.iter().position(|space| space.of == Owner::Home);
        let mut first = workspace.map_or(Space::new(Owner::Home), |at| self.spaces.remove(at));
        first.hidden = false;
        self.spaces.insert(0, first);
        // The workspaces' spaces in the one order (`reorder`); the tabs follow.
        self.settle_space_order();
        self.regroup();
    }

    /// After every step, once the tabs out settled: the main window's space shown, its tab that
    /// space's last, and the workspace selected last. A tab out is no space's selection.
    pub(crate) fn settle_selected(&mut self) {
        let main = self.main_at();
        let (of, id) = (self.tabs[main].space.clone(), self.tabs[main].id);
        // The workspace selected last: where a stray tab goes.
        let before = std::mem::take(&mut self.last_spaces);
        self.last_spaces = before.into_iter().filter(|last| *last != of && self.owner_valid(last)).collect();
        if matches!(of, Owner::Task { .. } | Owner::Workbook { .. }) {
            self.last_spaces.push(of.clone());
        }
        let tabs: Vec<(Owner, u32)> = self.tabs.iter().map(|tab| (tab.space.clone(), tab.id)).collect();
        let out = self.is_out(id);
        let mut shown_again = false;
        for space in &mut self.spaces {
            // Never a tab out (the main window's tab closed mid-step: `settle_out` picks another).
            if space.of == of && !out {
                shown_again |= space.hidden;
                (space.hidden, space.last) = (false, Some(id));
            }
            space.last = space.last.filter(|last| tabs.contains(&(space.of.clone(), *last)));
        }
        // Shown again: it takes its place among the shown workspaces (`settle_space_order`).
        if shown_again {
            self.settle_space_order();
            self.regroup();
        }
    }

    /// Home, the first space, holds no pane and one empty tab, its screen (`HOME_TAB`). After
    /// every step, what else is there goes to a workspace. What the user made from Home (a
    /// pane put in its tab, a tab asked for or built), and a folder they opened: a NEW
    /// workspace. Anything else (another's pane or tab; a stray tab whose space went: a task
    /// dismissed, a workspace deleted, a session's after a restart): the workspace selected
    /// last, else the first on the row. One is made, kept like any other, if there is none. A
    /// stray tab that holds nothing is closed.
    fn settle_home(&mut self, origin: Origin, step: &HomeStep) {
        let new_tab = step.new_tab;
        let hint = self.home_hint.take();
        let panes = self.tab_panes();
        let home: Vec<usize> = (0..self.tabs.len()).filter(|at| self.tabs[*at].space == Owner::Home).collect();
        let own = |at: &usize| self.tabs[*at].id == HOME_TAB;
        let (moves, closes): (Vec<usize>, Vec<usize>) = home
            .iter()
            .filter(|at| !own(at) || !panes[**at].is_empty())
            .partition(|at| !panes[**at].is_empty() || new_tab);
        let filled = moves.iter().any(own);
        // Another's pane on Home under the user's typing: Home stays on show.
        // Another's pane or tab from Home under the user's typing: Home stays on show.
        let stays = step.typing && step.on_show && moves.contains(&self.active);
        if !moves.is_empty() {
            let new = moves.iter().any(|at| !step.tabs.contains(&self.tabs[*at].id));
            let made = origin == Origin::User && (hint.is_some() || (step.on_show && (filled || new)));
            let open =
                |of: &&Owner| self.owner_valid(of) && !self.closing(of) && self.spaces.iter().any(|s| s.of == **of);
            // Else the first workspace on the row, before one is made.
            let kept =
                |space: &&Space| !space.hidden && matches!(space.of, Owner::Task { .. } | Owner::Workbook { .. });
            let any = || self.spaces.iter().filter(|space| open(&&space.of)).find(kept).map(|space| space.of.clone());
            // Then the last one still kept, though closed: no file is made while one is.
            let kept_last = || self.last_spaces.iter().rfind(|of| self.owner_valid(of) && !self.closing(of)).cloned();
            // Then any kept workspace of no task: one is made only while there is none.
            let book =
                || self.workbooks.listed().find(|book| book.task.is_none()).map(|book| Owner::Workbook { id: book.id });
            // One on the row before a hidden one: a step of another's unhides none it need not.
            let shown = |of: &&Owner| open(of) && self.spaces.iter().any(|s| s.of == **of && !s.hidden);
            let last = self.last_spaces.iter().rfind(shown).cloned().or_else(any);
            let last = last.or_else(|| self.last_spaces.iter().rfind(open).cloned()).or_else(kept_last).or_else(book);
            let ids: Vec<u32> = moves.iter().map(|at| self.tabs[*at].id).collect();
            let to = match last.filter(|_| !made) {
                Some(of) => of,
                None => {
                    let to = self.stray_workspace(hint.as_ref().map(|(name, _)| name.clone()));
                    if let Owner::Workbook { id } = to {
                        self.workbooks.note_made(&ids, id);
                        // The user's, for a folder of the Repos list: it is its default.
                        if made {
                            self.workbooks.set_worktree(id, hint.map(|(_, path)| path));
                        }
                    }
                    to
                }
            };
            // A workspace bound to a task since it was selected: its tabs are the task's (seed 28003).
            // A task that joined another's space: there, as `place_tab` sends one.
            let to = match self.workbooks.space_of(&to) {
                Owner::Task { id } => self.task_space(id),
                of => of,
            };
            // A dismissal taken back finds its task's tabs where they went.
            for (_, tabs, home) in &mut self.unlinked {
                if tabs.iter().any(|id| ids.contains(id)) {
                    *home = to.clone();
                }
            }
            for at in moves {
                self.tabs[at].space = to.clone();
                // Home's own tab left it: a tab like any other, with the next id and name.
                if self.tabs[at].id == HOME_TAB {
                    self.next_tab += 1;
                    self.tabs[at].id = self.next_tab;
                    if self.tabs[at].title == HOME {
                        self.tabs[at].title = self.free_title();
                    }
                }
            }
            self.add_space(&to);
        }
        let closes: Vec<u32> = closes.into_iter().map(|at| self.tabs[at].id).collect();
        if !self.tabs.iter().any(|tab| tab.id == HOME_TAB) {
            let tab = Tab {
                id: HOME_TAB,
                title: HOME.into(),
                space: Owner::Home,
                layout: None,
                focused: None,
                display: None,
                folder: None,
                auto: None,
            };
            self.tabs.push(tab);
        }
        if stays {
            self.park_active();
            self.active = self.tabs.iter().position(|tab| tab.id == HOME_TAB).unwrap_or(0);
        }
        for id in closes {
            let Some(at) = self.tabs.iter().position(|tab| tab.id == id) else { continue };
            if at == self.active {
                // Empty: nothing is parked. Home's screen takes its place.
                self.active = self.tabs.iter().position(|tab| tab.id == HOME_TAB).unwrap_or(0);
            }
            self.tabs.remove(at);
            self.active -= usize::from(at < self.active);
            self.unlinked.iter_mut().for_each(|(_, ids, _)| ids.retain(|tab| *tab != id));
            self.marks.forget_tab(id);
            self.workbooks.forget_tab(id);
        }
        // Home alone: tab ids start again, as with the last tab closed.
        if self.tabs.len() == 1 {
            self.next_tab = 0;
        }
    }

    /// The drawer is on the selected space's conversation, whenever it shows one: a space of no
    /// agent shows none. Changed, it opens as the drawer was (or as the new task in it was), the
    /// keyboard in it only if it was there, the change was the user's and its agent runs.
    pub(crate) fn follow_space(&mut self, origin: Origin, drawer_keys: bool) {
        if let Some(talk) = &self.talk {
            self.drawer_open = talk.open;
        }
        // The main window's drawer: one per conversation, and the main window has it.
        let Some(subject) = self.space_subject(self.shown_space()) else {
            self.talk = None;
            return;
        };
        if self.talk.as_ref().is_some_and(|talk| talk.task == subject) {
            return;
        }
        let open = self.drawer_open || self.compose_host() == Some(Host::Drawer);
        // The box stays over it, pinned where it was opened (`compose_space`): a change of space takes no line.
        // Not under an app, the list, visual mode or a picture being drawn on.
        let free = self.live.is_none() && self.list.is_none() && self.visual.is_none() && self.annotated().is_none();
        // In Decisions the keys stay on the card: the drawer takes them only when asked (⌘L, a click).
        let keys = open
            && drawer_keys
            && free
            && *self.shown_space() != Owner::Decisions
            && origin == Origin::User
            && self.talk_live(&subject)
            && self.compose_host() != Some(Host::Drawer);
        self.talk = Some(Talk { task: subject.clone(), open, keys });
        self.drawer_open = open;
        self.opened.retain(|seen| *seen != subject);
        self.opened.push(subject);
    }
}

/// The new-task box is pinned to the space it was opened in (`docs/ai/spaces.md`, "Pinned").
impl Model {
    /// The space the box's ↵ starts its task in: the one it was opened in (or switched to),
    /// followed by a tab of it through renames. None while nothing pins it.
    pub fn compose_space(&self) -> Option<Owner> {
        // A capture's draft in the bar is no box of the drawer: its task goes where the user is.
        let (tab, of) = self.compose_to.as_ref().filter(|_| self.bar_draft().is_none())?;
        let by_tab = self.tabs.iter().find(|t| t.id == *tab).map(|t| t.space.clone());
        let of = by_tab.unwrap_or_else(|| of.clone());
        // Closed or hidden: it pins nothing.
        self.spaces.iter().any(|s| s.of == of && !s.hidden).then_some(of)
    }

    /// The box's space, else the one on show.
    pub(crate) fn box_space(&self) -> Owner {
        self.compose_space().unwrap_or_else(|| self.space().clone())
    }

    /// The box in the drawer goes to another space than the one on show: it says so, and offers Switch.
    pub fn compose_elsewhere(&self) -> bool {
        self.compose_host() == Some(Host::Drawer) && self.compose_space().is_some_and(|of| of != *self.space())
    }

    /// The box now goes to the space on show; a new worktree only while this one's repository offers one.
    pub(crate) fn compose_here(&mut self) {
        self.compose_to = Some((self.tabs[self.active].id, self.space().clone()));
        if self.box_worktree().is_none() {
            self.compose_worktree = false;
        }
    }

    /// The user closed the box they opened from the conversation's keys: the keys go back to the
    /// conversation on show, the one they switched to too, if its agent is there to take them.
    pub(crate) fn keys_back(&mut self, effects: &mut Vec<Effect>) {
        let open = self.talk.as_ref().filter(|talk| talk.open).map(|talk| talk.task.clone());
        let free = self.live.is_none() && self.list.is_none() && self.visual.is_none();
        if self.by_user && self.compose_talked && free && open.is_some_and(|task| self.talk_live(&task)) {
            self.talk_keys(true, effects);
        }
    }

    /// After every step: no box, no pin; its space gone, it goes where the user is.
    pub(crate) fn settle_compose_to(&mut self) {
        if self.compose.is_none() {
            (self.compose_to, self.compose_talked) = (None, false);
        } else if let Some(of) = self.compose_space() {
            if let Some((_, was)) = &mut self.compose_to {
                *was = of;
            }
        } else if self.bar_draft().is_none() {
            self.compose_here();
        }
    }
}
