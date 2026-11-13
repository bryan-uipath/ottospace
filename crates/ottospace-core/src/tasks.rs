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

/// A heading of the Tasks section's State view, in the order drawn: what needs the user, what
/// runs, what waits (captured or on hold), what is done. Last, the sessions that are no task's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heading {
    NeedsYou,
    Running,
    Later,
    Done,
    Sessions,
}

impl Heading {
    pub fn title(self) -> &'static str {
        match self {
            Heading::NeedsYou => Group::NeedsYou.title(),
            Heading::Running => Group::Running.title(),
            Heading::Later => "Later",
            Heading::Done => Group::Done.title(),
            Heading::Sessions => "Sessions",
        }
    }
}

impl From<Group> for Heading {
    fn from(group: Group) -> Self {
        match group {
            Group::NeedsYou => Heading::NeedsYou,
            Group::Running => Heading::Running,
            Group::Captured | Group::OnHold => Heading::Later,
            Group::Done => Heading::Done,
        }
    }
}

/// A row of the Tasks section, as drawn.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskRow {
    /// What its session does now (`shell · cargo test`), while it is at work, and its model in short.
    pub live: Option<String>,
    pub model: Option<String>,
    pub id: TaskId,
    /// The inbox group, whatever heading the row is under: its dot's colour.
    pub group: Group,
    /// Tabs and panes the task has open.
    pub tabs: usize,
    pub panes: usize,
    /// Its tabs are the ones on show.
    pub current: bool,
    /// The muted second line: `#canvas · >app · 2 tabs, 3 panes · 3h`.
    pub detail: String,
}

/// A heading of the Tasks section with its rows. A collapsed one keeps its count and no rows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskSection {
    pub heading: Heading,
    pub count: usize,
    pub collapsed: bool,
    pub rows: Vec<TaskRow>,
}

/// The listed tasks by project, as the Project view groups them: projects by name (whatever the
/// case of the letters), then the tasks of no project. In each: needs you, running, later, done;
/// within those, most urgent first, then oldest first. Every task of `inbox` once.
pub fn by_project<'a>(inbox: Vec<(Group, &'a Task)>) -> Vec<(Option<String>, Vec<(Group, &'a Task)>)> {
    let mut groups: Vec<(Option<String>, Vec<(Group, &Task)>)> = Vec::new();
    for (group, task) in inbox {
        match groups.iter_mut().find(|(project, _)| *project == task.project) {
            Some((_, tasks)) => tasks.push((group, task)),
            None => groups.push((task.project.clone(), vec![(group, task)])),
        }
    }
    let state = |group: Group| Heading::from(group);
    for (_, tasks) in &mut groups {
        tasks.sort_by_key(|(group, task)| (state(*group), task.priority, task.id));
    }
    let key = |project: &Option<String>| (project.is_none(), project.as_deref().map(str::to_lowercase));
    groups.sort_by(|a, b| key(&a.0).cmp(&key(&b.0)).then_with(|| a.0.cmp(&b.0)));
    groups
}

/// A project's tasks by state: the segments of its bar, in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Segments {
    pub needs_you: usize,
    pub running: usize,
    pub later: usize,
    pub done: usize,
}

impl Segments {
    pub fn of(tasks: &[(Group, &Task)]) -> Segments {
        let mut s = Segments::default();
        for (group, _) in tasks {
            match Heading::from(*group) {
                Heading::NeedsYou => s.needs_you += 1,
                Heading::Running => s.running += 1,
                Heading::Done => s.done += 1,
                Heading::Later | Heading::Sessions => s.later += 1,
            }
        }
        s
    }

    /// Each segment's share of the bar, by task count, in order. All 0 with no tasks.
    pub fn shares(&self) -> [f32; 4] {
        let counts = [self.needs_you, self.running, self.later, self.done];
        let total: usize = counts.iter().sum();
        counts.map(|n| if total == 0 { 0. } else { n as f32 / total as f32 })
    }

    /// The bar's tooltip: `1 needs you · 2 running · 0 later · 3 done`.
    pub fn tip(&self) -> String {
        let need = if self.needs_you == 1 { "needs" } else { "need" };
        format!("{} {need} you · {} running · {} later · {} done", self.needs_you, self.running, self.later, self.done)
    }
}

/// What the user just did to a task with the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Moved {
    Snoozed,
    Held,
    Dismissed,
    /// Not a task's: a mark was taken off. The Marks section offers it back.
    MarkRemoved,
    /// Not a task's either: the Apps section offers a pin, an unpin or a hidden suggestion back.
    AppPinned,
    AppUnpinned,
    AppHidden,
}

impl Moved {
    pub fn title(self) -> &'static str {
        match self {
            Moved::Snoozed => "Snoozed",
            Moved::Held => "On hold",
            Moved::Dismissed => "Dismissed",
            Moved::MarkRemoved => "Mark removed",
            Moved::AppPinned => "Pinned",
            Moved::AppUnpinned => "Unpinned",
            Moved::AppHidden => "Suggestion hidden",
        }
    }

    /// The Apps section's offer, not the Tasks section's.
    pub fn of_apps(self) -> bool {
        matches!(self, Moved::AppPinned | Moved::AppUnpinned | Moved::AppHidden)
    }
}

/// The line at the top of the Tasks section that offers to take a mouse move back: a click on a
/// row's button is easy to get wrong, and `u` is a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub moved: Moved,
    /// Shown until this time (milliseconds since 1970).
    pub until: u64,
}

/// How long a notice stays.
pub const NOTICE_MS: u64 = 6_000;

/// How long ago `then` was, in one unit: `now`, `5m`, `3h`, `2d`. Times are milliseconds.
pub fn age(then: u64, now: u64) -> String {
    let minutes = now.saturating_sub(then) / 60_000;
    match minutes {
        0 => "now".into(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
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
    /// When it was filed (milliseconds since 1970). `None`: filed before this was kept.
    #[serde(default)]
    pub created: Option<u64>,
    /// The session it was adopted from, if it was: that session is this task's.
    #[serde(default)]
    pub session: Option<crate::sessions::Link>,
    /// Nobody typed its title: it was filed from captures alone, and named by fixed words.
    /// A reader of its brief is told so, and not that the user said it.
    #[serde(default)]
    pub title_generated: bool,
    /// What was captured for it: each capture's file, as a path inside the materials folder,
    /// e.g. `7-k3f9a2c41b7d/1.json`. Its brief is `brief.md` beside them. The folder is named by
    /// the task's number and a word made up for it: numbers start again in another tasks folder,
    /// and a new task 7 must not be handed an old one's captures. Not in the tasks folder.
    #[serde(default)]
    pub materials: Vec<String>,
    /// What was open that the task points at, from `^name` words, as it was when the task was
    /// added. Stored and shown; nothing acts on it yet.
    #[serde(default)]
    pub refs: Vec<Ref>,
    /// When its agent first started, and when it was last done (milliseconds since 1970).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// What its agent's sessions did, one entry a session, the newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work: Vec<crate::progress::Work>,
    /// Keys this build does not know, written back as they were: a later build's (its tabs and
    /// layout, say) survive a write by this one.
    #[serde(flatten)]
    pub extra: crate::marks::Extra,
}

impl Task {
    /// The task as it is written to its file. A reference keeps what it is, its name, its id
    /// then and its app's name: not a window's title, a terminal's directory or program, or
    /// where an app is installed. The tasks folder may be shared; those stay in memory.
    pub fn for_file(&self) -> Task {
        let kept = |r: &Ref| Ref { title: None, cwd: None, program: None, bundle: None, ..r.clone() };
        Task { refs: self.refs.iter().map(kept).collect(), ..self.clone() }
    }
}

/// What a reference points at.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Tab,
    Pane,
    Mark,
    /// An installed app that was not running.
    App,
}

/// Something a task points at, as it was when the task was added. `title`, `cwd`, `program`
/// and `bundle` tell it apart while ottospace runs, and are not written to the task's file
/// (`Task::for_file`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ref {
    pub kind: RefKind,
    /// The name it was typed by: `Tab 1`, `flow-app`, `Sketchpad`.
    pub name: String,
    /// The tab's, pane's or mark's id then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// A tab's title, or an app pane's window title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The app of an app pane, or the app itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// A terminal pane's directory, and the program in front of its shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// Where an app that was not running is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
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

/// A task's folder of captures: `<id>-<word>`, the word 8 to 32 letters and digits.
pub fn materials_dir(id: TaskId, word: &str) -> Option<String> {
    let plain = (8..=32).contains(&word.len()) && word.bytes().all(|b| b.is_ascii_alphanumeric());
    plain.then(|| format!("{id}-{word}"))
}

/// The folder and the capture's number, if `file` is what a task names a capture by:
/// `<id>-<word>/<number>.json`, e.g. `7-k3f9a2c41b7d/1.json`. Nothing else is ever joined to
/// the materials folder.
pub fn material_file(id: TaskId, file: &str) -> Option<(&str, u32)> {
    let (dir, name) = file.split_once('/')?;
    let word = dir.strip_prefix(&format!("{id}-"))?;
    let number = name.strip_suffix(".json").filter(|n| n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()))?;
    materials_dir(id, word).map(|_| (dir, number.parse().ok())).and_then(|(dir, n)| Some((dir, n?)))
}

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
    /// The folder its captures are in, inside the materials folder. `None`: it has none.
    pub fn materials_dir(&self) -> Option<&str> {
        self.materials.iter().find_map(|file| material_file(self.id, file)).map(|(dir, _)| dir)
    }

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
            // The file is the user's to edit, or a notes repository's: a name there is not a path to follow.
            let id = task.id;
            let dir = task.materials.iter().find_map(|file| material_file(id, file)).map(|(dir, _)| dir.to_string());
            let kept = |file: &String| dir.is_some() && material_file(id, file).map(|(dir, _)| dir) == dir.as_deref();
            task.materials.retain(kept);
            task.materials.dedup();
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
            created: None,
            session: None,
            refs: Vec::new(),
            materials: Vec::new(),
            title_generated: false,
            started_at: None,
            finished_at: None,
            work: Vec::new(),
            extra: Default::default(),
        });
        self.next_id
    }

    /// Notes that nobody typed a task's title.
    pub fn untitled(&mut self, id: TaskId) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            task.title_generated = true;
        }
    }

    /// Notes what was captured for a task just filed.
    pub fn attach(&mut self, id: TaskId, materials: Vec<String>) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            task.materials = materials;
        }
    }

    /// Names the project and repository of a task just filed from the new-task box.
    pub fn tag(&mut self, id: TaskId, project: Option<String>, repo: Option<String>) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            (task.project, task.repo) = (project, repo);
        }
    }

    /// Sets what a task points at: at most `MAX_TAGS`, each name once.
    pub fn point(&mut self, id: TaskId, refs: Vec<Ref>) {
        let mut kept: Vec<Ref> = Vec::new();
        for reference in refs.into_iter().filter(|r| !r.name.is_empty() && r.name.chars().count() <= MAX_TAG) {
            if kept.len() < MAX_TAGS && kept.iter().all(|k| k.name != reference.name) {
                kept.push(reference);
            }
        }
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            task.refs = kept;
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

    /// Notes the session a task just filed was adopted from.
    pub fn link(&mut self, id: TaskId, session: crate::sessions::Link) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            task.session = Some(session);
        }
    }

    /// The task follows no session any more.
    pub fn unlink(&mut self, id: TaskId) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            task.session = None;
        }
    }

    /// Notes the state a task just took from the session it was adopted from.
    pub fn followed(&mut self, id: TaskId, state: State) {
        if let Some(link) = self.list.iter_mut().find(|t| t.id == id).and_then(|task| task.session.as_mut()) {
            link.state = Some(state);
        }
    }

    /// True if a task, listed or not, was adopted from this session.
    pub fn adopted(&self, runner: &str, id: &str) -> bool {
        let is = |link: &crate::sessions::Link| link.runner == runner && link.id == id;
        self.list.iter().any(|task| task.session.as_ref().is_some_and(is))
    }

    /// Notes when a task's agent first started. Kept from the first time on.
    pub fn started(&mut self, id: TaskId, at: Option<u64>) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id && t.started_at.is_none()) {
            task.started_at = at;
        }
    }

    /// A task that became done since `before` (each task's state then) was finished at `now`;
    /// one that is no longer done is not finished. `now` 0 (no clock yet): not known.
    pub fn finished(&mut self, before: &[(TaskId, State)], now: u64) {
        for task in &mut self.list {
            let Some((_, was)) = before.iter().find(|(id, _)| *id == task.id) else { continue };
            match (*was == State::Done, task.state == State::Done) {
                (false, true) => task.finished_at = Some(now).filter(|now| *now > 0),
                (true, false) => task.finished_at = None,
                _ => {}
            }
        }
    }

    /// Notes when a task just filed was filed.
    pub fn stamp(&mut self, id: TaskId, now: u64) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id) {
            task.created = Some(now);
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

    /// A snooze, hold or dismiss that changes nothing is no move: `undo` still takes back the
    /// one before it.
    pub fn snooze(&mut self, id: TaskId, until: u64) {
        if let Some(task) = self.list.iter_mut().find(|t| t.id == id).filter(|t| t.snoozed_until != Some(until)) {
            self.undo = Some(Undo::Snooze { id, before: task.snoozed_until.replace(until) });
        }
    }

    /// Pauses a task. Not one that is finished: there is nothing left to pause. Nor one on hold already.
    pub fn hold(&mut self, id: TaskId) {
        let open = |task: &&mut Task| !matches!(task.state, State::Done | State::Cleaned | State::Parked);
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

    /// There is a snooze, hold or dismiss that `undo` would take back.
    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
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
            created: None,
            session: None,
            refs: Vec::new(),
            materials: Vec::new(),
            title_generated: false,
            started_at: None,
            finished_at: None,
            work: Vec::new(),
            extra: Default::default(),
        };
        let mut tasks = Tasks::default();
        // A capture is named by its task's number and its own: nothing else is kept from a file.
        let mut odd = task(9, "odd");
        let names = [
            "9/1.json",
            "9-k3f9a2c41b7d/1.json",
            "../9-k3f9a2c41b7d/1.json",
            "9-k3f9a2c41b7d/../1.json",
            "/etc/passwd",
            "8-k3f9a2c41b7d/1.json",
            "9-k3f9a2c41b7d/x.json",
            "9-k3f9a2c41b7d/.json",
            "9-short/1.json",
            "9-k3f9a2c41b7d/2.json",
            "9-anotherword99/3.json",
        ];
        odd.materials = names.map(String::from).to_vec();
        tasks.restore(vec![odd], 0);
        let kept = tasks.get(9).expect("the task");
        // One folder a task: a name in another is dropped with the rest.
        assert_eq!(kept.materials, ["9-k3f9a2c41b7d/1.json", "9-k3f9a2c41b7d/2.json"]);
        assert_eq!(kept.materials_dir(), Some("9-k3f9a2c41b7d"));
        assert_eq!((materials_dir(9, "k3f9"), materials_dir(9, "k3f9a2c4/x")), (None, None));
        // With no name that is right, none is kept.
        let mut wrong = task(9, "odd");
        wrong.materials = vec!["/etc/passwd".into(), "9/1.json".into()];
        tasks.restore(vec![wrong], 0);
        assert_eq!(tasks.get(9).map(|task| (task.materials.len(), task.materials_dir())), Some((0, None)));
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
    fn age_is_one_unit() {
        let minute = 60_000;
        let ages = [0, 59_999, minute, 59 * minute, 60 * minute, 23 * 60 * minute, 24 * 60 * minute, 9 * 1440 * minute];
        assert_eq!(ages.map(|ms| age(1_000, 1_000 + ms)), ["now", "now", "1m", "59m", "1h", "23h", "1d", "9d"]);
        assert_eq!(age(5_000, 1_000), "now", "a clock that went back");
    }

    #[test]
    fn priority_stays_in_range() {
        let mut tasks = Tasks::default();
        let (low, high) = (tasks.add("a".into(), 0, State::Captured), tasks.add("b".into(), 200, State::Captured));
        assert_eq!((tasks.get(low).map(|t| t.priority), tasks.get(high).map(|t| t.priority)), (Some(1), Some(9)));
    }
}
