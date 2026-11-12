//! Marks: named, saved ways back to a pane or a whole tab. A mark records what to open, never a
//! window id or a pid alone: those do not survive a restart. `PaneRecord` and `TabRecord` have
//! the shape of `session.json`'s entries (`docs/ai/session-restore.md`), so session restore can
//! share them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::layout::{Axis, MIN_SHARE, Node};
use crate::list::ListRow;
use crate::model::{Effect, Kind, Mode, Model, Pane, PaneId};
use crate::tasks::{Moved, NOTICE_MS, Notice, TaskId};

pub type MarkId = u32;

/// The longest name a mark may have, in characters.
pub const MAX_NAME: usize = 40;

/// The `version` of `marks.json` this build reads and writes.
pub const VERSION: u32 = 1;

/// Why a socket client's request is refused while a mark waits for a letter or a name.
pub const INPUT_ON: &str = "a mark is waiting for a letter or a name";

/// What `marks.json` says once when it holds a `command`: no mark types anything into a shell.
pub const NO_COMMANDS: &str = "marks.json: commands are not supported; ignored";

/// Keys of a record this build does not know. Kept, so a newer build's file loses nothing here.
pub type Extra = BTreeMap<String, serde_json::Value>;

/// What a recorded pane shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordKind {
    Empty,
    Terminal,
    /// An app's window: the app by bundle id (else by name), the window by its title.
    App {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle: Option<String>,
        name: String,
        /// A hint: titles change. e.g. "general - Slack".
        #[serde(default)]
        title: String,
        #[serde(flatten)]
        extra: Extra,
    },
}

fn is_zero(id: &u32) -> bool {
    *id == 0
}

/// A pane as it is kept on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneRecord {
    /// The pane's id when it was recorded: what a tab's `layout` calls it. A new pane gets a new id.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub id: u32,
    pub kind: RecordKind,
    /// A terminal's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A tab as it is kept on disk: its split tree, whose leaves are the `id`s of `panes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabRecord {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<u32>,
    #[serde(default)]
    pub layout: Option<Node>,
    #[serde(default)]
    pub panes: Vec<PaneRecord>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What a mark goes back to: `{"pane": {..}}` or `{"tab": {..}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Pane(PaneRecord),
    Tab(TabRecord),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// This run's name for the mark. Not kept.
    #[serde(skip)]
    pub id: MarkId,
    /// The mark's own name for good, `id` in the file: two running ottospaces tell by it which
    /// mark the other added, changed or removed. The shell makes it up; empty until it has.
    #[serde(rename = "id", default, skip_serializing_if = "String::is_empty")]
    pub uid: String,
    pub name: String,
    /// The letter that jumps to it, `a` to `z`. No two marks share one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<char>,
    pub target: Target,
    /// The pane or tab id it stood for last, this run. Only a hint for "is it open now".
    #[serde(skip)]
    pub live: Option<u32>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Something a mark cannot be opened without.
#[derive(Clone, Debug, PartialEq)]
pub enum Need {
    /// A terminal's directory.
    Dir(String),
    /// An app that must be installed, or running.
    App { bundle: Option<String>, name: String },
}

impl Need {
    fn gone(&self) -> String {
        match self {
            Need::Dir(path) => format!("its directory is gone: {path}"),
            Need::App { name, .. } => format!("{name} is not installed"),
        }
    }
}

/// Which mark a request means.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkRef {
    Id(MarkId),
    /// Its name; the case of the letters does not matter.
    Name(String),
    Key(char),
    /// Its place in the list, from 0.
    Place(usize),
}

/// What to mark. `None`: the focused pane, or the tab on show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MarkOf {
    Pane(Option<PaneId>),
    /// By the tab's `id`.
    Tab(Option<u32>),
}

/// Which part of a mark is being typed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Name,
    Key,
}

/// A mark has the keyboard.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MarkInput {
    /// The jump prefix was pressed: the next letter goes to the mark with that letter.
    Jump,
    /// A mark's name is being typed. ↵ keeps it.
    Rename { mark: MarkId, text: String },
    /// A mark waits for its letter.
    SetKey { mark: MarkId },
}

/// A pane of a tab built from a mark that could not be given what its record names. It is an
/// empty pane, and says why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Unfilled {
    pub pane: PaneId,
    #[serde(skip)]
    pub record: PaneRecord,
    pub why: String,
}

/// A mark as the bar and the state show it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MarkRow {
    pub id: MarkId,
    pub name: String,
    pub key: Option<char>,
    /// `terminal`, `app` or `tab`.
    pub kind: &'static str,
    /// Its target is open now: `pane` or `tab` (a tab's `id`) says which.
    pub open: bool,
    pub pane: Option<PaneId>,
    pub tab: Option<u32>,
    /// False: it is not open and nothing of it can be opened.
    pub available: bool,
    /// Why it is not available, or what of a tab is missing.
    pub reason: Option<String>,
    /// The panes of its open tab that are still empty, each with why.
    pub unfilled: Vec<Unfilled>,
}

/// A window of the app a mark names, as the shell sees it now.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: u32,
    pub title: String,
    /// The app says it is its main window.
    pub main: bool,
    /// The pane that holds it already.
    pub pane: Option<PaneId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// Put this window in a pane.
    Host(u32),
    /// Its window is in this pane already: go there.
    Focus(PaneId),
    None,
}

/// Which window a recorded app pane gets. Among the windows no pane holds: the one titled
/// `title`, else the only one, else the app's main one. A window in a pane is never taken for
/// another pane; with no free window, the pane that holds the titled one (else the only one).
pub fn pick_window(title: &str, windows: &[Window]) -> Pick {
    fn titled<'a>(list: &[&'a Window], title: &str) -> Option<&'a Window> {
        list.iter().find(|w| !title.is_empty() && w.title == title).copied()
    }
    fn only<'a>(list: &[&'a Window]) -> Option<&'a Window> {
        if list.len() == 1 { list.first().copied() } else { None }
    }
    let free: Vec<&Window> = windows.iter().filter(|w| w.pane.is_none()).collect();
    let held: Vec<&Window> = windows.iter().filter(|w| w.pane.is_some()).collect();
    if let Some(window) = titled(&free, title).or(only(&free)).or(free.iter().find(|w| w.main).copied()) {
        return Pick::Host(window.id);
    }
    match titled(&held, title).or(only(&held)).and_then(|w| w.pane) {
        Some(pane) => Pick::Focus(pane),
        None => Pick::None,
    }
}

/// How long a started app's windows must stay the same in number before one is picked: an app
/// that brings its own windows back opens them one by one.
pub const SETTLE_MS: u64 = 1_500;

/// True once a started app's windows have settled: it has one, and their number last changed
/// at least `SETTLE_MS` ago.
pub fn settled(windows: usize, unchanged_ms: u64) -> bool {
    windows > 0 && unchanged_ms >= SETTLE_MS
}

/// True if two app identities name one app: by bundle id when both have one, else by name.
pub fn same_app(a: (Option<&str>, &str), b: (Option<&str>, &str)) -> bool {
    match (a.0, b.0) {
        (Some(a), Some(b)) => a == b,
        _ => a.1 == b.1,
    }
}

/// A key as a mark's letter: `a` to `z`, capitals as their letter.
pub fn letter(c: char) -> Option<char> {
    c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase())
}

/// A name as it is kept: one line, trimmed, at most `MAX_NAME` characters.
fn clean(name: &str) -> String {
    let line: String = name.chars().filter(|c| !c.is_control()).collect();
    line.trim().chars().take(MAX_NAME).collect::<String>().trim_end().to_string()
}

/// The last part of a path, e.g. "notes" for "/Users/me/notes".
fn directory_name(path: &str) -> Option<&str> {
    path.trim_end_matches('/').rsplit('/').next().filter(|name| !name.is_empty())
}

/// Text from the file that is short enough and one line: it is drawn, and looked up on disk.
fn plain(text: &str, most: usize) -> bool {
    text.chars().count() <= most && !text.chars().any(char::is_control)
}

impl PaneRecord {
    fn needs(&self) -> Option<Need> {
        match &self.kind {
            RecordKind::Terminal => self.cwd.clone().map(Need::Dir),
            RecordKind::App { bundle, name, .. } => Some(Need::App { bundle: bundle.clone(), name: name.clone() }),
            RecordKind::Empty => None,
        }
    }

    /// The name a mark of this pane gets: the app, or the terminal's directory.
    fn default_name(&self) -> String {
        match &self.kind {
            RecordKind::App { name, .. } => name.clone(),
            _ => self.cwd.as_deref().and_then(directory_name).unwrap_or("shell").to_string(),
        }
    }

    /// What is wrong with a record read from disk, if anything.
    fn unsound(&self) -> Option<&'static str> {
        match &self.cwd {
            Some(cwd) if !cwd.starts_with('/') => return Some("its directory is not an absolute path"),
            Some(cwd) if !plain(cwd, 1024) => return Some("its directory is too long or not one line"),
            _ => {}
        }
        match &self.kind {
            RecordKind::App { name, .. } if name.is_empty() => Some("its app has no name"),
            RecordKind::App { bundle, name, title, .. } => {
                let fine = plain(name, 256) && plain(title, 512) && bundle.as_deref().is_none_or(|b| plain(b, 256));
                (!fine).then_some("its app or title is too long or not one line")
            }
            _ => None,
        }
    }

    /// Takes a `command` out of a record read from disk. True if there was one.
    fn drop_command(&mut self) -> bool {
        self.extra.remove("command").is_some()
    }
}

impl TabRecord {
    /// The recorded panes the layout shows, in reading order.
    fn shown(&self) -> Vec<&PaneRecord> {
        let leaves = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
        leaves.iter().filter_map(|id| self.panes.iter().find(|pane| pane.id == *id)).collect()
    }

    /// What is wrong with a record read from disk, if anything.
    fn unsound(&self) -> Option<&'static str> {
        let leaves = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let mut seen = leaves.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != leaves.len() {
            return Some("its layout shows a pane twice");
        }
        let listed = |id: &u32| self.panes.iter().filter(|pane| pane.id == *id).count() == 1;
        if !leaves.iter().all(listed) {
            return Some("its layout names a pane it does not list once");
        }
        if leaves.len() > 64 || !plain(&self.title, 256) {
            return Some("its title is too long or not one line, or it has too many panes");
        }
        self.panes.iter().find_map(PaneRecord::unsound)
    }
}

/// A recorded tree with new pane ids (`ids`: old → new), and ratios a hand-written file got wrong
/// brought back in range.
fn renumber(tree: &Node, ids: &[(u32, PaneId)]) -> Option<Node> {
    Some(match tree {
        Node::Leaf(old) => Node::Leaf(ids.iter().find(|(from, _)| from == old)?.1),
        Node::Split { axis, ratio, a, b } => {
            let ratio = if ratio.is_finite() { ratio.clamp(MIN_SHARE, 1.0 - MIN_SHARE) } else { 0.5 };
            Node::Split { axis: *axis, ratio, a: Box::new(renumber(a, ids)?), b: Box::new(renumber(b, ids)?) }
        }
    })
}

impl Mark {
    pub fn kind(&self) -> &'static str {
        match &self.target {
            Target::Tab(_) => "tab",
            Target::Pane(PaneRecord { kind: RecordKind::App { .. }, .. }) => "app",
            Target::Pane(_) => "terminal",
        }
    }

    /// The records it opens: its pane, or the panes its tab shows.
    fn records(&self) -> Vec<&PaneRecord> {
        match &self.target {
            Target::Pane(pane) => vec![pane],
            Target::Tab(tab) => tab.shown(),
        }
    }

    /// Everything it is opened with: a tab's is what each of its panes needs.
    pub fn needs(&self) -> Vec<Need> {
        self.records().into_iter().filter_map(PaneRecord::needs).collect()
    }

    fn unsound(&self) -> Option<&'static str> {
        match &self.target {
            Target::Pane(pane) => pane.unsound(),
            Target::Tab(tab) => tab.unsound(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    marks: Vec<Mark>,
    #[serde(flatten)]
    extra: Extra,
}

/// What `marks.json` held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loaded {
    pub marks: Vec<Mark>,
    /// Keys beside `version` and `marks`, to write back.
    pub extra: Extra,
    /// A line for each mark left out because its record is unsound. With any, the file must be
    /// left as it is: writing would drop them.
    pub skipped: Vec<String>,
    /// A record held a `command`. None is ever typed: say so once.
    pub commands: bool,
}

/// Reads `marks.json`. An error names what is wrong with the whole file; it is then left as it is.
pub fn parse(text: &str) -> Result<Loaded, String> {
    let file: File = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if file.version != VERSION {
        return Err(format!("version {} is not one this build reads", file.version));
    }
    let mut loaded = Loaded { extra: file.extra, ..Loaded::default() };
    for mut mark in file.marks {
        loaded.commands |= match &mut mark.target {
            Target::Pane(pane) => pane.drop_command(),
            // Every pane's, so no `any`: that stops at the first.
            Target::Tab(tab) => tab.panes.iter_mut().filter_map(|pane| pane.drop_command().then_some(())).count() > 0,
        };
        // A hand-written mark has no id: its name stands in, the same at every read, so two
        // running ottospaces mean the same mark by it.
        if mark.uid.is_empty() {
            let named = format!("name:{}", clean(&mark.name));
            let taken = |uid: &String| loaded.marks.iter().any(|other| other.uid == *uid);
            let numbered = (2..).map(|n| format!("{named}#{n}"));
            mark.uid = std::iter::once(named.clone()).chain(numbered).find(|uid| !taken(uid)).unwrap_or(named);
        }
        match mark.unsound() {
            Some(wrong) => loaded.skipped.push(format!("mark {:?}: {wrong}", clean(&mark.name))),
            None => loaded.marks.push(mark),
        }
    }
    Ok(loaded)
}

/// The text of `marks.json`.
pub fn write(marks: &[Mark], extra: &Extra) -> String {
    let file = File { version: VERSION, marks: marks.to_vec(), extra: extra.clone() };
    serde_json::to_string_pretty(&file).unwrap_or_default() + "\n"
}

/// The marks of two running ottospaces as one list. `base`: the file as this one last read or
/// wrote it. `ours`: this one's marks now. `theirs`: the file now, which another has written.
/// By `uid`: what either added is there, what either removed is gone, and a mark this one
/// changed is this one's. The order is the file's unless this one changed it.
pub fn merge(base: &[Mark], ours: &[Mark], theirs: &[Mark]) -> Vec<Mark> {
    let find = |list: &[Mark], uid: &str| list.iter().find(|mark| mark.uid == uid).cloned();
    let same = |a: &Mark, b: &Mark| (&a.name, a.key, &a.target, &a.extra) == (&b.name, b.key, &b.target, &b.extra);
    let uids = |list: &[Mark]| list.iter().map(|mark| mark.uid.clone()).collect::<Vec<_>>();
    // Theirs, less what this one removed, each as this one has it if this one changed it.
    let mut merged: Vec<Mark> = Vec::new();
    for mark in theirs {
        match (find(base, &mark.uid), find(ours, &mark.uid)) {
            (Some(_), None) => {}
            (Some(was), Some(mine)) if !same(&was, &mine) => merged.push(mine),
            (_, Some(mine)) => merged.push(Mark { live: mine.live, id: mine.id, ..mark.clone() }),
            (None, None) => merged.push(mark.clone()),
        }
    }
    // What this one added. A mark of this one's that is gone from the file was removed there.
    let added = ours.iter().filter(|mark| find(base, &mark.uid).is_none() && find(theirs, &mark.uid).is_none());
    merged.extend(added.cloned());
    // This one's order, if this one moved a mark: the others' new marks after it.
    let both = |uid: &String| [base, ours, &merged].iter().all(|list| find(list, uid).is_some());
    let kept = |list: &[Mark]| uids(list).into_iter().filter(both).collect::<Vec<_>>();
    if kept(ours) != kept(base) {
        let place = |mark: &Mark| ours.iter().position(|mine| mine.uid == mark.uid).unwrap_or(usize::MAX);
        merged.sort_by_key(place);
    }
    merged
}

/// The marks, in the user's order.
#[derive(Debug, Default)]
pub struct Marks {
    pub list: Vec<Mark>,
    next: MarkId,
    /// What the shell last found missing on this machine.
    missing: Vec<Need>,
    /// The last mark removed and its place, to put it back.
    removed: Option<(usize, Mark)>,
    /// Panes of rebuilt tabs that are still empty.
    unfilled: Vec<Unfilled>,
    /// The file's own keys this build does not know.
    pub extra: Extra,
}

impl Marks {
    pub fn get(&self, id: MarkId) -> Option<&Mark> {
        self.list.iter().find(|mark| mark.id == id)
    }

    fn get_mut(&mut self, id: MarkId) -> Option<&mut Mark> {
        self.list.iter_mut().find(|mark| mark.id == id)
    }

    pub fn find(&self, which: &MarkRef) -> Option<MarkId> {
        let found = match which {
            MarkRef::Id(id) => self.get(*id),
            MarkRef::Name(name) => {
                let exact = self.list.iter().find(|mark| mark.name == *name);
                exact.or_else(|| self.list.iter().find(|mark| mark.name.to_lowercase() == name.to_lowercase()))
            }
            MarkRef::Key(key) => letter(*key).and_then(|key| self.list.iter().find(|mark| mark.key == Some(key))),
            MarkRef::Place(place) => self.list.get(*place),
        };
        found.map(|mark| mark.id)
    }

    /// Replaces the list: with the marks kept on disk at start, or with this run's merged with
    /// another's. A mark that was here (by `uid`) keeps this run's id for it and what it stood for.
    pub fn restore(&mut self, marks: Vec<Mark>) {
        let before = std::mem::take(&mut self.list);
        for mut mark in marks {
            let was = before.iter().find(|old| !mark.uid.is_empty() && old.uid == mark.uid);
            mark.live = mark.live.or(was.and_then(|old| old.live));
            mark.id = was.map_or(0, |old| old.id);
            self.add(mark);
        }
    }

    /// `name`, or `name 2`, `name 3`: no two marks share a name, so a name finds one mark.
    fn unique_name(&self, name: &str, but: Option<MarkId>) -> String {
        let name = Some(clean(name)).filter(|name| !name.is_empty()).unwrap_or_else(|| "mark".into());
        let taken = |name: &str| self.list.iter().any(|mark| Some(mark.id) != but && mark.name == name);
        let numbered = (2..).map(|n| format!("{name} {n}"));
        std::iter::once(name.clone()).chain(numbered).find(|name| !taken(name)).unwrap_or(name)
    }

    /// Adds a mark at the end, with a name no other has. Its letter moves to it. `id` 0: a new id.
    pub fn add(&mut self, mut mark: Mark) -> MarkId {
        if mark.id == 0 {
            self.next += 1;
            mark.id = self.next;
        }
        let id = mark.id;
        mark.name = self.unique_name(&mark.name, None);
        let key = mark.key.take().and_then(letter);
        self.list.push(mark);
        self.set_key(id, key);
        id
    }

    /// Takes a mark off the list. It is kept, with its place, until the next one is removed.
    pub fn remove(&mut self, id: MarkId) {
        if let Some(at) = self.list.iter().position(|mark| mark.id == id) {
            self.removed = Some((at, self.list.remove(at)));
        }
    }

    /// The mark `undo_remove` would put back.
    pub fn removed(&self) -> Option<&Mark> {
        self.removed.as_ref().map(|(_, mark)| mark)
    }

    /// Puts the last removed mark back where it was. A letter given away since stays given.
    pub fn undo_remove(&mut self) {
        let Some((at, mut mark)) = self.removed.take() else { return };
        mark.name = self.unique_name(&mark.name, None);
        mark.key = mark.key.filter(|key| self.list.iter().all(|other| other.key != Some(*key)));
        self.list.insert(at.min(self.list.len()), mark);
    }

    /// A name that is empty once cleaned keeps the old one.
    pub fn rename(&mut self, id: MarkId, name: &str) {
        if clean(name).is_empty() {
            return;
        }
        let name = self.unique_name(name, Some(id));
        if let Some(mark) = self.get_mut(id) {
            mark.name = name;
        }
    }

    /// Gives a mark its letter, or none. A letter another mark has moves: one letter, one mark.
    pub fn set_key(&mut self, id: MarkId, key: Option<char>) {
        let key = key.and_then(letter);
        if self.get(id).is_none() {
            return;
        }
        for mark in &mut self.list {
            if mark.id == id {
                mark.key = key;
            } else if key.is_some() && mark.key == key {
                mark.key = None;
            }
        }
    }

    /// Moves a mark down the list (positive) or up. It stops at the ends.
    pub fn shift(&mut self, id: MarkId, delta: i32) {
        let Some(at) = self.list.iter().position(|mark| mark.id == id) else { return };
        let to = (at as i64 + i64::from(delta)).clamp(0, self.list.len() as i64 - 1) as usize;
        let mark = self.list.remove(at);
        self.list.insert(to, mark);
    }

    /// A closed tab is no mark's any more: a later tab may get its id.
    pub(crate) fn forget_tab(&mut self, tab: u32) {
        let marks = self.list.iter_mut().filter(|mark| matches!(mark.target, Target::Tab(_)));
        marks.filter(|mark| mark.live == Some(tab)).for_each(|mark| mark.live = None);
    }

    fn bind(&mut self, id: MarkId, live: u32) {
        if let Some(mark) = self.get_mut(id) {
            mark.live = Some(live);
        }
    }

    /// Everything the marks need, each once: what the shell looks for on this machine.
    pub fn needs(&self) -> Vec<Need> {
        let mut needs: Vec<Need> = Vec::new();
        for need in self.list.iter().flat_map(Mark::needs) {
            if !needs.contains(&need) {
                needs.push(need);
            }
        }
        needs
    }

    pub fn missing(&self) -> &[Need] {
        &self.missing
    }

    pub fn set_missing(&mut self, missing: Vec<Need>) {
        self.missing = missing;
    }

    /// Why a record cannot be opened: what it needs is missing.
    fn gone(&self, record: &PaneRecord) -> Option<String> {
        record.needs().filter(|need| self.missing.contains(need)).map(|need| need.gone())
    }

    /// Why nothing of a mark can be opened: its pane's need is missing, or every pane of its
    /// tab has one that is. A tab with one pane that can open is built with the others empty.
    pub fn unavailable(&self, mark: &Mark) -> Option<String> {
        let records = mark.records();
        let gone: Vec<String> = records.iter().filter_map(|record| self.gone(record)).collect();
        gone.first().filter(|_| gone.len() == records.len()).cloned()
    }

    /// What of a closed tab's mark would stay empty, e.g. "1 of 2 panes cannot open: ..".
    fn partly_gone(&self, mark: &Mark) -> Option<String> {
        let records = mark.records();
        let gone: Vec<String> = records.iter().filter_map(|record| self.gone(record)).collect();
        let first = gone.first().filter(|_| gone.len() < records.len())?;
        Some(format!("{} of {} panes cannot open: {first}", gone.len(), records.len()))
    }

    /// The panes of rebuilt tabs that are still empty.
    pub fn unfilled(&self) -> &[Unfilled] {
        &self.unfilled
    }
}

impl Model {
    /// The open pane a pane mark stands for: the one it stood for last, while that is still a
    /// terminal or a window of the same app; else an app pane of the same app and title (the
    /// window was closed and hosted again under a new id).
    pub fn mark_pane(&self, mark: &Mark) -> Option<PaneId> {
        let Target::Pane(record) = &mark.target else { return None };
        let same = |pane: &&Pane| match (&record.kind, &pane.kind) {
            (RecordKind::Terminal, Kind::Terminal) => true,
            (RecordKind::App { bundle, name, .. }, Kind::App { bundle: b, name: n, .. }) => {
                same_app((bundle.as_deref(), name), (b.as_deref(), n))
            }
            _ => false,
        };
        let titled = |pane: &&Pane| match (&record.kind, &pane.kind) {
            (RecordKind::App { title, .. }, Kind::App { title: t, .. }) => same(pane) && title == t,
            _ => false,
        };
        let live = mark.live.and_then(|id| self.pane(id)).filter(same);
        live.or_else(|| self.panes.iter().find(titled)).map(|pane| pane.id)
    }

    /// The index of the open tab a tab mark stands for.
    fn mark_tab(&self, mark: &Mark) -> Option<usize> {
        let Target::Tab(_) = &mark.target else { return None };
        self.tabs.iter().position(|tab| Some(tab.id) == mark.live)
    }

    /// The marks as the bar lists them.
    pub fn mark_rows(&self) -> Vec<MarkRow> {
        let tab_panes = self.tab_panes();
        let row = |mark: &Mark| {
            let pane = self.mark_pane(mark);
            let index = self.mark_tab(mark);
            let tab = index.and_then(|index| self.tabs.get(index)).map(|tab| tab.id);
            let open = pane.is_some() || tab.is_some();
            // The panes of its open tab that are still empty.
            let held = index.and_then(|index| tab_panes.get(index)).cloned().unwrap_or_default();
            let unfilled: Vec<Unfilled> =
                self.marks.unfilled.iter().filter(|empty| held.contains(&empty.pane)).cloned().collect();
            // What is open needs nothing more.
            let gone = self.marks.unavailable(mark).filter(|_| !open);
            let partly = if open {
                unfilled.first().map(|empty| format!("{} of {} panes empty: {}", unfilled.len(), held.len(), empty.why))
            } else {
                self.marks.partly_gone(mark)
            };
            let (id, name, key, kind) = (mark.id, mark.name.clone(), mark.key, mark.kind());
            let (available, reason) = (gone.is_none(), gone.or(partly));
            MarkRow { id, name, key, kind, open, pane, tab, available, reason, unfilled }
        };
        self.marks.list.iter().map(row).collect()
    }

    /// The mark of the selected row, in list mode.
    pub(crate) fn selected_mark(&self) -> Option<MarkId> {
        match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
            Some(ListRow::Mark(id)) => Some(id),
            _ => None,
        }
    }

    fn record(&self, pane: &Pane, cwds: &[(PaneId, String)]) -> PaneRecord {
        let kind = match &pane.kind {
            Kind::Empty => RecordKind::Empty,
            Kind::Terminal => RecordKind::Terminal,
            Kind::App { title, bundle, name, .. } => {
                let (bundle, name, title) = (bundle.clone(), name.clone(), title.clone());
                RecordKind::App { bundle, name, title, extra: Extra::new() }
            }
        };
        // Where the shell is now, as the shell crate read it; else where it started.
        let now = cwds.iter().find(|(id, _)| *id == pane.id).map(|(_, cwd)| cwd.clone());
        let cwd = now.or_else(|| pane.cwd.clone()).filter(|_| pane.kind == Kind::Terminal);
        PaneRecord { id: pane.id, kind, cwd, extra: Extra::new() }
    }

    /// What `of` names, as a mark's target, default name and live id. The error says why it
    /// cannot be marked.
    fn mark_target(&self, of: MarkOf, cwds: &[(PaneId, String)]) -> Result<(Target, String, u32), String> {
        match of {
            MarkOf::Pane(pane) => {
                let id = pane.or(self.focused).ok_or("no pane to mark")?;
                let pane = self.pane(id).ok_or_else(|| format!("no pane {id}"))?;
                if pane.kind == Kind::Empty {
                    return Err("an empty pane has nothing to mark".into());
                }
                let record = self.record(pane, cwds);
                let name = record.default_name();
                Ok((Target::Pane(record), name, id))
            }
            MarkOf::Tab(tab) => {
                let index = match tab {
                    Some(id) => self.tabs.iter().position(|tab| tab.id == id).ok_or_else(|| format!("no tab {id}"))?,
                    None => self.active,
                };
                let tab = self.tabs.get(index).ok_or("no tab to mark")?;
                let (layout, focused) =
                    if index == self.active { (&self.layout, self.focused) } else { (&tab.layout, tab.focused) };
                let leaves = layout.as_ref().map(Node::leaves).unwrap_or_default();
                let panes: Vec<PaneRecord> =
                    leaves.iter().filter_map(|id| self.pane(*id)).map(|pane| self.record(pane, cwds)).collect();
                if panes.iter().all(|pane| pane.kind == RecordKind::Empty) {
                    return Err("a tab with nothing in it has nothing to mark".into());
                }
                let (title, task, layout) = (tab.title.clone(), tab.task, layout.clone());
                let record = TabRecord { title: title.clone(), task, focused, layout, panes, extra: Extra::new() };
                Ok((Target::Tab(record), title, tab.id))
            }
        }
    }

    /// Why `of` cannot be marked, for the shell to tell a socket client.
    pub fn mark_refusal(&self, of: MarkOf) -> Option<String> {
        self.mark_target(of, &[]).err()
    }

    /// The mark that stands for `of` now, if it has one.
    pub fn mark_of(&self, of: MarkOf) -> Option<MarkId> {
        let found = match of {
            MarkOf::Pane(pane) => {
                let id = pane.or(self.focused)?;
                self.marks.list.iter().find(|mark| self.mark_pane(mark) == Some(id))
            }
            MarkOf::Tab(tab) => {
                let id = tab.or_else(|| self.tabs.get(self.active).map(|tab| tab.id))?;
                self.marks.list.iter().find(|mark| matches!(mark.target, Target::Tab(_)) && mark.live == Some(id))
            }
        };
        found.map(|mark| mark.id)
    }

    /// Marks a pane or a tab. One that is marked already keeps its mark, which takes the name
    /// and letter given. `uid`: the new mark's name for good, made up by the shell.
    pub(crate) fn mark_add(
        &mut self,
        of: MarkOf,
        name: Option<String>,
        key: Option<char>,
        uid: String,
        cwds: &[(PaneId, String)],
    ) {
        let id = match self.mark_of(of) {
            Some(id) => {
                if let Some(name) = &name {
                    self.marks.rename(id, name);
                }
                id
            }
            None => {
                let Ok((target, default, live)) = self.mark_target(of, cwds) else { return };
                let name = name.unwrap_or(default);
                self.marks.add(Mark { id: 0, uid, name, key: None, target, live: Some(live), extra: Extra::new() })
            }
        };
        if key.is_some() {
            self.marks.set_key(id, key);
        }
    }

    /// Takes a mark off, and offers for a few seconds to put it back.
    pub(crate) fn mark_remove(&mut self, id: MarkId) {
        if self.marks.get(id).is_none() {
            return;
        }
        self.marks.remove(id);
        self.undo_offer = Some(Notice { moved: Moved::MarkRemoved, until: self.now.saturating_add(NOTICE_MS) });
        if matches!(&self.mark_input, Some(MarkInput::Rename { mark, .. } | MarkInput::SetKey { mark }) if *mark == id)
        {
            self.mark_input = None;
        }
    }

    /// Puts the last removed mark back.
    pub(crate) fn mark_undo(&mut self) {
        self.marks.undo_remove();
        self.undo_offer = self.undo_offer.filter(|offer| offer.moved != Moved::MarkRemoved);
    }

    /// Marks what is not marked, and takes the mark off what is. Marking again what was just
    /// unmarked puts that mark back, with its name and letter: a second ⌘B by mistake loses nothing.
    pub(crate) fn mark_toggle(&mut self, of: MarkOf, uid: String, cwds: &[(PaneId, String)]) {
        if let Some(id) = self.mark_of(of) {
            return self.mark_remove(id);
        }
        let Ok((target, _, live)) = self.mark_target(of, cwds) else { return };
        let same_kind = |mark: &Mark| matches!(mark.target, Target::Tab(_)) == matches!(target, Target::Tab(_));
        let offered = self.undo_offer.is_some_and(|offer| offer.moved == Moved::MarkRemoved);
        if offered && self.marks.removed().is_some_and(|mark| mark.live == Some(live) && same_kind(mark)) {
            return self.mark_undo();
        }
        self.mark_add(of, None, None, uid, cwds);
    }

    /// A terminal in `cwd`: it fills the focused pane if that is empty, else splits it along
    /// `axis`. Returns its pane.
    pub(crate) fn open_terminal_at(&mut self, axis: Axis, cwd: Option<String>, effects: &mut Vec<Effect>) -> PaneId {
        let pane = self.open(Kind::Terminal, axis, None, effects);
        if let Some(opened) = self.panes.iter_mut().find(|p| p.id == pane) {
            opened.cwd = cwd;
        }
        pane
    }

    /// Goes to a mark. Open now: its tab comes forward and its pane takes focus (an app pane is
    /// focused, not entered); panes of its tab that are still empty are tried again. Else it is
    /// opened: a terminal in the recorded directory, an app's window (the shell hosts it), or
    /// the tab with its splits.
    pub(crate) fn mark_go(&mut self, id: MarkId, axis: Axis, effects: &mut Vec<Effect>) {
        let Some(mark) = self.marks.get(id).cloned() else { return };
        if let Some(pane) = self.mark_pane(&mark) {
            self.list = None;
            self.focus(pane, effects);
            self.marks.bind(id, pane);
            return self.refresh_title(id, pane);
        }
        if let Some(index) = self.mark_tab(&mark) {
            self.list = None;
            self.select_tab(index, effects);
            return self.retry_unfilled(effects);
        }
        if self.marks.unavailable(&mark).is_some() {
            return;
        }
        match &mark.target {
            Target::Pane(PaneRecord { kind: RecordKind::Terminal, cwd, .. }) => {
                let pane = self.open_terminal_at(axis, cwd.clone(), effects);
                self.marks.bind(id, pane);
            }
            Target::Pane(PaneRecord { kind: RecordKind::App { bundle, name, title, .. }, .. }) => {
                let (bundle, name, title) = (bundle.clone(), name.clone(), title.clone());
                effects.push(Effect::HostWindow { mark: Some(id), bundle, name, title, into: None, axis });
            }
            Target::Pane(_) => {}
            Target::Tab(record) => {
                let tab = self.rebuild_tab(record, effects);
                self.marks.bind(id, tab);
            }
        }
    }

    /// An open app mark takes its window's title as it is now: the hint for finding it again.
    fn refresh_title(&mut self, id: MarkId, pane: PaneId) {
        let Some(Kind::App { title: now, .. }) = self.pane(pane).map(|pane| pane.kind.clone()) else { return };
        if let Some(Mark { target: Target::Pane(PaneRecord { kind: RecordKind::App { title, .. }, .. }), .. }) =
            self.marks.get_mut(id)
        {
            *title = now;
        }
    }

    /// A new tab from a record, on show: its splits, a terminal in each recorded directory, and
    /// an empty pane for each app window, which the shell fills. A pane whose directory is gone
    /// or whose app is not installed stays empty and says so. Returns the tab's `id`.
    fn rebuild_tab(&mut self, record: &TabRecord, effects: &mut Vec<Effect>) -> u32 {
        self.list = None;
        self.leave(effects);
        self.remember_tab();
        self.park_active();
        // A task that is gone: a tab of no task.
        self.current_task = record.task.filter(|task| self.tasks.get(*task).is_some());
        self.push_tab();
        let mut ids = Vec::new();
        for pane in record.shown() {
            self.next_id += 1;
            let id = self.next_id;
            ids.push((pane.id, id));
            self.panes.push(Pane { id, kind: Kind::Empty, cwd: None, fit: None });
            if pane.kind != RecordKind::Empty {
                let why = "not opened yet".to_string();
                self.marks.unfilled.push(Unfilled { pane: id, record: pane.clone(), why });
            }
        }
        let new_id = |old: u32| ids.iter().find(|(from, _)| *from == old).map(|(_, to)| *to);
        self.layout = record.layout.as_ref().and_then(|tree| renumber(tree, &ids));
        let first = self.layout.as_ref().and_then(|tree| tree.leaves().first().copied());
        self.focused = record.focused.and_then(new_id).or(first);
        self.retry_unfilled(effects);
        let tab = &mut self.tabs[self.active];
        tab.title = record.title.clone();
        tab.id
    }

    /// Fills the empty panes of the tab on show that a mark's record names: a terminal starts
    /// if its directory is there; the shell is asked for each app's window, terminals first.
    fn retry_unfilled(&mut self, effects: &mut Vec<Effect>) {
        let shown = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let mut hosts = Vec::new();
        for empty in self.marks.unfilled.clone().into_iter().filter(|empty| shown.contains(&empty.pane)) {
            let (pane, gone) = (empty.pane, self.marks.gone(&empty.record));
            match (&empty.record.kind, gone) {
                (_, Some(why)) => self.unfilled(pane, why),
                (RecordKind::Terminal, None) => {
                    if let Some(opened) = self.panes.iter_mut().find(|p| p.id == pane) {
                        (opened.kind, opened.cwd) = (Kind::Terminal, empty.record.cwd.clone());
                        effects.push(Effect::Opened(pane));
                    }
                }
                (RecordKind::App { bundle, name, title, .. }, None) => {
                    let (bundle, name, title) = (bundle.clone(), name.clone(), title.clone());
                    hosts.push(Effect::HostWindow {
                        mark: None,
                        bundle,
                        name,
                        title,
                        into: Some(pane),
                        axis: Axis::Right,
                    });
                }
                (RecordKind::Empty, None) => {}
            }
        }
        effects.extend(hosts);
        self.prune_unfilled();
    }

    /// The shell could not fill `pane`: it says why.
    pub(crate) fn unfilled(&mut self, pane: PaneId, why: String) {
        if let Some(empty) = self.marks.unfilled.iter_mut().find(|empty| empty.pane == pane) {
            empty.why = why;
        }
    }

    /// A pane that was filled or closed is no longer waiting.
    pub(crate) fn prune_unfilled(&mut self) {
        let panes = &self.panes;
        self.marks.unfilled.retain(|empty| panes.iter().any(|p| p.id == empty.pane && p.kind == Kind::Empty));
    }

    /// The window the shell hosted for a pane mark is in `pane`.
    pub(crate) fn mark_bind(&mut self, id: MarkId, pane: PaneId) {
        if self.pane(pane).is_some() {
            self.marks.bind(id, pane);
        }
    }

    /// Each app pane's window title as it is now.
    pub(crate) fn retitle(&mut self, titles: Vec<(PaneId, String)>) {
        for (id, now) in titles {
            if let Some(Kind::App { title, .. }) = self.panes.iter_mut().find(|p| p.id == id).map(|p| &mut p.kind) {
                *title = now;
            }
        }
    }

    /// A mark may take the keyboard from normal and list mode only, as sneak mode does.
    fn mark_input_may_start(&self) -> bool {
        matches!(self.mode(), Mode::Normal | Mode::List | Mode::Mark) && !self.keys
    }

    /// The jump prefix: waits for a mark's letter. Pressed again, it stops waiting.
    pub(crate) fn mark_prefix(&mut self) {
        if self.mark_input == Some(MarkInput::Jump) {
            self.mark_input = None;
        } else if self.mark_input_may_start() {
            self.mark_input = Some(MarkInput::Jump);
        }
    }

    /// Starts typing a mark's name or its letter.
    pub(crate) fn mark_edit(&mut self, id: MarkId, field: Field) {
        let Some(mark) = self.marks.get(id).filter(|_| self.mark_input_may_start()) else { return };
        self.mark_input = Some(match field {
            Field::Name => MarkInput::Rename { mark: id, text: mark.name.clone() },
            Field::Key => MarkInput::SetKey { mark: id },
        });
    }

    /// Typed text, to whatever a mark waits for: the letter to jump to (a letter of no mark only
    /// stops the wait), a mark's new letter, or more of its name.
    pub(crate) fn mark_type(&mut self, typed: &str, effects: &mut Vec<Effect>) {
        let first = typed.chars().next();
        match self.mark_input.take() {
            Some(MarkInput::Jump) => {
                if let Some(id) = first.and_then(|key| self.marks.find(&MarkRef::Key(key))) {
                    self.mark_go(id, Axis::Right, effects);
                }
            }
            Some(MarkInput::SetKey { mark }) => match first.and_then(letter) {
                Some(key) => self.marks.set_key(mark, Some(key)),
                // Not a letter: it still waits for one.
                None => self.mark_input = Some(MarkInput::SetKey { mark }),
            },
            Some(MarkInput::Rename { mark, mut text }) => {
                let room = MAX_NAME.saturating_sub(text.chars().count());
                text.extend(typed.chars().filter(|c| !c.is_control()).take(room));
                self.mark_input = Some(MarkInput::Rename { mark, text });
            }
            None => {}
        }
    }

    /// ⌫: takes a character off the name, or the letter off the mark.
    pub(crate) fn mark_backspace(&mut self) {
        match &mut self.mark_input {
            Some(MarkInput::Rename { text, .. }) => drop(text.pop()),
            Some(MarkInput::SetKey { mark }) => {
                let mark = *mark;
                self.marks.set_key(mark, None);
                self.mark_input = None;
            }
            Some(MarkInput::Jump) | None => {}
        }
    }

    /// ↵: keeps the typed name.
    pub(crate) fn mark_submit(&mut self) {
        if let Some(MarkInput::Rename { mark, text }) = &self.mark_input {
            let (mark, text) = (*mark, text.clone());
            self.marks.rename(mark, &text);
            self.mark_input = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis::{Down, Right};
    use crate::model::{Action, Origin};
    use crate::tasks::State;

    fn slack(window: u32, title: &str) -> Kind {
        Kind::App {
            window,
            title: title.into(),
            bundle: Some("com.tinyspeck.slackmacgap".into()),
            name: "Slack".into(),
        }
    }

    /// A new name for good, as the shell makes one up.
    fn uid() -> String {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        format!("u{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    fn file(m: &Model) -> String {
        write(&m.marks.list, &m.marks.extra)
    }

    fn restore(m: &mut Model, loaded: Loaded) {
        m.update(Action::RestoreMarks { marks: loaded.marks, extra: loaded.extra }.sent_by(Origin::System));
    }

    fn open(m: &mut Model, kind: Kind) -> PaneId {
        m.update(Action::Open { kind, axis: Right, beside: None });
        m.focused.expect("a focused pane")
    }

    fn mark(m: &mut Model, of: MarkOf, cwds: &[(PaneId, &str)]) {
        let cwds = cwds.iter().map(|(id, cwd)| (*id, cwd.to_string())).collect();
        m.update(Action::MarkAdd { of, name: None, key: None, uid: uid(), cwds });
    }

    fn toggle(m: &mut Model, of: MarkOf) {
        m.update(Action::MarkToggle { of, uid: uid(), cwds: Vec::new() });
    }

    fn go(m: &mut Model, name: &str) -> Vec<Effect> {
        m.update(Action::MarkGo { mark: MarkRef::Name(name.into()), axis: Right })
    }

    fn names(m: &Model) -> Vec<&str> {
        m.marks.list.iter().map(|mark| mark.name.as_str()).collect()
    }

    fn row<'a>(rows: &'a [MarkRow], name: &str) -> &'a MarkRow {
        rows.iter().find(|row| row.name == name).expect(name)
    }

    #[test]
    fn a_mark_is_named_for_its_directory_app_or_tab_and_no_two_share_a_name() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[(shell, "/Users/me/notes/")]);
        let app = open(&mut m, slack(7, "general"));
        mark(&mut m, MarkOf::Pane(Some(app)), &[]);
        mark(&mut m, MarkOf::Tab(None), &[]);
        assert_eq!(names(&m), ["notes", "Slack", "Tab 1"]);
        assert_eq!(m.mark_rows().iter().map(|r| r.kind).collect::<Vec<_>>(), ["terminal", "app", "tab"]);
        // A second shell in the same directory, and one whose directory nobody read.
        let second = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(Some(second)), &[(second, "/tmp/notes")]);
        let third = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(Some(third)), &[]);
        assert_eq!(names(&m)[3..], ["notes 2", "shell"]);
        // What cannot be marked says why, and adds nothing.
        m.update(Action::NewTab);
        assert_eq!(m.mark_refusal(MarkOf::Tab(None)), Some("a tab with nothing in it has nothing to mark".into()));
        assert_eq!(m.mark_refusal(MarkOf::Pane(None)), Some("no pane to mark".into()));
        assert_eq!(m.mark_refusal(MarkOf::Pane(Some(99))), Some("no pane 99".into()));
        m.update(Action::Split { axis: Right, kind: Kind::Empty });
        assert_eq!(m.mark_refusal(MarkOf::Pane(None)), Some("an empty pane has nothing to mark".into()));
        mark(&mut m, MarkOf::Pane(None), &[]);
        mark(&mut m, MarkOf::Tab(None), &[]);
        assert_eq!(m.marks.list.len(), 5);
    }

    #[test]
    fn toggle_marks_and_unmarks_and_add_updates_the_mark_that_is_there() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        let toggle = |m: &mut Model, of| {
            m.update(Action::MarkToggle { of, uid: uid(), cwds: vec![(shell, "/a/notes".into())] });
        };
        toggle(&mut m, MarkOf::Pane(None));
        toggle(&mut m, MarkOf::Tab(None));
        assert_eq!(names(&m), ["notes", "Tab 1"]);
        // Marking it again by request renames it and gives it a letter: no second mark.
        let (name, key) = (Some("journal".into()), Some('J'));
        m.update(Action::MarkAdd { of: MarkOf::Pane(Some(shell)), name, key, uid: uid(), cwds: Vec::new() });
        assert_eq!((names(&m), m.marks.list[0].key), (vec!["journal", "Tab 1"], Some('j')));
        // The same key takes each mark off again.
        toggle(&mut m, MarkOf::Pane(None));
        assert_eq!(names(&m), ["Tab 1"]);
        toggle(&mut m, MarkOf::Tab(None));
        assert!(m.marks.list.is_empty());
    }

    #[test]
    fn a_letter_belongs_to_one_mark_and_a_taken_one_moves() {
        let mut m = Model::default();
        for (title, key) in [("general", 'g'), ("random", 'r')] {
            let pane = open(&mut m, slack(1, title));
            let of = MarkOf::Pane(Some(pane));
            m.update(Action::MarkAdd { of, name: Some(title.into()), key: Some(key), uid: uid(), cwds: Vec::new() });
        }
        let keys = |m: &Model| m.marks.list.iter().map(|mark| mark.key).collect::<Vec<_>>();
        assert_eq!(keys(&m), [Some('g'), Some('r')]);
        let (first, second) = (m.marks.list[0].id, m.marks.list[1].id);
        m.marks.set_key(second, Some('G'));
        assert_eq!(keys(&m), [None, Some('g')]);
        // Not a letter: no key. A mark that is not there changes nothing.
        m.marks.set_key(first, Some('1'));
        m.marks.set_key(99, Some('g'));
        assert_eq!(keys(&m), [None, Some('g')]);
        assert_eq!(m.marks.find(&MarkRef::Key('G')), Some(second));
        assert_eq!((m.marks.find(&MarkRef::Key('x')), m.marks.find(&MarkRef::Key('1'))), (None, None));
    }

    #[test]
    fn marks_keep_the_users_order_and_are_found_by_name_letter_or_place() {
        let mut m = Model::default();
        for title in ["a", "b", "c"] {
            let pane = open(&mut m, slack(1, title));
            let of = MarkOf::Pane(Some(pane));
            m.update(Action::MarkAdd { of, name: Some(title.into()), key: None, uid: uid(), cwds: Vec::new() });
        }
        let c = m.marks.find(&MarkRef::Name("C".into())).expect("by name, whatever the case");
        m.marks.shift(c, -1);
        assert_eq!(names(&m), ["a", "c", "b"]);
        m.marks.shift(c, -5);
        assert_eq!(names(&m), ["c", "a", "b"]);
        m.marks.shift(c, 9);
        assert_eq!(names(&m), ["a", "b", "c"]);
        assert_eq!(m.marks.find(&MarkRef::Place(2)), Some(c));
        assert_eq!((m.marks.find(&MarkRef::Place(3)), m.marks.find(&MarkRef::Name("d".into()))), (None, None));
        // A rename keeps names apart; an empty one keeps the old name.
        m.marks.rename(c, "a");
        m.marks.rename(c, " \n ");
        assert_eq!(names(&m), ["a", "b", "a 2"]);
        m.update(Action::MarkRemove { mark: MarkRef::Name("b".into()) });
        assert_eq!(names(&m), ["a", "a 2"]);
    }

    #[test]
    fn going_to_an_open_mark_brings_its_tab_forward_and_focuses_without_entering() {
        let mut m = Model::default();
        let app = open(&mut m, slack(7, "general"));
        mark(&mut m, MarkOf::Pane(Some(app)), &[]);
        m.update(Action::NewTab);
        open(&mut m, Kind::Terminal);
        assert_eq!(m.active, 1);
        let rows = m.mark_rows();
        assert_eq!((rows[0].open, rows[0].pane, rows[0].available), (true, Some(app), true));
        // Focused, not entered: the app does not get the keyboard.
        assert_eq!((go(&mut m, "Slack"), m.active, m.focused, m.live), (vec![], 0, Some(app), None));
        assert_eq!(m.panes.len(), 2, "nothing new opened");
    }

    #[test]
    fn a_closed_terminal_mark_opens_a_shell_in_its_directory_and_types_nothing() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[(shell, "/Users/me/notes")]);
        m.update(Action::Close { pane: None });
        assert!(!m.mark_rows()[0].open);
        let effects = go(&mut m, "notes");
        let pane = m.focused.expect("the new shell");
        assert_eq!(effects, [Effect::Opened(pane)], "a shell, and nothing typed into it");
        assert_eq!(m.pane(pane).map(|p| (&p.kind, p.cwd.as_deref())), Some((&Kind::Terminal, Some("/Users/me/notes"))));
        assert_eq!(m.mark_rows()[0].pane, Some(pane));
        // Again: it is open now, so no second shell.
        assert_eq!((go(&mut m, "notes"), m.panes.len()), (vec![], 1));
    }

    #[test]
    fn a_terminal_that_changed_directory_keeps_its_mark_and_the_mark_its_directory() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[(shell, "/work/app")]);
        // The shell has moved since (`cd /tmp`): the core is not told, and the mark is of the pane.
        assert_eq!(m.mark_of(MarkOf::Pane(Some(shell))), Some(m.marks.list[0].id));
        assert_eq!((go(&mut m, "app"), m.focused), (vec![], Some(shell)));
        // Once it is closed, the mark opens where it was made, not where the shell went.
        m.update(Action::Close { pane: Some(shell) });
        go(&mut m, "app");
        assert_eq!(m.focused.and_then(|id| m.pane(id)).and_then(|p| p.cwd.as_deref()), Some("/work/app"));
        // Another shell is not the marked one, whatever its directory.
        let other = open(&mut m, Kind::Terminal);
        assert_eq!(m.mark_of(MarkOf::Pane(Some(other))), None);
    }

    #[test]
    fn an_app_pane_closed_and_hosted_again_under_a_new_id_is_its_mark_by_app_and_title() {
        let mut m = Model::default();
        let general = open(&mut m, slack(7, "general"));
        let random = open(&mut m, slack(8, "random"));
        mark(&mut m, MarkOf::Pane(Some(random)), &[]);
        m.update(Action::Close { pane: Some(random) });
        // The other Slack window is not it.
        assert_eq!((m.mark_rows()[0].open, m.mark_of(MarkOf::Pane(Some(general)))), (false, None));
        let effects = go(&mut m, "Slack");
        let bundle = Some("com.tinyspeck.slackmacgap".to_string());
        let host = |mark| {
            let (bundle, name, title) = (bundle.clone(), "Slack".to_string(), "random".to_string());
            Effect::HostWindow { mark, bundle, name, title, into: None, axis: Right }
        };
        assert_eq!(effects, [host(Some(m.marks.list[0].id))], "the shell is asked for the window titled random");
        assert_eq!(m.panes.len(), 1, "no pane until the window is in hand");
        // The shell hosts it: a new window id and a new pane id.
        let again = open(&mut m, slack(31, "random"));
        assert_ne!(again, random);
        assert_eq!(m.mark_rows()[0].pane, Some(again));
        assert_eq!((go(&mut m, "Slack"), m.focused), (vec![], Some(again)));
        // Another app with that title is not it either.
        m.update(Action::Close { pane: Some(again) });
        let other =
            Kind::App { window: 40, title: "random".into(), bundle: Some("com.other".into()), name: "Slack".into() };
        open(&mut m, other);
        assert!(!m.mark_rows()[0].open);
    }

    #[test]
    fn a_window_hosted_by_its_fallback_is_bound_to_its_mark_whatever_its_title() {
        let mut m = Model::default();
        let pane = open(&mut m, slack(7, "general"));
        mark(&mut m, MarkOf::Pane(Some(pane)), &[]);
        let id = m.marks.list[0].id;
        m.update(Action::Close { pane: Some(pane) });
        // Slack's only window now shows another channel: the shell hosts it and says which pane.
        let hosted = open(&mut m, slack(9, "random"));
        assert!(!m.mark_rows()[0].open);
        m.update(Action::MarkBind { mark: id, pane: hosted }.sent_by(Origin::System));
        assert_eq!(m.mark_rows()[0].pane, Some(hosted));
        // Not to a pane that is gone, or of another app.
        m.update(Action::MarkBind { mark: id, pane: 99 }.sent_by(Origin::System));
        assert_eq!(m.mark_rows()[0].pane, Some(hosted));
        let shell = open(&mut m, Kind::Terminal);
        m.update(Action::MarkBind { mark: id, pane: shell }.sent_by(Origin::System));
        assert!(!m.mark_rows()[0].open);
    }

    fn window(id: u32, title: &str, main: bool, pane: Option<PaneId>) -> Window {
        Window { id, title: title.into(), main, pane }
    }

    #[test]
    fn of_two_slack_windows_the_titled_one_is_hosted_and_one_in_a_pane_is_never_taken() {
        let two = [window(1, "general", true, None), window(2, "random", false, None)];
        assert_eq!(pick_window("random", &two), Pick::Host(2));
        assert_eq!(pick_window("general", &two), Pick::Host(1));
        // No window has the title any more: the main one. With none main: none, not a guess.
        assert_eq!(pick_window("design", &two), Pick::Host(1));
        let neither = [window(1, "general", false, None), window(2, "random", false, None)];
        assert_eq!(pick_window("design", &neither), Pick::None);
        // The only window, whatever its title.
        assert_eq!(pick_window("design", &two[1..]), Pick::Host(2));
        // The titled window is in pane 5 already: the free one is not it, but it is the only
        // free one, so it is hosted. With none free: go to the pane.
        let held = [window(1, "general", true, Some(5)), window(2, "random", false, None)];
        assert_eq!(pick_window("general", &held), Pick::Host(2));
        assert_eq!(pick_window("general", &held[..1]), Pick::Focus(5));
        let both = [window(1, "general", true, Some(5)), window(2, "random", false, Some(6))];
        assert_eq!(pick_window("random", &both), Pick::Focus(6));
        assert_eq!(pick_window("design", &both), Pick::None);
        // Two untitled windows: an empty hint matches neither.
        assert_eq!(pick_window("", &[window(1, "", false, None), window(2, "", false, None)]), Pick::None);
        assert_eq!(pick_window("x", &[]), Pick::None);
    }

    /// Tab 1: a shell left of (Slack over a shell), 30% wide.
    fn nested(m: &mut Model) -> (PaneId, PaneId, PaneId) {
        let left = open(m, Kind::Terminal);
        let app = open(m, slack(7, "general"));
        m.update(Action::Split { axis: Down, kind: Kind::Terminal });
        let below = m.focused.expect("the lower shell");
        m.update(Action::Focus { pane: left });
        for _ in 0..4 {
            m.update(Action::Resize { dir: crate::Dir::Left });
        }
        (left, app, below)
    }

    #[test]
    fn a_tab_mark_with_nested_splits_goes_round_through_the_file_and_is_built_again() {
        let mut m = Model::default();
        let (left, app, below) = nested(&mut m);
        m.update(Action::MarkAdd {
            of: MarkOf::Tab(None),
            name: Some("work".into()),
            key: Some('w'),
            uid: uid(),
            cwds: vec![(left, "/work/app".into()), (below, "/work/notes".into())],
        });
        let before = serde_json::to_string(&m.layout).unwrap();
        let text = file(&m);
        let read = parse(&text).expect("what was written parses");
        assert_eq!(write(&read.marks, &read.extra), text, "and writes the same again");
        assert!(!text.contains("\"id\": 0") && !text.contains("live"), "{text}");
        assert_eq!((&read.skipped, read.commands), (&vec![], false));

        // A new run: no panes, the marks from the file.
        let mut m = Model::default();
        restore(&mut m, read);
        let rows = m.mark_rows();
        assert_eq!((rows[0].name.as_str(), rows[0].key, rows[0].kind, rows[0].open), ("work", Some('w'), "tab", false));
        let effects = m.update(Action::MarkGo { mark: MarkRef::Key('w'), axis: Right });
        // New ids, the same tree: the shells start, and the shell is asked for Slack's window.
        let ids: Vec<PaneId> = m.panes.iter().map(|p| p.id).collect();
        let [new_left, new_app, new_below] = ids[..] else { panic!("three panes: {ids:?}") };
        let host = Effect::HostWindow {
            mark: None,
            bundle: Some("com.tinyspeck.slackmacgap".into()),
            name: "Slack".into(),
            title: "general".into(),
            into: Some(new_app),
            axis: Right,
        };
        assert_eq!(effects, [Effect::Opened(new_left), Effect::Opened(new_below), host]);
        let renamed = before
            .replace(&format!("\"leaf\":{left}"), "\"leaf\":L")
            .replace(&format!("\"leaf\":{app}"), "\"leaf\":A")
            .replace(&format!("\"leaf\":{below}"), "\"leaf\":B")
            .replace("\"leaf\":L", &format!("\"leaf\":{new_left}"))
            .replace("\"leaf\":A", &format!("\"leaf\":{new_app}"))
            .replace("\"leaf\":B", &format!("\"leaf\":{new_below}"));
        assert_eq!(serde_json::to_string(&m.layout).unwrap(), renamed, "splits and ratios as they were");
        assert!(renamed.contains("\"ratio\":0.3"), "{renamed}");
        let cwd = |id| m.pane(id).and_then(|p| p.cwd.as_deref());
        assert_eq!((cwd(new_left), cwd(new_app), cwd(new_below)), (Some("/work/app"), None, Some("/work/notes")));
        assert_eq!(m.pane(new_app).map(|p| &p.kind), Some(&Kind::Empty), "empty until its window is hosted");
        assert_eq!((m.tabs[m.active].title.as_str(), m.focused), ("Tab 1", Some(new_left)));
        assert_eq!(m.tabs.len(), 2, "a new tab beside the one that was there");
        // The shell fills the pane, wherever the user is by then.
        m.update(Action::NewTab);
        let filled = m.update(Action::Fill { pane: new_app, kind: slack(9, "general") }.sent_by(Origin::System));
        assert_eq!((filled, m.active), (vec![Effect::Opened(new_app)], 2));
        assert_eq!(m.update(Action::Fill { pane: new_app, kind: slack(10, "x") }.sent_by(Origin::System)), []);
        // Open now: going again selects the tab, and builds nothing.
        let effects = m.update(Action::MarkGo { mark: MarkRef::Name("work".into()), axis: Right });
        assert_eq!((effects, m.active, m.panes.len()), (vec![], 1, 3));
        assert!(m.mark_rows()[0].open);
        // Closed: it is built again.
        m.update(Action::CloseTab { index: None });
        assert!(!m.mark_rows()[0].open);
        m.update(Action::MarkGo { mark: MarkRef::Name("work".into()), axis: Right });
        assert_eq!(m.panes.len(), 3);
    }

    #[test]
    fn a_mark_of_a_tasks_tab_goes_to_that_task_and_to_no_task_once_it_is_gone() {
        let mut m = Model::default();
        m.update(Action::TaskAdd {
            title: "fix zoom".into(),
            priority: 3,
            state: State::Running,
            tab: None,
            assign: Default::default(),
        });
        m.update(Action::TaskOpen { id: Some(1) });
        open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Tab(None), &[]);
        m.update(Action::TaskLeave);
        assert_eq!(m.current_task, None);
        // Open in the background: its task's tabs come with it.
        go(&mut m, "fix zoom");
        assert_eq!((m.current_task, m.panes.len()), (Some(1), 1));
        // Closed: the new tab is the task's again.
        m.update(Action::CloseTab { index: None });
        m.update(Action::TaskLeave);
        go(&mut m, "fix zoom");
        assert_eq!((m.current_task, m.tabs[m.active].task), (Some(1), Some(1)));
        // The task is gone from this run: a tab of no task.
        let kept = parse(&file(&m)).unwrap();
        let mut m = Model::default();
        restore(&mut m, kept);
        go(&mut m, "fix zoom");
        assert_eq!((m.current_task, m.tabs[m.active].task, m.panes.len()), (None, None, 1));
    }

    #[test]
    fn a_mark_whose_directory_or_app_is_missing_is_unavailable_with_the_reason_and_can_be_removed() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[(shell, "/gone/notes")]);
        let app = open(&mut m, slack(7, "general"));
        mark(&mut m, MarkOf::Pane(Some(app)), &[]);
        mark(&mut m, MarkOf::Tab(None), &[(shell, "/gone/notes")]);
        let bundle = Some("com.tinyspeck.slackmacgap".to_string());
        let needs = vec![Need::Dir("/gone/notes".into()), Need::App { bundle, name: "Slack".into() }];
        assert_eq!(m.marks.needs(), needs, "each need once");
        m.update(Action::MarksChecked { missing: needs }.sent_by(Origin::System));
        // What is open is where it is: nothing is missing yet.
        assert!(m.mark_rows().iter().all(|row| row.available && row.open));
        m.update(Action::CloseTab { index: None });
        let rows = m.mark_rows();
        let reason = |name: &str| row(&rows, name).reason.clone();
        assert_eq!(reason("notes"), Some("its directory is gone: /gone/notes".into()));
        assert_eq!(reason("Slack"), Some("Slack is not installed".into()));
        assert_eq!(reason("Tab 1"), Some("its directory is gone: /gone/notes".into()));
        assert!(rows.iter().all(|row| !row.available && !row.open));
        // Going to one does nothing: no shell somewhere else, no half a tab.
        for name in ["notes", "Slack", "Tab 1"] {
            assert_eq!((go(&mut m, name), m.panes.len()), (vec![], 0), "{name}");
        }
        m.update(Action::MarkRemove { mark: MarkRef::Name("notes".into()) });
        assert_eq!(names(&m), ["Slack", "Tab 1"]);
        // Found again (the app was installed): available.
        m.update(Action::MarksChecked { missing: vec![Need::Dir("/gone/notes".into())] }.sent_by(Origin::System));
        assert!(row(&m.mark_rows(), "Slack").available);
    }

    #[test]
    fn the_jump_prefix_waits_for_a_letter_and_goes_to_that_mark() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        let of = MarkOf::Pane(Some(shell));
        m.update(Action::MarkAdd { of, name: Some("notes".into()), key: Some('n'), uid: uid(), cwds: Vec::new() });
        m.update(Action::NewTab);
        m.update(Action::MarkPrefix);
        assert_eq!((m.mode(), m.keyboard_terminal()), (Mode::Mark, None));
        m.update(Action::MarkType { text: "N".into() });
        assert_eq!((m.mode(), m.active, m.focused), (Mode::Normal, 0, Some(shell)));
        // A letter of no mark only ends the wait. So does the prefix again, and Esc.
        for leave in [Action::MarkType { text: "z".into() }, Action::MarkPrefix, Action::MarkCancel] {
            m.update(Action::MarkPrefix);
            assert_eq!(m.mode(), Mode::Mark);
            assert_eq!((m.update(leave), m.mode(), m.focused), (vec![], Mode::Normal, Some(shell)));
        }
        // Out of the wait a letter is no jump.
        m.update(Action::NewTab);
        m.update(Action::MarkType { text: "n".into() });
        assert_eq!(m.active, 2);
        // It starts from normal and list mode only: not over the picker, the box or the key reference.
        m.update(Action::ToggleKeys);
        m.update(Action::MarkPrefix);
        assert_eq!(m.mark_input, None);
        m.update(Action::CloseKeys);
        m.update(Action::OpenCompose { text: String::new() });
        m.update(Action::MarkPrefix);
        assert_eq!(m.mode(), Mode::Compose);
    }

    #[test]
    fn the_wait_for_a_letter_ends_on_the_users_moves_and_never_on_a_clients_or_news() {
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[]);
        m.update(Action::MarkPrefix);
        // An agent's split and a tick run underneath: the user's next letter is still the jump's.
        for news in [Action::Split { axis: Right, kind: Kind::Empty }, Action::Focus { pane: shell }] {
            m.update(news.sent_by(Origin::Socket));
            assert_eq!(m.mode(), Mode::Mark);
        }
        m.update(Action::Tick { now: 1 }.sent_by(Origin::System));
        assert_eq!((m.mode(), m.panes.len()), (Mode::Mark, 2));
        // Nor may a client take the keyboard from under it.
        let picker = Action::OpenPicker { axis: Right, new_tab: false, items: Vec::new() };
        assert_eq!(m.refusal(&picker, Origin::Socket), Some(INPUT_ON));
        m.update(picker.clone().sent_by(Origin::Socket));
        assert_eq!(m.mode(), Mode::Mark);
        // A key the client presses is input to the wait, as the user's is.
        assert_eq!(m.refusal(&Action::MarkPrefix, Origin::Socket), None);
        m.update(Action::MarkType { text: "z".into() }.sent_by(Origin::Socket));
        assert_eq!(m.mode(), Mode::Normal);
        // The user's own move ends the wait, and happens.
        m.update(Action::MarkPrefix);
        m.update(picker);
        assert_eq!((m.mode(), m.mark_input.is_none()), (Mode::Pick, true));
    }

    #[test]
    fn a_marks_name_and_letter_are_typed_in_its_row() {
        let mut m = Model::default();
        open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[]);
        mark(&mut m, MarkOf::Tab(None), &[]);
        let (shell, tab) = (m.marks.list[0].id, m.marks.list[1].id);
        m.update(Action::MarkEdit { mark: MarkRef::Id(shell), field: Field::Name });
        assert_eq!(m.mark_input, Some(MarkInput::Rename { mark: shell, text: "shell".into() }));
        for _ in 0..5 {
            m.update(Action::MarkBackspace);
        }
        m.update(Action::MarkType { text: "no\ntes".into() });
        assert_eq!(m.mark_input, Some(MarkInput::Rename { mark: shell, text: "notes".into() }));
        m.update(Action::MarkSubmit);
        assert_eq!((names(&m), m.mode()), (vec!["notes", "Tab 1"], Mode::Normal));
        // No longer than a name may be. Esc keeps the old name.
        m.update(Action::MarkEdit { mark: MarkRef::Id(shell), field: Field::Name });
        m.update(Action::MarkType { text: "x".repeat(100) });
        let Some(MarkInput::Rename { text, .. }) = &m.mark_input else { panic!("renaming") };
        assert_eq!(text.chars().count(), MAX_NAME);
        m.update(Action::MarkCancel);
        assert_eq!(names(&m), ["notes", "Tab 1"]);
        // The letter: the first one typed. A digit is not one, and the row goes on waiting.
        m.update(Action::MarkEdit { mark: MarkRef::Id(tab), field: Field::Key });
        m.update(Action::MarkType { text: "1".into() });
        assert_eq!(m.mark_input, Some(MarkInput::SetKey { mark: tab }));
        m.update(Action::MarkType { text: "T".into() });
        assert_eq!((m.marks.list[1].key, m.mode()), (Some('t'), Mode::Normal));
        // ⌫ takes the letter off.
        m.update(Action::MarkEdit { mark: MarkRef::Id(tab), field: Field::Key });
        m.update(Action::MarkBackspace);
        assert_eq!((m.marks.list[1].key, m.mode()), (None, Mode::Normal));
        // A mark that is gone has nothing to type.
        m.update(Action::MarkEdit { mark: MarkRef::Id(99), field: Field::Name });
        assert_eq!(m.mark_input, None);
    }

    #[test]
    fn list_mode_goes_to_moves_renames_and_removes_the_selected_mark() {
        let mut m = Model::default();
        for title in ["a", "b", "c"] {
            let pane = open(&mut m, slack(1, title));
            let of = MarkOf::Pane(Some(pane));
            m.update(Action::MarkAdd { of, name: Some(title.into()), key: None, uid: uid(), cwds: Vec::new() });
        }
        let a = m.panes[0].id;
        m.update(Action::ToggleSection { section: crate::Section::Marks });
        m.update(Action::FocusList);
        assert_eq!((m.mode(), m.list, m.list_rows().len()), (Mode::List, Some(0), 3));
        // ⇧J moves the mark down, and the selection goes with it.
        m.update(Action::ListReorder { delta: 1 });
        assert_eq!((names(&m), m.list), (vec!["b", "a", "c"], Some(1)));
        m.update(Action::ListMarkEdit { field: Field::Name });
        assert_eq!(m.mode(), Mode::Mark);
        m.update(Action::MarkType { text: "!".into() });
        m.update(Action::MarkSubmit);
        assert_eq!((names(&m), m.mode(), m.list), (vec!["b", "a!", "c"], Mode::List, Some(1)));
        // ↵ goes to it: the pane has the keyboard, not the list.
        assert_eq!(
            (m.update(Action::ListChoose { axis: Right }), m.mode(), m.focused),
            (vec![], Mode::Normal, Some(a))
        );
        // x removes the selected mark; the next row comes under the selection.
        m.update(Action::FocusList);
        m.update(Action::ListDismiss);
        assert_eq!((names(&m), m.list), (vec!["a!", "c"], Some(0)));
        // On a row that is no mark, ⇧J only moves the selection.
        m.update(Action::ToggleSection { section: crate::Section::Apps });
        m.update(Action::SetApps { names: vec!["x".into(), "y".into()] });
        m.update(Action::ListReorder { delta: 1 });
        m.update(Action::ListMarkEdit { field: Field::Name });
        assert_eq!((m.list, m.mode()), (Some(1), Mode::List));
    }

    #[test]
    fn a_file_that_is_wrong_is_refused_whole_and_a_hand_written_one_takes_defaults() {
        let read = parse(r#"{"version":1,"marks":[{"name":"notes","target":{"pane":{"kind":{"type":"terminal"}}}}]}"#);
        let marks = read.expect("the least a mark needs").marks;
        assert_eq!((marks[0].key, marks[0].live, marks[0].uid.as_str()), (None, None, "name:notes"));
        assert_eq!(parse(r#"{"version":1}"#), Ok(Loaded::default()));
        assert!(parse("{").is_err());
        assert_eq!(parse(r#"{"version":2,"marks":[]}"#), Err("version 2 is not one this build reads".into()));
        // Restored marks get ids, clean unique names, and one mark a letter; a ratio out of range is brought in.
        let wide = r#"{"split":{"axis":"down","ratio":7,"a":{"leaf":1},"b":{"leaf":2}}}"#;
        let panes = r#"[{"id":1,"kind":{"type":"terminal"}},{"id":2,"kind":{"type":"terminal"}}]"#;
        let mut loaded = parse(&file_of(&[("t", tab_target(wide, panes))])).unwrap();
        loaded.marks.push(Mark { name: " t\n".into(), key: Some('Q'), ..loaded.marks[0].clone() });
        loaded.marks.push(Mark { name: String::new(), key: Some('q'), ..loaded.marks[0].clone() });
        let mut m = Model::default();
        restore(&mut m, loaded);
        let kept: Vec<(u32, &str, Option<char>)> =
            m.marks.list.iter().map(|k| (k.id, k.name.as_str(), k.key)).collect();
        assert_eq!(kept, [(1, "t", None), (2, "t 2", None), (3, "mark", Some('q'))]);
        go(&mut m, "t");
        assert!(serde_json::to_string(&m.layout).unwrap().contains("\"ratio\":0.9"));
    }

    /// `marks.json` with one mark of each name and target given.
    fn file_of(targets: &[(&str, String)]) -> String {
        let mark = |(name, target): &(&str, String)| format!(r#"{{"name":"{name}","target":{target}}}"#);
        format!(r#"{{"version":1,"marks":[{}]}}"#, targets.iter().map(mark).collect::<Vec<_>>().join(","))
    }

    fn pane_target(kind: &str, rest: &str) -> String {
        format!(r#"{{"pane":{{"kind":{kind}{rest}}}}}"#)
    }

    fn tab_target(layout: &str, panes: &str) -> String {
        format!(r#"{{"tab":{{"title":"t","layout":{layout},"panes":{panes}}}}}"#)
    }

    const SHELL: &str = r#"{"type":"terminal"}"#;

    #[test]
    fn an_unsound_record_is_left_out_and_named_and_the_others_load() {
        let app = |name: &str, title: &str| format!(r#"{{"type":"app","name":"{name}","title":"{title}"}}"#);
        let long = "x".repeat(2000);
        let pane = r#"{"id":1,"kind":{"type":"terminal"}}"#;
        let twice = r#"{"split":{"axis":"right","ratio":0.5,"a":{"leaf":1},"b":{"leaf":1}}}"#;
        let text = file_of(&[
            ("good", pane_target(SHELL, r#","cwd":"/Users/me/notes""#)),
            ("relative", pane_target(SHELL, r#","cwd":"notes""#)),
            ("two lines", pane_target(SHELL, r#","cwd":"/a\nb""#)),
            ("long", pane_target(SHELL, &format!(r#","cwd":"/{long}""#))),
            ("nameless", pane_target(&app("", "x"), "")),
            ("escape", pane_target(&app("Slack", r"a\u001bb"), "")),
            ("missing", tab_target(r#"{"leaf":2}"#, &format!("[{pane}]"))),
            ("twice", tab_target(twice, &format!("[{pane}]"))),
            ("inner", tab_target(r#"{"leaf":1}"#, r#"[{"id":1,"kind":{"type":"terminal"},"cwd":"x"}]"#)),
            ("tab", tab_target(r#"{"leaf":1}"#, &format!("[{pane}]"))),
        ]);
        let loaded = parse(&text).expect("the file itself is sound");
        assert_eq!(loaded.marks.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["good", "tab"]);
        assert_eq!(
            loaded.skipped,
            [
                "mark \"relative\": its directory is not an absolute path",
                "mark \"two lines\": its directory is too long or not one line",
                "mark \"long\": its directory is too long or not one line",
                "mark \"nameless\": its app has no name",
                "mark \"escape\": its app or title is too long or not one line",
                "mark \"missing\": its layout names a pane it does not list once",
                "mark \"twice\": its layout shows a pane twice",
                "mark \"inner\": its directory is not an absolute path",
            ]
        );
    }

    #[test]
    fn a_command_in_the_file_is_never_typed_and_keys_this_build_does_not_know_are_kept() {
        let text = r#"{"version":1,"theme":"dark","marks":[
            {"id":"a1","name":"build","pinned":true,"target":{"pane":{"kind":{"type":"terminal"},"cwd":"/work","command":"rm -rf x","scrollback":9}}},
            {"id":"a2","name":"chat","target":{"pane":{"kind":{"type":"app","name":"Slack","title":"general","pid":811,"window":4211}}}},
            {"id":"a3","name":"both","target":{"tab":{"title":"t","display":2,"layout":{"leaf":1},
                "panes":[{"id":1,"kind":{"type":"terminal"},"command":"make","key":"k3f9"}]}}}]}"#;
        let loaded = parse(text).expect("it parses");
        assert!(loaded.commands && loaded.skipped.is_empty());
        let mut m = Model::default();
        restore(&mut m, loaded);
        // Going to them opens shells and types nothing: no effect could.
        assert_eq!(go(&mut m, "build"), [Effect::Opened(1)]);
        assert_eq!(go(&mut m, "both"), [Effect::Opened(2)]);
        // Written again: every other key is still there, and no command.
        let without = text.replace(r#","command":"rm -rf x""#, "").replace(r#","command":"make""#, "");
        let (again, before): (serde_json::Value, serde_json::Value) =
            (serde_json::from_str(&file(&m)).unwrap(), serde_json::from_str(&without).unwrap());
        assert_eq!(again, before);
        assert_eq!(parse(&file(&m)).map(|loaded| loaded.commands), Ok(false));
    }

    fn simple(uid: &str, name: &str) -> Mark {
        let record =
            PaneRecord { id: 0, kind: RecordKind::Terminal, cwd: Some(format!("/{name}")), extra: Extra::new() };
        let (target, extra) = (Target::Pane(record), Extra::new());
        Mark { id: 0, uid: uid.into(), name: name.into(), key: None, target, live: None, extra }
    }

    #[test]
    fn two_instances_merge_by_mark_and_neither_loses_what_the_other_did() {
        let named = |list: &[Mark]| list.iter().map(|m| m.name.clone()).collect::<Vec<_>>();
        let (a, b, c) = (simple("a", "a"), simple("b", "b"), simple("c", "c"));
        let base = vec![a.clone(), b.clone(), c.clone()];
        // Each added one: both are there, the file's first.
        let ours = [base.clone(), vec![simple("x", "mine")]].concat();
        let theirs = [base.clone(), vec![simple("y", "theirs")]].concat();
        assert_eq!(named(&merge(&base, &ours, &theirs)), ["a", "b", "c", "theirs", "mine"]);
        // This one removed b and renamed c; the other removed a and renamed b.
        let ours = vec![a.clone(), simple("c", "sea")];
        let theirs = vec![simple("b", "bee"), c.clone()];
        assert_eq!(named(&merge(&base, &ours, &theirs)), ["sea"]);
        // This one moved c up; the other's new mark goes after.
        let ours = vec![c.clone(), a.clone(), b.clone()];
        assert_eq!(
            named(&merge(&base, &ours, &[base.clone(), vec![simple("y", "theirs")]].concat())),
            ["c", "a", "b", "theirs"]
        );
        // Nothing changed here: the other's rename is taken, and the mark is still this run's.
        let mut m = Model::default();
        let shell = open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[(shell, "/work/notes")]);
        let (base, id) = (m.marks.list.clone(), m.marks.list[0].id);
        let theirs = vec![Mark { name: "journal".into(), ..base[0].clone() }, simple("y", "theirs")];
        let marks = merge(&base, &m.marks.list, &theirs);
        m.update(Action::RestoreMarks { marks, extra: Extra::new() }.sent_by(Origin::System));
        let rows = m.mark_rows();
        assert_eq!((rows[0].id, rows[0].name.as_str(), rows[0].pane), (id, "journal", Some(shell)));
        assert_eq!((rows[1].name.as_str(), rows[1].open), ("theirs", false));
    }

    #[test]
    fn a_removed_mark_is_offered_back_and_a_second_toggle_by_mistake_loses_nothing() {
        let mut m = Model::default();
        m.update(Action::Tick { now: 1_000 }.sent_by(Origin::System));
        let shell = open(&mut m, Kind::Terminal);
        let of = MarkOf::Pane(Some(shell));
        m.update(Action::MarkAdd { of, name: Some("notes".into()), key: Some('n'), uid: uid(), cwds: Vec::new() });
        mark(&mut m, MarkOf::Tab(None), &[]);
        let offer = Some(Notice { moved: Moved::MarkRemoved, until: 7_000 });
        // ⌘B on the marked pane: the mark is off, and offered back for six seconds.
        toggle(&mut m, MarkOf::Pane(None));
        assert_eq!((names(&m), m.undo_offer), (vec!["Tab 1"], offer));
        // ⌘B again: the same mark, in its place, with its name and letter. Not a new one.
        toggle(&mut m, MarkOf::Pane(None));
        assert_eq!((names(&m), m.marks.list[0].key, m.undo_offer), (vec!["notes", "Tab 1"], Some('n'), None));
        // Removed by its × and put back by the notice.
        m.update(Action::MarkRemove { mark: MarkRef::Name("notes".into()) });
        assert_eq!(m.undo_offer, offer);
        m.update(Action::MarkUndo);
        assert_eq!((names(&m), m.undo_offer), (vec!["notes", "Tab 1"], None));
        // The offer runs out. `u` in the Marks list still puts the mark back; in Tasks `u` is a task's undo.
        m.update(Action::MarkRemove { mark: MarkRef::Name("notes".into()) });
        m.update(Action::Tick { now: 8_000 }.sent_by(Origin::System));
        assert_eq!(m.undo_offer, None);
        m.update(Action::ToggleSection { section: crate::Section::Tasks });
        m.update(Action::FocusList);
        m.update(Action::ListUndo);
        assert_eq!(names(&m), ["Tab 1"]);
        m.update(Action::ToggleSection { section: crate::Section::Marks });
        m.update(Action::ListUndo);
        assert_eq!(names(&m), ["notes", "Tab 1"]);
        // After the offer, ⌘B on the pane it was of marks it anew: an old removal is not revived.
        toggle(&mut m, MarkOf::Pane(Some(shell)));
        m.update(Action::Tick { now: 20_000 }.sent_by(Origin::System));
        toggle(&mut m, MarkOf::Pane(Some(shell)));
        assert_eq!((names(&m), m.marks.list[1].key), (vec!["Tab 1", "shell"], None));
        // A letter given away while a mark was off stays where it went.
        let tab = m.marks.list[0].id;
        m.update(Action::MarkRemove { mark: MarkRef::Name("shell".into()) });
        m.update(Action::MarkUndo);
        m.marks.set_key(m.marks.list[1].id, Some('s'));
        m.update(Action::MarkRemove { mark: MarkRef::Name("shell".into()) });
        m.marks.set_key(tab, Some('s'));
        m.update(Action::MarkUndo);
        assert_eq!(m.marks.list.iter().map(|k| k.key).collect::<Vec<_>>(), [Some('s'), None]);
    }

    #[test]
    fn a_tab_mark_builds_what_it_can_and_says_which_panes_are_empty_and_tries_them_again() {
        let mut m = Model::default();
        let (left, _, below) = nested(&mut m);
        let cwds = vec![(left, "/work/app".into()), (below, "/gone/notes".into())];
        m.update(Action::MarkAdd { of: MarkOf::Tab(None), name: Some("work".into()), key: None, uid: uid(), cwds });
        m.update(Action::CloseTab { index: None });
        let app = Need::App { bundle: Some("com.tinyspeck.slackmacgap".into()), name: "Slack".into() };
        let missing = vec![Need::Dir("/gone/notes".into()), app];
        m.update(Action::MarksChecked { missing: missing.clone() }.sent_by(Origin::System));
        // One pane of three can open: the mark is available, and says what is missing.
        let row = m.mark_rows().remove(0);
        assert_eq!(
            (row.available, row.reason.as_deref()),
            (true, Some("2 of 3 panes cannot open: Slack is not installed"))
        );
        let effects = go(&mut m, "work");
        let [a, b, c] = m.panes.iter().map(|p| p.id).collect::<Vec<_>>()[..] else { panic!("three panes") };
        assert_eq!(effects, [Effect::Opened(a)], "the shell that can start does; nothing is asked of a missing app");
        let kinds: Vec<&Kind> = m.panes.iter().map(|p| &p.kind).collect();
        assert_eq!(kinds, [&Kind::Terminal, &Kind::Empty, &Kind::Empty]);
        let empty = |m: &Model| -> Vec<(PaneId, String)> {
            m.mark_rows()[0].unfilled.iter().map(|empty| (empty.pane, empty.why.clone())).collect()
        };
        assert_eq!(
            empty(&m),
            [(b, "Slack is not installed".to_string()), (c, "its directory is gone: /gone/notes".into())]
        );
        let row = m.mark_rows().remove(0);
        assert_eq!((row.open, row.reason.as_deref()), (true, Some("2 of 3 panes empty: Slack is not installed")));
        // Both are there now. Going to the open tab tries its empty panes again, from wherever.
        m.update(Action::MarksChecked { missing: Vec::new() }.sent_by(Origin::System));
        m.update(Action::NewTab);
        let effects = go(&mut m, "work");
        let host = Effect::HostWindow {
            mark: None,
            bundle: Some("com.tinyspeck.slackmacgap".into()),
            name: "Slack".into(),
            title: "general".into(),
            into: Some(b),
            axis: Right,
        };
        assert_eq!((effects, m.active), (vec![Effect::Opened(c), host], 1));
        assert_eq!(m.pane(c).map(|p| (&p.kind, p.cwd.as_deref())), Some((&Kind::Terminal, Some("/gone/notes"))));
        // The shell could not host the window: the pane says why, until it is filled or closed.
        m.update(Action::Unfilled { pane: b, why: "Slack has no window yet".into() }.sent_by(Origin::System));
        assert_eq!(empty(&m), [(b, "Slack has no window yet".to_string())]);
        m.update(Action::Fill { pane: b, kind: slack(9, "general") }.sent_by(Origin::System));
        assert_eq!((empty(&m), m.mark_rows()[0].reason.clone()), (vec![], None));
        // With nothing of it to open, a closed tab's mark is unavailable.
        m.update(Action::CloseTab { index: Some(1) });
        m.update(Action::MarksChecked { missing: [missing, vec![Need::Dir("/work/app".into())]].concat() });
        let row = m.mark_rows().remove(0);
        assert_eq!((row.available, row.reason.as_deref()), (false, Some("its directory is gone: /work/app")));
        assert_eq!((go(&mut m, "work"), m.panes.len()), (vec![], 0));
    }

    #[test]
    fn a_mark_records_the_windows_title_of_now_and_an_open_marks_hint_follows_it() {
        let mut m = Model::default();
        let pane = open(&mut m, slack(7, "general"));
        let shell = open(&mut m, Kind::Terminal);
        // The window shows another channel by the time it is marked: the shell reads it then.
        let titles = |now: &str| Action::Titles { titles: vec![(pane, now.into()), (shell, "x".into())] };
        m.update(titles("random").sent_by(Origin::System));
        mark(&mut m, MarkOf::Pane(Some(pane)), &[]);
        let hint = |m: &Model| match &m.marks.list[0].target {
            Target::Pane(PaneRecord { kind: RecordKind::App { title, .. }, .. }) => title.clone(),
            other => panic!("an app's mark: {other:?}"),
        };
        assert_eq!((hint(&m), m.pane(shell).map(|p| &p.kind)), ("random".into(), Some(&Kind::Terminal)));
        // Later it shows a third. The mark is still of that pane, and going to it takes the title.
        m.update(titles("design").sent_by(Origin::System));
        assert_eq!((m.mark_rows()[0].pane, hint(&m)), (Some(pane), "random".into()));
        go(&mut m, "Slack");
        assert_eq!(hint(&m), "design");
    }

    #[test]
    fn going_to_a_mark_waits_behind_what_has_the_keyboard_and_says_what() {
        let mut m = Model::default();
        open(&mut m, Kind::Terminal);
        mark(&mut m, MarkOf::Pane(None), &[]);
        m.update(Action::NewTab);
        let go = || Action::MarkGo { mark: MarkRef::Place(0), axis: Right };
        assert_eq!(m.waits(&go()), None);
        let boxes = [
            (
                Action::OpenPicker { axis: Right, new_tab: false, items: Vec::new() },
                Action::ClosePicker,
                "the picker is open",
            ),
            (Action::OpenCompose { text: String::new() }, Action::CloseCompose, "the new-task box is open"),
            (Action::ToggleKeys, Action::CloseKeys, "the key reference is open"),
        ];
        for (open, close, why) in boxes {
            m.update(open);
            assert_eq!((m.waits(&go()), m.update(go().sent_by(Origin::Socket)), m.active), (Some(why), vec![], 1));
            m.update(close);
        }
        // What does not act on panes never waits, e.g. what the shell found missing.
        m.update(Action::ToggleKeys);
        assert_eq!(m.waits(&Action::MarksChecked { missing: Vec::new() }), None);
        m.update(Action::CloseKeys);
        // Visual mode holds the keyboard on its terminal the same way.
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::ToggleVisual);
        assert_eq!((m.mode(), m.waits(&go())), (Mode::Visual, Some("visual mode is on")));
    }

    #[test]
    fn a_hosted_window_gets_its_pane_under_the_picker_but_none_in_a_pane_that_is_gone() {
        let mut m = Model::default();
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: Vec::new() });
        // `Open` does not wait: the window is in the shell's hand already.
        let open = Action::Open { kind: slack(1, "a"), axis: Right, beside: None };
        assert_eq!(m.update(open.sent_by(Origin::System)), [Effect::Opened(1)]);
        // A pane closed while its app started: nothing opens, so the shell gives the window back.
        assert_eq!(m.update(Action::Fill { pane: 9, kind: slack(2, "b") }.sent_by(Origin::System)), []);
        assert_eq!(m.panes.len(), 1);
    }

    #[test]
    fn a_started_apps_windows_have_settled_once_their_number_stops_changing() {
        assert!(!settled(0, 60_000), "no window yet");
        assert!(!settled(2, SETTLE_MS - 1));
        assert!(settled(2, SETTLE_MS));
    }
}
