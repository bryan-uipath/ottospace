//! The Apps section's lists of apps that are not running: the user's pinned apps, the Dock's
//! as suggestions, and every other installed app behind a heading. The shell reads the Dock,
//! the folders and the settings, and writes the settings; what is listed is decided here.

use serde::{Deserialize, Serialize};

use crate::edit::Field;
use crate::layout::Axis;
use crate::list::ListRow;
use crate::model::{Effect, MAX_FILTER, Model, Section};
use crate::settings::{Installed, ListChange};
use crate::tasks::{Moved, NOTICE_MS, Notice};

/// Why a client's pin, unpin, hide or undo is refused.
pub const USERS_ALONE: &str = "settings are the user's to change";

/// How many suggestions show before "and N more".
pub const SUGGESTED: usize = 12;

/// A line of `apps` or `apps_hidden` in the settings, and the app it names, if it was found.
/// A line that names no app found is kept as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// As written: `Slack`, or `/Users/me/Applications/Lumen.app`.
    pub text: String,
    pub app: Option<Installed>,
}

/// A part of the section that folds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// The suggestions past the first `SUGGESTED`.
    More,
    /// Every other installed app.
    All,
}

/// An app that is not running, as a row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub name: String,
    pub path: String,
    pub pinned: bool,
    /// It is listed because the Dock has it: it can be hidden.
    pub suggested: bool,
}

/// The section below the running apps, as drawn.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Bar {
    /// Pinned apps, then the Dock's, in the Dock's order.
    pub not_running: Vec<Row>,
    /// Suggestions past the first `SUGGESTED`: "and N more". 0: no such row.
    pub more: usize,
    pub more_open: bool,
    /// How many other apps are installed, whatever the filter.
    pub all_count: usize,
    pub all_open: bool,
    /// The other apps, by name, narrowed by the filter. None while the group is folded.
    pub all: Vec<Row>,
    pub filter: String,
    /// The filter has the keyboard.
    pub filtering: bool,
    /// What the line at the top offers to take back ("Pinned"), while it shows.
    pub undo: Option<&'static str>,
}

/// What the user chose for the section.
#[derive(Clone, Debug, PartialEq)]
pub struct Lists {
    pinned: Vec<Entry>,
    hidden: Vec<Entry>,
    /// `suggest_dock_apps`.
    suggest: bool,
    /// The Dock's apps, in its order.
    dock: Vec<Installed>,
    more_open: bool,
    all_open: bool,
    pub filter: Field,
    pub filtering: bool,
    /// `pinned` and `hidden` before the last pin, unpin or hide.
    before: Option<(Vec<Entry>, Vec<Entry>)>,
    /// A client may pin, hide and undo, which write the user's settings file: only in a test
    /// run, which says so. Otherwise those are the user's alone (`Model::refusal`).
    pub clients_write: bool,
}

impl Default for Lists {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            hidden: Vec::new(),
            suggest: true,
            dock: Vec::new(),
            more_open: false,
            all_open: false,
            filter: Field::new("", MAX_FILTER),
            filtering: false,
            before: None,
            clients_write: false,
        }
    }
}

/// The app a name in the settings means: the first the scan found of that name, whatever the
/// case of its letters. The scan goes through the folders in the order a name is looked for.
fn named<'a>(name: &str, installed: &'a [Installed]) -> Option<&'a Installed> {
    // No name names no app.
    installed.iter().find(|app| !name.trim().is_empty() && app.name.eq_ignore_ascii_case(name))
}

/// What the settings say of `app`: its name, so the file reads `"Slack"`, but only if that
/// name means this very bundle when it is read back. Else its path: two folders may each hold
/// a `Bar.app`, and a name would pin the other one at the next start.
fn entry_text(app: &Installed, installed: &[Installed]) -> String {
    let same = named(&app.name, installed).is_some_and(|found| found.path == app.path);
    if same { app.name.clone() } else { app.path.clone() }
}

/// What to change in the settings file to go from one pair of lists to another: each entry
/// that is new, each that is gone.
fn changes(before: &(Vec<Entry>, Vec<Entry>), after: &(Vec<Entry>, Vec<Entry>)) -> Vec<ListChange> {
    let mut found = Vec::new();
    for (key, was, is) in [("apps", &before.0, &after.0), ("apps_hidden", &before.1, &after.1)] {
        let has = |list: &[Entry], text: &str| list.iter().any(|entry| entry.text == text);
        let gone = was.iter().filter(|entry| !has(is, &entry.text));
        found.extend(gone.map(|entry| ListChange { key, add: false, text: entry.text.clone() }));
        let new = is.iter().filter(|entry| !has(was, &entry.text));
        found.extend(new.map(|entry| ListChange { key, add: true, text: entry.text.clone() }));
    }
    found
}

fn names(entries: &[Entry], path: &str) -> bool {
    entries.iter().any(|entry| entry.app.as_ref().is_some_and(|app| app.path == path))
}

/// A row for `app`, unless a row or a running app already has its bundle.
fn take<'a>(listed: &mut Vec<&'a str>, app: &'a Installed, pinned: bool) -> Option<Row> {
    let new = !listed.contains(&app.path.as_str());
    listed.push(&app.path);
    new.then(|| Row { name: app.name.clone(), path: app.path.clone(), pinned, suggested: false })
}

impl Lists {
    pub fn set(&mut self, pinned: Vec<Entry>, hidden: Vec<Entry>, suggest: bool) {
        // An empty line of the file names no app, whatever came with it: it is kept, and
        // nothing can be done to it from here.
        let read = |mut entry: Entry| {
            entry.app = entry.app.filter(|app| !entry.text.trim().is_empty() && !app.name.trim().is_empty());
            entry
        };
        let (pinned, hidden) = (pinned.into_iter().map(read).collect(), hidden.into_iter().map(read).collect());
        (self.pinned, self.hidden, self.suggest, self.before) = (pinned, hidden, suggest, None);
    }

    /// The scan came in: an entry that named no app found (one installed since, or read before
    /// the first scan) is looked for again, by its path or its name.
    pub fn resolve(&mut self, installed: &[Installed]) {
        for entry in self.pinned.iter_mut().chain(&mut self.hidden).filter(|entry| entry.app.is_none()) {
            let text = entry.text.trim();
            let by_path = installed.iter().find(|app| app.path == text);
            entry.app = by_path.or_else(|| named(text, installed).filter(|_| !text.contains('/'))).cloned();
        }
    }

    pub fn set_dock(&mut self, apps: Vec<Installed>) {
        self.dock = apps;
    }

    /// The rows. `installed`: the apps the scan found. `running`: the bundles that run now.
    /// An app is one bundle path: it is listed once, in the first group that has it.
    pub fn bar(&self, installed: &[Installed], running: &[String]) -> Bar {
        let mut listed: Vec<&str> = running.iter().map(String::as_str).collect();
        let pinned = self.pinned.iter().filter_map(|entry| entry.app.as_ref());
        let pinned: Vec<Row> = pinned.filter_map(|app| take(&mut listed, app, true)).collect();
        let offered = self.dock.iter().filter(|app| self.suggest && !names(&self.hidden, &app.path));
        let suggested: Vec<Row> = offered.filter_map(|app| take(&mut listed, app, false)).collect();
        let suggested: Vec<Row> = suggested.into_iter().map(|row| Row { suggested: true, ..row }).collect();
        let mut all: Vec<Row> = installed.iter().filter_map(|app| take(&mut listed, app, false)).collect();
        all.sort_by_key(|row| (row.name.to_lowercase(), row.path.clone()));
        let more = suggested.len().saturating_sub(SUGGESTED);
        let shown = if self.more_open { suggested.len() } else { SUGGESTED };
        let all_count = all.len();
        // As the picker's filter: names that start with it, then names that hold it.
        let query = self.filter.text().trim().to_lowercase();
        let hit = |row: &Row, starts: bool| {
            let name = row.name.to_lowercase();
            name.contains(&query) && name.starts_with(&query) == starts
        };
        let all: Vec<Row> = match (self.all_open, query.is_empty()) {
            (false, _) => Vec::new(),
            (true, true) => all,
            (true, false) => {
                let first = all.iter().filter(|row| hit(row, true));
                first.chain(all.iter().filter(|row| hit(row, false))).cloned().collect()
            }
        };
        Bar {
            not_running: pinned.into_iter().chain(suggested.into_iter().take(shown)).collect(),
            more,
            more_open: self.more_open,
            all_count,
            all_open: self.all_open,
            all,
            filter: self.filter.text().to_string(),
            filtering: self.filtering,
            undo: None,
        }
    }

    /// The app at `path`, if the settings, the Dock or the scan know it. One with no name is
    /// no app: nothing could be written for it.
    pub fn known(&self, path: &str, installed: &[Installed]) -> Option<Installed> {
        let entries = self.pinned.iter().filter_map(|entry| entry.app.as_ref());
        let found = entries.chain(&self.dock).chain(installed).find(|app| app.path == path).cloned();
        found.filter(|app| !app.name.trim().is_empty() && !app.path.trim().is_empty())
    }

    /// Pins the app at `path`, or unpins it if it is pinned. `None`: no such app is known.
    fn pin(&mut self, path: &str, installed: &[Installed]) -> Option<Moved> {
        let before = (self.pinned.clone(), self.hidden.clone());
        let moved = if names(&self.pinned, path) {
            self.pinned.retain(|entry| entry.app.as_ref().is_none_or(|app| app.path != path));
            Moved::AppUnpinned
        } else {
            let app = self.known(path, installed)?;
            // A pin says the app is wanted: it is no longer one to keep out of the suggestions.
            self.hidden.retain(|entry| entry.app.as_ref().is_none_or(|app| app.path != path));
            let text = entry_text(&app, installed);
            // A line that says the same already (it named no app found till now) is that pin.
            match self.pinned.iter_mut().find(|entry| entry.text == text) {
                Some(entry) => entry.app = Some(app),
                None => self.pinned.push(Entry { text, app: Some(app) }),
            }
            Moved::AppPinned
        };
        self.before = Some(before);
        Some(moved)
    }

    /// Stops suggesting the Dock's app at `path`. False: it is not a suggestion.
    fn hide(&mut self, path: &str, installed: &[Installed]) -> bool {
        let suggested = self.suggest && !names(&self.pinned, path) && !names(&self.hidden, path);
        let named = |app: &&Installed| suggested && !app.name.trim().is_empty();
        let Some(app) = self.dock.iter().find(|app| app.path == path).filter(named).cloned() else {
            return false;
        };
        self.before = Some((self.pinned.clone(), self.hidden.clone()));
        let text = entry_text(&app, installed);
        self.hidden.retain(|entry| entry.text != text);
        self.hidden.push(Entry { text, app: Some(app) });
        true
    }

    /// Takes the last pin, unpin or hide back. `None`: there is none. Else what it changes
    /// in the settings file.
    fn undo(&mut self) -> Option<Vec<ListChange>> {
        let before = self.before.take()?;
        let was = (std::mem::take(&mut self.pinned), std::mem::take(&mut self.hidden));
        let undone = changes(&was, &before);
        (self.pinned, self.hidden) = before;
        Some(undone)
    }

    pub fn can_undo(&self) -> bool {
        self.before.is_some()
    }

    /// What the last pin, unpin or hide changes in the settings file.
    fn last_changes(&self) -> Vec<ListChange> {
        let now = (self.pinned.clone(), self.hidden.clone());
        self.before.as_ref().map(|before| changes(before, &now)).unwrap_or_default()
    }
}

impl Model {
    /// The Apps section below the running apps.
    pub fn apps_bar(&self) -> Bar {
        let offer = self.undo_offer.filter(|offer| offer.moved.of_apps() && self.app_lists.can_undo());
        Bar { undo: offer.map(|offer| offer.moved.title()), ..self.app_lists.bar(&self.installed.0, &self.installed.1) }
    }

    /// The Apps section's rows for list mode, as drawn.
    pub(crate) fn app_rows(&self) -> Vec<ListRow> {
        let bar = self.apps_bar();
        let closed = |row: &Row| ListRow::Closed(row.path.clone());
        let mut rows: Vec<ListRow> = self.apps.iter().cloned().map(ListRow::App).collect();
        rows.extend(bar.not_running.iter().map(closed));
        rows.extend((bar.more > 0).then_some(ListRow::MoreApps));
        rows.extend((bar.all_count > 0).then_some(ListRow::AllApps));
        rows.extend(bar.all.iter().map(closed));
        rows
    }

    /// The path of the selected row, if it is an app that is not running.
    fn selected_closed(&self) -> Option<String> {
        match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
            Some(ListRow::Closed(path)) => Some(path),
            _ => None,
        }
    }

    /// Starts the listed app at `path`: the shell hosts its window once it has one.
    pub(crate) fn app_launch(&self, path: &str, axis: Axis, effects: &mut Vec<Effect>) {
        if let Some(app) = self.app_lists.known(path, &self.installed.0) {
            effects.push(Effect::LaunchApp { name: app.name, path: app.path, axis, new_tab: false });
        }
    }

    /// A change to the pinned or hidden apps: the shell writes it, and a line offers it back.
    fn apps_changed(&mut self, moved: Moved, effects: &mut Vec<Effect>) {
        self.undo_offer = Some(Notice { moved, until: self.now.saturating_add(NOTICE_MS) });
        // A pin of an app the file names already (by a line that found no app till now)
        // changes nothing in it.
        let changes = self.app_lists.last_changes();
        if !changes.is_empty() {
            effects.push(Effect::SaveApps { changes });
        }
    }

    pub(crate) fn app_pin(&mut self, path: &str, effects: &mut Vec<Effect>) {
        if let Some(moved) = self.app_lists.pin(path, &self.installed.0) {
            self.apps_changed(moved, effects);
        }
    }

    pub(crate) fn app_hide(&mut self, path: &str, effects: &mut Vec<Effect>) {
        if self.app_lists.hide(path, &self.installed.0) {
            self.apps_changed(Moved::AppHidden, effects);
        }
    }

    pub(crate) fn list_pin(&mut self, effects: &mut Vec<Effect>) {
        if let Some(path) = self.selected_closed() {
            self.app_pin(&path, effects);
        }
    }

    /// `x` on a suggestion hides it.
    pub(crate) fn list_hide(&mut self, effects: &mut Vec<Effect>) {
        if let Some(path) = self.selected_closed() {
            self.app_hide(&path, effects);
        }
    }

    pub(crate) fn apps_undo(&mut self, effects: &mut Vec<Effect>) {
        if let Some(changes) = self.app_lists.undo() {
            self.undo_offer = self.undo_offer.filter(|offer| !offer.moved.of_apps());
            if !changes.is_empty() {
                effects.push(Effect::SaveApps { changes });
            }
        }
    }

    pub(crate) fn apps_toggle(&mut self, group: Group) {
        let lists = &mut self.app_lists;
        match group {
            Group::More => lists.more_open = !lists.more_open,
            Group::All => {
                lists.all_open = !lists.all_open;
                // Folded, it starts over: no filter, and no keyboard for it.
                if !lists.all_open {
                    (lists.filter, lists.filtering) = (Field::new("", MAX_FILTER), false);
                }
            }
        }
    }

    /// Gives the keyboard to the filter of "All apps" (the group unfolds, list mode comes on),
    /// or takes it back: the selection then goes to the first app the filter left. Only in the
    /// Apps section: `/` is no key of the others.
    pub(crate) fn apps_filter(&mut self, on: bool, effects: &mut Vec<Effect>) {
        if on && self.sidebar != Some(Section::Apps) {
            return;
        }
        if on {
            self.leave(effects);
            (self.app_lists.all_open, self.app_lists.filtering) = (true, true);
        } else if self.app_lists.filtering {
            self.app_lists.filtering = false;
        } else {
            return;
        }
        let heading = self.app_rows().iter().position(|row| *row == ListRow::AllApps).unwrap_or(0);
        let first = (!on && !self.apps_bar().all.is_empty()).then_some(heading + 1);
        self.list = Some(first.unwrap_or(heading));
    }

    /// The filter keeps the keyboard only while list mode is on in the Apps section.
    pub(crate) fn settle_app_filter(&mut self) {
        if self.list.is_none() || self.sidebar != Some(Section::Apps) {
            self.app_lists.filtering = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;
    use crate::model::{Action, Origin};

    fn app(name: &str) -> Installed {
        Installed { name: name.into(), path: format!("/Applications/{name}.app") }
    }

    fn entry(name: &str) -> Entry {
        Entry { text: name.into(), app: Some(app(name)) }
    }

    fn path(name: &str) -> String {
        app(name).path
    }

    /// The names of rows, a `*` after a pinned one.
    fn shown(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|row| format!("{}{}", row.name, if row.pinned { "*" } else { "" })).collect()
    }

    struct Case {
        what: &'static str,
        pinned: &'static [&'static str],
        hidden: &'static [&'static str],
        dock: &'static [&'static str],
        installed: &'static [&'static str],
        running: &'static [&'static str],
        suggest: bool,
        not_running: &'static [&'static str],
        all: &'static [&'static str],
    }

    #[test]
    fn the_groups_are_pinned_then_the_dock_then_every_other_app() {
        const NONE: &[&str] = &[];
        let base = Case {
            what: "",
            pinned: NONE,
            hidden: NONE,
            dock: NONE,
            installed: NONE,
            running: NONE,
            suggest: true,
            not_running: NONE,
            all: NONE,
        };
        let cases = [
            Case {
                what: "pinned come before the Dock's, which keep the Dock's order",
                pinned: &["Zed", "Arc"],
                dock: &["Slack", "Notes", "Figma"],
                installed: &["Arc", "Figma", "Notes", "Slack", "Zed", "calc", "Bear"],
                not_running: &["Zed*", "Arc*", "Slack", "Notes", "Figma"],
                all: &["Bear", "calc"],
                ..base
            },
            Case {
                what: "an app that runs is in no group",
                pinned: &["Zed", "Arc"],
                dock: &["Slack", "Notes"],
                installed: &["Arc", "Notes", "Slack", "Zed", "Bear"],
                running: &["Arc", "Slack", "Bear"],
                not_running: &["Zed*", "Notes"],
                ..base
            },
            Case {
                what: "a hidden app is not suggested, and is still among all apps",
                hidden: &["Notes"],
                dock: &["Slack", "Notes"],
                installed: &["Notes", "Slack"],
                not_running: &["Slack"],
                all: &["Notes"],
                ..base
            },
            Case {
                what: "an app is listed once: pinned twice, pinned and in the Dock, twice in the Dock",
                pinned: &["Zed", "Zed", "Slack"],
                dock: &["Slack", "Notes", "Notes"],
                installed: &["Zed", "Slack", "Notes", "Notes", "Bear", "Bear"],
                not_running: &["Zed*", "Slack*", "Notes"],
                all: &["Bear"],
                ..base
            },
            Case {
                what: "suggestions turned off: the Dock's apps are among all apps",
                pinned: &["Zed"],
                dock: &["Slack"],
                installed: &["Slack", "Zed"],
                suggest: false,
                not_running: &["Zed*"],
                all: &["Slack"],
                ..base
            },
            Case { what: "nothing known, nothing listed", ..base },
        ];
        for case in cases {
            let mut lists = Lists::default();
            let entries = |names: &[&str]| names.iter().map(|name| entry(name)).collect();
            lists.set(entries(case.pinned), entries(case.hidden), case.suggest);
            lists.set_dock(case.dock.iter().map(|name| app(name)).collect());
            lists.all_open = true;
            let installed: Vec<Installed> = case.installed.iter().map(|name| app(name)).collect();
            let running: Vec<String> = case.running.iter().map(|name| path(name)).collect();
            let bar = lists.bar(&installed, &running);
            assert_eq!(shown(&bar.not_running), case.not_running, "{}", case.what);
            assert_eq!(
                (shown(&bar.all), bar.all_count),
                (case.all.iter().map(|s| s.to_string()).collect(), case.all.len())
            );
            // Only the Dock's can be hidden.
            assert!(bar.not_running.iter().all(|row| row.suggested != row.pinned), "{}", case.what);
        }
    }

    #[test]
    fn suggestions_past_the_cap_wait_behind_a_row_and_pinned_apps_never_do() {
        let names: Vec<String> = (1..=20).map(|n| format!("App {n:02}")).collect();
        let mut lists = Lists::default();
        lists.set((1..=14).map(|n| entry(&format!("Pin {n}"))).collect(), Vec::new(), true);
        lists.set_dock(names.iter().map(|name| app(name)).collect());
        let bar = lists.bar(&[], &[]);
        assert_eq!((bar.not_running.len(), bar.more, bar.more_open), (14 + SUGGESTED, 8, false));
        assert_eq!(bar.not_running.last().map(|row| row.name.as_str()), Some("App 12"));
        lists.more_open = true;
        let bar = lists.bar(&[], &[]);
        assert_eq!((bar.not_running.len(), bar.more, bar.more_open), (34, 8, true));
        // Exactly the cap: no row to unfold. Those that run do not count.
        lists.set_dock(names[..13].iter().map(|name| app(name)).collect());
        lists.more_open = false;
        assert_eq!(lists.bar(&[], &[path("App 01")]).more, 0);
        assert_eq!(lists.bar(&[], &[]).more, 1);
    }

    #[test]
    fn all_apps_are_folded_until_asked_for_and_a_filter_narrows_them() {
        let installed: Vec<Installed> = ["Skylark", "notes", "Lumen Desk", "Sketchpad", "Desk"].map(app).into();
        let mut lists = Lists::default();
        let bar = lists.bar(&installed, &[]);
        assert_eq!((bar.all_count, bar.all_open, shown(&bar.all).len()), (5, false, 0));
        lists.all_open = true;
        // By name, whatever the case of its letters.
        assert_eq!(shown(&lists.bar(&installed, &[]).all), ["Desk", "Lumen Desk", "notes", "Sketchpad", "Skylark"]);
        for (typed, rows) in [
            ("sk", &["Sketchpad", "Skylark", "Desk", "Lumen Desk"][..]),
            ("DESK", &["Desk", "Lumen Desk"]),
            (" lumen ", &["Lumen Desk"]),
            ("zz", &[]),
        ] {
            lists.filter = Field::new(typed, MAX_FILTER);
            let bar = lists.bar(&installed, &[]);
            assert_eq!((shown(&bar.all), bar.all_count), (rows.iter().map(|s| s.to_string()).collect(), 5), "{typed}");
        }
    }

    #[test]
    fn pin_unpin_and_hide_say_what_the_settings_must_hold() {
        let saved = |lists: &Lists| {
            let texts =
                |entries: &[Entry]| entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>().join(",");
            (texts(&lists.pinned), texts(&lists.hidden))
        };
        let mut lists = Lists::default();
        // A line that names no app found is kept as written, where it was.
        let gone = Entry { text: "Gone App".into(), app: None };
        lists.set(vec![gone.clone(), entry("Zed")], vec![gone], true);
        let lumen = Installed { name: "Lumen".into(), path: "/Users/me/Applications/Lumen.app".into() };
        lists.set_dock(vec![app("Slack"), app("Notes"), lumen.clone()]);
        let installed = [app("Bear"), app("Slack"), app("Notes"), app("Zed")];
        // An app the scan finds by its name is written by its name; another by its path.
        assert_eq!(lists.pin(&path("Slack"), &installed), Some(Moved::AppPinned));
        assert_eq!(lists.pin(&lumen.path, &installed), Some(Moved::AppPinned));
        assert_eq!(lists.pin(&path("Bear"), &installed), Some(Moved::AppPinned));
        assert_eq!(
            saved(&lists),
            ("Gone App,Zed,Slack,/Users/me/Applications/Lumen.app,Bear".into(), "Gone App".into())
        );
        // An app nothing knows is not pinned.
        assert_eq!((lists.pin("/tmp/Evil.app", &installed), lists.can_undo()), (None, true));
        // The same again unpins; the undo takes one step back.
        assert_eq!(lists.pin(&path("Zed"), &installed), Some(Moved::AppUnpinned));
        assert_eq!(saved(&lists).0, "Gone App,Slack,/Users/me/Applications/Lumen.app,Bear");
        assert!(lists.undo().is_some() && lists.undo().is_none() && !lists.can_undo());
        assert_eq!(saved(&lists).0, "Gone App,Zed,Slack,/Users/me/Applications/Lumen.app,Bear");
        // Only a suggestion can be hidden: not a pinned app, not one outside the Dock, not twice.
        let hide = |lists: &mut Lists, path: &str| lists.hide(path, &installed);
        assert!(
            !hide(&mut lists, &path("Slack")) && !hide(&mut lists, &path("Bear")) && !hide(&mut lists, "/tmp/Evil.app")
        );
        assert!(hide(&mut lists, &path("Notes")) && !hide(&mut lists, &path("Notes")));
        assert_eq!(saved(&lists).1, "Gone App,Notes");
        assert!(shown(&lists.bar(&[], &[]).not_running).iter().all(|name| name != "Notes"));
        // Pinning a hidden app takes it off the hidden list; the undo puts both back.
        assert_eq!(lists.pin(&path("Notes"), &installed), Some(Moved::AppPinned));
        assert_eq!(saved(&lists).1, "Gone App");
        assert!(lists.undo().is_some());
        assert_eq!(saved(&lists).1, "Gone App,Notes");
        // With suggestions off there is none to hide.
        lists.set(Vec::new(), Vec::new(), false);
        assert!(!lists.hide(&path("Slack"), &installed));
    }

    #[test]
    fn a_pin_is_written_so_that_it_reads_back_as_the_same_bundle() {
        let at = |folder: &str, name: &str| Installed { name: name.into(), path: format!("{folder}/{name}.app") };
        // Two folders each hold a `Bar.app`. The scan lists them in the order a name is looked for.
        let (first, second) = (at("/Applications", "Bar"), at("/Applications/Utilities", "Bar"));
        let installed = [first.clone(), second.clone(), at("/Applications/Utilities", "Disk")];
        let mut lists = Lists::default();
        lists.set_dock(vec![second.clone(), first.clone()]);
        // The name means the first: it is written by name. The other only by its path.
        assert_eq!(
            (entry_text(&first, &installed), entry_text(&second, &installed)),
            ("Bar".into(), second.path.clone())
        );
        assert_eq!(entry_text(&installed[2], &installed), "Disk", "alone of its name, wherever it is");
        // One the scan has not found (not yet in, or outside its folders): its path.
        assert_eq!(entry_text(&first, &[]), first.path);
        // What is written reads back as the bundle that was pinned, and as hidden the same.
        lists.pin(&second.path, &installed);
        lists.hide(&first.path, &installed);
        let read = |entries: &[Entry]| {
            let mut again = Lists::default();
            again.set(
                entries.iter().map(|entry| Entry { text: entry.text.clone(), app: None }).collect(),
                Vec::new(),
                true,
            );
            again.resolve(&installed);
            again.pinned.iter().map(|entry| entry.app.as_ref().map(|app| app.path.clone())).collect::<Vec<_>>()
        };
        assert_eq!(
            (read(&lists.pinned), read(&lists.hidden)),
            (vec![Some(second.path.clone())], vec![Some(first.path.clone())])
        );
        // An entry that named no app found is looked for again when the scan comes in, and a
        // pin of the app it names does not write it twice.
        let mut lists = Lists::default();
        lists.set(
            vec![Entry { text: "disk".into(), app: None }, Entry { text: "Gone".into(), app: None }],
            Vec::new(),
            true,
        );
        assert!(lists.bar(&installed, &[]).not_running.is_empty());
        let unfound = lists.clone();
        lists.resolve(&installed);
        assert_eq!(shown(&lists.bar(&installed, &[]).not_running), ["Disk*"]);
        let mut lists = unfound;
        lists.set(vec![Entry { text: "Disk".into(), app: None }], Vec::new(), true);
        assert_eq!(lists.pin(&installed[2].path, &installed), Some(Moved::AppPinned));
        assert_eq!((lists.pinned.len(), lists.last_changes()), (1, vec![]));
    }

    fn model() -> Model {
        let mut m = Model::default();
        let (pinned, hidden) = (vec![entry("Zed")], Vec::new());
        m.update(Action::SetAppSettings { pinned, hidden, suggest: true, clients_write: false });
        m.update(Action::SetDock { apps: vec![app("Slack"), app("Notes")] });
        m.update(Action::SetInstalled { apps: ["Bear", "Arc", "Zed", "Slack"].map(app).into(), running: Vec::new() });
        m.update(Action::SetApps { names: vec!["Finder".into()] });
        m.update(Action::ToggleSection { section: Section::Apps });
        m
    }

    #[test]
    fn list_mode_walks_the_groups_and_pins_hides_and_takes_back() {
        let mut m = model();
        let closed = |name: &str| ListRow::Closed(path(name));
        let top = [ListRow::App("Finder".into()), closed("Zed"), closed("Slack"), closed("Notes"), ListRow::AllApps];
        assert_eq!(m.list_rows(), top);
        m.update(Action::FocusList);
        // ↵ on "All apps" unfolds it, and the selection stays on the heading.
        m.update(Action::ListMove { delta: -1 });
        assert_eq!(m.update(Action::ListChoose { axis: Axis::Right }), []);
        assert_eq!((m.list, &m.list_rows()[5..]), (Some(4), &[closed("Arc"), closed("Bear")][..]));
        // ↵ on an app that is not running starts it; ⇧↵ puts its pane below.
        m.update(Action::ListMove { delta: 2 });
        let bear = Effect::LaunchApp { name: "Bear".into(), path: path("Bear"), axis: Axis::Down, new_tab: false };
        assert_eq!(m.update(Action::ListChoose { axis: Axis::Down }), [bear]);
        // P pins it: it moves up, the selection stays at its place, and the settings are written.
        m.update(Action::Tick { now: 1_000 });
        // What the file must gain or lose: `+apps:Bear`, `-hidden:Notes`.
        let saved = |specs: &[&str]| {
            let change = |spec: &&str| {
                let (key, text) = spec[1..].split_once(':').unwrap();
                let key = if key == "apps" { "apps" } else { "apps_hidden" };
                ListChange { key, add: spec.starts_with('+'), text: text.into() }
            };
            vec![Effect::SaveApps { changes: specs.iter().map(change).collect() }]
        };
        assert_eq!(m.update(Action::ListPin), saved(&["+apps:Bear"]));
        assert_eq!(
            (shown(&m.apps_bar().not_running), m.list),
            (vec!["Zed*".into(), "Bear*".into(), "Slack".into(), "Notes".into()], Some(6))
        );
        assert_eq!(m.apps_bar().undo, Some("Pinned"));
        assert_eq!(m.undo_offer, Some(Notice { moved: Moved::AppPinned, until: 1_000 + NOTICE_MS }));
        // `u` takes it back, and the line goes.
        assert_eq!(m.update(Action::ListUndo), saved(&["-apps:Bear"]));
        assert_eq!((m.apps_bar().undo, m.undo_offer), (None, None));
        // `x` hides a suggestion and nothing else; P on a pinned app unpins it.
        m.list = Some(1);
        assert_eq!(m.update(Action::ListDismiss), []);
        assert_eq!(m.update(Action::ListPin), saved(&["-apps:Zed"]));
        assert_eq!(m.apps_bar().undo, Some("Unpinned"));
        m.list = Some(2);
        assert_eq!(m.list_rows()[2], closed("Notes"));
        assert_eq!(m.update(Action::ListDismiss), saved(&["+hidden:/Applications/Notes.app"]));
        assert_eq!(
            (m.apps_bar().undo, shown(&m.apps_bar().not_running)),
            (Some("Suggestion hidden"), vec!["Slack".into()])
        );
        // The line runs out; `u` still takes the last change back. Nothing is left to take back then.
        m.update(Action::Tick { now: 1_000 + NOTICE_MS });
        assert_eq!(m.apps_bar().undo, None);
        assert_eq!(m.update(Action::AppsUndo), saved(&["-hidden:/Applications/Notes.app"]));
        assert_eq!(m.update(Action::AppsUndo), []);
        // By path, as a click does. An app nothing lists is neither pinned nor hidden nor started.
        assert_eq!(m.update(Action::AppPin { path: path("Arc") }), saved(&["+apps:Arc"]));
        assert_eq!(m.update(Action::AppHide { path: path("Slack") }), saved(&["+hidden:Slack"]));
        // A client's key or label does none of it: the settings file is the user's. A test
        // run says its clients may.
        for action in [Action::AppPin { path: path("Bear") }, Action::AppHide { path: path("Notes") }, Action::AppsUndo]
        {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(USERS_ALONE));
            assert_eq!(
                (m.update(action.clone().sent_by(Origin::Socket)), m.refusal(&action, Origin::User)),
                (vec![], None)
            );
        }
        for action in [Action::ListPin, Action::ListDismiss, Action::ListUndo] {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(USERS_ALONE));
        }
        m.app_lists.clients_write = true;
        assert_eq!(m.update(Action::AppsUndo.sent_by(Origin::Socket)), saved(&["-hidden:Slack"]));
        // In another section a client's `x` and `u` are what they were.
        m.app_lists.clients_write = false;
        m.update(Action::ToggleSection { section: Section::Tasks });
        assert_eq!(m.refusal(&Action::ListDismiss, Origin::Socket), None);
        m.update(Action::ToggleSection { section: Section::Apps });
        assert_eq!(m.update(Action::AppPin { path: "/tmp/Evil.app".into() }), []);
        assert_eq!(m.update(Action::AppHide { path: "/tmp/Evil.app".into() }), []);
        let mut effects = Vec::new();
        m.app_launch("/tmp/Evil.app", Axis::Right, &mut effects);
        assert_eq!(effects, []);
    }

    #[test]
    fn a_task_or_mark_offer_is_not_the_apps_and_the_other_way_round() {
        let mut m = model();
        m.update(Action::AppHide { path: path("Slack") });
        assert_eq!(m.undo_offer.map(|offer| offer.moved), Some(Moved::AppHidden));
        // A task move by key does not take the apps' offer away, and its undo is the task's.
        m.update(Action::TaskAdd {
            title: "t".into(),
            priority: 3,
            state: crate::tasks::State::NeedsYou,
            tab: None,
            assign: Default::default(),
        });
        m.update(Action::TaskDismiss { id: 1 });
        assert_eq!(m.apps_bar().undo, Some("Suggestion hidden"));
        m.update(Action::TaskUndo);
        assert_eq!((m.tasks.needs_you(), m.apps_bar().undo), (1, Some("Suggestion hidden")));
        // In another section `u` is that section's.
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        assert_eq!(m.update(Action::ListUndo), []);
        assert!(m.app_lists.can_undo());
    }

    #[test]
    fn the_filter_takes_the_keyboard_in_list_mode_and_gives_it_back() {
        let mut m = model();
        // Only the Apps section has the filter.
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::AppsFilter { on: true });
        assert_eq!((m.sidebar, m.field(), m.list), (Some(Section::Tasks), None, None));
        // There it unfolds the group, turns list mode on and selects the group's heading.
        m.update(Action::ToggleSection { section: Section::Apps });
        m.update(Action::AppsFilter { on: true });
        assert_eq!((m.sidebar, m.mode(), m.list), (Some(Section::Apps), crate::Mode::List, Some(4)));
        assert_eq!(m.field().map(|(of, field)| (of, field.text().to_string())), Some(("apps", String::new())));
        m.update(Action::FieldEdit { edit: Edit::Insert("b".into()) });
        let bar = m.apps_bar();
        assert_eq!(
            (shown(&bar.all), bar.all_count, bar.filter.as_str(), bar.filtering),
            (vec!["Bear".into()], 2, "b", true)
        );
        // ↵ gives the keys back to the rows, on the first app left. The filter stays.
        m.update(Action::AppsFilter { on: false });
        assert_eq!((m.field(), m.list, m.list_rows().get(5)), (None, Some(5), Some(&ListRow::Closed(path("Bear")))));
        assert_eq!(m.apps_bar().filter, "b");
        // Typing with no field does nothing; nor does giving back what is not held.
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
        m.update(Action::AppsFilter { on: false });
        assert_eq!((m.apps_bar().filter.as_str(), m.list), ("b", Some(5)));
        // A filter that leaves nothing: the selection waits on the heading.
        m.update(Action::AppsFilter { on: true });
        m.update(Action::FieldEdit { edit: Edit::Insert("zz".into()) });
        m.update(Action::AppsFilter { on: false });
        assert_eq!((m.apps_bar().all.len(), m.list), (0, Some(4)));
        // Leaving list mode, or the section, takes the keyboard from the filter.
        m.update(Action::AppsFilter { on: true });
        m.update(Action::LeaveList);
        assert_eq!((m.app_lists.filtering, m.mode()), (false, crate::Mode::Normal));
        m.update(Action::AppsFilter { on: true });
        m.update(Action::ListSection { delta: 1 });
        assert_eq!((m.app_lists.filtering, m.field()), (false, None));
        // Folding the group clears the filter.
        m.update(Action::ToggleSection { section: Section::Apps });
        m.update(Action::AppsToggle { group: Group::All });
        let bar = m.apps_bar();
        assert_eq!((bar.all_open, bar.filter.as_str(), bar.all.len(), bar.all_count), (false, "", 0, 2));
        // The picker's filter comes first while it is open: the list has lost the keyboard.
        m.update(Action::AppsFilter { on: true });
        m.update(Action::OpenPicker { axis: Axis::Right, new_tab: false, items: Vec::new() });
        assert_eq!(m.field().map(|(of, _)| of), Some("picker"));
    }

    #[test]
    fn the_filter_holds_the_keyboard_against_a_client() {
        let mut m = model();
        assert!(Action::AppsFilter { on: true }.takes_keyboard() && !Action::AppsFilter { on: false }.takes_keyboard());
        m.update(Action::AppsFilter { on: true });
        // A client adds a task "and starts it" while the user types in the filter: the task is
        // added, and the tabs on show stay.
        let line = Action::TaskFromLine { line: "theirs".into(), known: Default::default(), open: true };
        m.update(line.clone().sent_by(Origin::Socket));
        assert_eq!((m.tasks.list.len(), m.current_task, m.field().map(|(of, _)| of)), (1, None, Some("apps")));
        // With the keyboard given back, the same starts the task.
        m.update(Action::AppsFilter { on: false });
        m.update(line.sent_by(Origin::Socket));
        assert_eq!((m.tasks.list.len(), m.current_task), (2, Some(2)));
    }

    #[test]
    fn and_n_more_is_a_row_that_unfolds() {
        let mut m = Model::default();
        let dock: Vec<Installed> = (1..=15).map(|n| app(&format!("App {n:02}"))).collect();
        m.update(Action::SetDock { apps: dock });
        m.update(Action::ToggleSection { section: Section::Apps });
        assert_eq!((m.list_rows().len(), m.list_rows().last()), (13, Some(&ListRow::MoreApps)));
        m.update(Action::FocusList);
        m.update(Action::ListMove { delta: -1 });
        m.update(Action::ListChoose { axis: Axis::Right });
        // The selection follows the row to the end of the longer list.
        assert_eq!((m.list_rows().len(), m.list, m.apps_bar().more_open), (16, Some(15), true));
        m.update(Action::AppsToggle { group: Group::More });
        assert_eq!((m.list_rows().len(), m.list), (13, Some(12)));
    }
}
