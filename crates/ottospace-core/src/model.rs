//! Workspace state and its transitions. `Model::update` is the only way state changes.

use serde::{Deserialize, Serialize};

use crate::compose;
use crate::layout::{Axis, Dir, Node};
use crate::list::ListRow;
use crate::repos::{self, Repo};
use crate::tasks::{State, Task, TaskId, Tasks};

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
    /// File a task. `tab`: the `id` of the tab that is its workspace.
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
    /// Bring a task's tab forward (default: the first task that needs the user, "pop next").
    TaskOpen {
        id: Option<TaskId>,
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
}

/// One tab: its own split tree. Only the active tab is drawn.
#[derive(Debug, Serialize)]
pub struct Tab {
    pub id: u32,
    pub title: String,
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
    /// The key reference overlay is showing.
    pub keys: bool,
    /// The terminal pane in visual mode.
    pub visual: Option<PaneId>,
    /// The accessibility debug overlay is showing.
    pub ax_overlay: bool,
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
            keys: false,
            visual: None,
            ax_overlay: false,
            next_id: 0,
            next_tab: 0,
        };
        model.push_tab();
        model
    }
}

impl Model {
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn mode(&self) -> Mode {
        if self.picker.is_some() {
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

    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        let mut effects = Vec::new();
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
                | Action::Enter { .. }
                | Action::ToggleSection { .. }
                | Action::CloseSidebar
                | Action::NewTab
                | Action::SelectTab { .. }
                | Action::CycleTab { .. }
                | Action::CloseTab { .. }
        );
        // The key reference covers the workspace too, so the same actions wait behind it, and the
        // picker cannot open under it.
        let blocked_by_keys = self.keys && (for_panes || matches!(action, Action::OpenPicker { .. }));
        // Visual mode holds the keyboard on its terminal the same way. A close of a named pane still
        // runs: the shell sends it when a window or shell is gone, and must not leave a dead pane.
        let named_close = matches!(action, Action::Close { pane: Some(_) });
        let blocked_by_visual =
            self.visual.is_some() && !named_close && (for_panes || matches!(action, Action::OpenPicker { .. }));
        // The new-task box has the keyboard, as the picker does: what is behind it waits, and
        // neither opens over the other. It does not open under the key reference or in visual mode.
        let overlay = matches!(action, Action::OpenPicker { .. } | Action::OpenCompose { .. });
        let behind_box = (self.picker.is_some() || self.compose.is_some()) && (for_panes || overlay);
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
                | Action::Enter { .. }
                | Action::NewTab
                | Action::SelectTab { .. }
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
            Action::TaskAdd { title, priority, state, tab } => drop(self.tasks.add(title, priority, state, tab)),
            Action::TaskSetState { id, state } => self.tasks.set_state(id, state),
            Action::TaskSnooze { id, until } => self.tasks.snooze(id, until),
            Action::TaskHold { id } => self.tasks.hold(id),
            Action::TaskDismiss { id } => self.tasks.dismiss(id),
            Action::TaskUndo => self.tasks.undo(),
            Action::Tick { now } => self.tasks.tick(now),
            Action::TaskOpen { id } => {
                let task = id.map_or_else(|| self.tasks.next(), |id| self.tasks.get(id));
                let tab = task.and_then(|task| task.tab);
                if let Some(index) = tab.and_then(|tab| self.tabs.iter().position(|t| t.id == tab)) {
                    self.select_tab(index, effects);
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
                let tab = self.tabs.remove(index);
                self.tasks.forget_tab(tab.id);
                for id in tab.layout.as_ref().map(Node::leaves).unwrap_or_default() {
                    self.close_pane(id, effects);
                }
                if self.tabs.is_empty() {
                    self.next_tab = 0;
                    self.push_tab();
                } else {
                    // Stay on the same tab; if the closed one was active, show its right neighbour.
                    let active = if index < self.active { self.active - 1 } else { self.active };
                    self.active = active.min(self.tabs.len() - 1);
                    self.unpark_active();
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
                self.compose = Some(text);
            }
            Action::ComposeType { text } => {
                if let Some(line) = &mut self.compose {
                    line.push_str(&text);
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
                    // A task with no tab opens nothing: the list keeps the keyboard.
                    let tab = self.tasks.get(id).and_then(|task| task.tab);
                    if self.tabs.iter().any(|t| Some(t.id) == tab) {
                        self.list = None;
                    }
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
        }
    }

    /// Files `line` as a captured task. False if it has no title: nothing is filed.
    /// An `@name` that matches no listed repository stays in the title, as typed.
    fn file_line(&mut self, line: &str) -> bool {
        let mut draft = compose::parse(line);
        let repo = draft.repo.take().map(|name| (self.repo_named(&name), name));
        let path = match repo {
            Some((Some(path), _)) => Some(path),
            Some((None, name)) => {
                draft.title = format!("{} @{name}", draft.title).trim().to_string();
                None
            }
            None => None,
        };
        if draft.title.is_empty() {
            return false;
        }
        let id = self.tasks.add(draft.title, draft.priority.unwrap_or(3), State::Captured, None);
        self.tasks.tag(id, draft.project, path);
        true
    }

    /// The path of the listed repository or worktree called `name`, whatever the case of the letters.
    fn repo_named(&self, name: &str) -> Option<String> {
        let listed = self
            .repos
            .iter()
            .flat_map(|r| std::iter::once((&r.name, &r.path)).chain(r.worktrees.iter().map(|w| (&w.name, &w.path))));
        listed.into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, path)| path.clone())
    }

    /// Names that finish the `#project` or `@repo` being typed in the new-task box.
    pub fn compose_suggestions(&self) -> Vec<String> {
        match self.compose.as_deref().and_then(compose::completing) {
            Some(('@', typed)) => compose::suggest(typed, self.repos.iter().map(|r| r.name.as_str())),
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
        self.tabs.push(Tab { id: self.next_tab, title, layout: None, focused: None });
        self.active = self.tabs.len() - 1;
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
        self.park_active();
        self.active = index;
        self.unpark_active();
    }

    /// Removes a pane from whichever tab holds it.
    fn close_pane(&mut self, id: PaneId, effects: &mut Vec<Effect>) {
        let Some(i) = self.panes.iter().position(|p| p.id == id) else { return };
        let closed = self.panes.remove(i);
        self.layout = self.layout.take().and_then(|root| root.remove(id));
        for tab in &mut self.tabs {
            tab.layout = tab.layout.take().and_then(|root| root.remove(id));
            if tab.focused == Some(id) {
                tab.focused = tab.layout.as_ref().and_then(|l| l.leaves().into_iter().max());
            }
        }
        if self.live == Some(id) {
            self.live = None;
        }
        if self.visual == Some(id) {
            self.visual = None;
        }
        if self.focused == Some(id) {
            // The newest pane left in this tab, not in some other tab.
            self.focused = self.layout.as_ref().and_then(|l| l.leaves().into_iter().max());
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

    #[test]
    fn opening_a_task_brings_its_tab_forward() {
        let mut m = Model::default();
        term(&mut m);
        m.update(Action::NewTab);
        let (first, second) = (m.tabs[0].id, m.tabs[1].id);
        m.update(Action::TaskAdd { title: "waits".into(), priority: 2, state: State::NeedsYou, tab: Some(first) });
        m.update(Action::TaskAdd { title: "runs".into(), priority: 1, state: State::Running, tab: Some(second) });
        m.update(Action::TaskAdd { title: "no tab".into(), priority: 1, state: State::Captured, tab: None });
        // Pop next: the task that needs the user, not the more urgent one that is only running.
        m.update(Action::TaskOpen { id: None });
        assert_eq!(m.active, 0);
        m.update(Action::TaskOpen { id: Some(2) });
        assert_eq!(m.active, 1);
        // No tab, a closed tab, or nothing that needs the user: the view stays.
        m.update(Action::TaskOpen { id: Some(3) });
        m.update(Action::CloseTab { index: Some(0) });
        m.update(Action::TaskOpen { id: Some(1) });
        m.update(Action::TaskSetState { id: 1, state: State::Running });
        m.update(Action::TaskOpen { id: None });
        assert_eq!((m.tabs.len(), m.tabs[m.active].id), (1, second));
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
        // ↵ on a task with no tab opens nothing, so the list keeps the keyboard.
        m.update(Action::ListChoose { axis: Right });
        assert_eq!(m.mode(), Mode::List);
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
        assert_eq!(m.compose_suggestions(), ["app"]);
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
        m.update(Action::TaskFromLine { line: "ask about @nowhere".into() });
        assert_eq!((m.tasks.list[1].title.as_str(), m.tasks.list[1].repo.as_deref()), ("ask about @nowhere", None));
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
}
