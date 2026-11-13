//! Workbooks: saved, named tabs for a context that comes back ("Maestro DRI"). Recorded as a
//! tab's mark is, less window titles; one file each. Design: `docs/ai/workbook.md`.

use serde::{Deserialize, Serialize};

use crate::edit::{Edit, Field};
use crate::layout::Node;
use crate::marks::{self, Extra, MAX_NAME, MarkInput, Need, PaneRecord, RecordKind, TabRecord};
use crate::model::{Effect, Model, PaneId, Section};
use crate::tasks::NOTICE_MS;

pub type WorkbookId = u32;

/// The `version` of a workbook's file this build reads and writes.
pub const VERSION: u32 = 1;

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
    /// Its tabs. This build saves and opens the first; any others are kept as they are.
    pub tabs: Vec<TabRecord>,
    /// The id of its open tab, this run.
    #[serde(skip)]
    pub live: Option<u32>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// True if `uid` may name a file: exactly 16 lowercase hex digits. e.g. "9f3a61c2d4e5b708".
pub fn valid_uid(uid: &str) -> bool {
    uid.len() == 16 && uid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Reads one workbook's file. `stem`: the file's name without `.json`, which must be the id
/// inside: `x copy.json` is not `x`.
pub fn parse(text: &str, stem: &str) -> Result<Workbook, String> {
    let mut book: Workbook = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if book.version != VERSION {
        return Err(format!("version {} is not one this build reads", book.version));
    }
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
        match self.tabs.first() {
            None => Some("it has no tab"),
            Some(tab) => tab.unsound(),
        }
    }

    /// The panes its tab shows, in reading order.
    fn panes(&self) -> Vec<&PaneRecord> {
        self.tabs.first().map(TabRecord::shown).unwrap_or_default()
    }

    /// Everything it is opened with: each terminal's directory, each app.
    pub fn needs(&self) -> Vec<Need> {
        self.panes().into_iter().filter_map(PaneRecord::needs).collect()
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

/// A workbook as the bar and the state show it. No directory: the state names kinds only.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkbookRow {
    pub id: WorkbookId,
    pub name: String,
    pub panes: Vec<RowPane>,
    /// e.g. "2 terminals · Chrome, Slack".
    pub summary: String,
    /// Its tab is open now: `tab` is that tab's `id`.
    pub open: bool,
    pub tab: Option<u32>,
}

/// The workbooks, by name: a folder of files has no order of its own.
#[derive(Debug, Default)]
pub struct Workbooks {
    pub list: Vec<Workbook>,
    next: WorkbookId,
    offer: Option<Offer>,
}

impl Workbooks {
    pub fn get(&self, id: WorkbookId) -> Option<&Workbook> {
        self.list.iter().find(|book| book.id == id)
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

    /// A closed tab is no workbook's any more: a later tab may get its id.
    pub(crate) fn forget_tab(&mut self, tab: u32) {
        let books = self.list.iter_mut().chain(self.offer.as_mut().map(|offer| &mut offer.before));
        books.filter(|book| book.live == Some(tab)).for_each(|book| book.live = None);
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

/// "2 terminals · Chrome, Slack": what a workbook's tab holds, for its row.
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

impl Model {
    /// The workbooks as the bar lists them.
    pub fn workbook_rows(&self) -> Vec<WorkbookRow> {
        let row = |book: &Workbook| {
            let pane = |pane: &PaneRecord| match &pane.kind {
                RecordKind::Terminal => RowPane { kind: "terminal", app: None },
                RecordKind::App { name, .. } => RowPane { kind: "app", app: Some(name.clone()) },
                RecordKind::Empty => RowPane { kind: "empty", app: None },
            };
            let panes: Vec<RowPane> = book.panes().into_iter().map(pane).collect();
            let tab = self.tabs.iter().find(|tab| Some(tab.id) == book.live).map(|tab| tab.id);
            let (id, name, summary) = (book.id, book.name.clone(), summary(&panes));
            WorkbookRow { id, name, panes, summary, open: tab.is_some(), tab }
        };
        self.workbooks.list.iter().map(row).collect()
    }

    /// The workbook the tab on show came from, or was saved as.
    pub fn workbook_of_tab(&self) -> Option<WorkbookId> {
        let tab = self.tabs.get(self.active)?.id;
        self.workbooks.list.iter().find(|book| book.live == Some(tab)).map(|book| book.id)
    }

    /// The workbook of the selected row, in list mode.
    pub fn selected_workbook(&self) -> Option<WorkbookId> {
        match self.list.and_then(|at| self.list_rows().into_iter().nth(at)) {
            Some(crate::list::ListRow::Workbook(id)) => Some(id),
            _ => None,
        }
    }

    /// The tab on show as a workbook keeps it: splits, directories, apps by bundle id and name.
    /// No window title: it can hold other people's text.
    fn workbook_record(&self, cwds: &[(PaneId, String)]) -> Result<TabRecord, String> {
        let tab = self.tabs.get(self.active).ok_or("no tab to save")?;
        let leaves = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
        // A pane still waiting for its window or directory is saved as what it waits for, not as empty.
        let waiting = |id: PaneId| self.marks.unfilled().iter().find(|empty| empty.pane == id);
        let record = |pane: &crate::model::Pane| match waiting(pane.id) {
            Some(empty) => PaneRecord { id: pane.id, ..empty.record.clone() },
            None => self.record(pane, cwds),
        };
        let mut panes: Vec<PaneRecord> = leaves.iter().filter_map(|id| self.pane(*id)).map(record).collect();
        for pane in &mut panes {
            if let RecordKind::App { title, .. } = &mut pane.kind {
                title.clear();
            }
        }
        if panes.iter().all(|pane| pane.kind == RecordKind::Empty) {
            return Err("a tab with nothing in it has nothing to save".into());
        }
        let (title, layout) = (marks::kept_title(&tab.title, 256), self.layout.clone());
        let record = TabRecord { title, task: None, focused: self.focused, layout, panes, extra: Extra::new() };
        match record.unsound() {
            Some(wrong) => Err(format!("this tab cannot be saved: {wrong}")),
            None => Ok(record),
        }
    }

    /// Why the tab on show cannot be saved, for the shell to say.
    pub fn workbook_save_refusal(&self) -> Option<String> {
        self.workbook_record(&[]).err()
    }

    /// Saves the tab on show: updates the workbook it came from (the old one is offered back),
    /// else adds one named for the tab and starts typing its name. `uid`: a new one's file name.
    pub(crate) fn workbook_save(&mut self, uid: String, cwds: &[(PaneId, String)], effects: &mut Vec<Effect>) {
        let Ok(record) = self.workbook_record(cwds) else { return };
        let Some((tab, title)) = self.tabs.get(self.active).map(|tab| (tab.id, tab.title.clone())) else { return };
        let until = self.now.saturating_add(NOTICE_MS);
        match self.workbook_of_tab().and_then(|id| self.workbooks.list.iter().position(|book| book.id == id)) {
            Some(at) => {
                let book = &mut self.workbooks.list[at];
                let before = book.clone();
                match book.tabs.first_mut() {
                    // The tab's keys this build does not know stay.
                    Some(first) => *first = TabRecord { extra: std::mem::take(&mut first.extra), ..record },
                    None => book.tabs.push(record),
                }
                self.workbooks.offer = Some(Offer { change: Change::Updated, before, until });
            }
            None => {
                if !valid_uid(&uid) || self.workbooks.list.iter().any(|book| book.uid == uid) {
                    return;
                }
                let (version, tabs, live) = (VERSION, vec![record], Some(tab));
                let book = Workbook { id: 0, version, uid, name: title, tabs, live, extra: Extra::new() };
                let id = self.workbooks.add(book);
                self.workbooks.offer = None;
                self.leave(effects);
                self.workbook_edit(id);
            }
        }
        // The section shows what was saved.
        self.sidebar = Some(Section::Workbook);
        self.list = self.list.map(|_| 0);
    }

    /// The directories of a closed workbook's terminals that are gone, for the shell to say.
    pub fn workbook_gone(&self, id: WorkbookId) -> Vec<String> {
        let Some(book) = self.workbooks.get(id).filter(|book| self.workbook_tab(book).is_none()) else {
            return Vec::new();
        };
        let dirs = book.panes().into_iter().filter(|pane| pane.kind == RecordKind::Terminal);
        let dirs = dirs.filter_map(|pane| pane.cwd.clone());
        dirs.filter(|dir| self.marks.missing().contains(&Need::Dir(dir.clone()))).collect()
    }

    /// The index of a workbook's open tab.
    fn workbook_tab(&self, book: &Workbook) -> Option<usize> {
        self.tabs.iter().position(|tab| Some(tab.id) == book.live)
    }

    /// Selects a workbook's open tab, else builds it as a tab's mark is built (`rebuild_tab`),
    /// as a tab of no task. `home`: where a terminal starts whose directory is gone.
    pub(crate) fn workbook_open(&mut self, id: WorkbookId, home: Option<String>, effects: &mut Vec<Effect>) {
        let Some(book) = self.workbooks.get(id).cloned() else { return };
        if let Some(index) = self.workbook_tab(&book) {
            self.list = None;
            self.select_tab(index, effects);
            return self.retry_unfilled(effects);
        }
        let Some(first) = book.tabs.first() else { return };
        let mut record = TabRecord { title: book.name.clone(), task: None, ..first.clone() };
        let missing = self.marks.missing();
        for pane in record.panes.iter_mut().filter(|pane| pane.kind == RecordKind::Terminal) {
            if pane.cwd.as_ref().is_some_and(|dir| missing.contains(&Need::Dir(dir.clone()))) {
                pane.cwd = home.clone();
            }
        }
        let tab = self.rebuild_tab(&record, effects);
        if let Some(book) = self.workbooks.list.iter_mut().find(|book| book.id == id) {
            book.live = Some(tab);
        }
    }

    /// Starts typing a workbook's name in its row, with all of it selected.
    pub(crate) fn workbook_edit(&mut self, id: WorkbookId) {
        let Some(book) = self.workbooks.get(id).filter(|_| self.mark_input_may_start()) else { return };
        let mut text = Field::new(&book.name, MAX_NAME);
        text.apply(Edit::SelectAll);
        self.mark_input = Some(MarkInput::Workbook { workbook: id, text });
    }

    /// Keeps a typed name, unless it is empty once cleaned. Its open tab takes the name too.
    pub(crate) fn workbook_rename(&mut self, id: WorkbookId, name: &str) {
        if marks::clean(name).is_empty() {
            return;
        }
        let name = self.workbooks.unique_name(name, Some(id));
        let Some(book) = self.workbooks.list.iter_mut().find(|book| book.id == id) else { return };
        book.name = name.clone();
        let live = book.live;
        self.workbooks.sort();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| Some(tab.id) == live) {
            tab.title = name;
        }
    }

    /// Deletes a workbook, and offers for a few seconds to put it back. Its open tab stays.
    pub(crate) fn workbook_remove(&mut self, id: WorkbookId) {
        let Some(at) = self.workbooks.list.iter().position(|book| book.id == id) else { return };
        let before = self.workbooks.list.remove(at);
        let until = self.now.saturating_add(NOTICE_MS);
        self.workbooks.offer = Some(Offer { change: Change::Deleted, before, until });
        if matches!(&self.mark_input, Some(MarkInput::Workbook { workbook, .. }) if *workbook == id) {
            self.mark_input = None;
        }
    }

    /// Takes the last delete or update back. A name given since stays.
    pub(crate) fn workbook_undo(&mut self) {
        let Some(Offer { change, before, .. }) = self.workbooks.offer.take() else { return };
        match change {
            Change::Deleted => drop(self.workbooks.add(before)),
            Change::Updated => {
                if let Some(book) = self.workbooks.list.iter_mut().find(|book| book.id == before.id) {
                    book.tabs = before.tabs;
                }
            }
        }
    }

    /// Replaces the workbooks with those kept on disk. One that was here (by `uid`) keeps its
    /// id and open tab; an unsound one, or a second of one `uid`, is left out.
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
            (book.id, book.live) = (was.map_or(0, |old| old.id), was.and_then(|old| old.live).filter(open));
            self.workbooks.add(book);
        }
        if let Some(MarkInput::Workbook { workbook, .. }) = &self.mark_input
            && self.workbooks.get(*workbook).is_none()
        {
            self.mark_input = None;
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
        assert_eq!((book.name.as_str(), book.uid.as_str(), book.live), ("Tab 1", UID, Some(1)));
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
        assert_eq!(parse(&text, UID).map(|read| Workbook { id: book.id, live: book.live, ..read }).as_ref(), Ok(book));
        // The section opens on it, and its name is typed there, all selected.
        assert_eq!((m.sidebar, m.mode()), (Some(Section::Workbook), Mode::Mark));
        let field = m.field().map(|(of, field)| (of, field.text(), field.selected()));
        assert_eq!(field, Some(("workbook", "Tab 1", "Tab 1")));
        let row = &m.workbook_rows()[0];
        assert_eq!((row.summary.as_str(), row.open, row.tab), ("1 terminal · Slack", true, Some(1)));
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
        m.update(Action::WorkbookEdit { workbook: 2 });
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
        m.update(Action::MarkCancel);
        m.update(Action::WorkbookEdit { workbook: 2 });
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
        m.update(Action::WorkbookEdit { workbook: 1 });
        m.update(Action::FieldEdit { edit: Edit::Insert("DRI".into()) });
        m.update(Action::MarkSubmit);
        let shares = m.layout.clone();
        // Its tab is open: it comes forward, and no second one is built.
        m.update(Action::NewTab);
        assert_eq!(
            (m.update(Action::WorkbookOpen { workbook: 1, home: None }), m.active, m.tabs.len()),
            (vec![], 0, 2)
        );
        // Closed, it is built again: the splits and ratios, a terminal in its directory, the
        // app asked of the shell as a mark's tab asks, and the empty pane empty.
        m.update(Action::CloseTab { index: Some(0) });
        assert_eq!(m.workbook_rows()[0].open, false);
        let effects = m.update(Action::WorkbookOpen { workbook: 1, home: None });
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
    }

    #[test]
    fn a_terminal_whose_directory_is_gone_starts_in_the_users_folder() {
        let mut m = saved();
        m.update(Action::MarkCancel);
        m.update(Action::CloseTab { index: None });
        m.update(Action::MarksChecked { missing: vec![Need::Dir("/code/app".into())] }.sent_by(Origin::System));
        assert_eq!(m.workbook_gone(1), ["/code/app"]);
        m.update(Action::WorkbookOpen { workbook: 1, home: Some("/Users/me".into()) });
        let terminal = m.panes.iter().find(|pane| pane.kind == Kind::Terminal).and_then(|pane| pane.cwd.as_deref());
        assert_eq!((terminal, m.workbook_gone(1)), (Some("/Users/me"), vec![]));
    }

    #[test]
    fn deleting_offers_the_workbook_back_and_leaves_its_tab_open() {
        let mut m = saved();
        m.update(Action::WorkbookRemove { workbook: 1 });
        assert_eq!((names(&m), m.mode(), m.tabs.len()), (vec![], Mode::Normal, 1));
        assert_eq!((m.workbooks.offered(), m.workbook_of_tab()), (Some(Change::Deleted), None));
        m.update(Action::WorkbookUndo);
        assert_eq!((names(&m), m.workbook_of_tab(), m.workbooks.offered()), (vec!["Tab 1"], Some(1), None));
        // Put back after its tab closed, it stands for no tab: a later tab may get that id.
        m.update(Action::WorkbookRemove { workbook: 1 });
        m.update(Action::CloseTab { index: None });
        m.update(Action::WorkbookUndo);
        assert_eq!((m.workbooks.list[0].live, m.workbook_of_tab()), (None, None));
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
    fn only_the_user_saves_opens_renames_or_deletes() {
        for origin in [Origin::Socket, Origin::System] {
            let mut m = saved();
            let before = format!("{m:?}");
            // A client's key in the name being typed, and its ↵, are refused as well.
            let typed =
                [Action::FieldEdit { edit: Edit::Insert("x".into()) }, Action::MarkSubmit, Action::MarkBackspace];
            for action in typed {
                assert_eq!(m.refusal(&action, origin), Some(USERS_ALONE), "{action:?}");
            }
            m.update(Action::MarkCancel);
            m.update(Action::FocusList);
            let own = [
                Action::WorkbookSave { uid: UID_2.into(), cwds: vec![] },
                Action::WorkbookOpen { workbook: 1, home: None },
                Action::WorkbookEdit { workbook: 1 },
                Action::WorkbookRemove { workbook: 1 },
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
            m.update(Action::WorkbookEdit { workbook: 1 });
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
        assert_eq!(parse(&text.replace("\"version\": 1", "\"version\": 2"), UID).is_err(), true);
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
        let other = Workbook { id: 0, live: Some(99), uid: UID_2.into(), ..kept.clone() };
        let fresh = Workbook { id: 0, live: None, ..kept.clone() };
        m.update(
            Action::RestoreWorkbooks { workbooks: vec![fresh.clone(), fresh, stray, other] }.sent_by(Origin::System),
        );
        let list: Vec<(u32, &str, Option<u32>)> =
            m.workbooks.list.iter().map(|book| (book.id, book.name.as_str(), book.live)).collect();
        assert_eq!(list, [(1, "Tab 1", Some(1)), (2, "Tab 1 2", None)]);
        // The one whose name was being typed is still there: the typing goes on.
        assert_eq!(m.mode(), Mode::Mark);
        m.update(Action::RestoreWorkbooks { workbooks: vec![] }.sent_by(Origin::System));
        assert_eq!((m.mode(), names(&m)), (Mode::Normal, vec![]));
    }
}
