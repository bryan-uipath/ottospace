//! The user's wheel or trackpad over a parked app pane brings its real window up over the pane
//! for the scroll (a peek, `ottospace_core::app_wheel`); the app is activated so the window can
//! show above ottospace. macOS then scrolls the window under the pointer itself; the scroll that
//! started the peek reached ottospace, and is not passed on. The core says when the peek ends;
//! this module looks. See `docs/ai/sneak-apps.md`.

use std::time::{Duration, Instant};

use gpui::{Context, ScrollWheelEvent, Window};
use ottospace_core::{Action, Origin, PaneId};
use ottospace_mac::{input, wheel};

use crate::{Workspace, config, now_ms, on_screen, top_limit};

/// When the peek on now started, and counts for a test (the state's `counts`).
#[derive(Default)]
pub struct Wheel {
    pub over: u64,
    pub peeks: u64,
    /// A click or a key after this went to the app.
    since: Option<Instant>,
}

impl Workspace {
    /// A scroll over app pane `pane`'s picture: the core is asked for a peek.
    pub(crate) fn app_wheel(&mut self, pane: PaneId, _: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        self.wheel.over += 1;
        // Not while a drag is on.
        if config::get().settings.wheel_to_apps == Some(false) || input::left_button_down() {
            return;
        }
        let action = Action::AppWheel { pane, now: now_ms() };
        if let Some(why) = self.model.refusal(&action, self.input) {
            self.error = Some(why.into());
        }
        let effects = self.dispatch(action, self.input, window, cx);
        let fields =
            serde_json::json!({ "pane": pane, "origin": crate::trace::name(&self.input), "effects": effects.len() });
        crate::trace::record("wheel", fields);
    }

    /// `Effect::Peek`: the window goes over its pane as on entering, its app activated (it cannot
    /// be shown above ottospace otherwise).
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
