//! Workspace state and its transitions. `Model::update` is the only way state changes.

use serde::{Deserialize, Serialize};

use crate::agents;
use crate::apps;
use crate::ax::Target;
use crate::calendar::{self, Calendar};
use crate::capture::{self, Content, Facts, Tool};
use crate::capture_model::Collection;
use crate::commands::{self, Command, Palette, TaskChoice};
use crate::complete::{self, Candidate, Dropdown, Row};
use crate::compose::{self, Draft};
use crate::edit::{Edit, Field, Motion};
use crate::home::{self, Home};
use crate::hosted::{self, Fit, Stuck};
use crate::inspect::{self, Inspector};
use crate::keys::Mods;
use crate::layout::{Axis, Dir, Node, Rect, Zone};
use crate::list::ListRow;
use crate::marks::{self, Extra, MarkId, MarkInput, MarkOf, MarkRef, Marks, Need};
use crate::progress;
use crate::repos::{self, Repo};
use crate::runner::{RunnerId, Support};
use crate::sessions::{self, ENDED_GRACE_MS, ENDED_HELD_MS, ENDED_MS, Found};
use crate::settings::{Installed, closed_apps};
use crate::sneak::{Sneak, Typed};
use crate::talk::{self, Subject, Talk};
use crate::tasks::{
    self, Assign, Heading, MAX_TITLE, Moved, NOTICE_MS, Notice, Ref, RefKind, State, Task, TaskId, TaskRow,
    TaskSection, Tasks,
};
use crate::workbook::{self, History, Workbook, WorkbookId, Workbooks};

/// The axes of a `Fit`'s sizes, in their order: width, height.
const AXES: [Axis; 2] = [Axis::Right, Axis::Down];

pub type PaneId = u32;

/// What a pane shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// Nothing yet. The next thing opened fills it.
    Empty,
    /// A shell on a PTY. The shell crate owns the process.
    Terminal,
    /// Another app's window, by its window-server id. `bundle` and `name` say whose it is, e.g.
    /// `com.tinyspeck.slackmacgap` and `Slack`: a mark finds the app again by them.
    App {
        window: u32,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle: Option<String>,
        #[serde(default)]
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pane {
    pub id: PaneId,
    pub kind: Kind,
    /// Where a terminal pane's shell starts, when it was opened for a repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// An app pane whose window kept another size than the pane's; see `hosted::Fit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
}

/// Who gets the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// ottospace: its shortcuts work.
    Normal,
    /// The live pane's app: ottospace shortcuts do not fire.
    App,
    /// The pane picker: keys filter and choose.
    Pick,
    /// The command palette: keys filter commands and run one.
    Palette,
    /// The accessibility inspector's filter has the keyboard.
    Inspect,
    /// A cursor in a terminal's output: keys move it and select text to copy.
    Visual,
    /// The secondary bar: keys move over its rows and act on the selected one.
    List,
    /// The new-task box: keys type a line that becomes a task.
    Compose,
    /// Labels on ottospace's own controls: keys type a label, which does what a click would.
    Sneak,
    /// The quit question: ↵ quits, any other key cancels.
    Confirm,
    /// A mark has the keyboard: the jump prefix waits for a letter, or a row for its name or letter.
    Mark,
    /// A frozen picture of a pane is drawn on: drags circle, a click places a note.
    Annotate,
    /// The conversation in the right drawer: keys go to the task's agent.
    Talk,
}

/// A terminal that runs something quitting would end.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Running {
    pub pane: PaneId,
    /// The program's name, e.g. "cargo". `None`: it could not be read.
    pub program: Option<String>,
    /// A background or stopped job of the shell (`sleep 100 &`), not the program in front.
    pub background: bool,
}

/// Why a socket client's request is refused under the quit question.
pub const CONFIRM_UP: &str = "the quit question is up";
/// Why a client may not link a tab to a task, or unlink it.
pub const TABS_USERS: &str = "only the user says which task a tab is of";

/// The quit question: these terminals would lose a running program.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Confirm {
    pub running: Vec<Running>,
    /// Not the quit question: "Stop its agent and dismiss?" for this task, whose agent runs.
    pub dismiss: Option<TaskId>,
}

impl Confirm {
    /// What a key does to the question. The user's: ↵, Q or ⌘Q quits; every other key or
    /// shortcut cancels, and does nothing else, e.g. ⌘W cancels and closes nothing. A socket
    /// client's: ↵ or Q quits and Esc cancels, so a test can answer; `None` for any other key,
    /// which is refused: a client's shortcut must not answer for the user.
    pub fn answer(key: &str, mods: Mods, origin: Origin) -> Option<Action> {
        let plain = mods == Mods::default();
        let cmd = mods == Mods { cmd: true, ..Mods::default() };
        match (key, origin) {
            ("enter" | "q", _) if plain => Some(Action::QuitConfirm),
            ("q", Origin::User) if cmd => Some(Action::QuitConfirm),
            ("escape", _) if plain => Some(Action::QuitCancel),
            (_, Origin::User) => Some(Action::QuitCancel),
            (_, Origin::Socket | Origin::System) => None,
        }
    }

    /// "3 terminals are running". With none, the question is about a draft's captures alone.
    pub fn title(&self) -> String {
        if self.dismiss.is_some() {
            return "Stop its agent and dismiss?".into();
        }
        match self.running.len() {
            0 => "A draft is open".into(),
            1 => "1 terminal is running".into(),
            n => format!("{n} terminals are running"),
        }
    }

    /// One line a terminal, e.g. "pane 2 · cargo" or "pane 3 · sleep (background)": at most 5, then "and 2 more".
    pub fn lines(&self) -> Vec<String> {
        let line = |r: &Running| match (&r.program, r.background) {
            (Some(program), true) => format!("pane {} · {program} (background)", r.pane),
            (Some(program), false) => format!("pane {} · {program}", r.pane),
            (None, true) => format!("pane {} · (background)", r.pane),
            (None, false) => format!("pane {}", r.pane),
        };
        let mut lines: Vec<String> = self.running.iter().take(5).map(line).collect();
        if self.running.len() > 5 {
            lines.push(format!("and {} more", self.running.len() - 5));
        }
        lines
    }
}

/// Where the new-task box is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    /// The drawer, in its new-task state: the workspace stays in use beside it.
    Drawer,
    /// The title of the draft in the drawer's collecting state.
    Bar,
}

/// A new-task line put aside, with what its tags pointed at.
#[derive(Clone, Debug, PartialEq)]
struct KeptDraft {
    line: Field,
    refs: Vec<Ref>,
    attached: Vec<String>,
    theirs: bool,
}

/// How many repositories the drawer offers as picks.
pub const PICKS: usize = 4;

/// The new-task line's one-click picks: repository names (WHERE) and agent names (WHO).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Picks {
    pub wheres: Vec<String>,
    pub whos: Vec<String>,
}

/// What ↵ would do: start `agent` (unless it is a saved agent, `starts` false) in `dir`, a
/// listed repository's name (none: the home folder), on `branch`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Plan {
    pub agent: String,
    pub starts: bool,
    pub dir: Option<String>,
    pub branch: Option<String>,
}

/// What the user did to the dropdown, for the tag it was done to (`key`): for another tag it
/// counts for nothing.
#[derive(Clone, Debug, Default, PartialEq)]
struct Menu {
    /// Where the tag starts, its mark and what is typed of it.
    key: Option<(usize, char, String)>,
    /// Esc closed it.
    dismissed: bool,
    /// The row ↑ or ↓ went to, by its name: the rows may change under it (a scan lands, a
    /// task is added), and the highlight stays on what it was on.
    moved: Option<String>,
}

/// A path as a row or the line shows it: `~/..` inside the user's folder `home`.
fn short(home: Option<&str>, path: &str) -> String {
    match home.and_then(|home| path.strip_prefix(home)) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// True if two references are to one thing: the same tab, pane, mark or app.
fn same_target(a: &Ref, b: &Ref) -> bool {
    (a.kind, a.id, &a.bundle) == (b.kind, b.id, &b.bundle)
}

/// A terminal pane as the shell last saw it, for a `^reference` to tell panes apart.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalInfo {
    pub pane: PaneId,
    pub cwd: Option<String>,
    /// The program in front of the shell, if one is.
    pub program: Option<String>,
}

/// What an ⌥-click in the workspace points at while the new-task box is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointed {
    /// A tab, by its `id`.
    Tab(u32),
    Pane(PaneId),
}

/// The most characters the picker's filter holds: no app's name is longer, and a huge paste
/// is not shaped again every frame.
pub const MAX_FILTER: usize = 200;

/// Something the picker can put in a new pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PickItem {
    Terminal,
    /// A running app, by name. The shell hosts its main window.
    App {
        name: String,
    },
    /// An installed app that is not running, by where it is. Choosing it starts it.
    Closed {
        name: String,
        path: String,
    },
}

impl PickItem {
    pub fn label(&self) -> &str {
        match self {
            PickItem::Terminal => "Terminal",
            PickItem::App { name } | PickItem::Closed { name, .. } => name,
        }
    }
}

/// The open picker: what a new pane can show, narrowed by what the user typed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Picker {
    /// Where the new pane goes relative to the focused one.
    pub axis: Axis,
    /// Open the pick in a new tab instead of splitting.
    pub new_tab: bool,
    pub items: Vec<PickItem>,
    /// The filter text: a field, with a cursor and a selection.
    pub query: Field,
    /// Index into `matches()`.
    pub selected: usize,
    /// Every other installed app that is not running, by name. Offered once something is typed.
    #[serde(skip)]
    pub installed: Vec<PickItem>,
}

impl Picker {
    /// The rows. With nothing typed: `items`, as given. Else the items whose label contains
    /// the filter, ignoring case, those that start with it first: the terminal and running apps,
    /// then the apps that are not running, those of `items` (the user's own list) before the
    /// rest of what is installed.
    pub fn matches(&self) -> Vec<&PickItem> {
        let query = self.query.text().to_lowercase();
        if query.is_empty() {
            return self.items.iter().collect();
        }
        let closed = |item: &&PickItem| matches!(item, PickItem::Closed { .. });
        let groups: [Vec<&PickItem>; 3] = [
            self.items.iter().filter(|item| !closed(item)).collect(),
            self.items.iter().filter(closed).collect(),
            self.installed.iter().collect(),
        ];
        let mut rows = Vec::new();
        for group in groups {
            let hit = |starts: bool| {
                let query = &query;
                group.iter().copied().filter(move |item| {
                    let label = item.label().to_lowercase();
                    label.contains(query) && label.starts_with(query) == starts
                })
            };
            rows.extend(hit(true).chain(hit(false)));
        }
        rows
    }
}

/// The installed apps the picker offers beyond its own items: those not running (their bundle
/// is not in `running`) and not already an item, each bundle once, by name.
fn offered(installed: &[Installed], running: &[String], items: &[PickItem]) -> Vec<PickItem> {
    let listed = |path: &str| items.iter().any(|item| matches!(item, PickItem::Closed { path: p, .. } if p == path));
    let mut apps = closed_apps(installed, running);
    apps.retain(|app| !listed(&app.path));
    apps.sort_by_key(|app| (app.name.to_lowercase(), app.path.clone()));
    apps.into_iter().map(|app| PickItem::Closed { name: app.name, path: app.path }).collect()
}

/// A category on the left rail. Its list shows in the secondary bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Apps,
    Workbook,
    Tasks,
    Repos,
    Marks,
    /// Today's meetings. Only while a calendar feed is set (`Model::sections`).
    Calendar,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Split the focused pane; the new half shows `kind`.
    Split {
        axis: Axis,
        kind: Kind,
    },
    /// Show `kind` in `beside` (default: the focused pane) if that pane is empty; else split it.
    Open {
        kind: Kind,
        axis: Axis,
        beside: Option<PaneId>,
    },
    /// Add a pane for a window the app opened itself (e.g. DevTools), beside `beside`.
    /// Focus and the keyboard stay where they are: the user did not ask for this pane.
    Adopt {
        kind: Kind,
        beside: Option<PaneId>,
    },
    /// Close a pane (default: the focused one).
    Close {
        pane: Option<PaneId>,
    },
    Focus {
        pane: PaneId,
    },
    FocusDir {
        dir: Dir,
    },
    /// Move focus to the next pane (positive) or the previous one, in reading order. It wraps.
    FocusCycle {
        delta: i32,
    },
    /// Move the boundary of the nearest split around the focused pane one step toward `dir`.
    Resize {
        dir: Dir,
    },
    /// The focused pane trades places with its neighbour on that side. Focus stays on it.
    SwapDir {
        dir: Dir,
    },
    /// Move a pane (default: the focused one) to the tab at place `tab` on the tab bar, beside
    /// that tab's focused pane. The tab comes forward with the pane focused.
    MovePane {
        pane: Option<PaneId>,
        tab: usize,
    },
    /// Even out the splits of the active tab.
    Equalize,
    /// Drop pane `source` on pane `target`: an edge zone splits `target` and puts `source` on
    /// that side; the centre swaps the two. `source` takes focus, and its tab comes forward.
    DropPane {
        source: PaneId,
        target: PaneId,
        zone: Zone,
    },
    /// The shell measured an app pane after asking its window for the pane's size: the pane's
    /// `content` and whole (`outer`) size, and the size the `window` has. `[width, height]`, points.
    PaneWants {
        pane: PaneId,
        content: [f32; 2],
        outer: [f32; 2],
        window: [f32; 2],
    },
    /// Move the splits around a pane (default: the focused one) so it has its window's size, on
    /// each axis that has a split to move. Only ever on request. The pane may be in a
    /// background tab. See `Model::fit_refusal` for when it does nothing.
    Fit {
        pane: Option<PaneId>,
    },
    /// Hand the keyboard to an app pane.
    Enter {
        pane: PaneId,
    },
    /// Take the keyboard back.
    Leave,
    /// Open a rail section in the secondary bar. The open section's own button closes the bar.
    ToggleSection {
        section: Section,
    },
    CloseSidebar,
    /// Open the pane picker. `items` is what the shell can offer right now. With `new_tab`,
    /// the pick opens in a new tab.
    OpenPicker {
        axis: Axis,
        new_tab: bool,
        items: Vec<PickItem>,
    },
    /// A new, empty tab. It becomes the active tab. The user's joins the group of the task whose
    /// tab is on show.
    NewTab,
    /// ⌘T: a new tab holding one empty pane, shown. No picker: ⌘E picks what goes in it.
    NewEmptyTab,
    /// A new tab in `task`'s group, with a terminal in its repository; `None`: a tab of no task.
    /// A task's is the user's alone.
    NewTabOf {
        task: Option<TaskId>,
    },
    /// Move a tab (default: the one on show) on the bar: into a group, or before a tab, whose
    /// task it takes. The user's alone when its task changes.
    TabPlace {
        tab: Option<u32>,
        at: TabAt,
    },
    SelectTab {
        index: usize,
    },
    /// The next (positive) or previous tab. It wraps.
    CycleTab {
        delta: i32,
    },
    /// Close a tab (default: the active one) and every pane in it. The last tab is replaced by
    /// an empty one.
    CloseTab {
        index: Option<usize>,
    },
    /// The apps found installed, and the bundles of those running now. The shell sends it when
    /// the picker opens and when a scan ends.
    SetInstalled {
        apps: Vec<Installed>,
        running: Vec<String>,
    },
    /// Replace the picker's filter text.
    PickerQuery {
        query: String,
    },
    /// Move the picker's selection down (positive) or up; it wraps.
    PickerMove {
        delta: i32,
    },
    /// Open an item (default: the selected one) and close the picker. `index` is into the
    /// current matches, so a click opens the row it landed on.
    PickerChoose {
        index: Option<usize>,
    },
    ClosePicker,
    /// Open the command palette. Only the user's: it runs any command as theirs.
    PaletteOpen,
    /// Move Tab to Task…: the palette, listing the tasks (and no task) to move the tab on show
    /// to. Only the user's, as is which task a tab is of.
    PaletteTasks,
    /// Move the palette's highlight down (positive) or up; it wraps.
    PaletteMove {
        delta: i32,
    },
    /// Run a command (default: the highlighted one; `index` is into the matches) and close the palette.
    PaletteChoose {
        index: Option<usize>,
    },
    ClosePalette,
    /// Open the accessibility inspector on a pane, named for its `app`. Only the user's.
    InspectorOpen {
        pane: PaneId,
        app: String,
    },
    /// The shell read the inspected pane's window: its tree, the window's frame, how long it took.
    InspectorRead {
        pane: PaneId,
        tree: crate::ax::Tree,
        window: crate::ax::Frame,
        millis: f64,
    },
    /// The shell could not read it: why.
    InspectorFailed {
        pane: PaneId,
        why: String,
    },
    InspectorClose,
    /// Fold or unfold the node at `path` (child indices from the root).
    InspectorFold {
        path: Vec<usize>,
    },
    /// Select the node at `path`, or none: its frame is outlined on the pane.
    InspectorSelect {
        path: Option<Vec<usize>>,
    },
    /// Move the selection down (positive) or up the rows; it wraps.
    InspectorMove {
        delta: i32,
    },
    /// Give the keyboard to the inspector's filter, or take it back.
    InspectorFilter {
        on: bool,
    },
    /// Show or hide the key reference.
    ToggleKeys,
    CloseKeys,
    /// Enter visual mode on the focused terminal, or leave it.
    ToggleVisual,
    LeaveVisual,
    /// Show or hide the accessibility debug overlay on the focused app pane.
    ToggleAxOverlay,
    /// Replace the Repos list with what the shell found on disk.
    SetRepos {
        repos: Vec<Repo>,
    },
    /// The directories the Repos list is read from, as the shell resolved them.
    SetRepoDirs {
        dirs: repos::RepoDirs,
    },
    /// Add `dir` (resolved) to the settings' `repos`: `found` is what the shell read there. With
    /// no repository, a notice. Listed already: nothing is written. Its first row is selected.
    RepoAdd {
        dir: String,
        found: Vec<Repo>,
    },
    /// Stop listing the repository at `path`: its directory comes off the settings' `repos`,
    /// if it is named there as itself.
    RepoRemove {
        path: String,
    },
    /// Open a terminal in a listed repository or worktree, by its path. An empty tab takes its name.
    OpenRepo {
        path: String,
        axis: Axis,
    },
    /// Start from the tasks kept on disk. `floor`: new ids start above it (see `Tasks::restore`).
    RestoreTasks {
        tasks: Vec<Task>,
        floor: TaskId,
    },
    /// File a task. `tab`: ignored: only the user links a tab to a task.
    /// `assign`: who works it, the paths it is about and the skills it loads, kept as given.
    TaskAdd {
        title: String,
        priority: u8,
        state: State,
        tab: Option<u32>,
        assign: Assign,
    },
    /// What the shell read for `line` in the new-task box. `names`: the agents and skills a tag
    /// may name, when they were read again. `paths` and `entries` count only while the box
    /// still holds `line`: an answer for a line since changed is dropped.
    SetKnown {
        line: String,
        /// The path being typed at the cursor that `entries` finish, if one was.
        typing: Option<String>,
        names: Option<compose::Names>,
        paths: Vec<compose::PathFound>,
        entries: Vec<String>,
    },
    /// Each terminal pane's directory and program now: the shell tells it when the new-task
    /// box opens, for `^references`.
    SetTerminals {
        terminals: Vec<TerminalInfo>,
        /// The user's folder: a path inside it is shown as `~/..` on a dropdown's row.
        home: Option<String>,
    },
    TaskSetState {
        id: TaskId,
        state: State,
    },
    /// Hide a task from the inbox until `until` (milliseconds since 1970).
    TaskSnooze {
        id: TaskId,
        until: u64,
    },
    TaskHold {
        id: TaskId,
    },
    /// Take a task out of the inbox. It goes on running.
    TaskDismiss {
        id: TaskId,
    },
    /// Take back the last snooze, hold or dismiss.
    TaskUndo,
    /// Snooze, hold or dismiss a task with the buttons on its row (the mouse). As `TaskSnooze`
    /// and the others; the Tasks section then offers to take it back for a few seconds
    /// (`Model::undo_offer`), counted from the last `Tick`.
    RowSnooze {
        id: TaskId,
        until: u64,
    },
    RowHold {
        id: TaskId,
    },
    RowDismiss {
        id: TaskId,
    },
    /// Go to a task's tabs: the one of them shown last (default task: the first that needs the
    /// user). A task with no tab: nothing. The user's ⌘⇧N (no task named) shows that task's
    /// conversation instead, and the tab stays.
    TaskGo {
        id: Option<TaskId>,
    },
    /// Select a tab by its place on the tab bar.
    SelectVisibleTab {
        index: usize,
    },
    /// The time now, in milliseconds since 1970: snoozed tasks whose time has come return, and
    /// a notice whose time is up goes.
    Tick {
        now: u64,
    },
    /// Open the new-task box, with `text` already in it (an agent's draft for the user to edit).
    OpenCompose {
        text: String,
    },
    /// One edit to the text field that has the keyboard: the picker's filter, else the
    /// new-task box.
    FieldEdit {
        edit: Edit,
    },
    /// Tab: take the dropdown's highlighted row, or its first.
    ComposeComplete,
    /// Esc: close the dropdown if it is open, else the box.
    ComposeEscape,
    /// ↑ or ↓: move the dropdown's highlight. With no dropdown, to the line's start or end.
    ComposeMove {
        delta: i32,
    },
    /// A click on a row of the dropdown: take it.
    ComposeAccept {
        index: usize,
    },
    /// Files or folders dropped on the box, picked or pasted: each goes in at the cursor as a
    /// `/path` word. `home`: the user's folder, so paths inside it are written `~/..`.
    ComposeAttach {
        paths: Vec<String>,
        home: Option<String>,
        /// How many more were given whose names are not text (not UTF-8): they are left out,
        /// and the box says so.
        unreadable: usize,
    },
    /// An ⌥-click on a tab or a pane while the box is open: its `^reference` goes in at the cursor.
    ComposeRef {
        target: Pointed,
    },
    /// A click on one of the drawer's picks: the line's `>repo` (`mark` `>`) or `@agent` (`@`)
    /// becomes `name`, or goes with `None`. Only a name `compose_picks` offers.
    ComposePick {
        mark: char,
        name: Option<String>,
    },
    /// File the line as a task and close the box. A line with no title files nothing.
    /// `open`: show the new task's tabs at once ("add and start").
    ComposeSubmit {
        open: bool,
    },
    CloseCompose,
    /// File a task from one line, as the box does: `fix zoom flicker #canvas >app @claude p1`.
    /// `known`: what the shell read for this line. The new-task box's own is left alone.
    TaskFromLine {
        line: String,
        known: compose::Known,
        open: bool,
    },
    /// Collapse or expand a heading of the Tasks section.
    ToggleGroup {
        heading: Heading,
    },
    /// The Tasks section by state, or by project.
    TasksView {
        view: TasksView,
    },
    /// Fold or unfold a project of the Project view (`None`: the tasks of no project), or with
    /// `done` its "Done" line.
    ToggleProject {
        project: Option<String>,
        done: bool,
    },
    /// What a session of a task's agent did, as the shell last read it (`progress`).
    SessionWork {
        work: progress::Work,
    },
    /// Replace the sessions the shell found on this machine for the listed repositories. `day`:
    /// the start of the local day they were read in, from which Today lists.
    SetSessions {
        found: Vec<Found>,
        day: u64,
    },
    /// How long an adopted session must be quiet before its task needs the user, in
    /// milliseconds: the `session_quiet_minutes` setting. 0: never.
    SessionQuiet {
        ms: u64,
    },
    /// When the files of sessions that tasks were adopted from last changed: read by the shell
    /// while the Tasks section is closed, so those tasks follow their sessions all the same.
    SessionsSeen {
        seen: Vec<sessions::Seen>,
    },
    /// Make a task of a session: named after its repository, in its directory, worked by its
    /// runner, and linked to it, so the session is listed as that task from now on.
    SessionAdopt {
        runner: String,
        id: String,
    },
    /// Stop listing a session, until the app starts again.
    SessionHide {
        runner: String,
        id: String,
    },
    /// A click on a session's row: focus the pane it runs in, if that is known; else select the row.
    SessionClick {
        runner: String,
        id: String,
    },
    /// The user's click or ↵ on a session's row: its conversation in the right drawer, gone on
    /// with there unless it is at work elsewhere. No task is made. Only the user's.
    SessionOpen {
        runner: String,
        id: String,
    },
    /// Which sessions Other sessions lists, by when each was last active. The user's pick is
    /// written to the settings.
    SessionsRange {
        range: sessions::Range,
    },
    /// Replace the Apps list with the apps the shell can host now.
    SetApps {
        names: Vec<String>,
    },
    /// What the settings say of apps, read at start: the pinned ones (`apps`), those not to
    /// suggest (`apps_hidden`), and whether the Dock's are suggested at all.
    SetAppSettings {
        pinned: Vec<apps::Entry>,
        hidden: Vec<apps::Entry>,
        suggest: bool,
    },
    /// The Dock's apps, in its order, as the shell last read them.
    SetDock {
        apps: Vec<Installed>,
    },
    /// Pin the app at `path`, or unpin it if it is pinned. `Effect::SaveApps` says what to write.
    AppPin {
        path: String,
    },
    /// Stop suggesting the Dock's app at `path`.
    AppHide {
        path: String,
    },
    /// Take back the last pin, unpin or hide.
    AppsUndo,
    /// Pin or unpin the selected row's app.
    ListPin,
    /// Fold or unfold "and N more" or "All apps".
    AppsToggle {
        group: apps::Group,
    },
    /// Give the keyboard to the filter of "All apps", or take it back.
    AppsFilter {
        on: bool,
    },
    /// Give the keyboard to the secondary bar (it opens if it is closed), or take it back.
    FocusList,
    LeaveList,
    /// Move the list selection down (positive) or up. It wraps.
    ListMove {
        delta: i32,
    },
    /// Show the next (positive) or previous section, with its first row selected.
    ListSection {
        delta: i32,
    },
    /// Open the selected row: host the app, open the repository, or bring the task's tab forward.
    ListChoose {
        axis: Axis,
    },
    /// Snooze, hold or dismiss the selected row, if it is a task.
    ListSnooze {
        until: u64,
    },
    ListHold,
    ListDismiss,
    /// Label ottospace's own controls, as last drawn (`targets`), or leave sneak mode if it is on.
    /// Only from normal and list mode: elsewhere a box covers the controls, or an app has the keys.
    /// Starting hides the accessibility overlay, whose chips use the same letters.
    Sneak {
        targets: Vec<Target>,
    },
    /// Start sneak mode, as `Sneak` does. If it is on, nothing changes: two callers asking at once
    /// both find it on.
    SneakEnter {
        targets: Vec<Target>,
    },
    /// The controls the frame just painted drew. A label follows its control; the label of a
    /// control that is no longer drawn goes; a new control gets none.
    SneakDrawn {
        targets: Vec<Target>,
    },
    /// Letters of a label. A letter no label goes on with leaves the mode, and the letters after
    /// it are dropped.
    SneakType {
        text: String,
    },
    SneakBackspace,
    LeaveSneak,
    /// The focused app pane's window as just read, with no text (`read`), or `None`: it is gone or
    /// not shown. Its elements get labels on the first read; later reads move or drop them.
    SneakApp {
        pane: PaneId,
        read: Option<crate::sneak_apps::AppRead>,
    },
    /// The user's wheel or trackpad over a parked app pane, at `now` (ms): its window comes up
    /// over the pane for the scroll (a peek). See `app_wheel`.
    AppWheel {
        pane: PaneId,
        now: u64,
    },
    /// While a peek is on, the shell's look at it: the pointer is `over` its pane, a click or a
    /// key was `pressed` since it began (the app has both), the last scroll was at `scrolled` (ms).
    PeekCheck {
        now: u64,
        over: bool,
        pressed: bool,
        scrolled: Option<u64>,
    },
    /// ⌘Q. `running`: the terminals with a program in front of their shell, as the shell crate
    /// sees them now. None: quit. Some: ask first. Asked again while the question is up: quit.
    QuitAsked {
        running: Vec<Running>,
    },
    /// Answers to the quit question.
    QuitConfirm,
    QuitCancel,
    /// Quit without asking: the socket's `quit`.
    Quit,
    /// Replace the marks: with those kept on disk at start, or with this run's merged with
    /// another's (`marks::merge`). `extra`: the file's keys this build does not know.
    RestoreMarks {
        marks: Vec<marks::Mark>,
        extra: Extra,
    },
    /// Mark a pane or a tab; one that is marked takes the name and letter given. `uid`: the new
    /// mark's name for good, made up by the shell. `cwds`: each terminal's directory now, as
    /// the shell crate read it.
    MarkAdd {
        of: MarkOf,
        name: Option<String>,
        key: Option<char>,
        uid: String,
        cwds: Vec<(PaneId, String)>,
    },
    /// Mark what is not marked; take the mark off what is.
    MarkToggle {
        of: MarkOf,
        uid: String,
        cwds: Vec<(PaneId, String)>,
    },
    /// Each app pane's window title as it is now: the shell reads them before a mark is made
    /// or gone to, so a mark records the title of now, not of when the window was hosted.
    Titles {
        titles: Vec<(PaneId, String)>,
    },
    /// The shell could not fill a rebuilt tab's empty pane: why.
    Unfilled {
        pane: PaneId,
        why: String,
    },
    /// Put the last removed mark back.
    MarkUndo,
    /// `u` in list mode: in Marks, `MarkUndo`; else `TaskUndo`.
    ListUndo,
    MarkRemove {
        mark: MarkRef,
    },
    MarkRename {
        mark: MarkRef,
        name: String,
    },
    /// Go to a mark: to its pane or tab if that is open now, else open it. `axis`: where a new
    /// pane goes if the focused one is not empty.
    MarkGo {
        mark: MarkRef,
        axis: Axis,
    },
    /// The window the shell hosted for a pane mark is in `pane`.
    MarkBind {
        mark: MarkId,
        pane: PaneId,
    },
    /// What the marks need (`Marks::needs`) that the shell did not find on this machine.
    MarksChecked {
        missing: Vec<Need>,
    },
    /// Show `kind` in an empty pane, in whichever tab it is. Focus stays where it is: a rebuilt
    /// tab's app pane gets its window when the app has one.
    Fill {
        pane: PaneId,
        kind: Kind,
    },
    /// The jump prefix: wait for a mark's letter, or stop waiting.
    MarkPrefix,
    /// Start typing a mark's name or letter in its row.
    MarkEdit {
        mark: MarkRef,
        field: marks::Field,
    },
    /// Typed text, for whatever a mark waits for.
    MarkType {
        text: String,
    },
    MarkBackspace,
    /// Keep the typed name.
    MarkSubmit,
    MarkCancel,
    /// `MarkEdit` for the selected row, if it is a mark.
    ListMarkEdit {
        field: marks::Field,
    },
    /// Move the selected mark down (positive) or up the list. On any other row: `ListMove`.
    ListReorder {
        delta: i32,
    },
    /// The capture key: freeze a picture of a pane (default: the live one, else the focused
    /// one) and annotate it, as a material of the draft in the right bar. The bar opens if it
    /// is closed, with `draft` as its name; the new-task box's line moves there. A live app is
    /// parked first. `facts`: what the shell read of the panes on show. `images`: pictures are
    /// kept. With no pane to capture, only the bar opens.
    /// `word`: letters and digits made up by the shell for this draft (see `Collection::word`).
    /// Only the user starts a capture: it takes a picture of a pane, and reads its text.
    CaptureStart {
        pane: Option<PaneId>,
        draft: String,
        word: String,
        facts: Vec<Facts>,
        images: bool,
    },
    /// The shell froze the picture of a capture. `picture`: there was one to take. `stale`: the
    /// window's stream had stopped, so it may show the window as it was a while ago.
    CaptureTaken {
        draft: String,
        material: u32,
        picture: bool,
        stale: bool,
    },
    /// What the shell read of the panes on show, just before the draft is added.
    CaptureFacts {
        facts: Vec<Facts>,
    },
    /// The text read with a capture's picture. For a draft since gone it is dropped.
    CaptureContent {
        draft: String,
        material: u32,
        content: Content,
    },
    /// The button went down on the picture being annotated, at `at` (0..1 of it). `size`: the
    /// picture's, in points. ⇧ draws a box and ⌥ a freehand stroke.
    AnnotatePress {
        at: [f64; 2],
        shift: bool,
        alt: bool,
        size: [f64; 2],
    },
    AnnotateDrag {
        to: [f64; 2],
    },
    AnnotateRelease {
        at: [f64; 2],
    },
    /// What a plain drag draws from now on.
    AnnotateTool {
        tool: Tool,
    },
    AnnotateUndo,
    AnnotateRedo,
    /// Remove the selected annotation.
    AnnotateRemove,
    /// End the note being typed. `keep`: it stays (↵); else it is dropped (Esc).
    AnnotateNote {
        keep: bool,
    },
    /// Leave annotate mode, keeping what was drawn. `title`: the draft's title takes the keyboard.
    AnnotateLeave {
        title: bool,
    },
    /// Annotate a capture of the draft again, by its number, if its pane is on show.
    CaptureEdit {
        material: u32,
    },
    /// Take a capture out of the draft.
    CaptureRemove {
        material: u32,
    },
    /// × or Esc on the draft in the bar: it goes, after a question if it holds captures.
    CaptureCancel,
    /// The answer to that question.
    CaptureDiscard {
        yes: bool,
    },
    /// The draft's title takes the keyboard.
    CaptureTitle,
    /// Back to the app the capture key took the keyboard from.
    CaptureBack,
    /// Show a task's brief in the right bar, or hide it.
    BriefShow {
        id: TaskId,
    },
    BriefClose,
    /// Replace the workbooks with those kept on disk, at start.
    RestoreWorkbooks {
        workbooks: Vec<Workbook>,
    },
    /// Build the tabs `layout.json` kept, at start, into the fresh model: the shell's alone.
    /// `gone`: its directories not found now, whose terminals start in `home`. `launch`: an app
    /// a pane names that is not running may be started (`restore_apps`).
    RestoreLayout {
        layout: Box<crate::layout_file::Layout>,
        gone: Vec<String>,
        home: Option<String>,
        launch: bool,
    },
    /// File ▸ Start Fresh: the next start begins empty. The layout is not kept this run.
    StartFresh,
    /// "Open <App>" on a restored app pane that waits for its window.
    UnfilledOpen {
        pane: PaneId,
    },
    /// Save the tab on show as a workbook, or update its tab of the one it came from. `uid`: a
    /// new one's file name, made up by the shell. `cwds`: each terminal's directory now.
    /// `histories`: each terminal's last lines and the command in front, as the shell read them.
    WorkbookSave {
        uid: String,
        cwds: Vec<(PaneId, String)>,
        histories: Vec<(PaneId, History)>,
    },
    /// Add the tab on show to a workbook, or update it there.
    WorkbookAddTab {
        workbook: WorkbookId,
        cwds: Vec<(PaneId, String)>,
        histories: Vec<(PaneId, History)>,
    },
    /// Save the tabs on the bar as one new workbook.
    WorkbookSaveAll {
        uid: String,
        cwds: Vec<(PaneId, String)>,
        histories: Vec<(PaneId, History)>,
    },
    /// Open a workbook's tabs (`tab`: that one, by place) in new tabs, keeping those that are
    /// open, and select the first. `home`: the user's folder, where a terminal starts whose
    /// directory is gone.
    WorkbookOpen {
        workbook: WorkbookId,
        tab: Option<usize>,
        home: Option<String>,
    },
    /// Start typing a workbook's name in its row (`tab`: that tab's title). `MarkSubmit` keeps
    /// it, `MarkCancel` does not.
    WorkbookEdit {
        workbook: WorkbookId,
        tab: Option<usize>,
    },
    /// Delete a workbook, or (`tab`) one tab of it.
    WorkbookRemove {
        workbook: WorkbookId,
        tab: Option<usize>,
    },
    /// List a workbook's tabs under its row, or stop.
    WorkbookFold {
        workbook: WorkbookId,
    },
    /// Take the last delete or update of a workbook back.
    WorkbookUndo,
    /// What the settings say of agents, read at start: each runner's models (`agent_models`)
    /// and the agent a task with no `@agent` starts with (`default_agent`).
    SetAgents {
        claude: Vec<String>,
        codex: Vec<String>,
        default: Option<String>,
        /// `conversation_view`: how a Claude Code conversation shows by default.
        shows: crate::transcript::View,
    },
    /// What a conversation's native agent said, in order, read at `at` (ms since 1970). Only
    /// the shell's runner sends it (`Origin::System`).
    AgentEvents {
        task: Subject,
        events: Vec<crate::transcript::Event>,
        at: u64,
    },
    /// ↵ in the shown conversation's composer: its draft to the agent, if no turn runs and no
    /// card waits. Only the user's.
    TalkSend,
    /// Allow or Deny the shown conversation's first card. Allow only once a cut card was opened
    /// in full (`TalkExpand`). Deny carries the draft when the user wrote one for it
    /// (`TalkDenyNote`). `ticket`: the card the user saw; another card first (the CLI withdrew
    /// that one) gets nothing. Only the user's.
    TalkAnswer {
        allow: bool,
        ticket: u64,
    },
    /// "Show all" on the shown card, `ticket` as in `TalkAnswer`. Only the user's.
    TalkExpand {
        ticket: u64,
    },
    /// The composer takes the user's words for a Deny of the shown card. Only the user's.
    TalkDenyNote,
    /// Esc in a native conversation: ends a Deny's message; while a turn runs, the first says
    /// a second stops it; else the keyboard leaves to the workspace. Only the user's.
    TalkEscape,
    /// Stop the shown conversation's turn; the session goes on. Only the user's.
    TalkInterrupt,
    /// The shown conversation in the other view: its agent ends and its session goes on there.
    /// Not while a turn runs or a card waits. Only the user's.
    TalkView {
        shows: crate::transcript::View,
    },
    /// A click or ↵ on a task: its conversation opens in the right drawer, with the keyboard.
    /// Its agent is started if it never ran, or its session gone on with if no terminal of it
    /// is live. The workspace stays as it is. Only the user's.
    TaskTalk {
        id: TaskId,
    },
    /// ↵ in the new-task box: takes the highlighted row of the dropdown, else adds the task
    /// and starts it (`ComposeSubmit { open: true }`).
    ComposeEnter,
    /// The shell started the agent a `StartAgent` asked for, at `at` (ms since 1970).
    /// `session`: the id it gave the new session, if the runner's CLI takes one.
    AgentStarted {
        task: Subject,
        session: Option<String>,
        at: u64,
    },
    /// Sessions of a runner that names its own (Codex) that started in a folder an agent was
    /// started in (`Model::seeking`), read at `now`. `home`: where an agent with no repository
    /// runs. Each task waiting for its session is linked to the first one that started after it.
    AgentSessions {
        found: Vec<agents::Started>,
        home: Option<String>,
        now: u64,
    },
    /// A task's agent is gone: it ended, was stopped, or could not be started.
    AgentExited {
        task: Subject,
    },
    /// A session read for a resume the user asked for (`Effect::ReadSession`): where it started,
    /// whether it is at work or a process holds it. `None`: no listed checkout has it.
    SessionRead {
        runner: String,
        id: String,
        found: Option<Found>,
    },
    /// The session id an agent was started with has no file now its agent is gone: it never
    /// wrote one (`claude` not found, stopped at once). The task stops following it, so the
    /// next click starts a new session, not one that cannot be gone on with.
    SessionMissing {
        task: TaskId,
        id: String,
    },
    /// Stop a conversation's agent. Only the user's.
    TaskStop {
        id: Subject,
    },
    /// Close the drawer to the right rail, or open it again on the same conversation.
    TalkToggle,
    /// The drawer header's go word (⌘↵): see `Model::talk_go`. Only the user's.
    TalkGo,
    /// The keyboard goes into the conversation, or back to the workspace.
    TalkKeys {
        on: bool,
    },
    /// The drawer's edge was dragged: its width in points.
    TalkResize {
        width: f32,
    },
    /// Widen the drawer to half the window, or back.
    TalkWide,
    /// A click on a conversation's cell of the right rail (or ⌘⌃1…9): that conversation in the
    /// drawer, as `TaskTalk`; the one the drawer shows closes to the rail. Only the user's.
    TalkPick {
        id: Subject,
    },
    /// The settings name a calendar feed (`on`), and whether a meeting that starts alerts.
    SetCalendar {
        on: bool,
        notify: bool,
        /// Only events with a Zoom, Meet or Teams link alert (`calendar_notify_for: "links"`).
        links_only: bool,
    },
    /// The feed was read at `at`: today's and tomorrow's events, or `None` when the read failed
    /// (the last events stay). `days`: local midnight of today, tomorrow and the day after.
    CalendarRead {
        events: Option<Vec<calendar::Event>>,
        days: [u64; 3],
        at: u64,
    },
    /// Whether the user is in a call, as the shell saw it at `at`. `None`: it could not tell.
    CallSeen {
        in_call: Option<bool>,
        at: u64,
    },
    /// The meeting banner's Dismiss: no more alerts of that event. Only the user's.
    CalendarDismiss {
        id: u64,
    },
    /// Join: the event's `https:` link opens in its app. Only the user's.
    CalendarJoin {
        id: u64,
    },
    /// Fold or unfold the Calendar section's "Tomorrow" group.
    CalendarTomorrow,
}

/// What the right drawer shows (see `Model::drawer_view`).
#[derive(PartialEq)]
struct DrawerView {
    talk: Option<(Subject, bool)>,
    compose: Option<bool>,
    capture: Option<bool>,
    brief: Option<TaskId>,
}

impl DrawerView {
    fn shows(&self) -> bool {
        self.talk.is_some() || self.compose.is_some() || self.capture.is_some() || self.brief.is_some()
    }
}

/// Who sent an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The person: their own keys, clicks and menu items, and what a label they typed activates.
    User,
    /// A client of the control socket, also for a key or click it injects.
    Socket,
    /// Nobody's move: a timer, a shell that exited, a list read again, an app that came forward.
    System,
}

/// An action and who sent it: what `Model::update` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub action: Action,
    pub origin: Origin,
}

impl Action {
    pub fn sent_by(self, origin: Origin) -> Envelope {
        Envelope { action: self, origin }
    }

    /// Whether the action acts on panes, tabs or the bar: it waits while the picker, the new-task
    /// box, the key reference or visual mode has the keyboard.
    pub fn for_panes(&self) -> bool {
        matches!(
            self,
            Action::Split { .. }
                | Action::OpenRepo { .. }
                | Action::TaskGo { .. }
                | Action::FocusList
                | Action::AppsFilter { .. }
                | Action::ListSection { .. }
                | Action::ListChoose { .. }
                | Action::SessionClick { .. }
                | Action::Close { .. }
                | Action::Focus { .. }
                | Action::FocusDir { .. }
                | Action::FocusCycle { .. }
                | Action::Resize { .. }
                | Action::SwapDir { .. }
                | Action::MovePane { .. }
                | Action::Equalize
                | Action::DropPane { .. }
                | Action::Fit { .. }
                | Action::Enter { .. }
                | Action::ToggleSection { .. }
                | Action::CloseSidebar
                | Action::NewTab
                | Action::NewEmptyTab
                | Action::SelectTab { .. }
                | Action::SelectVisibleTab { .. }
                | Action::NewTabOf { .. }
                | Action::TabPlace { .. }
                | Action::CycleTab { .. }
                | Action::CloseTab { .. }
                | Action::MarkToggle { .. }
                | Action::MarkGo { .. }
                | Action::MarkPrefix
                | Action::MarkEdit { .. }
                | Action::ListMarkEdit { .. }
                | Action::ListReorder { .. }
                | Action::CaptureBack
                | Action::WorkbookSave { .. }
                | Action::WorkbookAddTab { .. }
                | Action::WorkbookSaveAll { .. }
                | Action::WorkbookOpen { .. }
                | Action::WorkbookEdit { .. }
                | Action::TaskTalk { .. }
                | Action::SessionOpen { .. }
                | Action::TalkPick { .. }
                | Action::TalkToggle
                | Action::TalkGo
                | Action::TalkKeys { .. }
        )
    }

    /// Whether the action takes the keyboard for a box, an app or another mode. Sneak mode lets
    /// only the user do that under its labels, and the quit question under itself
    /// (`Model::refusal`). Every action is named, with no
    /// catch-all: a new one must be put on a side.
    pub fn takes_keyboard(&self) -> bool {
        match self {
            Action::Enter { .. }
            | Action::OpenPicker { .. }
            | Action::PaletteOpen
            | Action::PaletteTasks
            | Action::InspectorFilter { on: true }
            | Action::OpenCompose { .. }
            | Action::ToggleKeys
            | Action::ToggleVisual
            | Action::ToggleAxOverlay
            // The Apps section's filter is a text field: typing in it takes the keyboard.
            | Action::AppsFilter { on: true }
            | Action::QuitAsked { .. }
            | Action::MarkPrefix
            | Action::MarkEdit { .. }
            | Action::ListMarkEdit { .. }
            | Action::CaptureStart { .. }
            | Action::CaptureEdit { .. }
            | Action::CaptureTitle
            | Action::CaptureBack
            // A new workbook's name is typed in its row.
            | Action::WorkbookSave { .. }
            | Action::WorkbookSaveAll { .. }
            | Action::WorkbookEdit { .. } => true,
            | Action::TaskTalk { .. }
            | Action::SessionOpen { .. }
            | Action::TalkPick { .. }
            | Action::TalkKeys { on: true } => true,
            Action::Split { .. }
            | Action::Open { .. }
            | Action::Adopt { .. }
            | Action::Close { .. }
            | Action::Focus { .. }
            | Action::FocusDir { .. }
            | Action::FocusCycle { .. }
            | Action::Resize { .. }
            | Action::SwapDir { .. }
            | Action::MovePane { .. }
            | Action::Equalize
            | Action::DropPane { .. }
            | Action::PaneWants { .. }
            | Action::Fit { .. }
            | Action::Leave
            | Action::ToggleSection { .. }
            | Action::CloseSidebar
            | Action::NewTab
            | Action::NewEmptyTab
            | Action::SelectTab { .. }
            | Action::CycleTab { .. }
            | Action::CloseTab { .. }
            | Action::PickerQuery { .. }
            | Action::SetInstalled { .. }
            | Action::PickerMove { .. }
            | Action::PickerChoose { .. }
            | Action::ClosePicker
            | Action::PaletteMove { .. }
            | Action::PaletteChoose { .. }
            | Action::ClosePalette
            | Action::InspectorOpen { .. }
            | Action::InspectorRead { .. }
            | Action::InspectorFailed { .. }
            | Action::InspectorClose
            | Action::InspectorFold { .. }
            | Action::InspectorSelect { .. }
            | Action::InspectorMove { .. }
            | Action::InspectorFilter { on: false }
            | Action::CloseKeys
            | Action::LeaveVisual
            | Action::SetRepos { .. }
            | Action::SetRepoDirs { .. }
            | Action::RepoAdd { .. }
            | Action::RepoRemove { .. }
            | Action::OpenRepo { .. }
            | Action::RestoreTasks { .. }
            | Action::TaskAdd { .. }
            | Action::SetKnown { .. }
            | Action::SetTerminals { .. }
            | Action::ComposeEscape
            | Action::ComposeMove { .. }
            | Action::ComposeAccept { .. }
            | Action::ComposeAttach { .. }
            | Action::ComposeRef { .. }
            | Action::ComposePick { .. }
            | Action::TaskSetState { .. }
            | Action::TaskSnooze { .. }
            | Action::TaskHold { .. }
            | Action::TaskDismiss { .. }
            | Action::TaskUndo
            | Action::TaskGo { .. }
            | Action::NewTabOf { .. }
            | Action::TabPlace { .. }
            | Action::SelectVisibleTab { .. }
            | Action::Tick { .. }
            | Action::FieldEdit { .. }
            | Action::ComposeComplete
            | Action::ComposeSubmit { .. }
            | Action::ToggleGroup { .. }
            | Action::TasksView { .. }
            | Action::ToggleProject { .. }
            | Action::SessionWork { .. }
            | Action::SetSessions { .. }
            | Action::SessionQuiet { .. }
            | Action::SessionsSeen { .. }
            | Action::SessionAdopt { .. }
            | Action::SessionHide { .. }
            | Action::SessionClick { .. }
            | Action::SessionsRange { .. }
            | Action::RowSnooze { .. }
            | Action::RowHold { .. }
            | Action::RowDismiss { .. }
            | Action::CloseCompose
            | Action::TaskFromLine { .. }
            | Action::SetApps { .. }
            | Action::SetAppSettings { .. }
            | Action::SetDock { .. }
            | Action::AppPin { .. }
            | Action::AppHide { .. }
            | Action::AppsUndo
            | Action::ListPin
            | Action::AppsToggle { .. }
            | Action::AppsFilter { on: false }
            | Action::FocusList
            | Action::LeaveList
            | Action::ListMove { .. }
            | Action::ListSection { .. }
            | Action::ListChoose { .. }
            | Action::ListSnooze { .. }
            | Action::ListHold
            | Action::ListDismiss
            | Action::Sneak { .. }
            | Action::SneakEnter { .. }
            | Action::SneakDrawn { .. }
            | Action::SneakType { .. }
            | Action::SneakBackspace
            | Action::LeaveSneak
            | Action::SneakApp { .. }
            | Action::AppWheel { .. }
            | Action::PeekCheck { .. }
            | Action::QuitConfirm
            | Action::QuitCancel
            | Action::Quit
            | Action::RestoreMarks { .. }
            | Action::Titles { .. }
            | Action::Unfilled { .. }
            | Action::MarkUndo
            | Action::ListUndo
            | Action::MarkAdd { .. }
            | Action::MarkToggle { .. }
            | Action::MarkRemove { .. }
            | Action::MarkRename { .. }
            | Action::MarkGo { .. }
            | Action::MarkBind { .. }
            | Action::MarksChecked { .. }
            | Action::Fill { .. }
            | Action::MarkType { .. }
            | Action::MarkBackspace
            | Action::MarkSubmit
            | Action::MarkCancel
            | Action::ListReorder { .. }
            | Action::CaptureFacts { .. }
            | Action::CaptureContent { .. }
            | Action::CaptureTaken { .. }
            | Action::AnnotatePress { .. }
            | Action::AnnotateDrag { .. }
            | Action::AnnotateRelease { .. }
            | Action::AnnotateTool { .. }
            | Action::AnnotateUndo
            | Action::AnnotateRedo
            | Action::AnnotateRemove
            | Action::AnnotateNote { .. }
            | Action::AnnotateLeave { .. }
            | Action::CaptureRemove { .. }
            | Action::CaptureCancel
            | Action::CaptureDiscard { .. }
            | Action::BriefShow { .. }
            | Action::BriefClose
            | Action::RestoreWorkbooks { .. }
            | Action::RestoreLayout { .. }
            | Action::StartFresh
            | Action::UnfilledOpen { .. }
            | Action::WorkbookAddTab { .. }
            | Action::WorkbookOpen { .. }
            | Action::WorkbookRemove { .. }
            | Action::WorkbookFold { .. }
            | Action::WorkbookUndo => false,
            | Action::SetAgents { .. }
            | Action::ComposeEnter
            | Action::AgentStarted { .. }
            | Action::AgentSessions { .. }
            | Action::SessionMissing { .. }
            | Action::AgentExited { .. }
            | Action::SessionRead { .. }
            | Action::TaskStop { .. }
            | Action::TalkToggle
            | Action::TalkKeys { on: false }
            | Action::TalkResize { .. }
            | Action::TalkGo
            | Action::TalkWide
            | Action::SetCalendar { .. }
            | Action::CalendarRead { .. }
            | Action::CallSeen { .. }
            | Action::CalendarDismiss { .. }
            | Action::CalendarJoin { .. }
            | Action::CalendarTomorrow
            | Action::AgentEvents { .. }
            | Action::TalkSend
            | Action::TalkAnswer { .. }
            | Action::TalkExpand { .. }
            | Action::TalkDenyNote
            | Action::TalkEscape
            | Action::TalkInterrupt
            | Action::TalkView { .. } => false,
        }
    }
}

/// In this crate's tests an action alone is the user's.
#[cfg(test)]
impl From<Action> for Envelope {
    fn from(action: Action) -> Self {
        action.sent_by(Origin::User)
    }
}

/// Where a listed task goes in the Tasks section.
struct TaskPlace<'a> {
    heading: Heading,
    group: tasks::Group,
    task: &'a Task,
}

/// How the Tasks section groups its tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TasksView {
    /// Needs you, Running, Later, Done.
    #[default]
    State,
    /// A group a `#project`.
    Project,
}

/// A project of the Project view, as drawn.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectGroup {
    /// `None`: the tasks of no project, last.
    pub project: Option<String>,
    pub folded: bool,
    /// Its tasks by state: the bar's segments.
    pub bar: tasks::Segments,
    /// Its open tasks, needs you first, then running, then later. None while folded.
    pub rows: Vec<TaskRow>,
    /// How many are done, and those while the "Done" line is open.
    pub done: usize,
    pub done_open: bool,
    pub done_rows: Vec<TaskRow>,
}

/// The Tasks section's footer: today's work.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Today {
    pub done: usize,
    pub running: usize,
    pub agent_ms: u64,
}

impl Today {
    /// `2 done · 1 running · 1h 05m`.
    pub fn line(&self) -> String {
        format!("{} done · {} running · {}", self.done, self.running, progress::duration(self.agent_ms))
    }
}

/// A task's hover card: numbers and names, and its title, which is the user's own.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Card {
    pub id: TaskId,
    pub title: String,
    /// Its state's heading, and what its session does now with its model.
    pub state: &'static str,
    pub group: Option<tasks::Group>,
    pub live: Option<String>,
    pub model: Option<String>,
    /// `#project`, `>repo`, `@agent`.
    pub tags: Vec<String>,
    /// How long ago its agent first started: `3h`.
    pub started: Option<String>,
    pub agent_time: String,
    /// `2 tabs · 3 panes`.
    pub workspace: String,
    /// By model, its sub-agents' apart, over all its sessions.
    pub tokens: Vec<progress::Tokens>,
    /// Tokens in the context window of its newest session, and the window, when known.
    pub context: Option<u64>,
    pub window: Option<u64>,
}

/// What the shell must do after an update.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Opened(PaneId),
    Closed(Pane),
    /// The app pane's real window goes over the pane.
    Live(PaneId),
    /// The app pane's real window goes back off screen.
    Parked(PaneId),
    /// The user picked a running app: host its main window in a new pane.
    /// With `new_tab`, in a new tab (send `NewTab` first, once hosting has succeeded).
    HostApp {
        name: String,
        axis: Axis,
        new_tab: bool,
    },
    /// The user picked how the Tasks section groups: write `tasks_view` in the settings file.
    SaveTasksView {
        view: TasksView,
    },
    /// The user picked the range of Other sessions: write `sessions_range` in the settings file.
    SaveSessionsRange {
        range: sessions::Range,
    },
    /// The pinned or hidden apps changed: make these changes to `apps` and `apps_hidden` in
    /// the settings file, as it is then.
    SaveApps {
        changes: Vec<crate::settings::ListChange>,
    },
    /// The directories of the Repos list changed: make these changes to `repos` in the
    /// settings file, as it is then.
    SaveRepos {
        changes: Vec<crate::settings::ListChange>,
    },
    /// Say this in the status bar: why a move of the user's did nothing.
    Notice {
        text: String,
    },
    /// The user picked an installed app that is not running: start the bundle at `path` in the
    /// background and host its window when it has one.
    LaunchApp {
        name: String,
        path: String,
        axis: Axis,
        new_tab: bool,
    },
    /// The terminal's visual-mode cursor turns on or off.
    Visual {
        pane: PaneId,
        on: bool,
    },
    /// A label was typed: do what a click on that control does. `id` is the shell's own name for
    /// it (e.g. `rail:apps`), and the control was in the last frame the shell reported.
    Activate {
        id: String,
    },
    /// The user typed a label in a hosted app: press the element at `path` in `pane`'s window
    /// (`focus`: focus the text field, then `Enter` the pane as the user), if it is still the one
    /// labelled (`sneak_apps::check` against `role` and `at`). Only ever for the user's own letters.
    PressApp {
        pane: PaneId,
        path: Vec<usize>,
        role: String,
        at: crate::ax::Frame,
        focus: bool,
    },
    /// A peek starts: `pane`'s real window goes over the pane, above ottospace (its app is
    /// activated); macOS gives it the scrolls that follow. Only ever the user's own wheel.
    Peek(PaneId),
    /// End the app.
    Quit,
    /// The user chose a command in the palette: run it as its menu item does.
    Run {
        command: Command,
    },
    /// Host a window of this app: the one titled `title`, else as `marks::pick_window` says; an
    /// app that is not running is started. `into`: the empty pane it fills (`Action::Fill`),
    /// else a new pane (`Action::Open` along `axis`). `mark`: the pane mark to bind to that pane.
    /// `launch`: false leaves an app that is not running as it is, and the pane waits (a
    /// restored layout's, unless `restore_apps`).
    HostWindow {
        mark: Option<MarkId>,
        bundle: Option<String>,
        name: String,
        title: String,
        into: Option<PaneId>,
        axis: Axis,
        launch: bool,
    },
    /// Start Fresh: remove `layout.json`, and write none this run.
    ForgetLayout,
    /// Show a workbook's saved history in terminal `pane`, just opened (`Opened`), above its
    /// prompt, and type its command there without running it.
    History {
        pane: PaneId,
        history: History,
    },
    /// Freeze a picture of `pane` now, as capture `material` of `draft`, and read its text.
    Snapshot {
        draft: String,
        material: u32,
        pane: PaneId,
    },
    /// The draft is gone, or with `material` one capture of it: what was kept of it goes too.
    CaptureDropped {
        draft: String,
        material: Option<u32>,
    },
    /// The draft became task `brief.task`: what was kept of it is the task's now, with the brief.
    /// `dir`: the task's folder inside the materials folder, a new one.
    CaptureKept {
        draft: String,
        dir: String,
        brief: Box<capture::Brief>,
    },
    /// Start `task`'s agent in a terminal for the right drawer, in `cwd` (none: the home
    /// folder), and answer `AgentStarted` or `AgentExited`. `brief`: the task's folder in the
    /// materials folder, if it has captures. The shell makes the session's id and asks
    /// `agents` for the command: nothing else goes into it.
    StartAgent {
        task: Subject,
        start: agents::Start,
        cwd: Option<String>,
        brief: Option<String>,
    },
    /// End `task`'s agent, and answer `AgentExited`.
    StopAgent {
        task: Subject,
    },
    /// The user's message to a native agent.
    AgentSend {
        task: Subject,
        text: String,
    },
    /// The user's answer to the request the shell numbered `ticket`: this one request, or not.
    AgentAnswer {
        task: Subject,
        ticket: u64,
        allow: bool,
        message: Option<String>,
    },
    /// End a native agent's turn; its session goes on.
    AgentInterrupt {
        task: Subject,
    },
    /// Read this one session now, off the main thread, and answer `SessionRead`.
    ReadSession {
        runner: String,
        id: String,
    },
    /// The agent started for `from` (a session, since adopted) is `to`'s now: keep its terminal
    /// under that name.
    AgentMoved {
        from: Subject,
        to: Subject,
    },
    /// A meeting started and no Zoom meeting is open: the system's notification and its sound.
    /// `title` is other people's text: shown, never logged.
    MeetingAlert {
        title: String,
        started: u64,
    },
    /// Open this `https:` link (`calendar::joinable`) with its app: the user's Join.
    OpenLink {
        url: String,
    },
}

/// One tab: its own split tree. Only the active tab is drawn.
#[derive(Debug, Serialize)]
pub struct Tab {
    pub id: u32,
    pub title: String,
    /// The task this tab belongs to. `None`: a tab of no task.
    pub task: Option<TaskId>,
    /// The tab's tree and focus while it is not the active tab. The active tab's live copies
    /// are `Model::layout` and `Model::focused`.
    #[serde(skip)]
    pub(crate) layout: Option<Node>,
    #[serde(skip)]
    pub(crate) focused: Option<PaneId>,
}

/// Where `TabPlace` puts a tab: at the end of a task's group (`None`: the group of no task),
/// or just before another tab, in that tab's group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabAt {
    End(Option<TaskId>),
    Before(u32),
}

/// The tab bar's rule, for tabs of these tasks in this order: the order they go in (indexes),
/// each task's together where its first tab is, the tabs of no task last. Stable.
pub fn bar_order(tasks: &[Option<TaskId>]) -> Vec<usize> {
    let first = |task: Option<TaskId>| tasks.iter().position(|t| *t == task).unwrap_or(0);
    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by_key(|at| (tasks[*at].is_none(), first(tasks[*at])));
    order
}

/// One task's tabs (or no task's), side by side on the bar: `tabs` index into `Model::tabs`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Group {
    pub task: Option<TaskId>,
    pub tabs: Vec<usize>,
}

pub mod zones;

pub mod chat;

#[derive(Debug, Serialize)]
pub struct Model {
    /// Panes of every tab.
    pub panes: Vec<Pane>,
    /// Never empty.
    pub tabs: Vec<Tab>,
    /// Index into `tabs`.
    pub active: usize,
    /// The active tab's split tree. `None` while it has no panes.
    pub layout: Option<Node>,
    /// The active tab's pane that splits and shortcuts act on.
    pub focused: Option<PaneId>,
    /// App pane that has the keyboard.
    pub live: Option<PaneId>,
    /// The parked app pane whose window is up for the user's scroll, until it parks again by
    /// itself. Not an entry: the mode stays as it was.
    #[serde(skip)]
    pub peek: Option<crate::app_wheel::Peek>,
    /// The tab each task showed last, to go back to it (`TaskGo`).
    #[serde(skip)]
    pub(crate) last_tab: Vec<(TaskId, u32)>,
    /// The tabs a dismissed task had, linked again if the dismissal is taken back.
    #[serde(skip)]
    unlinked: Vec<(TaskId, Vec<u32>)>,
    /// Rail section shown in the secondary bar. `None`: the bar is closed.
    pub sidebar: Option<Section>,
    /// The text in the new-task box, while it is open.
    pub compose: Option<Field>,
    /// The selected row, while the secondary bar has the keyboard.
    pub list: Option<usize>,
    /// A rail that has the keyboard (`zones`). In list mode, as the section list.
    pub rail_focus: Option<zones::RailFocus>,
    /// The row a section's list was left on.
    #[serde(skip)]
    list_kept: zones::KeptRow,
    /// The Apps section's list: names of the apps the shell can host.
    #[serde(skip)]
    pub apps: Vec<String>,
    /// The Repos section's list, as last read from disk.
    pub repos: Vec<Repo>,
    /// The directories it is read from.
    pub repo_dirs: repos::RepoDirs,
    pub tasks: Tasks,
    /// What the new-task box's tags may name, as the shell last read it.
    #[serde(skip)]
    known: compose::Known,
    /// The line `known`'s paths were read for.
    #[serde(skip)]
    known_line: Option<String>,
    /// The path at the cursor `known.entries` finish.
    #[serde(skip)]
    known_typing: Option<String>,
    #[serde(skip)]
    menu: Menu,
    /// What the line's `^references` pointed at when each went in: kept if it closes since.
    #[serde(skip)]
    compose_refs: Vec<Ref>,
    /// The paths put into the line whole (dropped, picked, pasted), as typed there: each is
    /// that very path, not one of that name inside the line's `>repo`.
    #[serde(skip)]
    compose_attached: Vec<String>,
    /// What the box says of the last thing asked of it that it could not do: "no room".
    #[serde(skip)]
    compose_note: Option<String>,
    /// The line holds text someone other than the user put there (a client's `compose`, a
    /// client's key). Such a line never becomes a capture draft's title: a brief says its ask
    /// was typed by the user.
    #[serde(skip)]
    pub(crate) compose_theirs: bool,
    /// The new-task line a conversation's opening put aside: back when the box opens empty.
    #[serde(skip)]
    compose_kept: Option<KeptDraft>,
    #[serde(skip)]
    terminals: Vec<TerminalInfo>,
    #[serde(skip)]
    home: Option<String>,
    /// Headings of the Tasks section that are collapsed. Not kept between runs.
    pub collapsed: Vec<Heading>,
    /// How the Tasks section groups its tasks. Not kept between runs, as `collapsed`.
    pub tasks_view: TasksView,
    /// Projects of the Project view that are folded, and those whose "Done" line is open.
    pub folded_projects: Vec<Option<String>>,
    pub open_done: Vec<Option<String>>,
    /// The offer to take back a snooze, hold or dismiss made with the mouse, while it shows.
    /// In the snapshot it is `tasks_bar.undo`: `notice` there is the status bar's.
    #[serde(skip)]
    pub undo_offer: Option<Notice>,
    /// Sessions the shell found, in the order listed. Shown through `sessions_bar` only.
    #[serde(skip)]
    found: Vec<Found>,
    /// How long an adopted session is quiet before its task needs the user. 0: never.
    #[serde(skip)]
    session_quiet_ms: u64,
    /// Sessions the user hid, as (runner, id). Not kept between runs.
    #[serde(skip)]
    hidden: Vec<(String, String)>,
    /// Which sessions Other sessions lists (`sessions_range` in the settings), and the start of
    /// the local day the shell last read them in.
    #[serde(skip)]
    pub sessions_range: sessions::Range,
    #[serde(skip)]
    pub(crate) session_day: u64,
    /// The time of the last `Tick`: when a task filed now was filed. 0: never told.
    #[serde(skip)]
    pub(crate) now: u64,
    /// The pane picker, while it is open.
    pub picker: Option<Picker>,
    /// The command palette, while it is open.
    pub palette: Option<Palette>,
    /// The accessibility inspector, while it is open.
    pub inspector: Option<Inspector>,
    /// The palette was opened over the conversation with the keyboard: closing it gives it back.
    #[serde(skip)]
    palette_over_talk: bool,
    /// The apps found installed and the bundles running, as the shell last said (`SetInstalled`).
    #[serde(skip)]
    pub(crate) installed: (Vec<Installed>, Vec<String>),
    /// The Apps section's pinned, suggested and hidden apps. In the snapshot it is `apps_bar`.
    #[serde(skip)]
    pub app_lists: apps::Lists,
    /// The quit question, while it is up.
    pub confirm: Option<Confirm>,
    /// The key reference overlay is showing.
    pub keys: bool,
    /// The terminal pane in visual mode.
    pub visual: Option<PaneId>,
    /// The accessibility debug overlay is showing.
    pub ax_overlay: bool,
    /// Sneak mode's labels, while it is on.
    #[serde(skip)]
    pub sneak: Option<Sneak>,
    /// The marks. Not in the state as they are kept: `mark_rows` is what it shows.
    #[serde(skip)]
    pub marks: Marks,
    /// The workbooks. Not in the state as they are kept: `workbook_rows` is what it shows.
    #[serde(skip)]
    pub workbooks: Workbooks,
    /// What a mark waits for, while one has the keyboard. Also a workbook's name being typed.
    pub mark_input: Option<MarkInput>,
    /// The draft in the right bar, while there is one: `compose` is its title.
    pub capture: Option<Collection>,
    /// The app pane the capture key took the keyboard from: the way back is offered.
    pub back_to: Option<PaneId>,
    /// The task whose brief the right bar shows.
    pub brief: Option<TaskId>,
    /// The conversation the right drawer is on. `None`: no drawer and no right rail.
    pub talk: Option<Talk>,
    /// The drawer's width in points, and whether it is widened to half the window.
    pub talk_width: f32,
    pub talk_wide: bool,
    /// The conversations whose agent runs in a terminal of the drawer's, in the order they started.
    pub agents: Vec<Subject>,
    /// The conversations the user opened in this run, the last opened last.
    #[serde(skip)]
    pub(crate) opened: Vec<Subject>,
    /// Agents started whose session's id is not known yet: their runner names it.
    #[serde(skip)]
    seeking: Vec<agents::Seek>,
    /// Those asked for and not yet started, each with the runner of a new session.
    #[serde(skip)]
    starting: Vec<(Subject, Option<RunnerId>)>,
    /// Conversations the user asked to go on with whose session is not listed (the Tasks section
    /// is closed): waiting on `SessionRead`, which says where it started and if it is in use.
    #[serde(skip)]
    awaiting: Vec<Subject>,
    /// What `SessionRead` said of a session (`None`: no listed checkout has it), by runner and id.
    #[serde(skip)]
    read_sessions: Vec<(String, String, Option<Found>)>,
    /// Sessions whose agent in the drawer ended, and when (`now` then): its last writes were ours.
    #[serde(skip)]
    ended: Vec<(String, String, u64)>,
    /// The title of each session conversation opened this run, as it was when opened.
    #[serde(skip)]
    talk_titles: Vec<(String, String, String)>,
    #[serde(skip)]
    models: agents::Models,
    #[serde(skip)]
    default_agent: Option<String>,
    /// The conversations drawn natively, with their transcripts and drafts (`chat`).
    #[serde(skip)]
    chats: Vec<chat::Chat>,
    /// The view the user picked for a conversation, and the settings' default for Claude Code.
    #[serde(skip)]
    views: Vec<(Subject, crate::transcript::View)>,
    #[serde(skip)]
    conversation_view: crate::transcript::View,
    /// Conversations whose agent was stopped for a switch of view: gone on with once it is gone.
    #[serde(skip)]
    switching: Vec<Subject>,
    /// The action being run is the user's own. False between actions.
    #[serde(skip)]
    by_user: bool,
    #[serde(skip)]
    pub(crate) next_id: PaneId,
    #[serde(skip)]
    next_tab: u32,
    /// The layout file as it was last told to the shell, but for what the shell reads.
    #[serde(skip)]
    pub(crate) layout_seen: Option<crate::layout_file::Layout>,
    /// False after Start Fresh: no change is told to the shell.
    #[serde(skip)]
    pub(crate) layout_kept: bool,
    /// The meeting timeline. Not in the state: its text is other people's (`calendar_snapshot`).
    #[serde(skip)]
    pub calendar: Calendar,
}

impl Default for Model {
    fn default() -> Self {
        let mut model = Self {
            panes: Vec::new(),
            tabs: Vec::new(),
            active: 0,
            layout: None,
            focused: None,
            live: None,
            peek: None,
            sidebar: None,
            compose: None,
            list: None,
            rail_focus: None,
            list_kept: zones::KeptRow::default(),
            apps: Vec::new(),
            repos: Vec::new(),
            repo_dirs: repos::RepoDirs::default(),
            tasks: Tasks::default(),
            known: compose::Known::default(),
            known_line: None,
            known_typing: None,
            menu: Menu::default(),
            compose_refs: Vec::new(),
            compose_attached: Vec::new(),
            compose_note: None,
            compose_theirs: false,
            compose_kept: None,
            terminals: Vec::new(),
            home: None,
            // Done is at the bottom, folded: what is finished is counted, not listed.
            collapsed: vec![Heading::Done, Heading::Sessions],
            tasks_view: TasksView::State,
            folded_projects: Vec::new(),
            open_done: Vec::new(),
            undo_offer: None,
            found: Vec::new(),
            session_quiet_ms: sessions::QUIET_MS,
            hidden: Vec::new(),
            sessions_range: sessions::Range::default(),
            session_day: 0,
            now: 0,
            picker: None,
            palette: None,
            inspector: None,
            palette_over_talk: false,
            installed: (Vec::new(), Vec::new()),
            app_lists: apps::Lists::default(),
            confirm: None,
            keys: false,
            visual: None,
            ax_overlay: false,
            sneak: None,
            marks: Marks::default(),
            workbooks: Workbooks::default(),
            mark_input: None,
            capture: None,
            back_to: None,
            brief: None,
            talk: None,
            talk_width: talk::WIDTH,
            talk_wide: false,
            agents: Vec::new(),
            opened: Vec::new(),
            seeking: Vec::new(),
            starting: Vec::new(),
            awaiting: Vec::new(),
            read_sessions: Vec::new(),
            ended: Vec::new(),
            talk_titles: Vec::new(),
            models: agents::Models::default(),
            default_agent: None,
            chats: Vec::new(),
            views: Vec::new(),
            conversation_view: crate::transcript::View::Native,
            switching: Vec::new(),
            by_user: false,
            next_id: 0,
            last_tab: Vec::new(),
            unlinked: Vec::new(),
            next_tab: 0,
            layout_seen: None,
            layout_kept: true,
            calendar: Calendar::default(),
        };
        model.push_tab();
        model
    }
}

/// A task's title as a tab's: at most 24 characters.
fn short_title(title: &str) -> String {
    match title.char_indices().nth(24) {
        Some((cut, _)) => format!("{}…", title[..cut].trim_end()),
        None => title.to_string(),
    }
}

impl Model {
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// What the right drawer shows, and whether it has the keyboard.
    fn drawer_view(&self) -> DrawerView {
        DrawerView {
            talk: self.talk.as_ref().filter(|talk| talk.open).map(|talk| (talk.task.clone(), talk.keys)),
            compose: self.compose.is_some().then(|| self.compose_keys()),
            capture: self.capture.as_ref().map(|draft| draft.typing),
            brief: self.brief,
        }
    }

    pub fn mode(&self) -> Mode {
        if self.confirm.is_some() {
            Mode::Confirm
        } else if self.sneak.is_some() {
            Mode::Sneak
        } else if self.mark_input.is_some() {
            Mode::Mark
        } else if self.picker.is_some() {
            Mode::Pick
        } else if self.palette.is_some() {
            Mode::Palette
        } else if self.compose_keys() {
            Mode::Compose
        } else if self.talk.as_ref().is_some_and(|talk| talk.keys) {
            Mode::Talk
        } else if self.inspector.as_ref().is_some_and(|i| i.filtering) {
            Mode::Inspect
        } else if self.live.is_some() {
            Mode::App
        } else if self.annotated().is_some() {
            Mode::Annotate
        } else if self.visual.is_some() {
            Mode::Visual
        } else if self.list.is_some() || self.rail_focus.is_some() {
            Mode::List
        } else {
            Mode::Normal
        }
    }

    /// The terminal that ⌘C copies from and ⌘V pastes into: the focused pane, if it is a
    /// terminal and ottospace has the keyboard (no picker, key reference or live app).
    pub fn keyboard_terminal(&self) -> Option<PaneId> {
        let id = self.focused.filter(|_| self.mode() == Mode::Normal && !self.keys)?;
        self.pane(id).filter(|p| p.kind == Kind::Terminal).map(|p| p.id)
    }

    /// The new-task box, the picker, the key reference, the Apps section's filter, visual
    /// mode, sneak mode or the quit question has the keyboard.
    fn holds_keyboard(&self) -> bool {
        let boxed = self.compose_keys() || self.picker.is_some() || self.keys || self.annotated().is_some();
        let boxed = boxed || self.palette.is_some() || self.inspector.as_ref().is_some_and(|i| i.filtering);
        // The Apps section's filter is a text field with the keyboard, as the picker's is.
        let filtering = self.app_lists.filtering;
        boxed || filtering || self.visual.is_some() || self.sneak.is_some() || self.confirm.is_some()
    }

    /// Why an action does nothing because of who sent it. Only the user may take the keyboard
    /// from under sneak mode's labels or the quit question, or close the picker by opening the
    /// key reference.
    /// `None`: the origin does not stop it. The shell answers a socket request with the reason.
    pub fn refusal(&self, action: &Action, origin: Origin) -> Option<&'static str> {
        let boxed = self.picker.is_some() || self.compose_keys();
        match action {
            _ if origin == Origin::User => None,
            // So does a scroll in another app.
            Action::AppWheel { .. } => Some(crate::app_wheel::USERS_ALONE),
            // A label in another app presses there: only the user's own letters spell one.
            Action::SneakType { text } if self.sneak.as_ref().is_some_and(|s| s.reaches_app(text)) => {
                Some(crate::sneak_apps::USERS_ALONE)
            }
            // A pin, an unpin, a hidden suggestion and their undo write the user's settings
            // file: no client's key or label does that.
            Action::AppPin { .. } | Action::AppHide { .. } | Action::AppsUndo | Action::ListPin => {
                Some(apps::USERS_ALONE)
            }
            // So do a directory added to the Repos list and one taken off it.
            Action::RepoAdd { .. } | Action::RepoRemove { .. } => Some(apps::USERS_ALONE),
            // ↵ on a meeting joins it, as its Join does.
            Action::ListChoose { .. } if self.sidebar == Some(Section::Calendar) && self.rail_focus.is_none() => {
                Some(calendar::USERS_ALONE)
            }
            // A meeting's link opens only by the user's own click or key; nor does a client
            // silence an alert meant for the user.
            Action::CalendarJoin { .. } | Action::CalendarDismiss { .. } => Some(calendar::USERS_ALONE),
            Action::ListDismiss | Action::ListUndo if self.sidebar == Some(Section::Apps) => Some(apps::USERS_ALONE),
            // The layout file is read by the shell at start, and its app panes start the user's apps.
            Action::RestoreLayout { .. } if origin == Origin::Socket => Some(crate::layout_file::USERS_ALONE),
            Action::StartFresh | Action::UnfilledOpen { .. } if origin != Origin::User => {
                Some(crate::layout_file::USERS_ALONE)
            }
            // A workbook is the user's file, and opening one starts their apps.
            Action::WorkbookSave { .. }
            | Action::WorkbookAddTab { .. }
            | Action::WorkbookSaveAll { .. }
            | Action::WorkbookOpen { .. }
            | Action::WorkbookEdit { .. }
            | Action::WorkbookRemove { .. }
            | Action::WorkbookUndo => Some(workbook::USERS_ALONE),
            Action::ListChoose { .. } | Action::ListDismiss | Action::ListUndo | Action::ListMarkEdit { .. }
                if self.sidebar == Some(Section::Workbook) =>
            {
                Some(workbook::USERS_ALONE)
            }
            Action::FieldEdit { .. }
            | Action::MarkSubmit
            | Action::MarkType { .. }
            | Action::MarkBackspace
            | Action::MarkCancel
            | Action::MarkPrefix
            | Action::MarkEdit { .. }
                if matches!(self.mark_input, Some(MarkInput::Workbook { .. })) =>
            {
                Some(workbook::USERS_ALONE)
            }
            // An agent runs with the user's rights: no client's request, key or label starts
            // or stops one. A client's ↵ in the box files the task and starts none (`start`).
            Action::TaskTalk { .. }
            | Action::SessionOpen { .. }
            | Action::TalkPick { .. }
            | Action::TaskStop { .. }
            | Action::TalkGo => Some(agents::USERS_ALONE),
            // "Stop its agent and dismiss?" is the user's to answer yes to.
            Action::QuitConfirm | Action::QuitCancel
                if self.confirm.as_ref().is_some_and(|confirm| confirm.dismiss.is_some()) =>
            {
                Some(agents::USERS_ALONE)
            }
            // Dismissing a task whose agent runs would stop the agent: the user is asked.
            Action::TaskDismiss { id } | Action::RowDismiss { id } if self.agent_runs(&Subject::Task(*id)) => {
                Some(agents::USERS_ALONE)
            }
            Action::ListDismiss if self.selected_task().is_some_and(|id| self.agent_runs(&Subject::Task(id))) => {
                Some(agents::USERS_ALONE)
            }
            // Which task a tab is of is the user's to say.
            Action::TabPlace { .. } | Action::NewTabOf { task: Some(_) } => Some(TABS_USERS),
            // What reaches a native agent, or stops its turn, is the user's: a message is as
            // strong as typing into it, an answer grants a tool call.
            Action::TalkSend
            | Action::TalkAnswer { .. }
            | Action::TalkExpand { .. }
            | Action::TalkDenyNote
            | Action::TalkEscape
            | Action::TalkInterrupt
            | Action::TalkView { .. } => Some(chat::USERS_ALONE),
            // So is the composer's text: what ↵ sends must be the user's own words.
            Action::FieldEdit { .. } if self.field().is_some_and(|(of, _)| of == "talk") => Some(chat::USERS_ALONE),
            // Only the shell's runner reports events (System); a client never does.
            // Only the shell's runner reports an agent's events (a user's is dropped in `chat_act`).
            Action::AgentEvents { .. } if origin == Origin::Socket => Some(chat::RUNNERS_ALONE),
            // On a rail list keys move over the rail: the user's, as the move that got there.
            Action::ListMove { .. } | Action::ListChoose { .. } | Action::LeaveList if self.rail_focus.is_some() => {
                Some(zones::RAILS_USERS)
            }
            action if self.sneak.is_some() && action.takes_keyboard() => Some("sneak mode is on"),
            // A capture hands over a pane's picture and its text: no client's request, key or
            // label asks for one. Sneak mode and the quit question say their own reason first.
            Action::CaptureStart { .. } if self.confirm.is_none() => Some(capture::USERS_ALONE),
            // Its brief says the ask, the notes and the drawings are the user's: so they are.
            action if self.capture.is_some() && self.drafts(action) => Some(capture::DRAFT_USERS),
            action if self.confirm.is_some() && action.takes_keyboard() => Some(CONFIRM_UP),
            // A key a client presses is input to the wait, as a label's letter is to sneak mode.
            Action::MarkPrefix | Action::MarkEdit { .. } | Action::ListMarkEdit { .. } => None,
            action if self.mark_input.is_some() && action.takes_keyboard() => Some(marks::INPUT_ON),
            // The palette runs any command as the user's: no client opens it, types in it, moves
            // in it or chooses. Nor does a client close it from under the user.
            Action::PaletteOpen
            | Action::PaletteTasks
            | Action::PaletteMove { .. }
            | Action::PaletteChoose { .. }
            | Action::ClosePalette => Some(commands::USERS_ALONE),
            Action::FieldEdit { .. } if self.field().is_some_and(|(of, _)| of == "palette") => {
                Some(commands::USERS_ALONE)
            }
            Action::ToggleKeys if self.palette.is_some() => Some(commands::PALETTE_OPEN),
            // The inspector shows another app's tree: only the user opens it and moves in it, and
            // only the shell's own read fills it. It adds nothing a client can read or do.
            Action::InspectorOpen { .. }
            | Action::InspectorClose
            | Action::InspectorFold { .. }
            | Action::InspectorSelect { .. }
            | Action::InspectorMove { .. }
            | Action::InspectorFilter { .. } => Some(inspect::USERS_ALONE),
            Action::InspectorRead { .. } | Action::InspectorFailed { .. } if origin == Origin::Socket => {
                Some(inspect::USERS_ALONE)
            }
            Action::FieldEdit { .. } if self.field().is_some_and(|(of, _)| of == "inspector") => {
                Some(inspect::USERS_ALONE)
            }
            Action::ToggleKeys if boxed && !self.keys => Some("the picker or the new-task box is open"),
            // For the user it waits, as every pane action does. A client is told why.
            Action::SessionClick { .. } if boxed || self.keys || self.visual.is_some() => {
                Some("the picker, the new-task box, the key reference or visual mode has the keyboard")
            }
            _ => None,
        }
    }

    /// Whether the action types in, draws on, adds or drops the capture draft in the right bar.
    /// What the shell tells of a capture (its picture, its text, what is open) is not among them.
    fn drafts(&self, action: &Action) -> bool {
        match action {
            Action::FieldEdit { .. } => self.field().is_some_and(|(of, _)| matches!(of, "compose" | "note")),
            Action::OpenCompose { .. }
            | Action::CloseCompose
            | Action::ComposeSubmit { .. }
            | Action::ComposeEnter
            | Action::ComposeComplete
            | Action::ComposeEscape
            | Action::ComposeMove { .. }
            | Action::ComposeAccept { .. }
            | Action::ComposeAttach { .. }
            | Action::ComposeRef { .. }
            | Action::ComposePick { .. }
            | Action::CaptureCancel
            | Action::CaptureDiscard { .. }
            | Action::CaptureRemove { .. }
            | Action::CaptureEdit { .. }
            | Action::CaptureTitle
            | Action::AnnotatePress { .. }
            | Action::AnnotateDrag { .. }
            | Action::AnnotateRelease { .. }
            | Action::AnnotateTool { .. }
            | Action::AnnotateUndo
            | Action::AnnotateRedo
            | Action::AnnotateRemove
            | Action::AnnotateNote { .. }
            | Action::AnnotateLeave { .. } => true,
            _ => false,
        }
    }

    /// Why an action does nothing for now, whoever sent it: what has the keyboard that it would
    /// act behind. `None`: nothing is in its way. The shell answers a socket request with it.
    pub fn waits(&self, action: &Action) -> Option<&'static str> {
        let for_panes = action.for_panes();
        let picker = matches!(action, Action::OpenPicker { .. });
        // A close of a named pane still runs: the shell sends it when a window or shell is
        // gone, and must not leave a dead pane.
        let named_close = matches!(action, Action::Close { pane: Some(_) });
        // The key reference covers the workspace, so pane actions wait behind it, and the
        // picker cannot open under it. Visual mode holds the keyboard on its terminal the same way.
        if self.keys && (for_panes || picker) {
            return Some("the key reference is open");
        }
        if self.visual.is_some() && !named_close && (for_panes || picker) {
            return Some("visual mode is on");
        }
        // The new-task box has the keyboard, as the picker does: what is behind it waits, and
        // neither opens over the other. (The picker may open again over itself: a `picker`
        // request while it is open.) The box does not open under the key reference
        // or in visual mode.
        // A draft in the right bar is not a box: the workspace stays in use beside it.
        let modal = self.compose.is_some() && self.capture.is_none();
        // The palette opens over nothing that has the keyboard, and holds it as the picker does.
        let opens = matches!(
            action,
            Action::PaletteOpen
                | Action::PaletteTasks
                | Action::InspectorFilter { on: true }
                | Action::InspectorOpen { .. }
        );
        if opens && (self.holds_keyboard() || self.mark_input.is_some()) {
            return Some("something else has the keyboard");
        }
        let boxed = self.picker.is_some() || modal || self.palette.is_some();
        // The draft's title or a picture taking the keyboard is the box opening, in the bar.
        let opens = matches!(action, Action::OpenCompose { .. } | Action::CaptureTitle | Action::CaptureEdit { .. });
        let over_other = ((modal || self.palette.is_some()) && picker) || (boxed && opens);
        // A conversation the user opens takes the drawer from the new task being written.
        let talks = matches!(action, Action::TaskTalk { .. } | Action::SessionOpen { .. } | Action::TalkPick { .. })
            && self.picker.is_none();
        if (boxed && for_panes && !named_close && !talks) || over_other {
            return Some(match (self.picker.is_some(), self.palette.is_some()) {
                (true, _) => "the picker is open",
                (_, true) => commands::PALETTE_OPEN,
                _ => "the new-task box is open",
            });
        }
        let no_box_now = opens && (self.keys || self.visual.is_some());
        no_box_now.then_some("the key reference or visual mode is in the way")
    }

    pub fn update(&mut self, sent: impl Into<Envelope>) -> Vec<Effect> {
        let sent = sent.into();
        let (origin, line) = (sent.origin, self.compose_line().cloned());
        let states: Vec<(TaskId, State)> = self.tasks.list.iter().map(|task| (task.id, task.state)).collect();
        let listed = self.sidebar.zip(self.list);
        // A restored layout puts back its own conversation: the drawer does not follow its tab.
        let shown = self
            .tabs
            .get(self.active)
            .map(|tab| tab.id)
            .filter(|_| !matches!(sent.action, Action::RestoreLayout { .. }));
        let undo = matches!(sent.action, Action::TaskUndo | Action::ListUndo);
        // Where the keyboard was before the step: a tab change moves it out of the drawer.
        let drawer_keys = self.talk.as_ref().is_some_and(|talk| talk.open && talk.keys) || self.composing();
        let mut effects = self.step(sent);
        // A peek ends when what it was over changes: the tab, the tree, the mode.
        if let Some(peek) = &self.peek
            && !(self.peekable(peek.pane) && peek.over(self.active, self.layout.as_ref()))
        {
            self.end_peek(&mut effects);
        }
        self.tasks.finished(&states, self.now);
        // Only the user's own undo links a dismissed task's tabs again.
        self.settle_tasks(undo && origin == Origin::User);
        self.follow_tab(shown, origin, drawer_keys);
        // However the list was left (Esc, a click on a pane), ⌘⌃← comes back to its row.
        if self.list.is_none() && listed.is_some() {
            self.list_kept = zones::KeptRow(listed);
        }
        self.settle_zone();
        // A line anyone but the user changed is theirs until it is empty or gone.
        let changed = origin != Origin::User && self.compose_line() != line.as_ref();
        self.compose_theirs = (self.compose_theirs || changed) && self.compose_line().is_some_and(|l| !l.is_empty());
        effects
    }

    fn step(&mut self, sent: Envelope) -> Vec<Effect> {
        let Envelope { action, origin } = sent;
        let user = origin == Origin::User;
        let mut effects = Vec::new();
        // An edit is for the field that has the keyboard now. With none (a mark waits for a
        // letter over the filter), it ends the wait as any move of the user's does, and types nothing.
        let unfielded = matches!(action, Action::FieldEdit { .. }) && self.field().is_none();
        // Sneak mode ends on the user's own moves. A socket client's request and news never end
        // it, or the next letter of a label would reach a terminal: the labels follow what is
        // drawn instead. Its own actions say below what they do to it, whoever sent them: a
        // letter typed through the socket is input to the mode.
        let own = matches!(
            action,
            Action::Sneak { .. }
                | Action::SneakEnter { .. }
                | Action::SneakDrawn { .. }
                | Action::SneakType { .. }
                | Action::SneakBackspace
                | Action::LeaveSneak
                | Action::SneakApp { .. }
                // As the wheel over a terminal, a scroll in an app ends no mode.
                | Action::AppWheel { .. }
        );
        if user && !own {
            self.sneak = None;
        }
        // A mark's wait for a letter or a name ends the same way, and for the same reason.
        let own_mark = matches!(
            action,
            Action::MarkPrefix
                | Action::MarkEdit { .. }
                | Action::ListMarkEdit { .. }
                | Action::MarkType { .. }
                | Action::MarkBackspace
                | Action::MarkSubmit
                | Action::MarkCancel
                | Action::AppWheel { .. }
        );
        // A name being typed is a text field: an edit of it is the wait's own, too.
        let renaming = self.mark_input.as_ref().is_some_and(|input| input.text().is_some());
        let own_mark = own_mark || (renaming && matches!(action, Action::FieldEdit { .. }));
        if user && !own_mark {
            self.mark_input = None;
        }
        // The inspector's filter gives the keyboard back on the user's other moves.
        // ← and → leave it by the zones (`zones`), which know it had the keyboard.
        let own_filter = matches!(
            action,
            Action::InspectorFilter { .. }
                | Action::InspectorMove { .. }
                | Action::FocusDir { dir: Dir::Left | Dir::Right }
        ) || (matches!(action, Action::FieldEdit { .. })
            && self.field().is_some_and(|(of, _)| of == "inspector"));
        if let Some(inspector) = self.inspector.as_mut().filter(|_| user && !own_filter) {
            inspector.filtering = false;
        }
        if self.refusal(&action, origin).is_some() || unfielded {
            return effects;
        }
        // "Add and start" from anyone but the user adds the task and leaves the tabs on show
        // alone while a box or a mode holds the keyboard: the view must not change under what
        // the user is doing. Not a refusal: the task is filed all the same.
        // ↵ with no row to take is "add and start".
        let takes = || self.dropdown().and_then(|menu| menu.selected).is_some_and(|row| self.changes(row));
        let action = match action {
            Action::ComposeEnter if !takes() => Action::ComposeSubmit { open: true },
            action => action,
        };
        let action = match action {
            // Nor for the user where `TaskOpen` itself would wait, e.g. in visual mode: its
            // terminal must stay on show.
            Action::TaskFromLine { line, known, open: true }
                if (!user && self.holds_keyboard()) || self.waits(&Action::TaskGo { id: None }).is_some() =>
            {
                Action::TaskFromLine { line, known, open: false }
            }
            // The same for the draft in the bar, which can be added while the picker, the key
            // reference or visual mode has the keyboard: the task is filed, the view stays.
            Action::ComposeSubmit { open: true }
                if self.capture.is_some() && self.waits(&Action::TaskGo { id: None }).is_some() =>
            {
                Action::ComposeSubmit { open: false }
            }
            action => action,
        };
        // The picker has the keyboard. Pane and bar actions wait until it closes, so a
        // shortcut cannot change what is behind it (e.g. ⌘W closing the pane it splits).
        let for_panes = action.for_panes();
        // Visual mode holds the keyboard on its terminal the same way. A close of a named pane still
        // runs: the shell sends it when a window or shell is gone, and must not leave a dead pane.
        let named_close = matches!(action, Action::Close { pane: Some(_) });
        // The quit question has the keyboard: anything else the user does answers no, and is
        // dropped. e.g. ⌘W cancels the quit and closes nothing. A client's request or news
        // never answers for the user: it runs underneath (an agent's split, a shell that
        // exited), or was refused above if it would take the keyboard. `QuitCancel` is an
        // answer from anyone: Esc through the socket.
        let cancels = matches!(action, Action::QuitCancel);
        let answers_no = (for_panes && !named_close)
            || matches!(
                action,
                Action::QuitCancel
                    | Action::OpenPicker { .. }
                    | Action::PaletteOpen
                    | Action::PaletteTasks
                    | Action::OpenCompose { .. }
                    | Action::ToggleKeys
                    | Action::ToggleVisual
                    | Action::ToggleAxOverlay
                    | Action::CaptureStart { .. }
                    | Action::CaptureEdit { .. }
                    | Action::CaptureTitle
                    | Action::CaptureBack
            );
        if self.confirm.is_some() && (cancels || (user && answers_no)) {
            self.confirm = None;
            return effects;
        }
        // The capture key is the user's "now": what lies over the workspace gives way to it.
        // (Visual mode ends after the picture is asked for: see `capture_start`.)
        if user && matches!(action, Action::CaptureStart { .. }) {
            (self.keys, self.picker, self.palette) = (false, None, None);
        }
        let action = self.capture_route(action);
        // From the new task in the drawer, focus moves on, as the box waits pane actions.
        let across = matches!(action, Action::FocusDir { dir: Dir::Left | Dir::Right });
        let drawer = across && self.in_right_zone();
        if self.waits(&action).is_some() && !(user && drawer) {
            return effects;
        }
        // Focus past the panes' edge, and keys over a rail: the user's alone.
        if user {
            self.by_user = true;
            let crossed = self.zones(&action, &mut effects);
            self.by_user = false;
            if crossed {
                return effects;
            }
        }
        // The keyboard goes to a pane or a tab: the list gives it up.
        let leaves_list = matches!(
            action,
            Action::Split { .. }
                | Action::Focus { .. }
                | Action::FocusDir { .. }
                | Action::FocusCycle { .. }
                | Action::MovePane { .. }
                | Action::DropPane { .. }
                | Action::Enter { .. }
                | Action::NewTab
                | Action::NewEmptyTab
                | Action::SelectTab { .. }
                | Action::SelectVisibleTab { .. }
                | Action::NewTabOf { .. }
                | Action::CycleTab { .. }
                | Action::CloseSidebar
                | Action::TaskGo { .. }
                | Action::ToggleVisual
                | Action::LeaveList
                | Action::CaptureStart { .. }
                | Action::CaptureEdit { .. }
                | Action::CaptureTitle
                | Action::CaptureBack
                | Action::TaskTalk { .. }
                | Action::SessionOpen { .. }
                | Action::TalkPick { .. }
                | Action::TalkKeys { on: true }
        );
        if leaves_list {
            (self.list, self.rail_focus) = (None, None);
        }
        // The user's own move in the list keeps the selection where it is, so the next row comes
        // under it (hold one task, then the next). Any other change (an agent files a task, a
        // snooze runs out) must not slide a different row under the selection: it follows its row.
        let own_move = matches!(
            action,
            Action::ListMove { .. }
                | Action::ListSection { .. }
                | Action::ListSnooze { .. }
                | Action::ListHold
                | Action::ListDismiss
                | Action::ListPin
                | Action::AppsFilter { .. }
                | Action::ListReorder { .. }
                | Action::TaskUndo
                | Action::ListUndo
                | Action::ToggleSection { .. }
                | Action::FocusList
                | Action::RepoAdd { .. }
                | Action::SessionClick { .. }
        );
        let selected = self.list.and_then(|at| self.list_rows().into_iter().nth(at)).filter(|_| !own_move);
        // The notice offers back one move. Another move, by key or socket, is not the one it names.
        let moves = matches!(
            action,
            Action::TaskSnooze { .. }
                | Action::TaskHold { .. }
                | Action::TaskDismiss { .. }
                | Action::ListSnooze { .. }
                | Action::ListHold
                | Action::ListDismiss
        );
        let trees = self.trees();
        // The user went to a pane: the draft's title gives up the keyboard. The bar stays.
        let to_panes = for_panes || matches!(action, Action::Open { .. } | Action::PickerChoose { .. });
        if let Some(draft) = self.capture.as_mut().filter(|_| user && to_panes) {
            draft.typing = false;
        }
        self.by_user = user;
        // The inspector shows in the drawer's place: what the drawer is asked to show next takes it back.
        let (drawer, inspecting) = (self.drawer_view(), matches!(action, Action::InspectorOpen { .. }));
        self.act(action, &mut effects);
        let view = self.drawer_view();
        if !inspecting && view != drawer && view.shows() {
            self.inspector = None;
        }
        self.by_user = false;
        self.settle_capture(&mut effects);
        // Nor is there one to offer once it is taken back, or an agent's report made it stale.
        // The offer to put a mark back goes with the mark it would put back.
        let of_mark = self.undo_offer.is_some_and(|offer| offer.moved == Moved::MarkRemoved);
        let of_apps = self.undo_offer.is_some_and(|offer| offer.moved.of_apps());
        let stale = match (of_mark, of_apps) {
            (true, _) => self.marks.removed().is_none(),
            (_, true) => !self.app_lists.can_undo(),
            _ => moves || !self.tasks.can_undo(),
        };
        if stale {
            self.undo_offer = None;
        }
        // A measure is of the layout it was taken in. Once a split of its tab moves or a pane
        // comes or goes there, it says nothing: a second fit on it would move the splits again (50/50, 38/62,
        // then 23/77). The shell measures again.
        let stale = |pane: &Pane| pane.fit.is_some() && !self.tree(pane.id).is_some_and(|tree| trees.contains(tree));
        let stale: Vec<PaneId> = self.panes.iter().filter(|pane| stale(pane)).map(|pane| pane.id).collect();
        self.panes.iter_mut().filter(|pane| stale.contains(&pane.id)).for_each(|pane| pane.fit = None);
        self.mark_stuck();
        self.prune_unfilled();
        self.settle_app_filter();
        self.settle_talk();
        // The inspector goes with its pane, and gives up the keyboard to an app that takes it.
        let panes = &self.panes;
        self.inspector = self.inspector.take().filter(|i| panes.iter().any(|p| p.id == i.pane));
        if let Some(inspector) = self.inspector.as_mut().filter(|_| self.live.is_some()) {
            inspector.filtering = false;
        }
        let rows = self.list_rows();
        let followed = selected.and_then(|row| rows.iter().position(|r| *r == row));
        // Its row is gone, or the move was the user's: stay at the same place, on a row.
        let stay = |at: usize| at.min(rows.len().saturating_sub(1));
        self.list = self.list.filter(|_| self.sidebar.is_some()).map(|at| followed.unwrap_or_else(|| stay(at)));
        // A pane that closed is not asked about; with none left there is no question. Never a
        // quit: the user has not answered.
        let panes = &self.panes;
        if let Some(confirm) = &mut self.confirm {
            confirm.running.retain(|r| panes.iter().any(|p| p.id == r.pane));
            // Nor is there one once the draft it would discard, and every agent, is gone too.
            let discards = self.capture.as_ref().is_some_and(|bar| !bar.materials.is_empty());
            // "Stop its agent and dismiss?" goes when that agent stopped some other way.
            let asks = match confirm.dismiss {
                Some(id) => self.agent_runs(&Subject::Task(id)),
                None => !confirm.running.is_empty() || discards || !self.agents.is_empty(),
            };
            if !asks {
                self.confirm = None;
            }
        }
        effects
    }

    /// A typed label in an app: press its element, or focus a text field. The shell gives the app
    /// the keyboard only once the field took focus. Nothing if the pane is gone or its window is live.
    fn press_app(&mut self, target: Target, effects: &mut Vec<Effect>) {
        let crate::ax::Source::App { pane, path, role, at } = target.source else { return };
        if self.pane(pane).is_none() || self.live == Some(pane) {
            return;
        }
        let focus = crate::sneak_apps::focuses(&role);
        effects.push(Effect::PressApp { pane, path, role, at, focus });
    }

    fn act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::Split { axis, kind } => {
                self.leave(effects);
                let id = self.add(kind, self.focused, axis);
                effects.push(Effect::Opened(id));
            }
            Action::Open { kind, axis, beside } => drop(self.open(kind, axis, beside, effects)),
            Action::SetRepos { repos } => self.repos = repos::arrange(repos),
            Action::SetRepoDirs { dirs } => self.repo_dirs = dirs,
            Action::RepoAdd { dir, found } => self.repo_add(dir, found, effects),
            Action::RepoRemove { path } => self.repo_remove(&path, effects),
            Action::RestoreTasks { tasks, floor } => {
                self.tasks.restore(tasks, floor);
                // A tab of a task that is not among them is a tab of no task: `update` unlinks it.
            }
            // Only the user links a tab to a task: a client's task gets none.
            Action::TaskAdd { title, priority, state, tab: _, assign } => {
                let id = self.file(title, priority, state);
                self.tasks.assign(id, assign);
            }
            Action::SetKnown { line, typing, names, paths, entries } => {
                if let Some(names) = names {
                    self.known.names = names;
                }
                if self.compose_line() == Some(&line) {
                    (self.known.paths, self.known.entries, self.known_line) = (paths, entries, Some(line));
                    self.known_typing = typing;
                }
            }
            Action::SetTerminals { terminals, home } => (self.terminals, self.home) = (terminals, home),
            Action::TaskSetState { id, state } => self.tasks.set_state(id, state),
            Action::TaskSnooze { id, until } => self.tasks.snooze(id, until),
            Action::TaskHold { id } => self.tasks.hold(id),
            Action::TaskDismiss { id } => self.dismiss(id, None, effects),
            Action::TaskUndo => self.tasks.undo(),
            Action::RowSnooze { id, until } => self.row_move(id, Moved::Snoozed, |tasks| tasks.snooze(id, until)),
            Action::RowHold { id } => self.row_move(id, Moved::Held, |tasks| tasks.hold(id)),
            Action::RowDismiss { id } => self.dismiss(id, Some(Moved::Dismissed), effects),
            Action::Tick { now } => {
                self.now = now;
                self.tasks.tick(now);
                self.workbooks.tick(now);
                self.undo_offer = self.undo_offer.filter(|offer| offer.until > now);
                let alerts = self.calendar.tick(now);
                effects.extend(alerts.into_iter().map(|n| Effect::MeetingAlert { title: n.title, started: n.started }));
            }
            // ⌘⇧N: the next task's conversation; the tab stays.
            Action::TaskGo { id: None } if self.by_user => {
                if let Some(id) = self.tasks.next().map(|task| task.id) {
                    self.talk_to(Subject::Task(id), effects);
                }
            }
            Action::TaskGo { id } => {
                let task = id.map_or_else(|| self.tasks.next(), |id| self.tasks.get(id)).map(|task| task.id);
                if let Some(index) = task.and_then(|id| self.tab_of_task(id)) {
                    self.select_tab(index, effects);
                }
                // What was captured for it comes up with it.
                if let Some(id) = task.filter(|id| self.tab_task() == Some(*id)) {
                    let briefed = self.tasks.get(id).is_some_and(|task| !task.materials.is_empty());
                    self.brief = Some(id).filter(|_| briefed);
                }
            }
            Action::SelectVisibleTab { index } => self.select_tab(index, effects),
            Action::OpenRepo { path, axis } => {
                // Only a listed path: the list is what the user sees and can choose from.
                if let Some(name) = self.repo_name(&path) {
                    let empty_tab = self.layout.is_none();
                    self.open_terminal_at(axis, Some(path), effects);
                    if let Some(tab) = self.tabs.get_mut(self.active).filter(|_| empty_tab) {
                        tab.title = name;
                    }
                }
            }
            Action::Adopt { kind, beside } => {
                self.next_id += 1;
                let id = self.next_id;
                self.panes.push(Pane { id, kind, cwd: None, fit: None });
                // The pane joins the tab that holds `beside`, even if that tab is in the background.
                let active = self.active;
                let parked = self.tabs.iter_mut().enumerate().find(|(i, tab)| {
                    *i != active && beside.is_some_and(|b| tab.layout.as_ref().is_some_and(|l| l.leaves().contains(&b)))
                });
                let (layout, focused) = match parked {
                    Some((_, tab)) => (&mut tab.layout, &mut tab.focused),
                    None => (&mut self.layout, &mut self.focused),
                };
                *layout = Some(match layout.take() {
                    Some(root) => root.insert(beside, id, Axis::Right),
                    None => Node::Leaf(id),
                });
                *focused = focused.or(Some(id));
                effects.push(Effect::Opened(id));
            }
            Action::Close { pane } => {
                if let Some(id) = pane.or(self.focused) {
                    self.close_pane(id, effects);
                }
            }
            Action::NewTab => {
                // The user's joins the task whose tab is on show; a client links no tab.
                let task = self.tab_task().filter(|_| self.by_user);
                self.leave(effects);
                self.park_active();
                self.push_tab_of(task);
            }
            Action::NewEmptyTab => {
                self.act(Action::NewTab, effects);
                self.open(Kind::Empty, Axis::Right, None, effects);
            }
            Action::NewTabOf { task } => {
                let task = task.and_then(|id| self.tasks.get(id)).map(|task| (task.id, task.repo.clone()));
                self.leave(effects);
                self.park_active();
                self.push_tab_of(task.as_ref().map(|(id, _)| *id));
                // A task's tab: named for where it works, with a terminal there.
                if let Some((id, repo)) = task {
                    let place = repo.as_deref().and_then(|path| self.repo_name(path));
                    let title = self.tasks.get(id).map(|task| short_title(&task.title)).unwrap_or_default();
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        tab.title = place.unwrap_or(title);
                    }
                    if let Some(path) = repo.filter(|path| self.repo_listed(path)) {
                        self.open_terminal_at(Axis::Right, Some(path), effects);
                    }
                }
            }
            Action::TabPlace { tab, at } => self.place_tab(tab, at),
            Action::SelectTab { index } => self.select_tab(index, effects),
            Action::CycleTab { delta } => {
                let to = (self.active as i64 + delta as i64).rem_euclid(self.tabs.len() as i64) as usize;
                self.select_tab(to, effects);
            }
            Action::CloseTab { index } => {
                let index = index.unwrap_or(self.active);
                if index >= self.tabs.len() {
                    return;
                }
                self.leave(effects);
                self.park_active();
                let was_on_show = index == self.active;
                let tab = self.tabs.remove(index);
                self.last_tab.retain(|(_, id)| *id != tab.id);
                // A closed tab's id is used again: no dismissal taken back may link the new one.
                self.unlinked.iter_mut().for_each(|(_, ids)| ids.retain(|id| *id != tab.id));
                self.marks.forget_tab(tab.id);
                self.workbooks.forget_tab(tab.id);
                for id in tab.layout.as_ref().map(Node::leaves).unwrap_or_default() {
                    self.close_pane(id, effects);
                }
                if !was_on_show {
                    // The tab on show stays; it may have moved one place left.
                    self.active -= usize::from(index < self.active);
                    self.unpark_active();
                    return;
                }
                // Its right neighbour takes its place, else the left one. The last tab closed gets
                // an empty one, so there is always a tab on show.
                match index.min(self.tabs.len().saturating_sub(1)) {
                    _ if self.tabs.is_empty() => {
                        self.next_tab = 0;
                        self.push_tab();
                    }
                    next => {
                        self.active = next;
                        self.unpark_active();
                    }
                }
            }
            Action::Focus { pane } => self.focus(pane, effects),
            Action::FocusDir { dir } => {
                let to = self.focused.and_then(|from| self.layout.as_ref()?.neighbor(from, dir));
                if let Some(to) = to {
                    self.focus(to, effects);
                }
            }
            Action::FocusCycle { delta } => {
                let order = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
                let at = self.focused.and_then(|f| order.iter().position(|id| *id == f));
                if let Some(at) = at {
                    let to = (at as i64 + delta as i64).rem_euclid(order.len() as i64) as usize;
                    self.focus(order[to], effects);
                }
            }
            Action::Resize { dir } => {
                if let (Some(pane), Some(layout)) = (self.focused, &mut self.layout) {
                    layout.resize(pane, dir);
                }
            }
            Action::SwapDir { dir } => {
                let to = self.focused.and_then(|from| Some((from, self.layout.as_ref()?.neighbor(from, dir)?)));
                if let Some((from, to)) = to {
                    self.swap_panes(from, to);
                }
            }
            Action::MovePane { pane, tab } => {
                let to = Some(tab).filter(|tab| *tab < self.tabs.len());
                if let (Some(id), Some(to)) = (pane.or(self.focused), to) {
                    self.move_pane(id, to, effects);
                }
            }
            Action::Equalize => {
                if let Some(layout) = &mut self.layout {
                    layout.equalize();
                }
            }
            Action::DropPane { source, target, zone } => self.drop_pane(source, target, zone, effects),
            Action::PaneWants { pane, content, outer, window } => {
                // Only an app pane has a window to measure: a fit is never a terminal's.
                let app = |p: &&mut Pane| p.id == pane && matches!(p.kind, Kind::App { .. });
                if let Some(pane) = self.panes.iter_mut().find(app) {
                    pane.fit = hosted::fit(content, outer, window);
                }
            }
            Action::Fit { pane } => {
                let fit = pane.or(self.focused).and_then(|id| Some((id, self.pane(id)?.fit.clone()?)));
                if let Some((id, fit)) = fit
                    && let Some(layout) = self.tree_mut(id)
                {
                    for (at, axis) in AXES.into_iter().enumerate() {
                        if let Some(by) = fit.scale(at) {
                            layout.scale(id, axis, by);
                        }
                    }
                }
            }
            Action::Enter { pane } => {
                let is_app = self.pane(pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
                if is_app && self.live != Some(pane) {
                    // A peek of this pane becomes the entry: its window is already up.
                    self.peek = self.peek.take().filter(|peek| peek.pane != pane);
                    // A live window over a tab that does not show its pane would cover other panes.
                    self.show_tab_of(pane, effects);
                    self.leave(effects);
                    self.live = Some(pane);
                    self.focused = Some(pane);
                    self.back_to = None; // the user is in an app again: no way back to offer
                    if let Some(draft) = &mut self.capture {
                        draft.typing = false; // the app has the keyboard, not the draft's title
                    }
                    effects.push(Effect::Live(pane));
                }
            }
            Action::Leave => self.leave(effects),
            // No rail cell, no section: the Calendar is there only while a feed is set.
            Action::ToggleSection { section: Section::Calendar } if !self.calendar.on => {}
            // The bar is ottospace's own UI, so using it takes the keyboard back.
            Action::ToggleSection { section } => {
                self.leave(effects);
                self.sidebar = (self.sidebar != Some(section)).then_some(section);
                self.list = self.list.map(|_| self.first_row());
            }
            Action::OpenCompose { text } => {
                self.leave(effects);
                self.compose = Some(Field::new(&text, MAX_TITLE));
                // Paths read for another line say nothing of this one.
                (self.known.paths, self.known.entries, self.known_line) = (Vec::new(), Vec::new(), None);
                self.known_typing = None;
                self.compose_reset();
                // An empty box takes back the line a conversation put aside, with what it pointed at.
                if text.is_empty()
                    && let Some(kept) = self.compose_kept.take()
                {
                    (self.compose, self.compose_refs, self.compose_attached) =
                        (Some(kept.line), kept.refs, kept.attached);
                    self.compose_theirs = kept.theirs;
                }
            }
            // Not under the key reference or the quit question: the field is hidden there.
            Action::FieldEdit { .. } if self.field().is_none() => {}
            // To the field that has the keyboard: more than one can be open (the draft's title in
            // the right bar beside a note, a filter or a mark's name).
            Action::FieldEdit { edit } => match self.field().map(|(of, _)| of) {
                Some("mark" | "workbook") => {
                    if let Some(text) = self.mark_input.as_mut().and_then(MarkInput::text_mut) {
                        text.apply(edit);
                    }
                }
                Some("inspector") => {
                    if let Some(inspector) = &mut self.inspector {
                        inspector.filter.apply(edit);
                    }
                }
                Some("palette") => {
                    if let Some(palette) = &mut self.palette
                        && palette.query.apply(edit)
                    {
                        palette.selected = 0;
                    }
                }
                Some("picker") => {
                    // A new filter starts at its first match.
                    if let Some(picker) = &mut self.picker
                        && picker.query.apply(edit)
                    {
                        picker.selected = 0;
                    }
                }
                Some("note") => {
                    if let Some(field) = self.note_mut() {
                        field.apply(edit);
                    }
                }
                Some("apps") => drop(self.app_lists.filter.apply(edit)),
                Some("talk") => drop(self.talk_draft_mut().map(|draft| draft.apply(edit))),
                Some(_) => {
                    if let Some(exact) = self.spelled_out(&edit) {
                        // A space after a name that is one of the rows takes that row.
                        self.accept(exact);
                    } else if let Some(field) = &mut self.compose {
                        self.compose_note = None;
                        field.apply(edit);
                    }
                }
                None => {}
            },
            Action::ComposeComplete => {
                if let Some(menu) = self.dropdown().filter(|menu| !menu.rows.is_empty()) {
                    self.accept(menu.selected.unwrap_or(0));
                }
            }
            Action::ComposeAccept { index } => self.accept(index),
            Action::ComposeMove { delta } => match self.dropdown().filter(|menu| !menu.rows.is_empty()) {
                Some(menu) => {
                    let to = complete::moved(menu.selected, delta, menu.rows.len());
                    let moved = to.and_then(|at| menu.rows.get(at)).map(|row| row.name.clone());
                    self.menu = Menu { key: self.menu_key(), dismissed: false, moved };
                }
                // No dropdown: ↑ and ↓ are the line's ends, as in any one-line field.
                None => {
                    let to = if delta < 0 { Motion::Start } else { Motion::End };
                    if let Some(field) = self.compose.as_mut().filter(|_| self.picker.is_none()) {
                        field.apply(Edit::Move { to, select: false });
                    }
                }
            },
            // Under the key reference the box is hidden: Esc is the reference's there.
            Action::ComposeEscape if self.compose.is_some() && !self.composing() => {}
            Action::ComposeEscape => {
                if self.dropdown().is_some() {
                    self.menu = Menu { key: self.menu_key(), dismissed: true, moved: None };
                } else if let Some(draft) = &mut self.capture {
                    // The draft's title: Esc is the bar's ×, and "no" to its question.
                    match draft.asked {
                        true => draft.asked = false,
                        false => self.capture_cancel(effects),
                    }
                } else {
                    self.compose = None;
                    self.compose_reset();
                }
            }
            Action::ComposeSubmit { open } => {
                // ↵ on a highlighted row takes the row, unless the word is that name already
                // or the name has no room: then there is nothing to take, and ↵ adds. ⌘↵ always adds.
                if let Some(row) =
                    self.dropdown().and_then(|menu| menu.selected).filter(|row| !open && self.changes(*row))
                {
                    self.accept(row);
                    return;
                }
                let line = self.compose_line().cloned().unwrap_or_default();
                // A draft with captures needs no title typed: key, circle, ↵.
                let untitled = self.draft(&line).title.is_empty();
                let named = self.capture_title().filter(|_| untitled);
                if let Some(id) = self.file_titled(&line, None, named) {
                    self.compose = None;
                    self.compose_reset();
                    self.capture_keep(id, effects);
                    self.start(id, open, effects);
                }
            }
            // Not while the box is hidden under the key reference: nothing changes a line unseen.
            Action::ComposeAttach { .. } | Action::ComposeRef { .. } | Action::ComposePick { .. }
                if !self.composing() => {}
            Action::ComposePick { mark, name } => self.compose_pick(mark, name),
            Action::ComposeAttach { paths, home, unreadable } => {
                // A name the line cannot hold (a line break in it) or one too long to keep
                // is left out, and the box says how many were.
                let named: Vec<String> = paths
                    .iter()
                    .filter(|path| path.starts_with('/'))
                    .map(|path| short(home.as_deref(), path))
                    .collect();
                let fits = |path: &&String| compose::writable(path) && path.chars().count() <= tasks::MAX_TAG;
                let whole: Vec<String> = named.iter().filter(fits).cloned().collect();
                let left_out = unreadable + named.len() - whole.len();
                let room = self.insert_words(whole.iter().map(|path| compose::escape(path)).collect());
                self.compose_note = match (room, left_out) {
                    (false, _) if !whole.is_empty() => Some("no room in the line: nothing was added".into()),
                    (_, 0) => None,
                    (_, 1) => Some("1 left out: its name is too long, or not one a line can hold".into()),
                    (_, n) => Some(format!("{n} left out: a name too long, or not one a line can hold")),
                };
                if room {
                    self.compose_attached.retain(|path| !whole.contains(path));
                    self.compose_attached.extend(whole);
                }
            }
            Action::ComposeRef { target } => {
                let wanted = match target {
                    Pointed::Tab(id) => (RefKind::Tab, id),
                    Pointed::Pane(id) => (RefKind::Pane, id as u32),
                };
                let pointed = self.ref_candidates().into_iter().map(|(reference, _)| reference);
                if let Some(reference) = pointed.into_iter().find(|r| (r.kind, r.id) == (wanted.0, Some(wanted.1))) {
                    let room = self.insert_words(vec![format!("^{}", compose::escape(&reference.name))]);
                    self.compose_note = (!room).then(|| "no room in the line: nothing was added".into());
                    if room {
                        self.capture(reference);
                    }
                }
            }
            Action::CloseCompose if self.capture.is_some() => self.capture_cancel(effects),
            Action::CloseCompose => {
                self.compose = None;
                self.compose_reset();
            }
            Action::TaskFromLine { line, known, open } => {
                if let Some(id) = self.file_line(&line, Some(known)) {
                    self.start(id, open, effects);
                }
            }
            Action::SetSessions { found, day } => {
                self.found = sessions::kept(found);
                self.session_day = day;
                let seen = self.found.iter().map(|s| (s.runner.clone(), s.id.clone(), s.active, s.last_activity));
                self.follow_sessions(seen.collect());
            }
            Action::SessionQuiet { ms } => self.session_quiet_ms = ms,
            Action::SessionsSeen { seen } => {
                let seen = seen.into_iter().map(|s| (s.runner, s.id, s.active, Some(s.last_activity)));
                self.follow_sessions(seen.collect());
            }
            Action::SessionAdopt { runner, id } => {
                if let Some(session) = self.session(&runner, &id).cloned() {
                    // A session at work is a task that runs; an idle one is one saved for later.
                    let state = if session.active { State::Running } else { State::Captured };
                    let task = self.file(self.session_name(&session), 3, state);
                    // The listed checkout, not where in it the session started: a task opens a
                    // terminal in its repository, and its tag names it.
                    self.tasks.tag(task, None, Some(session.repo));
                    self.tasks.assign(task, Assign { agent: Some(runner.clone()), ..Assign::default() });
                    self.tasks
                        .link(task, sessions::Link { runner: runner.clone(), id: id.clone(), state: Some(state) });
                    self.tasks.started(task, session.started);
                    // Its conversation is the task's: one open in the drawer stays there. The
                    // user's adopt shows it, gone on with if nothing runs it.
                    self.talk_moves(Subject::Session { runner, id }, Subject::Task(task), effects);
                    if self.by_user {
                        self.talk_to(Subject::Task(task), effects);
                    }
                }
            }
            Action::SessionHide { runner, id } => {
                if self.session(&runner, &id).is_some() {
                    self.hidden.push((runner, id));
                }
            }
            Action::SessionClick { runner, id } => {
                let row =
                    self.list_rows().into_iter().position(|row| row == ListRow::Session(runner.clone(), id.clone()));
                let pane = self.session(&runner, &id).and_then(|s| self.session_pane(s));
                match (pane, row) {
                    (Some(pane), _) => {
                        self.list = None;
                        self.focus(pane, effects);
                    }
                    // Nothing to show: the row is selected, as list mode would.
                    (None, Some(row)) => {
                        self.leave(effects);
                        self.list = Some(row);
                    }
                    (None, None) => {}
                }
            }
            Action::SessionOpen { runner, id } => {
                let pane = self.session(&runner, &id).and_then(|s| self.session_pane(s));
                self.talk_to(Subject::Session { runner, id }, effects);
                // It runs in a terminal of the tab on show: that one is focused too.
                if let Some(pane) = pane {
                    self.focus(pane, effects);
                }
            }
            Action::SessionsRange { range } => {
                if self.by_user && range != self.sessions_range {
                    effects.push(Effect::SaveSessionsRange { range });
                }
                self.sessions_range = range;
            }
            Action::ToggleGroup { heading } => {
                let open = !self.collapsed.contains(&heading);
                self.collapsed.retain(|h| *h != heading);
                if open {
                    self.collapsed.push(heading);
                }
            }
            // List mode's selection follows its task into the other view. The user's pick is
            // written to the settings; anyone else's lasts this run.
            Action::TasksView { view } => {
                if self.by_user && view != self.tasks_view {
                    effects.push(Effect::SaveTasksView { view });
                }
                self.tasks_view = view;
            }
            Action::ToggleProject { project, done } => {
                let list = if done { &mut self.open_done } else { &mut self.folded_projects };
                match list.iter().position(|p| *p == project) {
                    Some(at) => drop(list.remove(at)),
                    None => list.push(project),
                }
            }
            Action::SessionWork { work } => {
                let Some(work) = work.kept() else { return };
                let of = |task: &Task| {
                    task.session.as_ref().is_some_and(|link| (&link.runner, &link.id) == (&work.runner, &work.session))
                };
                if let Some(task) = self.tasks.list.iter_mut().find(|task| of(task)) {
                    progress::record(&mut task.work, work);
                }
            }
            Action::SetApps { names } => self.apps = names,
            Action::SetAppSettings { pinned, hidden, suggest } => {
                self.app_lists.set(pinned, hidden, suggest);
                self.app_lists.resolve(&self.installed.0);
            }
            Action::SetDock { apps } => self.app_lists.set_dock(apps),
            Action::AppPin { path } => self.app_pin(&path, effects),
            Action::AppHide { path } => self.app_hide(&path, effects),
            Action::AppsUndo => self.apps_undo(effects),
            Action::ListPin => self.list_pin(effects),
            Action::AppsToggle { group } => self.apps_toggle(group),
            Action::AppsFilter { on } => self.apps_filter(on, effects),
            Action::FocusList if self.list.is_some() => self.list = None,
            Action::FocusList => {
                self.leave(effects);
                self.sidebar = self.sidebar.or(Some(Section::Apps));
                self.list = Some(0);
                self.list = Some(self.first_row());
            }
            Action::LeaveList => {}
            Action::ListMove { delta } => {
                // i64: `delta` comes from the socket and may be any i32.
                let rows = self.list_rows().len() as i64;
                if let Some(at) = self.list.filter(|_| rows > 0) {
                    self.list = Some((at as i64 + i64::from(delta)).rem_euclid(rows) as usize);
                }
            }
            Action::ListSection { delta } => {
                let order = self.sections();
                let at = order.iter().position(|s| Some(*s) == self.sidebar).unwrap_or(0) as i64;
                if self.list.is_some() {
                    self.sidebar = Some(order[(at + i64::from(delta)).rem_euclid(order.len() as i64) as usize]);
                    self.list = Some(self.first_row());
                }
            }
            Action::ListChoose { axis } => match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
                // The shell hosts the window, then opens the pane; the list keeps the keys if it fails.
                Some(ListRow::App(name)) => effects.push(Effect::HostApp { name, axis, new_tab: false }),
                Some(ListRow::Repo(path)) => self.act(Action::OpenRepo { path, axis }, effects),
                // The user's ↵ on a task is a click on it: its conversation. A client's goes to its tabs.
                Some(ListRow::Task(id)) if self.by_user => self.talk_to(Subject::Task(id), effects),
                Some(ListRow::Task(id)) => {
                    self.list = None;
                    self.act(Action::TaskGo { id: Some(id) }, effects);
                }
                // The selection stays on the heading: the same key unfolds it again.
                Some(ListRow::Heading(heading)) => self.act(Action::ToggleGroup { heading }, effects),
                Some(ListRow::Project(project)) => self.act(Action::ToggleProject { project, done: false }, effects),
                Some(ListRow::ProjectDone(project)) => self.act(Action::ToggleProject { project, done: true }, effects),
                // The user's ↵ is a click: its conversation. A client's: the pane it runs in, if known.
                Some(ListRow::Session(runner, id)) if self.by_user => {
                    self.act(Action::SessionOpen { runner, id }, effects)
                }
                Some(ListRow::Session(runner, id)) => {
                    if self.session(&runner, &id).is_some_and(|s| self.session_pane(s).is_some()) {
                        self.act(Action::SessionClick { runner, id }, effects);
                    }
                }
                Some(ListRow::Mark(id)) => self.mark_go(id, axis, effects),
                Some(ListRow::Workbook(id)) => self.workbook_open(id, None, self.home.clone(), effects),
                Some(ListRow::WorkbookTab(id, at)) => self.workbook_open(id, Some(at), self.home.clone(), effects),
                Some(ListRow::Closed(path)) => self.app_launch(&path, axis, effects),
                Some(ListRow::MoreApps) => self.apps_toggle(apps::Group::More),
                Some(ListRow::AllApps) => self.apps_toggle(apps::Group::All),
                // ↵ on a meeting is its Join: the user's alone (`refusal`).
                Some(ListRow::Meeting(id)) => self.act(Action::CalendarJoin { id }, effects),
                Some(ListRow::MeetingsTomorrow) => self.act(Action::CalendarTomorrow, effects),
                None => {}
            },
            Action::ListSnooze { until } => {
                if let Some(id) = self.selected_task() {
                    self.tasks.snooze(id, until);
                }
            }
            Action::ListHold => {
                if let Some(id) = self.selected_task() {
                    self.tasks.hold(id);
                }
            }
            Action::ListDismiss => {
                if let Some(id) = self.selected_task() {
                    self.dismiss(id, None, effects);
                }
                // On a mark's row the same key removes the mark.
                if let Some(id) = self.selected_mark() {
                    self.mark_remove(id);
                }
                // On a workbook's row it deletes the workbook; on one of its tabs, that tab.
                if let Some((id, tab)) = self.selected_workbook() {
                    self.workbook_remove(id, tab);
                }
                // On a suggested app's row it hides the suggestion.
                self.list_hide(effects);
            }
            Action::OpenPicker { axis, new_tab, items } => {
                self.leave(effects);
                let installed = offered(&self.installed.0, &self.installed.1, &items);
                self.picker =
                    Some(Picker { axis, new_tab, items, query: Field::new("", MAX_FILTER), selected: 0, installed });
            }
            Action::SetInstalled { apps, running } => {
                self.app_lists.resolve(&apps);
                self.installed = (apps, running);
                if let Some(picker) = &mut self.picker {
                    // The rows change under the selection: it stays on its item, so ↵ opens
                    // what was highlighted. If the item is gone, on a row.
                    let chosen = picker.matches().get(picker.selected).copied().cloned();
                    picker.installed = offered(&self.installed.0, &self.installed.1, &picker.items);
                    let rows = picker.matches();
                    let same = chosen.and_then(|chosen| rows.iter().position(|row| **row == chosen));
                    picker.selected = same.unwrap_or(picker.selected.min(rows.len().saturating_sub(1)));
                }
            }
            Action::PickerQuery { query } => {
                if let Some(picker) = &mut self.picker {
                    picker.query = Field::new(&query, MAX_FILTER);
                    picker.selected = 0;
                }
            }
            Action::PickerMove { delta } => {
                if let Some(picker) = &mut self.picker {
                    let count = picker.matches().len();
                    if count > 0 {
                        // i64: `delta` comes from the socket and may be any i32.
                        picker.selected = (picker.selected as i64 + delta as i64).rem_euclid(count as i64) as usize;
                    }
                }
            }
            Action::PickerChoose { index } => {
                let Some(picker) = self.picker.take() else { return };
                let choice = picker.matches().get(index.unwrap_or(picker.selected)).copied().cloned();
                match choice {
                    // Same rule as an app: fill the focused pane if it is empty, else split it.
                    Some(PickItem::Terminal) => {
                        // As `NewTab`: the user's joins the task of the tab on show.
                        if picker.new_tab {
                            let task = self.tab_task().filter(|_| self.by_user);
                            self.park_active();
                            self.push_tab_of(task);
                        }
                        self.open(Kind::Terminal, picker.axis, None, effects);
                    }
                    // The shell makes the tab only once the window is in hand, so a failed
                    // host does not leave an empty tab.
                    Some(PickItem::App { name }) => {
                        effects.push(Effect::HostApp { name, axis: picker.axis, new_tab: picker.new_tab })
                    }
                    Some(PickItem::Closed { name, path }) => {
                        effects.push(Effect::LaunchApp { name, path, axis: picker.axis, new_tab: picker.new_tab })
                    }
                    // Nothing matches: keep the picker open so the user can fix the query.
                    None => self.picker = Some(picker),
                }
            }
            Action::ClosePicker => self.picker = None,
            Action::PaletteOpen | Action::PaletteTasks => {
                self.palette_over_talk = self.talk.as_ref().is_some_and(|talk| talk.keys);
                self.leave(effects);
                // Live tasks in their order, then no task.
                let tasks = matches!(action, Action::PaletteTasks).then(|| {
                    let live = self.tasks.list.iter().filter(|task| !task.dismissed);
                    let live = live.map(|task| TaskChoice { task: Some(task.id), title: task.title.clone() });
                    live.chain([TaskChoice { task: None, title: "No task".into() }]).collect()
                });
                self.palette = Some(Palette { query: Field::new("", MAX_FILTER), selected: 0, tasks });
            }
            Action::PaletteMove { delta } => {
                if let Some(palette) = &mut self.palette {
                    let count = palette.count() as i64;
                    if count > 0 {
                        palette.selected = (palette.selected as i64 + delta as i64).rem_euclid(count) as usize;
                    }
                }
            }
            Action::PaletteChoose { index } => {
                let Some(mut palette) = self.palette.take() else { return };
                let at = index.unwrap_or(palette.selected);
                // Listing tasks: the tab on show moves to the end of the chosen one's group.
                if let Some(tasks) = &palette.tasks {
                    match palette.task_matches().get(at).map(|choice| choice.task) {
                        // Dismissed since the list was read: its row goes, and the palette stays.
                        Some(Some(id)) if self.tasks.live(id).is_none() => {
                            let left = tasks.iter().filter(|choice| choice.task != Some(id)).cloned().collect();
                            (palette.tasks, palette.selected) = (Some(left), 0);
                            self.palette = Some(palette);
                        }
                        Some(task) => {
                            self.palette_closed(effects);
                            self.place_tab(None, TabAt::End(task));
                        }
                        None => self.palette = Some(palette),
                    }
                    return;
                }
                match palette.matches().get(at) {
                    // The command runs from where the keyboard was.
                    Some(&command) => {
                        self.palette_closed(effects);
                        effects.push(Effect::Run { command })
                    }
                    // Nothing there: it stays open, so the query can be fixed.
                    None => self.palette = Some(palette),
                }
            }
            Action::ClosePalette => {
                self.palette = None;
                self.palette_closed(effects);
            }
            Action::InspectorOpen { pane, app } => {
                if self.pane(pane).is_some() {
                    self.inspector = Some(Inspector::new(pane, &app));
                    // The drawer it covers keeps its state, but not the keyboard.
                    if self.talk.as_ref().is_some_and(|talk| talk.keys) {
                        self.talk_keys(false, effects);
                    }
                    if let Some(draft) = &mut self.capture {
                        draft.typing = false;
                    }
                }
            }
            Action::InspectorRead { pane, tree, window, millis } => {
                if let Some(inspector) = self.inspector.as_mut().filter(|i| i.pane == pane) {
                    inspector.take(inspect::Read { tree, window, millis });
                }
            }
            Action::InspectorFailed { pane, why } => {
                if let Some(inspector) = self.inspector.as_mut().filter(|i| i.pane == pane) {
                    inspector.failed(&why);
                }
            }
            Action::InspectorClose => self.inspector = None,
            Action::InspectorFold { path } => {
                if let Some(inspector) = &mut self.inspector {
                    inspector.fold(&path);
                }
            }
            Action::InspectorSelect { path } => {
                if let Some(inspector) = &mut self.inspector {
                    inspector.select(path);
                }
            }
            Action::InspectorMove { delta } => {
                if let Some(inspector) = &mut self.inspector {
                    inspector.step(delta);
                }
            }
            Action::InspectorFilter { on } => {
                if on && self.inspector.is_some() {
                    self.leave(effects);
                }
                if let Some(inspector) = &mut self.inspector {
                    inspector.filtering = on;
                }
            }
            // The overlay is ottospace's own UI: it needs the keyboard back to be dismissed.
            Action::ToggleKeys => {
                self.keys = !self.keys;
                if self.keys {
                    // One overlay at a time, and the overlay needs the keyboard to be dismissed.
                    (self.picker, self.palette) = (None, None);
                    self.leave(effects);
                }
            }
            Action::CloseKeys => self.keys = false,
            Action::ToggleVisual => {
                if self.visual.is_some() {
                    self.leave_visual(effects);
                } else if let Some(pane) = self.keyboard_terminal() {
                    self.visual = Some(pane);
                    effects.push(Effect::Visual { pane, on: true });
                }
            }
            Action::LeaveVisual => self.leave_visual(effects),
            // It draws over the parked stream; a live window would cover it.
            Action::ToggleAxOverlay => {
                self.ax_overlay = !self.ax_overlay;
                if self.ax_overlay {
                    self.leave(effects);
                }
            }
            Action::CloseSidebar => {
                self.leave(effects);
                self.sidebar = None;
            }
            Action::Sneak { .. } if self.sneak.is_some() => self.sneak = None,
            Action::SneakEnter { .. } if self.sneak.is_some() => {}
            Action::Sneak { targets } | Action::SneakEnter { targets } => {
                if matches!(self.mode(), Mode::Normal | Mode::List | Mode::Talk) && !self.keys {
                    // An app pane in focus: its elements get labels once read, beside these.
                    let app =
                        self.focused.and_then(|id| self.pane(id)).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
                    self.sneak = Sneak::leaving_room(targets, app);
                }
                // Two sets of chips from one alphabet would not say which a letter means.
                if let Some(sneak) = &mut self.sneak {
                    sneak.hid_overlay = std::mem::take(&mut self.ax_overlay);
                }
            }
            Action::SneakDrawn { targets } => {
                if let Some(sneak) = &mut self.sneak {
                    sneak.redraw(&targets);
                }
                // No labelled control is drawn any more: there is nothing to type, as when
                // the mode would start with nothing to label.
                self.sneak = self.sneak.take().filter(|sneak| !sneak.targets.is_empty());
            }
            Action::SneakType { text } => {
                let Some(sneak) = &mut self.sneak else { return };
                let mut letters = text.chars().map(|c| sneak.type_letter(c.to_ascii_lowercase()));
                // A letter of no label leaves: the user is not where the labels are.
                // A spelled label is spent: the mode ends before the control acts.
                match letters.find(|typed| *typed != Typed::Narrowed) {
                    Some(Typed::Chosen(id)) => {
                        self.sneak = None;
                        effects.push(Effect::Activate { id });
                    }
                    Some(Typed::App(target)) => {
                        self.sneak = None;
                        self.press_app(target, effects);
                    }
                    Some(Typed::NoLabel) => self.sneak = None,
                    Some(Typed::Narrowed) | None => {}
                }
            }
            Action::AppWheel { pane, now } => {
                if let Some(peek) = self.peek.as_mut().filter(|peek| peek.pane == pane) {
                    // Came before the window was up: the peek goes on.
                    peek.last = now;
                } else if self.peekable(pane) {
                    self.end_peek(effects);
                    self.peek = Some(crate::app_wheel::Peek::new(pane, now, self.active, self.layout.clone()));
                    effects.push(Effect::Peek(pane));
                }
            }
            Action::PeekCheck { now, over, pressed, scrolled } => {
                let Some(peek) = &mut self.peek else { return };
                let pane = peek.pane;
                // Away from the pane first: a click there went to another window, not the app.
                if !over {
                    return self.end_peek(effects);
                }
                if pressed {
                    // A click or a key in the app: a normal entry, the list's keys given up as on entering.
                    (self.list, self.rail_focus) = (None, None);
                    return self.act(Action::Enter { pane }, effects);
                }
                if let Some(at) = scrolled {
                    peek.last = peek.last.max(at);
                }
                if now >= peek.last.saturating_add(crate::app_wheel::QUIET_MS) {
                    self.end_peek(effects);
                }
            }
            Action::SneakApp { pane, read } => {
                let shown = self.pane(pane).is_some() && self.live != Some(pane);
                let focused = self.focused == Some(pane);
                let Some(sneak) = &mut self.sneak else { return };
                // The focused pane on the first read; after that the pane labelled.
                let ours = sneak.app.map_or(focused, |app| app.pane == pane);
                match read {
                    Some(read) if ours && shown => {
                        let (found, total) = crate::sneak_apps::targets(pane, &read);
                        sneak.read_app(pane, found, total);
                    }
                    _ if sneak.app.is_some_and(|app| app.pane == pane) => sneak.drop_app(),
                    _ => {}
                }
                self.sneak = self.sneak.take().filter(|sneak| !sneak.targets.is_empty());
            }
            Action::SneakBackspace => {
                if let Some(sneak) = &mut self.sneak {
                    sneak.backspace();
                }
            }
            Action::LeaveSneak => self.sneak = None,
            // Nothing to lose: no terminal runs anything, and no draft holds a capture.
            Action::QuitAsked { running }
                if (running.is_empty() && self.quit_discards() == 0 && self.agents.is_empty())
                    || self.confirm.as_ref().is_some_and(|confirm| confirm.dismiss.is_none()) =>
            {
                effects.push(Effect::Quit)
            }
            Action::QuitAsked { running } => {
                // The question is ottospace's own UI: a live window would cover it.
                self.leave(effects);
                self.confirm = Some(Confirm { running, dismiss: None });
            }
            // Yes to "Stop its agent and dismiss?": the agent stops as Stop stops it, then the task goes.
            Action::QuitConfirm if let Some(id) = self.confirm.as_ref().and_then(|confirm| confirm.dismiss) => {
                self.confirm = None;
                self.act(Action::TaskStop { id: Subject::Task(id) }, effects);
                self.row_move(id, Moved::Dismissed, |tasks| tasks.dismiss(id));
            }
            Action::QuitConfirm if self.confirm.is_some() => effects.push(Effect::Quit),
            Action::QuitConfirm | Action::QuitCancel => {}
            Action::Quit => effects.push(Effect::Quit),
            Action::RestoreMarks { marks, extra } => self.restore_marks(marks, extra),
            Action::MarkAdd { of, name, key, uid, cwds } => self.mark_add(of, name, key, uid, &cwds),
            Action::MarkToggle { of, uid, cwds } => self.mark_toggle(of, uid, &cwds),
            Action::Titles { titles } => self.retitle(titles),
            Action::Unfilled { pane, why } => self.unfilled(pane, why),
            Action::MarkUndo => self.mark_undo(),
            Action::ListUndo if self.sidebar == Some(Section::Marks) => self.mark_undo(),
            Action::ListUndo if self.sidebar == Some(Section::Apps) => self.apps_undo(effects),
            Action::ListUndo if self.sidebar == Some(Section::Workbook) => self.workbook_undo(),
            Action::ListUndo => self.tasks.undo(),
            Action::MarkRemove { mark } => {
                if let Some(id) = self.marks.find(&mark) {
                    self.mark_remove(id);
                }
            }
            Action::MarkRename { mark, name } => {
                if let Some(id) = self.marks.find(&mark) {
                    self.marks.rename(id, &name);
                }
            }
            Action::MarkGo { mark, axis } => {
                if let Some(id) = self.marks.find(&mark) {
                    self.mark_go(id, axis, effects);
                }
            }
            Action::MarkBind { mark, pane } => self.mark_bind(mark, pane),
            Action::MarksChecked { missing } => self.marks.set_missing(missing),
            Action::Fill { pane, kind } => {
                let empty = self.panes.iter_mut().find(|p| p.id == pane && p.kind == Kind::Empty);
                if let Some(empty) = empty {
                    empty.kind = kind;
                    effects.push(Effect::Opened(pane));
                }
            }
            Action::MarkPrefix => self.mark_prefix(),
            Action::MarkEdit { mark, field } => {
                if let Some(id) = self.marks.find(&mark) {
                    self.mark_edit(id, field);
                }
            }
            Action::MarkType { text } => self.mark_type(&text, effects),
            Action::MarkBackspace => self.mark_backspace(),
            Action::MarkSubmit => self.mark_submit(),
            Action::MarkCancel => self.mark_input = None,
            Action::ListMarkEdit { field } => {
                if let Some(id) = self.selected_mark() {
                    self.mark_edit(id, field);
                }
                // A workbook has a name and no letter, and so has each of its tabs.
                if let Some((id, tab)) = self.selected_workbook().filter(|_| field == marks::Field::Name) {
                    self.workbook_edit(id, tab);
                }
            }
            Action::RestoreWorkbooks { workbooks } => self.restore_workbooks(workbooks),
            Action::RestoreLayout { layout, gone, home, launch } => {
                self.layout_restore(*layout, &gone, home, launch, effects)
            }
            Action::StartFresh => {
                self.layout_kept = false;
                effects.push(Effect::ForgetLayout);
            }
            Action::UnfilledOpen { pane } => self.unfilled_open(pane, effects),
            Action::WorkbookSave { uid, cwds, histories } => self.workbook_save(uid, (&cwds, &histories), effects),
            Action::WorkbookAddTab { workbook, cwds, histories } => {
                self.workbook_add_tab(workbook, (&cwds, &histories))
            }
            Action::WorkbookSaveAll { uid, cwds, histories } => {
                self.workbook_save_all(uid, (&cwds, &histories), effects)
            }
            Action::WorkbookOpen { workbook, tab, home } => self.workbook_open(workbook, tab, home, effects),
            Action::WorkbookEdit { workbook, tab } => self.workbook_edit(workbook, tab),
            Action::WorkbookRemove { workbook, tab } => self.workbook_remove(workbook, tab),
            Action::WorkbookFold { workbook } => self.workbook_fold(workbook),
            Action::WorkbookUndo => self.workbook_undo(),
            Action::ListReorder { delta } => match self.selected_mark() {
                // The selection goes with the mark.
                Some(id) => {
                    self.marks.shift(id, delta);
                    self.list = self.marks.list.iter().position(|mark| mark.id == id);
                }
                None => self.act(Action::ListMove { delta }, effects),
            },
            Action::CaptureStart { pane, draft, word, facts, images } => {
                self.capture_start(pane, (draft, word), facts, images, effects)
            }
            Action::CaptureTaken { draft, material, picture, stale } => {
                let mine = self.capture.as_mut().filter(|c| c.draft == draft);
                if let Some(material) = mine.and_then(|c| c.material_mut(material)) {
                    (material.picture, material.stale) = (Some(picture), stale);
                }
            }
            Action::CaptureFacts { facts } => {
                if let Some(draft) = &mut self.capture {
                    draft.facts = facts;
                }
            }
            Action::CaptureContent { draft, material, content } => {
                let mine = self.capture.as_mut().filter(|c| c.draft == draft);
                if let Some(material) = mine.and_then(|c| c.material_mut(material)) {
                    material.content = Some(content);
                }
            }
            Action::AnnotatePress { at, shift, alt, size } => {
                if let Some(marks) = self.marks_mut() {
                    marks.press(at, marks.tool.held(shift, alt), size);
                }
                // The press is on the picture: the keys are annotate mode's again.
                if let Some(draft) = &mut self.capture {
                    draft.typing = false;
                }
            }
            Action::AnnotateDrag { to } => drop(self.marks_mut().map(|marks| marks.drag(to))),
            Action::AnnotateRelease { at } => drop(self.marks_mut().map(|marks| marks.release(at))),
            Action::AnnotateTool { tool } => drop(self.marks_mut().map(|marks| marks.tool = tool)),
            Action::AnnotateUndo => drop(self.marks_mut().map(capture::Marks::undo)),
            Action::AnnotateRedo => drop(self.marks_mut().map(capture::Marks::redo)),
            Action::AnnotateRemove => drop(self.marks_mut().map(capture::Marks::remove)),
            Action::AnnotateNote { keep } => drop(self.marks_mut().map(|marks| marks.end_note(keep))),
            Action::AnnotateLeave { title } => {
                if let Some(draft) = self.capture.as_mut().filter(|draft| draft.annotating.is_some()) {
                    draft.stop_annotating();
                    draft.typing = title;
                    // The title is ottospace's own: an app that is live in another pane gives way.
                    if title {
                        self.leave(effects);
                    }
                }
            }
            Action::CaptureEdit { material } => self.capture_edit(material, effects),
            Action::CaptureRemove { material } => {
                if let Some(draft) = &mut self.capture
                    && draft.remove(material)
                {
                    effects.push(Effect::CaptureDropped { draft: draft.draft.clone(), material: Some(material) });
                }
            }
            Action::CaptureCancel => self.capture_cancel(effects),
            // An answer to the question only: not a way to drop a draft unasked.
            Action::CaptureDiscard { yes: true } if self.capture.as_ref().is_some_and(|draft| draft.asked) => {
                self.capture_drop(effects)
            }
            Action::CaptureDiscard { yes: true } => {}
            Action::CaptureDiscard { yes: false } => {
                if let Some(draft) = &mut self.capture {
                    draft.asked = false;
                }
            }
            Action::CaptureTitle => {
                if let Some(draft) = &mut self.capture {
                    draft.typing = true;
                    self.leave(effects);
                }
            }
            Action::CaptureBack => {
                if let Some(pane) = self.back_to.take() {
                    self.act(Action::Enter { pane }, effects);
                }
            }
            Action::BriefShow { id } => {
                let briefed = self.tasks.get(id).is_some_and(|task| !task.materials.is_empty());
                let shown =
                    self.tab_task() == Some(id) || self.talk.as_ref().is_some_and(|talk| talk.task.task() == Some(id));
                self.brief = Some(id).filter(|_| briefed && shown);
            }
            Action::BriefClose => self.brief = None,
            Action::SetAgents { claude, codex, default, shows } => {
                self.models = agents::Models { claude, codex };
                self.default_agent = default;
                self.conversation_view = shows;
            }
            action @ (Action::AgentEvents { .. }
            | Action::TalkSend
            | Action::TalkAnswer { .. }
            | Action::TalkExpand { .. }
            | Action::TalkDenyNote
            | Action::TalkEscape
            | Action::TalkInterrupt
            | Action::TalkView { .. }) => self.chat_act(action, effects),
            Action::TaskTalk { id } => self.talk_to(Subject::Task(id), effects),
            // Under the inspector the rail's cell shows its conversation, as on a closed drawer.
            Action::TalkPick { id }
                if self.talk.as_ref().is_some_and(|talk| talk.open && talk.task == id) && self.inspector.is_none() =>
            {
                if let Some(talk) = &mut self.talk {
                    (talk.open, talk.keys) = (false, false);
                }
            }
            Action::TalkPick { id } => self.talk_to(id, effects),
            Action::ComposeEnter => {
                if let Some(row) = self.dropdown().and_then(|menu| menu.selected) {
                    self.accept(row);
                }
            }
            Action::AgentStarted { task: subject, session, at } => {
                if let Some(place) = self.starting.iter().position(|(id, _)| *id == subject) {
                    let (_, runner) = self.starting.remove(place);
                    self.agents.push(subject.clone());
                    self.chat_started(&subject, runner.is_none());
                    // A session's conversation is gone on with: nothing of a task to change.
                    let Some(task) = subject.task() else { return };
                    self.tasks.started(task, Some(at).filter(|at| *at > 0));
                    // A runner that names its own session: look for it (`AgentSessions`).
                    if let Some(runner) = runner.filter(|runner| !runner.spec().chooses_session()) {
                        let cwd = self.tasks.get(task).and_then(|task| task.repo.clone());
                        self.seeking.push(agents::Seek { task, runner, cwd, since: at });
                    }
                    // Its session gone on with: the task runs, as for a new one, and follows it.
                    if runner.is_none() && self.tasks.get(task).is_some_and(|task| task.session.is_some()) {
                        self.tasks.set_state(task, State::Running);
                        self.tasks.followed(task, State::Running);
                    }
                    // A new session: the task runs, and follows the session if its id is known.
                    if let Some(runner) = runner {
                        self.tasks.set_state(task, State::Running);
                        let id = session.filter(|id| runner.spec().chooses_session() && agents::is_uuid(id));
                        if let Some(id) = id {
                            let link = sessions::Link { runner: runner.name().into(), id, state: Some(State::Running) };
                            self.tasks.link(task, link);
                        }
                    }
                }
            }
            Action::AgentSessions { found, home, now } => {
                // Every task's session, followed or not, and those of the conversations no task owns.
                let links = self.tasks.list.iter().filter_map(|task| task.session.as_ref());
                let mut taken: Vec<(String, String)> =
                    links.map(|link| (link.runner.clone(), link.id.clone())).collect();
                let open =
                    self.opened.iter().chain(&self.agents).chain(self.starting.iter().map(|(subject, _)| subject));
                taken.extend(open.filter_map(|subject| match subject {
                    Subject::Session { runner, id } => Some((runner.clone(), id.clone())),
                    Subject::Task(_) => None,
                }));
                let links = agents::find_sessions(&self.seeking, &found, home.as_deref(), &taken);
                for (task, id) in links {
                    let runner = self.seeking.iter().find(|seek| seek.task == task).map(|seek| seek.runner);
                    if let Some(runner) = runner {
                        let link = sessions::Link { runner: runner.name().into(), id, state: Some(State::Running) };
                        self.tasks.link(task, link);
                    }
                    self.seeking.retain(|seek| seek.task != task);
                }
                // Not found in a while: the task runs, with no session to follow. Not while a later
                // agent there is looked for: a session it may have been is not that one's either.
                let old = |seek: &agents::Seek| now.saturating_sub(seek.since) > agents::SEEK_MS;
                let later = |seek: &agents::Seek| {
                    let there = |other: &&agents::Seek| (other.runner, &other.cwd) == (seek.runner, &seek.cwd);
                    self.seeking.iter().filter(there).any(|other| other.since > seek.since && !old(other))
                };
                let gone: Vec<TaskId> = self.seeking.iter().filter(|s| old(s) && !later(s)).map(|s| s.task).collect();
                self.seeking.retain(|seek| !gone.contains(&seek.task));
            }
            Action::SessionMissing { task, id } => {
                if self.tasks.get(task).and_then(|task| task.session.as_ref()).is_some_and(|link| link.id == id) {
                    self.tasks.unlink(task);
                }
            }
            Action::AgentExited { task: subject } => {
                let ran = self.agents.contains(&subject);
                // A switch of view: the session goes on in the other once this one is gone.
                let switched = self.chat_exited(&subject);
                // What its session's file shows of the last moments is ours, not another terminal's.
                if let Some((runner, id)) = self.session_of(&subject).filter(|_| ran) {
                    let now = self.now;
                    self.ended.retain(|(r, i, at)| (r, i) != (&runner, &id) && now.saturating_sub(*at) <= ENDED_MS);
                    self.ended.push((runner, id, now));
                }
                self.agents.retain(|id| *id != subject);
                self.starting.retain(|(id, _)| *id != subject);
                self.awaiting.retain(|id| *id != subject);
                let Some(task) = subject.task() else {
                    return if switched {
                        self.chat_restart(subject, effects)
                    };
                };
                // No longer looked for: a later agent's session must not be taken for its.
                self.seeking.retain(|seek| seek.task != task);
                // Its program is gone: the session is no longer at work there.
                if ran && self.tasks.get(task).is_some_and(|task| task.session.is_some()) {
                    self.tasks.followed(task, State::NeedsYou);
                }
                // It was at work and is gone: the task is the user's to look at. Not one dismissed
                // with it (`Confirm::dismiss`): that would bring it back.
                if ran && self.tasks.get(task).is_some_and(|task| task.state == State::Running && !task.dismissed) {
                    self.tasks.set_state(task, State::NeedsYou);
                    self.tasks.followed(task, State::NeedsYou);
                }
                if switched {
                    self.chat_restart(subject, effects);
                }
            }
            Action::SessionRead { runner, id, found } => {
                let found = found.and_then(|found| sessions::kept(vec![found]).pop());
                let found = found.filter(|found| (&found.runner, &found.id) == (&runner, &id));
                self.read_sessions.retain(|(r, i, _)| (r, i) != (&runner, &id));
                self.read_sessions.truncate(talk::RAIL);
                self.read_sessions.insert(0, (runner.clone(), id.clone(), found));
                // The resumes the user asked for go on, now that it is known where and if.
                let key = Some((runner, id));
                let answered: Vec<Subject> =
                    self.awaiting.iter().filter(|subject| self.session_of(subject) == key).cloned().collect();
                self.awaiting.retain(|subject| !answered.contains(subject));
                // What this read found counts, whatever an older listing since said.
                let busy = self.read_sessions[0].2.as_ref().is_some_and(|found| self.in_use(found));
                for subject in answered.into_iter().filter(|_| !busy) {
                    let start = Some(&subject).filter(|s| !self.agent_runs(s)).and_then(|s| self.agent_start(s));
                    if let Some((start @ agents::Start::Resume { .. }, cwd)) = start {
                        self.starting.push((subject.clone(), None));
                        effects.push(Effect::StartAgent { task: subject, start, cwd, brief: None });
                    }
                }
            }
            Action::TaskStop { id } => {
                // A resume waiting on its read is asked for no more, nor a switch of view.
                self.awaiting.retain(|subject| *subject != id);
                self.switching.retain(|subject| *subject != id);
                if self.agent_runs(&id) {
                    effects.push(Effect::StopAgent { task: id });
                }
            }
            Action::TalkToggle => {
                if let Some(open) = self.talk.as_ref().map(|talk| !talk.open) {
                    self.talk_keys(open, effects);
                    if let Some(talk) = &mut self.talk {
                        talk.open = open;
                    }
                }
            }
            Action::TalkKeys { on } => self.talk_keys(on, effects),
            Action::TalkGo => {
                let task = self.talk.as_ref().and_then(|talk| talk.task.task());
                match (self.talk_go(), task) {
                    (Some(talk::Go::Tabs), Some(id)) => self.act(Action::TaskGo { id: Some(id) }, effects),
                    (Some(talk::Go::Claim), Some(id)) => self.place_tab(None, TabAt::End(Some(id))),
                    (Some(talk::Go::New), Some(id)) => self.act(Action::NewTabOf { task: Some(id) }, effects),
                    _ => {}
                }
            }
            Action::TalkResize { width } => self.talk_width = talk::width(width),
            Action::TalkWide => self.talk_wide = !self.talk_wide,
            Action::SetCalendar { on, notify, links_only } => {
                (self.calendar.on, self.calendar.notify, self.calendar.links_only) = (on, notify, links_only);
                if !on {
                    self.sidebar = self.sidebar.filter(|s| *s != Section::Calendar);
                    if self.rail_focus == Some(zones::RailFocus::Left(Section::Calendar)) {
                        self.rail_focus = Some(zones::RailFocus::Left(Section::Tasks));
                    }
                }
            }
            Action::CalendarRead { events, days, at } => self.calendar.read(events, days, at),
            Action::CallSeen { in_call, at } => self.calendar.call_seen(in_call, at),
            Action::CalendarDismiss { id } => self.calendar.dismiss(id),
            Action::CalendarTomorrow => self.calendar.tomorrow_open = !self.calendar.tomorrow_open,
            Action::CalendarJoin { id } => effects.extend(self.calendar.join(id).map(|url| Effect::OpenLink { url })),
        }
    }

    /// The folders and runners whose new sessions the shell should read (`AgentSessions`): an
    /// agent was started there and its session is not known yet. `None` folder: the home folder.
    pub fn seeking(&self) -> Vec<(RunnerId, Option<String>)> {
        self.seeking.iter().map(|seek| (seek.runner, seek.cwd.clone())).collect()
    }

    /// True if the conversation's agent runs in the drawer, or is about to.
    pub fn agent_runs(&self, subject: &Subject) -> bool {
        self.agents.contains(subject) || self.starting.iter().any(|(id, _)| id == subject)
    }

    /// True if its agent runs, or is to once its session is read: the conversation keeps the keyboard.
    pub fn talk_live(&self, subject: &Subject) -> bool {
        self.agent_runs(subject) || self.awaiting.contains(subject)
    }

    /// Where a session is gone on with: the directory it started in (`claude --resume` looks
    /// for a session by it), if that is in a listed repository or worktree. `None`: outside.
    fn resume_dir(&self, found: &Found) -> Option<String> {
        let inside = self.repo_listed(&found.repo) && sessions::below(&found.cwd, &found.repo);
        inside.then(|| found.cwd.clone())
    }

    /// How a conversation's agent would come up now, and where: a task's session gone on with,
    /// else a new one by its `@agent` (none: the default agent); a listed session's gone on
    /// with. `None`: a saved agent, a session at work elsewhere, or one no built-in runner can
    /// go on with: nothing is started.
    fn agent_start(&self, subject: &Subject) -> Option<(agents::Start, Option<String>)> {
        let resume = |runner: &str, id: &str| {
            let runner = RunnerId::named(runner).filter(|_| agents::is_uuid(id))?;
            let takes = runner.spec().attach.take_over_when_idle == Support::Yes;
            takes.then(|| agents::Start::Resume { runner, session: id.into() })
        };
        match subject {
            // Idle, and no process holds it: else a second program on it would clash.
            Subject::Session { runner, id } => {
                let session = self.session(runner, id).filter(|s| !self.in_use(s))?;
                Some((resume(runner, id)?, Some(self.resume_dir(session)?)))
            }
            Subject::Task(id) => {
                let task = self.tasks.get(*id)?;
                match &task.session {
                    // At work elsewhere (a terminal of the user's): a second program on it would clash.
                    Some(link) if link.state == Some(State::Running) => None,
                    Some(link) => {
                        // Where it started, if it was read (outside the listed checkouts: not at
                        // all); else the task's repository.
                        let cwd = match self.facts(&link.runner, &link.id).flatten() {
                            Some(s) if self.in_use(s) => return None,
                            Some(s) => Some(self.resume_dir(s)?),
                            None => task.repo.clone(),
                        };
                        resume(&link.runner, &link.id).map(|r| (r, cwd))
                    }
                    None => {
                        let default = self.default_agent();
                        let launch = agents::launch(task.agent.as_deref(), default, &self.models, task.title.clone());
                        launch.map(|launch| (agents::Start::New(launch), task.repo.clone()))
                    }
                }
            }
        }
    }

    /// Why the drawer has no agent for the conversation it is on, if that is so: for the shell to say.
    pub fn talk_note(&self) -> Option<String> {
        let subject = &self.talk.as_ref()?.task;
        if self.agent_runs(subject) || self.agent_start(subject).is_some() {
            return None;
        }
        // Being read: nothing to say yet.
        if self.awaiting.contains(subject) {
            return None;
        }
        let found = |runner: &str, id: &str| self.facts(runner, id).flatten();
        let live = match subject {
            Subject::Session { runner, id } => found(runner, id).is_some_and(|s| self.in_use(s)),
            Subject::Task(id) => {
                let task = self.tasks.get(*id)?;
                if let (Some(agent), None) = (&task.agent, &task.session) {
                    return Some(format!("@{agent} is a saved agent: starting one is not built yet"));
                }
                task.session.as_ref().is_some_and(|link| {
                    let held = found(&link.runner, &link.id).is_some_and(|s| self.in_use(s));
                    link.state == Some(State::Running) || held
                })
            }
        };
        Some(match (live, subject) {
            (true, _) => "its session is live in another terminal: go on with it there".into(),
            (false, Subject::Task(_)) => "this task's session cannot be gone on with from here".into(),
            (false, Subject::Session { .. }) => "this session cannot be gone on with from here".into(),
        })
    }

    /// True if the session is listed under Sessions: one Adopt can make a task of.
    pub fn session_listed(&self, runner: &str, id: &str) -> bool {
        self.session(runner, id).is_some()
    }

    /// A session conversation's name: the session's title, else its repository's; not listed now
    /// (the Tasks section closed), the name it had when opened; else its runner.
    pub fn session_title(&self, runner: &str, id: &str) -> String {
        let found = self.found.iter().find(|s| s.runner == runner && s.id == id);
        let kept = || self.talk_titles.iter().find(|(r, i, _)| (r.as_str(), i.as_str()) == (runner, id));
        match found {
            Some(s) => s.title.clone().unwrap_or_else(|| self.session_name(s)),
            None => kept().map_or_else(|| runner.to_string(), |(_, _, title)| title.clone()),
        }
    }

    /// What is known of a session: as listed, else as `SessionRead` said (`Some(None)`: read,
    /// and no listed checkout has it). `None`: not known.
    fn facts(&self, runner: &str, id: &str) -> Option<Option<&Found>> {
        if let Some(found) = self.found.iter().find(|s| s.runner == runner && s.id == id) {
            return Some(Some(found));
        }
        self.read_sessions.iter().find(|(r, i, _)| (r.as_str(), i.as_str()) == (runner, id)).map(|(.., f)| f.as_ref())
    }

    /// True if what the session's file shows of its use is ours: its agent in the drawer ended
    /// lately, and it has not been written to since.
    fn ours(&self, session: &Found) -> bool {
        self.ours_at(&session.runner, &session.id, session.last_activity)
    }

    /// `ours`, of a session by runner and id whose file last changed at `last`.
    fn ours_at(&self, runner: &str, id: &str, last: Option<u64>) -> bool {
        self.ended.iter().any(|(r, i, at)| {
            let since = last.is_none_or(|last| last <= at + ENDED_GRACE_MS);
            (r.as_str(), i.as_str()) == (runner, id) && since && self.now.saturating_sub(*at) <= ENDED_MS
        })
    }

    /// True if something other than our own ended agent has the session: at work, or held by a
    /// live process.
    pub(crate) fn in_use(&self, session: &Found) -> bool {
        (session.active && !self.ours(session)) || self.held_elsewhere(session)
    }

    /// True if a live process other than our ended agent holds the session. A read made for a
    /// resume came after our agent ended: it is believed. A listing may be older, for a while.
    fn held_elsewhere(&self, session: &Found) -> bool {
        let listed = self.found.iter().any(|s| (&s.runner, &s.id) == (&session.runner, &session.id));
        let stale = self.ended.iter().any(|(runner, id, at)| {
            (runner, id) == (&session.runner, &session.id) && self.now.saturating_sub(*at) <= ENDED_HELD_MS
        });
        session.held && !(listed && stale)
    }

    /// The agent a task with no `@agent` starts: the settings', else the default runner.
    fn default_agent(&self) -> &str {
        self.default_agent.as_deref().unwrap_or(RunnerId::DEFAULT.name())
    }

    /// The session a conversation is of: a session's own, or its task's.
    pub(crate) fn session_of(&self, subject: &Subject) -> Option<(String, String)> {
        match subject {
            Subject::Session { runner, id } => Some((runner.clone(), id.clone())),
            Subject::Task(id) => {
                let link = self.tasks.get(*id)?.session.as_ref()?;
                Some((link.runner.clone(), link.id.clone()))
            }
        }
    }

    /// The right rail's conversations, in order: those that need the user, then those whose agent
    /// runs, in the order they started, then the rest, the last opened first. At most
    /// `talk::RAIL`: past it the oldest idle ones drop off. A session's shows while it is listed
    /// or its agent runs.
    pub fn rail(&self) -> Vec<talk::Cell> {
        let mut ids: Vec<Subject> = self.opened.iter().rev().cloned().collect();
        let live = self.agents.iter().chain(self.starting.iter().map(|(id, _)| id));
        for id in live {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        // A task's while it is not dismissed; once done, while it is shown or its agent runs.
        let shown = self.talk.as_ref().filter(|talk| talk.open).map(|talk| &talk.task);
        let shows = |subject: &Subject| match subject {
            Subject::Task(id) => self.tasks.get(*id).is_some_and(|task| {
                let done = matches!(task.state, State::Done | State::Cleaned);
                !task.dismissed && (!done || shown == Some(subject) || self.agent_runs(subject))
            }),
            Subject::Session { runner, id } => self.agent_runs(subject) || self.session(runner, id).is_some(),
        };
        let needs = |subject: &Subject| {
            subject
                .task()
                .and_then(|id| self.tasks.get(id))
                .is_some_and(|task| task.group() == Some(tasks::Group::NeedsYou))
        };
        let rank = |subject: &Subject, recent: usize| match self.agents.iter().position(|id| id == subject) {
            _ if needs(subject) => (0, recent),
            Some(started) => (1, started),
            None if self.agent_runs(subject) => (1, usize::MAX),
            None => (2, recent),
        };
        let mut listed: Vec<(usize, Subject)> = ids.into_iter().filter(shows).enumerate().collect();
        listed.sort_by_key(|(recent, subject)| rank(subject, *recent));
        listed.truncate(talk::RAIL);
        // A task's badge is its own (`badges`); a session's is told apart from those too.
        let sessions: Vec<(Subject, String)> = listed
            .iter()
            .filter_map(|(_, subject)| match subject {
                Subject::Session { runner, id } => Some((subject.clone(), self.session_title(runner, id))),
                Subject::Task(_) => None,
            })
            .collect();
        let badges = self.badges(&sessions);
        let badge = |subject: &Subject| badges.iter().find(|(of, _)| of == subject).map(|(_, mono)| mono.clone());
        let monos: Vec<String> = listed.iter().map(|(_, subject)| badge(subject).unwrap_or_default()).collect();
        let shown = self.talk.as_ref().filter(|talk| talk.open).map(|talk| &talk.task);
        let cells = listed.into_iter().zip(monos).map(|((_, subject), mono)| {
            let (runner, id) = match &subject {
                Subject::Task(id) => match self.tasks.get(*id).and_then(|task| task.session.as_ref()) {
                    Some(link) => (link.runner.as_str(), link.id.as_str()),
                    None => ("", ""),
                },
                Subject::Session { runner, id } => (runner.as_str(), id.as_str()),
            };
            let found = self.found.iter().find(|found| (found.runner.as_str(), found.id.as_str()) == (runner, id));
            let active = found.is_some_and(|found| found.active);
            let (group, working, tip) = match subject.task().and_then(|id| self.tasks.get(id)) {
                Some(task) => {
                    let runs = self.agent_runs(&subject) && task.state == State::Running;
                    // A native agent's call is known from its stream before its file says it.
                    let native = self.chat(&subject).and_then(|chat| chat.transcript.live_label());
                    let live = native.or(found.and_then(|found| found.label.as_deref()));
                    (task.group(), runs || active, self.rail_tip(task, live))
                }
                None => {
                    let group = active.then_some(tasks::Group::Running);
                    (group, active, self.session_tip(&subject, runner, found))
                }
            };
            talk::Cell { shown: shown == Some(&subject), task: subject, mono, group, working, tip }
        });
        cells.collect()
    }

    /// A rail cell's tooltip: the title; what its agent does now, else its state; and where,
    /// with which agent, how old.
    fn rail_tip(&self, task: &Task, live: Option<&str>) -> String {
        let state = task.group().map_or("Not listed", tasks::Group::title);
        let shown = self.talk.as_ref().is_some_and(|t| t.task == Subject::Task(task.id));
        let line = match (self.talk_note().filter(|_| shown), live) {
            (Some(note), _) => note,
            (None, Some(live)) => live.to_string(),
            (None, None) => state.to_string(),
        };
        let place = task.repo.as_deref().map_or_else(
            || "home folder".to_string(),
            |path| self.repo_name(path).unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string()),
        );
        let default = self.default_agent();
        let agent = task.agent.as_deref().unwrap_or(default);
        let age = task.created.map(|created| format!(" · {}", tasks::age(created, self.now))).unwrap_or_default();
        format!("{}\n{line}\n{place} · @{agent}{age}", task.title)
    }

    /// A session cell's tooltip: its title; what it does now, else whether it is at work; where,
    /// with which runner, how long ago it was last at work.
    fn session_tip(&self, subject: &Subject, runner: &str, found: Option<&Found>) -> String {
        let Subject::Session { id, .. } = subject else { return String::new() };
        let shown = self.talk.as_ref().is_some_and(|t| t.task == *subject);
        let state = if found.is_some_and(|s| s.active) { "Session at work" } else { "Session" };
        let line = match (self.talk_note().filter(|_| shown), found.and_then(|s| s.label.as_deref())) {
            (Some(note), _) => note,
            (None, Some(live)) => live.to_string(),
            (None, None) => state.to_string(),
        };
        let place = found.map_or_else(String::new, |s| format!("{} · ", self.session_name(s)));
        let age = found.and_then(|s| s.last_activity).map(|at| format!(" · {}", tasks::age(at, self.now)));
        format!("{}\n{line}\n{place}@{runner}{}", self.session_title(runner, id), age.unwrap_or_default())
    }

    /// Each live (not dismissed) task's badge letters, by `talk::monograms`, oldest task first
    /// so a new task's letters give way; then those of `sessions` (a session's title), told
    /// apart from them. The same letters on the tab bar, the panes, the rows and the rail.
    pub fn badges(&self, sessions: &[(Subject, String)]) -> Vec<(Subject, String)> {
        let live: Vec<&Task> = self.tasks.list.iter().filter(|task| !task.dismissed).collect();
        let subjects = live.iter().map(|task| Subject::Task(task.id)).chain(sessions.iter().map(|(s, _)| s.clone()));
        let titles: Vec<&str> =
            live.iter().map(|task| task.title.as_str()).chain(sessions.iter().map(|(_, t)| t.as_str())).collect();
        subjects.zip(talk::monograms(&titles)).collect()
    }

    /// What the drawer header's go word (⌘↵, `TalkGo`) does for the task shown: go to its tabs;
    /// with none, claim the tab on show if it is no task's, else make it a new tab. `None`: no
    /// task is shown, or its tab is on show.
    pub fn talk_go(&self) -> Option<talk::Go> {
        let task = self.talk.as_ref().filter(|talk| talk.open)?.task.task()?;
        match self.tab_of_task(task) {
            Some(index) => (index != self.active).then_some(talk::Go::Tabs),
            None if self.tab_task().is_none() => Some(talk::Go::Claim),
            None => Some(talk::Go::New),
        }
    }

    /// Opens the drawer on a conversation, with the keyboard if its agent runs, and starts the
    /// agent if it does not. A session's opens while it is listed or its agent runs.
    fn talk_to(&mut self, subject: Subject, effects: &mut Vec<Effect>) {
        let brief = match &subject {
            Subject::Task(id) => match self.tasks.get(*id) {
                Some(task) => task.materials_dir().map(str::to_string),
                None => return,
            },
            Subject::Session { runner, id } if self.session(runner, id).is_some() || self.agent_runs(&subject) => None,
            Subject::Session { .. } => return,
        };
        // A task's session not listed now (the Tasks section is closed): where it started, and
        // whether something has it, are read first (`ReadSession`), and the resume waits on that.
        let unlisted = |(runner, id): &(String, String)| !self.found.iter().any(|s| (&s.runner, &s.id) == (runner, id));
        let task = subject.task().and_then(|id| self.tasks.get(id));
        let resumes = task.and_then(|task| task.session.as_ref()).is_some_and(|l| l.state != Some(State::Running));
        let read = self.session_of(&subject).filter(|_| resumes && !self.talk_live(&subject));
        if let Some((runner, id)) = read.filter(unlisted) {
            self.read_sessions.retain(|(r, i, _)| (r, i) != (&runner, &id));
            self.awaiting.push(subject.clone());
            effects.push(Effect::ReadSession { runner, id });
        }
        let start = Some(&subject).filter(|s| !self.talk_live(s)).and_then(|s| self.agent_start(s));
        self.leave(effects);
        self.list = None;
        // The drawer shows one thing: the new task being written gives way, kept for the box's
        // next opening. A capture's draft is not the drawer's to close.
        if self.compose_host() == Some(Host::Drawer) {
            self.put_compose_aside();
        }
        let keys = self.talk_live(&subject) || start.is_some();
        self.talk = Some(Talk { task: subject.clone(), open: true, keys });
        self.opened.retain(|seen| *seen != subject);
        self.opened.push(subject.clone());
        // Its name, kept for while it is not listed.
        if let Subject::Session { runner, id } = &subject {
            let title = self.session_title(runner, id);
            self.talk_titles.retain(|(r, i, _)| (r, i) != (runner, id));
            self.talk_titles.push((runner.clone(), id.clone(), title));
        }
        if let Some((start, cwd)) = start {
            let runner = match &start {
                agents::Start::New(launch) => Some(launch.runner),
                agents::Start::Resume { .. } => None,
            };
            let brief = brief.filter(|_| runner.is_some());
            // The user's new task takes the tab on show, if that has panes and is no task's.
            if let Some(id) = subject.task().filter(|_| runner.is_some() && self.by_user)
                && self.tab_task().is_none()
                && self.layout.is_some()
                && self.tab_of_task(id).is_none()
            {
                self.place_tab(None, TabAt::End(Some(id)));
            }
            self.starting.push((subject.clone(), runner));
            effects.push(Effect::StartAgent { task: subject, start, cwd, brief });
        }
    }

    /// A session became a task's: its conversation, open or with an agent running, is the
    /// task's from now on, and the shell keeps its terminal under the task (`AgentMoved`).
    fn talk_moves(&mut self, from: Subject, to: Subject, effects: &mut Vec<Effect>) {
        let runs = self.agent_runs(&from);
        let rename = |subject: &mut Subject| {
            if *subject == from {
                *subject = to.clone();
            }
        };
        self.agents.iter_mut().for_each(rename);
        self.opened.iter_mut().for_each(rename);
        self.starting.iter_mut().for_each(|(subject, _)| rename(subject));
        self.talk.iter_mut().for_each(|talk| rename(&mut talk.task));
        self.chat_moves(&from, &to);
        if runs {
            effects.push(Effect::AgentMoved { from, to });
        }
    }

    /// The palette closed: the keyboard goes back into the conversation it was taken from.
    fn palette_closed(&mut self, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.palette_over_talk) && self.palette.is_none() {
            self.talk_keys(true, effects);
        }
    }

    /// Closes the new-task box, its line kept for the box's next opening (`OpenCompose` empty).
    fn put_compose_aside(&mut self) {
        let line = self.compose.take().filter(|line| !line.text().is_empty());
        let (refs, attached) = (std::mem::take(&mut self.compose_refs), std::mem::take(&mut self.compose_attached));
        self.compose_kept = line.map(|line| KeptDraft { line, refs, attached, theirs: self.compose_theirs });
        self.compose_reset();
    }

    /// The keyboard goes into the open conversation, if its agent runs, or leaves it.
    fn talk_keys(&mut self, on: bool, effects: &mut Vec<Effect>) {
        let Some(talk) = &self.talk else { return };
        let on = on && self.talk_live(&talk.task) && self.annotated().is_none();
        if on {
            self.leave(effects);
            self.list = None;
        }
        if let Some(talk) = &mut self.talk {
            (talk.open, talk.keys) = (talk.open || on, on);
        }
    }

    /// The drawer is on a conversation there is, and the keyboard is in it only while it is
    /// open, its agent runs and nothing else has taken the keyboard.
    fn settle_talk(&mut self) {
        self.settle_chats();
        let tasks = &self.tasks;
        let gone = |subject: &Subject| subject.task().is_some_and(|id| tasks.get(id).is_none());
        self.agents.retain(|id| !gone(id));
        self.opened.retain(|id| !gone(id));
        self.seeking.retain(|seek| tasks.get(seek.task).is_some_and(|task| task.session.is_none()));
        self.starting.retain(|(id, _)| !gone(id));
        self.awaiting.retain(|id| !gone(id));
        self.talk = self.talk.take().filter(|talk| !gone(&talk.task));
        // A picture being drawn on keeps the keyboard, as an app, the list and visual mode do.
        let drawing = self.annotated().is_some();
        let elsewhere = self.live.is_some() || self.list.is_some() || self.visual.is_some() || drawing;
        let runs = self.talk.as_ref().is_some_and(|talk| self.talk_live(&talk.task));
        if let Some(talk) = &mut self.talk {
            talk.keys = talk.keys && talk.open && runs && !elsewhere;
        }
    }

    /// What `line` would file. A tag counts only if its name is known: `>name` a listed
    /// repository; `@name`, `$name` and `/path` what the shell last read (`SetKnown`); `^name`
    /// what the box's line pointed at, or what is open by that name.
    pub fn draft(&self, line: &str) -> Draft {
        self.draft_knowing(line, &self.known, true)
    }

    /// What `line` would file, given what was read for it. `captures`: the line is the box's
    /// own, and a `^name` may be one it kept. A line from a client is read without them.
    fn draft_knowing(&self, line: &str, known: &compose::Known, captures: bool) -> Draft {
        let (repo, pointed) =
            (|name: &str| self.repo_named(name).is_some(), |name: &str| self.pointed(name, captures).is_some());
        compose::read(line, repo, pointed, known)
    }

    /// Where the tags of the line in the new-task box are: what it draws as understood.
    pub fn compose_spans(&self) -> Vec<compose::Span> {
        let Some(line) = self.compose_line() else { return Vec::new() };
        let (repo, pointed) =
            (|name: &str| self.repo_named(name).is_some(), |name: &str| self.pointed(name, true).is_some());
        compose::read_spans(line, repo, pointed, &self.known)
    }

    /// Where the new-task box is drawn: in the right bar while a draft is collected there; else in
    /// place on the home screen while it shows, else over the workspace. `None`: it is not open.
    pub fn compose_host(&self) -> Option<Host> {
        let host = if self.capture.is_some() { Host::Bar } else { Host::Drawer };
        self.compose.as_ref().map(|_| host)
    }

    /// The new-task box's line has the keyboard: it is open, and nothing is over it.
    fn composing(&self) -> bool {
        self.field().is_some_and(|(of, _)| of == "compose")
    }

    /// What the box says of the last thing it could not do, while it says it.
    pub fn compose_note(&self) -> Option<&str> {
        self.compose_note.as_deref().filter(|_| self.compose.is_some())
    }

    /// The paths in the box's line that were put in whole: the shell looks each up as that
    /// very path, not inside the line's repository.
    pub fn compose_attached(&self) -> &[String] {
        &self.compose_attached
    }

    /// The box closed, or its task was added: what it kept for its line is the line's no more.
    pub(crate) fn compose_reset(&mut self) {
        (self.menu, self.compose_refs, self.compose_attached, self.compose_note) =
            (Menu::default(), Vec::new(), Vec::new(), None);
    }

    /// What a `^name` points at: what is open by that name now; and for the box's own line
    /// (`captures`), first what a reference of that name pointed at when it went in.
    fn pointed(&self, name: &str, captures: bool) -> Option<Ref> {
        let kept = self.compose_refs.iter().find(|r| captures && r.name == name).cloned();
        let open = || {
            let all = if captures { self.ref_candidates() } else { self.named_refs(&[]) };
            all.into_iter().map(|(r, _)| r).find(|r| r.name == name)
        };
        kept.or_else(open)
    }

    /// Keeps what a reference points at, as it is now. One thing is kept once, under the name
    /// it has in this line.
    fn capture(&mut self, reference: Ref) {
        if !self.compose_refs.iter().any(|kept| same_target(kept, &reference)) {
            self.compose_refs.push(reference);
        }
    }

    /// What a `^reference` may name in the box's line, each with a word of detail: every tab,
    /// every pane that shows something, every mark, and the installed apps that are not
    /// running. What the line already points at keeps the name it has there.
    pub fn ref_candidates(&self) -> Vec<(Ref, String)> {
        self.named_refs(&self.compose_refs)
    }

    /// As `ref_candidates`, with `kept` the references whose names are taken. Two things of one
    /// name: the later ones get `-2`, `-3`, and never a name `kept` has for something else, so
    /// a name in a line means one thing as long as the line is there.
    fn named_refs(&self, kept: &[Ref]) -> Vec<(Ref, String)> {
        let blank = |kind, name: &str| Ref {
            kind,
            name: name.to_string(),
            id: None,
            title: None,
            app: None,
            cwd: None,
            program: None,
            bundle: None,
        };
        let mut found: Vec<(Ref, String)> = Vec::new();
        let panes = self.tab_panes();
        for (tab, inside) in self.tabs.iter().zip(&panes) {
            let detail = format!("tab · {} pane{}", inside.len(), if inside.len() == 1 { "" } else { "s" });
            let title = Some(tab.title.clone());
            found.push((Ref { id: Some(tab.id), title, ..blank(RefKind::Tab, &tab.title) }, detail));
        }
        for pane in &self.panes {
            let id = Some(pane.id as u32);
            match &pane.kind {
                Kind::Empty => {}
                Kind::Terminal => {
                    let info = self.terminals.iter().find(|info| info.pane == pane.id);
                    let cwd = info.and_then(|info| info.cwd.clone()).or(pane.cwd.clone());
                    let program = info.and_then(|info| info.program.clone());
                    // As a mark of the pane is named: its directory, else `shell`.
                    let name = cwd.as_deref().and_then(marks::directory_name).unwrap_or("shell").to_string();
                    let place = cwd.as_deref().map(|cwd| short(self.home.as_deref(), cwd)).unwrap_or_default();
                    let detail = ["terminal", program.as_deref().unwrap_or("shell"), &place];
                    let detail = detail.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
                    found.push((Ref { id, cwd, program, ..blank(RefKind::Pane, &name) }, detail));
                }
                Kind::App { title, name, .. } => {
                    let app = if name.is_empty() { title } else { name };
                    let reference =
                        Ref { id, title: Some(title.clone()), app: Some(app.clone()), ..blank(RefKind::Pane, app) };
                    found.push((reference, format!("app pane · {title}")));
                }
            }
        }
        for mark in self.mark_rows() {
            found.push((
                Ref { id: Some(mark.id as u32), ..blank(RefKind::Mark, &mark.name) },
                format!("mark · {}", mark.kind),
            ));
        }
        let mut closed = closed_apps(&self.installed.0, &self.installed.1);
        closed.sort_by_key(|app| app.name.to_lowercase());
        for app in closed {
            let reference =
                Ref { app: Some(app.name.clone()), bundle: Some(app.path), ..blank(RefKind::App, &app.name) };
            found.push((reference, "app · not running".to_string()));
        }
        // A name the line can hold, and nothing with no name at all.
        found.retain(|(reference, _)| !reference.name.trim().is_empty() && compose::writable(&reference.name));
        let mut names: Vec<String> = kept.iter().map(|r| r.name.clone()).collect();
        for (reference, _) in &mut found {
            if let Some(kept) = kept.iter().find(|kept| same_target(kept, reference)) {
                reference.name = kept.name.clone();
                continue;
            }
            let base = reference.name.clone();
            let mut n = 1;
            while names.contains(&reference.name) {
                n += 1;
                reference.name = format!("{base}-{n}");
            }
            names.push(reference.name.clone());
        }
        found
    }

    /// Puts `words` into the line, each a word of its own, as one undo step: in place of the
    /// selection, else after the word the cursor is in (never inside it). False, and nothing
    /// goes in, if the box is not open or has no room for all of them.
    fn insert_words(&mut self, words: Vec<String>) -> bool {
        let Some(field) = self.compose.as_mut().filter(|_| !words.is_empty()) else { return false };
        let selection = field.selection();
        // At the start of a word, or between two: there. Inside one: after it.
        let inside = field.text()[..selection.end].chars().next_back().is_some_and(|c| !c.is_whitespace());
        let end = if inside { compose::word_end(field.text(), selection.end) } else { selection.end };
        let at = if selection.is_empty() { end } else { selection.end };
        let range = if selection.is_empty() { at..at } else { selection };
        let before = field.text()[..range.start].chars().next_back();
        let lead = if before.is_none_or(char::is_whitespace) { "" } else { " " };
        let text = format!("{lead}{} ", words.join(" "));
        if !field.fits(range.clone(), &text) {
            return false;
        }
        field.splice(range, &text);
        true
    }

    /// Nothing is open yet: no pane in any tab, no task, no conversation. The drawer then shows
    /// itself, in its new-task state, and the center says what goes there.
    pub fn welcome(&self) -> bool {
        self.panes.is_empty() && self.tasks.list.is_empty() && self.talk.is_none() && self.capture.is_none()
    }

    /// The drawer's one-click picks for the new-task line: WHERE, the first listed repositories;
    /// WHO, the default agent, then each runner and its models.
    pub fn compose_picks(&self) -> Picks {
        let names = self.repos.iter().map(|repo| repo.name.clone()).filter(|name| compose::taggable('>', name));
        let wheres = names.take(PICKS).collect();
        let default = self.default_agent().to_string();
        let mut whos = vec![default];
        for name in agents::names(&self.models, Vec::new()) {
            if !whos.iter().any(|seen| seen.eq_ignore_ascii_case(&name)) {
                whos.push(name);
            }
        }
        Picks { wheres, whos }
    }

    /// The line's tag of `mark` becomes `name` (one the picks offer), or goes: every word that
    /// is such a tag is taken out, and the new one added at the end.
    fn compose_pick(&mut self, mark: char, name: Option<String>) {
        let picks = self.compose_picks();
        let offered = match mark {
            '>' => &picks.wheres,
            '@' => &picks.whos,
            _ => return,
        };
        if name.as_ref().is_some_and(|name| !offered.contains(name)) {
            return;
        }
        let Some(field) = self.compose.as_mut() else { return };
        // Only words that are such a tag go, each with the space after it; the rest is as typed.
        let tag = |word: &str| word.strip_prefix(mark).is_some_and(|rest| compose::taggable(mark, rest));
        let mut text: String = field.text().split_inclusive(' ').filter(|piece| !tag(piece.trim_end())).collect();
        if let Some(name) = name {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(&format!("{mark}{name} "));
        }
        let all = 0..field.text().len();
        if field.fits(all.clone(), &text) {
            field.splice(all, &text);
        }
    }

    /// What ↵ in the new-task box would do with the line as it is: start this agent, in this
    /// repository (none: the home folder), on its branch. `None`: the box is closed.
    pub fn compose_plan(&self) -> Option<Plan> {
        let draft = self.draft(self.compose_line()?);
        let default = self.default_agent();
        let agent = draft.agent.clone().unwrap_or_else(|| default.to_string());
        let starts = agents::launch(Some(&agent), default, &self.models, String::new()).is_some();
        let path = draft.repo.as_deref().and_then(|name| self.repo_named(name));
        let checkouts = self.repos.iter().flat_map(|repo| {
            let worktrees = repo.worktrees.iter().map(|w| (&w.path, &w.name, &w.branch));
            std::iter::once((&repo.path, &repo.name, &repo.branch)).chain(worktrees)
        });
        let found = path.and_then(|path| checkouts.into_iter().find(|(at, ..)| **at == path));
        let (dir, branch) = found.map_or((None, None), |(_, name, branch)| (Some(name.clone()), branch.clone()));
        Some(Plan { agent, starts, dir, branch })
    }

    /// The path being typed at the cursor of the new-task box: the shell lists what finishes it.
    pub fn compose_typing(&self) -> Option<String> {
        self.menu_key().filter(|(_, mark, _)| *mark == '/').map(|(_, _, typed)| typed)
    }

    /// The tag at the cursor of the new-task box, as the dropdown's state is kept for it.
    fn menu_key(&self) -> Option<(usize, char, String)> {
        let field = self.compose.as_ref()?;
        let before = &field.text()[..field.cursor()];
        let (mark, typed) = compose::completing(before)?;
        Some((compose::completion(before, "").0.start, mark, typed))
    }

    /// The names a tag may take, the likelier first, and whether they are all in: a path's
    /// are read by the shell a moment after it is typed. Only names that read back as the
    /// tag when they are in the line.
    fn candidates(&self, mark: char, typed: &str) -> (Vec<Candidate>, bool) {
        let of = |kind: &'static str, name: &str, detail: String| Candidate { name: name.to_string(), kind, detail };
        let list: Vec<Candidate> = match mark {
            '#' => {
                // The project with the most tasks first.
                let mut projects: Vec<(usize, &str)> = Vec::new();
                for name in self.tasks.list.iter().filter_map(|task| task.project.as_deref()) {
                    match projects.iter_mut().find(|(_, seen)| *seen == name) {
                        Some((count, _)) => *count += 1,
                        None => projects.push((1, name)),
                    }
                }
                projects.sort_by_key(|(count, name)| (std::cmp::Reverse(*count), name.to_lowercase()));
                let detail = |n: usize| format!("{n} task{}", if n == 1 { "" } else { "s" });
                projects.into_iter().map(|(count, name)| of("project", name, detail(count))).collect()
            }
            '>' => self.repo_names().map(|(name, path)| of("repo", name, short(self.home.as_deref(), path))).collect(),
            '@' => {
                let agents = self.known.names.agents.iter();
                agents.map(|name| of("agent", name, crate::agents::kind(name))).collect()
            }
            '$' => self.known.names.skills.iter().map(|name| of("skill", name, String::new())).collect(),
            '/' => {
                if self.known_typing.as_deref() != Some(typed) {
                    return (Vec::new(), false);
                }
                let kind = |path: &str| if path.ends_with('/') { "folder" } else { "file" };
                let entries = compose::path_suggestions(&self.known.entries);
                return (entries.iter().map(|path| of(kind(path), path, String::new())).collect(), true);
            }
            '^' => {
                let kind = |kind| match kind {
                    RefKind::Tab => "tab",
                    RefKind::Pane => "pane",
                    RefKind::Mark => "mark",
                    RefKind::App => "app",
                };
                let refs = self.ref_candidates();
                return (refs.into_iter().map(|(r, detail)| of(kind(r.kind), &r.name, detail)).collect(), true);
            }
            _ => Vec::new(),
        };
        (list.into_iter().filter(|candidate| compose::taggable(mark, &candidate.name)).collect(), true)
    }

    /// The completion dropdown of the new-task box, while it is open: the cursor is in a tag,
    /// the line has the keyboard, nothing is selected, and Esc has not closed it for this tag.
    pub fn dropdown(&self) -> Option<Dropdown> {
        if !self.composing() {
            return None;
        }
        let field = self.compose.as_ref()?;
        if !field.selection().is_empty() || field.marked().is_some() {
            return None;
        }
        let key = self.menu_key()?;
        let menu = Some(&self.menu).filter(|menu| menu.key.as_ref() == Some(&key));
        if menu.is_some_and(|menu| menu.dismissed) {
            return None;
        }
        let (start, mark, typed) = key;
        let (candidates, read) = self.candidates(mark, &typed);
        // A path's entries were listed for what is typed: all match, from the last `/` on.
        let (rows, name) = if mark == '/' {
            let from = typed.rfind('/').map_or(0, |at| typed[..at + 1].chars().count());
            let matched = |name: &str| (from..typed.chars().count().min(name.chars().count())).collect();
            let row = |c: Candidate| Row { matched: matched(&c.name), name: c.name, kind: c.kind, detail: c.detail };
            let rows = candidates.into_iter().take(complete::MAX_ROWS).map(row).collect::<Vec<_>>();
            (rows, typed.rsplit('/').next().unwrap_or(""))
        } else {
            (complete::rows(&typed, candidates), typed.as_str())
        };
        let note = match (rows.is_empty(), read, mark) {
            (false, ..) => None,
            (true, false, _) => return None,
            // A project is whatever is typed: one nobody used yet is a new one.
            (true, true, '#') if !typed.is_empty() => Some("new project"),
            (true, true, _) => Some("no match"),
        };
        // The row the user moved to, wherever it now is; one that is gone moves nothing.
        let moved =
            menu.and_then(|menu| menu.moved.as_ref()).and_then(|name| rows.iter().position(|row| row.name == *name));
        let selected = complete::selected(moved, name, rows.len());
        Some(Dropdown { mark, typed: typed.clone(), start, rows, selected, note })
    }

    /// The row a typed space takes: the dropdown's row whose name is exactly what is typed,
    /// whatever the case of the letters. Only at the end of the word: a space typed inside one
    /// is a space. Not for a path, where a space ends the word anyway.
    fn spelled_out(&self, edit: &Edit) -> Option<usize> {
        let space = matches!(edit, Edit::Insert(text) | Edit::Replace { range: None, text } if text == " ");
        let field = self.compose.as_ref()?;
        let at_end = compose::word_end(field.text(), field.cursor()) == field.cursor();
        let menu = self.dropdown().filter(|menu| space && at_end && menu.mark != '/' && !menu.typed.is_empty())?;
        menu.rows.iter().position(|row| row.name.to_lowercase() == menu.typed.to_lowercase())
    }

    /// What taking row `index` would put where: the word under the cursor, and the text for
    /// it. `None`: no such row, or the line has no room for the name whole.
    fn taking(&self, index: usize) -> Option<(std::ops::Range<usize>, String)> {
        let (menu, field) = (self.dropdown()?, self.compose.as_ref()?);
        let (word, text) = compose::completion_at(field.text(), field.cursor(), &menu.rows.get(index)?.name);
        field.fits(word.clone(), &text).then_some((word, text))
    }

    /// Whether taking row `index` would change the word under the cursor: false if it is
    /// spelled out already, or the name has no room. A folder named without its last `/` is
    /// spelled out.
    fn changes(&self, index: usize) -> bool {
        let (Some((word, text)), Some(field)) = (self.taking(index), &self.compose) else { return false };
        let (typed, taken) = (&field.text()[word], text.trim_end());
        typed != taken && Some(typed) != taken.strip_suffix('/')
    }

    /// Takes row `index` of the dropdown: its name replaces the whole word under the cursor,
    /// with a space after it, and nothing past that word. A folder takes no space: the
    /// dropdown stays, on what is in it. A name the line has no room for is not taken in
    /// part: the line stays, and the box says so.
    fn accept(&mut self, index: usize) {
        let Some(menu) = self.dropdown() else { return };
        let Some(row) = menu.rows.get(index) else { return };
        let Some((word, text)) = self.taking(index) else {
            self.compose_note = Some("no room in the line for that name".into());
            return;
        };
        if menu.mark == '^' {
            if let Some((reference, _)) = self.ref_candidates().into_iter().find(|(r, _)| r.name == row.name) {
                self.capture(reference);
            }
        }
        let Some(field) = &mut self.compose else { return };
        let spaced = text.ends_with(' ') || text.ends_with('/');
        field.splice(word, &text);
        // A space was there already: the cursor goes past it, out of the tag.
        if !spaced {
            field.apply(Edit::Move { to: Motion::Right, select: false });
        }
        (self.menu, self.compose_note) = (Menu::default(), None);
    }

    /// The shell has read what the tags of the line in the new-task box may name: what `draft`
    /// says of it will not change by itself.
    pub fn compose_read(&self) -> bool {
        self.compose.is_some() && self.compose_line() == self.known_line.as_ref()
    }

    /// The path of the listed repository `line` names with `>name`, for the shell to look in.
    pub fn draft_repo(&self, line: &str) -> Option<String> {
        self.draft(line).repo.and_then(|name| self.repo_named(&name))
    }

    /// A task's tags as one line: `#canvas · >app · @claude · 2 paths · 1 skill`.
    pub fn task_tags(&self, task: &Task) -> String {
        let count = |n: usize, word: &str| (n > 0).then(|| format!("{n} {word}{}", if n == 1 { "" } else { "s" }));
        let repo = task.repo.as_deref().map(|path| {
            let name = self.repo_name(path);
            format!(">{}", name.unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string()))
        });
        let project = task.project.as_ref().map(|project| format!("#{project}"));
        let agent = task.agent.as_ref().map(|agent| format!("@{agent}"));
        let (paths, skills) = (count(task.paths.len(), "path"), count(task.skills.len(), "skill"));
        let tags = [project, repo, agent, paths, skills, count(task.refs.len(), "reference")];
        tags.into_iter().flatten().collect::<Vec<_>>().join(" · ")
    }

    /// Files `line` as a captured task. `None` if it has no title, or one too long to keep:
    /// nothing is filed.
    pub(crate) fn file_line(&mut self, line: &str, known: Option<compose::Known>) -> Option<TaskId> {
        self.file_titled(line, known, None)
    }

    /// As `file_line`. `title`: the task's title, whatever the line's is; its tags are the line's.
    fn file_titled(&mut self, line: &str, known: Option<compose::Known>, title: Option<String>) -> Option<TaskId> {
        let mut draft = match &known {
            Some(known) => self.draft_knowing(line, known, false),
            None => self.draft(line),
        };
        let made_up = title.is_some();
        if let Some(title) = title {
            draft.title = title;
        }
        if draft.title.is_empty() || draft.title.chars().count() > MAX_TITLE {
            return None;
        }
        let path = draft.repo.as_deref().and_then(|name| self.repo_named(name));
        let id = self.file(draft.title, draft.priority.unwrap_or(3), State::Captured);
        self.tasks.tag(id, draft.project, path);
        let Draft { agent, paths, resolved, skills, refs, .. } = draft;
        self.tasks.assign(id, Assign { agent, paths, resolved, skills });
        let pointed = refs.iter().filter_map(|name| self.pointed(name, known.is_none())).collect();
        self.tasks.point(id, pointed);
        if made_up {
            self.tasks.untitled(id);
        }
        Some(id)
    }

    /// The left rail's sections, as drawn: Calendar only while a calendar feed is set.
    pub fn sections(&self) -> Vec<Section> {
        zones::SECTIONS.into_iter().filter(|s| *s != Section::Calendar || self.calendar.on).collect()
    }

    /// True if a `Tick` now would change what shows: a snoozed task's time has come, or the
    /// offer to undo has run out. The shell asks; it does not look at the times itself.
    pub fn due(&self, now: u64) -> bool {
        self.tasks.due(now)
            || self.undo_offer.is_some_and(|offer| offer.until <= now)
            || self.workbooks.due(now)
            || self.calendar.due(now)
    }

    /// Everything the marks and the workbooks need, each once: what the shell looks for on
    /// this machine (`Action::MarksChecked` says what it did not find).
    pub fn needs(&self) -> Vec<Need> {
        let mut needs = self.marks.needs();
        for need in self.workbooks.needs() {
            if !needs.contains(&need) {
                needs.push(need);
            }
        }
        needs
    }

    /// The row list mode starts on: the first that is not a heading.
    fn first_row(&self) -> usize {
        self.list_rows().iter().position(|row| !matches!(row, ListRow::Heading(_))).unwrap_or(0)
    }

    /// A move made with the mouse: if it changed the task, a notice offers to take it back.
    fn row_move(&mut self, id: TaskId, moved: Moved, change: impl FnOnce(&mut Tasks)) {
        let before = self.tasks.get(id).cloned();
        change(&mut self.tasks);
        if before.is_some() && self.tasks.get(id) != before.as_ref() {
            self.undo_offer = Some(Notice { moved, until: self.now.saturating_add(NOTICE_MS) });
        }
    }

    /// Dismisses a task. One whose agent runs is asked about first (`Confirm::dismiss`): a
    /// client's dismissal of it is refused. `moved`: the move the notice offers back.
    fn dismiss(&mut self, id: TaskId, moved: Option<Moved>, effects: &mut Vec<Effect>) {
        if self.agent_runs(&Subject::Task(id)) && self.tasks.live(id).is_some() {
            // The question is ottospace's own UI: a live window would cover it.
            self.leave(effects);
            self.confirm = Some(Confirm { running: Vec::new(), dismiss: Some(id) });
            return;
        }
        match moved {
            Some(moved) => self.row_move(id, moved, |tasks| tasks.dismiss(id)),
            None => self.tasks.dismiss(id),
        }
    }

    /// After every step: a tab of a task that is gone or dismissed is a tab of no task (the
    /// user's undo of the dismissal links again those of its tabs still open and of no task; a
    /// task back by other means, such as a state report, gets none); a dismissed task's
    /// conversation leaves the rail and the drawer; the groups stay side by side; the tab on
    /// show is its task's last.
    fn settle_tasks(&mut self, undone: bool) {
        let tasks = &self.tasks;
        let gone = |id: &TaskId| tasks.live(*id).is_none();
        for tab in &mut self.tabs {
            if let Some(id) = tab.task.filter(gone) {
                if tasks.get(id).is_some() {
                    match self.unlinked.iter_mut().find(|(task, _)| *task == id) {
                        Some((_, tabs)) => tabs.push(tab.id),
                        None => self.unlinked.push((id, vec![tab.id])),
                    }
                }
                tab.task = None;
            }
        }
        self.unlinked.retain(|(id, _)| tasks.get(*id).is_some());
        let back: Vec<(TaskId, Vec<u32>)> = self.unlinked.extract_if(.., |(id, _)| !gone(id)).collect();
        for (id, ids) in back.into_iter().filter(|_| undone) {
            for tab in self.tabs.iter_mut().filter(|tab| ids.contains(&tab.id) && tab.task.is_none()) {
                tab.task = Some(id);
            }
        }
        let live = |subject: &Subject| subject.task().is_none_or(|id| !gone(&id));
        self.opened.retain(live);
        if self.talk.as_ref().is_some_and(|talk| !live(&talk.task)) {
            self.talk = None;
            self.brief = None;
        }
        self.regroup();
        self.remember_tab();
    }

    /// The tab on show changed from `shown`: an open drawer shows the conversation of its task,
    /// if it has one, with no agent started and the keyboard where it was. While the keyboard
    /// is in the drawer only the user's own change of tab does this.
    fn follow_tab(&mut self, shown: Option<u32>, origin: Origin, drawer_keys: bool) {
        let Some(shown) = shown else { return };
        let Some(tab) = self.tabs.get(self.active).filter(|tab| tab.id != shown) else { return };
        let Some(id) = tab.task else { return };
        let open = self.talk.as_ref().is_some_and(|talk| talk.open) || self.compose_host() == Some(Host::Drawer);
        let subject = Subject::Task(id);
        if !open
            || (drawer_keys && origin != Origin::User)
            || self.talk.as_ref().is_some_and(|t| t.open && t.task == subject)
        {
            return;
        }
        if self.compose_host() == Some(Host::Drawer) {
            self.put_compose_aside();
        }
        let keys = drawer_keys && self.talk_live(&subject);
        self.talk = Some(Talk { task: subject.clone(), open: true, keys });
        self.opened.retain(|seen| *seen != subject);
        self.opened.push(subject);
    }

    /// Adds a task, filed at the time of the last `Tick`, if there was one.
    fn file(&mut self, title: String, priority: u8, state: State) -> TaskId {
        let id = self.tasks.add(title, priority, state);
        if self.now > 0 {
            self.tasks.stamp(id, self.now);
        }
        id
    }

    /// "Add and start": shows a task just filed, and the list gives up the keyboard.
    fn start(&mut self, id: TaskId, open: bool, effects: &mut Vec<Effect>) {
        // Only the user's own ↵ starts an agent, in the drawer: the workspace stays. A client's
        // "open" starts nothing, and a new task has no tab to go to.
        if open && self.by_user {
            self.talk_to(Subject::Task(id), effects);
        } else if open {
            self.list = None;
        }
    }

    /// Where each listed task goes in the State view, in the order drawn: under its state's
    /// heading, most urgent first, then oldest first. `list_rows` asks for the order alone.
    fn task_places(&self) -> Vec<TaskPlace<'_>> {
        let mut places: Vec<TaskPlace> = Vec::new();
        for (group, task) in self.tasks.inbox() {
            places.push(TaskPlace { heading: group.into(), group, task });
        }
        places.sort_by_key(|place| (place.heading, place.task.priority, place.task.id));
        places
    }

    /// How many tabs and panes a task has open.
    fn task_open(&self, id: TaskId) -> (usize, usize) {
        let panes = self.tab_panes();
        let mine = || self.tabs.iter().zip(&panes).filter(|(tab, _)| tab.task == Some(id));
        (mine().count(), mine().map(|(_, panes)| panes.len()).sum())
    }

    /// The session a task follows, as last listed: what it does now, and its model.
    fn task_session(&self, task: &Task) -> Option<&Found> {
        let link = task.session.as_ref()?;
        self.found.iter().find(|s| (&s.runner, &s.id) == (&link.runner, &link.id))
    }

    /// A task's row as drawn, at `now`.
    fn task_row(&self, task: &Task, group: tasks::Group, now: u64) -> TaskRow {
        let (tabs, panes) = self.task_open(task.id);
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let open = (tabs > 0).then(|| format!("{}, {}", plural(tabs, "tab"), plural(panes, "pane")));
        let tags = Some(self.task_tags(task)).filter(|tags| !tags.is_empty());
        // What was captured for it: its brief shows when it is opened.
        let kept = Some(task.materials.len()).filter(|n| *n > 0).map(|n| plural(n, "material"));
        let age = task.created.map(|created| tasks::age(created, now));
        let detail = [tags, kept, open, age].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let current = self.tab_task() == Some(task.id);
        let session = self.task_session(task);
        let (live, model) = (session.and_then(Found::doing), session.and_then(|s| s.model.clone()));
        TaskRow { live, model, id: task.id, group, tabs, panes, current, detail }
    }

    /// The Tasks section's State view as drawn: Needs you, Running, Later (captured and on hold),
    /// Done (folded at first). A task is listed once. No empty headings.
    pub fn task_sections(&self, now: u64) -> Vec<TaskSection> {
        let mut sections: Vec<TaskSection> = Vec::new();
        for TaskPlace { heading, group, task } in self.task_places() {
            if sections.last().is_none_or(|s| s.heading != heading) {
                let collapsed = self.collapsed.contains(&heading);
                sections.push(TaskSection { heading, count: 0, collapsed, rows: Vec::new() });
            }
            let Some(section) = sections.last_mut() else { continue };
            section.count += 1;
            if !section.collapsed {
                section.rows.push(self.task_row(task, group, now));
            }
        }
        sections
    }

    /// The Project view as drawn: a group a project, by name, then the tasks of none
    /// (`tasks::by_project`). In each, the open tasks by state, then a "Done" line folded at first.
    pub fn project_groups(&self, now: u64) -> Vec<ProjectGroup> {
        let groups = tasks::by_project(self.tasks.inbox());
        let row = |(group, task): &(tasks::Group, &Task)| self.task_row(task, *group, now);
        groups
            .into_iter()
            .map(|(project, places)| {
                let (done, open): (Vec<_>, Vec<_>) = places.iter().partition(|(g, _)| *g == tasks::Group::Done);
                let folded = self.folded_projects.contains(&project);
                let done_open = self.open_done.contains(&project);
                ProjectGroup {
                    bar: tasks::Segments::of(&places),
                    rows: if folded { Vec::new() } else { open.into_iter().map(row).collect() },
                    done: done.len(),
                    done_rows: if folded || !done_open { Vec::new() } else { done.into_iter().map(row).collect() },
                    project,
                    folded,
                    done_open,
                }
            })
            .collect()
    }

    /// The footer of the Tasks section: of the day that started at `day` (the shell's local
    /// midnight), the tasks finished, those running now, and the time agents were at work.
    pub fn today(&self, day: u64) -> Today {
        let list = &self.tasks.list;
        let done = list.iter().filter(|t| t.state == State::Done && t.finished_at.is_some_and(|at| at >= day)).count();
        let running = list.iter().filter(|t| t.group() == Some(tasks::Group::Running)).count();
        let agent_ms = list.iter().flat_map(|t| &t.work).map(|w| w.clock.on(day)).sum();
        Today { done, running, agent_ms }
    }

    /// What the hover card of a task says, at `now`.
    pub fn card(&self, id: TaskId, now: u64) -> Option<Card> {
        let task = self.tasks.get(id)?;
        let session = self.task_session(task);
        let (tabs, panes) = self.task_open(id);
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let mut tags: Vec<String> = Vec::new();
        tags.extend(task.project.as_ref().map(|p| format!("#{p}")));
        tags.extend(
            task.repo.as_ref().map(|path| format!(">{}", self.repo_name(path).unwrap_or_else(|| repos::name_of(path)))),
        );
        tags.extend(task.agent.as_ref().map(|a| format!("@{a}")));
        let last = task.work.last();
        Some(Card {
            id,
            title: task.title.clone(),
            state: task.group().map_or("Not listed", tasks::Group::title),
            group: task.group(),
            live: session.and_then(Found::doing),
            model: session.and_then(|s| s.model.clone()),
            tags,
            started: task.started_at.map(|at| tasks::age(at, now)),
            agent_time: progress::duration(task.work.iter().map(|w| w.clock.ms).sum()),
            workspace: format!("{} · {}", plural(tabs, "tab"), plural(panes, "pane")),
            tokens: progress::merged(task.work.iter().flat_map(|w| &w.tokens)),
            context: last.and_then(|w| w.context),
            window: last.and_then(|w| w.window),
        })
    }

    /// Tasks adopted from sessions follow them (`sessions::follow`), by the time of the last
    /// `Tick`. `seen`: (runner, id, at work, when its file last changed).
    fn follow_sessions(&mut self, seen: Vec<(String, String, bool, Option<u64>)>) {
        let follows = |task: &Task| {
            let link = task.session.as_ref()?;
            let (.., active, last) = seen.iter().find(|(runner, id, ..)| (runner, id) == (&link.runner, &link.id))?;
            // Our own agent's last writes, just stopped, are no work of the session's.
            let active = *active && !self.ours_at(&link.runner, &link.id, *last);
            let next = sessions::follow(active, *last, link.state, task.state, self.now, self.session_quiet_ms)?;
            Some((task.id, next))
        };
        let moves: Vec<(TaskId, State)> = self.tasks.list.iter().filter_map(follows).collect();
        for (id, state) in moves {
            self.tasks.set_state(id, state);
            self.tasks.followed(id, state);
        }
    }

    /// The sessions whose tasks still follow them, as (runner, id): the task has the state it
    /// last took from its session. Only these are worth reading while the Tasks section is closed.
    pub fn followed_sessions(&self) -> Vec<(String, String)> {
        let follows = |task: &Task| {
            let link = task.session.as_ref().filter(|link| link.state == Some(task.state))?;
            Some((link.runner.clone(), link.id.clone()))
        };
        self.tasks.list.iter().filter_map(follows).collect()
    }

    /// The time of the last `Tick` (milliseconds since 1970).
    pub fn now(&self) -> u64 {
        self.now
    }

    /// The sessions whose work the shell reads (`SessionWork`), as (runner, id, the task's
    /// repository): of the tasks that are listed and not done, or done in the last
    /// `progress::LAST_READ_MS` (what came after the last read). Those that need you or run come
    /// first; at most `progress::MAX_SESSIONS`.
    pub fn worked_sessions(&self) -> Vec<(String, String, Option<String>)> {
        let lately =
            |task: &Task| task.finished_at.is_some_and(|at| self.now.saturating_sub(at) <= progress::LAST_READ_MS);
        let read = |task: &&Task| match task.group() {
            Some(tasks::Group::Done) => lately(task),
            group => group.is_some(),
        };
        let mut linked: Vec<&Task> = self.tasks.list.iter().filter(read).filter(|t| t.session.is_some()).collect();
        linked.sort_by_key(|task| task.group().map(Heading::from));
        let key =
            |task: &Task| task.session.as_ref().map(|link| (link.runner.clone(), link.id.clone(), task.repo.clone()));
        linked.into_iter().filter_map(key).take(progress::MAX_SESSIONS).collect()
    }

    /// The sessions that are no task's and not hidden, in the order listed. At most `MAX_FOUND`.
    fn unadopted(&self) -> impl Iterator<Item = &Found> {
        let hidden = |s: &&Found| self.hidden.iter().any(|(runner, id)| (runner, id) == (&s.runner, &s.id));
        let listed = self.found.iter().filter(move |s| !hidden(s) && !self.tasks.adopted(&s.runner, &s.id));
        listed.take(sessions::MAX_FOUND)
    }

    /// A listed session, by runner and id: one that is no task's and not hidden.
    fn session(&self, runner: &str, id: &str) -> Option<&Found> {
        self.unadopted().find(|s| s.runner == runner && s.id == id)
    }

    /// The name of the listed repository or worktree a session runs in: as the Repos section
    /// names it, else the last part of its path.
    fn session_name(&self, session: &Found) -> String {
        self.repo_name(&session.repo).unwrap_or_else(|| repos::name_of(&session.repo))
    }

    /// The terminal pane a session runs in, when there is no doubt: the session is at work, one
    /// terminal of the tab on show was opened in its directory, and no other listed session at
    /// work started there. A past session in the same directory, a hidden one or a task's does
    /// not count. (A shell that has since changed directory, or a session started elsewhere in
    /// a pane, is not told: the core knows a pane's first directory only.)
    fn session_pane(&self, session: &Found) -> Option<PaneId> {
        let shown = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let there = |pane: &&Pane| {
            pane.kind == Kind::Terminal && shown.contains(&pane.id) && pane.cwd.as_deref() == Some(session.cwd.as_str())
        };
        let mut panes = self.panes.iter().filter(there);
        let (pane, another) = (panes.next()?, panes.next());
        let at_work = self.unadopted().filter(|s| s.active && s.cwd == session.cwd).count();
        (another.is_none() && session.active && at_work == 1).then_some(pane.id)
    }

    /// The Other sessions group as drawn, at the time of the last `Tick`: sessions found that are
    /// no task's, active first, those in the range alone.
    pub fn sessions_bar(&self) -> sessions::Bar {
        let (range, now, day) = (self.sessions_range, self.now, self.session_day);
        let (shown, out): (Vec<&Found>, Vec<&Found>) = self.unadopted().partition(|s| s.in_range(range, now, day));
        let collapsed = self.collapsed.contains(&Heading::Sessions);
        let drawn = shown.iter().filter(|_| !collapsed);
        let row = |s: &&Found| sessions::Row {
            runner: s.runner.clone(),
            id: s.id.clone(),
            name: self.session_name(s),
            active: s.active,
            doing: s.doing(),
            age: s.last_activity.map(|at| tasks::age(at, self.now)),
            pane: self.session_pane(s),
            quoted_title: s.title.clone(),
            model: s.model.clone(),
        };
        let rows = drawn.map(row).collect();
        sessions::Bar { count: shown.len(), total: shown.len() + out.len(), range, collapsed, rows }
    }

    /// Every listed repository and worktree, as (name, path).
    pub fn repo_names(&self) -> impl Iterator<Item = (&str, &str)> {
        self.repos.iter().flat_map(|r| {
            let worktrees = r.worktrees.iter().map(|w| (w.name.as_str(), w.path.as_str()));
            std::iter::once((r.name.as_str(), r.path.as_str())).chain(worktrees)
        })
    }

    /// The path of the listed repository or worktree called `name`, whatever the case of the
    /// letters. Of two with one name, the first listed.
    fn repo_named(&self, name: &str) -> Option<String> {
        self.repo_names().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, path)| path.to_string())
    }

    /// The text field that is open, and whose it is: a mark's name being typed, the picker's
    /// filter, else the new-task box. It may be under the key reference or the quit question: see `field`.
    pub fn open_field(&self) -> Option<(&'static str, &Field)> {
        match &self.mark_input {
            Some(MarkInput::Rename { text, .. }) => return Some(("mark", text)),
            Some(MarkInput::Workbook { text, .. }) => return Some(("workbook", text)),
            _ => {}
        }
        let picker = self.picker.as_ref().map(|picker| ("picker", &picker.query));
        // A note being typed on a capture, unless the draft's title has the keyboard.
        let note = self.note().filter(|_| !self.compose_keys()).map(|field| ("note", field));
        // The Apps section's filter, while it has the keyboard and no box does.
        let boxed = self.compose_modal() || self.compose_keys();
        let apps = (self.app_lists.filtering && !boxed).then_some(("apps", &self.app_lists.filter));
        let palette = self.palette.as_ref().map(|palette| ("palette", &palette.query));
        let palette = palette.or(self.inspector.as_ref().filter(|i| i.filtering).map(|i| ("inspector", &i.filter)));
        picker.or(palette).or(note).or(apps).or(self.compose.as_ref().map(|field| ("compose", field)))
    }

    /// A field by whose it is: `picker`, `note`, `apps`, `mark` or `compose`. Two can show at once
    /// (the draft's title in the bar, and a note or a filter), so the shell asks for each by name.
    pub fn field_of(&self, of: &str) -> Option<&Field> {
        match of {
            "picker" => self.picker.as_ref().map(|picker| &picker.query),
            "palette" => self.palette.as_ref().map(|palette| &palette.query),
            "inspector" => self.inspector.as_ref().map(|inspector| &inspector.filter),
            "note" => self.note(),
            "talk" => self.talk_draft(),
            "apps" => Some(&self.app_lists.filter),
            "mark" => match &self.mark_input {
                Some(MarkInput::Rename { text, .. }) => Some(text),
                _ => None,
            },
            "workbook" => match &self.mark_input {
                Some(MarkInput::Workbook { text, .. }) => Some(text),
                _ => None,
            },
            _ => self.compose.as_ref(),
        }
    }

    /// The open text field, if it has the keyboard: keys, the clipboard and the input method
    /// edit it. Not while the key reference, the quit question or sneak mode is over it.
    pub fn field(&self) -> Option<(&'static str, &Field)> {
        // Each field in its own mode: the Apps section's filter in list mode, a mark's name
        // while it is typed, a note on a capture in annotate mode. A mark that waits for a
        // letter is over the filter, not in it.
        let of = match self.mode() {
            Mode::Mark if matches!(self.mark_input, Some(MarkInput::Workbook { .. })) => "workbook",
            Mode::Mark => "mark",
            Mode::Pick => "picker",
            Mode::Palette => "palette",
            Mode::Inspect => "inspector",
            Mode::Compose => "compose",
            Mode::Annotate => "note",
            Mode::Talk if self.talk_typing() => "talk",
            Mode::List if self.app_lists.filtering => "apps",
            _ => return None,
        };
        self.field_of(of).filter(|_| !self.keys).map(|field| (of, field))
    }

    /// The line in the new-task box.
    pub fn compose_line(&self) -> Option<&String> {
        self.compose.as_ref().map(Field::line)
    }

    /// The rows of the open section, top to bottom, as the secondary bar draws them.
    pub fn list_rows(&self) -> Vec<ListRow> {
        match self.sidebar {
            Some(Section::Apps) => self.app_rows(),
            Some(Section::Repos) => {
                self.repos.iter().flat_map(Repo::checkouts).map(|checkout| ListRow::Repo(checkout.path)).collect()
            }
            Some(Section::Tasks) => self.task_list_rows(self.tasks_view, false),
            Some(Section::Marks) => self.marks.list.iter().map(|mark| ListRow::Mark(mark.id)).collect(),
            Some(Section::Calendar) => self.calendar.agenda(),
            // Each workbook, and the tabs of an unfolded one under it.
            Some(Section::Workbook) => {
                let mut rows = Vec::new();
                for book in &self.workbooks.list {
                    rows.push(ListRow::Workbook(book.id));
                    if self.workbooks.unfolded(book.id) {
                        rows.extend((0..book.tabs.len()).map(|at| ListRow::WorkbookTab(book.id, at)));
                    }
                }
                rows
            }
            None => Vec::new(),
        }
    }

    /// The Tasks section's rows in `view`, as drawn: each heading (or project, and its "Done"
    /// line), then its rows unless it is folded. A heading is a row, so a section that is all
    /// folded can be unfolded from the keyboard. `unfolded`: as if nothing were folded.
    pub(crate) fn task_list_rows(&self, view: TasksView, unfolded: bool) -> Vec<ListRow> {
        let collapsed = |heading: &Heading| !unfolded && self.collapsed.contains(heading);
        let folded = |project: &Option<String>| !unfolded && self.folded_projects.contains(project);
        let done_open = |project: &Option<String>| unfolded || self.open_done.contains(project);
        let mut rows: Vec<ListRow> = Vec::new();
        match view {
            TasksView::State => {
                for place in self.task_places() {
                    if rows.iter().all(|row| *row != ListRow::Heading(place.heading)) {
                        rows.push(ListRow::Heading(place.heading));
                    }
                    if !collapsed(&place.heading) {
                        rows.push(ListRow::Task(place.task.id));
                    }
                }
            }
            TasksView::Project => {
                for (project, places) in tasks::by_project(self.tasks.inbox()) {
                    rows.push(ListRow::Project(project.clone()));
                    if folded(&project) {
                        continue;
                    }
                    let done = |(group, _): &&(tasks::Group, &Task)| *group == tasks::Group::Done;
                    rows.extend(places.iter().filter(|p| !done(p)).map(|(_, task)| ListRow::Task(task.id)));
                    if places.iter().any(|p| done(&p)) {
                        rows.push(ListRow::ProjectDone(project.clone()));
                        if done_open(&project) {
                            rows.extend(places.iter().filter(done).map(|(_, task)| ListRow::Task(task.id)));
                        }
                    }
                }
            }
        }
        // Then the sessions that are no task's, as drawn: the heading (while any is listed, in
        // the range or not), and those in the range.
        let bar = self.sessions_bar();
        if bar.total > 0 {
            rows.push(ListRow::Heading(Heading::Sessions));
            rows.extend(bar.rows.iter().map(|row| ListRow::Session(row.runner.clone(), row.id.clone())));
        }
        rows
    }

    fn selected_task(&self) -> Option<TaskId> {
        match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
            Some(ListRow::Task(id)) => Some(id),
            _ => None,
        }
    }

    /// Shows `kind` in `beside` (default: the focused pane) if that pane is empty; else splits it.
    pub(crate) fn open(&mut self, kind: Kind, axis: Axis, beside: Option<PaneId>, effects: &mut Vec<Effect>) -> PaneId {
        self.list = None; // the new pane takes the keyboard
        // The new pane takes focus, so the live app must give the keyboard back, and visual mode ends.
        self.leave(effects);
        self.leave_visual(effects);
        let target = beside.or(self.focused);
        let empty = target.filter(|t| self.pane(*t).is_some_and(|p| p.kind == Kind::Empty));
        let id = match empty.and_then(|t| self.panes.iter_mut().find(|p| p.id == t)) {
            Some(pane) => {
                pane.kind = kind;
                let id = pane.id;
                // The empty pane may be in a background tab: focus is on a pane of the tab on show.
                self.show_tab_of(id, effects);
                id
            }
            None => self.add(kind, target, axis),
        };
        self.focused = Some(id);
        effects.push(Effect::Opened(id));
        id
    }

    pub(crate) fn push_tab(&mut self) {
        self.push_tab_of(None);
    }

    /// A new tab on show, at the end of `task`'s group: a new group goes before the tabs of no task.
    pub(crate) fn push_tab_of(&mut self, task: Option<TaskId>) {
        self.next_tab += 1;
        // The lowest number no tab has: closing "Tab 5" and opening one gives "Tab 5" again.
        let taken = |n: &u32| self.tabs.iter().any(|t| t.title == format!("Tab {n}"));
        let title = format!("Tab {}", (1..).find(|n| !taken(n)).unwrap_or(self.next_tab));
        let at = self.group_end(task);
        self.tabs.insert(at, Tab { id: self.next_tab, title, task, layout: None, focused: None });
        self.active = at;
    }

    /// Where a tab joining `task`'s group goes: after its last tab; a new group goes before the
    /// tabs of no task.
    fn group_end(&self, task: Option<TaskId>) -> usize {
        let end = self.tabs.iter().rposition(|t| t.task == task).map(|at| at + 1);
        let before_none = || self.tabs.iter().position(|t| t.task.is_none() && task.is_some());
        end.or_else(before_none).unwrap_or(self.tabs.len())
    }

    /// The task of the tab on show, if it has one.
    pub fn tab_task(&self) -> Option<TaskId> {
        self.tabs.get(self.active).and_then(|tab| tab.task)
    }

    /// The tab bar's groups in order: each task's tabs side by side, the tabs of no task last.
    pub fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for (at, tab) in self.tabs.iter().enumerate() {
            match groups.last_mut() {
                Some(group) if group.task == tab.task => group.tabs.push(at),
                _ => groups.push(Group { task: tab.task, tabs: vec![at] }),
            }
        }
        groups
    }

    /// The task's tab shown last, else its first. `None`: it has no tab.
    fn tab_of_task(&self, task: TaskId) -> Option<usize> {
        let last = self.last_tab.iter().find(|(id, _)| *id == task).map(|(_, tab)| *tab);
        let last = last.and_then(|id| self.tabs.iter().position(|tab| tab.id == id && tab.task == Some(task)));
        last.or_else(|| self.tabs.iter().position(|tab| tab.task == Some(task)))
    }

    /// Notes the tab on show as its task's last, to go back to it.
    pub(crate) fn remember_tab(&mut self) {
        if let Some((task, id)) = self.tabs.get(self.active).and_then(|tab| Some((tab.task?, tab.id)))
            && self.last_tab.first() != Some(&(task, id))
        {
            self.last_tab.retain(|(t, _)| *t != task);
            self.last_tab.insert(0, (task, id));
        }
    }

    /// Moves a tab into a group, or before a tab, taking that group's task. The tab on show stays on show.
    fn place_tab(&mut self, tab: Option<u32>, at: TabAt) {
        let Some(from) = tab.map_or(Some(self.active), |id| self.tabs.iter().position(|t| t.id == id)) else {
            return;
        };
        let task = match at {
            // A task that is gone or dismissed takes no tab.
            TabAt::End(Some(id)) => match self.tasks.live(id) {
                Some(_) => Some(id),
                None => return,
            },
            TabAt::End(None) => None,
            TabAt::Before(id) => match self.tabs.iter().find(|t| t.id == id) {
                Some(target) if target.id != self.tabs[from].id => target.task,
                _ => return,
            },
        };
        let shown = self.tabs[self.active].id;
        let mut moved = self.tabs.remove(from);
        moved.task = task;
        let to = match at {
            TabAt::Before(id) => self.tabs.iter().position(|t| t.id == id),
            TabAt::End(_) => None,
        };
        let to = to.unwrap_or_else(|| self.group_end(task));
        self.tabs.insert(to, moved);
        self.active = self.tabs.iter().position(|t| t.id == shown).unwrap_or(0);
        self.regroup();
    }

    /// Keeps each task's tabs side by side, groups in the order of their first tab, the tabs of
    /// no task last. The tab on show stays on show.
    fn regroup(&mut self) {
        let order = bar_order(&self.tabs.iter().map(|tab| tab.task).collect::<Vec<_>>());
        if order.is_sorted() {
            return;
        }
        let shown = self.tabs[self.active].id;
        let mut tabs: Vec<Option<Tab>> = std::mem::take(&mut self.tabs).into_iter().map(Some).collect();
        self.tabs = order.into_iter().filter_map(|at| tabs[at].take()).collect();
        self.active = self.tabs.iter().position(|t| t.id == shown).unwrap_or(0);
    }

    /// Stores the active tab's tree and focus in the tab, before another tab takes the stage.
    pub(crate) fn park_active(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.layout = self.layout.take();
            tab.focused = self.focused.take();
        }
    }

    fn unpark_active(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            self.layout = tab.layout.take();
            self.focused = tab.focused.take();
        }
    }

    /// Every tab's tree as it is now.
    fn trees(&self) -> Vec<Node> {
        self.layout.iter().chain(self.tabs.iter().filter_map(|tab| tab.layout.as_ref())).cloned().collect()
    }

    /// The tree that holds `pane`: the active tab's, or a background tab's.
    fn tree(&self, pane: PaneId) -> Option<&Node> {
        let holds = |tree: &&Node| tree.leaves().contains(&pane);
        let background = || self.tabs.iter().filter_map(|tab| tab.layout.as_ref()).find(holds);
        self.layout.as_ref().filter(holds).or_else(background)
    }

    fn tree_mut(&mut self, pane: PaneId) -> Option<&mut Node> {
        if self.layout.as_ref().is_some_and(|tree| tree.leaves().contains(&pane)) {
            return self.layout.as_mut();
        }
        self.tabs.iter_mut().filter_map(|tab| tab.layout.as_mut()).find(|tree| tree.leaves().contains(&pane))
    }

    /// Notes on each pane's `fit` whether a fit would move nothing, and why: the header then
    /// offers none. A fit moves something if a split would move on any axis that does not fit.
    fn mark_stuck(&mut self) {
        let stuck = |pane: &Pane| {
            let fit = pane.fit.as_ref()?;
            let tree = self.tree(pane.id);
            let would = |(at, axis): (usize, Axis)| tree?.scalable(pane.id, axis, fit.scale(at)?);
            let found: Vec<bool> = AXES.into_iter().enumerate().filter_map(would).collect();
            match found.as_slice() {
                [] => Some(Stuck::NoSplit),
                moves if moves.contains(&true) => None,
                _ => Some(Stuck::AtLimit),
            }
        };
        let marks: Vec<Option<Stuck>> = self.panes.iter().map(stuck).collect();
        for (pane, stuck) in self.panes.iter_mut().zip(marks) {
            if let Some(fit) = &mut pane.fit {
                fit.stuck = stuck;
            }
        }
    }

    /// Whether the shell's measure of a pane differs from the one held: it reports only then.
    pub fn wants_changed(&self, pane: PaneId, content: [f32; 2], outer: [f32; 2], window: [f32; 2]) -> bool {
        let Some(pane) = self.pane(pane) else { return false };
        match &pane.fit {
            Some(fit) => !fit.measured(content, outer, window),
            None => hosted::fit(content, outer, window).is_some(),
        }
    }

    /// Why `Action::Fit` on this pane (default: the focused one) would be refused, for a caller
    /// that should be told. `None`: it fits the pane, or there is nothing to fit (the window has
    /// the pane's size, or has not been measured since the layout last changed).
    pub fn fit_refusal(&self, pane: Option<PaneId>) -> Option<String> {
        let Some(id) = pane.or(self.focused) else { return Some("no pane to fit".into()) };
        let Some(pane) = self.pane(id) else { return Some(format!("no pane {id}")) };
        let kind = match pane.kind {
            Kind::App { .. } => None,
            Kind::Terminal => Some("a terminal"),
            Kind::Empty => Some("empty"),
        };
        if let Some(kind) = kind {
            return Some(format!("pane {id} is {kind}: fit is for an app pane whose window refused the pane's size"));
        }
        let stuck = pane.fit.as_ref()?.stuck?;
        Some(format!("pane {id} cannot be fitted: {}", stuck.why()))
    }

    /// Brings forward the tab that holds `id`, if it is a background tab.
    fn show_tab_of(&mut self, id: PaneId, effects: &mut Vec<Effect>) {
        let holds = |tab: &Tab| tab.layout.as_ref().is_some_and(|l| l.leaves().contains(&id));
        if let Some(index) = self.tabs.iter().position(holds) {
            self.select_tab(index, effects);
        }
    }

    /// The home screen, while it is on show: the tab on show has no panes.
    /// `keep_out`: where the shell draws text over the lattice (see `home::home`).
    pub fn home(&self, keep_out: &[Rect]) -> Option<Home<'_>> {
        self.layout.is_none().then(|| home::home(&self.tasks, keep_out))
    }

    /// Pane ids of each tab, in tab order. The state dump uses it to show which tab holds a pane.
    pub fn tab_panes(&self) -> Vec<Vec<PaneId>> {
        let leaves = |layout: &Option<Node>| layout.as_ref().map(Node::leaves).unwrap_or_default();
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| if i == self.active { leaves(&self.layout) } else { leaves(&tab.layout) })
            .collect()
    }

    /// The name of the listed repository or worktree at `path`.
    fn repo_name(&self, path: &str) -> Option<String> {
        let mut listed = self.repos.iter().flat_map(|r| {
            let worktrees = r.worktrees.iter().map(|w| (&w.path, &w.name));
            std::iter::once((&r.path, &r.name)).chain(worktrees)
        });
        listed.find(|(p, _)| *p == path).map(|(_, name)| name.clone())
    }

    /// True if `path` is a repository or worktree in the Repos list.
    pub fn repo_listed(&self, path: &str) -> bool {
        self.repo_name(path).is_some()
    }

    /// The Repos section's +: `dir` joins the settings' directories unless it is listed
    /// already, and list mode selects its first row. Nothing found there: a notice.
    fn repo_add(&mut self, dir: String, found: Vec<Repo>, effects: &mut Vec<Effect>) {
        let listed = self.repo_listed(&dir);
        let wanted: Vec<String> =
            if listed { vec![dir.clone()] } else { found.iter().map(|r| r.path.clone()).collect() };
        match self.repo_dirs.may_add(&dir, found.len(), listed) {
            Err(text) => return effects.push(Effect::Notice { text }),
            Ok(false) => {}
            Ok(true) => {
                let change = crate::settings::ListChange { key: "repos", add: true, text: dir.clone() };
                effects.push(Effect::SaveRepos { changes: vec![change] });
                self.repo_dirs.dirs.push(repos::RepoDir { text: dir.clone(), path: dir });
                self.repos = repos::arrange(self.repos.drain(..).chain(found).collect());
            }
        }
        self.leave(effects);
        self.sidebar = Some(Section::Repos);
        let row = |row: &ListRow| matches!(row, ListRow::Repo(path) if wanted.contains(path));
        self.list = self.list_rows().iter().position(row);
    }

    /// "Remove" on a repository's name: its directory comes off the settings, or a notice says
    /// why not. It stays listed while a directory that holds it is still named.
    fn repo_remove(&mut self, path: &str, effects: &mut Vec<Effect>) {
        match self.repo_dirs.to_remove(path) {
            Err(text) => effects.push(Effect::Notice { text }),
            Ok(dir) => {
                let change = crate::settings::ListChange { key: "repos", add: false, text: dir.text.clone() };
                effects.push(Effect::SaveRepos { changes: vec![change] });
                self.repo_dirs.dirs.retain(|d| *d != dir);
                let held = self.repo_dirs.holder(path).is_some();
                self.repos.retain(|repo| held || repo.path != path);
            }
        }
    }

    /// The `id` of the tab that holds `pane`, active tab included.
    pub fn tab_of(&self, pane: PaneId) -> Option<u32> {
        let index = self.tab_panes().iter().position(|panes| panes.contains(&pane))?;
        self.tabs.get(index).map(|tab| tab.id)
    }

    pub(crate) fn select_tab(&mut self, index: usize, effects: &mut Vec<Effect>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        // A live app window would stay on screen over the other tab.
        self.leave(effects);
        self.remember_tab();
        self.park_active();
        self.active = index;
        self.unpark_active();
    }

    /// Moves a pane to the tab at index `to`, beside that tab's focused pane, and shows it there.
    fn move_pane(&mut self, id: PaneId, to: usize, effects: &mut Vec<Effect>) {
        let from = self.tab_panes().iter().position(|panes| panes.contains(&id));
        if from.is_none() || from == Some(to) {
            return;
        }
        self.detach(id);
        self.select_tab(to, effects);
        self.layout = Some(match self.layout.take() {
            Some(root) => root.insert(self.focused, id, Axis::Right),
            None => Node::Leaf(id),
        });
        // Takes the keyboard back from an app that is live in this tab.
        self.focus(id, effects);
    }

    /// Puts pane `source` where it was dropped on pane `target`, in whichever tabs hold them.
    fn drop_pane(&mut self, source: PaneId, target: PaneId, zone: Zone, effects: &mut Vec<Effect>) {
        if source == target || self.pane(source).is_none() || self.pane(target).is_none() {
            return;
        }
        let (axis, before) = match zone {
            Zone::Left => (Axis::Right, true),
            Zone::Right => (Axis::Right, false),
            Zone::Top => (Axis::Down, true),
            Zone::Bottom => (Axis::Down, false),
            Zone::Center => {
                self.swap_panes(source, target);
                return self.focus(source, effects);
            }
        };
        // Already there: taking it out and putting it back would reset the split's ratio.
        let here = |tree: &Node| tree.is_split(target, source, axis, before);
        let parked = self.tabs.iter().filter_map(|tab| tab.layout.as_ref());
        if !self.layout.iter().chain(parked).any(here) {
            self.detach(source);
            let trees = std::iter::once(&mut self.layout).chain(self.tabs.iter_mut().map(|tab| &mut tab.layout));
            for tree in trees.flatten() {
                tree.split(target, source, axis, before);
            }
        }
        self.focus(source, effects);
    }

    /// Two panes trade places, in one tab or across two. A tab whose focused pane left it
    /// focuses the pane that came instead.
    fn swap_panes(&mut self, x: PaneId, y: PaneId) {
        let active = (&mut self.layout, &mut self.focused);
        let parked = self.tabs.iter_mut().map(|tab| (&mut tab.layout, &mut tab.focused));
        for (layout, focused) in std::iter::once(active).chain(parked) {
            let Some(tree) = layout else { continue };
            tree.swap(x, y);
            let leaves = tree.leaves();
            if focused.is_some_and(|f| !leaves.contains(&f)) {
                *focused = [x, y].into_iter().find(|id| leaves.contains(id));
            }
        }
    }

    /// Takes a pane out of whichever tab's tree holds it. Focus there goes to the newest pane left.
    fn detach(&mut self, id: PaneId) {
        self.layout = self.layout.take().and_then(|root| root.remove(id));
        for tab in &mut self.tabs {
            tab.layout = tab.layout.take().and_then(|root| root.remove(id));
            if tab.focused == Some(id) {
                tab.focused = tab.layout.as_ref().and_then(|l| l.leaves().into_iter().max());
            }
        }
        if self.focused == Some(id) {
            // The newest pane left in this tab, not in some other tab.
            self.focused = self.layout.as_ref().and_then(|l| l.leaves().into_iter().max());
        }
    }

    /// Removes a pane from whichever tab holds it.
    fn close_pane(&mut self, id: PaneId, effects: &mut Vec<Effect>) {
        let Some(i) = self.panes.iter().position(|p| p.id == id) else { return };
        let closed = self.panes.remove(i);
        self.detach(id);
        self.marks.forget_pane(id);
        if self.live == Some(id) {
            self.live = None;
        }
        if self.visual == Some(id) {
            self.visual = None;
        }
        effects.push(Effect::Closed(closed));
    }

    fn add(&mut self, kind: Kind, beside: Option<PaneId>, axis: Axis) -> PaneId {
        self.next_id += 1;
        let id = self.next_id;
        self.layout = Some(match self.layout.take() {
            Some(root) => root.insert(beside, id, axis),
            None => Node::Leaf(id),
        });
        self.panes.push(Pane { id, kind, cwd: None, fit: None });
        self.focused = Some(id);
        id
    }

    /// Focusing any pane but the live one takes the keyboard back.
    pub(crate) fn focus(&mut self, id: PaneId, effects: &mut Vec<Effect>) {
        if self.pane(id).is_none() {
            return;
        }
        self.show_tab_of(id, effects);
        if self.live != Some(id) {
            self.leave(effects);
        }
        self.focused = Some(id);
    }

    /// The peeked window goes back off screen, as on leaving.
    fn end_peek(&mut self, effects: &mut Vec<Effect>) {
        if let Some(peek) = self.peek.take() {
            effects.push(Effect::Parked(peek.pane));
        }
    }

    /// A peek may start over `pane`: a parked app pane on show, with nothing over the panes that
    /// holds the user's attention (sneak mode, the palette, a box, the quit question).
    fn peekable(&self, pane: PaneId) -> bool {
        let app = self.pane(pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
        let shown = self.layout.as_ref().is_some_and(|l| l.leaves().contains(&pane));
        let mode = matches!(self.mode(), Mode::Normal | Mode::List | Mode::Talk);
        app && shown && self.live.is_none() && mode && !self.keys && !self.ax_overlay
    }

    pub(crate) fn leave_visual(&mut self, effects: &mut Vec<Effect>) {
        if let Some(pane) = self.visual.take() {
            effects.push(Effect::Visual { pane, on: false });
        }
    }

    pub(crate) fn leave(&mut self, effects: &mut Vec<Effect>) {
        if let Some(id) = self.live.take() {
            effects.push(Effect::Parked(id));
        }
        self.end_peek(effects);
        // Whatever takes the keyboard from an app takes it from the conversation too.
        if let Some(talk) = &mut self.talk {
            talk.keys = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Motion;
    use crate::layout::Axis::{Down, Right};
    use crate::transcript::View;

    mod prop;

    fn app(window: u32) -> Kind {
        Kind::App { window, title: format!("w{window}"), bundle: None, name: "app".into() }
    }

    fn ids(model: &Model) -> Vec<PaneId> {
        model.panes.iter().map(|p| p.id).collect()
    }

    fn control(id: &str, x: f64) -> Target {
        let frame = crate::ax::Frame { x, y: 0., w: 20., h: 20. };
        Target { frame, source: crate::ax::Source::Own { id: id.into() } }
    }

    /// `n` controls in a row: `c0`, `c1`, ...
    fn controls(n: usize) -> Vec<Target> {
        (0..n).map(|i| control(&format!("c{i}"), i as f64 * 30.)).collect()
    }

    fn sneak(m: &mut Model, n: usize) -> Vec<Effect> {
        m.update(Action::Sneak { targets: controls(n) })
    }

    /// The label on the control `id`, if it has one.
    fn label_of(m: &Model, id: &str) -> Option<String> {
        let own = crate::ax::Source::Own { id: id.into() };
        m.sneak.as_ref()?.targets.iter().find(|t| t.target.source == own).map(|t| t.label.clone())
    }

    fn typed(m: &Model) -> Option<&str> {
        m.sneak.as_ref().map(|s| s.typed.as_str())
    }

    #[test]
    fn sneak_labels_the_controls_and_a_typed_label_activates_one() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!((sneak(&mut m, 3), m.mode()), (vec![], Mode::Sneak));
        // The terminal does not have the keyboard: ⌘V must not paste under the labels.
        assert_eq!(m.keyboard_terminal(), None);
        let effects = m.update(Action::SneakType { text: "S".into() });
        assert_eq!((effects, m.mode()), (vec![Effect::Activate { id: "c1".into() }], Mode::Normal));
        // Out of the mode, a letter is no label.
        assert_eq!(m.update(Action::SneakType { text: "a".into() }), []);
    }

    #[test]
    fn a_letter_of_no_label_leaves_sneak_and_the_letters_after_it_are_dropped() {
        let mut m = Model::default();
        sneak(&mut m, 3);
        // `z` is no label: the mode ends, and the `a` that came with it activates nothing.
        assert_eq!((m.update(Action::SneakType { text: "za".into() }), m.mode()), (vec![], Mode::Normal));
        // After the first letter of a two-letter label too.
        sneak(&mut m, 30);
        m.update(Action::SneakType { text: "p".into() });
        assert_eq!(typed(&m), Some("p"));
        assert_eq!((m.update(Action::SneakType { text: "z".into() }), m.mode()), (vec![], Mode::Normal));
        // A spelled label ends the mode: the `d` after it is typed nowhere.
        sneak(&mut m, 3);
        assert_eq!(m.update(Action::SneakType { text: "sd".into() }), [Effect::Activate { id: "c1".into() }]);
    }

    #[test]
    fn a_two_letter_sneak_label_narrows_then_activates_and_backspace_takes_a_letter_back() {
        let mut m = Model::default();
        sneak(&mut m, 30);
        let two = m.sneak.as_ref().unwrap().targets.iter().find(|t| t.label.len() == 2).unwrap().clone();
        let (first, second) = two.label.split_at(1);
        assert_eq!(m.update(Action::SneakType { text: first.into() }), []);
        assert_eq!(typed(&m), Some(first));
        m.update(Action::SneakBackspace);
        assert_eq!(m.sneak.as_ref().map(|s| (s.typed.as_str(), s.visible().count())), Some(("", 30)));
        m.update(Action::SneakType { text: first.into() });
        let crate::ax::Source::Own { id } = two.target.source else { panic!("own") };
        assert_eq!(m.update(Action::SneakType { text: second.into() }), [Effect::Activate { id }]);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn sneak_ends_on_the_users_moves_and_never_on_news() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        sneak(&mut m, 3);
        assert_eq!((m.update(Action::LeaveSneak), m.mode(), m.focused), (vec![], Mode::Normal, Some(1)));
        // Its own key again leaves.
        sneak(&mut m, 3);
        assert_eq!((sneak(&mut m, 3), m.mode()), (vec![], Mode::Normal));
        // A move ends it, and still happens: a click's `Focus`, a split, the picker.
        for action in [
            Action::Focus { pane: 1 },
            Action::Split { axis: Right, kind: Kind::Empty },
            Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] },
        ] {
            sneak(&mut m, 3);
            assert_eq!(m.mode(), Mode::Sneak);
            m.update(action);
            assert!(m.sneak.is_none());
        }
        assert_eq!((m.panes.len(), m.mode()), (2, Mode::Pick));
        m.update(Action::ClosePicker);
        // News does not, whatever it changes: the clock, a list read again, an agent's task, a
        // window an app opened, a shell that exited (the shell closes its pane by name).
        sneak(&mut m, 3);
        for news in [
            Action::Tick { now: 1 },
            Action::SetApps { names: vec!["Notes".into()] },
            Action::SetRepos { repos: Vec::new() },
            Action::Adopt { kind: app(7), beside: None },
            Action::Close { pane: Some(2) },
            Action::SneakDrawn { targets: controls(3) },
        ] {
            m.update(news.clone().sent_by(Origin::System));
            assert_eq!((m.mode(), typed(&m)), (Mode::Sneak, Some("")), "{news:?}");
        }
        assert!(m.pane(2).is_none(), "the dead pane is closed all the same");
        // So the letter after it is still a label's, not a terminal's.
        assert_eq!(m.update(Action::SneakType { text: "a".into() }), [Effect::Activate { id: "c0".into() }]);
    }

    #[test]
    fn a_socket_clients_request_runs_without_ending_sneak_and_the_labels_follow() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::Sneak { targets: vec![control("pane:1", 0.), control("rail:tasks", 300.)] });
        // An agent splits, moves focus and files a task while the user reads the labels.
        for request in [
            Action::Split { axis: Right, kind: Kind::Empty },
            Action::Focus { pane: 1 },
            Action::TaskAdd {
                title: "t".into(),
                priority: 3,
                state: State::Running,
                tab: None,
                assign: Assign::default(),
            },
        ] {
            m.update(request.clone().sent_by(Origin::Socket));
            assert_eq!(m.mode(), Mode::Sneak, "{request:?}");
        }
        assert_eq!((ids(&m), m.focused, m.tasks.list.len()), (vec![1, 2], Some(1), 1), "each request ran");
        // The frame after: pane 1 is narrower, pane 2 is new and has no label.
        let drawn = vec![control("pane:1", 0.), control("pane:2", 150.), control("rail:tasks", 300.)];
        m.update(Action::SneakDrawn { targets: drawn }.sent_by(Origin::System));
        assert_eq!((label_of(&m, "pane:1"), label_of(&m, "pane:2")), (Some("a".into()), None));
        // The same move by the user ends the mode, and runs.
        m.update(Action::Focus { pane: 2 });
        assert_eq!((m.mode(), m.focused), (Mode::Normal, Some(2)));
        // A pane closed by name: the shell's own, for a dead pane, keeps the mode; a click on its × ends it.
        sneak(&mut m, 3);
        m.update(Action::Close { pane: Some(2) }.sent_by(Origin::System));
        assert_eq!((m.mode(), ids(&m)), (Mode::Sneak, vec![1]));
        m.update(Action::Close { pane: Some(1) });
        assert_eq!((m.mode(), ids(&m)), (Mode::Normal, vec![]));
    }

    #[test]
    fn a_label_typed_through_the_socket_is_input_to_sneak_mode() {
        let socket = |action: Action| action.sent_by(Origin::Socket);
        let mut m = Model::default();
        // The `key` request: the mode's own key, letters, ⌫ and Esc, as from the keyboard.
        m.update(socket(Action::Sneak { targets: controls(30) }));
        assert_eq!(m.mode(), Mode::Sneak);
        m.update(socket(Action::SneakType { text: "p".into() }));
        assert_eq!(typed(&m), Some("p"));
        m.update(socket(Action::SneakBackspace));
        assert_eq!(typed(&m), Some(""));
        let effects = m.update(socket(Action::SneakType { text: "s".into() }));
        assert_eq!((effects, m.mode()), (vec![Effect::Activate { id: "c1".into() }], Mode::Normal));
        // Esc, the mode's key again, and a letter of no label leave.
        for leave in
            [Action::LeaveSneak, Action::Sneak { targets: controls(3) }, Action::SneakType { text: "z".into() }]
        {
            sneak(&mut m, 3);
            assert_eq!((m.update(socket(leave)), m.mode()), (vec![], Mode::Normal));
        }
    }

    #[test]
    fn under_sneak_a_client_or_news_cannot_take_the_keyboard_away() {
        let takers = || {
            [
                Action::Enter { pane: 1 },
                Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] },
                Action::OpenCompose { text: "a draft".into() },
                Action::ToggleKeys,
                Action::ToggleVisual,
                Action::ToggleAxOverlay,
            ]
        };
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        sneak(&mut m, 3);
        let taken =
            |m: &Model| (m.mode(), m.picker.is_some(), m.compose.is_some(), m.keys, m.visual, m.ax_overlay, m.live);
        let before = taken(&m);
        assert_eq!(before.0, Mode::Sneak);
        for origin in [Origin::Socket, Origin::System] {
            for action in takers() {
                assert_eq!(m.update(action.clone().sent_by(origin)), [], "{action:?} from {origin:?}");
                assert_eq!(taken(&m), before, "{action:?} from {origin:?}");
            }
        }
        // The user's own does both: leaves the mode and acts.
        assert_eq!((m.update(Action::Enter { pane: 1 }), m.mode()), (vec![Effect::Live(1)], Mode::App));
    }

    #[test]
    fn only_the_user_closes_the_picker_by_opening_the_key_reference() {
        let mut m = Model::default();
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&Action::ToggleKeys, origin), Some("the picker or the new-task box is open"));
            m.update(Action::ToggleKeys.sent_by(origin));
            assert_eq!((m.mode(), m.keys), (Mode::Pick, false), "{origin:?}");
        }
        assert_eq!(m.refusal(&Action::ToggleKeys, Origin::User), None);
        m.update(Action::ToggleKeys);
        assert_eq!((m.picker.is_none(), m.keys), (true, true));
        // A client may still hide a reference that is open: that request is aimed at it.
        m.update(Action::ToggleKeys.sent_by(Origin::Socket));
        assert!(!m.keys);
        // The new-task box is kept the same way.
        m.update(Action::OpenCompose { text: String::new() });
        m.update(Action::ToggleKeys.sent_by(Origin::Socket));
        assert_eq!((m.mode(), m.keys), (Mode::Compose, false));
    }

    /// Every `Action` variant, once, in the enum's order. `named` is a match with no catch-all:
    /// a new variant does not compile until it is named here. `COUNT` then grows, and the tests
    /// over `samples` fail until they have one of it.
    macro_rules! every_action {
        ($($variant:pat),* $(,)?) => {
            const COUNT: usize = [$(stringify!($variant)),*].len();

            /// The variant's place in the list.
            fn index(action: &Action) -> usize {
                let listed = [$(matches!(action, $variant)),*];
                listed.iter().position(|is| *is).unwrap_or(COUNT)
            }

            // Never called: it exists to fail the build on an unnamed variant.
            #[allow(dead_code)]
            fn named(action: &Action) {
                match action {
                    $($variant => {})*
                }
            }
        };
    }
    every_action!(
        Action::Split { .. },
        Action::Open { .. },
        Action::Adopt { .. },
        Action::Close { .. },
        Action::Focus { .. },
        Action::FocusDir { .. },
        Action::FocusCycle { .. },
        Action::Resize { .. },
        Action::SwapDir { .. },
        Action::MovePane { .. },
        Action::Equalize,
        Action::DropPane { .. },
        Action::PaneWants { .. },
        Action::Fit { .. },
        Action::Enter { .. },
        Action::Leave,
        Action::ToggleSection { .. },
        Action::CloseSidebar,
        Action::OpenPicker { .. },
        Action::NewTab,
        Action::NewEmptyTab,
        Action::SelectTab { .. },
        Action::CycleTab { .. },
        Action::CloseTab { .. },
        Action::PickerQuery { .. },
        Action::SetInstalled { .. },
        Action::PickerMove { .. },
        Action::PickerChoose { .. },
        Action::ClosePicker,
        Action::PaletteOpen,
        Action::PaletteTasks,
        Action::PaletteMove { .. },
        Action::PaletteChoose { .. },
        Action::ClosePalette,
        Action::InspectorOpen { .. },
        Action::InspectorRead { .. },
        Action::InspectorFailed { .. },
        Action::InspectorClose,
        Action::InspectorFold { .. },
        Action::InspectorSelect { .. },
        Action::InspectorMove { .. },
        Action::InspectorFilter { .. },
        Action::ToggleKeys,
        Action::CloseKeys,
        Action::ToggleVisual,
        Action::LeaveVisual,
        Action::ToggleAxOverlay,
        Action::SetRepos { .. },
        Action::SetRepoDirs { .. },
        Action::RepoAdd { .. },
        Action::RepoRemove { .. },
        Action::OpenRepo { .. },
        Action::RestoreTasks { .. },
        Action::TaskAdd { .. },
        Action::SetKnown { .. },
        Action::SetTerminals { .. },
        Action::ComposeEscape,
        Action::ComposeMove { .. },
        Action::ComposeAccept { .. },
        Action::ComposeAttach { .. },
        Action::ComposeRef { .. },
        Action::ComposePick { .. },
        Action::TaskSetState { .. },
        Action::TaskSnooze { .. },
        Action::TaskHold { .. },
        Action::TaskDismiss { .. },
        Action::TaskUndo,
        Action::RowSnooze { .. },
        Action::RowHold { .. },
        Action::RowDismiss { .. },
        Action::TaskGo { .. },
        Action::NewTabOf { .. },
        Action::TabPlace { .. },
        Action::SelectVisibleTab { .. },
        Action::Tick { .. },
        Action::OpenCompose { .. },
        Action::FieldEdit { .. },
        Action::ComposeComplete,
        Action::ComposeSubmit { .. },
        Action::CloseCompose,
        Action::TaskFromLine { .. },
        Action::ToggleGroup { .. },
        Action::TasksView { .. },
        Action::ToggleProject { .. },
        Action::SessionWork { .. },
        Action::SetSessions { .. },
        Action::SessionQuiet { .. },
        Action::SessionsSeen { .. },
        Action::SessionAdopt { .. },
        Action::SessionHide { .. },
        Action::SessionClick { .. },
        Action::SessionOpen { .. },
        Action::SessionsRange { .. },
        Action::SetApps { .. },
        Action::SetAppSettings { .. },
        Action::SetDock { .. },
        Action::AppPin { .. },
        Action::AppHide { .. },
        Action::AppsUndo,
        Action::ListPin,
        Action::AppsToggle { .. },
        Action::AppsFilter { .. },
        Action::FocusList,
        Action::LeaveList,
        Action::ListMove { .. },
        Action::ListSection { .. },
        Action::ListChoose { .. },
        Action::ListSnooze { .. },
        Action::ListHold,
        Action::ListDismiss,
        Action::Sneak { .. },
        Action::SneakEnter { .. },
        Action::SneakDrawn { .. },
        Action::SneakType { .. },
        Action::SneakBackspace,
        Action::LeaveSneak,
        Action::SneakApp { .. },
        Action::AppWheel { .. },
        Action::PeekCheck { .. },
        Action::QuitAsked { .. },
        Action::QuitConfirm,
        Action::QuitCancel,
        Action::Quit,
        Action::RestoreMarks { .. },
        Action::Titles { .. },
        Action::Unfilled { .. },
        Action::MarkUndo,
        Action::ListUndo,
        Action::MarkAdd { .. },
        Action::MarkToggle { .. },
        Action::MarkRemove { .. },
        Action::MarkRename { .. },
        Action::MarkGo { .. },
        Action::MarkBind { .. },
        Action::MarksChecked { .. },
        Action::Fill { .. },
        Action::MarkPrefix,
        Action::MarkEdit { .. },
        Action::MarkType { .. },
        Action::MarkBackspace,
        Action::MarkSubmit,
        Action::MarkCancel,
        Action::ListMarkEdit { .. },
        Action::ListReorder { .. },
        Action::CaptureStart { .. },
        Action::CaptureTaken { .. },
        Action::CaptureFacts { .. },
        Action::CaptureContent { .. },
        Action::AnnotatePress { .. },
        Action::AnnotateDrag { .. },
        Action::AnnotateRelease { .. },
        Action::AnnotateTool { .. },
        Action::AnnotateUndo,
        Action::AnnotateRedo,
        Action::AnnotateRemove,
        Action::AnnotateNote { .. },
        Action::AnnotateLeave { .. },
        Action::CaptureEdit { .. },
        Action::CaptureRemove { .. },
        Action::CaptureCancel,
        Action::CaptureDiscard { .. },
        Action::CaptureTitle,
        Action::CaptureBack,
        Action::BriefShow { .. },
        Action::BriefClose,
        Action::RestoreWorkbooks { .. },
        Action::RestoreLayout { .. },
        Action::StartFresh,
        Action::UnfilledOpen { .. },
        Action::WorkbookSave { .. },
        Action::WorkbookAddTab { .. },
        Action::WorkbookSaveAll { .. },
        Action::WorkbookOpen { .. },
        Action::WorkbookEdit { .. },
        Action::WorkbookRemove { .. },
        Action::WorkbookUndo,
        Action::SetAgents { .. },
        Action::TaskTalk { .. },
        Action::ComposeEnter,
        Action::AgentStarted { .. },
        Action::AgentSessions { .. },
        Action::SessionMissing { .. },
        Action::AgentExited { .. },
        Action::SessionRead { .. },
        Action::TaskStop { .. },
        Action::TalkToggle,
        Action::TalkGo,
        Action::TalkKeys { .. },
        Action::TalkResize { .. },
        Action::TalkWide,
        Action::TalkPick { .. },
        Action::SetCalendar { .. },
        Action::CalendarRead { .. },
        Action::CallSeen { .. },
        Action::CalendarDismiss { .. },
        Action::CalendarJoin { .. },
        Action::CalendarTomorrow,
        Action::AgentEvents { .. },
        Action::TalkSend,
        Action::TalkAnswer { .. },
        Action::TalkExpand { .. },
        Action::TalkDenyNote,
        Action::TalkEscape,
        Action::TalkInterrupt,
        Action::TalkView { .. },
        Action::WorkbookFold { .. }
    );

    /// One of each action.
    fn samples() -> Vec<Action> {
        let text = String::new;
        let samples = vec![
            Action::Split { axis: Right, kind: Kind::Empty },
            Action::Open { kind: app(1), axis: Right, beside: None },
            Action::Adopt { kind: app(1), beside: None },
            Action::DropPane { source: 1, target: 1, zone: Zone::Left },
            Action::PaneWants { pane: 1, content: [400., 600.], outer: [406., 638.], window: [520., 600.] },
            Action::Fit { pane: None },
            Action::Close { pane: None },
            Action::Focus { pane: 1 },
            Action::FocusDir { dir: Dir::Left },
            Action::FocusCycle { delta: 1 },
            Action::Resize { dir: Dir::Left },
            Action::SwapDir { dir: Dir::Left },
            Action::MovePane { pane: None, tab: 0 },
            Action::Equalize,
            Action::Enter { pane: 1 },
            Action::Leave,
            Action::ToggleSection { section: Section::Apps },
            Action::CloseSidebar,
            Action::OpenPicker { axis: Right, new_tab: false, items: Vec::new() },
            Action::NewTab,
            Action::NewEmptyTab,
            Action::SelectTab { index: 0 },
            Action::CycleTab { delta: 1 },
            Action::CloseTab { index: None },
            Action::PickerQuery { query: text() },
            Action::SetInstalled { apps: Vec::new(), running: Vec::new() },
            Action::PickerMove { delta: 1 },
            Action::PickerChoose { index: None },
            Action::ClosePicker,
            Action::PaletteOpen,
            Action::PaletteTasks,
            Action::PaletteMove { delta: 1 },
            Action::PaletteChoose { index: None },
            Action::ClosePalette,
            Action::InspectorOpen { pane: 1, app: text() },
            Action::InspectorRead {
                pane: 1,
                tree: crate::ax::Tree { root: Default::default(), count: 1, truncated: Default::default() },
                window: Default::default(),
                millis: 1.0,
            },
            Action::InspectorFailed { pane: 1, why: text() },
            Action::InspectorClose,
            Action::InspectorFold { path: vec![0] },
            Action::InspectorSelect { path: None },
            Action::InspectorMove { delta: 1 },
            Action::InspectorFilter { on: true },
            Action::ToggleKeys,
            Action::CloseKeys,
            Action::ToggleVisual,
            Action::LeaveVisual,
            Action::ToggleAxOverlay,
            Action::SetRepos { repos: Vec::new() },
            Action::SetRepoDirs { dirs: repos::RepoDirs::default() },
            Action::RepoAdd { dir: text(), found: Vec::new() },
            Action::RepoRemove { path: text() },
            Action::OpenRepo { path: text(), axis: Right },
            Action::RestoreTasks { tasks: Vec::new(), floor: 0 },
            Action::TaskAdd { title: text(), priority: 3, state: State::Running, tab: None, assign: Assign::default() },
            Action::SetKnown { line: text(), typing: None, names: None, paths: Vec::new(), entries: Vec::new() },
            Action::SetTerminals { terminals: Vec::new(), home: None },
            Action::ComposeEscape,
            Action::ComposeMove { delta: 1 },
            Action::ComposeAccept { index: 0 },
            Action::ComposeAttach { paths: Vec::new(), home: None, unreadable: 0 },
            Action::ComposeRef { target: Pointed::Tab(1) },
            Action::ComposePick { mark: '>', name: None },
            Action::TaskSetState { id: 1, state: State::Done },
            Action::TaskSnooze { id: 1, until: 0 },
            Action::TaskHold { id: 1 },
            Action::TaskDismiss { id: 1 },
            Action::TaskUndo,
            Action::TaskGo { id: None },
            Action::NewTabOf { task: None },
            Action::TabPlace { tab: None, at: TabAt::End(None) },
            Action::SelectVisibleTab { index: 0 },
            Action::Tick { now: 0 },
            Action::OpenCompose { text: text() },
            Action::FieldEdit { edit: Edit::Insert(text()) },
            Action::ComposeComplete,
            Action::ComposeSubmit { open: false },
            Action::CloseCompose,
            Action::ToggleGroup { heading: Heading::Done },
            Action::SetSessions { found: Vec::new(), day: 0 },
            Action::SessionQuiet { ms: 0 },
            Action::SessionsSeen { seen: Vec::new() },
            Action::SessionAdopt { runner: text(), id: text() },
            Action::SessionHide { runner: text(), id: text() },
            Action::SessionClick { runner: text(), id: text() },
            Action::SessionOpen { runner: text(), id: text() },
            Action::SessionsRange { range: sessions::Range::All },
            Action::RowSnooze { id: 1, until: 0 },
            Action::RowHold { id: 1 },
            Action::RowDismiss { id: 1 },
            Action::TaskFromLine { line: text(), known: Default::default(), open: false },
            Action::SetApps { names: Vec::new() },
            Action::SetAppSettings { pinned: Vec::new(), hidden: Vec::new(), suggest: true },
            Action::SetDock { apps: Vec::new() },
            Action::AppPin { path: text() },
            Action::AppHide { path: text() },
            Action::AppsUndo,
            Action::ListPin,
            Action::AppsToggle { group: apps::Group::All },
            Action::AppsFilter { on: true },
            Action::FocusList,
            Action::LeaveList,
            Action::ListMove { delta: 1 },
            Action::ListSection { delta: 1 },
            Action::ListChoose { axis: Right },
            Action::ListSnooze { until: 0 },
            Action::ListHold,
            Action::ListDismiss,
            Action::Sneak { targets: Vec::new() },
            Action::SneakEnter { targets: Vec::new() },
            Action::SneakDrawn { targets: controls(3) },
            Action::SneakType { text: text() },
            Action::SneakBackspace,
            Action::LeaveSneak,
            Action::SneakApp { pane: 1, read: None },
            Action::AppWheel { pane: 1, now: 0 },
            Action::PeekCheck { now: 0, over: true, pressed: true, scrolled: None },
            Action::QuitAsked { running: Vec::new() },
            Action::QuitConfirm,
            Action::QuitCancel,
            Action::Quit,
            Action::RestoreMarks { marks: Vec::new(), extra: Extra::new() },
            Action::Titles { titles: Vec::new() },
            Action::Unfilled { pane: 1, why: text() },
            Action::MarkUndo,
            Action::ListUndo,
            Action::MarkAdd { of: MarkOf::Pane(None), name: None, key: None, uid: text(), cwds: Vec::new() },
            Action::MarkToggle { of: MarkOf::Tab(None), uid: text(), cwds: Vec::new() },
            Action::MarkRemove { mark: MarkRef::Place(0) },
            Action::MarkRename { mark: MarkRef::Place(0), name: text() },
            Action::MarkGo { mark: MarkRef::Place(0), axis: Right },
            Action::MarkBind { mark: 1, pane: 1 },
            Action::MarksChecked { missing: Vec::new() },
            Action::Fill { pane: 1, kind: app(1) },
            Action::MarkPrefix,
            Action::MarkEdit { mark: MarkRef::Place(0), field: marks::Field::Name },
            Action::MarkType { text: text() },
            Action::MarkBackspace,
            Action::MarkSubmit,
            Action::MarkCancel,
            Action::ListMarkEdit { field: marks::Field::Key },
            Action::ListReorder { delta: 1 },
            Action::CaptureStart { pane: None, draft: text(), word: text(), facts: Vec::new(), images: true },
            Action::CaptureTaken { draft: text(), material: 1, picture: true, stale: false },
            Action::CaptureFacts { facts: Vec::new() },
            Action::CaptureContent { draft: text(), material: 1, content: Content::Missing(text()) },
            Action::AnnotatePress { at: [0.5, 0.5], shift: false, alt: false, size: [100., 100.] },
            Action::AnnotateDrag { to: [0.6, 0.6] },
            Action::AnnotateRelease { at: [0.6, 0.6] },
            Action::AnnotateTool { tool: Tool::Box },
            Action::AnnotateUndo,
            Action::AnnotateRedo,
            Action::AnnotateRemove,
            Action::AnnotateNote { keep: true },
            Action::AnnotateLeave { title: false },
            Action::CaptureEdit { material: 1 },
            Action::CaptureRemove { material: 1 },
            Action::CaptureCancel,
            Action::CaptureDiscard { yes: true },
            Action::CaptureTitle,
            Action::CaptureBack,
            Action::BriefShow { id: 1 },
            Action::BriefClose,
            Action::RestoreWorkbooks { workbooks: Vec::new() },
            Action::RestoreLayout {
                layout: Box::new(Model::default().layout_file(&[], &[], 0)),
                gone: Vec::new(),
                home: None,
                launch: false,
            },
            Action::StartFresh,
            Action::UnfilledOpen { pane: 1 },
            Action::WorkbookSave { uid: text(), cwds: Vec::new(), histories: Vec::new() },
            Action::WorkbookAddTab { workbook: 1, cwds: Vec::new(), histories: Vec::new() },
            Action::WorkbookSaveAll { uid: text(), cwds: Vec::new(), histories: Vec::new() },
            Action::WorkbookOpen { workbook: 1, tab: None, home: None },
            Action::WorkbookEdit { workbook: 1, tab: None },
            Action::WorkbookRemove { workbook: 1, tab: None },
            Action::WorkbookFold { workbook: 1 },
            Action::WorkbookUndo,
            Action::SetAgents { claude: Vec::new(), codex: Vec::new(), default: None, shows: View::Native },
            Action::TaskTalk { id: 1 },
            Action::ComposeEnter,
            Action::AgentStarted { task: Subject::Task(1), session: None, at: 0 },
            Action::AgentSessions { found: Vec::new(), home: None, now: 0 },
            Action::SessionMissing { task: 1, id: String::new() },
            Action::AgentExited { task: Subject::Task(1) },
            Action::SessionRead { runner: "claude".into(), id: String::new(), found: None },
            Action::TaskStop { id: Subject::Task(1) },
            Action::TalkToggle,
            Action::TalkGo,
            Action::TalkKeys { on: true },
            Action::TalkResize { width: 400. },
            Action::TalkWide,
            Action::TalkPick { id: Subject::Task(1) },
            Action::SetCalendar { on: true, notify: true, links_only: false },
            Action::CalendarRead { events: None, days: [0; 3], at: 0 },
            Action::CallSeen { in_call: Some(false), at: 0 },
            Action::CalendarDismiss { id: 1 },
            Action::CalendarJoin { id: 1 },
            Action::CalendarTomorrow,
            Action::AgentEvents { task: Subject::Task(1), events: Vec::new(), at: 0 },
            Action::TalkSend,
            Action::TalkAnswer { allow: false, ticket: 1 },
            Action::TalkExpand { ticket: 1 },
            Action::TalkDenyNote,
            Action::TalkEscape,
            Action::TalkInterrupt,
            Action::TalkView { shows: View::Terminal },
            Action::TasksView { view: TasksView::Project },
            Action::ToggleProject { project: None, done: false },
            Action::SessionWork { work: progress::Work::default() },
        ];
        let mut places: Vec<usize> = samples.iter().map(index).collect();
        places.sort_unstable();
        assert_eq!(places, (0..COUNT).collect::<Vec<_>>(), "one sample of every action");
        samples
    }

    /// What reaches a native agent: the user's alone, on any ground.
    fn talks(action: &Action) -> bool {
        matches!(
            action,
            Action::TalkSend
                | Action::TalkAnswer { .. }
                | Action::TalkExpand { .. }
                | Action::TalkDenyNote
                | Action::TalkEscape
                | Action::TalkInterrupt
                | Action::TalkView { .. }
        )
    }

    /// A pin, a hide or their undo: refused from all but the user on its own ground.
    fn writes_settings(action: &Action) -> bool {
        let apps =
            matches!(action, Action::AppPin { .. } | Action::AppHide { .. } | Action::AppsUndo | Action::ListPin);
        apps || matches!(action, Action::RepoAdd { .. } | Action::RepoRemove { .. })
    }

    /// A workbook saved, opened, renamed, deleted or put back: the user's alone, as those are.
    fn of_workbooks(action: &Action) -> bool {
        matches!(
            action,
            Action::WorkbookSave { .. }
                | Action::WorkbookAddTab { .. }
                | Action::WorkbookSaveAll { .. }
                | Action::WorkbookOpen { .. }
                | Action::WorkbookEdit { .. }
                | Action::WorkbookRemove { .. }
                | Action::WorkbookUndo
        )
    }

    /// A move in the palette: the user's alone. Opening it takes the keyboard, so a mode in its
    /// way says its own reason first.
    fn in_palette(action: &Action) -> bool {
        matches!(action, Action::PaletteMove { .. } | Action::PaletteChoose { .. } | Action::ClosePalette)
            || matches!(
                action,
                Action::InspectorOpen { .. }
                    | Action::InspectorClose
                    | Action::InspectorFold { .. }
                    | Action::InspectorSelect { .. }
                    | Action::InspectorMove { .. }
                    | Action::InspectorFilter { on: false }
            )
    }

    #[test]
    fn the_inspector_takes_the_drawers_place_and_gives_it_back() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        // Not over the new-task box while it has the keyboard: the box would type unseen.
        m.update(Action::OpenCompose { text: "fix".into() });
        m.update(Action::InspectorOpen { pane: 1, app: "A".into() });
        assert!(m.inspector.is_none());
        m.update(Action::CloseCompose);
        // A drawer that closes under it leaves it there; what the drawer is asked to show next takes the place back.
        m.update(Action::InspectorOpen { pane: 1, app: "A".into() });
        assert!(m.inspector.is_some());
        m.update(Action::OpenCompose { text: "fix".into() });
        assert_eq!((m.inspector.is_none(), m.compose_line().cloned()), (true, Some("fix".into())));
    }

    #[test]
    fn the_palette_runs_the_chosen_command_for_the_user_alone() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        // A client opens nothing, and the user's ⌘P opens it.
        assert!(m.update(Action::PaletteOpen.sent_by(Origin::Socket)).is_empty());
        assert_eq!(m.mode(), Mode::Normal);
        m.update(Action::PaletteOpen);
        assert_eq!((m.mode(), m.field().map(|(of, _)| of)), (Mode::Palette, Some("palette")));
        // A client types nothing in it and chooses nothing.
        m.update(Action::FieldEdit { edit: Edit::Insert("quit".into()) }.sent_by(Origin::Socket));
        assert!(m.update(Action::PaletteChoose { index: None }.sent_by(Origin::Socket)).is_empty());
        assert_eq!(m.palette.as_ref().map(|p| p.query.text().to_string()), Some(String::new()));
        // Behind it, pane actions wait, and no other box opens.
        assert_eq!(m.waits(&Action::Close { pane: None }), Some(commands::PALETTE_OPEN));
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!((m.picker.is_none(), m.compose.is_none(), m.panes.len()), (true, true, 1));
        // Typing narrows it to its first match; ↓ wraps; ↵ runs the highlighted one and closes it.
        m.update(Action::FieldEdit { edit: Edit::Insert("next".into()) });
        let rows = m.palette.as_ref().map(Palette::matches).unwrap_or_default();
        assert_eq!(rows[..2], [Command::FocusNext, Command::NextTab]);
        m.update(Action::PaletteMove { delta: rows.len() as i32 + 1 });
        assert_eq!(m.update(Action::PaletteChoose { index: None }), vec![Effect::Run { command: Command::NextTab }]);
        assert_eq!((m.palette.is_none(), m.mode()), (true, Mode::Normal));
        // Nothing matched: ↵ keeps it open, so the query can be fixed.
        m.update(Action::PaletteOpen);
        m.update(Action::FieldEdit { edit: Edit::Insert("zzz".into()) });
        assert!(m.update(Action::PaletteChoose { index: None }).is_empty());
        assert!(m.palette.is_some());
        // Over the conversation: the palette takes the keys, and closing it gives them back.
        m.update(Action::ClosePalette);
        m.update(Action::TaskAdd {
            title: "t".into(),
            priority: 3,
            state: State::Running,
            tab: None,
            assign: Assign::default(),
        });
        let id = m.tasks.list[0].id;
        (m.talk, m.agents) =
            (Some(talk::Talk { task: Subject::Task(id), open: true, keys: true }), vec![Subject::Task(id)]);
        assert_eq!(m.mode(), Mode::Talk);
        m.update(Action::PaletteOpen);
        assert_eq!(m.mode(), Mode::Palette);
        m.update(Action::ClosePalette);
        assert_eq!(m.mode(), Mode::Talk);
        // Not over the key reference.
        m.update(Action::ClosePalette);
        m.update(Action::ToggleKeys);
        m.update(Action::PaletteOpen);
        assert!(m.palette.is_none());
    }

    fn action_is_palettes(action: &Action) -> bool {
        matches!(action, Action::PaletteMove { .. } | Action::PaletteChoose { .. } | Action::ClosePalette)
    }

    /// Every action from every origin. The origin decides, not the action: only the user's own
    /// move ends sneak mode, and only the user takes the keyboard from under its labels.
    #[test]
    fn only_the_users_own_move_ends_sneak_whatever_the_action() {
        for action in &samples() {
            for origin in [Origin::User, Origin::Socket, Origin::System] {
                let mut m = Model::default();
                m.update(Action::Split { axis: Right, kind: Kind::Empty });
                sneak(&mut m, 3);
                let refused = match action {
                    _ if origin == Origin::User => None,
                    _ if writes_settings(action) => Some(apps::USERS_ALONE),
                    _ if of_workbooks(action) => Some(workbook::USERS_ALONE),
                    Action::TaskTalk { .. }
                    | Action::SessionOpen { .. }
                    | Action::TalkPick { .. }
                    | Action::TaskStop { .. }
                    | Action::TalkGo => Some(agents::USERS_ALONE),
                    Action::TabPlace { .. } | Action::NewTabOf { task: Some(_) } => Some(TABS_USERS),
                    Action::CalendarJoin { .. } | Action::CalendarDismiss { .. } => Some(calendar::USERS_ALONE),
                    _ if talks(action) => Some(chat::USERS_ALONE),
                    Action::AgentEvents { .. } if origin == Origin::Socket => Some(chat::RUNNERS_ALONE),
                    _ if in_palette(action) && action_is_palettes(action) => Some(commands::USERS_ALONE),
                    _ if in_palette(action) => Some(inspect::USERS_ALONE),
                    Action::InspectorRead { .. } | Action::InspectorFailed { .. } if origin == Origin::Socket => {
                        Some(inspect::USERS_ALONE)
                    }
                    Action::AppWheel { .. } => Some(crate::app_wheel::USERS_ALONE),
                    Action::RestoreLayout { .. } if origin == Origin::Socket => Some(crate::layout_file::USERS_ALONE),
                    Action::StartFresh | Action::UnfilledOpen { .. } => Some(crate::layout_file::USERS_ALONE),
                    _ => action.takes_keyboard().then_some("sneak mode is on"),
                };
                assert_eq!(m.refusal(action, origin), refused, "{action:?} from {origin:?}");
                m.update(action.clone().sent_by(origin));
                let ends = match action {
                    // The mode's own: its key and Esc leave from anywhere, the rest stay in it.
                    Action::Sneak { .. } | Action::LeaveSneak => true,
                    Action::SneakEnter { .. }
                    | Action::SneakDrawn { .. }
                    | Action::SneakType { .. }
                    | Action::SneakBackspace
                    | Action::SneakApp { .. }
                    | Action::AppWheel { .. } => false,
                    _ => origin == Origin::User,
                };
                assert_eq!(m.sneak.is_none(), ends, "{action:?} from {origin:?}");
            }
        }
    }

    /// The tab bar as the shell reports it: each tab and its ×, left to right.
    fn tab_bar(tabs: &[u32]) -> Vec<Target> {
        let at = |(i, id): (usize, &u32)| {
            let x = i as f64 * 100.;
            [control(&format!("tab:{id}"), x), control(&format!("tab-close:{id}"), x + 80.)]
        };
        tabs.iter().enumerate().flat_map(at).collect()
    }

    /// An agent's `close_tab` takes tab 2 off the bar while the labels are up, and tab 3 slides
    /// under the pill that was on tab 2's ×. Those letters must not close anything.
    #[test]
    fn a_label_never_reaches_a_control_that_is_no_longer_drawn_or_the_one_that_took_its_place() {
        let mut m = Model::default();
        m.update(Action::NewTab);
        m.update(Action::NewTab);
        assert_eq!(m.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2, 3]);
        m.update(Action::Sneak { targets: tab_bar(&[1, 2, 3]) });
        let second = label_of(&m, "tab-close:2").expect("a label on tab 2's ×");
        let third = label_of(&m, "tab-close:3").expect("a label on tab 3's ×");
        assert_eq!((second.len(), third.len()), (2, 2), "a × takes two letters");

        // News: the mode stays on, and tab 2 is closed.
        m.update(Action::CloseTab { index: Some(1) }.sent_by(Origin::Socket));
        assert_eq!((m.mode(), m.tabs.len()), (Mode::Sneak, 2));
        // The next frame draws tabs 1 and 3: tab 3 where tab 2 was.
        m.update(Action::SneakDrawn { targets: tab_bar(&[1, 3]) }.sent_by(Origin::System));
        assert_eq!(label_of(&m, "tab-close:2"), None, "not drawn: no label");
        assert_eq!(label_of(&m, "tab-close:3"), Some(third.clone()), "the same letters");
        let moved = m.sneak.as_ref().unwrap().targets.iter().find(|t| t.label == third).unwrap();
        assert_eq!(moved.target.frame.x, 180., "the pill went with its control");

        // The letters that were on tab 2's × name nothing now: no tab closes, hidden or shown.
        assert_eq!(m.update(Action::SneakType { text: second }), []);
        assert_eq!((m.mode(), m.tabs.len()), (Mode::Normal, 2));
        // A label is only ever of a control in the last frame reported.
        m.update(Action::Sneak { targets: tab_bar(&[1, 3]) });
        let third = label_of(&m, "tab-close:3").unwrap();
        assert_eq!(m.update(Action::SneakType { text: third }), [Effect::Activate { id: "tab-close:3".into() }]);
    }

    #[test]
    fn sneak_enter_starts_the_mode_and_changes_nothing_while_it_is_on() {
        let mut m = Model::default();
        assert_eq!((m.update(Action::SneakEnter { targets: controls(30) }), m.mode()), (vec![], Mode::Sneak));
        m.update(Action::SneakType { text: "p".into() });
        // A second caller, or the user's ⌘⌃G just before: still on, the labels and letters as they were.
        let before = m.sneak.clone();
        m.update(Action::SneakEnter { targets: controls(3) });
        assert_eq!((m.mode(), &m.sneak), (Mode::Sneak, &before));
        // It starts only where the key does.
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        assert_eq!((m.update(Action::SneakEnter { targets: controls(3) }), m.mode()), (vec![], Mode::Pick));
    }

    #[test]
    fn starting_sneak_hides_the_accessibility_overlay() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::ToggleAxOverlay);
        assert!(m.ax_overlay);
        sneak(&mut m, 3);
        assert_eq!((m.ax_overlay, m.sneak.as_ref().map(|s| s.hid_overlay)), (false, Some(true)));
        // It stays hidden after the mode; ⌘⌃A shows it again.
        m.update(Action::LeaveSneak);
        assert!(!m.ax_overlay);
        sneak(&mut m, 3);
        assert_eq!(m.sneak.as_ref().map(|s| s.hid_overlay), Some(false), "nothing to hide this time");
    }

    #[test]
    fn sneak_starts_only_where_ottospace_has_the_keyboard_and_its_controls_show() {
        let mut m = Model::default();
        assert_eq!((sneak(&mut m, 0), m.mode()), (vec![], Mode::Normal), "nothing to label");
        // An app has the keyboard: the user returns first.
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        assert_eq!((sneak(&mut m, 3), m.mode(), m.live), (vec![], Mode::App, Some(1)));
        m.update(Action::Leave);
        // The picker, the new-task box and the key reference cover the controls.
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        assert_eq!((sneak(&mut m, 3), m.mode()), (vec![], Mode::Pick));
        m.update(Action::ClosePicker);
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!((sneak(&mut m, 3), m.mode()), (vec![], Mode::Compose));
        m.update(Action::CloseCompose);
        m.update(Action::ToggleKeys);
        assert_eq!((sneak(&mut m, 3), m.sneak.is_none()), (vec![], true));
        m.update(Action::CloseKeys);
        // From list mode it starts, and Esc gives the list its keys back.
        m.update(Action::FocusList);
        assert_eq!((sneak(&mut m, 3), m.mode()), (vec![], Mode::Sneak));
        m.update(Action::LeaveSneak);
        assert_eq!(m.mode(), Mode::List);
    }

    #[test]
    fn split_adds_an_empty_pane_and_focuses_it() {
        let mut m = Model::default();
        assert_eq!(m.update(Action::Split { axis: Right, kind: Kind::Empty }), [Effect::Opened(1)]);
        assert_eq!(m.update(Action::Split { axis: Down, kind: Kind::Empty }), [Effect::Opened(2)]);
        assert_eq!(ids(&m), [1, 2]);
        assert_eq!(m.focused, Some(2));
        assert_eq!(m.pane(2).unwrap().kind, Kind::Empty);
    }

    #[test]
    fn the_home_screen_shows_while_the_tab_on_show_has_no_panes() {
        let mut m = Model::default();
        m.update(Action::TaskAdd {
            title: "ask".into(),
            priority: 1,
            state: State::NeedsYou,
            tab: None,
            assign: Assign::default(),
        });
        let home = m.home(&[]).expect("no panes: the home screen");
        assert_eq!((home.nodes.len(), home.needs_you.len()), (1, 1));
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        assert!(m.home(&[]).is_none(), "a pane takes its place");
        // A new tab has no pane yet: the home screen again.
        m.update(Action::NewTab);
        assert!(m.home(&[]).is_some());
        m.update(Action::SelectTab { index: 0 });
        assert!(m.home(&[]).is_none());
    }

    #[test]
    fn split_never_fills_an_empty_pane() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!(m.panes.iter().map(|p| &p.kind).collect::<Vec<_>>(), [&Kind::Empty, &Kind::Terminal]);
    }

    #[test]
    fn open_fills_an_empty_pane_before_it_splits() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        // Pane 1 is empty: the app takes it.
        assert_eq!(m.update(Action::Open { kind: app(7), axis: Right, beside: None }), [Effect::Opened(1)]);
        assert_eq!(ids(&m), [1]);
        // Pane 1 is now in use: the next app splits it.
        assert_eq!(m.update(Action::Open { kind: app(8), axis: Down, beside: None }), [Effect::Opened(2)]);
        assert_eq!(m.pane(2).unwrap().kind, app(8));
    }

    #[test]
    fn open_beside_splits_that_pane_not_the_focused_one() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        m.update(Action::Open { kind: app(3), axis: Down, beside: Some(1) });
        assert_eq!(m.layout.as_ref().unwrap().neighbor(1, Dir::Down), Some(3));
    }

    #[test]
    fn close_defaults_to_the_focused_pane_and_moves_focus() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        let effects = m.update(Action::Close { pane: None });
        assert_eq!(effects, [Effect::Closed(Pane { id: 2, kind: Kind::Empty, cwd: None, fit: None })]);
        assert_eq!(m.focused, Some(1));
        m.update(Action::Close { pane: None });
        assert_eq!((m.layout.is_none(), m.focused), (true, None));
        assert_eq!(m.update(Action::Close { pane: None }), []);
    }

    #[test]
    fn enter_gives_an_app_pane_the_keyboard() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        assert_eq!(m.update(Action::Enter { pane: 1 }), [Effect::Live(1)]);
        assert_eq!(m.mode(), Mode::App);
        // Entering another app parks the first.
        assert_eq!(m.update(Action::Enter { pane: 2 }), [Effect::Parked(1), Effect::Live(2)]);
        assert_eq!(m.update(Action::Leave), [Effect::Parked(2)]);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn enter_ignores_panes_that_are_not_apps() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        assert_eq!(m.update(Action::Enter { pane: 1 }), []);
        assert_eq!(m.update(Action::Enter { pane: 9 }), []);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn focus_on_another_pane_takes_the_keyboard_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::Focus { pane: 1 }), []); // still live
        assert_eq!(m.update(Action::FocusDir { dir: Dir::Right }), [Effect::Parked(1)]);
        assert_eq!((m.focused, m.live), (Some(2), None));
    }

    #[test]
    fn open_while_an_app_is_live_takes_the_keyboard_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        // e.g. DevTools appears while Chrome is live.
        let effects = m.update(Action::Open { kind: app(2), axis: Right, beside: Some(1) });
        assert_eq!(effects, [Effect::Parked(1), Effect::Opened(2)]);
        assert_eq!((m.focused, m.live, m.mode()), (Some(2), None, Mode::Normal));
    }

    #[test]
    fn rail_buttons_switch_sections_and_the_open_one_closes_the_bar() {
        let mut m = Model::default();
        assert_eq!(m.sidebar, None);
        m.update(Action::ToggleSection { section: Section::Apps });
        assert_eq!(m.sidebar, Some(Section::Apps));
        m.update(Action::ToggleSection { section: Section::Tasks });
        assert_eq!(m.sidebar, Some(Section::Tasks));
        m.update(Action::ToggleSection { section: Section::Tasks });
        assert_eq!(m.sidebar, None);
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::CloseSidebar);
        assert_eq!(m.sidebar, None);
    }

    #[test]
    fn opening_a_section_takes_the_keyboard_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::ToggleSection { section: Section::Apps }), [Effect::Parked(1)]);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn adopt_adds_a_pane_without_taking_focus_or_the_keyboard() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        // DevTools opens while Chrome is live: the user keeps typing in Chrome.
        assert_eq!(m.update(Action::Adopt { kind: app(2), beside: Some(1) }), [Effect::Opened(2)]);
        assert_eq!((m.focused, m.live, m.mode()), (Some(1), Some(1), Mode::App));
        assert_eq!(m.layout.as_ref().unwrap().neighbor(1, Dir::Right), Some(2));
    }

    #[test]
    fn closing_the_bar_takes_the_keyboard_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::CloseSidebar), [Effect::Parked(1)]);
    }

    fn picker(m: &mut Model) {
        let items = vec![
            PickItem::Terminal,
            PickItem::App { name: "Slack".into() },
            PickItem::App { name: "Google Chrome".into() },
            PickItem::App { name: "Google Slides".into() },
        ];
        m.update(Action::OpenPicker { axis: Down, new_tab: false, items });
    }

    fn labels(m: &Model) -> Vec<&str> {
        m.picker.as_ref().unwrap().matches().iter().map(|i| i.label()).collect()
    }

    #[test]
    fn picker_filters_by_what_is_typed_with_prefix_matches_first() {
        let mut m = Model::default();
        picker(&mut m);
        assert_eq!(m.mode(), Mode::Pick);
        assert_eq!(labels(&m), ["Terminal", "Slack", "Google Chrome", "Google Slides"]);
        m.update(Action::PickerQuery { query: "sl".into() });
        assert_eq!(labels(&m), ["Slack", "Google Slides"]);
        m.update(Action::PickerQuery { query: "zzz".into() });
        assert_eq!(labels(&m), [] as [&str; 0]);
    }

    #[test]
    fn picker_selection_wraps_and_resets_when_the_query_changes() {
        let mut m = Model::default();
        picker(&mut m);
        m.update(Action::PickerMove { delta: -1 });
        assert_eq!(m.picker.as_ref().unwrap().selected, 3);
        m.update(Action::PickerMove { delta: 1 });
        assert_eq!(m.picker.as_ref().unwrap().selected, 0);
        m.update(Action::PickerMove { delta: 2 });
        m.update(Action::PickerQuery { query: "g".into() });
        assert_eq!(m.picker.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn choosing_terminal_fills_an_empty_pane_like_an_app_does() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        picker(&mut m);
        assert_eq!(m.update(Action::PickerChoose { index: None }), [Effect::Opened(1)]);
        assert_eq!(m.panes, [Pane { id: 1, kind: Kind::Terminal, cwd: None, fit: None }]);
    }

    #[test]
    fn picker_move_survives_any_delta() {
        let mut m = Model::default();
        picker(&mut m);
        m.update(Action::PickerMove { delta: 1 });
        m.update(Action::PickerMove { delta: i32::MAX });
        m.update(Action::PickerMove { delta: i32::MIN });
        assert!(m.picker.as_ref().unwrap().selected < 4);
    }

    #[test]
    fn pane_actions_wait_while_the_picker_is_open() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        picker(&mut m);
        for action in [
            Action::Close { pane: None },
            Action::Split { axis: Right, kind: Kind::Terminal },
            Action::FocusDir { dir: Dir::Left },
            Action::Enter { pane: 1 },
            Action::ToggleSection { section: Section::Apps },
        ] {
            assert_eq!(m.update(action), []);
        }
        assert_eq!((ids(&m), m.focused, m.live, m.sidebar, m.mode()), (vec![1, 2], Some(2), None, None, Mode::Pick));
    }

    #[test]
    fn typing_edits_the_filter_and_a_click_opens_its_own_row() {
        let mut m = Model::default();
        picker(&mut m);
        m.update(Action::FieldEdit { edit: Edit::Insert("g".into()) });
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
        m.update(Action::FieldEdit { edit: Edit::Delete { to: Motion::Left } });
        assert_eq!(labels(&m), ["Google Chrome", "Google Slides"]);
        // Moving the cursor keeps the selected row; a changed filter starts at the first again.
        m.update(Action::PickerMove { delta: 1 });
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Start, select: false } });
        assert_eq!(m.picker.as_ref().map(|p| p.selected), Some(1));
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
        m.update(Action::FieldEdit { edit: Edit::Undo });
        assert_eq!((m.field().map(|(of, field)| (of, field.text())), labels(&m).len()), (Some(("picker", "g")), 2));
        assert_eq!(m.picker.as_ref().map(|p| p.selected), Some(0));
        // The selection is on row 0; the click is on row 1.
        let effects = m.update(Action::PickerChoose { index: Some(1) });
        assert_eq!(effects, [Effect::HostApp { name: "Google Slides".into(), axis: Down, new_tab: false }]);
    }

    #[test]
    fn choosing_terminal_splits_along_the_pickers_axis() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        picker(&mut m);
        assert_eq!(m.update(Action::PickerChoose { index: None }), [Effect::Opened(2)]);
        assert_eq!(m.pane(2).unwrap().kind, Kind::Terminal);
        assert_eq!(m.layout.as_ref().unwrap().neighbor(1, Dir::Down), Some(2));
        assert_eq!((m.picker.is_none(), m.mode()), (true, Mode::Normal));
    }

    #[test]
    fn choosing_an_app_asks_the_shell_to_host_it() {
        let mut m = Model::default();
        picker(&mut m);
        m.update(Action::PickerQuery { query: "chrome".into() });
        let effects = m.update(Action::PickerChoose { index: None });
        assert_eq!(effects, [Effect::HostApp { name: "Google Chrome".into(), axis: Down, new_tab: false }]);
        assert!(m.picker.is_none());
    }

    #[test]
    fn typing_also_offers_installed_apps_that_are_not_running() {
        let app = |name: &str| Installed { name: name.into(), path: format!("/Applications/{name}.app") };
        let closed = |name: &str| PickItem::Closed { name: name.into(), path: format!("/Applications/{name}.app") };
        let mut m = Model::default();
        // Slack runs. Notes is on the user's own list and does not. Sketch is listed twice.
        let apps = vec![app("Sketch"), app("Slack"), app("Notes"), app("Sketch"), app("Asana"), app("Slides")];
        m.update(Action::SetInstalled { apps: apps.clone(), running: vec!["/Applications/Slack.app".into()] });
        let items = vec![PickItem::Terminal, PickItem::App { name: "Slack".into() }, closed("Notes")];
        m.update(Action::OpenPicker { axis: Down, new_tab: false, items });
        // Nothing typed: what it showed before.
        assert_eq!(labels(&m), ["Terminal", "Slack", "Notes"]);
        // Running first, then the user's own, then the rest by name; a running app is not offered
        // again as one to start, and a bundle is offered once.
        m.update(Action::PickerQuery { query: "s".into() });
        assert_eq!(labels(&m), ["Slack", "Notes", "Sketch", "Slides", "Asana"]);
        let rows = m.picker.as_ref().unwrap().matches();
        assert_eq!(
            (rows[0], rows[1], rows[2]),
            (&PickItem::App { name: "Slack".into() }, &closed("Notes"), &closed("Sketch"))
        );
        // A scan that ends while the picker is open shows at once. The selection stays on its
        // app, wherever that now is: ↵ must not open another.
        m.update(Action::PickerMove { delta: 3 });
        let chosen = |m: &Model| m.picker.as_ref().map(|p| p.matches()[p.selected].label().to_string());
        assert_eq!(chosen(&m).as_deref(), Some("Slides"));
        let more = vec![app("Sketch"), app("Shazam"), app("Slides"), app("Skim")];
        m.update(Action::SetInstalled { apps: more, running: Vec::new() });
        assert_eq!(labels(&m), ["Slack", "Notes", "Shazam", "Sketch", "Skim", "Slides"]);
        assert_eq!(chosen(&m).as_deref(), Some("Slides"));
        // Its app is gone: the selection stays on a row.
        m.update(Action::SetInstalled { apps: vec![app("Sketch")], running: Vec::new() });
        assert_eq!((labels(&m), m.picker.as_ref().map(|p| p.selected)), (vec!["Slack", "Notes", "Sketch"], Some(2)));
        // The filter holds no more than a name may be long.
        m.update(Action::PickerQuery { query: "s".repeat(MAX_FILTER + 50) });
        m.update(Action::FieldEdit { edit: Edit::Paste("s".repeat(5000)) });
        assert_eq!(m.field().map(|(_, field)| field.text().len()), Some(MAX_FILTER));
        m.update(Action::PickerQuery { query: "s".into() });
        m.update(Action::PickerMove { delta: 2 });
        // Choosing one starts it, by where it is; no tab is made before it has a window.
        let effects = m.update(Action::PickerChoose { index: None });
        let path = "/Applications/Sketch.app".to_string();
        assert_eq!(effects, [Effect::LaunchApp { name: "Sketch".into(), path, axis: Down, new_tab: false }]);
        assert_eq!((m.picker.is_none(), m.tabs.len()), (true, 1));
    }

    #[test]
    fn choosing_with_no_match_keeps_the_picker_open() {
        let mut m = Model::default();
        picker(&mut m);
        m.update(Action::PickerQuery { query: "zzz".into() });
        assert_eq!(m.update(Action::PickerChoose { index: None }), []);
        assert!(m.picker.is_some());
        m.update(Action::ClosePicker);
        assert!(m.picker.is_none());
    }

    #[test]
    fn opening_the_picker_takes_the_keyboard_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        let effects = m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        assert_eq!(effects, [Effect::Parked(1)]);
        assert_eq!(m.mode(), Mode::Pick);
    }

    #[test]
    fn focus_cycles_through_panes_in_reading_order_and_wraps() {
        // 1 | 2
        //   | 3
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::Split { axis: Down, kind: Kind::Empty });
        assert_eq!(m.focused, Some(3));
        m.update(Action::FocusCycle { delta: 1 });
        assert_eq!(m.focused, Some(1));
        m.update(Action::FocusCycle { delta: -1 });
        assert_eq!(m.focused, Some(3));
        m.update(Action::FocusCycle { delta: -1 });
        assert_eq!(m.focused, Some(2));
        assert_eq!(Model::default().update(Action::FocusCycle { delta: 1 }), []);
    }

    #[test]
    fn an_empty_tab_has_one_empty_pane_and_opens_no_picker() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let effects = m.update(Action::NewEmptyTab.sent_by(Origin::User));
        assert_eq!((m.tabs.len(), m.active, m.tabs[1].title.as_str()), (2, 1, "Tab 2"));
        assert_eq!((m.layout.as_ref().map(Node::leaves), m.focused), (Some(vec![2]), Some(2)));
        assert_eq!(
            (m.pane(2).map(|p| p.kind.clone()), m.mode(), m.picker.is_none()),
            (Some(Kind::Empty), Mode::Normal, true)
        );
        assert_eq!(effects, [Effect::Opened(2)]);
    }

    #[test]
    fn a_new_tab_takes_the_lowest_free_number() {
        let mut m = Model::default();
        m.update(Action::NewTab);
        m.update(Action::NewTab);
        assert_eq!(tab_titles(&m), ["Tab 1", "Tab 2", "Tab 3"]);
        // Close the middle one: the next tab is "Tab 2" again, not "Tab 4".
        m.update(Action::CloseTab { index: Some(1) });
        m.update(Action::NewTab);
        assert_eq!(tab_titles(&m), ["Tab 1", "Tab 3", "Tab 2"]);
        // Ids stay unique: they are not the number in the title.
        let ids: std::collections::HashSet<u32> = m.tabs.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), m.tabs.len());
    }

    fn tab_titles(m: &Model) -> Vec<&str> {
        m.tabs.iter().map(|tab| tab.title.as_str()).collect()
    }

    fn groups(m: &Model) -> Vec<(Option<TaskId>, Vec<usize>)> {
        m.groups().into_iter().map(|group| (group.task, group.tabs)).collect()
    }

    fn add_task(m: &mut Model, title: &str, state: State) -> TaskId {
        let add = Action::TaskAdd { title: title.into(), priority: 1, state, tab: None, assign: Assign::default() };
        m.update(add.sent_by(Origin::Socket));
        m.tasks.list.last().map(|task| task.id).unwrap_or_default()
    }

    #[test]
    fn a_tasks_tabs_are_a_group_on_the_one_bar() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m); // pane 1, in a tab of no task
        for line in ["fix the zoom flicker on the canvas when zooming #canvas >app-2", "ask Ana about the retry plan"] {
            m.update(Action::TaskFromLine { line: line.into(), known: Default::default(), open: false });
        }
        assert_eq!((m.tab_task(), tab_titles(&m), groups(&m)), (None, vec!["Tab 1"], vec![(None, vec![0])]));
        // A task's new tab: named for its repository, with a terminal there, before the tabs of no task.
        assert_eq!(m.update(Action::NewTabOf { task: Some(1) }), [Effect::Opened(2)]);
        assert_eq!((m.tab_task(), tab_titles(&m)), (Some(1), vec!["app-2", "Tab 1"]));
        assert_eq!((m.focused, m.pane(2).and_then(|p| p.cwd.as_deref())), (Some(2), Some("/code/app-2")));
        // The user's new tab joins the task of the tab on show; "outside the task" joins none.
        m.update(Action::NewTab);
        m.update(Action::NewTabOf { task: None });
        assert_eq!(tab_titles(&m), ["app-2", "Tab 2", "Tab 1", "Tab 3"]);
        assert_eq!(groups(&m), [(Some(1), vec![0, 1]), (None, vec![2, 3])]);
        // ⌘1… and cycling count every tab on the bar.
        m.update(Action::SelectVisibleTab { index: 0 });
        assert_eq!(m.focused, Some(2));
        m.update(Action::CycleTab { delta: 1 });
        m.update(Action::CycleTab { delta: 1 });
        assert_eq!((m.active, m.focused, m.tab_task()), (2, Some(1), None));
        // Going to a task's tabs: the one it showed last. A task with no tab: nothing.
        m.update(Action::TaskGo { id: Some(1) });
        assert_eq!(m.active, 1);
        m.update(Action::TaskGo { id: Some(2) });
        assert_eq!(m.active, 1);
        // A client's new tab is of no task, whatever is on show.
        m.update(Action::NewTab.sent_by(Origin::Socket));
        assert_eq!((m.active, m.tab_task()), (4, None));
    }

    #[test]
    fn a_tab_moves_between_groups_by_the_user_alone() {
        let mut m = Model::default();
        let (a, b) = (add_task(&mut m, "a", State::Running), add_task(&mut m, "b", State::Running));
        m.update(Action::NewTabOf { task: Some(a) }); // tab 2
        m.update(Action::NewTabOf { task: Some(b) }); // tab 3
        m.update(Action::NewTabOf { task: None }); // tab 4
        let ids = |m: &Model| m.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
        assert_eq!(ids(&m), [2, 3, 1, 4]);
        // Onto another group's tab: it takes that task, before it. The tab on show stays.
        m.update(Action::TabPlace { tab: Some(4), at: TabAt::Before(2) });
        assert_eq!((ids(&m), m.tabs[m.active].id), (vec![4, 2, 3, 1], 4));
        assert_eq!(groups(&m), [(Some(a), vec![0, 1]), (Some(b), vec![2]), (None, vec![3])]);
        // Onto a group's badge: the end of that group. Onto no task's: unlinked, at the end.
        m.update(Action::TabPlace { tab: Some(2), at: TabAt::End(Some(b)) });
        assert_eq!(ids(&m), [4, 3, 2, 1]);
        m.update(Action::TabPlace { tab: Some(4), at: TabAt::End(None) });
        assert_eq!((ids(&m), m.tab_task()), (vec![3, 2, 1, 4], None));
        // The last tab of `a` gone, its group is gone: `b` is first.
        assert_eq!(groups(&m), [(Some(b), vec![0, 1]), (None, vec![2, 3])]);
        // A client links nothing, and a dismissed task takes no tab.
        let theirs = Action::TabPlace { tab: Some(1), at: TabAt::End(Some(a)) };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(TABS_USERS));
        m.update(Action::TaskDismiss { id: a }.sent_by(Origin::Socket));
        m.update(theirs.sent_by(Origin::User));
        assert_eq!(m.tabs.iter().find(|tab| tab.id == 1).and_then(|tab| tab.task), None);
    }

    #[test]
    fn move_tab_to_task_lists_the_live_tasks_and_no_task_and_moves_the_tab_on_show() {
        let mut m = Model::default();
        let (a, b, gone) = (
            add_task(&mut m, "fix login", State::Running),
            add_task(&mut m, "docs", State::Running),
            add_task(&mut m, "old", State::Running),
        );
        m.update(Action::TaskDismiss { id: gone });
        m.update(Action::NewTabOf { task: Some(a) }); // tab 2, on show
        let titles = |m: &Model| -> Vec<String> {
            let palette = m.palette.as_ref().map(|p| p.task_matches().into_iter().map(|c| c.title.clone()).collect());
            palette.unwrap_or_default()
        };
        // A client opens nothing; the user's lists the live tasks in order, then no task.
        assert!(m.update(Action::PaletteTasks.sent_by(Origin::Socket)).is_empty());
        assert!(m.palette.is_none());
        m.update(Action::PaletteTasks);
        assert_eq!((m.mode(), titles(&m)), (Mode::Palette, vec!["fix login".into(), "docs".into(), "No task".into()]));
        assert!(m.palette.as_ref().is_some_and(|p| p.matches().is_empty()));
        // Typing narrows it; a client chooses nothing.
        m.update(Action::FieldEdit { edit: Edit::Insert("doc".into()) });
        assert_eq!(titles(&m), ["docs"]);
        assert!(m.update(Action::PaletteChoose { index: None }.sent_by(Origin::Socket)).is_empty());
        assert_eq!(m.tab_task(), Some(a));
        // The user's ↵: the tab on show moves to the end of that task's group, still on show.
        m.update(Action::PaletteChoose { index: None });
        assert_eq!((m.palette.is_none(), m.tabs[m.active].id, m.tab_task()), (true, 2, Some(b)));
        // No task: unlinked, at the end. Nothing matched: it stays open.
        m.update(Action::PaletteTasks);
        m.update(Action::FieldEdit { edit: Edit::Insert("zzz".into()) });
        m.update(Action::PaletteChoose { index: None });
        assert!(m.palette.is_some());
        // A task dismissed while it is listed: choosing it takes its row off, and moves nothing.
        m.update(Action::ClosePalette);
        m.update(Action::PaletteTasks);
        m.update(Action::TaskDismiss { id: a }.sent_by(Origin::Socket));
        m.update(Action::PaletteChoose { index: Some(0) });
        assert_eq!((titles(&m), m.tab_task()), (vec!["docs".into(), "No task".into()], Some(b)));
        m.update(Action::PaletteChoose { index: Some(1) });
        assert_eq!((m.tab_task(), m.active), (None, m.tabs.len() - 1));
    }

    #[test]
    fn pop_next_shows_the_conversation_of_the_task_that_needs_the_user_and_the_tab_stays() {
        let mut m = Model::default();
        term(&mut m);
        add_task(&mut m, "runs", State::Running);
        let waits = add_task(&mut m, "waits", State::NeedsYou);
        // Not the more urgent one that is only running.
        m.update(Action::TaskGo { id: None });
        assert_eq!((m.talk.as_ref().and_then(|t| t.task.task()), m.active, m.tabs.len()), (Some(waits), 0, 1));
        // A client's goes to its tab, when it has one.
        m.update(Action::NewTabOf { task: Some(waits) });
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TaskGo { id: None }.sent_by(Origin::Socket));
        assert_eq!(m.tab_task(), Some(waits));
    }

    #[test]
    fn a_clients_task_takes_no_tab() {
        let mut m = Model::default();
        term(&mut m);
        let first = m.tabs[0].id;
        let add = Action::TaskAdd {
            title: "a".into(),
            priority: 1,
            state: State::Running,
            tab: Some(first),
            assign: Assign::default(),
        };
        m.update(add.sent_by(Origin::Socket));
        assert_eq!((m.tabs[0].task, m.tasks.list.len()), (None, 1));
    }

    #[test]
    fn closing_a_tasks_last_tab_leaves_the_task_with_none() {
        let mut m = Model::default();
        term(&mut m);
        let a = add_task(&mut m, "a", State::Running);
        m.update(Action::NewTabOf { task: Some(a) });
        m.update(Action::NewTab);
        term(&mut m); // pane 2, in the task's second tab
        assert_eq!(groups(&m), [(Some(a), vec![0, 1]), (None, vec![2])]);
        // Its right neighbour takes its place.
        m.update(Action::CloseTab { index: None });
        assert_eq!((m.tab_task(), tab_titles(&m), m.panes.len()), (None, vec!["a", "Tab 1"], 1));
        m.update(Action::CloseTab { index: Some(0) });
        assert_eq!((groups(&m), m.tasks.get(a).is_some()), (vec![(None, vec![0])], true));
    }

    #[test]
    fn starting_a_task_claims_the_tab_on_show_if_it_is_no_tasks_and_nothing_else_moves() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m);
        term(&mut m);
        let view = |m: &Model| (m.active, m.tabs.len(), m.layout.clone(), m.focused, m.panes.clone());
        let before = view(&m);
        m.update(Action::OpenCompose { text: "fix zoom >app".into() });
        let effects = m.update(Action::ComposeEnter);
        assert!(matches!(effects[..], [Effect::StartAgent { .. }]), "{effects:?}");
        assert_eq!((view(&m), m.tab_task(), m.mode()), (before.clone(), Some(1), Mode::Talk));
        // A tab of another task already: the next task starts with no tab.
        m.update(Action::OpenCompose { text: "second".into() });
        m.update(Action::ComposeEnter);
        assert_eq!((view(&m), m.tab_task(), m.groups().len()), (before, Some(1), 1));
        // An empty tab (nothing open): the task takes none, and the home screen stays.
        m.update(Action::NewTabOf { task: None });
        m.update(Action::OpenCompose { text: "third".into() });
        m.update(Action::ComposeEnter);
        assert_eq!((m.tab_task(), m.home(&[]).is_some()), (None, true));
        // Filed for later, or a client's ↵: no tab.
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TabPlace { tab: None, at: TabAt::End(None) });
        m.update(Action::OpenCompose { text: "later".into() });
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!(m.tab_task(), None);
    }

    #[test]
    fn the_drawer_follows_the_tab_on_show_and_never_the_other_way() {
        let mut m = Model::default();
        term(&mut m);
        let (a, b) = (add_task(&mut m, "a", State::Captured), add_task(&mut m, "b", State::Captured));
        m.update(Action::NewTabOf { task: Some(a) });
        m.update(Action::NewTabOf { task: Some(b) });
        // A click on a task: its conversation, the tab stays.
        m.update(Action::TaskTalk { id: a });
        let shown = |m: &Model| m.talk.as_ref().filter(|t| t.open).and_then(|t| t.task.task());
        assert_eq!((shown(&m), m.tab_task()), (Some(a), Some(b)));
        // The header offers its tabs; ⌘↵ goes there.
        assert_eq!(m.talk_go(), Some(talk::Go::Tabs));
        m.update(Action::TalkGo);
        assert_eq!((m.tab_task(), m.talk_go()), (Some(a), None));
        // A tab of another task, the drawer open: it shows that task's, starting nothing.
        assert_eq!(m.update(Action::SelectTab { index: 1 }), []);
        assert_eq!((shown(&m), m.agent_runs(&Subject::Task(b))), (Some(b), false));
        // A tab of no task: the drawer stays.
        m.update(Action::SelectTab { index: 2 });
        assert_eq!(shown(&m), Some(b));
        // Closed: it stays closed.
        m.update(Action::TalkToggle);
        m.update(Action::SelectTab { index: 0 });
        assert_eq!((shown(&m), m.talk.as_ref().map(|t| t.task.clone())), (None, Some(Subject::Task(b))));
        // The keyboard in the drawer: a client's change of tab does not change the conversation.
        m.update(Action::TalkToggle);
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TaskTalk { id: a }); // starts it: the keyboard goes in
        assert_eq!(m.mode(), Mode::Talk);
        m.update(Action::SelectTab { index: 0 }.sent_by(Origin::Socket));
        assert_eq!((shown(&m), m.tab_task()), (Some(a), Some(a)));
        m.update(Action::TaskTalk { id: a });
        assert_eq!(m.mode(), Mode::Talk);
        m.update(Action::SelectTab { index: 1 }.sent_by(Origin::Socket));
        assert_eq!((shown(&m), m.tab_task()), (Some(a), Some(b)), "a client's tab change, the keyboard in the drawer");
        // A client cannot answer "Stop its agent and dismiss?", yes or no.
        m.update(Action::RowDismiss { id: a });
        assert_eq!(m.refusal(&Action::QuitCancel, Origin::Socket), Some(agents::USERS_ALONE));
        m.update(Action::QuitCancel);
        // A task with no tab: claim the tab on show if it is no task's, else a new tab.
        let c = add_task(&mut m, "c", State::Captured);
        m.update(Action::TaskTalk { id: c });
        assert_eq!(m.talk_go(), Some(talk::Go::New));
        m.update(Action::SelectTab { index: 2 });
        assert_eq!(m.talk_go(), Some(talk::Go::Claim));
        m.update(Action::TalkGo);
        assert_eq!((m.tab_task(), m.talk_go()), (Some(c), None));
        assert_eq!(m.refusal(&Action::TalkGo, Origin::Socket), Some(agents::USERS_ALONE));
    }

    #[test]
    fn a_dismissed_task_leaves_the_rail_the_drawer_and_its_tabs_and_undo_brings_them_back() {
        let mut m = Model::default();
        term(&mut m);
        let (a, b) = (add_task(&mut m, "a", State::Captured), add_task(&mut m, "b", State::Captured));
        m.update(Action::TabPlace { tab: None, at: TabAt::End(Some(a)) });
        m.update(Action::TaskTalk { id: b });
        m.update(Action::AgentExited { task: Subject::Task(b) }.sent_by(Origin::System));
        m.update(Action::TaskTalk { id: a });
        m.update(Action::AgentExited { task: Subject::Task(a) }.sent_by(Origin::System));
        let rail = |m: &Model| m.rail().iter().map(|cell| cell.task.clone()).collect::<Vec<_>>();
        assert_eq!(rail(&m), [Subject::Task(a), Subject::Task(b)]);
        // Dismissed: off the rail, the drawer shows nothing (not `b`), its tab is no task's.
        m.update(Action::RowDismiss { id: a });
        assert_eq!((rail(&m), m.talk.is_none(), m.tab_task()), (vec![Subject::Task(b)], true, None));
        // Taken back: the task and its tab link; its conversation waits to be opened again.
        m.update(Action::TaskUndo);
        assert_eq!((rail(&m), m.tab_task()), (vec![Subject::Task(b)], Some(a)));
        // Done: off the rail unless shown or its agent runs.
        m.update(Action::TaskTalk { id: b });
        m.update(Action::AgentExited { task: Subject::Task(b) }.sent_by(Origin::System));
        m.update(Action::TaskSetState { id: b, state: State::Done });
        assert_eq!(rail(&m), [Subject::Task(b)], "shown");
        m.update(Action::TalkToggle);
        assert_eq!(rail(&m), Vec::<Subject>::new());
    }

    #[test]
    fn only_the_users_undo_links_a_dismissed_tasks_tabs_and_never_a_tab_made_since() {
        let mut m = Model::default();
        term(&mut m);
        let a = add_task(&mut m, "a", State::Captured);
        m.update(Action::TabPlace { tab: None, at: TabAt::End(Some(a)) });
        m.update(Action::TaskDismiss { id: a });
        // Back by a state report: its tab stays the user's, of no task.
        m.update(Action::TaskSetState { id: a, state: State::NeedsYou }.sent_by(Origin::Socket));
        assert_eq!((m.tasks.live(a).is_some(), m.tab_task()), (true, None));
        // Dismissed, its only tab closed, a new tab with the same id: undo links nothing.
        m.update(Action::TabPlace { tab: None, at: TabAt::End(Some(a)) });
        m.update(Action::TaskDismiss { id: a });
        m.update(Action::CloseTab { index: None });
        assert_eq!(m.tabs[0].id, 1, "the id is used again");
        m.update(Action::TaskUndo);
        assert_eq!((m.tasks.live(a).is_some(), m.tab_task()), (true, None));
    }

    #[test]
    fn starting_a_task_that_has_a_tab_takes_no_other() {
        let mut m = Model::default();
        term(&mut m);
        let a = add_task(&mut m, "a", State::Captured);
        m.update(Action::NewTabOf { task: Some(a) });
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TaskTalk { id: a });
        assert_eq!((m.active, m.tab_task(), groups(&m).len()), (1, None, 2));
    }

    #[test]
    fn dismissing_a_task_whose_agent_runs_asks_the_user_and_refuses_a_client() {
        let mut m = Model::default();
        term(&mut m);
        let a = add_task(&mut m, "a", State::Captured);
        m.update(Action::TaskTalk { id: a });
        assert!(m.agent_runs(&Subject::Task(a)));
        let theirs = Action::TaskDismiss { id: a };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(agents::USERS_ALONE));
        // The user's: "Stop its agent and dismiss?". No: nothing changes.
        m.update(Action::RowDismiss { id: a });
        assert_eq!(
            m.confirm.as_ref().map(|c| (c.dismiss, c.title())),
            Some((Some(a), "Stop its agent and dismiss?".into()))
        );
        m.update(Action::QuitCancel);
        assert_eq!((m.confirm.is_none(), m.tasks.get(a).map(|t| t.dismissed)), (true, Some(false)));
        // Yes: the agent is stopped as Stop does, and the task goes; its end does not bring it back.
        m.update(Action::RowDismiss { id: a });
        assert_eq!(m.update(Action::QuitConfirm), [Effect::StopAgent { task: Subject::Task(a) }]);
        m.update(Action::AgentExited { task: Subject::Task(a) }.sent_by(Origin::System));
        assert_eq!((m.tasks.get(a).map(|t| t.dismissed), m.rail().len(), m.confirm.is_none()), (Some(true), 0, true));
    }

    #[test]
    fn list_mode_moves_over_the_rows_and_opens_one() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        // With no section open the bar opens on Apps; on an open one it stays there.
        m.update(Action::FocusList);
        assert_eq!((m.mode(), m.sidebar), (Mode::List, Some(Section::Apps)));
        m.update(Action::FocusList);
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::FocusList);
        assert_eq!((m.mode(), m.sidebar, m.list), (Mode::List, Some(Section::Repos), Some(0)));
        assert_eq!(m.list_rows(), [ListRow::Repo("/code/app".into()), ListRow::Repo("/code/app-2".into())]);
        m.update(Action::ListMove { delta: -1 }); // wraps to the last row
        assert_eq!(m.list, Some(1));
        // Opening the row: a terminal in the worktree, and the keyboard goes to it.
        assert_eq!(m.update(Action::ListChoose { axis: Down }), [Effect::Opened(1)]);
        assert_eq!((m.pane(1).unwrap().cwd.as_deref(), m.mode()), (Some("/code/app-2"), Mode::Normal));
    }

    #[test]
    fn list_mode_acts_on_the_selected_task() {
        let mut m = Model::default();
        m.update(Action::NewTab);
        let second = m.tabs[1].id;
        m.update(Action::SelectTab { index: 0 });
        for (title, tab) in [("a", Some(second)), ("b", None), ("c", None)] {
            m.update(Action::TaskAdd {
                title: title.into(),
                priority: 1,
                state: State::NeedsYou,
                tab,
                assign: Assign::default(),
            });
        }
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        let (heading, task) = (ListRow::Heading, ListRow::Task);
        // The headings are rows too; the selection starts on the first task. "a" has a tab, and
        // stays under Needs you with the others.
        assert_eq!((m.list_rows(), m.list), (vec![heading(Heading::NeedsYou), task(1), task(2), task(3)], Some(1)));
        // Dismiss the second task: the selection stays at its place, now on the next task.
        m.update(Action::ListMove { delta: 1 });
        m.update(Action::ListDismiss);
        assert_eq!((m.list_rows(), m.list), (vec![heading(Heading::NeedsYou), task(1), task(3)], Some(2)));
        // Hold it: it goes under Later, and the heading that came is under the selection. The
        // task keys do nothing on a heading.
        m.update(Action::ListHold);
        let held = vec![heading(Heading::NeedsYou), task(1), heading(Heading::Later), task(3)];
        assert_eq!((m.list_rows(), m.list), (held.clone(), Some(2)));
        m.update(Action::ListSnooze { until: 9 });
        assert_eq!(m.list_rows(), held);
        // Snooze it: gone with its heading, and the selection moves up to a row that is there.
        m.update(Action::ListMove { delta: 1 });
        m.update(Action::ListSnooze { until: 9 });
        assert_eq!((m.list_rows(), m.list), (vec![heading(Heading::NeedsYou), task(1)], Some(1)));
        m.update(Action::TaskUndo);
        assert_eq!(m.tasks.get(3).map(|t| (t.state, t.snoozed_until)), Some((State::Parked, None)));
        // ↵ on a task opens its conversation, with the keyboard; the workspace stays.
        m.update(Action::ListChoose { axis: Right });
        assert_eq!((m.active, m.mode(), m.talk.as_ref().and_then(|talk| talk.task.task())), (0, Mode::Talk, Some(1)));
    }

    #[test]
    fn the_selection_follows_its_task_when_others_change_the_inbox() {
        let mut m = Model::default();
        for title in ["a", "b", "c"] {
            m.update(Action::TaskAdd {
                title: title.into(),
                priority: 2,
                state: State::NeedsYou,
                tab: None,
                assign: Assign::default(),
            });
        }
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        m.update(Action::ListMove { delta: 1 }); // on b
        // An agent files a more urgent task above, and another task leaves: still on b.
        m.update(Action::TaskAdd {
            title: "urgent".into(),
            priority: 1,
            state: State::NeedsYou,
            tab: None,
            assign: Assign::default(),
        });
        assert_eq!((m.list, m.list_rows()[3].clone()), (Some(3), ListRow::Task(2)));
        m.update(Action::TaskSetState { id: 1, state: State::Cleaned });
        assert_eq!(m.list, Some(2));
        // The user's own hold keeps the place: the next task (c) comes under the selection.
        m.update(Action::ListHold);
        assert_eq!((m.list, m.list_rows()[2].clone()), (Some(2), ListRow::Task(3)));
        // The selected task is taken away by someone else: the selection stays on a row.
        m.update(Action::TaskSetState { id: 3, state: State::Cleaned });
        assert_eq!((m.list, m.list_rows()[2].clone()), (Some(2), ListRow::Heading(Heading::Later)));
        // ↵ opens the task's conversation: the keyboard goes to it, and the list gives it up.
        m.update(Action::ListMove { delta: 1 });
        m.update(Action::ListChoose { axis: Right });
        assert_eq!(
            (m.mode(), m.tab_task(), m.talk.as_ref().and_then(|talk| talk.task.task())),
            (Mode::Talk, None, Some(2))
        );
    }

    #[test]
    fn list_mode_changes_section_hosts_apps_and_ends_with_the_bar() {
        let mut m = Model::default();
        m.update(Action::SetApps { names: vec!["Notes".into(), "Safari".into()] });
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::FocusList);
        m.update(Action::ListSection { delta: 1 }); // after Repos comes Marks, with no rows yet
        assert_eq!((m.sidebar, m.list_rows()), (Some(Section::Marks), vec![]));
        m.update(Action::ListSection { delta: 2 }); // then Tasks again, and Apps
        m.update(Action::ListMove { delta: 1 });
        assert_eq!((m.sidebar, m.list), (Some(Section::Apps), Some(1)));
        // The shell hosts the app; until it opens a pane, the list keeps the keyboard.
        let host = Effect::HostApp { name: "Safari".into(), axis: Right, new_tab: false };
        assert_eq!((m.update(Action::ListChoose { axis: Right }), m.mode()), (vec![host], Mode::List));
        // ⇧Tab: Apps, then Tasks. Nothing to move to or open there.
        m.update(Action::ListSection { delta: -1 });
        m.update(Action::ListMove { delta: 1 });
        assert_eq!(
            (m.sidebar, m.list, m.update(Action::ListChoose { axis: Right })),
            (Some(Section::Tasks), Some(0), vec![])
        );
        // Closing the bar, or a click on a pane, ends list mode. A second FocusList leaves too.
        m.update(Action::CloseSidebar);
        assert_eq!((m.mode(), m.list), (Mode::Normal, None));
        m.update(Action::FocusList);
        m.update(Action::FocusList);
        assert_eq!((m.mode(), m.sidebar), (Mode::Normal, Some(Section::Apps)));
        // Not behind the picker.
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        m.update(Action::FocusList);
        assert_eq!(m.mode(), Mode::Pick);
        m.update(Action::ClosePicker);
        assert_eq!(m.list, None);
    }

    #[test]
    fn the_new_task_box_files_a_line_as_a_task() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m);
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!(m.mode(), Mode::Compose);
        // It has the keyboard: a split waits, and the picker does not open over it.
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        assert_eq!((m.panes.len(), m.mode()), (1, Mode::Compose));
        // ↵ on an empty line files nothing and keeps the box.
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!((m.tasks.list.len(), m.mode()), (0, Mode::Compose));
        // Tab finishes the repository being typed; the name need not match in case.
        m.update(Action::FieldEdit { edit: Edit::Insert("fix zoom #Canvas >AP".into()) });
        assert_eq!(offered(&m), ["app", "app-2"], "worktrees too");
        m.update(Action::ComposeComplete);
        m.update(Action::FieldEdit { edit: Edit::Insert("p1x".into()) });
        m.update(Action::FieldEdit { edit: Edit::Delete { to: Motion::Left } });
        assert_eq!(m.compose.as_ref().map(Field::text), Some("fix zoom #Canvas >app p1"));
        // Tab finishes the word at the cursor, not the end of the line; ⌘Z takes both steps back.
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Start, select: false } });
        m.update(Action::FieldEdit { edit: Edit::Insert(">a ".into()) });
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Left, select: false } });
        m.update(Action::ComposeComplete);
        assert_eq!(m.field().map(|(of, field)| (of, field.text())), Some(("compose", ">app fix zoom #Canvas >app p1")));
        m.update(Action::FieldEdit { edit: Edit::Undo });
        m.update(Action::FieldEdit { edit: Edit::Undo });
        assert_eq!(m.compose_line().map(String::as_str), Some("fix zoom #Canvas >app p1"));
        // Inside a word, Tab replaces the whole word, not only what is before the cursor.
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::End, select: false } });
        for _ in 0..5 {
            m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Left, select: false } });
        }
        m.update(Action::ComposeComplete);
        assert_eq!(m.field().map(|(_, field)| (field.text(), field.cursor())), Some(("fix zoom #Canvas >app p1", 22)));
        // Under the key reference the line is hidden: it has no keyboard, and nothing edits it.
        m.keys = true;
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
        assert_eq!((m.field(), m.open_field().map(|(of, _)| of)), (None, Some("compose")));
        m.keys = false;
        assert_eq!(m.compose_line().map(String::as_str), Some("fix zoom #Canvas >app p1"));
        m.update(Action::ComposeSubmit { open: false });
        let task = &m.tasks.list[0];
        assert_eq!((task.title.as_str(), task.state, task.priority), ("fix zoom", State::Captured, 1));
        assert_eq!(
            (task.project.as_deref(), task.repo.as_deref(), m.mode()),
            (Some("Canvas"), Some("/code/app"), Mode::Normal)
        );
        // A project already used is suggested; an unknown repository stays in the title.
        m.update(Action::OpenCompose { text: "look #c".into() });
        assert_eq!(offered(&m), ["Canvas"]);
        m.update(Action::CloseCompose);
        m.update(Action::TaskFromLine { line: "ask @ana about >app-2".into(), known: Default::default(), open: false });
        assert_eq!(
            (m.tasks.list[1].title.as_str(), m.tasks.list[1].repo.as_deref()),
            ("ask @ana about", Some("/code/app-2"))
        );
        // The box holds no more than a title may: a longer task could not be read back.
        m.update(Action::OpenCompose { text: "x".repeat(MAX_TITLE + 50) });
        m.update(Action::FieldEdit { edit: Edit::Insert("yz".into()) });
        assert_eq!(m.compose.as_ref().map(|line| line.text().chars().count()), Some(MAX_TITLE));
        m.update(Action::CloseCompose);
        m.update(Action::TaskFromLine { line: "x".repeat(MAX_TITLE + 1), known: Default::default(), open: false });
        assert_eq!(m.tasks.list.len(), 2);
        // The picker may open again over itself, but not over the box; a dead pane closes behind either.
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        m.update(Action::OpenPicker { axis: Right, new_tab: true, items: vec![PickItem::Terminal] });
        assert_eq!(m.picker.as_ref().map(|p| p.new_tab), Some(true));
        m.update(Action::ClosePicker);
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!(m.update(Action::Close { pane: Some(1) }).len(), 1);
        m.update(Action::CloseCompose);
        // Not under the key reference.
        m.update(Action::ToggleKeys);
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!(m.compose, None);
    }

    fn keys(m: &mut Model, text: &str) {
        for c in text.chars() {
            m.update(Action::FieldEdit { edit: Edit::Insert(c.to_string()) });
        }
    }

    fn line(m: &Model) -> &str {
        m.compose_line().map_or("<closed>", String::as_str)
    }

    /// A box over one terminal, with two repositories, two agents, a skill and three projects.
    fn composing() -> Model {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m);
        for line in ["a #canvas", "b #canvas", "c #Cables", "d #flow-canvas"] {
            m.update(Action::TaskFromLine { line: line.into(), known: Default::default(), open: false });
        }
        m.update(Action::OpenCompose { text: String::new() });
        let names = compose::Names { agents: vec!["claude".into(), "codex".into()], skills: vec!["deploy".into()] };
        let known =
            Action::SetKnown { line: String::new(), typing: None, names: Some(names), paths: vec![], entries: vec![] };
        m.update(known);
        m
    }

    #[test]
    fn the_dropdown_opens_in_a_tag_and_its_keys_take_or_leave_a_row() {
        let mut m = composing();
        let shown = |m: &Model| m.dropdown().map(|menu| (menu.mark, offered(m), menu.selected, menu.note));
        // No tag at the cursor: no dropdown, and ↑ ↓ are the line's ends.
        keys(&mut m, "fix zoom");
        m.update(Action::ComposeMove { delta: -1 });
        assert_eq!((shown(&m), m.field().map(|(_, f)| f.cursor())), (None, Some(0)));
        m.update(Action::ComposeMove { delta: 1 });
        // A bare mark lists what there is, the project with the most tasks first, and
        // highlights nothing: ↵ would still add the task.
        keys(&mut m, " #");
        let all = vec!["canvas".to_string(), "Cables".into(), "flow-canvas".into()];
        assert_eq!(shown(&m), Some(('#', all, None, None)));
        // A letter narrows and highlights the first row. Matches at the start come first.
        keys(&mut m, "ca");
        let rows = m.dropdown().unwrap().rows;
        assert_eq!(shown(&m), Some(('#', vec!["canvas".into(), "Cables".into(), "flow-canvas".into()], Some(0), None)));
        assert_eq!((rows[0].kind, rows[0].detail.as_str(), &rows[0].matched), ("project", "2 tasks", &vec![0, 1]));
        assert_eq!(rows[2].matched, [5, 6], "the letters matched are told, for the shell to draw them stronger");
        // ↓ and ↑ move and wrap; typing more keeps to the rows that are left.
        m.update(Action::ComposeMove { delta: 1 });
        m.update(Action::ComposeMove { delta: 1 });
        m.update(Action::ComposeMove { delta: 1 });
        m.update(Action::ComposeMove { delta: -1 });
        assert_eq!(m.dropdown().unwrap().selected, Some(2));
        keys(&mut m, "b");
        assert_eq!(shown(&m), Some(('#', vec!["Cables".into()], Some(0), None)));
        // ↵ on a highlighted row takes it and adds no task. The box stays; the dropdown is gone.
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!((line(&m), shown(&m), m.tasks.list.len()), ("fix zoom #Cables ", None, 4));
        // Tab takes the highlighted row, or the first when none is.
        keys(&mut m, ">");
        m.update(Action::ComposeComplete);
        assert_eq!(line(&m), "fix zoom #Cables >app ");
        // Esc closes the dropdown and keeps the box; ↵ then adds, with the word as typed.
        keys(&mut m, "@co");
        m.update(Action::ComposeEscape);
        assert_eq!((shown(&m), m.mode()), (None, Mode::Compose));
        m.update(Action::ComposeMove { delta: 1 });
        assert_eq!(shown(&m), None, "closed, it does not take ↓");
        // Typing on opens it again; a second Esc, with it closed, closes the box.
        keys(&mut m, "d");
        assert_eq!(offered(&m), ["codex"]);
        m.update(Action::ComposeEscape);
        m.update(Action::ComposeEscape);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn the_agent_dropdown_offers_each_runner_and_its_models() {
        let mut m = composing();
        let agents = crate::agents::names(&Default::default(), vec!["ana".into()]);
        let names = compose::Names { agents, skills: Vec::new() };
        m.update(Action::SetKnown {
            line: String::new(),
            typing: None,
            names: Some(names),
            paths: vec![],
            entries: vec![],
        });
        keys(&mut m, "fix @");
        let rows = m.dropdown().unwrap().rows;
        let details: Vec<&str> = rows.iter().map(|row| row.detail.as_str()).collect();
        assert_eq!(offered(&m)[..3], ["claude", "claude-fable-5-2", "claude-opus-5-5"]);
        assert_eq!(
            (details[0], details[1], details[6], details[7]),
            ("runner", "claude model", "codex model", "saved agent")
        );
        // A word of the name finds it: `opus`, `gpt`.
        keys(&mut m, "opus");
        assert_eq!(offered(&m), ["claude-opus-5-5"]);
        m.update(Action::ComposeComplete);
        // A model that is not listed is a tag as typed.
        keys(&mut m, "@codex-gpt-7");
        assert_eq!(offered(&m), Vec::<String>::new());
        m.update(Action::ComposeSubmit { open: false });
        let task = m.tasks.list.last().unwrap();
        assert_eq!((task.title.as_str(), task.agent.as_deref()), ("fix", Some("codex-gpt-7")));
        assert_eq!(m.task_tags(task), "@codex-gpt-7");
    }

    #[test]
    fn enter_adds_the_task_unless_a_row_would_change_the_line() {
        // A name typed in full: its row is highlighted, but there is nothing to take. ↵ adds.
        let mut m = composing();
        keys(&mut m, "fix #canvas");
        assert_eq!(m.dropdown().and_then(|menu| menu.selected), Some(0));
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!((m.mode(), m.tasks.list[4].project.as_deref()), (Mode::Normal, Some("canvas")));
        // A bare mark, or a name nobody has: nothing is highlighted. ↵ adds.
        for text in ["fix #", "fix #brand-new", "fix @nobody"] {
            m.update(Action::OpenCompose { text: text.into() });
            let note = m.dropdown().and_then(|menu| menu.note);
            assert_eq!(
                note,
                Some(if text.ends_with('w') { "new project" } else { "no match" }).filter(|_| text != "fix #")
            );
            m.update(Action::ComposeSubmit { open: false });
            assert_eq!(m.mode(), Mode::Normal, "{text}");
        }
        // ⌘↵ adds whatever is highlighted.
        m.update(Action::OpenCompose { text: "fix #ca".into() });
        m.update(Action::ComposeSubmit { open: true });
        assert_eq!(
            (
                m.compose_line(),
                m.tasks.list.last().filter(|t| t.project.as_deref() == Some("ca")).map(|t| t.title.as_str())
            ),
            (None, Some("fix"))
        );
        // A space after a name that is a row's takes the row, spelled as it is kept.
        m.update(Action::OpenCompose { text: "fix #CABLES".into() });
        keys(&mut m, " ");
        assert_eq!(line(&m), "fix #Cables ");
        keys(&mut m, "#cab ");
        assert_eq!(line(&m), "fix #Cables #cab ", "part of a name is not the name");
        // A click takes its row; a row that is not there takes nothing.
        keys(&mut m, "@c");
        m.update(Action::ComposeAccept { index: 9 });
        m.update(Action::ComposeAccept { index: 1 });
        assert_eq!(line(&m), "fix #Cables #cab @codex ");
        // Inside a line the whole word goes, and the cursor leaves the tag past the space there.
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Start, select: false } });
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::WordRight, select: false } });
        keys(&mut m, " >a");
        m.update(Action::ComposeComplete);
        assert_eq!(
            (line(&m), m.field().map(|(_, f)| f.cursor()), m.dropdown()),
            ("fix >app #Cables #cab @codex ", Some(9), None)
        );
        // A selection, or the key reference over the box: no dropdown.
        keys(&mut m, "#c");
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Left, select: true } });
        assert_eq!(m.dropdown(), None);
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Right, select: false } });
        m.keys = true;
        assert_eq!(m.dropdown(), None);
    }

    #[test]
    fn a_folder_taken_from_the_dropdown_keeps_it_open_on_what_is_inside() {
        let mut m = composing();
        let read = |m: &mut Model, typing: &str, entries: &[&str]| {
            let (line, entries) = (m.compose_line().cloned().unwrap(), entries.iter().map(|e| e.to_string()).collect());
            m.update(Action::SetKnown { line, typing: Some(typing.into()), names: None, paths: vec![], entries });
        };
        keys(&mut m, "see /src/ex");
        // Not read yet: no dropdown, not "no match".
        assert_eq!(m.dropdown(), None);
        read(&mut m, "/src/ex", &["/src/export/", "/src/extra.rs"]);
        let menu = m.dropdown().unwrap();
        assert_eq!((menu.rows[0].kind, menu.rows[1].kind, &menu.rows[0].matched), ("folder", "file", &vec![5, 6]));
        // What was read for another word typed since is not shown.
        keys(&mut m, "p");
        assert_eq!(m.dropdown(), None);
        read(&mut m, "/src/exp", &["/src/export/"]);
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!((line(&m), m.dropdown()), ("see /src/export/", None));
        // Its entries arrive; nothing of a name is typed, so none is highlighted.
        read(&mut m, "/src/export/", &["/src/export/My Notes/", "/src/export/a.rs"]);
        assert_eq!((offered(&m).len(), m.dropdown().unwrap().selected), (2, None));
        m.update(Action::ComposeMove { delta: 1 });
        m.update(Action::ComposeComplete);
        assert_eq!(line(&m), r"see /src/export/My\ Notes/");
        read(&mut m, "/src/export/My Notes/", &[]);
        assert_eq!(m.dropdown().and_then(|menu| menu.note), Some("no match"));
        // A folder typed in full, without its last `/`: its row is highlighted, but ↵ adds.
        m.update(Action::FieldEdit { edit: Edit::Delete { to: Motion::Left } });
        read(&mut m, "/src/export/My Notes", &["/src/export/My Notes/"]);
        assert_eq!(m.dropdown().and_then(|menu| menu.selected), Some(0));
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!(
            (m.mode(), m.tasks.list.last().map(|task| task.title.as_str())),
            (Mode::Normal, Some(r"see /src/export/My\ Notes"))
        );
    }

    #[test]
    fn the_tags_understood_are_told_with_where_they_are() {
        let mut m = composing();
        keys(&mut m, "fix #canvas ");
        keys(&mut m, ">app ");
        keys(&mut m, ">web @claude p2, $nope");
        let marks: Vec<(char, &str)> =
            m.compose_spans().iter().map(|span| (span.mark, &line(&m)[span.range.clone()])).collect();
        assert_eq!(marks, [('#', "#canvas"), ('>', ">app"), ('@', "@claude"), ('p', "p2,")]);
    }

    #[test]
    fn files_go_in_as_path_words_at_the_cursor() {
        let mut m = composing();
        keys(&mut m, "read now");
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::WordLeft, select: false } });
        let paths = vec!["/Users/me/My Notes/a b.md".to_string(), "/etc/hosts".into(), "relative".into()];
        m.update(Action::ComposeAttach { paths: paths.clone(), home: Some("/Users/me".into()), unreadable: 0 });
        // Inside the home folder: `~/`. A space gets a backslash. Not a path: left out.
        assert_eq!(line(&m), r"read ~/My\ Notes/a\ b.md /etc/hosts now");
        assert_eq!(compose::path_words(line(&m)), [vec!["~/My Notes/a b.md"], vec!["/etc/hosts"]]);
        // One step to undo; a word is added after a word.
        m.update(Action::FieldEdit { edit: Edit::Undo });
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::End, select: false } });
        m.update(Action::ComposeAttach {
            paths: paths[1..2].to_vec(),
            home: Some("/Users/meadow".into()),
            unreadable: 0,
        });
        assert_eq!(line(&m), "read now /etc/hosts ");
        // No room for all of it: none goes in. No box: nothing happens.
        let long = vec![format!("/{}", "x".repeat(MAX_TITLE))];
        m.update(Action::ComposeAttach { paths: long.clone(), home: None, unreadable: 0 });
        assert_eq!(line(&m), "read now /etc/hosts ");
        m.update(Action::CloseCompose);
        m.update(Action::ComposeAttach { paths: long, home: None, unreadable: 0 });
        assert_eq!(m.compose, None);
    }

    #[test]
    fn a_reference_names_what_is_open_and_is_kept_as_it_was() {
        let mut m = Model::default();
        // Two tabs of one title; a terminal in a repository running vim; an app pane.
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Open { kind: app(7), axis: Right, beside: None });
        m.update(Action::NewTab);
        m.tabs[1].title = m.tabs[0].title.clone();
        m.update(Action::SelectTab { index: 0 });
        let info = TerminalInfo { pane: 1, cwd: Some("/code/flow app".into()), program: Some("vim".into()) };
        m.update(Action::SetTerminals { terminals: vec![info], home: Some("/code".into()) });
        let sketch = Installed { name: "Sketchpad".into(), path: "/Applications/Sketchpad.app".into() };
        m.update(Action::SetInstalled { apps: vec![sketch], running: Vec::new() });
        m.update(Action::OpenCompose { text: "compare ^".into() });
        let rows = m.dropdown().unwrap().rows;
        let shown: Vec<(&str, &str, &str)> =
            rows.iter().map(|r| (r.kind, r.name.as_str(), r.detail.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("tab", "Tab 1", "tab · 2 panes"),
                ("tab", "Tab 1-2", "tab · 0 panes"),
                ("pane", "flow app", "terminal · vim · ~/flow app"),
                ("pane", "app", "app pane · w7"),
                ("app", "Sketchpad", "app · not running"),
            ]
        );
        // Taken from the dropdown, a space in the name gets a backslash.
        keys(&mut m, "fl");
        m.update(Action::ComposeComplete);
        assert_eq!(line(&m), r"compare ^flow\ app ");
        // ⌥-click on a tab and on a pane: each goes in at the cursor; one that is gone does not.
        m.update(Action::ComposeRef { target: Pointed::Tab(m.tabs[1].id) });
        m.update(Action::ComposeRef { target: Pointed::Pane(2) });
        m.update(Action::ComposeRef { target: Pointed::Pane(99) });
        keys(&mut m, "^Sketchpad ^nothing");
        assert_eq!(line(&m), r"compare ^flow\ app ^Tab\ 1-2 ^app ^Sketchpad ^nothing");
        let marks: Vec<char> = m.compose_spans().iter().map(|span| span.mark).collect();
        assert_eq!(marks, ['^', '^', '^', '^']);
        // The terminal closes before the task is added: its reference is kept as it was.
        m.update(Action::Close { pane: Some(1) });
        m.update(Action::ComposeEscape);
        m.update(Action::ComposeSubmit { open: false });
        let task = m.tasks.list.last().unwrap();
        assert_eq!(task.title, "compare ^nothing");
        let kept: Vec<(RefKind, &str, Option<u32>)> =
            task.refs.iter().map(|r| (r.kind, r.name.as_str(), r.id)).collect();
        assert_eq!(
            kept,
            [
                (RefKind::Pane, "flow app", Some(1)),
                (RefKind::Tab, "Tab 1-2", Some(m.tabs[1].id)),
                (RefKind::Pane, "app", Some(2)),
                (RefKind::App, "Sketchpad", None)
            ]
        );
        assert_eq!(
            (task.refs[0].cwd.as_deref(), task.refs[0].program.as_deref()),
            (Some("/code/flow app"), Some("vim"))
        );
        assert_eq!(task.refs[3].bundle.as_deref(), Some("/Applications/Sketchpad.app"));
        assert_eq!(m.task_tags(task), "4 references");
        // Kept in the task's file, and read back; a task from before has none.
        let json = serde_json::to_value(task).unwrap();
        assert_eq!(
            json["refs"][1],
            serde_json::json!({"kind": "tab", "name": "Tab 1-2", "id": m.tabs[1].id, "title": "Tab 1"})
        );
        let back: Task = serde_json::from_value(json).unwrap();
        assert_eq!(&back, task);
        let old: Task = serde_json::from_str(r#"{"id":1,"title":"x","state":"captured"}"#).unwrap();
        assert!(old.refs.is_empty());
        // From the socket's line: what is open by that name now.
        m.update(Action::TaskFromLine {
            line: "look at ^app ^flow\\ app".into(),
            known: Default::default(),
            open: false,
        });
        let task = m.tasks.list.last().unwrap();
        assert_eq!((task.title.as_str(), task.refs.len()), (r"look at ^flow\ app", 1));
    }

    #[test]
    fn taking_a_row_never_loses_what_is_typed() {
        // A space typed inside a word is a space: the row is not taken, and nothing after
        // the cursor goes.
        let mut m = composing();
        keys(&mut m, "do #cablesfix the bug");
        for _ in 0.."fix the bug".len() {
            m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Left, select: false } });
        }
        assert_eq!(offered(&m), ["Cables"]);
        keys(&mut m, " ");
        assert_eq!(line(&m), "do #cables fix the bug");
        // At the end of the word it takes the row.
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::Left, select: false } });
        keys(&mut m, " ");
        assert_eq!(line(&m), "do #Cables fix the bug");
        m.update(Action::CloseCompose);

        // A line with no room for the name: the row is not taken in part. Tab says so and
        // leaves the line; ↵ adds the task with what is typed.
        let full = format!("{} #ca", "x".repeat(MAX_TITLE - 4));
        m.update(Action::OpenCompose { text: full.clone() });
        assert_eq!(
            (offered(&m), m.dropdown().unwrap().selected),
            (vec!["canvas".into(), "Cables".into(), "flow-canvas".into()], Some(0))
        );
        m.update(Action::ComposeComplete);
        assert_eq!((line(&m), m.compose_note()), (full.as_str(), Some("no room in the line for that name")));
        m.update(Action::ComposeAccept { index: 1 });
        assert_eq!(line(&m), full);
        // The note goes with the next key.
        m.update(Action::FieldEdit { edit: Edit::Move { to: Motion::End, select: false } });
        assert_eq!(m.compose_note(), None);
        let before = m.tasks.list.len();
        m.update(Action::ComposeSubmit { open: false });
        let task = m.tasks.list.last().unwrap();
        assert_eq!((m.tasks.list.len(), task.project.as_deref(), m.mode()), (before + 1, Some("ca"), Mode::Normal));

        // A file with no room is not put in in part either, and the box says so.
        m.update(Action::OpenCompose { text: "x".repeat(MAX_TITLE - 4) });
        m.update(Action::ComposeAttach { paths: vec!["/etc/hosts".into()], home: None, unreadable: 0 });
        assert_eq!((line(&m).len(), m.compose_note()), (MAX_TITLE - 4, Some("no room in the line: nothing was added")));
    }

    #[test]
    fn a_path_of_any_name_goes_in_as_one_word_or_not_at_all() {
        let mut m = composing();
        // macOS names a screenshot with a narrow no-break space before AM; a name may hold a backslash.
        let shot = "/Users/me/Desktop/Screenshot 2026-11-12 at 9.41.05\u{202f}AM.png".to_string();
        let odd = r"/tmp/a\b c".to_string();
        m.update(Action::ComposeAttach {
            paths: vec![shot.clone(), odd.clone()],
            home: Some("/Users/me".into()),
            unreadable: 0,
        });
        assert_eq!(line(&m), "~/Desktop/Screenshot\\ 2026-11-12\\ at\\ 9.41.05\\\u{202f}AM.png /tmp/a\\\\b\\ c ");
        let read = compose::path_words(line(&m));
        assert_eq!(read, [vec![shot.replace("/Users/me", "~")], vec![odd.clone()]]);
        assert_eq!(m.compose_attached(), [shot.replace("/Users/me", "~"), odd]);
        // A line break cannot be in the line; a name too long cannot be kept; one that is
        // not text was never given. Each is left out, and the box says how many.
        m.update(Action::FieldEdit { edit: Edit::SelectAll });
        let long = format!("/{}", "x".repeat(tasks::MAX_TAG));
        let paths = vec!["/tmp/a\nb".to_string(), long, "/etc/hosts".into()];
        m.update(Action::ComposeAttach { paths, home: None, unreadable: 1 });
        assert_eq!(
            (line(&m), m.compose_note()),
            ("/etc/hosts ", Some("3 left out: a name too long, or not one a line can hold"))
        );
        // Under the key reference the box is hidden: nothing goes into its line, and Esc is not its own.
        m.keys = true;
        m.update(Action::ComposeAttach { paths: vec!["/etc/passwd".into()], home: None, unreadable: 0 });
        m.update(Action::ComposeRef { target: Pointed::Pane(1) });
        m.update(Action::ComposeEscape);
        m.keys = false;
        assert_eq!((line(&m), m.mode()), ("/etc/hosts ", Mode::Compose));
    }

    #[test]
    fn a_name_in_a_line_means_one_thing_while_the_line_is_there() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let info = |pane, cwd: &str| TerminalInfo { pane, cwd: Some(cwd.into()), program: None };
        m.update(Action::SetTerminals { terminals: vec![info(1, "/code/repo"), info(2, "/code/other")], home: None });
        m.update(Action::OpenCompose { text: String::new() });
        // ⌥-click pane 1 (`repo`). It exits; pane 2 moves into a directory of that name; ⌥-click it.
        m.update(Action::ComposeRef { target: Pointed::Pane(1) });
        m.update(Action::Close { pane: Some(1) });
        m.update(Action::SetTerminals { terminals: vec![info(2, "/work/repo")], home: None });
        m.update(Action::ComposeRef { target: Pointed::Pane(2) });
        // The same pane again adds its word again, and is still one reference.
        m.update(Action::ComposeRef { target: Pointed::Pane(2) });
        assert_eq!(line(&m), "^repo ^repo-2 ^repo-2 ");
        keys(&mut m, "compare");
        m.update(Action::ComposeSubmit { open: false });
        let refs = &m.tasks.list[0].refs;
        let kept: Vec<(&str, Option<u32>, Option<&str>)> =
            refs.iter().map(|r| (r.name.as_str(), r.id, r.cwd.as_deref())).collect();
        assert_eq!(kept, [("repo", Some(1), Some("/code/repo")), ("repo-2", Some(2), Some("/work/repo"))]);

        // A draft that is cancelled keeps nothing: a client's line is read against what is
        // open, and never against what a box pointed at.
        m.update(Action::OpenCompose { text: String::new() });
        m.update(Action::ComposeRef { target: Pointed::Pane(2) });
        m.update(Action::Close { pane: Some(2) });
        assert_eq!(m.draft("x ^repo").refs, ["repo"], "the box's own line keeps it");
        let theirs = Action::TaskFromLine { line: "theirs ^repo".into(), known: Default::default(), open: false };
        m.update(theirs.clone());
        assert_eq!((m.tasks.list[1].title.as_str(), m.tasks.list[1].refs.len()), ("theirs ^repo", 0));
        m.update(Action::CloseCompose);
        assert_eq!((m.compose_refs.len(), m.draft("x ^repo").refs.len()), (0, 0));
        m.update(Action::OpenCompose { text: String::new() });
        m.update(Action::ComposeRef { target: Pointed::Tab(m.tabs[0].id) });
        m.update(Action::ComposeEscape);
        m.update(theirs);
        assert_eq!((m.compose_refs.len(), m.tasks.list[2].refs.len()), (0, 0));
        // A name that ends as a clause does reads as itself first, then without the dot.
        m.tabs[0].title = "notes v1.".into();
        m.update(Action::NewTab);
        m.tabs[1].title = "plan".into();
        let draft = m.draft(r"see ^notes\ v1. and ^plan. now");
        assert_eq!((draft.refs, draft.title.as_str()), (vec!["notes v1.".to_string(), "plan".into()], "see and now"));
    }

    #[test]
    fn the_highlight_stays_on_its_row_when_the_rows_change() {
        let mut m = composing();
        keys(&mut m, "fix #ca");
        m.update(Action::ComposeMove { delta: 1 });
        let on = |m: &Model| m.dropdown().and_then(|menu| menu.selected.map(|at| menu.rows[at].name.clone()));
        assert_eq!(on(&m).as_deref(), Some("Cables"));
        // A client adds tasks: `Cables` now has the most, and is the first row.
        for _ in 0..3 {
            m.update(
                Action::TaskFromLine { line: "more #Cables".into(), known: Default::default(), open: false }
                    .sent_by(Origin::Socket),
            );
        }
        assert_eq!((offered(&m)[0].as_str(), on(&m).as_deref()), ("Cables", Some("Cables")));
        // What the user moved to is gone from the rows: the first is highlighted again.
        m.tasks.list.retain(|task| task.project.as_deref() != Some("Cables"));
        assert_eq!(on(&m).as_deref(), Some("canvas"));
    }

    #[test]
    fn a_task_file_keeps_less_of_a_reference_than_the_session_does() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Open { kind: app(7), axis: Right, beside: None });
        let info = TerminalInfo { pane: 1, cwd: Some("/Users/me/clients/acme".into()), program: Some("vim".into()) };
        m.update(Action::SetTerminals { terminals: vec![info], home: None });
        let sketch = Installed { name: "Sketchpad".into(), path: "/Applications/Sketchpad.app".into() };
        m.update(Action::SetInstalled { apps: vec![sketch], running: Vec::new() });
        m.update(Action::OpenCompose { text: "look ^acme ^app ^Sketchpad ^Tab\\ 1".into() });
        m.update(Action::ComposeSubmit { open: false });
        let task = &m.tasks.list[0];
        // In memory, and in the state: what tells each apart.
        assert_eq!(
            (task.refs[0].cwd.as_deref(), task.refs[1].title.as_deref()),
            (Some("/Users/me/clients/acme"), Some("w7"))
        );
        // In the file: what it is, its name, its id then, its app. No window title, no
        // directory, no program, no place on disk.
        let file = serde_json::to_value(task.for_file()).unwrap();
        assert_eq!(
            file["refs"],
            serde_json::json!([
                {"kind": "pane", "name": "acme", "id": 1},
                {"kind": "pane", "name": "app", "id": 2, "app": "app"},
                {"kind": "app", "name": "Sketchpad", "app": "Sketchpad"},
                {"kind": "tab", "name": "Tab 1", "id": m.tabs[0].id},
            ])
        );
        assert_eq!(serde_json::from_value::<Task>(file).unwrap().refs.len(), 4);
    }

    #[test]
    fn the_box_is_the_drawers_new_task_state() {
        let mut m = Model::default();
        // Nothing open: the drawer shows itself, in its new-task state.
        assert!(m.welcome() && m.compose_host().is_none());
        m.update(Action::OpenCompose { text: "tidy up".into() });
        assert_eq!(m.compose_host(), Some(Host::Drawer));
        // ⇧↵ files it for later: the box closes. Nothing is open but a task: no welcome.
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!((m.tasks.list.len(), m.compose_host(), m.mode(), m.welcome()), (1, None, Mode::Normal, false));
        // ↵ adds and starts: its conversation opens, so the box goes.
        m.update(Action::OpenCompose { text: "start me".into() });
        m.update(Action::ComposeEnter);
        assert_eq!((m.compose_host(), m.talk.as_ref().and_then(|talk| talk.task.task())), (None, Some(2)));
        // With a pane showing it is the drawer too.
        term(&mut m);
        m.update(Action::OpenCompose { text: "over panes".into() });
        assert_eq!(m.compose_host(), Some(Host::Drawer));
    }

    #[test]
    fn the_drawers_picks_set_the_lines_repository_and_agent_and_say_what_enter_does() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::OpenCompose { text: "fix zoom >old @ana".into() });
        let picks = m.compose_picks();
        assert_eq!(picks.wheres, ["app"]);
        assert_eq!(picks.whos[..3], ["claude", "claude-fable-5-2", "claude-opus-5-5"]);
        // WHERE and WHO replace the line's own tags; "no repository" takes it out.
        m.update(Action::ComposePick { mark: '>', name: Some("app".into()) });
        m.update(Action::ComposePick { mark: '@', name: Some("codex-gpt-6-astra".into()) });
        assert_eq!(line(&m), "fix zoom >app @codex-gpt-6-astra ");
        let plan = Plan { agent: "codex-gpt-6-astra".into(), starts: true, dir: Some("app".into()), branch: None };
        assert_eq!(m.compose_plan(), Some(plan));
        m.update(Action::ComposePick { mark: '>', name: None });
        assert_eq!(line(&m), "fix zoom @codex-gpt-6-astra ");
        // A word that only starts with the mark is no tag: it stays, spaces and all.
        m.update(Action::FieldEdit { edit: Edit::Insert("  >= 3".into()) });
        m.update(Action::ComposePick { mark: '>', name: Some("app".into()) });
        assert_eq!(line(&m), "fix zoom @codex-gpt-6-astra   >= 3 >app ");
        m.update(Action::ComposePick { mark: '>', name: None });
        // Only what the picks offer: nothing else is written into the line.
        for (mark, name) in [('>', "elsewhere"), ('@', "ana"), ('#', "app"), ('@', "--yolo")] {
            m.update(Action::ComposePick { mark, name: Some(name.into()) });
            assert_eq!(line(&m), "fix zoom @codex-gpt-6-astra   >= 3 ", "{mark}{name}");
        }
        // No tag: the default agent, in the home folder. A saved agent starts nothing.
        m.update(Action::ComposePick { mark: '@', name: None });
        assert_eq!(line(&m), "fix zoom   >= 3 ");
        assert_eq!(
            m.compose_plan().map(|plan| (plan.agent, plan.starts, plan.dir)),
            Some(("claude".into(), true, None))
        );
        m.update(Action::SetAgents {
            claude: Vec::new(),
            codex: Vec::new(),
            default: Some("ana".into()),
            shows: View::Native,
        });
        assert_eq!(m.compose_plan().map(|plan| plan.starts), Some(false));
        m.update(Action::CloseCompose);
        assert_eq!(
            (m.compose_plan(), m.update(Action::ComposePick { mark: '>', name: Some("app".into()) })),
            (None, vec![])
        );
    }

    #[test]
    fn any_keys_in_the_box_leave_a_line_that_reads_and_a_dropdown_that_holds() {
        use unicode_segmentation::UnicodeSegmentation;
        let mut rng = crate::edit::fuzz::Rng(0x2545_f491_4f6c_dd1d);
        let mut m = composing();
        let sketch = Installed { name: "Sketch pad".into(), path: "/Applications/Sketch pad.app".into() };
        m.update(Action::SetInstalled { apps: vec![sketch], running: Vec::new() });
        for step in 0..6000 {
            if m.compose.is_none() {
                m.update(Action::OpenCompose { text: rng.text() });
            }
            let len = line(&m).len();
            let before = m.dropdown();
            let action = match rng.below(12) {
                0 => Action::ComposeMove { delta: rng.below(5) as i32 - 2 },
                1 => Action::ComposeComplete,
                2 => Action::ComposeAccept { index: rng.below(9) },
                3 => Action::ComposeEscape,
                4 if rng.below(8) == 0 => Action::ComposeSubmit { open: false },
                5 => Action::ComposeRef { target: Pointed::Pane(1) },
                6 if rng.below(4) == 0 => {
                    Action::ComposeAttach { paths: vec!["/a b/c".into()], home: None, unreadable: 0 }
                }
                _ => Action::FieldEdit { edit: rng.edit(len) },
            };
            let taken = match (&action, &before) {
                (Action::ComposeAccept { index }, Some(menu)) => {
                    menu.rows.get(*index).map(|row| (menu.mark, row.name.clone()))
                }
                (Action::ComposeComplete, Some(menu)) => {
                    menu.rows.get(menu.selected.unwrap_or(0)).map(|row| (menu.mark, row.name.clone()))
                }
                _ => None,
            };
            m.update(action.clone());
            let Some(field) = &m.compose else { continue };
            let text = field.text();
            let why = format!("step {step}: {text:?} after {action:?}");
            // The cursor is between clusters; the line reads; its spans are words of it.
            let clusters: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).chain([text.len()]).collect();
            assert!(clusters.contains(&field.cursor()), "{why}");
            let (draft, spans) = (m.draft(text), m.compose_spans());
            assert!(draft.title.len() <= text.len(), "{why}");
            assert!(
                spans.iter().all(|span| text
                    .get(span.range.clone())
                    .is_some_and(|word| !word.contains(' ') || word.contains('\\'))),
                "{why}"
            );
            // The highlight is on a row. A row taken reads back as the tag it is: a project,
            // a repository, an agent, a skill or a reference of that very name.
            if let Some(menu) = m.dropdown() {
                assert!(
                    menu.selected.is_none_or(|at| at < menu.rows.len()) && menu.rows.len() <= complete::MAX_ROWS,
                    "{why}"
                );
            }
            if let Some((mark, name)) = taken.filter(|(mark, _)| *mark != '/') {
                let named = match mark {
                    '#' => draft.project.as_ref() == Some(&name),
                    '>' => draft.repo.as_ref() == Some(&name),
                    '@' => draft.agent.as_ref() == Some(&name),
                    '$' => draft.skills.contains(&name),
                    _ => draft.refs.contains(&name),
                };
                assert!(named, "{mark}{name} was taken and does not read: {draft:?}; {why}");
            }
        }
    }

    /// The names on the dropdown's rows.
    fn offered(m: &Model) -> Vec<String> {
        m.dropdown().map(|menu| menu.rows.into_iter().map(|row| row.name).collect()).unwrap_or_default()
    }

    #[test]
    fn tags_count_only_for_names_the_shell_read() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        let line = "review the export >app @Codex $Code-Review $code-review /src/export ~/gone p2";
        // Nothing read yet: only the repository is known.
        assert_eq!(m.draft(line).title, "review the export @Codex $Code-Review $code-review /src/export ~/gone");
        assert_eq!(m.draft_repo(line).as_deref(), Some("/code/app"));
        let list = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        let names = compose::Names { agents: list(&["claude", "codex"]), skills: list(&["code-review", "deploy"]) };
        let found = compose::PathFound { typed: "/src/export".into(), resolved: "/code/app/src/export".into() };
        // What the shell read for a line counts for that line only: an answer that comes after
        // the line has changed is dropped. The names are good for any line.
        m.update(Action::OpenCompose { text: "see /src/ex".into() });
        assert!(!m.compose_read());
        let stale = Action::SetKnown {
            line: "see /src/e".into(),
            typing: Some("/src/e".into()),
            names: Some(names.clone()),
            paths: vec![found.clone()],
            entries: list(&["/src/elsewhere/"]),
        };
        m.update(stale);
        assert_eq!((offered(&m), m.compose_read()), (vec![], false));
        m.update(Action::SetKnown {
            line: "see /src/ex".into(),
            typing: Some("/src/ex".into()),
            names: None,
            paths: Vec::new(),
            entries: list(&["/src/export/", "/src/extra.rs"]),
        });
        assert_eq!((offered(&m), m.compose_read()), (list(&["/src/export/", "/src/extra.rs"]), true));
        m.update(Action::ComposeComplete);
        assert_eq!((m.compose.as_ref().map(Field::text), m.compose_read()), (Some("see /src/export/"), false));
        m.update(Action::CloseCompose);
        // Tab offers each tag's own names.
        for (typed, names) in [("@c", vec!["claude", "codex"]), ("$dep", vec!["deploy"])] {
            m.update(Action::OpenCompose { text: typed.into() });
            assert_eq!(offered(&m), names, "{typed}");
            m.update(Action::CloseCompose);
        }
        // Filed from the box: agent and skill as they are spelled where they were read, the skill
        // once; the path with where it was found; the unknown path stays in the title.
        m.update(Action::OpenCompose { text: line.into() });
        m.update(Action::SetKnown {
            line: line.into(),
            typing: None,
            names: None,
            paths: vec![found.clone()],
            entries: Vec::new(),
        });
        m.update(Action::ComposeSubmit { open: false });
        let task = &m.tasks.list[0];
        assert_eq!(
            (task.title.as_str(), task.repo.as_deref(), task.priority),
            ("review the export ~/gone", Some("/code/app"), 2)
        );
        assert_eq!((task.agent.as_deref(), task.skills.as_slice()), (Some("codex"), &list(&["code-review"])[..]));
        assert_eq!(
            (task.paths.as_slice(), task.resolved.as_slice()),
            (&list(&["/src/export"])[..], &list(&["/code/app/src/export"])[..])
        );
        assert_eq!(m.task_tags(task), ">app · @codex · 1 path · 1 skill");
        // Filed: the box closes.
        assert_eq!(m.compose_host(), None);
        // A line filed from outside brings what was read for it, and leaves what the box knows
        // alone: the line the user is typing keeps its tags.
        m.update(Action::OpenCompose { text: "mine $deploy".into() });
        let theirs = compose::Known {
            names: compose::Names { agents: list(&["ana"]), skills: Vec::new() },
            ..Default::default()
        };
        m.update(Action::TaskFromLine { line: "ask @ana $deploy".into(), known: theirs, open: false });
        assert_eq!((m.tasks.list[1].title.as_str(), m.tasks.list[1].agent.as_deref()), ("ask $deploy", Some("ana")));
        assert_eq!(m.draft("mine $deploy").skills, list(&["deploy"]));
        m.update(Action::ComposeSubmit { open: false });
        assert_eq!(
            (m.tasks.list[2].title.as_str(), m.tasks.list[2].skills.as_slice()),
            ("mine", &list(&["deploy"])[..])
        );
        // The socket's `task_add` names them without a line, in the same step.
        let assign = Assign { agent: Some("claude".into()), skills: list(&["a", "b"]), ..Assign::default() };
        m.update(Action::TaskAdd { title: "a".into(), priority: 1, state: State::Running, tab: None, assign });
        assert_eq!(m.task_tags(&m.tasks.list[3]), "@claude · 2 skills");
    }

    /// The Tasks section as (heading, count, ids of the rows drawn).
    fn sections(m: &Model) -> Vec<(Heading, usize, Vec<TaskId>)> {
        let sections = m.task_sections(0);
        sections.iter().map(|s| (s.heading, s.count, s.rows.iter().map(|r| r.id).collect())).collect()
    }

    #[test]
    fn the_tasks_section_lists_each_task_once_and_what_needs_you_first() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::Tick { now: 1_000 });
        for (title, state) in [("ask", State::NeedsYou), ("build", State::Running), ("idea", State::Captured)] {
            m.update(Action::TaskAdd { title: title.into(), priority: 2, state, tab: None, assign: Assign::default() });
        }
        m.update(Action::TaskFromLine {
            line: "fix zoom #canvas >app-2 p1".into(),
            known: Default::default(),
            open: false,
        });
        // Under its state's heading, open or not; within one, most urgent first, then oldest.
        m.update(Action::NewTabOf { task: Some(4) });
        m.update(Action::NewTab);
        m.update(Action::NewTabOf { task: Some(1) });
        m.update(Action::TaskSetState { id: 3, state: State::Parked });
        assert_eq!(
            sections(&m),
            [(Heading::NeedsYou, 1, vec![1]), (Heading::Running, 1, vec![2]), (Heading::Later, 2, vec![4, 3])],
            "Later holds what is captured and what is on hold"
        );
        // Each row: its tags, what it has open, how old it is. The one on show is marked.
        let sections = m.task_sections(1_000 + 3 * 3_600_000);
        let (needs, later) = (&sections[0].rows[0], &sections[2].rows[0]);
        assert_eq!(
            (needs.detail.as_str(), needs.tabs, needs.current, needs.group),
            ("1 tab, 0 panes · 3h", 1, true, tasks::Group::NeedsYou)
        );
        assert_eq!((later.detail.as_str(), later.current), ("#canvas · >app-2 · 2 tabs, 1 pane · 3h", false));
        // List mode walks the headings and rows in the order drawn.
        m.update(Action::ToggleSection { section: Section::Tasks });
        let (heading, task) = (ListRow::Heading, ListRow::Task);
        let drawn = [heading(Heading::NeedsYou), task(1), heading(Heading::Running), task(2)];
        assert_eq!(m.list_rows(), [&drawn[..], &[heading(Heading::Later), task(4), task(3)]].concat());
        // A finished task is under Done, last and folded: counted, not listed, until unfolded.
        m.update(Action::TaskSetState { id: 2, state: State::Done });
        let done = m.task_sections(0).pop().unwrap();
        assert_eq!((done.heading, done.count, done.collapsed, done.rows.len()), (Heading::Done, 1, true, 0));
        m.update(Action::ToggleGroup { heading: Heading::Done });
        assert_eq!(m.task_sections(0).pop().unwrap().rows[0].id, 2);
    }

    /// Tasks with these (title, project, state), filed in this order.
    fn filed(m: &mut Model, tasks: &[(&str, Option<&str>, State)]) {
        for (title, project, state) in tasks {
            let tags = project.map(|p| format!(" #{p}")).unwrap_or_default();
            m.update(Action::TaskFromLine { line: format!("{title}{tags}"), known: Default::default(), open: false });
            let id = m.tasks.list.last().map(|t| t.id).unwrap_or_default();
            m.update(Action::TaskSetState { id, state: *state });
        }
    }

    #[test]
    fn the_project_view_groups_by_project_with_no_project_last_and_a_bar_of_states() {
        let mut m = Model::default();
        use State::*;
        filed(
            &mut m,
            &[
                ("a", Some("web"), Done),
                ("b", None, Captured),
                ("c", Some("api"), Running),
                ("d", Some("web"), NeedsYou),
                ("e", Some("Beta"), Parked),
                ("f", Some("web"), Captured),
                ("g", Some("web"), Running),
            ],
        );
        let groups = m.project_groups(0);
        let names: Vec<Option<&str>> = groups.iter().map(|g| g.project.as_deref()).collect();
        assert_eq!(names, [Some("api"), Some("Beta"), Some("web"), None], "by name, whatever the case; none last");
        let web = &groups[2];
        // Needs you, running, later; Done is a folded line under them.
        assert_eq!(web.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [4, 7, 6]);
        assert_eq!((web.done, web.done_open, web.done_rows.len()), (1, false, 0));
        assert_eq!(web.bar, tasks::Segments { needs_you: 1, running: 1, later: 1, done: 1 });
        assert_eq!(web.bar.shares(), [0.25; 4]);
        assert_eq!(tasks::Segments::default().shares(), [0.; 4]);
        assert_eq!(web.bar.tip(), "1 needs you · 1 running · 1 later · 1 done");
        // List mode walks the projects, their tasks and their Done lines; ↵ on one folds it.
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        // A client's switch is not written; the user's is, once.
        assert_eq!(m.update(Action::TasksView { view: TasksView::Project }.sent_by(Origin::Socket)), []);
        m.update(Action::TasksView { view: TasksView::State }.sent_by(Origin::Socket));
        let saved = m.update(Action::TasksView { view: TasksView::Project });
        assert_eq!(saved, [Effect::SaveTasksView { view: TasksView::Project }]);
        assert_eq!(m.update(Action::TasksView { view: TasksView::Project }), []);
        let (project, task) = (|p: Option<&str>| ListRow::Project(p.map(String::from)), ListRow::Task);
        let web_done = ListRow::ProjectDone(Some("web".into()));
        assert_eq!(
            m.list_rows()[..9],
            [
                project(Some("api")),
                task(3),
                project(Some("Beta")),
                task(5),
                project(Some("web")),
                task(4),
                task(7),
                task(6),
                web_done.clone()
            ]
        );
        assert_eq!(m.list, Some(5), "still on the task it was on: d, which needs you");
        m.update(Action::ListMove { delta: 3 });
        m.update(Action::ListChoose { axis: Axis::Right });
        assert_eq!((m.open_done.as_slice(), m.list_rows()[9].clone()), (&[Some("web".to_string())][..], task(1)));
        m.update(Action::ToggleProject { project: Some("web".into()), done: false });
        assert_eq!(m.project_groups(0)[2].rows.len(), 0);
        assert_eq!(m.list_rows()[4..6], [project(Some("web")), project(None)]);
    }

    #[test]
    fn the_sessions_read_for_work_are_of_open_tasks_and_those_done_just_now_running_first() {
        let mut m = Model::default();
        m.update(Action::Tick { now: 1_000_000 });
        filed(&mut m, &[("a", None, State::Captured), ("b", None, State::Running), ("c", None, State::Running)]);
        for id in 1..=3 {
            m.tasks.link(id, sessions::Link { runner: "claude".into(), id: format!("s{id}"), state: None });
        }
        let ids = |m: &Model| m.worked_sessions().into_iter().map(|(_, id, _)| id).collect::<Vec<_>>();
        assert_eq!(ids(&m), ["s2", "s3", "s1"]);
        // Done: read a while longer, for what it wrote after the last read. Then no more.
        m.update(Action::TaskSetState { id: 3, state: State::Done });
        assert_eq!(ids(&m), ["s2", "s1", "s3"]);
        m.update(Action::Tick { now: 1_000_000 + progress::LAST_READ_MS + 1 });
        assert_eq!(ids(&m), ["s2", "s1"]);
    }

    #[test]
    fn a_tasks_time_and_tokens_come_from_its_sessions_work() {
        const UUID: &str = "8f14e45f-ceea-467f-a8d3-0242ac130003";
        let mut m = Model::default();
        m.update(Action::Tick { now: 5_000 });
        filed(&mut m, &[("ship it", Some("web"), State::Captured)]);
        let link = sessions::Link { runner: "claude".into(), id: UUID.into(), state: None };
        m.tasks.link(1, link);
        let tokens =
            |model: &str, sub, input| progress::Tokens { model: model.into(), sub, input, output: 2, cached: 1 };
        let work = |session: &str, ms, day, day_ms, rows| progress::Work {
            runner: "claude".into(),
            session: session.into(),
            clock: progress::Clock { ms, last: None, day, day_ms },
            tokens: rows,
            context: Some(50_000),
            window: Some(200_000),
        };
        // Another session's work, or a runner not listed: no task takes it.
        m.update(Action::SessionWork { work: work("other", 9, 0, 0, Vec::new()) });
        assert!(m.tasks.list[0].work.is_empty());
        let rows = vec![tokens("claude-opus-5-5", false, 10), tokens("claude-haiku-5", true, 4)];
        m.update(Action::SessionWork { work: work(UUID, 60_000, 0, 0, rows.clone()) });
        // Read again: it replaces what was kept of that session, it does not add to it.
        m.update(Action::SessionWork { work: work(UUID, 90_000, 1_000, 30_000, rows) });
        let task = &m.tasks.list[0];
        assert_eq!(
            (task.work.len(), task.work[0].clock.ms, task.work[0].tokens[0].model.as_str()),
            (1, 90_000, "opus-5.5")
        );
        // It started when its agent did; it is finished when it is done, and not when reopened.
        m.tasks.started(1, Some(4_000));
        m.update(Action::Tick { now: 7_000 });
        m.update(Action::TaskSetState { id: 1, state: State::Done });
        assert_eq!(m.tasks.list[0].finished_at, Some(7_000));
        // The footer: what was done since the day began, what runs, and today's time at work.
        assert_eq!(m.today(1_000).line(), "1 done · 0 running · 30s");
        assert_eq!(m.today(8_000), Today { done: 0, running: 0, agent_ms: 0 });
        // The card: its state, tags, times, workspace and tokens.
        let card = m.card(1, 4_000 + 3 * 3_600_000).unwrap();
        assert_eq!(
            (card.state, card.tags.as_slice(), card.started.as_deref()),
            ("Done", &["#web".to_string()][..], Some("3h"))
        );
        assert_eq!((card.agent_time.as_str(), card.workspace.as_str()), ("1m", "0 tabs · 0 panes"));
        let table: Vec<_> = card.tokens.iter().map(|t| (t.model.as_str(), t.sub, t.input)).collect();
        assert_eq!(table, [("opus-5.5", false, 10), ("haiku-5", true, 4)]);
        assert_eq!((card.context, card.window), (Some(50_000), Some(200_000)));
        m.update(Action::TaskSetState { id: 1, state: State::Running });
        assert_eq!(m.tasks.list[0].finished_at, None);
        // A key this build does not know is written back as it was.
        let text = r#"{"id":1,"title":"t","state":"done","workspace":{"tabs":[1]}}"#;
        let read: Task = serde_json::from_str(text).unwrap();
        assert!(serde_json::to_string(&read).unwrap().contains(r#""workspace":{"tabs":[1]}"#));
    }

    #[test]
    fn a_collapsed_heading_keeps_its_count_and_hides_its_rows() {
        let mut m = Model::default();
        for (title, state) in [("a", State::NeedsYou), ("b", State::NeedsYou), ("c", State::Running)] {
            m.update(Action::TaskAdd { title: title.into(), priority: 2, state, tab: None, assign: Assign::default() });
        }
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        m.update(Action::ListMove { delta: 1 }); // on "b"
        m.update(Action::ToggleGroup { heading: Heading::Running });
        assert_eq!(sections(&m), [(Heading::NeedsYou, 2, vec![1, 2]), (Heading::Running, 1, vec![])]);
        let (heading, task) = (ListRow::Heading, ListRow::Task);
        let (needs, running) = (heading(Heading::NeedsYou), heading(Heading::Running));
        assert_eq!(
            (m.list_rows(), m.list),
            (vec![needs.clone(), task(1), task(2), running.clone()], Some(2)),
            "it stays"
        );
        // The selected row's own heading: the selection goes to a row that is still drawn.
        m.update(Action::ToggleGroup { heading: Heading::Running });
        m.update(Action::ToggleGroup { heading: Heading::NeedsYou });
        assert_eq!(
            (m.list_rows(), m.list, m.collapsed.as_slice()),
            (
                vec![needs.clone(), running.clone(), task(3)],
                Some(2),
                &[Heading::Done, Heading::Sessions, Heading::NeedsYou][..]
            )
        );
        // All folded is no dead end: a heading is a row, and ↵ on it unfolds it. The selection stays on it.
        m.update(Action::ToggleGroup { heading: Heading::Running });
        assert_eq!((m.list_rows(), m.list), (vec![needs.clone(), running.clone()], Some(1)));
        assert_eq!(m.update(Action::ListChoose { axis: Right }), []);
        assert_eq!(
            (m.list_rows(), m.list, m.mode()),
            (vec![needs.clone(), running.clone(), task(3)], Some(1), Mode::List)
        );
        m.update(Action::ListChoose { axis: Right });
        assert_eq!(m.collapsed.as_slice(), &[Heading::Done, Heading::Sessions, Heading::NeedsYou, Heading::Running]);
        m.update(Action::ToggleGroup { heading: Heading::NeedsYou });
        assert_eq!((sections(&m)[0].2.len(), m.collapsed.len()), (2, 3));
    }

    #[test]
    fn other_sessions_start_folded_and_list_mode_skips_their_rows() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::Tick { now: 5_000 });
        m.update(Action::SetSessions { found: vec![session("claude", "busy", "/code/app", true, 5_000)], day: 0 });
        m.update(Action::ToggleSection { section: Section::Tasks });
        // At work, and still folded: the heading counts it, list mode has the heading alone.
        let bar = m.sessions_bar();
        assert_eq!((bar.count, bar.collapsed, bar.rows.len()), (1, true, 0));
        assert_eq!(m.list_rows(), [ListRow::Heading(Heading::Sessions)]);
        assert_eq!(Heading::Sessions.title(), "Other sessions");
        // ↵ on the heading unfolds it, as any other.
        m.update(Action::FocusList);
        m.update(Action::ListChoose { axis: Right });
        assert_eq!(m.list_rows()[1], ListRow::Session("claude".into(), "busy".into()));
    }

    fn session(runner: &str, id: &str, cwd: &str, active: bool, at: u64) -> Found {
        let (runner, id, cwd, repo) = (runner.to_string(), id.to_string(), cwd.to_string(), cwd.to_string());
        let (station, label, title, model, held) = (None, None, None, None, false);
        let (started, last_activity) = (None, Some(at));
        Found { runner, id, cwd, repo, started, last_activity, active, station, label, title, model, held }
    }

    /// The Other sessions group as (count in range, ids of the rows drawn, count in all).
    fn session_rows(m: &Model) -> (usize, Vec<String>, usize) {
        let bar = m.sessions_bar();
        (bar.count, bar.rows.into_iter().map(|row| row.id).collect(), bar.total)
    }

    #[test]
    fn sessions_that_are_no_tasks_are_listed_active_first_and_in_their_range() {
        let mut m = Model::default();
        m.update(Action::ToggleGroup { heading: Heading::Sessions }); // folded at first
        m.update(Action::SetRepos { repos: listed() });
        // 10 days in; the local day started 5 hours ago.
        let (now, hour) = (10 * sessions::DAY_MS, 3_600_000);
        m.update(Action::Tick { now });
        let mut busy = session("claude", "busy", "/code/app/crates", true, now - 2_000);
        (busy.repo, busy.station, busy.label) = ("/code/app".into(), Some("shell".into()), Some("cargo test".into()));
        m.update(Action::SetSessions {
            found: vec![
                session("codex", "old", "/code/app", false, now - 3 * sessions::DAY_MS),
                session("codex", "idle", "/code/app-2", false, now - 3 * hour),
                busy,
            ],
            day: now - 5 * hour,
        });
        // Today, at first: the one at work and the one idle since the day started.
        assert_eq!(session_rows(&m), (2, vec!["busy".into(), "idle".into()], 3));
        let rows = m.sessions_bar().rows;
        // Named after the listed repository or worktree, wherever below it the session started.
        assert_eq!(
            (
                rows[0].runner.as_str(),
                rows[0].name.as_str(),
                rows[0].active,
                rows[0].doing.as_deref(),
                rows[0].age.as_deref()
            ),
            ("claude", "app", true, Some("shell · cargo test"), Some("now"))
        );
        assert_eq!(
            (
                rows[1].runner.as_str(),
                rows[1].name.as_str(),
                rows[1].active,
                rows[1].doing.as_deref(),
                rows[1].age.as_deref()
            ),
            ("codex", "app-2", false, None, Some("3h"))
        );
        // 7 days takes in the one of three days ago. The user's pick is written; a client's is not.
        let week = Action::SessionsRange { range: sessions::Range::Week };
        assert_eq!(m.update(week.clone().sent_by(Origin::Socket)), []);
        assert_eq!(session_rows(&m), (3, vec!["busy".into(), "idle".into(), "old".into()], 3));
        let today = Action::SessionsRange { range: sessions::Range::Today };
        assert_eq!(
            m.update(today.sent_by(Origin::User)),
            [Effect::SaveSessionsRange { range: sessions::Range::Today }]
        );
        assert_eq!(
            m.update(week.clone().sent_by(Origin::User)),
            [Effect::SaveSessionsRange { range: sessions::Range::Week }]
        );
        assert_eq!(m.update(week.sent_by(Origin::User)), [], "the same range: nothing to write");
        // The heading folds like any other.
        m.update(Action::ToggleGroup { heading: Heading::Sessions });
        assert_eq!(session_rows(&m), (3, vec![], 3));
        m.update(Action::ToggleGroup { heading: Heading::Sessions });
        // List mode walks the tasks, then the sessions as drawn: their heading, the rows.
        m.update(Action::TaskAdd {
            title: "a".into(),
            priority: 2,
            state: State::Captured,
            tab: None,
            assign: Assign::default(),
        });
        m.update(Action::ToggleSection { section: Section::Tasks });
        let row = |id: &str, runner: &str| ListRow::Session(runner.into(), id.into());
        let tasks = [ListRow::Heading(Heading::Later), ListRow::Task(1), ListRow::Heading(Heading::Sessions)];
        let sessions = [row("busy", "claude"), row("idle", "codex"), row("old", "codex")];
        assert_eq!(m.list_rows(), [&tasks[..], &sessions[..]].concat());
        // ↵ on the heading folds the group. None in range: the heading stays, to widen it again.
        m.update(Action::FocusList);
        m.update(Action::ListMove { delta: 1 });
        m.update(Action::ListChoose { axis: Right });
        assert_eq!((m.list_rows(), m.list), (tasks.to_vec(), Some(2)));
        m.update(Action::ListChoose { axis: Right });
        m.update(Action::LeaveList);
        m.update(Action::Tick { now: now + 9 * sessions::DAY_MS });
        m.update(Action::SetSessions { found: vec![session("codex", "old", "/code/app", false, 0)], day: now });
        assert_eq!((session_rows(&m), m.list_rows().len()), ((0, vec![], 1), 3));
        // Hidden: gone for this run, whatever the shell finds again. The group goes with its last row.
        m.update(Action::SessionHide { runner: "codex".into(), id: "old".into() });
        m.update(Action::SetSessions { found: vec![session("codex", "old", "/code/app", true, now)], day: 0 });
        assert_eq!(session_rows(&m), (0, vec![], 0));
    }

    #[test]
    fn a_sessions_title_and_model_are_in_its_row_and_in_no_task() {
        let mut m = Model::default();
        m.update(Action::ToggleGroup { heading: Heading::Sessions }); // folded at first
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::Tick { now: 5_000 });
        let mut titled = session("claude", "s1", "/code/app", false, 4_000);
        titled.title = Some("Rename it #urgent >app-2 @codex p0\nand run this".into());
        titled.model = Some("claude-opus-4-7[1m]".into());
        m.update(Action::SetSessions {
            found: vec![titled, session("codex", "s2", "/code/app", false, 1_000)],
            day: 0,
        });
        let rows = m.sessions_bar().rows;
        let shown = |row: &sessions::Row| (row.quoted_title.clone(), row.model.clone());
        assert_eq!(shown(&rows[0]), (Some("Rename it #urgent >app-2 @codex p0".into()), Some("opus-4.7".into())));
        assert_eq!(shown(&rows[1]), (None, None));
        // Adopted, the task is named after the repository as ever: the title names nothing, and
        // its words are no tags.
        m.update(Action::SessionAdopt { runner: "claude".into(), id: "s1".into() });
        let task = m.tasks.list[0].clone();
        assert_eq!((task.title.as_str(), task.project.as_deref()), ("app", None));
        assert_eq!((task.repo.as_deref(), task.agent.as_deref()), (Some("/code/app"), Some("claude")));
        assert!(!serde_json::to_string(&m.tasks).unwrap().contains("Rename it"));
    }

    /// Sessions of the listed checkouts, for the conversation tests: `idle` started below its
    /// repository, `busy` at work, `held` idle with a live process, `astray` left the checkout,
    /// `plain` with an id that is no UUID.
    fn conversations() -> Model {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::Tick { now: 9_000_000 });
        let below = |runner: &str, id: &str, cwd: &str| Found {
            repo: "/code/app".into(),
            ..session(runner, id, cwd, false, 1_000)
        };
        let found = vec![
            Found { title: Some("Fix the login".into()), ..below("claude", IDLE, "/code/app/crates/core") },
            session("codex", BUSY, "/code/app-2", true, 9_000_000),
            Found { held: true, ..session("claude", HELD, "/code/app", false, 2_000) },
            below("claude", ASTRAY, "/code/app/../other"),
            session("claude", "plain", "/code/app", false, 500),
        ];
        m.update(Action::SetSessions { found, day: 0 });
        m
    }

    const IDLE: &str = "8f14e45f-ceea-467f-a8d3-0242ac130001";
    const BUSY: &str = "8f14e45f-ceea-467f-a8d3-0242ac130002";
    const HELD: &str = "8f14e45f-ceea-467f-a8d3-0242ac130003";
    const ASTRAY: &str = "8f14e45f-ceea-467f-a8d3-0242ac130004";

    fn of_session(runner: &str, id: &str) -> Subject {
        Subject::Session { runner: runner.into(), id: id.into() }
    }

    /// `claude --resume <id>` (or Codex's) for `subject`, in `cwd`.
    fn resumed(subject: Subject, runner: &str, id: &str, cwd: &str) -> Effect {
        let start = agents::Start::Resume { runner: RunnerId::named(runner).unwrap(), session: id.into() };
        Effect::StartAgent { task: subject, start, cwd: Some(cwd.into()), brief: None }
    }

    /// A session another terminal holds is watched, read-only: what it did, no card, no turn;
    /// once idle it may be taken over, by the user's click.
    #[test]
    fn a_session_held_elsewhere_is_watched_and_taken_over_once_idle() {
        use crate::transcript::{Event, Text};
        let mut m = conversations();
        let held = of_session("claude", HELD);
        assert_eq!(m.update(Action::SessionOpen { runner: "claude".into(), id: HELD.into() }), []);
        assert_eq!(m.watch_target().map(|(subject, ..)| subject), Some(held.clone()));
        assert!(!m.take_over_ready(&held));
        let tool = Event::ToolStart {
            id: "a".into(),
            sub: false,
            tool: "Bash".into(),
            label: "git".into(),
            detail: Text::default(),
            diff: None,
        };
        let card = Event::Permission {
            ticket: 1,
            tool: "Bash".into(),
            label: "rm".into(),
            detail: Text::default(),
            len: 0,
            cut: false,
            claim: Text::default(),
        };
        let events = Action::AgentEvents { task: held.clone(), events: vec![Event::TurnStarted, tool, card], at: 1 };
        assert!(m.update(events.clone().sent_by(Origin::Socket)).is_empty() && m.chat(&held).is_none());
        m.update(events.sent_by(Origin::System));
        let t = &m.chat(&held).expect("a watched transcript").transcript;
        assert!(t.watched && !t.live && t.turn.is_none() && t.cards.is_empty(), "{t:?}");
        assert_eq!(t.items().len(), 1);
        // Watched again, the file is read from its start: what it shows once, not twice.
        let again = vec![
            Event::WatchFromStart,
            Event::ToolStart {
                id: "a".into(),
                sub: false,
                tool: "Bash".into(),
                label: "git".into(),
                detail: Text::default(),
                diff: None,
            },
        ];
        m.update(Action::AgentEvents { task: held.clone(), events: again, at: 2 }.sent_by(Origin::System));
        assert_eq!(m.chat(&held).map(|chat| chat.transcript.items().len()), Some(1));
        // Idle and no longer held: no more watching, and Take over (the user's open) goes on with it.
        let free = Found { repo: "/code/app".into(), ..session("claude", HELD, "/code/app", false, 2_000) };
        m.update(Action::SetSessions { found: vec![free], day: 0 });
        assert!(m.watch_target().is_none() && m.take_over_ready(&held));
        let took = m.update(Action::SessionOpen { runner: "claude".into(), id: HELD.into() });
        assert!(matches!(&took[..], [Effect::StartAgent { start: agents::Start::Resume { .. }, .. }]), "{took:?}");
    }

    #[test]
    fn opening_a_session_goes_on_with_it_in_the_drawer_where_it_started() {
        let mut m = conversations();
        let open = |m: &mut Model, runner: &str, id: &str| {
            m.update(Action::SessionOpen { runner: runner.into(), id: id.into() })
        };
        let idle = of_session("claude", IDLE);
        // Gone on with in the folder it started in, below the repository: `claude --resume`
        // finds a session by its directory. No task, no pane, no tab: the workspace stays.
        assert_eq!(open(&mut m, "claude", IDLE), [resumed(idle.clone(), "claude", IDLE, "/code/app/crates/core")]);
        assert_eq!((m.tasks.list.len(), m.panes.len(), m.tabs.len(), m.mode()), (0, 0, 1, Mode::Talk));
        assert_eq!(m.talk, Some(Talk { task: idle.clone(), open: true, keys: true }));
        // Once: a second open, before the shell says it started and after, starts nothing.
        assert_eq!(open(&mut m, "claude", IDLE), []);
        m.update(Action::AgentStarted { task: idle.clone(), session: None, at: 1 }.sent_by(Origin::System));
        assert_eq!((open(&mut m, "claude", IDLE), m.agents.clone()), (vec![], vec![idle.clone()]));
        // On the rail like a task's: its title's badge, a ring only while at work, no task.
        let cell = &m.rail()[0];
        assert_eq!((&cell.task, cell.mono.as_str(), cell.group, cell.shown), (&idle, "FT", None, true));
        assert_eq!(cell.tip, "Fix the login\nSession\napp · @claude · 2h");
        // Stop ends it; a click goes on with it again.
        assert_eq!(m.update(Action::TaskStop { id: idle.clone() }), [Effect::StopAgent { task: idle.clone() }]);
        m.update(Action::AgentExited { task: idle.clone() }.sent_by(Origin::System));
        assert_eq!(open(&mut m, "claude", IDLE).len(), 1);
        // At work elsewhere, or idle and held by a live process: the drawer says so, and no
        // second program is started on it.
        for (runner, id) in [("codex", BUSY), ("claude", HELD)] {
            assert_eq!(open(&mut m, runner, id), [], "{id}");
            assert_eq!(m.talk.as_ref().map(|talk| (&talk.task, talk.keys)), Some((&of_session(runner, id), false)));
            let note = m.talk_note().unwrap_or_default();
            assert_eq!(note, "its session is live in another terminal: go on with it there");
        }
        // Out of the listed checkout, or an id no command is built from: nothing started.
        for id in [ASTRAY, "plain"] {
            assert_eq!(open(&mut m, "claude", id), [], "{id}");
            assert_eq!(m.talk_note().as_deref(), Some("this session cannot be gone on with from here"));
        }
        // Not listed (hidden, never read): nothing opens.
        m.update(Action::SessionHide { runner: "codex".into(), id: BUSY.into() });
        let before = m.talk.clone();
        assert_eq!((open(&mut m, "codex", BUSY), open(&mut m, "claude", "nope")), (vec![], vec![]));
        assert_eq!(m.talk, before);
    }

    /// Our own agent just stopped: what its session's file shows (written lately, a process
    /// recorded for it) is ours, and reopening goes on with it at once.
    #[test]
    fn a_session_whose_agent_we_stopped_is_not_live_elsewhere() {
        let mut m = conversations();
        let idle = of_session("claude", IDLE);
        let system = |m: &mut Model, action: Action| m.update(action.sent_by(Origin::System));
        let open = Action::SessionOpen { runner: "claude".into(), id: IDLE.into() };
        let listed = |m: &mut Model, last: u64, held: bool| {
            let found = m.found.iter().map(|s| match s.id == IDLE {
                true => Found { active: true, held, last_activity: Some(last), ..s.clone() },
                false => s.clone(),
            });
            let found = found.collect();
            system(m, Action::SetSessions { found, day: 0 });
        };
        let now = 9_000_000;
        m.update(open.clone());
        system(&mut m, Action::AgentStarted { task: idle.clone(), session: None, at: now });
        // It wrote while it ran, and Claude Code recorded its process.
        listed(&mut m, now + 4_000, true);
        system(&mut m, Action::Tick { now: now + 5_000 });
        m.update(Action::TaskStop { id: idle.clone() });
        system(&mut m, Action::AgentExited { task: idle.clone() });
        // Read again within the 30 s after which a session reads idle: still "at work".
        listed(&mut m, now + 5_500, false);
        assert_eq!(m.talk_note(), None);
        let resumes = [resumed(idle.clone(), "claude", IDLE, "/code/app/crates/core")];
        assert_eq!(m.update(open.clone()), resumes);
        system(&mut m, Action::AgentExited { task: idle.clone() });
        // Written to since it ended, by another terminal: live there.
        listed(&mut m, now + 30_000, true);
        assert_eq!(m.update(open.clone()), []);
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));

        // A task's the same: its agent stopped, the stale record of its process is ours.
        let stopped = || {
            let mut m = conversations();
            m.update(Action::SessionAdopt { runner: "claude".into(), id: IDLE.into() });
            system(&mut m, Action::AgentStarted { task: Subject::Task(1), session: None, at: now });
            listed(&mut m, now, true);
            m.update(Action::TaskStop { id: Subject::Task(1) });
            system(&mut m, Action::AgentExited { task: Subject::Task(1) });
            // A read in flight when it stopped says the session is at work: the task is not moved by it.
            listed(&mut m, now, true);
            m
        };
        let mut m = stopped();
        assert_eq!(m.tasks.get(1).map(|task| task.state), Some(State::NeedsYou));
        assert_eq!(m.talk_note(), None);
        let resumes = [resumed(Subject::Task(1), "claude", IDLE, "/code/app/crates/core")];
        assert_eq!(m.update(Action::TaskTalk { id: 1 }), resumes);
        // Two rounds on, a listing that records a process for it is believed.
        let mut m = stopped();
        system(&mut m, Action::Tick { now: now + ENDED_HELD_MS + 1 });
        listed(&mut m, now, true);
        assert_eq!(m.update(Action::TaskTalk { id: 1 }), []);
        // A read made for the resume came after it stopped: a process it records is another's.
        let mut m = stopped();
        system(&mut m, Action::SetSessions { found: Vec::new(), day: 0 });
        assert_eq!(m.update(Action::TaskTalk { id: 1 }).len(), 1);
        let held = Found { held: true, repo: "/code/app".into(), ..session("claude", IDLE, "/code/app", false, now) };
        let read = Action::SessionRead { runner: "claude".into(), id: IDLE.into(), found: Some(held) };
        assert_eq!(system(&mut m, read), []);
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));
    }

    /// An adopted session gone on with in the drawer: its task runs at once, as on a new start.
    #[test]
    fn a_task_gone_on_with_runs_at_once() {
        let mut m = conversations();
        m.update(Action::SessionAdopt { runner: "claude".into(), id: IDLE.into() }.sent_by(Origin::Socket));
        assert_eq!(m.tasks.get(1).map(|task| task.state), Some(State::Captured));
        assert_eq!(m.update(Action::TaskTalk { id: 1 }).len(), 1);
        m.update(Action::AgentStarted { task: Subject::Task(1), session: None, at: 1 }.sent_by(Origin::System));
        let task = m.tasks.get(1).expect("the task");
        assert_eq!(
            (task.state, task.session.as_ref().and_then(|link| link.state)),
            (State::Running, Some(State::Running))
        );
        // Its agent gone: the user's to look at.
        m.update(Action::AgentExited { task: Subject::Task(1) }.sent_by(Origin::System));
        assert_eq!(m.tasks.get(1).map(|task| task.state), Some(State::NeedsYou));
    }

    /// The Tasks section closed: a session's conversation keeps the name and badge it had.
    #[test]
    fn a_session_conversation_keeps_its_name_when_it_is_not_listed() {
        let mut m = conversations();
        m.update(Action::SessionOpen { runner: "claude".into(), id: IDLE.into() });
        m.update(
            Action::AgentStarted { task: of_session("claude", IDLE), session: None, at: 1 }.sent_by(Origin::System),
        );
        m.update(Action::SetSessions { found: Vec::new(), day: 0 }.sent_by(Origin::System));
        assert_eq!(m.session_title("claude", IDLE), "Fix the login");
        assert_eq!(m.rail()[0].mono, "FT");
        assert!(m.rail()[0].tip.starts_with("Fix the login\n"), "{}", m.rail()[0].tip);
    }

    #[test]
    fn only_the_user_opens_a_sessions_conversation() {
        for origin in [Origin::Socket, Origin::System] {
            let mut m = conversations();
            let open = Action::SessionOpen { runner: "claude".into(), id: IDLE.into() };
            assert_eq!(m.refusal(&open, origin), Some(agents::USERS_ALONE));
            assert_eq!((m.update(open.sent_by(origin)), m.talk.clone()), (vec![], None));
            // A client's adopt makes the task, as before, and starts nothing.
            let adopt = Action::SessionAdopt { runner: "claude".into(), id: IDLE.into() };
            assert_eq!((m.update(adopt.sent_by(origin)), m.tasks.list.len(), m.talk.clone()), (vec![], 1, None));
        }
    }

    #[test]
    fn adopting_a_session_makes_its_conversation_the_tasks() {
        let mut m = conversations();
        let adopt = |m: &mut Model, runner: &str, id: &str| {
            m.update(Action::SessionAdopt { runner: runner.into(), id: id.into() })
        };
        // Open in the drawer: the conversation becomes the task's, the same program, no second one.
        let idle = of_session("claude", IDLE);
        m.update(Action::SessionOpen { runner: "claude".into(), id: IDLE.into() });
        m.update(Action::AgentStarted { task: idle.clone(), session: None, at: 1 }.sent_by(Origin::System));
        assert_eq!(adopt(&mut m, "claude", IDLE), [Effect::AgentMoved { from: idle, to: Subject::Task(1) }]);
        assert_eq!(
            (m.agents.clone(), m.talk.clone()),
            (vec![Subject::Task(1)], Some(Talk { task: Subject::Task(1), open: true, keys: true }))
        );
        assert_eq!((m.panes.len(), m.tabs.len(), m.tab_task()), (0, 1, None), "no shell tab");
        assert_eq!(m.rail().iter().map(|cell| cell.task.clone()).collect::<Vec<_>>(), [Subject::Task(1)]);
        // Not open: the task's conversation, gone on with where the session started.
        m.update(Action::SetSessions {
            found: vec![Found { repo: "/code/app".into(), ..session("claude", HELD, "/code/app/web", false, 2_000) }],
            day: 0,
        });
        assert_eq!(adopt(&mut m, "claude", HELD), [resumed(Subject::Task(2), "claude", HELD, "/code/app/web")]);
        assert_eq!(m.tasks.get(2).and_then(|task| task.repo.as_deref()), Some("/code/app"));
        // At work elsewhere: the task, its conversation with the note, no second program.
        let mut m = conversations();
        assert_eq!(adopt(&mut m, "codex", BUSY), []);
        assert_eq!(m.talk.as_ref().map(|talk| &talk.task), Some(&Subject::Task(1)));
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));
    }

    /// A task kept on disk before conversations were gone on with in the drawer: a click on it
    /// goes on with its session there, where the session started.
    #[test]
    fn a_task_adopted_before_goes_on_with_its_session_in_the_drawer() {
        let mut m = conversations();
        let kept = format!(
            r#"{{"id":7,"title":"app","state":"needs_you","priority":3,"repo":"/code/app","agent":"claude","created":1,
                "session":{{"runner":"claude","id":"{IDLE}","state":"needs_you"}}}}"#
        );
        let task: Task = serde_json::from_str(&kept).expect("a task as kept");
        m.update(Action::RestoreTasks { tasks: vec![task], floor: 7 });
        assert_eq!(
            m.update(Action::TaskTalk { id: 7 }),
            [resumed(Subject::Task(7), "claude", IDLE, "/code/app/crates/core")]
        );
        // Not listed (the Tasks section closed): that one session is read first, and the resume
        // waits on it with the keyboard in the drawer. Read: where it started.
        let closed = || {
            let mut m = Model::default();
            m.update(Action::SetRepos { repos: listed() });
            m.update(Action::RestoreTasks { tasks: vec![serde_json::from_str(&kept).expect("kept")], floor: 7 });
            let read = Effect::ReadSession { runner: "claude".into(), id: IDLE.into() };
            assert_eq!(m.update(Action::TaskTalk { id: 7 }), [read]);
            assert_eq!((m.mode(), m.agent_runs(&Subject::Task(7)), m.talk_note()), (Mode::Talk, false, None));
            m
        };
        let read = |found: Option<Found>| Action::SessionRead { runner: "claude".into(), id: IDLE.into(), found };
        let below = Found { repo: "/code/app".into(), ..session("claude", IDLE, "/code/app/crates/core", false, 1) };
        let mut m = closed();
        let resumes = [resumed(Subject::Task(7), "claude", IDLE, "/code/app/crates/core")];
        assert_eq!(m.update(read(Some(below.clone())).sent_by(Origin::System)), resumes);
        assert_eq!((m.mode(), m.agent_runs(&Subject::Task(7))), (Mode::Talk, true));
        // Held by a live process: not gone on with, and the drawer says why.
        let mut m = closed();
        assert_eq!(m.update(read(Some(Found { held: true, ..below.clone() })).sent_by(Origin::System)), []);
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));
        assert_eq!(m.mode(), Mode::Normal);
        // No listed checkout has it: in the task's repository. Asked once: a second read starts nothing.
        let mut m = closed();
        let home = [resumed(Subject::Task(7), "claude", IDLE, "/code/app")];
        assert_eq!(m.update(read(None).sent_by(Origin::System)), home);
        assert_eq!(m.update(read(None).sent_by(Origin::System)), []);
        // At work in another terminal as it is read: not gone on with.
        let mut m = closed();
        let busy = Found { active: true, last_activity: Some(9_000_000), ..below.clone() };
        assert_eq!(m.update(read(Some(busy)).sent_by(Origin::System)), []);
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));
        // Stopped while it is read: the resume is asked for no more.
        let mut m = closed();
        assert_eq!(m.update(Action::TaskStop { id: Subject::Task(7) }), []);
        assert_eq!(m.update(read(None).sent_by(Origin::System)), []);
        // A read nobody waits on starts nothing.
        let mut m = Model::default();
        m.update(Action::RestoreTasks { tasks: vec![serde_json::from_str(&kept).expect("kept")], floor: 7 });
        assert_eq!(m.update(read(Some(below)).sent_by(Origin::System)), []);
        // A live process holds it: not gone on with.
        let mut m = conversations();
        let found = m.found.iter().map(|s| Found { held: s.id == IDLE, ..s.clone() }).collect();
        m.update(Action::SetSessions { found, day: 0 });
        m.update(Action::RestoreTasks { tasks: vec![serde_json::from_str(&kept).expect("a task as kept")], floor: 7 });
        assert_eq!(m.update(Action::TaskTalk { id: 7 }), []);
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));
        // Read, and where it started is no longer listed: not gone on with in the repository instead.
        let mut m = conversations();
        m.update(Action::SetRepos { repos: Vec::new() });
        m.update(Action::RestoreTasks { tasks: vec![serde_json::from_str(&kept).expect("a task as kept")], floor: 7 });
        assert_eq!(m.update(Action::TaskTalk { id: 7 }), []);
    }

    #[test]
    fn adopting_a_session_makes_it_a_task() {
        let mut m = Model::default();
        m.update(Action::ToggleGroup { heading: Heading::Sessions }); // folded at first
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::Tick { now: 5_000 });
        m.update(Action::SetSessions {
            found: vec![
                session("claude", "s1", "/code/app-2", true, 4_000),
                session("codex", "s2", "/code/app", false, 1_000),
            ],
            day: 0,
        });
        m.update(Action::SessionAdopt { runner: "claude".into(), id: "s1".into() });
        let task = &m.tasks.list[0];
        assert_eq!((task.title.as_str(), task.state, task.created), ("app-2", State::Running, Some(5_000)));
        assert_eq!((task.repo.as_deref(), task.agent.as_deref()), (Some("/code/app-2"), Some("claude")));
        let link = sessions::Link { runner: "claude".into(), id: "s1".into(), state: Some(State::Running) };
        assert_eq!(task.session, Some(link));
        assert_eq!(m.task_tags(task), ">app-2 · @claude");
        // It left the Sessions group, and stays out when the shell finds it again or the task is dismissed.
        assert_eq!(session_rows(&m), (1, vec!["s2".into()], 1));
        m.update(Action::SessionAdopt { runner: "claude".into(), id: "s1".into() });
        m.update(Action::TaskDismiss { id: 1 });
        m.update(Action::SetSessions {
            found: vec![
                session("claude", "s1", "/code/app-2", true, 4_500),
                session("codex", "s2", "/code/app", false, 1_000),
            ],
            day: 0,
        });
        assert_eq!((session_rows(&m), m.tasks.list.len()), ((1, vec!["s2".into()], 1), 1));
        // One that started below a listed repository is that repository's task: the tag names
        // it, and opening the task opens a terminal there. Where below is not kept.
        let mut below = session("claude", "s3", "/code/app/crates/core", true, 4_800);
        below.repo = "/code/app".into();
        m.update(Action::SetSessions { found: vec![below, session("codex", "s2", "/code/app", false, 1_000)], day: 0 });
        m.update(Action::SessionAdopt { runner: "claude".into(), id: "s3".into() });
        let task = m.tasks.list.last().unwrap().clone();
        assert_eq!(
            (task.title.as_str(), task.repo.as_deref(), m.task_tags(&task)),
            ("app", Some("/code/app"), ">app · @claude".into())
        );
        assert!(!serde_json::to_string(&task).unwrap().contains("crates"), "no path from the session is kept");
        assert_eq!(m.update(Action::NewTabOf { task: Some(task.id) }).len(), 1, "a terminal opens");
        assert_eq!(m.pane(1).and_then(|pane| pane.cwd.as_deref()), Some("/code/app"));
        m.update(Action::TaskSetState { id: task.id, state: State::Cleaned });
        // An idle one is saved for later. One that is not listed is not adopted.
        m.update(Action::SessionAdopt { runner: "codex".into(), id: "s2".into() });
        m.update(Action::SessionAdopt { runner: "codex".into(), id: "nope".into() });
        assert_eq!(
            (m.tasks.list.len() - 1, m.tasks.list[2].state, m.tasks.list[2].agent.as_deref()),
            (2, State::Captured, Some("codex"))
        );
    }

    #[test]
    fn an_adopted_task_follows_its_session() {
        const MINUTE: u64 = 60_000;
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        let start = 100 * MINUTE;
        // What the shell found at `now`, of a session whose file last changed `ago` before.
        let at = |m: &mut Model, now: u64, ago: u64| {
            m.update(Action::Tick { now }.sent_by(Origin::System));
            let found = vec![session("claude", "s1", "/code/app", ago <= 30_000, now - ago)];
            m.update(Action::SetSessions { found, day: 0 }.sent_by(Origin::System));
        };
        let state = |m: &Model| (m.tasks.list[0].state, m.tasks.list[0].session.as_ref().and_then(|link| link.state));
        at(&mut m, start, 0);
        m.update(Action::SessionAdopt { runner: "claude".into(), id: "s1".into() });
        assert_eq!(state(&m), (State::Running, Some(State::Running)));
        // Idle for a minute, as during a build that prints nothing: the task runs on.
        at(&mut m, start + MINUTE, MINUTE);
        assert_eq!((state(&m).0, m.tasks.needs_you()), (State::Running, 0));
        // Quiet for three minutes: the task needs you, once. It works again: the task runs.
        at(&mut m, start + 3 * MINUTE, 3 * MINUTE);
        assert_eq!((state(&m), m.tasks.needs_you()), ((State::NeedsYou, Some(State::NeedsYou)), 1));
        at(&mut m, start + 4 * MINUTE, 4 * MINUTE);
        at(&mut m, start + 5 * MINUTE, 0);
        assert_eq!(state(&m), (State::Running, Some(State::Running)));
        // The user puts it on hold: it is theirs now, whatever the session does.
        m.update(Action::TaskHold { id: 1 });
        at(&mut m, start + 9 * MINUTE, 4 * MINUTE);
        at(&mut m, start + 10 * MINUTE, 0);
        assert_eq!(state(&m), (State::Parked, Some(State::Running)));
        // Back to the state it took from the session (undo), it follows again.
        m.update(Action::TaskUndo);
        assert_eq!(m.followed_sessions(), [("claude".to_string(), "s1".to_string())]);
        at(&mut m, start + 14 * MINUTE, 4 * MINUTE);
        assert_eq!(state(&m).0, State::NeedsYou);
        // With the Tasks section closed the shell reads only when the file last changed: the same rule.
        let seen = |m: &mut Model, now: u64, ago: u64| {
            m.update(Action::Tick { now }.sent_by(Origin::System));
            let (runner, id) = ("claude".to_string(), "s1".to_string());
            let seen = vec![sessions::Seen { runner, id, last_activity: now - ago, active: ago <= 30_000 }];
            m.update(Action::SessionsSeen { seen }.sent_by(Origin::System));
        };
        seen(&mut m, start + 15 * MINUTE, 0);
        assert_eq!(state(&m), (State::Running, Some(State::Running)));
        seen(&mut m, start + 16 * MINUTE, MINUTE);
        assert_eq!(state(&m).0, State::Running);
        seen(&mut m, start + 19 * MINUTE, 4 * MINUTE);
        assert_eq!((state(&m).0, m.tasks.needs_you()), (State::NeedsYou, 1));
        // The setting: one minute is enough; 0 never moves it.
        seen(&mut m, start + 20 * MINUTE, 0);
        m.update(Action::SessionQuiet { ms: MINUTE }.sent_by(Origin::System));
        seen(&mut m, start + 21 * MINUTE, MINUTE);
        assert_eq!(state(&m).0, State::NeedsYou);
        seen(&mut m, start + 22 * MINUTE, 0);
        m.update(Action::SessionQuiet { ms: 0 }.sent_by(Origin::System));
        seen(&mut m, start + 99 * MINUTE, 70 * MINUTE);
        assert_eq!(state(&m).0, State::Running);
        m.update(Action::SessionQuiet { ms: sessions::QUIET_MS }.sent_by(Origin::System));
        // An agent's report is kept too, and such a task is no longer read for. A session that
        // is not found says nothing.
        m.update(Action::TaskSetState { id: 1, state: State::Done }.sent_by(Origin::Socket));
        assert_eq!(m.followed_sessions(), Vec::<(String, String)>::new());
        seen(&mut m, start + 100 * MINUTE, 0);
        m.update(Action::SetSessions { found: Vec::new(), day: 0 }.sent_by(Origin::System));
        m.update(Action::SessionsSeen { seen: Vec::new() }.sent_by(Origin::System));
        assert_eq!(state(&m).0, State::Done);
    }

    #[test]
    fn the_cap_on_sessions_listed_counts_those_that_can_be_listed() {
        let mut m = Model::default();
        m.update(Action::ToggleGroup { heading: Heading::Sessions }); // folded at first
        let found: Vec<Found> = (0..sessions::MAX_FOUND + 5)
            .map(|n| session("claude", &format!("s{n:04}"), "/code/app", true, 9))
            .collect();
        m.update(Action::SetSessions { found, day: 0 });
        assert_eq!(m.sessions_bar().count, sessions::MAX_FOUND);
        // Hiding some makes room for those behind them.
        for n in 0..5 {
            m.update(Action::SessionHide { runner: "claude".into(), id: format!("s{n:04}") });
        }
        let bar = m.sessions_bar();
        assert_eq!((bar.count, bar.rows.last().map(|row| row.id.as_str())), (sessions::MAX_FOUND, Some("s0204")));
    }

    #[test]
    fn a_click_on_a_session_focuses_its_pane_only_when_there_is_no_doubt() {
        let mut m = Model::default();
        m.update(Action::ToggleGroup { heading: Heading::Sessions }); // folded at first
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m); // pane 1, opened nowhere in particular
        m.update(Action::OpenRepo { path: "/code/app".into(), axis: Right }); // pane 2, in /code/app
        m.update(Action::Focus { pane: 1 });
        let click = |m: &mut Model, id: &str| m.update(Action::SessionClick { runner: "claude".into(), id: id.into() });
        m.update(Action::SetSessions {
            found: vec![
                session("claude", "here", "/code/app", true, 9),
                session("claude", "there", "/code/app-2", true, 8),
            ],
            day: 0,
        });
        assert_eq!(m.sessions_bar().rows.iter().map(|row| row.pane).collect::<Vec<_>>(), [Some(2), None]);
        click(&mut m, "here");
        assert_eq!((m.focused, m.mode()), (Some(2), Mode::Normal));
        // No pane to show: the row is selected, and nothing else happens to the session.
        m.update(Action::ToggleSection { section: Section::Tasks });
        assert_eq!(click(&mut m, "there"), []);
        assert_eq!((m.list, m.mode(), m.focused), (Some(2), Mode::List, Some(2)), "after the heading and the first");
        // Choosing it: its conversation in the drawer, no pane, no task. A client's opens nothing.
        m.update(Action::ListChoose { axis: Right }.sent_by(Origin::Socket));
        assert_eq!((m.list, m.panes.len(), m.talk.is_none()), (Some(2), 2, true), "a client's opens nothing");
        m.update(Action::ListChoose { axis: Right });
        let there = Subject::Session { runner: "claude".into(), id: "there".into() };
        assert_eq!((m.list, m.panes.len(), m.tasks.list.len()), (None, 2, 0));
        assert_eq!(m.talk.as_ref().map(|talk| &talk.task), Some(&there));
        // A click on one row, then on another: the selection is on the one clicked last.
        m.update(Action::SetSessions {
            found: vec![
                session("claude", "one", "/code/app-2", true, 9),
                session("claude", "two", "/code/app-2", true, 8),
            ],
            day: 0,
        });
        click(&mut m, "one");
        assert_eq!(m.list_rows()[m.list.unwrap()], ListRow::Session("claude".into(), "one".into()));
        click(&mut m, "two");
        assert_eq!(m.list_rows()[m.list.unwrap()], ListRow::Session("claude".into(), "two".into()));
        m.update(Action::LeaveList);
        // Two sessions at work started there, or two panes opened there: which is which is not known.
        m.update(Action::SetSessions {
            found: vec![session("claude", "here", "/code/app", true, 9), session("codex", "too", "/code/app", true, 8)],
            day: 0,
        });
        assert_eq!(m.sessions_bar().rows.iter().map(|row| row.pane).collect::<Vec<_>>(), [None, None]);
        // A past session in the same directory is no doubt: nor one hidden, nor one that is a task's.
        let found = || {
            vec![
                session("claude", "here", "/code/app", true, 9),
                session("codex", "past", "/code/app", false, 1),
                session("codex", "hidden", "/code/app", true, 8),
                session("codex", "adopted", "/code/app", true, 7),
            ]
        };
        m.update(Action::SetSessions { found: found(), day: 0 });
        m.update(Action::SessionHide { runner: "codex".into(), id: "hidden".into() });
        m.update(Action::SessionAdopt { runner: "codex".into(), id: "adopted".into() });
        let panes = |m: &Model| m.sessions_bar().rows.iter().map(|row| (row.id.clone(), row.pane)).collect::<Vec<_>>();
        assert_eq!(panes(&m), [("here".to_string(), Some(2)), ("past".to_string(), None)], "an idle one is in no pane");
        // The picker, the new-task box, the key reference and visual mode keep the focus where it
        // is: the user's click waits like any pane action, and a client's is refused with the reason.
        m.update(Action::Focus { pane: 1 });
        let holds: [(Action, Action); 4] = [
            (Action::OpenPicker { axis: Right, new_tab: false, items: Vec::new() }, Action::ClosePicker),
            (Action::OpenCompose { text: String::new() }, Action::CloseCompose),
            (Action::ToggleKeys, Action::CloseKeys),
            (Action::ToggleVisual, Action::LeaveVisual),
        ];
        for (hold, release) in holds {
            m.update(hold.clone());
            let click = Action::SessionClick { runner: "claude".into(), id: "here".into() };
            assert!(m.refusal(&click, Origin::Socket).is_some_and(|why| why.contains("has the keyboard")), "{hold:?}");
            for origin in [Origin::User, Origin::Socket] {
                assert_eq!(m.update(click.clone().sent_by(origin)), [], "{hold:?} {origin:?}");
                assert_eq!(
                    (m.focused, m.visual.is_none() || m.visual == Some(1)),
                    (Some(1), true),
                    "{hold:?} {origin:?}"
                );
            }
            m.update(release);
        }
        m.update(Action::SetSessions { found: vec![session("claude", "here", "/code/app", true, 9)], day: 0 });
        m.update(Action::OpenRepo { path: "/code/app".into(), axis: Right });
        assert_eq!(m.sessions_bar().rows[0].pane, None);
    }

    #[test]
    fn a_move_by_mouse_can_be_taken_back_for_a_few_seconds() {
        let mut m = Model::default();
        for (title, state) in [("a", State::NeedsYou), ("b", State::Done)] {
            m.update(Action::TaskAdd { title: title.into(), priority: 2, state, tab: None, assign: Assign::default() });
        }
        m.update(Action::Tick { now: 1_000 });
        m.update(Action::RowDismiss { id: 1 });
        let notice = Notice { moved: Moved::Dismissed, until: 1_000 + NOTICE_MS };
        assert_eq!((m.undo_offer, m.tasks.needs_you()), (Some(notice), 0));
        // It stays until its time is up; undo, as `u` does, takes the move back and the notice with it.
        m.update(Action::Tick { now: 1_000 + NOTICE_MS - 1 });
        assert_eq!(m.undo_offer, Some(notice));
        m.update(Action::TaskUndo);
        assert_eq!((m.undo_offer, m.tasks.needs_you()), (None, 1));
        // Time up: the notice goes, the move stays, and `u` still takes it back.
        m.update(Action::RowSnooze { id: 1, until: 99_000 });
        assert_eq!(m.undo_offer.map(|n| n.moved), Some(Moved::Snoozed));
        m.update(Action::Tick { now: 1_000 + 2 * NOTICE_MS });
        assert_eq!((m.undo_offer, m.tasks.needs_you()), (None, 0));
        m.update(Action::TaskUndo);
        assert_eq!(m.tasks.needs_you(), 1);
        // A move that changes nothing has nothing to offer: a finished task is not put on hold.
        m.update(Action::RowHold { id: 2 });
        m.update(Action::RowDismiss { id: 9 });
        assert_eq!(m.undo_offer, None);
        // The notice names one move. After another by key or socket, or once an agent's report
        // has made the move stale, it would take back something else: it goes.
        m.update(Action::RowHold { id: 1 });
        assert_eq!(m.undo_offer.map(|n| n.moved), Some(Moved::Held));
        m.update(Action::TaskDismiss { id: 2 });
        assert_eq!(m.undo_offer, None);
        m.update(Action::RowDismiss { id: 1 });
        m.update(Action::TaskSetState { id: 1, state: State::Running });
        assert_eq!(m.undo_offer, None);
    }

    #[test]
    fn a_move_that_changes_nothing_leaves_the_undo_alone() {
        let mut m = Model::default();
        for (title, state) in [("a", State::NeedsYou), ("b", State::Parked)] {
            m.update(Action::TaskAdd { title: title.into(), priority: 2, state, tab: None, assign: Assign::default() });
        }
        m.update(Action::Tick { now: 1_000 });
        // Dismiss "a", then, while the offer shows, hold "b", which is on hold already.
        m.update(Action::RowDismiss { id: 1 });
        m.update(Action::RowHold { id: 2 });
        assert_eq!(
            m.undo_offer.map(|offer| offer.moved),
            Some(Moved::Dismissed),
            "the offer names the move it takes back"
        );
        m.update(Action::TaskUndo);
        assert_eq!((m.tasks.get(1).map(|t| t.dismissed), m.undo_offer), (Some(false), None));
        // The same by key or socket, and for the other moves: a second dismiss, a snooze to the same time.
        m.update(Action::TaskSnooze { id: 1, until: 9_000 });
        m.update(Action::TaskSnooze { id: 1, until: 9_000 });
        m.update(Action::TaskHold { id: 2 });
        m.update(Action::TaskUndo);
        assert_eq!(m.tasks.get(1).and_then(|t| t.snoozed_until), None);
        m.update(Action::TaskDismiss { id: 2 });
        m.update(Action::TaskDismiss { id: 2 });
        m.update(Action::TaskUndo);
        assert_eq!(m.tasks.get(2).map(|t| (t.dismissed, t.state)), Some((false, State::Parked)));
        // `due`: a tick would change what shows.
        m.update(Action::RowDismiss { id: 1 });
        assert_eq!((m.due(1_000 + NOTICE_MS - 1), m.due(1_000 + NOTICE_MS)), (false, true));
        m.update(Action::TaskUndo);
        m.update(Action::TaskSnooze { id: 1, until: 5_000 });
        assert_eq!((m.due(4_999), m.due(5_000)), (false, true));
    }

    #[test]
    fn add_and_start_from_outside_does_not_change_tabs_under_a_box_or_a_mode() {
        let from = |origin| {
            Action::TaskFromLine { line: "theirs".into(), known: Default::default(), open: true }.sent_by(origin)
        };
        // Each of these holds the keyboard: the task is added, the tabs on show stay.
        let holds: [(&str, fn(&mut Model)); 4] = [
            ("the new-task box", |m| drop(m.update(Action::OpenCompose { text: "mine".into() }))),
            ("the picker", |m| drop(m.update(Action::OpenPicker { axis: Right, new_tab: false, items: Vec::new() }))),
            ("the key reference", |m| drop(m.update(Action::ToggleKeys))),
            ("visual mode", |m| drop(m.update(Action::ToggleVisual))),
        ];
        for (what, hold) in holds {
            let mut m = Model::default();
            term(&mut m);
            hold(&mut m);
            let (mode, tabs) = (m.mode(), m.tabs.len());
            assert!(mode != Mode::Normal || m.keys, "{what}");
            assert_eq!(m.update(from(Origin::Socket)), [], "{what}");
            assert_eq!((m.tasks.list.len(), m.tab_task(), m.mode(), m.tabs.len()), (1, None, mode, tabs), "{what}");
            assert_eq!(m.compose.as_ref().map(Field::text), (what == "the new-task box").then_some("mine"));
        }
        // Under sneak mode's labels too: the mode stays, and so do the tabs.
        let mut m = Model::default();
        sneak(&mut m, 3);
        m.update(from(Origin::Socket));
        assert_eq!((m.tasks.list.len(), m.tab_task(), m.mode()), (1, None, Mode::Sneak));
        // With nothing in the way the same: a client's task has no tab. The box's own ⌘↵ starts
        // it, and the box closes first.
        let mut m = Model::default();
        m.update(from(Origin::Socket));
        assert_eq!((m.tab_task(), m.tabs.len()), (None, 1));
        m.update(Action::OpenCompose { text: "mine".into() });
        m.update(Action::ComposeSubmit { open: true });
        let talk = m.talk.as_ref().and_then(|talk| talk.task.task());
        assert_eq!((m.tab_task(), talk, m.compose.as_ref().map(Field::text)), (None, Some(2), None));
    }

    #[test]
    fn a_task_talks_in_the_drawer_and_only_the_user_starts_its_agent() {
        const UUID: &str = "8f14e45f-ceea-467f-a8d3-0242ac130003";
        let system = |m: &mut Model, action: Action| m.update(action.sent_by(Origin::System));
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m);
        let names = compose::Names { agents: vec!["ana".into()], skills: Vec::new() };
        let known = compose::Known { names, ..Default::default() };
        for line in ["fix zoom >app @codex-gpt-6-astra", "plain", "ask @ana"] {
            m.update(Action::TaskFromLine { line: line.into(), known: known.clone(), open: false });
        }
        // A client starts nothing, and is told why.
        let theirs = Action::TaskTalk { id: 1 };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(agents::USERS_ALONE));
        assert_eq!((m.update(theirs.sent_by(Origin::Socket)), m.talk.clone()), (vec![], None));
        // The user's click: the conversation opens with the keyboard, the agent the tag names
        // is asked for in the task's repository, and the workspace stays as it is.
        let launch = |runner, model: Option<&str>, title: &str| {
            agents::Start::New(agents::Launch { runner, model: model.map(str::to_string), title: title.into() })
        };
        let asked = |task, start, cwd: Option<&str>| Effect::StartAgent {
            task: Subject::Task(task),
            start,
            cwd: cwd.map(str::to_string),
            brief: None,
        };
        let codex = launch(RunnerId::Codex, Some("gpt-6-astra"), "fix zoom");
        assert_eq!(m.update(Action::TaskTalk { id: 1 }), [asked(1, codex, Some("/code/app"))]);
        assert_eq!(m.talk.clone(), Some(Talk { task: Subject::Task(1), open: true, keys: true }));
        assert_eq!((m.mode(), m.tab_task(), m.tabs.len()), (Mode::Talk, Some(1), 1));
        // Started: the task runs. Codex names its own session, so there is none to follow.
        system(&mut m, Action::AgentStarted { task: Subject::Task(1), session: Some(UUID.into()), at: 0 });
        let task = m.tasks.get(1).unwrap();
        assert_eq!(
            (m.agents.clone(), task.state, task.session.is_none()),
            (vec![Subject::Task(1)], State::Running, true)
        );
        // A second click starts no second agent.
        assert_eq!(m.update(Action::TaskTalk { id: 1 }), []);
        // The keyboard goes back to the workspace and in again; closing to the rail takes it out.
        m.update(Action::TalkKeys { on: false });
        assert_eq!((m.mode(), m.talk.as_ref().map(|talk| talk.open)), (Mode::Normal, Some(true)));
        m.update(Action::TalkKeys { on: true });
        assert_eq!(m.mode(), Mode::Talk);
        m.update(Action::Focus { pane: 1 });
        assert_eq!(m.mode(), Mode::Normal, "a click on a pane takes the keyboard");
        m.update(Action::TalkToggle);
        assert_eq!(m.talk.clone(), Some(Talk { task: Subject::Task(1), open: false, keys: false }));
        m.update(Action::TalkToggle);
        assert_eq!(m.talk.clone(), Some(Talk { task: Subject::Task(1), open: true, keys: true }));
        // Stop asks the shell; the agent gone, the task needs the user.
        assert_eq!(m.update(Action::TaskStop { id: Subject::Task(1) }.sent_by(Origin::Socket)), []);
        assert_eq!(m.update(Action::TaskStop { id: Subject::Task(1) }), [Effect::StopAgent { task: Subject::Task(1) }]);
        system(&mut m, Action::AgentExited { task: Subject::Task(1) });
        assert_eq!((m.agents.len(), m.tasks.get(1).unwrap().state, m.mode()), (0, State::NeedsYou, Mode::Normal));
        assert_eq!(m.update(Action::TaskStop { id: Subject::Task(1) }), []);

        // No `@agent`: the default agent, in the home folder. Claude takes the session's id:
        // the task follows that session from the start.
        assert_eq!(m.update(Action::TaskTalk { id: 2 }), [asked(2, launch(RunnerId::Claude, None, "plain"), None)]);
        system(&mut m, Action::AgentStarted { task: Subject::Task(2), session: Some(UUID.into()), at: 0 });
        let link = sessions::Link { runner: "claude".into(), id: UUID.into(), state: Some(State::Running) };
        assert_eq!(m.tasks.get(2).unwrap().session, Some(link));
        assert_eq!(m.followed_sessions(), [("claude".to_string(), UUID.to_string())]);
        // Its terminal gone, a click goes on with that session: nothing of the task is sent again.
        system(&mut m, Action::AgentExited { task: Subject::Task(2) });
        let resume = agents::Start::Resume { runner: RunnerId::Claude, session: UUID.into() };
        let read = Effect::ReadSession { runner: "claude".into(), id: UUID.into() };
        assert_eq!(m.update(Action::TaskTalk { id: 2 }), [read]);
        let none = Action::SessionRead { runner: "claude".into(), id: UUID.into(), found: None };
        assert_eq!(system(&mut m, none), [asked(2, resume, None)]);
        // An agent the shell could not start is not running: the keyboard is not left in nothing.
        system(&mut m, Action::AgentExited { task: Subject::Task(2) });
        assert_eq!((m.agent_runs(&Subject::Task(2)), m.mode(), m.talk_note().is_some()), (false, Mode::Normal, false));

        // A saved agent names no runner: the drawer says so and starts nothing.
        assert_eq!(m.update(Action::TaskTalk { id: 3 }), []);
        assert_eq!(
            (m.talk.clone(), m.mode()),
            (Some(Talk { task: Subject::Task(3), open: true, keys: false }), Mode::Normal)
        );
        assert!(m.talk_note().is_some_and(|note| note.starts_with("@ana is a saved agent")));
        // The settings name the default agent.
        let codex = Action::SetAgents {
            claude: Vec::new(),
            codex: Vec::new(),
            default: Some("codex".into()),
            shows: View::Native,
        };
        system(&mut m, codex);
        m.update(Action::TaskFromLine { line: "other".into(), known: Default::default(), open: false });
        assert_eq!(m.update(Action::TaskTalk { id: 4 }), [asked(4, launch(RunnerId::Codex, None, "other"), None)]);
        // ↵ on a task's row is that click; the drawer's width stays within bounds.
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        let row = m.list_rows().iter().position(|row| *row == ListRow::Task(1)).unwrap();
        m.update(Action::ListMove { delta: row as i32 - m.list.unwrap() as i32 });
        let effects = m.update(Action::ListChoose { axis: Right });
        assert!(matches!(effects[..], [Effect::StartAgent { task: Subject::Task(1), .. }]), "{effects:?}");
        assert_eq!((m.talk.as_ref().and_then(|talk| talk.task.task()), m.list, m.tabs.len()), (Some(1), None, 1));
        m.update(Action::TalkResize { width: 9e9 });
        m.update(Action::TalkWide);
        assert_eq!((m.talk_width, m.talk_wide), (talk::MAX_WIDTH, true));
        // Started, it took the tab on show: the header offers no go.
        assert_eq!((m.tab_task(), m.talk_go()), (Some(1), None));
    }

    #[test]
    fn the_right_rail_lists_open_conversations_needs_you_then_running_then_the_last_opened() {
        let system = |m: &mut Model, action: Action| m.update(action.sent_by(Origin::System));
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        for i in 1..=12 {
            m.update(Action::TaskFromLine { line: format!("task {i} >app"), known: Default::default(), open: false });
        }
        let order = |m: &Model| m.rail().iter().filter_map(|cell| cell.task.task()).collect::<Vec<_>>();
        assert!(order(&m).is_empty(), "nothing opened: the rail has only its +");
        // Opened and started: 1, 2 and 3 run, in the order they started.
        for id in [1, 2, 3] {
            m.update(Action::TaskTalk { id });
            system(&mut m, Action::AgentStarted { task: Subject::Task(id), session: None, at: 0 });
        }
        assert_eq!(order(&m), [1, 2, 3]);
        // 2 needs the user: first. 3 ends while at work, so it needs the user too: the last
        // opened first.
        system(&mut m, Action::TaskSetState { id: 2, state: State::NeedsYou });
        system(&mut m, Action::AgentExited { task: Subject::Task(3) });
        assert_eq!(order(&m), [3, 2, 1]);
        // Opened and never started (a saved agent's, or one that failed): idle, the last opened first.
        for id in 4..=12 {
            m.update(Action::TaskTalk { id });
            system(&mut m, Action::AgentExited { task: Subject::Task(id) });
        }
        // At most eight: the oldest idle ones drop off; what needs the user and what runs stay.
        assert_eq!(order(&m), [3, 2, 1, 12, 11, 10, 9, 8]);
        let rail = m.rail();
        assert_eq!((rail[3].shown, rail[0].shown), (true, false));
        // `task 12` would be `T1`, as `task 1` is; `T2` is taken too: among every live task its place tells it apart.
        assert_eq!((rail[0].mono.as_str(), rail[2].mono.as_str(), rail[3].mono.as_str()), ("T3", "T1", "T12"));
        assert_eq!(rail[1].tip, "task 2\nNeeds you\napp · @claude");
        // A click on the shown cell closes the drawer; on another, that one shows. Only the user's.
        m.update(Action::TalkPick { id: Subject::Task(12) });
        assert_eq!(m.talk.clone(), Some(Talk { task: Subject::Task(12), open: false, keys: false }));
        assert!(m.rail().iter().all(|cell| !cell.shown));
        assert_eq!(m.refusal(&Action::TalkPick { id: Subject::Task(2) }, Origin::Socket), Some(agents::USERS_ALONE));
        m.update(Action::TalkPick { id: Subject::Task(2) });
        assert_eq!(m.talk.as_ref().map(|talk| (talk.task.task(), talk.open)), Some((Some(2), true)));
        // A task that goes is off the rail.
        m.update(Action::RestoreTasks { tasks: Vec::new(), floor: 12 });
        assert!(order(&m).is_empty());
    }

    #[test]
    fn two_codex_agents_in_one_repository_follow_two_sessions() {
        let system = |m: &mut Model, action: Action| m.update(action.sent_by(Origin::System));
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        let names = compose::Names { agents: vec!["codex".into()], skills: Vec::new() };
        let known = compose::Known { names, ..Default::default() };
        for line in ["one >app @codex", "two >app @codex", "mine >app", "late >app @codex"] {
            m.update(Action::TaskFromLine { line: line.into(), known: known.clone(), open: false });
        }
        for (task, at) in [(1, 1_000), (2, 2_000), (3, 3_000)] {
            m.update(Action::TaskTalk { id: task });
            system(&mut m, Action::AgentStarted { task: Subject::Task(task), session: None, at });
        }
        // Codex names its own sessions: they are looked for in the repository. Claude's is not.
        let app = (RunnerId::Codex, Some("/code/app".to_string()));
        assert_eq!(m.seeking(), [app.clone(), app]);
        let id = |n: u8| format!("8f14e45f-ceea-467f-a8d3-0242ac1300{n:02}");
        let started =
            |n, at| agents::Started { runner: RunnerId::Codex, id: id(n), cwd: "/code/app".into(), started: at };
        let found = vec![started(7, 2_500), started(8, 1_200)];
        system(&mut m, Action::AgentSessions { found: found.clone(), home: None, now: 3_000 });
        let link = |m: &Model, task| m.tasks.get(task).and_then(|task| task.session.as_ref()).map(|l| l.id.clone());
        assert_eq!((link(&m, 1), link(&m, 2), link(&m, 3)), (Some(id(8)), Some(id(7)), None));
        assert!(m.seeking().is_empty());
        // Read again, nothing changes; a task whose session is never found stops being looked for.
        system(&mut m, Action::AgentSessions { found, home: None, now: 4_000 });
        assert_eq!((link(&m, 1), link(&m, 2)), (Some(id(8)), Some(id(7))));
        m.update(Action::TaskTalk { id: 4 });
        system(&mut m, Action::AgentStarted { task: Subject::Task(4), session: None, at: 5_000 });
        system(&mut m, Action::AgentSessions { found: Vec::new(), home: None, now: 5_000 + agents::SEEK_MS + 1 });
        assert!(m.seeking().is_empty() && link(&m, 4).is_none());
        // An agent gone before its session was found takes none started after it.
        m.update(Action::TaskTalk { id: 3 });
        system(&mut m, Action::AgentStarted { task: Subject::Task(3), session: None, at: 70_000 });
        assert_eq!(m.seeking().len(), 0, "task 3 is Claude's: its id is its own");
        m.update(Action::TaskFromLine { line: "gone >app @codex".into(), known: known.clone(), open: false });
        m.update(Action::TaskTalk { id: 5 });
        system(&mut m, Action::AgentStarted { task: Subject::Task(5), session: None, at: 80_000 });
        system(&mut m, Action::AgentExited { task: Subject::Task(5) });
        system(&mut m, Action::AgentSessions { found: vec![started(9, 81_000)], home: None, now: 82_000 });
        assert!(m.seeking().is_empty() && link(&m, 5).is_none());
    }

    /// The first agent's session came after the second started: which is whose cannot be told.
    /// Once the first is no longer looked for, the second still does not take it.
    #[test]
    fn an_expired_codex_agent_keeps_a_later_one_from_a_session_that_may_be_its() {
        let system = |m: &mut Model, action: Action| m.update(action.sent_by(Origin::System));
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        let names = compose::Names { agents: vec!["codex".into()], skills: Vec::new() };
        let known = compose::Known { names, ..Default::default() };
        for (task, at) in [(1, 0), (2, 10_000)] {
            m.update(Action::TaskFromLine { line: format!("t{task} >app @codex"), known: known.clone(), open: false });
            m.update(Action::TaskTalk { id: task });
            system(&mut m, Action::AgentStarted { task: Subject::Task(task), session: None, at });
        }
        let id = "8f14e45f-ceea-467f-a8d3-0242ac130011";
        let found =
            vec![agents::Started { runner: RunnerId::Codex, id: id.into(), cwd: "/code/app".into(), started: 11_000 }];
        let link = |m: &Model, task| m.tasks.get(task).and_then(|task| task.session.as_ref()).map(|l| l.id.clone());
        for now in [12_000, agents::SEEK_MS + 2_000, agents::SEEK_MS + 9_000] {
            system(&mut m, Action::AgentSessions { found: found.clone(), home: None, now });
            assert_eq!((link(&m, 1), link(&m, 2), m.seeking().len()), (None, None, 2), "at {now}");
        }
        // The later one's minute is up too: both go, neither linked.
        system(&mut m, Action::AgentSessions { found, home: None, now: agents::SEEK_MS + 10_001 });
        assert_eq!((link(&m, 2), m.seeking().len()), (None, 0));
    }

    #[test]
    fn a_conversation_neither_clashes_with_a_session_at_work_nor_goes_on_with_one_never_written() {
        const UUID: &str = "8f14e45f-ceea-467f-a8d3-0242ac130003";
        let system = |m: &mut Model, action: Action| m.update(action.sent_by(Origin::System));
        let mut m = Model::default();
        m.update(Action::TaskFromLine { line: "mine".into(), known: Default::default(), open: false });
        // Adopted from a session at work in the user's own terminal: no second program on it.
        let link = sessions::Link { runner: "claude".into(), id: UUID.into(), state: Some(State::Running) };
        m.tasks.link(1, link);
        assert_eq!(m.update(Action::TaskTalk { id: 1 }), []);
        assert!(m.talk_note().is_some_and(|note| note.contains("live in another terminal")));
        // Ours: started, then gone. Its session never written: the next click starts a new one.
        m.tasks.unlink(1);
        assert!(matches!(m.update(Action::TaskTalk { id: 1 })[..], [Effect::StartAgent { .. }]));
        system(&mut m, Action::AgentStarted { task: Subject::Task(1), session: Some(UUID.into()), at: 0 });
        // Quitting asks while it runs, with no terminal running anything.
        m.update(Action::QuitAsked { running: Vec::new() });
        assert!(m.confirm.is_some());
        m.update(Action::QuitCancel);
        system(&mut m, Action::AgentExited { task: Subject::Task(1) });
        system(&mut m, Action::SessionMissing { task: 1, id: "another".into() });
        assert!(m.tasks.get(1).unwrap().session.is_some(), "another id: not this task's");
        system(&mut m, Action::SessionMissing { task: 1, id: UUID.into() });
        let effects = m.update(Action::TaskTalk { id: 1 });
        assert!(matches!(&effects[..], [Effect::StartAgent { start: agents::Start::New(_), .. }]), "{effects:?}");
        // The drawer shows one thing: opening a conversation closes the new task being written.
        m.update(Action::OpenCompose { text: "half".into() });
        m.update(Action::TaskTalk { id: 1 });
        assert_eq!((m.compose_host(), m.talk.as_ref().map(|talk| talk.open)), (None, Some(true)));
        // The line is kept: the box opened empty takes it back; one opened with text leaves it.
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!(m.compose_line().map(String::as_str), Some("half"));
        m.update(Action::TaskTalk { id: 1 });
        m.update(Action::OpenCompose { text: "other".into() });
        m.update(Action::ComposeEscape);
        m.update(Action::OpenCompose { text: String::new() });
        assert_eq!(m.compose_line().map(String::as_str), Some("half"));
        // A repository whose name no tag can hold is no pick.
        let repo =
            |name: &str| Repo { name: name.into(), path: format!("/code/{name}"), branch: None, worktrees: Vec::new() };
        m.update(Action::SetRepos { repos: vec![repo("2048-game"), repo("my app"), repo("app")] });
        assert_eq!(m.compose_picks().wheres, ["app"]);
    }

    #[test]
    fn enter_in_the_box_takes_a_row_or_adds_and_starts() {
        let mut m = composing();
        keys(&mut m, "fix #ca");
        // A highlighted row that would change the line: ↵ takes it.
        assert_eq!((m.update(Action::ComposeEnter), line(&m), m.tasks.list.len()), (vec![], "fix #canvas ", 4));
        // Nothing to take: the task is added and its agent started, with the keyboard.
        let effects = m.update(Action::ComposeEnter);
        assert!(matches!(effects[..], [Effect::StartAgent { task: Subject::Task(5), .. }]), "{effects:?}");
        assert_eq!(
            (m.tasks.list.len(), m.mode(), m.talk.as_ref().and_then(|talk| talk.task.task())),
            (5, Mode::Talk, Some(5))
        );
        // A client's ↵ files the task: no agent, and no tab.
        m.update(Action::OpenCompose { text: "theirs".into() }.sent_by(Origin::Socket));
        assert_eq!(m.update(Action::ComposeEnter.sent_by(Origin::Socket)), []);
        assert_eq!((m.tasks.list.len(), m.tab_task(), m.agent_runs(&Subject::Task(6))), (6, Some(5), false));
    }

    #[test]
    fn file_and_start_starts_its_agent_and_leaves_the_workspace() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        m.update(Action::OpenCompose { text: "fix zoom >app".into() });
        // Its agent, in the drawer, with the keyboard, in its repository. No tab opens.
        let launch = agents::Launch { runner: RunnerId::Claude, model: None, title: "fix zoom".into() };
        let start = agents::Start::New(launch);
        let agent = Effect::StartAgent { task: Subject::Task(1), start, cwd: Some("/code/app".into()), brief: None };
        assert_eq!(m.update(Action::ComposeSubmit { open: true }), [agent]);
        assert_eq!((m.compose.as_ref().map(Field::text), m.tab_task(), m.mode()), (None, None, Mode::Talk));
        assert_eq!((m.tabs.len(), m.layout.is_none()), (1, true));
        // No title: nothing is filed, nothing opens, the box stays.
        m.update(Action::OpenCompose { text: "p1".into() });
        assert_eq!(m.update(Action::ComposeSubmit { open: true }), []);
        assert_eq!((m.compose.as_ref().map(Field::text), m.tasks.list.len()), (Some("p1"), 1));
        m.update(Action::CloseCompose);
        // A client's "open": no agent, no tab, no terminal.
        let theirs = Action::TaskFromLine { line: "ask Ana >app".into(), known: Default::default(), open: true };
        assert_eq!(m.update(theirs.sent_by(Origin::Socket)), []);
        assert_eq!((m.tab_task(), m.tasks.list.len(), m.panes.len()), (None, 2, 0));
    }

    #[test]
    fn a_directory_is_added_to_the_repos_list_and_taken_off_it_by_the_user_alone() {
        let repo = |path: &str| Repo { name: repos::name_of(path), path: path.into(), branch: None, worktrees: vec![] };
        let dir = |text: &str, path: &str| repos::RepoDir { text: text.into(), path: path.into() };
        let change = |add: bool, text: &str| crate::settings::ListChange { key: "repos", add, text: text.into() };
        let notice = |text: &str| vec![Effect::Notice { text: text.into() }];
        let mut m = Model::default();
        m.update(Action::SetRepoDirs { dirs: repos::RepoDirs { dirs: vec![dir("~/code", "/code")], fixed: false } });
        m.update(Action::SetRepos { repos: listed() });
        let add = |dir: &str, found: Vec<Repo>| Action::RepoAdd { dir: dir.into(), found };
        // A client writes no settings: refused, and nothing changes.
        assert_eq!(m.refusal(&add("/src/zed", vec![repo("/src/zed")]), Origin::Socket), Some(apps::USERS_ALONE));
        // No repository there: said, and nothing is written.
        assert_eq!(m.update(add("/tmp", vec![])), notice("No repository in /tmp"));
        assert_eq!((m.repo_dirs.dirs.len(), m.list), (1, None));
        // A repository: written, listed, and its row selected in list mode.
        let saved = vec![Effect::SaveRepos { changes: vec![change(true, "/src/zed")] }];
        assert_eq!(m.update(add("/src/zed", vec![repo("/src/zed")])), saved);
        assert_eq!(m.repo_dirs.dirs[1], dir("/src/zed", "/src/zed"));
        assert_eq!((m.sidebar, m.mode(), m.list), (Some(Section::Repos), Mode::List, Some(2)));
        assert_eq!(m.list_rows()[2], ListRow::Repo("/src/zed".into()));
        // Listed already, as itself or through `~/code`: selected, nothing written.
        m.update(Action::LeaveList);
        assert_eq!((m.update(add("/src/zed", vec![repo("/src/zed")])), m.list), (vec![], Some(2)));
        assert_eq!((m.update(add("/code/app", vec![repo("/code/app")])), m.list), (vec![], Some(0)));
        assert_eq!(m.repo_dirs.dirs.len(), 2);
        // Remove: only a directory named as itself. One held by `~/code` is the user's to edit.
        let through = "app is listed through ~/code: take that directory out of `repos` in settings.json";
        assert_eq!(m.update(Action::RepoRemove { path: "/code/app".into() }), notice(through));
        let saved = vec![Effect::SaveRepos { changes: vec![change(false, "/src/zed")] }];
        assert_eq!(m.update(Action::RepoRemove { path: "/src/zed".into() }), saved);
        assert_eq!((m.repo_dirs.dirs.len(), m.repos.len(), m.repo_listed("/src/zed")), (1, 1, false));
    }

    #[test]
    fn a_task_is_stamped_with_the_time_last_told() {
        let mut m = Model::default();
        m.update(Action::TaskAdd {
            title: "a".into(),
            priority: 1,
            state: State::Captured,
            tab: None,
            assign: Assign::default(),
        });
        m.update(Action::Tick { now: 5_000 });
        m.update(Action::TaskFromLine { line: "b".into(), known: Default::default(), open: false });
        assert_eq!(m.tasks.list.iter().map(|t| t.created).collect::<Vec<_>>(), [None, Some(5_000)]);
    }

    fn listed() -> Vec<Repo> {
        let worktree = repos::Worktree { name: "app-2".into(), path: "/code/app-2".into(), branch: None };
        vec![Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![worktree] }]
    }

    #[test]
    fn opening_a_repo_starts_a_terminal_there_and_names_an_empty_tab() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        assert_eq!(m.update(Action::OpenRepo { path: "/code/app-2".into(), axis: Right }), [Effect::Opened(1)]);
        let pane = m.pane(1).unwrap();
        assert_eq!((&pane.kind, pane.cwd.as_deref()), (&Kind::Terminal, Some("/code/app-2")));
        assert_eq!(m.tabs[0].title, "app-2");
        // A second repo splits the tab, which keeps its name.
        assert_eq!(m.update(Action::OpenRepo { path: "/code/app".into(), axis: Down }), [Effect::Opened(2)]);
        assert_eq!((m.pane(2).unwrap().cwd.as_deref(), m.tabs[0].title.as_str()), (Some("/code/app"), "app-2"));
        assert_eq!(m.layout.as_ref().unwrap().neighbor(1, Dir::Down), Some(2));
    }

    #[test]
    fn a_path_that_is_not_listed_opens_nothing() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        assert_eq!(m.update(Action::OpenRepo { path: "/etc".into(), axis: Right }), []);
        assert!(m.panes.is_empty());
        // An empty pane takes the repo, as it takes anything opened.
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        assert_eq!(m.update(Action::OpenRepo { path: "/code/app".into(), axis: Right }), [Effect::Opened(1)]);
        assert_eq!((m.panes.len(), m.tabs[0].title.as_str()), (1, "Tab 1"));
    }

    fn term(m: &mut Model) {
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
    }

    #[test]
    fn each_tab_keeps_its_own_panes_and_focus() {
        let mut m = Model::default();
        assert_eq!((m.tabs.len(), m.active), (1, 0));
        term(&mut m);
        term(&mut m);
        m.update(Action::Focus { pane: 1 });
        m.update(Action::NewTab);
        assert_eq!((m.active, m.layout.is_none(), m.focused), (1, true, None));
        term(&mut m);
        assert_eq!(m.layout.as_ref().unwrap().leaves(), [3]);
        m.update(Action::SelectTab { index: 0 });
        assert_eq!((m.layout.as_ref().unwrap().leaves(), m.focused), (vec![1, 2], Some(1)));
        m.update(Action::CycleTab { delta: -1 });
        assert_eq!((m.active, m.focused), (1, Some(3)));
        assert_eq!(ids(&m), [1, 2, 3]);
    }

    #[test]
    fn switching_tabs_parks_the_live_app() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::NewTab), [Effect::Parked(1)]);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn closing_a_tab_closes_its_panes_and_keeps_one_tab() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        term(&mut m);
        // Close the background tab: the active tab is untouched.
        let effects = m.update(Action::CloseTab { index: Some(0) });
        assert_eq!(effects, [Effect::Closed(Pane { id: 1, kind: Kind::Terminal, cwd: None, fit: None })]);
        assert_eq!((m.tabs.len(), m.active, m.focused), (1, 0, Some(3)));
        // Close the last tab: its panes go, an empty tab remains.
        assert_eq!(m.update(Action::CloseTab { index: None }).len(), 2);
        assert_eq!((m.tabs.len(), m.panes.len(), m.layout.is_none()), (1, 0, true));
        assert_eq!(m.tabs[0].title, "Tab 1");
    }

    #[test]
    fn closing_a_pane_moves_focus_within_its_tab() {
        let mut m = Model::default();
        term(&mut m);
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::Close { pane: Some(2) });
        assert_eq!(m.focused, Some(1)); // not pane 3, which is in the other tab
        // A pane closed in a background tab leaves that tab's tree.
        m.update(Action::Close { pane: Some(3) });
        m.update(Action::SelectTab { index: 1 });
        assert_eq!((m.layout.is_none(), m.focused), (true, None));
    }

    #[test]
    fn focusing_a_pane_in_a_background_tab_brings_that_tab_forward() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        m.update(Action::Focus { pane: 1 });
        assert_eq!((m.active, m.focused), (0, Some(1)));
    }

    fn shares(m: &Model) -> Vec<(PaneId, f32)> {
        let unit = crate::layout::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
        let rects = m.layout.as_ref().map(|l| l.rects(unit)).unwrap_or_default();
        rects.into_iter().map(|(id, r)| (id, (r.w * 100.0).round())).collect()
    }

    #[test]
    fn resize_and_equalize_act_on_the_focused_panes_splits() {
        let mut m = Model::default();
        term(&mut m);
        term(&mut m);
        m.update(Action::Resize { dir: Dir::Left });
        assert_eq!(shares(&m), [(1, 45.0), (2, 55.0)]);
        // Behind the key reference the layout waits, as other pane actions do.
        m.update(Action::ToggleKeys);
        m.update(Action::Resize { dir: Dir::Left });
        m.update(Action::ToggleKeys);
        assert_eq!(shares(&m), [(1, 45.0), (2, 55.0)]);
        m.update(Action::Equalize);
        assert_eq!(shares(&m), [(1, 50.0), (2, 50.0)]);
    }

    #[test]
    fn fit_moves_the_splits_so_the_pane_has_its_windows_size_only_on_request() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        let hint = |m: &Model| m.pane(2).and_then(|p| p.fit.as_ref()).map(|f| f.hint.clone());
        // In a tab 1000 pt wide, pane 2 is 500 pt and shows 494. Its window kept 614 by 900.
        let wants = Action::PaneWants { pane: 2, content: [494., 766.], outer: [500., 800.], window: [614., 900.] };
        m.update(wants);
        assert_eq!(hint(&m).as_deref(), Some("needs 120×134 pt more"));
        assert_eq!(shares(&m), [(1, 50.0), (2, 50.0)], "the report alone moves nothing");
        // Fit: 120 pt more of 1000. No split is above the pane on the other axis: the height stays.
        m.update(Action::Focus { pane: 1 });
        m.update(Action::Fit { pane: Some(2) });
        assert_eq!(shares(&m), [(1, 38.0), (2, 62.0)]);
        // The shell measures again: the width fits now, the height still does not.
        m.update(Action::PaneWants { pane: 2, content: [614., 766.], outer: [620., 800.], window: [614., 900.] });
        assert_eq!(hint(&m).as_deref(), Some("needs 134 pt more height"));
        m.update(Action::Fit { pane: Some(2) });
        assert_eq!(shares(&m), [(1, 38.0), (2, 62.0)]);
        // The window takes the pane's size after all: the hint goes.
        m.update(Action::PaneWants { pane: 2, content: [614., 766.], outer: [620., 800.], window: [614., 766.] });
        assert_eq!(hint(&m), None);
        // A pane that fits, and the focused pane by default.
        m.update(Action::Fit { pane: None });
        assert_eq!(shares(&m), [(1, 38.0), (2, 62.0)]);
        m.update(Action::PaneWants { pane: 1, content: [374., 766.], outer: [380., 800.], window: [232., 320.] });
        m.update(Action::Fit { pane: None });
        assert_eq!(shares(&m), [(1, 24.0), (2, 76.0)], "232 + 6 of 1000");
    }

    #[test]
    fn a_second_fit_waits_for_a_fresh_measure() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        let wants = || Action::PaneWants { pane: 2, content: [494., 766.], outer: [500., 766.], window: [614., 766.] };
        m.update(wants());
        // A double-click on `fit`, ⌘⌃0 held down, two socket calls: the second does nothing.
        m.update(Action::Fit { pane: Some(2) });
        m.update(Action::Fit { pane: Some(2) });
        m.update(Action::Fit { pane: Some(2) });
        assert_eq!(shares(&m), [(1, 38.0), (2, 62.0)]);
        assert_eq!(m.pane(2).and_then(|p| p.fit.as_ref()), None, "the measure went with the layout it was of");
        // Whatever else moves a split or a pane drops the measures too.
        let moves = [
            Action::Resize { dir: Dir::Left },
            Action::Equalize,
            Action::SwapDir { dir: Dir::Left },
            Action::Split { axis: Right, kind: Kind::Empty },
            Action::MovePane { pane: Some(1), tab: 1 },
        ];
        m.update(Action::NewTab);
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::Focus { pane: 2 });
        for action in moves {
            m.update(wants());
            assert!(m.pane(2).is_some_and(|p| p.fit.is_some()));
            let name = format!("{action:?}");
            m.update(action);
            assert_eq!(m.pane(2).and_then(|p| p.fit.as_ref()), None, "{name}");
        }
        // A change that moves nothing keeps it: focus, another tab.
        m.update(Action::SelectTab { index: 0 });
        m.update(wants());
        m.update(Action::Focus { pane: 2 });
        m.update(Action::SelectTab { index: 1 });
        assert!(m.pane(2).is_some_and(|p| p.fit.is_some()));
    }

    #[test]
    fn a_fit_that_would_move_nothing_is_marked_and_refused_with_the_reason() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        let small = |pane| Action::PaneWants { pane, content: [494., 766.], outer: [500., 800.], window: [160., 120.] };
        let stuck = |m: &Model, id| m.pane(id).and_then(|p| p.fit.as_ref()).and_then(|f| f.stuck);
        // A single pane: no split on either axis.
        m.update(small(1));
        assert_eq!(stuck(&m, 1), Some(Stuck::NoSplit));
        assert_eq!(m.fit_refusal(Some(1)).as_deref(), Some("pane 1 cannot be fitted: no split to move"));
        assert_eq!(m.fit_refusal(None), m.fit_refusal(Some(1)), "the focused pane by default");
        // Beside another pane its width can change; its height still spans the tab.
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        m.update(small(1));
        assert_eq!((stuck(&m, 1), m.fit_refusal(Some(1))), (None, None));
        // At the split's limit: fit to 10%, measured again, still too large for the window.
        m.update(Action::Fit { pane: Some(1) });
        assert_eq!(shares(&m), [(1, 17.0), (2, 83.0)]);
        m.update(Action::PaneWants { pane: 1, content: [94., 766.], outer: [100., 800.], window: [20., 766.] });
        m.update(Action::Fit { pane: Some(1) });
        assert_eq!(shares(&m), [(1, 10.0), (2, 90.0)]);
        m.update(Action::PaneWants { pane: 1, content: [94., 766.], outer: [100., 800.], window: [20., 766.] });
        assert_eq!(stuck(&m, 1), Some(Stuck::AtLimit));
        assert_eq!(m.fit_refusal(Some(1)).as_deref(), Some("pane 1 cannot be fitted: splits at their limit"));
        m.update(Action::Fit { pane: Some(1) });
        assert_eq!(shares(&m), [(1, 10.0), (2, 90.0)]);
        // The mark follows the layout: the other pane closes, and no split is left.
        m.update(Action::Close { pane: Some(2) });
        m.update(small(1));
        assert_eq!(stuck(&m, 1), Some(Stuck::NoSplit));
        // Panes that are not for fitting, and a pane with nothing to fit.
        assert_eq!(m.fit_refusal(Some(9)).as_deref(), Some("no pane 9"));
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let terminal = m.focused.unwrap();
        assert!(m.fit_refusal(Some(terminal)).is_some_and(|why| why.contains("is a terminal")));
        assert_eq!(m.fit_refusal(Some(1)), None, "not measured since the split");
        assert_eq!(Model::default().fit_refusal(None).as_deref(), Some("no pane to fit"));
    }

    #[test]
    fn fit_reaches_a_pane_in_a_background_tab_and_the_shell_reports_only_a_new_measure() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        let measure = ([494., 766.], [500., 800.], [614., 766.]);
        assert!(m.wants_changed(2, measure.0, measure.1, measure.2));
        assert!(!m.wants_changed(1, measure.0, measure.1, [494., 766.]), "a window that fits is no news");
        m.update(Action::PaneWants { pane: 2, content: measure.0, outer: measure.1, window: measure.2 });
        assert!(!m.wants_changed(2, measure.0, measure.1, measure.2));
        assert!(m.wants_changed(2, measure.0, measure.1, [494., 766.]), "it fits now: the hint must go");
        assert!(!m.wants_changed(9, measure.0, measure.1, measure.2), "no such pane");
        // Another tab comes forward; the pane is fitted where it is, and its tab stays behind.
        m.update(Action::NewTab);
        assert_eq!(m.active, 1);
        assert_eq!(m.fit_refusal(Some(2)), None);
        m.update(Action::Fit { pane: Some(2) });
        assert_eq!(m.active, 1);
        m.update(Action::SelectTab { index: 0 });
        assert_eq!(shares(&m), [(1, 38.0), (2, 62.0)]);
    }

    #[test]
    fn swap_trades_places_with_the_neighbour_and_keeps_focus_and_the_keyboard() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        term(&mut m);
        m.update(Action::Enter { pane: 1 });
        // The live app pane moves right; it stays live, so the shell re-places its window.
        assert_eq!(m.update(Action::SwapDir { dir: Dir::Right }), []);
        assert_eq!(m.layout.as_ref().unwrap().leaves(), [2, 1]);
        assert_eq!((m.focused, m.live), (Some(1), Some(1)));
        // No neighbour on that side: nothing moves.
        m.update(Action::SwapDir { dir: Dir::Right });
        assert_eq!(m.layout.as_ref().unwrap().leaves(), [2, 1]);
    }

    #[test]
    fn a_pane_moves_to_another_tab_on_the_bar_and_takes_focus_there() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::Enter { pane: 1 });
        // The live window is parked before its pane leaves the tab on show.
        assert_eq!(m.update(Action::MovePane { pane: None, tab: 1 }), [Effect::Parked(1)]);
        assert_eq!((m.active, m.focused, m.live), (1, Some(1), None));
        assert_eq!(m.tab_panes(), [vec![2], vec![3, 1]]);
        // Its own tab, or a place with no tab: nothing happens.
        m.update(Action::MovePane { pane: None, tab: 1 });
        m.update(Action::MovePane { pane: None, tab: 7 });
        assert_eq!(m.tab_panes(), [vec![2], vec![3, 1]]);
        // A named pane in a background tab comes to the tab on show. The tab it leaves keeps
        // its place on the bar, empty, as when its last pane is closed.
        m.update(Action::MovePane { pane: Some(2), tab: 1 });
        assert_eq!((m.tabs.len(), m.active, m.focused), (2, 1, Some(2)));
        assert_eq!(m.tab_panes(), [vec![], vec![3, 1, 2]]);
        m.update(Action::SelectTab { index: 0 });
        assert_eq!((m.layout.is_none(), m.focused), (true, None));
        // A pane that arrives beside a live app takes focus, so the app gives the keyboard back.
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::NewTab);
        term(&mut m);
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::MovePane { pane: Some(4), tab: 1 }), [Effect::Parked(1)]);
        assert_eq!((m.active, m.focused, m.live), (1, Some(4), None));
    }

    #[test]
    fn a_pane_dropped_on_an_edge_splits_the_target_on_that_side() {
        // 1 | 2 | 3
        let mut m = Model::default();
        term(&mut m);
        term(&mut m);
        term(&mut m);
        let tree = |m: &Model| serde_json::to_value(&m.layout).unwrap();
        let leaf = |id: u32| serde_json::json!({ "leaf": id });
        let split = |axis: &str, a, b| serde_json::json!({ "split": { "axis": axis, "ratio": 0.5, "a": a, "b": b } });
        // 3 on the top of 1: 3 over 1, then 2.
        m.update(Action::DropPane { source: 3, target: 1, zone: Zone::Top });
        assert_eq!(tree(&m), split("right", split("down", leaf(3), leaf(1)), leaf(2)));
        assert_eq!(m.focused, Some(3));
        // 1 on the right of 2.
        m.update(Action::DropPane { source: 1, target: 2, zone: Zone::Right });
        assert_eq!(tree(&m), split("right", leaf(3), split("right", leaf(2), leaf(1))));
        m.update(Action::DropPane { source: 1, target: 3, zone: Zone::Left });
        m.update(Action::DropPane { source: 2, target: 3, zone: Zone::Bottom });
        assert_eq!(tree(&m), split("right", leaf(1), split("down", leaf(3), leaf(2))));
        // On itself, or with a pane that is gone: nothing moves.
        m.update(Action::DropPane { source: 2, target: 2, zone: Zone::Left });
        m.update(Action::DropPane { source: 2, target: 9, zone: Zone::Left });
        m.update(Action::DropPane { source: 9, target: 2, zone: Zone::Center });
        assert_eq!(tree(&m), split("right", leaf(1), split("down", leaf(3), leaf(2))));
        assert_eq!(ids(&m), [1, 2, 3]);
    }

    #[test]
    fn a_pane_dropped_where_it_already_is_keeps_the_splits_size() {
        let mut m = Model::default();
        term(&mut m);
        term(&mut m);
        m.update(Action::Resize { dir: Dir::Left });
        // 2 is already the right half of the 1|2 split, and 1 the left: only focus moves.
        m.update(Action::DropPane { source: 2, target: 1, zone: Zone::Right });
        m.update(Action::DropPane { source: 1, target: 2, zone: Zone::Left });
        assert_eq!((shares(&m), m.focused), (vec![(1, 45.0), (2, 55.0)], Some(1)));
        // The other side is a real move: a new split, at equal halves.
        m.update(Action::DropPane { source: 2, target: 1, zone: Zone::Left });
        assert_eq!(shares(&m), [(2, 50.0), (1, 50.0)]);
    }

    #[test]
    fn a_drop_on_an_empty_pane_splits_or_swaps_it_and_leaves_it_empty() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        // An edge of the empty pane 1: 3 lands beside it. It is not put into it, as `Open` would.
        m.update(Action::DropPane { source: 3, target: 1, zone: Zone::Bottom });
        assert_eq!(m.tab_panes(), [vec![1, 3, 2], vec![]]);
        assert_eq!((m.pane(1).map(|p| &p.kind), m.focused), (Some(&Kind::Empty), Some(3)));
        // The centre: the empty pane trades places like any other, and can be the one dragged.
        m.update(Action::DropPane { source: 2, target: 1, zone: Zone::Center });
        m.update(Action::DropPane { source: 1, target: 3, zone: Zone::Left });
        assert_eq!((m.tab_panes()[0].clone(), m.focused), (vec![2, 1, 3], Some(1)));
        assert_eq!(ids(&m), [1, 2, 3]);
    }

    #[test]
    fn a_drop_leaves_the_home_screen_and_a_tab_it_empties_shows_it() {
        let mut m = Model::default();
        term(&mut m);
        term(&mut m);
        m.update(Action::NewTab);
        // The tab on show has no panes: the home screen. A drop between panes of the other tab
        // (the socket can name them) brings that tab forward.
        assert!(m.home(&[]).is_some());
        m.update(Action::DropPane { source: 2, target: 1, zone: Zone::Top });
        assert_eq!((m.active, m.focused, m.home(&[]).is_none()), (0, Some(2), true));
        // The only pane of a tab dropped in another tab: the view follows the pane, and the tab
        // it left is the home screen when it is shown again.
        m.update(Action::SelectTab { index: 1 });
        term(&mut m);
        m.update(Action::DropPane { source: 3, target: 1, zone: Zone::Right });
        assert_eq!((m.tab_panes(), m.active), (vec![vec![2, 1, 3], vec![]], 0));
        assert!(m.home(&[]).is_none());
        m.update(Action::SelectTab { index: 1 });
        assert!(m.home(&[]).is_some());
    }

    #[test]
    fn a_pane_dropped_on_the_centre_swaps_and_a_live_app_stays_live() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        term(&mut m);
        m.update(Action::Enter { pane: 1 });
        // The dragged pane is live: it stays so, and the shell re-places its window.
        assert_eq!(m.update(Action::DropPane { source: 1, target: 2, zone: Zone::Center }), []);
        assert_eq!(m.layout.as_ref().unwrap().leaves(), [2, 1]);
        assert_eq!((m.focused, m.live), (Some(1), Some(1)));
        // Another pane dropped beside it takes focus, so the app gives the keyboard back.
        assert_eq!(m.update(Action::DropPane { source: 2, target: 1, zone: Zone::Bottom }), [Effect::Parked(1)]);
        assert_eq!((m.focused, m.live), (Some(2), None));
    }

    #[test]
    fn a_drop_reaches_across_tabs_and_brings_the_panes_tab_forward() {
        let mut m = Model::default();
        term(&mut m);
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        // 1, focused in the background tab, swaps with 3 of the tab on show: 1 comes to this
        // tab, and the other tab focuses 3.
        m.update(Action::Focus { pane: 1 });
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::DropPane { source: 1, target: 3, zone: Zone::Center });
        assert_eq!(m.tab_panes(), [vec![3, 2], vec![1]]);
        assert_eq!((m.active, m.focused), (1, Some(1)));
        m.update(Action::SelectTab { index: 0 });
        assert_eq!(m.focused, Some(3));
        // 1 lands beside 2 in the tab on show; the tab it left is empty and stays.
        m.update(Action::DropPane { source: 1, target: 2, zone: Zone::Left });
        assert_eq!(m.tab_panes(), [vec![3, 1, 2], vec![]]);
        assert_eq!((m.active, m.focused, m.tabs.len()), (0, Some(1), 2));
    }

    #[test]
    fn a_pane_moves_to_a_tab_by_its_place_on_the_whole_bar() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::TaskAdd {
            title: "fix".into(),
            priority: 3,
            state: State::Captured,
            tab: None,
            assign: Assign::default(),
        });
        let task = m.tasks.list[0].id;
        m.update(Action::NewTabOf { task: Some(task) });
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        // Place 0 on the bar is the task's first tab: the task's tabs come first.
        m.update(Action::MovePane { pane: None, tab: 0 });
        assert_eq!(m.tab_panes(), [vec![2, 3], vec![], vec![1]]);
        assert_eq!((m.active, m.tab_task()), (0, Some(task)));
    }

    #[test]
    fn adopt_joins_the_tab_of_its_sibling() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::NewTab);
        term(&mut m);
        // DevTools appears for the app in the background tab.
        m.update(Action::Adopt { kind: app(9), beside: Some(1) });
        assert_eq!(m.layout.as_ref().unwrap().leaves(), [2]);
        m.update(Action::SelectTab { index: 0 });
        assert_eq!(m.layout.as_ref().unwrap().leaves(), [1, 3]);
    }

    #[test]
    fn picking_for_a_new_tab_opens_the_pick_there() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::OpenPicker { axis: Right, new_tab: true, items: vec![PickItem::Terminal] });
        // Closing the picker makes no tab.
        m.update(Action::ClosePicker);
        assert_eq!(m.tabs.len(), 1);
        m.update(Action::OpenPicker { axis: Right, new_tab: true, items: vec![PickItem::Terminal] });
        assert_eq!(m.update(Action::PickerChoose { index: None }), [Effect::Opened(2)]);
        assert_eq!((m.tabs.len(), m.active, m.layout.as_ref().unwrap().leaves()), (2, 1, vec![2]));
    }

    #[test]
    fn entering_a_background_app_pane_shows_its_tab() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::NewTab);
        term(&mut m);
        assert_eq!(m.update(Action::Enter { pane: 1 }), [Effect::Live(1)]);
        assert_eq!((m.active, m.focused, m.live), (0, Some(1), Some(1)));
        assert_eq!(m.tab_panes(), [vec![1], vec![2]]);
        // Active tab or background tab: each pane names its own tab.
        assert_eq!((m.tab_of(1), m.tab_of(2), m.tab_of(9)), (Some(m.tabs[0].id), Some(m.tabs[1].id), None));
    }

    #[test]
    fn picking_an_app_for_a_new_tab_leaves_the_tab_to_the_shell() {
        let mut m = Model::default();
        term(&mut m);
        let items = vec![PickItem::App { name: "Slack".into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: true, items });
        let effects = m.update(Action::PickerChoose { index: None });
        assert_eq!(effects, [Effect::HostApp { name: "Slack".into(), axis: Right, new_tab: true }]);
        // No tab yet: if hosting fails, nothing is left behind.
        assert_eq!(m.tabs.len(), 1);
    }

    #[test]
    fn the_key_reference_toggles_and_takes_the_keyboard_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::ToggleKeys), [Effect::Parked(1)]);
        assert!(m.keys);
        m.update(Action::ToggleKeys);
        assert!(!m.keys);
        m.update(Action::ToggleKeys);
        m.update(Action::CloseKeys);
        assert!(!m.keys);
    }

    #[test]
    fn the_ax_overlay_toggles_and_parks_the_live_pane() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::ToggleAxOverlay), [Effect::Parked(1)]);
        assert!(m.ax_overlay);
        assert_eq!(m.update(Action::ToggleAxOverlay), []);
        assert!(!m.ax_overlay);
    }

    #[test]
    fn pane_actions_and_the_picker_wait_while_the_key_reference_is_open() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        picker(&mut m);
        m.update(Action::ToggleKeys);
        assert!(m.picker.is_none(), "opening the reference closes the picker");
        for action in [
            Action::Close { pane: None },
            Action::Split { axis: Right, kind: Kind::Terminal },
            Action::NewTab,
            Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] },
        ] {
            assert_eq!(m.update(action), []);
        }
        assert_eq!((ids(&m), m.tabs.len(), m.picker.is_none()), (vec![1], 1, true));
    }

    #[test]
    fn copy_and_paste_act_on_the_focused_terminal_only_while_ottospace_has_the_keyboard() {
        let mut m = Model::default();
        assert_eq!(m.keyboard_terminal(), None);
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!(m.keyboard_terminal(), Some(1));
        m.update(Action::ToggleKeys);
        assert_eq!(m.keyboard_terminal(), None, "behind the key reference");
        m.update(Action::CloseKeys);
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        assert_eq!(m.keyboard_terminal(), None, "the picker has the keys");
        m.update(Action::ClosePicker);
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        assert_eq!(m.keyboard_terminal(), None, "an app pane is focused");
        m.update(Action::Focus { pane: 1 });
        assert_eq!(m.keyboard_terminal(), Some(1));
    }

    #[test]
    fn visual_mode_starts_on_the_focused_terminal_and_holds_pane_actions() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        assert_eq!(m.update(Action::ToggleVisual), [], "not a terminal");
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!(m.update(Action::ToggleVisual), [Effect::Visual { pane: 2, on: true }]);
        assert_eq!((m.mode(), m.keyboard_terminal()), (Mode::Visual, None));
        for action in [
            Action::Focus { pane: 1 },
            Action::Close { pane: None },
            Action::NewTab,
            Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] },
        ] {
            assert_eq!(m.update(action), []);
        }
        assert_eq!((m.focused, m.picker.is_none(), m.tabs.len()), (Some(2), true, 1));
        assert_eq!(m.update(Action::ToggleVisual), [Effect::Visual { pane: 2, on: false }]);
        assert_eq!(m.mode(), Mode::Normal);
        assert_eq!(m.update(Action::LeaveVisual), []);
    }

    #[test]
    fn opening_a_pane_or_closing_a_named_one_ends_or_passes_visual_mode() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::ToggleVisual);
        // e.g. a hosted window closed: its pane goes, visual mode stays on its own terminal.
        assert_eq!(
            m.update(Action::Close { pane: Some(1) }),
            [Effect::Closed(Pane { id: 1, kind: Kind::Terminal, cwd: None, fit: None })]
        );
        assert_eq!(m.mode(), Mode::Visual);
        // The visual terminal's shell exited.
        m.update(Action::Close { pane: Some(2) });
        assert_eq!((m.mode(), m.visual), (Mode::Normal, None));
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::ToggleVisual);
        let effects = m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        assert_eq!(effects, [Effect::Visual { pane: 3, on: false }, Effect::Opened(4)]);
        assert_eq!((m.mode(), m.focused), (Mode::Normal, Some(4)));
    }

    #[test]
    fn visual_mode_waits_behind_the_key_reference() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::ToggleKeys);
        assert_eq!(m.update(Action::ToggleVisual), []);
        m.update(Action::CloseKeys);
        m.update(Action::ToggleVisual);
        assert_eq!(m.update(Action::LeaveVisual), [Effect::Visual { pane: 1, on: false }]);
    }

    #[test]
    fn closing_the_live_pane_returns_to_normal_mode() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        m.update(Action::Close { pane: Some(1) });
        assert_eq!(m.mode(), Mode::Normal);
    }

    fn running(pane: PaneId, program: &str) -> Running {
        Running { pane, program: Some(program.into()), background: false }
    }

    #[test]
    fn quit_is_at_once_when_nothing_runs() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!(m.update(Action::QuitAsked { running: vec![] }), [Effect::Quit]);
        assert_eq!((m.mode(), &m.confirm), (Mode::Normal, &None));
    }

    #[test]
    fn quit_asks_while_a_terminal_runs_and_enter_confirms() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!(m.update(Action::QuitConfirm), [], "no question, no answer");
        assert_eq!(m.update(Action::QuitAsked { running: vec![running(1, "sleep")] }), []);
        assert_eq!(m.mode(), Mode::Confirm);
        assert_eq!(m.confirm, Some(Confirm { running: vec![running(1, "sleep")], dismiss: None }));
        assert_eq!(m.update(Action::QuitConfirm), [Effect::Quit]);
    }

    #[test]
    fn a_second_quit_key_confirms() {
        let mut m = Model::default();
        m.update(Action::QuitAsked { running: vec![running(1, "sleep")] });
        // The program may have ended since: the user has answered all the same.
        assert_eq!(m.update(Action::QuitAsked { running: vec![] }), [Effect::Quit]);
    }

    #[test]
    fn the_quit_question_is_cancelled_by_anything_else_the_user_does() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let ask = || Action::QuitAsked { running: vec![running(1, "sleep")] };
        for no in [Action::QuitCancel, Action::Close { pane: None }, Action::ToggleKeys, Action::NewTab] {
            m.update(ask());
            assert_eq!(m.update(no.clone()), [], "{no:?}");
            assert_eq!((m.mode(), ids(&m), m.keys, m.tabs.len()), (Mode::Normal, vec![1, 2], false, 1), "{no:?}");
        }
        // What the user did not do leaves the question up: time passes, a shell exits.
        m.update(ask());
        m.update(Action::Tick { now: 1 });
        assert_eq!(m.update(Action::Close { pane: Some(2) }).len(), 1);
        assert_eq!((m.mode(), ids(&m)), (Mode::Confirm, vec![1]));
    }

    #[test]
    fn under_the_quit_question_a_measure_is_taken_and_the_users_fit_answers_no() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        m.update(Action::QuitAsked { running: vec![running(1, "sleep")] });
        // The shell's measure is nobody's answer: it lands, and the question stays.
        let wants = Action::PaneWants { pane: 2, content: [494., 766.], outer: [500., 766.], window: [614., 766.] };
        m.update(wants.sent_by(Origin::System));
        assert!(m.pane(2).is_some_and(|p| p.fit.is_some()));
        assert_eq!(m.mode(), Mode::Confirm);
        // A client's fit runs underneath; the user's is an answer, and moves nothing.
        m.update(Action::Fit { pane: Some(2) }.sent_by(Origin::Socket));
        assert_eq!((m.mode(), shares(&m)), (Mode::Confirm, vec![(1, 38.0), (2, 62.0)]));
        m.update(Action::Equalize.sent_by(Origin::Socket));
        m.update(
            Action::PaneWants { pane: 2, content: [494., 766.], outer: [500., 766.], window: [614., 766.] }
                .sent_by(Origin::System),
        );
        m.update(Action::Fit { pane: Some(2) });
        assert_eq!((m.mode(), shares(&m)), (Mode::Normal, vec![(1, 50.0), (2, 50.0)]));
    }

    #[test]
    fn a_pane_that_closes_leaves_the_quit_question_and_the_last_one_ends_it() {
        let mut m = Model::default();
        for _ in 0..4 {
            m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        }
        m.update(Action::QuitAsked { running: vec![running(1, "sleep"), running(2, "vim")] });
        m.update(Action::Close { pane: Some(3) });
        assert_eq!(m.confirm.as_ref().map(|c| c.running.len()), Some(2), "pane 3 ran nothing");
        m.update(Action::Close { pane: Some(1) });
        assert_eq!(m.confirm, Some(Confirm { running: vec![running(2, "vim")], dismiss: None }));
        // Nothing left to ask about: no question, and no quit either.
        let effects = m.update(Action::Close { pane: Some(2) });
        assert!(!effects.contains(&Effect::Quit), "{effects:?}");
        assert_eq!((m.mode(), &m.confirm, ids(&m)), (Mode::Normal, &None, vec![4]));
    }

    #[test]
    fn enter_q_and_cmd_q_answer_yes_and_every_other_key_of_the_users_answers_no() {
        let cmd = Mods { cmd: true, ..Mods::default() };
        let shift = Mods { shift: true, ..Mods::default() };
        let user = |key, mods| Confirm::answer(key, mods, Origin::User);
        for (key, mods) in [("enter", Mods::default()), ("q", Mods::default()), ("q", cmd)] {
            assert_eq!(user(key, mods), Some(Action::QuitConfirm), "{key} {mods:?}");
        }
        let cmd_ctrl = Mods { ctrl: true, ..cmd };
        let no = [("escape", Mods::default()), ("x", Mods::default()), ("q", shift), ("enter", cmd)];
        for (key, mods) in no.into_iter().chain([("v", cmd), ("c", cmd), ("h", cmd), ("w", cmd), ("f", cmd_ctrl)]) {
            assert_eq!(user(key, mods), Some(Action::QuitCancel), "{key} {mods:?}");
        }
    }

    #[test]
    fn a_socket_key_answers_with_enter_q_or_esc_and_is_refused_otherwise() {
        let cmd = Mods { cmd: true, ..Mods::default() };
        let socket = |key, mods| Confirm::answer(key, mods, Origin::Socket);
        assert_eq!(socket("enter", Mods::default()), Some(Action::QuitConfirm));
        assert_eq!(socket("q", Mods::default()), Some(Action::QuitConfirm));
        assert_eq!(socket("escape", Mods::default()), Some(Action::QuitCancel));
        for (key, mods) in [("x", Mods::default()), ("q", cmd), ("w", cmd), ("v", cmd), ("escape", cmd)] {
            assert_eq!(socket(key, mods), None, "{key} {mods:?}");
            assert_eq!(Confirm::answer(key, mods, Origin::System), None, "{key} {mods:?}");
        }
    }

    /// Every action from every origin, under the quit question. Only the user's own move
    /// cancels it; a client's or the system's runs underneath, or is refused if it would take
    /// the keyboard. Esc (`QuitCancel`) and ↵ (`QuitConfirm`) answer from the socket too.
    #[test]
    fn only_the_user_cancels_the_quit_question_whatever_the_action() {
        for action in &samples() {
            for origin in [Origin::User, Origin::Socket, Origin::System] {
                let mut m = Model::default();
                m.update(Action::Split { axis: Right, kind: Kind::Terminal });
                m.update(Action::Split { axis: Right, kind: Kind::Terminal });
                m.update(Action::QuitAsked { running: vec![running(1, "sleep")] });
                let user = origin == Origin::User;
                let refused = match action {
                    _ if user => None,
                    _ if writes_settings(action) => Some(apps::USERS_ALONE),
                    _ if of_workbooks(action) => Some(workbook::USERS_ALONE),
                    Action::TaskTalk { .. }
                    | Action::SessionOpen { .. }
                    | Action::TalkPick { .. }
                    | Action::TaskStop { .. }
                    | Action::TalkGo => Some(agents::USERS_ALONE),
                    Action::TabPlace { .. } | Action::NewTabOf { task: Some(_) } => Some(TABS_USERS),
                    Action::CalendarJoin { .. } | Action::CalendarDismiss { .. } => Some(calendar::USERS_ALONE),
                    _ if talks(action) => Some(chat::USERS_ALONE),
                    Action::AgentEvents { .. } if origin == Origin::Socket => Some(chat::RUNNERS_ALONE),
                    _ if in_palette(action) && action_is_palettes(action) => Some(commands::USERS_ALONE),
                    _ if in_palette(action) => Some(inspect::USERS_ALONE),
                    Action::InspectorRead { .. } | Action::InspectorFailed { .. } if origin == Origin::Socket => {
                        Some(inspect::USERS_ALONE)
                    }
                    Action::AppWheel { .. } => Some(crate::app_wheel::USERS_ALONE),
                    Action::RestoreLayout { .. } if origin == Origin::Socket => Some(crate::layout_file::USERS_ALONE),
                    Action::StartFresh | Action::UnfilledOpen { .. } => Some(crate::layout_file::USERS_ALONE),
                    _ => action.takes_keyboard().then_some(CONFIRM_UP),
                };
                assert_eq!(m.refusal(action, origin), refused, "{action:?} from {origin:?}");
                let before = (ids(&m), m.focused, m.tabs.len(), m.keys);
                let effects = m.update(action.clone().sent_by(origin));
                let quits = match action {
                    Action::QuitConfirm | Action::Quit => true,
                    // ⌘Q again is the user's yes.
                    Action::QuitAsked { .. } => user,
                    _ => false,
                };
                assert_eq!(effects.contains(&Effect::Quit), quits, "{action:?} from {origin:?}");
                if refused.is_some() {
                    assert_eq!((m.mode(), &effects[..]), (Mode::Confirm, &[][..]), "{action:?} from {origin:?}");
                    assert_eq!(before, (ids(&m), m.focused, m.tabs.len(), m.keys), "{action:?} from {origin:?}");
                }
                // Not the user's: the question stays while the pane it lists does.
                if !user && !matches!(action, Action::QuitCancel) {
                    assert_eq!(m.confirm.is_some(), m.pane(1).is_some(), "{action:?} from {origin:?}");
                }
                if matches!(action, Action::QuitCancel) {
                    assert_eq!(m.mode(), Mode::Normal, "{action:?} from {origin:?}");
                }
            }
        }
    }

    #[test]
    fn a_clients_request_runs_under_the_quit_question_and_the_users_cancels_it() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let ask = Action::QuitAsked { running: vec![running(1, "sleep"), running(2, "vim")] };
        m.update(ask.clone());
        // An agent splits: the pane opens underneath, the question stays.
        let split = Action::Split { axis: Right, kind: Kind::Terminal };
        assert_eq!(m.update(split.clone().sent_by(Origin::Socket)), [Effect::Opened(3)]);
        assert_eq!((m.mode(), ids(&m)), (Mode::Confirm, vec![1, 2, 3]));
        // An agent closes the focused pane: it goes, and nothing the question lists changed.
        assert_eq!(m.update(Action::Close { pane: None }.sent_by(Origin::Socket)).len(), 1);
        assert_eq!((m.mode(), ids(&m)), (Mode::Confirm, vec![1, 2]));
        // A shell exits: its pane drops off the list.
        m.update(Action::Close { pane: Some(1) }.sent_by(Origin::System));
        assert_eq!(m.confirm, Some(Confirm { running: vec![running(2, "vim")], dismiss: None }));
        // A client may not take the keyboard from under it, nor ask again as the user's yes.
        for taking in [Action::ToggleKeys, Action::Enter { pane: 2 }, ask.clone()] {
            assert_eq!(m.refusal(&taking, Origin::Socket), Some(CONFIRM_UP), "{taking:?}");
            assert_eq!(m.update(taking.sent_by(Origin::Socket)), []);
            assert_eq!((m.mode(), m.keys), (Mode::Confirm, false));
        }
        // The user's split cancels, and opens nothing.
        assert_eq!(m.update(split), []);
        assert_eq!((m.mode(), ids(&m)), (Mode::Normal, vec![2]));
    }

    #[test]
    fn the_quit_question_takes_the_keyboard_from_a_live_app() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 2 });
        assert_eq!(m.update(Action::QuitAsked { running: vec![running(1, "vim")] }), [Effect::Parked(2)]);
        assert_eq!(m.mode(), Mode::Confirm);
    }

    #[test]
    fn the_socket_quit_never_asks() {
        let mut m = Model::default();
        assert_eq!(m.update(Action::Quit), [Effect::Quit]);
        m.update(Action::QuitAsked { running: vec![running(1, "sleep")] });
        assert_eq!(m.update(Action::Quit), [Effect::Quit]);
    }

    #[test]
    fn the_quit_question_lists_five_terminals_then_counts_the_rest() {
        let one = Confirm { running: vec![Running { pane: 4, program: None, background: false }], dismiss: None };
        assert_eq!((one.title().as_str(), one.lines()), ("1 terminal is running", vec!["pane 4".to_string()]));
        let job = Confirm {
            running: vec![Running { pane: 2, program: Some("sleep".into()), background: true }],
            dismiss: None,
        };
        assert_eq!(job.lines(), ["pane 2 · sleep (background)"]);
        let seven = Confirm { running: (1..=7).map(|pane| running(pane, "claude")).collect(), dismiss: None };
        assert_eq!(seven.title(), "7 terminals are running");
        let lines = seven.lines();
        assert_eq!((lines.len(), lines[0].as_str(), lines[5].as_str()), (6, "pane 1 · claude", "and 2 more"));
    }
}
