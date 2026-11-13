//! The scrollbar: GPUI scrolls overflow but draws no bar. An overlay thumb at the right edge of
//! a scroll area, in apollo-ui's future-dark style. Where the thumb is and what a drag or a
//! click scrolls to are the core's rules (`ottospace_core::scrollbar`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Bounds, DispatchPhase, Div, HitboxBehavior, Hsla, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, ScrollHandle, Stateful, StatefulInteractiveElement, Styled,
    canvas, div, fill, point, px, rgb, size,
};
use ottospace_core::scrollbar::{self as rules, Extent};
use ottospace_core::theme;

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
    /// The thumb is held: how far below its start.
    grab: Option<f32>,
    /// When the pending redraw is due.
    wake: Option<Instant>,
    /// The track, the thumb and the opacity as last painted. `None`: the content fits.
    drawn: Option<(Bounds<Pixels>, Bounds<Pixels>, f32)>,
}

/// Every scroll area's bar, by the area's id.
#[derive(Clone, Default)]
pub struct Bars(Rc<RefCell<BTreeMap<String, Bar>>>);

impl Bars {
    /// A frame starts: forgets the areas the last one did not render.
    pub fn begin(&self) {
        self.0.borrow_mut().retain(|_, bar| std::mem::take(&mut bar.seen));
        self.0.borrow_mut().values_mut().for_each(|bar| bar.drawn = None);
    }

    /// `list` (an `overflow_y_scroll` element that fills its parent) with its bar over it.
    pub fn area(&self, id: &str, list: Stateful<Div>) -> Div {
        let handle = self.0.borrow_mut().entry(id.into()).or_default().handle.clone();
        self.tracked(id, list.track_scroll(&handle), &handle)
    }

    /// As `area`, for a list that already scrolls under `handle`.
    pub fn tracked(&self, id: &str, list: impl IntoElement, handle: &ScrollHandle) -> Div {
        let (read, write) = (handle.clone(), handle.clone());
        let extent = move || {
            let (viewport, offset) = (f32::from(read.bounds().size.height), -f32::from(read.offset().y));
            (Extent { viewport, content: viewport + f32::from(read.max_offset().height), offset }, offset)
        };
        let scroll = move |offset: f32| write.set_offset(point(write.offset().x, px(-offset)));
        div().relative().flex_1().min_h_0().flex().flex_col().child(list).child(self.overlay(id, extent, scroll))
    }

    /// The bar alone, over its parent. `read`: the area as laid out this frame, and the number
    /// a scroll changes (a change of it shows the bar). `scroll` moves the area to an offset.
    pub fn overlay(
        &self,
        id: &str,
        read: impl Fn() -> (Extent, f32) + 'static,
        scroll: impl Fn(f32) + 'static,
    ) -> AnyElement {
        self.0.borrow_mut().entry(id.into()).or_default().seen = true;
        let (bars, id) = (self.clone(), id.to_string());
        canvas(
            move |area, window, _| {
                let (extent, moved_by) = read();
                let inset = px(rules::INSET);
                let track = Bounds::new(
                    point(area.right() - px(rules::TRACK), area.top() + inset),
                    size(px(rules::TRACK), area.size.height - inset * 2.),
                );
                let thumb = extent.thumb(track.size.height.into())?;
                // The track takes presses from what is under it; the wheel still scrolls the list.
                let over = window.insert_hitbox(area, HitboxBehavior::Normal);
                let bar = window.insert_hitbox(track, HitboxBehavior::BlockMouseExceptScroll);
                Some((extent, moved_by, thumb, over, bar))
            },
            move |_, laid_out, window, cx| {
                let Some((extent, moved_by, (start, length), over, hitbox)) = laid_out else { return };
                let track = hitbox.bounds;
                let track_length = f32::from(track.size.height);
                let now = Instant::now();
                // E2E runs always fade, whatever this Mac is set to.
                let always = !cx.should_auto_hide_scrollbars() && std::env::var_os("OTTOSPACE_E2E").is_none();
                let mut all = bars.0.borrow_mut();
                let Some(bar) = all.get_mut(&id) else { return };
                let engaged = over.is_hovered(window) || bar.grab.is_some();
                let over_track = hitbox.is_hovered(window);
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
                let thumb = Bounds::new(
                    point(track.left() + px((rules::TRACK - width) / 2.), track.top() + px(start)),
                    size(px(width), px(length)),
                );
                bar.drawn = Some((track, thumb, opacity));
                drop(all);
                if opacity > 0. {
                    let color = Hsla::from(rgb(color)).opacity(opacity);
                    window.paint_quad(fill(thumb, color).corner_radii(px(width / 2.)));
                }

                // A press on the thumb holds it; one on the track pages. Nothing under the bar
                // gets the press: no row click, no selection, no pane drag.
                let scroll = Rc::new(scroll);
                window.on_mouse_event({
                    let (bars, id, scroll, hitbox) = (bars.clone(), id.clone(), scroll.clone(), hitbox.clone());
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        if !hitbox.is_hovered(window) {
                            return;
                        }
                        let at = f32::from(event.position.y - track.top());
                        if (start..start + length).contains(&at) {
                            if let Some(bar) = bars.0.borrow_mut().get_mut(&id) {
                                bar.grab = Some(at - start);
                            }
                        } else {
                            scroll(extent.paged(track_length, at));
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
                        match bar.grab {
                            // The thumb follows the pointer, also past the area's edge.
                            Some(grab) if event.pressed_button == Some(MouseButton::Left) => {
                                let at = f32::from(event.position.y - track.top());
                                drop(all);
                                scroll(extent.dragged(track_length, at - grab));
                                cx.stop_propagation();
                            }
                            // Released where no release was seen, e.g. outside the window.
                            Some(_) => (bar.grab, bar.active) = (None, Some(Instant::now())),
                            None if over.is_hovered(window) == engaged && hitbox.is_hovered(window) == over_track => {
                                return;
                            }
                            None => {}
                        }
                        window.refresh();
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
    /// `[x, y, w, h]` in window points and its `opacity` (0: faded out, but a press still lands).
    pub fn snapshot(&self) -> serde_json::Value {
        let rect = |b: Bounds<Pixels>| -> [f32; 4] {
            [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()]
        };
        let bars = self.0.borrow();
        let drawn = bars.iter().filter_map(|(id, bar)| {
            let (track, thumb, opacity) = bar.drawn?;
            Some((id.clone(), serde_json::json!({"track": rect(track), "thumb": rect(thumb), "opacity": opacity})))
        });
        serde_json::Value::Object(drawn.collect())
    }
}
