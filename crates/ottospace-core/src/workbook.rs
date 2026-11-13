//! Workbooks: saved, named sets of tabs for a context that comes back ("Maestro DRI"). Each tab
//! is recorded as a tab's mark is, less window titles; one file each. Design: `docs/ai/workbook.md`.

use serde::{Deserialize, Serialize};

use crate::edit::{Edit, Field};
use crate::layout::Node;
use crate::marks::{self, Extra, MAX_NAME, MarkInput, Need, PaneRecord, RecordKind, TabRecord};
use crate::model::{Effect, Model, PaneId, Section};
use crate::tasks::NOTICE_MS;

pub type WorkbookId = u32;

/// The `version` of a workbook's file this build writes. It reads 1 too: stage 1's, one tab.
pub const VERSION: u32 = 2;

/// The most tabs a workbook keeps.
pub const MAX_TABS: usize = 32;

/// Why a client's workbook action does nothing: they are the user's files and apps.
pub const USERS_ALONE: &str = "only the user saves, opens, renames or deletes a workbook";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workbook {
    /// This run's name for it. Not kept.
    #[serde(skip)]
    pub id: WorkbookId,
    pub version: u32,
    /// Its name for good, and its file's: `<uid>.json`. 16 hex digits, made up by the shell.
    #[serde(rename = "id")]
    pub uid: String,
    pub name: String,
    /// Its tabs, in the order they open.
    pub tabs: Vec<TabRecord>,
    /// The id of each tab's open tab, this run, by place in `tabs`. Shorter than `tabs`: the rest are closed.
    #[serde(skip)]
    pub live: Vec<Option<u32>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// True if `uid` may name a file: exactly 16 lowercase hex digits. e.g. "9f3a61c2d4e5b708".
pub fn valid_uid(uid: &str) -> bool {
    uid.len() == 16 && uid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Reads one workbook's file. `stem`: the file's name without `.json`, which must be the id
/// inside: `x copy.json` is not `x`. A version 1 file reads as version 2: the same shape.
pub fn parse(text: &str, stem: &str) -> Result<Workbook, String> {
    let mut book: Workbook = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if !matches!(book.version, 1 | VERSION) {
        return Err(format!("version {} is not one this build reads", book.version));
    }
    book.version = VERSION;
    if !valid_uid(&book.uid) || book.uid != stem {
        return Err("its id is not its file's name".into());
    }
    book.name = marks::clean(&book.name);
    match book.unsound() {
        Some(wrong) => Err(wrong.into()),
        None => Ok(book),
    }
}

/// The text of a workbook's file.
pub fn write(book: &Workbook) -> String {
    serde_json::to_string_pretty(book).unwrap_or_default() + "\n"
}

impl Workbook {
    /// What is wrong with it as read from disk, if anything.
    fn unsound(&self) -> Option<&'static str> {
        if self.name.is_empty() {
            return Some("it has no name");
        }
        match self.tabs.len() {
            0 => Some("it has no tab"),
            n if n > MAX_TABS => Some("it has too many tabs"),
            _ => self.tabs.iter().find_map(TabRecord::unsound),
        }
    }

    /// The panes its tabs show, tab by tab, in reading order.
    fn panes(&self) -> Vec<&PaneRecord> {
        self.tabs.iter().flat_map(TabRecord::shown).collect()
    }

    /// Everything it is opened with: each terminal's directory, each app.
    pub fn needs(&self) -> Vec<Need> {
        self.panes().into_iter().filter_map(PaneRecord::needs).collect()
    }

    /// The open tab of its tab at `at`.
    pub fn live(&self, at: usize) -> Option<u32> {
        self.live.get(at).copied().flatten()
    }

    fn set_live(&mut self, at: usize, tab: Option<u32>) {
        if self.live.len() <= at {
            self.live.resize(at + 1, None);
        }
        self.live[at] = tab;
    }

    /// The place among its tabs of the open tab `tab`.
    fn place_of(&self, tab: u32) -> Option<usize> {
        self.live.iter().position(|live| *live == Some(tab))
    }

    /// The title its tab at `at` opens with: a workbook of one tab names it, as stage 1 did.
    fn title(&self, at: usize) -> String {
        match self.tabs.len() {
            1 => self.name.clone(),
            _ => self.tabs.get(at).map(|tab| tab.title.clone()).unwrap_or_default(),
        }
    }
}

/// What the section offers to take back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Deleted,
    Updated,
}

impl Change {
    pub fn title(self) -> &'static str {
        match self {
            Change::Deleted => "Workbook deleted",
            Change::Updated => "Updated",
        }
    }
}

/// The last delete or update, and the workbook as it was.
#[derive(Clone, Debug, PartialEq)]
struct Offer {
    change: Change,
    before: Workbook,
    until: u64,
}

/// A pane of a workbook's tab, as the state lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RowPane {
    /// `terminal`, `app` or `empty`.
    pub kind: &'static str,
    /// An app pane's app, by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

/// One tab of a workbook, as its row lists it when unfolded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RowTab {
    pub title: String,
    pub panes: Vec<RowPane>,
    /// e.g. "2 terminals · Chrome".
    pub summary: String,
    /// It is open now: `tab` is that tab's `id`.
    pub open: bool,
    pub tab: Option<u32>,
}

/// A workbook as the bar and the state show it. No directory: the state names kinds only.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkbookRow {
    pub id: WorkbookId,
    pub name: String,
    /// The panes of all its tabs, tab by tab.
    pub panes: Vec<RowPane>,
    /// e.g. "3 tabs · 4 terminals · Chrome, Slack"; one tab: "2 terminals · Chrome, Slack".
    pub summary: String,
    /// One of its tabs is open now: `tab` is the first open one's `id`.
    pub open: bool,
    pub tab: Option<u32>,
    pub tabs: Vec<RowTab>,
    /// Its tabs are listed under it. Only a workbook of more than one tab unfolds.
    pub unfolded: bool,
}

/// The workbooks, by name: a folder of files has no order of its own.
#[derive(Debug, Default)]
pub struct Workbooks {
    pub list: Vec<Workbook>,
    next: WorkbookId,
    offer: Option<Offer>,
    /// The workbooks whose tabs are listed under them, this run.
    unfolded: Vec<WorkbookId>,
}

impl Workbooks {
    pub fn get(&self, id: WorkbookId) -> Option<&Workbook> {
        self.list.iter().find(|book| book.id == id)
    }

    fn get_mut(&mut self, id: WorkbookId) -> Option<&mut Workbook> {
        self.list.iter_mut().find(|book| book.id == id)
    }

    /// `name`, or `name 2`, `name 3`: no two workbooks share a name.
    fn unique_name(&self, name: &str, but: Option<WorkbookId>) -> String {
        let taken = |name: &str| self.list.iter().any(|book| Some(book.id) != but && book.name == name);
        marks::unique_name(name, "workbook", taken)
    }

    /// Adds a workbook, with a name no other has. `id` 0: a new id.
    fn add(&mut self, mut book: Workbook) -> WorkbookId {
        if book.id == 0 {
            self.next += 1;
            book.id = self.next;
        }
        book.name = self.unique_name(&book.name, None);
        let id = book.id;
        self.list.push(book);
        self.sort();
        id
    }

    fn sort(&mut self) {
        self.list.sort_by_key(|book| book.name.to_lowercase());
    }

    /// A closed tab is no workbook's any more: a later tab may get its id. Also a tab saved
    /// into another workbook: a tab stands for one workbook's tab at most.
    pub(crate) fn forget_tab(&mut self, tab: u32) {
        let books = self.list.iter_mut().chain(self.offer.as_mut().map(|offer| &mut offer.before));
        books.flat_map(|book| book.live.iter_mut()).filter(|live| **live == Some(tab)).for_each(|live| *live = None);
    }

    /// The workbook and place of the open tab `tab`.
    pub fn entry_of(&self, tab: u32) -> Option<(WorkbookId, usize)> {
        self.list.iter().find_map(|book| Some((book.id, book.place_of(tab)?)))
    }

    /// True if its tabs are listed under it.
    pub fn unfolded(&self, id: WorkbookId) -> bool {
        self.unfolded.contains(&id) && self.get(id).is_some_and(|book| book.tabs.len() > 1)
    }

    /// The change the section offers to take back, while it offers.
    pub fn offered(&self) -> Option<Change> {
        self.offer.as_ref().map(|offer| offer.change)
    }

    /// True if the offer's time is up at `now`.
    pub(crate) fn due(&self, now: u64) -> bool {
        self.offer.as_ref().is_some_and(|offer| offer.until <= now)
    }

    pub(crate) fn tick(&mut self, now: u64) {
        self.offer = self.offer.take().filter(|offer| offer.until > now);
    }

    /// Everything the workbooks need, for the shell to look for on this machine.
    pub fn needs(&self) -> Vec<Need> {
        self.list.iter().flat_map(Workbook::needs).collect()
    }
}

/// "2 terminals · Chrome, Slack": what some panes hold, for a row.
fn summary(panes: &[RowPane]) -> String {
    let terminals = panes.iter().filter(|pane| pane.kind == "terminal").count();
    let mut apps: Vec<&str> = Vec::new();
    for app in panes.iter().filter_map(|pane| pane.app.as_deref()) {
        if !apps.contains(&app) {
            apps.push(app);
        }
    }
    let terminals = match terminals {
        0 => None,
        1 => Some("1 terminal".to_string()),
        n => Some(format!("{n} terminals")),
    };
    let apps = (!apps.is_empty()).then(|| apps.join(", "));
    let parts: Vec<String> = terminals.into_iter().chain(apps).collect();
    if parts.is_empty() { "empty".into() } else { parts.join(" · ") }
}

fn row_pane(pane: &PaneRecord) -> RowPane {
    match &pane.kind {
        RecordKind::Terminal => RowPane { kind: "terminal", app: None },
        RecordKind::App { name, .. } => RowPane { kind: "app", app: Some(name.clone()) },
        RecordKind::Empty => RowPane { kind: "empty", app: None },
    }
}

impl Model {
    /// The workbooks as the bar lists them.
    pub fn workbook_rows(&self) -> Vec<WorkbookRow> {
        let open = |tab: Option<u32>| self.tabs.iter().find(|open| Some(open.id) == tab).map(|open| open.id);
        let row = |book: &Workbook| {
            let tab = |(at, record): (usize, &TabRecord)| {
                let panes: Vec<RowPane> = record.shown().into_iter().map(row_pane).collect();
                let (summary, tab) = (summary(&panes), open(book.live(at)));
                RowTab { title: book.title(at), panes, summary, open: tab.is_some(), tab }
            };
            let tabs: Vec<RowTab> = book.tabs.iter().enumerate().map(tab).collect();
            let panes: Vec<RowPane> = tabs.iter().flat_map(|tab| tab.panes.clone()).collect();
            let summary = match tabs.len() {
                1 => summary(&panes),
                n => format!("{n} tabs · {}", summary(&panes)),
            };
            let tab = tabs.iter().find_map(|tab| tab.tab);
            let (id, name, unfolded) = (book.id, book.name.clone(), self.workbooks.unfolded(book.id));
            WorkbookRow { id, name, panes, summary, open: tab.is_some(), tab, tabs, unfolded }
        };
        self.workbooks.list.iter().map(row).collect()
    }

    /// The workbook the tab on show came from, or was saved into.
    pub fn workbook_of_tab(&self) -> Option<WorkbookId> {
        let tab = self.tabs.get(self.active)?.id;
        self.workbooks.entry_of(tab).map(|(id, _)| id)
    }

    /// The workbook of the selected row in list mode, and the place of its tab on a tab's row.
    pub fn selected_workbook(&self) -> Option<(WorkbookId, Option<usize>)> {
        match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
            Some(crate::list::ListRow::Workbook(id)) => Some((id, None)),
            Some(crate::list::ListRow::WorkbookTab(id, at)) => Some((id, Some(at))),
            _ => None,
        }
    }

    /// The tab at `index` as a workbook keeps it: splits, directories, apps by bundle id and
    /// name. No window title: it can hold other people's text.
    fn workbook_record(&self, index: usize, cwds: &[(PaneId, String)]) -> Result<TabRecord, String> {
        let tab = self.tabs.get(index).ok_or("no tab to save")?;
        let (layout, focused) = match index == self.active {
            true => (self.layout.clone(), self.focused),
            false => (tab.layout.clone(), tab.focused),
        };
        let leaves = layout.as_ref().map(Node::leaves).unwrap_or_default();
        // A pane still waiting for its window or directory is saved as what it waits for, not as empty.
        let waiting = |id: PaneId| self.marks.unfilled().iter().find(|empty| empty.pane == id);
        let record = |pane: &crate::model::Pane| match waiting(pane.id) {
            Some(empty) => PaneRecord { id: pane.id, ..empty.record.clone() },
            None => self.record(pane, cwds),
        };
        let mut panes: Vec<PaneRecord> = leaves.iter().filter_map(|id| self.pane(*id)).map(record).collect();
        // A saved pane is a new record: no keys a waiting one kept from its file (a URL, say).
        for pane in &mut panes {
            pane.extra.clear();
            if let RecordKind::App { title, extra, .. } = &mut pane.kind {
                title.clear();
                extra.clear();
            }
        }
        if panes.iter().all(|pane| pane.kind == RecordKind::Empty) {
            return Err("a tab with nothing in it has nothing to save".into());
        }
        let title = marks::kept_title(&tab.title, 256);
        let record = TabRecord { title, task: None, focused, layout, panes, extra: Extra::new() };
        match record.unsound() {
            Some(wrong) => Err(format!("this tab cannot be saved: {wrong}")),
            None => Ok(record),
        }
    }

    /// Why the tab on show cannot be saved, for the shell to say.
    pub fn workbook_save_refusal(&self) -> Option<String> {
        self.workbook_record(self.active, &[]).err()
    }

    /// Why the tab on show cannot be added to workbook `id`, for the shell to say.
    pub fn workbook_add_refusal(&self, id: WorkbookId) -> Option<String> {
        let tab = self.tabs.get(self.active).map(|tab| tab.id)?;
        let book = self.workbooks.get(id)?;
        let full = book.place_of(tab).is_none() && book.tabs.len() >= MAX_TABS;
        let full = full.then(|| format!("a workbook keeps {MAX_TABS} tabs at most"));
        self.workbook_save_refusal().or(full)
    }

    /// Why the tabs on the bar cannot be saved as a workbook, for the shell to say.
    pub fn workbook_save_all_refusal(&self) -> Option<String> {
        let saved = self.visible_tabs().into_iter().filter(|at| self.workbook_record(*at, &[]).is_ok()).count();
        (saved == 0).then(|| "no tab on the bar has anything to save".into())
    }

    /// The section shows workbook `id`, its row selected in list mode.
    fn show_workbook(&mut self, id: WorkbookId) {
        self.sidebar = Some(Section::Workbook);
        let row = self.list_rows().iter().position(|row| *row == crate::list::ListRow::Workbook(id));
        self.list = self.list.map(|_| row.unwrap_or(0));
    }

    /// Replaces the record of workbook `id`'s tab at `at` (`None`: adds one) with `record`, open
    /// as tab `tab`, and offers the old workbook back.
    fn workbook_put(&mut self, id: WorkbookId, at: Option<usize>, record: TabRecord, tab: u32) {
        let until = self.now.saturating_add(NOTICE_MS);
        let Some(before) = self.workbooks.get(id).cloned() else { return };
        // A tab stands for one workbook's tab at most.
        self.workbooks.forget_tab(tab);
        let Some(book) = self.workbooks.get_mut(id) else { return };
        let at = match at.filter(|at| *at < book.tabs.len()) {
            Some(at) => {
                // The tab's keys this build does not know stay.
                let first = &mut book.tabs[at];
                *first = TabRecord { extra: std::mem::take(&mut first.extra), ..record };
                at
            }
            None if book.tabs.len() < MAX_TABS => {
                // Its first tab opened under the workbook's name, and goes on doing so.
                if let [only] = book.tabs.as_mut_slice() {
                    only.title = book.name.clone();
                }
                book.tabs.push(record);
                book.tabs.len() - 1
            }
            None => return,
        };
        book.set_live(at, Some(tab));
        self.workbooks.offer = Some(Offer { change: Change::Updated, before, until });
    }

    /// Saves the tab on show: updates its tab of the workbook it came from (the old one is
    /// offered back), else adds a workbook of that tab, named for it, and starts typing its
    /// name. `uid`: a new one's file name.
    pub(crate) fn workbook_save(&mut self, uid: String, cwds: &[(PaneId, String)], effects: &mut Vec<Effect>) {
        let Ok(record) = self.workbook_record(self.active, cwds) else { return };
        let Some((tab, title)) = self.tabs.get(self.active).map(|tab| (tab.id, tab.title.clone())) else { return };
        let saved = match self.workbooks.entry_of(tab) {
            Some((id, at)) => {
                self.workbook_put(id, Some(at), record, tab);
                id
            }
            None => match self.workbook_new(uid, title, vec![(tab, record)], effects) {
                Some(id) => id,
                None => return,
            },
        };
        self.show_workbook(saved);
    }

    /// Adds the tab on show to workbook `id`, or updates it there if it is one of its tabs.
    /// A tab of another workbook is that one's no longer: it stands for its tab here.
    pub(crate) fn workbook_add_tab(&mut self, id: WorkbookId, cwds: &[(PaneId, String)]) {
        let Ok(record) = self.workbook_record(self.active, cwds) else { return };
        let Some(tab) = self.tabs.get(self.active).map(|tab| tab.id) else { return };
        let Some(book) = self.workbooks.get(id) else { return };
        let at = book.place_of(tab);
        if at.is_none() && book.tabs.len() >= MAX_TABS {
            return;
        }
        self.workbook_put(id, at, record, tab);
        self.show_workbook(id);
    }

    /// Saves every tab on the bar that has something in it as one new workbook, in bar order.
    pub(crate) fn workbook_save_all(&mut self, uid: String, cwds: &[(PaneId, String)], effects: &mut Vec<Effect>) {
        let record = |at: usize| Some((self.tabs[at].id, self.workbook_record(at, cwds).ok()?));
        let tabs: Vec<(u32, TabRecord)> = self.visible_tabs().into_iter().filter_map(record).take(MAX_TABS).collect();
        if tabs.is_empty() {
            return;
        }
        if let Some(id) = self.workbook_new(uid, "Workbook".into(), tabs, effects) {
            self.show_workbook(id);
        }
    }

    /// A new workbook of `tabs` (each with the open tab it stands for), and the typing of its name.
    fn workbook_new(
        &mut self,
        uid: String,
        name: String,
        tabs: Vec<(u32, TabRecord)>,
        effects: &mut Vec<Effect>,
    ) -> Option<WorkbookId> {
        if !valid_uid(&uid) || self.workbooks.list.iter().any(|book| book.uid == uid) {
            return None;
        }
        for (tab, _) in &tabs {
            self.workbooks.forget_tab(*tab);
        }
        let live = tabs.iter().map(|(tab, _)| Some(*tab)).collect();
        let tabs = tabs.into_iter().map(|(_, record)| record).collect();
        let book = Workbook { id: 0, version: VERSION, uid, name, tabs, live, extra: Extra::new() };
        let id = self.workbooks.add(book);
        self.workbooks.offer = None;
        self.leave(effects);
        self.workbook_edit(id, None);
        Some(id)
    }

    /// The directories of a workbook's closed tabs' terminals that are gone, for the shell to
    /// say. `tab`: of that tab alone.
    pub fn workbook_gone(&self, id: WorkbookId, tab: Option<usize>) -> Vec<String> {
        let Some(book) = self.workbooks.get(id) else { return Vec::new() };
        let closed = |at: &usize| self.open_tab(book.live(*at)).is_none() && tab.is_none_or(|tab| tab == *at);
        let records = (0..book.tabs.len()).filter(closed).flat_map(|at| book.tabs[at].shown());
        let dirs = records.filter(|pane| pane.kind == RecordKind::Terminal).filter_map(|pane| pane.cwd.clone());
        let mut gone: Vec<String> = Vec::new();
        for dir in dirs.filter(|dir| self.marks.missing().contains(&Need::Dir(dir.clone()))) {
            if !gone.contains(&dir) {
                gone.push(dir);
            }
        }
        gone
    }

    /// The index of the open tab `tab`.
    fn open_tab(&self, tab: Option<u32>) -> Option<usize> {
        self.tabs.iter().position(|open| Some(open.id) == tab)
    }

    /// Opens a workbook's tabs (`tab`: that one alone) in order, each built as a tab's mark is
    /// (`rebuild_tab`), as a tab of no task, and selects the first. One that is open is kept, not
    /// built again. `home`: where a terminal starts whose directory is gone.
    pub(crate) fn workbook_open(
        &mut self,
        id: WorkbookId,
        tab: Option<usize>,
        home: Option<String>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(book) = self.workbooks.get(id).cloned() else { return };
        let places: Vec<usize> = match tab {
            Some(at) if at < book.tabs.len() => vec![at],
            Some(_) => return,
            None => (0..book.tabs.len()).collect(),
        };
        let Some(&first) = places.first() else { return };
        let was_open = self.open_tab(book.live(first)).is_some();
        let missing = self.marks.missing().to_vec();
        let closed: Vec<usize> = places.into_iter().filter(|at| self.open_tab(book.live(*at)).is_none()).collect();
        for at in closed {
            let mut record = TabRecord { title: book.title(at), task: None, ..book.tabs[at].clone() };
            for pane in record.panes.iter_mut().filter(|pane| pane.kind == RecordKind::Terminal) {
                if pane.cwd.as_ref().is_some_and(|dir| missing.contains(&Need::Dir(dir.clone()))) {
                    pane.cwd = home.clone();
                }
            }
            // Each tab asks for its windows as it is built; the shell parks those of tabs not on show.
            let tab = self.rebuild_tab(&record, effects);
            if let Some(book) = self.workbooks.get_mut(id) {
                book.set_live(at, Some(tab));
            }
        }
        self.list = None;
        let first = self.workbooks.get(id).and_then(|book| self.open_tab(book.live(first)));
        if let Some(index) = first {
            self.select_tab(index, effects);
        }
        // Its first tab was open: its empty panes are tried again, as a tab's mark does.
        if was_open {
            self.retry_unfilled(effects);
        }
    }

    /// Starts typing a workbook's name in its row (`tab`: the title of that tab), all selected.
    pub(crate) fn workbook_edit(&mut self, id: WorkbookId, tab: Option<usize>) {
        let Some(book) = self.workbooks.get(id).filter(|_| self.mark_input_may_start()) else { return };
        let name = match tab {
            None => book.name.clone(),
            Some(at) if at < book.tabs.len() && book.tabs.len() > 1 => book.tabs[at].title.clone(),
            Some(_) => return,
        };
        let mut text = Field::new(&name, MAX_NAME);
        text.apply(Edit::SelectAll);
        self.mark_input = Some(MarkInput::Workbook { workbook: id, tab, text });
    }

    /// Keeps a typed name, unless it is empty once cleaned. Its open tab takes the name too: a
    /// tab's title, or a workbook's name when it has one tab.
    pub(crate) fn workbook_rename(&mut self, id: WorkbookId, tab: Option<usize>, name: &str) {
        let name = marks::clean(name);
        if name.is_empty() {
            return;
        }
        let name = match tab {
            None => self.workbooks.unique_name(&name, Some(id)),
            Some(_) => name,
        };
        let Some(book) = self.workbooks.get_mut(id) else { return };
        let live = match tab {
            // One tab opens under the workbook's name: it keeps that title once a second is added.
            None if book.tabs.len() == 1 => {
                book.tabs[0].title = name.clone();
                book.live(0)
            }
            None => None,
            Some(at) => match book.tabs.get_mut(at) {
                Some(record) => {
                    record.title = name.clone();
                    book.live(at)
                }
                None => return,
            },
        };
        if tab.is_none() {
            book.name = name.clone();
            self.workbooks.sort();
        }
        if let Some(open) = self.tabs.iter_mut().find(|open| Some(open.id) == live) {
            open.title = name;
        }
    }

    /// Deletes a workbook (`tab`: that tab of it; its last tab, the workbook), and offers for a
    /// few seconds to put it back. Its open tabs stay.
    pub(crate) fn workbook_remove(&mut self, id: WorkbookId, tab: Option<usize>) {
        let Some(at) = self.workbooks.list.iter().position(|book| book.id == id) else { return };
        let until = self.now.saturating_add(NOTICE_MS);
        let book = &mut self.workbooks.list[at];
        match tab {
            Some(place) if place >= book.tabs.len() => return,
            Some(place) if book.tabs.len() > 1 => {
                let before = book.clone();
                book.tabs.remove(place);
                if place < book.live.len() {
                    book.live.remove(place);
                }
                self.workbooks.offer = Some(Offer { change: Change::Updated, before, until });
            }
            _ => {
                let before = self.workbooks.list.remove(at);
                self.workbooks.offer = Some(Offer { change: Change::Deleted, before, until });
            }
        }
        // The name being typed is of a workbook or a tab that is gone, or has moved.
        if matches!(&self.mark_input, Some(MarkInput::Workbook { workbook, .. }) if *workbook == id) {
            self.mark_input = None;
        }
    }

    /// Lists a workbook's tabs under it, or stops.
    pub(crate) fn workbook_fold(&mut self, id: WorkbookId) {
        let known = self.workbooks.get(id).is_some();
        let unfolded = &mut self.workbooks.unfolded;
        match unfolded.iter().position(|book| *book == id) {
            Some(at) => drop(unfolded.remove(at)),
            None if known => unfolded.push(id),
            None => {}
        }
    }

    /// Takes the last delete or update back. A name given since stays. A tab it stood for that
    /// is closed, or stands for another workbook's tab now, is not its tab again.
    pub(crate) fn workbook_undo(&mut self) {
        let Some(Offer { change, mut before, .. }) = self.workbooks.offer.take() else { return };
        let id = before.id;
        let claimed = |tab: u32| self.workbooks.list.iter().any(|book| book.id != id && book.place_of(tab).is_some());
        let keep = |tab: &u32| self.tabs.iter().any(|open| open.id == *tab) && !claimed(*tab);
        let live: Vec<Option<u32>> = before.live.iter().map(|live| live.filter(keep)).collect();
        before.live = live;
        match change {
            Change::Deleted => drop(self.workbooks.add(before)),
            Change::Updated => {
                if let Some(book) = self.workbooks.get_mut(id) {
                    (book.tabs, book.live) = (before.tabs, before.live);
                }
            }
        }
        if let Some(MarkInput::Workbook { workbook, tab: Some(_), .. }) = &self.mark_input
            && *workbook == id
        {
            self.mark_input = None;
        }
    }

    /// Replaces the workbooks with those kept on disk. One that was here (by `uid`) keeps its
    /// id and open tabs; an unsound one, or a second of one `uid`, is left out.
    pub(crate) fn restore_workbooks(&mut self, books: Vec<Workbook>) {
        let before = std::mem::take(&mut self.workbooks.list);
        self.workbooks.offer = None;
        for mut book in books {
            let known = self.workbooks.list.iter().any(|other| other.uid == book.uid);
            if !valid_uid(&book.uid) || known || book.version != VERSION || book.unsound().is_some() {
                continue;
            }
            let was = before.iter().find(|old| old.uid == book.uid);
            let open = |tab: &u32| self.tabs.iter().any(|open| open.id == *tab);
            let mut live: Vec<Option<u32>> = was.map(|old| old.live.clone()).unwrap_or_default();
            live.truncate(book.tabs.len());
            (book.id, book.live) = (was.map_or(0, |old| old.id), live.into_iter().map(|l| l.filter(open)).collect());
            self.workbooks.add(book);
        }
        let list = &self.workbooks.list;
        self.workbooks.unfolded.retain(|id| list.iter().any(|book| book.id == *id));
        if let Some(MarkInput::Workbook { workbook, tab, .. }) = &self.mark_input {
            let book = self.workbooks.get(*workbook);
            if book.is_none_or(|book| tab.is_some_and(|at| at >= book.tabs.len() || book.tabs.len() < 2)) {
                self.mark_input = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis::{Down, Right};
    use crate::model::{Action, Kind, Mode, Origin};

    const UID: &str = "9f3a61c2d4e5b708";
    const UID_2: &str = "1b7c0e9a55d2f346";

    fn slack(window: u32) -> Kind {
        Kind::App {
            window,
            title: "alice: the secret plan".into(),
            bundle: Some("com.slack".into()),
            name: "Slack".into(),
        }
    }

    fn save(m: &mut Model, uid: &str, cwds: &[(PaneId, &str)]) -> Vec<Effect> {
        let cwds = cwds.iter().map(|(pane, dir)| (*pane, dir.to_string())).collect();
        m.update(Action::WorkbookSave { uid: uid.into(), cwds })
    }

    /// A terminal beside Slack over an empty pane, saved as a workbook.
    fn saved() -> Model {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::Open { kind: slack(7), axis: Right, beside: None });
        m.update(Action::Split { axis: Down, kind: Kind::Empty });
        m.update(Action::Resize { dir: crate::Dir::Up });
        save(&mut m, UID, &[(1, "/code/app")]);
        m
    }

    fn names(m: &Model) -> Vec<&str> {
        m.workbooks.list.iter().map(|book| book.name.as_str()).collect()
    }

    #[test]
    fn saving_a_tab_records_its_splits_directories_and_apps_and_no_title() {
        let m = saved();
        let book = &m.workbooks.list[0];
        assert_eq!((book.name.as_str(), book.uid.as_str(), &book.live), ("Tab 1", UID, &vec![Some(1)]));
        let tab = &book.tabs[0];
        assert_eq!((tab.layout.as_ref(), tab.task), (m.layout.as_ref(), None));
        let app = RecordKind::App {
            bundle: Some("com.slack".into()),
            name: "Slack".into(),
            title: "".into(),
            extra: Extra::new(),
        };
        let kinds: Vec<(&RecordKind, Option<&str>)> = tab.panes.iter().map(|p| (&p.kind, p.cwd.as_deref())).collect();
        assert_eq!(kinds, [(&RecordKind::Terminal, Some("/code/app")), (&app, None), (&RecordKind::Empty, None)]);
        // The file holds no window title, and reads back as it was written.
        let text = write(book);
        assert!(!text.contains("secret") && !text.contains("title\": \"alice"), "{text}");
        assert_eq!(
            parse(&text, UID).map(|read| Workbook { id: book.id, live: book.live.clone(), ..read }).as_ref(),
            Ok(book)
        );
        // The section opens on it, and its name is typed there, all selected.
        assert_eq!((m.sidebar, m.mode()), (Some(Section::Workbook), Mode::Mark));
        let field = m.field().map(|(of, field)| (of, field.text(), field.selected()));
        assert_eq!(field, Some(("workbook", "Tab 1", "Tab 1")));
        let row = &m.workbook_rows()[0];
        assert_eq!((row.summary.as_str(), row.open, row.tab), ("1 terminal · Slack", true, Some(1)));
    }

    #[test]
    fn a_tab_split_too_deep_to_read_back_is_not_saved() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        for _ in 0..63 {
            m.update(Action::Split { axis: Down, kind: Kind::Terminal });
        }
        save(&mut m, UID, &[]);
        let refused = Some("this tab cannot be saved: its splits are nested too deep");
        assert_eq!((m.workbook_save_refusal().as_deref(), names(&m)), (refused, vec![]));
        // As deep as is kept, it reads back.
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        for _ in 0..48 {
            m.update(Action::Split { axis: Down, kind: Kind::Terminal });
        }
        save(&mut m, UID, &[]);
        assert!(parse(&write(&m.workbooks.list[0]), UID).is_ok());
    }

    #[test]
    fn a_tab_with_nothing_in_it_is_not_saved_nor_one_with_an_id_that_is_no_file_name() {
        let mut m = Model::default();
        save(&mut m, UID, &[]);
        assert_eq!(m.workbook_save_refusal().as_deref(), Some("a tab with nothing in it has nothing to save"));
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        for uid in ["", "../../etc/passwd", "9F3A61C2D4E5B708", "9f3a61c2d4e5b70", "9f3a61c2d4e5b708/"] {
            save(&mut m, uid, &[]);
        }
        assert_eq!((names(&m), m.sidebar), (vec![], None));
    }

    #[test]
    fn typing_a_name_renames_it_and_its_open_tab_and_no_two_share_a_name() {
        let mut m = saved();
        m.update(Action::FieldEdit { edit: Edit::Insert("Maestro DRI".into()) });
        m.update(Action::MarkSubmit);
        assert_eq!((names(&m), m.tabs[0].title.as_str(), m.mode()), (vec!["Maestro DRI"], "Maestro DRI", Mode::Normal));
        // A second tab saved under the same name is numbered.
        m.update(Action::NewTab);
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        save(&mut m, UID_2, &[]);
        m.update(Action::FieldEdit { edit: Edit::Insert("Maestro DRI".into()) });
        m.update(Action::MarkSubmit);
        assert_eq!(names(&m), ["Maestro DRI", "Maestro DRI 2"]);
        // Esc keeps the name it had; an empty name too.
        m.update(Action::WorkbookEdit { workbook: 2, tab: None });
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
        m.update(Action::MarkCancel);
        m.update(Action::WorkbookEdit { workbook: 2, tab: None });
        m.update(Action::FieldEdit { edit: Edit::Insert(" ".into()) });
        m.update(Action::MarkSubmit);
        assert_eq!(names(&m), ["Maestro DRI", "Maestro DRI 2"]);
    }

    #[test]
    fn saving_a_workbooks_tab_again_updates_it_and_offers_the_old_one_back() {
        let mut m = saved();
        m.update(Action::MarkCancel);
        let before = m.workbooks.list[0].tabs.clone();
        m.update(Action::Tick { now: 1_000 });
        m.update(Action::Close { pane: Some(3) });
        save(&mut m, UID_2, &[(1, "/code/other")]);
        let book = &m.workbooks.list[0];
        assert_eq!((names(&m), book.uid.as_str(), book.tabs[0].panes.len()), (vec!["Tab 1"], UID, 2));
        assert_eq!((book.tabs[0].panes[0].cwd.as_deref(), m.mode()), (Some("/code/other"), Mode::Normal));
        assert_eq!((m.workbooks.offered(), m.due(1_000 + NOTICE_MS)), (Some(Change::Updated), true));
        m.update(Action::WorkbookUndo);
        assert_eq!((&m.workbooks.list[0].tabs, m.workbooks.offered()), (&before, None));
        // The offer goes when its time is up.
        save(&mut m, UID_2, &[]);
        m.update(Action::Tick { now: 1_000 + NOTICE_MS }.sent_by(Origin::System));
        assert_eq!(m.workbooks.offered(), None);
    }

    /// What opening `saved()`'s workbook asks of the shell, into panes `term` and `app`.
    fn opened(term: PaneId, app: PaneId) -> Vec<Effect> {
        let (bundle, name, title) = (Some("com.slack".to_string()), "Slack".to_string(), String::new());
        vec![Effect::Opened(term), Effect::HostWindow { mark: None, bundle, name, title, into: Some(app), axis: Right }]
    }

    #[test]
    fn opening_builds_a_tab_of_that_name_and_an_open_one_is_selected() {
        let mut m = saved();
        m.update(Action::MarkCancel);
        m.update(Action::WorkbookEdit { workbook: 1, tab: None });
        m.update(Action::FieldEdit { edit: Edit::Insert("DRI".into()) });
        m.update(Action::MarkSubmit);
        let shares = m.layout.clone();
        // Its tab is open: it comes forward, and no second one is built.
        m.update(Action::NewTab);
        assert_eq!(
            (m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None }), m.active, m.tabs.len()),
            (vec![], 0, 2)
        );
        // Closed, it is built again: the splits and ratios, a terminal in its directory, the
        // app asked of the shell as a mark's tab asks, and the empty pane empty.
        m.update(Action::CloseTab { index: Some(0) });
        assert_eq!(m.workbook_rows()[0].open, false);
        m.workbooks.list[0].tabs[0].panes[1].extra.insert("url".into(), serde_json::json!("https://x"));
        let effects = m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None });
        assert_eq!(effects, opened(4, 5));
        let tab = &m.tabs[m.active];
        assert_eq!((tab.title.as_str(), tab.task, m.workbook_rows()[0].tab), ("DRI", None, Some(tab.id)));
        let renumbered = |tree: &Node| format!("{tree:?}").replace("Leaf(4)", "Leaf(1)").replace("Leaf(5)", "Leaf(2)");
        let same = m.layout.as_ref().map(renumbered).map(|tree| tree.replace("Leaf(6)", "Leaf(3)"));
        assert_eq!(same, shares.as_ref().map(|tree| format!("{tree:?}")));
        let panes: Vec<(&Kind, Option<&str>)> = m.panes.iter().map(|p| (&p.kind, p.cwd.as_deref())).collect();
        assert_eq!(
            &panes[panes.len() - 3..],
            [(&Kind::Terminal, Some("/code/app")), (&Kind::Empty, None), (&Kind::Empty, None)]
        );
        // Saving it now updates the same workbook. The pane still waiting for Slack's window
        // is saved as Slack's, not as empty, and a key of the tab this build does not know stays.
        m.workbooks.list[0].tabs[0].extra.insert("pinned".into(), serde_json::json!(true));
        save(&mut m, UID_2, &[]);
        assert_eq!((names(&m), m.workbooks.offered()), (vec!["DRI"], Some(Change::Updated)));
        let tab = &m.workbooks.list[0].tabs[0];
        let kinds: Vec<&str> = m.workbook_rows()[0].panes.iter().map(|pane| pane.kind).collect();
        assert_eq!(
            (kinds, tab.extra.get("pinned")),
            (vec!["terminal", "app", "empty"], Some(&serde_json::json!(true)))
        );
        // The waiting pane is a new record: the URL its file held is not kept.
        assert!(!write(&m.workbooks.list[0]).contains("https://x"));
    }

    #[test]
    fn a_terminal_whose_directory_is_gone_starts_in_the_users_folder() {
        let mut m = saved();
        m.update(Action::MarkCancel);
        m.update(Action::CloseTab { index: None });
        m.update(Action::MarksChecked { missing: vec![Need::Dir("/code/app".into())] }.sent_by(Origin::System));
        assert_eq!(m.workbook_gone(1, None), ["/code/app"]);
        m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: Some("/Users/me".into()) });
        let terminal = m.panes.iter().find(|pane| pane.kind == Kind::Terminal).and_then(|pane| pane.cwd.as_deref());
        assert_eq!((terminal, m.workbook_gone(1, None)), (Some("/Users/me"), vec![]));
    }

    #[test]
    fn deleting_offers_the_workbook_back_and_leaves_its_tab_open() {
        let mut m = saved();
        m.update(Action::WorkbookRemove { workbook: 1, tab: None });
        assert_eq!((names(&m), m.mode(), m.tabs.len()), (vec![], Mode::Normal, 1));
        assert_eq!((m.workbooks.offered(), m.workbook_of_tab()), (Some(Change::Deleted), None));
        m.update(Action::WorkbookUndo);
        assert_eq!((names(&m), m.workbook_of_tab(), m.workbooks.offered()), (vec!["Tab 1"], Some(1), None));
        // Put back after its tab closed, it stands for no tab: a later tab may get that id.
        m.update(Action::WorkbookRemove { workbook: 1, tab: None });
        m.update(Action::CloseTab { index: None });
        m.update(Action::WorkbookUndo);
        assert_eq!((&m.workbooks.list[0].live, m.workbook_of_tab()), (&vec![None], None));
    }

    #[test]
    fn list_mode_opens_renames_deletes_and_puts_back() {
        let mut m = saved();
        m.update(Action::MarkCancel);
        m.update(Action::CloseTab { index: None });
        m.update(Action::FocusList);
        assert_eq!((m.sidebar, m.list_rows()), (Some(Section::Workbook), vec![crate::list::ListRow::Workbook(1)]));
        m.update(Action::ListMarkEdit { field: marks::Field::Name });
        assert_eq!(m.field().map(|(of, _)| of), Some("workbook"));
        m.update(Action::MarkCancel);
        m.update(Action::ListDismiss);
        m.update(Action::ListUndo);
        assert_eq!(names(&m), ["Tab 1"]);
        assert_eq!((m.update(Action::ListChoose { axis: Right }), m.mode()), (opened(4, 5), Mode::Normal));
        // Tab reaches the section between Apps and Tasks.
        m.update(Action::ToggleSection { section: Section::Apps });
        m.update(Action::FocusList);
        m.update(Action::ListSection { delta: 1 });
        assert_eq!(m.sidebar, Some(Section::Workbook));
    }

    #[test]
    fn saving_from_another_sections_list_selects_the_row_saved() {
        let mut m = saved();
        m.update(Action::FieldEdit { edit: Edit::Insert("Beta".into()) });
        m.update(Action::MarkSubmit);
        m.update(Action::NewTab);
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        save(&mut m, UID_2, &[]);
        m.update(Action::FieldEdit { edit: Edit::Insert("Alpha".into()) });
        m.update(Action::MarkSubmit);
        // List mode in Tasks, Beta's tab on show: the section turns to Beta's row, not Alpha's.
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::FocusList);
        save(&mut m, UID, &[]);
        assert_eq!((m.sidebar, m.workbooks.offered()), (Some(Section::Workbook), Some(Change::Updated)));
        assert_eq!(m.selected_workbook(), Some((1, None)));
    }

    #[test]
    fn only_the_user_saves_opens_renames_or_deletes() {
        for origin in [Origin::Socket, Origin::System] {
            let mut m = saved();
            let before = format!("{m:?}");
            // A client's key in the name being typed, its ↵ and its Esc, are refused as well.
            let typed = [
                Action::FieldEdit { edit: Edit::Insert("x".into()) },
                Action::MarkSubmit,
                Action::MarkBackspace,
                Action::MarkCancel,
                Action::MarkPrefix,
            ];
            for action in typed {
                assert_eq!(m.refusal(&action, origin), Some(USERS_ALONE), "{action:?}");
            }
            m.update(Action::MarkCancel);
            m.update(Action::FocusList);
            let own = [
                Action::WorkbookSave { uid: UID_2.into(), cwds: vec![] },
                Action::WorkbookOpen { workbook: 1, tab: None, home: None },
                Action::WorkbookEdit { workbook: 1, tab: None },
                Action::WorkbookRemove { workbook: 1, tab: None },
                Action::WorkbookUndo,
                Action::ListChoose { axis: Right },
                Action::ListDismiss,
                Action::ListUndo,
                Action::ListMarkEdit { field: marks::Field::Name },
            ];
            for action in own {
                assert_eq!(m.refusal(&action, origin), Some(USERS_ALONE), "{action:?}");
                assert_eq!(m.update(action.clone().sent_by(origin)), vec![], "{action:?}");
            }
            m.update(Action::LeaveList);
            m.update(Action::WorkbookEdit { workbook: 1, tab: None });
            assert_eq!(format!("{m:?}"), before);
        }
    }

    #[test]
    fn a_file_is_read_only_under_its_own_id_and_unknown_keys_are_kept() {
        let text = r#"{"version": 1, "id": "9f3a61c2d4e5b708", "name": " DRI ", "colour": "red",
            "tabs": [{"title": "t", "pinned": true, "layout": {"leaf": 1},
                      "panes": [{"id": 1, "kind": {"type": "terminal"}, "cwd": "/code/app", "shell": "zsh"}]},
                     {"title": "later", "panes": []}]}"#;
        let book = parse(text, UID).expect("it reads");
        assert_eq!((book.name.as_str(), book.tabs.len()), ("DRI", 2));
        let written = write(&book);
        for kept in ["\"colour\": \"red\"", "\"pinned\": true", "\"shell\": \"zsh\"", "\"later\""] {
            assert!(written.contains(kept), "{kept} in {written}");
        }
        assert_eq!(parse(text, UID_2), Err("its id is not its file's name".into()));
        assert_eq!(parse(&text.replace("\"version\": 1", "\"version\": 3"), UID).is_err(), true);
        let wrong = [
            ("\"/code/app\"", "\"code/app\"", "its directory is not an absolute path"),
            ("{\"leaf\": 1}", "{\"leaf\": 2}", "its layout names a pane it does not list once"),
            ("\" DRI \"", "\" \"", "it has no name"),
        ];
        for (from, to, why) in wrong {
            assert_eq!(parse(&text.replace(from, to), UID), Err(why.into()));
        }
        assert_eq!(parse(&text.replace("9f3a61c2d4e5b708", "../../../x"), "../../../x").is_err(), true);
    }

    #[test]
    fn restoring_keeps_an_open_tab_by_id_and_leaves_out_what_is_unsound() {
        let mut m = saved();
        let kept = m.workbooks.list[0].clone();
        let stray = Workbook { uid: "nope".into(), ..kept.clone() };
        let other = Workbook { id: 0, live: vec![Some(99)], uid: UID_2.into(), ..kept.clone() };
        let fresh = Workbook { id: 0, live: vec![], ..kept.clone() };
        m.update(
            Action::RestoreWorkbooks { workbooks: vec![fresh.clone(), fresh, stray, other] }.sent_by(Origin::System),
        );
        let list: Vec<(u32, &str, Option<u32>)> =
            m.workbooks.list.iter().map(|book| (book.id, book.name.as_str(), book.live(0))).collect();
        assert_eq!(list, [(1, "Tab 1", Some(1)), (2, "Tab 1 2", None)]);
        // The one whose name was being typed is still there: the typing goes on.
        assert_eq!(m.mode(), Mode::Mark);
        m.update(Action::RestoreWorkbooks { workbooks: vec![] }.sent_by(Origin::System));
        assert_eq!((m.mode(), names(&m)), (Mode::Normal, vec![]));
    }

    /// Two tabs on the bar, the first a terminal beside Slack, the second a terminal: saved as
    /// one workbook named "DRI", closed again.
    fn two_tabs() -> Model {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::Open { kind: slack(7), axis: Right, beside: None });
        m.update(Action::NewTab);
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::SelectTab { index: 0 });
        m.update(Action::WorkbookSaveAll { uid: UID.into(), cwds: vec![(1, "/code/app".into()), (3, "/logs".into())] });
        m.update(Action::FieldEdit { edit: Edit::Insert("DRI".into()) });
        m.update(Action::MarkSubmit);
        m
    }

    #[test]
    fn saving_all_tabs_keeps_each_in_bar_order_and_each_tab_stands_for_its_own() {
        let mut m = two_tabs();
        let book = &m.workbooks.list[0];
        let titles: Vec<&str> = book.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!((book.name.as_str(), titles, &book.live), ("DRI", vec!["Tab 1", "Tab 2"], &vec![Some(1), Some(2)]));
        let row = &m.workbook_rows()[0];
        assert_eq!((row.summary.as_str(), row.open, row.tab), ("2 tabs · 2 terminals · Slack", true, Some(1)));
        // A workbook of more than one tab names neither of its open tabs: those keep their titles.
        assert_eq!(m.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(), ["Tab 1", "Tab 2"]);
        // Saving the second tab updates its own place, not the first.
        m.update(Action::SelectTab { index: 1 });
        m.update(Action::Split { axis: Down, kind: Kind::Terminal });
        save(&mut m, UID_2, &[]);
        let book = &m.workbooks.list[0];
        assert_eq!((names(&m), book.tabs[0].panes.len(), book.tabs[1].panes.len()), (vec!["DRI"], 2, 2));
        assert_eq!(m.workbooks.offered(), Some(Change::Updated));
        // An empty bar has nothing to save.
        let empty = Model::default();
        assert_eq!(empty.workbook_save_all_refusal().as_deref(), Some("no tab on the bar has anything to save"));
    }

    #[test]
    fn opening_builds_every_closed_tab_in_order_keeps_the_open_ones_and_selects_the_first() {
        let mut m = two_tabs();
        m.update(Action::CloseTab { index: Some(0) });
        assert_eq!(
            (m.tabs.len(), m.workbook_rows()[0].tabs[0].open, m.workbook_rows()[0].tabs[1].open),
            (1, false, true)
        );
        let effects = m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None });
        // Only the closed tab is built: its terminal starts, and Slack is asked for, into it.
        let titles: Vec<&str> = m.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!((titles, m.active, m.tabs.len()), (vec!["Tab 2", "Tab 1"], 1, 2));
        assert_eq!(effects.len(), 2, "{effects:?}");
        assert!(
            matches!(&effects[1], Effect::HostWindow { into: Some(_), name, .. } if name == "Slack"),
            "{effects:?}"
        );
        // Everything is open: the first comes forward again, nothing is built, and its empty
        // pane asks for Slack again, as a tab's mark does.
        m.update(Action::SelectTab { index: 0 });
        let again = m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None });
        assert_eq!((again, m.active, m.tabs.len()), (vec![effects[1].clone()], 1, 2));
        // All closed: both are built, in order, and the first is on show.
        m.update(Action::CloseTab { index: Some(1) });
        m.update(Action::CloseTab { index: Some(0) });
        m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None });
        let titles: Vec<&str> = m.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!((titles, m.active), (vec!["Tab 1", "Tab 1", "Tab 2"], 1));
        let row = &m.workbook_rows()[0];
        assert_eq!((row.tabs[0].tab, row.tabs[1].tab), (Some(m.tabs[1].id), Some(m.tabs[2].id)));
        // One tab alone, by its place.
        m.update(Action::CloseTab { index: Some(2) });
        m.update(Action::WorkbookOpen { workbook: 1, tab: Some(1), home: None });
        assert_eq!((m.tabs.len(), m.tabs[m.active].title.as_str()), (3, "Tab 2"));
        assert_eq!(m.update(Action::WorkbookOpen { workbook: 1, tab: Some(2), home: None }), vec![]);
    }

    #[test]
    fn a_tab_is_added_to_a_workbook_and_stands_for_one_workbooks_tab_at_most() {
        let mut m = saved();
        m.update(Action::MarkCancel);
        m.update(Action::NewTab);
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        // A tab with nothing in it is not added.
        m.update(Action::NewTab);
        assert!(m.workbook_add_refusal(1).is_some());
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![] });
        assert_eq!(m.workbooks.list[0].tabs.len(), 1);
        m.update(Action::CloseTab { index: None });
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![(4, "/logs".into())] });
        let book = &m.workbooks.list[0];
        assert_eq!((book.tabs.len(), book.tabs[1].panes[0].cwd.as_deref(), book.live(1)), (2, Some("/logs"), Some(2)));
        assert_eq!(
            (m.workbooks.offered(), m.sidebar, m.mode()),
            (Some(Change::Updated), Some(Section::Workbook), Mode::Normal)
        );
        // Undo takes the tab out again, and the open tab stands for nothing.
        m.update(Action::WorkbookUndo);
        assert_eq!((m.workbooks.list[0].tabs.len(), m.workbook_of_tab()), (1, None));
        // Saved as a new workbook, then added to the first: it is the first's only.
        m.update(Action::WorkbookSave { uid: UID_2.into(), cwds: vec![] });
        m.update(Action::MarkCancel);
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![] });
        let lives: Vec<Vec<Option<u32>>> = m.workbooks.list.iter().map(|book| book.live.clone()).collect();
        assert_eq!(lives, [vec![Some(1), Some(2)], vec![None]]);
        // A workbook is full at MAX_TABS.
        m.workbooks.list[0].tabs = vec![m.workbooks.list[0].tabs[0].clone(); MAX_TABS];
        m.update(Action::NewTab);
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        let full = format!("a workbook keeps {MAX_TABS} tabs at most");
        assert_eq!(m.workbook_add_refusal(1), Some(full));
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![] });
        assert_eq!(m.workbooks.list[0].tabs.len(), MAX_TABS);
    }

    #[test]
    fn an_unfolded_workbook_lists_its_tabs_each_opened_renamed_or_removed_alone() {
        let mut m = two_tabs();
        m.update(Action::FocusList);
        assert_eq!(m.list_rows(), [crate::list::ListRow::Workbook(1)]);
        m.update(Action::WorkbookFold { workbook: 1 });
        use crate::list::ListRow::{Workbook as Book, WorkbookTab};
        assert_eq!(
            (m.list_rows(), m.workbook_rows()[0].unfolded),
            (vec![Book(1), WorkbookTab(1, 0), WorkbookTab(1, 1)], true)
        );
        // R on a tab's row types its title; its open tab takes it.
        m.update(Action::ListMove { delta: 2 });
        assert_eq!(m.selected_workbook(), Some((1, Some(1))));
        m.update(Action::ListMarkEdit { field: marks::Field::Name });
        assert_eq!(m.field().map(|(of, field)| (of, field.text())), Some(("workbook", "Tab 2")));
        m.update(Action::FieldEdit { edit: Edit::Insert("logs".into()) });
        m.update(Action::MarkSubmit);
        assert_eq!((m.workbooks.list[0].tabs[1].title.as_str(), m.tabs[1].title.as_str()), ("logs", "logs"));
        // X removes that tab; the workbook stays, and folds no more with one tab.
        m.update(Action::ListDismiss);
        assert_eq!(
            (names(&m), m.workbooks.list[0].tabs.len(), m.workbooks.offered()),
            (vec!["DRI"], 1, Some(Change::Updated))
        );
        assert_eq!(m.list_rows(), [Book(1)]);
        // U puts it back, open as it was.
        m.update(Action::ListUndo);
        assert_eq!((m.workbooks.list[0].tabs.len(), m.workbooks.list[0].live(1)), (2, Some(2)));
        // ↵ on a tab's row opens that one alone.
        m.update(Action::CloseTab { index: Some(1) });
        m.list = Some(2);
        m.update(Action::ListChoose { axis: Right });
        assert_eq!((m.tabs.len(), m.tabs[m.active].title.as_str()), (2, "logs"));
        // Removing a workbook's last tab deletes it.
        m.update(Action::WorkbookRemove { workbook: 1, tab: Some(0) });
        m.update(Action::WorkbookRemove { workbook: 1, tab: Some(0) });
        assert_eq!((names(&m), m.workbooks.offered()), (vec![], Some(Change::Deleted)));
    }

    #[test]
    fn a_stage_1_file_reads_as_a_workbook_of_one_tab_and_is_written_as_version_2() {
        let text = r#"{"version": 1, "id": "9f3a61c2d4e5b708", "name": "DRI",
            "tabs": [{"title": "t", "layout": {"leaf": 1}, "panes": [{"id": 1, "kind": {"type": "terminal"}, "cwd": "/code/app"}]}]}"#;
        let book = parse(text, UID).expect("it reads");
        assert_eq!((book.version, book.tabs.len()), (VERSION, 1));
        assert!(write(&book).contains("\"version\": 2"));
        // Every tab must be sound now, not the first alone, and there are not too many.
        let two = text.replace("}]}]}", "}]}, {\"title\": \"x\", \"layout\": {\"leaf\": 9}, \"panes\": []}]}");
        assert_eq!(parse(&two, UID), Err("its layout names a pane it does not list once".into()));
        let tab = r#"{"title": "t", "layout": {"leaf": 1}, "panes": [{"id": 1, "kind": {"type": "terminal"}}]}"#;
        let many =
            format!(r#"{{"version": 2, "id": "{UID}", "name": "x", "tabs": [{}]}}"#, vec![tab; MAX_TABS + 1].join(","));
        assert_eq!(parse(&many, UID), Err("it has too many tabs".into()));
    }
}
