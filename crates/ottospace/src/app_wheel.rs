//! The user's wheel or trackpad over a parked app pane, handed to that app's window, which stays
//! where it is (`ottospace_core::app_wheel`). Scrolls are added up per frame, go through the
//! core, and are checked against the pane's window when posted. See `docs/ai/sneak-apps.md`.

use gpui::{Context, ScrollDelta, ScrollWheelEvent, Window};
use objc2_core_foundation::CGPoint;
use ottospace_core::hosted::replay;
use ottospace_core::{Action, Origin, PaneId};
use ottospace_mac::wheel::Scroll;

use crate::{Workspace, config};

/// A scroll waiting for the next tick, for the window it was over when it came.
struct Pending {
    pane: PaneId,
    origin: Origin,
    pid: i32,
    window: u32,
    /// The point in the window's real place (global, top-left origin) under the pointer.
    at: CGPoint,
    scroll: Scroll,
}

/// The scrolls of this frame, and the one the core is being asked about.
#[derive(Default)]
pub struct Wheel {
    pending: Vec<Pending>,
    posting: Option<Pending>,
}

impl Workspace {
    /// A scroll over app pane `pane`'s picture: kept for the next tick, at the point of the
    /// window under the pointer. Over the gap beside a window smaller than the pane: nothing.
    pub(crate) fn app_wheel(&mut self, pane: PaneId, event: &ScrollWheelEvent) {
        let off = config::get().settings.wheel_to_apps == Some(false);
        let Some(h) = self.hosted.get(&pane).filter(|_| !off && self.model.live != Some(pane)) else { return };
        let preview = h.preview.get();
        let x = f64::from(f32::from(event.position.x - preview.origin.x));
        let y = f64::from(f32::from(event.position.y - preview.origin.y));
        let Some((x, y)) = replay((x, y), (h.shown.width, h.shown.height)) else { return };
        // The picture shows the window below its cropped title bar, unscaled from its top left.
        let at = CGPoint::new(h.parked_at.x + x, h.parked_at.y + h.shown_crop + y);
        // The system's own event, phases and momentum and all; a test's is made from its deltas.
        let (delta, precise) = match event.delta {
            ScrollDelta::Pixels(p) => ([f32::from(p.x).into(), f32::from(p.y).into()], true),
            ScrollDelta::Lines(l) => ([l.x.into(), l.y.into()], false),
        };
        let made = || Scroll::new(delta[0], delta[1], precise);
        let Some(scroll) = Scroll::current(delta).or_else(made) else { return };
        let (origin, pid, window) = (self.input, h.pid, h.window_id);
        // Added to the last one of the same pane, window, sender and phase: one event a frame.
        match self.wheel.pending.last_mut() {
            Some(last)
                if (last.pane, last.window, last.origin) == (pane, window, origin)
                    && last.scroll.phases() == scroll.phases() =>
            {
                last.scroll.add(&scroll);
                last.at = at;
            }
            _ => self.wheel.pending.push(Pending { pane, origin, pid, window, at, scroll }),
        }
    }

    /// Every tick: each scroll of the last frame goes to the core as its sender's, which says
    /// whether it reaches the app (`Effect::AppWheel`) and how far.
    pub(crate) fn flush_wheel(&mut self, window: &Window, cx: &mut Context<Self>) {
        for pending in std::mem::take(&mut self.wheel.pending) {
            let ([dx, dy], pane, origin) = (pending.scroll.points(), pending.pane, pending.origin);
            self.wheel.posting = Some(pending);
            self.dispatch(Action::AppWheel { pane, dx, dy }, origin, window, cx);
            self.wheel.posting = None;
        }
    }

    /// `Effect::AppWheel`: posts the scroll being flushed, `dx`, `dy` points at most, to the app
    /// alone. Only if the pane still hosts that window of that process, parked.
    pub(crate) fn post_wheel(&mut self, pane: PaneId, dx: f64, dy: f64) {
        let Some(mut pending) = self.wheel.posting.take().filter(|p| p.pane == pane) else { return };
        let hosted = self.hosted.get(&pane);
        let same = hosted.is_some_and(|h| (h.pid, h.window_id) == (pending.pid, pending.window));
        if same && self.model.live != Some(pane) {
            pending.scroll.limit([dx, dy]);
            pending.scroll.post(pending.pid, pending.window, pending.at);
        }
    }
}
