//! Workspace state and its transitions. `Model::update` is the only way state changes.

use serde::{Deserialize, Serialize};

use crate::ax::Target;
use crate::compose;
use crate::home::{self, Home};
use crate::keys::Mods;
use crate::layout::{Axis, Dir, Node, Rect, Zone};
use crate::list::ListRow;
use crate::repos::{self, Repo};
use crate::sneak::{Sneak, Typed};
use crate::tasks::{MAX_TITLE, State, Task, TaskId, Tasks};

pub type PaneId = u32;

/// What a pane shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// Nothing yet. The next thing opened fills it.
    Empty,
    /// A shell on a PTY. The shell crate owns the process.
    Terminal,
    /// Another app's window, by its window-server id.
    App { window: u32, title: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pane {
    pub id: PaneId,
    pub kind: Kind,
    /// Where a terminal pane's shell starts, when it was opened for a repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
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

/// The quit question: these terminals would lose a running program.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Confirm {
    pub running: Vec<Running>,
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

    /// "3 terminals are running".
    pub fn title(&self) -> String {
        match self.running.len() {
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

/// Something the picker can put in a new pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PickItem {
    Terminal,
    /// A running app, by name. The shell hosts its main window.
    App {
        name: String,
    },
}

impl PickItem {
    pub fn label(&self) -> &str {
        match self {
            PickItem::Terminal => "Terminal",
            PickItem::App { name } => name,
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
    pub query: String,
    /// Index into `matches()`.
    pub selected: usize,
}

impl Picker {
    /// Items whose label contains the query, ignoring case. Labels that start with it come first.
    pub fn matches(&self) -> Vec<&PickItem> {
        let query = self.query.to_lowercase();
        let (mut starts, mut contains) = (Vec::new(), Vec::new());
        for item in &self.items {
            let label = item.label().to_lowercase();
            if label.starts_with(&query) {
                starts.push(item);
            } else if label.contains(&query) {
                contains.push(item);
            }
        }
        starts.extend(contains);
        starts
    }
}

/// A category on the left rail. Its list shows in the secondary bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Apps,
    Workbook,
    Tasks,
    Repos,
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
    /// A new, empty tab. It becomes the active tab.
    NewTab,
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
    /// Replace the picker's filter text.
    PickerQuery {
        query: String,
    },
    /// Move the picker's selection down (positive) or up; it wraps.
    PickerMove {
        delta: i32,
    },
    /// Add typed text to the picker's filter.
    PickerType {
        text: String,
    },
    /// Remove the last character of the picker's filter.
    PickerBackspace,
    /// Open an item (default: the selected one) and close the picker. `index` is into the
    /// current matches, so a click opens the row it landed on.
    PickerChoose {
        index: Option<usize>,
    },
    ClosePicker,
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
    /// File a task. `tab`: the `id` of an open tab of no task, which becomes the task's first tab.
    TaskAdd {
        title: String,
        priority: u8,
        state: State,
        tab: Option<u32>,
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
    /// Show a task's tabs (default: the first task that needs the user, "pop next"). A task
    /// opened for the first time gets a tab: a terminal in its repository, named after it, or
    /// an empty tab named after the task.
    TaskOpen {
        id: Option<TaskId>,
    },
    /// Show the tabs of no task again.
    TaskLeave,
    /// Select a tab by its place on the tab bar, which shows the current task's tabs only.
    SelectVisibleTab {
        index: usize,
    },
    /// The time now, in milliseconds since 1970: snoozed tasks whose time has come return.
    Tick {
        now: u64,
    },
    /// Open the new-task box, with `text` already in it (an agent's draft for the user to edit).
    OpenCompose {
        text: String,
    },
    ComposeType {
        text: String,
    },
    ComposeBackspace,
    /// Finish the `#project` or `@repo` being typed with the first suggestion.
    ComposeComplete,
    /// File the line as a task and close the box. A line with no title files nothing.
    ComposeSubmit,
    CloseCompose,
    /// File a task from one line, as the box does: `fix zoom flicker #canvas @app p1`.
    TaskFromLine {
        line: String,
    },
    /// Replace the Apps list with the apps the shell can host now.
    SetApps {
        names: Vec<String>,
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

    /// Whether the action takes the keyboard for a box, an app or another mode. Sneak mode lets
    /// only the user do that under its labels, and the quit question under itself
    /// (`Model::refusal`). Every action is named, with no
    /// catch-all: a new one must be put on a side.
    pub fn takes_keyboard(&self) -> bool {
        match self {
            Action::Enter { .. }
            | Action::OpenPicker { .. }
            | Action::OpenCompose { .. }
            | Action::ToggleKeys
            | Action::ToggleVisual
            | Action::ToggleAxOverlay
            | Action::QuitAsked { .. } => true,
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
            | Action::Leave
            | Action::ToggleSection { .. }
            | Action::CloseSidebar
            | Action::NewTab
            | Action::SelectTab { .. }
            | Action::CycleTab { .. }
            | Action::CloseTab { .. }
            | Action::PickerQuery { .. }
            | Action::PickerMove { .. }
            | Action::PickerType { .. }
            | Action::PickerBackspace
            | Action::PickerChoose { .. }
            | Action::ClosePicker
            | Action::CloseKeys
            | Action::LeaveVisual
            | Action::SetRepos { .. }
            | Action::OpenRepo { .. }
            | Action::RestoreTasks { .. }
            | Action::TaskAdd { .. }
            | Action::TaskSetState { .. }
            | Action::TaskSnooze { .. }
            | Action::TaskHold { .. }
            | Action::TaskDismiss { .. }
            | Action::TaskUndo
            | Action::TaskOpen { .. }
            | Action::TaskLeave
            | Action::SelectVisibleTab { .. }
            | Action::Tick { .. }
            | Action::ComposeType { .. }
            | Action::ComposeBackspace
            | Action::ComposeComplete
            | Action::ComposeSubmit
            | Action::CloseCompose
            | Action::TaskFromLine { .. }
            | Action::SetApps { .. }
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
            | Action::QuitConfirm
            | Action::QuitCancel
            | Action::Quit => false,
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
    /// End the app.
    Quit,
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
    layout: Option<Node>,
    #[serde(skip)]
    focused: Option<PaneId>,
}

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
    /// The task whose tabs are on show. `None`: the tabs of no task. Tabs of other tasks keep
    /// their panes and go on running; they are only off the tab bar.
    pub current_task: Option<TaskId>,
    /// The tab each task (or no task) showed last, to come back to it.
    #[serde(skip)]
    last_tab: Vec<(Option<TaskId>, u32)>,
    /// Rail section shown in the secondary bar. `None`: the bar is closed.
    pub sidebar: Option<Section>,
    /// The text in the new-task box, while it is open.
    pub compose: Option<String>,
    /// The selected row, while the secondary bar has the keyboard.
    pub list: Option<usize>,
    /// The Apps section's list: names of the apps the shell can host.
    #[serde(skip)]
    pub apps: Vec<String>,
    /// The Repos section's list, as last read from disk.
    pub repos: Vec<Repo>,
    pub tasks: Tasks,
    /// The pane picker, while it is open.
    pub picker: Option<Picker>,
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
    #[serde(skip)]
    next_id: PaneId,
    #[serde(skip)]
    next_tab: u32,
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
            sidebar: None,
            compose: None,
            list: None,
            apps: Vec::new(),
            repos: Vec::new(),
            tasks: Tasks::default(),
            picker: None,
            confirm: None,
            keys: false,
            visual: None,
            ax_overlay: false,
            sneak: None,
            next_id: 0,
            current_task: None,
            last_tab: Vec::new(),
            next_tab: 0,
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

    pub fn mode(&self) -> Mode {
        if self.confirm.is_some() {
            Mode::Confirm
        } else if self.sneak.is_some() {
            Mode::Sneak
        } else if self.picker.is_some() {
            Mode::Pick
        } else if self.compose.is_some() {
            Mode::Compose
        } else if self.live.is_some() {
            Mode::App
        } else if self.visual.is_some() {
            Mode::Visual
        } else if self.list.is_some() {
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

    /// Why an action does nothing because of who sent it. Only the user may take the keyboard
    /// from under sneak mode's labels or the quit question, or close the picker by opening the
    /// key reference.
    /// `None`: the origin does not stop it. The shell answers a socket request with the reason.
    pub fn refusal(&self, action: &Action, origin: Origin) -> Option<&'static str> {
        let boxed = self.picker.is_some() || self.compose.is_some();
        match action {
            _ if origin == Origin::User => None,
            action if self.sneak.is_some() && action.takes_keyboard() => Some("sneak mode is on"),
            action if self.confirm.is_some() && action.takes_keyboard() => Some(CONFIRM_UP),
            Action::ToggleKeys if boxed && !self.keys => Some("the picker or the new-task box is open"),
            _ => None,
        }
    }

    pub fn update(&mut self, sent: impl Into<Envelope>) -> Vec<Effect> {
        let Envelope { action, origin } = sent.into();
        let user = origin == Origin::User;
        let mut effects = Vec::new();
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
        );
        if user && !own {
            self.sneak = None;
        }
        if self.refusal(&action, origin).is_some() {
            return effects;
        }
        // The picker has the keyboard. Pane and bar actions wait until it closes, so a
        // shortcut cannot change what is behind it (e.g. ⌘W closing the pane it splits).
        let for_panes = matches!(
            action,
            Action::Split { .. }
                | Action::OpenRepo { .. }
                | Action::TaskOpen { .. }
                | Action::FocusList
                | Action::ListSection { .. }
                | Action::ListChoose { .. }
                | Action::Close { .. }
                | Action::Focus { .. }
                | Action::FocusDir { .. }
                | Action::FocusCycle { .. }
                | Action::Resize { .. }
                | Action::SwapDir { .. }
                | Action::MovePane { .. }
                | Action::Equalize
                | Action::DropPane { .. }
                | Action::Enter { .. }
                | Action::ToggleSection { .. }
                | Action::CloseSidebar
                | Action::NewTab
                | Action::SelectTab { .. }
                | Action::SelectVisibleTab { .. }
                | Action::TaskLeave
                | Action::CycleTab { .. }
                | Action::CloseTab { .. }
        );
        // The key reference covers the workspace too, so the same actions wait behind it, and the
        // picker cannot open under it.
        let blocked_by_keys = self.keys && (for_panes || matches!(action, Action::OpenPicker { .. }));
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
                    | Action::OpenCompose { .. }
                    | Action::ToggleKeys
                    | Action::ToggleVisual
                    | Action::ToggleAxOverlay
            );
        if self.confirm.is_some() && (cancels || (user && answers_no)) {
            self.confirm = None;
            return effects;
        }
        let blocked_by_visual =
            self.visual.is_some() && !named_close && (for_panes || matches!(action, Action::OpenPicker { .. }));
        // The new-task box has the keyboard, as the picker does: what is behind it waits, and
        // neither opens over the other. (The picker may open again over itself: ⌘T while it is
        // open turns the pick into a new tab.) A pane whose window or shell is gone still closes.
        // The box does not open under the key reference or in visual mode.
        let boxed = self.picker.is_some() || self.compose.is_some();
        let over_other = (self.compose.is_some() && matches!(action, Action::OpenPicker { .. }))
            || (boxed && matches!(action, Action::OpenCompose { .. }));
        let behind_box = (boxed && for_panes && !named_close) || over_other;
        let no_box_now = matches!(action, Action::OpenCompose { .. }) && (self.keys || self.visual.is_some());
        if behind_box || no_box_now || blocked_by_keys || blocked_by_visual {
            return effects;
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
                | Action::SelectTab { .. }
                | Action::SelectVisibleTab { .. }
                | Action::TaskLeave
                | Action::CycleTab { .. }
                | Action::CloseSidebar
                | Action::TaskOpen { .. }
                | Action::ToggleVisual
                | Action::LeaveList
        );
        if leaves_list {
            self.list = None;
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
                | Action::TaskUndo
                | Action::ToggleSection { .. }
                | Action::FocusList
        );
        let selected = self.list.and_then(|at| self.list_rows().into_iter().nth(at)).filter(|_| !own_move);
        self.act(action, &mut effects);
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
            if confirm.running.is_empty() {
                self.confirm = None;
            }
        }
        effects
    }

    fn act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::Split { axis, kind } => {
                self.leave(effects);
                let id = self.add(kind, self.focused, axis);
                effects.push(Effect::Opened(id));
            }
            Action::Open { kind, axis, beside } => self.open(kind, axis, beside, effects),
            Action::SetRepos { repos } => self.repos = repos::arrange(repos),
            Action::RestoreTasks { tasks, floor } => self.tasks.restore(tasks, floor),
            Action::TaskAdd { title, priority, state, tab } => {
                let id = self.tasks.add(title, priority, state);
                // The tab named becomes the task's, unless another task has it: an agent that
                // files a follow-up from a task's tab must not take the tab away. If it is the one
                // on show, the view is now the task's.
                let free = |t: &Tab| Some(t.id) == tab && t.task.is_none();
                if let Some(index) = self.tabs.iter().position(free) {
                    self.tabs[index].task = Some(id);
                    if index == self.active {
                        self.remember_tab();
                        self.current_task = Some(id);
                    }
                }
            }
            Action::TaskSetState { id, state } => self.tasks.set_state(id, state),
            Action::TaskSnooze { id, until } => self.tasks.snooze(id, until),
            Action::TaskHold { id } => self.tasks.hold(id),
            Action::TaskDismiss { id } => self.tasks.dismiss(id),
            Action::TaskUndo => self.tasks.undo(),
            Action::Tick { now } => self.tasks.tick(now),
            Action::TaskOpen { id } => {
                let task = id.map_or_else(|| self.tasks.next(), |id| self.tasks.get(id));
                if let Some((id, title, repo)) = task.map(|t| (t.id, t.title.clone(), t.repo.clone())) {
                    let first_time = !self.tabs.iter().any(|tab| tab.task == Some(id));
                    self.show_tabs_of(Some(id), effects);
                    if first_time {
                        // The bar already names the task: the tab is named for where it works.
                        let place = repo.as_deref().and_then(|path| self.repo_name(path));
                        if let Some(tab) = self.tabs.get_mut(self.active) {
                            tab.title = place.unwrap_or_else(|| short_title(&title));
                        }
                        // The task's repository is where its work starts.
                        if let Some(path) = repo.filter(|path| self.repo_listed(path)) {
                            self.open(Kind::Terminal, Axis::Right, None, effects);
                            if let Some(pane) = self.focused.and_then(|id| self.panes.iter_mut().find(|p| p.id == id)) {
                                pane.cwd = Some(path);
                            }
                        }
                    }
                }
            }
            Action::TaskLeave => self.show_tabs_of(None, effects),
            Action::SelectVisibleTab { index } => {
                if let Some(real) = self.visible_tabs().get(index).copied() {
                    self.select_tab(real, effects);
                }
            }
            Action::OpenRepo { path, axis } => {
                // Only a listed path: the list is what the user sees and can choose from.
                if let Some(name) = self.repo_name(&path) {
                    let empty_tab = self.layout.is_none();
                    self.open(Kind::Terminal, axis, None, effects);
                    if let Some(pane) = self.focused.and_then(|id| self.panes.iter_mut().find(|p| p.id == id)) {
                        pane.cwd = Some(path);
                    }
                    if let Some(tab) = self.tabs.get_mut(self.active).filter(|_| empty_tab) {
                        tab.title = name;
                    }
                }
            }
            Action::Adopt { kind, beside } => {
                self.next_id += 1;
                let id = self.next_id;
                self.panes.push(Pane { id, kind, cwd: None });
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
                self.leave(effects);
                self.park_active();
                self.push_tab();
            }
            Action::SelectTab { index } => self.select_tab(index, effects),
            Action::CycleTab { delta } => {
                // Among the tabs on the bar: those of the current task.
                let visible = self.visible_tabs();
                if let Some(at) = visible.iter().position(|index| *index == self.active) {
                    let to = (at as i64 + delta as i64).rem_euclid(visible.len() as i64) as usize;
                    self.select_tab(visible[to], effects);
                }
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
                for id in tab.layout.as_ref().map(Node::leaves).unwrap_or_default() {
                    self.close_pane(id, effects);
                }
                if !was_on_show {
                    // The tab on show stays; it may have moved one place left.
                    self.active -= usize::from(index < self.active);
                    self.unpark_active();
                    return;
                }
                // Its right neighbour in the same task takes its place, else the left one. A task
                // (or no task) left with no tab gets an empty one, so there is always a tab on show.
                let scope = self.current_task;
                let mine = |(i, t): (usize, &Tab)| (t.task == scope).then_some(i);
                let right = self.tabs.iter().enumerate().skip(index).find_map(mine);
                let left = self.tabs.iter().enumerate().take(index).filter_map(mine).last();
                match right.or(left) {
                    Some(next) => {
                        self.active = next;
                        self.unpark_active();
                    }
                    None => {
                        if self.tabs.is_empty() {
                            self.next_tab = 0;
                        }
                        self.push_tab();
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
                let to = self.visible_tabs().get(tab).copied();
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
            Action::Enter { pane } => {
                let is_app = self.pane(pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
                if is_app && self.live != Some(pane) {
                    // A live window over a tab that does not show its pane would cover other panes.
                    self.show_tab_of(pane, effects);
                    self.leave(effects);
                    self.live = Some(pane);
                    self.focused = Some(pane);
                    effects.push(Effect::Live(pane));
                }
            }
            Action::Leave => self.leave(effects),
            // The bar is ottospace's own UI, so using it takes the keyboard back.
            Action::ToggleSection { section } => {
                self.leave(effects);
                self.sidebar = (self.sidebar != Some(section)).then_some(section);
                self.list = self.list.map(|_| 0);
            }
            Action::OpenCompose { text } => {
                self.leave(effects);
                self.compose = Some(text.chars().take(MAX_TITLE).collect());
            }
            Action::ComposeType { text } => {
                // No longer than a task's title may be: a longer one could not be kept.
                if let Some(line) = &mut self.compose {
                    let room = MAX_TITLE.saturating_sub(line.chars().count());
                    line.extend(text.chars().take(room));
                }
            }
            Action::ComposeBackspace => {
                if let Some(line) = &mut self.compose {
                    line.pop();
                }
            }
            Action::ComposeComplete => {
                if let (Some(line), Some(first)) = (&self.compose, self.compose_suggestions().first()) {
                    self.compose = Some(compose::complete(line, first));
                }
            }
            Action::ComposeSubmit => {
                let line = self.compose.clone().unwrap_or_default();
                if self.file_line(&line) {
                    self.compose = None;
                }
            }
            Action::CloseCompose => self.compose = None,
            Action::TaskFromLine { line } => drop(self.file_line(&line)),
            Action::SetApps { names } => self.apps = names,
            Action::FocusList if self.list.is_some() => self.list = None,
            Action::FocusList => {
                self.leave(effects);
                self.sidebar = self.sidebar.or(Some(Section::Repos));
                self.list = Some(0);
            }
            Action::LeaveList => {}
            Action::ListMove { delta } => {
                let rows = self.list_rows().len() as i32;
                if let Some(at) = self.list.filter(|_| rows > 0) {
                    self.list = Some((at as i32 + delta).rem_euclid(rows) as usize);
                }
            }
            Action::ListSection { delta } => {
                // Workbook has no rows yet, so the keys pass over it.
                const ORDER: [Section; 3] = [Section::Apps, Section::Tasks, Section::Repos];
                let at = ORDER.iter().position(|s| Some(*s) == self.sidebar).unwrap_or(0) as i32;
                if self.list.is_some() {
                    self.sidebar = Some(ORDER[(at + delta).rem_euclid(ORDER.len() as i32) as usize]);
                    self.list = Some(0);
                }
            }
            Action::ListChoose { axis } => match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
                // The shell hosts the window, then opens the pane; the list keeps the keys if it fails.
                Some(ListRow::App(name)) => effects.push(Effect::HostApp { name, axis, new_tab: false }),
                Some(ListRow::Repo(path)) => self.act(Action::OpenRepo { path, axis }, effects),
                Some(ListRow::Task(id)) => {
                    self.list = None;
                    self.act(Action::TaskOpen { id: Some(id) }, effects);
                }
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
                    self.tasks.dismiss(id);
                }
            }
            Action::OpenPicker { axis, new_tab, items } => {
                self.leave(effects);
                self.picker = Some(Picker { axis, new_tab, items, query: String::new(), selected: 0 });
            }
            Action::PickerQuery { query } => {
                if let Some(picker) = &mut self.picker {
                    picker.query = query;
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
            Action::PickerType { text } => {
                if let Some(picker) = &mut self.picker {
                    picker.query.push_str(&text);
                    picker.selected = 0;
                }
            }
            Action::PickerBackspace => {
                if let Some(picker) = &mut self.picker {
                    picker.query.pop();
                    picker.selected = 0;
                }
            }
            Action::PickerChoose { index } => {
                let Some(picker) = self.picker.take() else { return };
                let choice = picker.matches().get(index.unwrap_or(picker.selected)).copied().cloned();
                match choice {
                    // Same rule as an app: fill the focused pane if it is empty, else split it.
                    Some(PickItem::Terminal) => {
                        if picker.new_tab {
                            self.park_active();
                            self.push_tab();
                        }
                        self.open(Kind::Terminal, picker.axis, None, effects)
                    }
                    // The shell makes the tab only once the window is in hand, so a failed
                    // host does not leave an empty tab.
                    Some(PickItem::App { name }) => {
                        effects.push(Effect::HostApp { name, axis: picker.axis, new_tab: picker.new_tab })
                    }
                    // Nothing matches: keep the picker open so the user can fix the query.
                    None => self.picker = Some(picker),
                }
            }
            Action::ClosePicker => self.picker = None,
            // The overlay is ottospace's own UI: it needs the keyboard back to be dismissed.
            Action::ToggleKeys => {
                self.keys = !self.keys;
                if self.keys {
                    // One overlay at a time, and the overlay needs the keyboard to be dismissed.
                    self.picker = None;
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
                if matches!(self.mode(), Mode::Normal | Mode::List) && !self.keys {
                    self.sneak = Sneak::new(targets);
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
                    Some(Typed::NoLabel) => self.sneak = None,
                    Some(Typed::Narrowed) | None => {}
                }
            }
            Action::SneakBackspace => {
                if let Some(sneak) = &mut self.sneak {
                    sneak.backspace();
                }
            }
            Action::LeaveSneak => self.sneak = None,
            Action::QuitAsked { running } if running.is_empty() || self.confirm.is_some() => effects.push(Effect::Quit),
            Action::QuitAsked { running } => {
                // The question is ottospace's own UI: a live window would cover it.
                self.leave(effects);
                self.confirm = Some(Confirm { running });
            }
            Action::QuitConfirm if self.confirm.is_some() => effects.push(Effect::Quit),
            Action::QuitConfirm | Action::QuitCancel => {}
            Action::Quit => effects.push(Effect::Quit),
        }
    }

    /// What `line` would file: `@name` counts only for a listed repository.
    pub fn draft(&self, line: &str) -> compose::Draft {
        compose::parse(line, |name| self.repo_named(name).is_some())
    }

    /// Files `line` as a captured task. False if it has no title, or one too long to keep:
    /// nothing is filed.
    fn file_line(&mut self, line: &str) -> bool {
        let draft = self.draft(line);
        if draft.title.is_empty() || draft.title.chars().count() > MAX_TITLE {
            return false;
        }
        let path = draft.repo.as_deref().and_then(|name| self.repo_named(name));
        let id = self.tasks.add(draft.title, draft.priority.unwrap_or(3), State::Captured);
        self.tasks.tag(id, draft.project, path);
        true
    }

    /// Every listed repository and worktree, as (name, path).
    fn repo_names(&self) -> impl Iterator<Item = (&str, &str)> {
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

    /// Names that finish the `#project` or `@repo` being typed in the new-task box.
    pub fn compose_suggestions(&self) -> Vec<String> {
        match self.compose.as_deref().and_then(compose::completing) {
            Some(('@', typed)) => compose::suggest(typed, self.repo_names().map(|(name, _)| name)),
            Some((_, typed)) => compose::suggest(typed, self.tasks.projects()),
            None => Vec::new(),
        }
    }

    /// The rows of the open section, top to bottom, as the secondary bar draws them.
    pub fn list_rows(&self) -> Vec<ListRow> {
        match self.sidebar {
            Some(Section::Apps) => self.apps.iter().cloned().map(ListRow::App).collect(),
            Some(Section::Repos) => {
                let paths = self
                    .repos
                    .iter()
                    .flat_map(|r| std::iter::once(r.path.clone()).chain(r.worktrees.iter().map(|w| w.path.clone())));
                paths.map(ListRow::Repo).collect()
            }
            Some(Section::Tasks) => self.tasks.inbox().into_iter().map(|(_, task)| ListRow::Task(task.id)).collect(),
            Some(Section::Workbook) | None => Vec::new(),
        }
    }

    fn selected_task(&self) -> Option<TaskId> {
        match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
            Some(ListRow::Task(id)) => Some(id),
            _ => None,
        }
    }

    /// Shows `kind` in `beside` (default: the focused pane) if that pane is empty; else splits it.
    fn open(&mut self, kind: Kind, axis: Axis, beside: Option<PaneId>, effects: &mut Vec<Effect>) {
        self.list = None; // the new pane takes the keyboard
        // The new pane takes focus, so the live app must give the keyboard back, and visual mode ends.
        self.leave(effects);
        self.leave_visual(effects);
        let target = beside.or(self.focused);
        let empty = target.filter(|t| self.pane(*t).is_some_and(|p| p.kind == Kind::Empty));
        let id = match empty.and_then(|t| self.panes.iter_mut().find(|p| p.id == t)) {
            Some(pane) => {
                pane.kind = kind;
                pane.id
            }
            None => self.add(kind, target, axis),
        };
        self.focused = Some(id);
        effects.push(Effect::Opened(id));
    }

    fn push_tab(&mut self) {
        self.next_tab += 1;
        let title = format!("Tab {}", self.next_tab);
        self.tabs.push(Tab { id: self.next_tab, title, task: self.current_task, layout: None, focused: None });
        self.active = self.tabs.len() - 1;
    }

    /// Places on the tab bar → indexes into `tabs`: the tabs of the current task, or of no task.
    pub fn visible_tabs(&self) -> Vec<usize> {
        let on_show = |(i, tab): (usize, &Tab)| (tab.task == self.current_task).then_some(i);
        self.tabs.iter().enumerate().filter_map(on_show).collect()
    }

    /// Notes which tab the current task (or no task) has on show, to come back to it.
    fn remember_tab(&mut self) {
        if let Some(tab) = self.tabs.get(self.active) {
            let scope = self.current_task;
            self.last_tab.retain(|(task, _)| *task != scope);
            self.last_tab.push((scope, tab.id));
        }
    }

    /// Puts the tabs of `scope` (a task, or no task) on the bar: the one it showed last, else its
    /// first, else a new empty one.
    fn show_tabs_of(&mut self, scope: Option<TaskId>, effects: &mut Vec<Effect>) {
        if scope == self.current_task {
            return;
        }
        self.remember_tab();
        let last = self.last_tab.iter().find(|(task, _)| *task == scope).map(|(_, tab)| *tab);
        let last = last.and_then(|id| self.tabs.iter().position(|tab| tab.id == id && tab.task == scope));
        let index = last.or_else(|| self.tabs.iter().position(|tab| tab.task == scope));
        // A live app window would stay on screen over the other tabs.
        self.leave(effects);
        self.park_active();
        self.current_task = scope;
        match index {
            Some(index) => {
                self.active = index;
                self.unpark_active();
            }
            None => self.push_tab(),
        }
    }

    /// Stores the active tab's tree and focus in the tab, before another tab takes the stage.
    fn park_active(&mut self) {
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

    /// The `id` of the tab that holds `pane`, active tab included.
    pub fn tab_of(&self, pane: PaneId) -> Option<u32> {
        let index = self.tab_panes().iter().position(|panes| panes.contains(&pane))?;
        self.tabs.get(index).map(|tab| tab.id)
    }

    fn select_tab(&mut self, index: usize, effects: &mut Vec<Effect>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        // A live app window would stay on screen over the other tab.
        self.leave(effects);
        self.remember_tab();
        self.park_active();
        self.active = index;
        // A tab of another task brings that task's tabs with it.
        self.current_task = self.tabs[index].task;
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
        self.panes.push(Pane { id, kind, cwd: None });
        self.focused = Some(id);
        id
    }

    /// Focusing any pane but the live one takes the keyboard back.
    fn focus(&mut self, id: PaneId, effects: &mut Vec<Effect>) {
        if self.pane(id).is_none() {
            return;
        }
        self.show_tab_of(id, effects);
        if self.live != Some(id) {
            self.leave(effects);
        }
        self.focused = Some(id);
    }

    fn leave_visual(&mut self, effects: &mut Vec<Effect>) {
        if let Some(pane) = self.visual.take() {
            effects.push(Effect::Visual { pane, on: false });
        }
    }

    fn leave(&mut self, effects: &mut Vec<Effect>) {
        if let Some(id) = self.live.take() {
            effects.push(Effect::Parked(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis::{Down, Right};

    fn app(window: u32) -> Kind {
        Kind::App { window, title: format!("w{window}") }
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
            Action::TaskAdd { title: "t".into(), priority: 3, state: State::Running, tab: None },
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
        Action::Enter { .. },
        Action::Leave,
        Action::ToggleSection { .. },
        Action::CloseSidebar,
        Action::OpenPicker { .. },
        Action::NewTab,
        Action::SelectTab { .. },
        Action::CycleTab { .. },
        Action::CloseTab { .. },
        Action::PickerQuery { .. },
        Action::PickerMove { .. },
        Action::PickerType { .. },
        Action::PickerBackspace,
        Action::PickerChoose { .. },
        Action::ClosePicker,
        Action::ToggleKeys,
        Action::CloseKeys,
        Action::ToggleVisual,
        Action::LeaveVisual,
        Action::ToggleAxOverlay,
        Action::SetRepos { .. },
        Action::OpenRepo { .. },
        Action::RestoreTasks { .. },
        Action::TaskAdd { .. },
        Action::TaskSetState { .. },
        Action::TaskSnooze { .. },
        Action::TaskHold { .. },
        Action::TaskDismiss { .. },
        Action::TaskUndo,
        Action::TaskOpen { .. },
        Action::TaskLeave,
        Action::SelectVisibleTab { .. },
        Action::Tick { .. },
        Action::OpenCompose { .. },
        Action::ComposeType { .. },
        Action::ComposeBackspace,
        Action::ComposeComplete,
        Action::ComposeSubmit,
        Action::CloseCompose,
        Action::TaskFromLine { .. },
        Action::SetApps { .. },
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
        Action::QuitAsked { .. },
        Action::QuitConfirm,
        Action::QuitCancel,
        Action::Quit
    );

    /// One of each action.
    fn samples() -> Vec<Action> {
        let text = String::new;
        let samples = vec![
            Action::Split { axis: Right, kind: Kind::Empty },
            Action::Open { kind: app(1), axis: Right, beside: None },
            Action::Adopt { kind: app(1), beside: None },
            Action::DropPane { source: 1, target: 1, zone: Zone::Left },
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
            Action::SelectTab { index: 0 },
            Action::CycleTab { delta: 1 },
            Action::CloseTab { index: None },
            Action::PickerQuery { query: text() },
            Action::PickerMove { delta: 1 },
            Action::PickerType { text: text() },
            Action::PickerBackspace,
            Action::PickerChoose { index: None },
            Action::ClosePicker,
            Action::ToggleKeys,
            Action::CloseKeys,
            Action::ToggleVisual,
            Action::LeaveVisual,
            Action::ToggleAxOverlay,
            Action::SetRepos { repos: Vec::new() },
            Action::OpenRepo { path: text(), axis: Right },
            Action::RestoreTasks { tasks: Vec::new(), floor: 0 },
            Action::TaskAdd { title: text(), priority: 3, state: State::Running, tab: None },
            Action::TaskSetState { id: 1, state: State::Done },
            Action::TaskSnooze { id: 1, until: 0 },
            Action::TaskHold { id: 1 },
            Action::TaskDismiss { id: 1 },
            Action::TaskUndo,
            Action::TaskOpen { id: None },
            Action::TaskLeave,
            Action::SelectVisibleTab { index: 0 },
            Action::Tick { now: 0 },
            Action::OpenCompose { text: text() },
            Action::ComposeType { text: text() },
            Action::ComposeBackspace,
            Action::ComposeComplete,
            Action::ComposeSubmit,
            Action::CloseCompose,
            Action::TaskFromLine { line: text() },
            Action::SetApps { names: Vec::new() },
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
            Action::QuitAsked { running: Vec::new() },
            Action::QuitConfirm,
            Action::QuitCancel,
            Action::Quit,
        ];
        let mut places: Vec<usize> = samples.iter().map(index).collect();
        places.sort_unstable();
        assert_eq!(places, (0..COUNT).collect::<Vec<_>>(), "one sample of every action");
        samples
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
                let refused = (origin != Origin::User && action.takes_keyboard()).then_some("sneak mode is on");
                assert_eq!(m.refusal(action, origin), refused, "{action:?} from {origin:?}");
                m.update(action.clone().sent_by(origin));
                let ends = match action {
                    // The mode's own: its key and Esc leave from anywhere, the rest stay in it.
                    Action::Sneak { .. } | Action::LeaveSneak => true,
                    Action::SneakEnter { .. }
                    | Action::SneakDrawn { .. }
                    | Action::SneakType { .. }
                    | Action::SneakBackspace => false,
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

    /// An agent's `task_add` takes tab 2 off the bar while the labels are up, and tab 3 slides
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

        // News: the mode stays on, and tab 2 is the task's, off the bar.
        let add = Action::TaskAdd { title: "t".into(), priority: 3, state: State::Running, tab: Some(2) };
        m.update(add.sent_by(Origin::Socket));
        assert_eq!((m.mode(), m.visible_tabs()), (Mode::Sneak, vec![0, 2]));
        // The next frame draws tabs 1 and 3: tab 3 where tab 2 was.
        m.update(Action::SneakDrawn { targets: tab_bar(&[1, 3]) }.sent_by(Origin::System));
        assert_eq!(label_of(&m, "tab-close:2"), None, "not drawn: no label");
        assert_eq!(label_of(&m, "tab-close:3"), Some(third.clone()), "the same letters");
        let moved = m.sneak.as_ref().unwrap().targets.iter().find(|t| t.label == third).unwrap();
        assert_eq!(moved.target.frame.x, 180., "the pill went with its control");

        // The letters that were on tab 2's × name nothing now: no tab closes, hidden or shown.
        assert_eq!(m.update(Action::SneakType { text: second }), []);
        assert_eq!((m.mode(), m.tabs.len()), (Mode::Normal, 3));
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
        m.update(Action::TaskAdd { title: "ask".into(), priority: 1, state: State::NeedsYou, tab: None });
        let home = m.home(&[]).expect("no panes: the home screen");
        assert_eq!((home.nodes.len(), home.needs_you.len()), (1, 1));
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        assert!(m.home(&[]).is_none(), "a pane takes its place");
        // The task's own tab has no pane yet: the home screen again.
        m.update(Action::TaskOpen { id: Some(1) });
        assert!(m.home(&[]).is_some());
        m.update(Action::TaskLeave);
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
        assert_eq!(effects, [Effect::Closed(Pane { id: 2, kind: Kind::Empty, cwd: None })]);
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
        assert_eq!(m.panes, [Pane { id: 1, kind: Kind::Terminal, cwd: None }]);
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
        m.update(Action::PickerType { text: "g".into() });
        m.update(Action::PickerType { text: "x".into() });
        m.update(Action::PickerBackspace);
        assert_eq!(labels(&m), ["Google Chrome", "Google Slides"]);
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

    fn tab_titles(m: &Model) -> Vec<&str> {
        m.visible_tabs().into_iter().map(|i| m.tabs[i].title.as_str()).collect()
    }

    #[test]
    fn a_task_has_its_own_tabs_and_opening_it_puts_them_on_the_bar() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        term(&mut m); // pane 1, in a tab of no task
        m.update(Action::TaskFromLine {
            line: "fix the zoom flicker on the canvas when zooming #canvas @app-2".into(),
        });
        m.update(Action::TaskFromLine { line: "ask Ana about the retry plan before Friday".into() });
        assert_eq!((m.current_task, tab_titles(&m)), (None, vec!["Tab 1"]));

        // The first time: the task gets a tab named after it, with a terminal in its repository.
        assert_eq!(m.update(Action::TaskOpen { id: Some(1) }), [Effect::Opened(2)]);
        assert_eq!((m.current_task, tab_titles(&m)), (Some(1), vec!["app-2"]));
        assert_eq!((m.focused, m.pane(2).and_then(|p| p.cwd.as_deref())), (Some(2), Some("/code/app-2")));
        // New tabs are the task's. The bar counts only them: ⌘1 is the task's first tab.
        m.update(Action::NewTab);
        assert_eq!((m.tabs.len(), tab_titles(&m).len(), m.tabs[m.active].task), (3, 2, Some(1)));
        m.update(Action::SelectVisibleTab { index: 0 });
        assert_eq!(m.focused, Some(2));
        m.update(Action::CycleTab { delta: 1 });
        m.update(Action::CycleTab { delta: 1 });
        assert_eq!(m.focused, Some(2), "cycling stays inside the task");

        // A task with no repository gets an empty tab. The first task's panes go on, off the bar.
        assert_eq!(m.update(Action::TaskOpen { id: Some(2) }), []);
        assert_eq!(
            (m.current_task, tab_titles(&m), m.focused, m.panes.len()),
            (Some(2), vec!["ask Ana about the retry…"], None, 2)
        );
        // Coming back: the tab it showed last, not a new one.
        m.update(Action::TaskOpen { id: Some(1) });
        assert_eq!((m.focused, m.tabs.len()), (Some(2), 4));
        // Leaving the task shows the tabs of no task again.
        m.update(Action::TaskLeave);
        assert_eq!((m.current_task, tab_titles(&m), m.focused), (None, vec!["Tab 1"], Some(1)));
        // Selecting a tab by its index among all tabs (the socket does) brings its task along.
        m.update(Action::SelectTab { index: 3 });
        assert_eq!(m.current_task, Some(2));
    }

    #[test]
    fn pop_next_opens_the_task_that_needs_the_user() {
        let mut m = Model::default();
        m.update(Action::TaskAdd { title: "runs".into(), priority: 1, state: State::Running, tab: None });
        m.update(Action::TaskAdd { title: "waits".into(), priority: 2, state: State::NeedsYou, tab: None });
        // Not the more urgent one that is only running.
        m.update(Action::TaskOpen { id: None });
        assert_eq!((m.current_task, tab_titles(&m)), (Some(2), vec!["waits"]));
        // Nothing needs the user: the view stays.
        m.update(Action::TaskSetState { id: 2, state: State::Running });
        m.update(Action::TaskLeave);
        m.update(Action::TaskOpen { id: None });
        assert_eq!(m.current_task, None);
    }

    #[test]
    fn a_tab_named_when_a_task_is_filed_becomes_the_tasks() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::NewTab);
        let (first, second) = (m.tabs[0].id, m.tabs[1].id);
        // A background tab: the view stays where it is.
        m.update(Action::TaskAdd { title: "a".into(), priority: 1, state: State::Running, tab: Some(first) });
        assert_eq!((m.current_task, m.tabs[0].task, tab_titles(&m).len()), (None, Some(1), 1));
        // The tab on show: the view is now the task's.
        m.update(Action::TaskAdd { title: "b".into(), priority: 1, state: State::Running, tab: Some(second) });
        assert_eq!((m.current_task, tab_titles(&m).len()), (Some(2), 1));
        // Opening the first task shows the tab it was given, with its pane.
        m.update(Action::TaskOpen { id: Some(1) });
        assert_eq!((m.active, m.focused), (0, Some(1)));
        // A follow-up filed from that tab does not take it from the task that has it.
        m.update(Action::TaskAdd { title: "c".into(), priority: 1, state: State::Captured, tab: Some(first) });
        assert_eq!((m.current_task, m.tabs[0].task), (Some(1), Some(1)));
    }

    #[test]
    fn closing_a_tasks_last_tab_leaves_it_an_empty_one() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::TaskAdd { title: "a".into(), priority: 1, state: State::Running, tab: None });
        m.update(Action::TaskOpen { id: Some(1) });
        m.update(Action::NewTab);
        term(&mut m); // pane 2, in the task's second tab
        // The task's other tab takes its place, not the tab of no task beside it.
        m.update(Action::CloseTab { index: None });
        assert_eq!((m.current_task, tab_titles(&m), m.panes.len()), (Some(1), vec!["a"], 1));
        // Its last tab: the task keeps the bar, with a new empty tab.
        m.update(Action::CloseTab { index: None });
        assert_eq!((m.current_task, m.visible_tabs().len(), m.tabs.len(), m.focused), (Some(1), 1, 2, None));
        // A background tab closed by index: the tab on show stays.
        m.update(Action::CloseTab { index: Some(0) });
        assert_eq!((m.current_task, m.tabs.len(), m.panes.len()), (Some(1), 1, 0));
    }

    #[test]
    fn list_mode_moves_over_the_rows_and_opens_one() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: listed() });
        // The bar opens on Repos and takes the keyboard.
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
        for (title, tab) in [("a", None), ("b", Some(second)), ("c", None)] {
            m.update(Action::TaskAdd { title: title.into(), priority: 1, state: State::NeedsYou, tab });
        }
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        // Dismiss the first: the selection stays at the top, now on the next task.
        m.update(Action::ListDismiss);
        assert_eq!((m.list_rows(), m.list), (vec![ListRow::Task(2), ListRow::Task(3)], Some(0)));
        // Hold the last: it stays listed (On hold); snooze it: one row left, and the selection moves up to it.
        m.update(Action::ListMove { delta: 1 });
        m.update(Action::ListHold);
        m.update(Action::ListSnooze { until: 9 });
        assert_eq!((m.list_rows(), m.list), (vec![ListRow::Task(2)], Some(0)));
        m.update(Action::TaskUndo);
        assert_eq!(m.tasks.get(3).map(|t| (t.state, t.snoozed_until)), Some((State::Parked, None)));
        // Opening a task brings its tab forward and gives the keyboard back.
        m.update(Action::ListMove { delta: -1 });
        m.update(Action::ListMove { delta: 1 });
        m.update(Action::ListChoose { axis: Right });
        assert_eq!((m.active, m.mode()), (1, Mode::Normal));
    }

    #[test]
    fn the_selection_follows_its_task_when_others_change_the_inbox() {
        let mut m = Model::default();
        for title in ["a", "b", "c"] {
            m.update(Action::TaskAdd { title: title.into(), priority: 2, state: State::NeedsYou, tab: None });
        }
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        m.update(Action::ListMove { delta: 1 }); // on b
        // An agent files a more urgent task above, and another task leaves: still on b.
        m.update(Action::TaskAdd { title: "urgent".into(), priority: 1, state: State::NeedsYou, tab: None });
        assert_eq!((m.list, m.list_rows()[2].clone()), (Some(2), ListRow::Task(2)));
        m.update(Action::TaskSetState { id: 1, state: State::Cleaned });
        assert_eq!(m.list, Some(1));
        // The user's own hold keeps the place: the next task (c) comes under the selection.
        m.update(Action::ListHold);
        assert_eq!((m.list, m.list_rows()[1].clone()), (Some(1), ListRow::Task(3)));
        // The selected task is taken away by someone else: the selection stays on a row.
        m.update(Action::TaskSetState { id: 3, state: State::Cleaned });
        assert_eq!((m.list, m.list_rows().len()), (Some(1), 2));
        // ↵ opens the task: its tabs are on the bar, and the keyboard goes back to the panes.
        m.update(Action::ListChoose { axis: Right });
        assert_eq!((m.mode(), m.current_task), (Mode::Normal, Some(2)));
    }

    #[test]
    fn list_mode_changes_section_hosts_apps_and_ends_with_the_bar() {
        let mut m = Model::default();
        m.update(Action::SetApps { names: vec!["Notes".into(), "Safari".into()] });
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::FocusList);
        m.update(Action::ListSection { delta: 1 }); // after Repos comes Apps
        m.update(Action::ListMove { delta: 1 });
        assert_eq!((m.sidebar, m.list), (Some(Section::Apps), Some(1)));
        // The shell hosts the app; until it opens a pane, the list keeps the keyboard.
        let host = Effect::HostApp { name: "Safari".into(), axis: Right, new_tab: false };
        assert_eq!((m.update(Action::ListChoose { axis: Right }), m.mode()), (vec![host], Mode::List));
        // Tab passes over Workbook, which has no rows: Apps, then Tasks. Nothing to move to or open there.
        m.update(Action::ListSection { delta: 1 });
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
        assert_eq!((m.mode(), m.sidebar), (Mode::Normal, Some(Section::Repos)));
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
        m.update(Action::ComposeSubmit);
        assert_eq!((m.tasks.list.len(), m.mode()), (0, Mode::Compose));
        // Tab finishes the repository being typed; the name need not match in case.
        m.update(Action::ComposeType { text: "fix zoom #Canvas @AP".into() });
        assert_eq!(m.compose_suggestions(), ["app", "app-2"], "worktrees too");
        m.update(Action::ComposeComplete);
        m.update(Action::ComposeType { text: "p1x".into() });
        m.update(Action::ComposeBackspace);
        assert_eq!(m.compose.as_deref(), Some("fix zoom #Canvas @app p1"));
        m.update(Action::ComposeSubmit);
        let task = &m.tasks.list[0];
        assert_eq!((task.title.as_str(), task.state, task.priority), ("fix zoom", State::Captured, 1));
        assert_eq!(
            (task.project.as_deref(), task.repo.as_deref(), m.mode()),
            (Some("Canvas"), Some("/code/app"), Mode::Normal)
        );
        // A project already used is suggested; an unknown repository stays in the title.
        m.update(Action::OpenCompose { text: "look #c".into() });
        assert_eq!(m.compose_suggestions(), ["Canvas"]);
        m.update(Action::CloseCompose);
        m.update(Action::TaskFromLine { line: "ask @ana about @app-2".into() });
        assert_eq!(
            (m.tasks.list[1].title.as_str(), m.tasks.list[1].repo.as_deref()),
            ("ask @ana about", Some("/code/app-2"))
        );
        // The box holds no more than a title may: a longer task could not be read back.
        m.update(Action::OpenCompose { text: "x".repeat(MAX_TITLE + 50) });
        m.update(Action::ComposeType { text: "yz".into() });
        assert_eq!(m.compose.as_ref().map(|line| line.chars().count()), Some(MAX_TITLE));
        m.update(Action::CloseCompose);
        m.update(Action::TaskFromLine { line: "x".repeat(MAX_TITLE + 1) });
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
        assert_eq!(effects, [Effect::Closed(Pane { id: 1, kind: Kind::Terminal, cwd: None })]);
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
    fn a_pane_moves_only_among_the_tabs_of_the_task_on_show() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::TaskAdd { title: "fix".into(), priority: 3, state: State::Captured, tab: None });
        let task = m.tasks.list[0].id;
        m.update(Action::TaskOpen { id: Some(task) });
        term(&mut m);
        m.update(Action::NewTab);
        term(&mut m);
        // Place 0 on the bar is the task's first tab, not the tab of no task.
        m.update(Action::MovePane { pane: None, tab: 0 });
        assert_eq!(m.tab_panes(), [vec![1], vec![2, 3], vec![]]);
        assert_eq!((m.active, m.current_task), (1, Some(task)));
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
            [Effect::Closed(Pane { id: 1, kind: Kind::Terminal, cwd: None })]
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
        assert_eq!(m.confirm, Some(Confirm { running: vec![running(1, "sleep")] }));
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
    fn a_pane_that_closes_leaves_the_quit_question_and_the_last_one_ends_it() {
        let mut m = Model::default();
        for _ in 0..4 {
            m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        }
        m.update(Action::QuitAsked { running: vec![running(1, "sleep"), running(2, "vim")] });
        m.update(Action::Close { pane: Some(3) });
        assert_eq!(m.confirm.as_ref().map(|c| c.running.len()), Some(2), "pane 3 ran nothing");
        m.update(Action::Close { pane: Some(1) });
        assert_eq!(m.confirm, Some(Confirm { running: vec![running(2, "vim")] }));
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
                let refused = (!user && action.takes_keyboard()).then_some(CONFIRM_UP);
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
        assert_eq!(m.confirm, Some(Confirm { running: vec![running(2, "vim")] }));
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
        let one = Confirm { running: vec![Running { pane: 4, program: None, background: false }] };
        assert_eq!((one.title().as_str(), one.lines()), ("1 terminal is running", vec!["pane 4".to_string()]));
        let job = Confirm { running: vec![Running { pane: 2, program: Some("sleep".into()), background: true }] };
        assert_eq!(job.lines(), ["pane 2 · sleep (background)"]);
        let seven = Confirm { running: (1..=7).map(|pane| running(pane, "claude")).collect() };
        assert_eq!(seven.title(), "7 terminals are running");
        let lines = seven.lines();
        assert_eq!((lines.len(), lines[0].as_str(), lines[5].as_str()), (6, "pane 1 · claude", "and 2 more"));
    }
}
