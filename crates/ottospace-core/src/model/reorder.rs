//! The user's drag that reorders the workspaces: a row of the Workspaces section, or its space on
//! the top row. One order, the workbooks' (kept in their files): the top row shows the open ones
//! in it, after Home; a space of no workspace (a task's, a session's) comes after them all, in
//! its own order (`settle_space_order`). The shell says where the pointer is (`ReorderOver`); the
//! core says what moves.

use serde::{Deserialize, Serialize};

use super::{Model, Owner};
use crate::workbook::WorkbookId;

/// Why a client's reorder is refused: the order follows the user's pointer.
pub const REORDER_USERS: &str = "only the user drags a workspace to another place";

/// What a reorder drag carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReorderItem {
    /// A row of the Workspaces section.
    Workbook { id: WorkbookId },
    /// A workspace's space on the top row (its task's, while bound to one). Never Home's.
    Space { of: Owner },
}

/// A drag in flight: `item`, and the place it takes if dropped now, among the rows listed (the
/// section's) or the spaces shown (the top row's, Home at 0).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reorder {
    pub item: ReorderItem,
    pub to: usize,
}

/// Moves `list[from]` to place `to` among the elements `shown` says, by index, the others staying
/// where they are. e.g. shown [a, b, c], hidden x: [a, x, b, c], b to 0 → [b, a, x, c]. False: no move.
fn move_among<T>(list: &mut Vec<T>, from: usize, to: usize, shown: &[bool]) -> bool {
    let places: Vec<usize> = (0..list.len()).filter(|at| shown.get(*at) == Some(&true)).collect();
    let Some(at) = places.iter().position(|place| *place == from) else { return false };
    if to == at || to >= places.len() {
        return false;
    }
    let item = list.remove(from);
    // The shown element it goes before (or after, the last), its index once `item` is out.
    let others: Vec<usize> =
        places.into_iter().filter(|place| *place != from).map(|p| p - usize::from(p > from)).collect();
    let index = match others.get(to) {
        Some(before) => *before,
        None => others.last().map_or(0, |last| last + 1),
    };
    list.insert(index, item);
    true
}

impl Model {
    /// The drag in flight, if any.
    pub fn reorder(&self) -> Option<&Reorder> {
        self.reorder.as_ref()
    }

    /// How many places `item` moves among, and the least it may take (Home stays first). None:
    /// it is not there to move.
    fn reorder_places(&self, item: &ReorderItem) -> Option<(usize, usize)> {
        match item {
            ReorderItem::Workbook { id } => {
                let listed: Vec<WorkbookId> = self.workbooks.listed().map(|book| book.id).collect();
                listed.contains(id).then_some((listed.len(), 0))
            }
            // Only a workspace's: it goes among theirs, after Home and before every other.
            ReorderItem::Space { of } => {
                let shown = self.shown_spaces();
                let books = shown.iter().filter(|space| self.space_book(&space.of).is_some()).count();
                // Its place among the shown is its place on the row (`settle_space_order`).
                let shown = self.space_book(of).is_some() && shown.iter().any(|space| space.of == *of);
                shown.then_some((books + 1, 1))
            }
        }
    }

    /// The user carries `item` over place `to`: a drag starts, or goes on. Kept within the list.
    pub(crate) fn reorder_over(&mut self, item: ReorderItem, to: usize) {
        self.reorder = self.reorder_places(&item).map(|(len, least)| Reorder { item, to: to.clamp(least, len - 1) });
    }

    /// The drop: the item carried takes its place. A workbook's makes the section's order the
    /// user's from then on, kept in each file (`Workbook::order`).
    pub(crate) fn reorder_drop(&mut self) {
        let Some(Reorder { item, to }) = self.reorder.take() else { return };
        match item {
            ReorderItem::Workbook { id } => {
                let listed: Vec<bool> = self.workbooks.list.iter().map(|book| book.closing.is_none()).collect();
                self.workbooks_move(id, to, &listed);
            }
            // The same move in the one order, among the workspaces on the row (after Home).
            ReorderItem::Space { of } => {
                let Some(id) = self.space_book(&of).map(|book| book.id) else { return };
                let shown: Vec<Owner> = self.shown_spaces().iter().map(|space| space.of.clone()).collect();
                let on_row = |book: &crate::workbook::Workbook| {
                    let of = self.workbooks.space_of(&Owner::Workbook { id: book.id });
                    book.closing.is_none() && shown.contains(&of)
                };
                let on_row: Vec<bool> = self.workbooks.list.iter().map(on_row).collect();
                self.workbooks_move(id, to.saturating_sub(1), &on_row);
            }
        }
    }

    /// Workbook `id` to place `to` among those `among` says: the order is the user's from then on.
    fn workbooks_move(&mut self, id: WorkbookId, to: usize, among: &[bool]) {
        let list = &mut self.workbooks.list;
        let Some(from) = list.iter().position(|book| book.id == id) else { return };
        if move_among(list, from, to, among) {
            self.workbooks.renumber();
        }
    }

    /// After every step: the top row in the one order. Home first, then each workspace's space as
    /// the section lists it, then every other space in the order it had.
    pub(crate) fn settle_space_order(&mut self) {
        let place = |of: &Owner| -> (u8, usize) {
            if *of == Owner::Home {
                return (0, 0);
            }
            let id = self.space_book(of).map(|book| book.id);
            let book = id.and_then(|id| self.workbooks.list.iter().position(|book| book.id == id));
            book.map_or((2, 0), |at| (1, at))
        };
        let keys: Vec<(u8, usize)> = self.spaces.iter().map(|space| place(&space.of)).collect();
        let mut keyed: Vec<_> = keys.into_iter().zip(std::mem::take(&mut self.spaces)).collect();
        keyed.sort_by_key(|(key, _)| *key);
        self.spaces = keyed.into_iter().map(|(_, space)| space).collect();
    }

    /// After every step: a drag whose row or space is gone (deleted, closed, hidden) ends, and its
    /// place stays within the list.
    pub(crate) fn settle_reorder(&mut self) {
        let Some(Reorder { item, to }) = self.reorder.take() else { return };
        self.reorder_over(item, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Origin};
    use crate::workbook::Workbook;

    #[test]
    fn move_among_skips_what_is_not_shown() {
        let moved = |from: usize, to: usize| {
            let mut list = vec!['a', 'x', 'b', 'c'];
            let moved = move_among(&mut list, from, to, &[true, false, true, true]);
            (moved, list.into_iter().collect::<String>())
        };
        assert_eq!(moved(2, 0), (true, "baxc".into()));
        assert_eq!(moved(0, 2), (true, "xbca".into()));
        assert_eq!(moved(3, 0), (true, "caxb".into()));
        assert_eq!(moved(0, 1), (true, "xbac".into()));
        // Onto itself, past the end, or a hidden one: nothing.
        assert_eq!(moved(2, 1), (false, "axbc".into()));
        assert_eq!(moved(2, 3), (false, "axbc".into()));
        assert_eq!(moved(1, 0), (false, "axbc".into()));
    }

    const UIDS: [&str; 4] = ["000000000000000a", "000000000000000b", "000000000000000c", "000000000000000d"];

    /// A model with kept workbooks named `names`, read from files as at start.
    fn books(names: &[&str]) -> Model {
        let mut m = Model::default();
        let workbooks = names.iter().zip(UIDS).map(|(name, uid)| file(uid, name, None)).collect();
        m.update(Action::RestoreWorkbooks { workbooks }.sent_by(Origin::System));
        m
    }

    fn file(uid: &str, name: &str, order: Option<u32>) -> Workbook {
        let text = serde_json::json!({ "version": 2, "id": uid, "name": name, "tabs": [], "order": order });
        crate::workbook::parse(&text.to_string(), uid).expect("reads")
    }

    fn names(m: &Model) -> Vec<String> {
        m.workbook_rows().into_iter().map(|row| row.name).collect()
    }

    fn id(m: &Model, name: &str) -> WorkbookId {
        m.workbooks.list.iter().find(|book| book.name == name).map(|book| book.id).unwrap_or_default()
    }

    fn drag(m: &mut Model, name: &str, to: usize) {
        let item = ReorderItem::Workbook { id: id(m, name) };
        m.update(Action::ReorderOver { item, to });
        m.update(Action::ReorderDrop);
    }

    #[test]
    fn a_row_dragged_first_to_last_last_to_first_or_onto_itself() {
        let mut m = books(&["Arima", "Slack", "Zoom"]);
        drag(&mut m, "Arima", 2);
        assert_eq!(names(&m), ["Slack", "Zoom", "Arima"]);
        drag(&mut m, "Arima", 0);
        assert_eq!(names(&m), ["Arima", "Slack", "Zoom"]);
        drag(&mut m, "Slack", 1);
        assert_eq!(names(&m), ["Arima", "Slack", "Zoom"]);
        // Past the end: the last place.
        drag(&mut m, "Arima", 9);
        assert_eq!(names(&m), ["Slack", "Zoom", "Arima"]);
        assert!(m.reorder().is_none());
    }

    #[test]
    fn the_first_drag_makes_the_order_the_users_and_a_new_one_goes_last() {
        let mut m = books(&["Arima", "Slack", "Zoom"]);
        assert!(m.workbooks.list.iter().all(|book| book.order.is_none()), "alphabetical until a drag");
        drag(&mut m, "Zoom", 0);
        assert_eq!(names(&m), ["Zoom", "Arima", "Slack"]);
        // A rename no longer sorts, and + puts the new one at the end.
        m.workbook_rename(id(&m, "Zoom"), None, "Zed");
        m.update(Action::WorkbookNew { uid: "00000000000000ee".into() });
        assert_eq!(names(&m), ["Zed", "Arima", "Slack", "Workspace 2"]);
    }

    #[test]
    fn a_cancelled_drag_moves_nothing_and_a_folded_or_closing_one_is_skipped() {
        let mut m = books(&["Arima", "Slack", "Zoom"]);
        let item = ReorderItem::Workbook { id: id(&m, "Zoom") };
        m.update(Action::ReorderOver { item: item.clone(), to: 0 });
        assert_eq!(m.reorder(), Some(&Reorder { item, to: 0 }));
        m.update(Action::ReorderCancel);
        assert_eq!((names(&m), m.reorder()), (vec!["Arima".into(), "Slack".into(), "Zoom".into()], None));
        // A closing temporary one is not listed: places count the listed rows only.
        m.update(Action::WorkbookNew { uid: "00000000000000ee".into() });
        let temp = id(&m, "Workspace 2");
        let index = row(&m).iter().position(|of| *of == Owner::Workbook { id: temp });
        m.update(Action::SpaceHide { index });
        m.update(Action::WorkbookFold { workbook: id(&m, "Slack") });
        drag(&mut m, "Arima", 2);
        assert_eq!(names(&m), ["Slack", "Zoom", "Arima"]);
        assert!(m.workbooks.closing(temp));
    }

    #[test]
    fn the_order_comes_back_from_the_files_after_a_restart() {
        let mut m = books(&["Arima", "Slack", "Zoom"]);
        drag(&mut m, "Zoom", 0);
        let files: Vec<(String, String)> =
            m.workbooks.stored().map(|book| (book.uid.clone(), crate::workbook::write(book))).collect();
        // Read back in any order, as a folder lists them; one an older build wrote, with none, goes last.
        let mut read: Vec<Workbook> =
            files.iter().rev().map(|(uid, text)| crate::workbook::parse(text, uid).expect("reads")).collect();
        read.insert(0, file(UIDS[3], "Alpha", None));
        let mut again = Model::default();
        again.update(Action::RestoreWorkbooks { workbooks: read }.sent_by(Origin::System));
        assert_eq!(names(&again), ["Zoom", "Arima", "Slack", "Alpha"]);
    }

    #[test]
    fn an_older_files_workspace_read_between_placed_ones_takes_none_of_their_places() {
        let read = vec![file(UIDS[0], "A", Some(0)), file(UIDS[1], "Legacy", None), file(UIDS[2], "B", Some(1))];
        let mut m = Model::default();
        m.update(Action::RestoreWorkbooks { workbooks: read }.sent_by(Origin::System));
        assert_eq!(names(&m), ["A", "B", "Legacy"]);
    }

    #[test]
    fn a_drag_ends_when_its_row_goes() {
        let mut m = books(&["Arima", "Slack"]);
        let item = ReorderItem::Workbook { id: id(&m, "Slack") };
        m.update(Action::ReorderOver { item, to: 0 });
        m.update(Action::WorkbookRemove { workbook: id(&m, "Slack"), tab: None });
        assert_eq!(m.reorder(), None);
    }

    #[test]
    fn only_the_user_reorders() {
        let m = books(&["Arima", "Slack"]);
        let item = ReorderItem::Workbook { id: id(&m, "Slack") };
        for origin in [Origin::Socket, Origin::System] {
            for action in
                [Action::ReorderOver { item: item.clone(), to: 0 }, Action::ReorderDrop, Action::ReorderCancel]
            {
                assert_eq!(m.refusal(&action, origin), Some(REORDER_USERS), "{origin:?} {action:?}");
            }
        }
        assert_eq!(m.refusal(&Action::ReorderOver { item, to: 0 }, Origin::User), None);
    }

    /// The top row's owners, Home first.
    fn row(m: &Model) -> Vec<Owner> {
        m.shown_spaces().into_iter().map(|space| space.of.clone()).collect()
    }

    #[test]
    fn a_space_moves_along_the_top_row_never_before_home() {
        let mut m = Model::default();
        for uid in ["00000000000000e1", "00000000000000e2", "00000000000000e3"] {
            m.update(Action::WorkbookNew { uid: uid.into() });
        }
        let [home, a, b, c] = <[Owner; 4]>::try_from(row(&m)).expect("Home and three");
        let over = |of: &Owner, to| Action::ReorderOver { item: ReorderItem::Space { of: of.clone() }, to };
        m.update(over(&a, 3));
        m.update(Action::ReorderDrop);
        assert_eq!(row(&m), [home.clone(), b.clone(), c.clone(), a.clone()]);
        // Before Home: just after it.
        m.update(over(&c, 0));
        assert_eq!(m.reorder().map(|r| r.to), Some(1));
        m.update(Action::ReorderDrop);
        assert_eq!(row(&m), [home.clone(), c.clone(), b.clone(), a.clone()]);
        // Home itself is not carried.
        m.update(over(&home, 2));
        assert_eq!(m.reorder(), None);
        // Hidden mid-drag: the drag ends.
        m.update(over(&b, 1));
        let index = row(&m).iter().position(|of| *of == b);
        m.update(Action::SpaceHide { index });
        assert_eq!(m.reorder(), None);
    }

    #[test]
    fn the_list_and_the_top_row_share_one_order_and_a_tasks_space_comes_after() {
        let mut m = Model::default();
        for uid in ["00000000000000e1", "00000000000000e2", "00000000000000e3"] {
            m.update(Action::WorkbookNew { uid: uid.into() });
        }
        // A task's space of no workspace: after the workspaces', whatever was selected.
        let (state, assign) = (crate::tasks::State::Running, Default::default());
        m.update(Action::TaskAdd { title: "fix".into(), priority: 1, state, tab: None, assign });
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        m.update(Action::SpaceSelect { index: 1 });
        m.update(Action::WorkbookNew { uid: "00000000000000e4".into() });
        let books =
            |m: &Model| m.workbook_rows().into_iter().map(|row| Owner::Workbook { id: row.id }).collect::<Vec<_>>();
        let row_with = |m: &Model| [vec![Owner::Home], books(m), vec![Owner::Task { id: 1 }]].concat();
        assert_eq!(row(&m), row_with(&m));
        // A row dragged in the list: the top row follows.
        drag(&mut m, "Workspace 4", 0);
        assert_eq!(names(&m)[0], "Workspace 4");
        assert_eq!(row(&m), row_with(&m));
        // A space dragged on the row: the list follows. Not past the task's, nor the task's at all.
        let first = row(&m)[1].clone();
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: first.clone() }, to: 9 });
        assert_eq!(m.reorder().map(|r| r.to), Some(4));
        m.update(Action::ReorderDrop);
        assert_eq!((books(&m).last(), row(&m)), (Some(&first), row_with(&m)));
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: Owner::Task { id: 1 } }, to: 1 });
        assert_eq!(m.reorder(), None);
    }

    #[test]
    fn the_top_rows_order_comes_back_from_the_layout_file() {
        let mut m = Model::default();
        for uid in ["00000000000000e1", "00000000000000e2"] {
            m.update(Action::WorkbookNew { uid: uid.into() });
            m.update(Action::WorkbookKeep { workbook: m.workbooks.list.last().map_or(0, |book| book.id) });
        }
        let last = row(&m)[2].clone();
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: last }, to: 1 });
        m.update(Action::ReorderDrop);
        let names = |m: &Model| row(m).iter().map(|of| m.space_title(of)).collect::<Vec<_>>();
        let before = names(&m);
        let file = m.layout_file(&[], &[], 0);
        let mut again = Model::default();
        again.update(Action::RestoreWorkbooks { workbooks: m.workbooks.list.clone() }.sent_by(Origin::System));
        let layout = Box::new(file);
        again.update(
            Action::RestoreLayout { layout, gone: Vec::new(), home: None, launch: true }.sent_by(Origin::System),
        );
        assert_eq!(names(&again), before);
    }
}
