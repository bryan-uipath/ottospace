//! The pane picker's second step: a running app's windows. Choosing the app lists them; choosing
//! one hosts it, or goes to the pane that holds it already.

use super::{Action, Effect, Field, Kind, MAX_FILTER, Model, PaneId, PickItem, PickWindow, Picker};

impl Model {
    /// The pane that holds window `window`, if one does.
    pub fn pane_holding(&self, window: u32) -> Option<PaneId> {
        let holds = |kind: &Kind| matches!(kind, Kind::App { window: held, .. } if *held == window);
        self.panes.iter().find(|pane| holds(&pane.kind)).map(|pane| pane.id)
    }

    /// `PickerWindows`: the picker at its first step, `app` among its rows, lists its windows.
    pub(super) fn picker_windows(&mut self, app: String, windows: Vec<PickWindow>) {
        let window = |w: PickWindow| {
            let pane = self.pane_holding(w.window);
            PickItem::Window { app: app.clone(), window: w.window, title: w.title, pane }
        };
        let items: Vec<PickItem> = windows.into_iter().map(window).collect();
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
            PickItem::Window { window, title, pane, .. } => (*window, title.clone(), *pane),
            other => panic!("not a window: {other:?}"),
        };
        m.picker.as_ref().map(|picker| picker.matches().iter().map(row).collect()).unwrap_or_default()
    }

    #[test]
    fn choosing_an_app_lists_its_windows_and_one_is_hosted() {
        let mut m = Model::default();
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
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
        m.update(Action::PickerWindows {
            app: "Chrome".into(),
            windows: vec![PickWindow { window: 9, title: "".into() }],
        });
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
        m.update(Action::PickerWindows { app: "zoom.us".into(), windows });
        assert_eq!(rows(&m).iter().map(|row| row.0).collect::<Vec<_>>(), [30, 31, 32]);
        let effects = m.update(Action::PickerChoose { index: Some(2) });
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
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
        assert_eq!(rows(&m), [(7, "Inbox".into(), None), (8, "Docs".into(), Some(1))]);
        // No second copy: the pane that holds it is shown and focused.
        let effects = m.update(Action::PickerChoose { index: Some(1) });
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
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
        m.update(Action::PickerChoose { index: Some(1) });
        assert_eq!((m.focused, m.list, m.mode()), (Some(1), None, crate::Mode::Normal));
    }

    #[test]
    fn windows_come_only_to_the_first_step_of_a_picker_that_lists_the_app() {
        let mut m = Model::default();
        // No picker, or one without the app: nothing.
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
        assert!(m.picker.is_none());
        let items = vec![PickItem::App { name: "Slack".into() }];
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items });
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
        assert_eq!(m.picker.as_ref().map(|picker| picker.app.is_none()), Some(true));
        // A second answer does not replace the windows listed.
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: vec![] });
        assert_eq!(rows(&m).len(), 2);
        // Titles are drawn, never in the state.
        let state = serde_json::to_string(&m.picker).unwrap_or_default();
        assert!(!state.contains("Inbox") && state.contains("\"window\":7"), "{state}");
    }

    #[test]
    fn a_window_that_went_into_a_pane_since_it_was_listed_is_not_hosted_twice() {
        let mut m = Model::default();
        chosen(&mut m);
        m.update(Action::PickerWindows { app: "Chrome".into(), windows: windows() });
        let kind = Kind::App { window: 7, title: "Inbox".into(), bundle: None, name: "Chrome".into() };
        m.update(Action::Open { kind, axis: Right, beside: None }.sent_by(Origin::Socket));
        let effects = m.update(Action::PickerChoose { index: Some(0) });
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::HostPicked { .. })), "{effects:?}");
    }
}
