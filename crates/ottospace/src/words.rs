//! The words at the end of a list row (Open, Adopt, Snooze, Pin, Rename…), shown on hover or
//! while list mode selects the row.

use gpui::{AnyView, Context, Div, ElementId, Stateful, Window, div, prelude::*, px, rgb, svg};
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
        self.row_button(control, tip, click, cx).px_1().text_xs().text_color(rgb(theme::TEXT)).child(label)
    }

    /// A word drawn as an icon (`icons/plus.svg`), as `word` otherwise.
    pub fn icon_word(
        &self,
        control: String,
        (icon, tip): (&'static str, &'static str),
        click: impl Fn(&mut Self, &Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let icon = svg().path(icon).size(px(12.)).text_color(rgb(theme::TEXT));
        self.row_button(control, tip, click, cx).size(px(18.)).flex().items_center().justify_center().child(icon)
    }

    /// What a word and an icon share: its id and sneak mark, the hover, the tooltip and the click.
    fn row_button(
        &self,
        control: String,
        tip: &'static str,
        click: impl Fn(&mut Self, &Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(ElementId::Name(control.clone().into()))
            .relative()
            .child(self.controls.mark(control))
            .rounded_sm()
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation(); // not a click on the row
                click(this, window, cx);
            }))
    }
}
