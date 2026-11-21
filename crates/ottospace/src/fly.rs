//! The centred new-task box (`overlay_panels`) in the shell: the dim over the panes, the box, and
//! the ghost that plays when it leaves (`ottospace_core::fly`). ↵ grows it into the drawer, which is
//! already the task's and takes keys under it; any other close fades it. The ghost takes no click.

use std::time::Instant;

use gpui::{AnyElement, Bounds, Context, Pixels, Window, div, hsla, prelude::*, px, rgb};
use ottospace_core::{fly, theme};

use crate::Workspace;

/// The box's width at most, and its margin from the panes' edges.
const WIDTH: f32 = 680.;
const MARGIN: f32 = 24.;
/// How dark the dim over the panes is.
const DIM: f32 = 0.45;

/// The box leaving: since when, where it was drawn, its line, and whether it grows.
pub(crate) struct Flying {
    since: Instant,
    from: Bounds<Pixels>,
    line: String,
    grow: bool,
}

/// The ghost playing, if one is, and the frames it asked for (`counts.fly_frames`, test runs).
#[derive(Default)]
pub(crate) struct Fly {
    now: Option<Flying>,
    pub(crate) frames: u64,
}

impl Workspace {
    /// Before a step: the centred box's bounds and line, if it is open.
    pub(crate) fn fly_before(&self) -> Option<(Bounds<Pixels>, String)> {
        let at = self.model.compose_centred().then(|| self.controls.bounds("new-task-centred")).flatten()?;
        Some((at, self.model.compose_line()?.clone()))
    }

    /// After it: the box closed or was put aside. ↵ that filed its task grows; Esc, ⇧↵, ⌘↵ or a
    /// click outside fade. Handed to the drawer still open (⌃G's editor, the flag off): none.
    pub(crate) fn fly_after(&mut self, before: Option<(Bounds<Pixels>, String)>, entered: bool) {
        let Some((from, line)) = before.filter(|_| self.model.compose.is_none()) else { return };
        let grow = entered && self.model.compose.is_none() && !self.model.drawer_collapsed();
        self.fly.now = Some(Flying { since: Instant::now(), from, line, grow });
    }

    /// The dim, the centred box over it, and the ghost of one leaving. None at rest with no box.
    pub(crate) fn render_fly(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        if self.model.compose_centred() {
            self.fly.now = None;
            let width = WIDTH.min(w - crate::RAIL - crate::drawer_rail::WIDTH - 2. * MARGIN).max(200.);
            let dim = div().absolute().size_full().bg(hsla(0., 0., 0., DIM));
            // Centred over the panes, a little above the middle.
            let place = div().absolute().top(px(crate::TOP_BAR)).bottom_0().flex().justify_center().items_start();
            let place = place.left(px(crate::RAIL)).right(px(crate::drawer_rail::WIDTH));
            let place = place.pt(px((h * 0.2).max(MARGIN))).child(self.render_new_task_centred(width, cx));
            return Some(div().absolute().size_full().child(dim).child(place).into_any_element());
        }
        // The box's ✎ marks up: the box is tucked away to a line at the top that says the keys.
        // It takes no click: the panes under it are what is marked.
        if self.model.pen_marking() {
            let hint = "Marking up · drag: circle · ⇧ box · ⌥ pen · click: note · ↵ back to the box · Esc cancel";
            let pill = crate::ui::lifted(div()).px_3().py_1().text_xs().text_color(rgb(theme::TEXT)).child(hint);
            let place = div().absolute().top(px(crate::TOP_BAR + 8.)).left_0().right_0().flex().justify_center();
            return Some(place.child(pill).into_any_element());
        }
        let flying = self.fly.now.as_ref()?;
        let t = flying.since.elapsed().as_secs_f32() * 1000.;
        let Some(frame) = fly::frame(t, flying.grow, crate::dial::calm()) else {
            self.fly.now = None;
            return None;
        };
        self.fly.frames += 1;
        window.request_animation_frame();
        // Where the drawer lies at rest: against its rail, under the top row.
        let inner = (self.drawer.width - crate::drawer_rail::WIDTH).max(0.);
        let to = Bounds::new(
            gpui::point(px(w - self.drawer.width), px(crate::TOP_BAR)),
            gpui::size(px(inner), px(h - crate::TOP_BAR - crate::STATUS_BAR)),
        );
        let from = flying.from;
        let mix = |a: Pixels, b: Pixels| a + (b - a) * frame.at;
        let ghost = div()
            .absolute()
            .left(mix(from.origin.x, to.origin.x))
            .top(mix(from.origin.y, to.origin.y))
            .w(mix(from.size.width, to.size.width))
            .h(mix(from.size.height, to.size.height))
            .rounded(px(frame.radius))
            .overflow_hidden()
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::COMPOSE))
            .opacity(frame.alpha)
            .p_3()
            .text_sm()
            .text_color(rgb(theme::TEXT))
            .child(flying.line.clone());
        let dim = div().absolute().size_full().bg(hsla(0., 0., 0., DIM * frame.dim));
        Some(div().absolute().size_full().child(dim).child(ghost).into_any_element())
    }
}
