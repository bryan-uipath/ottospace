//! Imperative shell: draws the workspace, hosts other apps' windows in panes, and turns input
//! (mouse, keys, control socket) into `ottospace_core` actions. State rules live in the core.
//!
//! An app pane shows its parked window's SCK stream. Clicking it moves the real window over the
//! pane ("live") and replays the click; clicking ottospace itself parks it again.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_video::pixel_buffer::{CVPixelBuffer, CVPixelBufferRef};
use gpui::{
    AnyElement, AnyView, App, Application, AssetSource, BorderStyle, Bounds, BoxShadow, ClickEvent, ClipboardItem,
    ContentMask, Context, Div, ElementId, FocusHandle, FontWeight, KeyBinding, KeyDownEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString,
    Size, Stateful, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, black, canvas, div,
    img, point, prelude::*, px, quad, relative, rgb, size, surface, svg, transparent_black,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_core::keys::{self, Mods};
use ottospace_core::list::{ListKey, list_key};
use ottospace_core::rules::{self, Facts};
use ottospace_core::settings::{Installed, closed_apps};
use ottospace_core::tasks::{Group, MAX_TITLE, Task, TaskId};
use ottospace_core::terminal::{CellPoint, VisualKey, cell_at, scroll_key, select_kind, visual_key};
use ottospace_core::{
    Action, Axis, Dir, Effect, Kind, Mode, Model, Node, Pane, PaneId, PickItem, Picker, Rect, Section, theme,
    titlebar::{self, TitleBar},
};
use ottospace_mac::{ax, capture, display, hotkey, icon, input, recovery};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use screencapturekit::prelude::*;

mod ax_debug;
mod config;
mod control;
mod home;
mod menus;
mod repos;
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
/// `crop` > 0: show only the window below its top `crop` points (the title bar), unscaled.
fn stream_config(size: CGSize, crop: f64, scale: f64) -> SCStreamConfiguration {
    // 420f is the only format gpui's surface renderer accepts.
    let config = SCStreamConfiguration::new()
        .with_width((size.width * scale).max(1.0) as u32)
        .with_height((size.height * scale).max(1.0) as u32)
        .with_scales_to_fit(true)
        .with_pixel_format(PixelFormat::YCbCr_420f)
        .with_fps(60);
    if crop > 0.0 {
        config.with_source_rect(screencapturekit::cg::CGRect::new(0.0, crop, size.width, size.height))
    } else {
        config
    }
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
    /// Height of the window's plain native title bar (0: none to crop), measured once.
    bar: f64,
    /// Title-bar crop the window was parked and the stream configured with.
    crop: f64,
    /// Body bounds in window-content coordinates, captured during prepaint.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Size the window was parked at, to re-park only when the pane resizes.
    parked_size: CGSize,
    /// Last pane content rect the live window was placed over, to re-place only on change.
    placed: CGRect,
}

impl Hosted {
    fn new(target: capture::Target) -> Result<Self, String> {
        let capture::Target { window, sc_window, id } = target;
        let original = window.frame().ok_or("could not read window frame")?;
        let latest = Arc::new(Latest::default());
        let radius = capture::corner_radius(&sc_window, 2.0).unwrap_or(0.0);
        let bar = title_bar(&window);
        eprintln!("hosting window {id}: corner radius {radius:.1}pt, title bar {bar}pt");
        let filter = SCContentFilter::create().with_window(&sc_window).build().map_err(|e| format!("{e:?}"))?;
        // Resized to the pane on park.
        let config = stream_config(original.size, 0.0, 2.0);
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
            bar,
            crop: 0.0,
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
    /// With a `crop`, the window is that much taller and the stream shows only the part below it.
    fn park(&mut self, scale: f64, crop: f64) {
        let size = to_cg(self.bounds.get().size);
        // Only resize/reconfigure when the pane changed; redoing it on every park flickers.
        if size != self.parked_size || crop != self.crop {
            // Move first. Resized where it stood, macOS clamps the window to what fits above the
            // Dock there (a pane 818pt tall parked at 805pt), and the stream no longer matches.
            let _ = self.window.set_position(display::parking_spot());
            let _ = self.window.set_size(CGSize::new(size.width, size.height + crop));
            // Match the stream to the pane so frames aren't letterboxed into a stale-sized buffer.
            let config = stream_config(size, crop, scale);
            self.latest.expect(&config);
            let _ = self.stream.update_configuration(&config);
        }
        let _ = self.window.set_position(display::parking_spot());
        self.parked_size = size;
        self.crop = crop;
        self.placed = CGRect::default();
    }

    /// Places the window over pane content `rect`, raised by `crop` so its title bar covers the header.
    fn place(&mut self, rect: CGRect, crop: f64) {
        let frame = raised(rect, crop);
        let _ = self.window.set_size(frame.size);
        let _ = self.window.set_position(frame.origin);
        self.placed = rect;
    }

    /// The crop for this window with its pane content at `rect`; see `titlebar::crop`.
    fn crop_at(&self, rect: CGRect, limit: f64) -> f64 {
        titlebar::crop(self.bar as f32, rect.origin.y as f32, limit as f32) as f64
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

/// What an agent needs to drive ottospace. Compiled in, so the guide matches the binary.
const SKILL_GUIDE: &str = include_str!("../skill/guide.md");
const SKILL_STUB: &str = include_str!("../skill/SKILL.md");

/// The return key's name (e.g. "F18") once it is registered. Unset: there is no return key.
static RETURN_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Height of the top bar (the native title strip, so all of it drags the window), and the width
/// it keeps clear on the left for the traffic lights.
const TOP_BAR: f32 = 28.;
const TRAFFIC_LIGHTS: f32 = 80.;
/// Height of a pane's header (label and close button).
const PANE_HEADER: f32 = 28.;

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
const KEYS_ARRANGE: &[(&str, &str)] = &[
    ("⌘⌃ arrows / H J K L", "Resize: move the split that way"),
    ("⌘⌃=", "Even out the splits"),
    ("⌘⇧⌥ arrows / H J K L", "Swap with the pane that way"),
    ("⌘⇧⌥1 … ⌘⇧⌥9", "Move the pane to a tab (US layout)"),
];
const KEYS_TABS: &[(&str, &str)] = &[
    ("⌘T", "New tab: pick what opens in it"),
    ("⌘1 … ⌘9", "Go to a tab"),
    ("⌘⇧] / ⌘⇧[", "Next / previous tab"),
    ("⌃Tab / ⌃⇧Tab", "Next / previous tab"),
    ("⌘⇧W", "Close the tab and its panes"),
];
const KEYS_TERMINAL: &[(&str, &str)] = &[
    ("Wheel / trackpad", "Scroll back through history"),
    ("⇧PageUp / ⇧PageDown", "Scroll a page"),
    ("⇧Home / ⇧End", "Top / bottom of history"),
    ("Drag / 2× / 3× click", "Select text, a word, a line"),
    ("⌘C", "Copy the selection"),
    ("⌘V", "Paste"),
    ("⌘⇧C", "Visual mode: select with the keyboard"),
    ("⌘⇧E", "List mode: keys for the bar (Apps, Tasks, Repos)"),
    ("⌘N", "New task: type a line, ↵ files it"),
    ("⌘⇧N", "Next task: show the tabs of the first that needs you"),
    ("⌘H", "Hide ottospace"),
];
const KEYS_LIST: &[(&str, &str)] = &[
    ("J K / ↓ ↑", "Move the selection"),
    ("↵ / ⇧↵", "Open the row / open it below"),
    ("Tab / ⇧Tab", "Next / previous section"),
    ("S", "Task: snooze one hour"),
    ("H", "Task: put on hold"),
    ("X", "Task: dismiss from the inbox"),
    ("U", "Task: undo the last of these"),
    ("Esc", "Leave"),
];
const KEYS_VISUAL: &[(&str, &str)] = &[
    ("H J K L / arrows", "Move the cursor"),
    ("W B E · 0 ^ $", "Word · line start, text, end"),
    ("⇧H ⇧M ⇧L", "Top, middle, bottom of the screen"),
    ("V / ⇧V", "Select from here / whole lines; again clears"),
    ("Y or ↵", "Copy and leave"),
    ("Esc", "Leave"),
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

actions!(
    ottospace,
    [
        SplitRight, SplitDown, PickPane, ClosePane, FocusLeft, FocusRight, FocusUp, FocusDown, FocusNext, FocusPrev,
        Zoom, NewTab, NextTab, PrevTab, CloseTab, ShowKeys, AxOverlay
    ]
);
// Pane arrangement.
actions!(ottospace, [ResizeLeft, ResizeRight, ResizeUp, ResizeDown, SwapLeft, SwapRight, SwapUp, SwapDown, Equalize]);
// Terminal selection.
actions!(ottospace, [CopySelection, PasteText, VisualMode, ListMode, NewTask, NextTask]);
// App menu.
actions!(ottospace, [Quit, Hide]);

struct Workspace {
    model: Model,
    /// Shell half of each app pane.
    hosted: HashMap<PaneId, Hosted>,
    /// Shell half of each terminal pane.
    terminals: HashMap<PaneId, Terminal>,
    /// One terminal character cell.
    cell: Size<Pixels>,
    /// The terminal pane a mouse drag is selecting in.
    dragging: Option<PaneId>,
    /// The last text copied. E2E runs use it as the clipboard, so tests never touch the user's.
    copied: Option<String>,
    system_clipboard: bool,
    /// Each pane's body bounds in window coordinates, recorded during prepaint.
    cells: HashMap<PaneId, Rc<Cell<Bounds<Pixels>>>>,
    /// Each pane's bounds inside its border, for e2e to check the live window's gap.
    frames: HashMap<PaneId, Rc<Cell<Bounds<Pixels>>>>,
    /// What the home screen last drew, and the lattice lines kept between frames.
    home: home::Shared,
    apps: Vec<capture::App>,
    /// Apps the settings name that are not running: listed under the running ones.
    closed: Vec<Installed>,
    /// Apps started from the list, and where each one's window goes once it has one.
    launching: Vec<Launch>,
    /// Icons of apps seen so far, by app name. Kept: an icon is read and encoded once.
    icons: HashMap<String, Arc<gpui::Image>>,
    /// A problem found at start (settings, font, tasks folder). It stays: unlike `error`, no
    /// later success makes it untrue.
    notice: Option<String>,
    /// False: the tasks folder could not be read, so nothing is written to it.
    keeping: bool,
    /// The last task that could not be written, until a write of it succeeds.
    unsaved: Option<String>,
    /// The tasks as last written to disk, to write only what changes.
    stored: Vec<Task>,
    /// Scroll position of the bar's list, so list mode can bring the selected row into view.
    list_scroll: ScrollHandle,
    /// Window ids seen per hosted pid; anything new that appears later gets its own pane.
    known: Vec<(i32, HashSet<u32>)>,
    /// When the live pane went live.
    live_since: Option<Instant>,
    /// The hosted app that was the front app at the last look, and the window it showed, to see
    /// one come forward or change window.
    front_hosted: Option<(i32, Option<u32>)>,
    /// The pane to give the keyboard to because its app brought its window forward, kept until
    /// that can be done.
    wanted: Option<PaneId>,
    error: Option<String>,
    /// False in e2e runs, which must never move the user's real windows.
    hosting: bool,
    /// Test runs: the pid of the one app that may be hosted.
    host_only: Option<i32>,
    control: Option<Receiver<Call>>,
    /// The accessibility overlay's last read. Drawn while `model.ax_overlay` is on.
    ax_overlay: Option<ax_debug::Overlay>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// App icons: drawn 24 points wide, read at 48 pixels: pixel for pixel on a Retina display,
/// and an even half on others. Scaling 64 pixels down to 36 in the shell made them rough.
const ICON_POINTS: f32 = 24.;
const ICON_PIXELS: u8 = 48;

/// An app started from the list, waiting for its first window.
struct Launch {
    app: Installed,
    axis: Axis,
    new_tab: bool,
    since: Instant,
}

/// How long to wait for a started app's window before giving up.
const LAUNCH_WAIT: Duration = Duration::from_secs(30);

/// How much of a test run's window stays on screen, in points, at the corner it sits in.
const E2E_SLIVER: f32 = 12.;

/// How long the `s` key snoozes a task: one hour.
const SNOOZE_MS: u64 = 3_600_000;

/// Longest a `wait` request may run: 10 minutes.
const MAX_WAIT_MS: u64 = 600_000;

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
                        this.follow_front_app(window, cx);
                        this.host_launched(window, cx);
                        // A snoozed task returns when its time comes.
                        let now = now_ms();
                        if this.model.tasks.due(now) {
                            this.dispatch(Action::Tick { now }, window, cx);
                            cx.notify();
                        }
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
                // Tests use the socket, not the mouse. A window put on show can be clicked.
                let click_through = std::env::var_os("OTTOSPACE_E2E_SHOW").is_none();
                unsafe { ottospace_mac::window::keep_on_top(appkit.ns_view.as_ptr(), click_through) };
            }
        }

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut model = Model::default();
        model.update(Action::SetRepos { repos: repos::scan() });
        let kept = config::load_tasks();
        let unread = kept.unread;
        // A folder that could not be listed is left alone: writing could replace tasks it holds.
        let keeping = kept.error.is_none();
        model.update(Action::RestoreTasks { tasks: kept.tasks, floor: kept.highest });
        // The first problem found at start, for the status bar: settings, the font, then tasks.
        let unread = (unread > 0).then(|| format!("tasks: {unread} file(s) in the tasks folder could not be read"));
        let notice = config::get().error.clone().or(terminal::choose_font(window)).or(kept.error).or(unread);
        if hosting {
            // Repos is the way in while there are no panes: most work starts in a repository.
            model.update(Action::ToggleSection { section: Section::Repos });
        }
        let mut this = Self {
            model,
            hosted: HashMap::new(),
            terminals: HashMap::new(),
            cell: terminal::cell_size(window),
            dragging: None,
            copied: None,
            system_clipboard: std::env::var_os("OTTOSPACE_E2E").is_none(),
            cells: HashMap::new(),
            frames: HashMap::new(),
            home: home::Shared::default(),
            apps: Vec::new(),
            closed: Vec::new(),
            launching: Vec::new(),
            icons: HashMap::new(),
            list_scroll: ScrollHandle::new(),
            known: Vec::new(),
            live_since: None,
            front_hosted: None,
            wanted: None,
            error: None,
            notice,
            keeping,
            unsaved: None,
            stored: Vec::new(),
            hosting,
            host_only,
            control,
            ax_overlay: None,
            focus,
            _subscriptions: subscriptions,
        };
        this.refresh_apps();
        this.stored = this.kept_tasks(); // what was read needs no writing back
        this
    }

    /// The one path that changes state: updates the model, then carries out its effects.
    fn dispatch(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) -> Vec<Effect> {
        // Opening the Repos section reads the list again: repositories come and go on disk.
        let opens_repos = self.model.sidebar != Some(Section::Repos);
        if opens_repos && matches!(action, Action::ToggleSection { section: Section::Repos }) {
            self.model.update(Action::SetRepos { repos: repos::scan() });
        }
        let effects = self.model.update(action);
        self.store_tasks();
        let another_goes_live = effects.iter().any(|effect| matches!(effect, Effect::Live(_)));
        for effect in &effects {
            match effect {
                Effect::Opened(id) => {
                    if self.model.pane(*id).is_some_and(|p| p.kind == Kind::Terminal) {
                        // The shell learns where it runs, so a program in it can name its own pane.
                        let tab = self.model.tab_of(*id);
                        let mut vars = vec![("OTTOSPACE_PANE", id.to_string())];
                        vars.extend(tab.map(|tab| ("OTTOSPACE_TAB", tab.to_string())));
                        let cwd = self.model.pane(*id).and_then(|pane| pane.cwd.as_deref());
                        match Terminal::spawn(&vars, cwd) {
                            Ok(terminal) => drop(self.terminals.insert(*id, terminal)),
                            Err(err) => self.error = Some(format!("terminal: {err}")),
                        }
                    }
                }
                Effect::Closed(pane) => {
                    self.cells.remove(&pane.id);
                    self.frames.remove(&pane.id);
                    self.terminals.remove(&pane.id); // drop ends its shell
                    if let Some(hosted) = self.hosted.remove(&pane.id) {
                        hosted.release();
                    }
                }
                Effect::Parked(id) => {
                    self.live_since = None;
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        let crop = hosted.crop_at(on_screen(hosted.bounds.get(), window, cx), top_limit(window, cx));
                        hosted.park(window.scale_factor() as f64, crop);
                        // Going live activated the app. If it still has the keyboard, take it
                        // back, or keys keep going to its off-screen window while the status
                        // bar says NORMAL. If the user moved to a third app, leave them there.
                        // Not when another pane goes live in this same step: that pane's app is
                        // taking the keyboard, and ottospace must not race it.
                        if !another_goes_live && ax::is_frontmost(hosted.pid) {
                            cx.activate(true);
                        }
                    }
                }
                Effect::HostApp { name, axis, new_tab } => {
                    let _ = self.open_app(name, *axis, *new_tab, window, cx);
                }
                Effect::Visual { pane, on } => {
                    if let Some(terminal) = self.terminals.get(pane) {
                        terminal.set_visual(*on);
                    }
                }
                Effect::Live(id) => {
                    self.live_since = Some(Instant::now());
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        let rect = on_screen(hosted.bounds.get(), window, cx);
                        hosted.place(rect, hosted.crop_at(rect, top_limit(window, cx)));
                        hosted.window.raise();
                    }
                }
            }
        }
        cx.notify();
        effects
    }

    /// The tasks as they are kept on disk.
    fn kept_tasks(&self) -> Vec<Task> {
        self.model.tasks.list.clone()
    }

    /// Writes the tasks that changed since the last write, one file each. A task that could not
    /// be written counts as changed still, so the next change to anything tries it again.
    fn store_tasks(&mut self) {
        let kept = self.kept_tasks();
        if kept == self.stored || !self.keeping {
            return;
        }
        let mut failed = None;
        let mut stored = Vec::with_capacity(kept.len());
        for task in kept {
            let before = self.stored.iter().find(|t| t.id == task.id);
            if before == Some(&task) {
                stored.push(task);
                continue;
            }
            match config::save_task(&task) {
                Ok(()) => stored.push(task),
                Err(err) => {
                    failed = Some(format!("tasks: could not save task {}: {err}", task.id));
                    stored.extend(before.cloned()); // what the file still holds, if there is one
                }
            }
        }
        (self.stored, self.unsaved) = (stored, failed);
    }

    /// What the status bar says is wrong, most pressing first: the last request that failed, a
    /// task that is not saved, then a problem found at start.
    fn problem(&self) -> Option<String> {
        self.error.clone().or(self.unsaved.clone()).or(self.notice.clone())
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
            // Keys go through GPUI's own dispatch, after this update: their handlers update `self`.
            if let Request::Key { keys } = &request {
                let parsed: Result<Vec<Keystroke>, _> = keys.split_whitespace().map(Keystroke::parse).collect();
                match parsed {
                    Ok(keystrokes) => self.press(keystrokes, reply, window, cx),
                    Err(err) => drop(reply.send(control::error(&err.to_string()))),
                }
                continue;
            }
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
                Request::Resize { dir } => Some(Action::Resize { dir }),
                Request::Swap { dir } => Some(Action::SwapDir { dir }),
                Request::MovePane { pane: Some(pane), .. } if self.model.pane(pane).is_none() => {
                    failure = Some(format!("no pane {pane}"));
                    None
                }
                Request::MovePane { tab, .. } if tab >= self.model.visible_tabs().len() => {
                    failure = Some(format!("no tab at place {tab} on the tab bar"));
                    None
                }
                Request::MovePane { pane, tab } => Some(Action::MovePane { pane, tab }),
                Request::Equalize => Some(Action::Equalize),
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
                Request::Key { .. } => None, // answered by `press`
                Request::MouseDown { pane, x, y, clicks } => {
                    match self.cell_position(pane, x, y) {
                        Ok((id, position)) => {
                            let modifiers = Modifiers::default();
                            let (button, first_mouse) = (MouseButton::Left, false);
                            let event =
                                MouseDownEvent { button, position, modifiers, click_count: clicks, first_mouse };
                            // In the order GPUI calls them: the root's capture, the terminal's
                            // handler, then the pane's.
                            if self.model.visual.is_some() {
                                self.dispatch(Action::LeaveVisual, window, cx);
                            }
                            self.terminal_mouse_down(id, &event, cx);
                            self.click_pane(id, &event, window, cx);
                        }
                        Err(err) => failure = Some(err),
                    }
                    None
                }
                Request::MouseDrag { pane, x, y } => {
                    match self.cell_position(pane, x, y) {
                        Ok((_, position)) => {
                            let pressed_button = Some(MouseButton::Left);
                            let event = MouseMoveEvent { position, pressed_button, modifiers: Modifiers::default() };
                            self.mouse_move(&event, cx);
                        }
                        Err(err) => failure = Some(err),
                    }
                    None
                }
                Request::Scroll { pane, lines } => {
                    match pane.or(self.model.focused).filter(|id| self.terminals.contains_key(id)) {
                        Some(id) => self.wheel(id, ScrollDelta::Lines(point(0., lines)), cx),
                        None => failure = Some("no terminal pane to scroll".into()),
                    }
                    None
                }
                Request::Compose { text } => {
                    self.model.update(Action::SetRepos { repos: repos::scan() });
                    let was_open = self.model.compose.is_some();
                    self.dispatch(Action::OpenCompose { text }, window, cx);
                    // A draft that did not reach the user must not look delivered.
                    if was_open || self.model.compose.is_none() {
                        failure = Some("the box did not open: it is open already, or the picker, the key reference or visual mode is in the way".into());
                    }
                    None
                }
                Request::TaskNew { line } if line.chars().count() > MAX_TITLE => {
                    failure = Some(format!("the line is longer than {MAX_TITLE} characters"));
                    None
                }
                Request::TaskNew { line } => {
                    let before = self.model.tasks.list.len();
                    self.dispatch(Action::TaskFromLine { line }, window, cx);
                    let filed = self.model.tasks.list.get(before);
                    let message = match filed {
                        Some(task) => serde_json::json!({ "ok": true, "task": task }).to_string(),
                        None => control::error("the line has no title"),
                    };
                    let _ = reply.send(message);
                    continue;
                }
                Request::TaskAdd { title, .. } if title.chars().count() > MAX_TITLE => {
                    failure = Some(format!("the title is longer than {MAX_TITLE} characters"));
                    None
                }
                // A tab that is gone, or that another task has, cannot become this task's: say so,
                // or the caller believes its work is filed under the task.
                Request::TaskAdd { tab: Some(tab), .. } if self.model.tabs.iter().all(|t| t.id != tab) => {
                    failure = Some(format!("no tab with id {tab}"));
                    None
                }
                Request::TaskAdd { tab: Some(tab), .. }
                    if self.model.tabs.iter().any(|t| t.id == tab && t.task.is_some()) =>
                {
                    failure =
                        Some(format!("tab {tab} belongs to another task; leave out `tab` to file a task without one"));
                    None
                }
                Request::TaskAdd { title, priority, state, tab } => {
                    self.dispatch(Action::TaskAdd { title, priority, state, tab }, window, cx);
                    // The new task, so the caller has its id.
                    let task = serde_json::json!(self.model.tasks.list.last());
                    let _ = reply.send(serde_json::json!({ "ok": true, "task": task }).to_string());
                    continue;
                }
                Request::TaskUndo => Some(Action::TaskUndo),
                Request::TaskSetState { id, .. }
                | Request::TaskSnooze { id, .. }
                | Request::TaskHold { id }
                | Request::TaskDismiss { id }
                    if self.model.tasks.get(id).is_none() =>
                {
                    failure = Some(format!("no task {id}"));
                    None
                }
                Request::TaskSetState { id, state } => Some(Action::TaskSetState { id, state }),
                Request::TaskSnooze { id, seconds } => {
                    Some(Action::TaskSnooze { id, until: now_ms().saturating_add(seconds.saturating_mul(1000)) })
                }
                Request::TaskHold { id } => Some(Action::TaskHold { id }),
                Request::TaskDismiss { id } => Some(Action::TaskDismiss { id }),
                Request::TaskList => {
                    self.dispatch(Action::Tick { now: now_ms() }, window, cx);
                    let _ = reply.send(serde_json::json!({ "ok": true, "inbox": self.inbox() }).to_string());
                    continue;
                }
                Request::TaskPop => {
                    self.dispatch(Action::Tick { now: now_ms() }, window, cx);
                    let next = self.model.tasks.next().map(|task| task.id);
                    let task = serde_json::json!(self.model.tasks.next());
                    self.dispatch(Action::TaskOpen { id: None }, window, cx);
                    // False: the picker or the key reference is in the way, so the view stayed.
                    let opened = next.is_some() && self.model.current_task == next;
                    let _ = reply.send(serde_json::json!({ "ok": true, "task": task, "opened": opened }).to_string());
                    continue;
                }
                Request::TaskOpen { id } if self.model.tasks.get(id).is_none() => {
                    failure = Some(format!("no task {id}"));
                    None
                }
                Request::TaskOpen { id } => {
                    // Its repository as it is now, for the terminal a first opening starts.
                    self.model.update(Action::SetRepos { repos: repos::scan() });
                    Some(Action::TaskOpen { id: Some(id) })
                }
                Request::TaskLeave => Some(Action::TaskLeave),
                Request::Repos => Some(Action::SetRepos { repos: repos::scan() }),
                Request::OpenRepo { path, .. } if !self.model.repo_listed(&path) => {
                    failure = Some("no listed repository at that path".into());
                    None
                }
                // Deleted or moved since the list was read: do not open a shell somewhere else.
                Request::OpenRepo { path, .. } if !std::path::Path::new(&path).is_dir() => {
                    failure = Some(format!("{path} is not a directory any more"));
                    None
                }
                Request::OpenRepo { path, axis } => Some(Action::OpenRepo { path, axis }),
                Request::Capture { pane, history } => {
                    let id = pane.or(self.model.focused);
                    let message = match id.and_then(|id| self.terminals.get(&id).map(|t| (id, t))) {
                        Some((id, t)) => {
                            serde_json::json!({ "ok": true, "pane": id, "text": t.text(history) }).to_string()
                        }
                        None => control::error("no terminal pane to capture"),
                    };
                    let _ = reply.send(message);
                    continue;
                }
                Request::Wait { pane, pattern, timeout_ms } => {
                    self.wait(pane, &pattern, timeout_ms, reply, cx);
                    continue;
                }
                Request::AxTree { pane, depth, nodes } => {
                    let source = self.ax_source(pane.or(self.model.focused));
                    ax_debug::serve_tree(source, depth, nodes, reply, cx.background_executor());
                    continue;
                }
                Request::AxOverlay { refresh } => {
                    failure = self.toggle_ax_overlay(refresh, window, cx).err();
                    None
                }
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

    /// A point in a terminal pane (default: the focused one), given in cells from its top-left, in
    /// window points.
    fn cell_position(&self, pane: Option<PaneId>, x: f32, y: f32) -> Result<(PaneId, gpui::Point<Pixels>), String> {
        let id = pane.or(self.model.focused).filter(|id| self.terminals.contains_key(id)).ok_or("no terminal pane")?;
        let bounds = self.cells.get(&id).map(|c| c.get()).filter(|b| b.size.width > px(1.)).ok_or("not laid out")?;
        Ok((id, bounds.origin + point(self.cell.width * x, self.cell.height * y)))
    }

    /// The inbox as JSON rows: each task with the group it is listed under.
    fn inbox(&self) -> Vec<serde_json::Value> {
        let row = |(group, task): (Group, &Task)| {
            let mut row = serde_json::json!(task);
            row["group"] = serde_json::json!(group);
            row
        };
        self.model.tasks.inbox().into_iter().map(row).collect()
    }

    /// Replies when a line of a terminal pane's text matches `pattern`, or with an error at the
    /// time limit. Polls: the terminal has no "new output" event to wait on.
    fn wait(
        &self,
        pane: Option<PaneId>,
        pattern: &str,
        timeout_ms: Option<u64>,
        reply: Sender<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = pane.or(self.model.focused).filter(|id| self.terminals.contains_key(id)) else {
            return drop(reply.send(control::error("no terminal pane to wait on")));
        };
        let regex = match regex::Regex::new(pattern) {
            Ok(regex) => regex,
            Err(err) => return drop(reply.send(control::error(&err.to_string()))),
        };
        // Capped: nothing tells this task that its client went away.
        let limit = Duration::from_millis(timeout_ms.unwrap_or(10_000).min(MAX_WAIT_MS));
        let start = Instant::now();
        cx.spawn(async move |this, cx| {
            let message = loop {
                let text = this.update(cx, |this, _| this.terminals.get(&id).map(|t| t.text(true)));
                let Ok(text) = text else { break control::error("ottospace is closing") };
                let Some(text) = text else { break control::error(&format!("pane {id} closed")) };
                // Line by line, so `^done$` skips the echoed command, which starts with the prompt.
                if let Some(line) = text.lines().find(|line| regex.is_match(line)) {
                    break serde_json::json!({ "ok": true, "pane": id, "line": line }).to_string();
                }
                if start.elapsed() >= limit {
                    break control::error(&format!("no match for the pattern in {} ms", limit.as_millis()));
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            };
            let _ = reply.send(message);
        })
        .detach();
    }

    /// Presses `keystrokes` as if typed, once this update is done, then replies with the state.
    fn press(&self, keystrokes: Vec<Keystroke>, reply: Sender<String>, window: &Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            for keystroke in keystrokes {
                window.dispatch_keystroke(keystroke, cx);
            }
            let state = this.update(cx, |this, _| this.snapshot(window));
            let _ = reply.send(state.unwrap_or_else(|_| control::error("ottospace is closing")));
        });
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
        let crop: BTreeMap<PaneId, f64> = self.hosted.iter().map(|(id, h)| (*id, h.crop)).collect();
        let frame = window.bounds();
        let frame: [f32; 4] =
            [frame.origin.x.into(), frame.origin.y.into(), frame.size.width.into(), frame.size.height.into()];
        // Only panes of the active tab are drawn; a background pane's last bounds are stale.
        let drawn = self.model.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let drawn_rects = |cells: &HashMap<PaneId, Rc<Cell<Bounds<Pixels>>>>| -> BTreeMap<PaneId, [f32; 4]> {
            cells
                .iter()
                .filter(|(id, _)| drawn.contains(id))
                .map(|(id, cell)| {
                    let b = cell.get();
                    (*id, [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()])
                })
                .collect()
        };
        let (bounds, frames) = (drawn_rects(&self.cells), drawn_rects(&self.frames));
        let terminals: BTreeMap<PaneId, serde_json::Value> = self
            .terminals
            .iter()
            .map(|(id, t)| {
                let (cols, rows) = t.grid_size();
                let (display_offset, history) = t.scrollback();
                let terminal = serde_json::json!({
                    "cols": cols, "rows": rows, "lines": t.lines(),
                    // Lines scrolled back from the bottom, and lines of history kept.
                    "display_offset": display_offset, "history": history,
                    // What ⌘C would copy.
                    "selection": t.selected(),
                    "visual_cursor": t.visual_cursor(),
                });
                (*id, terminal)
            })
            .collect();
        serde_json::json!({
            "ok": true,
            "mode": self.model.mode(),
            "state": self.model,
            "bounds": bounds,
            // Each drawn pane's bounds inside its border.
            "frames": frames,
            "tab_panes": self.model.tab_panes(),
            // ottospace's own window frame, so a test can turn pane bounds into screen positions.
            "window": frame,
            // Where parked windows go, so a test can tell "parked" from "moved somewhere".
            "parking": [parking.x, parking.y],
            // The registered return key, or null if there is none.
            "return_key": RETURN_KEY.get(),
            // Which build this is, as the status bar shows it: "0.1.0 · a4a2582".
            "version": version_tag(env!("CARGO_PKG_VERSION"), option_env!("OTTOSPACE_COMMIT")),
            // The terminal font in use, and the size of one character cell in points.
            "font": terminal::font_in_use(),
            "cell": [f32::from(self.cell.width), f32::from(self.cell.height)],
            // The problem the status bar shows, or null. Not `error`: that key means the request failed.
            "notice": self.problem(),
            // Each hosted window's real frame, in the same global coordinates as `parking`.
            "hosted": hosted,
            // Each hosted pane's title-bar crop in points (0: none).
            "crop": crop,
            "terminals": terminals,
            // The last text ⌘C copied.
            "copied": self.copied,
            // Tasks as the Tasks section lists them, and how many need the user.
            "inbox": self.inbox(),
            "needs_you": self.model.tasks.needs_you(),
            // The home screen, or null while the tab on show has a pane: each task's node (its
            // lane, depth and crossing of the lattice) and the rows under "Needs you".
            "home": self.model.home(&self.home.borrow().keep_out()),
            // What the home screen drew, in window points: its `area`, the `panel` (the new-task
            // line with its list) and each node's click target by task id. Also how often
            // lattice lines were built and painted, here and in empty panes.
            "home_drawn": self.home.borrow().snapshot(),
            // Apps from the settings that are not running: `open_app` starts one.
            "closed_apps": self.closed.iter().map(|app| &app.name).collect::<Vec<_>>(),
            // The Apps list: what can be hosted now. `icon`: its icon was read.
            "apps": self.apps.iter().map(|app| serde_json::json!({
                "name": app.name, "windows": app.windows, "icon": self.icons.contains_key(&app.name),
            })).collect::<Vec<_>>(),
            "ax_overlay": self.ax_overlay.as_ref().filter(|_| self.model.ax_overlay),
        })
        .to_string()
    }

    /// The hosted window behind `pane`, if it is an app pane.
    fn ax_source(&self, pane: Option<PaneId>) -> Option<ax_debug::Hosted> {
        let id = pane?;
        let h = self.hosted.get(&id)?;
        Some(ax_debug::Hosted::new(id, h.pid, h.window_id, h.crop, h.bounds.get()))
    }

    /// Toggles the accessibility overlay, or with `refresh` shows it; either way a shown overlay
    /// reads the focused app pane's tree again, off the UI thread.
    fn toggle_ax_overlay(&mut self, refresh: bool, window: &Window, cx: &mut Context<Self>) -> Result<(), String> {
        let source = self.ax_source(self.model.focused);
        let showing = self.model.ax_overlay;
        if (refresh || !showing) && source.is_none() {
            return Err("the accessibility overlay needs a focused app pane".into());
        }
        if !(refresh && showing) {
            self.dispatch(Action::ToggleAxOverlay, window, cx);
        }
        if let Some(source) = source.filter(|_| self.model.ax_overlay) {
            cx.spawn(async move |this, cx| {
                let overlay = cx.background_executor().spawn(async move { ax_debug::Overlay::load(source) }).await;
                let _ = this.update(cx, |this, cx| {
                    // A slow read may land after focus moved; keep only the focused pane's.
                    if this.model.focused != Some(overlay.pane) {
                        return;
                    }
                    this.ax_overlay = Some(overlay);
                    cx.notify();
                });
            })
            .detach();
        }
        Ok(())
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
            Some(pid) if capture::name_of(pid).as_deref() == Some(name) => capture::find_pid(pid, window_managed)?,
            Some(_) => return Err("this test run may host only the app it started".into()),
            None => match capture::find(name, window_managed) {
                Ok(target) => target,
                // An app from the settings that is not running: start it, and host its window
                // when it has one.
                Err(err) => match self.closed.iter().find(|closed| closed.name == name).cloned() {
                    Some(app) => return self.launch(app, axis, new_tab),
                    None => return Err(err),
                },
            },
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

    /// Starts an app that is not running, in the background: its window is wanted in a pane, not
    /// in front of ottospace. By the path it was found at, so nothing but that app can start.
    fn launch(&mut self, app: Installed, axis: Axis, new_tab: bool) -> Result<(), String> {
        if self.launching.iter().any(|launch| launch.app.path == app.path) {
            return Ok(()); // already on its way
        }
        match std::process::Command::new("open").arg("-g").arg(&app.path).status() {
            Ok(status) if status.success() => {
                self.launching.push(Launch { app, axis, new_tab, since: Instant::now() });
                Ok(())
            }
            Ok(_) => Err(format!("{} could not be opened", app.name)),
            Err(err) => Err(format!("{} could not be opened: {err}", app.name)),
        }
    }

    /// Hosts the window of each app started from the list, once it has one. A started app is
    /// known by its bundle, not its name: `Visual Studio Code` runs as `Code`. It gives up after
    /// a while: some apps open a sign-in window, or none.
    fn host_launched(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.launching.is_empty() {
            return;
        }
        // First: an app that now runs leaves the "not running" list, so a window that is not
        // ready yet is tried again here, not started a second time by `open_app`.
        self.refresh_apps();
        let running = self.apps.clone();
        for launch in std::mem::take(&mut self.launching) {
            let bundle = Some(PathBuf::from(&launch.app.path));
            let app = running.iter().find(|app| icon::bundle_of(app.pid) == bundle);
            // Listed, but its main window may not be ready yet: try again until the time is up.
            let hosted =
                app.is_some_and(|app| self.open_app(&app.name, launch.axis, launch.new_tab, window, cx).is_ok());
            if hosted {
                continue;
            }
            if launch.since.elapsed() > LAUNCH_WAIT {
                let seconds = LAUNCH_WAIT.as_secs();
                self.error = Some(format!("{} did not open a window in {seconds} seconds", launch.app.name));
            } else {
                self.error = None; // a try that was too early is not a failure yet
                self.launching.push(launch);
            }
        }
        cx.notify();
    }

    /// A hosted app came to the front by itself (a click on its Dock icon, ⌘Tab) showing a
    /// window ottospace holds. That window is parked off screen, so the user would see nothing
    /// happen: go to its pane and give it the keyboard.
    ///
    /// It acts when the app comes forward or shows another window, not for as long as it is in
    /// front: the user may be in a window of that app that ottospace does not hold, and must be
    /// left there. A move that cannot be made yet (a box is open) is kept and made when it can.
    fn follow_front_app(&mut self, window: &Window, cx: &mut Context<Self>) {
        // One question an app, not a pane. An app that is in front is asked which window it shows.
        let mut pids: Vec<i32> = self.hosted.values().map(|hosted| hosted.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        let front = pids.into_iter().find(|pid| ax::is_frontmost(*pid)).map(|pid| (pid, ax::Window::main_id(pid)));
        if front != self.front_hosted {
            self.front_hosted = front;
            // The pane of the window it shows, if ottospace holds that window. No stand-in: a
            // window ottospace does not hold, or may not hold, is the user's business.
            let shown = front.and_then(|(_, window)| window);
            self.wanted = self.hosted.iter().find(|(_, h)| Some(h.window_id) == shown).map(|(id, _)| *id);
        }
        let Some(pane) = self.wanted else { return };
        if self.model.live == Some(pane) {
            self.wanted = None; // already the one on screen
            return;
        }
        // The core would drop the move behind a box or in visual mode. Wait: do not come
        // forward either, which would only take the app away from the user.
        if self.model.keys || matches!(self.model.mode(), Mode::Pick | Mode::Compose | Mode::Visual) {
            return;
        }
        self.wanted = None;
        // ottospace must be on screen under the window it is about to place. In front, but not
        // the front app: activating ottospace here would race the app's own activation and
        // could win, leaving the keys with ottospace while the status bar says APP.
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
        {
            unsafe { ottospace_mac::window::order_front(appkit.ns_view.as_ptr()) };
        }
        self.dispatch(Action::Enter { pane }, window, cx);
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
                // Its app may still be frontmost with no window here: take the keyboard back.
                if self.model.live == Some(id) && ax::is_frontmost(hosted.pid) {
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
                // A window the rules leave alone is seen once and never becomes a pane.
                if let Some(id) = window.id()
                    && seen.insert(id)
                    && window_managed(&window)
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
        let limit = top_limit(window, cx);
        for (id, hosted) in &mut self.hosted {
            let rect = on_screen(hosted.bounds.get(), window, cx);
            if rect.size.width < 1.0 {
                continue; // not laid out yet
            }
            let crop = hosted.crop_at(rect, limit);
            if live == Some(*id) {
                let drifted = check && hosted.window.frame().is_some_and(|f| f.origin != raised(rect, crop).origin);
                if hosted.placed != rect || drifted {
                    hosted.place(rect, crop);
                }
            } else if hosted.parked_size != rect.size || hosted.crop != crop {
                hosted.park(scale, crop);
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
        // ⌘⇧⌥1..9 move the focused pane to a tab. GPUI gives no physical key, so the shifted
        // characters are trusted on a US layout only (see `move_to_tab_key`).
        let m = key.modifiers;
        let us_layout = matches!(cx.keyboard_layout().id(), "com.apple.keylayout.US" | "com.apple.keylayout.ABC");
        let place = keys::move_to_tab_key(&key.key, m.shift, us_layout);
        if let Some(tab) = place.filter(|_| m.platform && m.alt && !m.control) {
            self.dispatch(Action::MovePane { pane: None, tab }, window, cx);
            cx.stop_propagation();
            return;
        }
        // ⌘1..⌘9 select a tab.
        if key.modifiers.platform
            && let Some(digit) = key.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d))
        {
            self.dispatch(Action::SelectVisibleTab { index: digit - 1 }, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.model.picker.is_some() {
            self.picker_key(event, window, cx);
            return;
        }
        if self.model.compose.is_some() {
            self.compose_key(event, window, cx);
            return;
        }
        if self.model.mode() == Mode::Visual {
            self.visual_key(event, window, cx);
            return;
        }
        if self.model.mode() == Mode::List {
            self.list_key(event, window, cx);
            return;
        }
        let Some(terminal) = self.model.focused.and_then(|id| self.terminals.get(&id)) else { return };
        let key = &event.keystroke;
        let m = key.modifiers;
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, cmd: m.platform };
        if let Some(scroll) = scroll_key(&key.key, mods, terminal.alt_screen()) {
            terminal.scroll(scroll);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if let Some(bytes) = keys::encode(&key.key, key.key_char.as_deref(), mods, terminal.app_cursor()) {
            terminal.write(bytes);
            cx.stop_propagation();
        }
    }

    /// The wheel or trackpad over a terminal pane.
    fn wheel(&mut self, id: PaneId, delta: ScrollDelta, cx: &mut Context<Self>) {
        let height = self.cell.height;
        if let Some(terminal) = self.terminals.get_mut(&id) {
            terminal.wheel(delta.pixel_delta(height).y.into(), height.into());
            cx.notify();
        }
    }

    /// A press on a terminal starts a selection there: a drag, or a word or line by click count.
    /// It also focuses the pane (`click_pane`), so the first click on an unfocused pane selects too.
    fn terminal_mouse_down(&mut self, id: PaneId, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(at) = self.cell_under(id, event.position) else { return };
        if let Some(terminal) = self.terminals.get(&id) {
            terminal.select_start(at, select_kind(event.click_count));
            self.dragging = Some(id);
            cx.notify();
        }
    }

    /// A drag extends the selection, even past the pane's edge. Released anywhere, it ends.
    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(id) = self.dragging else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = None;
            return;
        }
        if let (Some(at), Some(terminal)) = (self.cell_under(id, event.position), self.terminals.get(&id)) {
            terminal.select_to(at);
            cx.notify();
        }
    }

    /// The terminal cell under a window position, from the same bounds the grid is painted in.
    fn cell_under(&self, id: PaneId, position: gpui::Point<Pixels>) -> Option<CellPoint> {
        let (bounds, terminal) = (self.cells.get(&id)?.get(), self.terminals.get(&id)?);
        let local = position - bounds.origin;
        let cell = (self.cell.width.into(), self.cell.height.into());
        Some(cell_at(local.x.into(), local.y.into(), cell, terminal.grid_size()))
    }

    /// The new-task box takes every key but ⌘ shortcuts: text types, ↵ files, Tab completes.
    fn compose_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        let action = match key.key.as_str() {
            _ if m.platform => return, // ⌘ shortcuts still reach their bindings; pane ones wait
            "escape" => Action::CloseCompose,
            "enter" if !event.is_held => Action::ComposeSubmit,
            "enter" => return,
            "tab" => Action::ComposeComplete,
            "backspace" => Action::ComposeBackspace,
            // Whatever the key types, ⌥ included: on some layouts `@` and `#` need it.
            _ => match key.key_char.as_deref().filter(|_| !m.control) {
                Some(text) if !text.chars().any(char::is_control) => Action::ComposeType { text: text.into() },
                _ => return,
            },
        };
        self.dispatch(action, window, cx);
        cx.stop_propagation();
    }

    /// List mode takes every key but ⌘ shortcuts: they move over the bar's rows and act on one.
    fn list_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.platform {
            return; // ⌘ shortcuts still reach their bindings; ⌘⇧E leaves
        }
        cx.stop_propagation();
        // ⌃X is not `x`. A held key only moves: one press dismisses or opens one row.
        let pressed = list_key(&key.key, key.modifiers.shift).filter(|_| !key.modifiers.control && !key.modifiers.alt);
        let action = match pressed.filter(|key| !event.is_held || matches!(key, ListKey::Move(_))) {
            Some(ListKey::Move(delta)) => Action::ListMove { delta },
            Some(ListKey::Open { down }) => Action::ListChoose { axis: if down { Axis::Down } else { Axis::Right } },
            Some(ListKey::Section(delta)) => {
                self.refresh_lists(); // the section it lands on shows what is there now
                Action::ListSection { delta }
            }
            Some(ListKey::Snooze) => Action::ListSnooze { until: now_ms().saturating_add(SNOOZE_MS) },
            Some(ListKey::Hold) => Action::ListHold,
            Some(ListKey::Dismiss) => Action::ListDismiss,
            Some(ListKey::Undo) => Action::TaskUndo,
            Some(ListKey::Leave) => Action::LeaveList,
            None => return,
        };
        self.dispatch(action, window, cx);
        if let Some(child) = self.selected_child() {
            self.list_scroll.scroll_to_item(child);
        }
        cx.notify();
    }

    /// Where the selected row is among the children of its list, to scroll it into view. Tasks
    /// and Repos have headings between their rows.
    fn selected_child(&self) -> Option<usize> {
        let at = self.model.list?;
        match self.model.sidebar {
            Some(Section::Tasks) => {}
            // A heading before each repository's rows.
            Some(Section::Repos) => {
                let mut rows = 0;
                let headings = self.model.repos.iter().take_while(|repo| {
                    let starts_before = rows <= at;
                    rows += 1 + repo.worktrees.len();
                    starts_before
                });
                return Some(at + headings.count());
            }
            // A "Not running" heading before the apps that are not.
            Some(Section::Apps) => return Some(at + usize::from(at >= self.apps.len())),
            _ => return Some(at),
        }
        let inbox = self.model.tasks.inbox();
        let mut groups: Vec<Group> = inbox.iter().take(at + 1).map(|(group, _)| *group).collect();
        groups.dedup();
        Some(at + groups.len())
    }

    /// Reads the Apps and Repos lists again.
    fn refresh_lists(&mut self) {
        self.refresh_apps();
        self.model.update(Action::SetRepos { repos: repos::scan() });
    }

    /// Visual mode takes every key but ⌘ shortcuts: vi keys move, select, copy and leave.
    fn visual_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.platform {
            return; // ⌘ shortcuts still reach their bindings; ⌘⇧C leaves
        }
        cx.stop_propagation();
        let Some(id) = self.model.visual else { return };
        let (Some(terminal), Some(action)) = (self.terminals.get(&id), visual_key(&key.key, key.key_char.as_deref()))
        else {
            return;
        };
        match action {
            VisualKey::Move(motion) => terminal.visual_move(motion),
            VisualKey::Select => terminal.visual_select(false),
            VisualKey::SelectLines => terminal.visual_select(true),
            VisualKey::Copy => {
                if let Some(text) = terminal.selected() {
                    self.copy_text(text, cx);
                }
                self.dispatch(Action::LeaveVisual, window, cx);
            }
            VisualKey::Exit => drop(self.dispatch(Action::LeaveVisual, window, cx)),
        }
        cx.notify();
    }

    /// ⌘C: copies the focused terminal's selection, in visual mode too. With nothing selected it
    /// does nothing.
    fn copy(&mut self, cx: &mut Context<Self>) {
        let pane = self.model.visual.or_else(|| self.model.keyboard_terminal());
        let terminal = pane.and_then(|id| self.terminals.get(&id));
        let Some(text) = terminal.and_then(Terminal::selected) else { return };
        self.copy_text(text, cx);
    }

    fn copy_text(&mut self, text: String, cx: &mut Context<Self>) {
        if self.system_clipboard {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        }
        self.copied = Some(text);
    }

    /// ⌘V: pastes the clipboard's text into the focused terminal.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(terminal) = self.model.keyboard_terminal().and_then(|id| self.terminals.get(&id)) else { return };
        let text = match self.system_clipboard {
            true => cx.read_from_clipboard().and_then(|item| item.text()),
            false => self.copied.clone(),
        };
        if let Some(text) = text {
            terminal.paste(&text);
        }
    }

    /// Opens the pane picker: a terminal, then each running app.
    fn open_picker(&mut self, axis: Axis, new_tab: bool, window: &Window, cx: &mut Context<Self>) {
        self.refresh_apps();
        // What `refresh_apps` just listed: running apps, then those that are not.
        let apps = self.model.apps.iter().map(|name| PickItem::App { name: name.clone() });
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
                    .child(group("MOVE", rows(KEYS_MOVE)))
                    .child(group("ARRANGE", rows(KEYS_ARRANGE))),
            )
            .child(div().flex().flex_col().gap_4().child(group("TABS", rows(KEYS_TABS))).child(group(
                "WINDOW",
                vec![
                    (return_key, "From any app: back to ottospace"),
                    ("⌘⌃F".into(), "Zoom the window"),
                    ("⌘⌃A".into(), "Accessibility overlay (debug)"),
                    ("⌘/".into(), "This list"),
                ],
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(group("TERMINAL", rows(KEYS_TERMINAL)))
                    .child(group("VISUAL", rows(KEYS_VISUAL)))
                    .child(group("LIST", rows(KEYS_LIST))),
            );
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

    /// Opens the new-task box: ⌘N, or a click on the home screen's line.
    fn new_task(&mut self, window: &Window, cx: &mut Context<Self>) {
        // The repositories as they are now, for `@name`.
        self.model.update(Action::SetRepos { repos: repos::scan() });
        self.dispatch(Action::OpenCompose { text: String::new() }, window, cx);
    }

    /// The new-task box, over the workspace: the line, what it will file, and names that finish
    /// the `#project` or `@repo` being typed. A click outside it closes it.
    fn render_compose(&self, line: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let draft = self.model.draft(line);
        let chip = |label: String, color: u32| {
            div().px_1p5().rounded_sm().bg(rgb(theme::SELECTED)).text_xs().text_color(rgb(color)).child(label)
        };
        // What ↵ would file.
        let reads = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .children(draft.priority.map(|p| chip(format!("P{p}"), theme::TEXT)))
            .children(draft.project.map(|p| chip(format!("#{p}"), theme::COMPOSE)))
            .children(draft.repo.map(|r| chip(format!("@{r}"), theme::PRIMARY)));
        let suggestions = self.model.compose_suggestions();
        let hint = if suggestions.is_empty() {
            "#project · @repo · p1 to p9 · ↵ file · esc close".to_string()
        } else {
            format!("tab: {}  ·  ↵ file · esc close", suggestions.join("  ·  "))
        };
        let text = div()
            .min_h(px(28.))
            .flex()
            .items_center()
            .text_base()
            .when(line.is_empty(), |d| d.text_color(rgb(theme::MUTED)).child("What needs doing?"))
            .when(!line.is_empty(), |d| d.child(line.to_string()))
            // The caret: there is no cursor movement, so it is always at the end.
            .child(div().ml(px(1.)).w(px(2.)).h(px(18.)).bg(rgb(theme::COMPOSE)));
        let panel = div()
            .w(px(560.))
            .max_w_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::COMPOSE))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.7),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
            }])
            // A click inside the box must not reach the backdrop, which closes it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_xs().text_color(rgb(theme::COMPOSE)).child("NEW TASK"))
            .child(text)
            .child(reads)
            .child(div().text_xs().text_color(rgb(theme::MUTED)).child(hint));
        div()
            .id("compose-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_start() // the box is as tall as its lines, not the window
            .pt(px(120.))
            .px_4()
            .bg(black().opacity(0.4))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::CloseCompose, window, cx);
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
                .child(match item {
                    // An app shows its icon where the tag would be.
                    PickItem::App { name } if self.icons.contains_key(name) => {
                        div().w(px(36.)).child(self.app_icon(name))
                    }
                    _ => div().w(px(36.)).text_xs().text_color(rgb(color)).child(tag),
                })
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
            .child(div().id("picker-rows").flex_1().min_h_0().overflow_y_scroll().children(rows))
            // The status bar no longer lists these, and ⌘/ closes the picker.
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(rgb(theme::BORDER))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child("↑↓ move · ↵ open · esc close"),
            );
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
            .child(div().flex_1())
            // Reads the section's list again. Tasks and Workbook have nothing to read.
            .children(matches!(section, Section::Apps | Section::Repos).then(|| {
                div()
                    .id("bar-refresh")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                    .child("↻")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_lists();
                        cx.notify();
                    }))
            }))
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
            Section::Repos => self.render_repos(cx).into_any_element(),
            Section::Tasks => self.render_tasks(cx).into_any_element(),
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

    /// Lists the apps that can be hosted now, and reads the icon of each one not seen before.
    fn refresh_apps(&mut self) {
        self.apps = running_apps(self.hosting, self.host_only);
        let running: Vec<String> = self.apps.iter().map(|app| app.name.clone()).collect();
        // Never in a test run: choosing one starts a real app.
        let offer = self.hosting && self.host_only.is_none();
        let wanted: Vec<Installed> = config::get()
            .settings
            .apps
            .iter()
            .filter(|_| offer)
            .filter_map(|name| {
                let path = icon::app_path(name.trim())?.to_string_lossy().into_owned();
                Some(Installed { name: name.trim().to_string(), path })
            })
            .collect();
        let bundles: Vec<String> = if wanted.is_empty() {
            Vec::new()
        } else {
            let bundle = |app: &capture::App| icon::bundle_of(app.pid).map(|path| path.to_string_lossy().into_owned());
            self.apps.iter().filter_map(bundle).collect()
        };
        self.closed = closed_apps(&wanted, &bundles);
        // The list as list mode walks it: running apps, then the others.
        let names = running.iter().cloned().chain(self.closed.iter().map(|app| app.name.clone())).collect();
        self.model.update(Action::SetApps { names });
        // An app that quit loses its icon: a later app of the same name may have another.
        self.icons.retain(|name, _| running.contains(name) || self.closed.iter().any(|app| app.name == *name));
        let image = |png: Vec<u8>| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png));
        for app in &self.apps {
            if !self.icons.contains_key(&app.name)
                && let Some(png) = icon::png(app.pid, ICON_PIXELS)
            {
                self.icons.insert(app.name.clone(), image(png));
            }
        }
        for app in &self.closed {
            if !self.icons.contains_key(&app.name)
                && let Some(png) = icon::png_of_app(Path::new(&app.path), ICON_PIXELS)
            {
                self.icons.insert(app.name.clone(), image(png));
            }
        }
    }

    /// An app's icon, or an empty box of the same size so names stay in line.
    fn app_icon(&self, name: &str) -> AnyElement {
        let side = px(ICON_POINTS);
        match self.icons.get(name) {
            Some(icon) => img(icon.clone()).size(side).flex_none().into_any_element(),
            None => div().size(side).flex_none().into_any_element(),
        }
    }

    /// The inbox: tasks under their group headings, most pressing first. A click brings the
    /// task's tab forward.
    fn render_tasks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut heading = None;
        let mut listed = 0; // tasks so far: headings are not rows of the list
        for (group, task) in self.model.tasks.inbox() {
            if heading != Some(group) {
                heading = Some(group);
                let color = if group == Group::NeedsYou { theme::SNEAK } else { theme::MUTED };
                rows.push(div().px_2().pt_2().text_xs().text_color(rgb(color)).child(group.title()).into_any_element());
            }
            let (id, index) = (task.id, listed);
            listed += 1;
            rows.push(
                list_button(("task", id as usize), self.model.list == Some(index))
                    .children(task_cells(task, 90.))
                    .on_click(open_task(id, cx))
                    .into_any_element(),
            );
        }
        let empty = rows.is_empty().then(|| {
            let hint = "No tasks. File one from a shell: ottospace ctl '{\"cmd\":\"task_add\",\"title\":\"...\"}'";
            div().px_2().text_xs().text_color(rgb(theme::MUTED)).child(hint)
        });
        let list = div().id("tasks").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        list.track_scroll(&self.list_scroll).children(empty).children(rows)
    }

    /// Repositories, each with its worktrees under it. A click opens a terminal there: it fills
    /// the focused pane if that is empty, else splits it (right, or down with shift).
    fn render_repos(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // One checkout: the branch it is on, and for a linked worktree its directory's name.
        let row = |index: usize, path: &str, branch: Option<&str>, directory: Option<&str>| {
            let target = path.to_string();
            let branch = branch.unwrap_or("(no branch)").to_string();
            list_row(("repo", index), self.model.list == Some(index))
                .child(div().flex_1().min_w_0().truncate().child(branch))
                .children(directory.map(|name| {
                    div().max_w(px(110.)).truncate().text_xs().text_color(rgb(theme::MUTED)).child(name.to_string())
                }))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                    this.dispatch(Action::OpenRepo { path: target.clone(), axis }, window, cx);
                }))
        };
        // The repository's name as a heading, then its checkouts: its own directory first.
        let mut items: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for repo in &self.model.repos {
            let name = div().px_2().pt_2().text_sm().font_weight(FontWeight::SEMIBOLD).truncate();
            items.push(name.child(repo.name.clone()).into_any_element());
            items.push(row(index, &repo.path, repo.branch.as_deref(), None).into_any_element());
            index += 1;
            for worktree in &repo.worktrees {
                let branch = worktree.branch.as_deref();
                items.push(row(index, &worktree.path, branch, Some(&worktree.name)).into_any_element());
                index += 1;
            }
        }
        let empty = self.model.repos.is_empty().then(|| {
            let roots: Vec<String> = repos::roots().iter().map(|root| root.display().to_string()).collect();
            let hint = format!(
                "No repositories in: {}. Set OTTOSPACE_REPOS to the directories that hold yours.",
                if roots.is_empty() { "(no directories)".to_string() } else { roots.join(", ") }
            );
            div().px_2().text_xs().text_color(rgb(theme::MUTED)).child(hint)
        });
        let list = div().id("repos").flex_1().flex().flex_col().p_2().overflow_y_scroll();
        list.track_scroll(&self.list_scroll).children(empty).children(items)
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
            .track_scroll(&self.list_scroll)
            .children(self.apps.iter().enumerate().map(|(i, app)| {
                let target = app.name.clone();
                list_button(("app", i), self.model.list == Some(i))
                    .py_1p5()
                    .gap_3()
                    .child(self.app_icon(&app.name))
                    .child(div().flex_1().min_w_0().truncate().child(app.name.clone()))
                    // How many windows the app has open, when more than one.
                    .when(app.windows > 1, |row| {
                        // A fixed box with the digits centred in it. Digits have no descender, so
                        // a line centred by its box sits high: one point of padding evens it.
                        let badge = div().h(px(18.)).min_w(px(18.)).px_1().pt(px(1.)).flex_none().flex();
                        let badge = badge.items_center().justify_center().rounded_full().bg(rgb(theme::SELECTED));
                        let badge = badge.text_xs().line_height(px(16.)).text_color(rgb(theme::MUTED));
                        row.child(badge.child(app.windows.to_string()))
                    })
                    // Fills the focused pane if it is empty. Else splits it: right, or down with shift.
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                        let _ = this.open_app(&target, axis, false, window, cx);
                    }))
            }))
            // Apps the settings name that are not running. A click starts one and hosts its window.
            .children(
                (!self.closed.is_empty())
                    .then(|| div().px_2().pt_2().text_xs().text_color(rgb(theme::MUTED)).child("Not running")),
            )
            .children(self.closed.iter().enumerate().map(|(i, app)| {
                let (name, target, row) = (&app.name, app.name.clone(), self.apps.len() + i);
                let waiting = self.launching.iter().any(|launch| launch.app.path == app.path);
                list_button(("closed-app", i), self.model.list == Some(row))
                    .py_1p5()
                    .gap_3()
                    .text_color(rgb(theme::MUTED))
                    .child(self.app_icon(name))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                    .when(waiting, |row| row.child(div().flex_none().text_xs().child("opening…")))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                        let _ = this.open_app(&target, axis, false, window, cx);
                    }))
            }))
    }

    fn render_pane(&self, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement {
        let id = pane.id;
        let cell = self.cells.get(&id).cloned().unwrap_or_default();
        let frame = self.frames.get(&id).cloned().unwrap_or_default();
        let bar = self.hosted.get(&id).map_or(0.0, |h| h.bar as f32);
        let (tag, tag_color, title) = match &pane.kind {
            Kind::Empty => ("EMPTY", theme::MUTED, String::new()),
            Kind::Terminal => ("TERM", theme::PRIMARY, "shell".into()),
            Kind::App { title, .. } => ("APP", theme::APP, title.clone()),
        };
        // flex_none: a short pane must not squeeze the header, or a raised bar would overlap the pane above.
        let header = div()
            .flex_none()
            .h(px(titlebar::header(bar, PANE_HEADER)))
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
            let mode = self.model.mode();
            let has_keys =
                self.model.focused == Some(id) && matches!(mode, Mode::Normal | Mode::Visual) && !self.model.keys;
            let grid = canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| terminal::paint(&screen, bounds, cell, has_keys, window, cx),
            )
            .size_full();
            div()
                .absolute()
                .size_full()
                .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| this.wheel(id, e.delta, cx)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| this.terminal_mouse_down(id, e, cx)),
                )
                .child(grid)
                .into_any_element()
        } else {
            match pane.kind {
                Kind::Terminal => {
                    div().text_sm().text_color(rgb(theme::ERROR)).child("the shell did not start").into_any_element()
                }
                _ => home::empty_pane("⌘E to pick what goes here", id, &self.home).into_any_element(),
            }
        };
        let (radius, cropped) = self.hosted.get(&id).map_or((0.0, false), |h| (h.radius, h.crop > 0.0));
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
                canvas(|_, _, _| {}, move |bounds, _, window, _| paint_corner_mask(bounds, radius, cropped, window))
                    .absolute()
                    .size_full(),
            );
        // The click handler is on the content, not the padded body: a click in the 2pt gap would be
        // replayed at a point the app window does not cover, land on ottospace again, and loop.
        let inner = inner.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| this.click_pane(id, e, window, cx)),
        );
        // With a title bar, the header sits inside the 2pt gap too, so the raised live window's bar
        // covers it exactly and the gap stays on all four sides.
        let body = div().id(("pane", id)).flex_1().min_h_0().when(bar == 0.0, |d| d.p(px(2.))).child(inner);

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
            // In list mode the bar has the keyboard: the focused pane is only where a row would open.
            .border_color(rgb(match (focused, self.model.mode()) {
                (true, Mode::List) => theme::BORDER_STRONG,
                (true, mode) => theme::accent(mode),
                (false, _) => theme::BORDER,
            }))
            .when(faded, |d| d.opacity(0.5))
            .relative()
            .when(bar > 0.0, |d| d.p(px(2.)))
            .child(
                canvas(move |bounds, _, _| frame.set(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full(),
            )
            .child(header)
            .child(body)
    }

    /// Leaves render as panes; splits as a row (`Right`) or column (`Down`), `a` taking `ratio` of it.
    fn render_node(&self, node: &Node, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Leaf(id) => match self.model.pane(*id) {
                Some(pane) => self.render_pane(pane, cx).into_any_element(),
                None => div().into_any_element(),
            },
            Node::Split { axis, ratio, a, b } => {
                let down = *axis == Axis::Down;
                // `a` gets its share of the split; `b` takes what is left.
                let first = div().flex_none().flex().overflow_hidden();
                let first = if down { first.w_full().h(relative(*ratio)) } else { first.h_full().w(relative(*ratio)) };
                let second = div().flex_1().min_w_0().min_h_0().flex().overflow_hidden();
                div()
                    .flex_1()
                    .size_full()
                    .overflow_hidden()
                    .flex()
                    .when(down, |d| d.flex_col())
                    .child(first.child(self.render_node(a, cx)))
                    .child(second.child(self.render_node(b, cx)))
                    .into_any_element()
            }
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
            // The task whose tabs these are. × goes back to the tabs of no task; the task goes on.
            .children(self.model.current_task.and_then(|id| self.model.tasks.get(id)).map(|task| {
                div()
                    .id("task-chip")
                    .h_full()
                    .pl_3()
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_r_1()
                    .border_color(rgb(theme::BORDER))
                    .text_xs()
                    .text_color(rgb(theme::COMPOSE))
                    .child(div().max_w(px(220.)).truncate().child(task.title.to_uppercase()))
                    .child(
                        div()
                            .id("task-leave")
                            .px_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_color(rgb(theme::MUTED))
                            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                            .child("×")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dispatch(Action::TaskLeave, window, cx);
                            })),
                    )
            }))
            // Only the current task's tabs (or those of no task) are on the bar.
            .children(
                self.model
                    .visible_tabs()
                    .into_iter()
                    .filter_map(|index| Some((index, self.model.tabs.get(index)?)))
                    .map(|(index, tab)| {
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
                    }),
            )
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
            // One hint, not the key list: ⌘/ shows the keys of every mode. A mode you can be in
            // by accident also says how to leave. The picker and the new-task box show theirs.
            Mode::Normal => ("NORMAL", "⌘/ keys"),
            Mode::Pick => ("PICK", ""),
            Mode::App => ("APP", app_hint.as_str()),
            Mode::Visual => ("VISUAL", "esc leaves · ⌘/ keys"),
            Mode::List => ("LIST", "esc leaves · ⌘/ keys"),
            Mode::Compose => ("TASK", ""),
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
            // Long text gives way, so the build tag on the right stays in view.
            .child(div().min_w_0().truncate().child(focused))
            .child(div().flex_none().child(hint))
            // Pushes what follows to the right. An auto margin there closed the gaps on the left.
            .child(div().flex_1())
            // The one thing tasks put here: how many wait for the user.
            .children(Some(self.model.tasks.needs_you()).filter(|n| *n > 0).map(|n| {
                let label = if n == 1 { "1 task needs you".to_string() } else { format!("{n} tasks need you") };
                div().text_color(rgb(theme::SNEAK)).child(label)
            }))
            // Shown here, not in the Apps list, so it is visible whichever section is open.
            .children(self.problem().map(|e| div().min_w_0().truncate().text_color(rgb(theme::ERROR)).child(e)))
            // Which build this is, last and dim: it matters when something looks wrong.
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(theme::BORDER_STRONG))
                    .child(version_tag(env!("CARGO_PKG_VERSION"), option_env!("OTTOSPACE_COMMIT"))),
            )
    }
}

/// What a task's row shows: its priority, its title, and its project (at most `tag` points
/// wide) if it has one.
fn task_cells(task: &Task, tag: f32) -> Vec<Div> {
    let urgent = if task.priority == 1 { theme::ERROR } else { theme::MUTED };
    let priority = div().flex_none().text_xs().text_color(rgb(urgent)).child(format!("P{}", task.priority));
    let title = div().flex_1().min_w_0().truncate().child(task.title.clone());
    let project = task.project.as_ref().map(|project| {
        let tag = div().flex_none().max_w(px(tag)).truncate().text_xs();
        tag.text_color(rgb(theme::MUTED)).child(format!("#{project}"))
    });
    [priority, title].into_iter().chain(project).collect()
}

/// A click that shows a task's tabs.
fn open_task(id: TaskId, cx: &mut Context<Workspace>) -> Box<dyn Fn(&ClickEvent, &mut Window, &mut App)> {
    Box::new(cx.listener(move |this, _, window, cx| {
        this.dispatch(Action::TaskOpen { id: Some(id) }, window, cx);
        cx.notify();
    }))
}

/// A row under a heading (a repository's checkout): no outline at rest, so a long list stays
/// quiet. Hover and selection mark it.
fn list_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        // Always there, so a row does not shift by a point when it is selected.
        .border_1()
        .border_color(transparent_black())
        .text_sm()
        .text_color(rgb(theme::MUTED))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
        .when(selected, |row| row.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)).border_color(rgb(theme::LIST)))
}

/// The time now, in milliseconds since 1970. The core has no clock: actions carry this.
fn now_ms() -> u64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    since.map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A row of a secondary-bar list. A button at rest: its outline shows. Hover lifts it.
/// `selected`: list mode's keys act on this row.
fn list_button(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    let lifted = BoxShadow {
        color: black().opacity(0.7),
        offset: point(px(0.), px(3.)),
        blur_radius: px(10.),
        spread_radius: px(0.),
    };
    div()
        .id(id)
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .text_sm()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::SELECTED)).border_color(rgb(theme::BORDER_STRONG)).shadow(vec![lifted]))
        .when(selected, |row| row.bg(rgb(theme::SELECTED)).border_color(rgb(theme::LIST)))
}

/// True if the window rules (the user's, then the built-in ones) let this window be put in a pane.
fn window_managed(window: &ax::Window) -> bool {
    let (bundle, name) = ax::app_identity(window.pid());
    let (title, subrole) = (window.title().unwrap_or_default(), window.subrole().unwrap_or_default());
    let facts =
        Facts { app: name.as_deref().unwrap_or_default(), bundle: bundle.as_deref(), title: &title, subrole: &subrole };
    rules::manages(&config::get().settings.windows, &facts)
}

/// Apps that can be hosted right now. In a test run limited to one app, only that app.
fn running_apps(hosting: bool, only: Option<i32>) -> Vec<capture::App> {
    match (hosting, only) {
        (false, _) => Vec::new(),
        (true, Some(pid)) => {
            // One snapshot: with its real window count, or by name if no window of it counts yet.
            let found: Vec<capture::App> = capture::apps().into_iter().filter(|app| app.pid == pid).collect();
            let named = || capture::name_of(pid).map(|name| capture::App { name, pid, windows: 1 });
            if found.is_empty() { named().into_iter().collect() } else { found }
        }
        (true, None) => capture::apps(),
    }
}

/// Covers the window's rounded corners (transparent, so black in the YUV stream) with the
/// background: a ring whose inner edge is the window's rounded outline, clipped to the pane.
/// `cropped`: the top corners went with the title bar, so only the bottom ones are masked.
fn paint_corner_mask(bounds: Bounds<Pixels>, radius: f64, cropped: bool, window: &mut Window) {
    if radius <= 0.0 {
        return;
    }
    let r = px(radius as f32);
    // Lifting the ring by r puts its rounded top edge above the clip.
    let lift = if cropped { r } else { px(0.) };
    let ring = Bounds { origin: bounds.origin - point(r, r + lift), size: bounds.size + size(r * 2., r * 2. + lift) };
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        window.paint_quad(quad(ring, r * 2., transparent_black(), r, rgb(theme::BACKGROUND), BorderStyle::Solid));
    });
}

/// Lowest global y a raised live window may reach: the bottom of ottospace's top bar.
fn top_limit(window: &Window, cx: &App) -> f64 {
    on_screen(Bounds { origin: point(px(0.), px(TOP_BAR)), size: Size::default() }, window, cx).origin.y
}

/// The window frame for pane content `rect` with a title bar of `crop` above it.
fn raised(rect: CGRect, crop: f64) -> CGRect {
    CGRect {
        origin: CGPoint::new(rect.origin.x, rect.origin.y - crop),
        size: CGSize::new(rect.size.width, rect.size.height + crop),
    }
}

/// The window's croppable title-bar height from AX (0: none or unreadable); see `titlebar::height`.
fn title_bar(window: &ax::Window) -> f64 {
    let rect = |r: CGRect| Rect {
        x: r.origin.x as f32,
        y: r.origin.y as f32,
        w: r.size.width as f32,
        h: r.size.height as f32,
    };
    let Some(frame) = window.frame() else { return 0.0 };
    let facts = TitleBar {
        window: rect(frame),
        close: window.part("AXCloseButton").map(rect),
        title: window.part("AXTitleUIElement").map(rect),
        toolbar: window.has_child("AXToolbar").unwrap_or(true),
    };
    titlebar::height(&facts) as f64
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
            self.frames.entry(pane.id).or_default();
        }
        let tree = self.model.layout.as_ref().map(|root| self.render_node(root, cx));
        // Only while the tab on show has no panes. Its lines are built once and kept: a frame
        // that moves nothing (a terminal printing in another tab) only paints them again.
        let keep_out = self.home.borrow().keep_out();
        let home = self.model.home(&keep_out).map(|home| self.render_home(home, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            // On the root, so a selection drag keeps going over other panes.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            // A release ends the drag, so a later drag elsewhere does not extend this selection.
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = None))
            // Any click leaves visual mode first, before the clicked element acts: visual mode has no
            // backdrop, and e.g. a click on an app pane must go live, not be replayed while blocked.
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                if this.model.visual.is_some() {
                    this.dispatch(Action::LeaveVisual, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &PasteText, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &VisualMode, window, cx| {
                this.dispatch(Action::ToggleVisual, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ListMode, window, cx| {
                this.refresh_lists();
                this.dispatch(Action::FocusList, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewTask, window, cx| this.new_task(window, cx)))
            .on_action(cx.listener(|this, _: &NextTask, window, cx| {
                this.dispatch(Action::Tick { now: now_ms() }, window, cx);
                this.dispatch(Action::TaskOpen { id: None }, window, cx);
            }))
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
            .on_action(cx.listener(|this, _: &AxOverlay, window, cx| {
                if let Err(err) = this.toggle_ax_overlay(false, window, cx) {
                    this.error = Some(err);
                    cx.notify();
                }
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
            .on_action(cx.listener(|this, _: &ResizeLeft, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Left }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeRight, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Right }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeUp, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Up }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeDown, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Down }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapLeft, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Left }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapRight, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Right }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapUp, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Up }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapDown, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Down }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Equalize, window, cx| {
                this.dispatch(Action::Equalize, window, cx);
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
                            // Let the canvas shrink below its content, so a wide image cannot
                            // push it past the window; it scales down instead.
                            .min_w_0()
                            .h_full()
                            .flex()
                            .overflow_hidden()
                            .children(home)
                            .children(tree),
                    ),
            )
            .child(self.render_status())
            // Only over a parked, drawn pane: a live window sits above ottospace and would cover it.
            .children(
                self.ax_overlay
                    .as_ref()
                    .filter(|o| self.model.ax_overlay && self.model.live != Some(o.pane))
                    .filter(|o| self.model.layout.as_ref().is_some_and(|l| l.leaves().contains(&o.pane)))
                    .map(ax_debug::Overlay::render),
            )
            .children(self.model.picker.as_ref().map(|picker| self.render_picker(picker, cx)))
            .children(self.model.compose.as_ref().map(|line| self.render_compose(line, cx)))
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

/// The `--version` line. `scripts/package.sh` passes the commit and its date in at compile time;
/// a plain `cargo build` has neither.
fn version_line(version: &str, commit: Option<&str>, date: Option<&str>) -> String {
    match (commit.filter(|c| !c.is_empty()), date.filter(|d| !d.is_empty())) {
        (Some(commit), Some(date)) => format!("ottospace {version} ({commit}, {date})"),
        (Some(commit), None) => format!("ottospace {version} ({commit})"),
        _ => format!("ottospace {version} (local build)"),
    }
}

/// The build, short, for the status bar: `0.1.0 · a4a2582` (seven characters of the commit), or
/// `0.1.0 · local` for a plain `cargo build`.
fn version_tag(version: &str, commit: Option<&str>) -> String {
    match commit.filter(|c| !c.is_empty()) {
        Some(commit) => format!("{version} · {}", commit.chars().take(7).collect::<String>()),
        None => format!("{version} · local"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "ctl") {
        ctl(&args[2..]);
    }
    // `ottospace skill` prints the agent guide; `--stub` prints the SKILL.md that points to it.
    if args.get(1).is_some_and(|a| a == "skill") {
        let stub = args.get(2).is_some_and(|a| a == "--stub");
        print!("{}", if stub { SKILL_STUB } else { SKILL_GUIDE });
        return;
    }
    // Which build this is, e.g. "ottospace 0.1.0 (a4a2582c0ffe, 2026-11-12T08:09:00Z)".
    if args.get(1).is_some_and(|a| a == "--version" || a == "-V") {
        let (commit, date) = (option_env!("OTTOSPACE_COMMIT"), option_env!("OTTOSPACE_BUILD_DATE"));
        println!("{}", version_line(env!("CARGO_PKG_VERSION"), commit, date));
        return;
    }
    // Anything else is a mistake (`ottospace skills`, `--help`). Starting the app for it would
    // open a second window and restore the running instance's hosted windows.
    if let Some(unknown) = args.get(1) {
        eprintln!("unknown command: {unknown}\nusage: ottospace [ctl '<json request>' | skill [--stub] | --version]");
        std::process::exit(2);
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
        // Run as a bare binary (`ottospace`, `cargo run`) there is no bundle to supply the icon.
        ottospace_mac::window::set_dock_icon(include_bytes!("../assets/ottospace.icns"));
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
            // Resize the focused pane's split, and even all splits out.
            KeyBinding::new("cmd-ctrl-left", ResizeLeft, None),
            KeyBinding::new("cmd-ctrl-right", ResizeRight, None),
            KeyBinding::new("cmd-ctrl-up", ResizeUp, None),
            KeyBinding::new("cmd-ctrl-down", ResizeDown, None),
            KeyBinding::new("cmd-ctrl-h", ResizeLeft, None),
            KeyBinding::new("cmd-ctrl-l", ResizeRight, None),
            KeyBinding::new("cmd-ctrl-k", ResizeUp, None),
            KeyBinding::new("cmd-ctrl-j", ResizeDown, None),
            KeyBinding::new("cmd-ctrl-=", Equalize, None),
            // Swap the focused pane with its neighbour. ⌘⇧⌥1..9 move it to a tab (`key_down`).
            KeyBinding::new("cmd-shift-alt-left", SwapLeft, None),
            KeyBinding::new("cmd-shift-alt-right", SwapRight, None),
            KeyBinding::new("cmd-shift-alt-up", SwapUp, None),
            KeyBinding::new("cmd-shift-alt-down", SwapDown, None),
            KeyBinding::new("cmd-shift-alt-h", SwapLeft, None),
            KeyBinding::new("cmd-shift-alt-l", SwapRight, None),
            KeyBinding::new("cmd-shift-alt-k", SwapUp, None),
            KeyBinding::new("cmd-shift-alt-j", SwapDown, None),
            // Fill the screen without leaving the Space (see `zoom_instead_of_fullscreen`).
            KeyBinding::new("cmd-ctrl-f", Zoom, None),
            // Debug: outline the focused app pane's accessibility tree.
            KeyBinding::new("cmd-ctrl-a", AxOverlay, None),
            // Tabs. ⌘1..⌘9 select one (handled in `key_down`).
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-}", NextTab, None),
            KeyBinding::new("cmd-{", PrevTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("cmd-shift-w", CloseTab, None),
            // Terminal selection. ⌘C never sends ^C: that is ⌃C.
            KeyBinding::new("cmd-c", CopySelection, None),
            KeyBinding::new("cmd-v", PasteText, None),
            KeyBinding::new("cmd-shift-c", VisualMode, None),
            KeyBinding::new("cmd-shift-e", ListMode, None),
            KeyBinding::new("cmd-n", NewTask, None),
            KeyBinding::new("cmd-shift-n", NextTask, None),
            KeyBinding::new("cmd-h", Hide, None),
        ]);
        // The menu bar lists the same actions; the app-level ones are handled here.
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.set_menus(menus::app_menus());
        let mut bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        // A test run stays out of the user's way: a background app that never takes the keyboard,
        // with its window at a free corner of the screens, all but a sliver of it off screen.
        // (A sliver, because a window with nothing showing is not drawn, and tests wait on layout.)
        // `OTTOSPACE_E2E_SHOW` keeps the window where it can be looked at.
        let quiet = !hosting;
        if quiet {
            ottospace_mac::window::stay_in_background();
            if std::env::var_os("OTTOSPACE_E2E_SHOW").is_none() {
                let corner = display::parking_spot();
                bounds.origin = point(px(corner.x as f32 - E2E_SLIVER), px(corner.y as f32 - E2E_SLIVER));
            }
        }
        // Draw under the title bar; the top bar leaves room for the traffic lights.
        let titlebar = TitlebarOptions {
            title: Some("ottospace".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(10.), px(7.))),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(titlebar),
            focus: !quiet,
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

    #[test]
    fn version_line_names_the_commit_when_the_build_was_given_one() {
        assert_eq!(
            version_line("0.1.0", Some("a4a2582c0ffe"), Some("2026-11-12T08:09:00Z")),
            "ottospace 0.1.0 (a4a2582c0ffe, 2026-11-12T08:09:00Z)"
        );
        assert_eq!(version_line("0.1.0", Some("a4a2582c0ffe"), None), "ottospace 0.1.0 (a4a2582c0ffe)");
        assert_eq!(version_line("0.1.0", None, None), "ottospace 0.1.0 (local build)");
        assert_eq!(version_line("0.1.0", Some(""), Some("")), "ottospace 0.1.0 (local build)");
        // The status bar's short form.
        assert_eq!(version_tag("0.1.0", Some("a4a2582c0ffe")), "0.1.0 · a4a2582");
        assert_eq!(
            (version_tag("0.1.0", None), version_tag("0.1.0", Some(""))),
            ("0.1.0 · local".into(), "0.1.0 · local".into())
        );
    }

    /// Every `ottospace ctl '<json>'` line in the agent guide must be a request the app accepts,
    /// so the guide cannot drift from the socket protocol.
    #[test]
    fn every_request_in_the_agent_guide_parses() {
        let examples: Vec<&str> = SKILL_GUIDE
            .lines()
            .filter_map(|line| line.trim().strip_prefix("ottospace ctl '"))
            .filter_map(|rest| rest.split_once('\'').map(|(json, _)| json))
            .collect();
        assert!(examples.len() > 20, "found {} examples", examples.len());
        for json in examples {
            let parsed: Result<Request, _> = serde_json::from_str(json);
            assert!(parsed.is_ok(), "the guide shows a request the app refuses: {json}: {:?}", parsed.err());
        }
    }

    /// Every request the app accepts must have an example in the guide. The names come from
    /// serde's own error for an unknown command, so a new `Request` variant fails this test.
    #[test]
    fn the_agent_guide_covers_every_request() {
        let error = serde_json::from_str::<Request>(r#"{"cmd":"?"}"#).expect_err("? is not a command").to_string();
        let names: Vec<&str> = error.split('`').skip(3).step_by(2).collect();
        assert!(names.len() > 20, "could not read the command names from: {error}");
        for name in names {
            assert!(SKILL_GUIDE.contains(&format!(r#""cmd":"{name}""#)), "the guide has no example of `{name}`");
        }
    }

    /// A missing asset draws as nothing, with no error. Catch a renamed file here.
    #[test]
    fn every_rail_icon_is_embedded() {
        let icons = SECTIONS.iter().map(|(_, icon, _)| *icon).chain(SPLITS.iter().map(|(icon, ..)| *icon));
        for path in icons {
            assert!(Assets.load(path).unwrap().is_some(), "{path} is not in Assets::load");
        }
    }
}
