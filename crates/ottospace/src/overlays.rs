//! The side overlays in the shell (`overlay_panels`, `ottospace_core::model::overlay`): the open
//! sidebar and the open drawer drawn over the panes at their rails, sliding in, with a shadow.
//! The panes are laid out as with both collapsed, so nothing under them moves. A live app's
//! window is above ottospace and may cover one: the part that shows still takes clicks.

use std::time::Instant;

use gpui::{AnyElement, Context, Div, MouseButton, MouseDownEvent, Pixels, Point, Window, div, prelude::*, px};
use ottospace_core::Action;
use ottospace_core::model::overlay::slide;

use crate::{Mark, Workspace, mark};

/// One side's slide: when it last came into view.
#[derive(Default)]
struct Slide {
    since: Option<Instant>,
}

impl Slide {
    /// This frame's share in (1 at rest), and whether it still moves.
    fn step(&mut self, shown: bool, calm: bool) -> (f32, bool) {
        if !shown {
            self.since = None;
            return (1., false);
        }
        let since = *self.since.get_or_insert_with(Instant::now);
        let t = slide(since.elapsed().as_secs_f32() * 1000., calm);
        (t, t < 1.)
    }
}

/// Both sides' slides, and the frames they asked for (the state's `counts.overlay_frames`).
#[derive(Default)]
pub(crate) struct Slides {
    left: Slide,
    right: Slide,
    pub(crate) frames: u64,
}

impl Slides {
    /// Once a frame: how far in each side is (left, right). Asks for the next frame while one moves.
    pub(crate) fn step(&mut self, left: bool, right: bool, window: &mut Window) -> (f32, f32) {
        let calm = crate::dial::calm();
        let ((l, left_moves), (r, right_moves)) = (self.left.step(left, calm), self.right.step(right, calm));
        if left_moves || right_moves {
            self.frames += 1;
            window.request_animation_frame();
        }
        (l, r)
    }
}

impl Workspace {
    /// `body` over the panes, `width` wide, against the left rail (`left`) or the drawer's rail,
    /// `top` points down and `inset` points in from the window's edge. Slid `t` of the way in.
    /// Its body takes the pointer where it is drawn, sliding too: nothing under it is hovered or clicked.
    pub(crate) fn overlay(&self, body: AnyElement, left: bool, width: f32, (top, inset, t): (f32, f32, f32)) -> Div {
        let shift = (1. - t) * width;
        let inner = div().id(if left { "overlay-left" } else { "overlay-right" }).absolute().top_0().bottom_0();
        let inner = inner.w(px(width)).flex().occlude().child(body).child(mark(&self.marks, Mark::Overlay(left)));
        let inner = if left { inner.left(px(-shift)) } else { inner.right(px(-shift)) };
        let place = div().absolute().top(px(top)).bottom_0();
        let place = if left { place.left(px(inset)) } else { place.right(px(inset)) };
        // As the side zones' shadow onto the workspace, from the overlay's inner edge.
        place.w(px(width)).overflow_hidden().shadow(crate::side_shadow(left)).child(inner)
    }

    /// A press anywhere in the window, before what is under it takes it: outside both overlays
    /// and both rails, the user's click dismisses them (the core says what closes). The click goes
    /// on to what is under it. Over an overlay nothing here is hovered: it never arrives.
    pub(crate) fn overlay_press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.model.overlays() && (self.model.sidebar.is_some() || !self.model.drawer_collapsed());
        // ⌥-click is the new-task box's reference to what is under it (`compose_point`), not outside it.
        let alt = event.modifiers.alt;
        if !open || alt || event.button != MouseButton::Left || self.on_rails(event.position, window) {
            return;
        }
        let marks = self.marks.borrow();
        let over = [true, false]
            .into_iter()
            .any(|left| marks.get(&Mark::Overlay(left)).is_some_and(|b| b.contains(&event.position)));
        drop(marks);
        if !over {
            self.dispatch(Action::OverlayDismiss, self.input, window, cx);
        }
    }

    /// Over the left rail or the drawer's: their clicks open, switch and close the overlays themselves.
    /// They reach as low as the overlays: the status bar under them is outside.
    fn on_rails(&self, at: Point<Pixels>, window: &Window) -> bool {
        let width = f32::from(window.viewport_size().width);
        let marks = self.marks.borrow();
        let foot =
            [true, false].into_iter().filter_map(|left| marks.get(&Mark::Overlay(left))).map(|b| b.bottom()).max();
        let x = f32::from(at.x);
        (x < crate::RAIL || x > width - crate::drawer_rail::WIDTH) && foot.is_some_and(|foot| at.y <= foot)
    }
}
