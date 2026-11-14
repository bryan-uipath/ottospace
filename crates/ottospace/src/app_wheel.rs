//! The user's wheel or trackpad over a parked app pane brings its real window up over the pane
//! for the scroll (a peek, `ottospace_core::app_wheel`); the app is activated so the window can
//! show above ottospace. macOS then scrolls the window under the pointer itself. The core says when the peek ends; this module looks.
//! See `docs/ai/sneak-apps.md`.

use std::time::{Duration, Instant};

use gpui::{Context, ScrollDelta, ScrollWheelEvent, Window};
use ottospace_core::{Action, Origin, PaneId};
use ottospace_mac::{input, wheel, wheel::Scroll};

use crate::{Workspace, config, now_ms, on_screen, top_limit};

/// How long the window server gets to put the window under the pointer before the first scroll
/// is handed on.
const SETTLE: Duration = Duration::from_millis(60);

/// The scroll that started a peek, and counts for a test (the state's `counts`).
#[derive(Default)]
pub struct Wheel {
    first: Option<Scroll>,
    pub over: u64,
    pub peeks: u64,
    /// When the peek on now started: a click or a key after it is in the app.
    since: Option<Instant>,
    pub handed: u64,
}

impl Workspace {
    /// A scroll over app pane `pane`'s picture: the core is asked for a peek.
    pub(crate) fn app_wheel(
        &mut self,
        pane: PaneId,
        event: &ScrollWheelEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.wheel.over += 1;
        // Not while a drag is on, nor for the first scroll handed on, come back here.
        let off = config::get().settings.wheel_to_apps == Some(false);
        let (held, back) = (input::left_button_down(), wheel::handed_on());
        if off || held || back {
            crate::trace::record("wheel_skipped", serde_json::json!({ "pane": pane, "held": held, "back": back }));
            return;
        }
        let delta = match event.delta {
            ScrollDelta::Pixels(p) => [f32::from(p.x).into(), f32::from(p.y).into()],
            ScrollDelta::Lines(l) => [l.x.into(), l.y.into()],
        };
        // Only a scroll the system queued (the user's hardware) is handed on; a test's is not.
        self.wheel.first = Scroll::current(delta);
        let action = Action::AppWheel { pane, now: now_ms() };
        if let Some(why) = self.model.refusal(&action, self.input) {
            self.error = Some(why.into());
        }
        let effects = self.dispatch(action, self.input, window, cx);
        self.wheel.first = None;
        let fields =
            serde_json::json!({ "pane": pane, "origin": crate::trace::name(&self.input), "effects": effects.len() });
        crate::trace::record("wheel", fields);
    }

    /// `Effect::Peek`: the window goes over its pane as on entering, its app activated (it cannot
    /// be shown above ottospace otherwise). The first scroll follows it.
    pub(crate) fn peek(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        let Some(hosted) = self.hosted.get_mut(&pane) else { return };
        crate::trace::record("peek", serde_json::json!({ "pane": pane, "window": hosted.window_id }));
        let rect = on_screen(hosted.bounds.get(), window, cx);
        hosted.place(rect, hosted.crop_at(rect, top_limit(window, cx)));
        // An inactive app's window stays under the active app's (AXRaise alone, or ottospace's
        // window ordered below it, left it there on CI): the app is activated, as on entering.
        hosted.window.raise();
        hosted.show(window.scale_factor() as f64);
        self.wheel.peeks += 1;
        self.wheel.since = Some(Instant::now());
        let Some(first) = self.wheel.first.take() else { return };
        let pid = hosted.pid;
        cx.spawn_in(window, async move |this, cx| {
            // Once the app is in front (its window above ottospace's), or after half a second.
            for _ in 0..25 {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                if ottospace_mac::ax::is_frontmost(pid) {
                    break;
                }
            }
            cx.background_executor().timer(SETTLE).await;
            let _ = this.update(cx, |this, _| {
                let front = ottospace_mac::ax::is_frontmost(pid);
                crate::trace::record("peek_hand_on", serde_json::json!({ "pane": pane, "front": front }));
                // Still up: to the window under the pointer, where the user scrolled.
                if this.model.peek.as_ref().is_some_and(|peek| peek.pane == pane) {
                    first.hand_on();
                    this.wheel.handed += 1;
                }
            });
        })
        .detach();
    }

    /// While a peek is on: where the pointer is, whether a click or a key came since it began,
    /// and when the last scroll was. The core decides what that ends.
    pub(crate) fn check_peek(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(pane) = self.model.peek.as_ref().map(|peek| peek.pane) else { return };
        let Some(hosted) = self.hosted.get(&pane) else { return };
        let (rect, at) = (on_screen(hosted.bounds.get(), window, cx), wheel::pointer());
        let over = rect.origin.x <= at.x
            && at.x < rect.origin.x + rect.size.width
            && rect.origin.y <= at.y
            && at.y < rect.origin.y + rect.size.height;
        // The app has the keyboard and the pointer: any press since the peek began went to it.
        let up = self.wheel.since.map_or(Duration::ZERO, |since| since.elapsed());
        let pressed = wheel::since_press().is_some_and(|ago| ago < up);
        let now = now_ms();
        let scrolled = wheel::since_scroll().map(|ago| now.saturating_sub(ago.as_millis() as u64));
        self.dispatch(Action::PeekCheck { now, over, pressed, scrolled }, Origin::System, window, cx);
    }
}
