//! Step 2: a sidebar of running apps and a row of panes, each hosting one window. Unfocused, a
//! pane shows its parked window's SCK stream. Clicking a pane moves its real window over it
//! ("live") and replays the click; clicking ottospace itself parks it again.
//! New windows of a hosted app (e.g. undocked DevTools) open in their own pane.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_video::pixel_buffer::{CVPixelBuffer, CVPixelBufferRef};
use gpui::{
    App, Application, Bounds, Context, MouseButton, MouseDownEvent, Pixels, Subscription, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, rgb, size, surface,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_mac::{ax, capture, display, input, recovery};
use screencapturekit::prelude::*;

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

/// One hosted window: its stream, input target and parking state.
struct Pane {
    id: u32,
    title: String,
    window: ax::Window,
    original: CGRect,
    latest: Arc<Latest>,
    stream: SCStream,
    pid: i32,
    /// Body bounds in window-content coordinates, captured during prepaint.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Size the window was parked at, to re-park only when the pane resizes.
    parked_size: CGSize,
    /// Last rect the live window was placed at, to re-place only on change.
    placed: CGRect,
}

impl Pane {
    fn new(target: capture::Target) -> Result<Self, String> {
        let capture::Target { window, sc_window, id } = target;
        let original = window.frame().ok_or("could not read window frame")?;
        let latest = Arc::new(Latest::default());
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
            id,
            title,
            window,
            original,
            latest,
            stream,
            pid,
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
        recovery::forget(self.id);
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

struct Workspace {
    apps: Vec<String>,
    panes: Vec<Pane>,
    /// Window ids seen per hosted pid; anything new that appears later gets its own pane.
    known: Vec<(i32, HashSet<u32>)>,
    /// Pane (by window id) whose real window sits over it, and when it went live.
    live: Option<(u32, Instant)>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            // Clicking ottospace itself (title bar, empty space) raises it over the live window,
            // so hand that pane back to its stream. Ignore the activation that the swap click
            // itself delivers late (the race that used to bounce windows straight back).
            cx.observe_window_activation(window, |this, window, cx| {
                let settled = this.live.is_some_and(|(_, at)| at.elapsed() > Duration::from_millis(400));
                if window.is_window_active() && settled {
                    this.park_live(window);
                    cx.notify();
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
                this.panes.drain(..).for_each(Pane::release);
                async {}
            }),
        ];

        // ~60Hz tick: redraw on new frames and keep windows matched to pane layout.
        // Every ~500ms, also look for new windows of hosted apps.
        cx.spawn_in(window, async move |this, cx| {
            for tick in 0u64.. {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if tick % 30 == 0 {
                        this.adopt_new_windows();
                        this.drop_closed(cx);
                    }
                    this.sync_layout(window, cx, tick % 30 == 15);
                    if this.panes.iter().any(|p| p.latest.dirty.swap(false, Relaxed)) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            apps: capture::apps(),
            panes: Vec::new(),
            known: Vec::new(),
            live: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn pane(&self, id: u32) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    fn add(&mut self, target: Result<capture::Target, String>, cx: &mut Context<Self>) {
        match target.and_then(Pane::new) {
            Ok(pane) => {
                let pid = pane.pid;
                if !self.known.iter().any(|(p, _)| *p == pid) {
                    let ids = ax::Window::standard_of(pid).iter().filter_map(ax::Window::id).collect();
                    self.known.push((pid, ids));
                }
                self.panes.push(pane);
                self.error = None;
            }
            Err(err) => self.error = Some(err),
        }
        cx.notify();
    }

    fn select_app(&mut self, name: &str, cx: &mut Context<Self>) {
        let target = capture::find(name);
        if let Ok(t) = &target
            && self.pane(t.id).is_some()
        {
            return; // already hosted
        }
        self.add(target, cx);
    }

    fn close(&mut self, id: u32, cx: &mut Context<Self>) {
        if let Some(i) = self.panes.iter().position(|p| p.id == id) {
            self.panes.remove(i).release();
        }
        if self.live_id() == Some(id) {
            self.live = None;
        }
        cx.notify();
    }

    fn live_id(&self) -> Option<u32> {
        self.live.map(|(id, _)| id)
    }

    /// Removes panes whose window is gone (closed by the user or the app).
    fn drop_closed(&mut self, cx: &mut Context<Self>) {
        let gone: Vec<u32> = self.panes.iter().filter(|p| p.window.frame().is_none()).map(|p| p.id).collect();
        for id in gone {
            eprintln!("window {id} gone (AX frame unreadable); dropping its pane");
            if let Some(i) = self.panes.iter().position(|p| p.id == id) {
                let _ = self.panes.remove(i).stream.stop_capture();
                recovery::forget(id);
            }
            if self.live_id() == Some(id) {
                self.live = None;
            }
            cx.notify();
        }
    }

    /// Hosts windows that appeared since we started watching their app (e.g. undocked DevTools).
    fn adopt_new_windows(&mut self) {
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
        for window in fresh {
            if let Ok(pane) = capture::target(window).and_then(Pane::new) {
                self.panes.push(pane);
            }
        }
    }

    /// Parks panes whose size changed, and keeps the live window glued to its pane: re-places it
    /// when the pane moves, and every ~500ms (`check`) if the app or user moved it away.
    fn sync_layout(&mut self, window: &Window, cx: &App, check: bool) {
        let scale = window.scale_factor() as f64;
        let live = self.live_id();
        let rects: Vec<CGRect> = self.panes.iter().map(|p| pane_on_screen(p, window, cx)).collect();
        for (pane, rect) in self.panes.iter_mut().zip(rects) {
            if rect.size.width < 1.0 {
                continue; // not laid out yet
            }
            if live == Some(pane.id) {
                let drifted = check && pane.window.frame().is_some_and(|f| f.origin != rect.origin);
                if pane.placed != rect || drifted {
                    pane.place(rect);
                }
            } else if pane.parked_size != rect.size {
                pane.park(scale);
            }
        }
    }

    /// Moves the pane's real window over it, then replays the click that triggered the swap so
    /// it lands in the app (otherwise the first click is spent on the swap).
    fn go_live(&mut self, id: u32, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        self.park_live(window);
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == id) else { return };
        let rect = pane_on_screen(pane, window, cx);
        pane.place(rect);
        pane.window.raise();
        self.live = Some((id, Instant::now()));
        cx.notify();

        let local = event.position - pane.bounds.get().origin;
        let at = CGPoint::new(rect.origin.x + f32::from(local.x) as f64, rect.origin.y + f32::from(local.y) as f64);
        let clicks = event.click_count as i64;
        cx.spawn(async move |_, cx| {
            // Give the window server a moment to put the real window under the cursor.
            cx.background_executor().timer(Duration::from_millis(80)).await;
            input::click(at, clicks);
        })
        .detach();
    }

    fn park_live(&mut self, window: &Window) {
        let Some((id, _)) = self.live.take() else { return };
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == id) {
            pane.park(window.scale_factor() as f64);
        }
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
                    this.park_live(window);
                    cx.notify();
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
                        this.apps = capture::apps();
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
                    .on_click(cx.listener(move |this, _, _, cx| this.select_app(&target, cx)))
            }))
            .children(self.error.clone().map(|e| div().p_2().text_xs().text_color(rgb(0xf38ba8)).child(e)))
    }

    fn render_pane(&self, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement {
        let id = pane.id;
        let cell = pane.bounds.clone();
        // While live, keep showing the last frame under the real window, so any gap during a swap is invisible.
        let content = match pane.frame() {
            Some(buffer) => surface(buffer).size_full().into_any_element(),
            None => div().child("waiting for frames…").into_any_element(),
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
                cx.listener(|this, _, window, cx| {
                    this.park_live(window);
                    cx.notify();
                }),
            )
            .child(div().overflow_hidden().child(pane.title.clone()))
            .child(
                div()
                    .id(("close", id))
                    .px_1()
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(0xf38ba8)))
                    .child("×")
                    .on_click(cx.listener(move |this, _, _, cx| this.close(id, cx))),
            );
        let body = div()
            .id(("pane", id))
            .relative()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            // Record the body's layout bounds so AX can match the real window to it.
            .child(canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(content)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.go_live(id, e, window, cx)),
            );

        // While another pane is live, fade this one: input goes to the live app, not here.
        let faded = self.live_id().is_some_and(|live| live != id);
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(0x313244))
            .when(faded, |d| d.opacity(0.5))
            .child(header)
            .child(body)
    }
}

/// Pane body rect in global top-left screen coordinates.
fn pane_on_screen(pane: &Pane, window: &Window, cx: &App) -> CGRect {
    // gpui window bounds are relative to the window's display; offset by its global origin.
    let screen = window.display(cx).map_or(CGPoint::ZERO, |d| display::bounds(d.id().into()).origin);
    let frame = window.bounds();
    let titlebar = frame.size.height - window.viewport_size().height;
    let body = pane.bounds.get();
    let x = screen.x + f32::from(frame.origin.x + body.origin.x) as f64;
    let y = screen.y + f32::from(frame.origin.y + titlebar + body.origin.y) as f64;
    CGRect { origin: CGPoint::new(x, y), size: to_cg(body.size) }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panes: Vec<_> = self.panes.iter().map(|p| self.render_pane(p, cx).into_any_element()).collect();
        div()
            .size_full()
            .flex()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .child(div().h_full().when(self.live.is_some(), |d| d.opacity(0.5)).child(self.render_sidebar(cx)))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .when(panes.is_empty(), |d| d.items_center().justify_center().child("pick an app from the sidebar"))
                    .children(panes),
            )
    }
}

fn main() {
    capture::init();
    if !ax::is_trusted(true) {
        eprintln!("grant Accessibility access to your terminal, then rerun");
        std::process::exit(1);
    }
    recovery::restore_leftovers();

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
