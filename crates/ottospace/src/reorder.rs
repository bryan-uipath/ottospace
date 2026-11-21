//! The user's drag of a Workspaces row, a top-row space or a repository's name to another place:
//! the pointer turned into a place among them (`Action::ReorderOver`). What moves, and where it
//! may go, is the core's (`model::reorder`).

use gpui::{Context, Div, Pixels, Point, Window, div, prelude::*, px, rgb};
use ottospace_core::list::ListRow;
use ottospace_core::model::reorder::ReorderItem;
use ottospace_core::{Action, theme};

use crate::{Workspace, sneak::Controls};

/// What a reorder drag carries: the item, and a space's place on the row (its click's).
#[derive(Clone)]
pub(crate) struct DraggedItem {
    pub(crate) item: ReorderItem,
    pub(crate) place: usize,
}

/// The Workspaces list's gap between rows (`gap_1`).
pub(crate) const GAP: f32 = 4.;

/// How far the pointer goes from the press before a press is a drag: less is a click.
const THRESHOLD: f64 = 4.;

impl Workspace {
    /// The pointer moved, carrying `item`: past `THRESHOLD`, its place among the rows or spaces.
    pub(crate) fn reorder_moved(
        &mut self,
        item: &ReorderItem,
        at: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let carried = self.model.reorder().filter(|reorder| reorder.item == *item).map(|reorder| reorder.to);
        let far = self.reorder_from.is_none_or(|from| f64::from((at - from).magnitude()) >= THRESHOLD);
        if carried.is_none() && !far {
            return;
        }
        let to = match item {
            ReorderItem::Workbook { id } => {
                // The room it takes now, to the next row (gap and its tabs too), kept while it is not
                // drawn: the list's height holds, so a list scrolled to its end does not jump.
                let bounds = |row: u32| self.controls.bounds(&format!("workbook:{row}"));
                if let Some(b) = bounds(*id) {
                    let rows = self.model.workbook_rows();
                    let next = rows.iter().skip_while(|row| row.id != *id).skip(1).find_map(|row| bounds(row.id));
                    self.reorder_room = closes_up(Some(b), next, |b| b.origin.y).max(f32::from(b.size.height) + GAP);
                    // Not drawn while carried: its hover would never end.
                    if self.row_hovered == Some(ListRow::Workbook(*id)) {
                        self.row_hovered = None;
                    }
                }
                self.row_place(*id, f32::from(at.y))
            }
            ReorderItem::Space { of } => {
                let Some(from) = self.model.shown_spaces().iter().position(|space| space.of == *of) else { return };
                // The room it takes now, kept while it is not drawn: the row's drop area stays as wide.
                if let Some(b) = self.controls.bounds(&format!("space:{from}")) {
                    self.reorder_room = f32::from(b.size.width);
                }
                self.space_place(from, f32::from(at.x))
            }
            ReorderItem::Repo { path } => {
                let bounds = |path: &str| self.controls.bounds(&format!("repo-fold:{path}"));
                // The room it takes now, to the next entry's name (its checkouts too).
                if let Some(b) = bounds(path) {
                    let paths = self.repo_entry_paths();
                    let next = paths.iter().skip_while(|p| *p != path).skip(1).find_map(|p| bounds(p));
                    self.reorder_room = closes_up(Some(b), next, |b| b.origin.y).max(f32::from(b.size.height));
                }
                self.repo_place(path, f32::from(at.y))
            }
        };
        if carried != Some(to) {
            self.dispatch(Action::ReorderOver { item: item.clone(), to }, self.input, window, cx);
        }
    }

    /// The place among the rows other than `id`'s for the pointer at `y`, as if `id`'s were not
    /// drawn, so the place holds as the others close up.
    fn row_place(&self, id: ottospace_core::workbook::WorkbookId, y: f32) -> usize {
        let bounds = |row: u32| self.controls.bounds(&format!("workbook:{row}"));
        let rows = self.model.workbook_rows();
        let from = rows.iter().position(|row| row.id == id).unwrap_or(rows.len());
        let after = rows.iter().skip(from + 1).map(|row| row.id);
        let shift = closes_up(bounds(id), after.filter_map(bounds).next(), |b| b.origin.y);
        let middle =
            |(at, row): (usize, u32)| bounds(row).map(|b| f32::from(b.center().y) - if at > from { shift } else { 0. });
        let others = rows.iter().enumerate().filter(|(_, row)| row.id != id).map(|(at, row)| (at, row.id));
        place_among(&others.map(middle).collect::<Vec<_>>(), y)
    }

    /// The Repos entries' paths as drawn (`Model::repo_entries`): a repository's own, a group's folder.
    pub(crate) fn repo_entry_paths(&self) -> Vec<String> {
        let dirs = &self.model.repo_dirs.dirs;
        self.model.repo_entries().into_iter().map(|at| dirs[at].path.clone()).collect()
    }

    /// The place among the Repos entries other than `path`'s for the pointer at `y`, as if its
    /// name and checkouts were not drawn.
    fn repo_place(&self, path: &str, y: f32) -> usize {
        let bounds = |path: &str| self.controls.bounds(&format!("repo-fold:{path}"));
        let paths = self.repo_entry_paths();
        let from = paths.iter().position(|p| p == path).unwrap_or(paths.len());
        let shift = closes_up(bounds(path), paths.iter().skip(from + 1).find_map(|p| bounds(p)), |b| b.origin.y);
        let middle =
            |(at, p): (usize, &String)| bounds(p).map(|b| f32::from(b.center().y) - if at > from { shift } else { 0. });
        let others = paths.iter().enumerate().filter(|(_, p)| *p != path);
        place_among(&others.map(middle).collect::<Vec<_>>(), y)
    }

    /// The place on the top row of the space at `from` for the pointer at `x`: the others whose
    /// middle is left of it, as if it were not drawn.
    fn space_place(&self, from: usize, x: f32) -> usize {
        let bounds = |place: usize| self.controls.bounds(&format!("space:{place}"));
        let len = self.model.shown_spaces().len();
        let shift = closes_up(bounds(from), (from + 1..len).filter_map(bounds).next(), |b| b.origin.x);
        let middle =
            |place: usize| bounds(place).map(|b| f32::from(b.center().x) - if place > from { shift } else { 0. });
        place_among(&(0..len).filter(|place| *place != from).map(middle).collect::<Vec<_>>(), x)
    }

    /// Released over the list or the row: the item carried takes its place; one never carried
    /// (it moved less than `THRESHOLD`) was a click.
    pub(crate) fn reorder_released(&mut self, dragged: &DraggedItem, window: &Window, cx: &mut Context<Self>) {
        // Where it is let go counts: a drag of one move has no move after it began.
        let at = window.mouse_position();
        self.reorder_moved(&dragged.item, at, window, cx);
        let near = self.reorder_from.take().is_some_and(|from| f64::from((at - from).magnitude()) < THRESHOLD);
        if self.model.reorder().is_some_and(|reorder| reorder.item == dragged.item) {
            match dragged.item {
                // From the settings as they are now: an entry written by hand since stays.
                ReorderItem::Repo { .. } => self.change_repos(Action::ReorderDrop, self.input, window, cx),
                _ => drop(self.dispatch(Action::ReorderDrop, self.input, window, cx)),
            }
            return;
        }
        // A click only if it stayed put: a client's drag, refused, is no click either.
        if !near {
            return;
        }
        match &dragged.item {
            ReorderItem::Workbook { id } => self.click_workbook("workbook", *id, None, window, cx),
            ReorderItem::Space { .. } => {
                self.dispatch(Action::SpaceClick { index: dragged.place }, self.input, window, cx);
            }
            ReorderItem::Repo { path } => {
                self.dispatch(Action::RepoFold { path: path.clone(), fold: None }, self.input, window, cx);
            }
        }
    }

    /// A release anywhere else, or Esc: the drag ends and nothing moves.
    pub(crate) fn reorder_cancel(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.reorder_from = None;
        if self.model.reorder().is_some() {
            self.dispatch(Action::ReorderCancel, self.input, window, cx);
        }
    }
}

/// How far the ones after the item carried move up or left once it is not drawn: from its start to
/// the next one's, as the last frame drew them (gap and anything under it too). None drawn: 0.
fn closes_up(
    carried: Option<gpui::Bounds<Pixels>>,
    next: Option<gpui::Bounds<Pixels>>,
    along: impl Fn(&gpui::Bounds<Pixels>) -> Pixels,
) -> f32 {
    carried.zip(next).map_or(0., |(carried, next)| f32::from(along(&next) - along(&carried)))
}

/// Where the item carried lands: a bar in the accent colour on the edge of the one it goes before
/// (`end`: after; the top row's only). `across`: the top row's, upright; else a list row's, level.
pub(crate) fn bar(controls: &Controls, across: bool, end: bool) -> Div {
    let bar = div().absolute().rounded_full().bg(rgb(theme::PRIMARY)).child(controls.mark("reorder-bar"));
    match (across, end) {
        (true, false) => bar.top_0().bottom_0().left_0().w(px(2.)),
        (true, true) => bar.top_0().bottom_0().right_0().w(px(2.)),
        (false, _) => bar.left_0().right_0().h(px(2.)).top(px(-3.)),
    }
}

/// The place among others (each's middle, `None` if not drawn: scrolled out, or off a row short of
/// room) for the pointer at `p`: before the first drawn one past it, else after the last drawn.
/// e.g. [None, 10, 30, None], p 20 → 2; p 40 → 3.
fn place_among(middles: &[Option<f32>], p: f32) -> usize {
    let drawn = || middles.iter().enumerate().filter_map(|(at, middle)| Some((at, (*middle)?)));
    match drawn().find(|(_, middle)| p < *middle) {
        Some((at, _)) => at,
        None => drawn().last().map_or(0, |(at, _)| at + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::place_among;

    #[test]
    fn a_place_counts_what_is_not_drawn_where_it_lies() {
        let middles = [None, Some(10.), Some(30.), None];
        assert_eq!([0., 20., 40.].map(|p| place_among(&middles, p)), [1, 2, 3]);
        assert_eq!(place_among(&[None, None], 5.), 0);
    }
}
