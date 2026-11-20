//! A tab out's window: one for each tab folded out to another display
//! (`ottospace_core::model::displays`). Under a "From" strip (its workspace, the tab, Fold back
//! in) it shows that tab's panes, and its workspace's rail on the edge away from the main window.
//! The workspace draws it and takes its keys and clicks; the core says which tab is where.

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, Bounds, Context, Div, ElementId, FocusHandle, Render, Stateful,
    Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, div, point, prelude::*, px, rgb,
    size, svg,
};
use ottospace_core::model::rail::{RailItem, RailSection};
use ottospace_core::model::{HOME_TAB, Owner};
use ottospace_core::{Action, Origin, theme};

use crate::home::Floor;
use crate::{TOP_BAR, Tooltip, Workspace, cut_text, display, drawer_rail, keycaps};

/// The root of a tab out's window.
pub(crate) struct OutWindow {
    ws: WeakEntity<Workspace>,
    tab: u32,
    focus: FocusHandle,
    _activation: Subscription,
}

impl Render for OutWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (tab, focus) = (self.tab, self.focus.clone());
        let drawn = self.ws.update(cx, |ws, cx| ws.render_out(tab, &focus, window, cx));
        drawn.unwrap_or_else(|_| div().into_any_element())
    }
}

/// The windows open for tabs out, by tab id. `None`: asked for, not open yet.
pub(crate) type OutWindows = Vec<(u32, Option<AnyWindowHandle>)>;

impl Workspace {
    /// After every step: a window for each tab out, on its display; none for a tab folded in.
    /// Opened and closed after the step, outside the update that asked for it.
    pub(crate) fn sync_out_windows(&mut self, cx: &mut Context<Self>) {
        let out = self.model.tabs_out();
        self.out_frames.retain(|tab, _| out.iter().any(|(id, _)| id == tab));
        for (tab, handle) in std::mem::take(&mut self.out_windows) {
            if out.iter().any(|(id, _)| *id == tab) {
                self.out_windows.push((tab, handle));
            } else if let Some(handle) = handle {
                cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window())));
            }
        }
        for (tab, place) in out {
            if self.out_windows.iter().any(|(id, _)| *id == tab) {
                continue;
            }
            let Some(frame) = self.model.displays().get(place).map(|d| d.frame) else { continue };
            self.out_windows.push((tab, None));
            let (ws, quiet) = (cx.entity().downgrade(), !self.hosting);
            // Key only if the keyboard is on it: a display plugged in again takes back its tab,
            // not the keyboard from what the user types in.
            let keys = self.model.tabs.get(self.model.active).is_some_and(|t| t.id == tab);
            cx.defer(move |cx| open(ws, tab, frame, quiet, keys, cx));
        }
    }

    /// The keyboard moved to another window by the user's step: that window becomes key. A test
    /// run's windows never take the keyboard by themselves.
    pub(crate) fn follow_keys(&mut self, cx: &mut Context<Self>) {
        let keys = self.model.tabs.get(self.model.active).map(|tab| tab.id).filter(|id| self.model.is_out(*id));
        if keys == self.keys_in || !self.hosting {
            self.keys_in = keys;
            return;
        }
        self.keys_in = keys;
        let to = match keys {
            Some(tab) => self.out_windows.iter().find(|(id, _)| *id == tab).and_then(|(_, handle)| *handle),
            None => Some(self.main_window),
        };
        if let Some(to) = to {
            cx.defer(move |cx| drop(to.update(cx, |_, window, _| window.activate_window())));
        }
    }

    /// The windows for the state: the main one, then each tab out's.
    pub(crate) fn windows_state(&self) -> Vec<serde_json::Value> {
        let main = self.model.tabs.get(self.model.main_at()).map(|tab| tab.id).filter(|id| *id != HOME_TAB);
        let keys_main = self.model.main_at() == self.model.active;
        let first = serde_json::json!({
            "main": true, "tab": main, "display": self.model.main_display(), "key": keys_main,
        });
        let out = self.model.tabs_out().into_iter().map(|(tab, display)| {
            let open = self.out_windows.iter().any(|(id, handle)| *id == tab && handle.is_some());
            let drawn = self.out_frames.get(&tab).copied().filter(|_| open);
            let (frame, frames, number) = (drawn.map(|d| d.0), drawn.map(|d| d.1), drawn.map(|d| d.2));
            let key = self.model.tabs.get(self.model.active).is_some_and(|t| t.id == tab);
            serde_json::json!({
                "main": false, "tab": tab, "display": display, "key": key, "frame": frame, "frames": frames,
                // For `screencapture -l`.
                "number": number,
            })
        });
        std::iter::once(first).chain(out).collect()
    }

    /// The window of tab out `tab`: its "From" strip over its panes, its workspace's rail on the
    /// edge away from the main window.
    pub(crate) fn render_out(
        &mut self,
        tab: u32,
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = div().size_full().flex().flex_col().bg(rgb(theme::BACKGROUND)).text_color(rgb(theme::TEXT));
        let Some(at) = self.model.out_at(tab) else { return root.into_any_element() };
        let b = window.bounds();
        let frame = [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()];
        let drawn = self.out_frames.get(&tab).map_or(0, |(_, n, _)| *n) + 1;
        // SAFETY: GPUI's own view, on the main thread.
        let number = crate::ns_view(window).map_or(0, |view| unsafe { ottospace_mac::window::number(view) });
        self.out_frames.insert(tab, (frame, drawn, number));
        // Its controls are its window's: noted apart, never as the main window's (sneak, tests).
        std::mem::swap(&mut self.controls, &mut self.out_controls);
        self.controls.begin();
        // Its own modifiers light its empty panes' caps.
        self.caps_drawn.set(keycaps::held(window));
        let tree = self.model.tree_at(at).map(|root| self.render_node(root, cx));
        let empty = tree.is_none().then(|| self.render_empty(Floor::Out(tab), None, keycaps::held(window), cx));
        let of = self.model.tabs[at].space.clone();
        let panes = div().flex_1().min_w_0().h_full().flex().overflow_hidden().children(empty).children(tree);
        // Left of the main window, its outer edge is its left.
        let place = self.model.tabs_out().into_iter().find(|(id, _)| *id == tab).map(|(_, place)| place);
        let left = place.zip(self.model.main_display()).is_some_and(|(place, main)| place < main);
        let rail = self.out_rail(&of, left, cx);
        let body = div().flex_1().min_h_0().w_full().flex();
        let body = if left { body.child(rail).child(panes) } else { body.child(panes).child(rail) };
        let root = self.input_handlers(root.track_focus(focus), cx);
        let drawn = root.child(self.out_strip(at, cx)).child(body).into_any_element();
        std::mem::swap(&mut self.controls, &mut self.out_controls);
        drawn
    }

    /// Where the main window has its tab row: "From" the workspace, a dot, the tab, and Fold back
    /// in at the right end. The window's title bar: room for its buttons on the left.
    fn out_strip(&self, at: usize, cx: &mut Context<Self>) -> Div {
        let tab = &self.model.tabs[at];
        let of = tab.space.clone();
        let colour = self.space_colour(&of);
        let icon = svg().path("icons/display-out.svg").size(px(14.)).flex_none().text_color(rgb(colour));
        let from = format!("From {}", self.model.space_title(&of));
        div()
            .h(px(TOP_BAR))
            .flex_none()
            .w_full()
            .pl(px(crate::TRAFFIC_LIGHTS))
            .pr_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .text_color(rgb(theme::MUTED))
            .child(icon)
            .child(cut_text(from).flex_shrink())
            .child("·")
            .child(cut_text(tab.title.clone()).text_color(rgb(theme::TEXT)).flex_shrink())
            .child(div().flex_1())
            .child(self.fold_in_button(tab.id, cx).child("Fold back in"))
    }

    /// Fold back in, for tab out `id`: on its tab in the main window's row and in its window's strip.
    pub(crate) fn fold_in_button(&self, id: u32, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(("tab-in", id))
            .flex_none()
            .px_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
            .child(svg().path("icons/fold-in.svg").size(px(12.)).flex_none().text_color(rgb(theme::MUTED)))
            .child(self.controls.mark(format!("tab-in:{id}")))
            .tooltip(|_, cx| AnyView::from(cx.new(|_| Tooltip("Fold back in  ⌘⇧O".into()))))
            .on_click(cx.listener(move |this, _, window, cx| {
                // Not also a click on its tab.
                cx.stop_propagation();
                this.dispatch(Action::TabIn { tab: Some(id) }, this.input, window, cx);
            }))
    }

    /// The workspace's rail, in its window: its items as the drawer's rail has them (`rail_of`),
    /// + for a new task there. A task's click shows it in the main window's drawer, the only one;
    /// the transcript's shows it there too.
    fn out_rail(&self, of: &Owner, left: bool, cx: &mut Context<Self>) -> Div {
        let item = |it: RailItem| {
            let (face, tip) = self.rail_face(it)?;
            let id = match it {
                RailItem::Task { id } => ElementId::Name(format!("out-rail:{id}").into()),
                RailItem::Transcript => ElementId::Name("out-rail:transcript".into()),
            };
            let item = div()
                .id(id)
                .flex_none()
                .size(px(36.))
                .rounded_lg()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::RAISED)))
                .child(face)
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let RailItem::Task { id } = it {
                        this.dispatch(Action::TaskGo { id: Some(id) }, this.input, window, cx);
                    }
                    this.dispatch(Action::RailSelect { item: it }, this.input, window, cx);
                }));
            Some((item, it.section() == RailSection::Everywhere))
        };
        let (foot, top): (Vec<_>, Vec<_>) = self.model.rail_of(of).into_iter().filter_map(item).partition(|(_, f)| *f);
        let new = div()
            .id("out-rail-new")
            .flex_none()
            .w(px(36.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(svg().path("icons/plus.svg").size(px(14.)).text_color(rgb(theme::MUTED)))
            .tooltip(|_, cx| AnyView::from(cx.new(|_| Tooltip("New task in this workspace (⌘N)".into()))))
            .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)));
        // The tasks and + scroll; the transcript stays at the foot.
        let tasks = div()
            .id("out-rail-tasks")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .overflow_y_scroll()
            .children(top.into_iter().map(|(item, _)| item))
            .child(new);
        let column = div()
            .w(px(drawer_rail::WIDTH))
            .flex_none()
            .h_full()
            .pt_2()
            .pb_2()
            .flex()
            .flex_col()
            .items_center()
            .bg(rgb(theme::BACKGROUND))
            .border_color(rgb(theme::BORDER))
            .child(tasks)
            .children(foot.into_iter().map(|(item, _)| item));
        if left { column.border_r_1() } else { column.border_l_1() }
    }
}

/// Opens tab `tab`'s window on the display at `frame` (global, top-left origin). A test run's
/// sits at the free corner as a sliver beside the main window's, as tall and wide as its display.
fn open(ws: WeakEntity<Workspace>, tab: u32, frame: ottospace_core::ax::Frame, quiet: bool, keys: bool, cx: &mut App) {
    let (w, h) = (frame.w as f32, frame.h as f32);
    let origin = if quiet && std::env::var_os("OTTOSPACE_E2E_SHOW").is_none() {
        let corner = display::parking_spot(f64::INFINITY);
        point(px(corner.x as f32 - crate::E2E_SLIVER), px(corner.y as f32 - 4. * crate::E2E_SLIVER))
    } else {
        // Inside the display, clear of its menu bar and Dock.
        point(px(frame.x as f32 + w * 0.05), px(frame.y as f32 + h * 0.08))
    };
    let size = if quiet { size(px(w), px(h)) } else { size(px(w * 0.9), px(h * 0.84)) };
    let titlebar = TitlebarOptions {
        title: Some("ottospace".into()),
        appears_transparent: true,
        traffic_light_position: Some(point(px(10.), px(13.))),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size })),
        titlebar: Some(titlebar),
        focus: !quiet && keys,
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.new(|cx| {
            // The user went to this window: the keyboard goes there.
            let keys = ws.clone();
            let activation = cx.observe_window_activation(window, move |_: &mut OutWindow, window, cx| {
                if window.is_window_active() {
                    in_main(&keys, Action::KeyWindow { tab }, cx);
                }
            });
            OutWindow { ws: ws.clone(), tab, focus, _activation: activation }
        })
    });
    // No window: none is asked for again; ⌘⇧O in the main window's row folds the tab in.
    let Ok(handle) = opened else { return };
    let handle: AnyWindowHandle = handle.into();
    let _ = handle.update(cx, |_, window, _| {
        if let Some(view) = crate::ns_view(window) {
            // SAFETY: GPUI's own view, on the main thread.
            unsafe { ottospace_mac::window::zoom_instead_of_fullscreen(view) };
        }
        if quiet {
            crate::keep_on_top(window);
        }
    });
    // A test run's main window stays in front of it: its sliver shows, so both draw.
    let _ = ws.update(cx, |ws, cx| {
        let main = ws.main_window;
        if quiet {
            cx.defer(move |cx| {
                let _ = main.update(cx, |_, window, _| {
                    // SAFETY: GPUI's own view, on the main thread.
                    crate::ns_view(window).map(|view| unsafe { ottospace_mac::window::order_front(view) })
                });
            });
        }
    });
    let _ = handle.update(cx, |_, window, cx| {
        // The red button folds it in: it closes no tab.
        let ws = ws.clone();
        window.on_window_should_close(cx, move |_, cx| {
            in_main(&ws, Action::TabIn { tab: Some(tab) }, cx);
            false
        });
    });
    let _ = ws.update(cx, |ws, cx| {
        match ws.out_windows.iter_mut().find(|(id, _)| *id == tab) {
            Some(entry) => entry.1 = Some(handle),
            // Folded in meanwhile.
            None => cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window()))),
        }
        cx.notify();
    });
}

/// The user's `action`, from a tab out's window, carried out against the main window: what it
/// asks of hosted windows (park one, say) is measured there.
fn in_main(ws: &WeakEntity<Workspace>, action: Action, cx: &mut App) {
    let Ok(main) = ws.read_with(cx, |ws, _| ws.main_window) else { return };
    let ws = ws.clone();
    cx.defer(move |cx| {
        let _ = main.update(cx, |_, window, cx| ws.update(cx, |ws, cx| ws.dispatch(action, Origin::User, window, cx)));
    });
}
