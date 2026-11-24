//! The arrows that slide the sidebar and the drawer into their rails and out again: one button
//! for both sides. Its key is in its tooltip only (the owner: shortcuts of small buttons in tooltips).

use gpui::{AnyView, Context, Div, ElementId, MouseButton, Stateful, Window, prelude::*};
use ottospace_core::Action;

use crate::{Tooltip, Workspace, ui};

/// The arrows: `|←` slides toward the left edge, `→|` toward the right.
pub(crate) const LEFT: &str = "icons/slide-left.svg";
pub(crate) const RIGHT: &str = "icons/slide-right.svg";
/// The sidebar's key and the drawer's, as their tooltips say.
pub(crate) const SIDEBAR_KEY: &str = "⌘⇧L";
pub(crate) const DRAWER_KEY: &str = "⌘⇧R";
/// The drawer pulled out over the panes, `|←`, and back, `|→`; its key.
const PULL: &str = "icons/slide-left.svg";
const PULL_BACK: &str = "icons/pull-back.svg";
const PULL_KEY: &str = "⌘⌥R";

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
        ui::button(ElementId::Name(control.into()), ui::Button::Icon(icon), ui::State::Rest)
            .child(self.controls.mark(control))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
            // The press must not reach the drawer, which takes the keyboard on a press.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    /// The drawer's →, at the end of its header: collapsed into its rail, as ⌘⇧R does.
    pub(crate) fn drawer_collapse(&self, control: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        self.slide_button(control, (RIGHT, DRAWER_KEY), "Collapse into the rail")
            .on_click(cx.listener(|this, _, window, cx| this.drawer_toggle(window, cx)))
    }

    /// The drawer's |←, beside its →|: pulled out over the panes, an edge of them left showing; or
    /// back. Only as an overlay (`overlay_panels`). None without.
    pub(crate) fn drawer_pull_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (icon, tip) = match self.model.talk_pulled() {
            true => (PULL_BACK, "Back to its width"),
            false => (PULL, "Pull out over the panes"),
        };
        let button = self.slide_button("talk:talk-wide", (icon, PULL_KEY), tip);
        let button = button.on_click(cx.listener(|this, _, window, cx| this.drawer_pull(window, cx)));
        self.model.overlays().then_some(button)
    }

    /// ⌘⌥R or the drawer's |←, as whoever acts: a client's is refused.
    pub(crate) fn drawer_pull(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::TalkWide, self.input, window, cx);
        cx.notify();
    }

    /// ⌘⇧R or the drawer's →, as whoever acts: a client's is refused.
    pub(crate) fn drawer_toggle(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::TalkToggle, self.input, window, cx);
        cx.notify();
    }

    /// ⌘⇧L: the bar closes, or opens on the section open last.
    pub(crate) fn sidebar_toggle(&mut self, window: &Window, cx: &mut Context<Self>) {
        let section = self.model.sidebar_toggled();
        self.dispatch(Action::ToggleSection { section }, self.input, window, cx);
        cx.notify();
    }
}
