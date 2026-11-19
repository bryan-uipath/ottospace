//! The picker's second step: a running app's windows, read for the core (`PickerWindows`), and
//! the one chosen hosted (`Effect::HostPicked`), or a new window asked for by its recipe
//! (`Effect::NewWindow`) and hosted, owned, once it appears. In a test run its fixture's made-up
//! apps (`OTTOSPACE_PICK_APPS`) stand for running ones, and nothing is asked or hosted.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, Window, div, prelude::*, px, rgb};
use ottospace_core::model::PickWindow;
use ottospace_core::new_window::{self, Recipe};
use ottospace_core::rules::{self, Shape};
use ottospace_core::{Action, Axis, Origin, PaneId, PickItem, Picker, theme};
use ottospace_mac::{ax, capture};
use serde::Deserialize;

use crate::{Hosted, Workspace, config, dialog, picker_title, with_facts};

/// A made-up running app of the fixture: `{"name":"Lumen","windows":[{"window":7,"title":"Inbox"}]}`.
#[derive(Deserialize)]
pub(crate) struct FixtureApp {
    pub(crate) name: String,
    #[serde(default)]
    bundle: Option<String>,
    /// It has a menu item titled "New Window".
    #[serde(default)]
    new_window_item: bool,
    #[serde(default)]
    windows: Vec<FixtureWindow>,
}

/// A new window asked of process `pid` by `origin`: the first window it then has that `known`
/// does not is it. Kept a while after the core gave up (`since`), so a late one is passed over.
pub(crate) struct Asked {
    pid: i32,
    known: HashSet<u32>,
    origin: Origin,
    since: Instant,
}

/// How long a late window is looked for after the ask, to leave it outside.
const LATE: Duration = Duration::from_secs(30);

impl Asked {
    /// Whether `pid`'s new windows are this ask's to host: none other may split them in.
    pub(crate) fn of(&self, pid: i32) -> bool {
        self.pid == pid
    }
}

#[derive(Deserialize)]
struct FixtureWindow {
    window: u32,
    title: String,
}

/// A started `open` or `osascript`, waited for off the main thread so it leaves no zombie.
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || child.wait());
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
    unpickable(window).is_none()
}

/// Why `pickable` says no, for the log: `minimised`, `sheet`, `tiny`, or the rule (`built-in rule 0: ignore`).
fn unpickable(window: &ax::Window) -> Option<String> {
    let (frame, sheet) = window.quick(|w| (w.frame(), w.sheet()));
    let shape = Shape { sheet, size: frame.map(|f| [f.size.width, f.size.height]), ..Shape::default() };
    if window.minimized() {
        return Some("minimised".into());
    }
    let (why, rule) = with_facts(window, |facts| rules::unpickable(&config::live().windows, facts, &shape))?;
    Some(match rule {
        Some(source) => format!("{}: {}", source.label(), rules::Action::Ignore.name()),
        None => why.name().into(),
    })
}

/// A window's subrole for the log: an app may give any text, so only an `AX` word, else `other`.
fn logged_subrole(window: &ax::Window) -> String {
    let subrole = window.quick(ax::Window::subrole).unwrap_or_default();
    let word = subrole.len() <= 40 && subrole.starts_with("AX") && subrole.chars().all(|c| c.is_ascii_alphanumeric());
    if word { subrole } else { "other".into() }
}

impl Workspace {
    /// Carries out `Effect::PickWindows`: `app`'s windows that may go in a pane, to the core.
    pub(crate) fn pick_windows(&mut self, app: &str, window: &Window, cx: &mut Context<Self>) {
        let fixture = fixture_apps().into_iter().find(|fixture| fixture.name == app);
        let mut bundle = fixture.as_ref().and_then(|fixture| fixture.bundle.clone());
        let mut menu_item = fixture.as_ref().is_some_and(|fixture| fixture.new_window_item);
        let fixture = fixture.into_iter().flat_map(|fixture| fixture.windows);
        let mut windows: Vec<PickWindow> = fixture.map(|w| PickWindow { window: w.window, title: w.title }).collect();
        if let Some(pid) = self.pid_of(app) {
            let read = |w: ax::Window| Some(PickWindow { window: w.id()?, title: w.title().unwrap_or_default() });
            let mut skipped = Vec::new();
            for w in ax::Window::all_of(pid) {
                // A window that closed while read has no id: said, not lost.
                let why = unpickable(&w).or_else(|| w.id().is_none().then(|| "no_id".into()));
                match why {
                    None => windows.extend(read(w)),
                    Some(why) => skipped.push(serde_json::json!({ "why": why, "subrole": logged_subrole(&w) })),
                }
            }
            bundle = ax::app_identity(pid).0;
            // What the list left out and why, so a window that cannot be picked can be told apart: never a title.
            let fields = serde_json::json!({ "bundle": bundle, "listed": windows.len(), "skipped": skipped });
            crate::event_log::event("pick_list", fields);
            // Read only for an app with no recipe of its own: it walks the menus.
            menu_item =
                new_window::recipe(bundle.as_deref(), false).is_none() && ax::has_menu_item(pid, new_window::MENU_ITEM);
        }
        let action = Action::PickerWindows { app: app.into(), bundle, menu_item, windows };
        self.dispatch(action, self.input, window, cx);
    }

    /// The running app of that name, as the picker listed it.
    fn pid_of(&self, app: &str) -> Option<i32> {
        self.apps.iter().find(|running| running.name == app).map(|running| running.pid)
    }

    /// Carries out `Effect::NewWindow`: asks `app` by `recipe`, then waits for the window
    /// (`host_new_window`). The core says when it is too late. A test run counts the ask.
    pub(crate) fn new_window(
        &mut self,
        app: &str,
        bundle: Option<&str>,
        recipe: Recipe,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let kind = match recipe {
            Recipe::Open { .. } => 0,
            Recipe::Tell { .. } => 1,
            Recipe::MenuItem => 2,
        };
        self.new_window_asks[kind] += 1;
        self.asked = None;
        let Some(pid) = self.pid_of(app).filter(|_| self.hosting) else { return };
        if self.host_only.is_some_and(|only| only != pid) {
            return;
        }
        // Every window it has now, each by its id: one not read would pass for the new one.
        let known: Option<HashSet<u32>> = ax::Window::all_of(pid).iter().map(ax::Window::id).collect();
        let Some(known) = known.filter(|known| !known.is_empty() || ax::Window::answers(pid)) else {
            let text = format!("{app} did not say which windows it has: no new window asked");
            self.dispatch(Action::NewWindowFailed { text }, Origin::System, window, cx);
            return;
        };
        self.asked = Some(Asked { pid, known, origin: self.input, since: Instant::now() });
        // Fixed argv: the bundle id and the command come from the recipe table, never from input.
        let asked = match (recipe, bundle) {
            (Recipe::Open { args }, Some(bundle)) => std::process::Command::new("/usr/bin/open")
                .args(["-n", "-g", "-b", bundle, "--args"])
                .args(args)
                .spawn()
                .map(reap)
                .map_err(|why| why.to_string()),
            (Recipe::Tell { command }, Some(bundle)) => std::process::Command::new("/usr/bin/osascript")
                .args(["-e", &format!("tell application id \"{bundle}\" to {command}")])
                .spawn()
                .map(reap)
                .map_err(|why| why.to_string()),
            _ => ax::press_menu_item(pid, new_window::MENU_ITEM),
        };
        // Said as it failed: the wait ends, and no "opened no new window" follows.
        if let Err(why) = asked {
            self.asked = None;
            self.dispatch(Action::NewWindowFailed { text: format!("{app} {why}") }, Origin::System, window, cx);
        }
    }

    /// While "New window" waits: the asked app's first window that is new, hosted, owned, as
    /// whoever asked. It comes before `adopt_new_windows`, which would split it in as a pop-up.
    /// Once the core gave up, a late one is left outside, and no pop-up rule splits it in either.
    pub(crate) fn host_new_window(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(asked) = &self.asked else { return };
        let (pid, late) = (asked.pid, self.model.new_window.is_none());
        if late && asked.since.elapsed() > LATE {
            self.asked = None;
            return;
        }
        let new = ax::Window::all_of(pid).into_iter().filter(|w| w.id().is_some_and(|id| !asked.known.contains(&id)));
        let Some(found) = new.into_iter().find(pickable) else { return };
        let (id, origin) = (found.id(), asked.origin);
        self.asked = None;
        // Seen: no pop-up rule judges it again.
        if let Some((_, seen)) = self.known.iter_mut().find(|(known, _)| *known == pid) {
            seen.extend(id);
        }
        if late {
            return;
        }
        let Ok(hosted) = capture::target(found).and_then(Hosted::new) else { return };
        let kind = hosted.kind();
        self.with_origin(origin, |this| this.attach(hosted, Action::NewWindowCame { kind }, window, cx));
    }

    /// About twice a second: whether each owned pane's app says its window has unsaved changes.
    /// A window that does not answer keeps what was read last.
    pub(crate) fn read_edited(&mut self, window: &Window, cx: &mut Context<Self>) {
        let owned: Vec<PaneId> = self.model.panes.iter().filter(|pane| pane.owned).map(|pane| pane.id).collect();
        for pane in owned {
            let Some(edited) = self.hosted.get(&pane).and_then(|hosted| hosted.window.quick(ax::Window::edited)) else {
                continue;
            };
            if edited != self.model.edited.contains(&pane) {
                self.dispatch(Action::WindowEdited { pane, edited }, Origin::System, window, cx);
            }
        }
    }

    /// Before the user's close of `pane`, if it is owned: read now, as the poll may be half a
    /// second old. A window that does not answer counts as having unsaved changes: asked.
    pub(crate) fn edited_now(&mut self, pane: Option<PaneId>, window: &Window, cx: &mut Context<Self>) {
        let Some(pane) = pane.filter(|id| self.model.pane(*id).is_some_and(|p| p.owned)) else { return };
        let Some(hosted) = self.hosted.get(&pane) else { return };
        let edited = hosted.window.quick(ax::Window::edited).unwrap_or(true);
        if edited != self.model.edited.contains(&pane) {
            self.dispatch(Action::WindowEdited { pane, edited }, Origin::System, window, cx);
        }
    }

    /// Carries out `Effect::CloseWindow`: the owned window goes back where it was, so a save
    /// question of its app's shows, then its close button is pressed.
    pub(crate) fn close_window(&mut self, pane: PaneId, id: u32) {
        let Some(hosted) = self.hosted.remove(&pane) else { return };
        let (pid, app) = (hosted.pid, hosted.app.clone());
        // Not back on a screen: closed there, its app's question would go unseen. It stays open.
        if !hosted.give_back() {
            return self.broke(format!("{app}'s window did not come back on screen: it stays open"));
        }
        crate::give_back::forget(id);
        if let Some(found) = ax::Window::find(pid, id)
            && let Err(why) = found.close()
        {
            self.broke(format!("{app}'s window {why}"));
        }
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
        let pid = self.pid_of(app).ok_or_else(|| format!("{app} is not running"))?;
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
        let rows = picker.matches().into_iter().enumerate().map(|(i, item)| {
            let muted = |text: String| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(text);
            let (icon, note) = match item {
                // Opened for the pane: it closes with it.
                PickItem::NewWindow { .. } => {
                    let plus = div().text_color(rgb(theme::PICK)).child("+");
                    (
                        div().w(px(28.)).flex_none().flex().justify_center().child(plus),
                        Some(muted("closes with its pane".into())),
                    )
                }
                // Held now: a pane may have opened or closed since the windows were read.
                PickItem::Window { window: id, .. } => {
                    let held = self.model.pane_holding(*id).map(|pane| muted(format!("in pane {pane}")));
                    (div().w(px(28.)).flex_none().child(self.app_icon(app)), held)
                }
                _ => (div().w(px(28.)).flex_none(), None),
            };
            dialog::filter_row(("pick", i), i == selected)
                .child(icon)
                .child(div().flex_1().min_w_0().truncate().child(item.label().to_string()))
                .children(note)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::PickerChoose { index: Some(i) }, this.input, window, cx);
                }))
                .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        let title = format!("{} {app}", picker_title(picker)).into();
        self.picker_panel(360., title, rows, "No match", cx)
    }
}
