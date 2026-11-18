//! Every overflowing list's motif: GPUI scrolls overflow but draws no bar. An overlay thumb at
//! the right (or bottom) edge of a scroll area, in apollo-ui's future-dark style, and a soft fade
//! at each edge with more beyond it. Where the thumb is, what a drag or a click scrolls to and
//! which edges fade are the core's rules (`ottospace_core::scrollbar`).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, Corners, DispatchPhase, Div, HitboxBehavior, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    ScrollHandle, ScrollWheelEvent, Stateful, StatefulInteractiveElement, Styled, Window, canvas, div, fill,
    linear_color_stop, linear_gradient, point, px, rgb, size,
};
use ottospace_core::scrollbar::{self as rules, EDGE_FADE, Extent, Step};
use ottospace_core::theme;

/// How far a line step from the keyboard scrolls a list.
const LINE: f32 = 40.;

/// Which way an area scrolls: a list down, the tab row across.
#[derive(Clone, Copy, PartialEq)]
pub enum Axis {
    Down,
    Across,
}

impl Axis {
    /// How far `at` lies along the axis.
    fn along(self, at: Point<Pixels>) -> f32 {
        f32::from(if self == Axis::Down { at.y } else { at.x })
    }
}

/// One scroll area's bar, kept between frames.
#[derive(Default)]
struct Bar {
    /// Scroll position of an area that has no handle of its own.
    handle: ScrollHandle,
    /// Rendered this frame. A bar that was not is forgotten: a closed list opens at its top.
    seen: bool,
    /// What a scroll changes, as last painted, and when it last changed or the pointer left.
    moved_by: f32,
    active: Option<Instant>,
    engaged: bool,
    over_track: bool,
    /// The pointer left the window: wherever it was last seen, it is over nothing.
    left: bool,
    /// The thumb is held: how far along from its start.
    grab: Option<f32>,
    /// When the pending redraw is due.
    wake: Option<Instant>,
    /// The track, the thumb and the opacity as last painted. `None`: the content fits.
    drawn: Option<(Bounds<Pixels>, Bounds<Pixels>, f32)>,
    /// Its axis and which edges fade, `(start, end)`, as last laid out; also for a list that fits.
    faded: Option<(Axis, bool, bool)>,
    /// The user's wheel events over a row that only they show (`tracked_row`).
    wheels: Rc<Cell<f32>>,
}

/// Every scroll area's bar, by the area's id.
#[derive(Clone, Default)]
pub struct Bars(Rc<RefCell<BTreeMap<String, Bar>>>);

/// A list's extent along `axis` as last laid out.
pub(crate) fn extent(handle: &ScrollHandle, axis: Axis) -> Extent {
    let (bounds, offset, max) = (handle.bounds().size, handle.offset(), handle.max_offset());
    let (viewport, offset, max) = match axis {
        Axis::Down => (bounds.height, offset.y, max.height),
        Axis::Across => (bounds.width, offset.x, max.width),
    };
    let viewport = f32::from(viewport);
    Extent { viewport, content: viewport + f32::from(max), offset: -f32::from(offset) }
}

fn scroll_to(handle: &ScrollHandle, axis: Axis, offset: f32) {
    let now = handle.offset();
    handle.set_offset(match axis {
        Axis::Down => point(now.x, px(-offset)),
        Axis::Across => point(px(-offset), now.y),
    });
}

impl Bars {
    /// A frame starts: forgets the areas the last one did not render.
    pub fn begin(&self) {
        self.0.borrow_mut().retain(|_, bar| std::mem::take(&mut bar.seen));
        self.0.borrow_mut().values_mut().for_each(|bar| (bar.drawn, bar.faded) = (None, None));
    }

    /// `list` (an `overflow_y_scroll` element that fills its parent) with its bar over it, its
    /// edges fading into `bg`, the colour behind it.
    pub fn area(&self, id: &str, list: Stateful<Div>, bg: u32) -> Div {
        self.rounded(id, list, Fade::to(bg))
    }

    /// As `area`, its edges fading as `fade` says (a panel's rounded corners).
    pub fn rounded(&self, id: &str, list: Stateful<Div>, fade: Fade) -> Div {
        let handle = self.0.borrow_mut().entry(id.into()).or_default().handle.clone();
        self.down(id, list.track_scroll(&handle), &handle, fade, true)
    }

    /// As `area`, for a list that already scrolls under `handle`.
    pub fn tracked(&self, id: &str, list: impl IntoElement, handle: &ScrollHandle, bg: u32) -> Div {
        self.down(id, list, handle, Fade::to(bg), true)
    }

    /// As `tracked`, its track paging nothing: a press beside the thumb is the item's under it (the rail's).
    pub fn tracked_under(&self, id: &str, list: impl IntoElement, handle: &ScrollHandle, bg: u32) -> Div {
        self.down(id, list, handle, Fade::to(bg), false)
    }

    fn down(&self, id: &str, list: impl IntoElement, handle: &ScrollHandle, fade: Fade, pages: bool) -> Div {
        let bar = self.bar_of(id, Axis::Down, handle, fade, pages, |extent| extent.offset, |_| {});
        div().relative().flex_1().min_h_0().flex().flex_col().child(list).child(bar)
    }

    /// As `tracked`, for a row that scrolls across (`overflow_x_scroll`), its bar along the
    /// bottom: as wide as the row's content, narrower when its parent has less room. The bar only
    /// shows: a press on it is the row's (a tab's), and the wheel scrolls it. Only the user's
    /// wheel shows it, not a scroll from code (the tab on show brought into view).
    pub fn tracked_row(&self, id: &str, row: impl IntoElement, handle: &ScrollHandle, bg: u32) -> Div {
        let wheels = self.0.borrow_mut().entry(id.into()).or_default().wheels.clone();
        let counted = wheels.clone();
        let bar = self.bar_of(id, Axis::Across, handle, Fade::to(bg), false, move |_| counted.get(), |_| {});
        let wheeled = move |_: &ScrollWheelEvent, window: &mut Window, _: &mut App| {
            wheels.set(wheels.get() + 1.);
            window.refresh();
        };
        div().relative().flex_shrink().min_w_0().h_full().flex().child(row).child(bar).on_scroll_wheel(wheeled)
    }

    /// The bar and fades of a list under `handle`, alone, over its parent. `pinned`: the list
    /// keeps to its end (a chat on its latest), so growing there is no scroll that shows the bar.
    /// `moved`: after the bar scrolled it (a drag, a page).
    pub fn follow(
        &self,
        id: &str,
        axis: Axis,
        handle: &ScrollHandle,
        fade: Fade,
        pinned: bool,
        moved: impl Fn(&mut App) + 'static,
    ) -> AnyElement {
        let by = move |extent: Extent| if pinned { extent.offset - extent.max_offset() } else { extent.offset };
        self.bar_of(id, axis, handle, fade, true, by, moved)
    }

    /// The bar of a list under `handle`: `by`, the number whose change shows it.
    #[allow(clippy::too_many_arguments)] // Each is one area's own; a struct would only rename them.
    fn bar_of(
        &self,
        id: &str,
        axis: Axis,
        handle: &ScrollHandle,
        fade: Fade,
        pages: bool,
        by: impl Fn(Extent) -> f32 + 'static,
        moved: impl Fn(&mut App) + 'static,
    ) -> AnyElement {
        let (read, write) = (handle.clone(), handle.clone());
        let read = move || {
            let extent = extent(&read, axis);
            (extent, by(extent))
        };
        let scroll = move |offset, cx: &mut App| {
            scroll_to(&write, axis, offset);
            moved(cx);
        };
        self.overlay(id, axis, pages, Some(fade), read, scroll)
    }

    /// Scrolls an `area`'s list so its child `ix` is in view.
    pub fn scroll_to_item(&self, id: &str, ix: usize) {
        if let Some(bar) = self.0.borrow().get(id) {
            bar.handle.scroll_to_item(ix);
        }
    }

    /// A step from the keyboard through an `area`.
    pub fn step(&self, id: &str, step: Step) {
        if let Some(bar) = self.0.borrow().get(id) {
            scroll_to(&bar.handle, Axis::Down, extent(&bar.handle, Axis::Down).stepped(step, LINE));
        }
    }

    /// The bar alone, over its parent, along `axis`. `read`: the area as laid out this frame,
    /// and a number that changes when the user scrolls (a change of it shows the bar). `scroll`
    /// moves the area to an offset. `pages`: a press on the track beside the thumb pages;
    /// without it, that press goes to what is under the track. `fade`: the colour behind the
    /// area, which its edges fade into where more lies beyond, and its corners' radii; `None`, no fade.
    pub fn overlay(
        &self,
        id: &str,
        axis: Axis,
        pages: bool,
        fade: Option<Fade>,
        read: impl Fn() -> (Extent, f32) + 'static,
        scroll: impl Fn(f32, &mut App) + 'static,
    ) -> AnyElement {
        self.0.borrow_mut().entry(id.into()).or_default().seen = true;
        let (bars, id) = (self.clone(), id.to_string());
        let (laid, laid_id) = (bars.clone(), id.clone());
        canvas(
            move |area, window, _| {
                let (extent, moved_by) = read();
                let edges = extent.beyond(fade.map_or(0., |fade| fade.within));
                // Only an area that fades is listed: a terminal's lines run to its edges.
                if let Some(bar) = laid.0.borrow_mut().get_mut(&laid_id).filter(|_| fade.is_some()) {
                    bar.faded = Some((axis, edges.0, edges.1));
                }
                let inset = px(rules::INSET);
                let track = match axis {
                    Axis::Down => Bounds::new(
                        point(area.right() - px(rules::TRACK), area.top() + inset),
                        size(px(rules::TRACK), area.size.height - inset * 2.),
                    ),
                    Axis::Across => Bounds::new(
                        point(area.left() + inset, area.bottom() - px(rules::TRACK)),
                        size(area.size.width - inset * 2., px(rules::TRACK)),
                    ),
                };
                let thumb = extent.thumb(axis.along(point(track.size.width, track.size.height)))?;
                // Neither blocks what is under it: the root, the pane and the list still see the
                // pointer there. A press the bar takes is stopped in its own listener.
                let over = window.insert_hitbox(area, HitboxBehavior::Normal);
                let bar = window.insert_hitbox(track, HitboxBehavior::Normal);
                Some((extent, moved_by, thumb, over, bar, edges))
            },
            move |area, laid_out, window, cx| {
                let Some((extent, moved_by, (start, length), over, hitbox, edges)) = laid_out else { return };
                if let Some(fade) = fade {
                    paint_fades(area, axis, edges, fade, window);
                }
                let track = hitbox.bounds;
                let (track_start, track_length) =
                    (axis.along(track.origin), axis.along(point(track.size.width, track.size.height)));
                let now = Instant::now();
                // E2E runs always fade, whatever this Mac is set to.
                let always = !cx.should_auto_hide_scrollbars() && std::env::var_os("OTTOSPACE_E2E").is_none();
                let mut all = bars.0.borrow_mut();
                let Some(bar) = all.get_mut(&id) else { return };
                let over_track = !bar.left && hitbox.is_hovered(window);
                let engaged = over_track || (!bar.left && over.is_hovered(window)) || bar.grab.is_some();
                // A scroll shows the bar; so does the pointer, until a moment after it leaves.
                if (moved_by - bar.moved_by).abs() > 0.01 || (bar.engaged && !engaged) {
                    bar.active = Some(now);
                }
                (bar.moved_by, bar.engaged, bar.over_track) = (moved_by, engaged, over_track);
                let idle = bar.active.map(|at| now.duration_since(at).as_millis() as u64);
                let opacity = rules::opacity(always, engaged, idle);
                // Draw again when the fade starts, then every frame of it.
                match idle.filter(|_| !always && !engaged) {
                    Some(ms) if ms >= rules::IDLE_MS && opacity > 0. => window.request_animation_frame(),
                    Some(ms) if ms < rules::IDLE_MS && bar.wake.is_none_or(|due| due <= now) => {
                        let wait = Duration::from_millis(rules::IDLE_MS - ms);
                        bar.wake = Some(now + wait);
                        let handle = window.window_handle();
                        cx.spawn(async move |cx| {
                            cx.background_executor().timer(wait).await;
                            let _ = handle.update(cx, |_, window, _| window.refresh());
                        })
                        .detach();
                    }
                    _ => {}
                }
                let (width, color) = match (bar.grab, over_track) {
                    (Some(_), _) => (rules::TRACK, theme::SCROLL_THUMB_HELD),
                    (None, true) => (rules::TRACK, theme::SCROLL_THUMB_HOVER),
                    (None, false) => (rules::THUMB, theme::SCROLL_THUMB),
                };
                // Centred across the track, `start` along it.
                let side = px((rules::TRACK - width) / 2.);
                let thumb = match axis {
                    Axis::Down => {
                        Bounds::new(point(track.left() + side, track.top() + px(start)), size(px(width), px(length)))
                    }
                    Axis::Across => {
                        Bounds::new(point(track.left() + px(start), track.top() + side), size(px(length), px(width)))
                    }
                };
                bar.drawn = Some((track, thumb, opacity));
                drop(all);
                if opacity > 0. {
                    let color = Hsla::from(rgb(color)).opacity(opacity);
                    window.paint_quad(fill(thumb, color).corner_radii(px(width / 2.)));
                }

                // A press on the thumb holds it; one on the track pages. Nothing under the bar
                // gets that press: no row click, no selection, no pane drag. A bar that has
                // faded out takes none: what shows there does.
                let scroll = Rc::new(scroll);
                window.on_mouse_event({
                    let (bars, id, scroll, hitbox) = (bars.clone(), id.clone(), scroll.clone(), hitbox.clone());
                    move |event: &MouseDownEvent, phase, window, cx| {
                        let off = opacity <= 0. || axis == Axis::Across;
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || off {
                            return;
                        }
                        if !hitbox.is_hovered(window) {
                            return;
                        }
                        let at = axis.along(event.position) - track_start;
                        if (start..start + length).contains(&at) {
                            if let Some(bar) = bars.0.borrow_mut().get_mut(&id) {
                                bar.grab = Some(at - start);
                            }
                        } else if pages {
                            scroll(extent.paged(track_length, at), cx);
                        } else {
                            return;
                        }
                        cx.stop_propagation();
                        window.refresh();
                    }
                });
                window.on_mouse_event({
                    let (bars, id) = (bars.clone(), id.clone());
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        let mut all = bars.0.borrow_mut();
                        let Some(bar) = all.get_mut(&id) else { return };
                        let back = std::mem::take(&mut bar.left);
                        match bar.grab {
                            // The thumb follows the pointer, also past the area's edge.
                            Some(grab) if event.pressed_button == Some(MouseButton::Left) => {
                                let at = axis.along(event.position) - track_start;
                                drop(all);
                                scroll(extent.dragged(track_length, at - grab), cx);
                                cx.stop_propagation();
                            }
                            // Released where no release was seen, e.g. outside the window.
                            Some(_) => (bar.grab, bar.active) = (None, Some(Instant::now())),
                            None => {
                                let hovered = hitbox.is_hovered(window);
                                if !back && (hovered || over.is_hovered(window)) == engaged && hovered == over_track {
                                    return;
                                }
                            }
                        }
                        window.refresh();
                    }
                });
                window.on_mouse_event({
                    let (bars, id) = (bars.clone(), id.clone());
                    move |_: &MouseExitEvent, phase, window, _| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        if let Some(bar) = bars.0.borrow_mut().get_mut(&id) {
                            bar.left = true;
                            window.refresh();
                        }
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, _| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    if let Some(bar) = bars.0.borrow_mut().get_mut(&id).filter(|bar| bar.grab.is_some()) {
                        (bar.grab, bar.active) = (None, Some(Instant::now()));
                        window.refresh();
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    /// For the state dump: each area that can scroll, with its bar's `track` and `thumb` as
    /// `[x, y, w, h]` in window points and its `opacity` (0: faded out, and it takes no press).
    pub fn snapshot(&self) -> serde_json::Value {
        let bars = self.0.borrow();
        let drawn = bars.iter().filter_map(|(id, bar)| {
            let (track, thumb, opacity) = bar.drawn?;
            let bar = serde_json::json!({"track": crate::rect(track), "thumb": crate::rect(thumb), "opacity": opacity});
            Some((id.clone(), bar))
        });
        serde_json::Value::Object(drawn.collect())
    }

    /// For a test run's state: each area laid out this frame, also one that fits, with the edges
    /// that fade: `{"top", "bottom"}` down a list, `{"left", "right"}` across a row.
    pub fn overflow(&self) -> serde_json::Value {
        let bars = self.0.borrow();
        let laid = bars.iter().filter_map(|(id, bar)| {
            let (axis, start, end) = bar.faded?;
            let edges = match axis {
                Axis::Down => serde_json::json!({"top": start, "bottom": end}),
                Axis::Across => serde_json::json!({"left": start, "right": end}),
            };
            Some((id.clone(), edges))
        });
        serde_json::Value::Object(laid.collect())
    }
}

/// How an area's edges fade: into `bg`, the colour behind it, its outer corners rounded by
/// `radius` (start, end) as its container's, where more than `within` lies beyond (`Extent::beyond`).
#[derive(Clone, Copy, Default)]
pub struct Fade {
    pub bg: u32,
    pub radius: (f32, f32),
    pub within: f32,
}

impl Fade {
    /// Into `bg`, square, wherever anything lies beyond.
    pub fn to(bg: u32) -> Fade {
        Fade { bg, ..Fade::default() }
    }
}

/// The fades at an area's edges with more beyond them (`edges`: start, end): `bg`, the colour
/// behind the area, clearing into the area over `EDGE_FADE` (at most a quarter of it), its outer
/// corners rounded by `radius` (start, end) as its panel's. Drawn only: a press or hover goes through to the row under it.
fn paint_fades(
    area: Bounds<Pixels>,
    axis: Axis,
    edges: (bool, bool),
    Fade { bg, radius, .. }: Fade,
    window: &mut Window,
) {
    let (solid, clear) = (Hsla::from(rgb(bg)), Hsla::from(rgb(bg)).opacity(0.));
    let (w, h) = (area.size.width, area.size.height);
    let depth = px(EDGE_FADE).min(if axis == Axis::Down { h } else { w } / 4.);
    let (s, e) = (px(radius.0).min(depth), px(radius.1).min(depth));
    let corners =
        |top_left, top_right, bottom_right, bottom_left| Corners { top_left, top_right, bottom_right, bottom_left };
    let none = px(0.);
    // Each edge's band, its gradient's angle, solid at the edge (180°: top to bottom), and its corners.
    let (start, end) = match axis {
        Axis::Down => (
            (Bounds::new(area.origin, size(w, depth)), 180., corners(s, s, none, none)),
            (Bounds::new(point(area.left(), area.bottom() - depth), size(w, depth)), 0., corners(none, none, e, e)),
        ),
        Axis::Across => (
            (Bounds::new(area.origin, size(depth, h)), 90., corners(s, none, none, s)),
            (Bounds::new(point(area.right() - depth, area.top()), size(depth, h)), 270., corners(none, e, e, none)),
        ),
    };
    for (on, (band, angle, round)) in [(edges.0, start), (edges.1, end)] {
        if on {
            let shade = linear_gradient(angle, linear_color_stop(solid, 0.), linear_color_stop(clear, 1.));
            window.paint_quad(fill(band, shade).corner_radii(round));
        }
    }
}
