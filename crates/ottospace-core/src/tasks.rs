//! Tasks as an inbox: what needs the user first. See `docs/ai/1-initial-brainstorm/tasks-inbox.md`.
//! Time comes in with the actions; nothing here reads a clock.

use serde::{Deserialize, Serialize};

pub type TaskId = u32;

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
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub state: State,
    /// 1 is most urgent. Shown as P1, P2, ...
    pub priority: u8,
    /// Hidden from the inbox until this time (milliseconds since 1970).
    pub snoozed_until: Option<u64>,
    /// Taken out of the inbox by the user. The task itself goes on.
    pub dismissed: bool,
    /// The `id` of the tab that is this task's workspace, if it has one.
    pub tab: Option<u32>,
}

impl Task {
    /// The inbox heading for this task. `None`: not listed (cleaned, snoozed or dismissed).
    pub fn group(&self) -> Option<Group> {
        if self.dismissed || self.snoozed_until.is_some() {
            return None;
        }
        match self.state {
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

/// The task list, and the one step that `undo` takes back.
#[derive(Debug, Default, Serialize)]
pub struct Tasks {
    pub list: Vec<Task>,
    /// The task as it was before the last snooze, hold or dismiss.
    #[serde(skip)]
    undo: Option<Task>,
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
        // Ids rise with age, so the id is the "oldest first" tie-break. Groups never compare unordered.
        rows.sort_by(|(ga, a), (gb, b)| {
            ga.partial_cmp(gb).unwrap_or(std::cmp::Ordering::Equal).then((a.priority, a.id).cmp(&(b.priority, b.id)))
        });
        rows
    }

    /// The task "pop next" opens: the first one under Needs you.
    pub fn next(&self) -> Option<&Task> {
        self.inbox().into_iter().find(|(group, _)| *group == Group::NeedsYou).map(|(_, task)| task)
    }

    pub fn needs_you(&self) -> usize {
        self.list.iter().filter(|t| t.group() == Some(Group::NeedsYou)).count()
    }

    pub fn add(&mut self, title: String, priority: u8, state: State, tab: Option<u32>) -> TaskId {
        self.next_id += 1;
        let priority = priority.clamp(1, 9);
        self.list.push(Task { id: self.next_id, title, state, priority, snoozed_until: None, dismissed: false, tab });
        self.next_id
    }

    /// A task that starts to need the user comes back into the inbox, snoozed or dismissed or not.
    pub fn set_state(&mut self, id: TaskId, state: State) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            if state == State::NeedsYou && task.state != State::NeedsYou {
                (task.snoozed_until, task.dismissed) = (None, false);
            }
            task.state = state;
        }
    }

    pub fn snooze(&mut self, id: TaskId, until: u64) {
        self.change(id, |task| task.snoozed_until = Some(until));
    }

    pub fn hold(&mut self, id: TaskId) {
        self.change(id, |task| task.state = State::Parked);
    }

    pub fn dismiss(&mut self, id: TaskId) {
        self.change(id, |task| task.dismissed = true);
    }

    /// Takes back the last snooze, hold or dismiss. One step only.
    pub fn undo(&mut self) {
        if let Some(before) = self.undo.take()
            && let Some(task) = self.list.iter_mut().find(|t| t.id == before.id)
        {
            *task = before;
        }
    }

    /// Snoozed tasks whose time has come are listed again.
    pub fn tick(&mut self, now: u64) {
        for task in &mut self.list {
            if task.snoozed_until.is_some_and(|until| until <= now) {
                task.snoozed_until = None;
            }
        }
    }

    fn change(&mut self, id: TaskId, edit: impl FnOnce(&mut Task)) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            self.undo = Some(task.clone());
            edit(task);
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
        tasks.add("build".into(), 2, State::Running, None);
        tasks.add("old question".into(), 2, State::NeedsYou, None);
        tasks.add("idea".into(), 3, State::Captured, None);
        tasks.add("urgent question".into(), 1, State::NeedsYou, None);
        tasks.add("new question".into(), 2, State::NeedsYou, None);
        tasks.add("open pr".into(), 1, State::Review, None);
        tasks.add("paused".into(), 1, State::Parked, None);
        tasks.add("forgotten".into(), 3, State::Stale, None);
        tasks.add("merged".into(), 1, State::Done, None);
        tasks.add("gone".into(), 1, State::Cleaned, None);
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
        let a = tasks.add("a".into(), 1, State::NeedsYou, None);
        tasks.add("b".into(), 2, State::NeedsYou, None);
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
        let a = tasks.add("a".into(), 1, State::Running, None);
        let b = tasks.add("b".into(), 1, State::Running, None);
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
        let a = tasks.add("a".into(), 1, State::NeedsYou, Some(4));
        tasks.hold(a);
        assert_eq!(titles(&tasks), [(Group::OnHold, "a")]);
        tasks.undo();
        assert_eq!(titles(&tasks), [(Group::NeedsYou, "a")]);
        tasks.dismiss(a);
        assert_eq!((tasks.inbox().len(), tasks.get(a).map(|t| t.state)), (0, Some(State::NeedsYou)));
        tasks.undo();
        tasks.undo(); // nothing left to take back
        assert_eq!((titles(&tasks), tasks.get(a).and_then(|t| t.tab)), (vec![(Group::NeedsYou, "a")], Some(4)));
        // An unknown id changes nothing, and does not replace what undo holds.
        tasks.snooze(a, 5);
        tasks.hold(99);
        tasks.undo();
        assert_eq!(tasks.get(a).and_then(|t| t.snoozed_until), None);
    }

    #[test]
    fn priority_stays_in_range() {
        let mut tasks = Tasks::default();
        let (low, high) =
            (tasks.add("a".into(), 0, State::Captured, None), tasks.add("b".into(), 200, State::Captured, None));
        assert_eq!((tasks.get(low).map(|t| t.priority), tasks.get(high).map(|t| t.priority)), (Some(1), Some(9)));
    }
}
