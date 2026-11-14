//! The layout file, `layout.json`: the tabs in bar order (each with its task, if any), their
//! splits and panes, the tab on show, the section and the drawer, kept between runs. Session restore's layer 1 (`docs/ai/session-restore.md`). A pane is kept as a workbook
//! keeps one (`marks::PaneRecord`, `workbook::History`), by the same rules. The shell reads and
//! writes the file; read, it is untrusted input and is checked here.

use serde::{Deserialize, Serialize};

use crate::marks::{self, Extra, RecordKind, TabRecord, Target};
use crate::model::{Effect, Kind, Model, PaneId, Section};
use crate::talk::{self, Subject, Talk};
use crate::tasks::TaskId;
use crate::workbook::{self, History};

/// The `version` of `layout.json` this build reads and writes.
pub const VERSION: u32 = 1;
/// The largest file read, as a workbook's.
pub const MAX_FILE_BYTES: usize = workbook::MAX_FILE_BYTES;
/// The most tabs kept.
pub const MAX_TABS: usize = 8 * workbook::MAX_TABS;
/// The most marks one tab names.
const MAX_LINKS: usize = 64;

/// Why a client's request does nothing: Start Fresh, or an app a restored pane names.
pub const USERS_ALONE: &str = "only the user starts fresh or opens a restored pane's app";

/// `layout.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    /// The tab on show, by its place in `tabs`.
    #[serde(default)]
    pub selected: Option<usize>,
    /// The left section that was open.
    #[serde(default)]
    pub section: Option<Section>,
    #[serde(default)]
    pub drawer: Drawer,
    /// The tabs, in bar order. A task with no tab has none here: that is normal.
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The right drawer. Its conversation is never started again here: it is shown, to go on with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drawer {
    /// The task whose conversation it was on. A session's that is no task's is not kept.
    #[serde(default)]
    pub task: Option<TaskId>,
    #[serde(default)]
    pub open: bool,
    #[serde(default = "width")]
    pub width: f32,
    #[serde(default)]
    pub wide: bool,
    /// The view each task's conversation was switched to (Chat or Terminal), where the user picked one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<(TaskId, crate::transcript::View)>,
}

fn width() -> f32 {
    talk::WIDTH
}

impl Default for Drawer {
    fn default() -> Self {
        Drawer { task: None, open: false, width: talk::WIDTH, wide: false, views: Vec::new() }
    }
}

/// A tab: as a workbook keeps one (with its `task`), and what pointed at it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    #[serde(flatten)]
    pub tab: TabRecord,
    /// The workbook's tab it stands for: the workbook's id and the place of the tab in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workbook: Option<BookLink>,
    /// The marks that stood for it, or for one of its panes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<MarkLink>,
    /// The terminal pane, by its id in the record, that Open Settings opened: restored, it runs
    /// the editor on `settings.json` again. A mark, not a command: the shell builds the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookLink {
    pub id: String,
    pub tab: usize,
}

/// A mark by its id for good (`Mark::uid`); `pane`: a pane mark's pane, by its id in the tab's record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkLink {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<u32>,
}

/// Why a file is not read.
#[derive(Debug, PartialEq)]
pub enum Unread {
    /// A newer build wrote it: left as it is, and not written this run.
    Newer(u32),
    /// It does not read, or does not pass the checks: set aside.
    Bad(String),
}

#[derive(Deserialize)]
struct Versioned {
    version: u32,
}

/// Reads `layout.json`. Histories are cleaned and capped as a workbook's are.
pub fn parse(text: &str) -> Result<Layout, Unread> {
    let versioned: Versioned = serde_json::from_str(text).map_err(|err| Unread::Bad(err.to_string()))?;
    match versioned.version {
        VERSION => {}
        newer if newer > VERSION => return Err(Unread::Newer(newer)),
        old => return Err(Unread::Bad(format!("version {old} is not one this build reads"))),
    }
    let mut value: serde_json::Value = serde_json::from_str(text).map_err(|err| Unread::Bad(err.to_string()))?;
    empty_unknown_kinds(&mut value);
    let mut layout: Layout = serde_json::from_value(value).map_err(|err| Unread::Bad(err.to_string()))?;
    let mut records: Vec<TabRecord> = layout.tabs.iter().map(|saved| saved.tab.clone()).collect();
    workbook::fit_history(&mut records);
    for (saved, record) in layout.tabs.iter_mut().zip(records) {
        saved.tab = record;
    }
    match layout.unsound() {
        Some(wrong) => Err(Unread::Bad(wrong.into())),
        None => Ok(layout),
    }
}

/// A pane of a kind this build does not know (a newer build's) is an empty pane, not an unread
/// file. Here only: `marks.json` and workbooks leave such a file alone, so nothing of it is lost.
fn empty_unknown_kinds(value: &mut serde_json::Value) {
    let tabs = value.get_mut("tabs").and_then(serde_json::Value::as_array_mut);
    let panes = tabs.into_iter().flatten().filter_map(|tab| tab.get_mut("panes")?.as_array_mut()).flatten();
    for pane in panes {
        let Some(kind) = pane.get_mut("kind") else { continue };
        let known = kind.get("type").and_then(serde_json::Value::as_str);
        if !matches!(known, Some("empty" | "terminal" | "app")) {
            *kind = serde_json::json!({ "type": "empty" });
        }
    }
}

/// The text of `layout.json`.
pub fn write(layout: &Layout) -> String {
    crate::pretty_json(layout)
}

/// A mark's id for good as the file may name it: short and one line.
fn plain_uid(uid: &str) -> bool {
    !uid.is_empty() && marks::plain(uid, 64)
}

impl Layout {
    /// What is wrong with it as read, if anything: the caps and checks of a workbook's tabs.
    pub fn unsound(&self) -> Option<&'static str> {
        if self.version != VERSION {
            return Some("its version is not this build's");
        }
        if self.tabs.len() > MAX_TABS {
            return Some("it has too many tabs");
        }
        self.tabs.iter().find_map(|saved| {
            let links = saved.marks.len() > MAX_LINKS || !saved.marks.iter().all(|link| plain_uid(&link.id));
            let book = saved.workbook.as_ref().is_some_and(|book| !workbook::valid_uid(&book.id));
            match (links, book) {
                (true, _) => Some("a tab names too many marks, or one that is not an id"),
                (_, true) => Some("a tab names a workbook by what is not an id"),
                _ => saved.tab.unsound(),
            }
        })
    }

    /// The directories its terminals start in, each once: the shell looks for them.
    pub fn dirs(&self) -> Vec<String> {
        let mut dirs: Vec<String> = Vec::new();
        let records = self.tabs.iter().flat_map(|saved| saved.tab.shown());
        for dir in records.filter(|pane| pane.kind == RecordKind::Terminal).filter_map(|pane| pane.cwd.clone()) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs
    }
}

/// The tabs in the order the bar keeps them (`model::bar_order`); `selected` follows its tab.
pub fn grouped(layout: &mut Layout) {
    let order = crate::model::bar_order(&layout.tabs.iter().map(|saved| saved.tab.task).collect::<Vec<_>>());
    layout.selected = layout.selected.and_then(|was| order.iter().position(|at| *at == was));
    let mut tabs: Vec<Option<SavedTab>> = std::mem::take(&mut layout.tabs).into_iter().map(Some).collect();
    layout.tabs = order.into_iter().filter_map(|at| tabs[at].take()).collect();
}

impl Model {
    /// The layout file of the model as it is. `cwds` and `histories`: each terminal's directory
    /// and text as the shell read them; `lines`: how many lines a terminal keeps (the
    /// `workbook_scrollback_lines` setting). 0 keeps no terminal text at all, the command included.
    pub fn layout_file(&self, cwds: &[(PaneId, String)], histories: &[(PaneId, History)], lines: usize) -> Layout {
        let histories: Vec<(PaneId, History)> = match lines {
            0 => Vec::new(),
            most => histories.iter().map(|(pane, history)| (*pane, history.kept(most))).collect(),
        };
        let known = |task: Option<TaskId>| task.filter(|id| self.tasks.get(*id).is_some());
        // A mark is named by its id for good, if no other mark has it.
        let unique = |uid: &str| plain_uid(uid) && self.marks.list.iter().filter(|mark| mark.uid == uid).count() == 1;
        // An editor's screen is not history: the Settings pane keeps none.
        let histories: Vec<(PaneId, History)> =
            histories.into_iter().filter(|(pane, _)| Some(*pane) != self.settings_pane).collect();
        let (mut records, mut links, mut selected) = (Vec::new(), Vec::new(), None);
        for (index, tab) in self.tabs.iter().enumerate().take(MAX_TABS) {
            let Some(mut record) = self.tab_record(index, cwds, &histories) else { continue };
            record.task = known(tab.task);
            // A pane the file would not read back is kept as what it can be: no directory, or empty.
            for pane in record.panes.iter_mut().filter(|pane| pane.unsound().is_some()) {
                pane.cwd = None;
                if pane.unsound().is_some() {
                    pane.kind = RecordKind::Empty;
                }
            }
            if record.unsound().is_some() {
                continue;
            }
            // Panes by their place in reading order, from 1: the file says nothing of this run's ids.
            let ids: Vec<(u32, PaneId)> = record.panes.iter().zip(1..).map(|(pane, new)| (pane.id, new)).collect();
            let new_id = |old: u32| ids.iter().find(|(from, _)| *from == old).map(|(_, to)| *to);
            record.layout = record.layout.as_ref().and_then(|tree| marks::renumber(tree, &ids));
            record.focused = record.focused.and_then(new_id);
            record.panes.iter_mut().for_each(|pane| pane.id = new_id(pane.id).unwrap_or(0));
            if index == self.active {
                selected = Some(records.len());
            }
            let book = self
                .workbooks
                .entry_of(tab.id)
                .and_then(|(id, at)| Some(BookLink { id: self.workbooks.get(id)?.uid.clone(), tab: at }));
            let mut marked: Vec<MarkLink> = Vec::new();
            for mark in self.marks.list.iter().filter(|mark| unique(&mark.uid)) {
                let pane = match (&mark.target, mark.live) {
                    (Target::Tab(_), Some(live)) if live == tab.id => None,
                    (Target::Pane(_), Some(live)) => match new_id(live) {
                        Some(pane) => Some(pane),
                        None => continue,
                    },
                    _ => continue,
                };
                marked.push(MarkLink { id: mark.uid.clone(), pane });
            }
            marked.truncate(MAX_LINKS);
            let settings = self.settings_pane.and_then(new_id);
            records.push(record);
            links.push((book, marked, settings));
        }
        workbook::fit_history(&mut records);
        let tabs = records.into_iter().zip(links);
        let tabs = tabs.map(|(tab, (workbook, marks, settings))| SavedTab { tab, workbook, marks, settings }).collect();
        let drawer = Drawer {
            task: self.talk.as_ref().and_then(|talk| known(talk.task.task())),
            open: self.talk.as_ref().is_some_and(|talk| talk.open && known(talk.task.task()).is_some()),
            width: self.talk_width,
            wide: self.talk_wide,
            // Of live tasks only, as a restore keeps them: a dismissed task's pick goes with it.
            views: self.saved_views().into_iter().filter(|(task, _)| self.tasks.live(*task).is_some()).collect(),
        };
        // Not the Calendar: a build without it could not read the file back.
        let section = self.sidebar.filter(|s| *s != Section::Calendar);
        let mut layout = Layout { version: VERSION, selected, section, drawer, tabs, extra: Extra::new() };
        // What is written always reads back: the last histories go first, then the last tabs.
        while write(&layout).len() > MAX_FILE_BYTES {
            let held = layout.tabs.iter_mut().flat_map(|saved| saved.tab.panes.iter_mut()).rev();
            match held.into_iter().find(|pane| pane.extra.contains_key("history")) {
                Some(pane) => drop(pane.extra.remove("history")),
                None => drop(layout.tabs.pop()),
            }
        }
        layout.selected = layout.selected.filter(|at| *at < layout.tabs.len());
        layout
    }

    /// True while the model is as it starts: one empty tab of no task. Only that is restored into.
    fn fresh(&self) -> bool {
        let empty = |tab: &crate::model::Tab| tab.task.is_none() && tab.layout.is_none();
        self.panes.is_empty() && self.layout.is_none() && self.tabs.iter().all(empty)
    }

    /// Builds the tabs of `layout` in bar order, each as a workbook's (`rebuild_tab`), and puts
    /// back the tab on show, the section, the drawer and what
    /// marks and workbooks stood for. A tab whose task is gone is a tab of no task. Terminals in
    /// a directory of `gone` start in `home`; each shows its history, its command typed and not
    /// run. App panes ask for their windows; `launch`: an app that is not running may be started.
    /// No conversation starts. Only into a fresh model, and only a sound file.
    pub(crate) fn layout_restore(
        &mut self,
        layout: Layout,
        gone: &[String],
        home: Option<String>,
        launch: bool,
        effects: &mut Vec<Effect>,
    ) {
        if !self.fresh() || layout.unsound().is_some() {
            return;
        }
        let exists = |model: &Model, task: Option<TaskId>| task.filter(|id| model.tasks.live(*id).is_some());
        let mut layout = layout;
        // A tab whose task is gone or dismissed is a tab of no task; then each task's tabs go together.
        for saved in &mut layout.tabs {
            saved.tab.task = exists(self, saved.tab.task);
        }
        grouped(&mut layout);
        if !layout.tabs.is_empty() {
            // The empty tabs go: nothing may stand for them (a workbook saved from one), as on close.
            for tab in std::mem::take(&mut self.tabs) {
                self.marks.forget_tab(tab.id);
                self.workbooks.forget_tab(tab.id);
            }
            (self.last_tab, self.active) = (Vec::new(), 0);
        }
        let mut built: Vec<(u32, Vec<PaneId>)> = Vec::new();
        for saved in &layout.tabs {
            let mut record = saved.tab.clone();
            for pane in record.panes.iter_mut().filter(|pane| pane.kind == RecordKind::Terminal) {
                if pane.cwd.as_ref().is_some_and(|dir| gone.contains(dir)) {
                    pane.cwd = home.clone();
                }
            }
            let (base, at) = (self.next_id, effects.len());
            let tab = self.rebuild_tab(&record, effects);
            for effect in &mut effects[at..] {
                if let Effect::HostWindow { launch: may, .. } = effect {
                    *may = launch;
                }
            }
            // As a workbook's: each terminal that started shows its history, typed command and all.
            let panes: Vec<PaneId> = (base + 1..).take(record.shown().len()).collect();
            for (pane, kept) in panes.iter().zip(record.shown()) {
                let started = self.pane(*pane).is_some_and(|open| open.kind == Kind::Terminal);
                // The Settings tab's terminal: the editor again, once, with no history. Its title
                // is the one kept ("Settings").
                if started && saved.settings == Some(kept.id) && self.settings_pane.is_none() {
                    self.settings_pane = Some(*pane);
                    effects.push(Effect::EditSettings { pane: *pane });
                    continue;
                }
                if let Some(history) = workbook::history_of(kept).filter(|_| started) {
                    effects.push(Effect::History { pane: *pane, history });
                }
            }
            built.push((tab, panes));
        }
        self.restore_selected(layout.selected, &built, effects);
        // Building each tab noted it as its task's last: none is, so each task shows its first.
        self.last_tab.clear();
        // A Calendar left open by a run that had a feed: none without one.
        self.sidebar = layout.section.filter(|s| *s != Section::Calendar || self.calendar.on);
        self.talk_width = talk::width(layout.drawer.width);
        self.talk_wide = layout.drawer.wide;
        let views = layout.drawer.views.iter().filter_map(|(task, view)| Some((exists(self, Some(*task))?, *view)));
        let views: Vec<_> = views.collect();
        self.restore_views(views);
        // Shown to go on with, as a click on its task offers: never started here.
        if let Some(task) = exists(self, layout.drawer.task) {
            let subject = Subject::Task(task);
            self.talk = Some(Talk { task: subject.clone(), open: layout.drawer.open, keys: false });
            // Once: the user may have opened it already this run.
            self.opened.retain(|opened| *opened != subject);
            self.opened.push(subject);
        }
        self.restore_links(&layout, &built);
    }

    /// The tab that was on show, else the first.
    fn restore_selected(&mut self, selected: Option<usize>, built: &[(u32, Vec<PaneId>)], effects: &mut Vec<Effect>) {
        let id = selected.and_then(|at| built.get(at)).or(built.first()).map(|(tab, _)| *tab);
        if let Some(at) = id.and_then(|id| self.tabs.iter().position(|tab| tab.id == id)) {
            self.select_tab(at, effects);
        }
    }

    /// Marks and workbooks stand again for the tabs and panes they stood for.
    fn restore_links(&mut self, layout: &Layout, built: &[(u32, Vec<PaneId>)]) {
        for (saved, (tab, panes)) in layout.tabs.iter().zip(built) {
            let pane_of = |id: u32| saved.tab.shown().iter().position(|pane| pane.id == id).map(|at| panes[at]);
            for link in &saved.marks {
                let mut named = self.marks.list.iter().filter(|mark| mark.uid == link.id);
                let (Some(mark), None) = (named.next(), named.next()) else { continue };
                let live = match (&mark.target, link.pane) {
                    (Target::Tab(_), None) => Some(*tab),
                    (Target::Pane(_), Some(pane)) => pane_of(pane),
                    _ => None,
                };
                if let Some(live) = live {
                    let id = mark.id;
                    self.marks.bind(id, live);
                }
            }
            let Some(link) = &saved.workbook else { continue };
            if let Some(book) = self.workbooks.list.iter_mut().find(|book| book.uid == link.id)
                && link.tab < book.tabs.len()
                && book.live(link.tab).is_none()
            {
                book.set_live(link.tab, Some(*tab));
            }
        }
    }

    /// "Open <App>" on a restored app pane that waits: its app's window is hosted there, the
    /// app started if it is not running.
    pub(crate) fn unfilled_open(&mut self, pane: PaneId, effects: &mut Vec<Effect>) {
        let Some(empty) = self.marks.unfilled().iter().find(|empty| empty.pane == pane) else { return };
        if let RecordKind::App { bundle, name, .. } = &empty.record.kind {
            let (bundle, name) = (bundle.clone(), name.clone());
            let axis = crate::Axis::Right;
            effects.push(Effect::HostWindow {
                mark: None,
                bundle,
                name,
                title: String::new(),
                into: Some(pane),
                axis,
                launch: true,
            });
        }
    }

    /// True if the layout the file would hold (but for what the shell reads) changed since the
    /// last time this was asked: the shell writes the file a moment later. False after Start Fresh.
    pub fn layout_moved(&mut self) -> bool {
        if !self.layout_kept {
            return false;
        }
        let now = self.layout_file(&[], &[], 0);
        let moved = self.layout_seen.as_ref() != Some(&now);
        self.layout_seen = Some(now);
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis::{Down, Right};
    use crate::layout::{Dir, Node};
    use crate::marks::MarkOf;
    use crate::model::{Action, Origin, Tab};
    use crate::tasks::{Assign, State};

    fn task(m: &mut Model, title: &str) {
        let assign = Assign::default();
        m.update(Action::TaskAdd { title: title.into(), priority: 2, state: State::Running, tab: None, assign });
    }

    fn split(m: &mut Model, axis: crate::Axis, kind: Kind) {
        m.update(Action::Split { axis, kind });
    }

    fn restore(layout: Layout) -> Action {
        Action::RestoreLayout { layout: Box::new(layout), gone: Vec::new(), home: None, launch: false }
    }

    /// Two tabs of no task with splits, a task's tab, the drawer on the task, the Tasks section.
    fn workspace() -> Model {
        let mut m = Model::default();
        task(&mut m, "fix zoom");
        split(&mut m, Right, Kind::Terminal);
        split(&mut m, Down, Kind::Terminal);
        m.update(Action::Resize { dir: Dir::Up });
        m.update(Action::NewTab);
        split(&mut m, Right, Kind::Terminal);
        let title = "alice: the plan".to_string();
        let slack = Kind::App { window: 9, title, bundle: Some("com.slack".into()), name: "Slack".into() };
        m.update(Action::Open { kind: slack, axis: Right, beside: None });
        m.update(Action::NewTabOf { task: Some(1) });
        split(&mut m, Right, Kind::Terminal);
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.talk = Some(Talk { task: Subject::Task(1), open: true, keys: true });
        m
    }

    /// A fresh model with `m`'s tasks, and the file of `m` restored into it.
    fn restored(m: &Model, file: Layout, gone: &[&str], launch: bool) -> (Model, Vec<Effect>) {
        let mut fresh = Model::default();
        fresh.update(Action::RestoreTasks { tasks: m.tasks.list.clone(), floor: 9 }.sent_by(Origin::System));
        let gone = gone.iter().map(|dir| dir.to_string()).collect();
        let restore = Action::RestoreLayout { layout: Box::new(file), gone, home: Some("/Users/me".into()), launch };
        let effects = fresh.update(restore.sent_by(Origin::System));
        (fresh, effects)
    }

    fn titles(m: &Model) -> Vec<(String, Option<TaskId>)> {
        m.tabs.iter().map(|tab: &Tab| (tab.title.clone(), tab.task)).collect()
    }

    #[test]
    fn each_tasks_tabs_go_together_and_no_tasks_last_and_the_selected_one_follows() {
        let mut file = Model::default().layout_file(&[], &[], 0);
        let tab = |title: &str, task| SavedTab {
            tab: TabRecord {
                title: title.into(),
                task,
                focused: None,
                layout: None,
                panes: Vec::new(),
                extra: Extra::new(),
            },
            workbook: None,
            marks: Vec::new(),
            settings: None,
        };
        file.tabs = vec![tab("a", None), tab("b", Some(2)), tab("c", Some(1)), tab("d", Some(2)), tab("e", None)];
        file.selected = Some(3);
        grouped(&mut file);
        let order: Vec<&str> = file.tabs.iter().map(|saved| saved.tab.title.as_str()).collect();
        assert_eq!((order, file.selected), (vec!["b", "d", "c", "a", "e"], Some(1)));
    }

    #[test]
    fn a_workspace_comes_back_as_it_was_and_no_conversation_starts() {
        let m = workspace();
        let cwds = vec![(1, "/code/app".to_string()), (2, "/code/web".to_string())];
        let file = parse(&write(&m.layout_file(&cwds, &[], 500))).unwrap();
        assert_eq!((file.tabs.len(), file.selected, file.section), (3, Some(1), Some(Section::Tasks)));
        assert_eq!(file.drawer.task, Some(1));
        let (r, effects) = restored(&m, file, &[], false);
        // The task's tab first: each task's tabs go together, no task's last.
        let bar = |m: &Model| m.tabs.iter().map(|tab: &Tab| (tab.title.clone(), tab.task)).collect::<Vec<_>>();
        let tab = |title: &str, task| (title.to_string(), task);
        assert_eq!(bar(&r), [tab("fix zoom", Some(1)), tab("Tab 1", None), tab("Tab 2", None)]);
        assert_eq!((r.tab_task(), r.active, r.sidebar), (None, 1, Some(Section::Tasks)));
        let mut was = m.layout_file(&cwds, &[], 0);
        grouped(&mut was);
        assert_eq!(write(&r.layout_file(&[], &[], 0)), write(&was));
        // The split's ratio, and each terminal's directory.
        assert!(matches!(r.layout, Some(Node::Split { ratio, .. }) if (ratio - 0.5).abs() > 0.01), "{:?}", r.layout);
        let dirs: Vec<Option<&str>> = r.panes.iter().skip(1).take(2).map(|pane| pane.cwd.as_deref()).collect();
        assert_eq!(dirs, [Some("/code/app"), Some("/code/web")]);
        // The drawer shows the task's conversation, without the keyboard and with no agent.
        let talk = r.talk.as_ref().unwrap();
        assert_eq!((&talk.task, talk.open, talk.keys), (&Subject::Task(1), true, false));
        assert!(r.agents.is_empty());
        // The view the user picked for its conversation comes back with it.
        let mut picked = workspace();
        picked.restore_views(vec![(1, crate::transcript::View::Terminal), (99, crate::transcript::View::Native)]);
        let file = parse(&write(&picked.layout_file(&[], &[], 0))).unwrap();
        assert_eq!(file.drawer.views, [(1, crate::transcript::View::Terminal)], "a task that is gone is not kept");
        let (back, _) = restored(&picked, file, &[], false);
        assert_eq!(back.talk_view(&Subject::Task(1)), crate::transcript::View::Terminal);
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::StartAgent { .. } | Effect::ReadSession { .. })));
        // The app pane waits for its window: no app is started for it.
        let hosts: Vec<&Effect> = effects.iter().filter(|e| matches!(e, Effect::HostWindow { .. })).collect();
        assert!(matches!(hosts[..], [Effect::HostWindow { launch: false, into: Some(_), .. }]), "{hosts:?}");
    }

    #[test]
    fn the_file_keeps_no_title_and_no_terminal_text_when_none_is_to_be_kept() {
        let m = workspace();
        let history = History { lines: vec!["TOKEN-ish".into()], command: Some("npm run dev".into()), saved: 1 };
        let none = write(&m.layout_file(&[], &[(1, history.clone())], 0));
        assert!(!none.contains("TOKEN-ish") && !none.contains("npm run dev") && !none.contains("history"), "{none}");
        let kept = write(&m.layout_file(&[], &[(1, history)], 500));
        assert!(kept.contains("TOKEN-ish") && kept.contains("npm run dev"));
        assert!(!kept.contains("alice") && kept.contains("com.slack"));
    }

    #[test]
    fn a_terminal_shows_its_history_and_one_in_a_directory_gone_starts_at_home() {
        let m = workspace();
        let history = History { lines: vec!["built".into()], command: Some("npm run dev".into()), saved: 1 };
        let file = m.layout_file(&[(1, "/gone".into())], &[(1, history)], 500);
        let (r, effects) = restored(&m, file, &["/gone"], false);
        // The task's tab comes first: the first terminal of no task's is pane 2.
        assert_eq!(r.pane(2).and_then(|pane| pane.cwd.as_deref()), Some("/Users/me"));
        let at = effects.iter().position(|effect| *effect == Effect::Opened(2)).unwrap();
        let shown = effects[at..].iter().find_map(|effect| match effect {
            Effect::History { pane: 2, history } => Some(history.command.clone()),
            _ => None,
        });
        assert_eq!(shown, Some(Some("npm run dev".into())));
    }

    #[test]
    fn a_tampered_file_runs_nothing_and_one_that_is_unsound_or_newer_is_not_read() {
        let m = workspace();
        let mut file = m.layout_file(&[], &[], 0);
        let tampered = serde_json::json!({"lines": ["\u{1b}]0;x\u{7}ok"], "command": "rm -rf ~\nreboot", "saved": 1});
        file.tabs[0].tab.panes[0].extra.insert("history".into(), tampered);
        file.tabs[0].tab.panes[0].extra.insert("command".into(), "reboot".into());
        let read = parse(&write(&file)).unwrap();
        let (_, effects) = restored(&m, read, &[], false);
        let history = effects.iter().find_map(|effect| match effect {
            Effect::History { history, .. } => Some(history.clone()),
            _ => None,
        });
        assert_eq!(history.map(|h| (h.lines, h.command)), Some((vec!["ok".to_string()], None)));
        // A directory that is not absolute, a newer build's file, not JSON, an old version.
        let mut relative = m.layout_file(&[], &[], 0);
        relative.tabs[0].tab.panes[0].cwd = Some("../etc".into());
        assert!(matches!(parse(&write(&relative)), Err(Unread::Bad(_))));
        assert_eq!(parse(r#"{"version": 2, "tabs": "anything"}"#), Err(Unread::Newer(2)));
        assert!(matches!(parse("{"), Err(Unread::Bad(_))));
        assert!(matches!(parse(r#"{"version": 0}"#), Err(Unread::Bad(_))));
    }

    #[test]
    fn a_tab_of_a_task_that_is_gone_is_a_tab_of_no_task_and_restore_goes_only_into_a_fresh_model() {
        let m = workspace();
        let file = m.layout_file(&[], &[], 0);
        let mut fresh = Model::default();
        fresh.update(restore(file.clone()).sent_by(Origin::System));
        assert_eq!(fresh.tabs.iter().filter(|tab| tab.task.is_some()).count(), 0);
        assert_eq!((fresh.tabs.len(), fresh.talk.is_none()), (3, true));
        // Not over a workspace in use, nor from a client.
        let mut used = workspace();
        let before = titles(&used);
        used.update(restore(file.clone()).sent_by(Origin::System));
        assert_eq!(titles(&used), before);
        let mut fresh = Model::default();
        fresh.update(restore(file).sent_by(Origin::Socket));
        assert_eq!(fresh.tabs.len(), 1);
    }

    #[test]
    fn a_task_with_no_tab_gets_none_and_the_tab_on_show_comes_back() {
        let mut m = workspace();
        task(&mut m, "later");
        m.update(Action::SelectTab { index: 2 });
        let file = m.layout_file(&[], &[], 0);
        let (r, _) = restored(&m, file, &[], false);
        assert_eq!((r.tabs.len(), r.tabs.iter().any(|tab| tab.task == Some(2))), (3, false));
        assert_eq!((r.tabs[r.active].title.as_str(), r.tabs[r.active].task), ("Tab 2", None));
    }

    /// ⌘T's tab (one empty pane) and the open section come back as they were.
    #[test]
    fn an_empty_tab_and_the_open_section_come_back() {
        let mut m = workspace();
        m.update(Action::NewEmptyTab);
        m.update(Action::ToggleSection { section: Section::Apps });
        let (r, _) = restored(&m, m.layout_file(&[], &[], 0), &[], false);
        let pane = r.focused.and_then(|id| r.pane(id)).map(|pane| pane.kind.clone());
        let leaves = r.layout.as_ref().map(Node::leaves).map(|leaves| leaves.len());
        assert_eq!((r.tabs.len(), r.active, leaves, pane), (m.tabs.len(), m.active, Some(1), Some(Kind::Empty)));
        assert_eq!(r.sidebar, Some(Section::Apps));
    }

    #[test]
    fn marks_stand_for_their_tabs_and_panes_again() {
        let mut m = workspace();
        let mark = |of, uid: &str| Action::MarkAdd {
            of,
            name: Some(uid.into()),
            key: None,
            uid: uid.into(),
            cwds: Vec::new(),
        };
        m.update(mark(MarkOf::Tab(None), "m1"));
        m.update(mark(MarkOf::Pane(Some(2)), "m2"));
        let file = m.layout_file(&[], &[], 0);
        let mut fresh = Model::default();
        fresh.update(Action::RestoreTasks { tasks: m.tasks.list.clone(), floor: 9 }.sent_by(Origin::System));
        fresh.update(Action::RestoreMarks { marks: m.marks.list.clone(), extra: Extra::new() }.sent_by(Origin::System));
        fresh.update(restore(file).sent_by(Origin::System));
        let open: Vec<(bool, Option<PaneId>)> = fresh.mark_rows().iter().map(|row| (row.open, row.pane)).collect();
        assert_eq!(open, [(true, None), (true, Some(3))]);
    }

    #[test]
    fn open_on_a_waiting_app_pane_starts_its_app_and_only_for_the_user() {
        let m = workspace();
        let (mut r, effects) = restored(&m, m.layout_file(&[], &[], 0), &[], false);
        let pane = effects.iter().find_map(|effect| match effect {
            Effect::HostWindow { into, .. } => *into,
            _ => None,
        });
        let pane = pane.unwrap();
        assert!(r.update(Action::UnfilledOpen { pane }.sent_by(Origin::Socket)).is_empty());
        let effects = r.update(Action::UnfilledOpen { pane }.sent_by(Origin::User));
        assert!(
            matches!(&effects[..], [Effect::HostWindow { launch: true, name, .. }] if name == "Slack"),
            "{effects:?}"
        );
        let (_, launched) = restored(&m, m.layout_file(&[], &[], 0), &[], true);
        assert!(launched.iter().any(|effect| matches!(effect, Effect::HostWindow { launch: true, .. })));
    }

    #[test]
    fn a_long_row_evened_out_keeps_its_shares_under_a_tenth() {
        let mut m = Model::default();
        for _ in 0..12 {
            split(&mut m, Right, Kind::Terminal);
        }
        m.update(Action::Equalize);
        let before = write(&m.layout_file(&[], &[], 0));
        let (r, _) = restored(&m, parse(&before).unwrap(), &[], false);
        assert_eq!(write(&r.layout_file(&[], &[], 0)), before);
    }

    #[test]
    fn a_change_is_told_once_and_never_after_start_fresh() {
        let mut m = Model::default();
        assert!(m.layout_moved());
        assert!(!m.layout_moved());
        split(&mut m, Right, Kind::Terminal);
        assert!(m.layout_moved());
        assert!(m.update(Action::StartFresh.sent_by(Origin::Socket)).is_empty());
        assert_eq!(m.update(Action::StartFresh), [Effect::ForgetLayout]);
        split(&mut m, Right, Kind::Terminal);
        assert!(!m.layout_moved());
    }

    #[test]
    fn the_settings_tab_is_kept_as_a_mark_and_comes_back_running_the_editor_once() {
        let mut m = Model::default();
        split(&mut m, Right, Kind::Terminal);
        m.update(Action::EditSettings);
        let pane = m.settings_pane.unwrap_or_default();
        // Kept as a mark on its pane, with none of its screen: an editor's screen is not history.
        let screen = History { lines: vec!["~ settings.json".into()], command: Some("vi".into()), saved: 1 };
        let file = m.layout_file(&[], &[(pane, screen)], 500);
        let text = write(&file);
        assert!(
            text.contains("\"settings\": 1") && !text.contains("settings.json") && !text.contains("\"vi\""),
            "{text}"
        );
        let read = parse(&text).unwrap();
        assert_eq!(read, file);
        // Restored by the shell at start: the editor again, in its tab, and no history there.
        let (mut r, effects) = restored(&m, read.clone(), &[], false);
        let again = r.settings_pane.unwrap_or_default();
        assert!(effects.contains(&Effect::EditSettings { pane: again }) && effects.contains(&Effect::Opened(again)));
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::History { .. })), "{effects:?}");
        assert_eq!(r.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(), ["Tab 1", "Settings"]);
        // ⌘, brings it forward: no second one.
        r.update(Action::SelectTab { index: 0 });
        assert_eq!((r.update(Action::EditSettings), r.tabs.len(), r.active), (vec![], 2, 1));
        // A client restores nothing, and opens none.
        let mut fresh = Model::default();
        assert!(fresh.update(restore(read).sent_by(Origin::Socket)).is_empty());
        assert_eq!(fresh.refusal(&Action::EditSettings, Origin::Socket), Some(crate::model::SETTINGS_USERS));
    }

    #[test]
    fn a_hand_made_settings_mark_runs_the_editor_only_in_a_terminal_of_its_tab_and_once() {
        let mut m = Model::default();
        split(&mut m, Right, Kind::Terminal);
        let slack = Kind::App { window: 9, title: "general".into(), bundle: None, name: "Slack".into() };
        m.update(Action::Open { kind: slack, axis: Right, beside: None });
        m.update(Action::NewTab);
        split(&mut m, Right, Kind::Terminal);
        let mut file = m.layout_file(&[], &[], 0);
        // On the app pane, on no pane, and twice on terminals: one editor, in the first.
        for (marks, editors) in [([Some(2), None], 0), ([Some(99), Some(99)], 0), ([Some(1), Some(1)], 1)] {
            file.tabs[0].settings = marks[0];
            file.tabs[1].settings = marks[1];
            let (r, effects) = restored(&m, file.clone(), &[], false);
            let edits: Vec<&Effect> = effects.iter().filter(|e| matches!(e, Effect::EditSettings { .. })).collect();
            assert_eq!((edits.len(), r.settings_pane.is_some()), (editors, editors == 1), "{marks:?}");
        }
    }

    #[test]
    fn a_kind_this_build_does_not_know_is_an_empty_pane_not_an_unread_file() {
        let text = r#"{"version": 1, "tabs": [{"title": "t", "layout": {"leaf": 1},
            "panes": [{"id": 1, "kind": {"type": "hologram", "size": 3}}]}]}"#;
        let file = parse(text).unwrap();
        assert_eq!(file.tabs[0].tab.panes[0].kind, RecordKind::Empty);
        let (r, _) = restored(&Model::default(), file, &[], false);
        assert_eq!(r.panes.iter().map(|pane| pane.kind.clone()).collect::<Vec<_>>(), [Kind::Empty]);
        // `marks.json` with one is not read, so it is left alone: nothing of it is lost.
        let marks = r#"{"version": 1, "marks": [{"name": "x", "target": {"pane": {"kind": {"type": "hologram"}}}}]}"#;
        assert!(crate::marks::parse(marks).is_err());
    }
}
