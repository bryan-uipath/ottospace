//! The pane picker's second step: a running app's windows. Choosing the app lists them; choosing
//! one hosts it, or goes to the pane that holds it already.

use super::{
    Action, Effect, Field, Kind, MAX_FILTER, Model, NewWindowWait, Owner, Pane, PaneId, PickItem, PickWindow, Picker,
};
use crate::layout::{Axis, Node};
use crate::new_window;

impl Model {
    /// The pane that holds window `window`, if one does.
    pub fn pane_holding(&self, window: u32) -> Option<PaneId> {
        let holds = |kind: &Kind| matches!(kind, Kind::App { window: held, .. } if *held == window);
        self.panes.iter().find(|pane| holds(&pane.kind)).map(|pane| pane.id)
    }

    /// `PickerWindows`: the picker at its first step, `app` among its rows, lists "New window" (an
    /// app with a recipe or a "New Window" menu item), then its windows.
    pub(super) fn picker_windows(
        &mut self,
        app: String,
        bundle: Option<String>,
        menu_item: bool,
        windows: Vec<PickWindow>,
    ) {
        let window = |w: PickWindow| {
            let pane = self.pane_holding(w.window);
            PickItem::Window { app: app.clone(), window: w.window, title: w.title, pane }
        };
        let offered = new_window::recipe(bundle.as_deref(), menu_item).is_some();
        let new = offered.then(|| PickItem::NewWindow { app: app.clone(), bundle, menu_item });
        let items: Vec<PickItem> = new.into_iter().chain(windows.into_iter().map(window)).collect();
        let listed = |picker: &&mut Picker| {
            let has = |item: &PickItem| matches!(item, PickItem::App { name } if *name == app);
            !picker.transcribe && picker.app.is_none() && picker.items.iter().any(has)
        };
        let Some(picker) = self.picker.as_mut().filter(listed) else { return };
        *picker = Picker {
            items,
            installed: Vec::new(),
            query: Field::new("", MAX_FILTER),
            selected: 0,
            app: Some(app),
            ..picker.clone()
        };
    }

    /// Whether `PickerChoose { index }` would choose "New window".
    pub fn chooses_new_window(&self, index: Option<usize>) -> bool {
        let Some(picker) = &self.picker else { return false };
        matches!(picker.matches().get(index.unwrap_or(picker.selected)), Some(PickItem::NewWindow { .. }))
    }

    /// "New window": the app is asked for one by its recipe, and the picker waits for it. One at
    /// a time: a second is refused while the first is on its way.
    pub(super) fn pick_new_window(
        &mut self,
        picker: &Picker,
        app: String,
        bundle: Option<String>,
        menu_item: bool,
        effects: &mut Vec<Effect>,
    ) {
        let Some(recipe) = new_window::recipe(bundle.as_deref(), menu_item) else { return };
        if let Some(wait) = &self.new_window {
            effects.push(Effect::Notice { text: format!("Still waiting for {}'s new window", wait.app) });
            return;
        }
        // As a terminal picked for a new tab: the user's joins the selected space.
        let of = if self.by_user { self.space().clone() } else { Owner::Home };
        let tab_of = picker.new_tab.then_some(of);
        let (tab, beside) = (self.tabs[self.active].id, self.focused);
        let (since, axis) = (self.now, picker.axis);
        self.new_window = Some(NewWindowWait { app: app.clone(), since, axis, tab_of, tab, beside });
        effects.push(Effect::NewWindow { app, bundle, recipe });
    }

    /// The window "New window" asked for: in a pane, owned. None asked for (it came too late):
    /// it is left where its app put it.
    pub(super) fn new_window_came(&mut self, kind: Kind, effects: &mut Vec<Effect>) {
        let Some(wait) = self.new_window.take() else { return };
        if !matches!(kind, Kind::App { .. }) {
            return;
        }
        let id = match (wait.tab_of, self.tabs.iter().position(|tab| tab.id == wait.tab)) {
            // The user moved on: its new tab is made behind the one on show, and the user stays.
            (Some(of), at) if at != Some(self.active) => {
                let shown = self.tabs[self.active].id;
                self.park_active();
                self.push_tab_of(of);
                let made = self.active;
                self.active = self.tabs.iter().position(|tab| tab.id == shown).unwrap_or(made);
                self.unpark_active();
                self.open_in_background(made, kind, wait.axis, None, effects)
            }
            (Some(of), _) => {
                self.leave(effects);
                self.park_active();
                self.push_tab_of(of);
                self.open(kind, wait.axis, None, effects)
            }
            // The tab it was asked from, in the background now: it goes there, and the user stays.
            (None, Some(at)) if at != self.active => self.open_in_background(at, kind, wait.axis, wait.beside, effects),
            // That tab, on show; or gone: the tab on show.
            _ => {
                let shown = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
                let beside = wait.beside.filter(|pane| shown.contains(pane));
                self.open(kind, wait.axis, beside, effects)
            }
        };
        // A slot owns no window: one given back, and the next it holds, stay open when it closes.
        let slot = self.slots.contains_key(&id);
        if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id && !slot) {
            pane.owned = true;
        }
    }

    /// A pane of `kind` in background tab `at`, beside `beside` if that tab holds it, else its
    /// focused pane; it fills that pane if empty. The tab on show does not change.
    fn open_in_background(
        &mut self,
        at: usize,
        kind: Kind,
        axis: Axis,
        beside: Option<PaneId>,
        effects: &mut Vec<Effect>,
    ) -> PaneId {
        let tab = &self.tabs[at];
        let held = tab.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let beside = beside.filter(|pane| held.contains(pane)).or(tab.focused.filter(|pane| held.contains(pane)));
        if let Some(empty) = beside.and_then(|id| self.panes.iter_mut().find(|p| p.id == id && p.kind == Kind::Empty)) {
            empty.kind = kind;
            effects.push(Effect::Opened(empty.id));
            return empty.id;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.panes.push(Pane { id, kind, cwd: None, fit: None, owned: false });
        let tab = &mut self.tabs[at];
        tab.layout = Some(match tab.layout.take() {
            Some(root) => root.insert(beside, id, axis),
            None => Node::Leaf(id),
        });
        tab.focused = tab.focused.or(Some(id));
        effects.push(Effect::Opened(id));
        id
    }

    /// Whether an owned pane's app says it has unsaved changes. A shared pane's is not kept.
    pub(super) fn window_edited(&mut self, pane: PaneId, edited: bool) {
        let owned = self.pane(pane).is_some_and(|p| p.owned);
        self.edited.retain(|id| *id != pane);
        if edited && owned {
            self.edited.push(pane);
        }
    }

    /// No window came in time: a toast says so, and one that comes later stays outside.
    pub(super) fn new_window_late(&mut self, now: u64, effects: &mut Vec<Effect>) {
        if let Some(wait) = self.new_window.take_if(|wait| wait.since + new_window::WAIT_MS <= now) {
            effects.push(Effect::Failed { text: new_window::late(&wait.app) });
        }
    }

    /// A window chosen in the second step: its pane if one holds it, else the shell hosts it.
    pub(super) fn pick_window(&mut self, picker: &Picker, app: String, window: u32, effects: &mut Vec<Effect>) {
        match self.pane_holding(window) {
            Some(pane) => {
                // As a pick that opens a pane: that pane takes the keyboard from the list.
                self.list = None;
                self.act(Action::Focus { pane }, effects)
            }
            None => effects.push(Effect::HostPicked { app, window, axis: picker.axis, new_tab: picker.new_tab }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Action, Effect, Kind, Model, Origin, PickItem, PickWindow};
    use crate::layout::Axis::Right;
    use crate::new_window;

    fn windows() -> Vec<PickWindow> {
        let window = |window, title: &str| PickWindow { window, title: title.into() };
        vec![window(7, "Inbox"), window(8, "Docs")]
    }

    /// The picker on a terminal and Chrome, Chrome chosen: the shell is asked for its windows.
    fn chosen(m: &mut Model) {
        let items = vec![PickItem::Terminal, PickItem::App { name: "Chrome".into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items });
        let effects = m.update(Action::PickerChoose { index: Some(1) });
        assert_eq!(effects, [Effect::PickWindows { app: "Chrome".into() }]);
    }

    fn rows(m: &Model) -> Vec<(u32, String, Option<u32>)> {
        let row = |item: &&PickItem| match item {
            PickItem::Window { window, title, pane, .. } => Some((*window, title.clone(), *pane)),
            _ => None,
        };
        m.picker.as_ref().map(|picker| picker.matches().iter().filter_map(row).collect()).unwrap_or_default()
    }

    #[test]
    fn choosing_an_app_lists_its_windows_and_one_is_hosted() {
        let mut m = Model::default();
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        assert_eq!(rows(&m), [(7, "Inbox".into(), None), (8, "Docs".into(), None)]);
        assert_eq!(m.picker.as_ref().and_then(|picker| picker.app.as_deref()), Some("Chrome"));
        // The filter narrows the windows by title; the installed apps join no second step.
        m.update(Action::PickerQuery { query: "do".into() });
        assert_eq!(rows(&m), [(8, "Docs".into(), None)]);
        let effects = m.update(Action::PickerChoose { index: None });
        assert_eq!(effects, [Effect::HostPicked { app: "Chrome".into(), window: 8, axis: Right, new_tab: false }]);
        assert!(m.picker.is_none());
        // A window with no title is found by the name its row shows.
        chosen(&mut m);
        let untitled = vec![PickWindow { window: 9, title: "".into() }];
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: untitled });
        m.update(Action::PickerQuery { query: "untitled".into() });
        assert_eq!(rows(&m), [(9, String::new(), None)]);
    }

    #[test]
    fn every_zoom_window_the_shell_lists_is_a_row_and_the_floated_meeting_one_is_hosted() {
        // The shell lists what `rules::pickable` lets in: the main window, the share viewer and
        // the meeting window that floated as a pop-up.
        let mut m = Model::default();
        let items = vec![PickItem::App { name: "zoom.us".into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items });
        m.update(Action::PickerChoose { index: None });
        let window = |window, title: &str| PickWindow { window, title: title.into() };
        let windows = vec![window(30, "Zoom Workplace"), window(31, "Zoom Meeting"), window(32, "Zoom Meeting")];
        m.update(Action::PickerWindows {
            app: "zoom.us".into(),
            bundle: Some("us.zoom.xos".into()),
            menu_item: true,
            windows,
        });
        assert_eq!(rows(&m).iter().map(|row| row.0).collect::<Vec<_>>(), [30, 31, 32]);
        let effects = m.update(Action::PickerChoose { index: Some(3) });
        assert_eq!(effects, [Effect::HostPicked { app: "zoom.us".into(), window: 32, axis: Right, new_tab: false }]);
    }

    #[test]
    fn a_window_in_a_pane_says_so_and_choosing_it_goes_there() {
        let mut m = Model::default();
        let kind = Kind::App { window: 8, title: "Docs".into(), bundle: None, name: "Chrome".into() };
        m.update(Action::Open { kind, axis: Right, beside: None });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!(m.focused, Some(2));
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        assert_eq!(rows(&m), [(7, "Inbox".into(), None), (8, "Docs".into(), Some(1))]);
        // No second copy: the pane that holds it is shown and focused.
        let effects = m.update(Action::PickerChoose { index: Some(2) });
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::HostPicked { .. })), "{effects:?}");
        assert_eq!((m.focused, m.picker.is_none()), (Some(1), true));
    }

    #[test]
    fn going_to_a_held_window_from_the_list_gives_the_keyboard_to_its_pane() {
        let mut m = Model::default();
        let kind = Kind::App { window: 8, title: "Docs".into(), bundle: None, name: "Chrome".into() };
        m.update(Action::Open { kind, axis: Right, beside: None });
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::ToggleSection { section: crate::Section::Tasks });
        m.update(Action::FocusList);
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        m.update(Action::PickerChoose { index: Some(2) });
        assert_eq!((m.focused, m.list, m.mode()), (Some(1), None, crate::Mode::Normal));
    }

    #[test]
    fn windows_come_only_to_the_first_step_of_a_picker_that_lists_the_app() {
        let mut m = Model::default();
        // No picker, or one without the app: nothing.
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        assert!(m.picker.is_none());
        let items = vec![PickItem::App { name: "Slack".into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items });
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        assert_eq!(m.picker.as_ref().map(|picker| picker.app.is_none()), Some(true));
        // A second answer does not replace the windows listed.
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: vec![] });
        assert_eq!(rows(&m).len(), 2);
        // Titles are drawn, never in the state.
        let state = serde_json::to_string(&m.picker).unwrap_or_default();
        assert!(!state.contains("Inbox") && state.contains("\"window\":7"), "{state}");
    }

    #[test]
    fn a_window_that_went_into_a_pane_since_it_was_listed_is_not_hosted_twice() {
        let mut m = Model::default();
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: windows() });
        let kind = Kind::App { window: 7, title: "Inbox".into(), bundle: None, name: "Chrome".into() };
        m.update(Action::Open { kind, axis: Right, beside: None }.sent_by(Origin::Socket));
        let effects = m.update(Action::PickerChoose { index: Some(1) });
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::HostPicked { .. })), "{effects:?}");
    }

    /// The second step on Chrome, its bundle known.
    fn listed(m: &mut Model) {
        chosen(m);
        let bundle = Some("com.google.Chrome".to_string());
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle, menu_item: false, windows: windows() });
    }

    fn owned(window: u32) -> Kind {
        Kind::App { window, title: "New Tab".into(), bundle: None, name: "Chrome".into() }
    }

    #[test]
    fn new_window_comes_first_asks_by_the_apps_recipe_and_its_window_is_owned() {
        let mut m = Model::default();
        m.update(Action::Tick { now: 1_000 });
        listed(&mut m);
        let first = m.picker.as_ref().and_then(|picker| picker.matches().first().copied().cloned());
        let bundle = Some("com.google.Chrome".to_string());
        assert_eq!(first, Some(PickItem::NewWindow { app: "Chrome".into(), bundle: bundle.clone(), menu_item: false }));
        let effects = m.update(Action::PickerChoose { index: None });
        let recipe = new_window::Recipe::Open { args: &["--new-window"] };
        assert_eq!(effects, [Effect::NewWindow { app: "Chrome".into(), bundle, recipe }]);
        assert_eq!((m.picker.is_none(), m.new_window.as_ref().map(|wait| wait.since)), (true, Some(1_000)));
        // The window comes: a pane, owned. A shared one is not.
        let effects = m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("a pane");
        assert!(effects.contains(&Effect::Opened(id)) && m.new_window.is_none());
        assert!(m.pane(id).is_some_and(|pane| pane.owned));
        // A second window after it opens nothing: none is asked for.
        m.update(Action::NewWindowCame { kind: owned(41) }.sent_by(Origin::System));
        assert_eq!(m.pane_holding(41), None);
    }

    #[test]
    fn no_window_in_three_seconds_is_said_and_a_late_one_stays_out() {
        let mut m = Model::default();
        m.update(Action::Tick { now: 1_000 });
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        assert!(m.update(Action::Tick { now: 3_999 }).is_empty() && m.due(4_000) && !m.due(3_999));
        assert_eq!(m.next_due(), Some(4_000));
        let text = new_window::late("Chrome");
        assert_eq!(m.update(Action::Tick { now: 4_000 }), [Effect::Failed { text }]);
        assert_eq!(text_of(&m), None);
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        assert_eq!(m.pane_holding(40), None);
    }

    fn text_of(m: &Model) -> Option<&str> {
        m.new_window.as_ref().map(|wait| wait.app.as_str())
    }

    #[test]
    fn a_new_window_for_a_new_tab_opens_in_one() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let items = vec![PickItem::App { name: "Chrome".into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: true, items });
        m.update(Action::PickerChoose { index: None });
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: vec![] });
        let tabs = m.tabs.len();
        m.update(Action::PickerChoose { index: None });
        // No tab until the window is in hand.
        assert_eq!(m.tabs.len(), tabs);
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40);
        assert_eq!((m.tabs.len(), m.tab_panes()[m.active].clone()), (tabs + 1, id.into_iter().collect::<Vec<_>>()));
    }

    #[test]
    fn closing_an_owned_pane_closes_its_window_and_a_shared_one_goes_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: owned(5), axis: Right, beside: None });
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("owned");
        let effects = m.update(Action::Close { pane: Some(id) });
        assert!(effects.contains(&Effect::CloseWindow { pane: id, window: 40 }), "{effects:?}");
        let effects = m.update(Action::Close { pane: Some(1) });
        assert!(!effects.iter().any(|e| matches!(e, Effect::CloseWindow { .. })), "{effects:?}");
    }

    #[test]
    fn the_users_close_of_an_owned_window_with_unsaved_changes_asks_first() {
        let mut m = Model::default();
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("owned");
        // Only an owned pane's edits are kept, and only the shell says them.
        assert_eq!(
            m.refusal(&Action::WindowEdited { pane: id, edited: true }, Origin::Socket),
            Some(new_window::SHELLS_ALONE)
        );
        m.update(Action::WindowEdited { pane: id, edited: true }.sent_by(Origin::System));
        assert_eq!(m.edited, [id]);
        // The user's close asks; Esc keeps it.
        assert!(m.update(Action::Close { pane: None }).is_empty());
        assert_eq!(m.confirm.as_ref().and_then(|confirm| confirm.edited), Some(id));
        assert_eq!(m.refusal(&Action::QuitConfirm, Origin::Socket), Some(new_window::USERS_ALONE));
        m.update(Action::QuitCancel);
        assert!(m.confirm.is_none() && m.pane(id).is_some());
        // Yes closes it, with its window.
        m.update(Action::Close { pane: Some(id) });
        let effects = m.update(Action::QuitConfirm);
        assert!(effects.contains(&Effect::CloseWindow { pane: id, window: 40 }) && m.pane(id).is_none());
        assert!(m.edited.is_empty());
    }

    #[test]
    fn saved_again_it_closes_without_a_question_and_a_timers_close_never_asks() {
        let mut m = Model::default();
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("owned");
        m.update(Action::WindowEdited { pane: id, edited: true }.sent_by(Origin::System));
        m.update(Action::Close { pane: Some(id) });
        // Saved while asked: the question goes.
        m.update(Action::WindowEdited { pane: id, edited: false }.sent_by(Origin::System));
        assert!(m.confirm.is_none());
        m.update(Action::WindowEdited { pane: id, edited: true }.sent_by(Origin::System));
        // The shell's close (its window is gone) does not ask.
        m.update(Action::Close { pane: Some(id) }.sent_by(Origin::System));
        assert!(m.pane(id).is_none() && m.confirm.is_none());
        // A shared pane's edits are not kept.
        m.update(Action::Open { kind: owned(5), axis: Right, beside: None });
        let shared = m.pane_holding(5).expect("shared");
        m.update(Action::WindowEdited { pane: shared, edited: true }.sent_by(Origin::System));
        assert!(m.edited.is_empty());
    }

    #[test]
    fn only_the_users_pick_asks_for_a_new_window_and_one_at_a_time() {
        let mut m = Model::default();
        listed(&mut m);
        // A client's "New window" would press in another app.
        let choose = Action::PickerChoose { index: Some(0) };
        assert_eq!(m.refusal(&choose, Origin::Socket), Some(new_window::PICK_USERS));
        assert_eq!(m.refusal(&choose, Origin::System), Some(new_window::PICK_USERS));
        assert_eq!(m.refusal(&Action::PickerChoose { index: Some(1) }, Origin::Socket), None);
        m.update(choose.clone());
        // A second while the first is on its way: said, and nothing asked.
        listed(&mut m);
        let effects = m.update(choose);
        assert_eq!(effects, [Effect::Notice { text: "Still waiting for Chrome's new window".into() }]);
        assert_eq!(m.new_window.as_ref().map(|wait| wait.app.as_str()), Some("Chrome"));
    }

    #[test]
    fn an_owned_pane_kept_as_a_slot_owns_no_window_after() {
        let mut m = Model::default();
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("owned");
        m.update(Action::WindowEdited { pane: id, edited: true }.sent_by(Origin::System));
        m.update(Action::SlotMake { pane: id, pid: 9, slot: crate::slot::Slot::default() });
        assert!(m.pane(id).is_some_and(|pane| !pane.owned) && m.edited.is_empty());
        let effects = m.update(Action::Close { pane: Some(id) });
        assert!(!effects.iter().any(|e| matches!(e, Effect::CloseWindow { .. })), "{effects:?}");
    }

    /// The second step on `app` (bundle `bundle`), its "New Window" item said.
    fn listed_as(m: &mut Model, app: &str, bundle: Option<&str>, menu_item: bool) {
        let items = vec![PickItem::App { name: app.into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items });
        m.update(Action::PickerChoose { index: None });
        let bundle = bundle.map(str::to_string);
        m.update(Action::PickerWindows { app: app.into(), bundle, menu_item, windows: windows() });
    }

    fn first_row(m: &Model) -> Option<PickItem> {
        m.picker.as_ref().and_then(|picker| picker.matches().first().copied().cloned())
    }

    #[test]
    fn new_window_is_offered_only_by_a_recipe_or_a_new_window_menu_item() {
        // Notes: no recipe, no "New Window" item. Its ⌘N makes a note: no row.
        let mut m = Model::default();
        listed_as(&mut m, "Notes", Some("com.apple.Notes"), false);
        assert!(matches!(first_row(&m), Some(PickItem::Window { window: 7, .. })), "{:?}", first_row(&m));
        // An app with the item: offered, and the item is what is pressed.
        let mut m = Model::default();
        listed_as(&mut m, "Mail", Some("com.apple.mail"), true);
        assert!(matches!(first_row(&m), Some(PickItem::NewWindow { menu_item: true, .. })));
        let effects = m.update(Action::PickerChoose { index: Some(0) });
        let bundle = Some("com.apple.mail".to_string());
        assert_eq!(effects, [Effect::NewWindow { app: "Mail".into(), bundle, recipe: new_window::Recipe::MenuItem }]);
        // An app of the table: offered with no item.
        let mut m = Model::default();
        listed_as(&mut m, "Safari", Some("com.apple.Safari"), false);
        assert!(matches!(first_row(&m), Some(PickItem::NewWindow { menu_item: false, .. })));
    }

    #[test]
    fn the_new_window_goes_to_the_tab_it_was_asked_from_and_the_user_stays_where_they_went() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let (asked_tab, asked_pane) = (m.tabs[m.active].id, m.focused);
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        // The user opens another tab before the window comes.
        m.update(Action::NewTab);
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let shown = m.tabs[m.active].id;
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("a pane");
        assert_eq!((m.tab_of(id), m.tabs[m.active].id), (Some(asked_tab), shown));
        let tab = m.tabs.iter().find(|tab| tab.id == asked_tab).and_then(|tab| tab.layout.clone());
        assert_eq!(tab.map(|layout| layout.leaves()), asked_pane.map(|pane| vec![pane, id]));
        assert!(m.pane(id).is_some_and(|pane| pane.owned));
    }

    #[test]
    fn a_new_window_for_a_new_tab_after_the_user_moved_on_opens_its_tab_behind() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::OpenPicker {
            axis: Right,
            new_tab: true,
            items: vec![PickItem::App { name: "Chrome".into() }],
        });
        m.update(Action::PickerChoose { index: None });
        m.update(Action::PickerWindows { app: "Chrome".into(), bundle: None, menu_item: true, windows: vec![] });
        m.update(Action::PickerChoose { index: None });
        // The user opens another tab before the window comes.
        m.update(Action::NewTab);
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        let (tabs, shown, layout) = (m.tabs.len(), m.tabs[m.active].id, m.layout.clone());
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("a pane");
        assert_eq!((m.tabs.len(), m.tabs[m.active].id, m.layout.clone()), (tabs + 1, shown, layout));
        let made = m.tab_of(id).and_then(|tab| m.tabs.iter().find(|t| t.id == tab));
        assert!(made.is_some_and(|tab| tab.id != shown && tab.focused == Some(id)));
    }

    #[test]
    fn a_new_window_whose_tab_closed_goes_to_the_tab_on_show() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        listed(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::NewTab);
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::CloseTab { index: Some(0) });
        let shown = m.tabs[m.active].id;
        m.update(Action::NewWindowCame { kind: owned(40) }.sent_by(Origin::System));
        let id = m.pane_holding(40).expect("a pane");
        assert_eq!(m.tab_of(id), Some(shown));
    }

    #[test]
    fn an_ask_that_failed_is_said_as_it_failed_and_no_timeout_follows() {
        let mut m = Model::default();
        m.update(Action::Tick { now: 1_000 });
        listed_as(&mut m, "Mail", Some("com.apple.mail"), true);
        m.update(Action::PickerChoose { index: Some(0) });
        let text = "Mail did not take its New Window item".to_string();
        assert_eq!(
            m.refusal(&Action::NewWindowFailed { text: text.clone() }, Origin::Socket),
            Some(new_window::SHELLS_ALONE)
        );
        let effects = m.update(Action::NewWindowFailed { text: text.clone() }.sent_by(Origin::System));
        assert_eq!((effects, m.new_window.is_none()), (vec![Effect::Failed { text }], true));
        assert!(m.update(Action::Tick { now: 9_000 }).is_empty());
    }
}
