//! Displays, and tabs folded out to them: a tab out shows in its own window on another display,
//! one per display, never the main window's. It stays in its space and its place in the tab row.
//! The keyboard is in one window: `active` is its tab; `away`, while that is a tab out, the main
//! window's. See `docs/ai/multi-monitor.md`.

use serde::{Deserialize, Serialize};

use super::{Action, Effect, Kind, Model, Owner, spaces::HOME_TAB};
use crate::ax::Frame;
use crate::features::Feature;

/// Why a client's fold, out or in, is refused: which display shows what is the user's layout.
pub const USERS_ALONE: &str = "only the user folds a tab out to another display, or back in";
/// Why anyone but the shell's watcher may not say what the displays are.
pub const SHELLS_ALONE: &str = "only ottospace reads the displays";
/// ⌘⇧O with no display but the main window's.
pub const NO_OTHER: &str = "No other display";
/// ⌘⇧O on Home.
pub const HOME_STAYS: &str = "Home stays in the main window";
/// ⌘⇧O on a tab that holds an app pane: hosted windows follow in a later slice.
pub const APPS_STAY: &str = "A tab with an app pane stays in the main window";

/// The capture key on a pane in a tab out.
pub const CAPTURE_HERE: &str = "Capture a pane in the main window: fold its tab back in first";

/// A display, as the shell read it. `uid` is its UUID: the same monitor keeps it across
/// unplugs and restarts. Never in the state: it names the user's hardware.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Display {
    pub uid: String,
    /// Global, top-left origin, in points (the AX space).
    pub frame: Frame,
    pub scale: f32,
}

/// A tab folded out, and the display it is on. `stand_in`: the empty tab made for its space in
/// the main window when it was that space's only one (its id and title), removed when the tab
/// folds back in if still empty and not renamed.
#[derive(Clone, Debug, PartialEq)]
pub struct Out {
    pub tab: u32,
    pub display: String,
    pub stand_in: Option<(u32, String)>,
}

/// Which window holds the keyboard: the main one, or a tab out's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Window {
    Main,
    Out(u32),
}

impl Model {
    /// The displays, left to right, then top to bottom.
    pub fn displays(&self) -> &[Display] {
        &self.displays
    }

    /// The place in `displays` of the main window's display.
    pub fn main_display(&self) -> Option<usize> {
        self.displays.iter().position(|d| Some(&d.uid) == self.main_display.as_ref())
    }

    /// The tabs out, by id, and the place in `displays` of each one's display.
    pub fn tabs_out(&self) -> Vec<(u32, usize)> {
        let place = |uid: &String| self.displays.iter().position(|d| d.uid == *uid);
        self.out.iter().filter_map(|out| Some((out.tab, place(&out.display)?))).collect()
    }

    /// Whether tab `id` is out on another display.
    pub fn is_out(&self, id: u32) -> bool {
        self.out.iter().any(|out| out.tab == id)
    }

    /// The place in `tabs` of the tab the main window shows: `active`, unless the keyboard is
    /// in a tab out's window.
    pub fn main_at(&self) -> usize {
        let away = self.away.and_then(|id| self.tabs.iter().position(|tab| tab.id == id));
        away.unwrap_or(self.active)
    }

    /// The main window's space: its top row's selection, its tab row, its drawer.
    pub fn shown_space(&self) -> &Owner {
        &self.tabs[self.main_at().min(self.tabs.len() - 1)].space
    }

    /// The tree of the tab at `at`: the live one for `active`, else the one it keeps.
    pub fn tree_at(&self, at: usize) -> Option<&super::Node> {
        if at == self.active { self.layout.as_ref() } else { self.tabs.get(at)?.layout.as_ref() }
    }

    /// The focused pane of the tab at `at`.
    pub fn focused_at(&self, at: usize) -> Option<super::PaneId> {
        if at == self.active { self.focused } else { self.tabs.get(at)?.focused }
    }

    /// The panes the main window shows. App panes are only ever there (`APPS_STAY`).
    pub fn main_panes(&self) -> Vec<super::PaneId> {
        self.tree_at(self.main_at()).map(super::Node::leaves).unwrap_or_default()
    }

    /// The panes some window shows: the main window's, and each tab out's.
    pub fn shown_panes(&self) -> Vec<super::PaneId> {
        let out = self.out.iter().filter_map(|out| self.tabs.iter().position(|tab| tab.id == out.tab));
        let trees = std::iter::once(self.main_at()).chain(out).filter_map(|at| self.tree_at(at));
        trees.flat_map(super::Node::leaves).collect()
    }

    /// The place in `tabs` of tab `id`, if it is out.
    pub fn out_at(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id).filter(|_| self.is_out(id))
    }

    fn window(&self) -> Window {
        match self.away {
            Some(_) => Window::Out(self.tabs[self.active].id),
            None => Window::Main,
        }
    }

    fn holds_app(&self, at: usize) -> bool {
        let panes = self.tree_at(at).map(super::Node::leaves).unwrap_or_default();
        panes.iter().any(|id| self.pane(*id).is_some_and(|p| matches!(p.kind, Kind::App { .. })))
    }

    /// Whether display `uid` may hold a tab out: listed, not the main window's, with a UUID.
    fn other_display(&self, uid: &str) -> bool {
        !uid.is_empty() && self.main_display.as_deref() != Some(uid) && self.displays.iter().any(|d| d.uid == uid)
    }

    /// Moves the keyboard to the main window. Its tab does not change.
    pub(crate) fn key_main(&mut self, effects: &mut Vec<Effect>) {
        let Some(id) = self.away.take() else { return };
        let to = self.tabs.iter().position(|tab| tab.id == id).unwrap_or(self.active);
        self.key_to(to, effects);
    }

    /// Makes tab `at` the keyboard's, the windows showing what they did. The keyboard leaves
    /// the main window's list and rails.
    fn key_to(&mut self, at: usize, effects: &mut Vec<Effect>) {
        if at != self.active {
            (self.list, self.rail_focus) = (None, None);
        }
        self.select_tab(at, effects);
    }

    /// The tab the main window shows instead of the one at `at`, gone out: its space's next tab
    /// that is not out, else its previous; else the last space selected that has one; else Home.
    fn main_after(&self, at: usize) -> u32 {
        let free = |i: &usize| *i != at && !self.is_out(self.tabs[*i].id) && self.tabs[*i].id != HOME_TAB;
        let of = &self.tabs[at].space;
        let mine: Vec<usize> = self.space_tabs(of).into_iter().filter(free).collect();
        let next = mine.iter().copied().find(|i| *i > at).or(mine.last().copied());
        let last = || {
            let spaces = self.last_spaces.iter().rev().filter(|space| *space != of);
            spaces.flat_map(|space| self.space_tabs(space)).find(free)
        };
        next.or_else(last).map_or(HOME_TAB, |i| self.tabs[i].id)
    }

    /// The display a tab folds out to: its own last one, if free; else the next free one after
    /// the main window's, left to right; else that next one, whose tab folds in.
    fn display_for(&self, at: usize) -> Option<String> {
        let free = |uid: &str| self.other_display(uid) && !self.out.iter().any(|out| out.display == uid);
        if let Some(uid) = self.tabs[at].display.as_deref().filter(|uid| free(uid)) {
            return Some(uid.into());
        }
        let main = self.main_display()?;
        let n = self.displays.len();
        let order: Vec<&str> = (1..n).map(|step| self.displays[(main + step) % n].uid.as_str()).collect();
        let order: Vec<&str> = order.into_iter().filter(|uid| self.other_display(uid)).collect();
        order.iter().find(|uid| free(uid)).or(order.first()).map(|uid| uid.to_string())
    }

    /// ⌘⇧O: tab `id` (default: the keyboard's) out to another display, the keyboard with it.
    /// A tab out folds in instead.
    pub(crate) fn tab_out(&mut self, id: Option<u32>, effects: &mut Vec<Effect>) {
        // The new-task box, or a draft in the bar, holds the keyboard in the main window.
        if self.compose.is_some() {
            return;
        }
        let id = id.unwrap_or(self.tabs[self.active].id);
        if self.is_out(id) {
            return self.tab_in(Some(id), effects);
        }
        let Some(at) = self.tabs.iter().position(|tab| tab.id == id) else { return };
        // A closing workspace's tab stays in: only its Undo brings it back.
        if self.closing(&self.tabs[at].space) {
            return;
        }
        let notice = |text: &str| Effect::Notice { text: text.into() };
        if id == HOME_TAB {
            return effects.push(notice(HOME_STAYS));
        }
        if self.holds_app(at) {
            return effects.push(notice(APPS_STAY));
        }
        let Some(display) = self.display_for(at) else { return effects.push(notice(NO_OTHER)) };
        // A stand-in folded out is the user's tab now: no fold in takes it away.
        for out in self.out.iter_mut().filter(|out| out.stand_in.as_ref().is_some_and(|(tab, _)| *tab == id)) {
            out.stand_in = None;
        }
        // The display's tab, if it shows one, folds in: the two swap. Its stand-in may go: places move.
        let was = self.out.iter().find(|out| out.display == display).map(|out| out.tab);
        if let Some(was) = was {
            self.tab_in(Some(was), effects);
        }
        let Some(at) = self.tabs.iter().position(|tab| tab.id == id) else { return };
        let main = self.tabs[self.main_at()].id;
        // Its space's only tab in the main window: an empty one stands in for it there, so the
        // space keeps a tab on show (its empty state).
        let free = |i: &usize| *i != at && !self.is_out(self.tabs[*i].id) && self.tabs[*i].id != HOME_TAB;
        let alone = main == id && !self.space_tabs(&self.tabs[at].space).iter().any(free);
        let stand_in = alone.then(|| self.stand_in(at));
        let main = match stand_in {
            Some((tab, _)) => tab,
            // A swap brings the tab coming in to the main window if it is of the same space: the
            // main window keeps its space (else another tab of it).
            None if main == id => was
                .filter(|was| self.space_of(*was) == Some(&self.tabs[at].space))
                .unwrap_or_else(|| self.main_after(at)),
            None => main,
        };
        self.out.push(Out { tab: id, display: display.clone(), stand_in });
        self.claim(at, &display);
        // Its space on the row: the main window's row lists it, marked out.
        let of = self.tabs[at].space.clone();
        self.add_space(&of);
        self.key_to(at, effects);
        self.away = Some(main);
    }

    fn space_of(&self, id: u32) -> Option<&Owner> {
        self.tabs.iter().find(|tab| tab.id == id).map(|tab| &tab.space)
    }

    /// A new empty tab right after tab `at`, in its space and folder, not put on show: its id and title.
    fn stand_in(&mut self, at: usize) -> (u32, String) {
        self.next_tab += 1;
        let (id, title) = (self.next_tab, self.free_title());
        let (space, folder) = (self.tabs[at].space.clone(), self.tabs[at].folder.clone());
        let tab = super::Tab { id, title: title.clone(), space, layout: None, focused: None, display: None, folder };
        self.tabs.insert(at + 1, tab);
        self.active += usize::from(self.active > at);
        (id, title)
    }

    /// Tab `out` folded back in: its stand-in goes if it is still empty and not renamed, and the
    /// main window shows the tab back instead. Filled or renamed, both stay.
    fn drop_stand_in(&mut self, out: &Out, effects: &mut Vec<Effect>) {
        let Some((id, title)) = &out.stand_in else { return };
        let Some(at) = self.tabs.iter().position(|tab| tab.id == *id) else { return };
        let untouched = self.tree_at(at).is_none() && self.tabs[at].title == *title && !self.is_out(*id);
        let Some(back) = self.tabs.iter().position(|tab| tab.id == out.tab).filter(|_| untouched) else { return };
        if self.main_at() == at {
            match self.away {
                Some(_) => self.away = Some(out.tab),
                None => self.select_tab(back, effects),
            }
        }
        self.close_background_tab(at, effects);
    }

    /// The tab out that tab `id` stands in for, while it is still empty and not renamed: a saved
    /// layout leaves it out, as the tab out folds in at the next start.
    pub(crate) fn idle_stand_in(&self, id: u32) -> Option<u32> {
        let out = self.out.iter().find(|out| out.stand_in.as_ref().is_some_and(|(tab, _)| *tab == id))?;
        let (at, title) = (self.tabs.iter().position(|tab| tab.id == id)?, &out.stand_in.as_ref()?.1);
        (self.tree_at(at).is_none() && self.tabs[at].title == *title).then_some(out.tab)
    }

    /// Tab `at` is display `uid`'s: no other tab is taken back there on a replug.
    fn claim(&mut self, at: usize, uid: &str) {
        for tab in self.tabs.iter_mut().filter(|tab| tab.display.as_deref() == Some(uid)) {
            tab.display = None;
        }
        self.tabs[at].display = Some(uid.into());
    }

    /// Fold back in: tab `id` (default: the keyboard's, if out) is an ordinary tab again, not put
    /// on show. The keyboard, if it was there, goes to the main window.
    pub(crate) fn tab_in(&mut self, id: Option<u32>, effects: &mut Vec<Effect>) {
        let Some(id) = id.or(self.away.map(|_| self.tabs[self.active].id)) else { return };
        let Some(place) = self.out.iter().position(|out| out.tab == id) else { return };
        if self.tabs[self.active].id == id {
            self.key_main(effects);
        }
        let out = self.out.remove(place);
        self.drop_stand_in(&out, effects);
        // The user's fold in: no display takes it back by itself.
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.display = None;
        }
    }

    /// The user is in the window showing tab `id`: the keyboard goes there.
    pub(crate) fn key_window(&mut self, id: u32, effects: &mut Vec<Effect>) {
        let Some(at) = self.tabs.iter().position(|tab| tab.id == id).filter(|_| self.compose.is_none()) else {
            return;
        };
        if !self.is_out(id) {
            // The main window's: any other tab is no window's.
            return if at == self.main_at() {
                self.key_main(effects)
            };
        }
        let main = self.tabs[self.main_at()].id;
        self.key_to(at, effects);
        self.away = Some(main);
    }

    /// The displays the shell read. A tab out on one gone, or on the main window's, folds in and
    /// keeps its display; a display back takes back its tab, unless the main window shows it.
    pub(crate) fn set_displays(&mut self, mut displays: Vec<Display>, main: Option<String>) {
        displays
            .sort_by(|a, b| (a.frame.x, a.frame.y).partial_cmp(&(b.frame.x, b.frame.y)).unwrap_or(a.uid.cmp(&b.uid)));
        let before: Vec<String> = self.displays.iter().map(|d| d.uid.clone()).collect();
        (self.displays, self.main_display) = (displays, main);
        // Folds in by `settle_out`: the display is gone.
        let back: Vec<String> = (self.displays.iter())
            .map(|d| d.uid.clone())
            .filter(|uid| !before.contains(uid) && self.other_display(uid))
            .collect();
        for uid in back {
            let taken = |model: &Self| model.out.iter().any(|out| out.display == uid);
            let main = self.tabs[self.main_at()].id;
            let at = (0..self.tabs.len()).find(|at| {
                let tab = &self.tabs[*at];
                tab.display.as_ref() == Some(&uid) && !self.is_out(tab.id) && tab.id != main && !self.holds_app(*at)
            });
            if let Some(at) = at.filter(|_| !taken(self)) {
                self.out.push(Out { tab: self.tabs[at].id, display: uid.clone(), stand_in: None });
                self.claim(at, &uid);
            }
        }
    }

    /// After every step: each tab out is a tab on a display other than the main window's, one per
    /// display, holding no app pane, and the flag on; any other folds in. `away` is the main
    /// window's tab exactly while the keyboard is on a tab out. Only the user moves the keyboard
    /// between windows; nobody's step leaves it in the main window's chrome while it is out.
    pub(crate) fn settle_out(&mut self, from: (u32, Window, Owner), user: bool, effects: &mut Vec<Effect>) {
        let on = self.features.on(Feature::MultiMonitor);
        let mut seen: Vec<String> = Vec::new();
        let mut gone = Vec::new();
        for out in std::mem::take(&mut self.out) {
            let at = self.tabs.iter().position(|tab| tab.id == out.tab);
            let keeps = on
                && out.tab != HOME_TAB
                && self.other_display(&out.display)
                && !seen.contains(&out.display)
                && !self.out.iter().any(|kept| kept.tab == out.tab)
                && at.is_some_and(|at| !self.holds_app(at))
                // A closing workspace's tab folds in: nothing but its Undo or time changes it.
                && at.is_some_and(|at| !self.closing(&self.tabs[at].space));
            if keeps {
                seen.push(out.display.clone());
                self.out.push(out);
            } else {
                gone.push(out);
            }
        }
        // The keyboard on a tab folded in goes to the main window.
        if gone.iter().any(|out| out.tab == self.tabs[self.active].id) {
            self.key_main(effects);
        }
        // A hide changes no tab: a closing workspace keeps its stand-in until its Undo brings it back
        // (then an empty one goes) or it ends.
        for out in gone {
            match self.tabs.iter().find(|tab| tab.id == out.tab) {
                Some(tab) if self.closing(&tab.space) => self.folded_closing.push(out),
                _ => self.drop_stand_in(&out, effects),
            }
        }
        for out in std::mem::take(&mut self.folded_closing) {
            match self.tabs.iter().find(|tab| tab.id == out.tab) {
                Some(tab) if self.closing(&tab.space) => self.folded_closing.push(out),
                Some(_) => self.drop_stand_in(&out, effects),
                None => {}
            }
        }
        let main = |model: &Self, id: u32| model.tabs.iter().any(|tab| tab.id == id && !model.is_out(id));
        match (self.is_out(self.tabs[self.active].id), self.away) {
            // A tab out put on show (a click on it in the row): the keyboard went to its window,
            // the main window shows what it did.
            (true, None) => {
                self.away = Some(if main(self, from.0) { from.0 } else { self.instead(&from.2) });
            }
            // The main window's tab closed: another of its space, else Home.
            (true, Some(id)) if !main(self, id) => self.away = Some(self.instead(&from.2)),
            // The keyboard's tab out closed: the keyboard goes to the main window, as it was.
            (false, Some(_)) if matches!(from.1, Window::Out(id) if !self.tabs.iter().any(|tab| tab.id == id)) => {
                self.key_main(effects)
            }
            // Another tab put on show from a tab out's window: the main window shows it.
            (false, Some(_)) => self.away = None,
            _ => {}
        }
        // Only the user's move takes the keyboard to another window.
        if !user && self.window() != from.1 {
            match from.1 {
                Window::Main => self.key_main(effects),
                Window::Out(id) if self.is_out(id) => self.key_window(id, effects),
                Window::Out(_) => {}
            }
        }
        // The main window's own parts with the keyboard: the keyboard is in the main window. A
        // new-task box opened from a tab out's window starts its task in that tab's workspace.
        // A capture's draft is no such box: its task goes where the user is.
        if self.compose.is_none() || self.capture.is_some() {
            self.compose_joins = None;
        } else if self.away.is_some() && self.compose_joins.is_none() {
            self.compose_joins = Some(self.space().clone());
        }
        // A client's step that took the list or a rail from a window out is undone: it never moves
        // the keyboard between windows. (The new-task box goes to the main window, the keyboard with it.)
        if !user && self.away.is_some() {
            (self.list, self.rail_focus) = (None, None);
            if let Some(talk) = &mut self.talk {
                talk.keys = false;
            }
        }
        // So do the boxes and modes it draws: the picker, the palette, ⌘/, a mark's name, the quit question.
        let talking = self.talk.as_ref().is_some_and(|talk| talk.keys);
        let boxed = self.picker.is_some() || self.palette.is_some() || self.keys || self.confirm.is_some();
        let boxed = boxed || self.mark_input.is_some();
        if talking || boxed || self.list.is_some() || self.rail_focus.is_some() || self.compose.is_some() {
            self.key_main(effects);
        }
    }

    /// Where the keyboard is before a step, for `settle_out`: the main window's tab, the window, and
    /// the main window's space.
    pub(crate) fn out_from(&self) -> (u32, Window, Owner) {
        (self.tabs[self.main_at()].id, self.window(), self.shown_space().clone())
    }

    /// A tab of space `of` for the main window, none out: its first, else Home's.
    fn instead(&self, of: &Owner) -> u32 {
        let here = self.space_tabs(of).into_iter().map(|at| self.tabs[at].id).find(|id| !self.is_out(*id));
        here.unwrap_or(HOME_TAB)
    }

    /// A capture of a pane out: not drawn there yet. Dropped, with a notice.
    pub(crate) fn captures_out(&self, action: &Action) -> bool {
        let Action::CaptureStart { pane, .. } = action else { return false };
        pane.or(self.focused).and_then(|id| self.tab_of(id)).is_some_and(|tab| self.is_out(tab))
    }

    /// From a tab out's window, the main window's tab row: ⌘T and ⌘} act there, the keyboard
    /// going with them; ⌘⇧W closes the tab out, the main window's tab staying as it was.
    pub(crate) fn main_first(&mut self, action: Action, effects: &mut Vec<Effect>) -> Action {
        if self.away.is_none() {
            return action;
        }
        match action {
            Action::NewTab | Action::NewEmptyTab | Action::CycleTab { .. } => {
                // A new tab opens in the main window, carrying the checkout of the tab the key
                // was pressed in.
                if matches!(action, Action::NewTab | Action::NewEmptyTab) {
                    self.new_tab_from = Some(self.tabs.get(self.active).and_then(|tab| tab.folder.clone()));
                }
                self.key_main(effects);
                action
            }
            Action::CloseTab { index } if index.is_none_or(|at| at == self.active) => {
                let id = self.tabs[self.active].id;
                self.key_main(effects);
                Action::CloseTab { index: self.tabs.iter().position(|tab| tab.id == id) }
            }
            action => action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Axis, Origin, PaneId};

    fn display(uid: &str, x: f64) -> Display {
        Display { uid: uid.into(), frame: Frame::new(x, 0., 1728., 1117.), scale: 2. }
    }

    fn set(m: &mut Model, displays: &[Display], main: &str) {
        let action = Action::SetDisplays { displays: displays.to_vec(), main: Some(main.into()) };
        m.update(action.sent_by(Origin::System));
    }

    /// Two displays side by side, the main window on the left one; a workspace of tabs 1 and 2,
    /// a terminal in each, tab 2 on show.
    fn two() -> Model {
        let mut m = Model::default();
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "left");
        m.update(Action::NewEmptyTab);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::NewEmptyTab);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m
    }

    fn id(m: &Model) -> u32 {
        m.tabs[m.active].id
    }

    fn main_id(m: &Model) -> u32 {
        m.tabs[m.main_at()].id
    }

    #[test]
    fn a_tab_folds_out_with_the_keyboard_and_the_main_window_shows_its_neighbour() {
        let mut m = two();
        assert_eq!(id(&m), 2);
        m.update(Action::TabOut { tab: None });
        assert_eq!(m.tabs_out(), [(2, 1)]);
        // The keyboard went with it; the main window shows the workspace's other tab.
        assert_eq!((id(&m), main_id(&m)), (2, 1));
        assert_eq!(m.shown_space(), &m.tabs[0].space);
        // ⌘⇧O again folds it back in: not put on show, the keyboard back in the main window.
        m.update(Action::TabOut { tab: None });
        assert_eq!((m.tabs_out(), id(&m), m.away), (vec![], 1, None));
        assert_eq!(m.tabs[1].display, None);
    }

    /// ⌘T pressed in a folded-out window: the new tab opens in the main window (as before) and
    /// carries the checkout of the folded-out tab, not the main window's.
    #[test]
    fn a_new_tab_from_a_folded_out_window_carries_its_tabs_checkout() {
        let mut m = two();
        (m.tabs[0].folder, m.tabs[1].folder) = (Some("/code/app".into()), Some("/code/web".into()));
        m.update(Action::TabOut { tab: None });
        assert_eq!((id(&m), main_id(&m)), (2, 1));
        m.update(Action::NewTab);
        assert_eq!(m.active, m.main_at(), "in the main window");
        assert_eq!(m.tabs[m.active].folder.as_deref(), Some("/code/web"));
        // From the main window: its tab's, as before.
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::NewTab);
        assert_eq!(m.tabs[m.active].folder.as_deref(), Some("/code/app"));
    }

    /// A workspace of one tab, a terminal in it.
    fn one() -> Model {
        let mut m = Model::default();
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "left");
        m.update(Action::NewEmptyTab);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m
    }

    fn space_tabs(m: &Model) -> Vec<u32> {
        m.space_tabs(&m.tabs[m.active].space).into_iter().map(|at| m.tabs[at].id).collect()
    }

    /// Its only tab folded out: before, the main window went to Home and the workspace had no tab
    /// there. Now an empty tab of the workspace stands in; folded back in, the tab takes its place.
    #[test]
    fn the_only_tab_out_leaves_an_empty_one_in_its_place_until_it_folds_back_in() {
        let mut m = one();
        let space = m.tabs[m.active].space.clone();
        m.update(Action::TabOut { tab: None });
        assert_eq!((id(&m), m.tabs_out().len()), (1, 1), "out, with the keyboard");
        let stand_in = main_id(&m);
        assert_ne!(stand_in, 1);
        assert_eq!((m.shown_space(), space_tabs(&m)), (&space, vec![1, stand_in]));
        assert!(m.tree_at(m.main_at()).is_none() && m.home(&[]).is_none(), "its empty state, not Home");
        // In the main window, the keyboard is on the stand-in; ⌘⇧O from the window out folds in.
        m.update(Action::KeyWindow { tab: stand_in });
        assert_eq!((id(&m), m.focused), (stand_in, None));
        m.update(Action::KeyWindow { tab: 1 });
        m.update(Action::TabOut { tab: None });
        assert_eq!((m.tabs_out(), id(&m), space_tabs(&m)), (vec![], 1, vec![1]));
        // `one`'s split filled its lone empty pane (#413): one leaf.
        assert_eq!(m.tree_at(m.active).map(crate::layout::Node::leaves), Some(vec![1]));
        // From the main window's row, too.
        m.update(Action::TabOut { tab: None });
        let stand_in = main_id(&m);
        m.update(Action::KeyWindow { tab: stand_in });
        m.update(Action::TabIn { tab: Some(1) });
        assert_eq!((id(&m), space_tabs(&m)), (1, vec![1]));
    }

    /// Filled, or folded in by an unplug: a used stand-in stays; an empty one goes.
    #[test]
    fn a_filled_stand_in_stays_and_an_unplug_folds_in_as_the_user_does() {
        let mut m = one();
        m.update(Action::TabOut { tab: None });
        let stand_in = main_id(&m);
        m.update(Action::KeyWindow { tab: stand_in });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::TabIn { tab: Some(1) });
        assert_eq!((id(&m), space_tabs(&m)), (stand_in, vec![1, stand_in]));
        let mut m = one();
        m.update(Action::TabOut { tab: None });
        set(&mut m, &[display("left", 0.)], "left");
        assert_eq!((m.tabs_out(), id(&m), space_tabs(&m)), (vec![], 1, vec![1]));
    }

    /// A swap (two displays): the tab coming in drops its empty stand-in, and the one going out,
    /// its space's only tab, gets one. Places move under the swap; the right tab goes out.
    #[test]
    fn a_swap_moves_the_stand_in_to_the_space_whose_only_tab_goes_out() {
        let mut m = one();
        m.update(Action::TabOut { tab: None });
        let first = main_id(&m);
        m.update(Action::KeyWindow { tab: first });
        // Another workspace, its only tab a terminal, folded out over the first.
        m.update(Action::SpaceSelect { index: 0 });
        m.update(Action::NewEmptyTab);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let (second, space) = (id(&m), m.tabs[m.active].space.clone());
        m.update(Action::TabOut { tab: None });
        assert_eq!((id(&m), m.tabs_out().iter().map(|(tab, _)| *tab).collect::<Vec<_>>()), (second, vec![second]));
        assert!(!m.tabs.iter().any(|tab| tab.id == first), "the first stand-in went with its tab back in");
        assert_eq!(m.shown_space(), &space, "the second space keeps the main window");
        assert!(m.tree_at(m.main_at()).is_none() && main_id(&m) != second);
    }

    /// A stand-in folded out (three displays) is the user's tab: folding the first one back in
    /// takes nothing away, and that tab's window stays.
    #[test]
    fn a_stand_in_folded_out_is_kept() {
        let mut m = one();
        set(&mut m, &[display("left", 0.), display("mid", 1728.), display("right", 3456.)], "left");
        m.update(Action::TabOut { tab: None });
        let stand_in = main_id(&m);
        m.update(Action::KeyWindow { tab: stand_in });
        m.update(Action::TabOut { tab: None });
        assert_eq!(m.tabs_out().len(), 2);
        m.update(Action::TabIn { tab: Some(1) });
        assert!(m.is_out(stand_in) && m.tabs.iter().any(|tab| tab.id == 1));
    }

    /// Saved while out: the empty stand-in is not kept, and the tab is the one on show at the next start.
    #[test]
    fn a_saved_layout_keeps_the_tab_out_not_its_empty_stand_in() {
        let mut m = one();
        m.update(Action::TabOut { tab: None });
        let stand_in = main_id(&m);
        m.update(Action::KeyWindow { tab: stand_in });
        assert_eq!(m.idle_stand_in(stand_in), Some(1));
        let saved = m.layout_file(&[], &[], 0);
        assert_eq!(saved.tabs.len(), 1, "{saved:?}");
        assert_eq!(saved.selected, Some(0));
        // Filled, it is a tab of its own.
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(m.idle_stand_in(stand_in), None);
    }

    /// A temporary workspace closed while its only tab is out: the tab folds in, its empty
    /// stand-in kept while it closes; its Undo brings back the tab alone. A fold out of a
    /// closing workspace's tab does nothing.
    #[test]
    fn a_closing_workspaces_tab_folds_in_and_its_undo_brings_no_stand_in() {
        let mut m = one();
        let space = m.tabs[m.active].space.clone();
        let Owner::Workbook { id: book } = space else { panic!("a workspace") };
        m.update(Action::TabOut { tab: None });
        let stand_in = main_id(&m);
        m.update(Action::KeyWindow { tab: stand_in });
        m.update(Action::SpaceHide { index: None });
        assert!(m.workbooks.closing(book), "closing, its Undo offered");
        assert_eq!((m.tabs_out(), m.away), (vec![], None));
        assert_eq!(space_tabs_of(&m, &space), [1, stand_in], "a hide changes no tab");
        m.update(Action::TabOut { tab: Some(1) });
        assert!(m.tabs_out().is_empty() && m.workbooks.closing(book), "its tab stays in");
        m.update(Action::WorkbookReopen { workbook: book });
        assert_eq!(space_tabs_of(&m, &space), [1]);
    }

    fn space_tabs_of(m: &Model, of: &Owner) -> Vec<u32> {
        m.space_tabs(of).into_iter().map(|at| m.tabs[at].id).collect()
    }

    #[test]
    fn the_main_windows_space_takes_the_tab_it_shows_once_the_tabs_out_settle() {
        let mut m = two();
        m.update(Action::NewEmptyTab);
        m.update(Action::TabOut { tab: None });
        // A client's jump to a mark of a tab with nothing in it: the tab made for it goes, and the
        // main window shows a tab of its workspace again.
        let marks = r#"[{"id":"name:a","name":"a","target":{"tab":{"title":"app","task":null,"focused":null,"layout":null,"panes":[]}}}]"#;
        let marks = serde_json::from_str(marks).expect("a mark");
        m.update(Action::RestoreMarks { marks, extra: Default::default() }.sent_by(Origin::System));
        m.update(Action::MarkGo { mark: crate::model::MarkRef::Place(0), axis: Axis::Down }.sent_by(Origin::Socket));
        let last = m.spaces.iter().find(|space| space.of == m.tabs[m.main_at()].space).and_then(|space| space.last);
        assert_eq!(last, Some(main_id(&m)));
    }

    #[test]
    fn fold_back_in_from_the_main_windows_row() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::KeyWindow { tab: 1 });
        assert_eq!((id(&m), m.away), (1, None));
        m.update(Action::TabIn { tab: Some(2) });
        assert_eq!((m.tabs_out(), id(&m)), (vec![], 1));
    }

    #[test]
    fn one_display_home_and_app_panes_stay_in_the_main_window() {
        let mut m = Model::default();
        set(&mut m, &[display("only", 0.)], "only");
        let told = |effects: Vec<Effect>| {
            effects.into_iter().find_map(|e| match e {
                Effect::Notice { text } => Some(text),
                _ => None,
            })
        };
        assert_eq!(told(m.update(Action::TabOut { tab: None })).as_deref(), Some(HOME_STAYS));
        m.update(Action::NewEmptyTab);
        assert_eq!(told(m.update(Action::TabOut { tab: None })).as_deref(), Some(NO_OTHER));
        set(&mut m, &[display("only", 0.), display("other", 1728.)], "only");
        let app = Kind::App { window: 9, title: "w".into(), bundle: None, name: "Slack".into() };
        m.update(Action::Open { kind: app, axis: Axis::Right, beside: None });
        assert_eq!(told(m.update(Action::TabOut { tab: None })).as_deref(), Some(APPS_STAY));
        assert!(m.tabs_out().is_empty());
    }

    #[test]
    fn only_the_user_folds_and_only_the_shell_says_the_displays() {
        let mut m = two();
        m.update(Action::TabOut { tab: None }.sent_by(Origin::Socket));
        m.update(Action::TabOut { tab: None }.sent_by(Origin::System));
        assert!(m.tabs_out().is_empty());
        m.update(Action::SetDisplays { displays: vec![], main: None }.sent_by(Origin::Socket));
        assert_eq!(m.displays().len(), 2);
        m.update(Action::TabOut { tab: None });
        m.update(Action::TabIn { tab: None }.sent_by(Origin::Socket));
        m.update(Action::KeyWindow { tab: 1 }.sent_by(Origin::Socket));
        assert_eq!((m.tabs_out(), id(&m)), (vec![(2, 1)], 2));
    }

    #[test]
    fn unplugged_the_tab_folds_in_and_plugged_in_again_it_goes_back() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        set(&mut m, &[display("left", 0.)], "left");
        // Folded in; the keyboard in the main window, which shows what it did.
        assert_eq!((m.tabs_out(), id(&m), m.away), (vec![], 1, None));
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "left");
        assert_eq!(m.tabs_out(), [(2, 1)]);
        // The keyboard stays where it is.
        assert_eq!((id(&m), m.away), (1, None));
        // The same list again moves nothing.
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "left");
        assert_eq!(m.tabs_out(), [(2, 1)]);
    }

    #[test]
    fn plugged_in_again_while_the_main_window_shows_it_it_stays() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        set(&mut m, &[display("left", 0.)], "left");
        m.update(Action::SelectTab { index: 1 });
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "left");
        assert!(m.tabs_out().is_empty());
        // ⌘⇧O sends it where it was.
        m.update(Action::TabOut { tab: None });
        assert_eq!(m.tabs_out(), [(2, 1)]);
    }

    #[test]
    fn folded_in_by_the_user_a_replug_leaves_it() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::TabIn { tab: None });
        set(&mut m, &[display("left", 0.)], "left");
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "left");
        assert!(m.tabs_out().is_empty());
    }

    #[test]
    fn the_main_window_moved_onto_the_tabs_display_folds_it_in() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        set(&mut m, &[display("left", 0.), display("right", 1728.)], "right");
        assert_eq!((m.tabs_out(), id(&m)), (vec![], 1));
    }

    #[test]
    fn displays_are_in_screen_order_and_a_tab_goes_to_the_next_free_one() {
        let mut m = two();
        set(&mut m, &[display("c", 3456.), display("a", -1728.), display("b", 0.)], "b");
        let uids: Vec<&str> = m.displays().iter().map(|d| d.uid.as_str()).collect();
        assert_eq!((uids, m.main_display()), (vec!["a", "b", "c"], Some(1)));
        // After the main window's, left to right, wrapping: c, then a.
        m.update(Action::TabOut { tab: None });
        assert_eq!(m.tabs_out(), [(2, 2)]);
        m.update(Action::KeyWindow { tab: 1 });
        m.update(Action::TabOut { tab: None });
        assert_eq!(m.tabs_out(), [(2, 2), (1, 0)]);
    }

    #[test]
    fn with_two_displays_a_second_fold_out_swaps() {
        let mut m = two();
        m.update(Action::NewEmptyTab);
        let third = id(&m);
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TabOut { tab: None });
        m.update(Action::KeyWindow { tab: main_id(&m) });
        m.update(Action::TabOut { tab: Some(third) });
        assert_eq!((m.tabs_out(), id(&m)), (vec![(third, 1)], third));
        assert!(!m.is_out(2));
        // The main tab folding out swaps with the one out: that one comes to the main window.
        m.update(Action::KeyWindow { tab: main_id(&m) });
        let main = main_id(&m);
        m.update(Action::TabOut { tab: None });
        assert_eq!((m.tabs_out(), id(&m), main_id(&m)), (vec![(main, 1)], main, third));
    }

    #[test]
    fn a_replug_takes_back_the_tab_that_was_on_the_display_last() {
        let mut m = two();
        m.update(Action::NewEmptyTab);
        let third = id(&m);
        let both = [display("left", 0.), display("right", 1728.)];
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::TabOut { tab: None });
        set(&mut m, &[display("left", 0.)], "left");
        // Tab 1 on show in the main window as the display comes back: it stays.
        m.update(Action::SelectTab { index: 0 });
        set(&mut m, &both, "left");
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TabOut { tab: None });
        set(&mut m, &[display("left", 0.)], "left");
        m.update(Action::SelectTab { index: m.tabs.iter().position(|t| t.id == third).unwrap_or(0) });
        set(&mut m, &both, "left");
        assert_eq!(m.tabs_out(), [(2, 1)]);
    }

    #[test]
    fn the_keyboard_leaves_the_main_windows_list_for_the_window_out() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::KeyWindow { tab: 1 });
        m.update(Action::FocusList);
        m.update(Action::KeyWindow { tab: 2 });
        assert_eq!((id(&m), m.list), (2, None));
        // A client's FocusList there leaves the keyboard where it is.
        m.update(Action::FocusList.sent_by(Origin::Socket));
        assert_eq!((id(&m), m.list), (2, None));
    }

    #[test]
    fn the_main_windows_tab_closed_under_a_window_out_gives_way_to_its_neighbour() {
        let mut m = two();
        m.update(Action::NewEmptyTab);
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::TabOut { tab: None });
        let main = main_id(&m);
        let at = m.tabs.iter().position(|t| t.id == main);
        m.update(Action::CloseTab { index: at }.sent_by(Origin::Socket));
        assert_ne!(main_id(&m), HOME_TAB);
        assert_eq!(m.shown_space(), &m.tabs[0].space);
    }

    #[test]
    fn cmd_t_and_cmd_digits_in_the_window_out_act_on_the_main_one() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::NewTab);
        // A tab of the main window's space, on show there, the keyboard with it.
        assert_eq!(
            (m.away, m.tabs_out(), m.tabs[m.active].space.clone()),
            (None, vec![(2, 1)], m.tabs[0].space.clone())
        );
        m.update(Action::KeyWindow { tab: 2 });
        m.update(Action::SelectVisibleTab { index: 0 });
        assert_eq!((id(&m), m.away, m.tabs_out()), (1, None, vec![(2, 1)]));
    }

    #[test]
    fn cmd_shift_w_in_the_window_out_closes_its_tab_and_the_main_window_stays() {
        let mut m = two();
        m.update(Action::NewEmptyTab);
        let third = id(&m);
        m.update(Action::TabOut { tab: None });
        // Its left neighbour: it had none to its right.
        assert_eq!(main_id(&m), 2);
        m.update(Action::CloseTab { index: None });
        assert_eq!((m.tabs_out(), id(&m), m.tabs.iter().any(|t| t.id == third)), (vec![], 2, false));
    }

    #[test]
    fn a_client_never_moves_the_keyboard_between_windows() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::KeyWindow { tab: 1 });
        let out_pane: PaneId = m.tabs[1].layout.as_ref().map(|l| l.leaves()[0]).unwrap_or_default();
        m.update(Action::Focus { pane: out_pane }.sent_by(Origin::Socket));
        assert_eq!((id(&m), m.away), (1, None));
        // The user's click there takes it.
        m.update(Action::Focus { pane: out_pane });
        assert_eq!((id(&m), main_id(&m)), (2, 1));
    }

    #[test]
    fn the_drawer_and_the_list_are_the_main_windows() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::FocusList);
        assert_eq!((id(&m), m.away), (1, None));
    }

    /// The user's main window goes to the first workspace (the tab out's stand-in left behind),
    /// and back to the window out.
    fn main_elsewhere(m: &mut Model) {
        let out = id(m);
        m.update(Action::KeyWindow { tab: main_id(m) });
        m.update(Action::SpaceSelect { index: 1 });
        m.update(Action::KeyWindow { tab: out });
    }

    #[test]
    fn a_new_task_from_the_window_out_joins_its_workspace() {
        let mut m = two();
        m.update(Action::SpaceSelect { index: 0 });
        m.update(Action::NewEmptyTab);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let of = m.tabs[m.active].space.clone();
        if let Owner::Workbook { id } = of {
            m.update(Action::WorkbookKeep { workbook: id });
        }
        m.update(Action::TabOut { tab: None });
        main_elsewhere(&mut m);
        assert_ne!(m.shown_space(), &of);
        m.update(Action::OpenCompose { text: "fix it".into() });
        // The box is the main window's, and so is the keyboard; its task goes where it was asked.
        assert_eq!((m.away, m.joins()), (None, Some(of)));
        m.update(Action::CloseCompose);
        assert_eq!(m.compose_joins, None);
    }

    #[test]
    fn a_draft_from_the_box_of_the_window_out_starts_its_task_where_the_user_is() {
        let mut m = two();
        m.update(Action::SpaceSelect { index: 0 });
        m.update(Action::NewEmptyTab);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let of = m.tabs[m.active].space.clone();
        if let Owner::Workbook { id } = of {
            m.update(Action::WorkbookKeep { workbook: id });
        }
        m.update(Action::TabOut { tab: None });
        main_elsewhere(&mut m);
        m.update(Action::OpenCompose { text: "fix it".into() });
        // A capture takes the box's line: its task goes where the user is, not to the window out's.
        let word = "k3f9a2c41b7d".into();
        m.update(Action::CaptureStart { pane: None, draft: "d".into(), word, facts: vec![], images: true });
        assert_eq!((m.compose_joins.clone(), m.joins()), (None, Some(m.space().clone())));
        assert_ne!(m.joins(), Some(of));
    }

    #[test]
    fn the_palette_and_the_picker_open_in_the_main_window_with_the_keyboard() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        m.update(Action::PaletteOpen);
        assert_eq!((id(&m), m.away), (1, None));
        m.update(Action::ClosePalette);
        m.update(Action::KeyWindow { tab: 2 });
        m.update(Action::OpenPicker { axis: Axis::Right, new_tab: false, items: Vec::new() });
        assert_eq!((id(&m), m.away), (1, None));
    }

    #[test]
    fn with_the_flag_off_nothing_is_out() {
        let mut m = two();
        m.update(Action::TabOut { tab: None });
        let off = crate::features::Features::all().with(Feature::MultiMonitor, false);
        m.update(Action::SetFeatures { features: off });
        assert_eq!((m.tabs_out(), m.away, id(&m)), (vec![], None, 1));
        m.update(Action::TabOut { tab: None });
        assert!(m.tabs_out().is_empty());
    }
}
