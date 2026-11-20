//! Where work happens: a task's worktree, a workspace's default worktree, and the Repos click
//! that finds or makes the workspace of a checkout. A repository's own folder counts as its main
//! checkout. See `docs/ai/repos.md` and `docs/ai/spaces.md`.

use super::{Effect, Model, Space};
use crate::layout::Axis;
use crate::model::spaces::Owner;
use crate::runner::RunnerId;
use crate::sessions;
use crate::tasks::TaskId;
use crate::workbook::Workbook;

/// How often a folder an agent moved into, that the Repos list does not hold yet, is looked for
/// again (ms), and for how long: a worktree made a moment ago may not be read yet.
pub(crate) const MOVE_RETRY_MS: u64 = 2_000;
pub(crate) const MOVE_WAIT_MS: u64 = 10_000;

/// A folder a task's agent said it works in, not yet a listed checkout of its repository.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Moving {
    pub(crate) task: TaskId,
    pub(crate) cwd: String,
    /// When it was said, and when the list was last asked for again (ms).
    pub(crate) since: u64,
    pub(crate) asked: u64,
}

/// A new task's worktree the box asked git for (`asked`, on `branch`), not made yet.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Making {
    pub(crate) task: TaskId,
    pub(crate) asked: String,
    pub(crate) branch: String,
    /// A bug report's (`bug_send`): its repository need not be listed.
    pub(crate) bug: bool,
}

/// The new-task box's choice, while it offers one: the task works in its checkout, or (`new`)
/// in a new worktree of `repo` (its name) on `branch`.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct WorktreeChoice {
    pub new: bool,
    pub repo: String,
    pub branch: String,
}

/// What ↵ asks for with the box's choice on: the worktree of the repository at `repo` on
/// `branch`, inside `root`.
pub(crate) struct NewWorktree {
    repo: String,
    branch: String,
    root: String,
}

/// The user's delete of a linked worktree that git is at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Deleting {
    pub(crate) path: String,
    pub(crate) repo: String,
    pub(crate) branch: String,
}

/// A repository's checkouts: its own folder, then its worktrees.
fn paths(repo: &crate::repos::Repo) -> impl Iterator<Item = &str> {
    std::iter::once(repo.path.as_str()).chain(repo.worktrees.iter().map(|worktree| worktree.path.as_str()))
}

/// True if `path` is in a worktree an agent's sub-agent made: Claude Code's
/// `<repo>/.claude/worktrees/agent-<id>`. Such a move is never the task's.
pub(crate) fn subagent_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.windows(3).any(|w| w[0] == ".claude" && w[1] == "worktrees" && w[2].starts_with("agent-"))
}

impl Model {
    /// The listed checkout `dir` is in or below: a repository's own folder or a worktree, the longest.
    pub fn checkout_at(&self, dir: &str) -> Option<&str> {
        let all = self.repos.iter().flat_map(paths);
        all.filter(|path| sessions::below(dir, path)).max_by_key(|path| path.len())
    }

    /// The worktree a task's agent works in: the one it moved into, while listed (`task_moved`),
    /// else the listed checkout of its folder. `None`: none (the home folder, or a folder outside
    /// the list).
    pub fn task_worktree(&self, id: TaskId) -> Option<&str> {
        self.task_moved(id).or_else(|| self.checkout_at(self.tasks.get(id)?.dir()?))
    }

    /// The listed worktree a task's agent moved into, if it did: where it works and starts now.
    pub fn task_moved(&self, id: TaskId) -> Option<&str> {
        let path = self.tasks.get(id)?.worktree.as_deref()?;
        self.repo_listed(path).then_some(path)
    }

    /// The checkouts of the repository that has `checkout` among its own.
    fn sibling_checkouts(&self, checkout: &str) -> Vec<&str> {
        let repo = self.repos.iter().find(|repo| paths(repo).any(|path| path == checkout));
        repo.map(|repo| paths(repo).collect()).unwrap_or_default()
    }

    /// A task's session said where its agent works now (`SessionWork`, the system's reading of
    /// it): in another checkout of its repository, the task works there from now on. Not for a
    /// runner whose records do not move with the agent, nor into a sub-agent's worktree. A folder
    /// the list does not hold yet: the list is read again (`Effect::ReadRepos`), a while.
    pub(crate) fn agent_cwd(&mut self, runner: &str, session: &str, cwd: String, effects: &mut Vec<Effect>) {
        if !RunnerId::named(runner).is_some_and(|runner| runner.spec().disk.cwd_moves) {
            return;
        }
        let of = |task: &crate::tasks::Task| {
            task.session.as_ref().is_some_and(|link| link.runner == runner && link.id == session)
        };
        let Some(id) = self.tasks.list.iter().find(|task| of(task)).map(|task| task.id) else { return };
        // Said again (each read says the last one): nothing new.
        if self.agent_cwds.iter().any(|(task, seen)| *task == id && *seen == cwd) {
            return;
        }
        self.agent_cwds.retain(|(task, _)| *task != id);
        self.agent_cwds.push((id, cwd.clone()));
        self.moving.retain(|moving| moving.task != id);
        if subagent_path(&cwd) {
            return;
        }
        let now = self.now;
        self.moving.push(Moving { task: id, cwd, since: now, asked: now });
        if !self.follow_moves() {
            effects.push(Effect::ReadRepos);
        }
    }

    /// Each folder an agent moved into that is now a listed checkout of its task's repository:
    /// the task works there. True if none waits for the list. One that does stays, a task's
    /// last: any later read of the list (the user's + too) may find it.
    pub(crate) fn follow_moves(&mut self) -> bool {
        let moving = std::mem::take(&mut self.moving);
        let mut waits = false;
        for moving in moving {
            let Some(task) = self.tasks.get(moving.task) else { continue };
            // The worktree it moved into last, though no longer listed: what its workspace may still name.
            let stored = task.worktree.clone();
            let Some(current) = self.task_worktree(moving.task).map(String::from) else {
                self.moving.push(moving);
                continue;
            };
            let siblings = self.sibling_checkouts(&current);
            let at = siblings.into_iter().filter(|path| sessions::below(&moving.cwd, path)).max_by_key(|p| p.len());
            let at = at.map(String::from);
            let gone = stored.as_ref().filter(|stored| **stored != current);
            match at {
                Some(new) if new != current => self.moved(moving.task, stored.as_deref().unwrap_or(&current), new),
                // Back in the checkout it works in, after its last one went: that one is forgotten.
                Some(new) if gone.is_some() && new == moving.cwd => {
                    let was = gone.cloned().unwrap_or_default();
                    self.moved(moving.task, &was, new)
                }
                // In its own checkout: a `cd`, unless it is a new worktree there not listed yet.
                Some(same) if same == moving.cwd => {}
                _ => {
                    waits = true;
                    self.moving.push(moving)
                }
            }
        }
        !waits
    }

    /// The task works in `new` now. Its workspace's default follows it from `was` (its worktree
    /// until now) when it is the workspace's only task; with others there, it stays as it is.
    fn moved(&mut self, id: TaskId, was: &str, new: String) {
        self.default_follows(id, was, &new);
        if let Some(task) = self.tasks.list.iter_mut().find(|task| task.id == id) {
            task.worktree = Some(new);
        }
    }

    /// A task's workspace's default follows it from `was` to `new` when it is the workspace's
    /// only task; with others there, it stays as it is.
    fn default_follows(&mut self, id: TaskId, was: &str, new: &str) {
        let of = self.task_space(id);
        let book = self.space_book(&of).filter(|book| book.worktree.as_deref() == Some(was)).map(|book| book.id);
        if let Some(book) = book.filter(|_| self.members(&of) == [id]) {
            self.workbooks.set_worktree(book, Some(new.to_string()));
        }
    }

    /// What ↵ in the new-task box would ask for with a new worktree: offered while the box is
    /// the drawer's, the task would start an agent in a listed checkout (the line's `>repo`, else
    /// the workspace's), not in a folder of its own, and there is a folder for worktrees.
    pub(crate) fn box_worktree(&self) -> Option<NewWorktree> {
        let root = self.repo_dirs.worktrees.clone()?;
        if self.compose_host() != Some(super::Host::Drawer) || self.compose_folder().is_some() {
            return None;
        }
        let draft = self.draft(self.compose_line()?);
        let starts = self.compose_plan().is_some_and(|plan| plan.starts);
        let named = draft.repo.as_deref().and_then(|name| self.repo_named(name));
        let place = match named {
            Some(path) => path,
            None => match self.shared_place() {
                (Some(repo), None) => repo,
                _ => return None,
            },
        };
        let repo = self.repos.iter().find(|repo| paths(repo).any(|path| path == place)).filter(|_| starts)?;
        let taken = |branch: &str| {
            let dir = crate::repos::worktree_dir(&root, &repo.path, &self.repos, branch);
            let branches = repo.checkouts().into_iter().map(|checkout| checkout.branch);
            branches.into_iter().any(|b| b == branch)
                || self.repos.iter().any(|r| paths(r).any(|path| path == dir))
                || self.making.iter().any(|making| making.asked == dir)
        };
        let branch = crate::repos::branch_for(&draft.title, taken);
        Some(NewWorktree { repo: repo.path.clone(), branch, root })
    }

    /// The box's choice of where the task works, while it offers one (`box_worktree`).
    pub fn compose_worktree(&self) -> Option<WorktreeChoice> {
        let NewWorktree { repo, branch, .. } = self.box_worktree()?;
        let repo = self.repos.iter().find(|r| r.path == repo).map_or(repo, |r| r.name.clone());
        Some(WorktreeChoice { new: self.compose_worktree, repo, branch })
    }

    /// The user's ↵ with a new worktree chosen: the task is in its workspace and its
    /// conversation in front, as any started there; git makes the worktree (`Effect::AddWorktree`)
    /// and its agent starts there once it is (`worktree_made`). A saved agent: as without.
    pub(crate) fn start_in_worktree(&mut self, id: TaskId, new: NewWorktree, effects: &mut Vec<Effect>) {
        let subject = super::Subject::Task(id);
        if self.agent_start(&subject).is_none() {
            return self.start(id, true, effects);
        }
        self.join_space(id);
        let NewWorktree { repo, branch, root } = new;
        let path = crate::repos::worktree_dir(&root, &repo, &self.repos, &branch);
        self.making.push(Making { task: id, asked: path.clone(), branch: branch.clone(), bug: false });
        effects.push(Effect::AddWorktree { repo, branch, path, root, fresh: true });
        self.talk_to(subject, true, effects);
    }

    /// A new task's worktree is made at `path`: the task works there (and its workspace's
    /// default with it, if it is its one task), and its agent starts, unless it went or runs.
    pub(crate) fn worktree_made(&mut self, id: TaskId, path: String, effects: &mut Vec<Effect>) {
        let Some(task) = self.tasks.live(id) else { return };
        let (was, brief) = (task.dir().cloned(), task.materials_dir().map(str::to_string));
        if let Some(was) = was.as_deref().and_then(|dir| self.checkout_at(dir)).map(String::from) {
            self.default_follows(id, &was, &path);
        }
        if let Some(task) = self.tasks.list.iter_mut().find(|task| task.id == id) {
            (task.repo, task.folder) = (Some(path), None);
        }
        let subject = super::Subject::Task(id);
        // Nor one the user marked done or put away while git made it.
        let busy = self.talk_live(&subject) || self.starting.iter().any(|(of, _)| *of == subject) || self.put_away(id);
        if let Some(start) = self.agent_start(&subject).filter(|_| !busy) {
            self.launch(subject, start, brief, effects);
        }
    }

    /// A while after a move into a folder not listed yet: the list is read again, until it is
    /// there or the wait is over (then only a read for another reason finds it).
    pub(crate) fn retry_moves(&mut self, effects: &mut Vec<Effect>) {
        let now = self.now;
        let waiting = |moving: &&mut Moving| now.saturating_sub(moving.since) <= MOVE_WAIT_MS;
        let due =
            self.moving.iter_mut().filter(waiting).filter(|moving| now.saturating_sub(moving.asked) >= MOVE_RETRY_MS);
        let mut ask = false;
        for moving in due {
            moving.asked = now;
            ask = true;
        }
        if ask {
            effects.push(Effect::ReadRepos);
        }
    }

    /// The kept workspace of a space: its own, or the one bound to its task.
    pub(crate) fn space_book(&self, of: &Owner) -> Option<&Workbook> {
        let id = match of {
            Owner::Workbook { id } => Some(*id),
            Owner::Task { id } => self.workbooks.bound_to(*id),
            Owner::Home | Owner::Session { .. } | Owner::Decisions => None,
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
        // A task not done first; a done one's workspace opens with none put in front.
        let task = working.iter().max_by_key(|task| (task.undone(), rank(&self.task_space(task.id))))?;
        Some((self.task_space(task.id), Some(task.id).filter(|_| task.undone())))
    }

    /// A Repos row's click: the workspace of that checkout (`repo_space`), else, the user's, a
    /// new kept workspace for it (named by `checkout_title`, the checkout its default) with a
    /// terminal there, selected. A client's opens a terminal as `OpenRepo` does.
    pub(crate) fn repo_click(&mut self, path: String, axis: Axis, user: bool, effects: &mut Vec<Effect>) {
        // Only the user's own step changes the front: git's answer to their + opens as a click, front aside.
        let front = self.by_user;
        match self.repo_space(&path, user) {
            Some((Owner::Workbook { id }, _)) if !self.spaces.iter().any(|s| s.of == Owner::Workbook { id }) => {
                self.workbook_open(id, None, self.home.clone(), effects)
            }
            Some((of, task)) => {
                self.open_space(of, effects);
                if let Some(task) = task.filter(|_| front) {
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
        let after = self.spaces.iter().position(|space| space.of == *self.shown_space() && space.of != Owner::Home);
        self.spaces.insert(after.map_or(self.spaces.len(), |at| at + 1), Space::new(of.clone()));
        self.list = None;
        self.open_space(of, effects);
        self.open_terminal_at(axis, Some(path), effects);
    }

    /// The worktree a task's row and rail item name while its workspace's tasks work in more
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

impl Model {
    /// The listed linked worktree at `path`, and its repository. Never a repository's own folder.
    pub fn linked_worktree(&self, path: &str) -> Option<(&crate::repos::Repo, &crate::repos::Worktree)> {
        if self.repos.iter().any(|repo| repo.path == path) {
            return None;
        }
        self.repos.iter().find_map(|repo| Some((repo, repo.worktrees.iter().find(|w| w.path == path)?)))
    }

    /// True if a terminal is in `path` or below: its shell, the program in front of it, or the
    /// folder it started in.
    pub(crate) fn terminal_in(&self, path: &str) -> bool {
        let terminal = |pane: &&super::Pane| pane.kind == super::Kind::Terminal;
        let mut cwds = self.panes.iter().filter(terminal).flat_map(|pane| {
            let live = self.terminals.iter().filter(|t| t.pane == pane.id);
            let live: Vec<&str> = live.flat_map(|t| [t.cwd.as_deref(), t.shell.as_deref()]).flatten().collect();
            live.into_iter().chain(pane.cwd.as_deref())
        });
        cwds.any(|dir| sessions::below(dir, path))
    }

    /// Why the worktree at `path` is not deleted now: a terminal is in it, or a task works there
    /// (not done, or its agent runs), as Done keeps one (`KEPT_TASK`): an adopted session's agent
    /// may run with no pane of ottospace's.
    pub fn worktree_busy(&self, path: &str) -> Option<&'static str> {
        if self.terminal_in(path) {
            return Some(crate::repos::TERMINAL_IN);
        }
        let works = |t: &&crate::tasks::Task| {
            !t.dismissed && (t.state != crate::tasks::State::Done || self.agent_runs(&crate::talk::Subject::Task(t.id)))
        };
        let inside = |t: &crate::tasks::Task| self.task_worktree(t.id).is_some_and(|w| sessions::below(w, path));
        self.tasks.list.iter().filter(works).any(inside).then_some(crate::repos::TASK_IN)
    }

    /// The user may be asked to delete it: a listed linked worktree, nothing in it, not being
    /// deleted, nor removed by a Done.
    pub(crate) fn deletable(&self, path: &str) -> bool {
        let asked = self.deleting.iter().any(|d| d.path == path) || self.removing(path).is_some();
        self.linked_worktree(path).is_some() && self.worktree_busy(path).is_none() && !asked
    }

    /// Delete worktree…: the question first, No its default. In use: why not, and no question.
    pub(crate) fn worktree_delete_ask(&mut self, path: String, effects: &mut Vec<Effect>) {
        if let Some(why) = self.worktree_busy(&path) {
            return effects.push(Effect::Notice { text: why.into() });
        }
        if !self.deletable(&path) {
            return;
        }
        // The question is ottospace's own UI: a live window would cover it.
        self.leave(effects);
        self.confirm = Some(super::Confirm { delete: Some(path), ..super::Confirm::default() });
    }

    /// Yes to "Delete this worktree?": git removes it, without `--force`; its branch stays.
    pub(crate) fn worktree_delete(&mut self, path: String, effects: &mut Vec<Effect>) {
        if !self.deletable(&path) {
            return;
        }
        let Some((repo, worktree)) = self.linked_worktree(&path) else { return };
        let (repo, branch) = (repo.path.clone(), worktree.branch.clone().unwrap_or_else(|| "(no branch)".into()));
        self.deleting.push(Deleting { path: path.clone(), repo: repo.clone(), branch });
        effects.push(Effect::RemoveWorktree { task: None, repo, path });
    }

    /// What git did with the user's delete: gone (the list is read again; a workspace whose
    /// default it was falls back to its repository), or kept, and git's reason.
    pub(crate) fn worktree_deleted(&mut self, path: &str, kept: Option<String>, effects: &mut Vec<Effect>) {
        let Some(at) = self.deleting.iter().position(|d| d.path == path) else { return };
        let Deleting { repo, branch, .. } = self.deleting.remove(at);
        if let Some(why) = kept {
            return effects.push(Effect::Notice { text: why });
        }
        self.workbooks.default_gone(path, &repo);
        // A done task's own: removed, as its Done would (Undo adds it back).
        for task in self.tasks.list.iter_mut() {
            if let Some(made) = task.made.as_mut().filter(|made| made.path == path) {
                made.removed = true;
            }
        }
        effects.push(Effect::Told { text: format!("Deleted the worktree of {branch} · its branch is kept") });
        effects.push(Effect::ReadRepos);
    }
}
