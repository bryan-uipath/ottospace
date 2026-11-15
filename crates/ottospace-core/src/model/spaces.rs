//! The top row: Workspace, then each open task, session and workbook, as a space holding its own
//! tabs. One space is selected: the tab on show is its, and the drawer is on its agent. A space
//! hidden leaves the row; nothing in it ends. See `docs/ai/spaces.md`.

use serde::{Deserialize, Serialize};

use super::{Action, Effect, Host, Model, Origin, Tab};
use crate::talk::{Subject, Talk};
use crate::tasks::TaskId;
use crate::workbook::WorkbookId;

/// Whose a space is.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Owner {
    /// The tabs of no task or workbook. Always first, never hidden.
    Workspace,
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
}

impl Owner {
    /// The conversation the drawer is on while this space is selected.
    pub fn subject(&self) -> Option<Subject> {
        match self {
            Owner::Task { id } => Some(Subject::Task(*id)),
            Owner::Session { runner, id } => Some(Subject::Session { runner: runner.clone(), id: id.clone() }),
            Owner::Workspace | Owner::Workbook { .. } => None,
        }
    }

    /// The space a conversation's agent lives in.
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
}

/// A space as the state shows it: its tabs by place in `tabs`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpaceRow {
    pub of: Owner,
    pub hidden: bool,
    pub selected: bool,
    pub tabs: Vec<usize>,
}

/// Why a client's change of space waits: the user's keys are in the chat.
pub const CHAT_TYPING: &str = "the user is typing in the chat";
/// Why a client's Esc or close of the new-task box is refused: the user is writing in it.
pub const BOX_TYPING: &str = "the user is writing a new task";

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

    /// Every space, with its tabs, for the state.
    pub fn space_rows(&self) -> Vec<SpaceRow> {
        let row = |space: &Space| SpaceRow {
            of: space.of.clone(),
            hidden: space.hidden,
            selected: *self.space() == space.of,
            tabs: self.space_tabs(&space.of),
        };
        self.spaces.iter().map(row).collect()
    }

    /// The places in `tabs` of a space's tabs.
    pub fn space_tabs(&self, of: &Owner) -> Vec<usize> {
        (0..self.tabs.len()).filter(|at| self.tabs[*at].space == *of).collect()
    }

    /// The place in `tabs` of the tab a space shows when selected: its last, else its first.
    fn space_tab(&self, of: &Owner) -> Option<usize> {
        let tabs = self.space_tabs(of);
        let last = self.spaces.iter().find(|space| space.of == *of).and_then(|space| space.last);
        let last = last.and_then(|id| tabs.iter().copied().find(|at| self.tabs[*at].id == id));
        last.or(tabs.first().copied())
    }

    /// Whether a space may hold tabs: its task is live, its workbook kept.
    pub(crate) fn owner_valid(&self, of: &Owner) -> bool {
        match of {
            Owner::Workspace | Owner::Session { .. } => true,
            Owner::Task { id } => self.tasks.live(*id).is_some(),
            Owner::Workbook { id } => self.workbooks.get(*id).is_some(),
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
            None => self.spaces.push(Space { of: of.clone(), hidden: false, last: None }),
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
            Owner::Workspace => None,
        };
        self.leave(effects);
        self.leave_visual(effects);
        self.park_active();
        self.push_tab_of(of);
        if let Some((tab, title)) = self.tabs.get_mut(self.active).zip(title) {
            tab.title = title;
        }
    }

    /// Selects the space at `place` on the top row.
    pub(crate) fn select_space(&mut self, place: usize, effects: &mut Vec<Effect>) {
        let Some(of) = self.shown_spaces().get(place).map(|space| space.of.clone()) else { return };
        if let Some(at) = self.space_tab(&of) {
            self.select_tab(at, effects);
        }
    }

    /// The next (positive) or previous space on the row. It wraps.
    pub(crate) fn cycle_space(&mut self, delta: i32, effects: &mut Vec<Effect>) {
        let shown = self.shown_spaces();
        let at = shown.iter().position(|space| space.of == *self.space()).unwrap_or(0);
        let to = (at as i64 + i64::from(delta)).rem_euclid(shown.len() as i64) as usize;
        self.select_space(to, effects);
    }

    /// Takes the space at `place` (default: the selected one) off the row. Workspace stays. A
    /// selected one gives way to its right neighbour, else its left.
    pub(crate) fn hide_space(&mut self, place: Option<usize>, effects: &mut Vec<Effect>) {
        let shown = self.shown_spaces();
        let at = place.unwrap_or_else(|| shown.iter().position(|space| space.of == *self.space()).unwrap_or(0));
        let Some(of) = shown.get(at).map(|space| space.of.clone()).filter(|of| *of != Owner::Workspace) else {
            return;
        };
        if *self.space() == of
            && let Some(tab) = self.space_after(&of)
        {
            self.select_tab(tab, effects);
        }
        if let Some(space) = self.spaces.iter_mut().find(|space| space.of == of) {
            space.hidden = true;
        }
    }

    /// A space's place on the row: the order tabs keep. One not on the row goes last.
    pub(crate) fn space_rank(&self, of: &Owner) -> usize {
        self.spaces.iter().position(|space| space.of == *of).unwrap_or(usize::MAX)
    }

    /// The tab to show instead of space `of`, hidden or left with no tab: its right neighbour's
    /// on the row, else its left one's.
    pub(crate) fn space_after(&self, of: &Owner) -> Option<usize> {
        let shown = self.shown_spaces();
        let at = shown.iter().position(|space| space.of == *of)?;
        let next = shown.get(at + 1).or(shown.get(at.wrapping_sub(1)))?;
        self.space_tab(&next.of)
    }

    /// Whether a selection would change the selected space: what a client may not do while the
    /// user types in the chat.
    fn changes_space(&self, action: &Action) -> bool {
        let shown = self.shown_spaces();
        let selected = shown.iter().position(|space| space.of == *self.space());
        let other = |of: Option<&Owner>| of.is_some_and(|of| of != self.space());
        match action {
            Action::SelectTab { index } => other(self.tabs.get(*index).map(|tab| &tab.space)),
            Action::SpaceSelect { index } => other(shown.get(*index).map(|space| &space.of)),
            Action::SpaceCycle { .. } => shown.len() > 1,
            Action::SpaceHide { index } => index.or(selected) == selected && *self.space() != Owner::Workspace,
            Action::SpaceOpen { space: of } => other(Some(of)),
            Action::TaskGo { id } => id.is_none_or(|id| other(Some(&Owner::Task { id }))),
            Action::OpenRepo { .. } | Action::RepoClick { .. } => *self.space() != Owner::Workspace,
            // The selected space's last tab closed: the space goes.
            Action::CloseTab { index } => {
                let at = index.unwrap_or(self.active);
                at == self.active && self.space_tabs(self.space()).len() == 1
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

    /// After every step: Workspace first and shown, with a tab; each tab's space on the row, its
    /// tabs side by side in row order; a space with no tab gone, as one whose workbook is; the
    /// selected space shown, and the tab on show its last.
    pub(crate) fn settle_spaces(&mut self) {
        let gone: Vec<Owner> = self.spaces.iter().map(|s| s.of.clone()).filter(|of| !self.owner_valid(of)).collect();
        for tab in self.tabs.iter_mut().filter(|tab| gone.contains(&tab.space)) {
            tab.space = Owner::Workspace;
        }
        let owners: Vec<Owner> = self.tabs.iter().map(|tab| tab.space.clone()).collect();
        self.spaces.retain(|space| space.of == Owner::Workspace || owners.contains(&space.of));
        for of in owners {
            if !self.spaces.iter().any(|space| space.of == of) {
                self.spaces.push(Space { of, hidden: false, last: None });
            }
        }
        let workspace = self.spaces.iter().position(|space| space.of == Owner::Workspace);
        let mut first =
            workspace.map_or(Space { of: Owner::Workspace, hidden: false, last: None }, |at| self.spaces.remove(at));
        first.hidden = false;
        self.spaces.insert(0, first);
        if !self.tabs.iter().any(|tab| tab.space == Owner::Workspace) {
            self.next_tab += 1;
            let title = self.free_title();
            self.tabs.insert(0, Tab { id: self.next_tab, title, space: Owner::Workspace, layout: None, focused: None });
            self.active += 1;
        }
        self.regroup();
        let (of, id) = (self.tabs[self.active].space.clone(), self.tabs[self.active].id);
        let tabs: Vec<(Owner, u32)> = self.tabs.iter().map(|tab| (tab.space.clone(), tab.id)).collect();
        for space in &mut self.spaces {
            if space.of == of {
                (space.hidden, space.last) = (false, Some(id));
            }
            space.last = space.last.filter(|last| tabs.contains(&(space.of.clone(), *last)));
        }
    }

    /// The drawer is on the selected space's conversation, whenever it shows one: a space of no
    /// agent shows none. Changed, it opens as the drawer was (or as the new task in it was), the
    /// keyboard in it only if it was there, the change was the user's and its agent runs.
    pub(crate) fn follow_space(&mut self, origin: Origin, drawer_keys: bool) {
        if let Some(talk) = &self.talk {
            self.drawer_open = talk.open;
        }
        let Some(subject) = self.space().subject() else {
            self.talk = None;
            return;
        };
        if self.talk.as_ref().is_some_and(|talk| talk.task == subject) {
            return;
        }
        let open = self.drawer_open || self.compose_host() == Some(Host::Drawer);
        if open && self.compose_host() == Some(Host::Drawer) {
            self.put_compose_aside();
        }
        // Not under an app, the list, visual mode or a picture being drawn on.
        let free = self.live.is_none() && self.list.is_none() && self.visual.is_none() && self.annotated().is_none();
        let keys = open && drawer_keys && free && origin == Origin::User && self.talk_live(&subject);
        self.talk = Some(Talk { task: subject.clone(), open, keys });
        self.drawer_open = open;
        self.opened.retain(|seen| *seen != subject);
        self.opened.push(subject);
    }
}
