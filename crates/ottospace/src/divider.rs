//! The divider between two panes, under the pointer: a drag moves it (`Action::DividerDrag`), a
//! double-click evens it out (`DividerEven`), Esc during the drag puts it back. What moves, and
//! how far it may, is the core's (`Node::drag`); this file turns the pointer into a share.

use gpui::{
    App, Context, DragMoveEvent, EmptyView, MouseButton, MouseDownEvent, Pixels, Point, Window, div, prelude::*,
};
use gpui::{px, relative};
use ottospace_core::{Action, Axis, PaneId, layout::Node};

use crate::{Workspace, ui};

/// What a divider's drag carries: the divider, as `Node::divider` names it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct DraggedDivider {
    pane: PaneId,
    axis: Axis,
}

/// A press on a divider: its share then and the pointer along its axis, so a drag moves it by
/// what the pointer moved, with no jump to the pointer. `cancelled`: Esc put it back.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hold {
    divider: DraggedDivider,
    start: f32,
    from: f32,
    cancelled: bool,
}

fn along(axis: Axis, at: Point<Pixels>) -> f32 {
    f32::from(if axis == Axis::Right { at.x } else { at.y })
}

impl Workspace {
    /// A split's grab, over the line at share `ratio` between its sides, and what the split's
    /// element listens for: drags of its own divider, measured against its own bounds.
    pub(crate) fn divider_grab(&self, split: gpui::Div, node: &Node, cx: &mut Context<Self>) -> gpui::Div {
        let (Some(key), Node::Split { ratio, .. }) = (node.divider(), node) else { return split };
        let (pane, axis) = key;
        let divider = DraggedDivider { pane, axis };
        let ratio = *ratio;
        let held = self.divider_held(cx).is_some_and(|hold| hold.divider == divider);
        let state = if held { ui::State::Pressed } else { ui::State::Rest };
        let grab = ui::divider(gpui::ElementId::Name(format!("divider:{pane}:{axis:?}").into()), axis, state)
            // Over the panes under it: a press here is the divider's, not theirs.
            .block_mouse_except_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    if e.click_count == 2 {
                        this.divider = None;
                        this.dispatch(Action::DividerEven { pane, axis }, this.input, window, cx);
                        return;
                    }
                    // Drawn this frame: the share as it is now.
                    let from = along(axis, e.position);
                    this.divider = Some(Hold { divider, start: ratio, from, cancelled: false });
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.divider_released(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.divider_released(cx)))
            .on_drag(divider, |_, _, _, cx| cx.new(|_| EmptyView));
        // A line with no width at the share, the grab centred on it.
        let half = px(-ui::DIVIDER_GRAB / 2.);
        let place = div().absolute();
        let place = match axis {
            Axis::Right => {
                place.top_0().bottom_0().left(relative(ratio)).child(div().absolute().h_full().left(half).child(grab))
            }
            Axis::Down => {
                place.left_0().right_0().top(relative(ratio)).child(div().absolute().w_full().top(half).child(grab))
            }
        };
        split.relative().child(place).on_drag_move(cx.listener(
            move |this, e: &DragMoveEvent<DraggedDivider>, window, cx| {
                if *e.drag(cx) == divider {
                    this.divider_moved(divider, e, window, cx);
                }
            },
        ))
    }

    /// The pointer moved during a drag of `divider`, over its split's `e.bounds`.
    fn divider_moved(
        &mut self,
        divider: DraggedDivider,
        e: &DragMoveEvent<DraggedDivider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hold) = self.divider.filter(|hold| hold.divider == divider && !hold.cancelled) else { return };
        let extent = f32::from(if divider.axis == Axis::Right { e.bounds.size.width } else { e.bounds.size.height });
        if extent < 1. {
            return;
        }
        let ratio = hold.start + (along(divider.axis, e.event.position) - hold.from) / extent;
        let (pane, axis) = (divider.pane, divider.axis);
        self.dispatch(Action::DividerDrag { pane, axis, ratio }, self.input, window, cx);
    }

    /// The button came up: the drag is over, and the next frame sizes hosted windows to their panes.
    fn divider_released(&mut self, cx: &mut Context<Self>) {
        if self.divider.take().is_some() {
            cx.notify();
        }
    }

    /// The divider being dragged, if any: pressed and moved, the button still down.
    pub(crate) fn divider_held(&self, cx: &App) -> Option<Hold> {
        self.divider.filter(|_| cx.has_active_drag())
    }

    /// Esc during a drag: the divider goes back to where the drag began, and later moves of this
    /// drag do nothing. True if it was taken.
    pub(crate) fn divider_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(hold) = self.divider_held(cx).filter(|hold| !hold.cancelled) else { return false };
        let (pane, axis) = (hold.divider.pane, hold.divider.axis);
        self.divider = Some(Hold { cancelled: true, ..hold });
        self.dispatch(Action::DividerDrag { pane, axis, ratio: hold.start }, self.input, window, cx);
        true
    }
}
