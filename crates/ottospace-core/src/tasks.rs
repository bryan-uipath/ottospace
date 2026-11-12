//! Tasks as an inbox: what needs the user first. See `docs/ai/1-initial-brainstorm/tasks-inbox.md`.
//! Time comes in with the actions; nothing here reads a clock.

use serde::{Deserialize, Serialize};

pub type TaskId = u32;

/// The highest id a kept task may have. A file that claims more is not a task of ours, and
/// counting on from it would run out of numbers.
pub const MAX_ID: TaskId = 1_000_000_000;

/// The longest title, in characters. It keeps a task's file small enough to read back.
pub const MAX_TITLE: usize = 500;

/// Where a task is in its life. See `quick-capture.md` for the full lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Captured,
    Triaged,
    Running,
    NeedsYou,
    Review,
    Stale,
    Parked,
    Done,
    Cleaned,
}

/// A heading of the inbox, in the order shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    NeedsYou,
    Running,
    Captured,
    OnHold,
    Done,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::NeedsYou => "Needs you",
            Group::Running => "Running",
            Group::Captured => "Captured",
            Group::OnHold => "On hold",
            Group::Done => "Done",
        }
    }
}

/// Also the shape of a task's file on disk; fields a hand-written file leaves out take a default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub state: State,
    /// 1 is most urgent. Shown as P1, P2, ...
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// Hidden from the inbox until this time (milliseconds since 1970).
    #[serde(default)]
    pub snoozed_until: Option<u64>,
    /// Taken out of the inbox by the user. The task itself goes on.
    #[serde(default)]
    pub dismissed: bool,
    /// The project this task serves, from a `#tag`. Many tasks serve none.
    #[serde(default)]
    pub project: Option<String>,
    /// The path of the repository the work is in, from a `>name`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Who works it, from an `@name`. Stored and shown; nothing acts on it yet.
    #[serde(default)]
    pub agent: Option<String>,
    /// Paths the task is about, from `/path` words, as typed.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Where each of `paths` was found when the task was filed, as an absolute path, in the same
    /// order. Empty: not known (the paths were given outright, or the task is from before this
    /// was kept). Not checked since: validate before anything acts on it.
    #[serde(default)]
    pub resolved: Vec<String>,
    /// Skills to load for it, from `$name` words.
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Who works a task, the paths it is about and the skills it loads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assign {
    pub agent: Option<String>,
    pub paths: Vec<String>,
    /// Where each path was found, in the same order; or empty.
    pub resolved: Vec<String>,
    pub skills: Vec<String>,
}

/// The longest resolved path kept.
pub const MAX_RESOLVED: usize = 1024;

/// The longest agent, path or skill name kept, and the most paths or skills on one task.
pub const MAX_TAG: usize = 200;
pub const MAX_TAGS: usize = 20;

fn default_priority() -> u8 {
    3
}

impl State {
    /// The inbox heading for a task in this state. `None`: cleaned tasks are not listed.
    pub fn group(self) -> Option<Group> {
        match self {
            State::NeedsYou | State::Stale => Some(Group::NeedsYou),
            // A PR in review needs nothing from the user until checks or comments say so
            // (not read yet), so it sits with the running tasks.
            State::Running | State::Review => Some(Group::Running),
            State::Captured | State::Triaged => Some(Group::Captured),
            State::Parked => Some(Group::OnHold),
            State::Done => Some(Group::Done),
            State::Cleaned => None,
        }
    }
}

impl Task {
    /// The inbox heading for this task. `None`: not listed (cleaned, snoozed or dismissed).
    pub fn group(&self) -> Option<Group> {
        self.state.group().filter(|_| !self.dismissed && self.snoozed_until.is_none())
    }
}

/// What `undo` puts back: only the field the user's move changed, so a state an agent reported
/// since then is not lost.
#[derive(Debug)]
enum Undo {
    Snooze { id: TaskId, before: Option<u64> },
    Hold { id: TaskId, before: State },
    Dismiss { id: TaskId },
}

/// The task list, and the one step that `undo` takes back.
#[derive(Debug, Default, Serialize)]
pub struct Tasks {
    pub list: Vec<Task>,
    /// The last snooze, hold or dismiss, until it is taken back or the task's state changes.
    #[serde(skip)]
    undo: Option<Undo>,
    #[serde(skip)]
    next_id: TaskId,
}

impl Tasks {
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.list.iter().find(|t| t.id == id)
    }

    /// The inbox: listed tasks by group, then most urgent first, then oldest first.
    pub fn inbox(&self) -> Vec<(Group, &Task)> {
        let mut rows: Vec<(Group, &Task)> = self.list.iter().filter_map(|t| Some((t.group()?, t))).collect();
        // Ids rise with age, so the id is the "oldest first" tie-break.
        rows.sort_by_key(|(group, task)| (*group, task.priority, task.id));
        rows
    }

    /// The task "pop next" opens: the first one under Needs you.
    pub fn next(&self) -> Option<&Task> {
        self.inbox().into_iter().find(|(group, _)| *group == Group::NeedsYou).map(|(_, task)| task)
    }

    pub fn needs_you(&self) -> usize {
        self.list.iter().filter(|t| t.group() == Some(Group::NeedsYou)).count()
    }

    /// Starts from the tasks kept on disk. Of two tasks with one id the first is kept. New ids go on after the highest, and after
    /// `floor`: the highest id among files that could not be read, so none of them is overwritten.
    pub fn restore(&mut self, mut tasks: Vec<Task>, floor: TaskId) {
        tasks.retain(|task| (1..=MAX_ID).contains(&task.id));
        tasks.sort_by_key(|task| task.id);
        tasks.dedup_by_key(|task| task.id);
        for task in &mut tasks {
            task.priority = task.priority.clamp(1, 9);
        }
        self.next_id = tasks.last().map_or(0, |task| task.id).max(floor.min(MAX_ID));
        (self.list, self.undo) = (tasks, None);
    }

    pub fn add(&mut self, title: String, priority: u8, state: State) -> TaskId {
        self.next_id += 1;
        let priority = priority.clamp(1, 9);
        let (id, snoozed_until, dismissed, project, repo) = (self.next_id, None, false, None, None);
        let Assign { agent, paths, resolved, skills } = Assign::default();
        self.list.push(Task {
            id,
            title,
            state,
            priority,
            snoozed_until,
            dismissed,
            project,
            repo,
            agent,
            paths,
            resolved,
            skills,
        });
        self.next_id
    }

    /// Names the project and repository of a task just filed from the new-task box.
    pub fn tag(&mut self, id: TaskId, project: Option<String>, repo: Option<String>) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            (task.project, task.repo) = (project, repo);
        }
    }

    /// Names who works a task, the paths it is about and the skills it loads. Names too long to
    /// keep, and those past the twentieth, are dropped. Where the paths were found is kept only
    /// if it is known for every one of them.
    pub fn assign(&mut self, id: TaskId, assign: Assign) {
        let Assign { agent, paths, resolved, skills } = assign;
        let fits = |name: &String| !name.is_empty() && name.chars().count() <= MAX_TAG;
        let kept = |names: Vec<String>| names.into_iter().filter(fits).take(MAX_TAGS).collect();
        let found = |path: &String| path.starts_with('/') && path.chars().count() <= MAX_RESOLVED;
        let places: Vec<Option<String>> = if resolved.len() == paths.len() && resolved.iter().all(found) {
            resolved.into_iter().map(Some).collect()
        } else {
            paths.iter().map(|_| None).collect()
        };
        let pairs = paths.into_iter().zip(places).filter(|(path, _)| fits(path)).take(MAX_TAGS);
        let (paths, places): (Vec<String>, Vec<Option<String>>) = pairs.unzip();
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            (task.agent, task.paths, task.skills) = (agent.filter(fits), paths, kept(skills));
            task.resolved = places.into_iter().flatten().collect();
        }
    }

    /// Every project named by a task, once each, by name.
    pub fn projects(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.list.iter().filter_map(|t| t.project.as_deref()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// A task that moves under Needs you comes back into the inbox, snoozed or dismissed or not.
    /// A report of the state it already has is not news: it stays hidden.
    pub fn set_state(&mut self, id: TaskId, state: State) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            let needs_you = |state: State| state.group() == Some(Group::NeedsYou);
            if needs_you(state) && !needs_you(task.state) {
                (task.snoozed_until, task.dismissed) = (None, false);
            }
            task.state = state;
            // The report is newer than the user's last move on this task: undo must not hide it.
            let undone = self.undo.as_ref().map(|u| match u {
                Undo::Snooze { id, .. } | Undo::Hold { id, .. } | Undo::Dismiss { id } => *id,
            });
            if undone == Some(id) {
                self.undo = None;
            }
        }
    }

    pub fn snooze(&mut self, id: TaskId, until: u64) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            self.undo = Some(Undo::Snooze { id, before: task.snoozed_until.replace(until) });
        }
    }

    /// Pauses a task. Not one that is finished: there is nothing left to pause.
    pub fn hold(&mut self, id: TaskId) {
        let open = |task: &&mut Task| !matches!(task.state, State::Done | State::Cleaned);
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id).filter(open) {
            self.undo = Some(Undo::Hold { id, before: std::mem::replace(&mut task.state, State::Parked) });
        }
    }

    pub fn dismiss(&mut self, id: TaskId) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id).filter(|t| !t.dismissed) {
            task.dismissed = true;
            self.undo = Some(Undo::Dismiss { id });
        }
    }

    /// Takes back the last snooze, hold or dismiss. One step only.
    pub fn undo(&mut self) {
        let Some(undo) = self.undo.take() else { return };
        let (Undo::Snooze { id, .. } | Undo::Hold { id, .. } | Undo::Dismiss { id }) = undo;
        let Some(task) = self.list.iter_mut().find(|t| t.id == id) else { return };
        match undo {
            Undo::Snooze { before, .. } => task.snoozed_until = before,
            Undo::Hold { before, .. } => task.state = before,
            Undo::Dismiss { .. } => task.dismissed = false,
        }
    }

    /// True if a snoozed task's time has come, so `tick` would change the inbox.
    pub fn due(&self, now: u64) -> bool {
        self.list.iter().any(|t| t.snoozed_until.is_some_and(|until| until <= now))
    }

    /// Snoozed tasks whose time has come are listed again.
    pub fn tick(&mut self, now: u64) {
        for task in &mut self.list {
            if task.snoozed_until.is_some_and(|until| until <= now) {
                task.snoozed_until = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(tasks: &Tasks) -> Vec<(Group, &str)> {
        tasks.inbox().into_iter().map(|(group, task)| (group, task.title.as_str())).collect()
    }

    #[test]
    fn inbox_is_grouped_then_by_priority_then_oldest_first() {
        let mut tasks = Tasks::default();
        tasks.add("build".into(), 2, State::Running);
        tasks.add("old question".into(), 2, State::NeedsYou);
        tasks.add("idea".into(), 3, State::Captured);
        tasks.add("urgent question".into(), 1, State::NeedsYou);
        tasks.add("new question".into(), 2, State::NeedsYou);
        tasks.add("open pr".into(), 1, State::Review);
        tasks.add("paused".into(), 1, State::Parked);
        tasks.add("forgotten".into(), 3, State::Stale);
        tasks.add("merged".into(), 1, State::Done);
        tasks.add("gone".into(), 1, State::Cleaned);
        assert_eq!(
            titles(&tasks),
            [
                (Group::NeedsYou, "urgent question"),
                (Group::NeedsYou, "old question"),
                (Group::NeedsYou, "new question"),
                (Group::NeedsYou, "forgotten"),
                (Group::Running, "open pr"),
                (Group::Running, "build"),
                (Group::Captured, "idea"),
                (Group::OnHold, "paused"),
                (Group::Done, "merged"),
            ]
        );
        assert_eq!((tasks.needs_you(), tasks.next().map(|t| t.title.as_str())), (4, Some("urgent question")));
    }

    #[test]
    fn a_snoozed_task_is_hidden_until_its_time() {
        let mut tasks = Tasks::default();
        let a = tasks.add("a".into(), 1, State::NeedsYou);
        tasks.add("b".into(), 2, State::NeedsYou);
        tasks.snooze(a, 1_000);
        assert_eq!(tasks.next().map(|t| t.title.as_str()), Some("b"), "pop skips the snoozed task");
        tasks.tick(999);
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "b")]);
        tasks.tick(1_000);
        assert_eq!(tasks.next().map(|t| t.title.as_str()), Some("a"));
    }

    #[test]
    fn needing_the_user_again_brings_a_hidden_task_back() {
        let mut tasks = Tasks::default();
        let a = tasks.add("a".into(), 1, State::Running);
        let b = tasks.add("b".into(), 1, State::Running);
        tasks.snooze(a, 1_000);
        tasks.dismiss(b);
        assert!(tasks.inbox().is_empty());
        // Still running: nothing new, so both stay hidden.
        tasks.set_state(a, State::Running);
        assert!(tasks.inbox().is_empty());
        tasks.set_state(a, State::NeedsYou);
        tasks.set_state(b, State::NeedsYou);
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "a"), (Group::NeedsYou, "b")]);
    }

    #[test]
    fn hold_and_dismiss_move_a_task_and_undo_takes_one_step_back() {
        let mut tasks = Tasks::default();
        let a = tasks.add("a".into(), 1, State::NeedsYou);
        tasks.hold(a);
        assert_eq!(titles(&tasks), [(Group::OnHold, "a")]);
        tasks.undo();
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "a")]);
        tasks.dismiss(a);
        assert_eq!((tasks.inbox().len(), tasks.get(a).map(|t| t.state)), (0, Some(State::NeedsYou)));
        tasks.undo();
        tasks.undo(); // nothing left to take back
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "a")]);
        // An unknown id changes nothing, and does not replace what undo holds.
        tasks.snooze(a, 5);
        tasks.hold(99);
        tasks.undo();
        assert_eq!(tasks.get(a).and_then(|t| t.snoozed_until), None);
    }

    #[test]
    fn undo_does_not_take_back_what_an_agent_reported_since() {
        let mut tasks = Tasks::default();
        let a = tasks.add("a".into(), 1, State::Running);
        tasks.dismiss(a);
        tasks.set_state(a, State::NeedsYou);
        tasks.undo();
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "a")], "the report stands");
        tasks.hold(a);
        tasks.set_state(a, State::Done);
        tasks.undo();
        assert_eq!(tasks.get(a).map(|t| t.state), Some(State::Done));
        // Undo puts back only what the move changed.
        let b = tasks.add("b".into(), 1, State::Running);
        tasks.snooze(b, 10);
        tasks.snooze(b, 20);
        tasks.undo();
        assert_eq!(tasks.get(b).and_then(|t| t.snoozed_until), Some(10));
    }

    #[test]
    fn a_finished_task_cannot_be_put_on_hold() {
        let mut tasks = Tasks::default();
        let (done, cleaned) = (tasks.add("a".into(), 1, State::Done), tasks.add("b".into(), 1, State::Cleaned));
        tasks.hold(done);
        tasks.hold(cleaned);
        assert_eq!(titles(&tasks), [(Group::Done, "a")]);
    }

    #[test]
    fn a_task_that_goes_stale_comes_back_but_a_repeated_report_does_not() {
        let mut tasks = Tasks::default();
        let a = tasks.add("a".into(), 1, State::Parked);
        tasks.dismiss(a);
        tasks.set_state(a, State::Stale);
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "a")]);
        // Snoozed while it waits: the same report again is not news.
        tasks.snooze(a, 1_000);
        tasks.set_state(a, State::NeedsYou);
        assert!(tasks.inbox().is_empty() && tasks.due(1_000) && !tasks.due(999));
    }

    #[test]
    fn restored_tasks_keep_their_place_and_new_ids_go_on() {
        let task = |id, title: &str| Task {
            id,
            title: title.into(),
            state: State::NeedsYou,
            priority: 2,
            snoozed_until: None,
            dismissed: false,
            project: None,
            repo: None,
            agent: None,
            paths: Vec::new(),
            resolved: Vec::new(),
            skills: Vec::new(),
        };
        let mut tasks = Tasks::default();
        tasks.restore(vec![task(7, "late"), task(2, "early"), task(7, "twin")], 0);
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "early"), (Group::NeedsYou, "late")]);
        assert_eq!(tasks.add("new".into(), 3, State::Captured), 8);
        // A file that could not be read keeps its number: new ids go on after it too.
        tasks.restore(vec![task(2, "early"), task(0, "no id"), task(u32::MAX, "not ours")], 40);
        assert_eq!((tasks.list.len(), tasks.add("after".into(), 3, State::Captured)), (1, 41));
        tasks.restore(Vec::new(), u32::MAX);
        assert_eq!(tasks.add("capped".into(), 3, State::Captured), MAX_ID + 1);
        // A file written by hand may leave fields out.
        let read: Task = serde_json::from_str(r#"{"id":1,"title":"by hand","state":"captured"}"#).unwrap();
        assert_eq!((read.priority, read.snoozed_until, read.dismissed), (3, None, false));
        assert_eq!((read.agent, read.paths.len(), read.skills.len()), (None, 0, 0));
    }

    #[test]
    fn a_task_keeps_its_agent_paths_and_skills_within_limits() {
        let mut tasks = Tasks::default();
        let a = tasks.add("a".into(), 1, State::Captured);
        let many: Vec<String> = (0..30).map(|n| format!("/p{n}")).collect();
        let skills = vec!["deploy".into(), String::new(), "x".repeat(MAX_TAG + 1)];
        let assign = Assign { agent: Some("claude".into()), paths: many, resolved: Vec::new(), skills };
        tasks.assign(a, assign);
        let task = tasks.get(a).unwrap();
        assert_eq!(
            (task.agent.as_deref(), task.paths.len(), &task.skills, task.resolved.len()),
            (Some("claude"), MAX_TAGS, &vec!["deploy".to_string()], 0)
        );
        // Where the paths were found stays in step with them: a path dropped takes its place with it.
        let list = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        let paths = vec!["/a".to_string(), "x".repeat(MAX_TAG + 1), "~/c".into()];
        let resolved = list(&["/code/app/a", "/long", "/Users/me/c"]);
        tasks.assign(a, Assign { paths: paths.clone(), resolved, ..Assign::default() });
        let task = tasks.get(a).unwrap();
        assert_eq!((&task.paths, &task.resolved), (&list(&["/a", "~/c"]), &list(&["/code/app/a", "/Users/me/c"])));
        // Known for some only: not kept, rather than out of step.
        tasks.assign(a, Assign { paths, resolved: list(&["/code/app/a"]), ..Assign::default() });
        assert_eq!((tasks.get(a).unwrap().paths.len(), tasks.get(a).unwrap().resolved.len()), (2, 0));
    }

    #[test]
    fn priority_stays_in_range() {
        let mut tasks = Tasks::default();
        let (low, high) = (tasks.add("a".into(), 0, State::Captured), tasks.add("b".into(), 200, State::Captured));
        assert_eq!((tasks.get(low).map(|t| t.priority), tasks.get(high).map(|t| t.priority)), (Some(1), Some(9)));
    }
}
