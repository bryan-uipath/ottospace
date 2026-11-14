//! The words at the end of a list row (Open, Adopt, Snooze, Pin, Rename…), shown on hover or
//! while list mode selects the row.

use gpui::{AnyView, Context, Div, ElementId, Stateful, Window, div, prelude::*, px, rgb};
use ottospace_core::theme;

use crate::{Tooltip, Workspace};

/// Where the words sit: over the end of the row, on the row's own colour, so hidden they take no
/// room. Hidden unless `selected` or the pointer is on `group`. The caller places it vertically.
pub fn words(group: &'static str, selected: bool) -> Div {
    div()
        .absolute()
        .right(px(6.))
        .flex()
        .gap_0p5()
        .pl_1()
        .bg(rgb(theme::SELECTED))
        .when(!selected, |d| d.invisible().group_hover(group, |s| s.visible()))
}

impl Workspace {
    /// One word: `control` is its id and sneak mark (`task-snooze:3`), `tip` its tooltip. A click
    /// runs `click`, and is no click on the row.
    pub fn word(
        &self,
        control: String,
        (label, tip): (&'static str, &'static str),
        click: impl Fn(&mut Self, &Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(ElementId::Name(control.clone().into()))
            .relative()
            .child(self.controls.mark(control))
            .px_1()
            .rounded_sm()
            .text_xs()
            .text_color(rgb(theme::TEXT))
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
            .child(label)
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation(); // not a click on the row
                click(this, window, cx);
            }))
    }
}
