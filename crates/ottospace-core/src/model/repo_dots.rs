//! The dot at the left of a Repos checkout row: what happens in that checkout. Grey while idle,
//! lit while an agent or a session works there, amber while a task there needs the user, and
//! one pulse when a commit lands. See `docs/ai/repos.md`, "The dot".

use std::collections::BTreeMap;

use serde::Serialize;

use super::Model;
use crate::activity::Pulse;
use crate::repos::Repo;
use crate::talk::Subject;
use crate::tasks::{Group, Task, TaskId};

/// How long a commit's pulse plays (ms): one ease out.
pub const PULSE_MS: u64 = 500;

/// What a checkout's dot says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Idle,
    Working,
    NeedsYou,
}

/// A checkout's dot at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub activity: Activity,
    /// The task it stands for: its colour. `None` while working: a session no task has.
    pub task: Option<TaskId>,
    /// How far a commit's pulse has played, eased: 0 to 1. `None`: none plays.
    pub pulse: Option<f32>,
}

/// A checkout's `HEAD` log as last read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Head {
    /// Its length (bytes): it grows by a line per commit.
    log: u64,
    /// When the last pulse started (ms since 1970).
    pulsed: Option<u64>,
    /// Pulses since it was first read.
    pub(crate) pulses: u32,
}

/// `now` into a pulse that started `at`: eased, 0 to 1; `None` before it or once it is over.
fn pulse(at: u64, now: u64) -> Option<f32> {
    let t = now.checked_sub(at).filter(|t| *t < PULSE_MS)?;
    Some(crate::seal::ease_out(t as f32 / PULSE_MS as f32))
}

impl Model {
    /// The dot of the checkout at `path`, at `now`. A task there that needs the user first (the
    /// Tasks section's order); else the task at work there that started last; else a session no
    /// task has at work there.
    pub fn checkout_dot(&self, path: &str, now: u64) -> Dot {
        let pulse = self.heads.get(path).and_then(|head| pulse(head.pulsed?, now));
        let (activity, task) = self.activity(&[path]);
        Dot { activity, task, pulse }
    }

    /// A folded repository's name: one dot for all its checkouts, by the rule of one (`activity`
    /// over them all); `None` while unfolded (its rows say it) or every checkout is idle. No pulse.
    pub fn repo_dot(&self, repo: &Repo) -> Option<Dot> {
        let checkouts = repo.checkouts();
        let paths: Vec<&str> = checkouts.iter().map(|c| c.path.as_str()).collect();
        let (activity, task) = self.activity(&paths);
        (self.repo_folded(repo) && activity != Activity::Idle).then_some(Dot { activity, task, pulse: None })
    }

    /// What happens in the checkouts at `paths`: a task there that needs the user, first by the
    /// Tasks section's order; else the task at work that started last; else a session no task has.
    fn activity(&self, paths: &[&str]) -> (Activity, Option<TaskId>) {
        let in_paths = |dir: Option<&str>| dir.is_some_and(|dir| paths.contains(&dir));
        let there: Vec<&Task> =
            self.tasks.list.iter().filter(|t| t.undone() && in_paths(self.task_worktree(t.id))).collect();
        let needs = there.iter().filter(|t| t.group() == Some(Group::NeedsYou)).min_by_key(|t| (t.priority, t.id));
        if let Some(task) = needs {
            return (Activity::NeedsYou, Some(task.id));
        }
        let working = there.iter().filter(|t| self.task_works(t)).max_by_key(|t| (t.started_at, t.created, t.id));
        if let Some(task) = working {
            return (Activity::Working, Some(task.id));
        }
        let session = self.at_work.iter().any(|s| in_paths(Some(&s.repo)) && !self.tasks.adopted(&s.runner, &s.id));
        (if session { Activity::Working } else { Activity::Idle }, None)
    }

    /// A task's agent works: its turn runs (the drawer's dial), or the session it follows is active.
    fn task_works(&self, task: &Task) -> bool {
        let turn = self.chat(&Subject::Task(task.id)).is_some_and(|chat| chat.transcript.pulse() == Pulse::Running);
        let link = task.session.as_ref();
        turn || link.is_some_and(|l| self.at_work.iter().any(|s| (&s.runner, &s.id) == (&l.runner, &l.id)))
    }

    /// The checkouts' `HEAD` logs as read `at` (`SetHeads`): one that grew since the last read
    /// pulses, unless `calm` (Reduce Motion). The first read of a checkout only records it.
    pub(crate) fn set_heads(&mut self, heads: Vec<(String, u64)>, at: u64, calm: bool) {
        for (path, log) in heads {
            if !self.repo_listed(&path) {
                continue;
            }
            match self.heads.get_mut(&path) {
                Some(head) if log > head.log && !calm => {
                    (head.pulsed, head.pulses) = (Some(at), head.pulses + 1);
                    head.log = log;
                }
                Some(head) => head.log = log,
                None => drop(self.heads.insert(path, Head { log, ..Head::default() })),
            }
        }
    }

    /// Only listed checkouts keep a read, and only while the Repos section is open: one removed
    /// mid-pulse stops; on its next opening a commit made while it was closed does not pulse.
    pub(crate) fn heads_listed(&mut self) {
        if self.sidebar != Some(super::Section::Repos) {
            return self.heads.clear();
        }
        let heads = std::mem::take(&mut self.heads);
        self.heads = heads.into_iter().filter(|(path, _)| self.repo_listed(path)).collect::<BTreeMap<_, _>>();
    }

    /// Pulses since each checkout was first read, in all.
    pub fn head_pulses(&self) -> u32 {
        self.heads.values().map(|head| head.pulses).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Action;
    use crate::repos::{Repo, Worktree};
    use crate::sessions::{Found, Link};
    use crate::tasks::State;

    const APP: &str = "/code/app";
    const WT: &str = "/code/app-2";

    fn listed() -> Model {
        let mut m = Model::default();
        let worktree = Worktree { name: "app-2".into(), path: WT.into(), branch: Some("fix".into()) };
        let repo =
            Repo { name: "app".into(), path: APP.into(), branch: Some("main".into()), worktrees: vec![worktree] };
        m.update(Action::SetRepos { repos: vec![repo] });
        m.update(Action::ToggleSection { section: crate::model::Section::Repos });
        m
    }

    fn task(m: &mut Model, id: TaskId, dir: &str, state: State) {
        assert_eq!(m.tasks.add(format!("t{id}"), 3, state), id);
        let task = m.tasks.list.last_mut().expect("added");
        (task.repo, task.started_at) = (Some(dir.into()), Some(u64::from(id)));
    }

    /// The sessions read with Repos open: (id, at work), each in `repo`.
    fn sessions(m: &mut Model, repo: &str, read: &[(&str, bool)]) {
        let found = |&(id, active): &(&str, bool)| Found {
            runner: "claude".into(),
            id: id.into(),
            cwd: repo.into(),
            repo: repo.into(),
            started: None,
            last_activity: None,
            active,
            station: None,
            label: None,
            title: None,
            model: None,
            held: false,
        };
        m.update(Action::SessionsAtWork { found: read.iter().map(found).collect() });
    }

    fn dot(m: &Model, path: &str) -> (Activity, Option<TaskId>) {
        let dot = m.checkout_dot(path, 0);
        (dot.activity, dot.task)
    }

    #[test]
    fn idle_checkouts_are_quiet_the_repositorys_own_folder_too() {
        let m = listed();
        assert_eq!(dot(&m, APP), (Activity::Idle, None));
        assert_eq!(dot(&m, WT), (Activity::Idle, None));
    }

    #[test]
    fn a_session_no_task_has_lights_its_checkout_while_active() {
        let mut m = listed();
        sessions(&mut m, WT, &[("s1", false)]);
        assert_eq!(dot(&m, WT), (Activity::Idle, None));
        sessions(&mut m, WT, &[("s1", true)]);
        assert_eq!(dot(&m, WT), (Activity::Working, None));
        // Read with Repos open: never listed in the Elsewhere group.
        assert_eq!(m.sessions_bar().total, 0);
        assert_eq!(dot(&m, APP), (Activity::Idle, None), "only its own checkout");
    }

    #[test]
    fn a_tasks_session_lights_it_in_the_tasks_colour() {
        let mut m = listed();
        task(&mut m, 1, WT, State::Running);
        assert_eq!(dot(&m, WT), (Activity::Idle, None), "running, but nothing at work");
        sessions(&mut m, WT, &[("s1", true)]);
        m.tasks.list[0].session = Some(Link { runner: "claude".into(), id: "s1".into(), state: None });
        assert_eq!(dot(&m, WT), (Activity::Working, Some(1)));
    }

    #[test]
    fn needs_you_wins_then_the_latest_at_work() {
        let mut m = listed();
        for (id, s) in [(1, "s1"), (2, "s2")] {
            task(&mut m, id, WT, State::Running);
            m.tasks.list[id as usize - 1].session = Some(Link { runner: "claude".into(), id: s.into(), state: None });
        }
        sessions(&mut m, WT, &[("s1", true), ("s2", true)]);
        assert_eq!(dot(&m, WT), (Activity::Working, Some(2)), "the one started last");
        task(&mut m, 3, WT, State::NeedsYou);
        task(&mut m, 4, WT, State::NeedsYou);
        m.tasks.list[3].priority = 1;
        assert_eq!(dot(&m, WT), (Activity::NeedsYou, Some(4)), "the first that needs you, by priority");
        m.tasks.list[3].state = State::Done;
        assert_eq!(dot(&m, WT), (Activity::NeedsYou, Some(3)));
        m.tasks.list[2].snoozed_until = Some(10);
        assert_eq!(dot(&m, WT), (Activity::Working, Some(2)), "a snoozed task does not need you now");
    }

    /// The repository's name's dot: (activity, task), `None` for none.
    fn head(m: &Model) -> Option<(Activity, Option<TaskId>)> {
        m.repo_dot(&m.repos[0]).map(|dot| (dot.activity, dot.task))
    }

    fn fold(m: &mut Model, fold: bool) {
        m.update(Action::RepoFold { path: APP.into(), fold: Some(fold) });
    }

    #[test]
    fn a_folded_repositorys_name_has_a_dot_only_while_a_checkout_is_not_idle() {
        let mut m = listed();
        sessions(&mut m, WT, &[("s1", true)]);
        assert_eq!(head(&m), None, "unfolded: its rows say it");
        fold(&mut m, true);
        assert_eq!(head(&m), Some((Activity::Working, None)), "a session no task has: the working colour");
        sessions(&mut m, WT, &[("s1", false)]);
        assert_eq!(head(&m), None, "all idle: no grey dot");
        // The repository's own folder counts too.
        sessions(&mut m, APP, &[("s2", true)]);
        assert_eq!(head(&m), Some((Activity::Working, None)));
        assert_eq!(m.repo_dot(&m.repos[0]).and_then(|dot| dot.pulse), None, "never a pulse");
    }

    #[test]
    fn a_folded_repositorys_dot_takes_the_task_the_rows_would_across_its_checkouts() {
        let mut m = listed();
        for (id, dir, s) in [(1, APP, "s1"), (2, WT, "s2")] {
            task(&mut m, id, dir, State::Running);
            m.tasks.list[id as usize - 1].session = Some(Link { runner: "claude".into(), id: s.into(), state: None });
        }
        sessions(&mut m, WT, &[("s1", true), ("s2", true)]);
        // Folded while busy: held folded.
        fold(&mut m, true);
        assert_eq!(head(&m), Some((Activity::Working, Some(2))), "the one started last, in either checkout");
        task(&mut m, 3, APP, State::NeedsYou);
        task(&mut m, 4, WT, State::NeedsYou);
        m.tasks.list[3].priority = 1;
        assert_eq!(head(&m), Some((Activity::NeedsYou, Some(4))), "needs you wins, first by the Tasks order");
        assert_eq!(dot(&m, APP), (Activity::NeedsYou, Some(3)), "each row keeps its own");
        fold(&mut m, false);
        assert_eq!(head(&m), None);
    }

    #[test]
    fn a_commit_pulses_once_from_the_time_passed_in() {
        let mut m = listed();
        m.update(Action::SetHeads { heads: vec![(WT.into(), 100)], at: 1_000, calm: false });
        assert_eq!(m.checkout_dot(WT, 1_000).pulse, None, "the first read only records");
        m.update(Action::SetHeads { heads: vec![(WT.into(), 100)], at: 2_000, calm: false });
        assert_eq!(m.head_pulses(), 0, "no change, no pulse");
        m.update(Action::SetHeads { heads: vec![(WT.into(), 180)], at: 3_000, calm: false });
        assert_eq!(m.checkout_dot(WT, 3_000).pulse, Some(0.));
        let mid = m.checkout_dot(WT, 3_250).pulse.expect("playing");
        assert!(mid > 0.5 && mid < 1., "eased out: {mid}");
        assert_eq!(m.checkout_dot(WT, 3_000 + PULSE_MS).pulse, None, "over, settled");
        assert_eq!(m.checkout_dot(WT, 2_999).pulse, None);
        assert_eq!(m.checkout_dot(APP, 3_100).pulse, None, "only its checkout");
        assert_eq!(m.head_pulses(), 1);
    }

    #[test]
    fn under_reduce_motion_a_commit_does_not_pulse() {
        let mut m = listed();
        m.update(Action::SetHeads { heads: vec![(WT.into(), 100)], at: 1_000, calm: true });
        m.update(Action::SetHeads { heads: vec![(WT.into(), 180)], at: 2_000, calm: true });
        assert_eq!((m.checkout_dot(WT, 2_000).pulse, m.head_pulses()), (None, 0));
        // Nor later, for the commit it was calm for.
        m.update(Action::SetHeads { heads: vec![(WT.into(), 180)], at: 3_000, calm: false });
        assert_eq!(m.head_pulses(), 0);
    }

    #[test]
    fn a_checkout_removed_mid_pulse_keeps_no_dot() {
        let mut m = listed();
        m.update(Action::SetHeads { heads: vec![(WT.into(), 100), ("/elsewhere".into(), 5)], at: 1_000, calm: false });
        assert!(!m.heads.contains_key("/elsewhere"), "not listed: not kept");
        m.update(Action::SetHeads { heads: vec![(WT.into(), 180)], at: 2_000, calm: false });
        let only_app = Repo { name: "app".into(), path: APP.into(), branch: None, worktrees: Vec::new() };
        m.update(Action::SetRepos { repos: vec![only_app.clone()] });
        assert!(m.heads.get(WT).is_none());
        // Listed again with a longer log: read afresh, no pulse.
        m.update(Action::SetRepos { repos: listed().repos });
        m.update(Action::SetHeads { heads: vec![(WT.into(), 400)], at: 2_100, calm: false });
        assert_eq!(m.checkout_dot(WT, 2_100).pulse, None);
    }

    #[test]
    fn a_commit_made_while_repos_was_closed_does_not_pulse_when_it_opens() {
        let mut m = listed();
        m.update(Action::SetHeads { heads: vec![(WT.into(), 100)], at: 1_000, calm: false });
        m.update(Action::ToggleSection { section: crate::model::Section::Repos });
        assert!(m.heads.is_empty(), "closed: no read kept");
        m.update(Action::ToggleSection { section: crate::model::Section::Repos });
        m.update(Action::SetHeads { heads: vec![(WT.into(), 180)], at: 9_000, calm: false });
        assert_eq!((m.checkout_dot(WT, 9_000).pulse, m.head_pulses()), (None, 0));
    }
}
