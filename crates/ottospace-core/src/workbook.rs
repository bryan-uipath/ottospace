//! Workbooks: saved, named sets of tabs for a context that comes back ("Maestro DRI"). Each tab
//! is recorded as a tab's mark is, less window titles; one file each. Design: `docs/ai/workbook.md`.

use serde::{Deserialize, Serialize};

use crate::edit::{Edit, Field};
use crate::layout::Node;
use crate::marks::{self, Extra, MAX_NAME, MarkInput, Need, PaneRecord, RecordKind, TabRecord};
use crate::model::{Effect, Kind, Model, PaneId, Section};
use crate::tasks::NOTICE_MS;

pub type WorkbookId = u32;

/// The `version` of a workbook's file this build writes. It reads 1 too: stage 1's, one tab.
pub const VERSION: u32 = 2;

/// The most tabs a workbook keeps.
pub const MAX_TABS: usize = 32;

/// The largest file a workbook may have: the shell reads no larger one. A save that would
/// write one is refused, so what is saved always reads back. Room for `MAX_BOOK_HISTORY` of
/// terminal history, as JSON.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

const TOO_LARGE: &str = "the workbook would be too large to read back";
const EMPTY: &str = "a tab with nothing in it has nothing to save";

/// Why a client's workbook action does nothing: they are the user's files and apps.
pub const USERS_ALONE: &str = "only the user saves, opens, renames or deletes a workbook";

/// What the shell read of its terminals at a save: each one's directory, and its history.
type Read<'a> = (&'a [(PaneId, String)], &'a [(PaneId, History)]);

/// The lines of a terminal's history a workbook keeps by default (`workbook_scrollback_lines`), and at most.
pub const HISTORY_LINES: usize = 500;
pub const MAX_HISTORY_LINES: usize = 2000;
/// The longest line kept, in characters, and the most bytes of one terminal's lines.
const MAX_LINE: usize = 1000;
const MAX_HISTORY_BYTES: usize = 64 * 1024;
/// The most bytes of history one workbook keeps, its terminals together: its file stays small.
pub const MAX_BOOK_HISTORY: usize = 1024 * 1024;
/// The longest command kept, in characters. A longer one is not kept: cut, it would run something else.
pub const MAX_COMMAND: usize = 1024;

/// What a terminal showed when its workbook was saved, and the program it ran then: kept in its
/// pane's record as `history`. Untrusted when read (another program may have written the
/// file): cleaned when kept, and again whenever it is used. Never in the state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub lines: Vec<String>,
    /// The command line of the program in front, e.g. "npm run dev". Typed on the prompt when
    /// the workbook opens, never run: the user presses ↵.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// When it was saved, in ms since 1970.
    #[serde(default)]
    pub saved: u64,
}

/// `text` without escape sequences (CSI, OSC, DCS and the like) and characters that do not
/// show; a tab is a space. e.g. "\x1b[31mred\x1b[0m" → "red".
fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters, then one final byte from @ to ~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC, DCS, SOS, PM, APC: to BEL or ESC \.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => out.push(' '),
            c if crate::capture::shows(c) => out.push(c),
            _ => {}
        }
    }
    out
}

impl History {
    /// As a workbook keeps it: the last `most` lines (at most `MAX_HISTORY_LINES`), each plain,
    /// cut to `MAX_LINE` and its secrets hidden, `MAX_HISTORY_BYTES` in all; and a command that is one plain line
    /// of at most `MAX_COMMAND` characters with nothing credential-shaped in it, else none.
    pub fn kept(&self, most: usize) -> History {
        let most = most.min(MAX_HISTORY_LINES);
        let lines = self.lines.iter().flat_map(|text| text.split(['\n', '\r']));
        let lines: Vec<String> = lines.map(|line| plain_text(line).chars().take(MAX_LINE).collect()).collect();
        // Credential-shaped text hidden, best effort (`secrets`).
        let lines = crate::secrets::hide_lines(lines);
        let mut lines: Vec<String> = lines[lines.len().saturating_sub(most)..].to_vec();
        while lines.iter().map(String::len).sum::<usize>() > MAX_HISTORY_BYTES {
            lines.remove(0);
        }
        let command = self.command.as_deref().map(str::trim).filter(|command| {
            !command.is_empty()
                && command.chars().count() <= MAX_COMMAND
                && command.chars().all(crate::capture::shows)
                && !crate::secrets::holds_secret(command)
        });
        History { lines, command: command.map(String::from), saved: self.saved }
    }

    fn bytes(&self) -> usize {
        self.lines.iter().map(String::len).sum::<usize>() + self.command.as_ref().map_or(0, String::len)
    }
}

/// A terminal record's history, as it is used: cleaned again. `None`: none, or none that reads.
pub fn history_of(pane: &PaneRecord) -> Option<History> {
    let kept: History = serde_json::from_value(pane.extra.get("history")?.clone()).ok()?;
    let kept = kept.kept(MAX_HISTORY_LINES);
    (pane.kind == RecordKind::Terminal && (!kept.lines.is_empty() || kept.command.is_some())).then_some(kept)
}

/// `ms` since 1970 as a minute of the time zone `offset` seconds east of UTC (the shell reads
/// it): "2026-11-13 15:02".
fn local_minute(ms: u64, offset: i64) -> String {
    let at = (ms / 1000) as i64 + offset;
    let (days, secs) = (at.div_euclid(86_400), at.rem_euclid(86_400));
    let (year, month, day) = crate::ics::civil(days);
    format!("{year}-{month:02}-{day:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// What the shell writes into a restored terminal's view, before its shell starts: the saved
/// lines in grey, then a divider that says when, and what is typed on the prompt below it.
/// Written to the view, never to the shell: nothing of it runs. `offset`: the local time zone's,
/// in seconds east of UTC, for the date.
pub fn history_view(history: &History, offset: i64) -> Vec<u8> {
    let history = history.kept(MAX_HISTORY_LINES);
    let mut text = String::new();
    for line in &history.lines {
        text.push_str(&format!("\x1b[90m{line}\x1b[0m\r\n"));
    }
    let typed = history.command.as_ref().map(|command| format!(" · on the prompt, ↵ runs: {command}"));
    let divider = format!("── saved {}{} ──", local_minute(history.saved, offset), typed.unwrap_or_default());
    text.push_str(&format!("\x1b[90m{divider}\x1b[0m\r\n"));
    text.into_bytes()
}

/// A workbook's histories fitted: to `MAX_BOOK_HISTORY`, then the last ones dropped until its
/// file is small enough to read back (JSON adds to each line). A file too large without any
/// history is still too large: `sized` refuses it.
fn fit(book: &mut Workbook) {
    fit_history(&mut book.tabs);
    while sized(book).is_err() {
        let held = book
            .tabs
            .iter_mut()
            .flat_map(|tab| tab.panes.iter_mut())
            .rev()
            .find(|pane| pane.extra.contains_key("history"));
        match held {
            Some(pane) => drop(pane.extra.remove("history")),
            None => break,
        }
    }
}

/// Each terminal's history in `tabs`, cleaned, until `MAX_BOOK_HISTORY` is spent: later ones keep none.
pub(crate) fn fit_history(tabs: &mut [TabRecord]) {
    let mut left = MAX_BOOK_HISTORY;
    for pane in tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
        let Some(history) = history_of(pane) else {
            pane.extra.remove("history");
            continue;
        };
        match history.bytes() <= left {
            true => {
                left -= history.bytes();
                pane.extra.insert("history".into(), serde_json::json!(history));
            }
            false => drop(pane.extra.remove("history")),
        }
    }
}

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
    // Stage 1 never saved a history: one in such a file was put there by something else.
    if book.version == 1 {
        book.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()).for_each(|pane| drop(pane.extra.remove("history")));
    }
    book.version = VERSION;
    fit_history(&mut book.tabs);
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
    crate::pretty_json(book)
}

/// `Ok` if `book`'s file reads back: no larger than `MAX_FILE_BYTES`, with room for its id and a
/// name of `MAX_NAME` characters it may not have yet.
fn sized(book: &Workbook) -> Result<(), String> {
    (write(book).len() + 256 <= MAX_FILE_BYTES).then_some(()).ok_or_else(|| TOO_LARGE.into())
}

impl Workbook {
    fn new(uid: String, name: String, tabs: Vec<TabRecord>) -> Self {
        Workbook { id: 0, version: VERSION, uid, name, tabs, live: Vec::new(), extra: Extra::new() }
    }

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

    pub(crate) fn set_live(&mut self, at: usize, tab: Option<u32>) {
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
    /// A terminal's saved history: how many lines, and whether a command waits on its prompt.
    /// Counts only: the text is never in the state.
    #[serde(skip_serializing_if = "is_zero")]
    pub lines: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_command: bool,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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
    // Counted where it is kept, cleaned when it was saved or read: rows are drawn every frame.
    let history = pane.extra.get("history");
    let lines = history.and_then(|history| history.get("lines")).and_then(|lines| lines.as_array()).map_or(0, Vec::len);
    let has_command = history.and_then(|history| history.get("command")).is_some_and(|command| command.is_string());
    match &pane.kind {
        RecordKind::Terminal => RowPane { kind: "terminal", app: None, lines, has_command },
        RecordKind::App { name, .. } => RowPane { kind: "app", app: Some(name.clone()), lines: 0, has_command: false },
        RecordKind::Empty => RowPane { kind: "empty", app: None, lines: 0, has_command: false },
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
            // Saved terminal text is never a surprise: the row says it holds some.
            let held = panes.iter().any(|pane| pane.lines > 0 || pane.has_command);
            let summary = if held { format!("{summary} · history") } else { summary };
            let tab = tabs.iter().find_map(|tab| tab.tab);
            let (id, name, unfolded) = (book.id, book.name.clone(), self.workbooks.unfolded(book.id));
            WorkbookRow { id, name, panes, summary, open: tab.is_some(), tab, tabs, unfolded }
        };
        self.workbooks.list.iter().map(row).collect()
    }

    /// What the last save kept, for the status bar: `Saved "DRI": 2 tabs, 500 lines of history,
    /// 1 command`. Of the tab on show's workbook, else of the one whose name is being typed.
    pub fn workbook_saved_note(&self) -> Option<String> {
        let typing = match &self.mark_input {
            Some(MarkInput::Workbook { workbook, .. }) => Some(*workbook),
            _ => None,
        };
        let book = self.workbooks.get(self.workbook_of_tab().or(typing)?)?;
        let histories: Vec<History> =
            book.tabs.iter().flat_map(|tab| tab.panes.iter()).filter_map(history_of).collect();
        let lines: usize = histories.iter().map(|history| history.lines.len()).sum();
        let commands = histories.iter().filter(|history| history.command.is_some()).count();
        let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let tabs = count(book.tabs.len(), "tab", "tabs");
        let lines = count(lines, "line of history", "lines of history");
        Some(format!("Saved \"{}\": {tabs}, {lines}, {}", book.name, count(commands, "command", "commands")))
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
    /// `histories`: what each terminal showed, and ran, as the shell read it.
    fn workbook_record(
        &self,
        index: usize,
        cwds: &[(PaneId, String)],
        histories: &[(PaneId, History)],
    ) -> Result<TabRecord, String> {
        let record = self.tab_record(index, cwds, histories).ok_or("no tab to save")?;
        if record.panes.iter().all(|pane| pane.kind == RecordKind::Empty) {
            return Err(EMPTY.into());
        }
        match record.unsound() {
            Some(wrong) => Err(format!("this tab cannot be saved: {wrong}")),
            None => Ok(record),
        }
    }

    /// The tab at `index` as a record of no task, whatever is in it: what a workbook and the
    /// layout file keep of a tab. No window title, no keys a waiting pane kept from its file.
    pub(crate) fn tab_record(
        &self,
        index: usize,
        cwds: &[(PaneId, String)],
        histories: &[(PaneId, History)],
    ) -> Option<TabRecord> {
        let tab = self.tabs.get(index)?;
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
            let history =
                histories.iter().find(|(id, _)| *id == pane.id).map(|(_, history)| history.kept(MAX_HISTORY_LINES));
            if let Some(history) = history.filter(|_| pane.kind == RecordKind::Terminal) {
                pane.extra.insert("history".into(), serde_json::json!(history));
            }
        }
        let title = marks::kept_title(&tab.title, 256);
        Some(TabRecord { title, task: None, focused, layout, panes, extra: Extra::new() })
    }

    /// Why the tab on show cannot be saved, for the shell to say.
    pub fn workbook_save_refusal(&self) -> Option<String> {
        let record = match self.workbook_record(self.active, &[], &[]) {
            Ok(record) => record,
            Err(why) => return Some(why),
        };
        let tab = self.tabs.get(self.active)?;
        match self.workbooks.entry_of(tab.id) {
            Some((id, at)) => self.workbook_with(id, Some(at), record).err(),
            None => {
                let mut book = Workbook::new(String::new(), tab.title.clone(), vec![record]);
                fit(&mut book);
                sized(&book).err()
            }
        }
    }

    /// Why the tab on show cannot be added to workbook `id`, for the shell to say.
    pub fn workbook_add_refusal(&self, id: WorkbookId) -> Option<String> {
        let tab = self.tabs.get(self.active).map(|tab| tab.id)?;
        let at = self.workbooks.get(id)?.place_of(tab);
        match self.workbook_record(self.active, &[], &[]) {
            Ok(record) => self.workbook_with(id, at, record).err(),
            Err(why) => Some(why),
        }
    }

    /// Why the tabs on the bar cannot be saved as a workbook, for the shell to say.
    pub fn workbook_save_all_refusal(&self) -> Option<String> {
        self.workbook_all_records((&[], &[])).err()
    }

    /// Each tab on the bar as a workbook keeps it, in bar order, but those with nothing in them.
    /// Refused whole rather than saved in part: more than `MAX_TABS`, or one that cannot be kept.
    fn workbook_all_records(&self, read: Read) -> Result<Vec<(u32, TabRecord)>, String> {
        let mut tabs = Vec::new();
        for at in 0..self.tabs.len() {
            match self.workbook_record(at, read.0, read.1) {
                Ok(record) => tabs.push((self.tabs[at].id, record)),
                Err(why) if why == EMPTY => {}
                Err(why) => return Err(format!("{}: {why}", self.tabs[at].title)),
            }
        }
        match tabs.len() {
            0 => Err("no tab on the bar has anything to save".into()),
            n if n > MAX_TABS => Err(format!("{n} tabs on the bar; a workbook keeps {MAX_TABS} at most")),
            _ => {
                let mut book =
                    Workbook::new(String::new(), String::new(), tabs.iter().map(|(_, r)| r.clone()).collect());
                fit(&mut book);
                sized(&book).map(|_| tabs)
            }
        }
    }

    /// Workbook `id` with its tab at `at` (`None`: a new one at the end) replaced by `record`,
    /// or why it cannot be.
    fn workbook_with(&self, id: WorkbookId, at: Option<usize>, record: TabRecord) -> Result<Workbook, String> {
        let mut book = self.workbooks.get(id).cloned().ok_or("no such workbook")?;
        match at.filter(|at| *at < book.tabs.len()) {
            Some(at) => {
                // The tab's keys this build does not know stay.
                let kept = &mut book.tabs[at];
                *kept = TabRecord { extra: std::mem::take(&mut kept.extra), ..record };
            }
            None if book.tabs.len() < MAX_TABS => {
                // Its first tab opened under the workbook's name, and goes on doing so.
                if let [only] = book.tabs.as_mut_slice() {
                    only.title = book.name.clone();
                }
                book.tabs.push(record);
            }
            None => return Err(format!("a workbook keeps {MAX_TABS} tabs at most")),
        }
        fit(&mut book);
        sized(&book).map(|_| book)
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
        let (Some(before), Ok(mut after)) = (self.workbooks.get(id).cloned(), self.workbook_with(id, at, record))
        else {
            return;
        };
        // A tab stands for one workbook's tab at most.
        self.workbooks.forget_tab(tab);
        after.live = self.workbooks.get(id).map(|book| book.live.clone()).unwrap_or_default();
        after.set_live(at.filter(|at| *at < before.tabs.len()).unwrap_or(after.tabs.len() - 1), Some(tab));
        if let Some(book) = self.workbooks.get_mut(id) {
            *book = after;
        }
        self.workbooks.offer = Some(Offer { change: Change::Updated, before, until });
    }

    /// Saves the tab on show: updates its tab of the workbook it came from (the old one is
    /// offered back), else adds a workbook of that tab, named for it, and starts typing its
    /// name. `uid`: a new one's file name.
    pub(crate) fn workbook_save(&mut self, uid: String, read: Read, effects: &mut Vec<Effect>) {
        let Ok(record) = self.workbook_record(self.active, read.0, read.1) else { return };
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
    pub(crate) fn workbook_add_tab(&mut self, id: WorkbookId, read: Read) {
        let Ok(record) = self.workbook_record(self.active, read.0, read.1) else { return };
        let Some(tab) = self.tabs.get(self.active).map(|tab| tab.id) else { return };
        let Some(at) = self.workbooks.get(id).map(|book| book.place_of(tab)) else { return };
        self.workbook_put(id, at, record, tab);
        self.show_workbook(id);
    }

    /// Saves every tab on the bar that has something in it as one new workbook, in bar order.
    pub(crate) fn workbook_save_all(&mut self, uid: String, read: Read, effects: &mut Vec<Effect>) {
        let Ok(tabs) = self.workbook_all_records(read) else { return };
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
        let live = tabs.iter().map(|(tab, _)| Some(*tab)).collect();
        let mut book =
            Workbook { live, ..Workbook::new(uid, name, tabs.iter().map(|(_, record)| record.clone()).collect()) };
        fit(&mut book);
        sized(&book).ok()?;
        for (tab, _) in &tabs {
            self.workbooks.forget_tab(*tab);
        }
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
            let base = self.next_id;
            let tab = self.rebuild_tab(&record, effects);
            // Each terminal that started shows its history, in panes numbered from `base` in reading order.
            for (pane, kept) in (base + 1..).zip(record.shown()) {
                let started = self.pane(pane).is_some_and(|open| open.kind == Kind::Terminal);
                if let Some(history) = history_of(kept).filter(|_| started) {
                    effects.push(Effect::History { pane, history });
                }
            }
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
        m.update(Action::WorkbookSave { uid: uid.into(), cwds, histories: Vec::new() })
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
        vec![
            Effect::Opened(term),
            Effect::HostWindow { mark: None, bundle, name, title, into: Some(app), axis: Right, launch: true },
        ]
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
                Action::WorkbookSave { uid: UID_2.into(), cwds: vec![], histories: Vec::new() },
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
        m.update(Action::WorkbookSaveAll {
            uid: UID.into(),
            cwds: vec![(1, "/code/app".into()), (3, "/logs".into())],
            histories: Vec::new(),
        });
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
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![], histories: Vec::new() });
        assert_eq!(m.workbooks.list[0].tabs.len(), 1);
        m.update(Action::CloseTab { index: None });
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![(4, "/logs".into())], histories: Vec::new() });
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
        m.update(Action::WorkbookSave { uid: UID_2.into(), cwds: vec![], histories: Vec::new() });
        m.update(Action::MarkCancel);
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![], histories: Vec::new() });
        let lives: Vec<Vec<Option<u32>>> = m.workbooks.list.iter().map(|book| book.live.clone()).collect();
        assert_eq!(lives, [vec![Some(1), Some(2)], vec![None]]);
        // A workbook is full at MAX_TABS.
        m.workbooks.list[0].tabs = vec![m.workbooks.list[0].tabs[0].clone(); MAX_TABS];
        m.update(Action::NewTab);
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        let full = format!("a workbook keeps {MAX_TABS} tabs at most");
        assert_eq!(m.workbook_add_refusal(1), Some(full));
        m.update(Action::WorkbookAddTab { workbook: 1, cwds: vec![], histories: Vec::new() });
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

    fn history(lines: &[&str], command: Option<&str>) -> History {
        History {
            lines: lines.iter().map(|line| line.to_string()).collect(),
            command: command.map(String::from),
            saved: 0,
        }
    }

    #[test]
    fn saved_text_is_plain_and_capped_and_a_command_is_one_line_or_none() {
        // Escape sequences, controls and characters that draw nothing go; a tab is a space.
        let dirty =
            history(&["\x1b[31mred\x1b[0m\tok", "\x1b]0;title\x07after", "a\u{202e}b\u{7}c\x1b]8;;u\x1b\\link"], None);
        assert_eq!(dirty.kept(500).lines, ["red ok", "after", "abclink"]);
        // A line break inside a line is two lines; the last `most` lines are kept.
        let many = history(&["1\n2\r3", "4"], None);
        assert_eq!(many.kept(2).lines, ["3", "4"]);
        assert_eq!(many.kept(0).lines, Vec::<String>::new());
        assert_eq!(many.kept(usize::MAX).lines.len(), 4);
        // Lines are cut, and the oldest go past the byte cap.
        let long = "x".repeat(5000);
        let kept = history(&vec![long.as_str(); 200], None).kept(MAX_HISTORY_LINES);
        assert!(kept.lines.iter().all(|line| line.len() == MAX_LINE) && kept.bytes() <= MAX_HISTORY_BYTES);
        // A command of two lines, of controls, too long or blank is not kept; one line is, trimmed.
        for wrong in ["make\nrm -rf ~", "ls\x1b[2J", "a\u{202e}b", " ", &"y".repeat(MAX_COMMAND + 1)] {
            assert_eq!(history(&[], Some(wrong)).kept(500).command, None, "{wrong:?}");
        }
        assert_eq!(history(&[], Some(" npm run dev ")).kept(500).command.as_deref(), Some("npm run dev"));
    }

    #[test]
    fn the_view_shows_the_lines_and_a_divider_naming_the_command_and_holds_no_escape_of_the_file() {
        let saved = History { saved: 1_763_042_520_000, ..history(&["$ make", "\x1b[2Jbuilt"], Some("npm run dev")) };
        let view = String::from_utf8(history_view(&saved, 3600)).unwrap();
        assert_eq!(
            view,
            "\x1b[90m$ make\x1b[0m\r\n\x1b[90mbuilt\x1b[0m\r\n\
             \x1b[90m── saved 2025-11-13 15:02 · on the prompt, ↵ runs: npm run dev ──\x1b[0m\r\n"
        );
        // Only its own colour codes: nothing that clears, moves or titles.
        assert!(!view.replace("\x1b[90m", "").replace("\x1b[0m", "").contains('\x1b'));
        assert_eq!(
            (local_minute(0, 0), local_minute(951_782_400_000, 0), local_minute(0, -3600)),
            ("1970-01-01 00:00".into(), "2000-02-29 00:00".into(), "1969-12-31 23:00".into())
        );
    }

    #[test]
    fn a_terminals_history_is_saved_with_its_pane_and_shown_when_it_opens_and_only_counted_in_the_state() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::Open { kind: slack(7), axis: Right, beside: None });
        // Slack's pane has no history, whatever the shell sends.
        let histories =
            vec![(1, history(&["secret-token-1", "$ "], Some("sleep 293"))), (2, history(&["x"], Some("y")))];
        m.update(Action::WorkbookSave { uid: UID.into(), cwds: vec![(1, "/code/app".into())], histories });
        m.update(Action::MarkCancel);
        let text = write(&m.workbooks.list[0]);
        assert!(text.contains("secret-token-1") && text.contains("sleep 293") && !text.contains("\"y\""), "{text}");
        let rows = serde_json::to_string(&m.workbook_rows()).unwrap();
        assert!(!rows.contains("secret") && !rows.contains("sleep"), "{rows}");
        let pane = &m.workbook_rows()[0].panes[0];
        assert_eq!((pane.lines, pane.has_command), (2, true));
        assert!(rows.contains("\"lines\":2,\"has_command\":true"), "{rows}");
        // Opened again, its terminal starts, then shows the history; nothing is typed by the core.
        m.update(Action::CloseTab { index: None });
        let effects = m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None });
        let shown = history(&["secret-token-1", "$ "], Some("sleep 293"));
        assert_eq!(
            (&effects[0], effects.last()),
            (&Effect::Opened(3), Some(&Effect::History { pane: 3, history: shown }))
        );
        assert!(!serde_json::to_string(&m).unwrap().contains("secret"));
        // A setting of 0 lines: the shell sends none, and nothing is kept.
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::WorkbookSave { uid: UID.into(), cwds: vec![], histories: vec![] });
        assert!(!write(&m.workbooks.list[0]).contains("history"));
    }

    #[test]
    fn a_history_in_a_stage_1_file_is_dropped_and_one_from_another_program_is_cleaned() {
        let file = |version: u32| {
            format!(
                r#"{{"version": {version}, "id": "{UID}", "name": "x", "tabs": [{{"title": "t", "layout": {{"leaf": 1}},
                "panes": [{{"id": 1, "kind": {{"type": "terminal"}}, "history": {{"lines": ["\u001b[2Jhi"], "command": "rm -rf ~\nls"}}}}]}}]}}"#
            )
        };
        let old = parse(&file(1), UID).expect("it reads");
        assert_eq!(history_of(&old.tabs[0].panes[0]), None);
        let new = parse(&file(2), UID).expect("it reads");
        assert_eq!(history_of(&new.tabs[0].panes[0]), Some(history(&["hi"], None)));
        assert!(!write(&new).contains("rm -rf"));
    }

    #[test]
    fn a_workbook_keeps_so_much_history_in_all_and_later_terminals_keep_none() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        for _ in 0..19 {
            m.update(Action::Split { axis: Down, kind: Kind::Terminal });
        }
        let line = "z".repeat(MAX_LINE);
        let full = history(&vec![line.as_str(); 64], None);
        let histories = (1..=20).map(|pane| (pane, full.clone())).collect();
        m.update(Action::WorkbookSave { uid: UID.into(), cwds: vec![], histories });
        let lines: Vec<usize> = m.workbook_rows()[0].panes.iter().map(|pane| pane.lines).collect();
        let kept: usize = m.workbooks.list[0].tabs[0].panes.iter().filter_map(history_of).map(|h| h.bytes()).sum();
        assert!(kept <= MAX_BOOK_HISTORY && lines[0] == 64 && lines[19] == 0, "{lines:?}");
    }

    #[test]
    fn the_row_says_a_workbook_holds_history_and_the_save_says_how_much() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        let histories = vec![(1, history(&["a", "export API_TOKEN=s3cr3t", "b"], Some("psql postgres://u:pw@db")))];
        m.update(Action::WorkbookSave { uid: UID.into(), cwds: vec![], histories });
        assert_eq!(m.workbook_saved_note().as_deref(), Some("Saved \"Tab 1\": 1 tab, 3 lines of history, 0 commands"));
        assert_eq!(m.workbook_rows()[0].summary, "1 terminal · history");
        // The secret is hidden in the line, and a command holding one is not kept at all.
        let text = write(&m.workbooks.list[0]);
        assert!(text.contains("API_TOKEN=[hidden]") && !text.contains("s3cr3t") && !text.contains("psql"), "{text}");
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::WorkbookSave {
            uid: UID.into(),
            cwds: vec![],
            histories: vec![(1, history(&[], Some("k9s")))],
        });
        assert_eq!(m.workbook_saved_note().as_deref(), Some("Saved \"Tab 1\": 1 tab, 0 lines of history, 1 command"));
    }

    #[test]
    fn save_all_refuses_rather_than_leave_a_tab_out_and_no_save_writes_a_file_too_large_to_read() {
        let mut m = Model::default();
        for _ in 0..=MAX_TABS {
            m.update(Action::NewTab);
            m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        }
        let refused = Some(format!("{} tabs on the bar; a workbook keeps {MAX_TABS} at most", MAX_TABS + 1));
        assert_eq!(m.workbook_save_all_refusal(), refused);
        m.update(Action::WorkbookSaveAll { uid: UID.into(), cwds: vec![], histories: Vec::new() });
        assert_eq!(names(&m), Vec::<&str>::new());
        // Long directories in many panes: the file would pass the reader's cap. Not saved.
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        for _ in 0..39 {
            m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        }
        let dir = format!("/{}", "d".repeat(1000));
        let cwds: Vec<(PaneId, String)> = (1..=40).map(|pane| (pane, dir.clone())).collect();
        m.update(Action::WorkbookSave { uid: UID.into(), cwds: cwds.clone(), histories: Vec::new() });
        let first = write(&m.workbooks.list[0]).len();
        assert!(first < MAX_FILE_BYTES, "{first}");
        // Added again and again under the cap, then refused with the reason.
        let mut added = 1;
        while m.workbook_add_refusal(1).is_none() && added < MAX_TABS {
            m.update(Action::NewTab);
            for pane in 0..40 {
                let kind = Kind::Terminal;
                if pane == 0 {
                    m.update(Action::Open { kind, axis: Right, beside: None });
                } else {
                    m.update(Action::Split { axis: Right, kind });
                }
            }
            let ids: Vec<PaneId> = m.layout.as_ref().map(Node::leaves).unwrap_or_default();
            let cwds: Vec<(PaneId, String)> = ids.iter().map(|id| (*id, dir.clone())).collect();
            m.update(Action::WorkbookAddTab { workbook: 1, cwds, histories: Vec::new() });
            added += 1;
        }
        // With room for history, the tab cap comes first: the file still reads back.
        assert_eq!(m.workbooks.list[0].tabs.len(), MAX_TABS);
        assert!(write(&m.workbooks.list[0]).len() <= MAX_FILE_BYTES);
    }
}
