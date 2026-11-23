//! The user's drag that reorders: a row of the Workspaces section, any space on the top row but
//! Home, or a repository's name in the Repos section (kept in the settings). The workspaces have
//! one order, the workbooks' (kept in their files): their spaces take their places on the row in it. A space of no workspace (a task's, a session's) keeps its own
//! place on the row, and the workspaces' go around it (`settle_space_order`). The shell says where
//! the pointer is (`ReorderOver`); the core says what moves.

use serde::{Deserialize, Serialize};

use super::{Effect, Model, Owner, Space};
use crate::repos;
use crate::workbook::WorkbookId;

/// Why a client's reorder is refused: the order follows the user's pointer.
pub const REORDER_USERS: &str = "only the user drags a row or a tab to another place";

/// What a reorder drag carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReorderItem {
    /// A row of the Workspaces section.
    Workbook { id: WorkbookId },
    /// A space on the top row: a workspace's (its task's, while bound to one) or another. Never Home's.
    Space { of: Owner },
    /// A repository's name in the Repos section, by its path: one the settings name as itself.
    Repo { path: String },
    /// A row of the Projects section, by its path: it goes among the placed ones (`Projects::place`).
    /// The path is never in the state.
    Project {
        #[serde(skip)]
        path: String,
    },
}

/// A drag in flight: `item`, and the place it takes if dropped now, among the rows listed (the
/// section's), the spaces shown (the top row's, Home at 0) or the Repos entries drawn.
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

    /// The least and most place `item` may take (Home stays first; a pinned repository among
    /// the pinned). None: it is not there to move.
    fn reorder_places(&self, item: &ReorderItem) -> Option<(usize, usize)> {
        match item {
            ReorderItem::Workbook { id } => {
                let listed: Vec<WorkbookId> = self.workbooks.listed().map(|book| book.id).collect();
                listed.contains(id).then(|| (0, listed.len() - 1))
            }
            // Anywhere after Home.
            ReorderItem::Space { of } => {
                let shown = self.shown_spaces();
                (*of != Owner::Home && shown.iter().any(|space| space.of == *of)).then(|| (1, shown.len() - 1))
            }
            ReorderItem::Repo { path } => {
                let entries = self.repo_entries();
                let dirs = &self.repo_dirs.dirs;
                // A repository's own entry, not a group's folder.
                let listed = self.repos.iter().any(|repo| repo.path == *path) && !self.repo_dirs.fixed;
                let own = entries.iter().position(|at| dirs[*at].path == *path).filter(|_| listed)?;
                let pinned = entries.iter().filter(|at| dirs[**at].pinned).count();
                Some(if own < pinned { (0, pinned - 1) } else { (pinned, entries.len() - 1) })
            }
            ReorderItem::Project { path } => self.projects.places(path),
        }
    }

    /// The Repos entries drawn, by index in `repo_dirs`, in the section's order: each repository's
    /// own, and each group's folder. One with nothing found in it is not drawn.
    pub fn repo_entries(&self) -> Vec<usize> {
        let mut entries: Vec<usize> = self.repos.iter().filter_map(|repo| self.repo_dirs.place(&repo.path)).collect();
        entries.dedup();
        entries
    }

    /// The user carries `item` over place `to`: a drag starts, or goes on. Kept within the list.
    pub(crate) fn reorder_over(&mut self, item: ReorderItem, to: usize) {
        self.reorder = self.reorder_places(&item).map(|(least, most)| Reorder { item, to: to.clamp(least, most) });
    }

    /// The drop: the item carried takes its place. A workbook's makes the section's order the
    /// user's from then on, kept in each file (`Workbook::order`); a repository's, in the settings.
    pub(crate) fn reorder_drop(&mut self, effects: &mut Vec<Effect>) {
        let Some(Reorder { item, to }) = self.reorder.take() else { return };
        match item {
            ReorderItem::Workbook { id } => {
                let listed: Vec<bool> = self.workbooks.list.iter().map(|book| book.closing.is_none()).collect();
                self.workbooks_move(id, to, &listed);
            }
            // Moved on the row; a workspace's moves the same in the one order, among the
            // workspaces shown: past another space, or a hidden one, the list does not change.
            ReorderItem::Space { of } => {
                let shown: Vec<bool> = self.spaces.iter().map(|space| !space.hidden).collect();
                let Some(from) = self.spaces.iter().position(|space| space.of == of) else { return };
                if !move_among(&mut self.spaces, from, to, &shown) {
                    return;
                }
                let Some(id) = self.space_book(&of).map(|book| book.id) else { return };
                let shown = self.spaces.iter().filter(|space| !space.hidden);
                let books: Vec<WorkbookId> =
                    shown.filter_map(|space| self.space_book(&space.of)).map(|book| book.id).collect();
                let Some(place) = books.iter().position(|book| *book == id) else { return };
                let on_row: Vec<bool> = self.workbooks.list.iter().map(|book| books.contains(&book.id)).collect();
                self.workbooks_move(id, place, &on_row);
            }
            ReorderItem::Repo { path } => {
                // The row selected stays selected where it now is.
                let selected = self.list.and_then(|at| self.list_rows().get(at).cloned());
                let entries = self.repo_entries();
                let dirs = &mut self.repo_dirs.dirs;
                let drawn: Vec<bool> = (0..dirs.len()).map(|at| entries.contains(&at)).collect();
                let Some(from) = dirs.iter().position(|dir| dir.path == path) else { return };
                if !move_among(dirs, from, to, &drawn) {
                    return;
                }
                // Past an entry not drawn: pinned ones first again, e.g. [P, x, C] with P after x.
                self.repo_dirs.settle();
                self.repos = repos::arrange(std::mem::take(&mut self.repos), &self.repo_dirs);
                self.list = selected.and_then(|row| self.list_rows().iter().position(|r| *r == row)).or(self.list);
                effects.push(Effect::SaveRepos { dirs: self.repo_dirs.dirs.clone() });
            }
            ReorderItem::Project { path } => {
                let selected = self.list.and_then(|at| self.list_rows().get(at).cloned());
                self.projects.place(&path, to);
                self.list = selected.and_then(|row| self.list_rows().iter().position(|r| *r == row)).or(self.list);
                effects.push(Effect::SaveProjects { kept: self.projects.kept() });
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

    /// After every step (Home already first): the top row in the one order. A space of no
    /// workspace keeps its place; the workspaces' spaces take the other places, as the section
    /// lists them; a hidden one keeps its place, to come back to.
    /// One new this step: a workspace's just after the one before it in the list shown on the
    /// row (else just after Home), any other last. e.g. [Home, A, task, B], the list B, A:
    /// [Home, B, task, A]; then C new, listed last: [Home, B, task, A, C]; a task new: at the end.
    pub(crate) fn settle_space_order(&mut self) {
        let list = |of: &Owner| {
            let id = self.space_book(of).map(|book| book.id);
            id.and_then(|id| self.workbooks.list.iter().position(|book| book.id == id))
        };
        let keys: Vec<Option<usize>> = self.spaces.iter().map(|space| list(&space.of)).collect();
        let keyed = keys.into_iter().zip(std::mem::take(&mut self.spaces));
        let (mut row, new): (Vec<_>, Vec<_>) = keyed.partition(|(_, space)| space.placed || space.of == Owner::Home);
        // The shown workspaces' places, filled in the list's order.
        let fills = |(key, space): &&mut (Option<usize>, Space)| key.is_some() && !space.hidden;
        let mut books: Vec<_> = row.iter_mut().filter(fills).map(|slot| slot.clone()).collect();
        books.sort_by_key(|(key, _)| *key);
        for (slot, book) in row.iter_mut().filter(fills).zip(books) {
            *slot = book;
        }
        let (mut fresh, others): (Vec<_>, Vec<_>) = new.into_iter().partition(|(key, _)| key.is_some());
        fresh.sort_by_key(|(key, _)| *key);
        for (key, space) in fresh {
            // Home is first (`settle_spaces`).
            let after = row.iter().rposition(|(other, s)| !s.hidden && other.is_some() && *other < key).unwrap_or(0);
            row.insert(after + 1, (key, space));
        }
        row.extend(others);
        self.spaces = row.into_iter().map(|(_, space)| Space { placed: true, ..space }).collect();
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
    fn the_list_and_the_top_row_share_one_order_and_a_new_tasks_space_comes_last() {
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
        let row_with = |m: &Model| [vec![Owner::Home], listed(m), vec![Owner::Task { id: 1 }]].concat();
        assert_eq!(row(&m), row_with(&m));
        // A row dragged in the list: the top row follows.
        drag(&mut m, "Workspace 4", 0);
        assert_eq!(names(&m)[0], "Workspace 4");
        assert_eq!(row(&m), row_with(&m));
        // A space dragged on the row to its end, past the task's: the list follows.
        let first = row(&m)[1].clone();
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: first.clone() }, to: 9 });
        assert_eq!(m.reorder().map(|r| r.to), Some(5));
        m.update(Action::ReorderDrop);
        let mut after = row_with(&m);
        after.swap(4, 5);
        assert_eq!((listed(&m).last(), row(&m)), (Some(&first), after));
    }

    /// Home, three kept workspaces a, b, c (listed so), and task 1's space at the row's end.
    fn three_and_a_task() -> (Model, [Owner; 4]) {
        let mut m = Model::default();
        for uid in ["00000000000000e1", "00000000000000e2", "00000000000000e3"] {
            m.update(Action::WorkbookNew { uid: uid.into() });
            m.update(Action::WorkbookKeep { workbook: m.workbooks.list.last().map_or(0, |book| book.id) });
        }
        let (state, assign) = (crate::tasks::State::Running, Default::default());
        m.update(Action::TaskAdd { title: "fix".into(), priority: 1, state, tab: None, assign });
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        let [_, a, b, c, t] = <[Owner; 5]>::try_from(row(&m)).expect("Home, three and a task");
        assert_eq!(t, Owner::Task { id: 1 });
        (m, [a, b, c, t])
    }

    fn carry(m: &mut Model, of: &Owner, to: usize) {
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: of.clone() }, to });
        m.update(Action::ReorderDrop);
    }

    fn listed(m: &Model) -> Vec<Owner> {
        m.workbook_rows().into_iter().map(|row| Owner::Workbook { id: row.id }).collect()
    }

    #[test]
    fn a_tasks_tab_goes_anywhere_after_home_and_the_list_stays() {
        let (mut m, [a, b, c, t]) = three_and_a_task();
        let list = listed(&m);
        carry(&mut m, &t, 2);
        assert_eq!(row(&m), [Owner::Home, a.clone(), t.clone(), b.clone(), c.clone()]);
        // Before Home: just after it.
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: t.clone() }, to: 0 });
        assert_eq!(m.reorder().map(|r| r.to), Some(1));
        m.update(Action::ReorderDrop);
        assert_eq!(row(&m), [Owner::Home, t, a, b, c]);
        assert_eq!(listed(&m), list);
    }

    #[test]
    fn a_workspace_dragged_past_a_tasks_tab_moves_in_the_list_only_among_the_workspaces() {
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 2);
        // a past the task's tab alone: the list is as it was.
        carry(&mut m, &a, 2);
        assert_eq!(row(&m), [Owner::Home, t.clone(), a.clone(), b.clone(), c.clone()]);
        assert_eq!(listed(&m), [a.clone(), b.clone(), c.clone()]);
        // a past b: the list too.
        carry(&mut m, &a, 3);
        assert_eq!(row(&m), [Owner::Home, t, b.clone(), a.clone(), c.clone()]);
        assert_eq!(listed(&m), [b, a, c]);
    }

    #[test]
    fn a_tasks_tab_keeps_its_place_as_the_workspaces_around_it_close_come_back_and_move() {
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 2);
        let hide = |m: &mut Model, of: &Owner| {
            let index = row(m).iter().position(|shown| shown == of);
            m.update(Action::SpaceHide { index });
        };
        // a closed: the task's tab is second; a back: where it was, before it.
        hide(&mut m, &a);
        assert_eq!(row(&m), [Owner::Home, t.clone(), b.clone(), c.clone()]);
        m.update(Action::SpaceOpen { space: a.clone() });
        assert_eq!(row(&m), [Owner::Home, a.clone(), t.clone(), b.clone(), c.clone()]);
        // c dragged first in the list: the workspaces' places take the list's order, the task's stays.
        let Owner::Workbook { id } = c else { panic!("a workspace's space") };
        m.update(Action::ReorderOver { item: ReorderItem::Workbook { id }, to: 0 });
        m.update(Action::ReorderDrop);
        assert_eq!(row(&m), [Owner::Home, c.clone(), t.clone(), a.clone(), b.clone()]);
        // c removed: the ones after it move up one.
        m.update(Action::WorkbookRemove { workbook: id, tab: None });
        assert_eq!(row(&m), [Owner::Home, t, a, b]);
    }

    fn hide(m: &mut Model, of: &Owner) {
        let index = row(m).iter().position(|shown| shown == of);
        m.update(Action::SpaceHide { index });
    }

    /// The whole row, hidden spaces too.
    fn all(m: &Model) -> Vec<Owner> {
        m.spaces().iter().map(|space| space.of.clone()).collect()
    }

    #[test]
    fn a_hidden_workspace_moves_nothing_shown_and_is_not_moved_past() {
        // a past the task's tab alone, b hidden after it: the list as it was.
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 2);
        hide(&mut m, &b);
        carry(&mut m, &a, 2);
        assert_eq!(row(&m), [Owner::Home, t.clone(), a.clone(), c.clone()]);
        assert_eq!(listed(&m), [a.clone(), b.clone(), c.clone()]);
        // a hidden, c dragged first in the list: the task's tab stays second.
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 2);
        hide(&mut m, &a);
        let Owner::Workbook { id } = c else { panic!("a workspace's space") };
        m.update(Action::ReorderOver { item: ReorderItem::Workbook { id }, to: 0 });
        m.update(Action::ReorderDrop);
        assert_eq!(row(&m), [Owner::Home, t.clone(), c.clone(), b.clone()]);
        assert_eq!(all(&m), [Owner::Home, a.clone(), t.clone(), c.clone(), b.clone()]);
        // a back, listed c, a, b: the shown workspaces' places in that order, the task's tab stays.
        m.update(Action::SpaceOpen { space: a.clone() });
        assert_eq!(row(&m), [Owner::Home, c, t, a, b]);
    }

    #[test]
    fn a_new_workspace_goes_after_the_one_before_it_shown_not_a_hidden_one() {
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 1);
        for of in [&a, &b, &c] {
            hide(&mut m, of);
        }
        m.update(Action::WorkbookNew { uid: "00000000000000e4".into() });
        let d = row(&m)[1].clone();
        assert_eq!((listed(&m).last(), row(&m)), (Some(&d), vec![Owner::Home, d.clone(), t]));
    }

    #[test]
    fn a_new_workspace_goes_after_the_one_before_it_in_the_list_and_a_new_tasks_tab_last() {
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 1);
        // Listed last: after c, the task's tab where it was; a new task's tab at the end.
        m.update(Action::WorkbookNew { uid: "00000000000000e4".into() });
        let d = row(&m)[5].clone();
        assert_eq!(listed(&m).last(), Some(&d));
        assert_eq!(row(&m), [Owner::Home, t.clone(), a.clone(), b.clone(), c.clone(), d.clone()]);
        let (state, assign) = (crate::tasks::State::Running, Default::default());
        m.update(Action::TaskAdd { title: "two".into(), priority: 1, state, tab: None, assign });
        m.update(Action::SpaceSelect { index: 2 });
        m.update(Action::SpaceOpen { space: Owner::Task { id: 2 } });
        assert_eq!(row(&m), [Owner::Home, t, a, b, c, d, Owner::Task { id: 2 }]);
    }

    #[test]
    fn a_carried_tasks_tab_that_closes_ends_the_drag() {
        let (mut m, [.., t]) = three_and_a_task();
        m.update(Action::ReorderOver { item: ReorderItem::Space { of: t.clone() }, to: 1 });
        assert_eq!(m.reorder().map(|r| r.to), Some(1));
        m.update(Action::SpaceHide { index: Some(4) });
        assert_eq!(m.reorder(), None);
        // Not dropped: the row is as it was, the task's off it.
        assert_eq!(row(&m).len(), 4);
    }

    #[test]
    fn a_tasks_tabs_place_comes_back_from_the_layout_file() {
        let (mut m, [a, b, c, t]) = three_and_a_task();
        carry(&mut m, &t, 2);
        let file = m.layout_file(&[], &[], 0);
        let mut again = Model::default();
        again.update(Action::RestoreTasks { tasks: m.tasks.list.clone(), floor: 9 }.sent_by(Origin::System));
        again.update(Action::RestoreWorkbooks { workbooks: m.workbooks.list.clone() }.sent_by(Origin::System));
        let layout = Box::new(file);
        again.update(
            Action::RestoreLayout { layout, gone: Vec::new(), home: None, launch: true }.sent_by(Origin::System),
        );
        assert_eq!(row(&again), [Owner::Home, a, t, b, c]);
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

    /// Repos listing `entries` (`!` before a path: pinned; `/old` a group holding `/old/x`, `/old/y`).
    fn repos_model(entries: &[&str]) -> Model {
        let mut m = Model::default();
        let dir = |e: &&str| {
            let path = e.trim_start_matches('!');
            repos::RepoDir { pinned: e.starts_with('!'), ..repos::RepoDir::new(path, path) }
        };
        let dirs = repos::RepoDirs { dirs: entries.iter().map(dir).collect(), ..Default::default() };
        let paths = entries.iter().map(|e| e.trim_start_matches('!')).flat_map(|path| match path {
            "/old" => vec!["/old/x".to_string(), "/old/y".to_string()],
            _ => vec![path.to_string()],
        });
        let repo = |path: String| repos::Repo { name: repos::name_of(&path), path, branch: None, worktrees: vec![] };
        let found = paths.map(repo).collect();
        m.update(Action::SetRepoDirs { dirs }.sent_by(Origin::System));
        m.update(Action::SetRepos { repos: found }.sent_by(Origin::System));
        m
    }

    fn repo_paths(m: &Model) -> Vec<&str> {
        m.repos.iter().map(|repo| repo.path.as_str()).collect()
    }

    fn move_repo(m: &mut Model, path: &str, to: usize) -> Vec<Effect> {
        m.update(Action::ReorderOver { item: ReorderItem::Repo { path: path.into() }, to });
        m.update(Action::ReorderDrop)
    }

    #[test]
    fn a_repository_dragged_takes_its_place_and_the_settings_keep_it() {
        let mut m = repos_model(&["/a", "/b", "/c"]);
        let effects = move_repo(&mut m, "/a", 2);
        assert_eq!(repo_paths(&m), ["/b", "/c", "/a"]);
        let written = effects.iter().find_map(|e| match e {
            Effect::SaveRepos { dirs } => Some(dirs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>()),
            _ => None,
        });
        assert_eq!(written, Some(vec!["/b", "/c", "/a"]));
        assert_eq!(m.reorder(), None);
        // Onto itself: nothing moves, nothing is written.
        assert!(!move_repo(&mut m, "/c", 1).iter().any(|e| matches!(e, Effect::SaveRepos { .. })));
        // A cancelled one moves nothing either.
        m.update(Action::ReorderOver { item: ReorderItem::Repo { path: "/a".into() }, to: 0 });
        m.update(Action::ReorderCancel);
        assert_eq!((repo_paths(&m), m.reorder()), (vec!["/b", "/c", "/a"], None));
    }

    #[test]
    fn a_folded_repository_moves_the_same_and_the_selected_row_stays_selected() {
        let mut m = repos_model(&["/a", "/b", "/c"]);
        m.update(Action::RepoFold { path: "/b".into(), fold: Some(true) });
        m.sidebar = Some(super::super::Section::Repos);
        let row = crate::list::ListRow::Repo("/c".into());
        m.list = m.list_rows().iter().position(|r| *r == row);
        move_repo(&mut m, "/b", 0);
        assert_eq!(repo_paths(&m), ["/b", "/a", "/c"]);
        assert_eq!(m.list.and_then(|at| m.list_rows().get(at).cloned()), Some(row));
    }

    #[test]
    fn a_pinned_repository_moves_among_the_pinned_and_another_after_them() {
        let mut m = repos_model(&["!/p", "!/q", "/a", "/b"]);
        m.update(Action::ReorderOver { item: ReorderItem::Repo { path: "/b".into() }, to: 0 });
        assert_eq!(m.reorder().map(|r| r.to), Some(2));
        m.update(Action::ReorderDrop);
        m.update(Action::ReorderOver { item: ReorderItem::Repo { path: "/p".into() }, to: 9 });
        assert_eq!(m.reorder().map(|r| r.to), Some(1));
        m.update(Action::ReorderDrop);
        assert_eq!(repo_paths(&m), ["/q", "/p", "/b", "/a"]);
        // An unpinned entry with nothing found, between: a pinned one still goes before it.
        let mut m = repos_model(&["!/p", "!/q", "/gone", "/a"]);
        m.update(Action::SetRepos { repos: m.repos.iter().filter(|r| r.path != "/gone").cloned().collect() });
        move_repo(&mut m, "/p", 1);
        let dirs: Vec<&str> = m.repo_dirs.dirs.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(dirs, ["/q", "/p", "/gone", "/a"]);
    }

    #[test]
    fn a_group_is_a_place_but_neither_it_nor_its_repositories_are_carried() {
        let mut m = repos_model(&["/a", "/old", "/b"]);
        assert_eq!(repo_paths(&m), ["/a", "/old/x", "/old/y", "/b"]);
        for path in ["/old", "/old/x", "/elsewhere"] {
            m.update(Action::ReorderOver { item: ReorderItem::Repo { path: path.into() }, to: 0 });
            assert_eq!(m.reorder(), None, "{path}");
        }
        // Above the group: its repositories stay under it, after the one moved.
        move_repo(&mut m, "/b", 1);
        assert_eq!(repo_paths(&m), ["/a", "/b", "/old/x", "/old/y"]);
        // An entry with nothing found in it is no place, and stays where it was.
        let mut m = repos_model(&["/a", "/gone", "/b"]);
        m.update(Action::SetRepos { repos: m.repos.iter().filter(|r| r.path != "/gone").cloned().collect() });
        move_repo(&mut m, "/b", 0);
        let dirs: Vec<&str> = m.repo_dirs.dirs.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(dirs, ["/b", "/a", "/gone"]);
    }

    #[test]
    fn none_is_carried_while_the_environment_names_the_repositories_or_once_it_is_unlisted() {
        let mut m = repos_model(&["/a", "/b"]);
        m.update(Action::ReorderOver { item: ReorderItem::Repo { path: "/b".into() }, to: 0 });
        let dirs = repos::RepoDirs { fixed: true, ..m.repo_dirs.clone() };
        m.update(Action::SetRepoDirs { dirs }.sent_by(Origin::System));
        assert_eq!(m.reorder(), None);
        m.update(Action::ReorderOver { item: ReorderItem::Repo { path: "/b".into() }, to: 0 });
        assert_eq!(m.reorder(), None);
    }
}
