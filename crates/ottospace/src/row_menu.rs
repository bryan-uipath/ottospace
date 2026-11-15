//! A row's menu in the shell: the … on a row, and the short list it opens under it. What it
//! offers and what a key does are the core's (`ottospace_core::row_menu`).

use gpui::{Context, ElementId, Stateful, Window, div, prelude::*, px, rgb};
use ottospace_core::list::ListRow;
use ottospace_core::row_menu::menu_key;
use ottospace_core::{Action, theme};

use crate::{Workspace, dialog, rect};

/// The menu's width, in points.
const WIDTH: f32 = 150.;

/// The control id of an item of the open menu: `row-menu:1`.
fn item_control(index: usize) -> String {
    format!("row-menu:{index}")
}

impl Workspace {
    /// The … of `row`, drawn as the control `control`: shown while `shown` (the pointer is on its
    /// row, list mode's selection is, or its menu is open), there for a label always.
    pub(crate) fn row_dots(
        &self,
        control: String,
        row: ListRow,
        shown: bool,
        cx: &Context<Self>,
    ) -> Stateful<gpui::Div> {
        let open = self.model.row_menu().is_some_and(|(menu, _)| menu.row == row);
        div()
            .id(ElementId::Name(control.clone().into()))
            .absolute()
            .right(px(6.))
            .bottom(px(4.))
            .px_1()
            .rounded_sm()
            .text_color(rgb(theme::TEXT))
            .opacity(if shown || open { 1. } else { 0. })
            .when(open, |d| d.bg(rgb(theme::BORDER_STRONG)))
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
            .child("…")
            .child(self.controls.mark(control))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation(); // not a click on the row
                this.row_menu_act(Action::RowMenu { row: row.clone() }, window, cx);
            }))
    }

    /// Sends a menu action as whoever is acting; refused, the status bar says why.
    pub(crate) fn row_menu_act(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) {
        if let Some(why) = self.model.refusal(&action, self.input).or(self.model.waits(&action)) {
            self.error = Some(why.into());
            return cx.notify();
        }
        // Where its … is now: `render` cannot ask, the controls are noted as they are painted.
        if let Action::RowMenu { row } = &action {
            self.row_menu_at = Self::row_menu_anchor(row).and_then(|control| self.controls.bounds(&control));
        }
        // An undo it offers counts its seconds from now.
        self.model.update(Action::Tick { now: crate::now_ms() }.sent_by(ottospace_core::Origin::System));
        self.dispatch(action, self.input, window, cx);
    }

    /// A key while a menu is open: it is the menu's. `false`: no menu is open.
    pub(crate) fn row_menu_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(action) = self.model.row_menu().map(|(menu, _)| menu_key(menu, key)) else { return false };
        self.row_menu_act(action, window, cx);
        true
    }

    /// The control that opened the menu: the … of its row.
    fn row_menu_anchor(row: &ListRow) -> Option<String> {
        match row {
            ListRow::Workbook(id) => Some(format!("workbook-menu:{id}")),
            ListRow::WorkbookTab(id, at) => Some(format!("workbook-tab-menu:{id}:{at}")),
            _ => None,
        }
    }

    /// The open menu: its items under the … that opened it, each with its key, over a backdrop
    /// that takes the click outside and closes it.
    pub(crate) fn render_row_menu(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (menu, items) = self.model.row_menu()?;
        let anchor = self.row_menu_at?;
        let width = f32::from(window.viewport_size().width);
        let left = f32::from(anchor.left()).min(width - WIDTH - 4.).max(4.);
        // The click outside ends on the backdrop: closed on the press, its release would reach
        // what is under it.
        let backdrop = div()
            .id("row-menu-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| this.row_menu_act(Action::RowMenuClose, window, cx)));
        let rows = items.iter().enumerate().map(|(index, item)| {
            let on = index == menu.selected;
            div()
                .id(("row-menu", index))
                .relative()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .rounded_sm()
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(theme::SELECTED)))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(item.label())
                .child(div().text_xs().text_color(rgb(theme::MUTED)).child(item.key().to_uppercase()))
                .child(self.controls.mark(item_control(index)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.row_menu_act(Action::RowMenuChoose { index: Some(index) }, window, cx);
                }))
        });
        let list = dialog::floating(WIDTH).absolute().left(px(left)).top(anchor.bottom() + px(2.)).p_1().text_sm();
        Some(backdrop.child(list.children(rows)))
    }

    /// The open menu as the state shows it: its row, the item the keys are on, and each item
    /// with its key and where it was drawn.
    pub(crate) fn row_menu_json(&self) -> serde_json::Value {
        let Some((menu, items)) = self.model.row_menu() else { return serde_json::Value::Null };
        let (workbook, tab) = match menu.row {
            ListRow::Workbook(id) => (Some(id), None),
            ListRow::WorkbookTab(id, at) => (Some(id), Some(at)),
            _ => (None, None),
        };
        let item = |(index, item): (usize, &ottospace_core::row_menu::Item)| {
            let drawn = self.controls.bounds(&item_control(index)).map(rect);
            serde_json::json!({"item": item, "label": item.label(), "key": item.key(), "rect": drawn})
        };
        let items: Vec<serde_json::Value> = items.iter().enumerate().map(item).collect();
        serde_json::json!({"workbook": workbook, "tab": tab, "selected": menu.selected, "items": items})
    }

    /// A sneak label on an item of the menu: `row-menu:1`.
    pub(crate) fn click_row_menu(&mut self, name: &str, window: &Window, cx: &mut Context<Self>) {
        if let Ok(index) = name.parse() {
            self.row_menu_act(Action::RowMenuChoose { index: Some(index) }, window, cx);
        }
    }
}
