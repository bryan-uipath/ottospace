//! The picker's second step: a running app's windows, read for the core (`PickerWindows`), and
//! the one chosen hosted (`Effect::HostPicked`). In a test run its fixture's made-up apps
//! (`OTTOSPACE_PICK_APPS`) stand for running ones, and nothing is hosted.

use gpui::{AnyElement, Context, Window, div, prelude::*, px, rgb};
use ottospace_core::model::PickWindow;
use ottospace_core::rules::{self, Shape};
use ottospace_core::{Action, Axis, PickItem, Picker, theme};
use ottospace_mac::{ax, capture};
use serde::Deserialize;

use crate::{Hosted, Workspace, config, dialog, picker_title, with_facts};

/// A made-up running app of the fixture: `{"name":"Lumen","windows":[{"window":7,"title":"Inbox"}]}`.
#[derive(Deserialize)]
pub(crate) struct FixtureApp {
    pub(crate) name: String,
    #[serde(default)]
    windows: Vec<FixtureWindow>,
}

#[derive(Deserialize)]
struct FixtureWindow {
    window: u32,
    title: String,
}

/// `OTTOSPACE_PICK_APPS`, in a test run only: a JSON file of made-up running apps. None named: none.
pub(crate) fn fixture_apps() -> Vec<FixtureApp> {
    let path = std::env::var("OTTOSPACE_PICK_APPS").ok().filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some());
    let text = path.and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

/// A window the user's pick may put in a pane (`rules::pickable`): a pop-up the rules floated
/// too. Not a minimised one: its stream would draw nothing.
fn pickable(window: &ax::Window) -> bool {
    let (frame, sheet) = window.quick(|w| (w.frame(), w.sheet()));
    let shape = Shape { sheet, size: frame.map(|f| [f.size.width, f.size.height]), ..Shape::default() };
    !window.minimized() && with_facts(window, |facts| rules::pickable(&config::live().windows, facts, &shape))
}

impl Workspace {
    /// Carries out `Effect::PickWindows`: `app`'s windows that may go in a pane, to the core.
    pub(crate) fn pick_windows(&mut self, app: &str, window: &Window, cx: &mut Context<Self>) {
        let fixture =
            fixture_apps().into_iter().filter(|fixture| fixture.name == app).flat_map(|fixture| fixture.windows);
        let mut windows: Vec<PickWindow> = fixture.map(|w| PickWindow { window: w.window, title: w.title }).collect();
        if let Some(pid) = self.apps.iter().find(|running| running.name == app).map(|running| running.pid) {
            let read = |w: ax::Window| Some(PickWindow { window: w.id()?, title: w.title().unwrap_or_default() });
            windows.extend(ax::Window::all_of(pid).into_iter().filter(pickable).filter_map(read));
        }
        self.dispatch(Action::PickerWindows { app: app.into(), windows }, self.input, window, cx);
    }

    /// Carries out `Effect::HostPicked`: window `id` of `app` in a new pane. The status bar says
    /// why not.
    pub(crate) fn host_picked(
        &mut self,
        app: &str,
        id: u32,
        axis: Axis,
        new_tab: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.window_picks += 1;
        // Cleared first: a notice hosting sets (a window found off screen) outlives a success.
        self.error = None;
        if let Err(why) = self.try_host_picked(app, id, axis, new_tab, window, cx) {
            self.error = Some(why);
        }
        cx.notify();
    }

    fn try_host_picked(
        &mut self,
        app: &str,
        id: u32,
        axis: Axis,
        new_tab: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.hosting {
            return Err("window hosting is off in e2e runs".into());
        }
        let pid = self.apps.iter().find(|running| running.name == app).map(|running| running.pid);
        let pid = pid.ok_or_else(|| format!("{app} is not running"))?;
        if self.host_only.is_some_and(|only| only != pid) {
            return Err("this test run may host only the app it started".into());
        }
        let found = ax::Window::all_of(pid).into_iter().find(|w| w.id() == Some(id) && pickable(w));
        let hosted = capture::target(found.ok_or("the window is gone")?).and_then(Hosted::new)?;
        let opened = self.host_in_new_pane(hosted, axis, new_tab, window, cx);
        opened.map(drop).ok_or_else(|| format!("{app}: no pane could be opened for its window"))
    }
}

impl Workspace {
    /// The second step, in the first's place: `app`'s windows, each by its title, and the pane
    /// that holds it already.
    pub(crate) fn render_window_picker(&self, picker: &Picker, app: &str, cx: &mut Context<Self>) -> AnyElement {
        let selected = picker.selected;
        let rows = picker.matches().into_iter().enumerate().filter_map(|(i, item)| {
            let PickItem::Window { window: id, .. } = item else { return None };
            // Held now: a pane may have opened or closed since the windows were read.
            let held = self
                .model
                .pane_holding(*id)
                .map(|pane| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(format!("in pane {pane}")));
            let row = dialog::filter_row(("pick", i), i == selected)
                .child(div().w(px(28.)).flex_none().child(self.app_icon(app)))
                .child(div().flex_1().min_w_0().truncate().child(item.label().to_string()))
                .children(held)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::PickerChoose { index: Some(i) }, this.input, window, cx);
                }));
            Some(row.into_any_element())
        });
        let rows: Vec<AnyElement> = rows.collect();
        let title = format!("{} {app}", picker_title(picker)).into();
        self.picker_panel(360., title, rows, "No window to put in a pane", cx)
    }
}
