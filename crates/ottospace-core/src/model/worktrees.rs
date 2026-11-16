//! Where work happens: a task's worktree, a workspace's default worktree, and the Repos click
//! that finds or makes the workspace of a checkout. A repository's own folder counts as its main
//! checkout. See `docs/ai/repos.md` and `docs/ai/spaces.md`.

use super::{Effect, Model, Space};
use crate::layout::Axis;
use crate::model::spaces::Owner;
use crate::sessions;
use crate::tasks::TaskId;
use crate::workbook::Workbook;

impl Model {
    /// The listed checkout `dir` is in or below: a repository's own folder or a worktree, the longest.
    pub fn checkout_at(&self, dir: &str) -> Option<&str> {
        let paths = self.repos.iter().flat_map(|repo| {
            std::iter::once(repo.path.as_str()).chain(repo.worktrees.iter().map(|worktree| worktree.path.as_str()))
        });
        paths.filter(|path| sessions::below(dir, path)).max_by_key(|path| path.len())
    }

    /// The worktree a task's agent works in: the listed checkout of its folder. `None`: none
    /// (the home folder, or a folder outside the list).
    pub fn task_worktree(&self, id: TaskId) -> Option<&str> {
        self.checkout_at(self.tasks.get(id)?.dir()?)
    }

    /// The kept workspace of a space: its own, or the one bound to its task.
    fn space_book(&self, of: &Owner) -> Option<&Workbook> {
        let id = match of {
            Owner::Workbook { id } => Some(*id),
            Owner::Task { id } => self.workbooks.bound_to(*id),
            Owner::Home | Owner::Session { .. } => None,
        };
        self.workbooks.get(id?)
    }

    /// A space's default worktree, while it is listed: where its new terminals and tasks start.
    pub fn space_worktree(&self, of: &Owner) -> Option<&str> {
        let path = self.space_book(of)?.worktree.as_deref()?;
        self.repo_listed(path).then_some(path)
    }

    /// A pane that just became a terminal, with no folder of its own, starts in the default
    /// worktree of its tab's space, as listed now.
    pub(crate) fn start_in_default(&mut self, pane: u32) {
        let tab = self.tab_of(pane).and_then(|id| self.tabs.iter().find(|tab| tab.id == id));
        let default = tab.and_then(|tab| self.space_worktree(&tab.space)).map(String::from);
        let terminal = |p: &&mut super::Pane| p.id == pane && p.kind == super::Kind::Terminal && p.cwd.is_none();
        if let Some(opened) = self.panes.iter_mut().find(terminal) {
            opened.cwd = default;
        }
    }

    /// What a workspace opened for a checkout is named: a worktree's branch, else its folder's
    /// name; a repository's own folder, the repository's name.
    fn checkout_title(&self, path: &str) -> Option<String> {
        self.repos.iter().find_map(|repo| {
            let worktree = repo.worktrees.iter().find(|worktree| worktree.path == path);
            let named = worktree.map(|worktree| worktree.branch.clone().unwrap_or_else(|| worktree.name.clone()));
            named.or((repo.path == path).then(|| repo.name.clone()))
        })
    }

    /// How lately a space was selected: higher is later; never, 0.
    fn picked(&self, of: &Owner) -> usize {
        self.last_spaces.iter().rposition(|last| last == of).map_or(0, |at| at + 1)
    }

    /// The space a Repos row's click selects, and the task it puts in front: a workspace whose
    /// default it is, else one that holds a live task working there (the one started last). Of
    /// several, the one selected last, then the first on the row. `closed`: a kept workspace
    /// not on the row counts too (opening it is the user's).
    pub(crate) fn repo_space(&self, path: &str, closed: bool) -> Option<(Owner, Option<TaskId>)> {
        let on_row = |of: &Owner| self.spaces.iter().any(|space| space.of == *of);
        let books = self.workbooks.list.iter().map(|book| self.workbooks.space_of(&Owner::Workbook { id: book.id }));
        let defaults = books.filter(|of| self.space_worktree(of) == Some(path) && (closed || on_row(of)));
        let rank = |of: &Owner| (self.picked(of), std::cmp::Reverse(self.space_rank(of)));
        if let Some(of) = defaults.max_by_key(rank) {
            return Some((of, None));
        }
        let working =
            self.tasks.list.iter().filter(|task| !task.dismissed && self.task_worktree(task.id) == Some(path));
        let latest = |task: &&crate::tasks::Task| (task.started_at, task.created, task.id);
        let mut working: Vec<&crate::tasks::Task> = working.collect();
        working.sort_by_key(latest);
        let task = working.iter().max_by_key(|task| rank(&self.task_space(task.id)))?;
        Some((self.task_space(task.id), Some(task.id)))
    }

    /// A Repos row's click: the workspace of that checkout (`repo_space`), else, the user's, a
    /// new kept workspace for it (named by `checkout_title`, the checkout its default) with a
    /// terminal there, selected. A client's opens a terminal as `OpenRepo` does.
    pub(crate) fn repo_click(&mut self, path: String, axis: Axis, user: bool, effects: &mut Vec<Effect>) {
        match self.repo_space(&path, user) {
            Some((Owner::Workbook { id }, _)) if !self.spaces.iter().any(|s| s.of == Owner::Workbook { id }) => {
                self.workbook_open(id, None, self.home.clone(), effects)
            }
            Some((of, task)) => {
                self.open_space(of, effects);
                if let Some(task) = task.filter(|_| user) {
                    self.set_front(task);
                }
            }
            None if user => self.repo_workspace(path, axis, effects),
            None => self.act(super::Action::OpenRepo { path, axis }, effects),
        }
    }

    /// A new kept workspace for a listed checkout, after the selected space, selected, with a
    /// terminal there: the checkout is its default.
    fn repo_workspace(&mut self, path: String, axis: Axis, effects: &mut Vec<Effect>) {
        let Some(name) = self.checkout_title(&path) else { return };
        let of = self.stray_workspace(Some(name));
        if let Owner::Workbook { id } = of {
            self.workbooks.set_worktree(id, Some(path.clone()));
        }
        // After the selected workspace; from Home, at the row's end, as what Home makes goes.
        let after = self.spaces.iter().position(|space| space.of == *self.space() && space.of != Owner::Home);
        self.spaces.insert(after.map_or(self.spaces.len(), |at| at + 1), Space::new(of.clone()));
        self.list = None;
        self.open_space(of, effects);
        self.open_terminal_at(axis, Some(path), effects);
    }

    /// The worktree a task's row and strip tab name while its workspace's tasks work in more
    /// than one: its checkout's name (`app-2`), else its folder's (`~`: the home folder).
    pub fn worktree_label(&self, id: TaskId) -> Option<String> {
        let members = self.members(&self.task_space(id));
        let first = members.first().map(|first| self.task_worktree(*first));
        let apart = members.iter().any(|other| Some(self.task_worktree(*other)) != first);
        if !apart || !members.contains(&id) {
            return None;
        }
        let checkout = self.task_worktree(id).and_then(|path| self.repo_name(path));
        let folder = || self.tasks.get(id)?.dir().map(|dir| crate::repos::name_of(dir));
        Some(checkout.or_else(folder).unwrap_or_else(|| "~".into()))
    }
}
