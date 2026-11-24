//! A row's menu in the shell: the … on a row, and the short list it opens under it (a Repos row
//! has none: a right click opens its menu at the pointer). What it offers, what a key does and
//! when it closes are the core's (`ottospace_core::row_menu`).

use gpui::{Context, ElementId, MouseButton, MouseDownEvent, Stateful, Window, div, prelude::*, px, rgb};
use ottospace_core::layout::Rect;
use ottospace_core::list::ListRow;
use ottospace_core::row_menu::{MenuOf, menu_key};
use ottospace_core::{Action, Origin, theme};

use crate::{Workspace, dialog, rect};

/// The menu's width, and an item's height, in points.
const WIDTH: f32 = 150.;
const ITEM: f32 = 31.;
/// A menu with an item that says why it does nothing: wider, and that line's height.
const WIDE: f32 = 240.;
const WHY: f32 = 16.;
/// The status bar's height: the menu stays above it.
const FOOT: f32 = 28.;

/// The control id of an item of the open menu: `row-menu:1`.
fn item_control(index: usize) -> String {
    format!("row-menu:{index}")
}

/// The control that opens a menu: its … (the transcript header's ⋯).
fn dots_control(row: &MenuOf) -> Option<String> {
    match row {
        MenuOf::Row(ListRow::Workbook(id)) => Some(format!("workbook-menu:{id}")),
        MenuOf::Row(ListRow::WorkbookTab(id, at)) => Some(format!("workbook-tab-menu:{id}:{at}")),
        MenuOf::Space(id) => Some(format!("space-menu:{id}")),
        MenuOf::Transcript => Some("listen-menu".into()),
        MenuOf::ReposAdd => Some("bar:add".into()),
        // A right click's: the row itself.
        MenuOf::Row(ListRow::Repo(path)) => Some(format!("repo:{path}")),
        MenuOf::Pane(id) => Some(format!("slot-dots:{id}")),
        _ => None,
    }
}

impl Workspace {
    /// The … of `row`: shown while `shown` (the pointer is on its row, list mode's selection is,
    /// or its menu is open), there for a label always.
    pub(crate) fn row_dots(&self, row: ListRow, shown: bool, cx: &Context<Self>) -> Option<Stateful<gpui::Div>> {
        let row = MenuOf::Row(row);
        let control = dots_control(&row)?;
        let open = self.model.row_menu().is_some_and(|(menu, _)| menu.row == row);
        let dots = div()
            .id(ElementId::Name(control.clone().into()))
            .absolute()
            .right(px(6.))
            .bottom(px(4.))
            .px_1()
            .rounded_sm()
            .text_color(rgb(theme::TEXT))
            .opacity(if shown { 1. } else { 0. })
            .when(open, |d| d.bg(rgb(theme::BORDER_STRONG)))
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
            .child("…")
            .child(self.controls.mark(control))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation(); // not a click on the row
                this.open_row_menu(row.clone(), false, window, cx);
            }));
        Some(dots)
    }

    /// Where a row's … was last painted.
    fn dots_at(&self, row: &MenuOf) -> Option<Rect> {
        let [x, y, w, h] = rect(self.controls.bounds(&dots_control(row)?)?);
        Some(Rect { x, y, w, h })
    }

    /// Opens a row's menu (on the row whose menu is open: closes it). `keys`: by a key or a
    /// label, so the keys are on its first item.
    pub(crate) fn open_row_menu(&mut self, row: MenuOf, keys: bool, window: &Window, cx: &mut Context<Self>) {
        let dots = self.dots_at(&row);
        self.row_menu_act(Action::RowMenu { row, dots, keys }, window, cx);
    }

    /// Sends a menu action as whoever is acting; refused, the status bar says why.
    pub(crate) fn row_menu_act(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) {
        if let Some(why) = self.model.refusal(&action, self.input).or(self.model.waits(&action)) {
            self.error = Some(why.into());
            return cx.notify();
        }
        self.dispatch(action, self.input, window, cx);
    }

    /// Tells the core where the open menu's … is painted now: moved or gone, the menu closes
    /// rather than hang under another row. Every tick, and before a key or a press is the menu's.
    pub(crate) fn sync_row_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        // A row gone from under the pointer (deleted, folded away) is hovered no more.
        let listed = self.model.list_rows();
        self.row_hovered = self.row_hovered.take().filter(|row| listed.contains(row));
        let Some((menu, _)) = self.model.row_menu() else { return };
        let dots = self.dots_at(&menu.row);
        if dots != menu.dots {
            self.dispatch(Action::RowMenuDrawn { dots }, Origin::System, window, cx);
        }
    }

    /// Takes an item (default: the one the keys are on). What it stands for may have to wait
    /// (a name cannot be typed in visual mode): the menu closes and the status bar says why.
    pub(crate) fn row_menu_choose(&mut self, index: Option<usize>, window: &Window, cx: &mut Context<Self>) {
        let stood = self.model.row_menu_action(index);
        let why = stood.and_then(|stood| self.model.refusal(&stood, self.input).or(self.model.waits(&stood)));
        match why.filter(|_| self.input == Origin::User) {
            Some(why) => {
                self.dispatch(Action::RowMenuClose, self.input, window, cx);
                self.error = Some(why.into());
                cx.notify();
            }
            None => {
                // An undo it offers counts its seconds from now; a pause pauses now.
                self.tick_now();
                // What was heard before the user's pause is kept.
                let stood = self.model.row_menu_action(index);
                if matches!(stood, Some(Action::ListenPause { .. })) && self.input == Origin::User {
                    self.listen_flush(window, cx);
                }
                self.row_menu_act(Action::RowMenuChoose { index }, window, cx);
                // A pane's Create or Edit: the dialog, with a look at the app's windows now (the user's alone).
                if let Some(Action::SlotDialogOpen { pane, .. }) = stood.filter(|_| self.input == Origin::User) {
                    self.slot_dialog_open(pane, window, cx);
                }
            }
        }
    }

    /// A key of the user's while a menu is open: it is the menu's, and no other's. `false`: no
    /// menu is open, or the key is a client's, which goes where it went before.
    pub(crate) fn row_menu_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.input != Origin::User || self.model.row_menu().is_none() {
            return false;
        }
        self.sync_row_menu(window, cx);
        match self.model.row_menu().map(|_| menu_key(key)) {
            Some(Action::RowMenuChoose { index }) => self.row_menu_choose(index, window, cx),
            Some(action) => self.row_menu_act(action, window, cx),
            // Its … had moved: the key that found it so does nothing.
            None => {}
        }
        // Held past the menu's end, the key is not for what was under it.
        self.row_menu_spent = self.model.row_menu().is_none();
        true
    }

    /// A press of the user's while a menu is open: on an item or the menu's own …, theirs; anywhere
    /// else it closes the menu and is no press on what is under it (`true`). A client's is not
    /// the menu's: it goes where it went before.
    pub(crate) fn row_menu_press(&mut self, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.input != Origin::User || self.model.row_menu().is_none() {
            return false;
        }
        self.sync_row_menu(window, cx);
        let Some((menu, items)) = self.model.row_menu() else { return true };
        // A left press on the row a right click opened the menu of is not the menu's: it closes it.
        let row = dots_control(&menu.row)
            .filter(|_| !matches!(menu.row, MenuOf::Row(ListRow::Repo(_))) || event.button == MouseButton::Right);
        let own = (0..items.len()).map(item_control).chain(row);
        let inside = |control: String| self.controls.bounds(&control).is_some_and(|b| b.contains(&event.position));
        if own.into_iter().any(inside) {
            return false;
        }
        self.row_menu_act(Action::RowMenuClose, window, cx);
        true
    }

    /// The open menu: its items under the … that opened it, or over it where there is no room
    /// under.
    pub(crate) fn render_row_menu(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (menu, items) = self.model.row_menu()?;
        let dots = menu.dots?;
        // A right click's: at the pointer, on the row it opened on.
        let dots = match (&menu.row, self.row_menu_at) {
            (MenuOf::Row(ListRow::Repo(_)), Some(at)) => Rect { x: f32::from(at.x), y: f32::from(at.y), w: 0., h: 0. },
            _ => dots,
        };
        // An item that does nothing now is dimmed, with why under it.
        let whys: Vec<Option<&str>> = (0..items.len()).map(|index| self.model.row_menu_why(index)).collect();
        let width = if whys.iter().any(Option::is_some) { WIDE } else { WIDTH };
        let view = window.viewport_size();
        let left = dots.x.min(f32::from(view.width) - width - 4.).max(4.);
        let height = items.len() as f32 * ITEM + whys.iter().flatten().count() as f32 * WHY + 10.;
        let under = dots.y + dots.h + 2.;
        let top = if under + height > f32::from(view.height) - FOOT { (dots.y - height - 2.).max(4.) } else { under };
        let rows = items.iter().zip(whys).enumerate().map(|(index, (item, why))| {
            div()
                .id(("row-menu", index))
                .relative()
                .min_h(px(ITEM))
                .px_2()
                .flex()
                .flex_col()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .when(menu.selected == Some(index), |d| d.bg(rgb(theme::SELECTED)))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .when(why.is_some(), |d| d.pb_1().text_color(rgb(theme::MUTED)))
                .child(self.model.menu_label(*item))
                .children(why.map(|why| div().text_xs().child(why)))
                .child(self.controls.mark(item_control(index)))
                .on_click(cx.listener(move |this, _, window, cx| this.row_menu_choose(Some(index), window, cx)))
        });
        // What is under the list gets no hover and no click through it.
        let list = dialog::floating(width).absolute().left(px(left)).top(px(top)).p_1().text_sm().occlude();
        Some(list.children(rows))
    }

    /// The open menu as the state shows it: its row, the item the keys are on (null: none), and
    /// each item with where it was drawn.
    pub(crate) fn row_menu_json(&self) -> serde_json::Value {
        let Some((menu, items)) = self.model.row_menu() else { return serde_json::Value::Null };
        let (workbook, tab) = match menu.row {
            MenuOf::Row(ListRow::Workbook(id)) | MenuOf::Space(id) => (Some(id), None),
            MenuOf::Row(ListRow::WorkbookTab(id, at)) => (Some(id), Some(at)),
            _ => (None, None),
        };
        let (transcript, space) = (menu.row == MenuOf::Transcript, matches!(menu.row, MenuOf::Space(_)));
        let worktree = match &menu.row {
            MenuOf::Row(ListRow::Repo(path)) => Some(path),
            _ => None,
        };
        let pane = match menu.row {
            MenuOf::Pane(id) => Some(id),
            _ => None,
        };
        let item = |(index, item): (usize, &ottospace_core::row_menu::Item)| {
            let drawn = self.controls.bounds(&item_control(index)).map(rect);
            let why = self.model.row_menu_why(index);
            serde_json::json!({"item": item, "label": item.label(), "why": why, "rect": drawn})
        };
        let items: Vec<serde_json::Value> = items.iter().enumerate().map(item).collect();
        serde_json::json!({
            "workbook": workbook, "tab": tab, "space": space, "transcript": transcript, "worktree": worktree, "pane": pane,
            "selected": menu.selected, "items": items
        })
    }

    /// A sneak label on an item of the menu: `row-menu:1`.
    pub(crate) fn click_row_menu(&mut self, name: &str, window: &Window, cx: &mut Context<Self>) {
        if let Ok(index) = name.parse() {
            self.row_menu_choose(Some(index), window, cx);
        }
    }
}
