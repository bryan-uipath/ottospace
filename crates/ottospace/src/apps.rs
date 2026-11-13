//! The apps on this Mac that are not running, for the Apps section and the picker: the folders
//! scanned off the main thread (kept for the session, read again if old), the Dock's apps, and
//! the settings' pinned and hidden ones. The core decides what is listed; this reads, draws the
//! section, and writes a pin to the settings file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AnyElement, AnyView, ClickEvent, Context, ElementId, KeyDownEvent, Window, div, prelude::*, px, rgb};
use ottospace_core::apps::{Entry, Group, Row};
use ottospace_core::settings::{Installed, expand_home};
use ottospace_core::{Action, Axis, Origin, Section, theme};
use ottospace_mac::icon;

use crate::{ICON_PIXELS, Tooltip, Workspace, config, list_button};

/// A scan older than this is done again when the picker or the Apps section opens.
const FRESH: Duration = Duration::from_secs(60);
/// Icons read in one go: reading one draws it, on the main thread.
const ICONS_AT_ONCE: usize = 8;

#[derive(Default)]
pub struct Scan {
    apps: Vec<Installed>,
    /// When the last scan started. `None`: never.
    at: Option<Instant>,
    /// Apps whose icon was asked for, by bundle path: one that has none is not asked again.
    icons_tried: HashSet<String>,
    /// The bundles that ran when the core was last told.
    running: Vec<String>,
    /// More icons are on their way.
    loading: bool,
}

/// The folders scanned: `$OTTOSPACE_APPS_DIRS` (separated by `:`), else where apps usually are.
/// An E2E run that names none scans none: a test never lists the user's apps.
fn folders() -> Vec<PathBuf> {
    match std::env::var_os("OTTOSPACE_APPS_DIRS") {
        Some(dirs) => std::env::split_paths(&dirs).filter(|dir| !dir.as_os_str().is_empty()).collect(),
        None if std::env::var_os("OTTOSPACE_E2E").is_some() => Vec::new(),
        None => icon::app_folders(),
    }
}

/// The app bundle at `path`, resolved as the scan and the running apps are, so the same app
/// compares equal wherever it was named. e.g. `/Applications/Notes.app` → `Notes`.
fn bundle(path: &Path) -> Option<Installed> {
    let path = path.canonicalize().ok()?;
    let name = path.file_name()?.to_str()?.strip_suffix(".app")?.to_string();
    path.is_dir().then(|| Installed { name, path: path.to_string_lossy().into_owned() })
}

/// The app a line of `apps` or `apps_hidden` names: a path (`/…` or `~/…`), else a name looked
/// for in the scanned folders, whatever the case of its letters.
fn resolve(entry: &str) -> Option<Installed> {
    let entry = entry.trim();
    if entry.starts_with('/') || entry.starts_with('~') {
        return bundle(Path::new(&expand_home(entry, &std::env::var("HOME").unwrap_or_default())));
    }
    // As a mark looks an app up by its name: one name, one bundle.
    Some(Installed { name: entry.to_string(), ..bundle(&icon::app_path_in(entry, &folders())?)? })
}

/// The Dock's apps that exist, in its order: `$OTTOSPACE_DOCK_APPS` (bundle paths separated by
/// `:`), else the Dock's own preferences. A test run that names none has none: it never reads
/// the user's Dock.
fn dock_apps(unread: bool) -> Vec<Installed> {
    let paths: Vec<PathBuf> = match std::env::var_os("OTTOSPACE_DOCK_APPS") {
        Some(list) => std::env::split_paths(&list).filter(|path| !path.as_os_str().is_empty()).collect(),
        None if unread => Vec::new(),
        None => ottospace_mac::dock::persistent_apps(),
    };
    paths.iter().filter_map(|path| bundle(path)).collect()
}

/// The bundle path of every running app, windows or none: an app with no window on this Space
/// (minimised, on another Space, in the menu bar) is running, not one to start.
pub fn running_bundles() -> Vec<String> {
    icon::running_bundles().into_iter().map(|path| path.to_string_lossy().into_owned()).collect()
}

impl Workspace {
    /// The Dock's own preferences are not for reading: the settings turn suggestions off, or
    /// this is a test run (it hosts only its own test app, or no window at all).
    fn dock_unread(&self) -> bool {
        let off = config::get().settings.suggest_dock_apps == Some(false);
        off || self.host_only.is_some() || std::env::var_os("OTTOSPACE_E2E").is_some()
    }

    /// Tells the core what the settings say of apps. Once, at start: after that the core's
    /// lists are the ones a pin changes and `save_apps` writes.
    pub fn tell_app_settings(&mut self) {
        let settings = &config::get().settings;
        // A run that hosts only its own test app lists none of the user's.
        let offer = self.host_only.is_none();
        let entries = |list: &[String]| -> Vec<Entry> {
            let entry = |text: &String| Entry { text: text.clone(), app: resolve(text) };
            list.iter().filter(|_| offer).map(entry).collect()
        };
        let (pinned, hidden) = (entries(&settings.apps), entries(&settings.apps_hidden));
        let suggest = offer && settings.suggest_dock_apps.unwrap_or(true);
        let told = Action::SetAppSettings { pinned, hidden, suggest };
        self.model.update(told.sent_by(Origin::System));
        config::remove_stale_drafts();
    }

    /// The picker or the Apps section opened: tells the core what is installed as last scanned,
    /// reads the Dock's apps, and scans again if that was a while ago. The answers come later.
    pub fn offer_installed(&mut self, cx: &mut Context<Self>) {
        self.tell_installed();
        let scan = self.installed.at.is_none_or(|at| at.elapsed() >= FRESH);
        if scan {
            self.installed.at = Some(Instant::now());
        }
        let unread = self.dock_unread();
        let read = cx.background_executor().spawn(async move {
            let app = |(name, path): (String, PathBuf)| Installed { name, path: path.to_string_lossy().into_owned() };
            let apps = scan.then(|| icon::apps_in(&folders()).into_iter().map(app).collect::<Vec<_>>());
            (apps, dock_apps(unread))
        });
        cx.spawn(async move |this, cx| {
            let (apps, dock) = read.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(apps) = apps {
                    this.installed.apps = apps;
                }
                this.model.update(Action::SetDock { apps: dock }.sent_by(Origin::System));
                this.tell_installed();
                this.load_icons(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The scan's apps and the bundles running now: the core lists those that are not.
    pub fn tell_installed(&mut self) {
        let (apps, running) = (self.installed.apps.clone(), running_bundles());
        // An app that ran and quit is listed again: its icon is asked for again.
        let quit: Vec<String> = self.installed.running.iter().filter(|path| !running.contains(path)).cloned().collect();
        self.installed.icons_tried.retain(|path| !quit.contains(path));
        self.installed.running = running.clone();
        self.model.update(Action::SetInstalled { apps, running }.sent_by(Origin::System));
    }

    /// Reads the icons of the listed apps that are not running, a few at a time: "All apps" may
    /// hold hundreds, and a frame must not wait for them.
    pub fn load_icons(&mut self, cx: &mut Context<Self>) {
        if self.model.sidebar != Some(Section::Apps) || self.installed.loading {
            return;
        }
        let bar = self.model.apps_bar();
        let wanted =
            |row: &&Row| !self.icons.contains_key(&row.path) && !self.installed.icons_tried.contains(&row.path);
        let rows: Vec<Row> =
            bar.not_running.iter().chain(&bar.all).filter(wanted).take(ICONS_AT_ONCE).cloned().collect();
        for row in &rows {
            // By its path: two apps of one name each have their own icon.
            self.installed.icons_tried.insert(row.path.clone());
            if let Some(png) = icon::png_of_app(Path::new(&row.path), ICON_PIXELS) {
                let image = Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png));
                self.icons.insert(row.path.clone(), image);
            }
        }
        if rows.len() < ICONS_AT_ONCE {
            return;
        }
        self.installed.loading = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(10)).await;
            let _ = this.update(cx, |this, cx| {
                this.installed.loading = false;
                this.load_icons(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts a listed app that is not running. Only one the core knows (the scan, the Dock or
    /// the settings found it), and never in a test run: a real app would start.
    pub fn launch_picked(&mut self, name: &str, path: &str, axis: Axis, new_tab: bool) {
        let known = self.model.app_lists.known(path, &self.installed.apps).is_some();
        let started = if !self.hosting || self.host_only.is_some() {
            Err("window hosting is off in e2e runs".to_string())
        } else if !known {
            Err(format!("{name} is not an app found on this Mac"))
        } else {
            self.launch(Installed { name: name.into(), path: path.into() }, axis, new_tab, None)
        };
        self.error = started.err();
    }

    /// The pinned apps that are not running, in their order.
    pub fn pinned_closed(&self) -> impl Iterator<Item = Row> + use<> {
        self.model.apps_bar().not_running.into_iter().filter(|app| app.pinned)
    }

    /// Carries out `Effect::SaveApps`: makes the change to `apps` or `apps_hidden` in
    /// `settings.json` as it is now. A file that cannot be read as settings, or whose list is
    /// not a list of names, is left alone, and the status bar says so; the pin then lasts
    /// until ottospace quits.
    pub fn save_apps(&mut self, changes: &[ottospace_core::settings::ListChange]) {
        const NOT_WRITTEN: &str = "settings.json was not written: ";
        // The lists of a run that hosts only its test app are not the user's.
        if self.host_only.is_some() {
            return;
        }
        match config::save_apps(changes) {
            Err(why) => self.error = Some(format!("{NOT_WRITTEN}{why}")),
            Ok(()) if self.error.as_ref().is_some_and(|error| error.starts_with(NOT_WRITTEN)) => self.error = None,
            Ok(()) => {}
        }
    }

    /// A click on a control of the Apps section, by the control's name and the app's bundle
    /// path: `closed-app` (start it), `app-pin`, `app-hide`.
    pub(crate) fn click_app_control(
        &mut self,
        control: &str,
        path: &str,
        axis: Axis,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let action = match control {
            "app-pin" => Action::AppPin { path: path.into() },
            "app-hide" => Action::AppHide { path: path.into() },
            _ => {
                let name = self.model.app_lists.known(path, &self.installed.apps).map(|app| app.name);
                self.launch_picked(&name.unwrap_or_default(), path, axis, false);
                return cx.notify();
            }
        };
        self.dispatch(action, self.input, window, cx);
    }

    /// A click on `apps-more`, `apps-all`, `apps-filter` or `apps-undo`.
    pub(crate) fn click_apps_control(&mut self, control: &str, window: &Window, cx: &mut Context<Self>) {
        let action = match control {
            "apps-more" => Action::AppsToggle { group: Group::More },
            "apps-all" => Action::AppsToggle { group: Group::All },
            "apps-filter" => Action::AppsFilter { on: true },
            "apps-undo" => Action::AppsUndo,
            _ => return,
        };
        self.dispatch(action, self.input, window, cx);
    }

    /// A key while the filter of "All apps" has the keyboard: ↵, ↓, Tab or Esc give the keys
    /// back to the rows; any other edits the text or types into it.
    pub(crate) fn apps_filter_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if matches!(key.key.as_str(), "enter" | "down" | "tab" | "escape") && !key.modifiers.platform {
            self.dispatch(Action::AppsFilter { on: false }, self.input, window, cx);
            cx.stop_propagation();
        } else if self.field_key(event, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Where list row `at` of the Apps section is among the children of its list, to scroll it
    /// into view: the undo line, the "Not running" heading and the filter are no rows.
    pub(crate) fn apps_child(&self, at: usize) -> usize {
        let bar = self.model.apps_bar();
        let running = self.apps.len();
        let folded = running + bar.not_running.len() + usize::from(bar.more > 0) + usize::from(bar.all_count > 0);
        let heading = !bar.not_running.is_empty() && at >= running;
        at + usize::from(bar.undo.is_some()) + usize::from(heading) + usize::from(bar.all_open && at >= folded)
    }

    /// An app that is not running, as a row. A click starts it and hosts its window; hover (or
    /// list mode's selection) shows what `p` and `x` do, as buttons.
    fn closed_row(&self, row: &Row, at: usize, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.model.list == Some(at);
        let waiting = self.launching.iter().any(|launch| launch.app.path == row.path);
        let button = |name: &'static str, label: &'static str, tip: &'static str| {
            let path = row.path.clone();
            div()
                .id(ElementId::Name(format!("{name}:{}", row.path).into()))
                .relative()
                .child(self.controls.mark(format!("{name}:{}", row.path)))
                .px_1()
                .rounded_sm()
                .text_xs()
                .text_color(rgb(theme::TEXT))
                .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
                .child(label)
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation(); // not a click on the row
                    this.click_app_control(name, &path, Axis::Right, window, cx);
                }))
        };
        let pin = match row.pinned {
            true => button("app-pin", "Unpin", "Take off the pinned apps (P)"),
            false => button("app-pin", "Pin", "Keep in this list (P)"),
        };
        let hide = row.suggested.then(|| button("app-hide", "Hide", "Do not suggest this app (X)"));
        // Over the end of the row, so the name keeps its room while they are hidden.
        let actions = div()
            .absolute()
            .right(px(6.))
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .gap_0p5()
            .pl_1()
            .bg(rgb(theme::SELECTED))
            .when(!selected, |d| d.invisible().group_hover("closed-app-row", |s| s.visible()))
            .child(pin)
            .children(hide);
        let path = row.path.clone();
        list_button(ElementId::Name(format!("closed-app:{}", row.path).into()), selected)
            .group("closed-app-row")
            .relative()
            .py_1p5()
            .gap_3()
            .text_color(rgb(theme::MUTED))
            .child(self.app_icon(&row.path))
            .child(self.controls.mark(format!("closed-app:{}", row.path)))
            .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
            .when(row.pinned, |d| d.child(div().flex_none().text_xs().text_color(rgb(theme::BORDER_STRONG)).child("★")))
            .when(waiting, |d| d.child(div().flex_none().text_xs().child("opening…")))
            .child(actions)
            // Fills the focused pane if it is empty. Else splits it: right, or down with shift.
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                this.click_app_control("closed-app", &path, axis, window, cx);
            }))
            .into_any_element()
    }

    /// The Apps section: running apps (a click hosts the app's main window in a pane), then
    /// those that are not running: pinned, then the Dock's; then every other installed app,
    /// folded, with a filter. After a pin, unpin or hide a line at the top offers to undo it.
    pub(crate) fn render_apps(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = self.model.apps_bar();
        let muted = || div().px_2().text_xs().text_color(rgb(theme::MUTED));
        // A line that is a control: "and N more", the "All apps" heading.
        let fold = |id: &'static str, open: bool, label: String, at: usize, cx: &mut Context<Self>| {
            muted()
                .id(id)
                .relative()
                .flex()
                .items_center()
                .gap_1p5()
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .when(self.model.list == Some(at), |d| d.pb_0p5().rounded_md().bg(rgb(theme::SELECTED)))
                .child(div().w(px(8.)).child(if open { "▾" } else { "▸" }))
                .child(label)
                .child(self.controls.mark(id))
                .on_click(cx.listener(move |this, _, window, cx| this.click_apps_control(id, window, cx)))
        };
        let notice = bar.undo.map(|moved| {
            self.undo_line("apps-undo", moved)
                .on_click(cx.listener(|this, _, window, cx| this.click_apps_control("apps-undo", window, cx)))
                .into_any_element()
        });
        let running = self.apps.iter().enumerate().map(|(i, app)| {
            let target = app.name.clone();
            list_button(("app", i), self.model.list == Some(i))
                .py_1p5()
                .gap_3()
                .child(self.app_icon(&app.name))
                .child(self.controls.mark(format!("app:{target}")))
                .child(div().flex_1().min_w_0().truncate().child(app.name.clone()))
                // How many windows the app has open, when more than one.
                .when(app.windows > 1, |row| {
                    // A fixed box with the digits centred in it. Digits have no descender, so
                    // a line centred by its box sits high: one point of padding evens it.
                    let badge = div().h(px(18.)).min_w(px(18.)).px_1().pt(px(1.)).flex_none().flex();
                    let badge = badge.items_center().justify_center().rounded_full().bg(rgb(theme::SELECTED));
                    let badge = badge.text_xs().line_height(px(16.)).text_color(rgb(theme::MUTED));
                    row.child(badge.child(app.windows.to_string()))
                })
                // Fills the focused pane if it is empty. Else splits it: right, or down with shift.
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                    let _ = this.open_app(&target, axis, false, window, cx);
                }))
                .into_any_element()
        });
        let mut rows: Vec<AnyElement> = notice.into_iter().chain(running).collect();
        // List mode walks the rows as drawn: `at` is the next row's place.
        let mut at = self.apps.len();
        if !bar.not_running.is_empty() {
            rows.push(muted().pt_2().child("Not running").into_any_element());
        }
        for row in &bar.not_running {
            rows.push(self.closed_row(row, at, cx));
            at += 1;
        }
        if bar.more > 0 {
            let label = if bar.more_open { "Show fewer".to_string() } else { format!("and {} more", bar.more) };
            rows.push(fold("apps-more", bar.more_open, label, at, cx).py_0p5().into_any_element());
            at += 1;
        }
        if bar.all_count > 0 {
            let label = format!("All apps ({})", bar.all_count);
            rows.push(fold("apps-all", bar.all_open, label, at, cx).pt_2().into_any_element());
            at += 1;
        }
        if bar.all_open {
            // The field is the one that has the keyboard; at rest the line shows its text.
            let line = div().id("apps-filter").relative().mx_2().h(px(26.)).px_2().flex().items_center();
            let line = line.rounded_md().border_1().text_xs().child(self.controls.mark("apps-filter"));
            let line = match bar.filtering {
                true => line.border_color(rgb(theme::LIST)).child(self.render_field("type to filter", cx)),
                false => {
                    let text = if bar.filter.is_empty() { "/ to filter".to_string() } else { bar.filter.clone() };
                    line.border_color(rgb(theme::BORDER))
                        .text_color(rgb(theme::MUTED))
                        .cursor_text()
                        .child(text)
                        .on_click(cx.listener(|this, _, window, cx| this.click_apps_control("apps-filter", window, cx)))
                }
            };
            rows.push(line.into_any_element());
            if bar.all.is_empty() {
                rows.push(muted().child("No app has that in its name").into_any_element());
            }
        }
        for row in &bar.all {
            rows.push(self.closed_row(row, at, cx));
            at += 1;
        }
        let list = div().id("apps").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        // The section's scrollbar, over all three groups.
        self.bars.tracked("apps", list.track_scroll(&self.list_scroll).children(rows), &self.list_scroll)
    }
}
