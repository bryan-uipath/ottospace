//! The user's drag of a Workspaces row or a top-row space to another place: the pointer turned
//! into a place among them (`Action::ReorderOver`). What moves, and where it may go, is the
//! core's (`model::reorder`).

use gpui::{Context, Div, Pixels, Point, Window, div, prelude::*, px, rgb};
use ottospace_core::model::reorder::ReorderItem;
use ottospace_core::{Action, theme};

use crate::{Workspace, sneak::Controls};

/// What a reorder drag carries: the item, and a space's place on the row (its click's).
#[derive(Clone)]
pub(crate) struct DraggedItem {
    pub(crate) item: ReorderItem,
    pub(crate) place: usize,
}

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
            ReorderItem::Workbook { id } => self.row_place(*id, carried.is_some(), f32::from(at.y)),
            ReorderItem::Space { of } => {
                let Some(from) = self.model.shown_spaces().iter().position(|space| space.of == *of) else { return };
                self.space_place(from, carried.is_some(), f32::from(at.x))
            }
        };
        if carried != Some(to) {
            self.dispatch(Action::ReorderOver { item: item.clone(), to }, self.input, window, cx);
        }
    }

    /// The place among the rows other than `id`'s for the pointer at `y`, as if `id`'s were not
    /// drawn (`hidden`: it is not, being carried), so the place holds as the others close up.
    fn row_place(&self, id: ottospace_core::workbook::WorkbookId, hidden: bool, y: f32) -> usize {
        let bounds = |row: u32| self.controls.bounds(&format!("workbook:{row}"));
        let rows = self.model.workbook_rows();
        let from = rows.iter().position(|row| row.id == id).unwrap_or(rows.len());
        let height = bounds(id).filter(|_| !hidden).map_or(0., |b| f32::from(b.size.height));
        let middle = |(at, row): (usize, u32)| {
            let shift = if at > from { height } else { 0. };
            bounds(row).map(|b| f32::from(b.center().y) - shift)
        };
        let others = rows.iter().enumerate().filter(|(_, row)| row.id != id).map(|(at, row)| (at, row.id));
        place_among(&others.map(middle).collect::<Vec<_>>(), y)
    }

    /// The place on the top row of the space at `from` for the pointer at `x`: the others whose
    /// middle is left of it, as if it were not drawn (`hidden`: it is not, being carried).
    fn space_place(&self, from: usize, hidden: bool, x: f32) -> usize {
        let width = self.controls.bounds(&format!("space:{from}")).filter(|_| !hidden);
        let width = width.map_or(0., |b| f32::from(b.size.width));
        let others = (0..self.model.shown_spaces().len()).filter(|place| *place != from);
        let middle = |place: usize| {
            let shift = if place > from { width } else { 0. };
            self.controls.bounds(&format!("space:{place}")).map(|b| f32::from(b.center().x) - shift)
        };
        place_among(&others.map(middle).collect::<Vec<_>>(), x)
    }

    /// Released over the list or the row: the item carried takes its place; one never carried
    /// (it moved less than `THRESHOLD`) was a click.
    pub(crate) fn reorder_released(&mut self, dragged: &DraggedItem, window: &Window, cx: &mut Context<Self>) {
        // Where it is let go counts: a drag of one move has no move after it began.
        let at = window.mouse_position();
        self.reorder_moved(&dragged.item, at, window, cx);
        let near = self.reorder_from.take().is_some_and(|from| f64::from((at - from).magnitude()) < THRESHOLD);
        if self.model.reorder().is_some_and(|reorder| reorder.item == dragged.item) {
            self.dispatch(Action::ReorderDrop, self.input, window, cx);
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

/// Where the item carried lands: a bar in the accent colour on the edge of the one it goes before
/// (`end`: after). `across`: the top row's, upright; else a Workspaces row's, level.
pub(crate) fn bar(controls: &Controls, across: bool, end: bool) -> Div {
    let bar = div().absolute().rounded_full().bg(rgb(theme::PRIMARY)).child(controls.mark("reorder-bar"));
    match (across, end) {
        (true, false) => bar.top_0().bottom_0().left_0().w(px(2.)),
        (true, true) => bar.top_0().bottom_0().right_0().w(px(2.)),
        (false, false) => bar.left_0().right_0().h(px(2.)).top(px(-3.)),
        (false, true) => bar.left_0().right_0().h(px(2.)).bottom(px(-3.)),
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
