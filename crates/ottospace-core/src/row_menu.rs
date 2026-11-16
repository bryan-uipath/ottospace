//! A row's menu: … on a row of the left bar opens a short list of what may be done to it. One
//! at a time, the user's alone; any other move of the user's closes it, and so does its … moving
//! (the list under it changed). It holds the keyboard: ↓ ↑ move, ↵ takes, any other key closes.
//! No item has a letter: a key meant for what is behind the menu must never delete.
//! Today a workspace's row and its tabs' rows have one; another row gets one by naming its
//! items in `row_items`.

use serde::Serialize;

use crate::layout::Rect;
use crate::list::ListRow;
use crate::model::{Action, Model};

/// Why a client's request that takes the keyboard waits: the user's keys are in a row's menu.
pub const MENU_OPEN: &str = "a row's menu is open";

/// What a row's menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Item {
    Rename,
    Delete,
    /// A tab out of its workspace.
    Remove,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Rename => "Rename",
            Item::Delete => "Delete",
            Item::Remove => "Remove",
        }
    }
}

/// The open menu: whose row, the item the keys are on (none until a key moves, when a click
/// opened it), and where its … was drawn when it opened.
#[derive(Clone, Debug, PartialEq)]
pub struct RowMenu {
    pub row: ListRow,
    pub selected: Option<usize>,
    pub dots: Option<Rect>,
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
    pub fn row_menu(&self) -> Option<(&RowMenu, &'static [Item])> {
        self.row_menu.as_ref().map(|menu| (menu, row_items(&menu.row)))
    }

    /// … on a row: its menu, the keys on its first item if a key opened it. On the row whose
    /// menu is open: closed. `dots`: where the … is drawn.
    pub(crate) fn row_menu_open(&mut self, row: ListRow, dots: Option<Rect>, keys: bool) {
        let open = self.row_menu.take().is_some_and(|menu| menu.row == row);
        let listed = self.list_rows().contains(&row) && !row_items(&row).is_empty();
        self.row_menu = (!open && listed).then_some(RowMenu { row, selected: keys.then_some(0), dots });
    }

    /// The … of the open menu as last drawn. Elsewhere than it was (a row or a notice above came
    /// or went, the window changed): the menu goes with it, never left under another row. Not
    /// drawn any more: the menu closes.
    pub(crate) fn row_menu_drawn(&mut self, dots: Option<Rect>) {
        match (&mut self.row_menu, dots) {
            (Some(menu), Some(at)) => menu.dots = Some(at),
            _ => self.row_menu = None,
        }
    }

    /// ↓ or ↑: from no item, the first or the last. It wraps.
    pub(crate) fn row_menu_move(&mut self, delta: i32) {
        if let Some(menu) = &mut self.row_menu {
            let count = row_items(&menu.row).len() as i64;
            let from = menu.selected.map_or(if delta > 0 { -1 } else { count }, |at| at as i64);
            menu.selected = Some((from + i64::from(delta)).rem_euclid(count) as usize);
        }
    }

    /// What taking an item (default: the one the keys are on) does: the action its key in list
    /// mode sends. The shell asks why that would wait, to say so.
    pub fn row_menu_action(&self, index: Option<usize>) -> Option<Action> {
        let (menu, items) = self.row_menu()?;
        let item = items.get(index.or(menu.selected)?)?;
        let (workbook, tab) = match menu.row {
            ListRow::Workbook(id) => (id, None),
            ListRow::WorkbookTab(id, at) => (id, Some(at)),
            _ => return None,
        };
        Some(match item {
            Item::Rename => Action::WorkbookEdit { workbook, tab },
            Item::Delete | Item::Remove => Action::WorkbookRemove { workbook, tab },
        })
    }

    /// Takes an item: the menu closes, and the row is renamed (its name typed in place) or
    /// deleted. ↵ with the keys on no item: nothing, the menu stays.
    pub(crate) fn row_menu_choose(&mut self, index: Option<usize>) {
        let Some(action) = self.row_menu_action(index) else {
            // An item there is not closes it; ↵ on none leaves it.
            self.row_menu = self.row_menu.take().filter(|_| index.is_none());
            return;
        };
        self.row_menu = None;
        match action {
            Action::WorkbookEdit { workbook, tab } => self.workbook_edit(workbook, tab),
            Action::WorkbookRemove { workbook, tab } => self.workbook_remove(workbook, tab),
            _ => {}
        }
    }

    /// After every step: the menu is of a row the open section lists.
    pub(crate) fn settle_row_menu(&mut self) {
        if self.row_menu.as_ref().is_some_and(|menu| !self.list_rows().contains(&menu.row)) {
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

    /// One workspace, "Workspace 2", its section open.
    fn listed() -> Model {
        let mut m = Model::default();
        m.update(Action::WorkbookNew { uid: UID.into() });
        m.update(Action::ToggleSection { section: Section::Workbook });
        m
    }

    fn names(m: &Model) -> Vec<&str> {
        m.workbooks.list.iter().map(|book| book.name.as_str()).collect()
    }

    fn open(row: ListRow, keys: bool) -> Action {
        Action::RowMenu { row, dots: DOTS, keys }
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
    fn the_menu_follows_its_dots_and_closes_when_they_are_not_drawn() {
        let drawn = |dots: Option<Rect>| Action::RowMenuDrawn { dots }.sent_by(Origin::System);
        // Drawn where they were: it stays.
        let mut m = opened();
        m.update(drawn(DOTS));
        assert!(m.row_menu().is_some());
        // The rows moved up a line: the menu goes with its …, still of its own row.
        let moved = DOTS.map(|at| Rect { y: at.y - 30., ..at });
        m.update(drawn(moved));
        assert_eq!(m.row_menu().map(|(menu, _)| (menu.row.clone(), menu.dots)), Some((ListRow::Workbook(1), moved)));
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

    /// Review: Rename from the menu did nothing, and said nothing, where a name cannot be typed.
    #[test]
    fn what_an_item_stands_for_is_the_action_the_shell_asks_about() {
        let mut m = listed();
        m.update(Action::WorkbookNew { uid: UID_2.into() });
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
        m.update(Action::SpaceSelect { index: 0 });
        m.update(Action::ToggleSection { section: Section::Workbook });
        m.update(Action::ToggleVisual);
        assert!(m.visual.is_some());
        m.update(open(ListRow::Workbook(1), true));
        let rename = m.row_menu_action(None);
        assert_eq!(rename, Some(Action::WorkbookEdit { workbook: 1, tab: None }));
        assert_eq!(rename.and_then(|rename| m.waits(&rename)), Some("visual mode is on"));
    }
}
