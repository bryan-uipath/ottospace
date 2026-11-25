//! A row's menu: … on a row of the left bar opens a short list of what may be done to it. One
//! at a time, the user's alone; any other move of the user's closes it, and so does its … moving
//! (the list under it changed). It holds the keyboard: ↓ ↑ move, ↵ takes, any other key closes.
//! No item has a letter: a key meant for what is behind the menu must never delete.
//! Today a workspace's row and its tabs' rows have one, a temporary workspace's item on the top
//! row (Keep), the drawer's transcript header (its ⋯: Pause or Resume), and a linked worktree's
//! row of the Repos section (a right click: Delete worktree…), and the Repos section's + (Local
//! folder…, From GitHub…), and an app pane's or a slot's ⋯ (`slots` on: Create slot…, or Edit
//! slot…, Offer again, Give back the window and Clear slot); another row gets one by naming its
//! items in `row_items`.

use serde::Serialize;

use crate::features::Feature;
use crate::layout::Rect;
use crate::list::ListRow;
use crate::marks::MarkInput;
use crate::model::{Action, HOME_TAB, Kind, Model, Owner};
use crate::workbook::WorkbookId;

/// Why a client's request that takes the keyboard waits: the user's keys are in a row's menu.
pub const MENU_OPEN: &str = "a row's menu is open";

/// What a row's menu offers. In the state by its name (`Item::name`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Rename,
    Delete,
    /// A tab out of its workspace.
    Remove,
    /// A temporary workspace kept.
    Keep,
    /// The transcript on: its streams stop, or start again.
    Pause,
    Resume,
    /// A linked worktree's directory, after a question (`Confirm::delete`). Its branch stays.
    DeleteWorktree,
    /// The Repos section's +: the folder picker, as before; or a repository cloned from GitHub.
    LocalFolder,
    FromGitHub,
    /// An app pane's ⋯: the slot dialog, to keep this place for its app's windows.
    SlotKeep,
    /// A slot's ⋯: the dialog on its rule; the windows passed over offered again (while there
    /// are any); its window given back (while filled); or Clear slot.
    SlotEdit,
    SlotLookAgain,
    SlotGiveBack,
    /// Swap with…: another slot of its app on the tab on show, by pane.
    SlotSwap(crate::model::PaneId),
    /// Re-sort by size: its app's slots there pair their windows again.
    SlotResort,
    SlotClear,
    /// A tab closed, as its × does; or folded out to another display (`multi_monitor` on).
    Close,
    FoldOut,
    /// A Repos row: an explorer of that checkout.
    ExplorerHere,
    /// An explorer's row: the file in the viewer beside, or below; shown in Finder; its full
    /// path copied; a terminal in the folder.
    Open,
    /// A changed file's diff in the viewer beside (`review`).
    OpenDiff,
    OpenBelow,
    Reveal,
    CopyPath,
    TerminalHere,
}

impl Serialize for Item {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl Item {
    /// Its name in the state: `slot_swap`.
    pub fn name(self) -> &'static str {
        match self {
            Item::Rename => "rename",
            Item::Delete => "delete",
            Item::Remove => "remove",
            Item::Keep => "keep",
            Item::Pause => "pause",
            Item::Resume => "resume",
            Item::DeleteWorktree => "delete_worktree",
            Item::LocalFolder => "local_folder",
            Item::FromGitHub => "from_github",
            Item::SlotKeep => "slot_keep",
            Item::SlotEdit => "slot_edit",
            Item::SlotLookAgain => "slot_look_again",
            Item::SlotGiveBack => "slot_give_back",
            Item::SlotSwap(_) => "slot_swap",
            Item::SlotResort => "slot_resort",
            Item::SlotClear => "slot_clear",
            Item::Close => "close",
            Item::FoldOut => "fold_out",
            Item::ExplorerHere => "explorer_here",
            Item::Open => "open",
            Item::OpenDiff => "open_diff",
            Item::OpenBelow => "open_below",
            Item::Reveal => "reveal",
            Item::CopyPath => "copy_path",
            Item::TerminalHere => "terminal_here",
        }
    }

    /// Its words with no title: Swap's other slot by pane (`Model::menu_label` draws what it holds).
    pub fn label(self) -> &'static str {
        match self {
            Item::Rename => "Rename",
            Item::Delete => "Delete",
            Item::Remove => "Remove",
            Item::Keep => "Keep",
            Item::Pause => "Pause",
            Item::Resume => "Resume",
            Item::DeleteWorktree => "Delete worktree…",
            Item::LocalFolder => "Local folder…",
            Item::FromGitHub => "From GitHub…",
            Item::SlotKeep => "Create slot…",
            Item::SlotEdit => "Edit slot…",
            Item::SlotLookAgain => "Offer again",
            Item::SlotGiveBack => "Give back the window",
            Item::SlotSwap(_) => "Swap with…",
            Item::SlotResort => "Re-sort by size",
            Item::SlotClear => "Clear slot",
            Item::Close => "Close",
            Item::FoldOut => "Fold out",
            Item::ExplorerHere => "Open in explorer",
            Item::Open => "Open",
            Item::OpenDiff => "Open diff",
            Item::OpenBelow => "Open below",
            Item::Reveal => "Reveal in Finder",
            Item::CopyPath => "Copy path",
            Item::TerminalHere => "Open terminal here",
        }
    }
}

/// Whose menu it is: a row of the bar's, a workspace's item on the top row, or the drawer's
/// transcript header's.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuOf {
    Row(ListRow),
    Space(WorkbookId),
    Transcript,
    /// The Repos section's +.
    ReposAdd,
    /// An app pane's or a slot's ⋯ (`slots` on).
    Pane(crate::model::PaneId),
    /// A tab of the tab row, by its id: a right click on it.
    Tab(u32),
    /// A row of explorer `pane`, by its path from the root: a right click on it.
    Explorer(crate::model::PaneId, String),
}

impl From<ListRow> for MenuOf {
    fn from(row: ListRow) -> Self {
        MenuOf::Row(row)
    }
}

/// The open menu: whose row, the item the keys are on (none until a key moves, when a click
/// opened it), and where its … was drawn when it opened.
#[derive(Clone, Debug, PartialEq)]
pub struct RowMenu {
    pub row: MenuOf,
    pub selected: Option<usize>,
    pub dots: Option<Rect>,
    /// Its items when it opened: a change under it (a slot's Offer again going) closes it, so
    /// the keys' place never lands on another item.
    pub items: Vec<Item>,
}

/// What a row's menu offers, in order. None: the row has no menu.
pub fn row_items(row: &ListRow) -> &'static [Item] {
    match row {
        ListRow::Workbook(_) => &[Item::Rename, Item::Delete],
        ListRow::WorkbookTab(..) => &[Item::Rename, Item::Remove],
        _ => &[],
    }
}

/// What a key does while a menu is open: ↓ ↑ move, ↵ takes the item the keys are on, any other
/// key closes the menu and does nothing else.
pub fn menu_key(key: &str) -> Action {
    match key {
        "down" => Action::RowMenuMove { delta: 1 },
        "up" => Action::RowMenuMove { delta: -1 },
        "enter" => Action::RowMenuChoose { index: None },
        _ => Action::RowMenuClose,
    }
}

impl Model {
    /// The open menu and its items.
    pub fn row_menu(&self) -> Option<(&RowMenu, Vec<Item>)> {
        self.row_menu.as_ref().map(|menu| (menu, self.menu_items(&menu.row)))
    }

    /// What a menu offers now: a row's by its kind; the transcript's Pause while it runs, Resume
    /// while paused, none while it starts or with none shown.
    pub fn menu_items(&self, of: &MenuOf) -> Vec<Item> {
        let items: &[Item] = match of {
            // A temporary workspace offers Keep.
            MenuOf::Row(ListRow::Workbook(id)) if !self.workbooks.is_kept(*id) => {
                &[Item::Rename, Item::Keep, Item::Delete]
            }
            // Any checkout: its explorer; a linked worktree, never a repository's own folder: delete it.
            MenuOf::Row(ListRow::Repo(path)) if self.linked_worktree(path).is_some() => {
                &[Item::DeleteWorktree, Item::ExplorerHere]
            }
            MenuOf::Row(ListRow::Repo(_)) => &[Item::ExplorerHere],
            // A folder; a file; a link (shown, never read through).
            MenuOf::Explorer(pane, path) => {
                let row = self.explorer(*pane).and_then(|e| e.rows().into_iter().find(|row| row.path == *path));
                match row {
                    Some(row) if row.link => &[Item::CopyPath],
                    Some(row) if row.dir => &[Item::TerminalHere, Item::Reveal, Item::CopyPath],
                    // A changed file (`review`): its diff too.
                    Some(row) if self.features.on(Feature::Review) && crate::explorer::diffable(&row) => {
                        &[Item::Open, Item::OpenDiff, Item::OpenBelow, Item::Reveal, Item::CopyPath]
                    }
                    Some(_) => &[Item::Open, Item::OpenBelow, Item::Reveal, Item::CopyPath],
                    None => &[],
                }
            }
            MenuOf::Row(row) => row_items(row),
            // On the top row: Rename, and Keep while it is temporary.
            MenuOf::Space(id) if !self.workbooks.is_kept(*id) => &[Item::Rename, Item::Keep],
            MenuOf::Space(_) => &[Item::Rename],
            // A tab out has its Fold back in where its × is.
            MenuOf::Tab(id) if self.features.on(Feature::MultiMonitor) && !self.is_out(*id) => {
                &[Item::Rename, Item::FoldOut, Item::Close]
            }
            MenuOf::Tab(_) => &[Item::Rename, Item::Close],
            MenuOf::ReposAdd => &[Item::LocalFolder, Item::FromGitHub],
            MenuOf::Pane(_) if !self.features.on(Feature::Slots) => &[],
            // A slot's acts, with Swap and Re-sort among its app's; an app pane's: keep it as one.
            MenuOf::Pane(id) if self.slots.contains_key(id) => return self.slot_items(*id),
            MenuOf::Pane(id) => match self.pane(*id).map(|pane| &pane.kind) {
                Some(Kind::App { .. }) => &[Item::SlotKeep],
                _ => &[],
            },
            // One decision with the palette's and the menu bar's (`listen_pause_action`).
            MenuOf::Transcript => match self.listen_pause_action(0).filter(|_| self.listen.shown) {
                Some(Action::ListenPause { .. }) => &[Item::Pause],
                Some(Action::ListenResume { .. }) => &[Item::Resume],
                _ => &[],
            },
        };
        items.to_vec()
    }

    /// A slot's menu: Edit slot…, Offer again (while it passed over any), Give back the window
    /// (while filled), Swap with… each other slot of its app on the tab on show (one of the two
    /// holds a window), Re-sort by size (when it can: `Model::slot_resorts`), Clear slot.
    fn slot_items(&self, id: crate::model::PaneId) -> Vec<Item> {
        let filled = |id| matches!(self.pane(id).map(|pane| &pane.kind), Some(Kind::App { .. }));
        let passed = self.slots.get(&id).is_some_and(|run| !run.declined.is_empty());
        let group = self.slot_group(id);
        let mut items = vec![Item::SlotEdit];
        items.extend(passed.then_some(Item::SlotLookAgain));
        items.extend(filled(id).then_some(Item::SlotGiveBack));
        let others = group.iter().copied().filter(|other| *other != id && (filled(id) || filled(*other)));
        items.extend(others.map(Item::SlotSwap));
        items.extend(self.slot_resorts(id).then_some(Item::SlotResort));
        items.push(Item::SlotClear);
        items
    }

    /// An item's words as drawn. Swap names what the other slot holds (its window's title: drawn,
    /// never in the state), or `Swap` alone with two slots of the app; Re-sort names the app.
    pub fn menu_label(&self, item: Item) -> String {
        let Some((menu, items)) = self.row_menu() else { return item.label().into() };
        let swaps = items.iter().filter(|item| matches!(item, Item::SlotSwap(_))).count();
        let app = match menu.row {
            MenuOf::Pane(id) => self.slots.get(&id).map(|run| run.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        match item {
            Item::SlotSwap(_) if swaps == 1 => "Swap".into(),
            Item::SlotSwap(other) => match self.pane(other).map(|pane| &pane.kind) {
                Some(Kind::App { title, .. }) => format!("Swap with “{title}”"),
                _ => "Swap with the empty slot".into(),
            },
            Item::SlotResort => format!("Re-sort {app} windows by size"),
            item => item.label().into(),
        }
    }

    /// Whether `of`'s menu can be open: its row is listed, or the transcript is shown; and it offers something.
    fn menu_there(&self, of: &MenuOf) -> bool {
        let there = match of {
            MenuOf::Row(row) => self.list_rows().contains(row),
            MenuOf::Space(id) => self.shown_spaces().iter().any(|space| space.of == Owner::Workbook { id: *id }),
            MenuOf::Transcript => true,
            // `OTTOSPACE_REPOS` names the list: nothing is added to it.
            MenuOf::ReposAdd => self.sidebar == Some(crate::model::Section::Repos) && !self.repo_dirs.fixed,
            // On the tab on show.
            MenuOf::Pane(id) => self.layout.as_ref().is_some_and(|layout| layout.leaves().contains(id)),
            // A row of an explorer on show.
            MenuOf::Explorer(pane, _) => self.layout.as_ref().is_some_and(|layout| layout.leaves().contains(pane)),
            // On the row of the space on show; never Home's own, nor Decisions'.
            MenuOf::Tab(id) => self.space_tabs(self.shown_space()).iter().any(|at| {
                self.tabs
                    .get(*at)
                    .is_some_and(|tab| tab.id == *id && tab.id != HOME_TAB && tab.space != Owner::Decisions)
            }),
        };
        there && !self.menu_items(of).is_empty()
    }

    /// … on a row: its menu, the keys on its first item if a key opened it (the +'s always: ↵
    /// takes Local folder…, as the + did). On the row whose menu is open: closed. `dots`: where the … is drawn.
    pub(crate) fn row_menu_open(&mut self, row: MenuOf, dots: Option<Rect>, keys: bool) {
        let open = self.row_menu.take().is_some_and(|menu| menu.row == row);
        let listed = self.menu_there(&row);
        let selected = (keys || row == MenuOf::ReposAdd).then_some(0);
        let items = self.menu_items(&row);
        self.row_menu = (!open && listed).then_some(RowMenu { row, selected, dots, items });
    }

    /// The … of the open menu as last drawn. Elsewhere than when it opened (a row above came or
    /// went, the window changed), or not drawn: the menu closes, rather than hang under another row.
    pub(crate) fn row_menu_drawn(&mut self, dots: Option<Rect>) {
        if self.row_menu.as_ref().is_some_and(|menu| menu.dots != dots) {
            self.row_menu = None;
        }
    }

    /// ↓ or ↑: from no item, the first or the last. It wraps.
    pub(crate) fn row_menu_move(&mut self, delta: i32) {
        let count = self.row_menu().map_or(0, |(_, items)| items.len() as i64);
        if let Some(menu) = &mut self.row_menu
            && count > 0
        {
            let from = menu.selected.map_or(if delta > 0 { -1 } else { count }, |at| at as i64);
            menu.selected = Some((from + i64::from(delta)).rem_euclid(count) as usize);
        }
    }

    /// Why the item at `index` of the open menu does nothing now: drawn dimmed, with this.
    pub fn row_menu_why(&self, index: usize) -> Option<&'static str> {
        match self.row_menu_action(Some(index))? {
            Action::WorktreeDelete { path } => self.worktree_busy(&path),
            _ => None,
        }
    }

    /// What taking an item (default: the one the keys are on) does: the action its key in list
    /// mode sends. The shell asks why that would wait, to say so.
    pub fn row_menu_action(&self, index: Option<usize>) -> Option<Action> {
        let (menu, items) = self.row_menu()?;
        let item = items.get(index.or(menu.selected)?)?;
        let at = self.now;
        let (workbook, tab) = match (&menu.row, item) {
            (_, Item::Pause) => return Some(Action::ListenPause { at }),
            (_, Item::Resume) => return Some(Action::ListenResume { at }),
            (_, Item::LocalFolder) => return Some(Action::RepoPickFolder),
            (_, Item::FromGitHub) => return Some(Action::CloneOpen),
            (MenuOf::Row(ListRow::Repo(path)), Item::ExplorerHere) => {
                return Some(Action::ExplorerOpen { root: Some(path.clone()), axis: crate::layout::Axis::Right });
            }
            // The explorer's acts: done as the menu closes (`explorer_act`), none to wait for.
            (MenuOf::Explorer(..), _) => return Some(Action::RowMenuClose),
            (MenuOf::Row(ListRow::Repo(path)), Item::DeleteWorktree) => {
                return Some(Action::WorktreeDelete { path: path.clone() });
            }
            // Keep and Edit stand for the dialog: the shell reads the app's windows for its look.
            (MenuOf::Pane(pane), Item::SlotKeep | Item::SlotEdit) => {
                return Some(Action::SlotDialogOpen { pane: *pane, look: crate::slot::Look::default() });
            }
            (MenuOf::Pane(pane), Item::SlotClear) => return Some(Action::SlotClear { pane: *pane }),
            (MenuOf::Pane(pane), Item::SlotLookAgain) => return Some(Action::SlotLookAgain { pane: *pane }),
            (MenuOf::Pane(pane), Item::SlotGiveBack) => return Some(Action::SlotGiveBack { pane: *pane }),
            (MenuOf::Pane(pane), Item::SlotSwap(with)) => return Some(Action::SlotSwap { pane: *pane, with: *with }),
            (MenuOf::Pane(pane), Item::SlotResort) => return Some(Action::SlotResort { pane: *pane }),
            (MenuOf::Tab(tab), Item::Rename) => return Some(Action::TabEdit { tab: Some(*tab) }),
            (MenuOf::Tab(tab), Item::FoldOut) => return Some(Action::TabOut { tab: Some(*tab) }),
            // The shell closes it as its × does: asking first while a program runs there.
            (MenuOf::Tab(tab), Item::Close) => {
                let index = self.tabs.iter().position(|open| open.id == *tab);
                return Some(Action::CloseTab { index });
            }
            (MenuOf::Row(ListRow::Workbook(id)) | MenuOf::Space(id), _) => (*id, None),
            (MenuOf::Row(ListRow::WorkbookTab(id, at)), _) => (*id, Some(*at)),
            _ => return None,
        };
        match item {
            Item::Rename => Some(Action::WorkbookEdit { workbook, tab }),
            Item::Delete | Item::Remove => Some(Action::WorkbookRemove { workbook, tab }),
            Item::Keep => Some(Action::WorkbookKeep { workbook }),
            _ => None,
        }
    }

    /// Takes an item: the menu closes, and the row is renamed (its name typed in place) or
    /// deleted. ↵ with the keys on no item: nothing, the menu stays.
    pub(crate) fn row_menu_choose(&mut self, index: Option<usize>, effects: &mut Vec<crate::model::Effect>) {
        let Some(action) = self.row_menu_action(index) else {
            // An item there is not closes it; ↵ on none leaves it.
            self.row_menu = self.row_menu.take().filter(|_| index.is_none());
            return;
        };
        let menu = self.row_menu.take();
        let top = menu.as_ref().is_some_and(|menu| matches!(menu.row, MenuOf::Space(_)));
        if let Some(RowMenu { row: MenuOf::Explorer(pane, path), selected, items, .. }) = menu {
            if let Some(item) = index.or(selected).and_then(|at| items.get(at)) {
                self.explorer_act(pane, &path, *item, effects);
            }
            return;
        }
        match action {
            Action::ExplorerOpen { root, axis } => self.explorer_open(root, axis, effects),
            Action::WorkbookEdit { workbook, tab } => {
                self.workbook_edit(workbook, tab);
                // Typed in its item on the top row, where it was right-clicked.
                if let Some(MarkInput::Workbook { top: on, .. }) = &mut self.mark_input {
                    *on = top;
                }
            }
            Action::TabEdit { tab } => self.tab_edit(tab),
            Action::TabOut { tab } => self.tab_out(tab, effects),
            // The shell asks, as the ×'s click does (`ask_close_tab`).
            Action::CloseTab { .. } => {}
            Action::WorkbookRemove { workbook, tab } => self.workbook_remove(workbook, tab),
            Action::WorkbookKeep { workbook } => self.workbook_keep(workbook),
            action @ (Action::ListenPause { .. } | Action::ListenResume { .. }) => self.listen_act(action, effects),
            Action::WorktreeDelete { path } => self.worktree_delete_ask(path, effects),
            Action::RepoPickFolder => effects.push(crate::model::Effect::PickRepoFolder),
            Action::CloneOpen => self.clone_open(effects),
            // The shell opens the dialog with its look; the core only closed the menu.
            Action::SlotDialogOpen { .. } => {}
            action @ (Action::SlotClear { .. }
            | Action::SlotLookAgain { .. }
            | Action::SlotGiveBack { .. }
            | Action::SlotSwap { .. }
            | Action::SlotResort { .. }) => self.slot_act(action, effects),
            _ => {}
        }
    }

    /// After every step: the menu is of a row the open section lists, or of the transcript shown,
    /// and offers something.
    pub(crate) fn settle_row_menu(&mut self) {
        if self
            .row_menu
            .as_ref()
            .is_some_and(|menu| !self.menu_there(&menu.row) || self.menu_items(&menu.row) != menu.items)
        {
            self.row_menu = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis::Right;
    use crate::edit::Edit;
    use crate::model::{Kind, Origin, PickItem, Section};
    use crate::workbook::{Change, USERS_ALONE};

    const UID: &str = "9f3a61c2d4e5b708";
    const UID_2: &str = "1b7c0e9a55d2f346";
    const DOTS: Option<Rect> = Some(Rect { x: 280., y: 150., w: 20., h: 16. });

    /// One kept workspace, "Workspace 2", its section open.
    fn listed() -> Model {
        let mut m = Model::default();
        m.update(Action::WorkbookNew { uid: UID.into() });
        m.update(Action::WorkbookKeep { workbook: 1 });
        m.update(Action::ToggleSection { section: Section::Workbook });
        m
    }

    fn names(m: &Model) -> Vec<&str> {
        m.workbooks.list.iter().map(|book| book.name.as_str()).collect()
    }

    fn open(row: ListRow, keys: bool) -> Action {
        Action::RowMenu { row: row.into(), dots: DOTS, keys }
    }

    /// `listed`, the menu of its row open by a key: the keys on Rename.
    fn opened() -> Model {
        let mut m = listed();
        m.update(open(ListRow::Workbook(1), true));
        m
    }

    fn selected(m: &Model) -> Option<Item> {
        m.row_menu().and_then(|(menu, items)| Some(items[menu.selected?]))
    }

    #[test]
    fn a_workspaces_row_offers_rename_and_delete_and_no_other_row_has_a_menu() {
        let labels = |row: ListRow| row_items(&row).iter().map(|item| item.label()).collect::<Vec<_>>();
        assert_eq!(labels(ListRow::Workbook(1)), ["Rename", "Delete"]);
        assert_eq!(labels(ListRow::WorkbookTab(1, 0)), ["Rename", "Remove"]);
        assert!(labels(ListRow::Task(1)).is_empty());
        // A row that is not listed, or has no menu, opens none.
        let mut m = listed();
        for row in [ListRow::Workbook(9), ListRow::WorkbookTab(1, 0), ListRow::Task(1)] {
            m.update(open(row, true));
            assert_eq!(m.row_menu(), None);
        }
    }

    #[test]
    fn a_click_opens_it_on_no_item_and_a_key_on_the_first_and_the_arrows_move_and_enter_takes() {
        let mut m = listed();
        let row = ListRow::Workbook(1);
        // By a click: the keys on no item, and ↵ takes none.
        m.update(open(row.clone(), false));
        m.update(menu_key("enter"));
        assert_eq!((selected(&m), m.row_menu().is_some(), names(&m)), (None, true, vec!["Workspace 2"]));
        // ↓ from none is the first, ↑ the last; they wrap.
        m.update(menu_key("down"));
        assert_eq!(selected(&m), Some(Item::Rename));
        m.update(menu_key("up"));
        assert_eq!(selected(&m), Some(Item::Delete));
        m.update(open(row.clone(), false));
        m.update(open(row.clone(), false));
        m.update(menu_key("up"));
        assert_eq!(selected(&m), Some(Item::Delete));
        // By a key: on the first. ↵ on Rename: the menu closes, and the name is typed in the row.
        let mut m = opened();
        assert_eq!(selected(&m), Some(Item::Rename));
        m.update(menu_key("enter"));
        assert_eq!((m.row_menu(), m.field().map(|(of, _)| of)), (None, Some("workbook")));
        m.update(Action::FieldEdit { edit: Edit::Insert("dri".into()) });
        m.update(Action::MarkSubmit);
        assert_eq!(names(&m), ["dri"]);
        // ↓ ↵: Delete, and Undo is offered.
        m.update(open(row.clone(), true));
        m.update(menu_key("down"));
        m.update(menu_key("enter"));
        assert_eq!((m.row_menu(), names(&m), m.workbooks.offered()), (None, vec![], Some(Change::Deleted)));
        m.update(Action::WorkbookUndo);
        assert_eq!(names(&m), ["dri"]);
        // A click on an item takes it by its place; a place there is not closes the menu.
        m.update(open(row.clone(), false));
        m.update(Action::RowMenuChoose { index: Some(7) });
        assert_eq!((m.row_menu(), names(&m)), (None, vec!["dri"]));
        m.update(open(row, false));
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert!(names(&m).is_empty());
    }

    /// Review: `x` in the menu deleted. A client's picker over the menu, the user typing
    /// "xcode", lost a workspace; a held `x` went on to the rows under it.
    #[test]
    fn no_letter_takes_an_item_any_other_key_only_closes_the_menu() {
        for key in ["x", "r", "j", "k", "escape", "space", "a", "backspace"] {
            let mut m = opened();
            m.update(menu_key("down")); // on Delete
            m.update(menu_key(key));
            assert_eq!((m.row_menu(), names(&m), m.mark_input.is_none()), (None, vec!["Workspace 2"], true), "{key}");
        }
    }

    #[test]
    fn the_menu_closes_on_its_own_dots_and_on_any_other_move_of_the_users() {
        let row = ListRow::Workbook(1);
        // The … again.
        let mut m = opened();
        m.update(open(row.clone(), false));
        assert_eq!(m.row_menu(), None);
        // Another move of the user's: a pane opened, the section closed.
        for action in [Action::Open { kind: Kind::Terminal, axis: Right, beside: None }, Action::CloseSidebar] {
            let mut m = opened();
            m.update(action.clone());
            assert_eq!(m.row_menu(), None, "{action:?}");
        }
        // Not a client's move, nor news; sneak mode's labels reach its items.
        let mut m = opened();
        m.update(Action::NewTab.sent_by(Origin::Socket));
        m.update(Action::Tick { now: 5 }.sent_by(Origin::System));
        m.update(Action::Sneak { targets: Vec::new() });
        assert!(m.row_menu().is_some());
        // Its workspace gone from the list: closed.
        m.update(Action::RestoreWorkbooks { workbooks: Vec::new() }.sent_by(Origin::System));
        assert_eq!(m.row_menu(), None);
    }

    /// Review: the menu was drawn where its … had been. A line above the rows went (the offer to
    /// undo a delete ran out), the rows moved up, and the menu hung under another row's ….
    #[test]
    fn the_menu_closes_when_its_dots_are_drawn_elsewhere_or_not_at_all() {
        let drawn = |dots: Option<Rect>| Action::RowMenuDrawn { dots }.sent_by(Origin::System);
        // Drawn where they were: it stays.
        let mut m = opened();
        m.update(drawn(DOTS));
        assert!(m.row_menu().is_some());
        // The rows moved up a line: closed, and ↵ after it takes nothing.
        m.update(drawn(DOTS.map(|at| Rect { y: at.y - 30., ..at })));
        m.update(menu_key("enter"));
        assert_eq!((m.row_menu(), names(&m), m.mark_input.is_none()), (None, vec!["Workspace 2"], true));
        // Not drawn any more: closed.
        let mut m = opened();
        m.update(drawn(None));
        assert_eq!(m.row_menu(), None);
        // A client says nothing of what is drawn.
        let mut m = opened();
        let theirs = Action::RowMenuDrawn { dots: None };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(USERS_ALONE));
        m.update(theirs.sent_by(Origin::Socket));
        assert!(m.row_menu().is_some());
    }

    #[test]
    fn the_menu_is_the_users_alone() {
        let mut m = listed();
        let row = ListRow::Workbook(1);
        let theirs = [
            open(row.clone(), true),
            Action::RowMenuMove { delta: 1 },
            Action::RowMenuChoose { index: Some(1) },
            Action::RowMenuClose,
        ];
        for origin in [Origin::Socket, Origin::System] {
            for action in &theirs {
                assert_eq!(m.refusal(action, origin), Some(USERS_ALONE), "{action:?}");
            }
        }
        // Open for the user: a client neither chooses from it nor closes it.
        m.update(open(row, true));
        for action in theirs {
            m.update(action.sent_by(Origin::Socket));
        }
        assert_eq!((m.row_menu().map(|(menu, _)| menu.selected), names(&m)), (Some(Some(0)), vec!["Workspace 2"]));
    }

    /// Review: a client's `picker` opened over the user's menu, whose keys the menu still took.
    #[test]
    fn nothing_of_a_clients_takes_the_keyboard_from_under_the_menu() {
        let m = opened();
        let theirs = [
            Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] },
            Action::OpenCompose { text: "theirs".into() },
        ];
        for action in theirs {
            for origin in [Origin::Socket, Origin::System] {
                assert!(action.takes_keyboard(), "{action:?}");
                assert_eq!(m.refusal(&action, origin), Some(MENU_OPEN), "{action:?}");
                let mut m = opened();
                m.update(action.clone().sent_by(origin));
                assert!(m.row_menu().is_some() && m.picker.is_none() && m.compose.is_none(), "{action:?}");
            }
        }
        // The user's own: it closes the menu and opens.
        let mut m = opened();
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![PickItem::Terminal] });
        assert!(m.row_menu().is_none() && m.picker.is_some());
    }

    /// A temporary workspace's menu offers Keep, the user's alone; kept, it offers no more.
    #[test]
    fn a_temporary_workspaces_menu_keeps_it_and_a_kept_ones_has_no_keep() {
        let mut m = Model::default();
        m.update(Action::WorkbookNew { uid: UID.into() });
        m.update(Action::ToggleSection { section: Section::Workbook });
        m.update(open(ListRow::Workbook(1), true));
        let items = |m: &Model| m.row_menu().map(|(_, items)| items.to_vec()).unwrap_or_default();
        assert_eq!(items(&m), [Item::Rename, Item::Keep, Item::Delete]);
        assert_eq!(m.row_menu_action(Some(1)), Some(Action::WorkbookKeep { workbook: 1 }));
        // A client's Keep is refused: the file is the user's.
        let theirs = Action::WorkbookKeep { workbook: 1 }.sent_by(Origin::Socket);
        assert_eq!(m.refusal(&theirs.action, theirs.origin), Some(USERS_ALONE));
        m.update(theirs);
        assert!(!m.workbooks.is_kept(1));
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert!(m.workbooks.is_kept(1) && m.row_menu().is_none());
        m.update(open(ListRow::Workbook(1), true));
        assert_eq!(items(&m), [Item::Rename, Item::Delete]);
    }

    /// A workspace's item on the top row has a menu too: Rename, and Keep while temporary. Off
    /// the row, none.
    #[test]
    fn a_workspaces_item_on_the_top_row_offers_rename_and_keep_while_temporary() {
        let mut m = Model::default();
        m.update(Action::WorkbookNew { uid: UID.into() });
        m.update(Action::RowMenu { row: MenuOf::Space(1), dots: DOTS, keys: true });
        let items = |m: &Model| m.row_menu().map(|(_, items)| items.to_vec()).unwrap_or_default();
        assert_eq!(items(&m), [Item::Rename, Item::Keep]);
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert!(m.workbooks.is_kept(1) && m.row_menu().is_none());
        m.update(Action::RowMenu { row: MenuOf::Space(1), dots: DOTS, keys: true });
        assert_eq!(items(&m), [Item::Rename]);
        // Rename types the name in the item on the top row.
        m.update(Action::RowMenuChoose { index: None });
        assert!(matches!(m.mark_input, Some(MarkInput::Workbook { workbook: 1, tab: None, top: true, .. })));
        m.update(Action::MarkCancel);
        // Off the row (a client's × leaves it listed): no menu there.
        m.update(Action::WorkbookNew { uid: UID_2.into() });
        m.update(Action::SpaceHide { index: None }.sent_by(Origin::Socket));
        m.update(Action::RowMenu { row: MenuOf::Space(2), dots: DOTS, keys: true });
        assert!(m.row_menu().is_none());
    }

    /// A right click on a tab: Rename, Fold out (`multi_monitor` on), Close as its × does.
    #[test]
    fn a_tabs_menu_renames_folds_out_or_closes_it() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::NewTab);
        let (first, second) = (m.tabs[m.active - 1].id, m.tabs[m.active].id);
        let items = |m: &Model| m.row_menu().map(|(_, items)| items.to_vec()).unwrap_or_default();
        m.update(Action::RowMenu { row: MenuOf::Tab(first), dots: DOTS, keys: true });
        assert_eq!(items(&m), [Item::Rename, Item::FoldOut, Item::Close]);
        assert_eq!(m.row_menu_action(Some(2)), Some(Action::CloseTab { index: Some(m.active - 1) }));
        assert_eq!(m.row_menu_action(Some(1)), Some(Action::TabOut { tab: Some(first) }));
        m.update(Action::RowMenuChoose { index: None });
        assert!(matches!(m.mark_input, Some(MarkInput::Tab { tab, .. }) if tab == first));
        m.update(Action::MarkCancel);
        // The flag off: no Fold out.
        let features = m.features.with(Feature::MultiMonitor, false);
        m.update(Action::SetFeatures { features });
        m.update(Action::RowMenu { row: MenuOf::Tab(second), dots: DOTS, keys: false });
        assert_eq!(items(&m), [Item::Rename, Item::Close]);
        m.update(Action::RowMenuClose);
        // Home's own tab, or one not on the row on show: none.
        m.update(Action::RowMenu { row: MenuOf::Tab(HOME_TAB), dots: DOTS, keys: false });
        assert!(m.row_menu().is_none());
    }

    /// Review: Rename from the menu did nothing, and said nothing, where a name cannot be typed.
    #[test]
    fn what_an_item_stands_for_is_the_action_the_shell_asks_about() {
        let mut m = listed();
        m.update(Action::WorkbookNew { uid: UID_2.into() });
        m.update(Action::WorkbookKeep { workbook: 2 });
        m.update(Action::WorkbookFold { workbook: 1 });
        m.update(open(ListRow::Workbook(2), true));
        assert_eq!(m.row_menu_action(None), Some(Action::WorkbookEdit { workbook: 2, tab: None }));
        assert_eq!(m.row_menu_action(Some(1)), Some(Action::WorkbookRemove { workbook: 2, tab: None }));
        assert_eq!(m.row_menu_action(Some(2)), None);
        // By a click the keys are on none: ↵ stands for nothing.
        m.update(open(ListRow::Workbook(2), true));
        m.update(open(ListRow::Workbook(2), false));
        assert_eq!(m.row_menu_action(None), None);
        // Visual mode on a terminal: a name cannot be typed, and the action says why.
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Right, beside: None });
        m.update(Action::WorkbookNew { uid: UID.into() });
        m.update(Action::SpaceSelect { index: 1 });
        m.update(Action::ToggleSection { section: Section::Workbook });
        m.update(Action::ToggleVisual);
        assert!(m.visual.is_some());
        m.update(open(ListRow::Workbook(1), true));
        let rename = m.row_menu_action(None);
        assert_eq!(rename, Some(Action::WorkbookEdit { workbook: 1, tab: None }));
        assert_eq!(rename.and_then(|rename| m.waits(&rename)), Some("visual mode is on"));
    }

    /// A pane's ⋯ is a menu, the user's alone: an app pane's Create slot… (the dialog), a slot's
    /// Edit slot… (the dialog), Offer again, Give back the window and Clear slot. Nothing with
    /// `slots` off.
    #[test]
    fn a_panes_dots_offer_its_slot_acts_by_what_it_is() {
        let app = Kind::App { window: 1, title: "SECRET".into(), bundle: None, name: "Zoom".into() };
        let mut m = Model::default();
        m.update(Action::Open { kind: app, axis: Right, beside: None });
        let menu = |m: &mut Model, origin| {
            m.update(Action::RowMenu { row: MenuOf::Pane(1), dots: DOTS, keys: true }.sent_by(origin));
            m.row_menu().map(|(_, items)| items.to_vec()).unwrap_or_default()
        };
        assert!(menu(&mut m, Origin::Socket).is_empty(), "a client's");
        assert_eq!(menu(&mut m, Origin::User), [Item::SlotKeep]);
        // Create slot… stands for the dialog, which the shell opens with its look; the menu closes.
        let dialog = Action::SlotDialogOpen { pane: 1, look: crate::slot::Look::default() };
        assert_eq!(m.row_menu_action(None), Some(dialog.clone()));
        assert_eq!(m.refusal(&Action::RowMenuChoose { index: None }, Origin::Socket), Some(crate::slot::USERS_ALONE));
        m.update(Action::RowMenuChoose { index: None });
        assert!(m.row_menu.is_none());
        // A filled slot: Edit slot…, Give back the window, Clear slot.
        m.update(Action::SlotMake { pane: 1, pid: 7, slot: crate::slot::Slot::default() });
        assert_eq!(menu(&mut m, Origin::User), [Item::SlotEdit, Item::SlotGiveBack, Item::SlotClear]);
        assert_eq!(m.row_menu_action(None), Some(dialog));
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert_eq!(m.pane(1).map(|p| matches!(p.kind, Kind::Slot { .. })), Some(true), "given back");
        // Waiting, its window passed over: Offer again forgets that, and leaves the menu. Its
        // items changing under it (the window closed: passed over no more) close it.
        assert_eq!(menu(&mut m, Origin::User), [Item::SlotEdit, Item::SlotLookAgain, Item::SlotClear]);
        m.slots.get_mut(&1).map(|run| run.declined.clear());
        m.update(Action::RowMenuMove { delta: 1 });
        assert!(m.row_menu.is_none(), "closed, not on Clear slot");
        assert_eq!(menu(&mut m, Origin::User), [Item::SlotEdit, Item::SlotClear]);
        m.slots.get_mut(&1).map(|run| run.declined.insert(1));
        m.update(Action::RowMenuMove { delta: 1 });
        assert_eq!(menu(&mut m, Origin::User), [Item::SlotEdit, Item::SlotLookAgain, Item::SlotClear]);
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert!(m.slots[&1].declined.is_empty());
        assert_eq!(menu(&mut m, Origin::User), [Item::SlotEdit, Item::SlotClear]);
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert!(m.slots.is_empty() && m.pane(1).is_some_and(|p| p.kind == Kind::Empty), "cleared");
        // An empty pane has no ⋯ menu; nor has any pane with `slots` off.
        assert!(menu(&mut m, Origin::User).is_empty());
        let mut off = Model::default();
        off.update(Action::SetFeatures { features: crate::features::Features::default() }.sent_by(Origin::System));
        let app = Kind::App { window: 2, title: String::new(), bundle: None, name: "Zoom".into() };
        off.update(Action::Open { kind: app, axis: Right, beside: None });
        assert!(menu(&mut off, Origin::User).is_empty());
    }
}
