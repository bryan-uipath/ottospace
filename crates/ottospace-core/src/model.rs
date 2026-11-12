//! Workspace state and its transitions. `Model::update` is the only way state changes.

use serde::{Deserialize, Serialize};

use crate::layout::{Axis, Dir, Node};

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
    /// The pane picker, while it is open.
    pub picker: Option<Picker>,
    /// The key reference overlay is showing.
    pub keys: bool,
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
            picker: None,
            keys: false,
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
        } else if self.live.is_some() {
            Mode::App
        } else {
            Mode::Normal
        }
    }

    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        let mut effects = Vec::new();
        // The picker has the keyboard. Pane and bar actions wait until it closes, so a
        // shortcut cannot change what is behind it (e.g. ⌘W closing the pane it splits).
        let for_panes = matches!(
            action,
            Action::Split { .. }
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
        if self.picker.is_some() && for_panes {
            return effects;
        }
        match action {
            Action::Split { axis, kind } => {
                self.leave(&mut effects);
                let id = self.add(kind, self.focused, axis);
                effects.push(Effect::Opened(id));
            }
            Action::Open { kind, axis, beside } => self.open(kind, axis, beside, &mut effects),
            Action::Adopt { kind, beside } => {
                self.next_id += 1;
                let id = self.next_id;
                self.panes.push(Pane { id, kind });
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
                    self.close_pane(id, &mut effects);
                }
            }
            Action::NewTab => {
                self.leave(&mut effects);
                self.park_active();
                self.push_tab();
            }
            Action::SelectTab { index } => self.select_tab(index, &mut effects),
            Action::CycleTab { delta } => {
                let to = (self.active as i64 + delta as i64).rem_euclid(self.tabs.len() as i64) as usize;
                self.select_tab(to, &mut effects);
            }
            Action::CloseTab { index } => {
                let index = index.unwrap_or(self.active);
                if index >= self.tabs.len() {
                    return effects;
                }
                self.leave(&mut effects);
                self.park_active();
                let tab = self.tabs.remove(index);
                for id in tab.layout.as_ref().map(Node::leaves).unwrap_or_default() {
                    self.close_pane(id, &mut effects);
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
            Action::Focus { pane } => self.focus(pane, &mut effects),
            Action::FocusDir { dir } => {
                let to = self.focused.and_then(|from| self.layout.as_ref()?.neighbor(from, dir));
                if let Some(to) = to {
                    self.focus(to, &mut effects);
                }
            }
            Action::FocusCycle { delta } => {
                let order = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
                let at = self.focused.and_then(|f| order.iter().position(|id| *id == f));
                if let Some(at) = at {
                    let to = (at as i64 + delta as i64).rem_euclid(order.len() as i64) as usize;
                    self.focus(order[to], &mut effects);
                }
            }
            Action::Enter { pane } => {
                let is_app = self.pane(pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
                if is_app && self.live != Some(pane) {
                    // A live window over a tab that does not show its pane would cover other panes.
                    self.show_tab_of(pane, &mut effects);
                    self.leave(&mut effects);
                    self.live = Some(pane);
                    self.focused = Some(pane);
                    effects.push(Effect::Live(pane));
                }
            }
            Action::Leave => self.leave(&mut effects),
            // The bar is ottospace's own UI, so using it takes the keyboard back.
            Action::ToggleSection { section } => {
                self.leave(&mut effects);
                self.sidebar = (self.sidebar != Some(section)).then_some(section);
            }
            Action::OpenPicker { axis, new_tab, items } => {
                self.leave(&mut effects);
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
                let Some(picker) = self.picker.take() else { return effects };
                let choice = picker.matches().get(index.unwrap_or(picker.selected)).copied().cloned();
                match choice {
                    // Same rule as an app: fill the focused pane if it is empty, else split it.
                    Some(PickItem::Terminal) => {
                        if picker.new_tab {
                            self.park_active();
                            self.push_tab();
                        }
                        self.open(Kind::Terminal, picker.axis, None, &mut effects)
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
                self.leave(&mut effects);
                self.keys = !self.keys;
            }
            Action::CloseKeys => self.keys = false,
            Action::CloseSidebar => {
                self.leave(&mut effects);
                self.sidebar = None;
            }
        }
        effects
    }

    /// Shows `kind` in `beside` (default: the focused pane) if that pane is empty; else splits it.
    fn open(&mut self, kind: Kind, axis: Axis, beside: Option<PaneId>, effects: &mut Vec<Effect>) {
        // The new pane takes focus, so the live app must give the keyboard back.
        self.leave(effects);
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
        self.panes.push(Pane { id, kind });
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
        assert_eq!(effects, [Effect::Closed(Pane { id: 2, kind: Kind::Empty })]);
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
        assert_eq!(m.panes, [Pane { id: 1, kind: Kind::Terminal }]);
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
        assert_eq!(effects, [Effect::Closed(Pane { id: 1, kind: Kind::Terminal })]);
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
    fn closing_the_live_pane_returns_to_normal_mode() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        m.update(Action::Close { pane: Some(1) });
        assert_eq!(m.mode(), Mode::Normal);
    }
}
