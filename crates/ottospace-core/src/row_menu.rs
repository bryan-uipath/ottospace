//! A row's menu: … on a row of the left bar opens a short list of what may be done to it. One
//! at a time, the user's alone; any other move of the user's closes it. Today a workspace's row
//! and its tabs' rows have one; another row gets one by naming its items in `row_items`.

use serde::Serialize;

use crate::list::ListRow;
use crate::model::{Action, Model};

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

    /// Its key, in the menu and on the row in list mode.
    pub fn key(self) -> &'static str {
        match self {
            Item::Rename => "r",
            Item::Delete | Item::Remove => "x",
        }
    }
}

/// The open menu: whose row, and the item the keys are on.
#[derive(Clone, Debug, PartialEq)]
pub struct RowMenu {
    pub row: ListRow,
    pub selected: usize,
}

/// What a row's menu offers, in order. None: the row has no menu.
pub fn row_items(row: &ListRow) -> &'static [Item] {
    match row {
        ListRow::Workbook(_) => &[Item::Rename, Item::Delete],
        ListRow::WorkbookTab(..) => &[Item::Rename, Item::Remove],
        _ => &[],
    }
}

/// What a key does while a menu is open: ↓ ↑ (or j k) move, ↵ takes the item, an item's own key
/// takes it, any other key closes the menu.
pub fn menu_key(menu: &RowMenu, key: &str) -> Action {
    let own = row_items(&menu.row).iter().position(|item| item.key() == key);
    match key {
        "down" | "j" => Action::RowMenuMove { delta: 1 },
        "up" | "k" => Action::RowMenuMove { delta: -1 },
        "enter" => Action::RowMenuChoose { index: None },
        _ if own.is_some() => Action::RowMenuChoose { index: own },
        _ => Action::RowMenuClose,
    }
}

impl Model {
    /// The open menu and its items.
    pub fn row_menu(&self) -> Option<(&RowMenu, &'static [Item])> {
        self.row_menu.as_ref().map(|menu| (menu, row_items(&menu.row)))
    }

    /// … on a row: its menu, on its first item. On the row whose menu is open: closed.
    pub(crate) fn row_menu_open(&mut self, row: ListRow) {
        let open = self.row_menu.take().is_some_and(|menu| menu.row == row);
        let listed = self.list_rows().contains(&row) && !row_items(&row).is_empty();
        self.row_menu = (!open && listed).then_some(RowMenu { row, selected: 0 });
    }

    pub(crate) fn row_menu_move(&mut self, delta: i32) {
        if let Some(menu) = &mut self.row_menu {
            let count = row_items(&menu.row).len() as i64;
            menu.selected = (menu.selected as i64 + i64::from(delta)).rem_euclid(count) as usize;
        }
    }

    /// Takes an item (default: the one the keys are on): the menu closes, and the row is renamed
    /// (its name typed in place) or deleted, as its key in list mode does.
    pub(crate) fn row_menu_choose(&mut self, index: Option<usize>) {
        let Some(menu) = self.row_menu.take() else { return };
        let Some(item) = row_items(&menu.row).get(index.unwrap_or(menu.selected)) else { return };
        let (id, tab) = match menu.row {
            ListRow::Workbook(id) => (id, None),
            ListRow::WorkbookTab(id, at) => (id, Some(at)),
            _ => return,
        };
        match item {
            Item::Rename => self.workbook_edit(id, tab),
            Item::Delete | Item::Remove => self.workbook_remove(id, tab),
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
    use crate::model::{Kind, Origin, Section};
    use crate::workbook::{Change, USERS_ALONE};

    const UID: &str = "9f3a61c2d4e5b708";

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

    #[test]
    fn a_workspaces_row_offers_rename_and_delete_and_no_other_row_has_a_menu() {
        let labels = |row: ListRow| row_items(&row).iter().map(|item| (item.label(), item.key())).collect::<Vec<_>>();
        assert_eq!(labels(ListRow::Workbook(1)), [("Rename", "r"), ("Delete", "x")]);
        assert_eq!(labels(ListRow::WorkbookTab(1, 0)), [("Rename", "r"), ("Remove", "x")]);
        assert_eq!(labels(ListRow::Task(1)), []);
        // A row that is not listed, or has no menu, opens none.
        let mut m = listed();
        for row in [ListRow::Workbook(9), ListRow::WorkbookTab(1, 0), ListRow::Task(1)] {
            m.update(Action::RowMenu { row });
            assert_eq!(m.row_menu(), None);
        }
    }

    #[test]
    fn the_menu_renames_and_deletes_by_arrow_and_enter_or_by_its_keys() {
        let mut m = listed();
        let row = ListRow::Workbook(1);
        m.update(Action::RowMenu { row: row.clone() });
        let selected = |m: &Model| m.row_menu().map(|(menu, items)| items[menu.selected]);
        assert_eq!(selected(&m), Some(Item::Rename));
        // The keys move, and wrap.
        let menu = RowMenu { row: row.clone(), selected: 0 };
        m.update(menu_key(&menu, "down"));
        assert_eq!(selected(&m), Some(Item::Delete));
        m.update(menu_key(&menu, "j"));
        m.update(menu_key(&menu, "up"));
        assert_eq!(selected(&m), Some(Item::Delete));
        m.update(menu_key(&menu, "k"));
        // ↵ on Rename: the menu closes, and the name is typed in the row.
        m.update(menu_key(&menu, "enter"));
        assert_eq!((m.row_menu(), m.field().map(|(of, _)| of)), (None, Some("workbook")));
        m.update(Action::FieldEdit { edit: Edit::Insert("dri".into()) });
        m.update(Action::MarkSubmit);
        assert_eq!(names(&m), ["dri"]);
        // Its own key takes an item wherever the keys are: X deletes, and Undo is offered.
        m.update(Action::RowMenu { row: row.clone() });
        m.update(menu_key(&menu, "x"));
        assert_eq!((m.row_menu(), names(&m), m.workbooks.offered()), (None, vec![], Some(Change::Deleted)));
        m.update(Action::WorkbookUndo);
        assert_eq!(names(&m), ["dri"]);
        // A click on an item takes it by its place.
        m.update(Action::RowMenu { row });
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert_eq!(names(&m), Vec::<&str>::new());
    }

    #[test]
    fn the_menu_closes_on_esc_on_its_own_dots_and_on_any_other_move_of_the_users() {
        let row = ListRow::Workbook(1);
        let open = || {
            let mut m = listed();
            m.update(Action::RowMenu { row: ListRow::Workbook(1) });
            m
        };
        let menu = RowMenu { row: row.clone(), selected: 0 };
        // Esc, or any key that is not the menu's.
        for key in ["escape", "q"] {
            let mut m = open();
            m.update(menu_key(&menu, key));
            assert_eq!((m.row_menu(), names(&m)), (None, vec!["Workspace 2"]), "{key}");
        }
        // The … again.
        let mut m = open();
        m.update(Action::RowMenu { row: row.clone() });
        assert_eq!(m.row_menu(), None);
        // Another move of the user's: a pane opened, the section closed.
        for action in [Action::Open { kind: Kind::Terminal, axis: Right, beside: None }, Action::CloseSidebar] {
            let mut m = open();
            m.update(action.clone());
            assert_eq!(m.row_menu(), None, "{action:?}");
        }
        // Not a client's move, nor news; sneak mode's labels reach its items.
        let mut m = open();
        m.update(Action::NewTab.sent_by(Origin::Socket));
        m.update(Action::Tick { now: 5 }.sent_by(Origin::System));
        m.update(Action::Sneak { targets: Vec::new() });
        assert!(m.row_menu().is_some());
        // Its workspace gone from the list: closed.
        m.update(Action::RestoreWorkbooks { workbooks: Vec::new() }.sent_by(Origin::System));
        assert_eq!(m.row_menu(), None);
    }

    #[test]
    fn the_menu_is_the_users_alone() {
        let mut m = listed();
        let row = ListRow::Workbook(1);
        let theirs = [
            Action::RowMenu { row: row.clone() },
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
        m.update(Action::RowMenu { row });
        for action in theirs {
            m.update(action.sent_by(Origin::Socket));
        }
        assert_eq!((m.row_menu().map(|(menu, _)| menu.selected), names(&m)), (Some(0), vec!["Workspace 2"]));
    }
}
