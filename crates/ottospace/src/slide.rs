//! The arrows that slide the sidebar and the drawer into their rails and out again: one button
//! for both sides. Its key is in its tooltip only (the owner: shortcuts of small buttons in tooltips).

use gpui::{AnyView, Context, Div, ElementId, MouseButton, Stateful, Window, div, prelude::*, px, rgb, svg};
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace};

/// The arrows: `|←` slides toward the left edge, `→|` toward the right.
pub(crate) const LEFT: &str = "icons/slide-left.svg";
pub(crate) const RIGHT: &str = "icons/slide-right.svg";
/// The sidebar's key and the drawer's, as their tooltips say.
pub(crate) const SIDEBAR_KEY: &str = "⌘⇧L";
pub(crate) const DRAWER_KEY: &str = "⌘⇧R";
/// The headers that end in an arrow (`slide_head`): the drawer's and the sidebar's.
pub(crate) const DRAWER_HEAD: &str = "drawer-head";
pub(crate) const BAR_HEAD: &str = "bar-head";

impl Workspace {
    /// A slide button: its arrow, its key in the tooltip. `control` is its control id; `tip` says
    /// what it does.
    pub(crate) fn slide_button(
        &self,
        control: &'static str,
        (icon, key): (&'static str, &'static str),
        tip: &'static str,
    ) -> Stateful<Div> {
        let tip: gpui::SharedString = format!("{tip} · {key}").into();
        let arrow = svg().path(icon).size(px(14.)).text_color(rgb(theme::MUTED));
        div()
            .id(ElementId::Name(control.into()))
            .relative()
            .flex_none()
            .p_1()
            .flex()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(arrow)
            .child(self.controls.mark(control))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
            // The press must not reach the drawer, which takes the keyboard on a press.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    /// A header that ends in a slide button of `head`.
    pub(crate) fn slide_head(&self, header: Div, head: &'static str) -> Stateful<Div> {
        header.id(head)
    }

    /// The drawer's →, at the end of a header of `DRAWER_HEAD` (`slide_head`): collapsed into its rail, as ⌘⇧R does.
    pub(crate) fn drawer_collapse(&self, control: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        self.slide_button(control, (RIGHT, DRAWER_KEY), "Collapse into the rail")
            .on_click(cx.listener(|this, _, window, cx| this.drawer_toggle(window, cx)))
    }

    /// ⌘⇧R, the drawer's → or the rail's ←, as whoever acts: a client's is refused.
    pub(crate) fn drawer_toggle(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::TalkToggle, self.input, window, cx);
        cx.notify();
    }

    /// ⌘⇧L, the sidebar's ← or the left rail's →: the bar closes, or opens on the section open last.
    pub(crate) fn sidebar_toggle(&mut self, window: &Window, cx: &mut Context<Self>) {
        let section = self.model.sidebar_toggled();
        self.dispatch(Action::ToggleSection { section }, self.input, window, cx);
        cx.notify();
    }
}
