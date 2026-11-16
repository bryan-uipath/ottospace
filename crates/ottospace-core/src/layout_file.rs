//! The layout file, `layout.json`: the top row's spaces (hidden ones too), the tabs in row order
//! (each with its space, and its task, if any), their splits and panes, the tab on show, the
//! section and the drawer, kept between runs. Session restore's layer 1 (`docs/ai/session-restore.md`). A pane is kept as a workbook
//! keeps one (`marks::PaneRecord`, `workbook::History`), by the same rules. The shell reads and
//! writes the file; read, it is untrusted input and is checked here.

use serde::{Deserialize, Serialize};

use crate::marks::{self, Extra, RecordKind, TabRecord, Target};
use crate::model::{Effect, HOME_TAB, Kind, Model, Owner, PaneId, Section, Space};
use crate::talk::{self, Talk};
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
/// The most folded repositories kept.
const MAX_FOLDED: usize = 256;

/// Why a client's request does nothing: Start Fresh, or an app a restored pane names.
pub const USERS_ALONE: &str = "only the user starts fresh or opens a restored pane's app";

/// `layout.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    /// The tab on show, by its place in `tabs`.
    #[serde(default)]
    pub selected: Option<usize>,
    /// Home was on show: no tab of `tabs` is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub home: bool,
    /// The left section that was open.
    #[serde(default)]
    pub section: Option<Section>,
    #[serde(default)]
    pub drawer: Drawer,
    /// The tabs, in row order. A task with no tab has none here: that is normal.
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    /// The top row in order, hidden spaces too. None in an older build's file: its tabs go by
    /// their `task`, the others to the first space. The first, `workspace`, is Home: it has no
    /// tab. An older build's had: read, they are a kept workspace's, "Workspace".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<SavedSpace>,
    /// The Repos section's repositories folded under their name, by path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos_folded: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A space of the top row. Of a `kind` this build does not know, or a task or workbook gone, its
/// tabs are of no space, as a session's, which is not kept: read, they go to the workspace
/// selected last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSpace {
    /// `workspace`, `task` or `workbook`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskId>,
    /// A workbook's id for good (`Workbook::uid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workbook: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Its tab shown last, by place in `tabs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<usize>,
    /// The other tasks in a task's space, in the order they joined it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskId>,
    /// The task in front there, when not its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<TaskId>,
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
    /// Each task's dial as the user left it: collapsed (true) or open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dials: Vec<(TaskId, bool)>,
}

fn width() -> f32 {
    talk::WIDTH
}

impl Default for Drawer {
    fn default() -> Self {
        Drawer { task: None, open: false, width: talk::WIDTH, wide: false, views: Vec::new(), dials: Vec::new() }
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
    /// Its space, by place in `spaces`. None: a session's, which is not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<usize>,
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
/// A folded repository's path as kept: absolute, one line, not huge.
fn folded_path(path: &str) -> bool {
    path.starts_with('/') && path.len() <= 4096 && !path.chars().any(char::is_control)
}

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
        if self.spaces.len() > MAX_TABS + 1 {
            return Some("it has too many spaces");
        }
        if self.repos_folded.len() > MAX_FOLDED || !self.repos_folded.iter().all(|p| folded_path(p)) {
            return Some("it folds too many repositories, or one by what is not a path");
        }
        if self.spaces.iter().any(|space| space.workbook.as_ref().is_some_and(|uid| !workbook::valid_uid(uid))) {
            return Some("a space names a workbook by what is not an id");
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
        // The row as kept. A session's space is not: its tabs name none.
        let kept = |space: &Space| {
            let (kind, task, workbook) = match &space.of {
                Owner::Home => ("workspace", None, None),
                Owner::Task { id } => ("task", Some(known(Some(*id))?), None),
                Owner::Workbook { id } => ("workbook", None, Some(self.workbooks.get(*id)?.uid.clone())),
                Owner::Session { .. } => return None,
            };
            let tasks: Vec<TaskId> = space.joined.iter().copied().filter(|id| known(Some(*id)).is_some()).collect();
            let front = self.front(&space.of).filter(|id| tasks.contains(id));
            let saved = SavedSpace { kind: kind.into(), task, workbook, hidden: space.hidden, tab: None, tasks, front };
            Some((space.of.clone(), space.last, saved))
        };
        let mut spaces: Vec<(Owner, Option<u32>, SavedSpace)> = self.spaces.iter().filter_map(kept).collect();
        let (mut records, mut links, mut selected) = (Vec::new(), Vec::new(), None);
        // Home's own tab is not kept: it is always there.
        for (index, tab) in self.tabs.iter().enumerate().filter(|(_, tab)| tab.id != HOME_TAB).take(MAX_TABS) {
            let Some(mut record) = self.tab_record(index, cwds, &histories) else { continue };
            let space = spaces.iter().position(|(of, ..)| *of == tab.space);
            // An older build reads a tab's task from the record.
            record.task = known(space.and_then(|at| spaces[at].0.task()));
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
            let last = space.and_then(|at| spaces.get_mut(at)).filter(|(_, last, _)| *last == Some(tab.id));
            if let Some((_, last, saved)) = last {
                (saved.tab, *last) = (Some(records.len()), None);
            }
            records.push(record);
            links.push((book, marked, settings, space));
        }
        workbook::fit_history(&mut records);
        let tabs = records.into_iter().zip(links);
        let tabs = tabs
            .map(|(tab, (workbook, marks, settings, space))| SavedTab { tab, workbook, marks, settings, space })
            .collect();
        let spaces: Vec<SavedSpace> = spaces.into_iter().map(|(.., saved)| saved).collect();
        let drawer = Drawer {
            task: self.talk.as_ref().and_then(|talk| known(talk.task.task())),
            open: self.talk.as_ref().is_some_and(|talk| talk.open && known(talk.task.task()).is_some()),
            width: self.talk_width,
            wide: self.talk_wide,
            // Of live tasks only, as a restore keeps them: a dismissed task's pick goes with it.
            views: self.saved_views().into_iter().filter(|(task, _)| self.tasks.live(*task).is_some()).collect(),
            dials: self.saved_dials().into_iter().filter(|(task, _)| self.tasks.live(*task).is_some()).collect(),
        };
        // Not the Calendar: a build without it could not read the file back.
        let section = self.sidebar.filter(|s| *s != Section::Calendar);
        // Of listed repositories only: one gone from the list takes its fold with it.
        let listed = |path: &&String| folded_path(path) && self.repos.iter().any(|repo| repo.path == **path);
        let repos_folded = self.repos_folded.iter().filter(listed).take(MAX_FOLDED).cloned().collect();
        let home = *self.space() == Owner::Home;
        let extra = Extra::new();
        let mut layout = Layout { version: VERSION, selected, home, section, drawer, tabs, spaces, repos_folded, extra };
        // What is written always reads back: the last histories go first, then the last tabs.
        while write(&layout).len() > MAX_FILE_BYTES {
            let held = layout.tabs.iter_mut().flat_map(|saved| saved.tab.panes.iter_mut()).rev();
            match held.into_iter().find(|pane| pane.extra.contains_key("history")) {
                Some(pane) => drop(pane.extra.remove("history")),
                None => drop(layout.tabs.pop()),
            }
        }
        layout.selected = layout.selected.filter(|at| *at < layout.tabs.len());
        let count = layout.tabs.len();
        layout.spaces.iter_mut().for_each(|space| space.tab = space.tab.filter(|at| *at < count));
        layout
    }

    /// True while the model is as it starts: Home alone. Only that is restored into.
    fn fresh(&self) -> bool {
        let empty = |tab: &crate::model::Tab| tab.space == Owner::Home && tab.layout.is_none();
        self.panes.is_empty() && self.layout.is_none() && self.tabs.iter().all(empty) && self.spaces.len() == 1
    }

    /// Whose a kept space is now: no one's (`Owner::Home`), if its task is gone or its
    /// workbook not kept.
    fn owner_of(&self, saved: &SavedSpace) -> Owner {
        let task = saved.task.filter(|id| saved.kind == "task" && self.tasks.live(*id).is_some());
        let book = saved.workbook.as_ref().filter(|_| saved.kind == "workbook");
        let book = book.and_then(|uid| self.workbooks.list.iter().find(|book| book.uid == *uid));
        match (task, book) {
            (Some(id), _) => Owner::Task { id },
            (_, Some(book)) => Owner::Workbook { id: book.id },
            _ => Owner::Home,
        }
    }

    /// Builds the spaces of `layout` in row order, hidden ones hidden, and their tabs, each as a
    /// workbook's (`rebuild_tab`); then puts back the tab on show (or Home), each space's last,
    /// the section, the drawer and what marks and workbooks stood for. A file with no spaces (an
    /// older build's) puts a tab in its task's space, the others in the first. The first
    /// space's tabs (an older build's: Home has none) are a kept workspace's, "Workspace", made
    /// here if one of them holds a pane; a tab of no space (its task gone, its workbook not
    /// kept, a session's) goes to the workspace selected last at the step's end (`settle_home`).
    /// Terminals in a directory of `gone` start in `home`; each shows its history, its command
    /// typed and not run. App panes ask for their windows; `launch`: an app that is not running
    /// may be started. No conversation starts. Only into a fresh model, and only a sound file.
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
        self.repos_folded.clone_from(&layout.repos_folded);
        let mut owners: Vec<Owner> = layout.spaces.iter().map(|saved| self.owner_of(saved)).collect();
        // The first space's tabs, by place: all of it that is named, and in a file with no
        // spaces every tab of no live task.
        let named = |saved: &SavedTab| saved.space.and_then(|at| layout.spaces.get(at));
        let first = |saved: &SavedTab| match named(saved) {
            Some(space) => space.kind == "workspace",
            None => layout.spaces.is_empty() && saved.tab.task.is_none_or(|id| self.tasks.live(id).is_none()),
        };
        // They are a workspace's from now on, if there is anything in them.
        let holds = |saved: &SavedTab| !saved.tab.shown().is_empty();
        let upgraded = layout.tabs.iter().any(|saved| first(saved) && holds(saved));
        let firsts: Vec<bool> = layout.tabs.iter().map(first).collect();
        let upgraded = upgraded.then(|| self.stray_workspace(Some("Workspace".into())));
        // Where the first space was on the row: after Home.
        if let Some(of) = &upgraded {
            self.spaces.insert(1, Space::new(of.clone()));
        }
        let task_of = |model: &Model, saved: &SavedTab| {
            let task = saved.tab.task.filter(|id| model.tasks.live(*id).is_some());
            task.map_or(Owner::Home, |id| Owner::Task { id })
        };
        let of_tab = |model: &Model, at: usize, saved: &SavedTab| match (&upgraded, firsts[at]) {
            (Some(of), true) => of.clone(),
            _ => saved.space.and_then(|at| owners.get(at).cloned()).unwrap_or_else(|| task_of(model, saved)),
        };
        let of_tab: Vec<Owner> = layout.tabs.iter().enumerate().map(|(at, saved)| of_tab(self, at, saved)).collect();
        // Nothing may stand for Home's tab (a workbook saved from one, a mark), as on close.
        self.marks.forget_tab(HOME_TAB);
        self.workbooks.forget_tab(HOME_TAB);
        // The upgraded workspace is where the first space was on the row.
        for (owner, saved) in owners.iter_mut().zip(&layout.spaces) {
            if let Some(of) = upgraded.as_ref().filter(|_| saved.kind == "workspace") {
                *owner = of.clone();
            }
        }
        for of in &owners {
            self.add_space(of);
        }
        let mut built: Vec<(u32, Vec<PaneId>)> = Vec::new();
        for (saved, of) in layout.tabs.iter().zip(of_tab) {
            let mut record = saved.tab.clone();
            for pane in record.panes.iter_mut().filter(|pane| pane.kind == RecordKind::Terminal) {
                if pane.cwd.as_ref().is_some_and(|dir| gone.contains(dir)) {
                    pane.cwd = home.clone();
                }
            }
            let (base, at) = (self.next_id, effects.len());
            let tab = self.rebuild_tab(&record, of, effects);
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
        // Each space's last tab, as kept, else it shows its first; and hidden as it was.
        for space in &mut self.spaces {
            space.last = None;
        }
        for (of, saved) in owners.iter().zip(&layout.spaces) {
            let live = |id: &TaskId| self.tasks.live(*id).is_some();
            let joined: Vec<TaskId> = saved.tasks.iter().copied().filter(live).collect();
            if let Some(space) = self.spaces.iter_mut().find(|space| space.of == *of && *of != Owner::Home) {
                space.hidden = saved.hidden;
                // A task's space only; what is unsound of it (a task twice, one with a space
                // of its own) goes at the step's end (`settle_joined`).
                if of.task().is_some() {
                    (space.joined, space.front) = (joined, saved.front);
                }
            }
            let id = saved.tab.and_then(|at| built.get(at)).map(|(id, _)| *id);
            let id = id.filter(|id| self.tabs.iter().any(|tab| tab.id == *id && tab.space == *of));
            if let Some(space) = self.spaces.iter_mut().find(|space| space.of == *of).filter(|_| id.is_some()) {
                space.last = id;
            }
        }
        self.restore_selected(layout.selected.filter(|_| !layout.home), &built, effects);
        // Home was on show; and it is, with nothing else to show.
        if layout.home && let Some(at) = self.tabs.iter().position(|tab| tab.id == HOME_TAB) {
            self.select_tab(at, effects);
        }
        // A tab of no space goes to the selected workspace.
        self.last_spaces = vec![self.space().clone()];
        self.last_spaces.retain(|of| matches!(of, Owner::Task { .. } | Owner::Workbook { .. }));
        // A Calendar left open by a run that had a feed: none without one.
        // Not a section that is not on the rail: Calendar with no feed, one behind a flag that is off.
        self.sidebar = layout.section.filter(|s| self.sections().contains(s));
        self.talk_width = talk::width(layout.drawer.width);
        self.talk_wide = layout.drawer.wide;
        let live = |task: &TaskId| self.tasks.live(*task).is_some();
        let views = layout.drawer.views.iter().filter(|(task, _)| live(task)).map(|(task, view)| (*task, *view));
        let views: Vec<_> = views.collect();
        let dials = layout.drawer.dials.iter().filter(|(task, _)| live(task)).map(|(task, c)| (*task, *c));
        let dials: Vec<_> = dials.collect();
        self.restore_views(views);
        self.restore_dials(dials);
        // The selected space's conversation, shown to go on with: never started here.
        self.drawer_open = layout.drawer.open;
        if let Some(subject) = self.space_subject(self.space()) {
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
    use crate::talk::Subject;
    use crate::tasks::{Assign, State};
    use crate::workbook::valid_uid as valid;

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

    /// Two tabs of a workspace ("Workspace 2", made from Home) with splits, a task's space of
    /// one tab, selected, the drawer on its chat, the Tasks section.
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
        m.update(Action::TaskTalk { id: 1 });
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::ToggleSection { section: Section::Tasks });
        m
    }

    /// A fresh model with `m`'s tasks and workspaces, and the file of `m` restored into it.
    fn restored(m: &Model, file: Layout, gone: &[&str], launch: bool) -> (Model, Vec<Effect>) {
        let mut fresh = Model::default();
        fresh.update(Action::RestoreTasks { tasks: m.tasks.list.clone(), floor: 9 }.sent_by(Origin::System));
        fresh.update(Action::RestoreWorkbooks { workbooks: m.workbooks.list.clone() }.sent_by(Origin::System));
        let gone = gone.iter().map(|dir| dir.to_string()).collect();
        let restore = Action::RestoreLayout { layout: Box::new(file), gone, home: Some("/Users/me".into()), launch };
        let effects = fresh.update(restore.sent_by(Origin::System));
        (fresh, effects)
    }

    fn titles(m: &Model) -> Vec<(String, Owner)> {
        m.tabs.iter().map(|tab: &Tab| (tab.title.clone(), tab.space.clone())).collect()
    }

    /// `layout.json` as the build before Home wrote it: its first space, "Workspace", held tabs.
    const BEFORE_HOME: &str = include_str!("fixtures/layout-before-home.json");

    fn row(m: &Model) -> Vec<(String, Vec<usize>)> {
        m.space_rows().into_iter().map(|row| (m.space_title(&row.of), row.tabs)).collect()
    }

    #[test]
    fn an_older_files_first_space_is_a_kept_workspace_named_workspace_and_nothing_is_lost() {
        let mut m = Model::default();
        task(&mut m, "fix zoom");
        let file = parse(BEFORE_HOME).unwrap();
        let (r, effects) = restored(&m, file.clone(), &[], false);
        // Its tabs, the empty one too, are a workspace's, where the first space was; Home holds none.
        let home = ("Home".to_string(), vec![3]);
        assert_eq!(row(&r), [home, ("Workspace".into(), vec![0, 1]), ("fix zoom".into(), vec![2])]);
        let book = &r.workbooks.list[0];
        assert_eq!((book.name.as_str(), book.tabs.len(), valid(&book.uid)), ("Workspace", 0, true));
        // Selected, as it was, with its tab, its split and its terminals.
        assert_eq!((r.space(), r.tabs[r.active].title.as_str()), (&Owner::Workbook { id: book.id }, "notes"));
        assert!(matches!(&r.layout, Some(Node::Split { ratio, .. }) if (ratio - 0.4).abs() < 0.01), "{:?}", r.layout);
        assert_eq!((r.panes.len(), r.pane(1).and_then(|pane| pane.cwd.as_deref())), (3, Some("/code/app")));
        assert_eq!(effects.iter().filter(|effect| matches!(effect, Effect::Opened(_))).count(), 3);
        assert_eq!((r.tab_panes()[3].len(), r.sidebar), (0, Some(Section::Tasks)));
        // Written again, it names the workspace; read again, the same: no second one is made.
        let again = parse(&write(&r.layout_file(&[], &[], 0))).unwrap();
        let kinds: Vec<&str> = again.spaces.iter().map(|space| space.kind.as_str()).collect();
        assert_eq!((kinds, again.tabs.len(), again.home), (vec!["workspace", "workbook", "task"], 3, false));
        let (twice, _) = restored(&r, again, &[], false);
        assert_eq!((row(&twice), twice.workbooks.list.len()), (row(&r), 1));
        // The task's tab was on show: it is, and the workspace is there beside it.
        let (r, _) = restored(&m, Layout { selected: Some(2), ..file.clone() }, &[], false);
        assert_eq!((r.space(), r.spaces().len()), (&Owner::Task { id: 1 }, 3));
        // A name taken: the next free one, as + names one.
        let mut named = Model::default();
        named.update(Action::WorkbookNew { uid: "00000000000000a1".into() });
        named.workbooks.list[0].name = "Workspace".into();
        let (r, _) = restored(&named, file, &[], false);
        assert_eq!(r.workbooks.list.iter().map(|book| book.name.as_str()).collect::<Vec<_>>(), ["Workspace", "Workspace 2"]);
    }

    #[test]
    fn an_older_files_empty_first_space_is_just_home() {
        let mut file = parse(BEFORE_HOME).unwrap();
        file.tabs.remove(0);
        (file.spaces[0].tab, file.spaces[1].tab, file.selected) = (Some(0), Some(1), Some(0));
        let mut m = Model::default();
        task(&mut m, "fix zoom");
        let (r, _) = restored(&m, file.clone(), &[], false);
        // The empty tab was on show: Home is. No workspace is made for nothing.
        assert_eq!(row(&r), [("Home".to_string(), vec![1]), ("fix zoom".into(), vec![0])]);
        assert_eq!((r.space(), r.workbooks.list.len(), r.home(&[]).is_some()), (&Owner::Home, 0, true));
        // No tab at all: Home alone.
        let (r, _) = restored(&Model::default(), Layout { tabs: Vec::new(), spaces: Vec::new(), ..file }, &[], false);
        assert_eq!((r.tabs.len(), r.spaces().len(), r.workbooks.list.len()), (1, 1, 0));
    }

    #[test]
    fn home_on_show_comes_back_and_a_tab_of_no_space_goes_to_the_workspace_selected_last() {
        let mut m = workspace();
        m.update(Action::SpaceSelect { index: 0 });
        assert_eq!(m.space(), &Owner::Home);
        let file = parse(&write(&m.layout_file(&[], &[], 0))).unwrap();
        assert_eq!((file.home, file.selected, file.tabs.len()), (true, None, 3));
        let (r, _) = restored(&m, file.clone(), &[], false);
        assert_eq!((r.space(), r.tabs.len(), r.layout.is_none()), (&Owner::Home, 4, true));
        assert_eq!(write(&r.layout_file(&[], &[], 0)), write(&file));
        // The task gone: its tab is the selected workspace's, and no workspace is made for it.
        let mut gone = Model::default();
        gone.update(Action::RestoreWorkbooks { workbooks: m.workbooks.list.clone() }.sent_by(Origin::System));
        gone.update(restore(Layout { home: false, selected: Some(0), ..file.clone() }).sent_by(Origin::System));
        assert_eq!(row(&gone), [("Home".to_string(), vec![3]), ("Workspace 2".into(), vec![0, 1, 2])]);
        // Home on show and no workspace left either: one is made, kept like any other.
        let mut none = Model::default();
        none.update(restore(file).sent_by(Origin::System));
        assert_eq!(row(&none), [("Home".to_string(), vec![3]), ("Workspace 2".into(), vec![0, 1, 2])]);
        assert_eq!((none.workbooks.list.len(), none.tab_panes()[3].len()), (1, 0));
    }

    #[test]
    fn an_older_files_tabs_go_to_their_tasks_spaces_and_the_rest_to_a_workspace_first() {
        let mut m = Model::default();
        task(&mut m, "one");
        task(&mut m, "two");
        let mut file = Model::default().layout_file(&[], &[], 0);
        let pane = crate::marks::PaneRecord { id: 1, kind: RecordKind::Terminal, cwd: None, extra: Extra::new() };
        let tab = |title: &str, task, panes: Vec<crate::marks::PaneRecord>| SavedTab {
            tab: TabRecord {
                title: title.into(),
                task,
                focused: None,
                layout: panes.first().map(|pane| Node::Leaf(pane.id)),
                panes,
                extra: Extra::new(),
            },
            workbook: None,
            marks: Vec::new(),
            settings: None,
            space: None,
        };
        let none = Vec::new;
        file.tabs = vec![
            tab("b", Some(2), none()),
            tab("c", Some(1), none()),
            tab("d", Some(2), none()),
            tab("a", None, vec![pane]),
            tab("e", Some(7), none()),
        ];
        (file.spaces, file.selected, file.home) = (Vec::new(), Some(2), false);
        let (r, _) = restored(&m, file, &[], false);
        let order: Vec<&str> = r.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!((order, r.tabs[r.active].title.as_str()), (vec!["a", "e", "b", "d", "c", "Home"], "d"));
        let spaces = [("Home", vec![5]), ("Workspace", vec![0, 1]), ("two", vec![2, 3]), ("one", vec![4])];
        assert_eq!(row(&r), spaces.map(|(name, tabs)| (name.to_string(), tabs)));
    }

    #[test]
    fn a_workspaces_tasks_and_the_one_in_front_come_back() {
        let mut m = workspace();
        for line in ["second", "third"] {
            m.update(Action::OpenCompose { text: line.into() });
            m.update(Action::ComposeEnter);
        }
        m.update(Action::TalkFront { task: 2 });
        let of = Owner::Task { id: 1 };
        assert_eq!((m.members(&of), m.front(&of)), (vec![1, 2, 3], Some(2)));
        let file = parse(&write(&m.layout_file(&[], &[], 0))).unwrap();
        assert_eq!((&file.spaces[2].tasks, file.spaces[2].front), (&vec![2, 3], Some(2)));
        let (r, _) = restored(&m, file.clone(), &[], false);
        assert_eq!((r.members(&of), r.front(&of), r.tab_task()), (vec![1, 2, 3], Some(2), Some(2)));
        assert_eq!(r.talk.as_ref().map(|talk| &talk.task), Some(&Subject::Task(2)));
        assert_eq!(write(&r.layout_file(&[], &[], 0)), write(&file));
        // A task of the file that is gone, or in two spaces, is left out.
        let mut odd = file;
        odd.spaces[2].tasks = vec![3, 2, 3, 8, 1, 2];
        let (r, _) = restored(&m, odd, &[], false);
        assert_eq!((r.members(&of), r.front(&of)), (vec![1, 3, 2], Some(2)));
    }

    #[test]
    fn the_row_comes_back_with_its_hidden_spaces_and_each_ones_last_tab() {
        let mut m = workspace();
        m.update(Action::NewTab); // the task's second tab
        m.update(Action::SelectTab { index: 2 });
        m.update(Action::SpaceSelect { index: 1 });
        m.update(Action::SpaceHide { index: Some(2) });
        let file = parse(&write(&m.layout_file(&[], &[], 0))).unwrap();
        let kinds: Vec<(&str, bool, Option<usize>)> =
            file.spaces.iter().map(|space| (space.kind.as_str(), space.hidden, space.tab)).collect();
        // Home, which has no tab, the workspace and the task's space.
        assert_eq!(kinds, [("workspace", false, None), ("workbook", false, Some(1)), ("task", true, Some(2))]);
        let (mut r, _) = restored(&m, file.clone(), &[], false);
        assert_eq!(write(&r.layout_file(&[], &[], 0)), write(&file));
        assert_eq!((r.shown_spaces().len(), r.talk.is_none()), (2, true));
        // Opened again, it shows the tab it showed last, its chat as the drawer was.
        r.update(Action::TaskTalk { id: 1 });
        assert_eq!((r.active, r.talk.as_ref().map(|talk| talk.open)), (2, Some(true)));
        // A space of a kind this build does not know, or a task gone: its tabs are the
        // selected workspace's, but the one that holds nothing: it is closed.
        let mut odd = file;
        odd.spaces[2].kind = "garden".into();
        let (r, _) = restored(&m, odd, &[], false);
        assert_eq!((r.spaces().len(), r.tabs.len(), r.space_tabs(&Owner::Workbook { id: 1 }).len()), (2, 4, 3));
    }

    #[test]
    fn a_workspace_comes_back_as_it_was_and_no_conversation_starts() {
        let m = workspace();
        let cwds = vec![(1, "/code/app".to_string()), (2, "/code/web".to_string())];
        let file = parse(&write(&m.layout_file(&cwds, &[], 500))).unwrap();
        assert_eq!((file.tabs.len(), file.selected, file.section), (3, Some(2), Some(Section::Tasks)));
        assert_eq!(file.drawer.task, Some(1));
        let (r, effects) = restored(&m, file, &[], false);
        // The workspace's tabs first, then the task's space; Home's own tab after all.
        let (tab, book) = (|title: &str, of| (title.to_string(), of), Owner::Workbook { id: 1 });
        let task = tab("fix zoom", Owner::Task { id: 1 });
        assert_eq!(titles(&r), [tab("Tab 1", book.clone()), tab("Tab 2", book), task, tab("Home", Owner::Home)]);
        assert_eq!((r.tab_task(), r.active, r.sidebar), (Some(1), 2, Some(Section::Tasks)));
        assert_eq!(write(&r.layout_file(&[], &[], 0)), write(&m.layout_file(&cwds, &[], 0)));
        // The split's ratio, and each terminal's directory.
        let first = r.tabs[0].layout.as_ref();
        assert!(matches!(first, Some(Node::Split { ratio, .. }) if (ratio - 0.5).abs() > 0.01), "{first:?}");
        let dirs: Vec<Option<&str>> = r.panes.iter().take(2).map(|pane| pane.cwd.as_deref()).collect();
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
        // So does the dial the user opened on its ended agent.
        let mut opened = workspace();
        opened.restore_dials(vec![(1, false), (99, true)]);
        let file = parse(&write(&opened.layout_file(&[], &[], 0))).unwrap();
        assert_eq!(file.drawer.dials, [(1, false)], "a task that is gone is not kept");
        let (back, _) = restored(&opened, file, &[], false);
        assert!(!back.dial_collapsed(&Subject::Task(1)) && workspace().dial_collapsed(&Subject::Task(1)));
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
        // Workspace's tabs come first: its first terminal is pane 1.
        assert_eq!(r.pane(1).and_then(|pane| pane.cwd.as_deref()), Some("/Users/me"));
        let at = effects.iter().position(|effect| *effect == Effect::Opened(1)).unwrap();
        let shown = effects[at..].iter().find_map(|effect| match effect {
            Effect::History { pane: 1, history } => Some(history.command.clone()),
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
        // Nor of a workspace that is not kept: all three are one new workspace's.
        assert_eq!(fresh.tabs.iter().filter(|tab| tab.space.task().is_some()).count(), 0);
        assert_eq!(fresh.space_tabs(&Owner::Workbook { id: 1 }).len(), 3);
        assert_eq!((fresh.tabs.len(), fresh.talk.is_none()), (4, true));
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
        m.update(Action::SelectTab { index: 1 });
        let file = m.layout_file(&[], &[], 0);
        let (r, _) = restored(&m, file, &[], false);
        assert_eq!((r.tabs.len(), r.tabs.iter().any(|tab| tab.space == Owner::Task { id: 2 })), (4, false));
        assert_eq!((r.tabs[r.active].title.as_str(), &r.tabs[r.active].space), ("Tab 2", &Owner::Workbook { id: 1 }));
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
        assert_eq!(open, [(true, None), (true, Some(2))]);
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
        assert_eq!(r.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(), ["Tab 1", "Settings", "Home"]);
        // ⌘, brings it forward: no second one.
        r.update(Action::SelectTab { index: 0 });
        assert_eq!((r.update(Action::EditSettings), r.tabs.len(), r.active), (vec![], 3, 1));
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
