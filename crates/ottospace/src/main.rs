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
    AnyElement, AnyView, App, Application, AssetSource, BorderStyle, Bounds, BoxShadow, ClickEvent, ContentMask,
    Context, FocusHandle, ImageSource, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Resource,
    SharedString, Size, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, black, canvas,
    div, img, point, prelude::*, px, quad, rgb, size, surface, svg, transparent_black,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_core::keys::{self, Mods};
use ottospace_core::{
    Action, Axis, Dir, Effect, Kind, Mode, Model, Node, Pane, PaneId, PickItem, Picker, Section, theme,
};
use ottospace_mac::{ax, capture, display, hotkey, input, recovery};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use screencapturekit::prelude::*;

mod control;
mod terminal;
use control::{Call, PaneKind, Request};
use terminal::Terminal;

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

    fn kind(&self) -> Kind {
        Kind::App { window: self.window_id, title: self.title.clone() }
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
            // Move first. Resized where it stood, macOS clamps the window to what fits above the
            // Dock there (a pane 818pt tall parked at 805pt), and the stream no longer matches.
            let _ = self.window.set_position(display::parking_spot());
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

/// Icons and images, compiled into the binary.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "otto.svg" => Some(include_bytes!("../assets/otto.svg")),
            "icons/apps.svg" => Some(include_bytes!("../assets/icons/apps.svg")),
            "icons/workbook.svg" => Some(include_bytes!("../assets/icons/workbook.svg")),
            "icons/tasks.svg" => Some(include_bytes!("../assets/icons/tasks.svg")),
            "icons/repos.svg" => Some(include_bytes!("../assets/icons/repos.svg")),
            "icons/split-right.svg" => Some(include_bytes!("../assets/icons/split-right.svg")),
            "icons/split-down.svg" => Some(include_bytes!("../assets/icons/split-down.svg")),
            "icons/plus.svg" => Some(include_bytes!("../assets/icons/plus.svg")),
            _ => None,
        };
        Ok(bytes.map(Into::into))
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

/// The return key's name (e.g. "F18") once it is registered. Unset: there is no return key.
static RETURN_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Height of the top bar (the native title strip, so all of it drags the window), and the width
/// it keeps clear on the left for the traffic lights.
const TOP_BAR: f32 = 28.;
const TRAFFIC_LIGHTS: f32 = 80.;

/// Top-bar buttons: icon, tooltip, split direction, and whether the pick opens in a new tab.
const SPLITS: [(&str, &str, Axis, bool); 3] = [
    ("icons/plus.svg", "New tab…  ⌘T", Axis::Right, true),
    ("icons/split-right.svg", "Split right…  ⌘E", Axis::Right, false),
    ("icons/split-down.svg", "Split down…", Axis::Down, false),
];

/// The key reference, by group: keys and what they do. Keep in step with the bindings in `main`.
const KEYS_PANES: &[(&str, &str)] = &[
    ("⌘E", "Pick what opens in a new pane"),
    ("⌘D", "Terminal to the right"),
    ("⌘⇧D", "Terminal below"),
    ("⌘W", "Close the pane"),
];
const KEYS_MOVE: &[(&str, &str)] = &[
    ("⌘] / ⌘[", "Next / previous pane"),
    ("⌘⌥ arrows", "Pane in that direction"),
    ("⌘⌥ H J K L", "The same, on the home row"),
];
const KEYS_TABS: &[(&str, &str)] = &[
    ("⌘T", "New tab: pick what opens in it"),
    ("⌘1 … ⌘9", "Go to a tab"),
    ("⌘⇧] / ⌘⇧[", "Next / previous tab"),
    ("⌃Tab / ⌃⇧Tab", "Next / previous tab"),
    ("⌘⇧W", "Close the tab and its panes"),
];

/// Rail sections in order, with their icon and label.
const SECTIONS: [(Section, &str, &str); 4] = [
    (Section::Apps, "icons/apps.svg", "Apps"),
    (Section::Workbook, "icons/workbook.svg", "Workbook"),
    (Section::Tasks, "icons/tasks.svg", "Tasks"),
    (Section::Repos, "icons/repos.svg", "Repos"),
];

/// A small label shown when the pointer rests on a rail icon.
struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(theme::SELECTED))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .text_xs()
            .text_color(rgb(theme::TEXT))
            .child(self.0.clone())
    }
}

/// Otto with a hint under it: what an empty pane or an empty workspace shows.
fn otto(size: f32, hint: &'static str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        // Name the source kind: a bare "otto.svg" parses as a URL and is fetched over HTTP.
        .child(img(ImageSource::Resource(Resource::Embedded("otto.svg".into()))).size(px(size)).opacity(0.3))
        .child(div().text_sm().text_color(rgb(theme::MUTED)).child(hint))
}

actions!(
    ottospace,
    [
        SplitRight, SplitDown, PickPane, ClosePane, FocusLeft, FocusRight, FocusUp, FocusDown, FocusNext, FocusPrev,
        Zoom, NewTab, NextTab, PrevTab, CloseTab, ShowKeys
    ]
);

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
    /// Test runs: the pid of the one app that may be hosted.
    host_only: Option<i32>,
    control: Option<Receiver<Call>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    fn new(
        hosting: bool,
        host_only: Option<i32>,
        control: Option<Receiver<Call>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
                    if hotkey::take_press() {
                        this.return_to_ottospace(window, cx);
                    }
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

        // Native fullscreen would move ottospace to its own Space, where other apps' windows
        // cannot follow. Make the green button zoom instead.
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
        {
            unsafe { ottospace_mac::window::zoom_instead_of_fullscreen(appkit.ns_view.as_ptr()) };
            // A run with hosting off never activates the app, so keep its window visible another
            // way. Not with hosting on: a floating ottospace would cover the live app window.
            if !hosting {
                unsafe { ottospace_mac::window::keep_on_top(appkit.ns_view.as_ptr()) };
            }
        }

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut model = Model::default();
        if hosting {
            // Apps is the way in while there are no panes.
            model.update(Action::ToggleSection { section: Section::Apps });
        }
        Self {
            model,
            hosted: HashMap::new(),
            terminals: HashMap::new(),
            cell: terminal::cell_size(window),
            cells: HashMap::new(),
            apps: running_apps(hosting, host_only),
            known: Vec::new(),
            live_since: None,
            error: None,
            hosting,
            host_only,
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
                        // Going live activated the app. If it still has the keyboard, take it
                        // back, or keys keep going to its off-screen window while the status
                        // bar says NORMAL. If the user moved to a third app, leave them there.
                        if ax::is_active(hosted.pid) {
                            cx.activate(true);
                        }
                    }
                }
                Effect::HostApp { name, axis, new_tab } => {
                    let _ = self.open_app(name, *axis, *new_tab, window, cx);
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

    /// The global return key: take the keyboard back from a live app, and bring ottospace to the
    /// front from wherever the user is.
    fn return_to_ottospace(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::Leave, window, cx);
        cx.activate(true);
    }

    /// Answers queued control-socket calls. Each reply is the state after the call.
    fn serve_control(&mut self, window: &Window, cx: &mut Context<Self>) {
        let calls: Vec<Call> = self.control.iter().flat_map(|rx| rx.try_iter()).collect();
        for Call { request, reply } in calls {
            let quit = matches!(request, Request::Quit);
            let mut failure: Option<String> = None;
            let action = match request {
                Request::State | Request::Quit => None,
                Request::OpenApp { name, axis } => {
                    failure = self.open_app(&name, axis, false, window, cx).err();
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
                        None => failure = Some("no terminal pane to type into".into()),
                    }
                    None
                }
                Request::Close { pane } => Some(Action::Close { pane }),
                Request::Focus { pane } => Some(Action::Focus { pane }),
                Request::FocusDir { dir } => Some(Action::FocusDir { dir }),
                Request::FocusCycle { delta } => Some(Action::FocusCycle { delta }),
                Request::Enter { pane } => Some(Action::Enter { pane }),
                Request::Leave => Some(Action::Leave),
                Request::Section { section } => Some(Action::ToggleSection { section }),
                Request::Picker { axis, new_tab } => {
                    self.open_picker(axis, new_tab, window, cx);
                    None
                }
                Request::NewTab => Some(Action::NewTab),
                Request::SelectTab { index } => Some(Action::SelectTab { index }),
                Request::CycleTab { delta } => Some(Action::CycleTab { delta }),
                Request::CloseTab { index } => Some(Action::CloseTab { index }),
                Request::PickerQuery { query } => Some(Action::PickerQuery { query }),
                Request::PickerMove { delta } => Some(Action::PickerMove { delta }),
                Request::PickerChoose { index } => Some(Action::PickerChoose { index }),
                Request::PickerClose => Some(Action::ClosePicker),
                Request::Keys => Some(Action::ToggleKeys),
            };
            if let Some(action) = action {
                let effects = self.dispatch(action, window, cx);
                // A terminal pane with no shell behind it: say so instead of replying ok.
                let dead = effects.iter().any(|e| match e {
                    Effect::Opened(id) => {
                        self.model.pane(*id).is_some_and(|p| p.kind == Kind::Terminal)
                            && !self.terminals.contains_key(id)
                    }
                    _ => false,
                });
                // A picked app that could not be hosted: `open_app` left the reason in `error`.
                let unhosted = effects.iter().any(|e| matches!(e, Effect::HostApp { .. })) && self.error.is_some();
                if dead || unhosted {
                    failure = self.error.clone().or(Some("the shell did not start".into()));
                }
            }
            let _ = reply.send(failure.map_or_else(|| self.snapshot(window), |message| control::error(&message)));
            if quit {
                // Quit a moment later, so the connection thread can write the reply first.
                cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
            }
        }
    }

    /// State as JSON: the model, plus each pane's drawn bounds `[x, y, w, h]` in window points.
    fn snapshot(&self, window: &Window) -> String {
        let parking = display::parking_spot();
        let hosted: BTreeMap<PaneId, [f64; 4]> = self
            .hosted
            .iter()
            .filter_map(|(id, h)| {
                h.window.frame().map(|f| (*id, [f.origin.x, f.origin.y, f.size.width, f.size.height]))
            })
            .collect();
        let frame = window.bounds();
        let frame: [f32; 4] =
            [frame.origin.x.into(), frame.origin.y.into(), frame.size.width.into(), frame.size.height.into()];
        // Only panes of the active tab are drawn; a background pane's last bounds are stale.
        let drawn = self.model.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let bounds: BTreeMap<PaneId, [f32; 4]> = self
            .cells
            .iter()
            .filter(|(id, _)| drawn.contains(id))
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
            "tab_panes": self.model.tab_panes(),
            // ottospace's own window frame, so a test can turn pane bounds into screen positions.
            "window": frame,
            // Where parked windows go, so a test can tell "parked" from "moved somewhere".
            "parking": [parking.x, parking.y],
            // The registered return key, or null if there is none.
            "return_key": RETURN_KEY.get(),
            // Each hosted window's real frame, in the same global coordinates as `parking`.
            "hosted": hosted,
            "terminals": terminals,
        })
        .to_string()
    }

    /// Adds an app pane for an already-captured window.
    fn attach(&mut self, hosted: Hosted, action: Action, window: &Window, cx: &mut Context<Self>) {
        let pid = hosted.pid;
        if !self.known.iter().any(|(p, _)| *p == pid) {
            let ids = ax::Window::standard_of(pid).iter().filter_map(ax::Window::id).collect();
            self.known.push((pid, ids));
        }
        let effects = self.dispatch(action, window, cx);
        if let Some(Effect::Opened(id)) = effects.last() {
            // A pane in a background tab is not drawn, so the layout sync never parks its
            // window. Move it off screen now; it is sized when its tab is first shown.
            let drawn = self.model.layout.as_ref().is_some_and(|l| l.leaves().contains(id));
            if !drawn {
                let _ = hosted.window.set_position(display::parking_spot());
            }
            self.cells.insert(*id, hosted.bounds.clone());
            self.hosted.insert(*id, hosted);
        }
    }

    /// Hosts the app's main window in a pane. The error also shows in the Apps list.
    fn open_app(
        &mut self,
        name: &str,
        axis: Axis,
        new_tab: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let result = self.try_open_app(name, axis, new_tab, window, cx);
        self.error = result.clone().err();
        cx.notify();
        result
    }

    fn try_open_app(
        &mut self,
        name: &str,
        axis: Axis,
        new_tab: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.hosting {
            return Err("window hosting is off in e2e runs".into());
        }
        let target = match self.host_only {
            // By pid, not by name: another process may share the canary's name.
            Some(pid) if capture::name_of(pid).as_deref() == Some(name) => capture::find_pid(pid)?,
            Some(_) => return Err("this test run may host only the app it started".into()),
            None => capture::find(name)?,
        };
        // Already in a pane: go to it (and its tab) instead of doing nothing.
        if let Some((pane, _)) = self.hosted.iter().find(|(_, h)| h.window_id == target.id) {
            let pane = *pane;
            self.dispatch(Action::Focus { pane }, window, cx);
            return Ok(());
        }
        let hosted = Hosted::new(target)?;
        // The window is in hand, so the tab will not be left empty.
        if new_tab {
            self.dispatch(Action::NewTab, window, cx);
        }
        let action = Action::Open { kind: hosted.kind(), axis, beside: None };
        self.attach(hosted, action, window, cx);
        Ok(())
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
                // Its app may still be frontmost with no window here: take the keyboard back.
                if self.model.live == Some(id) && ax::is_active(hosted.pid) {
                    cx.activate(true);
                }
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
                let action = Action::Adopt { kind: hosted.kind(), beside };
                self.attach(hosted, action, window, cx);
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
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        // The key reference takes the keyboard: Esc closes it, and no other key reaches the
        // terminal or the picker behind it. (⌘ shortcuts go to their bindings; the core ignores
        // pane actions while it is open, and ⌘/ closes it.)
        if self.model.keys {
            if key.key == "escape" {
                self.dispatch(Action::CloseKeys, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        // ⌘1..⌘9 select a tab.
        if key.modifiers.platform
            && let Some(digit) = key.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d))
        {
            self.dispatch(Action::SelectTab { index: digit - 1 }, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.model.picker.is_some() {
            self.picker_key(event, window, cx);
            return;
        }
        let Some(terminal) = self.model.focused.and_then(|id| self.terminals.get(&id)) else { return };
        let key = &event.keystroke;
        let m = key.modifiers;
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, cmd: m.platform };
        if let Some(bytes) = keys::encode(&key.key, key.key_char.as_deref(), mods, terminal.app_cursor()) {
            terminal.write(bytes);
            cx.stop_propagation();
        }
    }

    /// Opens the pane picker: a terminal, then each running app.
    fn open_picker(&mut self, axis: Axis, new_tab: bool, window: &Window, cx: &mut Context<Self>) {
        self.apps = running_apps(self.hosting, self.host_only);
        let apps = self.apps.iter().map(|name| PickItem::App { name: name.clone() });
        let items = std::iter::once(PickItem::Terminal).chain(apps).collect();
        self.dispatch(Action::OpenPicker { axis, new_tab, items }, window, cx);
    }

    /// While the picker is open it takes every key: text filters, arrows move, enter opens.
    fn picker_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        let action = match key.key.as_str() {
            _ if m.platform => return, // ⌘ shortcuts still reach their bindings
            "escape" => Action::ClosePicker,
            "enter" => Action::PickerChoose { index: None },
            "down" | "tab" => Action::PickerMove { delta: 1 },
            "up" => Action::PickerMove { delta: -1 },
            "n" if m.control => Action::PickerMove { delta: 1 },
            "p" if m.control => Action::PickerMove { delta: -1 },
            "backspace" => Action::PickerBackspace,
            // ⌥ composes other characters (⌥B is "∫"); only plain typing filters.
            _ => match key.key_char.as_deref().filter(|_| !m.control && !m.alt) {
                Some(text) if !text.chars().any(char::is_control) => Action::PickerType { text: text.into() },
                _ => return,
            },
        };
        self.dispatch(action, window, cx);
        cx.stop_propagation();
    }

    /// The key reference: every shortcut, grouped. ⌘/ or Esc or a click closes it.
    fn render_keys(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let return_key = RETURN_KEY.get().cloned().unwrap_or_else(|| "off".into());
        let group = |title: &'static str, rows: Vec<(String, &'static str)>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(theme::PRIMARY)).child(title))
                .children(rows.into_iter().map(|(keys, what)| {
                    div()
                        .flex()
                        .gap_3()
                        .text_sm()
                        .child(div().w(px(150.)).text_color(rgb(theme::TEXT)).child(keys))
                        .child(div().text_color(rgb(theme::MUTED)).child(what))
                }))
        };
        let rows = |list: &[(&str, &'static str)]| list.iter().map(|(k, w)| (k.to_string(), *w)).collect::<Vec<_>>();
        let panel = div()
            .p_4()
            .flex()
            .gap_8()
            .rounded_lg()
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.7),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
            }])
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(group("PANES", rows(KEYS_PANES)))
                    .child(group("MOVE", rows(KEYS_MOVE))),
            )
            .child(div().flex().flex_col().gap_4().child(group("TABS", rows(KEYS_TABS))).child(group(
                "WINDOW",
                vec![
                    (return_key, "From any app: back to ottospace"),
                    ("⌘⌃F".into(), "Zoom the window"),
                    ("⌘/".into(), "This list"),
                ],
            )));
        div()
            .id("keys-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(black().opacity(0.5))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::CloseKeys, window, cx);
                }),
            )
            .child(panel)
    }

    /// The picker dropdown, under the split buttons. A click outside it closes it.
    fn render_picker(&self, picker: &Picker, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = picker.selected;
        let rows = picker.matches().into_iter().enumerate().map(|(i, item)| {
            let (tag, color) = match item {
                PickItem::Terminal => ("TERM", theme::PRIMARY),
                PickItem::App { .. } => ("APP", theme::APP),
            };
            div()
                .id(("pick", i))
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .gap_3()
                .text_sm()
                .cursor_pointer()
                .when(i == selected, |d| d.bg(rgb(theme::SELECTED)))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(div().w(px(36.)).text_xs().text_color(rgb(color)).child(tag))
                .child(item.label().to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::PickerChoose { index: Some(i) }, window, cx);
                }))
        });
        let rows: Vec<_> = rows.collect();
        let title = match (picker.new_tab, picker.axis) {
            (true, _) => "New tab with",
            (false, Axis::Right) => "Split right with",
            (false, Axis::Down) => "Split down with",
        };
        let query = if picker.query.is_empty() { "type to filter".to_string() } else { picker.query.clone() };
        let panel = div()
            .absolute()
            .top(px(TOP_BAR + 6.))
            .right(px(8.))
            .w(px(320.))
            .max_h(px(420.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_lg()
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.7),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
            }])
            // A click inside the panel must not reach the backdrop, which closes the picker.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER))
                    .text_xs()
                    .child(div().text_color(rgb(theme::PICK)).child(title))
                    .child(
                        div()
                            .text_color(rgb(if picker.query.is_empty() { theme::MUTED } else { theme::TEXT }))
                            .child(query),
                    ),
            )
            .when(rows.is_empty(), |d| d.child(div().p_3().text_xs().text_color(rgb(theme::MUTED)).child("No match")))
            // Long lists scroll inside the panel.
            .child(div().id("picker-rows").flex_1().min_h_0().overflow_y_scroll().children(rows));
        div()
            .id("picker-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::ClosePicker, window, cx);
                }),
            )
            .child(panel)
    }

    /// A click on a pane body. An app pane goes live, then the click is replayed so it lands in
    /// the app (otherwise the first click is spent on the swap). Other panes just take focus.
    fn click_pane(&mut self, id: PaneId, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        if !self.hosted.contains_key(&id) {
            self.dispatch(Action::Focus { pane: id }, window, cx);
            return;
        }
        // Already live: the real window takes its own clicks. Replaying one here would land on
        // ottospace again and loop.
        if self.model.live == Some(id) {
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

    /// Left rail: one icon per section. The open section's icon is raised.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("rail")
            .w(px(56.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .py_2()
            .border_r_1()
            .border_color(rgb(theme::BORDER))
            .children(SECTIONS.iter().enumerate().map(|(i, (section, icon, label))| {
                let (section, label) = (*section, *label);
                let active = self.model.sidebar == Some(section);
                div()
                    .id(("rail", i))
                    .size(px(44.))
                    .rounded_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)))
                    .when(active, |d| {
                        d.bg(rgb(theme::SELECTED)).shadow(vec![BoxShadow {
                            color: black().opacity(0.7),
                            offset: point(px(0.), px(3.)),
                            blur_radius: px(10.),
                            spread_radius: px(0.),
                        }])
                    })
                    .child(svg().path(*icon).size(px(20.)).text_color(rgb(if active {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    })))
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(label.into()))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(Action::ToggleSection { section }, window, cx);
                    }))
            }))
    }

    /// Secondary bar: the open section's list. Closable.
    fn render_bar(&self, section: Section, cx: &mut Context<Self>) -> impl IntoElement {
        let title = SECTIONS.iter().find(|(s, ..)| *s == section).map_or("", |(_, _, label)| label);
        let header = div()
            .h(px(40.))
            .pl_3()
            .pr_1()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child(title.to_uppercase())
            .child(
                div()
                    .id("bar-close")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                    .child("×")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::CloseSidebar, window, cx);
                    })),
            );
        let body = match section {
            Section::Apps => self.render_apps(cx).into_any_element(),
            _ => {
                div().p_3().text_xs().text_color(rgb(theme::MUTED)).child("Designed, not built yet.").into_any_element()
            }
        };
        div()
            .id("bar")
            .w(px(260.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::RAISED))
            .border_r_1()
            .border_color(rgb(theme::BORDER))
            // Clicking ottospace's own UI hands the live pane back to its stream.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::Leave, window, cx);
                }),
            )
            .child(header)
            .child(body)
    }

    /// Running apps. A click hosts the app's main window in a pane.
    fn render_apps(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("apps")
            .flex_1()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .overflow_y_scroll()
            .child(
                div()
                    .id("refresh")
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child("Running  ↻")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.apps = running_apps(this.hosting, this.host_only);
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
                    .hover(|s| s.bg(rgb(theme::SELECTED)))
                    .child(name.clone())
                    // Fills the focused pane if it is empty. Else splits it: right, or down with shift.
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                        let _ = this.open_app(&target, axis, false, window, cx);
                    }))
            }))
    }

    fn render_pane(&self, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement {
        let id = pane.id;
        let cell = self.cells.get(&id).cloned().unwrap_or_default();
        let (tag, tag_color, title) = match &pane.kind {
            Kind::Empty => ("EMPTY", theme::MUTED, String::new()),
            Kind::Terminal => ("TERM", theme::PRIMARY, "shell".into()),
            Kind::App { title, .. } => ("APP", theme::APP, title.clone()),
        };
        let header = div()
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(theme::RAISED))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
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
                    .hover(|s| s.text_color(rgb(theme::ERROR)))
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
            let has_keys = self.model.focused == Some(id) && self.model.mode() == Mode::Normal && !self.model.keys;
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| terminal::paint(&screen, bounds, cell, has_keys, window, cx),
            )
            .absolute()
            .size_full()
            .into_any_element()
        } else {
            match pane.kind {
                Kind::Terminal => {
                    div().text_sm().text_color(rgb(theme::ERROR)).child("the shell did not start").into_any_element()
                }
                _ => otto(110., "⌘E to pick what goes here").into_any_element(),
            }
        };
        let radius = self.hosted.get(&id).map_or(0.0, |h| h.radius);
        // The content sits 2pt inside the pane. A live app window is placed over it, and without
        // the gap that window's own edge covers the pane's accent border.
        let inner = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            // Record the content's layout bounds: AX matches the real window to it; e2e reads it.
            .child(canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(content)
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, _| paint_corner_mask(bounds, radius, window))
                    .absolute()
                    .size_full(),
            );
        // The click handler is on the content, not the padded body: a click in the 2pt gap would be
        // replayed at a point the app window does not cover, land on ottospace again, and loop.
        let inner = inner.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| this.click_pane(id, e, window, cx)),
        );
        let body = div().id(("pane", id)).flex_1().min_h_0().p(px(2.)).child(inner);

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
            .border_color(rgb(if focused { theme::accent(self.model.mode()) } else { theme::BORDER }))
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

    /// Top bar, drawn where the system title bar was: tabs on the left, new-tab and split
    /// buttons on the right.
    fn render_top(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(TOP_BAR))
            .pl(px(TRAFFIC_LIGHTS))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.dispatch(Action::Leave, window, cx);
                    // With a custom title bar, the system no longer zooms on double-click.
                    if e.click_count == 2 {
                        window.titlebar_double_click();
                    }
                }),
            )
            .children(self.model.tabs.iter().enumerate().map(|(index, tab)| {
                let active = index == self.model.active;
                div()
                    .id(("tab", tab.id))
                    .h_full()
                    .pl_3()
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_r_1()
                    .border_color(rgb(theme::BORDER))
                    .cursor_pointer()
                    .when(active, |d| d.bg(rgb(theme::RAISED)).text_color(rgb(theme::TEXT)))
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child(tab.title.clone())
                    .child(
                        div()
                            .id(("tab-close", tab.id))
                            .px_1()
                            .rounded_sm()
                            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::ERROR)))
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // Do not also select the tab that is going away.
                                cx.stop_propagation();
                                this.dispatch(Action::CloseTab { index: Some(index) }, window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(Action::SelectTab { index }, window, cx);
                    }))
            }))
            // New tab and split buttons: each opens the picker for that target.
            .child(div().ml_auto().mr_2().flex().gap_1().children(SPLITS.iter().enumerate().map(
                |(i, (icon, tip, axis, new_tab))| {
                    let (tip, axis, new_tab) = (*tip, *axis, *new_tab);
                    div()
                        .id(("split", i))
                        .w(px(28.))
                        .h(px(22.))
                        .rounded_md()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(theme::SELECTED)))
                        .child(svg().path(*icon).size(px(16.)).text_color(rgb(theme::MUTED)))
                        .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_picker(axis, new_tab, window, cx)))
                },
            )))
    }

    /// Bottom bar: who has the keyboard, and the focused pane.
    fn render_status(&self) -> impl IntoElement {
        let color = theme::accent(self.model.mode());
        // Name the return key only if it works: a wrong hint strands the user in the app.
        let app_hint = match RETURN_KEY.get() {
            Some(key) => format!("keys go to the app · {key} or a click on ottospace returns"),
            None => "keys go to the app · click ottospace to return".to_string(),
        };
        let (mode, hint) = match self.model.mode() {
            // One hint, not the whole key list: ⌘/ shows the rest.
            Mode::Normal => ("NORMAL", "⌘/ keys"),
            Mode::Pick => ("PICK", "type to filter · ↑↓ move · ↵ open · esc close"),
            Mode::App => ("APP", app_hint.as_str()),
        };
        let hint = hint.to_string();
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
            .bg(rgb(theme::BACKGROUND))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child(div().px_2().rounded_sm().bg(rgb(color)).text_color(rgb(theme::BACKGROUND)).child(mode))
            .child(focused)
            .child(hint)
            // Shown here, not in the Apps list, so it is visible whichever section is open.
            .children(self.error.clone().map(|e| div().ml_auto().text_color(rgb(theme::ERROR)).child(e)))
    }
}

/// Apps that can be hosted right now. In a test run limited to one app, only that app.
fn running_apps(hosting: bool, only: Option<i32>) -> Vec<String> {
    match (hosting, only) {
        (false, _) => Vec::new(),
        (true, Some(pid)) => capture::name_of(pid).into_iter().collect(),
        (true, None) => capture::apps(),
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
        window.paint_quad(quad(ring, r * 2., transparent_black(), r, rgb(theme::BACKGROUND), BorderStyle::Solid));
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
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                this.dispatch(Action::Split { axis: Axis::Right, kind: Kind::Terminal }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                this.dispatch(Action::Split { axis: Axis::Down, kind: Kind::Terminal }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PickPane, window, cx| {
                if this.model.picker.is_some() {
                    this.dispatch(Action::ClosePicker, window, cx);
                } else {
                    this.open_picker(Axis::Right, false, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| {
                this.dispatch(Action::FocusCycle { delta: 1 }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPrev, window, cx| {
                this.dispatch(Action::FocusCycle { delta: -1 }, window, cx);
            }))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            .on_action(cx.listener(|this, _: &ShowKeys, window, cx| {
                this.dispatch(Action::ToggleKeys, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.open_picker(Axis::Right, true, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.dispatch(Action::CycleTab { delta: 1 }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                this.dispatch(Action::CycleTab { delta: -1 }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.dispatch(Action::CloseTab { index: None }, window, cx);
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
            .child(self.render_top(cx))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .flex()
                    // While an app has the keyboard, fade ottospace's own chrome.
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .when(self.model.live.is_some(), |d| d.opacity(0.5))
                            .child(self.render_rail(cx))
                            .children(self.model.sidebar.map(|section| self.render_bar(section, cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .when(tree.is_none(), |d| {
                                d.items_center()
                                    .justify_center()
                                    .child(otto(220., "⌘E to open a pane · ⌘D for a terminal"))
                            })
                            .children(tree),
                    ),
            )
            .child(self.render_status())
            .children(self.model.picker.as_ref().map(|picker| self.render_picker(picker, cx)))
            .when(self.model.keys, |d| d.child(self.render_keys(cx)))
    }
}

/// `ottospace ctl '<json request>'`: sends one request to the running app, prints the reply.
fn ctl(args: &[String]) -> ! {
    let Some(request) = args.first() else {
        eprintln!("usage: ottospace ctl '{{\"cmd\":\"state\"}}'");
        std::process::exit(2);
    };
    let reply = control::send(&control::path(), request).unwrap_or_else(|err| {
        eprintln!("no ottospace at {}: {err}", control::path().display());
        std::process::exit(1);
    });
    println!("{reply}");
    // Exit 0 only when the app accepted the request, so scripts can test the result.
    let ok = serde_json::from_str::<serde_json::Value>(&reply).is_ok_and(|v| v["ok"] == true);
    std::process::exit(if ok { 0 } else { 1 });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "ctl") {
        ctl(&args[2..]);
    }

    // E2E runs get no window hosting, so tests cannot move or strand the user's real windows.
    // The exception is `OTTOSPACE_HOST_ONLY=<pid>`: hosting is on for that one process only (the
    // canary the test started), which lets tests cover real window handling.
    let host_only: Option<i32> = std::env::var("OTTOSPACE_HOST_ONLY").ok().and_then(|pid| pid.parse().ok());
    let hosting = std::env::var_os("OTTOSPACE_E2E").is_none() || host_only.is_some();
    if hosting {
        capture::init();
        // A test must not raise a system permission prompt: fail instead.
        if !ax::is_trusted(host_only.is_none()) {
            eprintln!("grant Accessibility access to your terminal, then rerun");
            std::process::exit(1);
        }
        // The recovery file belongs to the user's real session. A test run must not move its
        // leftovers, nor write to it.
        match host_only {
            None => recovery::restore_leftovers(),
            Some(_) => recovery::disable(),
        }
    }
    let control = control::serve().map_err(|err| eprintln!("control socket off: {err}")).ok();

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("cmd-d", SplitRight, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-e", PickPane, None),
            KeyBinding::new("cmd-/", ShowKeys, None),
            KeyBinding::new("cmd-w", ClosePane, None),
            KeyBinding::new("cmd-alt-left", FocusLeft, None),
            KeyBinding::new("cmd-alt-right", FocusRight, None),
            KeyBinding::new("cmd-alt-up", FocusUp, None),
            KeyBinding::new("cmd-alt-down", FocusDown, None),
            // The same moves on the home row, and a cycle through every pane.
            KeyBinding::new("cmd-alt-h", FocusLeft, None),
            KeyBinding::new("cmd-alt-l", FocusRight, None),
            KeyBinding::new("cmd-alt-k", FocusUp, None),
            KeyBinding::new("cmd-alt-j", FocusDown, None),
            KeyBinding::new("cmd-]", FocusNext, None),
            KeyBinding::new("cmd-[", FocusPrev, None),
            // Fill the screen without leaving the Space (see `zoom_instead_of_fullscreen`).
            KeyBinding::new("cmd-ctrl-f", Zoom, None),
            // Tabs. ⌘1..⌘9 select one (handled in `key_down`).
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-}", NextTab, None),
            KeyBinding::new("cmd-{", PrevTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("cmd-shift-w", CloseTab, None),
        ]);
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        // Draw under the title bar; the top bar leaves room for the traffic lights.
        let titlebar = TitlebarOptions {
            title: Some("ottospace".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(10.), px(7.))),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(titlebar),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(hosting, host_only, control, window, cx)))
            .expect("open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        if hosting {
            cx.activate(true);
            // The return key works from any app, so it is only useful with hosting on.
            let name = std::env::var("OTTOSPACE_RETURN_KEY").unwrap_or_else(|_| "f18".into());
            let registered = hotkey::key_code(&name)
                .ok_or_else(|| "it is not one of f13-f20".to_string())
                .and_then(hotkey::register);
            match registered {
                Ok(()) => drop(RETURN_KEY.set(name.to_uppercase())),
                Err(err) => eprintln!("return key {name} is off: {err}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing asset draws as nothing, with no error. Catch a renamed file here.
    #[test]
    fn every_rail_icon_and_otto_is_embedded() {
        let icons = SECTIONS.iter().map(|(_, icon, _)| *icon).chain(SPLITS.iter().map(|(icon, ..)| *icon));
        for path in icons.chain(["otto.svg"]) {
            assert!(Assets.load(path).unwrap().is_some(), "{path} is not in Assets::load");
        }
    }
}
