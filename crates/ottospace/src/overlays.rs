//! The side overlays in the shell (`overlay_panels`, `ottospace_core::model::overlay`): the open
//! sidebar and the open drawer drawn over the panes at their rails, sliding in, with a shadow.
//! The panes are laid out as with both collapsed, so nothing under them moves. A live app's
//! window is above ottospace and may cover one: the part that shows still takes clicks.
//! A closed one slides back into its rail, drawn only: it takes no click, and the model has moved on.

use std::cell::Cell;
use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, Pixels, Point, Window, div, prelude::*, px,
};
use ottospace_core::Action;
use ottospace_core::model::Section;
use ottospace_core::model::overlay::{SLIDE_MS, slide, travel};

use crate::{Mark, Workspace, mark};

/// One side's slide: where it was (milliseconds in), since when, and whether it moves in.
#[derive(Default)]
struct Slide {
    from: f32,
    since: Option<Instant>,
    opening: bool,
}

impl Slide {
    /// This frame's share in (0 out of sight, 1 at rest), and whether it still moves. A close plays
    /// the opening backwards from where it is; `at_once` (Reduce Motion, nothing to draw): no slide.
    fn step(&mut self, shown: bool, at_once: bool) -> (f32, bool) {
        let now = Instant::now();
        let at =
            |s: &Self| s.since.map_or(s.from, |since| travel(s.from, (now - since).as_secs_f32() * 1000., s.opening));
        if shown != self.opening {
            (self.from, self.since, self.opening) = (at(self), Some(now), shown);
        }
        if at_once {
            (self.from, self.since) = (if shown { SLIDE_MS } else { 0. }, None);
        }
        let at = at(self);
        (slide(at), if shown { at < SLIDE_MS } else { at > 0. })
    }
}

/// What the open drawer showed that the model keeps once it folds, so it can slide back drawn.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kept {
    Talk,
    Transcript,
}

/// Both sides' slides, and the frames they asked for (the state's `counts.overlay_frames`).
#[derive(Default)]
pub(crate) struct Slides {
    left: Slide,
    right: Slide,
    pub(crate) frames: u64,
    /// The sidebar's section last drawn: it slides back showing it.
    section: Option<Section>,
    /// What the drawer drew this frame (`Kept`), noted by what drew it.
    pub(crate) kept: Cell<Option<Kept>>,
    /// While the folded drawer slides back: what to draw in it although the model closed it.
    pub(crate) ghost: Option<Kept>,
}

impl Slides {
    /// Whether that side is open in the model, not only drawn sliding back.
    fn open(&self, left: bool) -> bool {
        if left { self.left.opening } else { self.right.opening }
    }
}

impl Workspace {
    /// Once a frame, after the drawer's body: the sidebar and the drawer over the panes, each slid as
    /// far in as it is, and how far (left, right). Closed, one slides back drawn from what the model
    /// still holds; asks for the next frame while one moves.
    pub(crate) fn overlays_drawn(
        &mut self,
        drawer: Option<AnyElement>,
        last: Option<Kept>,
        inner: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>, f32, f32) {
        let calm = crate::dial::calm();
        let section = self.model.sidebar.filter(|_| self.model.overlays());
        let (left, right) = (section.is_some(), drawer.is_some());
        let ((l, left_moves), (r, right_moves)) = (
            self.slides.left.step(left, calm || (!left && self.slides.section.is_none())),
            self.slides.right.step(right, calm || (!right && last.is_none())),
        );
        if left_moves || right_moves {
            self.slides.frames += 1;
            window.request_animation_frame();
        }
        if left {
            self.slides.section = section;
        } else if !left_moves {
            self.slides.section = None;
        }
        let bar = section.or(self.slides.section.filter(|_| left_moves)).map(|section| {
            let bar = self.render_bar(section, cx).into_any_element();
            self.overlay(bar, true, crate::SECTION, (crate::TOP_BAR, crate::RAIL, l))
        });
        // The folded drawer: its conversation or transcript again, as the model still holds it.
        let drawer = drawer.or_else(|| {
            let kept = last.filter(|_| right_moves)?;
            self.slides.ghost = Some(kept);
            let body = self.drawer_body(inner, cx);
            self.slides.ghost = None;
            body
        });
        let drawer =
            drawer.map(|body| self.overlay(body, false, inner, (crate::TOP_BAR, crate::drawer_rail::WIDTH, r)));
        (bar, drawer, l, r)
    }

    /// `body` over the panes, `width` wide, against the left rail (`left`) or the drawer's rail,
    /// `top` points down and `inset` points in from the window's edge. Slid `t` of the way in.
    /// Open, its body takes the pointer where it is drawn, sliding too: nothing under it is hovered or
    /// clicked. Closed and sliding back, the reverse: drawn only, the pointer goes to what is under it.
    fn overlay(&self, body: AnyElement, left: bool, width: f32, (top, inset, t): (f32, f32, f32)) -> AnyElement {
        let shift = (1. - t) * width;
        let inner = div().id(if left { "overlay-left" } else { "overlay-right" }).absolute().top_0().bottom_0();
        let inner = inner.w(px(width)).flex().child(body).child(mark(&self.marks, Mark::Overlay(left)));
        let inner = if left { inner.left(px(-shift)) } else { inner.right(px(-shift)) };
        let open = self.slides.open(left);
        let inner = if open { inner.occlude() } else { inner };
        let place = div().absolute().top(px(top)).bottom_0();
        let place = if left { place.left(px(inset)) } else { place.right(px(inset)) };
        // As the side zones' shadow onto the workspace, from the overlay's inner edge.
        let place: Div = place.w(px(width)).overflow_hidden().shadow(crate::side_shadow(left)).child(inner);
        if open { place.into_any_element() } else { Inert(place.into_any_element()).into_any_element() }
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
        let over = [true, false].into_iter().any(|left| {
            self.slides.open(left) && marks.get(&Mark::Overlay(left)).is_some_and(|b| b.contains(&event.position))
        });
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

/// Its child painted, but its hitboxes put under an empty mask: none is ever under the pointer, so
/// the click, the wheel and the hover go to what lies beneath it (a closing overlay's pane).
struct Inert(AnyElement);

impl IntoElement for Inert {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Inert {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.0.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds: Bounds::default() }), |window| {
            self.0.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0.paint(window, cx);
    }
}
