//! The user's wheel or trackpad over a parked app pane enters it (`ottospace_core::app_wheel`).
//! The scroll that entered reached ottospace's picture, not the app: it is held, with any that
//! follow before the window is up, and sent again as real input once it is. A scroll that
//! still reaches the picture of the live pane is the app's too, and is sent on the same way.

use std::time::Duration;

use gpui::{Context, ScrollWheelEvent, Window, px};
use ottospace_core::{Action, Origin, PaneId};
use ottospace_mac::{ax, input, wheel};

use crate::{Workspace, config};

/// How long the window takes to come up under the pointer, as for a click's replay.
const UP: Duration = Duration::from_millis(80);
/// Looks for one entry, `UP` apart (about 1 s): a scroll sent that came back to ottospace is held
/// again and sent at the next.
const TRIES: u8 = 12;

/// Scrolls held for the app, and counts for a test (the state's `counts`).
#[derive(Default)]
pub struct Wheel {
    pub over: u64,
    pub entries: u64,
    pub passed: u64,
    held: Option<Held>,
}

/// Points not yet sent to `pane`'s window, the looks left for this entry, and whether one is due.
/// `entry`: the going live it is for (`went_live`); a park and a new entry make it stale.
struct Held {
    pane: PaneId,
    entry: u64,
    dx: f32,
    dy: f32,
    tries: u8,
    sending: bool,
}

impl Workspace {
    /// A scroll over app pane `pane`'s picture: the core is asked whether it enters.
    pub(crate) fn app_wheel(
        &mut self,
        pane: PaneId,
        event: &ScrollWheelEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.wheel.over += 1;
        if config::live().wheel_to_apps == Some(false) {
            return;
        }
        // Lines as GPUI's own lists scroll them.
        let delta = event.delta.pixel_delta(px(16.));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let user = self.input == Origin::User && !self.injected;
        if self.model.live != Some(pane) {
            let action =
                Action::AppWheel { pane, dragging: input::left_button_down(), active: window.is_window_active() };
            if let Some(why) = self.model.refusal(&action, self.input) {
                self.error = Some(why.into());
            }
            let effects = self.dispatch(action, self.input, window, cx);
            let fields = serde_json::json!({ "pane": pane, "origin": crate::trace::name(&self.input), "effects": effects.len() });
            crate::trace::record("wheel", fields);
            if self.model.live != Some(pane) {
                return;
            }
            self.wheel.entries += 1;
            self.wheel.held = None;
        }
        // Only the user's own scroll goes on to another app.
        if user {
            self.hold_scroll(pane, dx, dy, cx);
        }
    }

    /// Adds a scroll to what `pane`'s window is sent, and sends it once the window is up.
    fn hold_scroll(&mut self, pane: PaneId, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let entry = self.went_live;
        if self.wheel.held.as_ref().is_none_or(|held| (held.pane, held.entry) != (pane, entry)) {
            self.wheel.held = Some(Held { pane, entry, dx: 0.0, dy: 0.0, tries: TRIES, sending: false });
        }
        let Some(held) = self.wheel.held.as_mut() else { return };
        (held.dx, held.dy) = (held.dx + dx, held.dy + dy);
        // Not once this entry's looks ran out: a scroll that keeps coming back is not sent forever.
        if !held.sending && held.tries > 0 {
            held.sending = true;
            self.send_later(entry, cx);
        }
    }

    fn send_later(&mut self, entry: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UP).await;
            let _ = this.update(cx, |this, cx| this.send_held(entry, cx));
        })
        .detach();
    }

    /// Sends what is held to the window under the pointer once its app is in front (before, the
    /// window is under ottospace's), while its pane is live. Tries again until `TRIES` run out.
    fn send_held(&mut self, entry: u64, cx: &mut Context<Self>) {
        let Some(held) = self.wheel.held.as_mut().filter(|held| held.entry == entry) else { return };
        held.tries = held.tries.saturating_sub(1);
        let live = self.model.live == Some(held.pane) && self.went_live == entry;
        let hosted = self.hosted.get(&held.pane).filter(|_| live && held.tries > 0);
        // Only to the window under the pointer: moved off it (onto a terminal, another pane), the scroll is dropped.
        let at = wheel::pointer();
        let under = hosted.is_some_and(|h| {
            let r = h.placed;
            r.origin.x <= at.x
                && at.x < r.origin.x + r.size.width
                && r.origin.y <= at.y
                && at.y < r.origin.y + r.size.height
        });
        let Some(pid) = hosted.filter(|_| under).map(|h| h.pid) else {
            (held.tries, held.sending) = (0, false);
            return;
        };
        if !ax::is_frontmost(pid) {
            return self.send_later(entry, cx);
        }
        let (dx, dy) = (held.dx.round() as i32, held.dy.round() as i32);
        (held.dx, held.dy, held.sending) = (0.0, 0.0, false);
        if (dx, dy) == (0, 0) {
            return;
        }
        wheel::scroll(at, dx, dy);
        self.wheel.passed += 1;
        crate::trace::record("wheel_passed", serde_json::json!({ "pane": held.pane, "dx": dx, "dy": dy }));
    }
}
