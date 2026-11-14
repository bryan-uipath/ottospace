//! What ottospace's floating panels share: the new-task box, the picker, the palette, the quit
//! question, the key reference.

use gpui::{
    AnyElement, BoxShadow, Context, Div, ElementId, KeyDownEvent, MouseButton, SharedString, Stateful, Styled, Window,
    black, div, point, prelude::*, px, rgb,
};
use ottospace_core::keys::{self, FilterKey};
use ottospace_core::{Action, theme};

use crate::Workspace;

/// Corner radius of a panel, and of what sits inside one.
pub const RADIUS: f32 = 13.;
pub const INNER_RADIUS: f32 = 8.;

/// Two layers: a tight dark one that lifts the edge, a wide soft one under the panel.
pub fn shadow() -> Vec<BoxShadow> {
    let layer = |opacity, y, blur| BoxShadow {
        color: black().opacity(opacity),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    };
    vec![layer(0.55, 2., 6.), layer(0.45, 18., 48.)]
}

/// A floating panel: raised, rounded, bordered in `border`, with the shared shadow.
pub fn panel(border: u32) -> Div {
    div().rounded(px(RADIUS)).bg(rgb(theme::RAISED)).border_1().border_color(rgb(border)).shadow(shadow())
}

/// A panel over a backdrop, `width` wide. A click inside it must not reach the backdrop, which closes it.
pub fn floating(width: f32) -> Div {
    panel(theme::BORDER_STRONG)
        .w(px(width))
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The window, dimmed by `dim`, under a panel: it takes every click, and one on it sends `close`.
pub fn backdrop(id: &'static str, dim: f32, close: Action, cx: &mut Context<Workspace>) -> Stateful<Div> {
    div().id(id).absolute().top_0().left_0().size_full().bg(black().opacity(dim)).occlude().on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, window, cx| drop(this.dispatch(close.clone(), this.input, window, cx))),
    )
}

/// The keys line at a panel's foot, e.g. "↵ quit · esc cancel".
pub fn hint(text: &'static str) -> Div {
    div()
        .px_3()
        .py_1p5()
        .border_t_1()
        .border_color(rgb(theme::BORDER))
        .text_xs()
        .text_color(rgb(theme::MUTED))
        .child(text)
}

/// A row of a filtered list: highlighted while `selected` or under the pointer.
pub fn filter_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .text_sm()
        .cursor_pointer()
        .when(selected, |d| d.bg(rgb(theme::SELECTED)))
        .hover(|s| s.bg(rgb(theme::SELECTED)))
}

impl Workspace {
    /// A filtered list in `panel`: `header` (title and filter), `empty` when there are no rows,
    /// the rows scrolling under `name`, then `keys`.
    pub fn list_panel(
        &self,
        panel: Div,
        header: Div,
        name: &'static str,
        rows: Vec<AnyElement>,
        (empty, keys): (&'static str, &'static str),
    ) -> Div {
        let none = rows.is_empty();
        let list =
            div().id(SharedString::from(format!("{name}-rows"))).flex_1().min_h_0().overflow_y_scroll().children(rows);
        panel
            .overflow_hidden()
            .child(header.px_3().flex().items_center().gap_2().border_b_1().border_color(rgb(theme::BORDER)))
            .when(none, |d| d.child(div().p_3().text_xs().text_color(rgb(theme::MUTED)).child(empty)))
            // Long lists scroll inside the panel.
            .child(self.bars.area(name, list))
            .child(hint(keys))
    }

    /// A key while a filtered list is open: `action` is what a list key does (`keys::filter_key`),
    /// else the filter's field takes it. ⌘ shortcuts the filter does not use reach their bindings.
    pub fn filtered_list_key(
        &mut self,
        event: &KeyDownEvent,
        action: impl FnOnce(FilterKey) -> Action,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = keys::filter_key(&event.keystroke.key, crate::mods(event.keystroke.modifiers)) else {
            // The filter is a text field: it edits, and what types text goes to the input method.
            if self.field_key(event, window, cx) {
                cx.stop_propagation();
            }
            return;
        };
        self.dispatch(action(key), self.input, window, cx);
        cx.stop_propagation();
    }
}
