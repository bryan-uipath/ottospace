//! Imperative shell: draws the workspace, hosts other apps' windows in panes, and turns input
//! (mouse, keys, control socket) into `ottospace_core` actions. State rules live in the core.
//!
//! An app pane shows its parked window's SCK stream. Clicking it moves the real window over the
//! pane ("live") and replays the click; clicking ottospace itself parks it again.

// The state snapshot is one `json!` with more keys than the default limit expands.
#![recursion_limit = "512"]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use conversation::NativeInput;
use core_foundation::base::TCFType;
use core_video::pixel_buffer::{CVPixelBuffer, CVPixelBufferRef};
use gpui::{
    AnyElement, AnyView, App, Application, AssetSource, BorderStyle, Bounds, BoxShadow, ClickEvent, ContentMask,
    Context, Div, ElementId, FocusHandle, FontWeight, KeyBinding, KeyDownEvent, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString, Size, Stateful,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, black, canvas, div, img, point,
    prelude::*, px, quad, relative, rgb, size, surface, svg, transparent_black,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_core::agent_mode::AppUse;
use ottospace_core::calendar::DayStep;
use ottospace_core::clip::{self, Verb};
use ottospace_core::compose;
use ottospace_core::edit::{self, Edit};
use ottospace_core::features::Feature;
use ottospace_core::fill;
use ottospace_core::hosted::{self as hosting, Click};
use ottospace_core::key_use::{Binding, Line};
use ottospace_core::keys::{self, FilterKey, Mods};
use ottospace_core::list::{ListKey, list_key};
use ottospace_core::marks::{Field, MarkInput, MarkOf, MarkRef, letter};
use ottospace_core::model::reorder::ReorderItem;
use ottospace_core::model::zones::RailFocus;
use ottospace_core::model::{CONFIRM_UP, Glyph, Owner, Space, TabAt};
use ottospace_core::notice::{self, Notice};
use ottospace_core::repos::{Checkout, Repo};
use ottospace_core::row_menu::MenuOf;
use ottospace_core::rules::{self, Facts};
use ottospace_core::scrollbar::{Extent, key_step};
use ottospace_core::settings::Installed;
use ottospace_core::sneak::{SneakKey, sneak_key};
use ottospace_core::tasks::{
    Assign, Group, Heading, MAX_TITLE, PENDING_LINES, Pending, RowAction, State, Task, TaskId, TaskRow,
};
use ottospace_core::terminal::{CellPoint, VisualKey, cell_at, clears_selection, edge_scroll, press, visual_key};
use ottospace_core::{
    Action, Axis, Dir, Effect, Kind, Mode, Model, Node, Origin, Pane, PaneId, PickItem, Picker, Rect, Section, Zone,
    cadence, theme,
    titlebar::{self, TitleBar},
};
use ottospace_mac::window::{Mouse, MouseKind};
use ottospace_mac::{ax, capture, display, hotkey, icon, input, recovery};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use screencapturekit::cm::CMSampleBufferSCExt;
use screencapturekit::prelude::*;

mod agent_mode;
mod app_wheel;
mod apps;
mod ax_debug;
mod btw;
mod bug;
mod calendar;
mod chat_select;
mod child;
mod city;
mod city_gpu;
mod clipboard;
mod collect;
mod commands;
mod config;
mod confirm;
mod control;
mod conversation;
mod decisions;
mod dial;
mod dialog;
mod divider;
mod drawer_rail;
mod editor;
mod event_log;
mod features;
mod field;
mod finish;
mod float_panel;
mod give_back;
mod highlight;
mod home;
mod inspector;
mod key_use;
mod keycaps;
mod layout_file;
mod listen;
mod listen_file;
mod listen_live;
mod listen_meter;
mod markdown;
mod marks;
mod menus;
mod new_task;
mod notices;
mod out_window;
mod overlays;
mod pass;
mod pick;
mod picture;
mod pop_ups;
mod progress;
mod prs;
mod record;
mod reorder;
mod repos;
mod row_menu;
mod running;
#[cfg(test)]
mod scratch;
mod scrollbar;
mod seal;
mod sessions;
mod signal_pipe;
mod slide;
mod slot_panes;
mod sneak;
mod sneak_apps;
mod stall;
mod status_bar;
mod syntax;
mod tags;
mod talk;
mod terminal;
mod toast;
mod trace;
mod ui;
mod viewer;
mod words;
mod workbook;
mod worktree;
use control::{Call, MouseAction, PaneKind, Request};
use terminal::Terminal;

/// Latest captured frame, handed from SCK's queue to the UI thread.
struct Frame(CVPixelBuffer);
// CVPixelBuffer is a CF object; retain/release are thread-safe.
unsafe impl Send for Frame {}

/// The part of the window a stream frame shows: its size in points, below `crop` points.
#[derive(Clone, Copy, Default)]
struct Shows {
    size: (f64, f64),
    crop: f64,
}

/// The frame held and what it shows, and what the stream is set to show. One lock: a frame is
/// checked against, and stamped with, the configuration of its time.
#[derive(Default)]
struct Held {
    frame: Option<(Frame, Shows)>,
    asked: Shows,
}

#[derive(Default)]
struct Latest {
    frame: Mutex<Held>,
    dirty: AtomicBool,
    /// Configured stream size as `width << 32 | height`. Frames of any other size are transitional
    /// (mid-reconfigure or mid-move) and render as a small window on black, so they're dropped.
    expected: AtomicU64,
    /// When the stream last said anything, a frame or "nothing changed" (milliseconds since
    /// 1970). It says nothing while its window is not drawn: minimised, or on another Space.
    seen: AtomicU64,
    /// What the stream has sent, frames and "nothing changed": a test run shows that a live
    /// window's stream sends none and starts again as it parks.
    samples: AtomicU64,
    /// The frame held is from before the stream last started: a capture waits for a new one.
    old: AtomicBool,
}

impl Latest {
    fn expect(&self, config: &SCStreamConfiguration) {
        let source = config.source_rect();
        if let Ok(mut held) = self.frame.lock() {
            held.asked = Shows { size: (source.size.width, source.size.height), crop: source.origin.y };
            self.expected.store((config.width() as u64) << 32 | config.height() as u64, Relaxed);
        }
    }
}

struct Output(Arc<Latest>);

impl SCStreamOutputTrait for Output {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, _: SCStreamOutputType) {
        self.0.seen.store(now_ms(), Relaxed);
        self.0.samples.fetch_add(1, Relaxed);
        let ptr = sample.image_buffer_ptr_borrowed();
        if ptr.is_null() {
            return; // idle frames carry no image
        }
        let buffer = unsafe { CVPixelBuffer::wrap_under_get_rule(ptr as CVPixelBufferRef) };
        let size = (buffer.get_width() as u64) << 32 | buffer.get_height() as u64;
        // Held only if of the size asked, its content of the asked shape (not the old window scaled
        // to fit); one held that no longer shows what is asked is drawn at its own size (`held`).
        if let Ok(mut held) = self.0.frame.lock() {
            let (w, h) = held.asked.size;
            let shape = if h > 0.0 { w / h } else { 0.0 };
            let content = sample.content_rect().map(|c| (c.size.width, c.size.height));
            if size != self.0.expected.load(Relaxed) || content.is_some_and(|c| !hosting::of_shape(shape, c)) {
                return;
            }
            held.frame = Some((Frame(buffer), held.asked));
            self.0.old.store(false, Relaxed);
            self.0.dirty.store(true, Relaxed);
        }
    }
}

/// Stream config showing `size` points of the window at `scale` (Retina = 2), from its top left
/// below its top `crop` points (the title bar), never scaled: a window that refused the pane's
/// size (Outlook's minimum) looks as it does when it is live. `scale` is the pane's display's:
/// the picture has the pixels the pane is drawn with (`hosted::capture_pixels`).
fn stream_config(size: CGSize, crop: f64, scale: f64) -> SCStreamConfiguration {
    let (width, height) = hosting::capture_pixels((size.width, size.height), scale);
    // 420f is the only format gpui's surface renderer accepts.
    SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_scales_to_fit(true)
        .with_pixel_format(PixelFormat::YCbCr_420f)
        .with_fps(60)
        .with_source_rect(screencapturekit::cg::CGRect::new(0.0, crop, size.width, size.height))
}

fn to_cg(size: gpui::Size<Pixels>) -> CGSize {
    CGSize::new(f32::from(size.width) as f64, f32::from(size.height) as f64)
}

/// Shell half of an app pane: the window, its stream and parking state.
struct Hosted {
    window_id: u32,
    title: String,
    /// Whose window it is: the app's bundle id, if it has one, and its name as the Apps list has it.
    bundle: Option<String>,
    app: String,
    window: ax::Window,
    original: CGRect,
    /// The window was off screen when hosted: `original` is the middle of the main screen.
    off_screen: bool,
    latest: Arc<Latest>,
    stream: SCStream,
    /// The stream is stopped while the window is live: a captured window loses quick clicks
    /// (Finder misses double-clicks). Holds what the stream was configured to show then.
    paused: Option<(CGSize, f64)>,
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
    /// The part of the window the stream shows, in points: no larger than the pane or the window.
    shown: CGSize,
    /// The title-bar crop the stream shows it with.
    shown_crop: f64,
    /// The pixels a point the stream captures at: ottospace's display's when it was configured.
    shown_scale: f64,
    /// The window's content size when `shown` was last worked out, for the corner mask.
    content: CGSize,
    /// How often `window_frame` read the window's frame.
    frame_reads: u64,
    /// The last size, crop and scale the stream refused, so the same one is not asked for every check.
    refused: Option<(CGSize, f64, f64)>,
    /// Where the preview was last drawn, in window-content coordinates.
    preview: Rc<Cell<Bounds<Pixels>>>,
    /// Last pane content rect the live window was placed over, to re-place only on change.
    placed: CGRect,
    /// Where it was last parked: it parks again when the spot moves (`display::parking_spot`).
    parked_at: CGPoint,
    /// The width the window kept when its pane asked for less: no split may leave it narrower.
    min_width: Option<f64>,
}

impl Hosted {
    /// Every way a window is hosted: a failure goes in the log, by the window's number only (why
    /// may be ScreenCaptureKit's text, which the log cannot vouch for).
    fn new(target: capture::Target) -> Result<Self, String> {
        let id = target.id;
        Self::start(target).inspect_err(|_| event_log::event("host_failed", serde_json::json!({ "window": id })))
    }

    fn start(target: capture::Target) -> Result<Self, String> {
        let capture::Target { window, sc_window, id } = target;
        let found = window.frame().ok_or("could not read window frame")?;
        // Found parked or off screen (left so by a crash, or a window the app re-made where the
        // old one was parked): it goes back to the middle of the main screen instead.
        let screens = recovery::screens();
        let off_screen = !ottospace_core::restore::reachable(recovery::to_frame(found), &screens.visible);
        let original = match off_screen {
            true => recovery::to_rect(ottospace_core::restore::centred(recovery::to_frame(found), screens.main)),
            false => found,
        };
        let latest = Arc::new(Latest::default());
        let radius = capture::corner_radius(&sc_window, 2.0).unwrap_or(0.0);
        let bar = title_bar(&window);
        eprintln!(
            "hosting window {id}: corner radius {radius:.1}pt, title bar {bar}pt, found at {found:?}, off screen {off_screen}"
        );
        let filter = SCContentFilter::create().with_window(&sc_window).build().map_err(|e| format!("{e:?}"))?;
        // Resized to the pane on park.
        let config = stream_config(original.size, 0.0, 2.0);
        latest.expect(&config);
        let mut stream = SCStream::new(&filter, &config).map_err(|e| format!("{e:?}"))?;
        stream.add_output_handler(Output(latest.clone()), SCStreamOutputType::Screen).map_err(|e| format!("{e:?}"))?;
        stream.start_capture().map_err(|e| format!("{e:?}"))?;
        let title = window.title().filter(|t| !t.is_empty()).unwrap_or_else(|| format!("window {id}"));
        let pid = window.pid();
        let (bundle, _) = ax::app_identity(pid);
        let app = capture::name_of(pid).unwrap_or_default();
        give_back::remember(pid, id, recovery::to_frame(original));
        Ok(Self {
            window_id: id,
            off_screen,
            title,
            bundle,
            app,
            window,
            original,
            latest,
            paused: None,
            stream,
            pid,
            radius,
            bar,
            crop: 0.0,
            bounds: Rc::default(),
            parked_size: CGSize::ZERO,
            shown: original.size,
            shown_crop: 0.0,
            shown_scale: 2.0,
            content: original.size,
            frame_reads: 0,
            refused: None,
            preview: Rc::default(),
            placed: CGRect::default(),
            parked_at: CGPoint::ZERO,
            min_width: None,
        })
    }

    fn kind(&self) -> Kind {
        let (bundle, name) = (self.bundle.clone(), self.app.clone());
        Kind::App { window: self.window_id, title: self.title.clone(), bundle, name }
    }

    /// Stops capture and puts the window back where we found it; forgets it once it is there.
    fn release(self) {
        let id = self.window_id;
        if self.give_back() {
            give_back::forget(id);
        }
    }

    /// As `release`, but the record stays: at a quit, for the guardian to check again once
    /// ottospace is gone. True when the window is back where the user can reach it.
    fn give_back(self) -> bool {
        let _ = self.stream.stop_capture();
        let back = recovery::put_back(&self.window, recovery::to_frame(self.original), &recovery::screens());
        if back.is_none() {
            eprintln!("window {}: not back on screen", self.window_id);
            event_log::event("give_back_failed", serde_json::json!({ "window": self.window_id }));
        }
        back.is_some()
    }

    /// Sizes the window to the pane (so the stream matches) and parks it on the off-screen sliver.
    /// With a `crop`, the window is that much taller and the stream shows only the part below it.
    fn park(&mut self, size: CGSize, scale: f64, crop: f64) {
        // On a display of the pane's scale, so the app draws the window at it.
        let spot = display::parking_spot(scale);
        // Only resize/reconfigure when the pane changed; redoing it on every park flickers.
        if size != self.parked_size || crop != self.crop {
            // Move first. Resized where it stood, macOS clamps the window to what fits above the
            // Dock there (a pane 818pt tall parked at 805pt), and the stream no longer matches.
            let _ = self.window.set_position(spot);
            let _ = self.window.set_size(CGSize::new(size.width, size.height + crop));
        }
        self.parked_size = size;
        self.crop = crop;
        self.show(scale);
        // A refused move (a busy app) is tried again at the next check.
        if self.window.set_position(spot).is_ok() {
            self.parked_at = spot;
        }
        self.placed = CGRect::default();
    }

    /// Makes the stream show what the pane and the window share now: a window that refused the
    /// pane's size is cut off (larger) or leaves a gap (smaller), never scaled. Reads the window's
    /// real size, so it also follows an app that resized itself. One accessibility read; the
    /// stream is reconfigured only when the result changed. Returns the window's content size.
    fn show(&mut self, scale: f64) -> Option<CGSize> {
        let pane = to_cg(self.bounds.get().size);
        let frame = self.window_frame().filter(|_| pane.width >= 1.0)?;
        let content = CGSize::new(frame.size.width, frame.size.height - self.crop);
        let shown = CGSize::new(pane.width.min(content.width), pane.height.min(content.height));
        let asked = (shown, self.crop);
        // Live: the click rule and the mask follow the window; `resume` configures the stream.
        if self.paused.is_some() {
            (self.shown, self.shown_crop, self.content) = (shown, self.crop, content);
            return Some(content);
        }
        // Also on a new scale: ottospace moved to a display of another, and the old picture
        // would be drawn up (blurred) or down.
        if asked == (self.shown, self.shown_crop) && scale == self.shown_scale {
            self.content = content;
            // Back to what it shows: a refusal of another is asked again when it comes back.
            self.refused = None;
            return Some(content);
        }
        if self.refused == Some((shown, self.crop, scale)) {
            return Some(content);
        }
        let config = stream_config(shown, self.crop, scale);
        // Frames of another size are dropped, so expect the new one only once it is set.
        if self.stream.update_configuration(&config).is_ok() {
            self.latest.expect(&config);
            (self.shown, self.shown_crop, self.shown_scale) = (shown, self.crop, scale);
            self.content = content;
            self.refused = None;
        } else {
            // `shown` keeps describing what the stream still shows.
            self.refused = Some((shown, self.crop, scale));
            eprintln!("window {}: the stream refused {}×{}", self.window_id, shown.width, shown.height);
        }
        Some(content)
    }

    /// The window's real frame: a synchronous accessibility call, counted so a test can tell
    /// a read each check from a read each tick.
    fn window_frame(&mut self) -> Option<CGRect> {
        self.frame_reads += 1;
        self.window.frame()
    }

    /// Places the window over pane content `rect`, raised by `crop` so its title bar covers the header.
    fn place(&mut self, rect: CGRect, crop: f64) {
        let frame = raised(rect, crop);
        let _ = self.window.set_size(frame.size);
        let _ = self.window.set_position(frame.origin);
        self.placed = rect;
        // Live shorter than parked (above the Dock while the window fills its display): the next
        // park sizes it again.
        if (rect.size, crop) != (self.parked_size, self.crop) {
            self.parked_size = CGSize::ZERO;
        }
    }

    /// Places the window over its pane and activates its app, on entering. The app may
    /// have resized its window since it was parked: a click is judged against what it covers now.
    fn bring_up(&mut self, keep: Option<ottospace_core::ax::Frame>, window: &Window, cx: &App) {
        let rect = on_screen(self.bounds.get(), window, cx);
        let crop = self.crop_at(rect, top_limit(window, cx));
        self.place(pane_rect(rect, keep, true, crop), crop);
        self.window.raise();
        self.show(window.scale_factor() as f64);
    }

    /// Stops the stream while the window is live: the real window is on screen, and a captured
    /// one loses quick clicks.
    fn pause(&mut self) {
        if self.paused.is_some() {
            return;
        }
        match self.stream.stop_capture() {
            Ok(()) => self.paused = Some((self.shown, self.shown_crop)),
            Err(_) => event_log::event("stream_stop_failed", serde_json::json!({ "window": self.window_id })),
        }
    }

    /// Starts the stream again once the window is parked, configured for what the pane shows.
    /// A failed start stays paused, and the next park tries again.
    fn resume(&mut self, scale: f64) {
        let Some(streamed) = self.paused else { return };
        // Here, not in `pause`: a frame sent as the stream stopped may land after the stop.
        self.latest.old.store(true, Relaxed);
        if self.stream.start_capture().is_err() {
            return event_log::event("stream_start_failed", serde_json::json!({ "window": self.window_id }));
        }
        self.paused = None;
        // What the stream shows; `show` configures it for the pane now.
        (self.shown, self.shown_crop) = streamed;
        self.show(scale);
    }

    /// The crop for this window with its pane content at `rect`; see `titlebar::crop`.
    fn crop_at(&self, rect: CGRect, limit: f64) -> f64 {
        titlebar::crop(self.bar as f32, rect.origin.y as f32, limit as f32) as f64
    }

    fn frame(&self) -> Option<CVPixelBuffer> {
        self.latest.frame.lock().ok()?.frame.as_ref().map(|f| f.0.0.clone())
    }

    /// The frame held; and, when it shows another part of the window than the stream now does,
    /// where it is drawn and whether its top is cut.
    fn held(&self) -> Option<(CVPixelBuffer, Option<([f64; 4], bool)>)> {
        let held = self.latest.frame.lock().ok()?;
        let (Frame(buffer), shows) = held.frame.as_ref()?;
        let stale = hosting::stale_at(shows.size, shows.crop, held.asked.size, held.asked.crop);
        Some((buffer.clone(), stale.map(|at| (at, shows.crop > 0.0))))
    }
}

/// Icons and images, compiled into the binary.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "icons/apps.svg" => Some(include_bytes!("../assets/icons/apps.svg")),
            "icons/robot.svg" => Some(include_bytes!("../assets/icons/robot.svg")),
            "icons/home.svg" => Some(include_bytes!("../assets/icons/home.svg")),
            "icons/workbook.svg" => Some(include_bytes!("../assets/icons/workbook.svg")),
            "icons/tasks.svg" => Some(include_bytes!("../assets/icons/tasks.svg")),
            "icons/repos.svg" => Some(include_bytes!("../assets/icons/repos.svg")),
            "icons/pull-request.svg" => Some(include_bytes!("../assets/icons/pull-request.svg")),
            "icons/marks.svg" => Some(include_bytes!("../assets/icons/marks.svg")),
            "icons/calendar.svg" => Some(include_bytes!("../assets/icons/calendar.svg")),
            "icons/folder.svg" => Some(include_bytes!("../assets/icons/folder.svg")),
            "icons/waveform.svg" => Some(include_bytes!("../assets/icons/waveform.svg")),
            "icons/split-right.svg" => Some(include_bytes!("../assets/icons/split-right.svg")),
            "icons/display-out.svg" => Some(include_bytes!("../assets/icons/display-out.svg")),
            "icons/fold-in.svg" => Some(include_bytes!("../assets/icons/fold-in.svg")),
            "icons/split-down.svg" => Some(include_bytes!("../assets/icons/split-down.svg")),
            "icons/plus.svg" => Some(include_bytes!("../assets/icons/plus.svg")),
            "icons/fill-expand.svg" => Some(include_bytes!("../assets/icons/fill-expand.svg")),
            "icons/fill-collapse.svg" => Some(include_bytes!("../assets/icons/fill-collapse.svg")),
            "icons/dismiss.svg" => Some(include_bytes!("../assets/icons/dismiss.svg")),
            "icons/slide-left.svg" => Some(include_bytes!("../assets/icons/slide-left.svg")),
            "icons/slide-right.svg" => Some(include_bytes!("../assets/icons/slide-right.svg")),
            "icons/trash.svg" => Some(include_bytes!("../assets/icons/trash.svg")),
            "icons/fold-open.svg" => Some(include_bytes!("../assets/icons/fold-open.svg")),
            "icons/fold-closed.svg" => Some(include_bytes!("../assets/icons/fold-closed.svg")),
            "icons/runner-claude.svg" => Some(include_bytes!("../assets/icons/runner-claude.svg")),
            "icons/runner-codex.svg" => Some(include_bytes!("../assets/icons/runner-codex.svg")),
            "icons/bug.svg" => Some(include_bytes!("../assets/icons/bug.svg")),
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

/// The window while it fills its display, in Accessibility's coordinates.
#[derive(Clone, Copy)]
struct Filled {
    /// Its frame before.
    before: ottospace_core::ax::Frame,
    /// The frame the mode gave it.
    frame: ottospace_core::ax::Frame,
    /// What the menu bar and the Dock leave of the display when they show.
    visible: ottospace_core::ax::Frame,
    /// The window has been given `frame`: until then it is not looked at (`fill_holds`).
    set: bool,
    /// Where the window was as the mode was turned off, before the bars came back.
    left: Option<ottospace_core::ax::Frame>,
}

/// Height of the top bar (the native title strip, so all of it drags the window), and the width
/// it keeps clear on the left for the traffic lights.
const TOP_BAR: f32 = 40.;
/// The second row: the selected space's tabs.
const SPACE_TABS: f32 = 30.;
/// Height of the status bar, at the window's foot.
const STATUS_BAR: f32 = 24.;
/// How narrow a space of the top row gets before the row clips, and how wide.
const MIN_SPACE: f32 = 72.;
/// What a space wants with its title: past the row's room for that many, the others are badges.
const TITLED_SPACE: f32 = 150.;
const MAX_SPACE: f32 = 220.;
/// Home's item on the top row: a house.
const HOME_ICON: &str = "icons/home.svg";
/// The top row's Fill the Whole Display button: off (arrows out), on (arrows in).
const FILL_ICONS: [&str; 2] = ["icons/fill-expand.svg", "icons/fill-collapse.svg"];
/// The bug button's icon, and its key (in its tooltip only).
const BUG_ICON: &str = "icons/bug.svg";
const BUG_KEY: &str = "cmd-ctrl-b";
/// A space drawn as its badge alone: its icon.
const BADGE_SPACE: f32 = 36.;
/// Home's item: its 16-point house, 12 points either side, and its border.
const HOME_SPACE: f32 = 41.;
const TRAFFIC_LIGHTS: f32 = 80.;
/// The narrowest a tab shrinks to when the strip is full: a few letters of its title and its ×.
const MIN_TAB: f32 = 80.;
/// The left rail's width, and the section list's beside it.
const RAIL: f32 = 56.;
const SECTION: f32 = 260.;
/// Height of every rail cell (left and right) and of the section panel's header: the open cell
/// and the header end on one line.
const RAIL_CELL: f32 = 40.;
/// The drawer is never narrower than this, unless the window is.
const MIN_DRAWER: f32 = ottospace_core::talk::MIN_WIDTH;
/// Height of a pane's header (label and close button).
const PANE_HEADER: f32 = 28.;
/// The least width a pane header keeps for its tag and title, e.g. `APP Calc…`.
const HEADER_TITLE_MIN: f32 = 72.;

/// Top-bar buttons: the name in the control's id (`top:split_right`), icon, tooltip, split
/// direction. No + for a tab, only ⌘T: two +s (workspace, tab) confused the owner.
const SPLITS: [(&str, &str, &str, Axis); 2] = [
    ("split_right", "icons/split-right.svg", "Split right…  ⌘E", Axis::Right),
    ("split_down", "icons/split-down.svg", "Split down…  ⌘⇧E", Axis::Down),
];

/// A rail cell's control id: `rail:tasks`. The Workspaces cell keeps `rail:workbook`, as the
/// requests and the state keep that name.
pub(crate) fn rail_id(label: &str) -> String {
    let name = if label == "Workspaces" { "workbook".into() } else { label.to_lowercase() };
    format!("rail:{name}")
}

/// Rail sections in order, with their icon and label.
const SECTIONS: [(Section, &str, &str); 6] = [
    (Section::Repos, "icons/repos.svg", "Repos"),
    (Section::Tasks, "icons/tasks.svg", "Tasks"),
    (Section::Calendar, "icons/calendar.svg", "Calendar"),
    (Section::Apps, "icons/apps.svg", "Apps"),
    (Section::Workbook, "icons/workbook.svg", "Workspaces"),
    (Section::Marks, "icons/marks.svg", "Marks"),
];

/// A small label shown when the pointer rests on a rail icon.
struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        ui::tooltip(self.0.clone(), ui::Tip::Hint)
    }
}

/// A pane on its way to another place: the value a drag of its header carries.
struct DraggedPane(PaneId);

/// A tab on its way to another place on the bar, or into another task's group.
struct DraggedTab(u32);

/// What follows the pointer during a pane drag: the pane's label. `offset`: where in the header
/// the drag began, as GPUI draws the view that far up and left of the pointer.
struct DragChip {
    label: SharedString,
    offset: gpui::Point<Pixels>,
}

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let chip = ui::tooltip(self.label.clone(), ui::Tip::Drag);
        div().pl(self.offset.x + px(8.)).pt(self.offset.y + px(8.)).child(chip)
    }
}

/// A drawn part the state gives a rect for, so a pointer can be aimed at it: a pane's header,
/// its ×, the tab at a place on the bar.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Mark {
    Header(PaneId),
    Close(PaneId),
    Tab(usize),
    /// A space of the top row, by its place there.
    Space(usize),
    /// A repository's name in the Repos section, by its place in `model.repos`.
    Repo(usize),
    /// A checkout's row in the Repos section, by its place in the list.
    Checkout(usize),
    /// The left column's title strip, and the open section's header.
    Strip,
    BarHeader,
    /// The Tasks rail cell's needs-you dot.
    TasksDot,
    /// The meeting timeline's band, its workday marks (0 start, 1 end), now's tick, its n-th block,
    /// and the marker of blocks before (0) or after (1) it.
    Band,
    BandEdge(usize),
    BandNow,
    BandBlock(usize),
    BandOut(usize),
    /// The Calendar section's all-day line, its first row and its fetch's line.
    CalendarAllDay,
    CalendarFirstRow,
    CalendarFetch,
    /// The Calendar section's day: the view, the scroll it is in, its n-th block and pressed free
    /// band, now's line and its time.
    DayView,
    DayViewport,
    DayBlock(usize),
    DayGap(usize),
    DayNow,
    DayNowLabel,
    /// The status bar, and its place, meeting and agents segments.
    Status,
    StatusPlace,
    StatusMeeting,
    StatusAgents,
    /// The toast's stack, and each card, its × and what it offers, by place from the top.
    Toast,
    ToastCard(usize),
    ToastClose(usize),
    ToastOffer(usize),
    ToastNewTask(usize),
    /// The key reference's `Known · n`.
    KeysKnown,
    /// The new-task box's Switch, while it is pinned to another space.
    ComposeSwitch,
    /// A Tasks heading's fold mark (0), name (1) and count (2): a test run's `heading_rects`.
    Heading(Heading, u8),
    /// A draft's row of the quit question, by its place in `Confirm::unsent`.
    QuitRow(usize),
    /// A side overlay (`overlay_panels`): the sidebar's (true) or the drawer's.
    Overlay(bool),
    /// A viewer's first marked line: a test run's `marked_rect`.
    ViewerMarked(PaneId),
    /// An offer chip's words, by pane, and their tooltip: a test run's `offer_rects`.
    OfferWords(PaneId),
    OfferTip,
}

type Marks = Rc<RefCell<HashMap<Mark, Bounds<Pixels>>>>;

/// An invisible child that fills its parent and records where it is drawn.
fn mark(marks: &Marks, key: Mark) -> impl IntoElement + use<> {
    let marks = marks.clone();
    let note = move |bounds, _: &mut Window, _: &mut App| {
        marks.borrow_mut().insert(key, bounds);
    };
    canvas(note, |_, _, _, _| {}).absolute().top_0().left_0().size_full()
}

/// As `mark`, only the part that shows: a tab scrolled out of its row is not noted.
fn shown_mark(marks: &Marks, key: Mark) -> impl IntoElement + use<> {
    let marks = marks.clone();
    let note = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
        let shown = bounds.intersect(&window.content_mask().bounds);
        if shown.size.width > px(1.) && shown.size.height > px(1.) {
            marks.borrow_mut().insert(key, shown);
        }
    };
    canvas(note, |_, _, _, _| {}).absolute().top_0().left_0().size_full()
}

/// `[x, y, w, h]` in window points.
pub(crate) fn rect(b: Bounds<Pixels>) -> [f32; 4] {
    [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()]
}

actions!(
    ottospace,
    [
        SplitRight,
        SplitDown,
        PickPane,
        ClosePane,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        FocusNext,
        FocusPrev,
        ZoomWindow,
        FillDisplay,
        NewTab,
        NextTab,
        PrevTab,
        CloseTab,
        FoldTab,
        ShowKeys,
        AxOverlay
    ]
);
// The picker for a pane below the focused one.
actions!(ottospace, [PickPaneDown]);
// The top row: the next or previous space, and hiding the selected one.
actions!(ottospace, [NextSpace, PrevSpace, HideSpace, GoHome, NextChat, PrevChat]);
// Pane arrangement.
actions!(
    ottospace,
    [ResizeLeft, ResizeRight, ResizeUp, ResizeDown, SwapLeft, SwapRight, SwapUp, SwapDown, Equalize, FitPane]
);
// Terminal selection.
// The Edit menu items for a text field.
actions!(ottospace, [Cut, SelectAll, Undo, Redo]);
actions!(ottospace, [CopySelection, PasteText, VisualMode, ListMode, SneakMode, NewTask, NextTask]);
// What waits on the person, one question at a time (`decisions`).
actions!(ottospace, [OpenDecisions]);
// Marks: mark the focused pane or the tab on show, and the jump prefix.
actions!(ottospace, [MarkPane, MarkTab, MarkJump]);
// Save the tab on show as a workbook, add it to one, or save all the tabs as one; keep the
// selected temporary workspace.
actions!(ottospace, [SaveWorkbook, AddTabToWorkbook, SaveAllWorkbook, KeepWorkbook]);
// The repository of the focused pane's folder, to the Repos list.
actions!(ottospace, [AddToRepos]);
// The next start is empty: the layout is not kept this run.
actions!(ottospace, [StartFresh]);
// What the capture key does, from the menu.
actions!(ottospace, [CapturePane]);
// The bug button's key: report a bug in ottospace, while `developer.ottospace_repo` is set.
actions!(ottospace, [ReportBug]);
// The right drawer: the keyboard in or out, closed or open, the task's workspace, its dial.
actions!(ottospace, [TalkFocus, TalkToggle, SidebarToggle, TalkGo, StopAgent, EditMessage, DialToggle]);
// A tab of no task, and the tab on show taken out of its task or moved to another.
actions!(ottospace, [MoveTabToTask]);
// App menu.
actions!(ottospace, [Quit, Hide, About]);

/// Every key binding: the menus show these keys.
fn keymap() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-d", SplitRight, None),
        KeyBinding::new("cmd-shift-d", SplitDown, None),
        KeyBinding::new("cmd-e", PickPane, None),
        KeyBinding::new("cmd-shift-e", PickPaneDown, None),
        KeyBinding::new("cmd-/", ShowKeys, None),
        KeyBinding::new("cmd-p", commands::CommandPalette, None),
        KeyBinding::new("cmd-,", commands::EditSettings, None),
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
        KeyBinding::new("cmd-shift-alt-left", ResizeLeft, None),
        KeyBinding::new("cmd-shift-alt-right", ResizeRight, None),
        KeyBinding::new("cmd-shift-alt-up", ResizeUp, None),
        KeyBinding::new("cmd-shift-alt-down", ResizeDown, None),
        KeyBinding::new("cmd-shift-alt-h", ResizeLeft, None),
        KeyBinding::new("cmd-shift-alt-l", ResizeRight, None),
        KeyBinding::new("cmd-shift-alt-k", ResizeUp, None),
        KeyBinding::new("cmd-shift-alt-j", ResizeDown, None),
        KeyBinding::new("cmd-ctrl-=", Equalize, None),
        KeyBinding::new("cmd-ctrl-0", FitPane, None),
        // Swap the focused pane with its neighbour. ⌘⇧⌥1..9 move it to a tab (`key_down`).
        KeyBinding::new("cmd-ctrl-left", SwapLeft, None),
        KeyBinding::new("cmd-ctrl-right", SwapRight, None),
        KeyBinding::new("cmd-ctrl-up", SwapUp, None),
        KeyBinding::new("cmd-ctrl-down", SwapDown, None),
        KeyBinding::new("cmd-ctrl-h", SwapLeft, None),
        KeyBinding::new("cmd-ctrl-l", SwapRight, None),
        KeyBinding::new("cmd-ctrl-k", SwapUp, None),
        KeyBinding::new("cmd-ctrl-j", SwapDown, None),
        // The Mac's Enter Full Screen key, free: native full screen is off (`zoom_instead_of_fullscreen`).
        KeyBinding::new("cmd-ctrl-f", FillDisplay, None),
        // A tab out to another display, and back in (`multi_monitor`).
        KeyBinding::new("cmd-shift-o", FoldTab, None),
        // Debug: outline the focused app pane's accessibility tree.
        KeyBinding::new("cmd-ctrl-a", AxOverlay, None),
        // The bug button's, while it shows; shown in its tooltip only.
        KeyBinding::new(BUG_KEY, ReportBug, None),
        // Tabs. ⌘1..⌘9 select one (handled in `key_down`).
        KeyBinding::new("cmd-t", NewTab, None),
        KeyBinding::new("cmd-}", NextTab, None),
        // The top row: ⌘⌃1..⌘⌃9 select a space (`key_down`).
        KeyBinding::new("cmd-ctrl-]", NextSpace, None),
        KeyBinding::new("cmd-ctrl-[", PrevSpace, None),
        KeyBinding::new("cmd-alt-]", NextChat, None),
        KeyBinding::new("cmd-alt-[", PrevChat, None),
        KeyBinding::new("cmd-ctrl-w", HideSpace, None),
        KeyBinding::new("cmd-{", PrevTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("cmd-shift-w", CloseTab, None),
        KeyBinding::new("cmd-shift-s", SaveWorkbook, None),
        // Terminal selection. ⌘C never sends ^C: that is ⌃C.
        KeyBinding::new("cmd-c", CopySelection, None),
        KeyBinding::new("cmd-v", PasteText, None),
        // A text field takes these keys itself; bound so the Edit menu shows them.
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-shift-c", VisualMode, None),
        KeyBinding::new("cmd-ctrl-e", ListMode, None),
        // The first is the one the menu shows.
        KeyBinding::new(sneak::KEYS[0], SneakMode, None),
        KeyBinding::new(sneak::KEYS[1], SneakMode, None),
        KeyBinding::new("cmd-n", NewTask, None),
        KeyBinding::new("cmd-j", NextTask, None),
        KeyBinding::new("cmd-l", TalkFocus, None),
        KeyBinding::new("cmd-shift-r", TalkToggle, None),
        KeyBinding::new("cmd-shift-l", SidebarToggle, None),
        KeyBinding::new("cmd-enter", TalkGo, None),
        KeyBinding::new("cmd-shift-a", DialToggle, None),
        KeyBinding::new("cmd-ctrl-t", listen::Transcribe, None),
        KeyBinding::new("cmd-ctrl-v", agent_mode::AgentMenuKey, None),
        // Marks: ⌘' waits for a mark's letter. ⌘⌥1..9 go to the first nine (`key_down`).
        KeyBinding::new("cmd-b", MarkPane, None),
        KeyBinding::new("cmd-shift-b", MarkTab, None),
        KeyBinding::new("cmd-'", MarkJump, None),
        KeyBinding::new("cmd-h", Hide, None),
        // Asks first while a terminal runs (`ask_quit`): ⌘Q sits next to ⌘W.
        KeyBinding::new("cmd-q", Quit, None),
    ]
}

/// What a socket call files once its tags are read.
enum Filing {
    /// `task_new`: the line the reader was asked about. `open`: also show the task's tabs.
    Line { open: bool },
    /// `task_add`: its `agent` and `skills` are checked against the names read.
    Task { title: String, priority: u8, state: State, tab: Option<u32>, assign: Assign },
}

/// A heading's name in a control's id and on the socket: `needs_you`, `open`, ...
fn heading_name(heading: Heading) -> String {
    serde_json::json!(heading).as_str().unwrap_or_default().to_string()
}

struct Workspace {
    model: Model,
    /// Shell half of each app pane.
    hosted: HashMap<PaneId, Hosted>,
    /// Shell half of each terminal pane.
    terminals: HashMap<PaneId, Terminal>,
    /// Each file viewer's file as read and drawn (`viewer`), by pane.
    viewers: HashMap<PaneId, viewer::FileView>,
    /// "File…" choices that would have shown the open panel: a test run shows none.
    file_prompts: u32,
    /// Windows the picker's second step asked to host (`Effect::HostPicked`).
    window_picks: u32,
    /// How often "New window" asked by each kind of recipe (`Effect::NewWindow`): open, tell, the
    /// "New Window" menu item.
    new_window_asks: [u32; 3],
    /// The app "New window" waits for, with the windows it had before.
    asked: Option<pick::Asked>,
    /// Each terminal's header, read from its processes a few times a second: `app-api · cargo test`.
    titles: HashMap<PaneId, String>,
    /// The right drawer: each task's agent, and where its conversation was drawn.
    drawer: talk::Drawer,
    /// One terminal character cell.
    cell: Size<Pixels>,
    /// The terminal pane a mouse drag is selecting in.
    dragging: Option<PaneId>,
    /// During a pane drag: the pane under the pointer and the zone of it a drop would land on.
    drop_at: Option<(PaneId, Zone)>,
    /// The divider the button went down on, until it comes up.
    divider: Option<divider::Hold>,
    /// Where the button went down on a Workspaces row or a top-row space: a drag starts past a few points.
    reorder_from: Option<gpui::Point<Pixels>>,
    /// A `mouse` request holds the button down: a move before its release is a drag.
    held: bool,
    /// Releases a button no request came back for. Replaced, so stopped, by each `mouse` request.
    abandoned: Option<gpui::Task<()>>,
    /// The pointer event in hand came from a `mouse` or `drag` request, not from the user.
    injected: bool,
    /// A launched app's window is being looked for, some time after the click that started it.
    launched_later: bool,
    /// Where headers, their × and tabs were last drawn.
    marks: Marks,
    /// Where every copy goes and every paste comes from: a stand-in in an E2E run.
    clipboard: clipboard::Clipboard,
    /// A paste that asks first ("Paste 4 lines?"): its pane and the text read at ⌘V.
    pasting: Option<(PaneId, String)>,
    /// Reduce motion, as read at the last copy or paste: the pulse holds still.
    clip_reduced: bool,
    /// Where the pointer is while a selection drag is held: past an edge, the terminal scrolls.
    drag_at: Option<gpui::Point<Pixels>>,
    /// The open text field as last drawn: the new-task line, and a note's and the picker's own.
    field: field::Shared,
    /// What an ⌥-click would reference in the new-task box: outlined while ⌥ is held over it.
    pointing: Option<Bounds<Pixels>>,
    /// Where the pointer last was: ⌥ pressed with it still outlines what is there.
    hover_at: gpui::Point<Pixels>,
    note_field: field::Shared,
    /// The bug report's comment being typed.
    bug_field: field::Shared,
    /// The window as frozen for the bug report.
    bug_shot: Option<bug::Frozen>,
    /// A report's picture is being taken: a second press takes none.
    bug_pending: bool,
    picker_field: field::Shared,
    apps_field: field::Shared,
    mark_field: field::Shared,
    inspector_field: field::Shared,
    btw_field: field::Shared,
    /// What is held of each capture of the draft in the bar, by its number.
    shots: HashMap<u32, collect::Shot>,
    /// Where the picture being annotated and the right bar were last drawn.
    annotate_at: Rc<Cell<Bounds<Pixels>>>,
    bar_at: Rc<Cell<Bounds<Pixels>>>,
    /// Briefs of tasks, as added in this run or read from their folders.
    briefs: HashMap<TaskId, collect::Shown>,
    /// The folders whose brief could not be read: not tried again in this run.
    no_brief: std::collections::HashSet<String>,
    /// Pictures asked for and not taken yet, and the terminals whose visual cursor goes after one.
    pending: Vec<collect::Pending>,
    visual_after: Vec<PaneId>,
    /// A global key that could not be registered, for the toast.
    key_notice: Option<String>,
    /// A test run's recording of its own window (`record_start`).
    recording: Option<record::Recording>,
    /// A test run's ear on the canary's sound (`audio_probe_start`).
    audio_probe: Option<listen_live::Probe>,
    /// Each pane's body bounds in window coordinates, recorded during prepaint.
    cells: HashMap<PaneId, Rc<Cell<Bounds<Pixels>>>>,
    /// Each pane's bounds inside its border, for e2e to check the live window's gap.
    frames: HashMap<PaneId, Rc<Cell<Bounds<Pixels>>>>,
    /// How often the workspace was rendered, and the count at the last dispatch: pane bounds
    /// are of the current layout only once it was rendered after that.
    renders: u64,
    dispatched_at: u64,
    /// Actions dispatched, and how many of them the last frame drew. Test runs hold a reply with
    /// the state until a frame has drawn what its request did (`answer`).
    seq: u64,
    drawn_seq: u64,
    settles: bool,
    /// Replies waiting for that frame: (since when, where the reply goes).
    settling: Vec<(Instant, Sender<String>)>,
    /// When the frame being drawn began, for the trace's frame time.
    frame_start: Rc<Cell<Option<Instant>>>,
    /// Frames that stopped coming (`stall`), and how often GPUI was kicked to draw again.
    stall: stall::Stall,
    redraws: u64,
    /// Stalls in a row, so a locked screen does not fill the log.
    stall_log: event_log::Streak,
    /// The display the window was last on, by number, for the log.
    on_display: Option<u32>,
    /// The displays last told to the core, and the main window's (`watch_displays`).
    displays_told: Option<(Vec<ottospace_core::model::displays::Display>, Option<String>)>,
    /// When the displays last changed: a display new since waits for 1 s of quiet.
    displays_changed: Option<Instant>,
    /// The main window's display, by number, as last told.
    displays_on: Option<u32>,
    /// A test run's made-up displays stand for the real ones (`displays_set`).
    displays_made_up: bool,
    /// The window of each tab out (`out_window`).
    out_windows: out_window::OutWindows,
    /// The tab out whose window has the keyboard, as last made key.
    keys_in: Option<u32>,
    /// The window ottospace started with: the main one.
    main_window: gpui::AnyWindowHandle,
    /// Each tab out's window as last drawn: its frame, and how many frames it drew.
    out_frames: HashMap<u32, ([f32; 4], u64, isize)>,
    /// The controls a tab out's window draws, apart from the main window's (`out_window`).
    out_controls: sneak::Controls,
    /// A terminal's or hosted window's output asked for a frame not drawn yet: a stall too.
    output: bool,
    /// What the home screen last drew, and the lattice lines kept between frames.
    home: home::Shared,
    /// ottospace's own controls as last painted: what sneak mode labels.
    controls: sneak::Controls,
    /// Where the agent chip's line was last painted; none while it is the badge alone.
    chip_line: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The shown dial's moving parts: view state, stepped each frame it is drawn.
    dial_motion: RefCell<dial::DialMotion>,
    /// The seal, fold and land of the user's Done, while it plays.
    seals: seal::Seals,
    /// The side overlays' slides in (`overlay_panels`).
    slides: overlays::Slides,
    /// The key that ended sneak mode is still down: its repeats are typed nowhere.
    sneak_spent: bool,
    /// Reads of the focused app pane's tree while sneak mode is on.
    sneak_reads: sneak_apps::Reads,
    /// Scrolls held for an app pane the wheel entered, and counts.
    wheel: app_wheel::Wheel,
    /// Whose key, click or request is being handled now: the person's, unless the control socket
    /// is being served (also a key or click it injects) or the tick is doing its own work.
    input: Origin,
    /// `Model::badges` as of this frame.
    badges: std::cell::RefCell<Vec<(ottospace_core::talk::Subject, String)>>,
    apps: Vec<capture::App>,
    /// Apps started from the list, and where each one's window goes once it has one.
    launching: Vec<Launch>,
    /// The apps found installed, for the picker.
    installed: apps::Scan,
    /// Icons of apps seen so far, by app name. Kept: an icon is read and encoded once.
    icons: HashMap<String, Arc<gpui::Image>>,
    /// A problem found at start (settings, font, tasks folder). It stays: unlike `error`, no
    /// later success makes it untrue.
    notice: Option<String>,
    /// `settings.json` as last looked at, and as last read into the core: a change is read
    /// once it has held for one look, so a file an editor is still writing is not.
    settings_looked: Option<config::Stamp>,
    settings_read: Option<config::Stamp>,
    /// False: the tasks folder could not be read, so nothing is written to it.
    keeping: bool,
    /// The last task that could not be written, until a write of it succeeds.
    unsaved: Option<String>,
    /// The tasks as last written to disk, to write only what changes.
    stored: Vec<Task>,
    /// `marks.json` as last read and written, and what the marks were last found to be missing.
    kept_marks: marks::Kept,
    /// The workbooks' files as last read and written.
    kept_workbooks: workbook::Kept,
    /// The row of the bar the pointer is on, if it has a menu: its … shows.
    row_hovered: Option<ottospace_core::list::ListRow>,
    /// A key closed a row's menu: held on, it is not for what was under the menu.
    row_menu_spent: bool,
    /// Where the right click that opened a row's menu was: the menu is drawn there.
    row_menu_at: Option<gpui::Point<Pixels>>,
    /// What a tick with no window brought (`tick_now`), carried out at the next dispatch: a
    /// closed temporary workspace's panes, its Undo run out.
    held_effects: Vec<Effect>,
    /// Runs of the tick so far, for a test run's state.
    ticks: u64,
    kept_layout: layout_file::Kept,
    /// `keys.json` as last read and written.
    kept_keys: key_use::Kept,
    /// The modifiers the empty tab's list was last drawn with (`keycaps`): input, not model state.
    caps_drawn: Cell<Modifiers>,
    /// Scroll position of the bar's list, so list mode can bring the selected row into view.
    list_scroll: ScrollHandle,
    /// The Calendar section's own: at the top when it opens and when its day changes. The frame
    /// it was last drawn in and the day it showed (`calendar::render_calendar`).
    calendar_scroll: ScrollHandle,
    calendar_shown: std::cell::Cell<(u64, i32)>,
    /// The section opened or its day changed: scrolled to now once its height is known.
    calendar_opening: std::cell::Cell<bool>,
    /// The meeting last brought into view: a new selection is (`calendar::render_calendar`).
    calendar_revealed: std::cell::Cell<Option<u64>>,
    /// The drawer rail's tasks; the frame and what it last drew (a new one scrolls to it
    /// once); the item still to bring into view, and the tries left (`drawer_rail`).
    rail_scroll: ScrollHandle,
    rail_scrolled: RefCell<(u64, Option<drawer_rail::Drawn>)>,
    rail_pending: Rc<Cell<Option<(usize, u8)>>>,
    /// The second row's tabs, and the tab last brought into view there.
    tabs_scroll: ScrollHandle,
    tabs_shown: Cell<Option<u32>>,
    /// Every scroll area's bar.
    bars: scrollbar::Bars,
    /// The minute of the last look at the clock: the Tasks section's ages are drawn again when it changes.
    minute: u64,
    /// Reads what tags may name, on a thread of its own.
    tags: Option<tags::Reader>,
    /// The repository the agents and skills were last asked for, while the new-task box is open.
    tags_repo: Option<Option<String>>,
    /// Worktrees asked for (`Effect::AddWorktree`) and made, or not, off the main thread.
    worktrees: (Sender<worktree::Made>, Receiver<worktree::Made>),
    /// The local day's start last told to the core (`LocalDay`).
    told_day: u64,
    /// Done's removals of a task's own worktree, off the main thread.
    removals: (Sender<worktree::Removed>, Receiver<worktree::Removed>),
    /// What the user's last step did, said for a few seconds as information (`Effect::Told`).
    told: Option<(String, Instant)>,
    /// Pull requests read with gh; why none are, when the settings say `"repos_prs": "on"`; how many
    /// the user opened (a test run counts them and opens none).
    prs: prs::Reader,
    pr_note: Option<&'static str>,
    /// Frames a checkout dot's pulse asked for (the state's `counts.repo_pulse_frames`, test runs).
    repo_pulse_frames: Cell<u64>,
    /// The status bar: the focused folder's checkout, read off the main thread; how its segments
    /// were last shortened; the meeting it last said (`id`, minutes to go, Join), to draw again on a change.
    status_git: status_bar::Reader,
    /// The code city behind a task's empty tab: its files, read off the main thread, and what
    /// the last frame drew of it.
    city: city::Reader,
    city_drawn: city::Shared,
    city_painter: city::Painter,
    status_fit: Cell<ottospace_core::status_bar::Fit>,
    status_said: Option<(u64, Option<u64>, bool)>,
    pr_opens: u32,
    /// Socket calls that file a task and wait for the reader: (the call's number, what to file, where to reply).
    filing: Vec<(u64, Filing, Sender<String>)>,
    /// How many such calls there were: each has its own number.
    calls: u64,
    /// Reads agent sessions on a thread of its own. `None`: the settings turn it off.
    sessions: Option<sessions::Poller>,
    /// The core holds sessions that were found: they are cleared when the Tasks section closes.
    sessions_listed: bool,
    /// The last read went to the Tasks section's list (else only to the Repos section's dots).
    sessions_tasks: bool,
    /// The core was told the `session_quiet_minutes` setting.
    sessions_quiet_told: bool,
    /// The user was told that the sessions reader stopped.
    sessions_stopped: bool,
    /// Window ids seen per hosted pid; anything new that appears later gets its own pane.
    known: Vec<(i32, HashSet<u32>)>,
    /// When the live pane went live.
    live_since: Option<Instant>,
    /// The app of the pane that last left live, and when: its own activation may land after.
    left_live: Option<(i32, Instant)>,
    /// How often a pane went live, for tests.
    went_live: u64,
    /// The pane parked because ottospace became active, until it is no longer active.
    left_on_activation: Option<PaneId>,
    /// The hosted app that was the front app at the last look, and the window it showed, to see
    /// one come forward or change window.
    front_hosted: Option<(i32, Option<u32>)>,
    /// The pane to give the keyboard to because its app brought its window forward, kept until
    /// that can be done.
    wanted: Option<PaneId>,
    error: Option<String>,
    /// The last `Effect::Notice`: while `error` says it, it is a refusal, not a failure.
    refused: Option<String>,
    /// `error` while it is the app's own failure, not an answer (`broke`).
    broke: Option<String>,
    /// What the bar holds of `error` (`swap_origin`): a client's answer is not on it.
    bar_line: notice::Held,
    /// `error` as `bar_line` last saw it, and where its text was: the same words said again
    /// are a new line (a new `String`), e.g. the user's refusal after a client's same one.
    error_seen: (Option<String>, Option<usize>),
    /// The toast as it comes and goes.
    toast: notices::Toast,
    /// The toast's floating panels, over a live app's window: one a piece (`toast::PIECES`).
    toast_panels: [float_panel::Panel; toast::PIECES],
    /// False in e2e runs, which must never move the user's real windows.
    hosting: bool,
    /// While the window fills its display.
    filled: Option<Filled>,
    /// A zoom of the window (its green button) fills the display instead (`zoom_asks`).
    zoom_asks: bool,
    /// A test run's `ui_gallery`: the components' gallery over the window.
    gallery: bool,
    /// Test runs: the pid of the one app that may be hosted.
    host_only: Option<i32>,
    control: Option<Receiver<Call>>,
    /// The accessibility overlay's last read. Drawn while `model.ax_overlay` is on.
    ax_overlay: Option<ax_debug::Overlay>,
    /// The meeting timeline's feed reader and what it draws.
    timeline: calendar::Timeline,
    /// What a transcript's streams hear: in a test run, its fixture being replayed.
    listen_feed: Option<listen::Feed>,
    /// Agent mode's pictures of app panes, by pane: their folders (`agent_mode::Looks`).
    looks: agent_mode::Looks,
    /// Panes whose window is gone while their app runs, since when (`drop_closed`).
    gone_since: HashMap<PaneId, u64>,
    /// A test run's fixture windows on offer (`window_fixture`), by id: their titles.
    offer_fixtures: HashMap<u32, String>,
    /// The apps the core asked to look at (`Effect::WatchSlots`), the looks last sent, and a
    /// test run's fixture looks (`slot_windows`).
    slot_watch: Vec<(Option<String>, String)>,
    slot_seen: Vec<ottospace_core::slot::Look>,
    slot_fixtures: Vec<ottospace_core::slot::Look>,
    next_fixture_window: u32,
    /// The file of the transcript on, if one is kept.
    listen_file: Option<listen::File>,
    /// Files of transcripts that ended, until their end is written.
    listen_ending: Vec<listen_file::Writer>,
    /// What the transcript on hears now, for the meter and its label alone.
    listen_meter: Option<listen_meter::Meter>,
    /// The transcript whose speech model the system is downloading, for its label.
    listen_downloading: Option<String>,
    /// Today's files of `transcripts_dir` as last handed to the core, and the one open read-only.
    listen_past: listen::Past,
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
    /// Who asked: the user's new tab joins the task on show once the window is in.
    origin: Origin,
    /// The window a mark asked for; else the app's main window, as a click in the list asks.
    wanted: Option<marks::Wanted>,
    /// How many windows the started app had at the last look, and since when: a mark's window
    /// is picked once they have settled (`marks::settled`).
    windows: Option<(usize, Instant)>,
}

/// How long to wait for a started app's window before giving up.
const LAUNCH_WAIT: Duration = Duration::from_secs(30);
/// How long a `mouse` request's button stays down with no further request.
const POINTER_IDLE: Duration = Duration::from_secs(5);
/// Longest a test run's reply waits for the frame that draws its request.
const SETTLE_LIMIT: Duration = Duration::from_secs(5);

/// How much of a test run's window stays on screen, in points, at the corner it sits in.
const E2E_SLIVER: f32 = 12.;

/// A test run's window size, `OTTOSPACE_E2E_WINDOW=1024x674` (CI's screen), read only under
/// `OTTOSPACE_E2E`. macOS keeps it no larger than the screen. 8000 at most: 100000 ended the app at start.
fn e2e_window() -> Option<(f32, f32)> {
    std::env::var_os("OTTOSPACE_E2E")?;
    let value = std::env::var("OTTOSPACE_E2E_WINDOW").ok()?;
    let size = value.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
    let size = size.filter(|&(w, h): &(f32, f32)| (100. ..=8_000.).contains(&w) && (100. ..=8_000.).contains(&h));
    if size.is_none() {
        eprintln!("OTTOSPACE_E2E_WINDOW={value:?} is not WIDTHxHEIGHT, 100 to 8000 each: the window is 1400x900");
    }
    size
}

/// How long the `s` key snoozes a task: one hour.
const SNOOZE_MS: u64 = 3_600_000;
/// How long a step's information (`Effect::Told`) stays in the toast.
const TOLD_FOR: Duration = Duration::from_secs(8);

/// Longest a `wait` request may run: 10 minutes.
const MAX_WAIT_MS: u64 = 600_000;

/// The active displays for the log: how many, and each one's size in points ("1728x1117"),
/// never its name.
fn displays() -> serde_json::Value {
    let sizes: Vec<String> = display::all().into_iter().map(size_of).collect();
    serde_json::json!({ "count": sizes.len(), "sizes": sizes })
}

fn size_of(rect: CGRect) -> String {
    format!("{}x{}", rect.size.width.round(), rect.size.height.round())
}

/// A test run's window floats on every Space; clicks pass through it unless it is put on show
/// (`OTTOSPACE_E2E_SHOW`): tests use the socket, not the mouse.
fn keep_on_top(window: &Window) {
    let click_through = std::env::var_os("OTTOSPACE_E2E_SHOW").is_none();
    if let Some(view) = ns_view(window) {
        // SAFETY: GPUI's own view, on the main thread.
        unsafe { ottospace_mac::window::keep_on_top(view, click_through) };
    }
}

/// The window's `NSView`, for the test requests that hand it an event.
fn ns_view(window: &Window) -> Option<*mut std::ffi::c_void> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr()),
        _ => None,
    }
}

impl Workspace {
    fn new(
        hosting: bool,
        host_only: Option<i32>,
        control: Option<Receiver<Call>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let subscriptions = vec![
            cx.observe_window_bounds(window, Self::fill_holds),
            // Clicking ottospace itself (title bar, empty space) raises it over the live window,
            // so hand that pane back to its stream. Ignore the activation that the swap click
            // itself delivers late (the race that used to bounce windows straight back).
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.left_on_activation = None;
                    return;
                }
                let settled = this.live_since.is_some_and(|at| at.elapsed() > Duration::from_millis(400));
                if settled {
                    // The click that caused this may still be on its way: see `click_pane`.
                    this.left_on_activation = this.model.live;
                    this.dispatch(Action::Leave, this.input, window, cx);
                }
            }),
            // A key's binding ran: counted, if the key was the user's. A menu or the palette runs
            // the same action with no keystroke, so it counts for nothing.
            cx.observe_keystrokes(|this, event, _, _| {
                if let Some(binding) = event.action.as_deref().and_then(key_use::binding_of) {
                    this.key_used(binding);
                }
            }),
            // Before the key's binding runs, so every ⌘ shortcut leaves sneak mode and then acts,
            // also one whose handler changes no state (⌘C, ⌘V). The quit question takes the key
            // whole: no binding and no key handler sees it.
            cx.intercept_keystrokes(move |event, window, cx| {
                let taken = this.update(cx, |this, cx| {
                    this.confirm_intercepts(&event.keystroke, window, cx)
                        || this.shortcut_leaves_sneak(&event.keystroke, window, cx)
                });
                if taken.unwrap_or(false) {
                    cx.stop_propagation();
                }
            }),
            cx.on_app_quit(|this, _| {
                // A draft does not outlive the run: what was kept of it goes.
                if let Some(draft) = this.model.capture.as_ref().map(|bar| bar.draft.clone()) {
                    this.drop_capture(&draft, None);
                }
                // Every quit ends here: ⌘Q, the menu, the close button, `ctl quit`, a signal.
                event_log::event("quit", serde_json::json!({ "hosted": this.hosted.len() }));
                // The layout first, while its panes and terminals are all there.
                this.store_layout(true);
                this.store_key_use(true);
                // A transcript's file gets its end now: lines were written as they came.
                this.listen_quit();
                // Its agents end with it: none is left on a session, to hold it from the next run.
                this.drawer.agents.values().for_each(|agent| agent.terminal.signal(libc::SIGHUP));
                this.drawer.native.clear();
                this.drawer.stopping.clear();
                this.hosted.drain().for_each(|(_, hosted)| {
                    hosted.give_back();
                });
                if this.control.is_some() {
                    // The socket and the lock file `control::serve` made beside it.
                    let _ = std::fs::remove_file(control::path());
                    let _ = std::fs::remove_file(control::path().with_extension("lock"));
                    // Its agents' pass files: they end with it.
                    pass::sweep(&pass::folder());
                }
                running::remove();
                async {}
            }),
        ];

        // Whether this Mac has the recogniser, asked once: then an app pane may be transcribed.
        cx.spawn_in(window, async move |this, cx| {
            let on = cx.background_executor().spawn(async { listen_live::available() }).await;
            if let Some(on) = on {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.dispatch(Action::ListenAvailable { on }, Origin::System, window, cx)
                });
            }
        })
        .detach();
        // The tick: answer control calls, redraw on new output, keep windows matched to panes;
        // every ~500ms, also look for new and closed windows of hosted apps. Each frame while
        // something moves or waits, or just happened; else asleep until woken, or the next slow
        // look or thing due (`cadence`).
        cx.spawn_in(window, async move |this, cx| {
            let mut clock = cadence::Clock::default();
            let mut wait = cadence::Wait::For(cadence::FRAME_MS);
            loop {
                match wait {
                    // One frame: a wakeup does not cut it short, so the loop runs at most each frame.
                    cadence::Wait::For(ms) if ms <= cadence::FRAME_MS => {
                        cx.background_executor().timer(Duration::from_millis(ms)).await
                    }
                    cadence::Wait::For(ms) => woken_or(cx.background_executor().timer(Duration::from_millis(ms))).await,
                    cadence::Wait::Woken => ottospace_mac::wake::woken().await,
                }
                let ticks = clock.step(loop_ms(), ottospace_mac::wake::take());
                let load = this.update_in(cx, |this, window, cx| {
                    this.ticks += 1;
                    if give_back::signalled() {
                        cx.quit();
                    }
                    this.serve_control(window, cx);
                    this.sync_viewers(window, cx);
                    // A grammar's query built aside: what was drawn plain or by the lexer, again.
                    if crate::syntax::take_built() {
                        cx.notify();
                    }
                    this.take_tags(window, cx);
                    this.store_layout(false);
                    this.store_key_use(false);
                    if ticks.every(15, 0) {
                        this.poll_sessions(window, cx);
                        this.read_titles(cx);
                        this.read_status_checkout(cx);
                        this.read_city(cx);
                    }
                    this.listen_menu_bar(window, cx);
                    if ticks.every(60, 0) {
                        this.listen_read_past(window, cx);
                    }
                    // The green button, Window ▸ Zoom or a title bar double-click: only the user's hand.
                    if ottospace_mac::window::take_zoom() {
                        this.dispatch(Action::FillDisplay, Origin::User, window, cx);
                    }
                    if hotkey::take_press() {
                        this.key_used(Binding::ReturnKey);
                        this.return_to_ottospace(window, cx);
                    }
                    if hotkey::take_capture() {
                        this.key_used(Binding::CaptureKey);
                        // From any app: ottospace comes forward with the picture to draw on.
                        this.error = this.capture_pane(None, Origin::User, window, cx).err();
                        cx.activate(true);
                        cx.notify();
                    }
                    this.with_origin(Origin::System, |this| {
                        // About every 2 s: slots' looks and Away's clock.
                        if ticks.every(120, 0) {
                            this.look_at_slots(window, cx);
                        }
                        if ticks.every(30, 0) {
                            this.host_new_window(window, cx);
                            this.adopt_new_windows(window, cx);
                            this.read_edited(window, cx);
                            this.drop_gone_offers(window, cx);
                            this.drop_closed(window, cx);
                            this.follow_front_app(window, cx);
                            this.host_launched(window, cx);
                            this.refresh_inspector(window, cx);
                            // A snoozed task returns when its time comes, and the offer to undo goes.
                            // A dispatch also carries out what a tick with no window held.
                            let now = now_ms();
                            if this.model.due(now) || !this.held_effects.is_empty() {
                                this.dispatch(Action::Tick { now }, Origin::System, window, cx);
                                cx.notify();
                            }
                            // An age shown ("5m") changes at most once a minute: draw again then,
                            // while the Tasks section or the drawer is open to show one.
                            let minute = now / 60_000;
                            // The Calendar's now line and its time too.
                            let shown = matches!(this.model.sidebar, Some(Section::Tasks | Section::Calendar))
                                || this.model.talk.is_some();
                            if minute != this.minute && shown {
                                cx.notify();
                            }
                            this.minute = minute;
                            // The meeting's minutes to go, its start and its end: on the status bar.
                            let said = this.status_meeting().map(|m| (m.id, m.in_min, m.join));
                            if said != this.status_said {
                                this.status_said = said;
                                cx.notify();
                            }
                            // The Marks list says what is missing as it is now.
                            if matches!(this.model.sidebar, Some(Section::Marks | Section::Workbook)) {
                                this.check_marks(cx);
                            }
                        }
                        // Tell the core which windows kept another size than their pane's. Not
                        // before the panes were drawn again after the last change: until then
                        // the sizes are those of the layout before it.
                        let drawn = this.renders > this.dispatched_at;
                        // A display that changed moves no window: looked at, not told.
                        if ticks.every(30, 15) {
                            this.fill_holds(window, cx);
                        }
                        for (pane, content, outer, kept) in this.sync_layout(window, cx, ticks.every(30, 15)) {
                            if drawn && this.model.wants_changed(pane, content, outer, kept) {
                                let wants = Action::PaneWants { pane, content, outer, window: kept };
                                this.dispatch(wants, Origin::System, window, cx);
                            }
                        }
                        this.sync_terminals(window, cx);
                        if ticks.every(3, 0) {
                            this.drag_scroll(cx);
                        }
                        // A pulse after a copy or paste is drawn frame by frame, and once more as it ends.
                        let pulse = this.model.clip.as_ref().filter(|clip| clip.pane.is_some());
                        if pulse.is_some_and(|clip| now_ms().saturating_sub(clip.at) < clip::PULSE_MS + 100) {
                            cx.notify();
                        }
                        this.sync_agents(window, cx);
                        // A notice whose time is up, or one that came with no frame asked for.
                        if ticks.every(6, 0) && this.notice_changed() {
                            cx.notify();
                        }
                        if ticks.every(3, 0) {
                            this.sync_toast_panel(window, cx);
                        }
                        // The header's time goes on while an agent runs.
                        let runs = this.model.talk.as_ref().is_some_and(|t| t.open && this.model.agent_runs(&t.task));
                        if runs && ticks.every(30, 0) {
                            cx.notify();
                        }
                        // A dial lit is drawn again each second; at rest only on a change.
                        if ticks.every(60, 0) && this.dial_moves() {
                            cx.notify();
                        }
                        if ticks.every(60, 0) {
                            this.seek_sessions(window, cx);
                        }
                        if ticks.every(15, 7) {
                            this.watch_settings(window, cx);
                        }
                        this.sync_sneak(window, cx);
                        this.sync_row_menu(window, cx);
                        this.sync_agent_looks();
                        this.read_sneak_app(window, cx);
                        this.tick_timeline(window, cx);
                        this.take_worktrees(window, cx);
                        this.tell_day(window, cx);
                        this.tick_prs(window, cx);
                        if ticks.every(60, 30) {
                            this.read_heads(cx);
                        }
                        this.watch_displays(window, cx);
                    });
                    this.take_snapshots(window, cx);
                    if this.hosted.values().any(|h| h.latest.dirty.swap(false, Relaxed)) {
                        this.output = true;
                        cx.notify();
                    }
                    this.watch_frames(window, cx);
                    this.load(cx)
                });
                let Ok(load) = load else { break };
                wait = clock.wait(loop_ms(), load);
            }
        })
        .detach();

        // Native fullscreen would move ottospace to its own Space, where other apps' windows
        // cannot follow. Make the green button zoom instead, and the zoom fill the display.
        let mut zoom_asks = false;
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
        {
            unsafe { ottospace_mac::window::zoom_instead_of_fullscreen(appkit.ns_view.as_ptr()) };
            // SAFETY: GPUI's own view, on the main thread.
            zoom_asks = unsafe { ottospace_mac::window::zoom_asks(appkit.ns_view.as_ptr()) };
            // A press on the top row's spaces is theirs (a click, a drag along the row), never the window's move.
            unsafe { ottospace_mac::window::titlebar_holds(appkit.ns_view.as_ptr()) };
        }
        // A run with hosting off never activates the app, so keep its window visible another
        // way. Not with hosting on: a floating ottospace would cover the live app window.
        if !hosting {
            keep_on_top(window);
        }

        let focus = cx.focus_handle();
        window.focus(&focus);
        // The user went back to the main window: the keyboard goes there from a tab out's.
        cx.observe_window_activation(window, |this, window, cx| {
            let main = this.model.tabs.get(this.model.main_at()).map(|tab| tab.id);
            if let Some(tab) = main.filter(|_| window.is_window_active() && this.model.main_at() != this.model.active) {
                this.dispatch(Action::KeyWindow { tab }, Origin::User, window, cx);
            }
        })
        .detach();
        let mut model = Model::default();
        model.update(Action::SetRepoDirs { dirs: config::repo_dirs() }.sent_by(Origin::System));
        // The Tasks section as the user last grouped it.
        let view = config::get().settings.tasks_view.unwrap_or_default();
        model.update(Action::TasksView { view }.sent_by(Origin::System));
        let range = config::get().settings.sessions_range.unwrap_or_default();
        model.update(Action::SessionsRange { range }.sent_by(Origin::System));
        let settings = &config::get().settings;
        let (on, notify) = (settings.calendar_ics.is_some(), settings.calendar_notify.unwrap_or(true));
        let links_only = settings.calendar_notify_for == Some(ottospace_core::settings::NotifyFor::Links);
        let (hide_free, colors) = (settings.calendar_hide_free.unwrap_or(false), settings.color_rules());
        model.update(Action::SetCalendar { on, notify, links_only, hide_free, colors }.sent_by(Origin::System));
        // Before what a restore reads: a section behind a flag that is off is not put back.
        model.update(Action::SetFeatures { features: features::startup() }.sent_by(Origin::System));
        // A test run's fixture is a transcript's sound: a terminal pane may be heard.
        if listen::fixture_path().is_some() {
            model.update(Action::ListenFixture.sent_by(Origin::System));
        }
        model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        let kept = config::load_tasks();
        let unread = kept.unread;
        // A folder that could not be listed is left alone: writing could replace tasks it holds.
        let keeping = kept.error.is_none();
        model.update(Action::RestoreTasks { tasks: kept.tasks, floor: kept.highest }.sent_by(Origin::System));
        // The first problem found at start, for the toast: settings, the font, then tasks.
        let unread = (unread > 0).then(|| format!("tasks: {unread} file(s) in the tasks folder could not be read"));
        let notice = config::get().error.clone().or(terminal::choose_font(window)).or(kept.error).or(unread);
        // A crashed run's drafts are deleted, not restored.
        editor::sweep();
        // What `settings.json` says at start: what is wrong in it stays in the toast until mended.
        let settings_read = config::settings_stamp();
        if let Some(text) = config::get().home.as_ref().map(|_| config::settings_text().unwrap_or_default()) {
            model.update(Action::SettingsRead { text, written: false }.sent_by(Origin::System));
        }
        model.update(Action::BugRepo { facts: bug::repo_facts(&config::get().settings) }.sent_by(Origin::System));
        let mut this = Self {
            model,
            hosted: HashMap::new(),
            terminals: HashMap::new(),
            viewers: HashMap::new(),
            file_prompts: 0,
            window_picks: 0,
            new_window_asks: [0; 3],
            asked: None,
            titles: HashMap::new(),
            drawer: talk::Drawer::default(),
            cell: terminal::cell_size(window),
            dragging: None,
            drop_at: None,
            divider: None,
            reorder_from: None,
            held: false,
            abandoned: None,
            injected: false,
            launched_later: false,
            marks: Marks::default(),
            clipboard: clipboard::Clipboard::new(std::env::var_os("OTTOSPACE_E2E").is_some()),
            pasting: None,
            clip_reduced: false,
            drag_at: None,
            field: field::Shared::default(),
            pointing: None,
            hover_at: gpui::Point::default(),
            note_field: field::Shared::default(),
            bug_field: field::Shared::default(),
            bug_shot: None,
            bug_pending: false,
            picker_field: field::Shared::default(),
            apps_field: field::Shared::default(),
            mark_field: field::Shared::default(),
            inspector_field: field::Shared::default(),
            btw_field: field::Shared::default(),
            shots: HashMap::new(),
            annotate_at: Rc::default(),
            bar_at: Rc::default(),
            briefs: HashMap::new(),
            no_brief: Default::default(),
            pending: Vec::new(),
            visual_after: Vec::new(),
            key_notice: None,
            recording: None,
            audio_probe: None,
            cells: HashMap::new(),
            frames: HashMap::new(),
            renders: 0,
            dispatched_at: 0,
            seq: 0,
            drawn_seq: 0,
            settles: std::env::var_os("OTTOSPACE_E2E").is_some(),
            settling: Vec::new(),
            frame_start: Rc::default(),
            stall: stall::Stall::new(),
            redraws: 0,
            stall_log: event_log::Streak::default(),
            on_display: None,
            displays_told: None,
            displays_changed: None,
            displays_on: None,
            displays_made_up: false,
            out_windows: Vec::new(),
            keys_in: None,
            main_window: Window::window_handle(window),
            out_frames: HashMap::new(),
            out_controls: sneak::Controls::default(),
            output: false,
            home: home::Shared::default(),
            controls: sneak::Controls::default(),
            chip_line: Rc::default(),
            dial_motion: RefCell::default(),
            seals: seal::Seals::default(),
            slides: overlays::Slides::default(),
            sneak_spent: false,
            sneak_reads: sneak_apps::Reads::default(),
            wheel: app_wheel::Wheel::default(),
            input: Origin::User,
            badges: Default::default(),
            apps: Vec::new(),
            launching: Vec::new(),
            installed: apps::Scan::default(),
            icons: HashMap::new(),
            list_scroll: ScrollHandle::new(),
            calendar_scroll: ScrollHandle::new(),
            calendar_shown: std::cell::Cell::new((0, 0)),
            calendar_opening: std::cell::Cell::new(false),
            calendar_revealed: std::cell::Cell::new(None),
            rail_scroll: ScrollHandle::new(),
            rail_scrolled: RefCell::new((0, None)),
            rail_pending: Rc::new(Cell::new(None)),
            tabs_scroll: ScrollHandle::new(),
            tabs_shown: Cell::new(None),
            bars: scrollbar::Bars::default(),
            minute: 0,
            tags: tags::Reader::start(),
            tags_repo: None,
            worktrees: std::sync::mpsc::channel(),
            removals: std::sync::mpsc::channel(),
            told_day: 0,
            told: None,
            prs: prs::Reader::new(),
            pr_note: None,
            repo_pulse_frames: Cell::default(),
            status_git: status_bar::Reader::new(),
            city: city::Reader::new(),
            city_drawn: city::Shared::default(),
            city_painter: city::Painter::new(),
            status_fit: Cell::default(),
            status_said: None,
            pr_opens: 0,
            filing: Vec::new(),
            calls: 0,
            sessions: sessions::Poller::start(),
            sessions_listed: false,
            sessions_tasks: false,
            sessions_stopped: false,
            sessions_quiet_told: false,
            known: Vec::new(),
            live_since: None,
            left_live: None,
            went_live: 0,
            left_on_activation: None,
            front_hosted: None,
            wanted: None,
            error: None,
            refused: None,
            broke: None,
            bar_line: notice::Held::default(),
            error_seen: (None, None),
            toast: notices::Toast::default(),
            toast_panels: Default::default(),
            notice,
            settings_looked: settings_read,
            settings_read,
            keeping,
            unsaved: None,
            stored: Vec::new(),
            kept_marks: marks::Kept::default(),
            kept_workbooks: workbook::Kept::default(),
            row_hovered: None,
            row_menu_spent: false,
            row_menu_at: None,
            held_effects: Vec::new(),
            ticks: 0,
            kept_layout: layout_file::Kept::default(),
            kept_keys: key_use::Kept::default(),
            caps_drawn: Cell::default(),
            hosting,
            filled: None,
            zoom_asks,
            gallery: false,
            host_only,
            control,
            ax_overlay: None,
            timeline: calendar::Timeline::start(),
            listen_feed: None,
            looks: agent_mode::Looks::default(),
            offer_fixtures: HashMap::new(),
            gone_since: HashMap::new(),
            slot_watch: Vec::new(),
            slot_seen: Vec::new(),
            slot_fixtures: Vec::new(),
            next_fixture_window: 0,
            listen_file: None,
            listen_ending: Vec::new(),
            listen_meter: None,
            listen_downloading: None,
            listen_past: listen::Past::default(),
            focus,
            _subscriptions: subscriptions,
        };
        this.tell_app_settings();
        this.refresh_apps();
        this.offer_installed(cx);
        this.stored = this.kept_tasks(); // what was read needs no writing back
        this.restore_key_use();
        this.restore_workbooks();
        this.restore_marks(cx);
        // After the tasks, workbooks and marks it names; the windows a run left were given back already.
        if !this.restore_layout(window, cx) && hosting {
            // Repos is the way in while there are no panes: most work starts in a repository.
            this.model.update(Action::ToggleSection { section: Section::Repos }.sent_by(Origin::System));
            // Nothing open yet: the keyboard is in the drawer's new task. Not in a test run, the
            // app-window loop's too: its tests start from normal mode.
            if this.model.welcome() && std::env::var_os("OTTOSPACE_E2E").is_none() {
                this.model.update(Action::OpenCompose { text: String::new() }.sent_by(Origin::System));
            }
        }
        this
    }

    /// Makes `origin` the sender of what is handled from now on. Returns the one before, to put
    /// back. For where the handlers run outside any one call (an injected key or pointer event);
    /// everywhere else `with_origin`.
    fn swap_origin(&mut self, origin: Origin) -> Origin {
        // What was said since the last swap was the step of the sender now left.
        self.bar_line = self.held_now();
        self.error_seen = (self.error.clone(), self.error_at());
        std::mem::replace(&mut self.input, origin)
    }

    /// What the bar holds of `error`, with what the sender's step has said since the last swap:
    /// a client's answer is its reply's alone, and its failure never over the user's (`Held::after`).
    pub(crate) fn held_now(&self) -> notice::Held {
        if (&self.error, self.error_at()) == (&self.error_seen.0, self.error_seen.1) {
            return self.bar_line.clone();
        }
        let is = |mark: &Option<String>| self.error.is_some() && self.error == *mark;
        let kind = notice::said(is(&self.refused), is(&self.broke), self.input);
        self.bar_line.after(self.error.clone().map(|text| Notice { kind, text }), self.input)
    }

    /// Where `error`'s text is kept.
    fn error_at(&self) -> Option<usize> {
        self.error.as_ref().map(|error| error.as_ptr() as usize)
    }

    /// The app's own failure (a shell or agent not started, a file not written, a window of
    /// the user's moved): on the bar whoever's step found it.
    pub(crate) fn broke(&mut self, why: String) {
        (self.error, self.broke) = (Some(why.clone()), Some(why));
    }

    /// Who sends a focus that follows a launch: the system's once it comes later than the click,
    /// so it takes no keyboard from what the user types in by then.
    pub(crate) fn follow_up(&self) -> Origin {
        if self.launched_later { Origin::System } else { self.input }
    }

    /// Runs `f` with `origin` as the sender, then puts the previous sender back.
    fn with_origin<R>(&mut self, origin: Origin, f: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.swap_origin(origin);
        let result = f(self);
        self.swap_origin(before);
        result
    }

    /// The one path that changes state: updates the model, then carries out its effects.
    /// `origin`: who sent the action. The person's own keys and clicks pass `self.input`.
    fn dispatch(&mut self, action: Action, origin: Origin, window: &Window, cx: &mut Context<Self>) -> Vec<Effect> {
        // What follows an action is the tick's: it runs each frame for a while. Not the system's:
        // the tick's own, or a thread's that woke it (a redraw it asks for wakes it as a frame).
        if origin != Origin::System {
            ottospace_mac::wake::post();
        }
        // The time it happens at: a task done now is done now, not at the last read.
        let now = now_ms();
        // "New window" waits from the moment it is picked, not from the last read.
        let picks_new = matches!(action, Action::PickerChoose { index } if self.model.chooses_new_window(index));
        let stale = now.saturating_sub(self.model.now()) >= 1_000 || picks_new;
        if stale && !matches!(action, Action::Tick { .. }) {
            self.dispatch(Action::Tick { now }, Origin::System, window, cx);
        }
        // The user's close of an owned window: whether it has unsaved changes, read now.
        if let (Origin::User, Action::Close { pane }) = (origin, &action) {
            self.edited_now(pane.or(self.model.focused), window, cx);
        }
        // Opening the Repos section reads the list again: repositories come and go on disk.
        let opens_repos = self.model.sidebar != Some(Section::Repos);
        if opens_repos && matches!(action, Action::ToggleSection { section: Section::Repos }) {
            self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        }
        // Opening the Apps section too: apps start and quit, and the Dock changes.
        if self.model.sidebar != Some(Section::Apps)
            && matches!(action, Action::ToggleSection { section: Section::Apps })
            && self.on(Feature::AppsSection)
        {
            self.refresh_apps();
            self.offer_installed(cx);
        }
        // A client's key, click or label at the user's draft, the drawer's ⌘⇧R or →, Done and Later, or a
        // done task's Clear: its reply's `notice` says why it did nothing.
        let said = [
            ottospace_core::capture::DRAFT_USERS,
            ottospace_core::model::rail::DRAWER_USERS,
            ottospace_core::model::finish::USERS_ALONE,
            ottospace_core::model::finish::NEW_TASK_USERS,
            ottospace_core::tasks::CLEAR_USERS,
            ottospace_core::model::decisions::USERS_ALONE,
        ];
        if let Some(why) =
            self.model.refusal(&action, origin).filter(|why| origin == Origin::Socket && said.contains(why))
        {
            self.error = Some(why.into());
        }
        let line = self.model.compose_line().cloned().map(|line| (line, self.model.compose_typing()));
        let field = self.model.field().map(|(of, _)| of);
        // The core has no clock: a task filed now is stamped with the time told here.
        // So is the notice that offers a mouse move back: it counts its seconds from here.
        let files = matches!(
            action,
            Action::TaskAdd { .. }
                | Action::TaskFromLine { .. }
                | Action::ComposeSubmit { .. }
                | Action::ComposeEnter
                | Action::SessionAdopt { .. }
        );
        // So is a capture. And what is open is read again just before a draft is added.
        let captures = matches!(action, Action::CaptureStart { .. });
        if matches!(action, Action::ComposeSubmit { .. } | Action::ComposeEnter) && self.model.capture.is_some() {
            self.model.update(Action::CaptureFacts { facts: self.pane_facts() }.sent_by(Origin::System));
        }
        let row = matches!(action, Action::RowSnooze { .. } | Action::RowHold { .. } | Action::RowDismiss { .. });
        // And the offer to put a removed mark back.
        let unmarks = matches!(action, Action::MarkRemove { .. } | Action::MarkToggle { .. } | Action::ListDismiss);
        // And the offer to take a pin, an unpin or a hidden suggestion back.
        let pins = matches!(action, Action::AppPin { .. } | Action::AppHide { .. } | Action::ListPin);
        // And a paste: its pane pulses from now.
        let pastes = matches!(action, Action::Paste { .. } | Action::QuitConfirm);
        if files || row || unmarks || pins || captures || pastes {
            self.tick_now();
        }
        let name = trace::name(&action);
        // An answer on the Decisions card: its seal plays over the pane while the next card takes keys.
        let answering = matches!(action, Action::DecisionsAsk { .. }).then(|| self.model.decision_card()).flatten();
        let effects = self.model.update(action.sent_by(origin));
        if answering.is_some() && self.model.decision_card().is_some_and(|card| Some(card) != answering) {
            if let Some(pane) = self.model.decisions.as_ref().map(|run| run.pane) {
                self.card_seal_start(pane);
            }
        }
        self.seq += 1;
        let made: Vec<String> = effects.iter().map(|effect| trace::name(effect)).collect();
        let fields =
            serde_json::json!({ "seq": self.seq, "name": name, "origin": trace::name(&origin), "effects": made });
        trace::record("action", fields);
        if self.model.field().map(|(of, _)| of) != field {
            self.field_changed_hands();
        }
        // The line in the new-task box changed: ask what its tags may name. The answer comes
        // later (`take_tags`): reading it is disk I/O, never done here.
        // Also when the cursor moved into another path: what finishes it is listed for it.
        match self.model.compose_line().cloned().map(|line| (line, self.model.compose_typing())) {
            Some(now) if line.as_ref() != Some(&now) => self.ask_tags(now.0, now.1),
            Some(_) => {}
            None => self.tags_repo = None,
        }
        self.dispatched_at = self.renders;
        self.store_tasks();
        self.store_marks();
        self.store_workbooks();
        self.layout_changed();
        // New marks need looking for; what was found is believed a while, so this is cheap.
        self.check_marks(cx);
        self.load_icons(cx);
        // A brief that shows is read from its task's folder, once.
        if let Some(task) = self.model.brief {
            self.brief_of(task);
        }
        let held = std::mem::take(&mut self.held_effects);
        if !held.is_empty() {
            self.with_origin(Origin::System, |this| this.carry_out(&held, window, cx));
        }
        // What an effect dispatches in turn (a picked app's pane, a label's control) has the same sender.
        self.with_origin(origin, |this| this.carry_out(&effects, window, cx));
        self.sync_out_windows(cx);
        self.follow_keys(cx);
        cx.notify();
        effects
    }

    fn carry_out(&mut self, effects: &[Effect], window: &Window, cx: &mut Context<Self>) {
        let another_goes_live = effects.iter().any(|effect| matches!(effect, Effect::Live(_)));
        // Every window of this pass that could not be hosted, not only the last.
        let asked = effects.iter().any(|effect| matches!(effect, Effect::HostWindow { .. }));
        let mut unhosted: Vec<String> = Vec::new();
        // What became of the agents asked for, told once the effects are carried out.
        let mut agents: Vec<Action> = Vec::new();
        for effect in effects {
            match effect {
                Effect::StartAgent { task, start, cwd, brief } => {
                    // Before the program starts: a session it writes cannot say it began earlier.
                    let at = now_ms();
                    match self.start_agent(task, start, cwd.as_deref(), brief.as_deref()) {
                        Ok(session) => agents.push(Action::AgentStarted { task: task.clone(), session, at }),
                        Err(why) => {
                            self.broke(format!("agent: {why}"));
                            agents.push(Action::AgentExited { task: task.clone() });
                        }
                    }
                }
                Effect::StopAgent { task } => agents.extend(self.stop_agent(task)),
                Effect::AgentSend { task, text } => self.native_input(task, NativeInput::Send(text.clone())),
                Effect::AgentAnswer { task, ticket, allow, message } => {
                    let answer = NativeInput::Answer { ticket: *ticket, allow: *allow, message: message.clone() };
                    self.native_input(task, answer);
                }
                Effect::AgentAnswers { task, ticket, answers } => {
                    let answers = NativeInput::Answers { ticket: *ticket, answers: answers.clone() };
                    self.native_input(task, answers);
                }
                Effect::AgentInterrupt { task } => self.native_input(task, NativeInput::Interrupt),
                Effect::AgentSideQuestion { task, id, question, history } => {
                    agents.extend(self.side_question(task, *id, question, history))
                }
                Effect::AgentMoved { from, to } => self.move_agent(from, to),
                Effect::MeetingAlert { .. } | Effect::OpenLink { .. } | Effect::CalendarRetry => {
                    self.meeting_effect(effect)
                }
                Effect::ReadSession { runner, id } => agents.extend(self.read_session(runner, id)),
                Effect::HostSlot { pane, pid, window: id } => self.host_slot(*pane, *pid, *id, window, cx),
                Effect::WatchSlots { apps } => {
                    self.slot_watch = apps.clone();
                    self.slot_seen.retain(|look| {
                        apps.iter().any(|(b, n)| {
                            ottospace_core::marks::same_app((b.as_deref(), n), (look.bundle.as_deref(), &look.name))
                        })
                    });
                }
                Effect::GiveBack { pane } => self.give_back_slot(*pane),
                // The log's line is written where the offer was sent (`offer_window`).
                Effect::Placed { .. } => {}
                Effect::HostOffer { pane, pid, window: id } => self.host_offer(*pane, *pid, *id, window, cx),
                Effect::Opened(id) => {
                    if self.model.pane(*id).is_some_and(|p| p.kind == Kind::Terminal) {
                        // The shell learns where it runs, so a program in it can name its own pane.
                        let tab = self.model.tab_of(*id);
                        let mut vars = vec![("OTTOSPACE_PANE", id.to_string())];
                        vars.extend(tab.map(|tab| ("OTTOSPACE_TAB", tab.to_string())));
                        // A terminal opened for a task names it: an agent there can ask for its brief.
                        let task = self.model.tabs.iter().find(|t| Some(t.id) == tab).and_then(|t| t.space.task());
                        vars.extend(task.map(|task| ("OTTOSPACE_TASK", task.to_string())));
                        let cwd = self.model.pane(*id).and_then(|pane| pane.cwd.as_deref());
                        // A workbook's terminal shows what it showed when saved: the core says so next.
                        let history = effects.iter().find_map(|effect| match effect {
                            Effect::History { pane, history } if pane == id => Some(history),
                            _ => None,
                        });
                        let spawned = Instant::now();
                        // Open Settings, or ⌃G in a viewer: the editor on the file, not a shell.
                        let edits = effects.iter().find_map(|effect| match effect {
                            Effect::EditFile { pane, path } if pane == id => Some(path),
                            _ => None,
                        });
                        let started = if effects.contains(&Effect::EditSettings { pane: *id }) {
                            let file = config::settings_file().map_err(std::io::Error::other);
                            file.and_then(|file| Terminal::edit(&file.to_string_lossy(), &vars))
                        } else if let Some(path) = edits {
                            Terminal::edit(path, &vars)
                        } else {
                            Terminal::spawn(&vars, cwd, history)
                        };
                        match started {
                            Ok(terminal) => drop(self.terminals.insert(*id, terminal)),
                            Err(err) => self.broke(format!("terminal: {err}")),
                        }
                        let (took, ok) = (spawned.elapsed().as_millis() as u64, self.terminals.contains_key(id));
                        trace::record("terminal_spawn", serde_json::json!({ "pane": id, "took_ms": took, "ok": ok }));
                    }
                }
                Effect::Closed(pane) => {
                    self.cells.remove(&pane.id);
                    self.frames.remove(&pane.id);
                    self.marks.borrow_mut().retain(|mark, _| {
                        ![Mark::Header(pane.id), Mark::Close(pane.id), Mark::ViewerMarked(pane.id)].contains(mark)
                    });
                    self.terminals.remove(&pane.id); // drop ends its shell
                    self.viewers.remove(&pane.id);
                    if let Some(hosted) = self.hosted.remove(&pane.id) {
                        hosted.release();
                    }
                }
                Effect::Parked(id) => {
                    let live_since = self.live_since.take();
                    let keep = self.bars_leave();
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        trace::record("park", serde_json::json!({ "pane": id, "window": hosted.window_id }));
                        let limit = top_limit(window, cx);
                        let rect = on_screen(hosted.bounds.get(), window, cx);
                        let crop = hosted.crop_at(rect, limit);
                        let scale = window.scale_factor() as f64;
                        hosted.park(pane_rect(rect, keep, false, crop).size, scale, crop);
                        hosted.resume(scale);
                        // Live under a second: `follow_front_app` may not have seen the app's own
                        // activation from going live yet, and must not take it for the user's.
                        if live_since.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
                            self.left_live = Some((hosted.pid, Instant::now()));
                        }
                        // Going live activated the app. If it still has the keyboard, take it
                        // back, or keys keep going to its off-screen window while the status
                        // bar shows no mode. If the user moved to a third app, leave them there.
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
                Effect::PickWindows { app } => self.pick_windows(app, window, cx),
                Effect::NewWindow { app, bundle, recipe } => {
                    self.new_window(app, bundle.as_deref(), *recipe, window, cx)
                }
                Effect::CloseWindow { pane, window: id } => self.close_window(*pane, *id),
                Effect::Failed { text } => self.broke(text.clone()),
                Effect::HostPicked { app, window: id, axis, new_tab } => {
                    self.host_picked(app, *id, *axis, *new_tab, window, cx)
                }
                Effect::LaunchApp { name, path, axis, new_tab } => self.launch_picked(name, path, *axis, *new_tab),
                Effect::SaveApps { changes } => self.save_apps(changes),
                Effect::SaveFeature { feature, on } => self.save_feature(*feature, *on, cx),
                Effect::SaveTasksView { view } => {
                    if let Err(why) = config::save_value("tasks_view", &serde_json::json!(view)) {
                        self.broke(format!("{}{why}", Self::NOT_WRITTEN));
                    }
                }
                // Later reads of the list look where the model now does (also after "Unlist?" is
                // answered); a file not written leaves them as the settings are.
                Effect::SaveRepos { dirs } => {
                    let said = self.error.take();
                    self.save_list(|| config::save_repos(dirs));
                    let failed = self.error.as_ref().is_some_and(|error| error.starts_with(Self::NOT_WRITTEN));
                    self.error = self.error.take().or(said);
                    if failed {
                        self.model.update(Action::SetRepoDirs { dirs: config::repo_dirs() }.sent_by(Origin::System));
                        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
                    } else {
                        config::set_repo_roots(&self.model.repo_dirs);
                    }
                }
                Effect::OpenPr { url } if ottospace_core::prs::openable(url) => match std::env::var_os("OTTOSPACE_E2E")
                {
                    Some(_) => self.pr_opens += 1,
                    None => drop(ottospace_mac::meeting::open_url(url)),
                },
                Effect::OpenPr { .. } => {}
                Effect::AddWorktree { repo, branch, path, root, fresh } => {
                    let done = self.worktrees.0.clone();
                    // Only in the folder the settings name now, whatever the core was told.
                    let root = Some(root.clone()).filter(|root| config::worktrees_dir().as_ref() == Some(root));
                    worktree::start(repo.clone(), branch.clone(), path.clone(), root, *fresh, done);
                }
                Effect::RemoveWorktree { task, repo, path } => {
                    let root = config::worktrees_dir();
                    worktree::start_remove(*task, repo.clone(), path.clone(), root, self.removals.0.clone());
                }
                Effect::Notice { text } => (self.error, self.refused) = (Some(text.clone()), Some(text.clone())),
                Effect::Seal { task } => self.seal_start(*task),
                Effect::ListenWrite { .. }
                | Effect::ListenOpen { .. }
                | Effect::ListenPauseStreams { .. }
                | Effect::ListenResumeStreams { .. }
                | Effect::ListenClose { .. } => self.listen_effect(effect, window, cx),
                Effect::NoticeDismissed { kind } => self.notice_dismissed(*kind),
                Effect::Told { text } => self.told = Some((text.clone(), Instant::now())),
                // A picture of the pane is asked for: the cursor's going scrolls the terminal to
                // its bottom, so it waits until the picture of what is on show is taken.
                Effect::Visual { pane, on: false } if self.snapshot_pending(*pane) => self.visual_after.push(*pane),
                Effect::Visual { pane, on } => {
                    if let Some(terminal) = self.terminals.get(pane) {
                        terminal.set_visual(*on);
                    }
                }
                Effect::Activate { id } => self.activate(id, window, cx),
                Effect::FillDisplay { on } => self.fill_display(*on, window, cx),
                Effect::Run { command } => self.run_command(*command, window, cx),
                Effect::PressApp { pane, path, role, at, .. } => self.press_app(*pane, path, role, *at, window, cx),
                Effect::HostWindow { mark, bundle, name, title, into, axis, launch } => {
                    let (bundle, name, title) = (bundle.clone(), name.clone(), title.clone());
                    let (mark, into, axis, launch) = (*mark, *into, *axis, *launch);
                    let wanted = marks::Wanted { mark, bundle, name, title, into, axis, launch };
                    if let Err(why) = self.host_window(wanted, window, cx) {
                        // A rebuilt tab's pane stays empty, and says why.
                        if let Some(pane) = into {
                            self.model.update(Action::Unfilled { pane, why: why.clone() }.sent_by(Origin::System));
                        }
                        if !unhosted.contains(&why) && notice::unhosted_on_bar(self.input, into.is_some()) {
                            unhosted.push(why);
                        }
                    }
                }
                // Carried out with its pane's `Opened`, before the shell starts.
                Effect::History { .. } | Effect::EditSettings { .. } | Effect::EditFile { .. } => {}
                // Read again from the start: the next tick does (`sync_viewers`).
                Effect::ViewFile { pane } => drop(self.viewers.remove(pane)),
                Effect::ViewerScroll { pane, top } => self.viewer_scroll(*pane, *top, cx),
                Effect::ChooseFile { axis, new_tab } => self.choose_file(*axis, *new_tab, window, cx),
                Effect::OpenEditor { text } => {
                    if let Err(why) = self.open_editor(text) {
                        self.error = Some(why);
                        agents.push(Action::EditorDone { saved: None, more: false });
                    }
                }
                Effect::Clip { text } => self.copy_text(text.clone(), None, Verb::Copied, cx),
                Effect::ApplySettings { settings } => {
                    self.apply_features(settings, cx);
                    agents.push(Action::BugRepo { facts: bug::repo_facts(settings) });
                    config::set_live(settings.as_ref().clone())
                }
                // The text read at ⌘V, into the pane it was read for.
                Effect::Paste { pane } => {
                    let text = self.pasting.take().filter(|(at, _)| at == pane).map(|(_, text)| text);
                    if let (Some(terminal), Some(text)) = (self.terminals.get(pane), text) {
                        terminal.paste(&text);
                    }
                }
                Effect::ForgetLayout => self.forget_layout(),
                // An agent moved into a folder the list does not hold yet: a worktree just made.
                Effect::ReadRepos => {
                    self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
                }
                Effect::Snapshot { draft, material, pane } => self.ask_snapshot(draft, *material, *pane),
                Effect::CaptureDropped { draft, material } => self.drop_capture(draft, *material),
                Effect::CaptureKept { draft, dir, brief } => self.keep_capture(draft, dir, brief, cx),
                Effect::BugDropped { word } => self.bug_dropped(word),
                Effect::BugFiled { word, dir, blank, brief } => self.bug_file(word, dir, blank, brief, cx),
                Effect::Quit => {
                    // A moment later, so a socket `quit` gets its reply first; one still held for
                    // a frame goes now.
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        let _ = this.update_in(cx, |this, window, cx| this.settle(true, window, cx));
                        let _ = cx.update(|_, cx| cx.quit());
                    })
                    .detach();
                }
                Effect::Live(id) => {
                    self.went_live += 1;
                    let keep = self.bars_leave();
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        trace::record("unpark", serde_json::json!({ "pane": id, "window": hosted.window_id }));
                        hosted.pause();
                        hosted.bring_up(keep, window, cx);
                    }
                    // After the blocking stop and AX calls: the activation they delay is still ignored.
                    self.live_since = Some(Instant::now());
                }
            }
        }
        if asked {
            self.error = (!unhosted.is_empty()).then(|| unhosted.join("; "));
        }
        // The app wrote `settings.json`: the toast speaks of the file as written.
        let wrote = effects.iter().any(|effect| {
            matches!(effect, Effect::SaveApps { .. } | Effect::SaveRepos { .. } | Effect::SaveTasksView { .. })
        });
        if wrote {
            self.read_settings_now(true, window, cx);
        }
        for news in agents {
            self.dispatch(news, Origin::System, window, cx);
        }
    }

    /// Asks the reader what the tags of the line in the new-task box may name. Agents and skills
    /// are read when the box opens and when the line's `>repo` changes; paths for every line.
    fn ask_tags(&mut self, line: String, typing: Option<String>) {
        let repo = self.model.draft_repo(&line);
        let names = self.tags_repo.as_ref() != Some(&repo);
        self.tags_repo = Some(repo.clone());
        if let Some(reader) = &self.tags {
            reader.ask(tags::Job { call: 0, line, typing, repo, names });
        }
    }

    /// Takes in what the reader has read since: for the box (the core drops an answer for a line
    /// that has changed since), and for the socket calls that waited for it.
    fn take_tags(&mut self, window: &Window, cx: &mut Context<Self>) {
        let read = self.tags.as_ref().map(tags::Reader::take).unwrap_or_default();
        for tags::Read { call, line, typing, names, paths, entries } in read {
            if call == 0 {
                let known = Action::SetKnown { line, typing, names, paths, entries };
                self.dispatch(known, Origin::System, window, cx);
                cx.notify();
                continue;
            }
            let Some(at) = self.filing.iter().position(|(number, _, _)| *number == call) else { continue };
            let (_, filing, reply) = self.filing.remove(at);
            let known = compose::Known { names: names.unwrap_or_default(), paths, entries: Vec::new() };
            let before = self.model.tasks.list.len();
            // `task_new` with `open`: whether the task's tabs are now on show. A client's new task
            // has none: only the user links a tab to a task.
            let mut opened = None;
            let refused = match filing {
                Filing::Line { open } => {
                    self.dispatch(Action::TaskFromLine { line, known, open }, Origin::Socket, window, cx);
                    opened = open.then(|| {
                        self.model.tasks.list.get(before).is_some_and(|t| self.model.tab_task() == Some(t.id))
                    });
                    "the line has no title".to_string()
                }
                Filing::Task { title, priority, state, tab, assign } => {
                    match compose::unknown(&known.names, assign.agent.as_deref(), &assign.skills) {
                        Some(name) => format!(
                            "`{name}` is not an agent or a skill this ottospace knows; `agent` and `skills` take the names `task_new` completes"
                        ),
                        None => {
                            // As they are spelled where they were read, as `task_new` keeps them.
                            let spell = |names: &[String], given: String| {
                                compose::spelled(names, &given).cloned().unwrap_or(given)
                            };
                            let agent = assign.agent.map(|agent| compose::agent(&known.names, &agent).unwrap_or(agent));
                            let skills =
                                assign.skills.into_iter().map(|skill| spell(&known.names.skills, skill)).collect();
                            let assign = Assign { agent, skills, ..assign };
                            self.dispatch(
                                Action::TaskAdd { title, priority, state, tab, assign },
                                Origin::Socket,
                                window,
                                cx,
                            );
                            String::new()
                        }
                    }
                }
            };
            // The new task, so the caller has its id.
            let message = match self.model.tasks.list.get(before) {
                Some(task) => {
                    let mut message = serde_json::json!({ "ok": true, "task": task });
                    if let Some(opened) = opened {
                        message["opened"] = serde_json::json!(opened);
                    }
                    message.to_string()
                }
                None => control::error(&refused),
            };
            let _ = reply.send(message);
            cx.notify();
        }
    }

    /// Files a task for a socket call once the reader has read what its tags may name: the
    /// reply waits for that, the window does not.
    fn file_when_read(&mut self, filing: Filing, line: String, reply: Sender<String>) {
        let Some(reader) = &self.tags else {
            return drop(reply.send(control::error("tags cannot be read: the reader did not start")));
        };
        self.calls += 1;
        let repo = self.model.draft_repo(&line);
        reader.ask(tags::Job { call: self.calls, line, typing: None, repo, names: true });
        self.filing.push((self.calls, filing, reply));
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

    /// Reads `settings.json` again once a change to it has held for one look: a file an editor
    /// is still writing, or has briefly renamed away, is not read.
    fn watch_settings(&mut self, window: &Window, cx: &mut Context<Self>) {
        let now = config::settings_stamp();
        let held = now == self.settings_looked;
        self.settings_looked = now;
        if held {
            self.read_settings_now(false, window, cx);
        }
    }

    /// Reads `settings.json` into the core if it changed since the last read. Gone: as at a
    /// start with no file, the defaults. `written`: the app has just written it.
    fn read_settings_now(&mut self, written: bool, window: &Window, cx: &mut Context<Self>) {
        let now = config::settings_stamp();
        if now == self.settings_read || config::get().home.is_none() {
            return;
        }
        let text = match now {
            Some(_) => match config::settings_text() {
                Some(text) => text,
                None => return,
            },
            None => String::new(),
        };
        (self.settings_looked, self.settings_read) = (now, now);
        drop(self.dispatch(Action::SettingsRead { text, written }, Origin::System, window, cx));
        cx.notify();
    }

    /// Open Settings: the user's editor on `settings.json` in a tab of its own, made `{}` first
    /// if there is none.
    pub(crate) fn edit_settings(&mut self, window: &Window, cx: &mut Context<Self>) {
        match config::settings_file() {
            Ok(_) => drop(self.dispatch(Action::EditSettings, self.input, window, cx)),
            Err(why) => self.error = Some(why),
        }
        cx.notify();
    }

    /// The global return key: take the keyboard back from a live app, and bring ottospace to the
    /// front from wherever the user is.
    fn return_to_ottospace(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::Leave, self.input, window, cx);
        cx.activate(true);
    }

    /// Answers queued control-socket calls. Each reply is the state after the call.
    fn serve_control(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.settle(false, window, cx);
        // What a request reaches through a shared method (`open_app`, a click) is the client's too.
        self.with_origin(Origin::Socket, |this| this.serve_calls(window, cx));
    }

    /// After a tick: what the next must look at (`cadence`). Busy: something moves, or waits for
    /// what no event announces. Slow: a look with no event of its own. Due: the next write or read.
    fn load(&self, cx: &App) -> cadence::Load {
        let model = &self.model;
        let now = now_ms();
        let clip = model.clip.as_ref().filter(|clip| clip.pane.is_some());
        let talk = model.talk.as_ref().filter(|talk| talk.open);
        let busy = !self.hosted.is_empty()
            || !self.launching.is_empty()
            || !self.settling.is_empty()
            || !self.filing.is_empty()
            || !self.pending.is_empty()
            || !self.held_effects.is_empty()
            || self.dragging.is_some()
            || cx.has_active_drag()
            || model.sneak.is_some()
            || model.row_menu().is_some()
            || model.agent.menu.is_some()
            || clip.is_some_and(|clip| now.saturating_sub(clip.at) < clip::PULSE_MS + 100)
            || self.toast.moves()
            || self.notice_changed()
            || talk.is_some_and(|talk| model.agent_runs(&talk.task))
            || self.stall.watching()
            || !self.drawer.native.is_empty()
            || !self.drawer.stopping.is_empty()
            || model.calendar.on && (self.timeline.pulse || self.timeline.sliding);
        // An app hosted once: its new windows are adopted, its panes all gone or not.
        let slow = !self.terminals.is_empty()
            || !self.known.is_empty()
            || config::get().home.is_some()
            || model.calendar.on
            || model.inspector.is_some()
            || matches!(model.sidebar, Some(Section::Marks | Section::Workbook | Section::Repos | Section::Tasks))
            || talk.is_some()
            || !model.seeking().is_empty()
            || model.listen.shown
            || model.fill_display
            || self.drawer.editor.is_some()
            || self.notice_shown().is_some();
        // In the loop's ms: a time of the wall clock, or an instant.
        let (ms, instant) = (loop_ms(), Instant::now());
        let wall = |at: u64| ms + at.saturating_sub(now);
        let soon = |at: Instant| ms + at.saturating_duration_since(instant).as_millis() as u64;
        // A save only while one is written; the told line only until it goes.
        let layout = self.kept_layout.due.filter(|_| self.kept_layout.writing);
        let keys = self.kept_keys.due.filter(|_| self.kept_keys.writing);
        let told = self.told.as_ref().map(|(_, at)| *at + TOLD_FOR).filter(|end| *end > instant);
        let instants = [layout, keys, told];
        let walls = [model.next_due(), self.sessions.as_ref().and_then(sessions::Poller::next_ask)];
        let due = instants.into_iter().chain([self.viewers_due()]).flatten().map(soon);
        let due = due.chain(walls.into_iter().flatten().map(wall)).min();
        cadence::Load { busy, slow, due }
    }

    /// The displays, told to the core at start, when the main window changes display, and after
    /// a change: one gone at once, one new after 1 s of quiet, so no window opens on a display
    /// still in flux. A test run's made-up list stands for them from its first (`displays_set`).
    fn watch_displays(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.displays_made_up {
            return;
        }
        // By number, cheaply each tick; its UUID only when told.
        let on = window.display(cx).map(|d| u32::from(d.id()));
        let moved = self.displays_told.is_some() && on != self.displays_on;
        if !(self.displays_told.is_none() || self.displays_changed.is_some() || moved) {
            return;
        }
        self.displays_on = on;
        let settled = self.displays_changed.is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        // Each tab out's window too is kicked to draw while the change settles (`stall`, #207).
        if !settled {
            for handle in self.out_windows.iter().filter_map(|(_, handle)| *handle) {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| {
                        // SAFETY: GPUI's own view, on the main thread, outside any update of its window.
                        ns_view(window).map(|view| unsafe { ottospace_mac::window::redraw(view) })
                    });
                });
            }
        }
        let mut list = display::list();
        if !settled {
            let known: Vec<String> =
                self.displays_told.iter().flat_map(|(told, _)| told.iter().map(|d| d.uid.clone())).collect();
            list.retain(|d| known.contains(&d.uid));
        }
        if settled {
            self.displays_changed = None;
        }
        let told = (list, on.and_then(display::uuid));
        if self.displays_told.as_ref() != Some(&told) {
            self.displays_told = Some(told.clone());
            let (displays, main) = told;
            self.dispatch(Action::SetDisplays { displays, main }, Origin::System, window, cx);
        }
    }

    /// Each tick: after a change of the displays, or while the shown window has something to draw
    /// and draws nothing, GPUI is kicked to draw again (`stall`). The tick is a timer, not a
    /// frame: it goes on when frames stop.
    fn watch_frames(&mut self, window: &mut Window, cx: &App) {
        let now = Instant::now();
        if display::take_change() {
            self.displays_changed = Some(now);
            self.stall.displays_changed(now);
            trace::record("displays", serde_json::json!({ "count": display::all().len() }));
            event_log::event("displays", displays());
        }
        // The display the window is on, by number: said when it changes.
        let on = window.display(cx).map(|d| u32::from(d.id()));
        if on != self.on_display {
            self.on_display = on;
            let size = on.map(|id| size_of(display::bounds(id)));
            event_log::event("window_display", serde_json::json!({ "display": on, "size": size }));
        }
        let wants = self.seq > self.drawn_seq || !self.settling.is_empty() || self.output;
        // SAFETY: GPUI's own view, on the main thread.
        let shown = ns_view(window).is_some_and(|view| unsafe { ottospace_mac::window::visible(view) });
        match self.stall.check(now, self.renders, wants, shown) {
            stall::Verdict::Nothing => {}
            stall::Verdict::Kick { stalled, first } => {
                let stalled = stalled.map(|waited| waited.as_millis() as u64);
                trace::record("redraw", serde_json::json!({ "stalled_ms": stalled, "first": first }));
                // A stall once, as it starts; not each kick after it, nor those after a display change.
                if let Some(n) = first.then(|| self.stall_log.stall(now)).flatten() {
                    event_log::event("stall", serde_json::json!({ "stalled_ms": stalled, "n": n }));
                }
                // SAFETY: GPUI's own view, on the main thread, in a timer's update: GPUI holds no
                // lock of its window here.
                // Counted only when GPUI's handlers were there to take it.
                if ns_view(window).is_some_and(|view| unsafe { ottospace_mac::window::redraw(view) }) {
                    self.redraws += 1;
                }
                window.refresh();
            }
            stall::Verdict::Recovered { kicks, lasted } => {
                let lasted = lasted.as_millis() as u64;
                trace::record("redrawn", serde_json::json!({ "kicks": kicks, "lasted_ms": lasted }));
                if let Some(n) = self.stall_log.redrawn() {
                    event_log::event("redrawn", serde_json::json!({ "kicks": kicks, "lasted_ms": lasted, "n": n }));
                }
            }
        }
    }

    /// Replies with the state. In a test run, not before a frame has drawn every action so far,
    /// so the bounds in the reply are of the new layout, never the one before it.
    fn answer(&mut self, reply: Sender<String>, window: &Window, cx: &mut Context<Self>) {
        if self.settles && (self.drawn_seq < self.seq || self.toast.moves() || self.notice_changed()) {
            self.settling.push((Instant::now(), reply));
            cx.notify();
        } else {
            let _ = reply.send(self.snapshot(window, cx));
        }
    }

    /// Sends the held replies once the last frame drew every action so far: the state they
    /// carry is the current one, so an action since their own must be drawn too. One that
    /// waited `SETTLE_LIMIT`, or all of them with `now` (the app quits), goes as it is, marked
    /// `"stale":true` if undrawn: the window drew no frame (hidden, or stuck).
    fn settle(&mut self, now: bool, window: &Window, cx: &mut Context<Self>) {
        // Nor while the toast comes or goes: its rects are of it at rest.
        let drawn = self.drawn_seq >= self.seq && !self.toast.moves() && !self.notice_changed();
        let (due, waiting) = std::mem::take(&mut self.settling)
            .into_iter()
            .partition(|(since, _)| drawn || now || since.elapsed() >= SETTLE_LIMIT);
        self.settling = waiting;
        for (since, reply) in due {
            let mut state = self.state(window, cx);
            if !drawn {
                state["stale"] = serde_json::json!(true);
                let fields = serde_json::json!({ "what": "settle", "seq": self.seq, "drawn": self.drawn_seq });
                trace::record("timeout", fields);
            }
            state["frame"]["waited_ms"] = serde_json::json!(since.elapsed().as_millis() as u64);
            let _ = reply.send(state.to_string());
        }
    }

    fn serve_calls(&mut self, window: &Window, cx: &mut Context<Self>) {
        let calls: Vec<Call> = self.control.iter().flat_map(|rx| rx.try_iter()).collect();
        // The reply's marks say what is missing, as last found; look again if that is old.
        if !calls.is_empty() {
            self.check_marks(cx);
        }
        for Call { request, reply, pass } in calls {
            if let Some(why) = control::test_refusal(&request, std::env::var_os("OTTOSPACE_E2E").is_some()) {
                let _ = reply.send(control::error(why));
                continue;
            }
            if let Some(why) = control::feature_refusal(&request, &self.model.features) {
                let _ = reply.send(control::error(why));
                continue;
            }
            // A test's `"as":"user"`: the event is the user's, and nothing kept from a client applies.
            let sender = |by: &Option<control::By>| by.map_or(Origin::Socket, |control::By::User| Origin::User);
            // Keys go through GPUI's own dispatch, after this update: their handlers update `self`.
            if let Request::Key { keys, by, repeat } = &request {
                let parsed: Result<Vec<Keystroke>, _> = keys.split_whitespace().map(Keystroke::parse).collect();
                match parsed {
                    Ok(keystrokes) if *repeat => self.hold(keystrokes, sender(by), reply, window, cx),
                    Ok(keystrokes) => self.press(keystrokes, sender(by), reply, window, cx),
                    Err(err) => drop(reply.send(control::error(&err.to_string()))),
                }
                continue;
            }
            let origin = match &request {
                Request::Mouse { by, .. } | Request::Drag { by, .. } => sender(by),
                _ => Origin::Socket,
            };
            // `compose_drop` is a drop from the pointer: refused where a click is.
            let pointer = origin == Origin::Socket
                && matches!(
                    request,
                    Request::Mouse { .. }
                        | Request::Drag { .. }
                        | Request::MouseDown { .. }
                        | Request::MouseDrag { .. }
                        | Request::ComposeDrop { .. }
                );
            if pointer && self.model.sneak.is_some() {
                let _ = reply.send(control::error(sneak::ON));
                continue;
            }
            // Nor while a mark waits for the user's letter or name: a click ends the wait.
            if pointer && self.model.mark_input.is_some() {
                let _ = reply.send(control::error(ottospace_core::marks::INPUT_ON));
                continue;
            }
            // Nor under the quit question: a client's click outside the box would cancel it.
            if pointer && self.model.confirm.is_some() {
                let _ = reply.send(control::error(CONFIRM_UP));
                continue;
            }
            // So do pointer events, through the window's own handlers: a drag is GPUI's state.
            let events = match &request {
                Request::Mouse { action, button, x, y, clicks, modifiers, dx, dy, precise, .. } => {
                    let has = |m: control::Modifier| modifiers.contains(&m);
                    let mouse = Mouse {
                        kind: match action {
                            MouseAction::Wheel => {
                                MouseKind::Wheel { dx: f64::from(*dx), dy: f64::from(*dy), precise: *precise }
                            }
                            MouseAction::Exit => MouseKind::Exit,
                            MouseAction::Enter => MouseKind::Enter,
                            _ => MouseKind::Move,
                        },
                        x: f64::from(*x),
                        y: f64::from(*y),
                        clicks: (*clicks).min(3) as isize,
                        cmd: has(control::Modifier::Cmd),
                        ctrl: has(control::Modifier::Ctrl),
                        alt: has(control::Modifier::Alt),
                        shift: has(control::Modifier::Shift),
                    };
                    // The right button: a press or a release, never held for a drag.
                    let event = match (button, action) {
                        (control::Button::Right, MouseAction::Down) => Mouse { kind: MouseKind::RightDown, ..mouse },
                        (control::Button::Right, MouseAction::Up) => Mouse { kind: MouseKind::RightUp, ..mouse },
                        _ => self.pointer(*action, mouse),
                    };
                    self.abandoned = self.held.then(|| self.release_later(window, cx));
                    Some(vec![event])
                }
                Request::Drag { from, to, steps, .. } => {
                    // At most 256 moves: more adds nothing, and each draws a frame.
                    let steps = (*steps).clamp(1, 256);
                    let along = |i: usize| {
                        let t = i as f32 / steps as f32;
                        let (x, y) = (from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t);
                        let (cmd, ctrl, alt, shift) = (false, false, false, false);
                        Mouse { kind: MouseKind::Move, x: x.into(), y: y.into(), clicks: 1, cmd, ctrl, alt, shift }
                    };
                    let mut events = vec![self.pointer(MouseAction::Down, along(0))];
                    events.extend((1..=steps).map(|i| self.pointer(MouseAction::Move, along(i))));
                    events.push(self.pointer(MouseAction::Up, along(steps)));
                    Some(events)
                }
                _ => None,
            };
            if let Some(events) = events {
                self.inject(events, origin, reply, window, cx);
                continue;
            }
            let mut failure: Option<String> = None;
            let action = match request {
                Request::State => None,
                Request::Quit => Some(Action::Quit),
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
                Request::MovePane { tab, .. } if tab >= self.model.bar_tabs() => {
                    failure = Some(format!("no tab at place {tab} on the tab bar"));
                    None
                }
                Request::MovePane { pane, tab } => Some(Action::MovePane { pane, tab }),
                Request::Equalize => Some(Action::Equalize),
                Request::DropPane { source, target, .. } if source == target => {
                    failure = Some("a pane cannot be dropped on itself".into());
                    None
                }
                Request::DropPane { source, target, .. }
                    if self.model.pane(source).is_none() || self.model.pane(target).is_none() =>
                {
                    failure = Some(format!("no pane {source} or no pane {target}"));
                    None
                }
                Request::DropPane { source, target, zone } => Some(Action::DropPane { source, target, zone }),
                Request::Fit { pane } => {
                    failure = self.model.fit_refusal(pane);
                    failure.is_none().then_some(Action::Fit { pane })
                }
                Request::Enter { pane } => Some(Action::Enter { pane }),
                Request::Leave => Some(Action::Leave),
                Request::Section { section } => Some(Action::ToggleSection { section }),
                // They would open over the labels: the core drops them, so say why.
                Request::Picker { .. } | Request::AxOverlay { .. } if self.model.sneak.is_some() => {
                    failure = Some(sneak::ON.into());
                    None
                }
                // These dispatch by themselves: the core's refusal would be a silent ok.
                Request::Picker { .. } | Request::AxOverlay { .. } | Request::Compose { .. }
                    if self.model.confirm.is_some() =>
                {
                    failure = Some(CONFIRM_UP.into());
                    None
                }
                // Nor while a mark waits for the user's letter or name.
                Request::Picker { .. } | Request::AxOverlay { .. } | Request::Compose { .. }
                    if self.model.mark_input.is_some() =>
                {
                    failure = Some(ottospace_core::marks::INPUT_ON.into());
                    None
                }
                Request::Picker { axis, new_tab } => {
                    self.open_picker(axis, new_tab, window, cx);
                    None
                }
                Request::NewTab => Some(Action::NewTab),
                // Home has no tab on the bar: `select_space` 0 goes there.
                Request::SelectTab { index } if index >= self.model.bar_tabs() => {
                    failure = Some(format!("no tab at place {index} on the tab bar"));
                    None
                }
                Request::SelectTab { index } => Some(Action::SelectTab { index }),
                Request::CycleTab { delta } => Some(Action::CycleTab { delta }),
                Request::SelectSpace { index } => Some(Action::SpaceSelect { index }),
                Request::HideSpace { index } => Some(Action::SpaceHide { index }),
                Request::OpenSpace { space } => Some(Action::SpaceOpen { space }),
                Request::CloseTab { index } => Some(Action::CloseTab { index }),
                Request::PickerQuery { query } => Some(Action::PickerQuery { query }),
                Request::PickerMove { delta } => Some(Action::PickerMove { delta }),
                Request::PickerChoose { .. } if self.model.picker.is_none() => {
                    failure = Some("the picker is not open".into());
                    None
                }
                Request::PickerChoose { index } => Some(Action::PickerChoose { index }),
                Request::PickerClose => Some(Action::ClosePicker),
                Request::Keys => Some(Action::ToggleKeys),
                Request::Key { .. } => None,                          // answered by `press`
                Request::Mouse { .. } | Request::Drag { .. } => None, // answered by `inject`
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
                                self.dispatch(Action::LeaveVisual, Origin::Socket, window, cx);
                            }
                            self.terminal_mouse_down(id, &event, window, cx);
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
                Request::ComposeDrop { .. } if self.model.compose.is_none() => {
                    failure = Some("the new-task box is not open".into());
                    None
                }
                Request::ComposeDrop { paths } => {
                    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
                    self.compose_attach(&paths, Origin::Socket, window, cx);
                    None
                }
                // A draft in the bar has the line: this text has no box to go to.
                Request::Compose { .. } if self.model.capture.is_some() => {
                    failure = Some(ottospace_core::capture_model::DRAFT_OPEN.into());
                    None
                }
                // The drawer the user collapsed: only they open it.
                Request::Compose { .. }
                    if self.model.refusal(&Action::OpenCompose { text: String::new() }, Origin::Socket)
                        == Some(ottospace_core::model::rail::DRAWER_USERS) =>
                {
                    failure = Some(ottospace_core::model::rail::DRAWER_USERS.into());
                    None
                }
                Request::Compose { text } => {
                    self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
                    self.tell_compose_context(cx);
                    let was_open = self.model.compose.is_some();
                    self.dispatch(Action::OpenCompose { text }, Origin::Socket, window, cx);
                    // A draft that did not reach the user must not look delivered.
                    if was_open || self.model.compose.is_none() {
                        failure = Some("the box did not open: it is open already, or the picker, the key reference, visual mode or sneak mode is in the way".into());
                    }
                    None
                }
                Request::TaskNew { line, .. } if line.chars().count() > MAX_TITLE => {
                    failure = Some(format!("the line is longer than {MAX_TITLE} characters"));
                    None
                }
                Request::TaskNew { line, open } => {
                    self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
                    self.file_when_read(Filing::Line { open }, line, reply);
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
                    if self.model.tabs.iter().any(|t| t.id == tab && t.space.task().is_some()) =>
                {
                    failure =
                        Some(format!("tab {tab} belongs to another task; leave out `tab` to file a task without one"));
                    None
                }
                Request::TaskAdd { title, priority, state, tab, agent, paths, skills } => {
                    let assign = Assign { agent, paths, skills, resolved: Vec::new() };
                    if assign.agent.is_none() && assign.skills.is_empty() {
                        // No name to check: nothing to read.
                        self.dispatch(
                            Action::TaskAdd { title, priority, state, tab, assign },
                            Origin::Socket,
                            window,
                            cx,
                        );
                        let task = serde_json::json!(self.model.tasks.list.last());
                        let _ = reply.send(serde_json::json!({ "ok": true, "task": task }).to_string());
                    } else {
                        self.file_when_read(Filing::Task { title, priority, state, tab, assign }, String::new(), reply);
                    }
                    continue;
                }
                Request::TaskUndo => Some(Action::TaskUndo),
                Request::TaskGroup { heading } => Some(Action::ToggleGroup { heading }),
                Request::SessionAdopt { runner, id } => Some(Action::SessionAdopt { runner, id }),
                Request::SessionHide { runner, id } => Some(Action::SessionHide { runner, id }),
                Request::SessionClick { runner, id } => Some(Action::SessionClick { runner, id }),
                Request::SessionsRange { range } => Some(Action::SessionsRange { range }),
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
                    self.dispatch(Action::Tick { now: now_ms() }, Origin::Socket, window, cx);
                    let _ = reply.send(serde_json::json!({ "ok": true, "inbox": self.inbox() }).to_string());
                    continue;
                }
                Request::TaskPop => {
                    self.dispatch(Action::Tick { now: now_ms() }, Origin::Socket, window, cx);
                    let next = self.model.tasks.next().map(|task| task.id);
                    let task = serde_json::json!(self.model.tasks.next());
                    self.dispatch(Action::TaskGo { id: None }, Origin::Socket, window, cx);
                    // False: it has no tab, or the picker or the key reference is in the way.
                    let opened = next.is_some() && self.model.tab_task() == next;
                    let _ = reply.send(serde_json::json!({ "ok": true, "task": task, "opened": opened }).to_string());
                    continue;
                }
                Request::TaskOpen { id } if self.model.tasks.get(id).is_none() => {
                    failure = Some(format!("no task {id}"));
                    None
                }
                Request::TaskOpen { id } => Some(Action::TaskGo { id: Some(id) }),
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
                // Judged before the core: one line, absolute, and on disk a regular file (the
                // viewer reads it again itself, and shows what it finds).
                Request::OpenFile { beside: Some(pane), .. } if self.model.pane(pane).is_none() => {
                    failure = Some(format!("no pane {pane}"));
                    None
                }
                Request::OpenFile { line: None, last: Some(_), .. } => {
                    failure = Some("open_file: `last` needs a `line`".into());
                    None
                }
                Request::OpenFile { line: Some(0), .. } | Request::OpenFile { last: Some(0), .. } => {
                    failure = Some("open_file: lines count from 1".into());
                    None
                }
                Request::OpenFile { path, beside, line, last } => match viewer::open_refusal(&path) {
                    Some(why) => {
                        failure = Some(why);
                        None
                    }
                    None => {
                        let at = line.and_then(|line| ottospace_core::viewer::Lines::new(line, last.unwrap_or(line)));
                        Some(Action::OpenFile { path, axis: Axis::Right, beside, reuse: false, at })
                    }
                },
                // A test run only (refused above otherwise): the folder the user's picker would give.
                Request::RepoAdd { path } => {
                    // All of it the user's, what it says too.
                    self.with_origin(Origin::User, |this| this.add_repo(Path::new(&path), Origin::User, window, cx));
                    None
                }
                // A test run only (refused above otherwise): the user's capture key, pressed by the test.
                Request::CaptureStart { pane } => {
                    failure =
                        self.with_origin(Origin::User, |this| this.capture_pane(pane, Origin::User, window, cx)).err();
                    None
                }
                // A test run only (refused above otherwise): the user's bug key, pressed by the test.
                Request::BugStart => {
                    failure = self.with_origin(Origin::User, |this| this.bug_start(Origin::User, window, cx)).err();
                    None
                }
                Request::InspectorFixture { pane, app, tree } => match pane.or(self.model.focused) {
                    Some(pane) => {
                        self.dispatch(Action::InspectorOpen { pane, app }, Origin::User, window, cx);
                        // As the shell's own read would land.
                        let frame = ottospace_core::ax::Frame { x: 0., y: 0., w: 4000., h: 4000. };
                        let read = Action::InspectorRead { pane, tree, window: frame, millis: 0. };
                        self.dispatch(read, Origin::System, window, cx);
                        None
                    }
                    None => {
                        failure = Some("no pane".into());
                        None
                    }
                },
                Request::TaskBrief { id, tab } => {
                    let caller = self.caller(pass.as_ref());
                    let _ = reply.send(self.serve_brief(id, caller, tab));
                    continue;
                }
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
                // Only to the task the user let see the pane (agent mode): its pass says who asks.
                Request::AppRead { pane, depth, nodes } => {
                    self.serve_app(pane, AppUse::Read, pass, (depth, nodes), reply, window, cx);
                    continue;
                }
                Request::AxTree { pane, depth, nodes } => {
                    // The canary's seam is a test run's alone: outside one, `ax_tree` is gated.
                    let canary = self.host_only.filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some());
                    if canary.is_none() && !self.on(Feature::AgentMode) {
                        let _ = reply.send(control::error(Feature::AgentMode.off()));
                        continue;
                    }
                    let Some(pane) = pane.or(self.model.focused) else {
                        let _ = reply.send(control::error("not an app pane"));
                        continue;
                    };
                    // An app-window test run hosts the canary alone, and has no user to grant
                    // a level: its tree is read as before (`OTTOSPACE_HOST_ONLY`, `e2e_apps.rs`).
                    match canary {
                        Some(_) => self.serve_canary_tree(pane, (depth, nodes), reply, cx),
                        None => self.serve_app(pane, AppUse::Read, pass, (depth, nodes), reply, window, cx),
                    }
                    continue;
                }
                Request::AppLook { pane } => {
                    self.serve_app(pane, AppUse::Look, pass, (None, None), reply, window, cx);
                    continue;
                }
                Request::AxOverlay { refresh } => {
                    failure = self.toggle_ax_overlay(refresh, window, cx).err();
                    None
                }
                Request::Sneak => {
                    self.serve_sneak(reply, window, cx);
                    continue;
                }
                Request::SneakAppTree { window: frame, crop, content, tree } => {
                    let read = ottospace_core::sneak_apps::AppRead { window: frame, crop, content, root: tree };
                    failure = self.serve_sneak_app_tree(read, window, cx).err();
                    None
                }
                Request::Marks => {
                    let _ = reply.send(serde_json::json!({ "ok": true, "marks": self.marks_json() }).to_string());
                    continue;
                }
                Request::Calendar { day, days } => {
                    let _ = reply.send(self.calendar_reply(day.as_deref(), days));
                    continue;
                }
                Request::CalendarCall { in_call } => Some(Action::CallSeen { in_call, at: now_ms() }),
                // As the shell would report it: the shell's news, and its line on the bar.
                // An app pane with no window, as the user's pick or a client's.
                Request::AppFixture { name, user } => {
                    let id = 900_000 + self.model.panes.len() as u32;
                    let kind = Kind::App { window: id, title: name.clone(), bundle: None, name };
                    let origin = if user { Origin::User } else { Origin::Socket };
                    self.dispatch(Action::Open { kind, axis: Axis::Right, beside: None }, origin, window, cx);
                    None
                }
                Request::WindowFixture { pane, title, identifier, subrole, size, modal } => {
                    let new = pop_ups::Fixture { pane, title, identifier, subrole, size, modal };
                    failure = self.window_fixture(new, window, cx).err();
                    None
                }
                Request::SlotWindows { look } => {
                    self.slot_windows_fixture(look, window, cx);
                    None
                }
                // The dialog's Keep, as the user's act.
                Request::SlotMake { pane, slot } => {
                    self.dispatch(Action::SlotMake { pane, pid: 0, slot }, Origin::User, window, cx);
                    None
                }
                Request::WindowFixtureClose { window: id } => {
                    match self.offer_fixtures.remove(&id) {
                        Some(_) => drop(self.dispatch(Action::OfferGone { window: id }, Origin::System, window, cx)),
                        None => failure = Some(format!("no fixture window {id}")),
                    }
                    None
                }
                Request::ListenEnd { why } => {
                    match self.model.listen.on.as_ref().map(|on| on.id.clone()) {
                        Some(id) => {
                            self.dispatch(Action::ListenEnded { id, why, at: now_ms() }, Origin::System, window, cx);
                            self.broke(why.said().into());
                        }
                        None => failure = Some("no transcript is on".into()),
                    }
                    None
                }
                Request::RecordStart { dir } => {
                    let size = to_cg(window.bounds().size);
                    let id = ns_view(window).map(|view| unsafe { ottospace_mac::window::number(view) });
                    let started = match (&self.recording, id) {
                        (Some(_), _) => Err("already recording".to_string()),
                        (None, Some(id)) if id > 0 => record::start(id as u32, size, dir.into()),
                        (None, _) => Err("the window has no number".to_string()),
                    };
                    let answer = match started {
                        Ok(recording) => {
                            self.recording = Some(recording);
                            serde_json::json!({ "ok": true }).to_string()
                        }
                        Err(why) => control::error(&why),
                    };
                    let _ = reply.send(answer);
                    continue;
                }
                Request::RecordStop => {
                    let answer = match self.recording.take() {
                        Some(recording) => serde_json::json!({ "ok": true, "frames": recording.stop() }).to_string(),
                        None => control::error("not recording"),
                    };
                    let _ = reply.send(answer);
                    continue;
                }
                Request::AudioProbeStart => {
                    let started = match self.audio_probe {
                        Some(_) => Err("already hearing".to_string()),
                        None => listen_live::Probe::start(),
                    };
                    let answer = match started {
                        Ok(probe) => {
                            self.audio_probe = Some(probe);
                            serde_json::json!({ "ok": true }).to_string()
                        }
                        Err(why) => control::error(&why),
                    };
                    let _ = reply.send(answer);
                    continue;
                }
                Request::AudioProbeStop => {
                    let answer = match self.audio_probe.take() {
                        Some(probe) => {
                            let (buffers, failed) = probe.stop();
                            serde_json::json!({ "ok": true, "buffers": buffers, "failed": failed }).to_string()
                        }
                        None => control::error("not hearing"),
                    };
                    let _ = reply.send(answer);
                    continue;
                }
                // A test run only: the window narrower or wider. Later, as `fill_display` sizes it:
                // GPUI drops a resize AppKit reports while the app is borrowed.
                Request::WindowWidth { width } => {
                    if let Some(view) = ns_view(window).map(|view| view as usize) {
                        cx.spawn(async move |_, _| {
                            let view = view as *mut std::ffi::c_void;
                            // SAFETY: the window's own view, alive while its workspace is; the main thread.
                            if let Some(at) = unsafe { ottospace_mac::window::placement(view) } {
                                let frame = ottospace_core::ax::Frame { w: width.clamp(300., 4_000.), ..at.window };
                                unsafe { ottospace_mac::window::set_frame(view, frame) };
                            }
                        })
                        .detach();
                    }
                    None
                }
                // A test run only: the green button. The next tick takes the zoom it asks for.
                Request::ZoomPress => {
                    match ns_view(window).filter(|_| self.zoom_asks) {
                        // SAFETY: GPUI's own view, on the main thread.
                        Some(view) => {
                            if !unsafe { ottospace_mac::window::press_zoom(view) } {
                                failure = Some("the zoom was not turned into a fill".into());
                            }
                        }
                        // It would zoom the test's window over the user's screen.
                        None => failure = Some("the window's zoom does not fill the display".into()),
                    }
                    None
                }
                // A test run only: made-up displays, from now on in place of the real ones.
                Request::DisplaysSet { displays, main } => {
                    let displays: Vec<_> = (displays.into_iter())
                        .map(|d| ottospace_core::model::displays::Display {
                            uid: d.uid,
                            frame: ottospace_core::ax::Frame::new(d.frame[0], d.frame[1], d.frame[2], d.frame[3]),
                            scale: d.scale,
                        })
                        .collect();
                    let main = main.or_else(|| displays.first().map(|d| d.uid.clone()));
                    self.displays_made_up = true;
                    self.dispatch(Action::SetDisplays { displays, main }, Origin::System, window, cx);
                    None
                }
                // A test run only: as macOS says it; the next tick kicks GPUI to draw again.
                Request::DisplayChange => {
                    display::note_change();
                    None
                }
                Request::UiGallery { on } => {
                    self.gallery = on;
                    cx.notify();
                    None
                }
                Request::Trace => {
                    let _ = reply.send(serde_json::json!({ "ok": true, "trace": trace::jsonl() }).to_string());
                    continue;
                }
                Request::MarkAdd { pane: Some(_), tab: Some(_), .. } => {
                    failure = Some("name a pane or a tab, not both".into());
                    None
                }
                Request::MarkAdd { key: Some(key), .. } if letter(key).is_none() => {
                    failure = Some("a mark's key is one letter, a to z".into());
                    None
                }
                Request::MarkAdd { pane, tab, name, key } => {
                    let of = tab.map_or(MarkOf::Pane(pane), |tab| MarkOf::Tab(Some(tab)));
                    let marked = self.model.mark_of(of).is_some();
                    if let Some(why) = self.model.mark_refusal(of).filter(|_| !marked) {
                        failure = Some(why);
                    } else {
                        self.add_mark(of, (name, key), window, cx);
                        // The mark, so the caller has its name.
                        let id = serde_json::json!(self.model.mark_of(of));
                        let mark = self.marks_json().into_iter().find(|row| row["id"] == id);
                        let _ = reply.send(serde_json::json!({ "ok": true, "mark": mark }).to_string());
                        continue;
                    }
                    None
                }
                Request::MarkGo { name, key, axis } => {
                    let found = self.mark_ref(name, key);
                    failure = found.and_then(|mark| self.go_to_mark(mark, axis, window, cx)).err();
                    None
                }
                Request::MarkRemove { name, key } => match self.mark_ref(name, key) {
                    Ok(mark) => Some(Action::MarkRemove { mark }),
                    Err(why) => {
                        failure = Some(why);
                        None
                    }
                },
                Request::MarkRename { to, .. } if to.trim().is_empty() => {
                    failure = Some("the new name is empty".into());
                    None
                }
                Request::MarkRename { name, key, to } => match self.mark_ref(name, key) {
                    Ok(mark) => Some(Action::MarkRename { mark, name: to }),
                    Err(why) => {
                        failure = Some(why);
                        None
                    }
                },
            };
            // What the core would drop because a client sent it is an error, not a silent ok.
            let refused = action.as_ref().and_then(|action| self.model.refusal(action, Origin::Socket));
            if let Some(why) = refused {
                failure = Some(why.into());
            } else if let Some(action) = action {
                let effects = self.dispatch(action, Origin::Socket, window, cx);
                // A terminal pane with no shell behind it: say so instead of replying ok.
                let dead = effects.iter().any(|e| match e {
                    Effect::Opened(id) => {
                        self.model.pane(*id).is_some_and(|p| p.kind == Kind::Terminal)
                            && !self.terminals.contains_key(id)
                    }
                    _ => false,
                });
                // A picked app that could not be hosted: `open_app` left the reason in `error`.
                let picked = |e: &Effect| {
                    matches!(e, Effect::HostApp { .. } | Effect::HostPicked { .. } | Effect::LaunchApp { .. })
                };
                let unhosted = effects.iter().any(picked) && self.error.is_some();
                if dead || unhosted {
                    failure = self.error.clone().or(Some("the shell did not start".into()));
                }
            }
            match failure {
                Some(message) => drop(reply.send(control::error(&message))),
                None => self.answer(reply, window, cx),
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

    /// The Repos section as laid out: each repository's `name` and `path` with its `header`,
    /// whether it is drawn `folded`, and its `rows` (a checkout's `path`, `branch`, `default`,
    /// `label`, its dot's `activity` and whether a commit's pulse plays) each with its `row`. Rects
    /// are in window points, scrolled out of view or not; null while the section is not drawn (a
    /// folded repository's rows too). A test run's `name_cut`.
    fn repos_bar(&self) -> Vec<serde_json::Value> {
        let marks = self.marks.borrow();
        let drawn = |mark: Mark| marks.get(&mark).copied().map(rect);
        let (mut index, now) = (0, now_ms());
        let group = |(n, repo): (usize, &Repo)| {
            let row = |checkout: Checkout| {
                let dot = self.model.checkout_dot(&checkout.path, now);
                // Its pull request by numbers and kinds: no title, name or link.
                let pr = self.model.checkout_pr(&checkout.path).map(ottospace_core::prs::Pr::summary);
                let mut row = serde_json::json!(checkout);
                row["pr"] = serde_json::json!(pr);
                let drawn = drawn(Mark::Checkout(index));
                // A folded row draws no dot: no pulse plays on it.
                let pulsing = dot.pulse.is_some() && drawn.is_some();
                (row["activity"], row["pulsing"]) = (serde_json::json!(dot.activity), pulsing.into());
                row["row"] = serde_json::json!(drawn);
                index += 1;
                row
            };
            let rows: Vec<serde_json::Value> = repo.checkouts().into_iter().map(row).collect();
            let at = marks.get(&Mark::Repo(n)).copied();
            let (header, folded) = (at.map(rect), self.model.repo_folded(repo));
            // Test runs only: its name's box and shaped width (`cut_text`).
            let name_cut = cut_drawn(&repo.name, at);
            // The group's folder it is drawn under, if any.
            let group = self.model.repo_dirs.group_of(&repo.path).map(|dir| &dir.path);
            serde_json::json!({
                "name": repo.name, "path": repo.path, "group": group, "header": header, "folded": folded, "rows": rows,
                "name_cut": name_cut,
            })
        };
        self.model.repos.iter().enumerate().map(group).collect()
    }

    /// The Sessions group as drawn, and how often sessions were read: `reads` stays 0 while
    /// reading is off (`"sessions": false`) or the Tasks section was never open.
    fn sessions_bar(&self) -> serde_json::Value {
        let mut bar = serde_json::json!(self.model.sessions_bar());
        bar["reads"] = serde_json::json!(self.sessions.as_ref().map_or(0, sessions::Poller::reads));
        bar["enabled"] = serde_json::json!(self.sessions.is_some());
        // Rounds that looked only at adopted sessions' files, with the section closed.
        bar["slow_reads"] = serde_json::json!(self.sessions.as_ref().map_or(0, sessions::Poller::slow_reads));
        bar
    }

    /// Asks for the sessions to be read again when it is time, and takes in what was found.
    /// The reading itself is on the poller's thread.
    fn poll_sessions(&mut self, window: &Window, cx: &mut Context<Self>) {
        let now = now_ms();
        // Read for the Repos section's dots too; listed only while Tasks is open.
        let tasks = self.model.sidebar == Some(Section::Tasks);
        let open = tasks || self.model.sidebar == Some(Section::Repos);
        if tasks != self.sessions_tasks && open {
            // Tasks closed into Repos: its list goes at once. Either way, read again now.
            self.sessions_tasks = tasks;
            if !tasks && self.sessions_listed {
                self.dispatch(
                    Action::SetSessions { found: Vec::new(), day: day_start(now) },
                    Origin::System,
                    window,
                    cx,
                );
            }
            if let Some(poller) = &mut self.sessions {
                poller.again();
            }
        }
        if !self.sessions_quiet_told {
            self.sessions_quiet_told = true;
            let live = config::live();
            self.dispatch(Action::SessionQuiet { ms: live.quiet_ms() }, Origin::System, window, cx);
            // Told with it, once: the models (read only at start) and the default agent a task starts with.
            let models = ottospace_core::agents::Models::with(config::get().settings.agent_models.as_ref());
            let agents = Action::SetAgents {
                claude: models.claude,
                codex: models.codex,
                default: live.default_agent.clone(),
                shows: live.conversation_view.unwrap_or(ottospace_core::settings::DEFAULT_VIEW),
            };
            self.dispatch(agents, Origin::System, window, cx);
        }
        // Closed, only the sessions that tasks still follow are looked at, and only their files.
        let Some(poller) = &mut self.sessions else { return };
        let model = &self.model;
        poller.ask(open, || model.followed_sessions(), model.worked_sessions(), now, day_start(now));
        let (read, stopped) = (poller.take(), poller.stopped());
        let (mut found, mut seen, mut worked) = (None, None, Vec::new());
        for read in read {
            match read {
                sessions::Read::Found(list) => found = Some(list),
                sessions::Read::Seen(list) => seen = Some(list),
                sessions::Read::Work(list) => worked.extend(list),
            }
        }
        for (work, cwd) in worked {
            self.dispatch(Action::SessionWork { work, cwd }, Origin::System, window, cx);
            cx.notify();
        }
        if let Some(seen) = seen {
            // Adopted tasks follow their sessions by this, with the section closed.
            self.dispatch(Action::Tick { now }, Origin::System, window, cx);
            self.dispatch(Action::SessionsSeen { seen }, Origin::System, window, cx);
            cx.notify();
        }
        // What was found is shown, and acted on, only while it is fresh: it is read while the
        // section is open, so when the section closes (or the reader is gone) the list is
        // emptied. Nothing then reads or adopts a session by what it was doing a while ago.
        let found = if open && !stopped { found } else { self.sessions_listed.then(Vec::new) };
        if let Some(found) = found {
            self.sessions_listed = open && !stopped;
            // The time too: ages and the range are counted from it.
            self.dispatch(Action::Tick { now }, Origin::System, window, cx);
            let read = match tasks || found.is_empty() {
                true => Action::SetSessions { found, day: day_start(now) },
                false => Action::SessionsAtWork { found },
            };
            self.dispatch(read, Origin::System, window, cx);
            cx.notify();
        }
        if stopped && !self.sessions_stopped {
            self.sessions_stopped = true;
            self.error =
                Some("sessions: the reader stopped; sessions are not listed until ottospace starts again".into());
            cx.notify();
        }
    }

    fn tasks_bar(&self) -> serde_json::Value {
        let sections = self.model.task_sections(now_ms());
        let undo = self.task_offer().map(|offer| offer.moved.title());
        serde_json::json!({ "empty": sections.is_empty(), "undo": undo, "sections": sections })
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
                    trace::record("timeout", serde_json::json!({ "what": "wait", "pane": id }));
                    break control::error(&format!("no match for the pattern in {} ms", limit.as_millis()));
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            };
            let _ = reply.send(message);
        })
        .detach();
    }

    /// Why a client's key is not pressed: under the user's labels it types a label or leaves,
    /// and nothing else (its ⌘ shortcut would end the mode, and the user's next letter reach a
    /// terminal); under the quit question it answers (↵, Q, Esc), and nothing else.
    fn key_refusal(&self, key: &Keystroke) -> Option<&'static str> {
        if self.model.sneak.is_some() && !sneak::takes(key) {
            return Some(sneak::ON);
        }
        // A client's letter of a label in another app: only the user presses there.
        if let Some(SneakKey::Type(letter)) = sneak_key(&key.key).filter(|_| self.model.sneak.is_some()) {
            let typed = Action::SneakType { text: letter.to_string() };
            if let Some(why) = self.model.refusal(&typed, Origin::Socket) {
                return Some(why);
            }
        }
        if let Some(confirm) = &self.model.confirm {
            let why = if confirm.paste.is_some() {
                clip::USERS_ALONE
            } else if confirm.delete.is_some() {
                ottospace_core::repos::DELETE_USERS
            } else {
                CONFIRM_UP
            };
            return self.confirm_answer(key, Origin::Socket).is_none().then_some(why);
        }
        // ⌘V into a terminal: the user's clipboard, theirs alone to paste (`Action::Paste`).
        let cmd = key.modifiers == Modifiers::command();
        if let Some(pane) =
            self.model.keyboard_terminal().filter(|_| cmd && key.key == "v" && self.model.field().is_none())
        {
            let paste = Action::Paste { pane, amount: clip::Amount::default() };
            if let Some(why) = self.model.refusal(&paste, Origin::Socket) {
                return Some(why);
            }
        }
        // The palette types, moves and runs for the user alone.
        self.model.palette.is_some().then_some(ottospace_core::commands::USERS_ALONE)
    }

    /// Presses `keystrokes` as if typed, once this update is done, then replies with the state.
    /// `origin`: the client, or in a test run the user.
    fn press(
        &self,
        keystrokes: Vec<Keystroke>,
        origin: Origin,
        reply: Sender<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let labelled = |cx: &App| this.read_with(cx, |this, _| this.model.sneak.is_some()).unwrap_or(false);
            let client = origin == Origin::Socket;
            // The key handlers run as for a real key, outside this call, but the client presses.
            let before = this.update(cx, |this, _| this.swap_origin(origin));
            let mut refused = None;
            for keystroke in keystrokes {
                let was = labelled(cx);
                if client {
                    refused = this.read_with(cx, |this, _| this.key_refusal(&keystroke)).unwrap_or(None);
                }
                if refused.is_some() {
                    break;
                }
                // A text field takes typed text through the input handler its last frame set:
                // draw one first, as between real keys, so a field the key before opened is there.
                if this.read_with(cx, |this, _| this.model.field().is_some()).unwrap_or(false) {
                    window.draw(cx).clear();
                }
                window.dispatch_keystroke(keystroke, cx);
                // The keys after the one that ended sneak mode were for labels, not a terminal.
                if client && was && !labelled(cx) {
                    break;
                }
            }
            // Gone: the reply is dropped with the closure, and the client is told the app is closing.
            let _ = this.update(cx, |this, cx| {
                this.swap_origin(before.unwrap_or(Origin::User));
                match refused {
                    Some(why) => drop(reply.send(control::error(why))),
                    None => this.answer(reply, window, cx),
                }
            });
        });
    }

    /// For tests: each of `keystrokes` as a key held down, repeating, then replies with the
    /// state. A key that types goes through the window's own `keyDown:`, never posted, as
    /// `inject`. A ⌘ shortcut reaches GPUI through the input method, which tells it of no repeat
    /// and takes no event the system did not queue: it is pressed as `press` does, while
    /// `key_repeating` says it is held.
    fn hold(
        &self,
        keystrokes: Vec<Keystroke>,
        origin: Origin,
        reply: Sender<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = ns_view(window) else {
            return drop(reply.send(control::error("no window to send the key to")));
        };
        cx.spawn_in(window, async move |this, cx| {
            let before = this.update(cx, |this, _| this.swap_origin(origin));
            let mut refused = None;
            for key in keystrokes {
                let was = this.read_with(cx, |this, _| this.model.sneak.is_some()).unwrap_or(false);
                if origin == Origin::Socket {
                    refused = this.read_with(cx, |this, _| this.key_refusal(&key).map(String::from)).unwrap_or(None);
                }
                // A frame first, as between a press and its repeat: the keys go to what it drew.
                if refused.is_some() || cx.update(|window, cx| window.draw(cx).clear()).is_err() {
                    break;
                }
                let m = key.modifiers;
                let sent = if m.platform {
                    let pressed = |window: &mut Window, cx: &mut App| {
                        ottospace_mac::window::while_held(|| window.dispatch_keystroke(key.clone(), cx));
                    };
                    cx.update(pressed).is_ok()
                } else {
                    unsafe { ottospace_mac::window::send_held_key(view, &key.key, m.control, m.alt, m.shift) }
                };
                if !sent {
                    refused = Some(format!("no key to hold down: {}", key.key));
                    break;
                }
                // As `press`: a client's keys after the one that ended sneak mode were for labels.
                let labelled = this.read_with(cx, |this, _| this.model.sneak.is_some()).unwrap_or(false);
                if origin == Origin::Socket && was && !labelled {
                    break;
                }
            }
            let _ = this.update(cx, |this, _| drop(this.swap_origin(before.unwrap_or(Origin::User))));
            match refused {
                Some(why) => drop(reply.send(control::error(&why))),
                None => drop(this.update_in(cx, |this, window, cx| this.answer(reply, window, cx))),
            }
        })
        .detach();
    }

    /// `mouse` as the press, release or move `action` asks for. A press holds the button until
    /// the release, and a move in between is a drag.
    fn pointer(&mut self, action: MouseAction, mouse: Mouse) -> Mouse {
        let kind = match action {
            MouseAction::Down => MouseKind::Down,
            MouseAction::Move if self.held => MouseKind::Drag,
            MouseAction::Move => MouseKind::Move,
            MouseAction::Up => MouseKind::Up,
            // The wheel, and the pointer leaving or coming back: as the caller made it.
            MouseAction::Wheel | MouseAction::Exit | MouseAction::Enter => mouse.kind,
        };
        self.held = match action {
            MouseAction::Down => true,
            MouseAction::Up => false,
            _ => self.held,
        };
        Mouse { kind, ..mouse }
    }

    /// A client that pressed and went away must not leave the button down: after `POINTER_IDLE`
    /// with no `mouse` request, releases it outside the window, where nothing is dropped.
    fn release_later(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(POINTER_IDLE).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !std::mem::take(&mut this.held) {
                    return;
                }
                let (kind, clicks, off) = (MouseKind::Up, 1, false);
                let up = Mouse { kind, x: -1.0, y: -1.0, clicks, cmd: off, ctrl: off, alt: off, shift: off };
                this.inject(vec![up], Origin::Socket, std::sync::mpsc::channel().0, window, cx);
            });
        })
    }

    /// Hands pointer events to the window's own AppKit handlers, as the system does, then replies
    /// with the state. (GPUI's `Window::dispatch_event` cannot be called from here: its return
    /// type is private.) A frame is drawn before each, as between real events: GPUI finds what
    /// is under the pointer in the last frame drawn. The events are never posted: no other app's
    /// window gets them, and the real pointer stays where it is.
    fn inject(
        &self,
        events: Vec<Mouse>,
        origin: Origin,
        reply: Sender<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = ns_view(window) else {
            return drop(reply.send(control::error("no window to send the pointer to")));
        };
        // A task of its own: the handlers update the app, which this update holds.
        cx.spawn_in(window, async move |this, cx| {
            // The handlers run outside any call of ours: `origin` is the sender until the last one.
            let before = this.update(cx, |this, _| {
                this.injected = true;
                this.swap_origin(origin)
            });
            let mut made = true;
            for event in events {
                // An error: the window is gone, and `view` with it.
                if cx.update(|window, cx| window.draw(cx).clear()).is_err() {
                    break;
                }
                made = unsafe { ottospace_mac::window::send_mouse(view, event) };
                if !made {
                    break;
                }
            }
            let _ = cx.update(|window, cx| window.draw(cx).clear());
            let _ = this.update(cx, |this, _| {
                this.injected = false;
                this.swap_origin(before.unwrap_or(Origin::User));
            });
            match made {
                true => drop(this.update_in(cx, |this, window, cx| this.answer(reply, window, cx))),
                false => drop(reply.send(control::error("the pointer event could not be made"))),
            }
        })
        .detach();
    }

    /// State as JSON: the model, plus each pane's drawn bounds `[x, y, w, h]` in window points.
    fn snapshot(&self, window: &Window, cx: &App) -> String {
        self.state(window, cx).to_string()
    }

    fn state(&self, window: &Window, cx: &App) -> serde_json::Value {
        let parking = display::parking_spot(window.scale_factor() as f64);
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
        // Only panes of the tabs on show are drawn; a background pane's last bounds are stale.
        let drawn = self.model.shown_panes();
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
        let marks = self.marks.borrow();
        let marked = |key: fn(PaneId) -> Mark| -> BTreeMap<PaneId, [f32; 4]> {
            drawn.iter().filter_map(|id| Some((*id, rect(*marks.get(&key(*id))?)))).collect()
        };
        // By place in `state.tabs`: null for a tab not drawn (one row shows one space's tabs).
        let tabs: Vec<Option<[f32; 4]>> =
            (0..self.model.bar_tabs()).map(|place| marks.get(&Mark::Tab(place)).copied().map(rect)).collect();
        let spaces: Vec<Option<[f32; 4]>> = (0..self.model.shown_spaces().len())
            .map(|place| marks.get(&Mark::Space(place)).copied().map(rect))
            .collect();
        let previews = self.hosted.iter().map(|(id, h)| (*id, h.preview.clone())).collect();
        let shown = drawn_rects(&previews);
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
                    "selection": t.selected_shown(),
                    // Its first and last cell as [line, column], line 0 the top of the screen and
                    // negative in history.
                    "selection_range": t.selection_range(),
                    "visual_cursor": t.visual_cursor(),
                });
                (*id, terminal)
            })
            .collect();
        let status = self.notice_replied();
        // Home's own tab, after all the others, is no tab of the bar: `tabs` leaves it out, and
        // `active` is null while Home is on show.
        let mut model = serde_json::to_value(&self.model).unwrap_or_default();
        let bar = self.model.bar_tabs();
        if let Some(tabs) = model.get_mut("tabs").and_then(serde_json::Value::as_array_mut) {
            tabs.truncate(bar);
        }
        if self.model.active >= bar {
            model["active"] = serde_json::Value::Null;
        }
        let mut tab_panes = self.model.tab_panes();
        tab_panes.truncate(bar);
        let unfilled: Vec<serde_json::Value> = self
            .model
            .marks
            .unfilled()
            .iter()
            .map(|empty| {
                serde_json::json!({
                    "pane": empty.pane, "why": empty.why,
                    "open": self.controls.bounds(&format!("unfilled-open:{}", empty.pane)).map(rect),
                })
            })
            .collect();
        let mut state = serde_json::json!({
            "ok": true,
            "mode": self.model.mode(),
            // Where the keyboard is: a pane, the list, a rail's cell, the drawer.
            "focus": self.model.focus_target(),
            "state": model,
            "bounds": bounds,
            // Each drawn pane's bounds inside its border.
            "frames": frames,
            // Each drawn pane's header (the handle a drag moves it by) and the × in it, and each
            // tab on the bar by place.
            "headers": marked(Mark::Header),
            // Each drawn pane's header text (`pane_title`).
            "titles": drawn.iter().filter_map(|id| Some((*id, self.pane_title(self.model.pane(*id)?)))).collect::<BTreeMap<_, _>>(),
            "closes": marked(Mark::Close),
            "tabs": tabs,
            // Each space of the top row, by its place there.
            "space_rects": spaces,
            // The agent chip, or null: the selected space's conversation while it is closed.
            "agent_chip": self.chip_snapshot(),
            // The left column's title strip, and the open section's header (null: none open).
            "strip": marks.get(&Mark::Strip).copied().map(rect),
            "bar_header": marks.get(&Mark::BarHeader).copied().map(rect),
            // The side overlays as drawn (`overlay_panels`), at rest: the sidebar's and the drawer's
            // place over the panes, also while a closed one slides back; null once in, or with the flag off.
            "overlays": {
                "sidebar": marks.get(&Mark::Overlay(true)).copied().map(rect),
                "drawer": marks.get(&Mark::Overlay(false)).copied().map(rect),
            },
            // The Tasks rail cell's needs-you dot (null: not drawn).
            "tasks_dot": marks.get(&Mark::TasksDot).copied().map(rect),
            // While a pane is dragged: the pane a drop would land on and the zone of it, both
            // null over anything else. Null with no drag.
            "drag": cx.has_active_drag().then(|| serde_json::json!({
                "target": self.drop_at.map(|(target, _)| target),
                "zone": self.drop_at.map(|(_, zone)| zone),
            })),
            // Each drawn app pane's preview: the part of the window it shows, where it is drawn.
            "shown": shown,
            "tab_panes": tab_panes,
            // ottospace's own window frame, so a test can turn pane bounds into screen positions.
            "window": frame,
            // The displays, left to right, by place: no uid or name, which would name the user's hardware.
            "displays": self.model.displays().iter().enumerate().map(|(id, d)| serde_json::json!({
                "id": id, "frame": [d.frame.x, d.frame.y, d.frame.w, d.frame.h], "scale": d.scale,
                "main": self.model.main_display() == Some(id),
            })).collect::<Vec<_>>(),
            // Each window: the main one, then each tab out's (`tab`, its display's place, whether
            // it has the keyboard, its frame and frames drawn once open).
            "windows": self.windows_state(),
            // Each tab folded out to another display: its id, and its display's place in `displays`.
            "out": self.model.tabs_out().into_iter().map(|(tab, display)| serde_json::json!({ "tab": tab, "display": display }))
                .collect::<Vec<_>>(),
            // Whether that window has the keyboard: a user's wheel over a parked app pane enters only then.
            "window_active": window.is_window_active(),
            // Where parked windows go, so a test can tell "parked" from "moved somewhere".
            "parking": [parking.x, parking.y],
            // The registered return key, or null if there is none.
            "return_key": RETURN_KEY.get(),
            // Which build this is, short: "0.1.0 · a4a2582". About and the status bar's hover give it in full.
            "version": version_tag(env!("CARGO_PKG_VERSION"), option_env!("OTTOSPACE_COMMIT")),
            // The terminal font in use, and the size of one character cell in points.
            "font": terminal::font_in_use(),
            "cell": [f32::from(self.cell.width), f32::from(self.cell.height)],
            // What the last step said (a client's refusal is here alone, never on the user's bar),
            // else what the toast says, or null. Not `error`: that key means the request failed.
            "notice": status.as_ref().map(Notice::public_text),
            // Its level: `info` (a restart to ask for), `warning` (a move that did nothing),
            // `meeting` (a meeting started: `notice` is "Meeting started") or `error`.
            "notice_level": status.as_ref().map(Notice::level),
            // The toast as drawn: its cards, and how far in while it comes or goes. Null: none.
            "toast": self.toast_snapshot(),
            // Where each draft's row of the quit question is drawn, by its place in `confirm.unsent`.
            "quit_rows": self.quit_rows(),
            // How many problems the last read of `settings.json` found.
            "settings_errors": self.model.settings_errors(),
            // The user's editor on a draft (⌃G): what it edits, never the text. Null: none runs.
            "editor": self.editor_snapshot(),
            // Each hosted window's real frame, in the same global coordinates as `parking`.
            "hosted": hosted,
            // Each hosted pane's title-bar crop in points (0: none).
            "crop": crop,
            "terminals": terminals,
            // Each file viewer as drawn: `body` (`markdown` with its blocks' `shape`, `lines`, `image`,
            // or null), never the file's text. Its path, kind, line and count are `state.viewers`.
            "viewers": self.viewers_snapshot(),
            // A test run's stand-in clipboard: the last text copied. Null outside a test run.
            "copied": self.clipboard.test_text(),
            // The picker's rows as drawn, or null: `state.picker.items` narrowed by its filter,
            // then, once something is typed, the installed apps that are not running (`closed`).
            "picker_rows": self.model.picker.as_ref().map(Picker::matches),
            // The command palette's rows as drawn, or null: each `command`, `name`, `group` and `key`.
            "palette_rows": self.palette_rows(cx),
            // The text field that has the keyboard (the new-task box's line or the picker's
            // filter), or null: its text, cursor and selection.
            "field": self.field_snapshot(),
            // Tasks as the Tasks section lists them, and how many need the user.
            "inbox": self.inbox(),
            "needs_you": self.model.tasks.needs_you(),
            // True once what the tags of the line in the new-task box may name has been read:
            // they are read on another thread, a moment after the line changes.
            "compose_read": self.model.compose_read(),
            // Where the new-task box is: `drawer` (its new-task state), `bar` (the title of the
            // draft being collected), or null while it is closed.
            "compose_host": self.model.compose_host(),
            // What ↵ in the box would do (`agent`, `starts`, `dir`, `branch`), or null; and the
            // drawer's picks: repository names (`wheres`) and agent names (`whos`).
            "compose_plan": self.model.compose_plan(),
            // The plan's line over the box, null when it is the default; the tag hint, while the box is open.
            "compose_plan_line": self.model.compose_plan_line(),
            "compose_hint": self.model.compose_hint(),
            "compose_picks": self.model.compose_picks(),
            // Where the open box's ↵ starts its task (`space`, `name`), and whether that is not the
            // space on show (`elsewhere`); null while it is closed.
            "compose_target": self.model.compose.as_ref().and_then(|_| self.model.compose_space()).map(|of| serde_json::json!({
                "space": of,
                // Null where ↵ makes a new workspace (Home, a session's space).
                "name": self.model.joins().map(|_| self.model.space_title(&of)),
                "elsewhere": self.model.compose_elsewhere(),
                // Where its Switch is drawn, while it is.
                "switch": marks.get(&Mark::ComposeSwitch).copied().map(rect).filter(|_| self.model.compose_elsewhere()),
            })),
            // Nothing is open yet: the drawer shows the new task by itself.
            "welcome": self.model.welcome(),
            // The dropdown that finishes the tag at the cursor, or null: its `mark`, what is
            // `typed`, the `rows` (name, kind, detail, matched letters) and the one `selected`.
            "compose_menu": self.model.dropdown(),
            // Where that dropdown was drawn, or null without one.
            "compose_menu_bounds": self.model.dropdown().map(|_| rect(self.field.borrow().menu)),
            // Where each tag's cap under the box was drawn, by its key (`#`, `>`, `p1`), or null
            // with the box closed. A click on one types its mark.
            "compose_tags": self.model.compose.as_ref().map(|_| {
                let at = |key: &str| self.controls.bounds(&format!("compose:tag:{key}")).map(|b| (key.to_string(), rect(b)));
                ottospace_core::model::TAG_KEYS.iter().filter_map(|(key, _)| at(key)).collect::<BTreeMap<_, _>>()
            }),
            // The folder taken from a path's dropdown that the task's agent will run in, as the
            // line has it (`~/notes/`), or null.
            "compose_folder": self.model.compose_folder(),
            // What the box says it could not do ("no room in the line for that name"), or null.
            "compose_note": self.model.compose_note(),
            // The tab or pane an ⌥-click would reference, outlined while ⌥ is held over it
            // with the box open: its rect, or null.
            "compose_pointing": self.pointing.filter(|_| self.model.compose.is_some()).map(rect),
            // The Tasks section as drawn: each heading with its count and rows (none while it is
            // collapsed). `empty`: no task is listed, so the section shows how to file one.
            // `undo`: the move the line at its top offers to take back ("Dismissed"), while it shows.
            "tasks_bar": self.tasks_bar(),
            // The Project view, the Today footer and each listed task's hover card.
            "tasks_progress": progress::snapshot(self),
            // Sessions that are no task's, as the Tasks section draws them below the tasks:
            // derived facts only (runner, id, repository name, active, station and label, age).
            "sessions_bar": self.sessions_bar(),
            // The Repos section: each repository's name with where its heading was laid out
            // (`header`), and its checkouts (`rows`: `path`, `branch`, `default`, `label`, `row`).
            "repos_bar": self.repos_bar(),
            // The home screen, or null while the tab on show has a pane: each task's node (its
            // lane, depth and crossing of the lattice) and the rows under "Needs you".
            "home": self.model.home(&self.home.borrow().keep_out()),
            // What the home screen drew, in window points: its `area`, the `panel` (the new-task
            // line with its list) and each node's click target by task id. Also how often
            // lattice lines were built and painted, here and in empty panes.
            "home_drawn": self.home.borrow().snapshot(),
            // A test run only: the code city drawn in a task's empty tab (`code_city`): counts, the
            // picture's size in pixels, where it and the lit buildings are, why there is none
            // (`failed`). Never a file's name.
            "city": std::env::var_os("OTTOSPACE_E2E").map(|_| self.city_snapshot(window.scale_factor())),
            // The pinned apps that are not running: `open_app` starts one. The Dock's and the
            // rest are in `apps_bar`: those are the user's to start.
            "closed_apps": self.pinned_closed().map(|app| app.name).collect::<Vec<_>>(),
            // The Apps section below the running apps: `not_running` (each with `name`, `path`,
            // `pinned`, `suggested`), `more` behind "and N more", `all` (every other installed
            // app, while `all_open`; `all_count` of them) narrowed by `filter`, and `undo`.
            // Empty with the section off (`apps_section`).
            "apps_bar": if self.on(Feature::AppsSection) { self.model.apps_bar() } else { Default::default() },
            // The Apps list: what can be hosted now. `icon`: its icon was read.
            "apps": self.apps.iter().map(|app| serde_json::json!({
                "name": app.name, "windows": app.windows, "icon": self.icons.contains_key(&app.name),
            })).collect::<Vec<_>>(),
            "ax_overlay": self.ax_overlay.as_ref().filter(|_| self.model.ax_overlay),
            // The accessibility inspector, or null: its pane, `counts` (nodes, pressable,
            // settable, each role's count), `rows` (depth, role, flags; no text), `selected`, `outline`.
            "inspector": self.inspector_snapshot(),
            // Sneak mode's labels, each with its control's id and frame, and the letters typed; or null.
            "sneak": self.model.sneak,
            // Test runs only: `live`, how often a pane went live; `frame_reads`, how often the
            // layout sync read a hosted window's frame over accessibility.
            "counts": std::env::var_os("OTTOSPACE_E2E").map(|_| serde_json::json!({
                "live": self.went_live,
                // Pull requests the user's click opened (a test run opens none).
                "pr_opens": self.pr_opens,
                // "File…" choices: the open panel a test run does not show.
                "file_prompts": self.file_prompts,
                // Windows the picker's second step asked to host (a test run hosts none).
                "window_picks": self.window_picks,
                // "New window" asks by recipe: `open` with arguments, `tell` an Apple Event, its
                // "New Window" menu item (a test run asks no app).
                "new_windows_open": self.new_window_asks[0],
                "new_windows_tell": self.new_window_asks[1],
                "new_windows_menu_item": self.new_window_asks[2],
                // File lists the code city read (`code_city`): none with the flag off, none at rest.
                "city_reads": self.city_reads(),
                // Frames the dial asked for because something in it moves: none at rest.
                "dial_frames": self.dial_motion.borrow().frames,
                // Frames the toast asked for as it came or went: none under Reduce Motion.
                "toast_frames": self.toast.frames,
                // Frames drawn while the calendar band's loading bar slides: none once a read lands.
                "band_frames": self.timeline.band_frames.get(),
                // Frames that drew the Calendar section's day: none at rest, its now line moves on the minute.
                "calendar_frames": self.timeline.day_frames.get(),
                // The frames the seal of a Done asked for, and those with its sparks: none under Reduce Motion.
                "seal_frames": self.seals.frames,
                // Frames a side overlay's slide in or back asked for (`overlay_panels`): none under Reduce Motion.
                "overlay_frames": self.slides.frames,
                "seal_spark_frames": self.seals.spark_frames,
                // Commits that pulsed a checkout's dot, and the frames the pulses asked for: none
                // at rest, none under Reduce Motion.
                "repo_pulses": self.model.head_pulses(),
                "repo_pulse_frames": self.repo_pulse_frames.get(),
                // How often GPUI's own handlers were kicked to draw again (`stall`): after a change
                // of the displays, or frames that stopped.
                "redraws": self.redraws,
                // Runs of the tick (`cadence`): each frame just after a change, else few at rest.
                "ticks": self.ticks,
                "frame_reads": self.hosted.values().map(|h| h.frame_reads).sum::<u64>(),
                // Scrolls over an app pane's picture, the entries they made, and the scrolls passed on to an app.
                "wheels": self.wheel.over,
                "wheel_entries": self.wheel.entries,
                "wheels_passed": self.wheel.passed,
            })),
            // Test runs only: each press in an app a sneak label asked for, `[pane, path]`; else null.
            "app_presses": std::env::var_os("OTTOSPACE_E2E").map(|_| &self.sneak_reads.presses),
            // The bindings the user knows (`key_use`), by id. Null outside a test run: what the
            // user's keys are used for is never on the socket.
            "keys_known": std::env::var_os("OTTOSPACE_E2E").map(|_| self.keys_known()),
            // Test runs only: the transcript drawer's rows (`from`, `final`, `text`; a pause as
            // `paused_ms`), and where its controls were drawn, by id. Null outside a test run:
            // what was heard never reaches a client.
            "listen_lines": self.listen_lines(),
            "listen_rects": self.listen_rects(),
            // Test runs only: where an offer chip's words (`words:<pane>`) and their tooltip
            // (`tip`, while it shows) were drawn. Never the words. Null outside a test run.
            "offer_rects": std::env::var_os("OTTOSPACE_E2E").map(|_| {
                let named = |(mark, bounds): (&Mark, &Bounds<Pixels>)| match mark {
                    Mark::OfferWords(pane) => Some((format!("words:{pane}"), rect(*bounds))),
                    Mark::OfferTip => Some(("tip".to_string(), rect(*bounds))),
                    _ => None,
                };
                marks.iter().filter_map(named).collect::<BTreeMap<_, _>>()
            }),
            // Test runs only: where each Tasks heading's fold mark, name and count were drawn
            // (`done:fold`, `done:label`, `done:count`). Null outside a test run.
            "heading_rects": std::env::var_os("OTTOSPACE_E2E").map(|_| {
                let parts = ["fold", "label", "count"];
                let named = |(mark, bounds): (&Mark, &Bounds<Pixels>)| match mark {
                    Mark::Heading(heading, at) => {
                        Some((format!("{}:{}", heading_name(*heading), parts[*at as usize]), rect(*bounds)))
                    }
                    _ => None,
                };
                marks.iter().filter_map(named).collect::<BTreeMap<_, _>>()
            }),
            // While the key reference shows, in a test run: its groups' binding ids in order, the
            // known ones and whether they are open, and where `Known · n` is drawn. Else null.
            "key_reference": std::env::var_os("OTTOSPACE_E2E").filter(|_| self.model.keys).map(|_| {
                let reference = self.key_reference(cx).0;
                key_use::snapshot(&reference, marks.get(&Mark::KeysKnown).copied().map(rect))
            }),
            // The marks in the user's order: name, key, kind, whether its pane or tab is open
            // now, and whether it can be opened. `row` and `buttons`: where the Marks section
            // drew it, or null.
            "marks": self.marks_json(),
            // The name of the mark just taken off, while the Marks section offers to put it back.
            "mark_removed": self.mark_offered(),
            // The workbooks by name: `name`, `panes` (each `kind`: `terminal`, `app` with the
            // `app`'s name, or `empty`), `summary`, `open` with its first open `tab`, and `tabs`
            // (each `title`, `panes`, `summary`, `open`, `tab`), `unfolded`. No directory.
            // `row` and `buttons`: where the Workbook section drew it, or null; so for each tab.
            "workbooks": self.workbooks_json(),
            "workbooks_closing": self.closing_json(),
            // What the Workbook section offers to take back (`deleted`, `updated`), or null.
            "workbook_offer": self.model.workbooks.offered(),
            // A row's open menu (its …), or null: `workbook` and `tab` (whose row), `selected`
            // (the item the keys are on), `items` (each `item`, `label`, `key` and `rect`).
            "row_menu": self.row_menu_json(),
            // Each scroll area whose content does not fit, by its id (`tasks`, `repos`, `apps`,
            // `marks`, `picker`, `keys`, `terminal:<pane>`): its bar's `track` and `thumb` in
            // window points, and the bar's `opacity` (0 once it has faded).
            "scrollbars": self.bars.snapshot(),
            // Test runs only: each scroll area laid out (not a terminal), also one that fits, and which of its edges
            // fade (`top`, `bottom`; the tab row's `left`, `right`). Null outside a test run.
            "overflow": std::env::var_os("OTTOSPACE_E2E").map(|_| self.bars.overflow()),
            // The capture key, and what was drawn of the draft or the brief in the right bar.
            "capture": self.capture_snapshot(),
            // The bug button (`icon`), the report being made (`report`: its marks with their
            // comments, what has the keyboard), the rects blanked, and the picture's size in pixels.
            "bug": self.bug_snapshot(),
            // The right drawer: where its conversation is drawn (`body`), each task's agent
            // (`model`, `ended`, its terminal's `lines`), the workspace ⌘↵ goes to, and why no
            // agent runs for the task on show (`note`).
            "drawer": self.talk_snapshot(),
            // The top row's spaces, hidden ones too: whose (`of`), `hidden`, `selected`, and
            // `tabs` as places; and each live task's badge letters.
            "spaces": self.model.space_rows(),
            "badges": self.model.badges().into_iter().filter_map(|(subject, mono)| Some((subject.task()?.to_string(), serde_json::Value::from(mono)))).collect::<serde_json::Map<String, serde_json::Value>>(),
            // The meeting timeline: counts, times and stages only, never a title or a place.
            "calendar": self.calendar_snapshot(),
            // The status bar's segments: the place (`text`, `rect`), whether a meeting shows (its
            // `rect` and its Join's, never its title), the agents (`text`, `rect`), and how many
            // steps they were shortened (`shrink`, 0 to 3).
            "status_bar": self.status_snapshot(),
            // Frames drawn (`number`), actions dispatched (`seq`) and how many of them the last
            // frame drew (`drawn`); `waited_ms`: how long a test run's reply waited for that frame.
            // `window`: the window's number, for `screencapture -l`.
            "frame": {
                "number": self.renders, "seq": self.seq, "drawn": self.drawn_seq, "waited_ms": 0,
                "window": ns_view(window).map(|view| unsafe { ottospace_mac::window::number(view) }),
            },
        });
        // Restored or rebuilt panes still empty: `pane`, `why`, and where its "Open <App>" is drawn, or null.
        state["unfilled"] = unfilled.into();
        // Test runs only: what each app pane's stream has sent, `stream_samples_<pane>` (`counts`
        // holds numbers only); the pixel size of the frame it holds and the scale it captures at;
        // the scale of ottospace's display and of the parked windows' display.
        if let Some(counts) = state["counts"].as_object_mut() {
            for (id, hosted) in &self.hosted {
                counts.insert(format!("stream_samples_{id}"), hosted.latest.samples.load(Relaxed).into());
                let held = hosted.frame().map_or((0, 0), |f| (f.get_width(), f.get_height()));
                counts.insert(format!("stream_width_{id}"), held.0.into());
                counts.insert(format!("stream_height_{id}"), held.1.into());
                counts.insert(format!("stream_scale_{id}"), hosted.shown_scale.into());
            }
            counts.insert("scale".into(), (window.scale_factor() as f64).into());
            counts.insert("parking_scale".into(), display::parking_scale(window.scale_factor() as f64).into());
        }
        // Where the box offers a choice of where the task works: `new` (a new worktree, the user's
        // choice), the repository's name and the `branch` it would make; else null.
        state["compose_worktree"] = serde_json::json!(self.model.compose_worktree());
        state
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
            self.dispatch(Action::ToggleAxOverlay, self.input, window, cx);
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

    /// Adds an app pane for an already-captured window. `None`: the core opened no pane for it,
    /// and the window is back where it was.
    fn attach(&mut self, hosted: Hosted, action: Action, window: &Window, cx: &mut Context<Self>) -> Option<PaneId> {
        let pid = hosted.pid;
        if !self.known.iter().any(|(p, _)| *p == pid) {
            // Its dialogs open now too: only those that open later are judged (`adopt_new_windows`).
            let ids = ax::Window::all_of(pid).iter().filter(|w| w.judged()).filter_map(ax::Window::id).collect();
            self.known.push((pid, ids));
        }
        let effects = self.dispatch(action, self.input, window, cx);
        // The pane opened last: a settle may ask for more after it (`Effect::WatchSlots`).
        let opened = effects.iter().rev().find(|effect| matches!(effect, Effect::Opened(_)));
        if let Some(Effect::Opened(id)) = opened {
            // A pane in a background tab is not drawn, so the layout sync never parks its
            // window. Move it off screen now; it is sized when its tab is first shown.
            let drawn = self.model.main_panes().contains(id);
            if !drawn {
                let _ = hosted.window.set_position(display::parking_spot(window.scale_factor() as f64));
            }
            if hosted.off_screen {
                self.broke(format!("{}'s window was off screen: it goes back to the middle of the screen", hosted.app));
            }
            trace::record("host", serde_json::json!({ "pane": id, "window": hosted.window_id }));
            self.cells.insert(*id, hosted.bounds.clone());
            self.hosted.insert(*id, hosted);
            return Some(*id);
        }
        trace::record("host_refused", serde_json::json!({ "window": hosted.window_id }));
        hosted.release();
        None
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
        // Cleared first: a notice hosting sets (a window found off screen) outlives a success.
        self.error = None;
        let result = self.try_open_app(name, axis, new_tab, window, cx);
        if let Err(err) = &result {
            self.error = Some(err.clone());
        }
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
                // A listed app that is not running: start it, and host its window when it has one.
                // Only one the user pinned: not every app the Dock or the folders have.
                Err(err) => match self.pinned_closed().find(|app| app.name == name) {
                    Some(app) => return self.launch(Installed { name: app.name, path: app.path }, axis, new_tab, None),
                    None => return Err(err),
                },
            },
        };
        // Already in a pane: go to it (and its tab) instead of doing nothing.
        if let Some((pane, _)) = self.hosted.iter().find(|(_, h)| h.window_id == target.id) {
            let pane = *pane;
            self.dispatch(Action::Focus { pane }, self.follow_up(), window, cx);
            return Ok(());
        }
        let hosted = Hosted::new(target)?;
        self.host_in_new_pane(hosted, axis, new_tab, window, cx);
        Ok(())
    }

    /// A captured window in a new pane along `axis`, or a new tab first. `None`: no pane opened.
    fn host_in_new_pane(
        &mut self,
        hosted: Hosted,
        axis: Axis,
        new_tab: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        // The window is in hand, so the tab will not be left empty.
        if new_tab {
            self.dispatch(Action::NewTab, self.input, window, cx);
        }
        let action = Action::Open { kind: hosted.kind(), axis, beside: None };
        self.attach(hosted, action, window, cx)
    }

    /// Starts an app that is not running, in the background: its window is wanted in a pane, not
    /// in front of ottospace. By the path it was found at, so nothing but that app can start.
    /// `wanted`: the window a mark asked for, which waits in line even if the app is on its way.
    fn launch(
        &mut self,
        app: Installed,
        axis: Axis,
        new_tab: bool,
        wanted: Option<marks::Wanted>,
    ) -> Result<(), String> {
        let on_its_way = self.launching.iter().any(|launch| launch.app.path == app.path);
        if on_its_way && wanted.is_none() {
            return Ok(());
        }
        // Running, with no window to host (minimised, on another Space): `open` would only
        // bring it to the front. A mark still asks: it waits for the window it wants.
        if wanted.is_none() && apps::running_bundles().contains(&app.path) {
            return Err(format!("{} is running but has no window on this Space", app.name));
        }
        if !on_its_way {
            match std::process::Command::new("open").arg("-g").arg(&app.path).status() {
                Ok(status) if status.success() => {}
                Ok(_) => return Err(format!("{} could not be opened", app.name)),
                Err(err) => return Err(format!("{} could not be opened: {err}", app.name)),
            }
        }
        let origin = self.input;
        self.launching.push(Launch { app, axis, new_tab, since: Instant::now(), origin, wanted, windows: None });
        Ok(())
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
        for mut launch in std::mem::take(&mut self.launching) {
            let bundle = Some(PathBuf::from(&launch.app.path));
            let app = running.iter().find(|app| icon::bundle_of(app.pid) == bundle);
            // An app that brings its own windows back opens them one by one: a mark's window is
            // picked by title only once their number has stopped changing.
            let settled = app.is_some_and(|app| {
                let count = ax::Window::standard_of(app.pid).len();
                let since =
                    launch.windows.filter(|(before, _)| *before == count).map_or_else(Instant::now, |(_, at)| at);
                launch.windows = Some((count, since));
                ottospace_core::marks::settled(count, since.elapsed().as_millis() as u64)
            });
            // Listed, but its main window may not be ready yet: try again until the time is up.
            self.launched_later = true;
            let hosted = app.is_some_and(|app| match &launch.wanted {
                Some(wanted) => settled && self.host_window_of(app.pid, wanted, window, cx).is_ok(),
                None => self.with_origin(launch.origin, |this| {
                    this.open_app(&app.name, launch.axis, launch.new_tab, window, cx).is_ok()
                }),
            });
            self.launched_later = false;
            if hosted {
                continue;
            }
            let late = launch.since.elapsed() > LAUNCH_WAIT;
            let seconds = LAUNCH_WAIT.as_secs();
            let why = late.then(|| format!("{} did not open a window in {seconds} seconds", launch.app.name));
            // A rebuilt tab's pane that waited for it says so.
            if let Some((pane, why)) = launch.wanted.as_ref().and_then(|wanted| wanted.into).zip(why.clone()) {
                self.model.update(Action::Unfilled { pane, why }.sent_by(Origin::System));
            }
            // A try that was too early is not a failure yet. Said as the launch's sender: a
            // client's wait neither clears nor covers what the user's step said.
            self.with_origin(launch.origin, |this| this.error.clone_from(&why));
            if !late {
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
            // An app in front within a second of its pane leaving live (a click, a key, the return
            // key): its own late activation from going live, not the user's choice.
            let left = self.left_live.filter(|(_, at)| at.elapsed() < Duration::from_secs(1));
            // Its keys would go to its parked window: ottospace takes them back.
            let late = left.is_some_and(|(pid, _)| front.is_some_and(|(front, _)| front == pid));
            if late && self.model.live.is_none() {
                self.wanted = None;
                cx.activate(true);
            }
        }
        let Some(pane) = self.wanted else { return };
        // Already the one on screen.
        if self.model.live == Some(pane) {
            self.wanted = None;
            return;
        }
        // The core would drop the move behind a box or in visual mode. Wait: do not come
        // forward either, which would only take the app away from the user. Sneak mode waits
        // too: the core keeps the keyboard under the labels.
        if self.model.keys
            || matches!(
                self.model.mode(),
                Mode::Pick | Mode::Palette | Mode::Compose | Mode::Visual | Mode::Sneak | Mode::Confirm
            )
            || self.model.mode() == Mode::Mark
        {
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
        self.dispatch(Action::Enter { pane }, Origin::System, window, cx);
        cx.notify();
    }

    /// Removes panes whose window is gone (closed by the user or the app).
    fn drop_closed(&mut self, window: &Window, cx: &mut Context<Self>) {
        // An unreadable frame alone is no proof: a busy app times the call out, and its parked
        // window would be stranded. The window server says whether the window is still there.
        let gone = |h: &Hosted| h.window.frame().is_none() && !recovery::window_exists(h.window_id);
        let gone: Vec<PaneId> = self.hosted.iter().filter(|(_, h)| gone(h)).map(|(id, _)| *id).collect();
        // An app that remakes its window closes the old one, then opens the new: while it runs, a
        // gone window is reported after `REMADE_MS`, so its new one is seen and offered first.
        let now = now_ms();
        self.gone_since.retain(|id, _| gone.contains(id));
        let gone: Vec<PaneId> = gone
            .into_iter()
            .filter(|id| {
                let running = self.hosted.get(id).is_some_and(|h| ax::app_identity(h.pid).1.is_some());
                let since = *self.gone_since.entry(*id).or_insert(now);
                !running || now.saturating_sub(since) >= REMADE_MS
            })
            .collect();
        for id in gone {
            self.gone_since.remove(&id);
            eprintln!("pane {id}: window gone");
            // Nothing to put back, so stop the stream here instead of `release` in `dispatch`.
            let mut app_running = false;
            if let Some(hosted) = self.hosted.remove(&id) {
                app_running = ax::app_identity(hosted.pid).1.is_some();
                trace::record("window_gone", serde_json::json!({ "pane": id, "window": hosted.window_id }));
                let _ = hosted.stream.stop_capture();
                give_back::forget(hosted.window_id);
                // Its app may still be frontmost with no window here: take the keyboard back.
                if self.model.live == Some(id) && ax::is_frontmost(hosted.pid) {
                    cx.activate(true);
                }
            }
            // A slot waits for it, and so does a pane offering a window its app remade; any other closes.
            let gone = Action::WindowGone { pane: id, app_running, at: now_ms() };
            self.dispatch(gone, Origin::System, window, cx);
        }
    }

    /// Judges windows that appeared since we started watching their app (e.g. a second browser
    /// window), each by its app's newest pane (`rules::decide`): none splits it by itself (only a
    /// `split` rule). It stays a free window over that pane, and a standard one is offered there.
    /// Each window is judged once, when first seen as standard or a dialog, and logged.
    fn adopt_new_windows(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut fresh = Vec::new();
        for (pid, seen) in &mut self.known {
            // "New window" hosts the window it asked for (`host_new_window`).
            if self.asked.as_ref().is_some_and(|asked| asked.of(*pid)) {
                continue;
            }
            for window in ax::Window::all_of(*pid) {
                // A toast or popover is looked at again next time: it may become a window.
                if let Some(id) = window.id()
                    && !seen.contains(&id)
                    && window.quick(ax::Window::judged)
                {
                    seen.insert(id);
                    fresh.push((id, window));
                }
            }
        }
        let drawn = self.model.main_panes();
        // The id read when it was first seen: read again later it may fail, and it is never seen again.
        for (id, ax_window) in fresh {
            let pid = ax_window.pid();
            let beside = self.hosted.iter().filter(|(_, h)| h.pid == pid).map(|(id, _)| *id).max();
            // The pane it would split, if drawn: a background tab's bounds are stale.
            let pane = beside.filter(|id| drawn.contains(id)).and_then(|id| self.hosted.get(&id));
            let pane = pane.map(|h| (h.bounds.get(), h.min_width)).filter(|(b, _)| f32::from(b.size.width) >= 1.0);
            // A short wait for each read: a hung app costs a blink, not seconds.
            let (frame, modal, sheet) = ax_window.quick(|w| (w.frame(), w.modal(), w.sheet()));
            let shape = rules::Shape {
                modal,
                sheet,
                size: frame.map(|f| [f.size.width, f.size.height]),
                pane: pane.map(|(b, _)| to_cg(b.size)).map(|s| [s.width, s.height]),
                min_width: pane.and_then(|(_, min)| min),
                alone: beside.is_none(),
            };
            let (decided, bundle, subrole, title, identifier) = ax_window.quick(|w| window_decide(w, &shape));
            let logged = pop_ups::Seen { bundle: bundle.as_deref(), subrole: &subrole };
            // An offer's line says what the core made of it: a slot's, or the chip's.
            let offers = decided.place == (rules::Place::Float { offer: true }) && beside.is_some();
            if !offers {
                pop_ups::log(&logged, &shape, &decided, None);
            }
            match decided.place {
                rules::Place::Split => {
                    if let Ok(hosted) = capture::target(ax_window).and_then(Hosted::new) {
                        let action = Action::Adopt { kind: hosted.kind(), beside, seen: None };
                        self.attach(hosted, action, window, cx);
                    }
                }
                // Only while its app has a pane: an app given back is left alone.
                rules::Place::Float { offer } if let Some(pane_id) = beside => {
                    let pane = pane.map(|(b, _)| recovery::to_frame(on_screen(b, window, cx)));
                    let screens = recovery::screens();
                    // Parked: no pane of its app is up and the app is not in front, so ottospace is.
                    let up = |id: &PaneId| self.model.live == Some(*id);
                    let parked = !self.hosted.iter().any(|(id, h)| h.pid == pid && up(id)) && !ax::is_active(pid);
                    let ottospace = Some(recovery::to_frame(own_frame(window, cx)));
                    let spots = &screens.spots;
                    let sight =
                        rules::Sight { screens: &screens.visible, main: screens.main, ottospace, parked, spots };
                    let to = frame.and_then(|f| rules::float_to(recovery::to_frame(f), pane, &sight));
                    let moved = to.is_some_and(|(x, y)| ax_window.set_position(CGPoint::new(x, y)).is_ok());
                    ax_window.raise_only();
                    trace::record("float", serde_json::json!({ "window": id, "moved": moved }));
                    if offer {
                        let frame = frame.map(recovery::to_frame).unwrap_or_default();
                        let subrole = subrole.clone();
                        let seen = ottospace_core::slot::Seen { pid, window: id, title, identifier, subrole, frame };
                        self.offer_window(pane_id, seen, (&logged, &shape, &decided), window, cx);
                    }
                }
                rules::Place::Float { .. } | rules::Place::Leave => {}
            }
        }
    }

    /// Parks panes whose size changed, and keeps the live window glued to its pane: re-places it
    /// when the pane moves, and every ~500ms (`check`) if the app or user moved it away. A check
    /// also reads each window's size, for an app that resized it by itself.
    /// On a `check`, returns each settled pane's content size, whole size and window size.
    fn sync_layout(&mut self, window: &Window, cx: &App, check: bool) -> Vec<(PaneId, [f32; 2], [f32; 2], [f32; 2])> {
        let mut measured = Vec::new();
        // Only panes of the main window's tab are drawn; a background pane's bounds are stale.
        let drawn = self.model.main_panes();
        let scale = window.scale_factor() as f64;
        // Only on a check, with windows hosted: it asks each display's mode, and this runs every frame.
        let spot = (check && !self.hosted.is_empty()).then(|| display::parking_spot(scale));
        let live = self.model.live;
        // The windows follow their panes as laid out in this frame.
        let limit = top_limit(window, cx);
        let keep = self.bars_leave();
        // While a divider is dragged, no window is resized: once more on release (`divider_released`), not each frame.
        let dragging = self.divider_held(cx).is_some();
        for (id, hosted) in &mut self.hosted {
            let rect = on_screen(hosted.bounds.get(), window, cx);
            if rect.size.width < 1.0 {
                continue; // not laid out yet
            }
            let crop = hosted.crop_at(rect, limit);
            let rect = pane_rect(rect, keep, live == Some(*id), crop);
            if live == Some(*id) && !dragging {
                let drifted = check && hosted.window_frame().is_some_and(|f| f.origin != raised(rect, crop).origin);
                if hosted.placed != rect || drifted {
                    hosted.place(rect, crop);
                }
            // Also when the spot moved: displays changed, or ottospace is on one of another scale.
            } else if !dragging
                && (hosted.parked_size != rect.size
                    || hosted.crop != crop
                    || spot.is_some_and(|spot| hosted.parked_at != spot))
            {
                hosted.park(rect.size, scale, crop);
            }
            // Only on a check: reading a window's size is a synchronous accessibility call.
            // An app that resized its own window: the preview and the click rule follow it.
            // The window was asked for the pane's size: what it has now is what it kept.
            // Not during a divider's drag: the window was not asked for the pane's size yet.
            if check
                && !dragging
                && let Some(kept) = hosted.show(scale)
                && let Some(frame) = self.frames.get(id)
                && drawn.contains(id)
            {
                hosted.min_width = rules::refused_width(hosted.min_width, kept.width, rect.size.width);
                let content = [rect.size.width as f32, rect.size.height as f32];
                // The pane's size with its 1pt border.
                let outer = frame.get().size;
                let outer = [f32::from(outer.width) + 2.0, f32::from(outer.height) + 2.0];
                measured.push((*id, content, outer, [kept.width as f32, kept.height as f32]));
            }
        }
        measured
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
            trace::record("terminal_exit", serde_json::json!({ "pane": id }));
            self.dispatch(Action::Close { pane: Some(id) }, Origin::System, window, cx);
        }
        if redraw {
            self.output = true;
            cx.notify();
        }
    }

    /// Keys go to the focused terminal. ⌘ shortcuts do not encode, so they fall through to bindings.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        // Still held after it ended sneak mode (a wrong letter, say): not typed into the terminal.
        if self.sneak_spent && event.is_held {
            cx.stop_propagation();
            return;
        }
        self.sneak_spent = false;
        // So for the key that closed a row's menu.
        if self.row_menu_spent && event.is_held {
            cx.stop_propagation();
            return;
        }
        self.row_menu_spent = false;
        // A bug report covers the window: it takes every key but ⌘ shortcuts.
        if self.model.bug.is_some() {
            return self.bug_key(event, window, cx);
        }
        // The key reference takes the keyboard: Esc closes it, and no other key reaches the
        // terminal or the picker behind it. (⌘ shortcuts go to their bindings; the core ignores
        // pane actions while it is open, and ⌘/ closes it.)
        if self.model.keys {
            let plain = !(key.modifiers.platform || key.modifiers.control || key.modifiers.alt);
            if key.key == "escape" {
                self.dispatch(Action::CloseKeys, self.input, window, cx);
            } else if key.key == "enter" && plain {
                self.dispatch(Action::KeysKnown, self.input, window, cx);
            } else if let Some(step) = key_step(&key.key, key.modifiers.shift).filter(|_| plain) {
                // The list is longer than a small window: these keys scroll it.
                self.bars.step("keys", step);
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        // A row's open menu takes the user's plain keys: its own, and any other closes it.
        let plain = !(key.modifiers.platform || key.modifiers.control || key.modifiers.alt);
        // The slot dialog: ↵ keeps, Esc closes, other keys wait.
        if plain && self.model.sneak.is_none() && self.slot_dialog_key(&key.key, window, cx) {
            cx.stop_propagation();
            return;
        }
        if plain && self.model.sneak.is_none() && self.agent_menu_key(&key.key, window, cx) {
            cx.stop_propagation();
            return;
        }
        if plain && self.model.sneak.is_none() && self.row_menu_key(&key.key, window, cx) {
            cx.stop_propagation();
            return;
        }
        // Esc during a pane drag cancels it, and a `mouse` request's button is up again.
        // Not a client's Esc over the user's reorder: only the user puts it back.
        let users_reorder = self.model.reorder().is_some() && self.input != Origin::User;
        if key.key == "escape" && cx.has_active_drag() && !users_reorder {
            self.reorder_cancel(window, cx);
            self.held = false;
            cx.stop_active_drag(window);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        // ⌘⇧⌥1..9 move the focused pane to a tab of the selected space, as ⌘1..9 count them. GPUI
        // gives no physical key, so the shifted characters are trusted on a US layout only.
        let m = key.modifiers;
        let us_layout = matches!(cx.keyboard_layout().id(), "com.apple.keylayout.US" | "com.apple.keylayout.ABC");
        let place = keys::move_to_tab_key(&key.key, m.shift, us_layout);
        if let Some(place) = place.filter(|_| m.platform && m.alt && !m.control) {
            self.key_used(Binding::MoveToTab);
            if let Some(tab) = self.model.visible_tab(place) {
                self.dispatch(Action::MovePane { pane: None, tab }, self.input, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        // ⌘⌥1..9 go to one of the first nine marks.
        let digit = key.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d));
        if let Some(digit) = digit.filter(|_| m.platform && m.alt && !m.shift && !m.control && self.on(Feature::Marks))
        {
            self.key_used(Binding::MarkNumber);
            let _ = self.go_to_mark(MarkRef::Place(digit - 1), Axis::Right, window, cx);
            cx.stop_propagation();
            return;
        }
        // ⌘⌃1..⌘⌃9 select a space of the top row.
        if let Some(digit) = digit.filter(|_| m.platform && m.control && !m.alt && !m.shift) {
            self.key_used(Binding::SpaceNumber);
            self.dispatch(Action::SpaceSelect { index: digit - 1 }, self.input, window, cx);
            cx.stop_propagation();
            return;
        }
        // ⌘1..⌘9 select a tab of the selected space.
        if key.modifiers.platform
            && let Some(digit) = key.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d))
        {
            self.key_used(Binding::TabNumber);
            self.dispatch(Action::SelectVisibleTab { index: digit - 1 }, self.input, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.model.sneak.is_some() {
            self.sneak_key(event, window, cx);
            return;
        }
        if self.model.mark_input.is_some() {
            self.mark_key(event, window, cx);
            return;
        }
        if self.model.picker.is_some() {
            self.picker_key(event, window, cx);
            return;
        }
        if self.model.palette.is_some() {
            self.palette_key(event, window, cx);
            return;
        }
        if self.model.mode() == Mode::Inspect {
            self.inspector_key(event, window, cx);
            return;
        }
        // The drawer's editor has the keyboard: every key but a ⌘ shortcut is its.
        if self.model.editor_keys() {
            return self.editor_key(event, cx);
        }
        if self.model.compose_keys() {
            self.compose_key(event, window, cx);
            return;
        }
        if self.model.mode() == Mode::Annotate {
            self.annotate_key(event, window, cx);
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
        if self.model.mode() == Mode::Talk {
            return self.talk_key(event, window, cx);
        }
        // The Decisions card has the keyboard: its numbers answer, S skips, L puts off, Esc closes.
        if self.model.keyboard_decision() && self.decision_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.model.keyboard_viewer().is_some() && self.viewer_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        // ↵ on a focused pane that waits for a window its app remade: Bring it here.
        if key.key == "enter"
            && !key.modifiers.modified()
            && let Some(id) = self.model.offer_enter()
        {
            self.offer_send(Action::OfferAccept { window: id, axis: Axis::Right }, window, cx);
            cx.stop_propagation();
            return;
        }
        // ↵ on a focused slot that offers a window: Bring it here (the core says which).
        if key.key == "enter"
            && !key.modifiers.modified()
            && let Some(pane) = self.model.enter_accepts()
        {
            self.offer_send(Action::SlotAccept { pane }, window, cx);
            cx.stop_propagation();
            return;
        }
        let Some(terminal) = self.model.focused.and_then(|id| self.terminals.get(&id)) else {
            // Esc that nothing else takes (no mode, no terminal to type it in) dismisses the notice.
            if key.key == "escape" && !key.modifiers.modified() && self.dismiss_notice(window, cx) {
                cx.stop_propagation();
            }
            return;
        };
        let key = &event.keystroke;
        let m = key.modifiers;
        let mods = crate::mods(m);
        if clears_selection(&key.key, mods, terminal.has_selection()) {
            terminal.clear_selection();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        terminal::send_key(terminal, event, cx);
    }

    /// The wheel or trackpad over a terminal pane.
    fn wheel(&mut self, id: PaneId, delta: ScrollDelta, cx: &mut Context<Self>) {
        let height = self.cell.height;
        if let Some(terminal) = self.terminals.get_mut(&id) {
            terminal.wheel(delta.pixel_delta(height).y.into(), height.into());
            cx.notify();
        }
    }

    /// A press on a terminal starts a selection there: a drag, or a word or line by click count;
    /// with ⇧ it moves the far end of the one there (`press`).
    /// It also focuses the pane (`click_pane`), so the first click on an unfocused pane selects too.
    fn terminal_mouse_down(&mut self, id: PaneId, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        // While a picture is annotated a click on another pane captures it: it selects nothing.
        if self.model.annotated().is_some() {
            return;
        }
        let Some(at) = self.cell_under(id, event.position) else { return };
        // ⌘-click on a text file's path: the viewer beside, reused for the next one.
        if event.modifiers.platform && self.terminal_path_click(id, at, window, cx) {
            return;
        }
        if let Some(terminal) = self.terminals.get(&id) {
            terminal.select_start(at, press(event.click_count, event.modifiers.shift));
            (self.dragging, self.drag_at) = (Some(id), None);
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
        self.drag_at = Some(event.position);
        if let (Some(at), Some(terminal)) = (self.cell_under(id, event.position), self.terminals.get(&id)) {
            terminal.select_to(at);
            cx.notify();
        }
    }

    /// While a selection drag is held past the grid's top or bottom, the terminal scrolls a
    /// little each step (`edge_scroll`), and the selection follows into the history.
    fn drag_scroll(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(at)) = (self.dragging, self.drag_at) else { return };
        let (Some(bounds), Some(terminal)) = (self.cells.get(&id).map(|c| c.get()), self.terminals.get(&id)) else {
            return;
        };
        let rows = terminal.grid_size().1 as f32 * f32::from(self.cell.height);
        let lines = edge_scroll(f32::from(at.y - bounds.origin.y), rows, self.cell.height.into());
        if let (true, Some(cell)) = (lines != 0, self.cell_under(id, at)) {
            terminal.select_scrolling(lines, cell);
            cx.notify();
        }
    }

    /// A pane drag moved: note the drop zone under the pointer, to draw it and to drop there.
    fn pane_drag_move(&mut self, source: PaneId, at: gpui::Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        // A live window would cover its pane's drop zones: park it. A click makes it live again.
        if self.model.live.is_some() {
            self.dispatch(Action::Leave, self.input, window, cx);
        }
        let drop_at = self.drop_zone(source, at);
        if drop_at != self.drop_at {
            self.drop_at = drop_at;
            cx.notify();
        }
    }

    /// The drawn pane under `at`, other than the dragged one, and the zone of it `at` is in.
    fn drop_zone(&self, source: PaneId, at: gpui::Point<Pixels>) -> Option<(PaneId, Zone)> {
        let drawn = self.model.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let under = |id: &PaneId| Some((*id, self.frames.get(id)?.get())).filter(|(_, b)| b.contains(&at));
        let (id, bounds) = drawn.iter().filter(|id| **id != source).find_map(under)?;
        let local = at - bounds.origin;
        Some((id, Zone::at(local.x / bounds.size.width, local.y / bounds.size.height)))
    }

    /// The terminal cell under a window position, from the same bounds the grid is painted in.
    fn cell_under(&self, id: PaneId, position: gpui::Point<Pixels>) -> Option<CellPoint> {
        let (bounds, terminal) = (self.cells.get(&id)?.get(), self.terminals.get(&id)?);
        let local = position - bounds.origin;
        let cell = (self.cell.width.into(), self.cell.height.into());
        Some(cell_at(local.x.into(), local.y.into(), cell, terminal.grid_size()))
    }

    /// List mode takes every key but ⌘ shortcuts: they move over the bar's rows and act on one.
    fn list_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if self.model.app_lists.filtering {
            return self.apps_filter_key(event, window, cx);
        }
        if key.modifiers.platform {
            return; // ⌘ shortcuts still reach their bindings; ⌘⌃E leaves
        }
        cx.stop_propagation();
        // ⌃X is not `x`. A held key only moves: one press dismisses or opens one row.
        let pressed = list_key(&key.key, key.modifiers.shift).filter(|_| !key.modifiers.control && !key.modifiers.alt);
        let repeats = |key: &ListKey| matches!(key, ListKey::Move(_) | ListKey::Reorder(_));
        let action = match pressed.filter(|key| !event.is_held || repeats(key)) {
            Some(ListKey::Move(delta)) => Action::ListMove { delta },
            // A workbook's row opens as a click on it does; space lists its tabs. A tab's row opens that tab.
            Some(ListKey::Open { .. }) if self.model.selected_workbook().is_some() => {
                let Some((id, tab)) = self.model.selected_workbook() else { return };
                let control = match (tab, key.key.as_str()) {
                    (Some(_), _) => "workbook-tab",
                    (None, "space") => "workbook-fold",
                    (None, _) => "workbook",
                };
                return self.click_workbook(control, id, tab, window, cx);
            }
            // In Repos: a new worktree of the selected row's repository.
            Some(ListKey::Add) if self.model.sidebar == Some(Section::Repos) => {
                if let Some(path) = self.model.selected_repo() {
                    self.add_worktree(&path, window, cx);
                }
                return cx.notify();
            }
            Some(ListKey::Add) => match self.model.selected_workbook() {
                Some((id, _)) => return self.click_workbook("workbook-add", id, None, window, cx),
                None => return,
            },
            Some(ListKey::Open { down }) => {
                self.check_marks(cx); // a mark's row opens only what is there
                Action::ListChoose { axis: if down { Axis::Down } else { Axis::Right } }
            }
            Some(ListKey::Reorder(delta)) => Action::ListReorder { delta },
            Some(ListKey::Rename) => Action::ListMarkEdit { field: Field::Name },
            Some(ListKey::Letter) => Action::ListMarkEdit { field: Field::Key },
            Some(ListKey::Section(delta)) => {
                self.refresh_lists(cx); // the section it lands on shows what is there now
                self.dispatch(Action::Tick { now: now_ms() }, Origin::System, window, cx);
                Action::ListSection { delta }
            }
            Some(ListKey::Snooze) => Action::ListSnooze { until: now_ms().saturating_add(SNOOZE_MS) },
            Some(ListKey::Hold) => Action::ListHold,
            Some(ListKey::Dismiss) => Action::ListDismiss,
            Some(ListKey::Undo) => Action::ListUndo,
            Some(ListKey::Pin) => Action::ListPin,
            Some(ListKey::Filter) => Action::AppsFilter { on: true },
            Some(ListKey::Leave) => Action::LeaveList,
            // On a rail ← and → cross zones; in the Calendar they change its day; elsewhere nothing.
            Some(ListKey::Side(side)) if self.model.rail_focus.is_some() => {
                Action::FocusDir { dir: if side < 0 { Dir::Left } else { Dir::Right } }
            }
            Some(ListKey::Side(side)) if self.model.sidebar == Some(Section::Calendar) => {
                let step = if side < 0 { DayStep::Back } else { DayStep::Forward };
                Action::CalendarDay { step }
            }
            // In Repos ← folds the selected row's repository under its name, → unfolds it.
            Some(ListKey::Side(side)) if self.model.sidebar == Some(Section::Repos) => {
                let Some(path) = self.model.selected_repo() else { return };
                Action::RepoFold { path, fold: Some(side < 0) }
            }
            Some(ListKey::Today) if self.model.sidebar == Some(Section::Calendar) => {
                Action::CalendarDay { step: DayStep::Today }
            }
            Some(ListKey::Side(_) | ListKey::Today) | None => return,
        };
        self.dispatch(action, self.input, window, cx);
        if self.model.sidebar == Some(Section::Calendar) {
            self.reveal_meeting();
        } else if let Some(child) = self.selected_child() {
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
            // Each repository's name is a row; the new worktree's field, under one, is not.
            Some(Section::Repos) => {
                let typed = match &self.model.mark_input {
                    Some(MarkInput::Worktree { repo, .. }) => {
                        Some(ottospace_core::list::ListRow::RepoHead(repo.clone()))
                    }
                    _ => None,
                };
                let field = typed.and_then(|head| self.model.list_rows().iter().position(|row| *row == head));
                return Some(at + usize::from(field.is_some_and(|field| at > field)));
            }
            Some(Section::Apps) => return Some(self.apps_child(at)),
            // The offer to put a mark back, if it shows, is above the rows.
            Some(Section::Marks) => return Some(at + usize::from(self.mark_offered().is_some())),
            _ => return Some(at),
        }
        // The offer to undo, if it shows; how to add a task, if none is listed; then the headings
        // and rows, which list mode walks as they are drawn.
        let empty = self.model.task_sections(0).is_empty();
        Some(usize::from(self.task_offer().is_some()) + usize::from(empty) + at)
    }

    /// The offer to take a task's move back, while it shows. A removed mark's is the Marks section's.
    fn task_offer(&self) -> Option<ottospace_core::tasks::Notice> {
        let tasks = |moved: ottospace_core::tasks::Moved| moved != ottospace_core::tasks::Moved::MarkRemoved;
        self.model.undo_offer.filter(|offer| tasks(offer.moved) && !offer.moved.of_apps())
    }

    /// ⌘J: the next task that needs you.
    fn next_task(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::Tick { now: now_ms() }, self.input, window, cx);
        // Decisions first under its flag (`Model::next_needs_you`).
        self.dispatch(self.model.next_needs_you(), self.input, window, cx);
    }

    /// Reads the Apps and Repos lists again.
    fn refresh_lists(&mut self, cx: &mut Context<Self>) {
        self.refresh_apps();
        self.offer_installed(cx);
        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        self.status_git.forget();
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
                    self.copy_text(text, Some(id), Verb::Copied, cx);
                }
                self.dispatch(Action::LeaveVisual, self.input, window, cx);
            }
            VisualKey::Exit => drop(self.dispatch(Action::LeaveVisual, self.input, window, cx)),
        }
        cx.notify();
    }

    /// ⌘C: copies the open text field's selection, else the focused terminal's, in visual mode too. With nothing selected it
    /// does nothing.
    fn copy(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.transcript_copy(cx) || self.field_copy(false, window, cx) {
            return;
        }
        let Some(id) = self.copy_pane() else { return };
        let Some(text) = self.terminals.get(&id).and_then(Terminal::selected) else { return };
        self.copy_text(text, Some(id), Verb::Copied, cx);
    }

    /// The terminal ⌘C copies from, while it has a selection: Copy in the Edit menu is on with it.
    fn copy_pane(&self) -> Option<PaneId> {
        let id = self.model.visual.or_else(|| self.model.keyboard_terminal())?;
        self.terminals.get(&id)?.has_selection().then_some(id)
    }

    /// Tells the core the time now, and gives a meeting alert that time brought (`Effect::MeetingAlert`).
    /// The rest (a closed workspace's panes) waits for the next dispatch, which has a window.
    fn tick_now(&mut self) {
        for effect in self.model.update(Action::Tick { now: now_ms() }.sent_by(Origin::System)) {
            self.meeting_effect(&effect);
            if !matches!(effect, Effect::MeetingAlert { .. } | Effect::OpenLink { .. }) {
                self.held_effects.push(effect);
            }
        }
    }

    /// Every copy of ottospace's: to the clipboard, then the notice that says so (`clip::Clip`).
    /// `pane`: where the text came from, which pulses. None: a text field or a chat's Copy.
    pub(crate) fn copy_text(&mut self, text: String, pane: Option<PaneId>, verb: Verb, cx: &mut Context<Self>) {
        let amount = clip::Amount::of(&text);
        self.clipboard.copy(text, cx);
        self.clip_reduced = ottospace_mac::meeting::reduce_motion();
        // The notice has no effects, so needs no window: a chat's Copy button has none to give.
        self.tick_now();
        self.model.update(Action::Copied { pane, verb, amount }.sent_by(self.input));
        cx.notify();
    }

    /// ⌘V: pastes the clipboard's text into the open text field, else the focused terminal.
    /// There the core decides: the user's alone, and text with a line break asks first.
    fn paste(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.field_paste(window, cx) {
            return;
        }
        let text = self.clipboard.text(cx);
        if self.editor_paste(text.as_deref()) || self.talk_paste(text.clone()) {
            return;
        }
        let (Some(pane), Some(text)) = (self.model.keyboard_terminal(), text) else { return };
        let amount = clip::Amount::of(&text);
        self.pasting = Some((pane, text));
        self.clip_reduced = ottospace_mac::meeting::reduce_motion();
        self.dispatch(Action::Paste { pane, amount }, self.input, window, cx);
        // Refused, or pasted at once: nothing waits.
        if self.model.confirm.as_ref().is_none_or(|confirm| confirm.paste.is_none()) {
            self.pasting = None;
        }
    }

    /// ⌘E / ⌘⇧E: the picker for a pane right of / below the focused one; either closes an open one.
    fn toggle_picker(&mut self, axis: Axis, window: &Window, cx: &mut Context<Self>) {
        if self.model.picker.is_some() {
            self.dispatch(Action::ClosePicker, self.input, window, cx);
        } else {
            self.open_picker(axis, false, window, cx);
        }
    }

    /// A top-bar split button: the picker.
    pub(crate) fn top_button(&mut self, axis: Axis, window: &Window, cx: &mut Context<Self>) {
        self.open_picker(axis, false, window, cx);
    }

    /// Opens the pane picker: a terminal, then each running app.
    fn open_picker(&mut self, axis: Axis, new_tab: bool, window: &Window, cx: &mut Context<Self>) {
        self.refresh_apps();
        // What `refresh_apps` just listed: running apps, then the pinned ones that are not.
        let apps = self.apps.iter().map(|app| PickItem::App { name: app.name.clone() });
        // A test run's made-up running apps (it runs none of its own).
        let apps = apps.chain(pick::fixture_apps().into_iter().map(|app| PickItem::App { name: app.name }));
        let closed = self.pinned_closed().map(|app| PickItem::Closed { name: app.name, path: app.path });
        let items = [PickItem::Terminal, PickItem::File].into_iter().chain(apps).chain(closed).collect();
        self.offer_installed(cx);
        self.dispatch(Action::OpenPicker { axis, new_tab, items }, self.input, window, cx);
    }

    /// While the picker is open it takes every key: text edits the filter, ↑↓ move, enter opens.
    /// ⌘ shortcuts the filter does not use still reach their bindings.
    fn picker_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let action = |key| match key {
            FilterKey::Close => Action::ClosePicker,
            FilterKey::Choose => Action::PickerChoose { index: None },
            FilterKey::Move(delta) => Action::PickerMove { delta },
        };
        self.filtered_list_key(event, action, window, cx);
    }

    /// The key reference, from the keymap: the keys not yet known first, each with a bar of its
    /// recent use, and the known ones folded under `Known · n`. ⌘/ or Esc or a click closes it.
    fn render_keys(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (reference, texts) = self.key_reference(cx);
        let line = |line: &Line| {
            let (keys, what, fill) = match line {
                Line::Key { binding, fill } => {
                    (texts.get(binding).cloned().unwrap_or_default(), binding.label(), *fill)
                }
                Line::Note { keys, what } => (keys.to_string(), *what, 0.),
            };
            div()
                .flex()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .w(px(112.))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_color(rgb(theme::TEXT)).child(keys))
                        .child(key_use::bar(fill)),
                )
                // A long line wraps in a narrow window.
                .child(div().min_w_0().text_color(rgb(theme::MUTED)).child(what))
        };
        let group = |title: String, lines: &[Line]| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(theme::PRIMARY)).child(title))
                .children(lines.iter().map(line))
        };
        let groups: Vec<(String, &[Line])> =
            reference.groups.iter().map(|(of, lines)| (of.label().to_uppercase(), &lines[..])).collect();
        let columns = key_use::columns(&groups, 3).into_iter().map(|column| {
            div().w(px(280.)).flex().flex_col().gap_4().children(column.into_iter().map(|(t, l)| group(t.clone(), l)))
        });
        let columns = div().flex().gap_8().children(columns);
        let known = (!reference.known.is_empty()).then(|| {
            let open = reference.known_open;
            let header = div()
                .id("keys-known")
                .relative()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .hover(|d| d.text_color(rgb(theme::TEXT)))
                .cursor_pointer()
                .child(format!("{} Known · {}", if open { "▾" } else { "▸" }, reference.known.len()))
                .child(mark(&self.marks, Mark::KeysKnown))
                // Not the backdrop's: a click anywhere else closes the reference.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.dispatch(Action::KeysKnown, this.input, window, cx);
                    }),
                );
            let lines =
                key_use::columns(&reference.known.iter().map(|l| ((), std::slice::from_ref(l))).collect::<Vec<_>>(), 3);
            let rows = lines.into_iter().map(|column| {
                div().w(px(280.)).flex().flex_col().gap_1().children(column.into_iter().map(|(_, l)| line(&l[0])))
            });
            div().flex().flex_col().gap_2().child(header).when(open, |d| d.child(div().flex().gap_8().children(rows)))
        });
        let body = div().p_4().flex().flex_col().gap_6().child(columns).children(known);
        // No taller than the window: the list scrolls inside the panel.
        let panel = dialog::panel(theme::BORDER_STRONG).min_h_0().max_w_full().flex().flex_col().overflow_hidden();
        // The columns keep their height inside the list: a flex row would stretch them to its own.
        let list = div().id("keys-rows").flex_1().overflow_y_scroll().child(body);
        let panel = panel.child(self.bars.rounded(
            "keys",
            list,
            scrollbar::Fade {
                radius: (dialog::RADIUS - 1., dialog::RADIUS - 1.),
                ..scrollbar::Fade::to(theme::RAISED)
            },
        ));
        dialog::backdrop("keys-backdrop", 0.5, Action::CloseKeys, cx)
            .p_6()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(panel)
    }

    /// Opens the new-task box: ⌘N, or a click on the home screen's line.
    fn new_task(&mut self, window: &Window, cx: &mut Context<Self>) {
        // The repositories as they are now, for `>name`; what is open, for `^name`.
        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        self.tell_compose_context(cx);
        self.dispatch(Action::OpenCompose { text: String::new() }, self.input, window, cx);
    }

    /// "Done · <task>"'s New task here: as ⌘N, but the user's alone (the core refuses a client's).
    fn new_task_here(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        self.tell_compose_context(cx);
        self.dispatch(Action::DoneNewTask, self.input, window, cx);
    }

    /// The picker dropdown, under the split buttons. A click outside it closes it.
    fn render_picker(&self, picker: &Picker, cx: &mut Context<Self>) -> AnyElement {
        if picker.transcribe {
            return self.render_sound_picker(picker, cx);
        }
        if let Some(app) = &picker.app {
            return self.render_window_picker(picker, app, cx);
        }
        let selected = picker.selected;
        let rows = picker.matches().into_iter().enumerate().map(|(i, item)| {
            let (tag, color) = match item {
                PickItem::Terminal => ("TERM", theme::PRIMARY),
                PickItem::File => ("FILE", theme::PRIMARY),
                PickItem::App { .. } => ("APP", theme::APP),
                PickItem::Closed { .. } => ("APP", theme::MUTED),
                PickItem::Sound { .. } | PickItem::Window { .. } | PickItem::NewWindow { .. } => ("APP", theme::APP),
            };
            dialog::filter_row(("pick", i), i == selected)
                .child(match item {
                    // An app shows its icon where the tag would be.
                    // A running app's icon is kept by its name, one that is not by its bundle.
                    PickItem::App { name: key } | PickItem::Closed { path: key, .. }
                        if self.icons.contains_key(key) =>
                    {
                        div().w(px(36.)).child(self.app_icon(key))
                    }
                    _ => div().w(px(36.)).text_xs().text_color(rgb(color)).child(tag),
                })
                .child(item.label().to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::PickerChoose { index: Some(i) }, this.input, window, cx);
                }))
        });
        let mut rows: Vec<AnyElement> = rows.map(IntoElement::into_any_element).collect();
        // Once something is typed, the apps that are not running come under a heading.
        let closed = |item: &&PickItem| matches!(item, PickItem::Closed { .. });
        if let Some(at) = picker.matches().iter().position(closed).filter(|_| !picker.query.text().is_empty()) {
            let heading = div().px_3().pt_2().pb_1().text_xs().text_color(rgb(theme::MUTED)).child("NOT RUNNING");
            rows.insert(at, heading.into_any_element());
        }
        self.picker_panel(320., picker_title(picker).into(), rows, "No match", cx)
    }

    /// ⌘E's dropdown, `width` wide: `title` and the filter over `rows`. A click outside closes it.
    fn picker_panel(
        &self,
        width: f32,
        title: SharedString,
        rows: Vec<AnyElement>,
        empty: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = dialog::floating(width).absolute().top(px(TOP_BAR + 6.)).right(px(8.)).max_h(px(420.));
        let header = div()
            .h(px(34.))
            .text_xs()
            .child(div().flex_none().text_color(rgb(theme::PICK)).child(title))
            .child(self.render_field("type to filter", cx));
        let panel = self.list_panel(panel, header, "picker", rows, empty);
        dialog::backdrop("picker-backdrop", 0.25, Action::ClosePicker, cx).child(panel).into_any_element()
    }

    /// The transcript's picker ("Transcribe a window…"), in ⌘E's style: each app that can play
    /// sound and its windows, the one playing now first and marked. A pick hears that app.
    fn render_sound_picker(&self, picker: &Picker, cx: &mut Context<Self>) -> AnyElement {
        let selected = picker.selected;
        let rows = picker.matches().into_iter().enumerate().filter_map(|(i, item)| {
            let PickItem::Sound { name, window, playing, .. } = item else { return None };
            let icon = match self.icons.contains_key(name) {
                true => div().w(px(28.)).flex_none().child(self.app_icon(name)),
                false => div().w(px(28.)).flex_none(),
            };
            let label = match window {
                Some((window, _)) => format!("{name} · {window}"),
                None => name.clone(),
            };
            let now = playing.then(|| {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(rgb(theme::RECORDING))
                    .child(div().size(px(6.)).rounded_full().bg(rgb(theme::RECORDING)))
                    .child("sound now")
            });
            let row = dialog::filter_row(("pick", i), i == selected)
                .child(icon)
                .child(div().flex_1().min_w_0().truncate().child(label))
                .children(now)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::PickerChoose { index: Some(i) }, this.input, window, cx);
                }));
            Some(row.into_any_element())
        });
        let rows: Vec<AnyElement> = rows.collect();
        let panel = dialog::floating(360.).absolute().top(px(TOP_BAR + 6.)).right(px(8.)).max_h(px(420.));
        let header = div()
            .h(px(34.))
            .text_xs()
            .child(div().text_color(rgb(theme::PICK)).child("Listen to…"))
            .child(self.render_field("type to filter", cx));
        let foot = div()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child("↵ listen · Esc cancel · needn't be in a pane");
        let panel = self.list_panel(panel, header, "picker", rows, "No app to listen to").child(foot);
        dialog::backdrop("picker-backdrop", 0.25, Action::ClosePicker, cx).child(panel).into_any_element()
    }

    /// Carries out `Effect::FillDisplay`. A run with hosting off changes no window: its own
    /// floats over the user's screen.
    fn fill_display(&mut self, on: bool, window: &Window, cx: &mut Context<Self>) {
        if !self.hosting {
            return;
        }
        // SAFETY: GPUI's own view, on the main thread.
        let at = ns_view(window).and_then(|view| unsafe { ottospace_mac::window::placement(view) });
        // Read first too: AppKit may push the window under a menu bar that comes back.
        if let (false, Some(filled)) = (on, &mut self.filled) {
            filled.left = at.as_ref().map(|at| at.window);
        }
        // The bars before anything that can fail: nothing leaves them away. `at` was read first:
        // `visible` is what they leave while they show.
        ottospace_mac::window::hide_bars(on);
        // Kept from a turn before whose frame is still to come: its `before` is the true one.
        if let (true, None, Some(at)) = (on, self.filled, at) {
            let frame = fill::frame(at.screen, at.safe_top);
            self.filled = Some(Filled { before: at.window, frame, visible: at.visible, set: false, left: None });
        }
        let Some(view) = ns_view(window).map(|view| view as usize) else { return };
        // Not in this update: GPUI drops the resize AppKit reports while the app is borrowed,
        // and the layout keeps the old size (as its own `zoom_window` defers). By then the mode
        // may have turned again: the frame is the one the model asks for now.
        cx.spawn(async move |this, cx| {
            let Ok((on, Some(filled))) = this.update(cx, |this, _| (this.model.fill_display, this.filled)) else {
                return;
            };
            let view = view as *mut std::ffi::c_void;
            // SAFETY: the window's own view, alive while its workspace is; the main thread.
            if let Some(at) = unsafe { ottospace_mac::window::placement(view) } {
                let screens = recovery::screens();
                let now = filled.left.unwrap_or(at.window);
                let back = || fill::back(now, filled.frame, filled.before, &screens.visible, screens.main);
                unsafe { ottospace_mac::window::set_frame(view, if on { filled.frame } else { back() }) };
            }
            let _ = this.update(cx, |this, cx| {
                this.filled = this.filled.filter(|_| on).map(|filled| Filled { set: true, ..filled });
                cx.notify();
            });
        })
        .detach();
    }

    /// Ends the mode when the window no longer has its display's frame: dragged, zoomed, moved
    /// by the system, or the display changed under it.
    fn fill_holds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filled.is_some_and(|filled| filled.set) || !self.model.fill_display {
            return;
        }
        // SAFETY: GPUI's own view, on the main thread.
        let at = ns_view(window).and_then(|view| unsafe { ottospace_mac::window::placement(view) });
        if at.is_some_and(|at| !fill::holds(at.window, fill::frame(at.screen, at.safe_top))) {
            self.dispatch(Action::FillEnded, Origin::System, window, cx);
        }
    }

    /// While the window fills its display: what the menu bar and the Dock leave when they show.
    fn bars_leave(&self) -> Option<ottospace_core::ax::Frame> {
        self.filled.filter(|_| self.model.fill_display).map(|filled| filled.visible)
    }

    /// A click on a pane body. An app pane goes live, then the click is replayed so it lands in
    /// the app (otherwise the first click is spent on the swap). Other panes just take focus.
    /// What a click on an app pane does is the core's `hosted::click` (`hosting` here).
    fn click_pane(&mut self, id: PaneId, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        // While a picture is annotated, a click on another pane is a capture of that one.
        if self.model.annotated().is_some_and(|material| material.pane != id) {
            self.error = self.capture_pane(Some(id), self.input, window, cx).err();
            return cx.notify();
        }
        // Its own pane, in the moment before its picture is taken: the click is for the picture.
        if self.snapshot_pending(id) {
            return;
        }
        let left = self.left_on_activation == Some(id);
        let live = hosting::was_live(self.model.live == Some(id), left, event.first_mouse);
        // Choose first: going live reads the window's size, so the click is judged against what
        // the window covers now. A pane that is no app's only takes focus.
        if !live {
            self.choose_pane(id, window, cx);
        }
        let Some(hosted) = self.hosted.get(&id) else { return };
        let local = event.position - hosted.bounds.get().origin;
        let local = (f32::from(local.x) as f64, f32::from(local.y) as f64);
        let (x, y) = match hosting::click(live, local, (hosted.shown.width, hosted.shown.height)) {
            // A `mouse` request stays inside ottospace: it enters the app but does not click in it.
            Click::Replay(_) if self.injected => return,
            Click::Replay(at) => at,
            Click::Leave => {
                let pid = hosted.pid;
                // Explicit: the activation observer ignores the first 400 ms after going live.
                self.dispatch(Action::Leave, self.input, window, cx);
                // A `mouse` request must not bring ottospace forward either.
                if self.injected {
                    return;
                }
                // Soon after going live the app's activation may still be on its way: ottospace
                // takes the keyboard back once it has landed.
                cx.activate(true);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.model.live.is_none() && ax::is_frontmost(pid) {
                            cx.activate(true);
                        }
                    });
                })
                .detach();
                return;
            }
            Click::Enter | Click::Nothing => return,
        };
        let origin = hosted.placed.origin;
        let at = CGPoint::new(origin.x + x, origin.y + y);
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
        // Cells as wide as the rail, as tall as the list's header. The open section's cell has the
        // list's background, an accent bar at its left edge and no border on its right: it reads as
        // joined to the list.
        let open = self.model.sidebar;
        let sections = self.model.sections();
        div()
            .id("rail")
            .w(px(RAIL))
            .h_full()
            // The raised cell's shadow falls on its neighbours, never over the list.
            .overflow_hidden()
            .flex()
            .flex_col()
            // Calendar only while a calendar feed is set.
            .children(SECTIONS.iter().filter(|(s, ..)| sections.contains(s)).enumerate().map(
                |(i, (section, icon, label))| {
                    let (section, label) = (*section, *label);
                    let active = open == Some(section);
                    let accent = active.then_some(theme::PRIMARY);
                    let state = ui::State::focus(self.model.rail_focus == Some(RailFocus::Left(section)));
                    ui::rail_cell(("rail", i), ui::Side::Left, accent, state)
                        .child(svg().path(*icon).size(px(20.)).text_color(rgb(if active {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        })))
                        .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(label.into()))))
                        .children((section == Section::Calendar).then(|| self.calendar_rail_mark()).flatten())
                        .children(self.model.section_marked(section).then(|| self.needs_you_dot()))
                        .child(self.controls.mark(rail_id(label)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.dispatch(Action::ToggleSection { section }, this.input, window, cx);
                        }))
                },
            ))
            .child(div().flex_1().border_r_1().border_color(rgb(theme::BORDER)))
    }

    /// The Tasks cell's dot while a task needs the user: at the icon's corner, as a badge's state dot.
    fn needs_you_dot(&self) -> Div {
        // A 7-point dot centred on the 20-point icon's top right corner.
        let (top, right) = ((RAIL_CELL - 20.) / 2. - 3.5, (RAIL - 20.) / 2. - 3.5);
        let dot = div().absolute().top(px(top)).right(px(right)).size(px(7.)).rounded_full();
        let dot = dot.bg(rgb(theme::task(Group::NeedsYou))).border_1().border_color(rgb(theme::BACKGROUND));
        dot.child(mark(&self.marks, Mark::TasksDot))
    }

    /// The strip over the rail and the section list: the traffic lights, and room to drag the
    /// window by.
    fn render_lights(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // No line runs up through it beside the window's buttons.
        let strip = div().relative().h(px(TOP_BAR)).flex_none().border_b_1().border_color(rgb(theme::BORDER));
        strip.child(mark(&self.marks, Mark::Strip)).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, e: &MouseDownEvent, window, cx| {
                this.dispatch(Action::Leave, this.input, window, cx);
                if e.click_count == 2 && !this.injected {
                    window.titlebar_double_click();
                }
            }),
        )
    }

    /// The bar's header, the same for every section: its name, + where the section can add
    /// something, and ← to collapse the bar into the rail.
    fn bar_header(&self, section: Section, cx: &mut Context<Self>) -> Stateful<Div> {
        let title = SECTIONS.iter().find(|(s, ..)| *s == section).map_or("", |(_, _, label)| label);
        // What + adds, and its sneak id. Apps and Marks have nothing to add from here.
        let add = match section {
            Section::Tasks => Some(("New task (⌘N)", "bar:add")),
            Section::Repos => Some(("Add a repository…", "bar:add")),
            Section::Workbook => Some(("New workspace", "bar:add")),
            _ => None,
        };
        let header = ui::panel_header(title)
            .child(mark(&self.marks, Mark::BarHeader))
            // How many tasks wait for the user.
            .children(Some(self.model.tasks.needs_you()).filter(|n| section == Section::Tasks && *n > 0).map(|n| {
                let badge = div().ml_2().h(px(18.)).min_w(px(18.)).px_1().pt(px(1.)).flex_none().flex();
                let badge = badge.items_center().justify_center().rounded_full().bg(rgb(theme::SELECTED));
                badge.line_height(px(16.)).text_color(rgb(theme::task(Group::NeedsYou))).child(n.to_string())
            }))
            .children((section == Section::Tasks).then(|| self.tasks_view_switch(cx)))
            .children((section == Section::Calendar).then(|| self.calendar_day_switch(cx)))
            .child(div().flex_1())
            .children(add.map(|(tip, mark)| {
                ui::button("bar-add", ui::Button::Ghost(ui::Face::Icon("icons/plus.svg")), ui::State::Rest)
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                    .child(self.controls.mark(mark))
                    .on_click(cx.listener(move |this, _, window, cx| this.bar_add(section, window, cx)))
            }))
            .child(
                self.slide_button("bar:close", (slide::LEFT, slide::SIDEBAR_KEY), "Collapse into the rail").on_click(
                    cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::CloseSidebar, this.input, window, cx);
                    }),
                ),
            );
        header.id("bar-head")
    }

    /// The header's +: a new task in Tasks, a repository to list in Repos, a new workspace in Workspaces.
    pub(crate) fn bar_add(&mut self, section: Section, window: &Window, cx: &mut Context<Self>) {
        match section {
            Section::Tasks => self.new_task(window, cx),
            Section::Repos => self.pick_repo(window, cx),
            Section::Workbook => self.new_workspace(window, cx),
            _ => {}
        }
    }

    /// The Repos section's +: the system's folder picker. Only for the user: a client opens no
    /// panel over their work. Not in a test run: nothing could answer the panel.
    fn pick_repo(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.input != Origin::User {
            self.error = Some(ottospace_core::apps::USERS_ALONE.into());
            return;
        }
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            return;
        }
        let options =
            gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Add".into()) };
        let chosen = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Some(path) = paths.first() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.add_repo(path, Origin::User, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// "Add Folder to Repos": the focused terminal's folder, else the folder the tab's task works
    /// in; its repository joins the list.
    fn add_focused_folder(&mut self, window: &Window, cx: &mut Context<Self>) {
        let cwd = self.model.focused.and_then(|id| self.terminals.get(&id)).and_then(|t| t.cwd());
        let task = || self.model.tab_task().and_then(|id| self.model.task_worktree(id)).map(PathBuf::from);
        match cwd.map(PathBuf::from).or_else(task) {
            Some(dir) => self.add_repo(&dir, self.input, window, cx),
            None => self.error = Some("No folder here: focus a terminal, or a task's tab".into()),
        }
        cx.notify();
    }

    /// A folder for the Repos list: resolved first (links, `..`), only a directory, then the
    /// repository it is in (`/code/app/src` → `/code/app`; a worktree's → its repository's). The
    /// core decides what to do with what is found there.
    fn add_repo(&mut self, path: &Path, origin: Origin, window: &Window, cx: &mut Context<Self>) {
        let dir =
            path.canonicalize().ok().filter(|dir| dir.is_dir()).map(|dir| repos::repository_of(&dir).unwrap_or(dir));
        let Some(dir) = dir.as_deref().and_then(Path::to_str) else {
            self.error = Some(format!("Not a directory: {}", path.display()));
            return;
        };
        let found = repos::found_in(Path::new(dir));
        self.change_repos(Action::RepoAdd { dir: dir.into(), found }, origin, window, cx);
    }

    /// Worktrees git made, or did not: the list is read again; a made one opens, a failure is said.
    fn take_worktrees(&mut self, window: &Window, cx: &mut Context<Self>) {
        let removed: Vec<worktree::Removed> = self.removals.1.try_iter().collect();
        for worktree::Removed { task, path, kept } in removed {
            self.dispatch(Action::WorktreeRemoved { task, path, kept }, Origin::System, window, cx);
            cx.notify();
        }
        let made: Vec<worktree::Made> = self.worktrees.1.try_iter().collect();
        for worktree::Made { repo, asked, fresh, result } in made {
            let repos = repos::scan();
            match result {
                // Made from a repository listed only as a worktree of another: not in the list to open.
                // A bug report's repository need not be listed (`developer.ottospace_repo`): made
                // where it was asked, in `worktrees_dir`.
                Ok(path)
                    if !(self.model.bug_worktree(&asked) && path == asked)
                        && !repos.iter().any(|r| r.worktrees.iter().any(|w| w.path == path)) =>
                {
                    self.error = Some(format!("Made {path}: its repository is not listed"));
                    self.dispatch(Action::WorktreeFailed { asked, fresh, repos }, Origin::System, window, cx);
                }
                Ok(path) => {
                    let made = Action::WorktreeAdded { repo, path, asked, fresh, repos };
                    drop(self.dispatch(made, Origin::System, window, cx))
                }
                Err(why) => {
                    self.error = Some(why);
                    self.dispatch(Action::WorktreeFailed { asked, fresh, repos }, Origin::System, window, cx);
                }
            }
            cx.notify();
        }
    }

    /// Pull requests: asked for while the Repos section is open (`prs::due`), and what gh said, to
    /// the core. `"repos_prs": "off"`: none read, and none shown.
    fn tick_prs(&mut self, window: &Window, cx: &mut Context<Self>) {
        let setting = config::live().repos_prs.unwrap_or_default();
        if setting == ottospace_core::prs::Setting::Off {
            for repo in self.model.prs.iter().map(|(repo, ..)| repo.clone()).collect::<Vec<_>>() {
                let none = Action::SetPrs { repo, prs: Vec::new(), branches: Vec::new() };
                self.dispatch(none, Origin::System, window, cx);
            }
            self.pr_note = None;
            return;
        }
        let checkouts = |repo: &Repo| repo.checkouts().into_iter().map(|c| (c.path, c.branch)).collect();
        let open = self.model.sidebar == Some(Section::Repos);
        let repos = self.model.repos.iter().filter(|_| open).map(|repo| (repo.path.clone(), checkouts(repo))).collect();
        self.prs.ask(open, repos, now_ms());
        let reads: Vec<prs::Read> = self.prs.reads.try_iter().collect();
        for prs::Read { repo, result } in reads {
            match result {
                Ok((prs, branches)) => {
                    self.pr_note = None;
                    self.dispatch(Action::SetPrs { repo, prs, branches }, Origin::System, window, cx);
                }
                // No marks from a read that failed; quiet unless asked for: a line only for an explicit `on`.
                Err(why) => {
                    self.pr_note = (setting == ottospace_core::prs::Setting::On).then_some(why);
                    let none = Action::SetPrs { repo, prs: Vec::new(), branches: Vec::new() };
                    self.dispatch(none, Origin::System, window, cx);
                }
            }
            cx.notify();
        }
    }

    /// While the Repos section is open: each checkout's `HEAD` log, by one stat, so a commit pulses its dot.
    fn read_heads(&mut self, cx: &mut Context<Self>) {
        if self.model.sidebar != Some(Section::Repos) {
            return;
        }
        let checkouts = self.model.repos.iter().flat_map(Repo::checkouts);
        let heads = checkouts.filter_map(|c| Some((c.path.clone(), repos::head_log(Path::new(&c.path))?))).collect();
        let pulses = self.model.head_pulses();
        // Not `dispatch`, which draws again: only a new pulse does, once a second at most.
        self.model.update(Action::SetHeads { heads, at: now_ms(), calm: dial::calm() }.sent_by(Origin::System));
        if self.model.head_pulses() != pulses {
            cx.notify();
        }
    }

    /// The user's + on a repository's name: the field for a new worktree's branch, with the
    /// worktrees folder as the settings say now.
    pub(crate) fn add_worktree(&mut self, repo: &str, window: &Window, cx: &mut Context<Self>) {
        self.model.update(Action::SetRepoDirs { dirs: config::repo_dirs() }.sent_by(Origin::System));
        self.dispatch(Action::WorktreeEdit { repo: repo.into() }, self.input, window, cx);
    }

    /// "Remove" on a repository's name in the Repos section.
    pub(crate) fn remove_repo(&mut self, path: &str, window: &Window, cx: &mut Context<Self>) {
        self.change_repos(Action::RepoRemove { path: path.into() }, self.input, window, cx);
    }

    /// A directory added to the Repos list or taken off it. The list is read before, so what is
    /// listed already is known, and after, from the directories as they then are. If the
    /// settings were not written the directories are as before: the next try writes again.
    fn change_repos(&mut self, action: Action, origin: Origin, window: &Window, cx: &mut Context<Self>) {
        // A refused one (a client's) leaves the toast and the list alone.
        if self.model.refusal(&action, origin).is_some() {
            self.dispatch(action, origin, window, cx);
            return;
        }
        config::reread_repos();
        self.model.update(Action::SetRepoDirs { dirs: config::repo_dirs() }.sent_by(Origin::System));
        let before = self.model.repo_dirs.clone();
        self.error = None; // what the last try said is not about this one
        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        self.dispatch(action, origin, window, cx);
        if self.error.as_ref().is_some_and(|error| error.starts_with(Self::NOT_WRITTEN)) {
            self.model.update(Action::SetRepoDirs { dirs: before }.sent_by(Origin::System));
        }
        config::set_repo_roots(&self.model.repo_dirs);
        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
    }

    /// Secondary bar: the open section's list. Closable.
    fn render_bar(&self, section: Section, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.bar_header(section, cx);
        let body = match section {
            Section::Apps => self.render_apps(cx).into_any_element(),
            Section::Repos => self.render_repos(cx).into_any_element(),
            Section::Tasks => self.render_tasks(cx).into_any_element(),
            Section::Marks => self.render_marks(cx).into_any_element(),
            Section::Calendar => self.render_calendar(cx).into_any_element(),
            Section::Workbook => self.render_workbooks(cx).into_any_element(),
        };
        div()
            .id("bar")
            .w(px(SECTION))
            // Min and max too: the layout first sizes a flex item by its content, ignoring `w`. Text
            // measured then with no width is never cut, and `truncate` shows no ellipsis.
            .min_w(px(SECTION))
            .max_w(px(SECTION))
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
                    this.dispatch(Action::Leave, this.input, window, cx);
                }),
            )
            .child(header)
            .child(body)
    }

    /// A pane's header text: a terminal's directory and program, an app's window title, nothing
    /// for an empty pane, `Slot · Zoom` for a slot that waits.
    fn pane_title(&self, pane: &Pane) -> String {
        match &pane.kind {
            Kind::Empty => String::new(),
            Kind::Slot { name, .. } => format!("Slot · {name}"),
            Kind::Terminal => self.titles.get(&pane.id).cloned().unwrap_or_default(),
            // A window with no title says whose it is.
            Kind::App { title, name, .. } if title.is_empty() => name.clone(),
            Kind::App { title, .. } => title.clone(),
            Kind::File { path } => viewer::name_and_dir(path).0,
            Kind::Decision => ottospace_core::model::decisions::NAME.into(),
        }
    }

    /// Reads each terminal's directory and program for its header; redraws if one changed.
    /// Only the drawn panes' are read again; others keep their last, so a tab comes back titled.
    fn read_titles(&mut self, cx: &mut Context<Self>) {
        let home = std::env::var("HOME").ok();
        let drawn = self.model.shown_panes();
        let titles: HashMap<PaneId, String> = self
            .terminals
            .iter()
            .filter_map(|(id, t)| match drawn.contains(id) {
                true => {
                    Some((*id, terminal_title(t.cwd().as_deref(), t.foreground_command().as_deref(), home.as_deref())))
                }
                false => Some((*id, self.titles.get(id)?.clone())),
            })
            .collect();
        if titles != self.titles {
            self.titles = titles;
            cx.notify();
        }
    }

    /// Lists the apps that can be hosted now, and reads the icon of each one not seen before.
    /// The core is told which bundles run: it lists the pinned and suggested apps that do not.
    fn refresh_apps(&mut self) {
        let before: Vec<String> = self.apps.iter().map(|app| app.name.clone()).collect();
        self.apps = running_apps(self.hosting, self.host_only);
        let running: Vec<String> = self.apps.iter().map(|app| app.name.clone()).collect();
        self.model.update(Action::SetApps { names: running.clone() }.sent_by(Origin::System));
        self.tell_installed();
        // An app that quit loses its icon: a later app of the same name may have another.
        self.icons.retain(|name, _| running.contains(name) || !before.contains(name));
        for app in &self.apps {
            if !self.icons.contains_key(&app.name)
                && let Some(png) = icon::png(app.pid, ICON_PIXELS)
            {
                let image = Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png));
                self.icons.insert(app.name.clone(), image);
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

    /// A click on a heading of the Tasks section folds or unfolds it.
    pub(crate) fn click_heading(&mut self, heading: Heading, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::ToggleGroup { heading }, self.input, window, cx);
        cx.notify();
    }

    /// A click on a button of a task's row, by the button's name: `task-snooze`, `task-dismiss`
    /// (Done) or `task-clear` (a done task's, as Done). Each offers to be undone.
    pub(crate) fn click_row_button(&mut self, button: &str, id: TaskId, window: &Window, cx: &mut Context<Self>) {
        let action = match button {
            "task-snooze" => Action::RowSnooze { id, until: now_ms().saturating_add(SNOOZE_MS) },
            "task-dismiss" | "task-clear" => Action::RowDismiss { id },
            _ => return,
        };
        self.dispatch(action, self.input, window, cx);
        cx.notify();
    }

    /// The line a section shows after a move that can be taken back: "Dismissed · Undo". The
    /// caller says what a click on it does.
    pub(crate) fn undo_line(&self, id: &'static str, what: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .relative()
            .child(self.controls.mark(id))
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_1p5()
            .rounded_md()
            .bg(rgb(theme::SELECTED))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .cursor_pointer()
            .child(what)
            .child("·")
            .child(div().text_color(rgb(theme::PRIMARY)).child("Undo"))
    }

    /// A click on the notice ("Dismissed · Undo"): what `u` does in list mode.
    pub(crate) fn click_undo(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::TaskUndo, self.input, window, cx);
        cx.notify();
    }

    /// A click on a session's row or on one of its buttons, by the control's name: `session` (its
    /// conversation in the drawer), `session-adopt` (Track: a task is made of it) or `session-hide`.
    pub(crate) fn click_session_control(
        &mut self,
        control: &str,
        runner: String,
        id: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let action = match control {
            // A client's click on the row is the one it always was: the pane, else the row.
            "session" if self.input != Origin::User => Action::SessionClick { runner, id },
            "session" => Action::SessionOpen { runner, id },
            "session-adopt" => Action::SessionAdopt { runner, id },
            "session-hide" => Action::SessionHide { runner, id },
            _ => return,
        };
        self.dispatch(action, self.input, window, cx);
        cx.notify();
    }

    /// A heading of the Tasks section, the Sessions one too: a fold mark, its name and how many
    /// it holds. A click folds it. `selected`: list mode is on it, and ↵ or space folds it.
    fn heading_row(
        &self,
        heading: Heading,
        count: usize,
        collapsed: bool,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let color = match heading {
            Heading::NeedsYou => theme::task(Group::NeedsYou),
            _ => theme::MUTED,
        };
        // Each part noted where it is drawn, for a test run (`heading_rects`).
        let part = |at: u8| mark(&self.marks, Mark::Heading(heading, at));
        let fold = ui::fold_mark(!collapsed).size_full().text_color(rgb(theme::MUTED));
        div()
            .id(("task-heading", heading as usize))
            .px_2()
            .pt_2()
            .flex()
            .items_center()
            .gap_1p5()
            .text_xs()
            .text_color(rgb(color))
            .cursor_pointer()
            .hover(|s| s.text_color(rgb(theme::TEXT)))
            .when(selected, |d| d.pb_0p5().rounded_md().bg(rgb(theme::SELECTED)))
            .child(div().flex_none().size(px(8.)).relative().child(fold).child(part(0)))
            .child(div().relative().child(heading.title()).child(part(1)))
            .child(div().relative().text_color(rgb(theme::MUTED)).child(format!("· {count}")).child(part(2)))
            .relative()
            .child(self.controls.mark(format!("task-heading:{}", heading_name(heading))))
            .on_click(cx.listener(move |this, _, window, cx| this.click_heading(heading, window, cx)))
    }

    /// The Elsewhere group: agent sessions found for the listed repositories that are no
    /// task's, those in its range. Empty when none is listed. `listed`: how many rows of the list
    /// come before; the heading and each session are rows, as list mode walks them.
    fn render_sessions(&self, mut listed: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let bar = self.model.sessions_bar();
        if bar.total == 0 {
            return Vec::new();
        }
        let selected = self.model.list == Some(listed);
        // Its count is of those in range (7 days unless the settings say otherwise).
        let heading = self.heading_row(Heading::Sessions, bar.count, bar.collapsed, selected, cx);
        let total = SharedString::from(format!("{} in all", bar.total));
        let heading = heading.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(total.clone()))));
        let mut rows = vec![heading.into_any_element()];
        listed += 1;
        let mut group = Vec::new();
        for row in &bar.rows {
            group.push(self.session_row(row, self.model.list == Some(listed), cx));
            listed += 1;
        }
        rows.extend(well(group, 4.));
        rows
    }

    /// A session's row, one line: a dot (lit while it is active), its title (`Row::title`) and its
    /// runner. The rest is in its hover card. Hover (or list mode's selection) shows Track and Hide.
    fn session_row(&self, row: &ottospace_core::sessions::Row, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let key = format!("{}:{}", row.runner, row.id);
        let button = |name: &'static str, label: &'static str, tip: &'static str| {
            let (runner, id) = (row.runner.clone(), row.id.clone());
            let click = move |this: &mut Self, window: &Window, cx: &mut Context<Self>| {
                this.click_session_control(name, runner.clone(), id.clone(), window, cx)
            };
            self.word(format!("{name}:{key}"), (label, tip), click, cx)
        };
        // A click on the row opens it: no Open word.
        let actions = words::words("session-row", selected)
            .top(px(3.5))
            .child(button("session-adopt", "Track", "Track as a task"))
            .child(button("session-hide", "Hide", "Hide"));
        let color = if row.active { theme::task(Group::Running) } else { theme::BORDER_STRONG };
        let (runner, id, card) = (row.runner.clone(), row.id.clone(), row.clone());
        sidebar_row(ElementId::Name(format!("session:{key}").into()), selected)
            .group("session-row")
            .child(home::dot(color))
            // The title may be text a model wrote: drawn as it is, on one line.
            .child(cut_text(row.title().to_string()).flex_1())
            .child(row_meta(row.runner.clone()))
            .child(actions)
            .child(self.controls.mark(format!("session:{key}")))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| progress::SessionCard(card.clone()))))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.click_session_control("session", runner.clone(), id.clone(), window, cx);
            }))
            .into_any_element()
    }

    /// The Tasks section: what needs the user, the other tasks with tabs open, then the rest of
    /// the inbox under its headings. A click on a row shows the task's tabs; a click on a heading
    /// folds it. After a snooze, hold or dismiss by mouse, a line at the top offers to undo it.
    fn render_tasks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut listed = 0; // rows of the list so far: a heading is one, as list mode walks them
        let by_state = self.model.tasks_view == ottospace_core::model::TasksView::State;
        if !by_state {
            rows = self.render_projects(&self.model.project_groups(now_ms()), &mut listed, cx);
        }
        for section in self.model.task_sections(now_ms()).into_iter().filter(|_| by_state) {
            let selected = self.model.list == Some(listed);
            rows.push(
                self.heading_row(section.heading, section.count, section.collapsed, selected, cx).into_any_element(),
            );
            listed += 1;
            let mut group = Vec::new();
            for row in &section.rows {
                if let Some(task) = self.model.tasks.get(row.id) {
                    group.push(self.task_row(task, row, self.model.list == Some(listed), cx));
                }
                listed += 1;
            }
            rows.extend(well(group, 4.));
        }
        // Nothing listed: say so, with the button that files a task.
        let empty = rows.is_empty().then(|| {
            let title = if self.model.tasks.list.is_empty() { "No tasks yet" } else { "Nothing in the inbox" };
            let plus = svg().path("icons/plus.svg").size(px(14.)).text_color(rgb(theme::TEXT));
            let new = list_button("task-new", false)
                .py_1p5()
                .child(plus)
                .child(div().flex_1().child("New task"))
                .child(div().text_xs().text_color(rgb(theme::MUTED)).child("⌘N"))
                .relative()
                .child(self.controls.mark("task-new"))
                .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)));
            div().flex().flex_col().gap_2().child(div().px_2().pt_1().text_sm().child(title)).child(new)
        });
        let notice = self.task_offer().map(|notice| {
            self.undo_line("task-undo", notice.moved.title())
                .on_click(cx.listener(|this, _, window, cx| this.click_undo(window, cx)))
        });
        let sessions = self.render_sessions(listed, cx);
        let list = div().id("tasks").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.list_scroll).children(notice).children(empty).children(rows);
        let list = self.bars.tracked("tasks", list.children(sessions), &self.list_scroll, theme::RAISED);
        // The footer stays under the list as it scrolls.
        let footer = (!self.model.tasks.list.is_empty()).then(|| self.render_today());
        div().flex_1().min_h_0().flex().flex_col().child(list).children(footer)
    }

    /// A task's row: its badge with the state dot, title (`P1` before it when urgent) and one fact
    /// at the right; while its agent works, a muted line of what it does. The rest is in its hover
    /// card. Hover (or list mode's selection) shows Snooze and Done; on a done task, Clear.
    fn task_row(&self, task: &Task, row: &TaskRow, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        // Words, as on a session's row: the icons said too little.
        let button = |name: &'static str, label: &'static str, tip: &'static str| {
            let click = move |this: &mut Self, window: &Window, cx: &mut Context<Self>| {
                this.click_row_button(name, id, window, cx)
            };
            self.word(format!("{name}:{id}"), (label, tip), click, cx)
        };
        // Over the end of the title line, on the row's own colour: hidden, they take no room from it.
        // No Start or Show: a click on the row opens its conversation. Hold is `h` in list mode.
        let actions =
            words::words("task-row", selected).top(px(3.5)).children(row.group.row_actions().iter().map(|action| {
                match action {
                    RowAction::Snooze => button("task-snooze", "Snooze", "1 hour · S"),
                    RowAction::Dismiss => button("task-dismiss", "Done", "Out of the inbox · X"),
                    RowAction::Clear => button("task-clear", "Clear", "Off the list · X"),
                }
            }));
        // The task on show: a bar at the row's edge, and its title in bold.
        let bar = div().absolute().left(px(-1.)).top(px(5.)).bottom(px(5.)).w(px(3.)).rounded_full();
        let muted = || div().text_xs().text_color(rgb(theme::MUTED));
        sidebar_row(("task", id as usize), selected)
            .group("task-row")
            // A second line while its agent works: the first stays where a one-line row has it.
            .items_start()
            .py(px(3.))
            .children(row.current.then(|| bar.bg(rgb(theme::PRIMARY))))
            .child(div().h(px(20.)).flex().items_center().children(self.task_badge(id, 18.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(row.current, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .children(task_title_cells(task))
                            .children(row.worktree.clone().map(|name| muted().flex_none().child(name)))
                            .children(row.fact.clone().map(|fact| muted().flex_none().child(fact))),
                    )
                    .children(row.live.clone().map(|live| muted().h(px(18.)).truncate().child(live)))
                    .children(self.task_pending(id, row, cx)),
            )
            .child(actions)
            .child(self.controls.mark(format!("task:{id}")))
            .when_some(progress::card_tip(self, id), |row, tip| row.tooltip(tip))
            .on_click(open_task(id, cx))
            .into_any_element()
    }

    /// A task's waiting cards, a line each under its row (`TaskRow::pending`): a question on
    /// show and how far in, or what a permission would run; then `+N more`. The words are the
    /// agent's: drawn here, never in the state. A click opens its conversation on the card with
    /// the keyboard in it (`TaskCard`, the user's alone); nothing is answered from the list.
    fn task_pending(&self, id: TaskId, row: &TaskRow, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let cards = self.model.chat(&ottospace_core::talk::Subject::Task(id)).map(|chat| &chat.transcript.cards);
        let mut lines = Vec::new();
        for (at, (pending, card)) in row.pending.iter().zip(cards.into_iter().flatten()).enumerate() {
            let (glyph, words, count) = match (pending, card.ask.as_ref()) {
                (Pending::Question { shown, of }, Some(form)) => {
                    // The question itself, as the card shows it in bold.
                    let text =
                        form.questions.get(shown.saturating_sub(1)).map(|q| q.question.text()).unwrap_or_default();
                    let question = match conversation::question_blocks(text).1 {
                        Some(asked) => markdown::plain(std::slice::from_ref(&asked)),
                        None => text.to_string(),
                    };
                    let count = (*of > 1).then(|| format!("{shown} of {of}"));
                    ("?", ottospace_core::transcript::clean(&question), count)
                }
                _ => {
                    // A command as it would run (`command: git push`); any other request, its tool
                    // and label (`Edit · auth.rs`), whose payload is JSON or a change's kind.
                    let first = card.detail.text().lines().next().unwrap_or_default();
                    let words = match first.strip_prefix("command: ") {
                        Some(command) => format!("Wants to run: {}", ottospace_core::transcript::clean(command)),
                        None => format!("Wants to use {} · {}", card.tool, card.label),
                    };
                    ("⚠", words, None)
                }
            };
            // One line; spaces inside kept as they are: `rm 'a  b'` names that file.
            let words = words.replace(['\n', '\r', '\t'], " ");
            lines.push(self.pending_line(id, at, Some(card.ticket), glyph, words, count, cx));
        }
        if row.more > 0 {
            lines.push(self.pending_line(id, PENDING_LINES, None, "", format!("+{} more", row.more), None, cx));
        }
        lines
    }

    #[allow(clippy::too_many_arguments)] // a line's place, its card and its three parts.
    fn pending_line(
        &self,
        id: TaskId,
        at: usize,
        ticket: Option<u64>,
        glyph: &'static str,
        words: String,
        count: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = || div().text_xs().text_color(rgb(theme::MUTED));
        div()
            .id(ElementId::Name(format!("task-pending:{id}:{at}").into()))
            .relative()
            .h(px(18.))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .children((!glyph.is_empty()).then(|| div().flex_none().text_color(rgb(theme::SNEAK)).child(glyph)))
            .child(div().flex_1().min_w_0().truncate().text_color(rgb(theme::TEXT)).child(words))
            .children(count.map(|count| muted().flex_none().child(count)))
            .child(self.controls.mark(format!("task-pending:{id}:{at}")))
            .on_click(cx.listener(move |this, _, window, cx| {
                // The card, first and with the keyboard: the user's alone (the core refuses a client's).
                this.dispatch(Action::TaskCard { id, ticket }, this.input, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .into_any_element()
    }

    /// Repositories, each with its worktrees under it. A click opens a terminal there: it fills
    /// the focused pane if that is empty, else splits it (right, or down with shift).
    fn render_repos(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // One checkout, on one line: its dot (what happens there), its branch, cut short before
        // the marks at its end; the path is in the tooltip.
        let (now, pulsing) = (now_ms(), Cell::new(false));
        let row = |index: usize, at: usize, checkout: Checkout, cx: &mut Context<Self>| {
            let selected = self.model.list == Some(at);
            // The task working there: its badge; a click selects it, and a word opens a terminal anyway.
            let task = self.model.repo_task(&checkout.path);
            let terminal = task.map(|_| {
                let path = checkout.path.clone();
                let click = move |this: &mut Self, window: &Window, cx: &mut Context<Self>| {
                    this.dispatch(Action::OpenRepo { path: path.clone(), axis: Axis::Right }, this.input, window, cx);
                };
                let tip = ("Terminal", "A terminal here, in Workspace");
                words::words("repo-row", selected).child(self.word(
                    format!("repo-terminal:{}", checkout.path),
                    tip,
                    click,
                    cx,
                ))
            });
            // Its state is the dot's to say: the badge has none, and is the dot's task if it has one.
            let dot = self.model.checkout_dot(&checkout.path, now);
            let badge = dot.task.or(task).and_then(|id| self.task_badge_plain(id, 16.));
            pulsing.set(pulsing.get() || dot.pulse.is_some());
            // At its end, in this order: the pull request's mark, the task's badge. The words sit
            // left of them, so the pull request stays there to click.
            let badge_room = if badge.is_some() { 24. } else { 0. };
            let pr = self.pr_marker(&checkout.path, cx);
            let pr_room = pr.as_ref().map_or(0., |(_, width)| width + 8.);
            let room = 8. + badge_room + pr_room;
            // The repository's own folder says so, where a pull request would be.
            let default = (checkout.default && pr.is_none()).then(|| row_meta("default"));
            let tip = match checkout.default {
                true => format!("{} · the repository's own folder", checkout.path),
                false => checkout.path.clone(),
            };
            sidebar_row(("repo", index), selected)
                .group("repo-row")
                .child(self.activity_dot(dot))
                .child(cut_text(checkout.branch).flex_1())
                .children(default)
                .children(pr.map(|(marker, _)| marker.flex_none()))
                .children(badge.map(|badge| badge.flex_none()))
                .children(terminal.map(|words| words.top(px(3.5)).right(px(room - 2.))))
                .child(mark(&self.marks, Mark::Checkout(index)))
                .child(self.controls.mark(format!("repo:{}", checkout.path)))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into()))))
                // A right click: the row's menu at the pointer (a linked worktree's: Delete worktree…).
                .on_mouse_down(MouseButton::Right, {
                    let row = ottospace_core::list::ListRow::Repo(checkout.path.clone());
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.row_menu_at = Some(e.position);
                        // Where each shell is now: a worktree a terminal is in is not deleted.
                        this.tell_terminals();
                        this.open_row_menu(row.clone().into(), false, window, cx);
                    })
                })
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                    this.dispatch(Action::RepoClick { path: checkout.path.clone(), axis }, this.input, window, cx);
                }))
        };
        // Each repository's name (a list row of its own), then its checkouts unless folded: its
        // own folder first. A group's folder over its repositories, drawn while it is unfolded.
        // `index` counts checkouts, drawn or not; `at` the list's rows.
        let mut items: Vec<AnyElement> = Vec::new();
        let (mut index, mut at) = (0, 0);
        let mut group: Option<String> = None;
        for (n, repo) in self.model.repos.iter().enumerate() {
            let checkouts = repo.checkouts();
            let of = self.model.repo_dirs.group_of(&repo.path).cloned();
            if of.as_ref().map(|dir| &dir.path) != group.as_ref() {
                group = of.as_ref().map(|dir| dir.path.clone());
                if let Some(dir) = &of {
                    items.push(self.repo_group(at, dir, cx).into_any_element());
                    at += 1;
                }
            }
            if group.as_ref().is_some_and(|path| !self.model.repos_open.contains(path)) {
                index += checkouts.len();
                continue;
            }
            let folded = self.model.repo_folded(repo);
            items.push(self.repo_heading(n, at, repo, folded, checkouts.len(), cx).into_any_element());
            at += 1;
            let typing =
                matches!(&self.model.mark_input, Some(MarkInput::Worktree { repo: of, .. }) if *of == repo.path);
            if typing {
                let field = self.render_field("Branch (new ones from HEAD)", cx);
                let line = div().flex_none().pl(px(26.)).pr_2().py_0p5().rounded_md().border_1();
                items.push(line.border_color(rgb(theme::MARK)).text_size(px(13.)).child(field).into_any_element());
            }
            let mut group = Vec::new();
            for checkout in checkouts {
                if !folded {
                    group.push(row(index, at, checkout, cx).into_any_element());
                    at += 1;
                }
                index += 1;
            }
            items.extend(well(group, 0.));
        }
        // A pulse plays: the next frame, and none once it is over.
        if pulsing.get() {
            self.repo_pulse_frames.set(self.repo_pulse_frames.get() + 1);
            let next = canvas(|_, _, _| {}, |_, _, window, _| window.request_animation_frame());
            items.push(next.absolute().size_0().into_any_element());
        }
        // Opt-in: none until the user adds one, by the + here or in the header, or by a drop.
        let empty = self.model.repos.is_empty().then(|| match self.model.repo_dirs.fixed {
            true => div()
                .px_2()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child("No repositories in the folders OTTOSPACE_REPOS names.")
                .into_any_element(),
            false => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    list_row("repos-empty-add", false)
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(svg().path("icons/plus.svg").size(px(12.)).text_color(rgb(theme::PRIMARY)))
                        .child("Add a repository")
                        .child(self.controls.mark("repos-empty:add"))
                        .on_click(cx.listener(|this, _, window, cx| this.bar_add(Section::Repos, window, cx))),
                )
                .child(div().px_2().text_xs().text_color(rgb(theme::MUTED)).child("Or drop its folder here."))
                .into_any_element(),
        });
        let list = div()
            .id("repos")
            .flex_1()
            .flex()
            .flex_col()
            .p_2()
            .overflow_y_scroll()
            .drag_over::<gpui::ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
            .on_drop(cx.listener(|this, dropped: &gpui::ExternalPaths, window, cx| {
                for path in dropped.paths() {
                    this.add_repo(path, this.input, window, cx);
                }
                cx.notify();
            }));
        self.bars.tracked(
            "repos",
            list.track_scroll(&self.list_scroll).children(empty).children(items).children(self.pr_note.map(|note| {
                div().px_2().pt_2().text_xs().text_color(rgb(theme::MUTED)).child(format!("Pull requests: {note}"))
            })),
            &self.list_scroll,
            theme::RAISED,
        )
    }

    /// A checkout's dot: grey while idle, the task's colour (or a session's) while at work, the
    /// needs-you colour; a commit's pulse is a ring that widens and fades from it.
    fn activity_dot(&self, dot: ottospace_core::model::repo_dots::Dot) -> Div {
        use ottospace_core::model::repo_dots::Activity;
        let colour = match (dot.activity, dot.task.and_then(|id| self.model.tasks.get(id))) {
            (Activity::Idle, _) => theme::BORDER_STRONG,
            (Activity::NeedsYou, _) => theme::task(Group::NeedsYou),
            (Activity::Working, Some(task)) => theme::identity(task.colour),
            (Activity::Working, None) => theme::task(Group::Running),
        };
        let ring = dot.pulse.map(|e| {
            let ring = if dot.activity == Activity::Idle { theme::TEXT } else { colour };
            div()
                .absolute()
                .top(px(-5. * e))
                .left(px(-5. * e))
                .size(px(8. + 10. * e))
                .rounded_full()
                .border_1()
                .border_color(rgb(ring))
                .opacity(1. - e)
        });
        div().relative().flex_none().size(px(8.)).child(home::dot(colour)).children(ring)
    }

    /// The open pull request of the checkout at `path`, as one small mark: the pull request icon
    /// and `#131`, green while open and muted for a draft, then a green tick (ready to merge), an
    /// amber dot (waiting) or a red dot (failing).
    /// Hover: its card; the user's click opens it. Its drawn width, for the row to leave room.
    fn pr_marker(&self, path: &str, cx: &mut Context<Self>) -> Option<(Stateful<Div>, f32)> {
        use ottospace_core::prs::Look;
        let pr = self.model.checkout_pr(path)?;
        let (summary, card) = (pr.summary(), pr.card());
        let number = format!("#{}", summary.number);
        let (sign, color) = match summary.look {
            Look::Ready => (Some("✓"), theme::PICK),
            Look::Pending => (Some("●"), theme::SNEAK),
            Look::Failing => (Some("●"), theme::ERROR),
            Look::Draft | Look::Open => (None, theme::MUTED),
        };
        let width = 16. + 7. * number.len() as f32 + if sign.is_some() { 12. } else { 0. };
        let state = if summary.draft { theme::MUTED } else { theme::PICK };
        let icon = svg().path("icons/pull-request.svg").size(px(12.)).flex_none().text_color(rgb(state));
        let path = path.to_string();
        let marker = div()
            .id(ElementId::Name(format!("repo-pr:{path}").into()))
            .flex()
            .items_center()
            .gap_1()
            .px_0p5()
            .rounded_sm()
            .text_xs()
            .line_height(px(17.))
            .text_color(rgb(state))
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
            .child(icon)
            .child(number)
            .children(sign.map(|sign| div().text_size(px(9.)).text_color(rgb(color)).child(sign)))
            .child(self.controls.mark(format!("repo-pr:{path}")))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| progress::PrCard(card.clone(), color))))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation(); // not a click on the row
                this.dispatch(Action::PrOpen { path: path.clone() }, this.input, window, cx);
            }));
        Some((marker, width))
    }

    /// A group's folder (an entry of the old settings that holds repositories): a chevron that
    /// folds them, its name as the settings write it, how many; under the pointer, the trash
    /// (stop listing all of them). `at`: its place among the list's rows.
    fn repo_group(&self, at: usize, dir: &ottospace_core::repos::RepoDir, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.model.list == Some(at);
        let open = self.model.repos_open.contains(&dir.path);
        let count = self.model.group_repos(&dir.path).len();
        let path = dir.path.clone();
        let remove = {
            let path = path.clone();
            let click =
                move |this: &mut Self, window: &Window, cx: &mut Context<Self>| this.remove_repo(&path, window, cx);
            let tip = ("icons/trash.svg", "Stop listing this folder's repositories (nothing on disk is touched)");
            self.icon_word(format!("repo-remove:{}", dir.path), tip, click, cx)
        };
        let tip =
            format!("{} · a folder of repositories from your old settings: remove it, add the ones you use", dir.path);
        let name = format!("{}/", dir.text.trim_end_matches('/'));
        ui::section_heading(("repo-group", at), open, name, count, ui::State::focus(selected))
            .group("repo-name")
            .when(at > 0, |d| d.mt_1())
            .mb_1()
            .child(words::words("repo-name", selected).top_0().bottom_0().items_center().child(remove))
            .child(self.controls.mark(format!("repo-fold:{path}")))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into()))))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.dispatch(Action::RepoFold { path: path.clone(), fold: None }, this.input, window, cx);
            }))
    }

    /// A repository's name: a chevron that folds its rows (a click anywhere on the line does),
    /// the name and the number of its checkouts; under the pointer, + (a worktree) and
    /// the trash (stop listing it). `at`: its place among the list's rows.
    fn repo_heading(
        &self,
        n: usize,
        at: usize,
        repo: &Repo,
        folded: bool,
        count: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.model.list == Some(at);
        let path = repo.path.clone();
        let add = {
            let path = path.clone();
            let click =
                move |this: &mut Self, window: &Window, cx: &mut Context<Self>| this.add_worktree(&path, window, cx);
            self.icon_word(format!("repo-add:{path}", path = repo.path), ("icons/plus.svg", "New worktree…"), click, cx)
        };
        // Not while `OTTOSPACE_REPOS` names the directories: there is no setting to take it from.
        let remove = (!self.model.repo_dirs.fixed).then(|| {
            let path = path.clone();
            let click =
                move |this: &mut Self, window: &Window, cx: &mut Context<Self>| this.remove_repo(&path, window, cx);
            let tip = ("icons/trash.svg", "Stop listing this repository (nothing on disk is touched)");
            self.icon_word(format!("repo-remove:{}", repo.path), tip, click, cx)
        });
        ui::section_heading(("repo-head", n), !folded, repo.name.clone(), count, ui::State::focus(selected))
            .group("repo-name")
            .when(at > 0, |d| d.mt_1())
            .mb_1()
            .child(words::words("repo-name", selected).top_0().bottom_0().items_center().child(add).children(remove))
            .child(mark(&self.marks, Mark::Repo(n)))
            .child(self.controls.mark(format!("repo-fold:{path}")))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.dispatch(Action::RepoFold { path: path.clone(), fold: None }, this.input, window, cx);
            }))
    }

    fn render_pane(&self, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement {
        let id = pane.id;
        let cell = self.cells.get(&id).cloned().unwrap_or_default();
        let frame = self.frames.get(&id).cloned().unwrap_or_default();
        let bar = self.hosted.get(&id).map_or(0.0, |h| h.bar as f32);
        // No kind word: the content says it. An app has its icon, if one was read.
        let title = self.pane_title(pane);
        let icon = match &pane.kind {
            Kind::App { name, .. } => self.icons.get(name).cloned(),
            _ => None,
        };
        // The window kept another size than the pane's: say so. In a narrow header the words
        // give way first: they shrink and are cut with an ellipsis.
        let hint = pane.fit.as_ref().map(|fit| cut_text(fit.label()).text_color(rgb(theme::SNEAK)));
        // And offer to fit the pane to it, unless a fit would move nothing (the hint says why).
        let word = pane.fit.as_ref().filter(|fit| fit.stuck.is_none()).map(|_| {
            div()
                .id(("fit", id))
                .flex_none()
                .cursor_pointer()
                .text_color(rgb(theme::MUTED))
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .child("fit")
                .child(self.controls.mark(format!("fit:{id}")))
                // The press must not reach the header: an unsteady click would start a drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.fit_pane(id, window, cx)))
        });
        // A viewer's Edit: the user's editor on its file, beside it (⌃G).
        let edit = matches!(pane.kind, Kind::File { .. }).then(|| {
            div()
                .id(("edit", id))
                .flex_none()
                .relative()
                .cursor_pointer()
                .text_color(rgb(theme::MUTED))
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .child("Edit")
                .child(self.controls.mark(format!("viewer-edit:{id}")))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(Action::ViewerEdit { pane: Some(id) }, this.input, window, cx))
                }))
        });
        // After a copy from it or a paste into it: "Copied 3 lines" in the header, and one pulse.
        let now = now_ms();
        let clip = self.model.clip.as_ref().filter(|clip| clip.pane == Some(id) && clip.shows(now));
        let glow = clip.map_or(0., |clip| clip.glow(now, self.clip_reduced));
        let clip = clip.map(|clip| div().flex_none().text_color(rgb(theme::PRIMARY)).child(clip.text.clone()));
        // The pane being annotated has its tools in its header.
        let drawn_on = self.model.annotated().filter(|m| m.pane == id && self.shots.contains_key(&m.n));
        let strip = drawn_on.map(|material| self.annotate_strip(material, cx));
        // flex_none: a short pane must not squeeze the header, or a raised bar would overlap the pane above.
        let label: SharedString = match &pane.kind {
            _ if !title.is_empty() => title.clone().into(),
            Kind::App { name, .. } => name.clone().into(),
            Kind::Terminal => "Terminal".into(),
            Kind::Empty => "Empty pane".into(),
            Kind::Slot { name, .. } => format!("Slot · {name}").into(),
            Kind::File { .. } => "File".into(),
            Kind::Decision => ottospace_core::model::decisions::NAME.into(),
        };
        // A marked pane says so, with its letter: ⌘B gives no other sign while the Marks list is shut.
        let marked = self.model.mark_of(MarkOf::Pane(Some(id))).filter(|_| self.on(Feature::Marks));
        let marked = marked.and_then(|mark| self.model.marks.get(mark));
        let marked = marked.map(|mark| mark.key.map_or("◆".to_string(), |key| format!("◆ {}", key.to_uppercase())));
        // Focus: a line in the mode's accent over the header's foot and its 1pt border, taking no room.
        // A raised window with a title bar covers the header: the line goes in the 2pt gap above it.
        // Agent mode's level, where a task may use the pane, takes the line's place on any pane.
        let focus = theme::focus_line(self.model.focused == Some(id), self.model.mode());
        let accent = self.agent_header_line(id).or(focus);
        let covered = bar > 0.0 && self.model.live == Some(id);
        let line = |d: Div| d.absolute().left_0().right_0().h(px(theme::FOCUS_LINE));
        let (line, top_line) = match accent {
            Some(colour) if covered => (None, Some(line(div()).top_0().bg(rgb(colour)))),
            Some(colour) => (Some(line(div()).bottom(px(-1.)).bg(rgb(colour))), None),
            None => (None, None),
        };
        // The header is the pane's handle: a drag of it carries the pane to a drop zone or a tab.
        let workspace = cx.weak_entity();
        let header = div()
            .id(("header", id))
            .on_drag(DraggedPane(id), move |_, offset, window, cx| {
                // The move that starts the drag is already over a pane: `on_drag_move` saw it
                // before the drag began, so note the drop zone here.
                let at = window.mouse_position();
                let _ = workspace.update(cx, |this, cx| this.pane_drag_move(id, at, window, cx));
                cx.new(|_| DragChip { label: label.clone(), offset })
            })
            .flex_none()
            .h(px(titlebar::header(bar, PANE_HEADER)))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .relative()
            .bg(rgb(theme::RAISED))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::Focus { pane: id }, this.input, window, cx);
                }),
            )
            .child(mark(&self.marks, Mark::Header(id)))
            .gap_2()
            // The title keeps a minimum; past it the title is cut, then the hint.
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(HEADER_TITLE_MIN))
                    .gap_2()
                    .overflow_hidden()
                    .items_center()
                    .children(icon.map(|icon| img(icon).size(px(14.)).flex_none()))
                    .child(cut_text(title))
                    .children(marked.map(|sign| div().flex_none().text_color(rgb(theme::MARK)).child(sign)))
                    // A filled slot says so.
                    .children(self.slot_mark(id)),
            )
            .child(self.controls.mark(format!("header:{id}")))
            // A transcript's indicator never shrinks: it shows while one is on.
            .children(self.listen_header(id, cx))
            // Agent mode's robot: what the workspace's task may do here. Never shrinks.
            .children(self.agent_header(id, cx))
            // A window its app opened by itself, floating: offered here, with why it is in no pane.
            .children(self.offer_header(id, cx))
            // `fit` and × never shrink, so they stay in the header however narrow the pane.
            .child(
                // The task of its tab: the same badge as on the bar.
                div()
                    .flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .children(clip)
                    .children(hint)
                    .children(word)
                    .children(edit)
                    .children(strip)
                    // ⋯: keep this pane as a slot, or edit the slot it is (the `slots` flag).
                    .children(self.slot_dots(id, cx))
                    .child(
                        div()
                            .id(("close", id))
                            .child(mark(&self.marks, Mark::Close(id)))
                            .flex_none()
                            .px_1()
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(theme::ERROR)))
                            .child("×")
                            .child(self.controls.mark(format!("close:{id}")))
                            // The press must not reach the header: an unsteady click would start a drag.
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, window, cx| this.close_pane(id, window, cx))),
                    ),
            )
            .children(line);
        // Once its picture is taken. Until then the pane draws as ever, beside the bar.
        let taken = |material: &&ottospace_core::capture::Material| self.shots.contains_key(&material.n);
        let annotated = self.model.annotated().filter(|material| material.pane == id).filter(taken);
        let content = if let Some(material) = annotated {
            self.render_annotate(material, cx)
        } else if let Some(hosted) = self.hosted.get(&id) {
            // While live, keep showing the last frame under the real window, so any gap during a swap is invisible.
            match hosted.held() {
                Some((buffer, stale)) => {
                    // At its own size at the top left, where the live window sits: a window smaller
                    // than the pane is neither stretched nor centred.
                    let preview = hosted.preview.clone();
                    let (radius, cropped) = (hosted.radius, hosted.shown_crop > 0.0);
                    let content = size(px(hosted.content.width as f32), px(hosted.content.height as f32));
                    // Until a frame of the new size comes, the old one as the window was, and its corners.
                    let stale = stale.map(|([x, y, w, h], cut)| {
                        (point(px(x as f32), px(y as f32)), size(px(w as f32), px(h as f32)), cut)
                    });
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px(hosted.shown.width as f32))
                        .h(px(hosted.shown.height as f32))
                        // The wheel enters the app, and the scroll is passed on to its window.
                        .on_scroll_wheel(
                            cx.listener(move |this, e: &ScrollWheelEvent, window, cx| {
                                this.app_wheel(id, e, window, cx)
                            }),
                        )
                        .overflow_hidden()
                        .child(match stale {
                            Some((at, held, _)) => {
                                surface(buffer).absolute().left(at.x).top(at.y).w(held.width).h(held.height)
                            }
                            None => surface(buffer).size_full(),
                        })
                        // Record where the preview is drawn (e2e reads it), and mask the window's
                        // corners there: the ring is the whole window's, clipped to the preview,
                        // so a window cut off by the pane gets no corners at the cut.
                        .child(
                            canvas(
                                move |bounds, _, _| preview.set(bounds),
                                move |bounds, _, window, _| {
                                    let (whole, cropped) = match stale {
                                        Some((at, held, cut)) => {
                                            (Bounds { origin: bounds.origin + at, size: held }, cut)
                                        }
                                        None => (Bounds { origin: bounds.origin, size: content }, cropped),
                                    };
                                    paint_corner_mask(whole, bounds, radius, cropped, window)
                                },
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        )
                        .into_any_element()
                }
                None => div().child("waiting for frames…").into_any_element(),
            }
        } else if let Kind::File { path } = &pane.kind {
            self.viewer_body(id, path, cx)
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
            // The scrollback as a scroll area: the screen is its last page.
            let ((_, rows), (back, history), line) =
                (terminal.grid_size(), terminal.scrollback(), f32::from(cell.height));
            let extent = Extent {
                viewport: rows as f32 * line,
                content: (history + rows) as f32 * line,
                offset: history.saturating_sub(back) as f32 * line,
            };
            // Shown by the user's own scrolls, not by output arriving. A press beside the thumb
            // is the terminal's: a selection, or the program's own mouse.
            let (scroll_to, scrolls) = (terminal.scroller(), terminal.scrolls() as f32);
            // No edge fade: a terminal's lines run to its edges.
            let scrollbar = self.bars.overlay(
                &format!("terminal:{id}"),
                scrollbar::Axis::Down,
                false,
                None,
                move || (extent, scrolls),
                move |offset, _| scroll_to(history.saturating_sub((offset / line).round() as usize)),
            );
            div()
                .absolute()
                .size_full()
                .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| this.wheel(id, e.delta, cx)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        this.terminal_mouse_down(id, e, window, cx)
                    }),
                )
                .child(grid)
                .child(scrollbar)
                .into_any_element()
        } else {
            match pane.kind {
                Kind::Terminal => {
                    div().text_sm().text_color(rgb(theme::ERROR)).child("the shell did not start").into_any_element()
                }
                // An app that remade its window: the pane waits, offering the new one.
                Kind::Empty if let Some(body) = self.offer_body(id, cx) => body,
                // A slot that waits says what for, and what it offers.
                Kind::Slot { .. } if let Some(body) = self.slot_body(id, cx) => body,
                Kind::Decision => self.decision_body(cx),
                _ => {
                    // A pane a mark's tab could not fill says why.
                    let hint = self.unfilled_why(id).unwrap_or_else(|| "⌘E to pick what goes here".into());
                    // A restored app pane that waits: the user's click hosts its app's window, started if need be.
                    let open = self.unfilled_app(id).map(|name| {
                        list_button(("unfilled-open", id as usize), false)
                            .mt_2()
                            .child(self.controls.mark(format!("unfilled-open:{id}")))
                            .child(format!("Open {name}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                drop(this.dispatch(Action::UnfilledOpen { pane: id }, this.input, window, cx))
                            }))
                    });
                    let empty = home::empty_pane(hint.into(), id, &self.home);
                    div().flex().flex_col().items_center().child(empty).children(open).into_any_element()
                }
            }
        };
        // The content sits 2pt inside the pane. A live app window is placed over it, and without
        // the gap that window's own edge covers the pane's border.
        let inner = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            // Record the content's layout bounds: AX matches the real window to it; e2e reads it.
            .child(canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(self.controls.mark(format!("pane:{id}")))
            .child(content)
            // Grouped offers' open list, over the body's top right, under the chip.
            .children(self.offer_list(id, cx));
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
        div()
            .flex_1()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .border_1()
            // Neutral on every pane: the header's line marks focus (`theme::focus_line`).
            .border_color(rgb(theme::BORDER))
            .when(faded, |d| d.opacity(0.5))
            .relative()
            .when(bar > 0.0, |d| d.p(px(2.)))
            .child(
                canvas(move |bounds, _, _| frame.set(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full(),
            )
            .child(header)
            .child(body)
            .children(top_line)
            // The pulse: the border and a tint in the focus colour, over the pane, taking no clicks.
            .when(glow > 0., |d| {
                let color = gpui::Hsla::from(rgb(theme::PRIMARY));
                let ring = div().absolute().inset_0().border_2().border_color(color.opacity(glow));
                d.child(ring.bg(color.opacity(glow * 0.1)))
            })
            .children(
                self.model
                    .confirm
                    .as_ref()
                    .and_then(|c| c.paste)
                    .filter(|ask| ask.pane == id)
                    .map(|ask| self.render_paste_ask(&ask)),
            )
            .on_drop(cx.listener(move |this, dragged: &DraggedPane, window, cx| {
                let zone = this.drop_zone(dragged.0, window.mouse_position()).filter(|(target, _)| *target == id);
                if let Some((target, zone)) = zone {
                    this.dispatch(Action::DropPane { source: dragged.0, target, zone }, this.input, window, cx);
                }
            }))
            // The drop indicator: the half an edge drop would take, or the whole pane for a swap.
            .children(self.drop_at.filter(|(target, _)| *target == id && cx.has_active_drag()).map(|(_, zone)| {
                let mark = div().absolute().bg(rgb(theme::PRIMARY)).opacity(0.25);
                match zone {
                    Zone::Left => mark.top_0().left_0().h_full().w(relative(0.5)),
                    Zone::Right => mark.top_0().right_0().h_full().w(relative(0.5)),
                    Zone::Top => mark.top_0().left_0().w_full().h(relative(0.5)),
                    Zone::Bottom => mark.bottom_0().left_0().w_full().h(relative(0.5)),
                    Zone::Center => mark.top_0().left_0().size_full(),
                }
            }))
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
                let split = div()
                    .flex_1()
                    .size_full()
                    .overflow_hidden()
                    .flex()
                    .when(down, |d| d.flex_col())
                    .child(first.child(self.render_node(a, cx)))
                    .child(second.child(self.render_node(b, cx)));
                // Its divider's grab, drawn last: over both sides.
                self.divider_grab(split, node, cx).into_any_element()
            }
        }
    }

    /// Top bar, drawn where the system title bar was: tabs on the left, new-tab and split
    /// buttons on the right.
    /// `room`: the points the top row has for its spaces.
    /// The colour a space is drawn in: its task's (`Model::space_task`), else the accent.
    fn space_colour(&self, of: &Owner) -> u32 {
        let task = self.model.space_task(of).and_then(|id| self.model.tasks.get(id));
        task.map_or(theme::PRIMARY, |task| theme::identity(task.colour))
    }

    /// The space carried along the top row, while the drag lasts: its place, and the one it would take.
    fn space_carried(&self, cx: &App) -> Option<(usize, usize)> {
        let reorder = self.model.reorder().filter(|_| cx.has_active_drag())?;
        let ReorderItem::Space { of } = &reorder.item else { return None };
        let from = self.model.shown_spaces().iter().position(|space| space.of == *of)?;
        Some((from, reorder.to))
    }

    /// What a space is called on the top row.
    /// The top row, in the window bar beside the traffic lights: a tab a space, the selected one
    /// raised with a line in its colour along its top; a quiet × hides one. They shrink, then
    /// clip: the + and the buttons on the right always show.
    fn render_spaces(&self, left: f32, room: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.model.shown_space().clone();
        let shown = self.model.shown_spaces();
        // Home is its icon alone, always. Short of room, a space not selected is its badge
        // alone, its title on hover. Shorter still, the badges that do not fit after the selected
        // one give way to a count: the selected space always shows.
        let compact = HOME_SPACE + shown.len().saturating_sub(1) as f32 * TITLED_SPACE > room;
        let tight = compact && room < HOME_SPACE + BADGE_SPACE + TITLED_SPACE;
        let selected_width = if tight { MIN_SPACE } else { TITLED_SPACE };
        let others = shown.iter().filter(|space| space.of != selected && space.of != Owner::Home).count();
        let fits = |n: usize| HOME_SPACE + selected_width + n as f32 * BADGE_SPACE <= room;
        let badges =
            if !compact || fits(others) { others } else { (0..others).rev().find(|n| fits(n + 1)).unwrap_or(0) };
        let mut room_for = badges;
        let mut placed: Vec<(usize, &Space)> = shown.into_iter().enumerate().collect();
        // The space carried is drawn where it would land: the others make room.
        let carried = self.space_carried(cx);
        if let Some((from, to)) = carried {
            let space = placed.remove(from);
            placed.insert(to.min(placed.len()), space);
        }
        let shown: Vec<_> = placed
            .into_iter()
            .filter(|(_, space)| {
                let keep = space.of == selected || space.of == Owner::Home || room_for > 0;
                if keep && space.of != selected && space.of != Owner::Home {
                    room_for -= 1;
                }
                keep
            })
            .collect();
        let more = others - badges;
        let dot = self.model.listen_dot();
        // The heard pane's space left off for room: the count carries its dot.
        let dot_in_more = matches!(&dot, Some(ottospace_core::listen::Dot::Space(of))
            if !shown.iter().any(|(_, space)| space.of == *of));
        let spaces = shown.into_iter().map(|(place, space)| {
            let of = space.of.clone();
            // The pane being heard is in this space, off show: a red dot at its corner.
            let heard = (dot == Some(ottospace_core::listen::Dot::Space(of.clone())))
                .then(|| self.off_show_dot(format!("listen-dot:space:{place}"), cx).top(px(5.)).left(px(3.)));
            let on = of == selected;
            let small = of == Owner::Home || (compact && !on);
            // A temporary workspace's name is muted and italic (selected or not); its tip says it closes without saving.
            let temporary = !self.model.space_kept(&of);
            // Decisions' one tab has no strip: its row reads as the tab, "Decisions · 1 of 2".
            let decisions_tab = self.model.tabs.iter().find(|tab| of == Owner::Decisions && tab.space == of);
            let title = decisions_tab.map_or_else(|| self.model.space_title(&of), |tab| tab.title.clone());
            let tip: SharedString =
                if temporary { format!("{title} · {}", workbook::TEMPORARY_TIP) } else { title.clone() }.into();
            let colour = self.space_colour(&of);
            let count = self.model.space_tabs(&of).len();
            // Home's house, a task's badge, a lone app's icon, a workbook's glyph; a session's chat
            // none. Which one is the core's (`Model::glyph`).
            let glyph = self.model.glyph(&of);
            let lead = match (&glyph, &of) {
                // As its pane header's icon, at the house's size.
                (Glyph::App { name }, _) if self.icons.contains_key(name) => {
                    Some(img(self.icons[name].clone()).size(px(16.)).flex_none().into_any_element())
                }
                // Its tasks' badges, the most urgent first and on top of the next.
                (Glyph::Tasks, _) => {
                    let tasks = self.model.space_badges(&of);
                    let badges: Vec<Div> = tasks.iter().take(5).filter_map(|id| self.task_badge(*id, 18.)).collect();
                    // Each 11 points right of the one before; drawn last to first, so the first is on top.
                    let width = 18. + 11. * badges.len().saturating_sub(1) as f32;
                    let placed = badges.into_iter().enumerate().rev();
                    let placed =
                        placed.map(|(at, badge)| div().absolute().top_0().left(px(11. * at as f32)).child(badge));
                    Some(div().relative().flex_none().w(px(width)).h(px(18.)).children(placed).into_any_element())
                }
                // A task's workspace whose tasks are all done draws as a workspace: never blank when small.
                (_, Owner::Workbook { .. }) | (Glyph::Name, Owner::Task { .. }) => Some(
                    svg()
                        .path("icons/workbook.svg")
                        .size(px(14.))
                        .flex_none()
                        .text_color(rgb(theme::MUTED))
                        .into_any_element(),
                ),
                (_, Owner::Session { .. }) if small => {
                    Some(div().child(ottospace_core::talk::monogram(&self.model.space_title(&of))).into_any_element())
                }
                (Glyph::Home, _) => {
                    let ink = if on { theme::TEXT } else { theme::MUTED };
                    let house = svg().path(HOME_ICON).size(px(16.)).flex_none().text_color(rgb(ink));
                    Some(house.into_any_element())
                }
                _ => None,
            };
            let hide = (of != Owner::Home).then(|| {
                div()
                    .id(("space-hide", place))
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .text_color(rgb(theme::MUTED))
                    .opacity(if on { 0.8 } else { 0.0 })
                    .group_hover("space", |s| s.opacity(0.8))
                    .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                    .child("×")
                    .child(self.controls.mark(format!("space-hide:{place}")))
                    .tooltip(move |_, cx| {
                        // A temporary workspace's × discards it, Undo aside.
                        let tip = if temporary {
                            "Close (⌘⌃W): not kept, Undo on the toast"
                        } else {
                            "Hide (⌘⌃W): nothing in it stops"
                        };
                        AnyView::from(cx.new(|_| Tooltip(tip.into())))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.hide_space(Some(place), window, cx);
                    }))
            });
            // A temporary workspace's …: its menu, Keep (`MenuOf::Space`).
            let dots = match &of {
                Owner::Workbook { id } if temporary && !small => {
                    let (id, control) = (*id, format!("space-menu:{id}"));
                    let open = self.model.row_menu().is_some_and(|(menu, _)| menu.row == MenuOf::Space(id));
                    Some(
                        div()
                            .id(ElementId::Name(control.clone().into()))
                            .flex_none()
                            .px_1()
                            .rounded_sm()
                            .text_color(rgb(theme::MUTED))
                            .opacity(if on || open { 0.8 } else { 0.0 })
                            .group_hover("space", |s| s.opacity(0.8))
                            .when(open, |d| d.bg(rgb(theme::SELECTED)))
                            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                            .child("…")
                            .child(self.controls.mark(control))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation(); // not a click on the space
                                this.open_row_menu(MenuOf::Space(id), false, window, cx);
                            })),
                    )
                }
                _ => None,
            };
            let to = of.clone();
            let dragged = reorder::DraggedItem { item: ReorderItem::Space { of: of.clone() }, place };
            let chip = SharedString::from(title.clone());
            let lifted = carried.is_some_and(|(from, _)| from == place);
            div()
                .id(("space", place))
                .group("space")
                // Home's is not carried: nothing goes before it.
                .when(of != Owner::Home, |d| {
                    d.on_drag(dragged, move |_, offset, _, cx| cx.new(|_| DragChip { label: chip.clone(), offset }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, e: &MouseDownEvent, _, _| this.reorder_from = Some(e.position)),
                        )
                })
                .when(lifted, |d| d.opacity(0.4))
                .relative()
                .h_full()
                // The selected space keeps its name; the others give way first.
                .when(!small, |d| d.min_w(px(MIN_SPACE)).max_w(px(MAX_SPACE)).flex_shrink())
                .when(on && compact && !small, |d| d.min_w(px(selected_width)))
                .when(small || temporary, |d| {
                    let tip = tip.clone();
                    d.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
                })
                .when(small, |d| d.flex_none())
                .overflow_hidden()
                .pl_3()
                .pr_1()
                // Home's house alone: as much room right as left. After `pr_1`, which would win.
                .when(of == Owner::Home, |d| d.pr_3())
                .flex()
                .items_center()
                .gap_2()
                .border_r_1()
                .border_color(rgb(theme::BORDER))
                .cursor_pointer()
                .text_sm()
                .when(on, |d| {
                    let line = div().absolute().top_0().left_0().right_0().h(px(2.)).bg(rgb(colour));
                    d.bg(rgb(theme::RAISED)).text_color(rgb(theme::TEXT)).child(line)
                })
                // Decisions: how far through its cards, over its line.
                .children((of == Owner::Decisions).then(|| self.decisions_progress()).flatten())
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .child(mark(&self.marks, Mark::Space(place)))
                .children(heard)
                .children(lead)
                .children((!small).then(|| {
                    let name = cut_text(title).flex_1();
                    if temporary { name.text_color(rgb(theme::MUTED)).italic() } else { name }
                }))
                .children(
                    (count > 1 && !small)
                        .then(|| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(count.to_string())),
                )
                .children(dots)
                .children(hide.filter(|_| !small))
                .child(self.controls.mark(format!("space:{place}")))
                // A tab dropped on a space moves there.
                .drag_over::<DraggedTab>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    let at = TabAt::End(to.clone());
                    this.dispatch(Action::TabPlace { tab: Some(dragged.0), at }, this.input, window, cx);
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::SpaceClick { index: place }, this.input, window, cx);
                }))
        });
        // The spaces left off for room: their count; a click goes on to the next space.
        let more = (more > 0).then(|| {
            div()
                .id("more-spaces")
                .relative()
                .when(dot_in_more, |d| {
                    d.child(self.off_show_dot("listen-dot:more".into(), cx).top(px(5.)).left(px(2.)))
                })
                .flex_none()
                .px_2()
                .h_full()
                .flex()
                .items_center()
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .child(format!("+{more}"))
                .tooltip(move |_, cx| {
                    AnyView::from(cx.new(|_| Tooltip(format!("{more} more: ⌘⌃] and ⌘⌃[ go along the row").into())))
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::SpaceCycle { delta: 1 }, this.input, window, cx);
                }))
        });
        // With one tab there is no second row: the agent chip is here, in room kept for its badge.
        let reserve = self.chip_reserve();
        let chip = self.agent_chip(TOP_BAR, cx).filter(|_| reserve > 0.).map(|(badge, whole)| {
            div().relative().flex_1().min_w(px(reserve)).h_full().flex().child(badge).child(whole)
        });
        let new_space = div()
            .id("new-workspace")
            .flex_none()
            .w(px(28.))
            .h(px(24.))
            .mx_1()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(rgb(theme::MUTED))
            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
            .child("+")
            .child(self.controls.mark("top:new_workspace"))
            .tooltip(|_, cx| AnyView::from(cx.new(|_| Tooltip("New workspace".into()))))
            .on_click(cx.listener(|this, _, window, cx| this.new_workspace(window, cx)));
        // The traffic lights sit over the left column: room for them only where it is narrower.
        let lights = (TRAFFIC_LIGHTS - left).max(0.);
        // The row sits below the selected space: a shadow at its foot, which only that space covers.
        let below = inner_shade(false, 7.).left(px(lights));
        div()
            .h(px(TOP_BAR))
            .flex_none()
            .relative()
            .child(below)
            .pl(px(lights))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::MUTED))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.dispatch(Action::Leave, this.input, window, cx);
                    // With a custom title bar, the system no longer zooms on double-click. Not
                    // for a `mouse` request: it would zoom or minimise the real window.
                    if e.click_count == 2 && !this.injected {
                        window.titlebar_double_click();
                    }
                }),
            )
            .child(
                div()
                    .relative()
                    .flex_shrink()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .overflow_hidden()
                    .children(spaces)
                    // The window is not moved from here: a press on a space is the space's.
                    .child(
                        canvas(
                            |b, _, _| {
                                let (o, s) = (b.origin, b.size);
                                let rect = [o.x, o.y, s.width, s.height].map(|v| f64::from(f32::from(v)));
                                ottospace_mac::window::set_held_titlebar(rect);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .on_drop(cx.listener(|this, dragged: &reorder::DraggedItem, window, cx| match dragged.item {
                        ReorderItem::Space { .. } => this.reorder_released(dragged, window, cx),
                        ReorderItem::Workbook { .. } => this.reorder_cancel(window, cx),
                    })),
            )
            .children(more)
            .child(new_space)
            .children(chip)
            .child(self.render_top_buttons(cx))
    }

    /// New tab and split buttons: a new empty tab, or the picker for that split. Then Fill the
    /// Whole Display, its arrows out while off and in while on.
    fn render_top_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Each the same: an icon, its sneak label, its tip.
        let button = |id: ElementId, icon: &'static str, name: String, tip: &'static str| {
            div()
                .id(id)
                .w(px(28.))
                .h(px(22.))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(svg().path(icon).size(px(16.)).text_color(rgb(theme::MUTED)))
                .child(self.controls.mark(format!("top:{name}")))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
        };
        let splits = SPLITS.iter().enumerate().map(|(i, (name, icon, tip, axis))| {
            let axis = *axis;
            button(("split", i).into(), icon, name.to_string(), tip)
                .on_click(cx.listener(move |this, _, window, cx| this.top_button(axis, window, cx)))
        });
        let on = self.model.fill_display;
        let tip = if on { "Back from the whole display  ⌃⌘F" } else { "Fill the whole display  ⌃⌘F" };
        let fill = button("fill-display".into(), FILL_ICONS[usize::from(on)], "fill".into(), tip).on_click(
            cx.listener(|this, _, window, cx| {
                this.dispatch(Action::FillDisplay, this.input, window, cx);
            }),
        );
        // Report a bug in ottospace: only while the settings name a checkout of it.
        let report = self.model.bug_icon().then(|| {
            button("report-bug".into(), BUG_ICON, "bug".into(), "Report a bug in ottospace  ⌃⌘B").on_click(cx.listener(
                |this, _, window, cx| {
                    this.error = this.bug_start(this.input, window, cx).err();
                    cx.notify();
                },
            ))
        });
        div().flex_none().ml_auto().mr_2().flex().items_center().gap_1().children(splits).child(fill).children(report)
    }

    /// The second row: the selected space's tabs, only while it has more than one. The one on
    /// show is underlined in the space's colour. A tab drags onto another tab or a space.
    fn render_space_tabs(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let of = self.model.shown_space().clone();
        let places = self.model.space_tabs(&of);
        if places.len() < 2 {
            // Back on this row (e.g. from Home), its tab on show scrolls into view again.
            self.tabs_shown.set(None);
            return None;
        }
        let colour = self.space_colour(&of);
        let chip = self.agent_chip(SPACE_TABS, cx);
        let dot = self.model.listen_dot();
        // The tab on show scrolls into view once, when it changes: a scroll may move it out again.
        // Asked after the row's layout: `scroll_to_item` reads the row's bounds of the frame before.
        let main = self.model.main_at();
        let at = places.iter().position(|place| *place == main);
        let on_show = at.and_then(|_| self.model.tabs.get(main)).map(|tab| tab.id);
        let changed = self.tabs_shown.replace(on_show) != on_show;
        let follow = at.filter(|_| changed).map(|at| {
            let handle = self.tabs_scroll.clone();
            let ask = move |_, window: &mut Window, _: &mut App| {
                handle.scroll_to_item(at);
                window.request_animation_frame();
            };
            canvas(ask, |_, _, _, _| {}).absolute().size_0()
        });
        let tabs =
            places.into_iter().filter_map(|place| Some((place, self.model.tabs.get(place)?))).map(|(place, tab)| {
                let (active, id) = (place == main, tab.id);
                let out = self.model.is_out(id);
                let title = SharedString::from(tab.title.clone());
                div()
                    .id(("tab", tab.id))
                    // A pane dropped on a tab moves there; a tab dropped on one goes before it.
                    .drag_over::<DraggedPane>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
                    .drag_over::<DraggedTab>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
                    .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                        // Dropped on itself: a click that moved a few points (GPUI drags past 2).
                        if dragged.0 == id {
                            return this.select_tab(id, window, cx);
                        }
                        let at = TabAt::Before(id);
                        this.dispatch(Action::TabPlace { tab: Some(dragged.0), at }, this.input, window, cx);
                    }))
                    .on_drag(DraggedTab(id), move |_, offset, _, cx| {
                        cx.new(|_| DragChip { label: title.clone(), offset })
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedPane, window, cx| {
                        this.dispatch(Action::MovePane { pane: Some(dragged.0), tab: place }, this.input, window, cx);
                    }))
                    .relative()
                    .h_full()
                    .min_w(px(MIN_TAB))
                    .max_w(px(MAX_SPACE))
                    // At its narrowest it clips, never paints over the next tab.
                    .overflow_hidden()
                    .pl_3()
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .when(active, |d| {
                        let line = div().absolute().left_0().right_0().bottom_0().h(px(2.)).bg(rgb(colour));
                        d.text_color(rgb(theme::TEXT)).child(line)
                    })
                    // A tab not on show sits below the one that is.
                    .when(!active, |d| d.child(inner_shade(false, 6.)))
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child(shown_mark(&self.marks, Mark::Tab(place)))
                    // The pane being heard is in this tab, off show.
                    .children(
                        (dot == Some(ottospace_core::listen::Dot::Tab(id)))
                            .then(|| self.off_show_dot(format!("listen-dot:tab:{id}"), cx).top(px(6.)).left(px(3.))),
                    )
                    .children(
                        self.model
                            .mark_of(MarkOf::Tab(Some(id)))
                            .filter(|_| self.on(Feature::Marks))
                            .map(|_| div().text_color(rgb(theme::MARK)).child("◆")),
                    )
                    // A tab of a workbook: its icon, and the workbook's name on hover.
                    .when_some(self.workbook_of(id), |d, book| {
                        let icon = svg().path("icons/workbook.svg").size(px(12.)).flex_none();
                        d.child(icon.text_color(rgb(theme::MUTED))).tooltip(move |_, cx| {
                            AnyView::from(cx.new(|_| Tooltip(format!("Workspace: {book}").into())))
                        })
                    })
                    // Out on another display: its mark, and Fold back in where its × is.
                    .when(out, |d| {
                        let icon = svg().path("icons/display-out.svg").size(px(12.)).flex_none();
                        d.child(icon.text_color(rgb(colour)))
                    })
                    .child(cut_text(tab.title.clone()).flex_1())
                    .child(self.controls.mark(format!("tab:{}", tab.id)))
                    .when(out, |d| d.child(self.fold_in_button(id, cx)))
                    .when(!out, |d| {
                        d.child(
                            div()
                                .id(("tab-close", tab.id))
                                .flex_none()
                                .px_1()
                                .rounded_sm()
                                .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::ERROR)))
                                .child("×")
                                .child(self.controls.mark(format!("tab-close:{}", tab.id)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    // Do not also select the tab that is going away.
                                    cx.stop_propagation();
                                    this.close_tab(id, window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.select_tab(id, window, cx)))
            });
        // Tabs past the row's width scroll across, under the overflow motif.
        let row = div()
            .id("tabs")
            .flex_shrink()
            .min_w_0()
            .h_full()
            .flex()
            .overflow_x_scroll()
            .track_scroll(&self.tabs_scroll)
            .children(tabs);
        Some(
            div()
                .h(px(SPACE_TABS))
                .flex_none()
                .flex()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .overflow_hidden()
                .child(self.bars.tracked_row("tabs", row, &self.tabs_scroll, theme::BACKGROUND))
                .children(follow)
                // The agent chip, in the room the tabs leave, at least its badge's: over the row's end.
                .relative()
                .children(chip.map(|(badge, whole)| {
                    div().relative().flex_1().min_w(px(self.chip_badge())).h_full().flex().child(badge).child(whole)
                })),
        )
    }

    /// Bottom bar: who has the keyboard when it is not normal, and what needs saying.
    fn render_status(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = theme::accent(self.model.mode());
        // Name the return key only if it works: a wrong hint strands the user in the app.
        let app_hint = match RETURN_KEY.get() {
            Some(key) => format!("{key} returns"),
            None => "click to return".to_string(),
        };
        let (mode, hint) = match self.model.mode() {
            // A key, not the key list: ⌘/ shows the keys of every mode. A mode you can be in by
            // accident names its way out. Normal is the focus line's colour alone: no chip.
            Mode::Normal => ("", ""),
            Mode::Pick => ("PICK", ""),
            Mode::Palette => ("COMMAND", ""),
            Mode::Inspect => ("INSPECT", ""),
            Mode::App => ("APP", app_hint.as_str()),
            Mode::Visual => ("VISUAL", "esc"),
            Mode::List => ("LIST", "esc"),
            Mode::Compose => ("TASK", ""),
            // Sneak turned the overlay off, and it stays off: say how to get it back.
            Mode::Sneak if self.model.sneak.as_ref().is_some_and(|sneak| sneak.hid_overlay) => {
                ("SNEAK", "esc · ⌘⌃A overlay")
            }
            Mode::Sneak => ("SNEAK", "esc"),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.dismiss.is_some()) => ("DISMISS?", ""),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.unlist.is_some()) => ("UNLIST?", ""),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.paste.is_some()) => ("PASTE?", ""),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.close.is_some()) => ("CLOSE?", ""),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.later.is_some()) => ("LATER?", ""),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.delete.is_some()) => ("DELETE?", ""),
            Mode::Confirm => ("QUIT?", ""),
            // The capture bar lists the annotate keys.
            Mode::Annotate => ("ANNOTATE", ""),
            Mode::Talk => ("TALK", ""),
            Mode::Mark => match self.model.mark_input {
                Some(MarkInput::Rename { .. }) => ("MARK", ""),
                Some(MarkInput::SetKey { .. }) => ("MARK", "a letter for the mark · ⌫ for none · esc cancels"),
                Some(MarkInput::Workbook { .. }) => ("WORKSPACE", ""),
                Some(MarkInput::Worktree { .. }) => ("WORKTREE", "↵ add · esc cancels"),
                _ => ("MARK?", "a mark's letter · esc cancels"),
            },
        };
        let hint = match self.sneak_app_hint() {
            Some(app) => format!("{hint} · {app}"),
            None => hint.to_string(),
        };
        // After a capture from a live app, with the draft gone: the way back to that app.
        let back = self.model.back_to.filter(|_| self.model.capture.is_none()).and_then(|pane| {
            let Kind::App { name, .. } = &self.model.pane(pane)?.kind else { return None };
            Some(format!("↩ back to {name}"))
        });
        // "Copied 3 lines", from anywhere: the status bar says each copy and paste.
        let clip = self.model.clip.as_ref().filter(|clip| clip.shows(now_ms())).map(|clip| clip.text.clone());
        let pending = self.model.viewer_pending();
        // What the middle keeps whole (the hint is cut): the room left is the segments'.
        let kept = [back.as_deref(), clip.as_deref(), pending.as_deref()];
        let middle = kept.iter().flatten().map(|text| status_bar::width(window, text) + 12.).sum::<f32>()
            + if mode.is_empty() { 0. } else { status_bar::width(window, mode) + 16. + 12. };
        let segments = self.status_segments(middle, window, cx);
        self.status_fit.set(segments.fit);
        div()
            .h(px(STATUS_BAR))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .bg(rgb(theme::BACKGROUND))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child(mark(&self.marks, Mark::Status))
            // Where you are: the focused pane's folder and branch, or its app.
            .children(segments.place)
            // The middle: the mode and what it says. Its hover names the build.
            .child(
                div()
                    .id("status-space")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .tooltip(|_, cx| {
                        let (commit, date) = (option_env!("OTTOSPACE_COMMIT"), option_env!("OTTOSPACE_BUILD_DATE"));
                        AnyView::from(cx.new(|_| Tooltip(version_line(env!("CARGO_PKG_VERSION"), commit, date).into())))
                    })
                    .when(!mode.is_empty(), |d| d.child(ui::pill(mode, ui::Pill::Mode(color))))
                    .child(cut_text(hint))
                    // A count or `g` typed in the viewer, waiting for its move (`10j`).
                    .children(pending.map(|typed| div().flex_none().text_color(rgb(theme::TEXT)).child(typed)))
                    .children(back.map(|text| {
                        let back =
                            div().id("status-back").relative().flex_none().cursor_pointer().text_color(rgb(theme::APP));
                        back.child(text)
                            .child(self.controls.mark("draft:back"))
                            .on_click(cx.listener(|this, _, window, cx| this.click_capture("back", window, cx)))
                    }))
                    .children(clip.map(|clip| div().flex_none().text_color(rgb(theme::PRIMARY)).child(clip))),
            )
            // What's next, and who's working.
            .children(segments.meeting)
            .child(div().flex_none().w(px(1.)).h(px(14.)).bg(rgb(theme::BORDER)))
            .child(segments.agents)
    }
}

/// A terminal's header: its directory's last segment (`~` for home), then the program in front
/// of the shell, if any: `app-api · cargo test`. Nothing read: empty.
fn terminal_title(cwd: Option<&str>, program: Option<&str>, home: Option<&str>) -> String {
    let dir = cwd.map(|cwd| match std::path::Path::new(cwd).file_name() {
        _ if Some(cwd) == home => "~".to_string(),
        Some(name) => name.to_string_lossy().into_owned(),
        None => cwd.to_string(),
    });
    dir.into_iter().chain(program.map(str::to_string)).collect::<Vec<_>>().join(" · ")
}

/// A task's priority and title, as cells of a row.
fn task_title_cells(task: &Task) -> impl Iterator<Item = Div> {
    // Only P1 is said: the other priorities are on the card.
    let priority = (task.priority == 1).then(|| div().flex_none().text_xs().text_color(rgb(theme::ERROR)).child("P1"));
    priority.into_iter().chain([cut_text(task.title.clone()).flex_1()])
}

/// A task's tags as a cell at the end of its row, no wider than `width`. No tags: no cell.
fn task_tags_cell(tags: String, width: f32) -> Option<Div> {
    let cell = div().flex_none().max_w(px(width)).truncate().text_xs().text_color(rgb(theme::MUTED));
    Some(tags).filter(|tags| !tags.is_empty()).map(|tags| cell.child(tags))
}

/// A click that shows a task's tabs.
fn open_task(id: TaskId, cx: &mut Context<Workspace>) -> Box<dyn Fn(&ClickEvent, &mut Window, &mut App)> {
    Box::new(cx.listener(move |this, _, window, cx| {
        // The user's click is the one verb: its conversation. A client's shows its tabs, as
        // its `task_open` does: only the user starts an agent.
        let action = if this.input == Origin::User { Action::TaskTalk { id } } else { Action::TaskGo { id: Some(id) } };
        this.dispatch(action, this.input, window, cx);
        cx.notify();
    }))
}

/// The shadow a side zone `width` points wide casts onto the workspace: from its inner edge,
/// to the right for the left zone (`left`), to the left for the right zone.
fn zone_shadow(width: f32, left: bool) -> Div {
    // Below the title strip, which is higher: no shadow falls on it, and none doubles at its corners.
    let edge = div().absolute().top(px(TOP_BAR)).bottom_0().w(px(1.));
    let edge = if left { edge.left(px(width - 1.)) } else { edge.right(px(width - 1.)) };
    edge.shadow(side_shadow(left))
}

/// A side's shadow onto the workspace: to the right from the left side (`left`), else to the left.
fn side_shadow(left: bool) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: black().opacity(0.65),
        offset: point(px(if left { 8. } else { -8. }), px(0.)),
        blur_radius: px(16.),
        spread_radius: px(-6.),
    }]
}

/// The title strip's shadow, down onto everything under it.
fn title_shadow() -> Div {
    div().absolute().left_0().top(px(TOP_BAR - 1.)).w_full().h(px(1.)).shadow(vec![BoxShadow {
        color: black().opacity(0.7),
        offset: point(px(0.), px(8.)),
        blur_radius: px(14.),
        spread_radius: px(-8.),
    }])
}

/// The shadow cast into a box from its top (`top`) or bottom edge, `depth` points deep: the box
/// sits below its edge. It takes no room and no clicks; the box is `relative`.
pub(crate) fn inner_shade(top: bool, depth: f32) -> Div {
    shade(top, depth, 0.6)
}

/// `inner_shade`, `strength` its darkest opacity: a well's is softer.
fn shade(top: bool, depth: f32, strength: f32) -> Div {
    let (dark, clear) = (black().opacity(strength), black().opacity(0.));
    let (from, to) = if top { (dark, clear) } else { (clear, dark) };
    let shade = gpui::linear_gradient(180., gpui::linear_color_stop(from, 0.), gpui::linear_color_stop(to, 1.));
    let edge = div().absolute().left_0().right_0().h(px(depth)).bg(shade);
    if top { edge.top_0() } else { edge.bottom_0() }
}

/// A row of an open sidebar group, in the group's well: a little darker than the section (`WELL`)
/// and across its padding, a soft shadow at the well's top (`first`) and bottom (`last`). Each row
/// stays a child of the list, so list mode still scrolls to it; it covers the list's `gap` above it.
/// Not Calendar's rows: they sit level, as Workspaces' do, a list of the day, not an opened-up group.
pub(crate) fn in_well(row: impl IntoElement, first: bool, last: bool, gap: f32) -> Div {
    const PAD: f32 = 8.; // the list's padding (`p_2`)
    const RIM: f32 = 6.; // the well's own room at its top and bottom, as deep as its shadows
    div()
        .flex_none()
        .relative()
        .mx(px(-PAD))
        .px(px(PAD))
        .bg(rgb(theme::WELL))
        .when(!first, |d| d.mt(px(-gap)).pt(px(gap)))
        .when(first, |d| d.pt(px(RIM)))
        .when(last, |d| d.pb(px(RIM)))
        .child(row)
        .when(first, |d| d.child(shade(true, RIM, 0.35)))
        .when(last, |d| d.child(shade(false, RIM, 0.35)))
}

/// `rows` of an open group, each in its well (`in_well`).
pub(crate) fn well(rows: Vec<AnyElement>, gap: f32) -> impl Iterator<Item = AnyElement> {
    let last = rows.len().saturating_sub(1);
    rows.into_iter().enumerate().map(move |(at, row)| in_well(row, at == 0, at == last, gap).into_any_element())
}

/// A row's height in a sidebar section (`sidebar_row`).
pub(crate) const ROW: f32 = 28.;

/// A row of a sidebar section, one grammar in each: `ROW` points, a leading mark that carries the
/// state's colour, the title cut with "…", one muted detail at the right, its actions on hover.
/// No outline at rest, so a long list stays quiet; hover and list mode's selection fill it.
pub(crate) fn sidebar_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    ui::list_row(id, ui::Row::Normal, ui::State::focus(selected))
}

/// A row's muted detail at its right: `4m`, `09:00`, `2 tabs`.
pub(crate) fn row_meta(text: impl Into<SharedString>) -> Div {
    div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(text.into())
}

/// A row under a heading (a repository's checkout): no outline at rest, so a long list stays
/// quiet. Hover and selection mark it.
fn list_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    ui::list_row(id, ui::Row::Dim, ui::State::focus(selected))
}

/// The time now, in milliseconds since 1970. The core has no clock: actions carry this.
/// When the local day of `now` began (milliseconds since 1970), by the system's time zone: with
/// the offset at that midnight, not now's, so a clock change during the day does not move it.
fn day_start(now: u64) -> u64 {
    use core_foundation::date::CFDate;
    use core_foundation::timezone::CFTimeZone;
    const SINCE_2001: f64 = 978_307_200.;
    let zone = CFTimeZone::system();
    let offset_at = |ms: i64| (zone.seconds_from_gmt(CFDate::new(ms as f64 / 1000. - SINCE_2001)) * 1000.) as i64;
    midnight_of(now, offset_at)
}

/// `day_start` with the zone's offset at a time (`offset_at`, in milliseconds). The date is
/// now's, by now's offset; only the offset is midnight's. e.g. 23:30 on the day clocks went back
/// is of that day, not the next.
fn midnight_of(now: u64, offset_at: impl Fn(i64) -> i64) -> u64 {
    let now_offset = offset_at(now as i64);
    let local = now as i64 + now_offset;
    let date = local - local.rem_euclid(86_400_000);
    (date - offset_at(date - now_offset)).max(0) as u64
}

fn now_ms() -> u64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    since.map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The main loop's clock (`cadence`): ms since it was first read. Monotonic, unlike `now_ms`.
fn loop_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Ready when `timer` is, or sooner when something posts a wakeup.
async fn woken_or(mut timer: gpui::Task<()>) {
    let mut woken = ottospace_mac::wake::woken();
    std::future::poll_fn(|cx| {
        let woke = std::pin::Pin::new(&mut woken).poll(cx).is_ready();
        if woke || std::pin::Pin::new(&mut timer).poll(cx).is_ready() {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

/// A key's modifiers as the core names them.
fn mods(m: gpui::Modifiers) -> Mods {
    Mods { ctrl: m.control, alt: m.alt, shift: m.shift, cmd: m.platform }
}

/// One line of `text`, cut with "…" where it does not fit. GPUI 0.2.2 keeps a text's first
/// measure, which for a flex item has no width: laid over a box of the row's height, it has one.
/// The box is as wide as the whole text, drawn clear, so it may shrink to fit as well as grow.
pub(crate) fn cut_text(text: impl Into<SharedString>) -> Div {
    // One line, as the cut shows: a line break would make the box taller and hide what follows it.
    let text: SharedString = text.into();
    let text: SharedString = if text.contains('\n') { text.replace('\n', " ").into() } else { text };
    let room = div().flex().pr(px(1.)).child(Room(text.clone()));
    let over = div().absolute().inset_0().truncate().child(text);
    div().relative().min_w_0().overflow_hidden().child(room).child(over)
}

/// `cut_text`'s room: its letters' widths added up unkerned, as GPUI's cut adds them, so a text
/// that fits is never cut (`ToTo`, emoji, accents); and no less than the text as shaped, which a
/// pair can widen (`_j`). Rounded up once: a text per letter rounded each, a point more a letter at
/// most (`notes · 2` vs `flow-workbench   · 2`).
struct Room(SharedString);

impl IntoElement for Room {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Room {
    /// The text's shaped width.
    type RequestLayoutState = Pixels;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Pixels) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let width =
            |piece: &str| window.text_system().layout_line(piece, font_size, &[style.to_run(piece.len())], None).width;
        // Past `EACH` letters, the rest in one piece: it is cut anyway.
        const EACH: usize = 128;
        let text = &self.0;
        let split = text.char_indices().nth(EACH).map_or(text.len(), |(at, _)| at);
        let letters = text[..split].chars().fold(px(0.), |sum, c| sum + width(c.encode_utf8(&mut [0; 4])));
        let rest = if split < text.len() { width(&text[split..]) } else { px(0.) };
        let shaped = width(text);
        let mut layout = gpui::Style::default();
        // As tall as a text's line: unrounded (`window.line_height()` rounds, and moved rows' words).
        let tall = style.line_height.to_pixels(font_size.into(), window.rem_size());
        layout.size = size((letters + rest).max(shaped).ceil(), tall).map(Into::into);
        (window.request_layout(layout, [], cx), shaped)
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        shaped: &mut Pixels,
        _: &mut Window,
        _: &mut App,
    ) {
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            CUT_TEXTS.with_borrow_mut(|cuts| cuts.push((self.0.clone(), bounds, *shaped)));
        }
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Pixels,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}

thread_local! {
    /// Test runs only: each text `cut_text` laid out this frame, its room's bounds, its shaped width.
    static CUT_TEXTS: RefCell<Vec<(SharedString, Bounds<Pixels>, Pixels)>> = RefCell::default();
}

/// A test run's `cut_text` of `text` drawn within `within`: its `room` (rect) and its shaped
/// width (`drawn`, unrounded). Null outside a test run.
fn cut_drawn(text: &str, within: Option<Bounds<Pixels>>) -> serde_json::Value {
    let within = within.filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some());
    let Some(within) = within else { return serde_json::Value::Null };
    CUT_TEXTS.with_borrow(|cuts| {
        let found = cuts.iter().find(|(cut, room, _)| cut == text && within.contains(&room.origin));
        found.map_or(
            serde_json::Value::Null,
            |(_, room, drawn)| serde_json::json!({"room": rect(*room), "drawn": f32::from(*drawn)}),
        )
    })
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

/// How long a hosted window may be gone while its app runs before its pane hears so: an app that
/// remakes a window opens the new one within it (the canary at once).
const REMADE_MS: u64 = 1_500;

/// True if the window rules (the user's, then the built-in ones) let this window be put in a pane.
fn window_managed(window: &ax::Window) -> bool {
    with_facts(window, |facts| rules::manages(&config::live().windows, facts))
}

/// Where a new window of a hosted app goes and why, by the window rules and its `shape`; with its
/// bundle id, subrole (for the log), title and identifier (for a slot and the offer's chip).
fn window_decide(
    window: &ax::Window,
    shape: &rules::Shape,
) -> (rules::Decision, Option<String>, String, String, Option<String>) {
    with_facts(window, |facts| {
        let decided = rules::decide(&config::live().windows, facts, shape);
        let identifier = facts.identifier.map(String::from);
        (decided, facts.bundle.map(String::from), facts.subrole.to_string(), facts.title.to_string(), identifier)
    })
}

/// `judge` given what the window rules select by, read from the window.
fn with_facts<R>(window: &ax::Window, judge: impl FnOnce(&Facts) -> R) -> R {
    let (bundle, name) = ax::app_identity(window.pid());
    let (title, subrole) = (window.title().unwrap_or_default(), window.subrole().unwrap_or_default());
    let identifier = window.identifier();
    let facts = Facts {
        app: name.as_deref().unwrap_or_default(),
        bundle: bundle.as_deref(),
        title: &title,
        subrole: &subrole,
        identifier: identifier.as_deref(),
    };
    judge(&facts)
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
/// background: a ring whose inner edge is the window's rounded outline (`bounds`), clipped to
/// the part of it that is drawn (`clip`).
/// `cropped`: the top corners went with the title bar, so only the bottom ones are masked.
fn paint_corner_mask(bounds: Bounds<Pixels>, clip: Bounds<Pixels>, radius: f64, cropped: bool, window: &mut Window) {
    if radius <= 0.0 {
        return;
    }
    let r = px(radius as f32);
    // Lifting the ring by r puts its rounded top edge above the clip.
    let lift = if cropped { r } else { px(0.) };
    let ring = Bounds { origin: bounds.origin - point(r, r + lift), size: bounds.size + size(r * 2., r * 2. + lift) };
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        window.paint_quad(quad(ring, r * 2., transparent_black(), r, rgb(theme::BACKGROUND), BorderStyle::Solid));
    });
}

/// What the picker's header says it opens: "Split right with".
fn picker_title(picker: &Picker) -> &'static str {
    match (picker.new_tab, picker.axis) {
        (true, _) => "New tab with",
        (false, Axis::Right) => "Split right with",
        (false, Axis::Down) => "Split down with",
    }
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

/// The part of an app pane's content `rect` on screen its window is sized to: while the window
/// fills its display (`keep`: what the bars leave), as `fill::app_rect` says.
fn pane_rect(rect: CGRect, keep: Option<ottospace_core::ax::Frame>, live: bool, crop: f64) -> CGRect {
    recovery::to_rect(fill::app_rect(recovery::to_frame(rect), keep, live, crop))
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

/// ottospace's own window, title bar too, in global top-left screen coordinates.
fn own_frame(window: &Window, cx: &App) -> CGRect {
    let size = window.bounds().size;
    on_screen(Bounds { origin: point(px(0.), window.viewport_size().height - size.height), size }, window, cx)
}

impl Workspace {
    /// The keys, actions and pointer handlers of a window's root: the main window's and each tab
    /// out's. Each runs in the workspace, for the window it fires in.
    fn input_handlers(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_key_down(cx.listener(Self::key_down))
            // Esc during a divider drag puts it back, before a terminal or a box sees the key.
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" && this.divider_escape(window, cx) {
                    cx.stop_propagation();
                }
            }))
            // A held modifier lights its caps in an empty tab's list.
            .on_modifiers_changed(cx.listener(|this, _, window, cx| this.modifiers_changed(window, cx)))
            .map(|root| self.command_actions(root, cx))
            // On the root, so a selection drag keeps going over other panes.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                this.mouse_move(e, cx);
                this.annotate_move(e, window, cx);
                this.bug_move(e, window, cx);
            }))
            .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<DraggedPane>, window, cx| {
                this.pane_drag_move(e.drag(cx).0, e.event.position, window, cx);
            }))
            .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<reorder::DraggedItem>, window, cx| {
                let item = e.drag(cx).item.clone();
                this.reorder_moved(&item, e.event.position, window, cx);
            }))
            .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<talk::DraggedEdge>, window, cx| {
                this.drawer_edge_moved(f32::from(e.event.position.x), window, cx);
            }))
            // A release ends the drag, so a later drag elsewhere does not extend this selection.
            // The user's release also ends a `mouse` request's hold.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseUpEvent, window, cx| {
                    (this.dragging, this.held) = (None, false);
                    // A reorder released anywhere but its list or row: put back.
                    this.reorder_cancel(window, cx);
                    this.annotate_release(e.position, window, cx);
                    this.bug_release(e.position, window, cx);
                }),
            )
            // Released outside the window: a reorder is put back too.
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| this.reorder_cancel(window, cx)))
            // Any click leaves visual mode first, before the clicked element acts: visual mode has no
            // backdrop, and e.g. a click on an app pane must go live, not be replayed while blocked.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                // A press outside a row's open menu closes it, and is no press on what is there.
                if this.row_menu_press(event, window, cx) || this.agent_menu_press(event, window, cx) {
                    return cx.stop_propagation();
                }
                // ⌥-click while the new-task box is open references what is under it, and
                // does nothing else: the tab is not selected, the pane not focused.
                if this.compose_point(event, window, cx) {
                    return cx.stop_propagation();
                }
                if this.model.visual.is_some() {
                    this.dispatch(Action::LeaveVisual, this.input, window, cx);
                }
                // The same for sneak mode: the click is the choice.
                if this.model.sneak.is_some() {
                    this.dispatch(Action::LeaveSneak, this.input, window, cx);
                }
                // And for a mark that waits: a click on another row's pencil starts that one.
                // Not a click in the name being typed: that places its cursor.
                if this.model.mark_input.is_some() && !this.in_mark_name(event.position) {
                    this.dispatch(Action::MarkCancel, this.input, window, cx);
                }
            }))
            // Copy, in the menu too, only with something selected: in a field, else in the terminal.
            .when(self.transcript_selected() || self.field_selected() || self.copy_pane().is_some(), |d| {
                d.on_action(cx.listener(|this, _: &CopySelection, window, cx| this.copy(window, cx)))
            })
            .on_action(cx.listener(|this, _: &PasteText, window, cx| this.paste(window, cx)))
            .on_action(cx.listener(|this, _: &Cut, window, cx| this.field_menu(edit::Key::Cut, window, cx)))
            .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
                if !this.transcript_select_all(cx) {
                    this.field_menu(edit::Key::Edit(Edit::SelectAll), window, cx)
                }
            }))
            // ⌘Z and ⇧⌘Z: the field's, else in annotate mode what was drawn.
            .on_action(cx.listener(|this, _: &Undo, window, cx| {
                if !this.annotate_step(Action::AnnotateUndo, window, cx) {
                    this.field_menu(edit::Key::Edit(Edit::Undo), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Redo, window, cx| {
                if !this.annotate_step(Action::AnnotateRedo, window, cx) {
                    this.field_menu(edit::Key::Edit(Edit::Redo), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &VisualMode, window, cx| {
                this.dispatch(Action::ToggleVisual, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ListMode, window, cx| {
                this.refresh_lists(cx);
                // The time, for the row it starts on (the Calendar's meeting that runs).
                this.dispatch(Action::Tick { now: now_ms() }, Origin::System, window, cx);
                this.dispatch(Action::FocusList, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SneakMode, window, cx| this.toggle_sneak(window, cx)))
            .on_action(cx.listener(|this, _: &NewTask, window, cx| this.new_task(window, cx)))
            .on_action(cx.listener(|this, _: &TalkFocus, window, cx| {
                let on = !this.model.talk.as_ref().is_some_and(|talk| talk.keys);
                this.dispatch(Action::TalkKeys { on }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TalkToggle, window, cx| {
                this.dispatch(Action::TalkToggle, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SidebarToggle, window, cx| this.sidebar_toggle(window, cx)))
            .on_action(cx.listener(|this, _: &DialToggle, window, cx| {
                this.dispatch(Action::DialToggle, this.input, window, cx);
            }))
            // ⌃G's, from the menu or the palette: the draft shown, or the new-task line, in the editor.
            .on_action(cx.listener(|this, _: &EditMessage, window, cx| {
                this.dispatch(Action::EditorOpen, this.input, window, cx);
            }))
            // The drawer's Stop, for the conversation it shows.
            .on_action(cx.listener(|this, _: &StopAgent, window, cx| {
                if let Some(id) = this.model.talk.as_ref().filter(|talk| talk.open).map(|talk| talk.task.clone()) {
                    this.dispatch(Action::TaskStop { id }, this.input, window, cx);
                }
            }))
            // ⌘↵ is the new-task box's "start" while it has the keyboard; in a conversation, its Done.
            .on_action(cx.listener(|this, _: &TalkGo, window, cx| {
                if this.model.bug.is_some() {
                    this.bug_send(window, cx);
                } else if this.model.compose_keys() {
                    this.dispatch(Action::ComposeSubmit { open: true }, this.input, window, cx);
                } else if let Some(Action::TaskDone { id }) = this.model.done_key() {
                    this.done(id, window, cx);
                }
            }))
            .on_action(cx.listener(
                |this, _: &agent_mode::AgentMenuKey, window, cx| match this.on(Feature::AgentMode) {
                    true => this.agent_open(None, true, window, cx),
                    false => cx.propagate(),
                },
            ))
            .on_action(cx.listener(|this, _: &agent_mode::AgentOff, window, cx| {
                this.agent_level(ottospace_core::agent_mode::Level::Off, window, cx)
            }))
            .on_action(cx.listener(|this, _: &agent_mode::AgentSee, window, cx| {
                this.agent_level(ottospace_core::agent_mode::Level::See, window, cx)
            }))
            .on_action(cx.listener(|this, _: &agent_mode::AgentAct, window, cx| {
                this.agent_level(ottospace_core::agent_mode::Level::Act, window, cx)
            }))
            .on_action(cx.listener(|this, _: &listen::Transcribe, window, cx| match this.on(Feature::Transcripts) {
                true => this.transcribe_key(window, cx),
                false => cx.propagate(),
            }))
            .on_action(cx.listener(
                |this, _: &listen::PauseTranscribing, window, cx| match this.on(Feature::Transcripts) {
                    true => this.pause_key(window, cx),
                    false => cx.propagate(),
                },
            ))
            // With no bug button its key is no key of ottospace's: it goes on as unbound. Only the
            // main window, where the report is drawn: not a tab out on another display.
            .on_action(cx.listener(|this, _: &ReportBug, window, cx| match this.model.bug_icon() {
                true if Window::window_handle(window) != this.main_window => {}
                true => {
                    this.error = this.bug_start(this.input, window, cx).err();
                    cx.notify();
                }
                false => cx.propagate(),
            }))
            .on_action(cx.listener(|this, _: &CapturePane, window, cx| {
                this.error = this.capture_pane(None, this.input, window, cx).err();
                cx.notify();
            }))
            // With marks off their keys are no keys of ottospace's: they go on as unbound.
            .on_action(cx.listener(|this, _: &MarkPane, window, cx| match this.on(Feature::Marks) {
                true => this.toggle_mark(MarkOf::Pane(None), window, cx),
                false => cx.propagate(),
            }))
            .on_action(cx.listener(|this, _: &MarkTab, window, cx| match this.on(Feature::Marks) {
                true => this.toggle_mark(MarkOf::Tab(None), window, cx),
                false => cx.propagate(),
            }))
            .on_action(cx.listener(|this, _: &SaveWorkbook, window, cx| this.save_workbook(window, cx)))
            .on_action(cx.listener(|this, _: &AddTabToWorkbook, window, cx| this.pick_workbook(window, cx)))
            .on_action(cx.listener(|this, _: &SaveAllWorkbook, window, cx| this.save_all_workbook(window, cx)))
            .on_action(cx.listener(|this, _: &KeepWorkbook, window, cx| this.keep_workspace(window, cx)))
            .on_action(cx.listener(|this, _: &AddToRepos, window, cx| this.add_focused_folder(window, cx)))
            .on_action(cx.listener(|this, _: &StartFresh, window, cx| {
                drop(this.dispatch(Action::StartFresh, this.input, window, cx))
            }))
            .on_action(cx.listener(|this, _: &MarkJump, window, cx| match this.on(Feature::Marks) {
                true => drop(this.dispatch(Action::MarkPrefix, this.input, window, cx)),
                false => cx.propagate(),
            }))
            .on_action(cx.listener(|this, _: &NextTask, window, cx| this.next_task(window, cx)))
            // Tasks ▸ Decisions and the palette: the cards waiting, else the next task, at the time now.
            .on_action(cx.listener(|this, _: &OpenDecisions, window, cx| match this.on(Feature::Decisions) {
                true => {
                    this.dispatch(Action::Tick { now: now_ms() }, this.input, window, cx);
                    drop(this.dispatch(Action::DecisionsOpen, this.input, window, cx))
                }
                false => cx.propagate(),
            }))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                this.dispatch(Action::Split { axis: Axis::Right, kind: Kind::Terminal }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                this.dispatch(Action::Split { axis: Axis::Down, kind: Kind::Terminal }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PickPane, window, cx| this.toggle_picker(Axis::Right, window, cx)))
            .on_action(cx.listener(|this, _: &PickPaneDown, window, cx| this.toggle_picker(Axis::Down, window, cx)))
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| {
                this.dispatch(Action::FocusCycle { delta: 1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPrev, window, cx| {
                this.dispatch(Action::FocusCycle { delta: -1 }, this.input, window, cx);
            }))
            // As the green button: `zoom_asks` turns it into Fill the Whole Display.
            .on_action(cx.listener(|_, _: &ZoomWindow, window, _| window.zoom_window()))
            // The main window's alone: a tab out's fills nothing yet (slice 8).
            .on_action(cx.listener(|this, _: &FillDisplay, window, cx| {
                if Window::window_handle(window) == this.main_window {
                    this.dispatch(Action::FillDisplay, this.input, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.ask_quit(window, cx)))
            .on_action(cx.listener(|this, _: &ShowKeys, window, cx| {
                this.dispatch(Action::ToggleKeys, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &AxOverlay, window, cx| {
                if let Err(err) = this.toggle_ax_overlay(false, window, cx) {
                    this.error = Some(err);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.dispatch(Action::NewEmptyTab, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveTabToTask, window, cx| {
                this.dispatch(Action::PaletteTasks, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextSpace, window, cx| {
                this.dispatch(Action::SpaceCycle { delta: 1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevSpace, window, cx| {
                this.dispatch(Action::SpaceCycle { delta: -1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextChat, window, cx| {
                this.dispatch(Action::TalkCycle { delta: 1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevChat, window, cx| {
                this.dispatch(Action::TalkCycle { delta: -1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &HideSpace, window, cx| {
                this.hide_space(None, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoHome, window, cx| {
                this.dispatch(Action::SpaceSelect { index: 0 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.dispatch(Action::CycleTab { delta: 1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                this.dispatch(Action::CycleTab { delta: -1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.ask_close_tab(None, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FoldTab, window, cx| match this.on(Feature::MultiMonitor) {
                true => drop(this.dispatch(Action::TabOut { tab: None }, this.input, window, cx)),
                false => cx.propagate(),
            }))
            // ⌘W: the focused pane closes; with the keyboard in the drawer, Later on its task.
            .on_action(cx.listener(|this, _: &ClosePane, window, cx| {
                this.dispatch(this.model.close_key(), this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusLeft, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Left }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusRight, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Right }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusUp, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Up }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusDown, window, cx| {
                this.dispatch(Action::FocusDir { dir: Dir::Down }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeLeft, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Left }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeRight, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Right }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeUp, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Up }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeDown, window, cx| {
                this.dispatch(Action::Resize { dir: Dir::Down }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapLeft, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Left }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapRight, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Right }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapUp, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Up }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwapDown, window, cx| {
                this.dispatch(Action::SwapDir { dir: Dir::Down }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Equalize, window, cx| {
                this.dispatch(Action::Equalize, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FitPane, window, cx| {
                this.dispatch(Action::Fit { pane: None }, this.input, window, cx);
            }))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        // A frame: the tick runs each frame for a while, to take what follows it.
        ottospace_mac::wake::post();
        // Every task's letters, worked out once a frame for each badge drawn (`task_badge`).
        *self.badges.borrow_mut() = self.model.badges();
        // This frame draws every action so far: the replies held for it can go (`settle`).
        self.drawn_seq = self.seq;
        self.output = false;
        self.frame_start.set(Some(Instant::now()));
        // `cut_texts` lists this frame's texts only.
        CUT_TEXTS.with_borrow_mut(Vec::clear);
        for pane in &self.model.panes {
            self.cells.entry(pane.id).or_default();
            self.frames.entry(pane.id).or_default();
        }
        self.controls.begin();
        self.chip_line.set(None);
        self.dial_motion.borrow_mut().moving = false;
        // Before anything is drawn this frame: where the drawer and the rail were is last frame's.
        let seal = self.render_seal(window, cx);
        let card_seal = self.render_card_seal(window);
        self.bars.begin();
        self.move_toast(window);
        // The Repos list, the tabs and spaces, and ⌘/'s Known are noted again only while drawn.
        let drawn_anew = |mark: &Mark| {
            matches!(
                mark,
                Mark::Repo(_)
                    | Mark::Checkout(_)
                    | Mark::BarHeader
                    | Mark::TasksDot
                    | Mark::Tab(_)
                    | Mark::ToastCard(_)
                    | Mark::ToastClose(_)
                    | Mark::ToastOffer(_)
                    | Mark::ToastNewTask(_)
                    | Mark::Space(_)
                    | Mark::KeysKnown
                    | Mark::ComposeSwitch
                    | Mark::Heading(..)
                    | Mark::StatusPlace
                    | Mark::StatusMeeting
                    | Mark::StatusAgents
                    | Mark::Overlay(_)
                    | Mark::OfferWords(_)
                    | Mark::OfferTip
            )
        };
        self.marks.borrow_mut().retain(|mark, _| !drawn_anew(mark));
        // The pane drag is over, however it ended (a drop, Esc, a release outside the window):
        // no drop zone may stay drawn.
        if !cx.has_active_drag() {
            self.drop_at = None;
        }
        // The main window's tab: the live tree, unless the keyboard is in a tab out's window.
        let tree = self.model.tree_at(self.model.main_at()).map(|root| self.render_node(root, cx));
        // Only while the tab on show has no panes. Its lines are built once and kept: a frame
        // that moves nothing (a terminal printing in another tab) only paints them again.
        let keep_out = self.home.borrow().keep_out();
        // The modifiers down now light their caps in the list of an empty tab.
        let held = keycaps::held(window);
        // Home's screen, or an empty tab of a task, a session or a workbook: the floor and what goes in it.
        let home = tree.is_none().then(|| self.render_home(self.model.home(&keep_out), held, cx));
        // The left column's width, and the drawer's: as dragged, or half the window when widened.
        // An overlay takes none: the panes lie as with both collapsed (`overlay_panels`).
        let left = RAIL + if self.model.sidebar_takes_room() { SECTION } else { 0. };
        let half = f32::from(window.viewport_size().width) / 2.;
        let drawer = if self.model.talk_wide { half } else { self.model.talk_width.min(half.max(MIN_DRAWER)) };
        // The editor gets its columns: a narrower drawer widens while it shows, up to half the window.
        let columns =
            ottospace_core::model::editor::COLUMNS as f32 * f32::from(self.cell.width) + 10. + drawer_rail::WIDTH;
        let drawer = if self.model.editor_shown() { drawer.max(columns.min(half.max(MIN_DRAWER))) } else { drawer };
        // The btw column widens it, the conversation kept as wide, to three quarters of the window at most.
        let most = f32::from(window.viewport_size().width) * 0.75;
        let drawer = if self.btw_drawn() { (drawer + btw::WIDTH).min(most).max(drawer) } else { drawer };
        self.drawer.width = drawer;
        // The drawer's place and its rail, always: the rail alone while it is collapsed, or an overlay.
        let last = self.slides.kept.take();
        let (right, right_width, drawer_over) = self.render_right(drawer, cx);
        let inner = (drawer - drawer_rail::WIDTH).max(0.);
        let (bar_over, drawer_over, left_in, right_in) = self.overlays_drawn(drawer_over, last, inner, window, cx);
        // The toast keeps clear of an overlay drawer or bar, as far in as it is.
        let toast_right = right_width + if drawer_over.is_some() { inner * right_in } else { 0. };
        let toast_left = if bar_over.is_some() { SECTION * left_in } else { 0. };
        // The top row's room for its spaces: over the centre and the drawer, less the lights, the +,
        // the buttons and the agent chip's badge.
        let centre = f32::from(window.viewport_size().width) - left;
        let room = centre - (TRAFFIC_LIGHTS - left).max(0.) - 140. - self.chip_reserve();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(Self::overlay_press))
            .map(|root| self.input_handlers(root, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    // Left: the traffic lights' strip over the rail and the section list. While an
                    // app has the keyboard, ottospace's own chrome fades.
                    .child(
                        div()
                            .h_full()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .when(self.model.live.is_some(), |d| d.opacity(0.5))
                            .child(self.render_lights(cx))
                            .child(
                                div().flex_1().min_h_0().flex().child(self.render_rail(cx)).children(
                                    self.model
                                        .sidebar
                                        .filter(|_| self.model.sidebar_takes_room())
                                        .map(|section| self.render_bar(section, cx)),
                                ),
                            ),
                    )
                    // The top row of spaces from the section list over the drawer; under it the
                    // selected space's tabs over the workspace, and the drawer beside them.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(self.render_spaces(left, room, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .flex()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .h_full()
                                            .flex()
                                            .flex_col()
                                            .children(self.render_space_tabs(cx))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    // Let the canvas shrink below its content, so a wide
                                                    // image cannot push it past the window; it scales down.
                                                    .min_w_0()
                                                    .min_h_0()
                                                    .w_full()
                                                    .flex()
                                                    .overflow_hidden()
                                                    .children(home)
                                                    .children(tree),
                                            ),
                                    )
                                    // The drawer (the draft being collected, a conversation, or a brief).
                                    .child(right),
                            ),
                    )
                    // The side zones sit above the workspace: each casts a soft shadow onto it.
                    // Drawn last, so it lies over the panes. A live app's window is above it all.
                    .child(zone_shadow(left, true))
                    .child(zone_shadow(right_width, false))
                    // The overlays, over the panes and their shadows (`overlay_panels`).
                    .children(bar_over)
                    .children(drawer_over)
                    // Highest, the title strip: its shadow falls on all three zones.
                    .child(title_shadow()),
            )
            // The meeting timeline, directly above the status bar.
            .children(self.render_timeline(cx))
            .child(self.render_status(window, cx))
            // Only over a parked, drawn pane: a live window sits above ottospace and would cover it.
            .children(
                self.ax_overlay
                    .as_ref()
                    .filter(|o| self.model.ax_overlay && self.model.live != Some(o.pane))
                    .filter(|o| self.model.main_panes().contains(&o.pane))
                    .map(ax_debug::Overlay::render),
            )
            .children(self.render_inspector_outline())
            // The toast, over the panes at the bottom right, clear of the overlays: it takes no room.
            // A live app's window is above it.
            .children(self.render_toast(toast_right, centre - toast_right - toast_left, cx))
            // Over everything of ottospace's own. It is never on while an app's window is.
            .children(self.render_row_menu(window, cx))
            .children(self.render_agent_menu(window, cx))
            .children(self.render_slot_dialog(cx))
            .children(self.model.sneak.as_ref().map(sneak::labels))
            .children(self.model.picker.as_ref().map(|picker| self.render_picker(picker, cx)))
            .children(self.model.palette.as_ref().map(|palette| self.render_palette(palette, cx)))
            .children(self.render_pointing())
            .when(self.model.keys, |d| d.child(self.render_keys(cx)))
            // Over the window's own controls: the bug report's frozen picture.
            .children(self.render_bug(window, cx))
            .children(self.model.confirm.as_ref().map(|confirm| self.render_confirm(confirm, cx)))
            .when(self.gallery, |d| d.child(ui::gallery::view()))
            .children(seal)
            .children(card_seal)
            .child(frame_probe(self.frame_start.clone(), self.renders, self.seq))
    }
}

/// Painted last: notes in the trace that frame `number` is drawn, and how long it took from
/// `render` (layout, prepaint and paint).
fn frame_probe(start: Rc<Cell<Option<Instant>>>, number: u64, seq: u64) -> impl IntoElement {
    let paint = move |_, _, _: &mut Window, _: &mut App| {
        if let Some(start) = start.take() {
            let took = start.elapsed().as_micros() as f64 / 1000.0;
            trace::record("frame", serde_json::json!({ "number": number, "seq": seq, "took_ms": took }));
        }
    };
    canvas(|_, _, _| {}, paint).absolute().size_0()
}

/// `ottospace ctl '<json request>'`: sends one request to the running app, prints the reply.
fn ctl(args: &[String]) -> ! {
    let Some(request) = args.first() else {
        eprintln!("usage: ottospace ctl '{{\"cmd\":\"state\"}}'");
        std::process::exit(2);
    };
    // An agent ottospace started for a task carries its pass: the reply knows whose it is.
    let request = control::with_pass(request, pass::from_env().as_deref());
    let reply = control::send(&control::path(), &request).unwrap_or_else(|err| {
        eprintln!("no ottospace at {}: {err}", control::path().display());
        std::process::exit(1);
    });
    println!("{reply}");
    // Exit 0 only when the app accepted the request, so scripts can test the result.
    let ok = serde_json::from_str::<serde_json::Value>(&reply).is_ok_and(|v| v["ok"] == true);
    std::process::exit(if ok { 0 } else { 1 });
}

/// `ottospace sessions [dir]`: the agent sessions of the directory's repository, worktrees included,
/// as JSON. Names, counts and times only; see `docs/ai/sessions.md`.
fn sessions(dir: Option<&String>) -> ! {
    use ottospace_sessions::{config_dirs, git::checkouts, readers, report::report};
    const USAGE: &str = "usage: ottospace sessions [dir]\n\
        Prints the Claude Code and Codex sessions of the repository at <dir> (an absolute path;\n\
        default: the current directory) as JSON: names, counts and times only.";
    if dir.is_some_and(|d| d == "--help" || d == "-h") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    // No directory, an empty or relative one, or one that is not there: an error, never "all sessions".
    let dir = dir.map(PathBuf::from).map_or_else(std::env::current_dir, Ok);
    let readers = readers(&config_dirs());
    let readers: Vec<_> = readers.iter().map(|reader| reader.as_ref()).collect();
    let report = dir.and_then(|dir| checkouts(&dir)).and_then(|dirs| report(&readers, &dirs, now_ms(), 5));
    match report {
        Ok(report) => println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default()),
        Err(err) => {
            eprintln!("agents: {err}\n{USAGE}");
            std::process::exit(2);
        }
    }
    std::process::exit(0);
}

/// The `--version` line. `scripts/package.sh` passes the commit and its date in at compile time;
/// a plain `cargo build` has neither.
fn version_line(version: &str, commit: Option<&str>, date: Option<&str>) -> String {
    format!("ottospace {version} ({})", build_of(commit, date))
}

/// The build in the `--version` line and the About panel: `a4a2582c0ffe, 2026-11-12T08:09:00Z`.
fn build_of(commit: Option<&str>, date: Option<&str>) -> String {
    match (commit.filter(|c| !c.is_empty()), date.filter(|d| !d.is_empty())) {
        (Some(commit), Some(date)) => format!("{commit}, {date}"),
        (Some(commit), None) => commit.to_string(),
        _ => "local build".to_string(),
    }
}

/// The build, short, for the state: `0.1.0 · a4a2582` (seven characters of the commit), or
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
    // Started from a task agent's shell (a dev build): that pass is its, not for our children.
    // SAFETY: no thread has started yet.
    unsafe { std::env::remove_var(pass::VAR) };
    // The variable that held the pass itself before it moved to a file (an older build's agent).
    // SAFETY: as above.
    unsafe { std::env::remove_var("OTTOSPACE_TASK_PASS") };
    // Hidden while its output has no stable shape. Reads other programs' session files.
    if args.get(1).is_some_and(|a| a == "sessions") {
        sessions(args.get(2));
    }
    // `ottospace skill` prints the agent guide; `--stub` prints the SKILL.md that points to it.
    if args.get(1).is_some_and(|a| a == "skill") {
        let stub = args.get(2).is_some_and(|a| a == "--stub");
        print!("{}", if stub { SKILL_STUB } else { SKILL_GUIDE });
        return;
    }
    // Brings back windows stranded off screen; no app need run.
    if args.get(1).is_some_and(|a| a == "rescue") {
        give_back::rescue_command();
    }
    // Hidden: the guardian the app starts, which gives its windows back if it dies.
    if args.get(1).is_some_and(|a| a == "guard") {
        give_back::guard(args.get(2));
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
        eprintln!(
            "unknown command: {unknown}\nusage: ottospace [ctl '<json request>' | skill [--stub] | rescue | --version]"
        );
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
        // Windows a run that ended without its quit left hosted. A test run keeps its record in
        // its own folder (`OTTOSPACE_HOME`), or none, and moves only its canary's.
        give_back::leftovers(host_only);
    }
    // TERM, INT, HUP: a quit, so hosted windows go back.
    give_back::on_signals();
    trace::arm();
    event_log::arm();
    trace::record("start", serde_json::json!({ "pid": std::process::id(), "hosting": hosting }));
    let (version, commit) = (env!("CARGO_PKG_VERSION"), option_env!("OTTOSPACE_COMMIT"));
    let start = serde_json::json!({
        "pid": std::process::id(), "version": version, "commit": commit, "hosting": hosting, "displays": displays()
    });
    event_log::event("start", start);
    let control = control::serve().map_err(|err| eprintln!("control socket off: {err}")).ok();
    // The socket is this run's: so is the folder of its agents' pass files beside it.
    if control.is_some() {
        pass::sweep(&pass::folder());
    }
    // For `osi`: which program the running app is.
    running::write();

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        // Run as a bare binary (`ottospace`, `cargo run`) there is no bundle to supply the icon.
        ottospace_mac::window::set_dock_icon(include_bytes!("../assets/ottospace.icns"));
        cx.bind_keys(keymap());
        // A change of the displays may leave GPUI drawing on none (`stall`).
        display::watch();
        // The menu bar lists the same actions; the app-level ones are handled here.
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &About, cx| {
            let (commit, date) = (option_env!("OTTOSPACE_COMMIT"), option_env!("OTTOSPACE_BUILD_DATE"));
            cx.activate(true);
            ottospace_mac::window::show_about(env!("CARGO_PKG_VERSION"), &build_of(commit, date));
        });
        cx.set_menus(menus::app_menus(&features::startup()));
        let (width, height) = e2e_window().unwrap_or((1400., 900.));
        let mut bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        // A test run stays out of the user's way: a background app that never takes the keyboard,
        // with its window at a free corner of the screens, all but a sliver of it off screen.
        // (A sliver, because a window with nothing showing is not drawn, and tests wait on layout.)
        // `OTTOSPACE_E2E_SHOW` keeps the window where it can be looked at.
        let quiet = !hosting;
        if quiet {
            ottospace_mac::window::stay_in_background();
            if std::env::var_os("OTTOSPACE_E2E_SHOW").is_none() {
                let corner = display::parking_spot(f64::INFINITY);
                bounds.origin = point(px(corner.x as f32 - E2E_SLIVER), px(corner.y as f32 - E2E_SLIVER));
            }
        }
        // Draw under the title bar; the top bar leaves room for the traffic lights.
        let titlebar = TitlebarOptions {
            title: Some("ottospace".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(10.), px(13.))),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(titlebar),
            focus: !quiet,
            ..Default::default()
        };
        let workspace = cx
            .open_window(options, |window, cx| cx.new(|cx| Workspace::new(hosting, host_only, control, window, cx)))
            .expect("open window");
        // Quit comes here from the menu when the window is not the active one (minimised, say):
        // the workspace handles it first otherwise. Either way it may ask.
        cx.on_action(move |_: &Quit, cx| {
            if workspace.update(cx, |this, window, cx| this.ask_quit(window, cx)).is_err() {
                cx.quit();
            }
        });
        // The red close button asks the same question; `Effect::Quit` ends the app, not the click.
        let _ = workspace.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, |window, cx| {
                if let Some(Some(workspace)) = window.root::<Workspace>() {
                    workspace.update(cx, |this, cx| this.ask_quit(window, cx));
                }
                false
            });
        });
        // Only the main window's closing quits: a tab out's folds in.
        let main: gpui::AnyWindowHandle = workspace.into();
        cx.on_window_closed(move |cx| {
            if !cx.windows().contains(&main) {
                cx.quit();
            }
        })
        .detach();
        if hosting {
            cx.activate(true);
            // The return key works from any app, so it is only useful with hosting on.
            let name = std::env::var("OTTOSPACE_RETURN_KEY").unwrap_or_else(|_| "f18".into());
            let registered = hotkey::key_code(&name)
                .ok_or_else(|| "it is not one of f13-f20".to_string())
                .and_then(hotkey::register);
            // A key another app holds is said in the toast too: stderr is seen by nobody.
            let returns = registered.err().map(|err| format!("return key {} is off: {err}", name.to_uppercase()));
            if returns.is_none() {
                let _ = RETURN_KEY.set(name.to_uppercase());
            }
            let captures = collect::register_key(host_only.is_some());
            let notice: Vec<String> = returns.into_iter().chain(captures).collect();
            if !notice.is_empty() {
                eprintln!("{}", notice.join("; "));
                let _ = workspace.update(cx, |this, _, cx| {
                    this.key_notice = Some(notice.join("; "));
                    cx.notify();
                });
            }
        }
        collect::sweep_drafts();
        agent_mode::sweep_looks();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_title_is_its_directory_then_its_program() {
        let home = Some("/Users/me");
        assert_eq!(terminal_title(Some("/Users/me/code/app-api"), Some("cargo test"), home), "app-api · cargo test");
        assert_eq!(terminal_title(Some("/Users/me/code/app-api"), None, home), "app-api");
        assert_eq!(terminal_title(Some("/Users/me"), None, home), "~");
        assert_eq!(terminal_title(Some("/"), Some("top"), home), "/ · top");
        assert_eq!(terminal_title(None, None, home), "");
    }

    #[test]
    fn version_line_names_the_commit_when_the_build_was_given_one() {
        assert_eq!(
            version_line("0.1.0", Some("a4a2582c0ffe"), Some("2026-11-12T08:09:00Z")),
            "ottospace 0.1.0 (a4a2582c0ffe, 2026-11-12T08:09:00Z)"
        );
        assert_eq!(version_line("0.1.0", Some("a4a2582c0ffe"), None), "ottospace 0.1.0 (a4a2582c0ffe)");
        assert_eq!(version_line("0.1.0", None, None), "ottospace 0.1.0 (local build)");
        assert_eq!(version_line("0.1.0", Some(""), Some("")), "ottospace 0.1.0 (local build)");
        // The short form.
        assert_eq!(version_tag("0.1.0", Some("a4a2582c0ffe")), "0.1.0 · a4a2582");
        assert_eq!(
            (version_tag("0.1.0", None), version_tag("0.1.0", Some(""))),
            ("0.1.0 · local".into(), "0.1.0 · local".into())
        );
    }

    /// Every command name the app accepts, read from serde's own error for an unknown one, so a
    /// new `Request` variant is in the list with no list to keep.
    fn request_names() -> Vec<String> {
        let error = serde_json::from_str::<Request>(r#"{"cmd":"?"}"#).expect_err("? is not a command").to_string();
        let names: Vec<String> = error.split('`').skip(3).step_by(2).map(String::from).collect();
        assert!(names.len() > 20, "could not read the command names from: {error}");
        names
    }

    /// Fails unless the app accepts `json` as a request. `who`: where the request is written.
    fn assert_parses(json: &str, who: &str) {
        let parsed: Result<Request, _> = serde_json::from_str(json);
        assert!(parsed.is_ok(), "{who} a request the app refuses: {json}: {:?}", parsed.err());
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
            assert_parses(json, "the guide shows");
        }
    }

    /// Every request the app accepts must have an example in the guide. But `capture_start`,
    /// `repo_add` and `sneak_app_tree`: they answer only in a test run, and an agent must not learn
    /// them (see `AGENTS.md`).
    #[test]
    fn the_agent_guide_covers_every_request() {
        const TESTS_ONLY: [&str; 21] = [
            "capture_start",
            "bug_start",
            "repo_add",
            "inspector_fixture",
            "sneak_app_tree",
            "calendar_call",
            "record_start",
            "record_stop",
            "listen_end",
            "app_fixture",
            "audio_probe_start",
            "audio_probe_stop",
            "zoom_press",
            "window_width",
            "display_change",
            "ui_gallery",
            "window_fixture",
            "window_fixture_close",
            "slot_windows",
            "slot_make",
            "displays_set",
        ];
        assert!(TESTS_ONLY.iter().all(|name| !SKILL_GUIDE.contains(name)));
        for name in request_names().into_iter().filter(|name| !TESTS_ONLY.contains(&name.as_str())) {
            assert!(SKILL_GUIDE.contains(&format!(r#""cmd":"{name}""#)), "the guide has no example of `{name}`");
        }
    }

    /// The TypeScript client (`sdk/`) writes one line per method to this file, as it sends it. Each
    /// must parse, and every request must be there: a new `Request` variant with no client method
    /// fails here, and so does a field the client names wrongly.
    #[test]
    fn the_sdk_fixture_holds_every_request() {
        let fixture = include_str!("../../../sdk/test/requests.jsonl");
        for json in fixture.lines() {
            assert_parses(json, "the SDK sends");
        }
        for name in request_names() {
            let sent = fixture.lines().any(|line| line.starts_with(&format!(r#"{{"cmd":"{name}""#)));
            assert!(sent, "the SDK has no method for `{name}`: add one to sdk/src/client.ts (see sdk/README.md)");
        }
    }

    /// A missing asset draws as nothing, with no error. Catch a renamed file here.
    #[test]
    fn every_rail_icon_is_embedded() {
        let icons = SECTIONS.iter().map(|(_, icon, _)| *icon).chain(SPLITS.iter().map(|(_, icon, ..)| *icon));
        for path in icons.chain([HOME_ICON, BUG_ICON]).chain(FILL_ICONS) {
            assert!(Assets.load(path).unwrap().is_some(), "{path} is not in Assets::load");
        }
    }

    /// Los Angeles: UTC-7 until 2026-11-01 09:00 UTC (2:00 PDT), UTC-8 after; UTC-8 until
    /// 2026-03-08 10:00 UTC (2:00 PST), UTC-7 after.
    #[test]
    fn the_day_starts_at_local_midnight_across_a_clock_change() {
        const H: i64 = 3_600_000;
        let (back, forward) = (1_793_523_600_000_i64, 1_772_964_000_000_i64);
        let la = |ms: i64| if (forward..back).contains(&ms) { -7 * H } else { -8 * H };
        // 2026-11-01 23:30 PST: the day began at 00:00 PDT (07:00 UTC), not 23:00 PST.
        assert_eq!(midnight_of((back + 22 * H + 30 * 60_000) as u64, la), (back - 2 * H) as u64);
        // 2026-03-08 23:00 PDT: the day began at 00:00 PST (08:00 UTC).
        assert_eq!(midnight_of((forward + 20 * H) as u64, la), (forward - 2 * H) as u64);
        // A day with no change: 04:00 PST began at 08:00 UTC.
        assert_eq!(midnight_of((back + 27 * H) as u64, la), (back + 23 * H) as u64);
    }

    /// The rail is drawn in the order the core moves through it.
    #[test]
    fn the_rail_is_drawn_in_the_cores_order() {
        let drawn: Vec<Section> = SECTIONS.iter().map(|(section, ..)| *section).collect();
        assert_eq!(drawn, ottospace_core::model::zones::SECTIONS);
    }
}
