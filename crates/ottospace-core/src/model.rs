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
}

#[derive(Debug, Default, Serialize)]
pub struct Model {
    pub panes: Vec<Pane>,
    /// `None` while there are no panes.
    pub layout: Option<Node>,
    /// Pane that splits and shortcuts act on.
    pub focused: Option<PaneId>,
    /// App pane that has the keyboard.
    pub live: Option<PaneId>,
    /// Rail section shown in the secondary bar. `None`: the bar is closed.
    pub sidebar: Option<Section>,
    #[serde(skip)]
    next_id: PaneId,
}

impl Model {
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn mode(&self) -> Mode {
        if self.live.is_some() { Mode::App } else { Mode::Normal }
    }

    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        let mut effects = Vec::new();
        match action {
            Action::Split { axis, kind } => {
                self.leave(&mut effects);
                let id = self.add(kind, self.focused, axis);
                effects.push(Effect::Opened(id));
            }
            Action::Open { kind, axis, beside } => {
                // The new pane takes focus, so the live app must give the keyboard back.
                self.leave(&mut effects);
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
            Action::Adopt { kind, beside } => {
                let focused = self.focused;
                let id = self.add(kind, beside, Axis::Right);
                self.focused = focused.or(Some(id));
                effects.push(Effect::Opened(id));
            }
            Action::Close { pane } => {
                let Some(id) = pane.or(self.focused) else { return effects };
                let Some(i) = self.panes.iter().position(|p| p.id == id) else { return effects };
                let closed = self.panes.remove(i);
                self.layout = self.layout.take().and_then(|root| root.remove(id));
                if self.live == Some(id) {
                    self.live = None;
                }
                if self.focused == Some(id) {
                    self.focused = self.panes.last().map(|p| p.id);
                }
                effects.push(Effect::Closed(closed));
            }
            Action::Focus { pane } => self.focus(pane, &mut effects),
            Action::FocusDir { dir } => {
                let to = self.focused.and_then(|from| self.layout.as_ref()?.neighbor(from, dir));
                if let Some(to) = to {
                    self.focus(to, &mut effects);
                }
            }
            Action::Enter { pane } => {
                let is_app = self.pane(pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
                if is_app && self.live != Some(pane) {
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
            Action::CloseSidebar => {
                self.leave(&mut effects);
                self.sidebar = None;
            }
        }
        effects
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

    #[test]
    fn closing_the_live_pane_returns_to_normal_mode() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(Action::Enter { pane: 1 });
        m.update(Action::Close { pane: Some(1) });
        assert_eq!(m.mode(), Mode::Normal);
    }
}
