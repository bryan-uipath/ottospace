//! A floating panel: a small borderless window of ottospace's over a live app's window, for what
//! must stay in view there (a card of the toast). Non-activating: it never takes the keyboard or
//! brings ottospace forward, and a click on it reaches what it draws. It is sized to what it
//! draws, so a click beside that goes to the window under it: what has gaps takes a panel a
//! piece. Generic: the workspace draws it (`Draw`) and says where it goes; what to show, and
//! when, is the caller's.

use gpui::{
    AnyElement, AnyWindowHandle, Bounds, Context, Render, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, div, point, prelude::*, px, size,
};
use ottospace_core::ax::Frame;

use crate::{Marks, Workspace, ns_view};

/// Draws the content of the caller's panel `at`, in the panel's window.
pub(crate) type Draw = fn(&mut Workspace, usize, &mut Window, &mut Context<Workspace>) -> AnyElement;
/// The panel `at` of those a workspace keeps for one use.
pub(crate) type Slot = fn(&mut Workspace, usize) -> &mut Panel;

/// The panel's root: the workspace draws it.
struct View {
    ws: WeakEntity<Workspace>,
    draw: Draw,
    at: usize,
}

impl Render for View {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (draw, at) = (self.draw, self.at);
        let drawn = self.ws.update(cx, |ws, cx| draw(ws, at, window, cx));
        drawn.unwrap_or_else(|_| div().into_any_element())
    }
}

/// One panel: closed, opening, or open at `frame`.
#[derive(Default)]
pub(crate) struct Panel {
    handle: Option<AnyWindowHandle>,
    opening: bool,
    /// Where it is, in Accessibility's coordinates (global, from the main screen's top left).
    frame: Option<Frame>,
    /// Where its parts are drawn, from the panel's top left.
    pub(crate) marks: Marks,
}

impl Panel {
    /// Where it shows, while it is open or opening.
    pub(crate) fn frame(&self) -> Option<Frame> {
        self.frame.filter(|_| self.handle.is_some() || self.opening)
    }

    /// Where it shows, once it has drawn what it shows (its marks noted).
    pub(crate) fn drawn(&self) -> Option<Frame> {
        self.frame.filter(|_| self.handle.is_some() && !self.marks.borrow().is_empty())
    }

    /// Open at `want`, moved there, or closed (`None`); drawn again with `redraw`. Windows open
    /// and close after the step, outside the update that asks.
    pub(crate) fn sync(
        &mut self,
        want: Option<Frame>,
        redraw: bool,
        at: usize,
        draw: Draw,
        slot: Slot,
        cx: &mut Context<Workspace>,
    ) {
        let Some(frame) = want else {
            self.frame = None;
            // Opened again, it has drawn nothing yet.
            self.marks.borrow_mut().clear();
            if let Some(handle) = self.handle.take() {
                cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window())));
            }
            return;
        };
        let moved = self.frame != Some(frame);
        self.frame = Some(frame);
        match self.handle {
            Some(handle) if moved || redraw => {
                if moved {
                    place(handle, frame, cx);
                }
                cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.refresh())));
            }
            Some(_) => {}
            None if !self.opening => {
                self.opening = true;
                let ws = cx.entity().downgrade();
                cx.defer(move |cx| open(ws, frame, at, draw, slot, cx));
            }
            None => {}
        }
    }
}

/// Moves the panel's window to `frame`, from a task: outside any update, so AppKit's resize
/// reaches GPUI and it lays the panel out at its new size. From inside one GPUI misses it.
fn place(handle: AnyWindowHandle, frame: Frame, cx: &mut gpui::App) {
    cx.spawn(async move |cx| {
        let view = handle.update(cx, |_, window, _| ns_view(window)).ok().flatten();
        if let Some(view) = view {
            // SAFETY: GPUI's own view of a live window, on the main thread.
            unsafe { ottospace_mac::window::set_frame(view, frame) };
        }
    })
    .detach();
}

fn open(ws: WeakEntity<Workspace>, frame: Frame, at: usize, draw: Draw, slot: Slot, cx: &mut gpui::App) {
    let bounds = Bounds {
        origin: point(px(frame.x as f32), px(frame.y as f32)),
        size: size(px(frame.w as f32), px(frame.h as f32)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        // A non-activating panel (GPUI's pop-up), floated as a hosted window is (`place`).
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let view = ws.clone();
    let opened = cx.open_window(options, move |_, cx| cx.new(|_| View { ws: view, draw, at }));
    let handle: Option<AnyWindowHandle> = opened.ok().map(Into::into);
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| {
            if let Some(view) = ns_view(window) {
                // SAFETY: GPUI's own view, on the main thread. Over other apps' windows, then placed.
                unsafe { ottospace_mac::window::float_panel(view) };
            }
        });
        place(handle, frame, cx);
    }
    let _ = ws.update(cx, |ws, cx| {
        let panel = slot(ws, at);
        panel.opening = false;
        match (handle, panel.frame) {
            (Some(handle), Some(now)) => {
                panel.handle = Some(handle);
                // Moved while it opened.
                if now != frame {
                    place(handle, now, cx);
                }
            }
            // Not wanted any more, or no window: none is kept.
            (Some(handle), None) => cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window()))),
            (None, _) => panel.frame = None,
        }
        cx.notify();
    });
}
