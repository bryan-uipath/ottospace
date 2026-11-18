//! The arrows that slide the sidebar and the drawer into their rails and out again: one button
//! for both sides, with its key's cap. In a header the cap shows beside it while the pointer is
//! over the header (shown always, it cut the transcript's clock short); on a rail under it, always.

use gpui::{AnyView, Context, Div, ElementId, MouseButton, Stateful, Window, div, prelude::*, px, rgb, svg};
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace, keycaps};

/// The arrows: `|←` slides toward the left edge, `→|` toward the right.
pub(crate) const LEFT: &str = "icons/slide-left.svg";
pub(crate) const RIGHT: &str = "icons/slide-right.svg";
/// The sidebar's key and the drawer's, as their caps say.
pub(crate) const SIDEBAR_KEY: &str = "⌘⇧L";
pub(crate) const DRAWER_KEY: &str = "⌘⇧R";
/// The headers whose arrow shows its cap on hover (`slide_head`): the drawer's and the sidebar's.
pub(crate) const DRAWER_HEAD: &str = "drawer-head";
pub(crate) const BAR_HEAD: &str = "bar-head";

impl Workspace {
    /// A slide button: its arrow and its key's cap. `head`: the hover group of the header it
    /// ends, the cap beside it while that is hovered; none, on a rail: the cap under it, always.
    /// `control` is its control id; `tip` says what it does.
    pub(crate) fn slide_button(
        &self,
        control: &'static str,
        (icon, key): (&'static str, &'static str),
        tip: &'static str,
        head: Option<&'static str>,
    ) -> Stateful<Div> {
        let tip: gpui::SharedString = format!("{tip} · {key}").into();
        let arrow = svg().path(icon).size(px(14.)).text_color(rgb(theme::MUTED));
        // Drawn, not hidden by a hover style: a display that changes after layout breaks the frame.
        let cap = (head.is_none() || self.slide_hover == head).then(|| keycaps::cap(key, false, true));
        div()
            .id(ElementId::Name(control.into()))
            .relative()
            .flex_none()
            .p_1()
            .flex()
            .when(head.is_none(), |d| d.flex_col())
            .items_center()
            .gap_1()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(arrow)
            .children(cap)
            .child(self.controls.mark(control))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
            // The press must not reach the drawer, which takes the keyboard on a press.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    /// A header that ends in a slide button of `head`: the pointer over it shows the button's cap.
    pub(crate) fn slide_head(&self, header: Div, head: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        header.id(head).on_hover(cx.listener(move |this, over: &bool, _, cx| {
            let now = if *over { Some(head) } else { this.slide_hover.filter(|was| *was != head) };
            if now != this.slide_hover {
                this.slide_hover = now;
                cx.notify();
            }
        }))
    }

    /// The drawer's →, at the end of a header of `DRAWER_HEAD` (`slide_head`): collapsed into its rail, as ⌘⇧R does.
    pub(crate) fn drawer_collapse(&self, control: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        self.slide_button(control, (RIGHT, DRAWER_KEY), "Collapse into the rail", Some(DRAWER_HEAD))
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
