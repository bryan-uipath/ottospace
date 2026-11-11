//! Imperative shell: draws the workspace, hosts other apps' windows in panes, and turns input
//! (mouse, keys, control socket) into `ottospace_core` actions. State rules live in the core.
//!
//! An app pane shows its parked window's SCK stream. Clicking it moves the real window over the
//! pane ("live") and replays the click; clicking ottospace itself parks it again.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_video::pixel_buffer::{CVPixelBuffer, CVPixelBufferRef};
use gpui::{
    AnyElement, App, Application, BorderStyle, Bounds, ClickEvent, ContentMask, Context, FocusHandle, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Size, Subscription, Window, WindowBounds, WindowOptions,
    actions, canvas, div, point, prelude::*, px, quad, rgb, size, surface, transparent_black,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_core::keys::{self, Mods};
use ottospace_core::{Action, Axis, Dir, Effect, Kind, Mode, Model, Node, Pane, PaneId};
use ottospace_mac::{ax, capture, display, input, recovery};
use screencapturekit::prelude::*;

mod control;
mod terminal;
use control::{Call, PaneKind, Request};
use terminal::Terminal;

/// Workspace background; also fills hosted windows' rounded corners.
const BG: u32 = 0x1e1e2e;

/// Latest captured frame, handed from SCK's queue to the UI thread.
struct Frame(CVPixelBuffer);
// CVPixelBuffer is a CF object; retain/release are thread-safe.
unsafe impl Send for Frame {}

#[derive(Default)]
struct Latest {
    frame: Mutex<Option<Frame>>,
    dirty: AtomicBool,
    /// Configured stream size as `width << 32 | height`. Frames of any other size are transitional
    /// (mid-reconfigure or mid-move) and render as a small window on black, so they're dropped.
    expected: AtomicU64,
}

impl Latest {
    fn expect(&self, config: &SCStreamConfiguration) {
        self.expected.store((config.width() as u64) << 32 | config.height() as u64, Relaxed);
    }
}

struct Output(Arc<Latest>);

impl SCStreamOutputTrait for Output {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, _: SCStreamOutputType) {
        let ptr = sample.image_buffer_ptr_borrowed();
        if ptr.is_null() {
            return; // idle frames carry no image
        }
        let buffer = unsafe { CVPixelBuffer::wrap_under_get_rule(ptr as CVPixelBufferRef) };
        let size = (buffer.get_width() as u64) << 32 | buffer.get_height() as u64;
        if size != self.0.expected.load(Relaxed) {
            return;
        }
        if let Ok(mut slot) = self.0.frame.lock() {
            *slot = Some(Frame(buffer));
            self.0.dirty.store(true, Relaxed);
        }
    }
}

/// Stream config for a pane of `size` points at `scale` (Retina = 2). Scales the window to fill it.
fn stream_config(size: CGSize, scale: f64) -> SCStreamConfiguration {
    // 420f is the only format gpui's surface renderer accepts.
    SCStreamConfiguration::new()
        .with_width((size.width * scale).max(1.0) as u32)
        .with_height((size.height * scale).max(1.0) as u32)
        .with_scales_to_fit(true)
        .with_pixel_format(PixelFormat::YCbCr_420f)
        .with_fps(60)
}

fn to_cg(size: gpui::Size<Pixels>) -> CGSize {
    CGSize::new(f32::from(size.width) as f64, f32::from(size.height) as f64)
}

/// Shell half of an app pane: the window, its stream and parking state.
struct Hosted {
    window_id: u32,
    title: String,
    window: ax::Window,
    original: CGRect,
    latest: Arc<Latest>,
    stream: SCStream,
    pid: i32,
    /// Window corner radius in points, for masking the stream's black corners.
    radius: f64,
    /// Body bounds in window-content coordinates, captured during prepaint.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Size the window was parked at, to re-park only when the pane resizes.
    parked_size: CGSize,
    /// Last rect the live window was placed at, to re-place only on change.
    placed: CGRect,
}

impl Hosted {
    fn new(target: capture::Target) -> Result<Self, String> {
        let capture::Target { window, sc_window, id } = target;
        let original = window.frame().ok_or("could not read window frame")?;
        let latest = Arc::new(Latest::default());
        let radius = capture::corner_radius(&sc_window, 2.0).unwrap_or(0.0);
        eprintln!("hosting window {id}: corner radius {radius:.1}pt");
        let filter = SCContentFilter::create().with_window(&sc_window).build().map_err(|e| format!("{e:?}"))?;
        // Resized to the pane on park.
        let config = stream_config(original.size, 2.0);
        latest.expect(&config);
        let mut stream = SCStream::new(&filter, &config).map_err(|e| format!("{e:?}"))?;
        stream.add_output_handler(Output(latest.clone()), SCStreamOutputType::Screen).map_err(|e| format!("{e:?}"))?;
        stream.start_capture().map_err(|e| format!("{e:?}"))?;
        let title = window.title().filter(|t| !t.is_empty()).unwrap_or_else(|| format!("window {id}"));
        let pid = window.pid();
        recovery::remember(pid, id, original);
        Ok(Self {
            window_id: id,
            title,
            window,
            original,
            latest,
            stream,
            pid,
            radius,
            bounds: Rc::default(),
            parked_size: CGSize::ZERO,
            placed: CGRect::default(),
        })
    }

    /// Stops capture and puts the window back where we found it.
    fn release(self) {
        let _ = self.stream.stop_capture();
        let _ = self.window.set_size(self.original.size);
        let _ = self.window.set_position(self.original.origin);
        recovery::forget(self.window_id);
    }

    /// Sizes the window to the pane (so the stream matches) and parks it on the off-screen sliver.
    fn park(&mut self, scale: f64) {
        let size = to_cg(self.bounds.get().size);
        // Only resize/reconfigure when the pane changed; redoing it on every park flickers.
        if size != self.parked_size {
            let _ = self.window.set_size(size);
            // Match the stream to the pane so frames aren't letterboxed into a stale-sized buffer.
            let config = stream_config(size, scale);
            self.latest.expect(&config);
            let _ = self.stream.update_configuration(&config);
        }
        let _ = self.window.set_position(display::parking_spot());
        self.parked_size = size;
        self.placed = CGRect::default();
    }

    fn place(&mut self, rect: CGRect) {
        let _ = self.window.set_size(rect.size);
        let _ = self.window.set_position(rect.origin);
        self.placed = rect;
    }

    fn frame(&self) -> Option<CVPixelBuffer> {
        self.latest.frame.lock().ok()?.as_ref().map(|f| f.0.clone())
    }
}

actions!(ottospace, [SplitRight, SplitDown, ClosePane, FocusLeft, FocusRight, FocusUp, FocusDown]);

struct Workspace {
    model: Model,
    /// Shell half of each app pane.
    hosted: HashMap<PaneId, Hosted>,
    /// Shell half of each terminal pane.
    terminals: HashMap<PaneId, Terminal>,
    /// One terminal character cell.
    cell: Size<Pixels>,
    /// Each pane's body bounds in window coordinates, recorded during prepaint.
    cells: HashMap<PaneId, Rc<Cell<Bounds<Pixels>>>>,
    apps: Vec<String>,
    /// Window ids seen per hosted pid; anything new that appears later gets its own pane.
    known: Vec<(i32, HashSet<u32>)>,
    /// When the live pane went live.
    live_since: Option<Instant>,
    error: Option<String>,
    /// False in e2e runs, which must never move the user's real windows.
    hosting: bool,
    control: Option<Receiver<Call>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    fn new(hosting: bool, control: Option<Receiver<Call>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            // Clicking ottospace itself (title bar, empty space) raises it over the live window,
            // so hand that pane back to its stream. Ignore the activation that the swap click
            // itself delivers late (the race that used to bounce windows straight back).
            cx.observe_window_activation(window, |this, window, cx| {
                let settled = this.live_since.is_some_and(|at| at.elapsed() > Duration::from_millis(400));
                if window.is_window_active() && settled {
                    this.dispatch(Action::Leave, window, cx);
                }
            }),
            // Native fullscreen puts ottospace in its own Space, where other apps' windows can't
            // follow. Swap it for pseudo fullscreen: leave the Space, then fill the screen.
            cx.observe_window_bounds(window, |_, window, cx| {
                if window.is_fullscreen() {
                    window.toggle_fullscreen();
                    cx.spawn_in(window, async move |_, cx| {
                        cx.background_executor().timer(Duration::from_millis(800)).await;
                        let _ = cx.update(|window, _| window.zoom_window());
                    })
                    .detach();
                }
            }),
            cx.on_app_quit(|this, _| {
                this.hosted.drain().for_each(|(_, hosted)| hosted.release());
                if this.control.is_some() {
                    let _ = std::fs::remove_file(control::path());
                }
                async {}
            }),
        ];

        // ~60Hz tick: answer control calls, redraw on new frames, keep windows matched to panes.
        // Every ~500ms, also look for new and closed windows of hosted apps.
        cx.spawn_in(window, async move |this, cx| {
            for tick in 0u64.. {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    this.serve_control(window, cx);
                    if tick % 30 == 0 {
                        this.adopt_new_windows(window, cx);
                        this.drop_closed(window, cx);
                    }
                    this.sync_layout(window, cx, tick % 30 == 15);
                    this.sync_terminals(window, cx);
                    if this.hosted.values().any(|h| h.latest.dirty.swap(false, Relaxed)) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            model: Model::default(),
            hosted: HashMap::new(),
            terminals: HashMap::new(),
            cell: terminal::cell_size(window),
            cells: HashMap::new(),
            apps: if hosting { capture::apps() } else { Vec::new() },
            known: Vec::new(),
            live_since: None,
            error: None,
            hosting,
            control,
            focus,
            _subscriptions: subscriptions,
        }
    }

    /// The one path that changes state: updates the model, then carries out its effects.
    fn dispatch(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) -> Vec<Effect> {
        let effects = self.model.update(action);
        for effect in &effects {
            match effect {
                Effect::Opened(id) => {
                    if self.model.pane(*id).is_some_and(|p| p.kind == Kind::Terminal) {
                        match Terminal::spawn() {
                            Ok(terminal) => drop(self.terminals.insert(*id, terminal)),
                            Err(err) => self.error = Some(format!("terminal: {err}")),
                        }
                    }
                }
                Effect::Closed(pane) => {
                    self.cells.remove(&pane.id);
                    self.terminals.remove(&pane.id); // drop ends its shell
                    if let Some(hosted) = self.hosted.remove(&pane.id) {
                        hosted.release();
                    }
                }
                Effect::Parked(id) => {
                    self.live_since = None;
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        hosted.park(window.scale_factor() as f64);
                    }
                }
                Effect::Live(id) => {
                    self.live_since = Some(Instant::now());
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        hosted.place(on_screen(hosted.bounds.get(), window, cx));
                        hosted.window.raise();
                    }
                }
            }
        }
        cx.notify();
        effects
    }

    /// Answers queued control-socket calls. Each reply is the state after the call.
    fn serve_control(&mut self, window: &Window, cx: &mut Context<Self>) {
        let calls: Vec<Call> = self.control.iter().flat_map(|rx| rx.try_iter()).collect();
        for Call { request, reply } in calls {
            let quit = matches!(request, Request::Quit);
            let mut failure = None;
            let action = match request {
                Request::State | Request::Quit => None,
                Request::OpenApp { name, axis } => {
                    self.open_app(&name, axis, window, cx);
                    None
                }
                Request::Split { axis, kind } => {
                    let kind = match kind {
                        PaneKind::Terminal => Kind::Terminal,
                        PaneKind::Empty => Kind::Empty,
                    };
                    Some(Action::Split { axis, kind })
                }
                Request::Type { pane, text } => {
                    match pane.or(self.model.focused).and_then(|id| self.terminals.get(&id)) {
                        Some(terminal) => terminal.write(text.into_bytes()),
                        None => failure = Some("no terminal pane to type into"),
                    }
                    None
                }
                Request::Close { pane } => Some(Action::Close { pane }),
                Request::Focus { pane } => Some(Action::Focus { pane }),
                Request::FocusDir { dir } => Some(Action::FocusDir { dir }),
                Request::Enter { pane } => Some(Action::Enter { pane }),
                Request::Leave => Some(Action::Leave),
            };
            if let Some(action) = action {
                self.dispatch(action, window, cx);
            }
            let _ = reply.send(failure.map_or_else(|| self.snapshot(), control::error));
            if quit {
                cx.quit();
            }
        }
    }

    /// State as JSON: the model, plus each pane's drawn bounds `[x, y, w, h]` in window points.
    fn snapshot(&self) -> String {
        let bounds: BTreeMap<PaneId, [f32; 4]> = self
            .cells
            .iter()
            .map(|(id, cell)| {
                let b = cell.get();
                (*id, [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()])
            })
            .collect();
        let terminals: BTreeMap<PaneId, serde_json::Value> = self
            .terminals
            .iter()
            .map(|(id, t)| {
                let (cols, rows) = t.grid_size();
                (*id, serde_json::json!({ "cols": cols, "rows": rows, "lines": t.lines() }))
            })
            .collect();
        serde_json::json!({
            "ok": true,
            "mode": self.model.mode(),
            "state": self.model,
            "bounds": bounds,
            "terminals": terminals,
            "error": self.error,
        })
        .to_string()
    }

    /// Adds an app pane for an already-captured window.
    fn attach(&mut self, hosted: Hosted, axis: Axis, beside: Option<PaneId>, window: &Window, cx: &mut Context<Self>) {
        let pid = hosted.pid;
        if !self.known.iter().any(|(p, _)| *p == pid) {
            let ids = ax::Window::standard_of(pid).iter().filter_map(ax::Window::id).collect();
            self.known.push((pid, ids));
        }
        let kind = Kind::App { window: hosted.window_id, title: hosted.title.clone() };
        let effects = self.dispatch(Action::Open { kind, axis, beside }, window, cx);
        if let Some(Effect::Opened(id)) = effects.last() {
            self.cells.insert(*id, hosted.bounds.clone());
            self.hosted.insert(*id, hosted);
        }
    }

    fn open_app(&mut self, name: &str, axis: Axis, window: &Window, cx: &mut Context<Self>) {
        if !self.hosting {
            self.error = Some("window hosting is off in e2e runs".into());
            return;
        }
        let target = capture::find(name);
        if let Ok(t) = &target
            && self.hosted.values().any(|h| h.window_id == t.id)
        {
            return; // already hosted
        }
        match target.and_then(Hosted::new) {
            Ok(hosted) => {
                self.attach(hosted, axis, None, window, cx);
                self.error = None;
            }
            Err(err) => self.error = Some(err),
        }
        cx.notify();
    }

    /// Removes panes whose window is gone (closed by the user or the app).
    fn drop_closed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let gone: Vec<PaneId> =
            self.hosted.iter().filter(|(_, h)| h.window.frame().is_none()).map(|(id, _)| *id).collect();
        for id in gone {
            eprintln!("pane {id}: window gone (AX frame unreadable); closing");
            // Nothing to put back, so stop the stream here instead of `release` in `dispatch`.
            if let Some(hosted) = self.hosted.remove(&id) {
                let _ = hosted.stream.stop_capture();
                recovery::forget(hosted.window_id);
            }
            self.dispatch(Action::Close { pane: Some(id) }, window, cx);
        }
    }

    /// Hosts windows that appeared since we started watching their app (e.g. undocked DevTools),
    /// each to the right of its app's newest pane.
    fn adopt_new_windows(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut fresh = Vec::new();
        for (pid, seen) in &mut self.known {
            for window in ax::Window::standard_of(*pid) {
                if let Some(id) = window.id()
                    && seen.insert(id)
                {
                    fresh.push(window);
                }
            }
        }
        for ax_window in fresh {
            if let Ok(hosted) = capture::target(ax_window).and_then(Hosted::new) {
                let beside = self.hosted.iter().filter(|(_, h)| h.pid == hosted.pid).map(|(id, _)| *id).max();
                self.attach(hosted, Axis::Right, beside, window, cx);
            }
        }
    }

    /// Parks panes whose size changed, and keeps the live window glued to its pane: re-places it
    /// when the pane moves, and every ~500ms (`check`) if the app or user moved it away.
    fn sync_layout(&mut self, window: &Window, cx: &App, check: bool) {
        let scale = window.scale_factor() as f64;
        let live = self.model.live;
        for (id, hosted) in &mut self.hosted {
            let rect = on_screen(hosted.bounds.get(), window, cx);
            if rect.size.width < 1.0 {
                continue; // not laid out yet
            }
            if live == Some(*id) {
                let drifted = check && hosted.window.frame().is_some_and(|f| f.origin != rect.origin);
                if hosted.placed != rect || drifted {
                    hosted.place(rect);
                }
            } else if hosted.parked_size != rect.size {
                hosted.park(scale);
            }
        }
    }

    /// Fits each terminal's grid to its pane, redraws on new output, closes panes whose shell exited.
    fn sync_terminals(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut redraw = false;
        let mut exited = Vec::new();
        for (id, terminal) in &mut self.terminals {
            if let Some(bounds) = self.cells.get(id) {
                terminal.fit(bounds.get().size, self.cell);
            }
            redraw |= terminal.take_dirty();
            if terminal.exited() {
                exited.push(*id);
            }
        }
        for id in exited {
            self.dispatch(Action::Close { pane: Some(id) }, window, cx);
        }
        if redraw {
            cx.notify();
        }
    }

    /// Keys go to the focused terminal. ⌘ shortcuts do not encode, so they fall through to bindings.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.model.focused.and_then(|id| self.terminals.get(&id)) else { return };
        let key = &event.keystroke;
        let m = key.modifiers;
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, cmd: m.platform };
        if let Some(bytes) = keys::encode(&key.key, key.key_char.as_deref(), mods, terminal.app_cursor()) {
            terminal.write(bytes);
            cx.stop_propagation();
        }
    }

    /// A click on a pane body. An app pane goes live, then the click is replayed so it lands in
    /// the app (otherwise the first click is spent on the swap). Other panes just take focus.
    fn click_pane(&mut self, id: PaneId, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        if !self.hosted.contains_key(&id) {
            self.dispatch(Action::Focus { pane: id }, window, cx);
            return;
        }
        self.dispatch(Action::Enter { pane: id }, window, cx);
        let Some(hosted) = self.hosted.get(&id) else { return };
        let local = event.position - hosted.bounds.get().origin;
        let origin = hosted.placed.origin;
        let at = CGPoint::new(origin.x + f32::from(local.x) as f64, origin.y + f32::from(local.y) as f64);
        let clicks = event.click_count as i64;
        cx.spawn(async move |_, cx| {
            // Give the window server a moment to put the real window under the cursor.
            cx.background_executor().timer(Duration::from_millis(80)).await;
            input::click(at, clicks);
        })
        .detach();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar")
            .w(px(220.))
            .h_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(rgb(0x181825))
            .border_r_1()
            .border_color(rgb(0x313244))
            .overflow_y_scroll()
            // Clicking ottospace's own UI hands the live pane back to its stream.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::Leave, window, cx);
                }),
            )
            .child(
                div()
                    .id("refresh")
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(0x7f849c))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(0xcdd6f4)))
                    .child("APPS  ↻")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.hosting {
                            this.apps = capture::apps();
                        }
                        cx.notify();
                    })),
            )
            .children(self.apps.iter().enumerate().map(|(i, name)| {
                let target = name.clone();
                div()
                    .id(("app", i))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x45475a)))
                    .child(name.clone())
                    // Fills the focused pane if it is empty. Else splits it: right, or down with shift.
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                        this.open_app(&target, axis, window, cx)
                    }))
            }))
            .children(self.error.clone().map(|e| div().p_2().text_xs().text_color(rgb(0xf38ba8)).child(e)))
    }

    fn render_pane(&self, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement {
        let id = pane.id;
        let cell = self.cells.get(&id).cloned().unwrap_or_default();
        let (tag, tag_color, title) = match &pane.kind {
            Kind::Empty => ("EMPTY", 0x7f849c, String::new()),
            Kind::Terminal => ("TERM", 0x89b4fa, "shell".into()),
            Kind::App { title, .. } => ("APP", 0xfab387, title.clone()),
        };
        let header = div()
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(0x181825))
            .border_b_1()
            .border_color(rgb(0x313244))
            .text_xs()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::Focus { pane: id }, window, cx);
                }),
            )
            .child(
                div().flex().gap_2().overflow_hidden().child(div().text_color(rgb(tag_color)).child(tag)).child(title),
            )
            .child(
                div()
                    .id(("close", id))
                    .px_1()
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(0xf38ba8)))
                    .child("×")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(Action::Close { pane: Some(id) }, window, cx);
                    })),
            );
        let content = if let Some(hosted) = self.hosted.get(&id) {
            // While live, keep showing the last frame under the real window, so any gap during a swap is invisible.
            match hosted.frame() {
                Some(buffer) => surface(buffer).size_full().into_any_element(),
                None => div().child("waiting for frames…").into_any_element(),
            }
        } else if let Some(terminal) = self.terminals.get(&id) {
            let screen = terminal.screen();
            let cell = self.cell;
            let has_keys = self.model.focused == Some(id) && self.model.live.is_none();
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| terminal::paint(&screen, bounds, cell, has_keys, window, cx),
            )
            .absolute()
            .size_full()
            .into_any_element()
        } else {
            let hint = match pane.kind {
                Kind::Terminal => "the shell did not start",
                _ => "empty: pick an app, or ⌘D / ⌘⇧D for a terminal",
            };
            div().text_sm().text_color(rgb(0x7f849c)).child(hint).into_any_element()
        };
        let radius = self.hosted.get(&id).map_or(0.0, |h| h.radius);
        let body = div()
            .id(("pane", id))
            .relative()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            // Record the body's layout bounds: AX matches the real window to it; e2e reads it.
            .child(canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(content)
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, _| paint_corner_mask(bounds, radius, window))
                    .absolute()
                    .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.click_pane(id, e, window, cx)),
            );

        // While another pane is live, fade this one: input goes to the live app, not here.
        let faded = self.model.live.is_some_and(|live| live != id);
        let focused = self.model.focused == Some(id);
        div()
            .flex_1()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(if focused { 0x89b4fa } else { 0x313244 }))
            .when(faded, |d| d.opacity(0.5))
            .child(header)
            .child(body)
    }

    /// Leaves render as panes; splits as a row (`Right`) or column (`Down`) of two equal halves.
    fn render_node(&self, node: &Node, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Leaf(id) => match self.model.pane(*id) {
                Some(pane) => self.render_pane(pane, cx).into_any_element(),
                None => div().into_any_element(),
            },
            Node::Split { axis, a, b } => div()
                .flex_1()
                .size_full()
                .overflow_hidden()
                .flex()
                .when(*axis == Axis::Down, |d| d.flex_col())
                .child(self.render_node(a, cx))
                .child(self.render_node(b, cx))
                .into_any_element(),
        }
    }

    /// Bottom bar: who has the keyboard, and the focused pane.
    fn render_status(&self) -> impl IntoElement {
        let (mode, color, hint) = match self.model.mode() {
            Mode::Normal => ("NORMAL", 0x89b4fa, "⌘D split right · ⌘⇧D split down · ⌘W close · ⌘⌥arrows focus"),
            Mode::App => ("APP", 0xfab387, "keys go to the app · click ottospace to return"),
        };
        let focused = match self.model.focused.and_then(|id| self.model.pane(id)).map(|p| &p.kind) {
            Some(Kind::App { title, .. }) => title.clone(),
            Some(Kind::Empty) => "empty pane".into(),
            Some(Kind::Terminal) => "shell".into(),
            None => "no pane".into(),
        };
        div()
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .bg(rgb(0x11111b))
            .border_t_1()
            .border_color(rgb(0x313244))
            .text_xs()
            .text_color(rgb(0xa6adc8))
            .child(div().px_2().rounded_sm().bg(rgb(color)).text_color(rgb(0x11111b)).child(mode))
            .child(focused)
            .child(hint)
    }
}

/// Covers the window's rounded corners (transparent, so black in the YUV stream) with the
/// background: a ring whose inner edge is the window's rounded outline, clipped to the pane.
fn paint_corner_mask(bounds: Bounds<Pixels>, radius: f64, window: &mut Window) {
    if radius <= 0.0 {
        return;
    }
    let r = px(radius as f32);
    let ring = Bounds { origin: bounds.origin - point(r, r), size: bounds.size + size(r * 2., r * 2.) };
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        window.paint_quad(quad(ring, r * 2., transparent_black(), r, rgb(BG), BorderStyle::Solid));
    });
}

/// A pane body's rect (window-content coordinates) in global top-left screen coordinates.
fn on_screen(body: Bounds<Pixels>, window: &Window, cx: &App) -> CGRect {
    // gpui window bounds are relative to the window's display; offset by its global origin.
    let screen = window.display(cx).map_or(CGPoint::ZERO, |d| display::bounds(d.id().into()).origin);
    let frame = window.bounds();
    let titlebar = frame.size.height - window.viewport_size().height;
    let x = screen.x + f32::from(frame.origin.x + body.origin.x) as f64;
    let y = screen.y + f32::from(frame.origin.y + titlebar + body.origin.y) as f64;
    CGRect { origin: CGPoint::new(x, y), size: to_cg(body.size) }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for pane in &self.model.panes {
            self.cells.entry(pane.id).or_default();
        }
        let tree = self.model.layout.as_ref().map(|root| self.render_node(root, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(0xcdd6f4))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                this.dispatch(Action::Split { axis: Axis::Right, kind: Kind::Terminal }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                this.dispatch(Action::Split { axis: Axis::Down, kind: Kind::Terminal }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ClosePane, window, cx| {
                this.dispatch(Action::Close { pane: None }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusLeft, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Left }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusRight, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Right }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusUp, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Up }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusDown, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Down }, window, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .flex()
                    .child(
                        div()
                            .h_full()
                            .when(self.model.live.is_some(), |d| d.opacity(0.5))
                            .child(self.render_sidebar(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .when(tree.is_none(), |d| {
                                d.items_center()
                                    .justify_center()
                                    .child("⌘D for a pane, or pick an app (shift-click splits down)")
                            })
                            .children(tree),
                    ),
            )
            .child(self.render_status())
    }
}

/// `ottospace ctl '<json request>'`: sends one request to the running app, prints the reply.
fn ctl(args: &[String]) -> ! {
    let Some(request) = args.first() else {
        eprintln!("usage: ottospace ctl '{{\"cmd\":\"state\"}}'");
        std::process::exit(2);
    };
    match control::send(&control::path(), request) {
        Ok(reply) => {
            println!("{reply}");
            std::process::exit(0);
        }
        Err(err) => {
            eprintln!("no ottospace at {}: {err}", control::path().display());
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "ctl") {
        ctl(&args[2..]);
    }

    // E2E runs get no window hosting, so tests cannot move or strand the user's real windows.
    let hosting = std::env::var_os("OTTOSPACE_E2E").is_none();
    if hosting {
        capture::init();
        if !ax::is_trusted(true) {
            eprintln!("grant Accessibility access to your terminal, then rerun");
            std::process::exit(1);
        }
        recovery::restore_leftovers();
    }
    let control = control::serve().map_err(|err| eprintln!("control socket off: {err}")).ok();

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("cmd-d", SplitRight, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-w", ClosePane, None),
            KeyBinding::new("cmd-alt-left", FocusLeft, None),
            KeyBinding::new("cmd-alt-right", FocusRight, None),
            KeyBinding::new("cmd-alt-up", FocusUp, None),
            KeyBinding::new("cmd-alt-down", FocusDown, None),
        ]);
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(hosting, control, window, cx)))
            .expect("open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        if hosting {
            cx.activate(true);
        }
    });
}
