//! Sneak labels inside the focused app pane: its window's tree is read off the UI thread while the
//! mode is on, and a label the user types presses that element. What gets a label, and who may
//! press, are the core's (`ottospace_core::sneak_apps`). See `docs/ai/sneak-apps.md`.

use std::time::{Duration, Instant};

use gpui::{Context, Window};
use ottospace_core::ax::Frame;
use ottospace_core::sneak_apps::{AppRead, CAP, Press, without_text};
use ottospace_core::{Action, Origin, PaneId};
use ottospace_mac::ax_press;

use crate::Workspace;

/// How long after one read of the app's tree the next starts, while the mode is on.
const AGAIN: Duration = Duration::from_millis(1000);

/// Reads of the app pane's tree for sneak mode.
#[derive(Default)]
pub struct Reads {
    /// A read is on its way: no second one starts.
    busy: bool,
    /// When the last one ended. `None`: the mode is off, so the next read starts at once.
    last: Option<Instant>,
    /// Test runs only: each press asked for, as `pane` and `path`, in the state's `counts`.
    pub presses: Vec<(PaneId, Vec<usize>)>,
}

impl Workspace {
    /// Every tick: while sneak mode is on, reads the focused app pane's tree (then the pane labelled)
    /// on a background thread, at most one read at a time and one a second. A pane no longer shown
    /// loses its labels.
    pub(crate) fn read_sneak_app(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(sneak) = &self.model.sneak else {
            self.sneak_reads.last = None;
            return;
        };
        let pane = sneak.app.map(|app| app.pane).or(self.model.focused);
        let Some(pane) = pane else { return };
        let shown = self.model.layout.as_ref().is_some_and(|l| l.leaves().contains(&pane));
        if !shown && sneak.app.is_some() {
            self.dispatch(Action::SneakApp { pane, read: None }, Origin::System, window, cx);
            return;
        }
        let due = self.sneak_reads.last.is_none_or(|last| last.elapsed() >= AGAIN);
        let Some(source) = self.ax_source(Some(pane)).filter(|_| due && !self.sneak_reads.busy) else { return };
        self.sneak_reads.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let read = cx.background_executor().spawn(async move { source.read_for_sneak() }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sneak_reads.busy = false;
                this.sneak_reads.last = Some(Instant::now());
                // No text of the app's is kept: an unreadable window is one with nothing to label.
                let read = read.ok();
                this.dispatch(Action::SneakApp { pane, read }, Origin::System, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// `Effect::PressApp`: the user typed the label of an element in `pane`'s window. It is found
    /// again, checked and pressed on a background thread; if it is not the one labelled any more,
    /// nothing is pressed and the status bar says why. A text field that took focus then gets the
    /// keyboard, as the user's own move; one that did not never does.
    pub(crate) fn press_app(
        &mut self,
        pane: PaneId,
        path: &[usize],
        role: &str,
        at: Frame,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            self.sneak_reads.presses.push((pane, path.to_vec()));
        }
        let Some(hosted) = self.hosted.get(&pane) else {
            self.error = Some(ax_press::GONE.into());
            return;
        };
        let (pid, id, path, role) = (hosted.pid, hosted.window_id, path.to_vec(), role.to_string());
        cx.spawn_in(window, async move |this, cx| {
            let pressed =
                cx.background_executor().spawn(async move { ax_press::press(pid, id, &path, &role, at) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match pressed {
                    Ok(Press::Focus) => drop(this.dispatch(Action::Enter { pane }, Origin::User, window, cx)),
                    Ok(Press::Press) => {}
                    Err(why) => this.error = Some(why.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The test-only `sneak_app_tree` request: `tree` as if read from the focused pane's window.
    /// Test runs host no window, so this is how a test gives sneak mode an app's elements.
    pub(crate) fn serve_sneak_app_tree(
        &mut self,
        read: AppRead,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let pane = self.model.focused.ok_or("no pane is focused")?;
        if self.model.sneak.is_none() {
            return Err("sneak mode is off".into());
        }
        let read = AppRead { root: without_text(read.root), ..read };
        self.dispatch(Action::SneakApp { pane, read: Some(read) }, Origin::System, window, cx);
        Ok(())
    }

    /// The status bar's word on the app's labels: e.g. `12 controls in Calculator`.
    pub(crate) fn sneak_app_hint(&self) -> Option<String> {
        let app = self.model.sneak.as_ref()?.app?;
        let name = self.hosted.get(&app.pane).map_or("the app", |h| h.app.as_str());
        let capped = if app.found > CAP { format!(" (of {})", app.found) } else { String::new() };
        let noun = if app.controls == 1 { "control" } else { "controls" };
        Some(format!("{} {noun}{capped} in {name}", app.controls))
    }
}
