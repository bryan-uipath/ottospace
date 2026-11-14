//! Imperative shell: draws the workspace, hosts other apps' windows in panes, and turns input
//! (mouse, keys, control socket) into `ottospace_core` actions. State rules live in the core.
//!
//! An app pane shows its parked window's SCK stream. Clicking it moves the real window over the
//! pane ("live") and replays the click; clicking ottospace itself parks it again.

// The state snapshot is one `json!` with more keys than the default limit expands.
#![recursion_limit = "256"]

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
    AnyElement, AnyView, App, Application, AssetSource, BorderStyle, Bounds, BoxShadow, ClickEvent, ClipboardItem,
    ContentMask, Context, Div, ElementId, FocusHandle, FontWeight, KeyBinding, KeyDownEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString,
    Size, Stateful, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, black, canvas, div,
    img, point, prelude::*, px, quad, relative, rgb, size, surface, svg, transparent_black,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_core::compose;
use ottospace_core::edit::{self, Edit};
use ottospace_core::hosted::{self as hosting, Click};
use ottospace_core::keys::{self, FilterKey, Mods};
use ottospace_core::list::{ListKey, list_key};
use ottospace_core::marks::{Field, MarkInput, MarkOf, MarkRef, letter};
use ottospace_core::model::zones::RailFocus;
use ottospace_core::model::{CONFIRM_UP, TabAt};
use ottospace_core::repos::{Checkout, Repo};
use ottospace_core::rules::{self, Facts};
use ottospace_core::scrollbar::{Extent, key_step};
use ottospace_core::settings::Installed;
use ottospace_core::sneak::{SneakKey, sneak_key};
use ottospace_core::tasks::{Assign, Group, Heading, MAX_TITLE, State, Task, TaskId, TaskRow};
use ottospace_core::terminal::{CellPoint, VisualKey, cell_at, scroll_key, select_kind, visual_key};
use ottospace_core::{
    Action, Axis, Dir, Effect, Kind, Mode, Model, Node, Origin, Pane, PaneId, PickItem, Picker, Rect, Section, Zone,
    theme,
    titlebar::{self, TitleBar},
};
use ottospace_mac::window::{Mouse, MouseKind};
use ottospace_mac::{ax, capture, display, hotkey, icon, input, recovery};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use screencapturekit::prelude::*;

mod app_wheel;
mod apps;
mod ax_debug;
mod calendar;
mod collect;
mod commands;
mod config;
mod confirm;
mod control;
mod conversation;
mod dialog;
mod field;
mod give_back;
mod home;
mod inspector;
mod layout_file;
mod markdown;
mod marks;
mod menus;
mod new_task;
mod progress;
mod repos;
mod running;
#[cfg(test)]
mod scratch;
mod scrollbar;
mod sessions;
mod sneak;
mod sneak_apps;
mod tags;
mod talk;
mod terminal;
mod trace;
mod words;
mod workbook;
use control::{Call, MouseAction, PaneKind, Request};
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
    /// When the stream last said anything, a frame or "nothing changed" (milliseconds since
    /// 1970). It says nothing while its window is not drawn: minimised, or on another Space.
    seen: AtomicU64,
}

impl Latest {
    fn expect(&self, config: &SCStreamConfiguration) {
        self.expected.store((config.width() as u64) << 32 | config.height() as u64, Relaxed);
    }
}

struct Output(Arc<Latest>);

impl SCStreamOutputTrait for Output {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, _: SCStreamOutputType) {
        self.0.seen.store(now_ms(), Relaxed);
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

/// Stream config showing `size` points of the window at `scale` (Retina = 2), from its top left
/// below its top `crop` points (the title bar), never scaled: a window that refused the pane's
/// size (Outlook's minimum) looks as it does when it is live.
fn stream_config(size: CGSize, crop: f64, scale: f64) -> SCStreamConfiguration {
    // 420f is the only format gpui's surface renderer accepts.
    SCStreamConfiguration::new()
        .with_width((size.width * scale).max(1.0) as u32)
        .with_height((size.height * scale).max(1.0) as u32)
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
    /// The window's content size when `shown` was last worked out, for the corner mask.
    content: CGSize,
    /// How often `window_frame` read the window's frame.
    frame_reads: u64,
    /// The last size and crop the stream refused, so the same one is not asked for every check.
    refused: Option<(CGSize, f64)>,
    /// Where the preview was last drawn, in window-content coordinates.
    preview: Rc<Cell<Bounds<Pixels>>>,
    /// Last pane content rect the live window was placed over, to re-place only on change.
    placed: CGRect,
}

impl Hosted {
    fn new(target: capture::Target) -> Result<Self, String> {
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
        let (bundle, known_as) = ax::app_identity(pid);
        let app = capture::name_of(pid).or(known_as).unwrap_or_default();
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
            stream,
            pid,
            radius,
            bar,
            crop: 0.0,
            bounds: Rc::default(),
            parked_size: CGSize::ZERO,
            shown: original.size,
            shown_crop: 0.0,
            content: original.size,
            frame_reads: 0,
            refused: None,
            preview: Rc::default(),
            placed: CGRect::default(),
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
        }
        back.is_some()
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
        }
        self.parked_size = size;
        self.crop = crop;
        self.show(scale);
        let _ = self.window.set_position(display::parking_spot());
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
        if asked == (self.shown, self.shown_crop) {
            self.content = content;
            return Some(content);
        }
        if self.refused == Some(asked) {
            return Some(content);
        }
        let config = stream_config(shown, self.crop, scale);
        // Frames of another size are dropped, so expect the new one only once it is set.
        if self.stream.update_configuration(&config).is_ok() {
            self.latest.expect(&config);
            (self.shown, self.shown_crop) = asked;
            self.content = content;
            self.refused = None;
        } else {
            // `shown` keeps describing what the stream still shows.
            self.refused = Some(asked);
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
    }

    /// Places the window over its pane and activates its app: entering, and a peek. The app may
    /// have resized its window since it was parked: a click is judged against what it covers now.
    fn bring_up(&mut self, window: &Window, cx: &App) {
        let rect = on_screen(self.bounds.get(), window, cx);
        self.place(rect, self.crop_at(rect, top_limit(window, cx)));
        self.window.raise();
        self.show(window.scale_factor() as f64);
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
            "icons/marks.svg" => Some(include_bytes!("../assets/icons/marks.svg")),
            "icons/calendar.svg" => Some(include_bytes!("../assets/icons/calendar.svg")),
            "icons/split-right.svg" => Some(include_bytes!("../assets/icons/split-right.svg")),
            "icons/split-down.svg" => Some(include_bytes!("../assets/icons/split-down.svg")),
            "icons/plus.svg" => Some(include_bytes!("../assets/icons/plus.svg")),
            "icons/dismiss.svg" => Some(include_bytes!("../assets/icons/dismiss.svg")),
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

/// Top-bar buttons: the name in the control's id (`top:new_tab`), icon, tooltip, split direction,
/// and whether it makes a new tab (no picker) instead.
const SPLITS: [(&str, &str, &str, Axis, bool); 3] = [
    ("new_tab", "icons/plus.svg", "New tab  ⌘T", Axis::Right, true),
    ("split_right", "icons/split-right.svg", "Split right…  ⌘E", Axis::Right, false),
    ("split_down", "icons/split-down.svg", "Split down…  ⌘⇧E", Axis::Down, false),
];

/// The key reference, by group: keys and what they do. Keep in step with the bindings in `main`.
const KEYS_PANES: &[(&str, &str)] = &[
    ("⌘E / ⌘⇧E", "Pick what opens in a new pane: right / below"),
    ("⌘D", "Terminal to the right"),
    ("⌘⇧D", "Terminal below"),
    ("⌘W", "Close the pane"),
];
const KEYS_MOVE: &[(&str, &str)] = &[
    ("⌘] / ⌘[", "Next / previous pane"),
    ("⌘⌃ arrows", "Pane in that direction"),
    ("⌘⌃ H J K L", "The same, on the home row"),
];
const KEYS_ARRANGE: &[(&str, &str)] = &[
    ("⌘⌥ arrows / H J K L", "Resize: move the split that way"),
    ("⌘⌃=", "Even out the splits"),
    ("⌘⌃0", "Fit the pane to its app's window"),
    ("⌘⇧⌥ arrows / H J K L", "Swap with the pane that way"),
    ("⌘⇧⌥1 … ⌘⇧⌥9", "Move the pane to a tab (US layout)"),
];
const KEYS_DRAG: &[(&str, &str)] = &[
    ("Drag a pane's header", "Onto a pane's edge: split there"),
    ("", "Onto its centre: swap the two"),
    ("", "Onto a tab: move it there · Esc cancels"),
    ("", "A live app is parked: click it to go back in"),
];
const KEYS_TABS: &[(&str, &str)] = &[
    ("⌘T", "New tab, with an empty pane: ⌘E picks what goes in it"),
    ("⌘1 … ⌘9", "Go to a tab"),
    ("⌘⇧] / ⌘⇧[", "Next / previous tab"),
    ("⌃Tab / ⌃⇧Tab", "Next / previous tab"),
    ("⌘⇧W", "Close the tab and its panes"),
    ("⌘⇧S", "Save the tab as a workbook; again updates it"),
    ("File menu", "Add the tab to a workbook · save all tabs as one"),
];
const KEYS_MARKS: &[(&str, &str)] = &[
    ("⌘B / ⌘⇧B", "Mark the pane / the tab; again unmarks"),
    ("⌘' then a letter", "Go to the mark with that letter"),
    ("⌘⌥1 … ⌘⌥9", "Go to one of the first nine marks"),
];
const KEYS_TERMINAL: &[(&str, &str)] = &[
    ("Wheel / trackpad", "Scroll back through history"),
    ("⇧PageUp / ⇧PageDown", "Scroll a page"),
    ("⇧Home / ⇧End", "Top / bottom of history"),
    ("Drag / 2× / 3× click", "Select text, a word, a line"),
    ("⌘C", "Copy the selection"),
    ("⌘V", "Paste"),
    ("⌘⇧C", "Visual mode: select with the keyboard"),
    ("⌘⌃E", "List mode: keys for the bar (Tasks, Apps, Workbook, Repos, Marks). ↵ or space on a heading folds it"),
    ("⌘; or ⌘⌃G", "Sneak mode: a letter label on every control"),
    ("⌘N", "New task: type a line, ↵ adds it. ↑↓ ↵ pick from its list; ⌘O adds files"),
    ("⌘⇧N", "Next task: show the tabs of the first that needs you"),
    ("⌘H", "Hide ottospace"),
    ("⌘Q", "Quit: asks first while a terminal runs"),
];
const KEYS_LIST: &[(&str, &str)] = &[
    ("J K / ↓ ↑", "Move the selection"),
    ("↵ / ⇧↵", "Open the row / open it below"),
    ("Tab / ⇧Tab", "Next / previous section"),
    ("S", "Task: snooze one hour"),
    ("H", "Task: put on hold"),
    ("X", "Task: dismiss from the inbox"),
    ("U", "Task: undo the last of these"),
    ("R / M / X / U", "Mark: rename / letter / remove / put back"),
    ("⇧J ⇧K", "Mark: move down / up"),
    ("R / X / U", "Workbook: rename / delete / undo; on a tab's row, that tab"),
    ("A / space", "Workbook: add this tab / list its tabs"),
    ("P / X / U", "App not running: pin or unpin / hide a suggestion / undo"),
    ("/", "Apps: filter all apps"),
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

/// The new-task line and the picker's filter.
const KEYS_FIELD: &[(&str, &str)] = &[
    ("⌥←/→ · ⌘←/→", "Move by word / to the line's ends"),
    ("⇧ + a move · ⌘A", "Select / select all"),
    ("⌥⌫ · ⌘⌫", "Delete a word / to the start"),
    ("⌘C ⌘X ⌘V", "Copy, cut, paste"),
    ("⌘Z / ⇧⌘Z", "Undo / redo"),
    ("Drag / 2× / 3× click", "Select text, a word, the line"),
];

const KEYS_SNEAK: &[(&str, &str)] = &[
    ("A S D F …", "Type a label: it does what a click would"),
    ("⌫", "Take a letter back"),
    ("Esc", "Leave; so does a letter of no label"),
];

/// Rail sections in order, with their icon and label.
const SECTIONS: [(Section, &str, &str); 6] = [
    (Section::Tasks, "icons/tasks.svg", "Tasks"),
    (Section::Calendar, "icons/calendar.svg", "Calendar"),
    (Section::Apps, "icons/apps.svg", "Apps"),
    (Section::Workbook, "icons/workbook.svg", "Workbook"),
    (Section::Repos, "icons/repos.svg", "Repos"),
    (Section::Marks, "icons/marks.svg", "Marks"),
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
        div().pl(self.offset.x + px(8.)).pt(self.offset.y + px(8.)).child(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(rgb(theme::RAISED))
                .border_1()
                .border_color(rgb(theme::PRIMARY))
                .text_xs()
                .text_color(rgb(theme::TEXT))
                .child(self.label.clone()),
        )
    }
}

/// A drawn part the state gives a rect for, so a pointer can be aimed at it: a pane's header,
/// its ×, the tab at a place on the bar.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Mark {
    Header(PaneId),
    Close(PaneId),
    Tab(usize),
    /// A repository's name in the Repos section, by its place in `model.repos`.
    Repo(usize),
    /// A checkout's row in the Repos section, by its place in the list.
    Checkout(usize),
    /// The left column's title strip, and the open section's header.
    Strip,
    BarHeader,
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

/// `[x, y, w, h]` in window points.
pub(crate) fn rect(b: Bounds<Pixels>) -> [f32; 4] {
    [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()]
}

actions!(
    ottospace,
    [
        SplitRight, SplitDown, PickPane, ClosePane, FocusLeft, FocusRight, FocusUp, FocusDown, FocusNext, FocusPrev,
        Zoom, NewTab, NextTab, PrevTab, CloseTab, ShowKeys, AxOverlay
    ]
);
// The picker for a pane below the focused one.
actions!(ottospace, [PickPaneDown]);
// Pane arrangement.
actions!(
    ottospace,
    [ResizeLeft, ResizeRight, ResizeUp, ResizeDown, SwapLeft, SwapRight, SwapUp, SwapDown, Equalize, FitPane]
);
// Terminal selection.
// The Edit menu items for a text field.
actions!(ottospace, [Cut, SelectAll, Undo, Redo]);
actions!(ottospace, [CopySelection, PasteText, VisualMode, ListMode, SneakMode, NewTask, NextTask]);
// Marks: mark the focused pane or the tab on show, and the jump prefix.
actions!(ottospace, [MarkPane, MarkTab, MarkJump]);
// Save the tab on show as a workbook, add it to one, or save all the tabs as one.
actions!(ottospace, [SaveWorkbook, AddTabToWorkbook, SaveAllWorkbook]);
// The next start is empty: the layout is not kept this run.
actions!(ottospace, [StartFresh]);
// What the capture key does, from the menu.
actions!(ottospace, [CapturePane]);
// The right drawer: the keyboard in or out, closed to the rail or open, the task's workspace.
actions!(ottospace, [TalkFocus, TalkToggle, TalkGo, StopAgent]);
// A tab of no task, and the tab on show taken out of its task or moved to another.
actions!(ottospace, [NewTabOutside, RemoveTabFromTask, MoveTabToTask]);
// App menu.
actions!(ottospace, [Quit, Hide]);

/// What a socket call files once its tags are read.
enum Filing {
    /// `task_new`: the line the reader was asked about. `open`: also show the task's tabs.
    Line { open: bool },
    /// `task_add`: its `agent` and `skills` are checked against the names read.
    Task { title: String, priority: u8, state: State, tab: Option<u32>, assign: Assign },
}

/// What a line in the new-task box may hold, and what its keys do: in the box and where the
/// Tasks section says how to add a task.
const TAG_HINT: &str = "#project · >repo · @agent · /path · $skill · ^open · p1–p9";
const KEYS_HINT: &str = "↵ start · ⇧↵ file for later · esc leaves";

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
    /// The right drawer: each task's agent, and where its conversation was drawn.
    drawer: talk::Drawer,
    /// One terminal character cell.
    cell: Size<Pixels>,
    /// The terminal pane a mouse drag is selecting in.
    dragging: Option<PaneId>,
    /// During a pane drag: the pane under the pointer and the zone of it a drop would land on.
    drop_at: Option<(PaneId, Zone)>,
    /// A `mouse` request holds the button down: a move before its release is a drag.
    held: bool,
    /// Releases a button no request came back for. Replaced, so stopped, by each `mouse` request.
    abandoned: Option<gpui::Task<()>>,
    /// The pointer event in hand came from a `mouse` or `drag` request, not from the user.
    injected: bool,
    /// Where headers, their × and tabs were last drawn.
    marks: Marks,
    /// The last text copied. E2E runs use it as the clipboard, so tests never touch the user's.
    copied: Option<String>,
    /// The open text field as last drawn: the new-task line, and a note's and the picker's own.
    field: field::Shared,
    /// What an ⌥-click would reference in the new-task box: outlined while ⌥ is held over it.
    pointing: Option<Bounds<Pixels>>,
    /// Where the pointer last was: ⌥ pressed with it still outlines what is there.
    hover_at: gpui::Point<Pixels>,
    note_field: field::Shared,
    picker_field: field::Shared,
    apps_field: field::Shared,
    mark_field: field::Shared,
    inspector_field: field::Shared,
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
    /// A global key that could not be registered, for the status bar.
    key_notice: Option<String>,
    system_clipboard: bool,
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
    /// What the home screen last drew, and the lattice lines kept between frames.
    home: home::Shared,
    /// ottospace's own controls as last painted: what sneak mode labels.
    controls: sneak::Controls,
    /// The key that ended sneak mode is still down: its repeats are typed nowhere.
    sneak_spent: bool,
    /// Reads of the focused app pane's tree while sneak mode is on.
    sneak_reads: sneak_apps::Reads,
    /// The scroll that started a peek of a parked app pane, and counts.
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
    kept_layout: layout_file::Kept,
    /// Scroll position of the bar's list, so list mode can bring the selected row into view.
    list_scroll: ScrollHandle,
    /// Every scroll area's bar.
    bars: scrollbar::Bars,
    /// The minute of the last look at the clock: the Tasks section's ages are drawn again when it changes.
    minute: u64,
    /// Reads what tags may name, on a thread of its own.
    tags: Option<tags::Reader>,
    /// The repository the agents and skills were last asked for, while the new-task box is open.
    tags_repo: Option<Option<String>>,
    /// Socket calls that file a task and wait for the reader: (the call's number, what to file, where to reply).
    filing: Vec<(u64, Filing, Sender<String>)>,
    /// How many such calls there were: each has its own number.
    calls: u64,
    /// Reads agent sessions on a thread of its own. `None`: the settings turn it off.
    sessions: Option<sessions::Poller>,
    /// The core holds sessions that were found: they are cleared when the Tasks section closes.
    sessions_listed: bool,
    /// The core was told the `session_quiet_minutes` setting.
    sessions_quiet_told: bool,
    /// The user was told that the sessions reader stopped.
    sessions_stopped: bool,
    /// Window ids seen per hosted pid; anything new that appears later gets its own pane.
    known: Vec<(i32, HashSet<u32>)>,
    /// When the live pane went live.
    live_since: Option<Instant>,
    /// The app a click in a live pane's gap took the keyboard from, and when.
    left_by_click: Option<(i32, Instant)>,
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
    /// False in e2e runs, which must never move the user's real windows.
    hosting: bool,
    /// Test runs: the pid of the one app that may be hosted.
    host_only: Option<i32>,
    control: Option<Receiver<Call>>,
    /// The accessibility overlay's last read. Drawn while `model.ax_overlay` is on.
    ax_overlay: Option<ax_debug::Overlay>,
    /// The meeting timeline's feed reader and what it draws.
    timeline: calendar::Timeline,
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

/// How long the `s` key snoozes a task: one hour.
const SNOOZE_MS: u64 = 3_600_000;

/// Longest a `wait` request may run: 10 minutes.
const MAX_WAIT_MS: u64 = 600_000;

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
                // The layout first, while its panes and terminals are all there.
                this.store_layout(true);
                this.hosted.drain().for_each(|(_, hosted)| {
                    hosted.give_back();
                });
                if this.control.is_some() {
                    // The socket and the lock file `control::serve` made beside it.
                    let _ = std::fs::remove_file(control::path());
                    let _ = std::fs::remove_file(control::path().with_extension("lock"));
                }
                running::remove();
                async {}
            }),
        ];

        // ~60Hz tick: answer control calls, redraw on new frames, keep windows matched to panes.
        // Every ~500ms, also look for new and closed windows of hosted apps.
        cx.spawn_in(window, async move |this, cx| {
            for tick in 0u64.. {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if give_back::signalled() {
                        cx.quit();
                    }
                    this.serve_control(window, cx);
                    this.take_tags(window, cx);
                    this.store_layout(false);
                    if tick % 15 == 0 {
                        this.poll_sessions(window, cx);
                    }
                    if hotkey::take_press() {
                        this.return_to_ottospace(window, cx);
                    }
                    if hotkey::take_capture() {
                        // From any app: ottospace comes forward with the picture to draw on.
                        this.error = this.capture_pane(None, Origin::User, window, cx).err();
                        cx.activate(true);
                        cx.notify();
                    }
                    this.with_origin(Origin::System, |this| {
                        if tick % 30 == 0 {
                            this.adopt_new_windows(window, cx);
                            this.drop_closed(window, cx);
                            this.follow_front_app(window, cx);
                            this.host_launched(window, cx);
                            this.refresh_inspector(window, cx);
                            // A snoozed task returns when its time comes, and the offer to undo goes.
                            let now = now_ms();
                            if this.model.due(now) {
                                this.dispatch(Action::Tick { now }, Origin::System, window, cx);
                                cx.notify();
                            }
                            // An age shown ("5m") changes at most once a minute: draw again then,
                            // while the Tasks section is open to show one.
                            let minute = now / 60_000;
                            if minute != this.minute && this.model.sidebar == Some(Section::Tasks) {
                                cx.notify();
                            }
                            this.minute = minute;
                            // The Marks list says what is missing as it is now.
                            if matches!(this.model.sidebar, Some(Section::Marks | Section::Workbook)) {
                                this.check_marks(cx);
                            }
                        }
                        // Tell the core which windows kept another size than their pane's. Not
                        // before the panes were drawn again after the last change: until then
                        // the sizes are those of the layout before it.
                        let drawn = this.renders > this.dispatched_at;
                        for (pane, content, outer, kept) in this.sync_layout(window, cx, tick % 30 == 15) {
                            if drawn && this.model.wants_changed(pane, content, outer, kept) {
                                let wants = Action::PaneWants { pane, content, outer, window: kept };
                                this.dispatch(wants, Origin::System, window, cx);
                            }
                        }
                        this.sync_terminals(window, cx);
                        this.sync_agents(window, cx);
                        // The header's time goes on while an agent runs.
                        let runs = this.model.talk.as_ref().is_some_and(|t| t.open && this.model.agent_runs(&t.task));
                        if runs && tick % 30 == 0 {
                            cx.notify();
                        }
                        if tick % 60 == 0 {
                            this.seek_sessions(window, cx);
                        }
                        this.sync_sneak(window, cx);
                        this.read_sneak_app(window, cx);
                        if tick % 4 == 0 {
                            this.check_peek(window, cx);
                        }
                        this.tick_timeline(window, cx);
                    });
                    this.take_snapshots(window, cx);
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
        model.update(Action::SetRepoDirs { dirs: config::repo_dirs() }.sent_by(Origin::System));
        // The Tasks section as the user last grouped it.
        let view = config::get().settings.tasks_view.unwrap_or_default();
        model.update(Action::TasksView { view }.sent_by(Origin::System));
        let range = config::get().settings.sessions_range.unwrap_or_default();
        model.update(Action::SessionsRange { range }.sent_by(Origin::System));
        let settings = &config::get().settings;
        let (on, notify) = (settings.calendar_ics.is_some(), settings.calendar_notify.unwrap_or(true));
        let links_only = settings.calendar_notify_for == Some(ottospace_core::settings::NotifyFor::Links);
        model.update(Action::SetCalendar { on, notify, links_only }.sent_by(Origin::System));
        model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        let kept = config::load_tasks();
        let unread = kept.unread;
        // A folder that could not be listed is left alone: writing could replace tasks it holds.
        let keeping = kept.error.is_none();
        model.update(Action::RestoreTasks { tasks: kept.tasks, floor: kept.highest }.sent_by(Origin::System));
        // The first problem found at start, for the status bar: settings, the font, then tasks.
        let unread = (unread > 0).then(|| format!("tasks: {unread} file(s) in the tasks folder could not be read"));
        let notice = config::get().error.clone().or(terminal::choose_font(window)).or(kept.error).or(unread);
        let mut this = Self {
            model,
            hosted: HashMap::new(),
            terminals: HashMap::new(),
            drawer: talk::Drawer::default(),
            cell: terminal::cell_size(window),
            dragging: None,
            drop_at: None,
            held: false,
            abandoned: None,
            injected: false,
            marks: Marks::default(),
            copied: None,
            field: field::Shared::default(),
            pointing: None,
            hover_at: gpui::Point::default(),
            note_field: field::Shared::default(),
            picker_field: field::Shared::default(),
            apps_field: field::Shared::default(),
            mark_field: field::Shared::default(),
            inspector_field: field::Shared::default(),
            shots: HashMap::new(),
            annotate_at: Rc::default(),
            bar_at: Rc::default(),
            briefs: HashMap::new(),
            no_brief: Default::default(),
            pending: Vec::new(),
            visual_after: Vec::new(),
            key_notice: None,
            system_clipboard: std::env::var_os("OTTOSPACE_E2E").is_none(),
            cells: HashMap::new(),
            frames: HashMap::new(),
            renders: 0,
            dispatched_at: 0,
            seq: 0,
            drawn_seq: 0,
            settles: std::env::var_os("OTTOSPACE_E2E").is_some(),
            settling: Vec::new(),
            frame_start: Rc::default(),
            home: home::Shared::default(),
            controls: sneak::Controls::default(),
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
            bars: scrollbar::Bars::default(),
            minute: 0,
            tags: tags::Reader::start(),
            tags_repo: None,
            filing: Vec::new(),
            calls: 0,
            sessions: sessions::Poller::start(),
            sessions_listed: false,
            sessions_stopped: false,
            sessions_quiet_told: false,
            known: Vec::new(),
            live_since: None,
            left_by_click: None,
            went_live: 0,
            left_on_activation: None,
            front_hosted: None,
            wanted: None,
            error: None,
            notice,
            keeping,
            unsaved: None,
            stored: Vec::new(),
            kept_marks: marks::Kept::default(),
            kept_workbooks: workbook::Kept::default(),
            kept_layout: layout_file::Kept::default(),
            hosting,
            host_only,
            control,
            ax_overlay: None,
            timeline: calendar::Timeline::start(),
            focus,
            _subscriptions: subscriptions,
        };
        this.tell_app_settings();
        this.refresh_apps();
        this.offer_installed(cx);
        this.stored = this.kept_tasks(); // what was read needs no writing back
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
        std::mem::replace(&mut self.input, origin)
    }

    /// Runs `f` with `origin` as the sender, then puts the previous sender back.
    fn with_origin<R>(&mut self, origin: Origin, f: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.swap_origin(origin);
        let result = f(self);
        self.input = before;
        result
    }

    /// The one path that changes state: updates the model, then carries out its effects.
    /// `origin`: who sent the action. The person's own keys and clicks pass `self.input`.
    fn dispatch(&mut self, action: Action, origin: Origin, window: &Window, cx: &mut Context<Self>) -> Vec<Effect> {
        // The time it happens at: a task done now is done now, not at the last read.
        let now = now_ms();
        if now.saturating_sub(self.model.now()) >= 1_000 && !matches!(action, Action::Tick { .. }) {
            self.dispatch(Action::Tick { now }, Origin::System, window, cx);
        }
        // Opening the Repos section reads the list again: repositories come and go on disk.
        let opens_repos = self.model.sidebar != Some(Section::Repos);
        if opens_repos && matches!(action, Action::ToggleSection { section: Section::Repos }) {
            self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
        }
        // Opening the Apps section too: apps start and quit, and the Dock changes.
        if self.model.sidebar != Some(Section::Apps)
            && matches!(action, Action::ToggleSection { section: Section::Apps })
        {
            self.refresh_apps();
            self.offer_installed(cx);
        }
        // A client's key, click or label at the user's draft: the status bar says why it did nothing.
        let draft = ottospace_core::capture::DRAFT_USERS;
        if origin == Origin::Socket && self.model.refusal(&action, origin) == Some(draft) {
            self.error = Some(draft.into());
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
        if files || row || unmarks || pins || captures {
            self.model.update(Action::Tick { now: now_ms() }.sent_by(Origin::System));
        }
        let name = trace::name(&action);
        let effects = self.model.update(action.sent_by(origin));
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
        // What an effect dispatches in turn (a picked app's pane, a label's control) has the same sender.
        self.with_origin(origin, |this| this.carry_out(&effects, window, cx));
        cx.notify();
        effects
    }

    fn carry_out(&mut self, effects: &[Effect], window: &Window, cx: &mut Context<Self>) {
        // A peek's app takes the keyboard too.
        let another_goes_live = effects.iter().any(|effect| matches!(effect, Effect::Live(_) | Effect::Peek(_)));
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
                            self.error = Some(format!("agent: {why}"));
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
                Effect::AgentInterrupt { task } => self.native_input(task, NativeInput::Interrupt),
                Effect::AgentMoved { from, to } => self.move_agent(from, to),
                Effect::MeetingAlert { .. } | Effect::OpenLink { .. } => self.meeting_effect(effect),
                Effect::ReadSession { runner, id } => agents.extend(self.read_session(runner, id)),
                Effect::Opened(id) => {
                    if self.model.pane(*id).is_some_and(|p| p.kind == Kind::Terminal) {
                        // The shell learns where it runs, so a program in it can name its own pane.
                        let tab = self.model.tab_of(*id);
                        let mut vars = vec![("OTTOSPACE_PANE", id.to_string())];
                        vars.extend(tab.map(|tab| ("OTTOSPACE_TAB", tab.to_string())));
                        // A terminal opened for a task names it: an agent there can ask for its brief.
                        let task = self.model.tabs.iter().find(|t| Some(t.id) == tab).and_then(|t| t.task);
                        vars.extend(task.map(|task| ("OTTOSPACE_TASK", task.to_string())));
                        let cwd = self.model.pane(*id).and_then(|pane| pane.cwd.as_deref());
                        // A workbook's terminal shows what it showed when saved: the core says so next.
                        let history = effects.iter().find_map(|effect| match effect {
                            Effect::History { pane, history } if pane == id => Some(history),
                            _ => None,
                        });
                        let spawned = Instant::now();
                        match Terminal::spawn(&vars, cwd, history) {
                            Ok(terminal) => drop(self.terminals.insert(*id, terminal)),
                            Err(err) => self.error = Some(format!("terminal: {err}")),
                        }
                        let (took, ok) = (spawned.elapsed().as_millis() as u64, self.terminals.contains_key(id));
                        trace::record("terminal_spawn", serde_json::json!({ "pane": id, "took_ms": took, "ok": ok }));
                    }
                }
                Effect::Closed(pane) => {
                    self.cells.remove(&pane.id);
                    self.frames.remove(&pane.id);
                    self.marks
                        .borrow_mut()
                        .retain(|mark, _| ![Mark::Header(pane.id), Mark::Close(pane.id)].contains(mark));
                    self.terminals.remove(&pane.id); // drop ends its shell
                    if let Some(hosted) = self.hosted.remove(&pane.id) {
                        hosted.release();
                    }
                }
                Effect::Parked(id) => {
                    self.live_since = None;
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        trace::record("park", serde_json::json!({ "pane": id, "window": hosted.window_id }));
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
                Effect::LaunchApp { name, path, axis, new_tab } => self.launch_picked(name, path, *axis, *new_tab),
                Effect::SaveApps { changes } => self.save_apps(changes),
                Effect::SaveTasksView { view } => {
                    if let Err(why) = config::save_value("tasks_view", &serde_json::json!(view)) {
                        self.error = Some(format!("{}{why}", Self::NOT_WRITTEN));
                    }
                }
                Effect::SaveSessionsRange { range } => {
                    if let Err(why) = config::save_value("sessions_range", &serde_json::json!(range)) {
                        self.error = Some(format!("{}{why}", Self::NOT_WRITTEN));
                    }
                }
                Effect::SaveRepos { changes } => self.save_apps(changes),
                Effect::Notice { text } => self.error = Some(text.clone()),
                // A picture of the pane is asked for: the cursor's going scrolls the terminal to
                // its bottom, so it waits until the picture of what is on show is taken.
                Effect::Visual { pane, on: false } if self.snapshot_pending(*pane) => self.visual_after.push(*pane),
                Effect::Visual { pane, on } => {
                    if let Some(terminal) = self.terminals.get(pane) {
                        terminal.set_visual(*on);
                    }
                }
                Effect::Activate { id } => self.activate(id, window, cx),
                Effect::Run { command } => self.run_command(*command, window, cx),
                Effect::PressApp { pane, path, role, at, .. } => self.press_app(*pane, path, role, *at, window, cx),
                Effect::Peek(pane) => self.peek(*pane, window, cx),
                Effect::HostWindow { mark, bundle, name, title, into, axis, launch } => {
                    let (bundle, name, title) = (bundle.clone(), name.clone(), title.clone());
                    let (mark, into, axis, launch) = (*mark, *into, *axis, *launch);
                    let wanted = marks::Wanted { mark, bundle, name, title, into, axis, launch };
                    if let Err(why) = self.host_window(wanted, window, cx) {
                        // A rebuilt tab's pane stays empty, and says why.
                        if let Some(pane) = into {
                            self.model.update(Action::Unfilled { pane, why: why.clone() }.sent_by(Origin::System));
                        }
                        if !unhosted.contains(&why) {
                            unhosted.push(why);
                        }
                    }
                }
                // Carried out with its pane's `Opened`, before the shell starts.
                Effect::History { .. } => {}
                Effect::ForgetLayout => self.forget_layout(),
                Effect::Snapshot { draft, material, pane } => self.ask_snapshot(draft, *material, *pane),
                Effect::CaptureDropped { draft, material } => self.drop_capture(draft, *material),
                Effect::CaptureKept { draft, dir, brief } => self.keep_capture(draft, dir, brief, cx),
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
                    self.live_since = Some(Instant::now());
                    self.went_live += 1;
                    if let Some(hosted) = self.hosted.get_mut(id) {
                        trace::record("unpark", serde_json::json!({ "pane": id, "window": hosted.window_id }));
                        hosted.bring_up(window, cx);
                    }
                }
            }
        }
        if asked {
            self.error = (!unhosted.is_empty()).then(|| unhosted.join("; "));
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
            let whole = self.model.compose_attached().to_vec();
            reader.ask(tags::Job { call: 0, line, typing, whole, repo, names });
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
        reader.ask(tags::Job { call: self.calls, line, typing: None, whole: Vec::new(), repo, names: true });
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

    /// What the status bar says is wrong, most pressing first: the last request that failed, a
    /// task that is not saved, then a problem found at start.
    fn problem(&self) -> Option<String> {
        // What is wrong with `marks.json` has its own place beside a notice from the start.
        let start = match (&self.notice, &self.kept_marks.notice) {
            (Some(notice), Some(marks)) => Some(format!("{notice} · {marks}")),
            (notice, marks) => notice.clone().or(marks.clone()),
        };
        let start = start.or(self.key_notice.clone()).or(self.kept_workbooks.notice.clone());
        let start = start.or(self.kept_layout.notice.clone());
        let unsaved = self.unsaved.clone().or(self.kept_marks.unsaved.clone()).or(self.kept_workbooks.unsaved.clone());
        let unsaved = unsaved.or(self.kept_layout.unsaved.clone());
        self.error.clone().or(unsaved).or(start)
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

    /// Replies with the state. In a test run, not before a frame has drawn every action so far,
    /// so the bounds in the reply are of the new layout, never the one before it.
    fn answer(&mut self, reply: Sender<String>, window: &Window, cx: &mut Context<Self>) {
        if self.settles && self.drawn_seq < self.seq {
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
        let drawn = self.drawn_seq >= self.seq;
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
        for Call { request, reply } in calls {
            if let Some(why) = control::test_refusal(&request, std::env::var_os("OTTOSPACE_E2E").is_some()) {
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
                Request::Mouse { action, x, y, clicks, modifiers, dx, dy, precise, .. } => {
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
                    let event = self.pointer(*action, mouse);
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
                Request::MovePane { tab, .. } if tab >= self.model.tabs.len() => {
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
                Request::SelectTab { index } => Some(Action::SelectTab { index }),
                Request::CycleTab { delta } => Some(Action::CycleTab { delta }),
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
                    if self.model.tabs.iter().any(|t| t.id == tab && t.task.is_some()) =>
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
                // A test run only (refused above otherwise): the folder the user's picker would give.
                Request::RepoAdd { path } => {
                    self.add_repo(Path::new(&path), Origin::User, window, cx);
                    None
                }
                // A test run only (refused above otherwise): the user's capture key, pressed by the test.
                Request::CaptureStart { pane } => {
                    failure = self.capture_pane(pane, Origin::User, window, cx).err();
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
                    let _ = reply.send(self.serve_brief(id, tab));
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
                Request::AxTree { pane, depth, nodes } => {
                    let source = self.ax_source(pane.or(self.model.focused));
                    ax_debug::serve_tree(source, depth, nodes, reply, cx.background_executor());
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
                Request::CalendarCall { in_call } => Some(Action::CallSeen { in_call, at: now_ms() }),
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
                let picked = |e: &Effect| matches!(e, Effect::HostApp { .. } | Effect::LaunchApp { .. });
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

    /// The Repos section as laid out: each repository's `name` and `path` with its `header`, and
    /// its `rows` (a checkout's `path`, `branch`, `default`, `label`) each with its `row`. Rects
    /// are in window points, scrolled out of view or not; null while the section is not drawn.
    fn repos_bar(&self) -> Vec<serde_json::Value> {
        let marks = self.marks.borrow();
        let drawn = |mark: Mark| marks.get(&mark).copied().map(rect);
        let mut index = 0;
        let group = |(n, repo): (usize, &Repo)| {
            let row = |checkout: Checkout| {
                let mut row = serde_json::json!(checkout);
                row["row"] = serde_json::json!(drawn(Mark::Checkout(index)));
                index += 1;
                row
            };
            let rows: Vec<serde_json::Value> = repo.checkouts().into_iter().map(row).collect();
            serde_json::json!({ "name": repo.name, "path": repo.path, "header": drawn(Mark::Repo(n)), "rows": rows })
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
        let open = self.model.sidebar == Some(Section::Tasks);
        if !self.sessions_quiet_told {
            self.sessions_quiet_told = true;
            let ms = config::session_quiet_ms();
            self.dispatch(Action::SessionQuiet { ms }, Origin::System, window, cx);
            // Told with it, once: the models and the default agent a task starts with.
            let settings = &config::get().settings;
            let models = ottospace_core::agents::Models::with(settings.agent_models.as_ref());
            let agents = Action::SetAgents {
                claude: models.claude,
                codex: models.codex,
                default: settings.default_agent.clone(),
                shows: settings.conversation_view.unwrap_or_default(),
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
        for work in worked {
            self.dispatch(Action::SessionWork { work }, Origin::System, window, cx);
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
            self.dispatch(Action::SetSessions { found, day: day_start(now) }, Origin::System, window, cx);
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
        if self.model.confirm.is_some() {
            return self.confirm_answer(key, Origin::Socket).is_none().then_some(CONFIRM_UP);
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
                this.input = before.unwrap_or(Origin::User);
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
            let _ = this.update(cx, |this, _| this.input = before.unwrap_or(Origin::User));
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
                this.input = before.unwrap_or(Origin::User);
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
        let marks = self.marks.borrow();
        let marked = |key: fn(PaneId) -> Mark| -> BTreeMap<PaneId, [f32; 4]> {
            drawn.iter().filter_map(|id| Some((*id, rect(*marks.get(&key(*id))?)))).collect()
        };
        let tabs: Vec<[f32; 4]> =
            (0..self.model.tabs.len()).filter_map(|place| marks.get(&Mark::Tab(place)).copied().map(rect)).collect();
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
                    "selection": t.selected(),
                    "visual_cursor": t.visual_cursor(),
                });
                (*id, terminal)
            })
            .collect();
        serde_json::json!({
            "ok": true,
            "mode": self.model.mode(),
            // Where the keyboard is: a pane, the list, a rail's cell, the drawer.
            "focus": self.model.focus_target(),
            "state": self.model,
            "bounds": bounds,
            // Each drawn pane's bounds inside its border.
            "frames": frames,
            // Each drawn pane's header (the handle a drag moves it by) and the × in it, and each
            // tab on the bar by place.
            "headers": marked(Mark::Header),
            "closes": marked(Mark::Close),
            "tabs": tabs,
            // The left column's title strip, and the open section's header (null: none open).
            "strip": marks.get(&Mark::Strip).copied().map(rect),
            "bar_header": marks.get(&Mark::BarHeader).copied().map(rect),
            // While a pane is dragged: the pane a drop would land on and the zone of it, both
            // null over anything else. Null with no drag.
            "drag": cx.has_active_drag().then(|| serde_json::json!({
                "target": self.drop_at.map(|(target, _)| target),
                "zone": self.drop_at.map(|(_, zone)| zone),
            })),
            // Each drawn app pane's preview: the part of the window it shows, where it is drawn.
            "shown": shown,
            "tab_panes": self.model.tab_panes(),
            // ottospace's own window frame, so a test can turn pane bounds into screen positions.
            "window": frame,
            // Whether that window has the keyboard: a user's wheel over a parked app pane peeks only then.
            "window_active": window.is_window_active(),
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
            "compose_picks": self.model.compose_picks(),
            // Nothing is open yet: the drawer shows the new task by itself.
            "welcome": self.model.welcome(),
            // The dropdown that finishes the tag at the cursor, or null: its `mark`, what is
            // `typed`, the `rows` (name, kind, detail, matched letters) and the one `selected`.
            "compose_menu": self.model.dropdown(),
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
            // The pinned apps that are not running: `open_app` starts one. The Dock's and the
            // rest are in `apps_bar`: those are the user's to start.
            "closed_apps": self.pinned_closed().map(|app| app.name).collect::<Vec<_>>(),
            // The Apps section below the running apps: `not_running` (each with `name`, `path`,
            // `pinned`, `suggested`), `more` behind "and N more", `all` (every other installed
            // app, while `all_open`; `all_count` of them) narrowed by `filter`, and `undo`.
            "apps_bar": self.model.apps_bar(),
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
                "frame_reads": self.hosted.values().map(|h| h.frame_reads).sum::<u64>(),
                // Scrolls over an app pane's picture, peeks they started, and the one on now (its pane; 0:
                // none).
                "wheels": self.wheel.over,
                "peeks": self.wheel.peeks,
                // Scrolls that asked for none: with the left button down, or the window not active.
                "wheels_held": self.wheel.held,
                "wheels_inactive": self.wheel.inactive,
                "peek": self.model.peek.as_ref().map_or(0, |peek| peek.pane),
            })),
            // Test runs only: each press in an app a sneak label asked for, `[pane, path]`; else null.
            "app_presses": std::env::var_os("OTTOSPACE_E2E").map(|_| &self.sneak_reads.presses),
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
            // What the Workbook section offers to take back (`deleted`, `updated`), or null.
            "workbook_offer": self.model.workbooks.offered(),
            // Each scroll area whose content does not fit, by its id (`tasks`, `repos`, `apps`,
            // `marks`, `picker`, `keys`, `terminal:<pane>`): its bar's `track` and `thumb` in
            // window points, and the bar's `opacity` (0 once it has faded).
            "scrollbars": self.bars.snapshot(),
            // The capture key, and what was drawn of the draft or the brief in the right bar.
            "capture": self.capture_snapshot(),
            // The right drawer: where its conversation is drawn (`body`), each task's agent
            // (`model`, `ended`, its terminal's `lines`), the workspace ⌘↵ goes to, and why no
            // agent runs for the task on show (`note`).
            "drawer": self.talk_snapshot(),
            // The tab bar's groups (`task`, `tabs` as places), and each live task's badge letters.
            "groups": self.model.groups(),
            "badges": self.model.badges(&[]).into_iter().filter_map(|(subject, mono)| Some((subject.task()?.to_string(), serde_json::Value::from(mono)))).collect::<serde_json::Map<String, serde_json::Value>>(),
            // The meeting timeline: counts, times and stages only, never a title or a place.
            "calendar": self.calendar_snapshot(),
            // Frames drawn (`number`), actions dispatched (`seq`) and how many of them the last
            // frame drew (`drawn`); `waited_ms`: how long a test run's reply waited for that frame.
            // `window`: the window's number, for `screencapture -l`.
            "frame": {
                "number": self.renders, "seq": self.seq, "drawn": self.drawn_seq, "waited_ms": 0,
                "window": ns_view(window).map(|view| unsafe { ottospace_mac::window::number(view) }),
            },
        })
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
            let ids = ax::Window::standard_of(pid).iter().filter_map(ax::Window::id).collect();
            self.known.push((pid, ids));
        }
        let effects = self.dispatch(action, self.input, window, cx);
        if let Some(Effect::Opened(id)) = effects.last() {
            // A pane in a background tab is not drawn, so the layout sync never parks its
            // window. Move it off screen now; it is sized when its tab is first shown.
            let drawn = self.model.layout.as_ref().is_some_and(|l| l.leaves().contains(id));
            if !drawn {
                let _ = hosted.window.set_position(display::parking_spot());
            }
            if hosted.off_screen {
                self.error =
                    Some(format!("{}'s window was off screen: it goes back to the middle of the screen", hosted.app));
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
            self.dispatch(Action::Focus { pane }, self.input, window, cx);
            return Ok(());
        }
        let hosted = Hosted::new(target)?;
        // The window is in hand, so the tab will not be left empty.
        if new_tab {
            self.dispatch(Action::NewTab, self.input, window, cx);
        }
        let action = Action::Open { kind: hosted.kind(), axis, beside: None };
        self.attach(hosted, action, window, cx);
        Ok(())
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
            let hosted = app.is_some_and(|app| match &launch.wanted {
                Some(wanted) => settled && self.host_window_of(app.pid, wanted, window, cx).is_ok(),
                None => self.with_origin(launch.origin, |this| {
                    this.open_app(&app.name, launch.axis, launch.new_tab, window, cx).is_ok()
                }),
            });
            if hosted {
                continue;
            }
            if launch.since.elapsed() > LAUNCH_WAIT {
                let seconds = LAUNCH_WAIT.as_secs();
                let why = format!("{} did not open a window in {seconds} seconds", launch.app.name);
                // A rebuilt tab's pane that waited for it says so.
                if let Some(pane) = launch.wanted.as_ref().and_then(|wanted| wanted.into) {
                    self.model.update(Action::Unfilled { pane, why: why.clone() }.sent_by(Origin::System));
                }
                self.error = Some(why);
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
            // An app in front within a second of a click that left it: its own late activation
            // from going live, not the user's choice.
            let left = self.left_by_click.filter(|(_, at)| at.elapsed() < Duration::from_secs(1));
            if left.is_some_and(|(pid, _)| front.is_some_and(|(front, _)| front == pid)) {
                self.wanted = None;
            }
        }
        let Some(pane) = self.wanted else { return };
        // Already the one on screen; or a peek, which activated the app itself.
        if self.model.live == Some(pane) || self.model.peek.as_ref().is_some_and(|peek| peek.pane == pane) {
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
        for id in gone {
            eprintln!("pane {id}: window gone; closing");
            // Nothing to put back, so stop the stream here instead of `release` in `dispatch`.
            if let Some(hosted) = self.hosted.remove(&id) {
                trace::record("window_gone", serde_json::json!({ "pane": id, "window": hosted.window_id }));
                let _ = hosted.stream.stop_capture();
                give_back::forget(hosted.window_id);
                // Its app may still be frontmost with no window here: take the keyboard back.
                let peeked = self.model.peek.as_ref().is_some_and(|peek| peek.pane == id);
                if (self.model.live == Some(id) || peeked) && ax::is_frontmost(hosted.pid) {
                    cx.activate(true);
                }
            }
            self.dispatch(Action::Close { pane: Some(id) }, Origin::System, window, cx);
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
    /// when the pane moves, and every ~500ms (`check`) if the app or user moved it away. A check
    /// also reads each window's size, for an app that resized it by itself.
    /// On a `check`, returns each settled pane's content size, whole size and window size.
    fn sync_layout(&mut self, window: &Window, cx: &App, check: bool) -> Vec<(PaneId, [f32; 2], [f32; 2], [f32; 2])> {
        let mut measured = Vec::new();
        // Only panes of the active tab are drawn; a background pane's bounds are stale.
        let drawn = self.model.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let scale = window.scale_factor() as f64;
        // A peeked window is kept over its pane as the live one is.
        let live = self.model.live.or(self.model.peek.as_ref().map(|peek| peek.pane));
        let limit = top_limit(window, cx);
        for (id, hosted) in &mut self.hosted {
            let rect = on_screen(hosted.bounds.get(), window, cx);
            if rect.size.width < 1.0 {
                continue; // not laid out yet
            }
            let crop = hosted.crop_at(rect, limit);
            if live == Some(*id) {
                let drifted = check && hosted.window_frame().is_some_and(|f| f.origin != raised(rect, crop).origin);
                if hosted.placed != rect || drifted {
                    hosted.place(rect, crop);
                }
            } else if hosted.parked_size != rect.size || hosted.crop != crop {
                hosted.park(scale, crop);
            }
            // Only on a check: reading a window's size is a synchronous accessibility call.
            // An app that resized its own window: the preview and the click rule follow it.
            // The window was asked for the pane's size: what it has now is what it kept.
            if check
                && let Some(kept) = hosted.show(scale)
                && let Some(frame) = self.frames.get(id)
                && drawn.contains(id)
            {
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
        // The key reference takes the keyboard: Esc closes it, and no other key reaches the
        // terminal or the picker behind it. (⌘ shortcuts go to their bindings; the core ignores
        // pane actions while it is open, and ⌘/ closes it.)
        if self.model.keys {
            let plain = !(key.modifiers.platform || key.modifiers.control || key.modifiers.alt);
            if key.key == "escape" {
                self.dispatch(Action::CloseKeys, self.input, window, cx);
            } else if let Some(step) = key_step(&key.key, key.modifiers.shift).filter(|_| plain) {
                // The list is longer than a small window: these keys scroll it.
                self.bars.step("keys", step);
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        // Esc during a pane drag cancels it, and a `mouse` request's button is up again.
        if key.key == "escape" && cx.has_active_drag() {
            self.held = false;
            cx.stop_active_drag(window);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        // ⌘⇧⌥1..9 move the focused pane to a tab. GPUI gives no physical key, so the shifted
        // characters are trusted on a US layout only (see `move_to_tab_key`).
        let m = key.modifiers;
        let us_layout = matches!(cx.keyboard_layout().id(), "com.apple.keylayout.US" | "com.apple.keylayout.ABC");
        let place = keys::move_to_tab_key(&key.key, m.shift, us_layout);
        if let Some(tab) = place.filter(|_| m.platform && m.alt && !m.control) {
            self.dispatch(Action::MovePane { pane: None, tab }, self.input, window, cx);
            cx.stop_propagation();
            return;
        }
        // ⌘⌥1..9 go to one of the first nine marks.
        let digit = key.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d));
        if let Some(digit) = digit.filter(|_| m.platform && m.alt && !m.shift && !m.control) {
            let _ = self.go_to_mark(MarkRef::Place(digit - 1), Axis::Right, window, cx);
            cx.stop_propagation();
            return;
        }
        // ⌘⌃1..⌘⌃9 show one of the right rail's conversations.
        if let Some(digit) = digit.filter(|_| m.platform && m.control && !m.alt && !m.shift) {
            self.rail_key(digit - 1, window, cx);
            cx.stop_propagation();
            return;
        }
        // ⌘1..⌘9 select a tab.
        if key.modifiers.platform
            && let Some(digit) = key.key.parse::<usize>().ok().filter(|d| (1..=9).contains(d))
        {
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
        let Some(terminal) = self.model.focused.and_then(|id| self.terminals.get(&id)) else { return };
        let key = &event.keystroke;
        let m = key.modifiers;
        let mods = crate::mods(m);
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
        // While a picture is annotated a click on another pane captures it: it selects nothing.
        if self.model.annotated().is_some() {
            return;
        }
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
                Action::ListSection { delta }
            }
            Some(ListKey::Snooze) => Action::ListSnooze { until: now_ms().saturating_add(SNOOZE_MS) },
            Some(ListKey::Hold) => Action::ListHold,
            Some(ListKey::Dismiss) => Action::ListDismiss,
            Some(ListKey::Undo) => Action::ListUndo,
            Some(ListKey::Pin) => Action::ListPin,
            Some(ListKey::Filter) => Action::AppsFilter { on: true },
            Some(ListKey::Leave) => Action::LeaveList,
            // On a rail ← and → cross zones; on a row they do nothing.
            Some(ListKey::Side(side)) if self.model.rail_focus.is_some() => {
                Action::FocusDir { dir: if side < 0 { Dir::Left } else { Dir::Right } }
            }
            Some(ListKey::Side(_)) | None => return,
        };
        self.dispatch(action, self.input, window, cx);
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
                    rows += repo.checkouts().len();
                    starts_before
                });
                return Some(at + headings.count());
            }
            Some(Section::Apps) => return Some(self.apps_child(at)),
            // The offer to put a mark back, if it shows, is above the rows.
            Some(Section::Marks) => return Some(at + usize::from(self.mark_offered().is_some())),
            Some(Section::Workbook) => return Some(at + usize::from(self.model.workbooks.offered().is_some())),
            // "Nothing today" is above the rows when the day is empty.
            Some(Section::Calendar) => return Some(at + usize::from(self.model.calendar.today().next().is_none())),
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

    /// Reads the Apps and Repos lists again.
    fn refresh_lists(&mut self, cx: &mut Context<Self>) {
        self.refresh_apps();
        self.offer_installed(cx);
        self.model.update(Action::SetRepos { repos: repos::scan() }.sent_by(Origin::System));
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
                self.dispatch(Action::LeaveVisual, self.input, window, cx);
            }
            VisualKey::Exit => drop(self.dispatch(Action::LeaveVisual, self.input, window, cx)),
        }
        cx.notify();
    }

    /// ⌘C: copies the open text field's selection, else the focused terminal's, in visual mode too. With nothing selected it
    /// does nothing.
    fn copy(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.field_copy(false, window, cx) {
            return;
        }
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

    /// ⌘V: pastes the clipboard's text into the open text field, else the focused terminal.
    fn paste(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.field_paste(window, cx) {
            return;
        }
        let text = self.clipboard_text(cx);
        if self.talk_paste(text) {
            return;
        }
        let Some(terminal) = self.model.keyboard_terminal().and_then(|id| self.terminals.get(&id)) else { return };
        if let Some(text) = self.clipboard_text(cx) {
            terminal.paste(&text);
        }
    }

    /// The clipboard's text. An E2E run's clipboard is `copied`, never the user's.
    fn clipboard_text(&self, cx: &App) -> Option<String> {
        match self.system_clipboard {
            true => cx.read_from_clipboard().and_then(|item| item.text()),
            false => self.copied.clone(),
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

    /// A top-bar button: + makes a new tab with an empty pane; a split opens the picker.
    pub(crate) fn top_button(&mut self, axis: Axis, new_tab: bool, window: &Window, cx: &mut Context<Self>) {
        if new_tab {
            self.dispatch(Action::NewEmptyTab, self.input, window, cx);
        } else {
            self.open_picker(axis, false, window, cx);
        }
    }

    /// Opens the pane picker: a terminal, then each running app.
    fn open_picker(&mut self, axis: Axis, new_tab: bool, window: &Window, cx: &mut Context<Self>) {
        self.refresh_apps();
        // What `refresh_apps` just listed: running apps, then the pinned ones that are not.
        let apps = self.apps.iter().map(|app| PickItem::App { name: app.name.clone() });
        let closed = self.pinned_closed().map(|app| PickItem::Closed { name: app.name, path: app.path });
        let items = std::iter::once(PickItem::Terminal).chain(apps).chain(closed).collect();
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

    /// The key reference: every shortcut, grouped. ⌘/ or Esc or a click closes it.
    fn render_keys(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let return_key = RETURN_KEY.get().cloned().unwrap_or_else(|| "off".into());
        let capture_key = collect::KEY.get().map_or_else(|| "off".into(), |key| collect::key_label(key));
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
                        .child(div().w(px(150.)).flex_none().text_color(rgb(theme::TEXT)).child(keys))
                        // A long line wraps in a narrow window.
                        .child(div().min_w_0().text_color(rgb(theme::MUTED)).child(what))
                }))
        };
        let rows = |list: &[(&str, &'static str)]| list.iter().map(|(k, w)| (k.to_string(), *w)).collect::<Vec<_>>();
        let columns = div()
            .p_4()
            .flex()
            .gap_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(group("PANES", rows(KEYS_PANES)))
                    .child(group("MOVE", rows(KEYS_MOVE)))
                    .child(group("ARRANGE", rows(KEYS_ARRANGE)))
                    .child(group("MOUSE", rows(KEYS_DRAG))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(group("TABS", rows(KEYS_TABS)))
                    .child(group("MARKS", rows(KEYS_MARKS)))
                    .child(group(
                        "WINDOW",
                        vec![
                            (return_key, "From any app: back to ottospace"),
                            ("⌘⌃F".into(), "Zoom the window"),
                            (capture_key, "From any app: capture the pane in use"),
                            ("⌘⌃A".into(), "Accessibility overlay (debug)"),
                            ("⌘P".into(), "Command palette: any command by name"),
                            ("⌘/".into(), "This list"),
                        ],
                    ))
                    .child(group("TEXT FIELDS", rows(KEYS_FIELD))),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(group("TERMINAL", rows(KEYS_TERMINAL)))
                    .child(group("VISUAL", rows(KEYS_VISUAL)))
                    .child(group("LIST", rows(KEYS_LIST)))
                    .child(group("SNEAK", rows(KEYS_SNEAK))),
            );
        // No taller than the window: the list scrolls inside the panel.
        let panel = dialog::panel(theme::BORDER_STRONG).min_h_0().max_w_full().flex().flex_col().overflow_hidden();
        // The columns keep their height inside the list: a flex row would stretch them to its own.
        let list = div().id("keys-rows").flex_1().overflow_y_scroll().child(columns);
        let panel = panel.child(self.bars.area("keys", list));
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

    /// The picker dropdown, under the split buttons. A click outside it closes it.
    fn render_picker(&self, picker: &Picker, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = picker.selected;
        let rows = picker.matches().into_iter().enumerate().map(|(i, item)| {
            let (tag, color) = match item {
                PickItem::Terminal => ("TERM", theme::PRIMARY),
                PickItem::App { .. } => ("APP", theme::APP),
                PickItem::Closed { .. } => ("APP", theme::MUTED),
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
        let title = match (picker.new_tab, picker.axis) {
            (true, _) => "New tab with",
            (false, Axis::Right) => "Split right with",
            (false, Axis::Down) => "Split down with",
        };
        let panel = dialog::floating(320.).absolute().top(px(TOP_BAR + 6.)).right(px(8.)).max_h(px(420.));
        let header = div()
            .h(px(34.))
            .text_xs()
            .child(div().text_color(rgb(theme::PICK)).child(title))
            .child(self.render_field("type to filter", cx));
        // The status bar no longer lists these keys, and ⌘/ closes the picker.
        let texts = ("No match", "↑↓ move · ↵ open · esc close");
        let panel = self.list_panel(panel, header, "picker", rows, texts);
        dialog::backdrop("picker-backdrop", 0.25, Action::ClosePicker, cx).child(panel)
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
                // Soon after going live the app's activation may still be on its way.
                // `follow_front_app` must not enter a pane again for it, and ottospace takes
                // the keyboard back once it has landed.
                self.left_by_click = Some((pid, Instant::now()));
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
        let needs_you = self.model.tasks.needs_you() > 0;
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
                    div()
                        .id(("rail", i))
                        .flex_none()
                        .w(px(RAIL))
                        .h(px(RAIL_CELL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(!active, |d| d.border_r_1().border_color(rgb(theme::BORDER)))
                        .hover(|s| s.bg(rgb(theme::SELECTED)))
                        .when(active, |d| {
                            // Amber on Tasks while a task needs the user, as its dot is.
                            let tasks = section == Section::Tasks && needs_you;
                            raised_cell(d, if tasks { theme::SNEAK } else { theme::PRIMARY }, false, i == 0)
                        })
                        .when(self.model.rail_focus == Some(RailFocus::Left(section)), |d| focus_ring(d.relative()))
                        .child(svg().path(*icon).size(px(20.)).text_color(rgb(if active {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        })))
                        .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(label.into()))))
                        .children((section == Section::Calendar).then(|| self.calendar_rail_mark()).flatten())
                        .child(self.controls.mark(format!("rail:{}", label.to_lowercase())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.dispatch(Action::ToggleSection { section }, this.input, window, cx);
                        }))
                },
            ))
            .child(div().flex_1().border_r_1().border_color(rgb(theme::BORDER)))
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
    /// something, and × to close the bar.
    fn bar_header(&self, section: Section, cx: &mut Context<Self>) -> Div {
        let title = SECTIONS.iter().find(|(s, ..)| *s == section).map_or("", |(_, _, label)| label);
        let button = |id: &'static str, icon: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(svg().path(icon).size(px(14.)).text_color(rgb(theme::MUTED)))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                .relative()
        };
        // What + adds, and its sneak id. Apps and Marks have nothing to add from here.
        let add = match section {
            Section::Tasks => Some(("New task (⌘N)", "bar:add")),
            Section::Repos => Some(("Add a repository…", "bar:add")),
            Section::Workbook => Some(("Save this tab as a workbook (⌘⇧S)", "bar:save_tab")),
            _ => None,
        };
        div()
            .relative()
            .flex_none()
            .h(px(RAIL_CELL))
            .child(mark(&self.marks, Mark::BarHeader))
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
            // How many tasks wait for the user.
            .children(Some(self.model.tasks.needs_you()).filter(|n| section == Section::Tasks && *n > 0).map(|n| {
                let badge = div().ml_2().h(px(18.)).min_w(px(18.)).px_1().pt(px(1.)).flex_none().flex();
                let badge = badge.items_center().justify_center().rounded_full().bg(rgb(theme::SELECTED));
                badge.line_height(px(16.)).text_color(rgb(theme::task(Group::NeedsYou))).child(n.to_string())
            }))
            .children((section == Section::Tasks).then(|| self.tasks_view_switch(cx)))
            .child(div().flex_1())
            .children(add.map(|(tip, mark)| {
                button("bar-add", "icons/plus.svg", tip)
                    .child(self.controls.mark(mark))
                    .on_click(cx.listener(move |this, _, window, cx| this.bar_add(section, window, cx)))
            }))
            .child(button("bar-close", "icons/dismiss.svg", "Close").child(self.controls.mark("bar:close")).on_click(
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::CloseSidebar, this.input, window, cx);
                }),
            ))
    }

    /// The header's +: a new task in Tasks, a repository to list in Repos, the tab on show in Workbook.
    pub(crate) fn bar_add(&mut self, section: Section, window: &Window, cx: &mut Context<Self>) {
        match section {
            Section::Tasks => self.new_task(window, cx),
            Section::Repos => self.pick_repo(window, cx),
            Section::Workbook => self.save_workbook(window, cx),
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

    /// A folder for the Repos list: resolved first (links, `..`), and only a directory. The
    /// core decides what to do with what is found there.
    fn add_repo(&mut self, path: &Path, origin: Origin, window: &Window, cx: &mut Context<Self>) {
        let dir = path.canonicalize().ok().filter(|dir| dir.is_dir());
        let Some(dir) = dir.as_deref().and_then(Path::to_str) else {
            self.error = Some(format!("Not a directory: {}", path.display()));
            return;
        };
        let found = repos::found_in(Path::new(dir));
        self.change_repos(Action::RepoAdd { dir: dir.into(), found }, origin, window, cx);
    }

    /// "Remove" on a repository's name in the Repos section.
    pub(crate) fn remove_repo(&mut self, path: &str, window: &Window, cx: &mut Context<Self>) {
        self.change_repos(Action::RepoRemove { path: path.into() }, self.input, window, cx);
    }

    /// A directory added to the Repos list or taken off it. The list is read before, so what is
    /// listed already is known, and after, from the directories as they then are. If the
    /// settings were not written the directories are as before: the next try writes again.
    fn change_repos(&mut self, action: Action, origin: Origin, window: &Window, cx: &mut Context<Self>) {
        // A refused one (a client's) leaves the status bar and the list alone.
        if self.model.refusal(&action, origin).is_some() {
            self.dispatch(action, origin, window, cx);
            return;
        }
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

    /// A click on a button of a task's row, by the button's name: `task-snooze`,
    /// `task-hold` or `task-dismiss`. Each offers to be undone.
    pub(crate) fn click_row_button(&mut self, button: &str, id: TaskId, window: &Window, cx: &mut Context<Self>) {
        let action = match button {
            "task-snooze" => Action::RowSnooze { id, until: now_ms().saturating_add(SNOOZE_MS) },
            "task-hold" => Action::RowHold { id },
            "task-dismiss" => Action::RowDismiss { id },
            "task-claim" => Action::TabPlace { tab: None, at: TabAt::End(Some(id)) },
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

    /// A click on a session's row or on one of its buttons, by the control's name: `session` or
    /// `session-open` (its conversation in the drawer), `session-adopt` (a task is made of it)
    /// or `session-hide`.
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
            "session" | "session-open" => Action::SessionOpen { runner, id },
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
            .child(div().w(px(8.)).text_color(rgb(theme::MUTED)).child(if collapsed { "▸" } else { "▾" }))
            .child(heading.title())
            .child(div().text_color(rgb(theme::MUTED)).child(count.to_string()))
            .relative()
            .child(self.controls.mark(format!("task-heading:{}", heading_name(heading))))
            .on_click(cx.listener(move |this, _, window, cx| this.click_heading(heading, window, cx)))
    }

    /// The Other sessions group: agent sessions found for the listed repositories that are no
    /// task's, those in its range. Empty when none is listed. `listed`: how many rows of the list
    /// come before; the heading and each session are rows, as list mode walks them.
    fn render_sessions(&self, mut listed: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let bar = self.model.sessions_bar();
        if bar.total == 0 {
            return Vec::new();
        }
        let selected = self.model.list == Some(listed);
        // Unfolded, the heading holds the range switch, on a line below when narrow; its count is
        // of those in range.
        let heading = self.heading_row(Heading::Sessions, bar.count, bar.collapsed, selected, cx);
        let total = SharedString::from(format!("{} in all", bar.total));
        let heading = heading
            .flex_wrap()
            .gap_y_1()
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(total.clone()))))
            .children((!bar.collapsed).then(|| self.sessions_range_switch(cx)));
        let mut rows = vec![heading.into_any_element()];
        listed += 1;
        for row in &bar.rows {
            rows.push(self.session_row(row, self.model.list == Some(listed), cx));
            listed += 1;
        }
        rows
    }

    /// A session's row: a dot (lit while it is active), where it runs and which runner, its title
    /// if it has one, then what it is doing, how long ago, and the model. Hover (or list mode's
    /// selection) shows Open, Adopt and Hide.
    fn session_row(&self, row: &ottospace_core::sessions::Row, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let key = format!("{}:{}", row.runner, row.id);
        let button = |name: &'static str, label: &'static str, tip: &'static str| {
            let (runner, id) = (row.runner.clone(), row.id.clone());
            let click = move |this: &mut Self, window: &Window, cx: &mut Context<Self>| {
                this.click_session_control(name, runner.clone(), id.clone(), window, cx)
            };
            self.word(format!("{name}:{key}"), (label, tip), click, cx)
        };
        let actions = words::words("session-row", selected)
            .bottom(px(4.))
            .child(button("session-open", "Open", "Its conversation, in the drawer"))
            .child(button("session-adopt", "Adopt", "Make a task of this session"))
            .child(button("session-hide", "Hide", "Hide until ottospace starts again"));
        let state = if row.active { row.doing.clone().unwrap_or_else(|| "active".into()) } else { "idle".into() };
        let detail = [Some(state), row.age.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let color = if row.active { theme::task(Group::Running) } else { theme::BORDER_STRONG };
        let muted = || div().flex_none().text_xs().text_color(rgb(theme::MUTED));
        let (runner, id) = (row.runner.clone(), row.id.clone());
        list_button(ElementId::Name(format!("session:{key}").into()), selected)
            .group("session-row")
            .relative()
            .items_start()
            .child(div().h(px(20.)).flex().items_center().child(home::dot(color)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
                            .children(row.pane.map(|pane| muted().child(format!("in pane {pane}"))))
                            .child(muted().child(row.runner.clone())),
                    )
                    // The session's title: text a model wrote, drawn as it is on one line.
                    .children(row.quoted_title.clone().map(|title| {
                        let color = gpui::Hsla::from(rgb(theme::TEXT)).opacity(0.8);
                        div().w_full().truncate().text_xs().line_height(px(18.)).text_color(color).child(title)
                    }))
                    .child(
                        div()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(muted().flex_1().min_w_0().truncate().child(detail))
                            .children(row.model.clone().map(|model| muted().child(model))),
                    ),
            )
            .child(actions)
            .child(self.controls.mark(format!("session:{key}")))
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
            for row in &section.rows {
                if let Some(task) = self.model.tasks.get(row.id) {
                    rows.push(self.task_row(task, row, self.model.list == Some(listed), cx));
                }
                listed += 1;
            }
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
        let list = self.bars.tracked("tasks", list.children(sessions), &self.list_scroll);
        // The footer stays under the list as it scrolls.
        let footer = (!self.model.tasks.list.is_empty()).then(|| self.render_today());
        div().flex_1().min_h_0().flex().flex_col().child(list).children(footer)
    }

    /// A task's row: a dot in its state's colour, priority and title, then a muted line of where
    /// it is filed and how old it is. Hover (or list mode's selection) shows what list mode's
    /// keys do, as buttons.
    fn task_row(&self, task: &Task, row: &TaskRow, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        // Words, as on a session's row: the icons said too little.
        let button = |name: &'static str, label: &'static str, tip: &'static str| {
            let click = move |this: &mut Self, window: &Window, cx: &mut Context<Self>| {
                this.click_row_button(name, id, window, cx)
            };
            self.word(format!("{name}:{id}"), (label, tip), click, cx)
        };
        // Over the end of the muted line, on the row's own colour: hidden, they take no room from it.
        let actions = words::words("task-row", selected)
            .bottom(px(4.))
            // No Start or Show: a click on the row opens its conversation.
            .child(button("task-snooze", "Snooze", "Snooze 1 hour (S)"))
            // A finished task cannot be put on hold, and one on hold is there already: no button
            // for a move that would change nothing.
            .children(
                (!matches!(row.group, Group::Done | Group::OnHold)).then(|| button("task-hold", "Hold", "Hold (H)")),
            )
            // The tab on show, of no task, can become this task's.
            .children(
                (self.model.tab_task().is_none() && row.group != Group::Done)
                    .then(|| button("task-claim", "Claim tab", "Make the tab on show this task's")),
            )
            .child(button("task-dismiss", "Dismiss", "Dismiss (X)"));
        // The task on show: a bar at the row's edge, and its title in bold.
        let bar = div().absolute().left(px(-1.)).top(px(5.)).bottom(px(5.)).w(px(3.)).rounded_full();
        let detail = div().flex_1().min_w_0().truncate().text_xs().text_color(rgb(theme::MUTED));
        list_button(("task", id as usize), selected)
            .group("task-row")
            .relative()
            .items_start()
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
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(row.current, |d| d.font_weight(FontWeight::SEMIBOLD))
                            // The tags are on the line below.
                            .children(task_title_cells(task))
                            // Under Needs you with its tabs open: said here, as it is not under Open.
                            .children((matches!(row.group, Group::NeedsYou | Group::Done) && row.tabs > 0).then(
                                || {
                                    let mark = div().flex_none().px_1().rounded_sm().bg(rgb(theme::SELECTED));
                                    mark.text_xs()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(rgb(theme::MUTED))
                                        .child("open")
                                },
                            )),
                    )
                    // What its session does now, while it works, else its tags and age; its model.
                    .child(
                        div()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(detail.child(row.live.clone().unwrap_or_else(|| row.detail.clone())))
                            .children(
                                row.model.clone().map(|model| {
                                    div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(model)
                                }),
                            )
                            .child(actions),
                    ),
            )
            .child(self.controls.mark(format!("task:{id}")))
            .when_some(progress::card_tip(self, id), |row, tip| row.tooltip(tip))
            .on_click(open_task(id, cx))
            .into_any_element()
    }

    /// Repositories, each with its worktrees under it. A click opens a terminal there: it fills
    /// the focused pane if that is empty, else splits it (right, or down with shift).
    fn render_repos(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // One checkout: the branch it is on; under it "Default", or a linked worktree's directory.
        let row = |index: usize, checkout: Checkout| {
            let place = if checkout.default { theme::PRIMARY } else { theme::MUTED };
            let icon = svg().path("icons/repos.svg").size(px(12.)).text_color(rgb(theme::MUTED));
            let label = div().truncate().text_xs().line_height(px(15.)).text_color(rgb(place));
            list_row(("repo", index), self.model.list == Some(index))
                // Beside the lines, not in a flex row with them: text in a flex row's cell is
                // first measured with no width, and is then never cut.
                .child(icon.absolute().left(px(8.)).top(px(5.)))
                .child(div().truncate().child(checkout.branch))
                .child(label.child(checkout.label))
                .child(mark(&self.marks, Mark::Checkout(index)))
                .child(self.controls.mark(format!("repo:{}", checkout.path)))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let axis = if e.modifiers().shift { Axis::Down } else { Axis::Right };
                    this.dispatch(Action::OpenRepo { path: checkout.path.clone(), axis }, this.input, window, cx);
                }))
        };
        // The repository's name as a heading, then its checkouts: its own directory first.
        let mut items: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for (n, repo) in self.model.repos.iter().enumerate() {
            // `flex_none`: a heading that may shrink is squeezed to nothing once the list overflows.
            let name = div().flex_none().px_2().pb_0p5().text_xs().font_weight(FontWeight::SEMIBOLD);
            let name = name.text_color(rgb(theme::MUTED)).map(|d| if n == 0 { d.pt_1() } else { d.pt_3() });
            let name = name.child(mark(&self.marks, Mark::Repo(n))).child(div().truncate().child(repo.name.clone()));
            // Under the pointer: a word that stops listing the repository. Not while
            // `OTTOSPACE_REPOS` names the directories: there is no setting to take it from.
            let path = repo.path.clone();
            let remove = (!self.model.repo_dirs.fixed).then(|| {
                let click =
                    move |this: &mut Self, window: &Window, cx: &mut Context<Self>| this.remove_repo(&path, window, cx);
                let tip = ("Remove", "Stop listing this repository");
                let word = self.word(format!("repo-remove:{}", repo.path), tip, click, cx);
                // On the heading's colour, not a row's; the heading's weight would make it bold.
                words::words("repo-name", false)
                    .bottom(px(1.))
                    .bg(rgb(theme::RAISED))
                    .font_weight(FontWeight::NORMAL)
                    .child(word)
            });
            items.push(name.group("repo-name").relative().children(remove).into_any_element());
            for checkout in repo.checkouts() {
                items.push(row(index, checkout).into_any_element());
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
        self.bars.tracked(
            "repos",
            list.track_scroll(&self.list_scroll).children(empty).children(items),
            &self.list_scroll,
        )
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
        // The window kept another size than the pane's: say so. In a narrow header the words
        // give way first: they shrink and are cut with an ellipsis.
        let hint =
            pane.fit.as_ref().map(|fit| div().min_w_0().truncate().text_color(rgb(theme::SNEAK)).child(fit.label()));
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
        // The pane being annotated has its tools in its header.
        let drawn_on = self.model.annotated().filter(|m| m.pane == id && self.shots.contains_key(&m.n));
        let strip = drawn_on.map(|material| self.annotate_strip(material, cx));
        // flex_none: a short pane must not squeeze the header, or a raised bar would overlap the pane above.
        let label: SharedString = if title.is_empty() { tag.into() } else { format!("{tag} {title}").into() };
        // A marked pane says so, with its letter: ⌘B gives no other sign while the Marks list is shut.
        let marked = self.model.mark_of(MarkOf::Pane(Some(id))).and_then(|mark| self.model.marks.get(mark));
        let marked = marked.map(|mark| mark.key.map_or("◆".to_string(), |key| format!("◆ {}", key.to_uppercase())));
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
            // The tag and the title keep a minimum; past it the title is cut, then the hint.
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(HEADER_TITLE_MIN))
                    .gap_2()
                    .overflow_hidden()
                    .child(div().flex_none().text_color(rgb(tag_color)).child(tag))
                    .child(div().min_w_0().truncate().child(title))
                    .children(marked.map(|sign| div().flex_none().text_color(rgb(theme::MARK)).child(sign))),
            )
            .child(self.controls.mark(format!("header:{id}")))
            // `fit` and × never shrink, so they stay in the header however narrow the pane.
            .child(
                // The task of its tab: the same badge as on the bar.
                div()
                    .flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .children(hint)
                    .children(word)
                    .children(strip)
                    .children(
                        self.model
                            .tab_of(id)
                            .and_then(|tab| self.model.tabs.iter().find(|t| t.id == tab)?.task)
                            .and_then(|task| self.task_badge(task, 16.)),
                    )
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
            );
        // Once its picture is taken. Until then the pane draws as ever, beside the bar.
        let taken = |material: &&ottospace_core::capture::Material| self.shots.contains_key(&material.n);
        let annotated = self.model.annotated().filter(|material| material.pane == id).filter(taken);
        let content = if let Some(material) = annotated {
            self.render_annotate(material, cx)
        } else if let Some(hosted) = self.hosted.get(&id) {
            // While live, keep showing the last frame under the real window, so any gap during a swap is invisible.
            match hosted.frame() {
                Some(buffer) => {
                    // At its own size at the top left, where the live window sits: a window smaller
                    // than the pane is neither stretched nor centred.
                    let preview = hosted.preview.clone();
                    let (radius, cropped) = (hosted.radius, hosted.shown_crop > 0.0);
                    let content = size(px(hosted.content.width as f32), px(hosted.content.height as f32));
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px(hosted.shown.width as f32))
                        .h(px(hosted.shown.height as f32))
                        // The wheel brings the app's window up for the scroll (a peek).
                        .on_scroll_wheel(
                            cx.listener(move |this, _: &ScrollWheelEvent, window, cx| this.app_wheel(id, window, cx)),
                        )
                        .child(surface(buffer).size_full())
                        // Record where the preview is drawn (e2e reads it), and mask the window's
                        // corners there: the ring is the whole window's, clipped to the preview,
                        // so a window cut off by the pane gets no corners at the cut.
                        .child(
                            canvas(
                                move |bounds, _, _| preview.set(bounds),
                                move |bounds, _, window, _| {
                                    let whole = Bounds { origin: bounds.origin, size: content };
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
            let scrollbar = self.bars.overlay(
                &format!("terminal:{id}"),
                false,
                move || (extent, scrolls),
                move |offset| scroll_to(history.saturating_sub((offset / line).round() as usize)),
            );
            div()
                .absolute()
                .size_full()
                .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| this.wheel(id, e.delta, cx)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| this.terminal_mouse_down(id, e, cx)),
                )
                .child(grid)
                .child(scrollbar)
                .into_any_element()
        } else {
            match pane.kind {
                Kind::Terminal => {
                    div().text_sm().text_color(rgb(theme::ERROR)).child("the shell did not start").into_any_element()
                }
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
        // the gap that window's own edge covers the pane's accent border.
        let inner = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            // Record the content's layout bounds: AX matches the real window to it; e2e reads it.
            .child(canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(self.controls.mark(format!("pane:{id}")))
            .child(content);
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
    fn render_top(&self, left: f32, cx: &mut Context<Self>) -> impl IntoElement {
        // The traffic lights sit over the left column: room for them only where it is narrower.
        div()
            .h(px(TOP_BAR))
            .flex_none()
            .pl(px((TRAFFIC_LIGHTS - left).max(0.)))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
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
            // Each task's tabs side by side behind its badge, the tabs of no task last. They shrink
            // to `MIN_TAB`, then clip: the buttons on the right always show.
            .child(div().flex_auto().min_w_0().h_full().flex().overflow_hidden().children(
                self.model.groups().into_iter().flat_map(|group| {
                    let task = group.task.and_then(|id| self.model.tasks.get(id));
                    let shown = task.is_some_and(|task| self.talk_names(task.id));
                    let underline = task.map(|task| (theme::identity(task.colour), shown));
                    let done = task.is_some_and(|task| matches!(task.state, State::Done | State::Cleaned));
                    // Its badge: a click shows its conversation; a tab dropped on it joins the group.
                    let head = group.task.and_then(|id| Some((id, self.task_badge(id, 18.)?))).map(|(id, badge)| {
                        div()
                            .id(("group", id))
                            .h_full()
                            .flex_none()
                            .pl_2()
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .child(badge)
                            .child(self.controls.mark(format!("group:{id}")))
                            .drag_over::<DraggedTab>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
                            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                                let at = TabAt::End(Some(id));
                                this.dispatch(Action::TabPlace { tab: Some(dragged.0), at }, this.input, window, cx);
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.dispatch(Action::TaskTalk { id }, this.input, window, cx);
                            }))
                            .into_any_element()
                    });
                    let tabs = group
                        .tabs
                        .into_iter()
                        .filter_map(|index| Some((index, self.model.tabs.get(index)?)))
                        .map(|(place, tab)| {
                            let (active, id) = (place == self.model.active, tab.id);
                            let title = SharedString::from(tab.title.clone());
                            div()
                                .id(("tab", tab.id))
                                // A pane dropped on a tab moves there; a tab dropped on one goes before it, in its group.
                                .drag_over::<DraggedPane>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
                                .drag_over::<DraggedTab>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
                                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                                    // Dropped on itself: a click that moved a few points (GPUI drags past 2).
                                    if dragged.0 == id {
                                        return this.select_tab(id, window, cx);
                                    }
                                    let at = TabAt::Before(id);
                                    this.dispatch(
                                        Action::TabPlace { tab: Some(dragged.0), at },
                                        this.input,
                                        window,
                                        cx,
                                    );
                                }))
                                .on_drag(DraggedTab(id), move |_, offset, _, cx| {
                                    cx.new(|_| DragChip { label: title.clone(), offset })
                                })
                                .on_drop(cx.listener(move |this, dragged: &DraggedPane, window, cx| {
                                    this.dispatch(
                                        Action::MovePane { pane: Some(dragged.0), tab: place },
                                        this.input,
                                        window,
                                        cx,
                                    );
                                }))
                                .h_full()
                                .min_w(px(MIN_TAB))
                                // At its narrowest it clips, never paints over the next tab.
                                .overflow_hidden()
                                .pl_3()
                                .pr_1()
                                .flex()
                                .items_center()
                                .gap_2()
                                .border_r_1()
                                .border_color(rgb(theme::BORDER))
                                .cursor_pointer()
                                .when(active, |d| d.bg(rgb(theme::RAISED)).text_color(rgb(theme::TEXT)))
                                // A task's tab is underlined in its colour, brighter while the drawer shows it.
                                .relative()
                                .when_some(underline, |d, (colour, shown)| {
                                    let line = div().absolute().left_0().right_0().bottom_0().h(px(2.)).bg(rgb(colour));
                                    d.child(line.when(!shown, |d| d.opacity(0.6)))
                                })
                                .hover(|s| s.text_color(rgb(theme::TEXT)))
                                .child(mark(&self.marks, Mark::Tab(place)))
                                .children(
                                    self.model
                                        .mark_of(MarkOf::Tab(Some(id)))
                                        .map(|_| div().text_color(rgb(theme::MARK)).child("◆")),
                                )
                                // A tab of a workbook: its icon, and the workbook's name on hover.
                                .when_some(self.workbook_of(id), |d, book| {
                                    let icon = svg().path("icons/workbook.svg").size(px(12.)).flex_none();
                                    d.child(icon.text_color(rgb(theme::MUTED))).tooltip(move |_, cx| {
                                        AnyView::from(cx.new(|_| Tooltip(format!("Workbook: {book}").into())))
                                    })
                                })
                                .child(div().flex_1().min_w_0().truncate().child(tab.title.clone()))
                                .child(self.controls.mark(format!("tab:{}", tab.id)))
                                .child(
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
                                .on_click(cx.listener(move |this, _, window, cx| this.select_tab(id, window, cx)))
                                .when(done, |d| d.opacity(0.5))
                                .into_any_element()
                        });
                    let tabs: Vec<_> = tabs.collect();
                    head.into_iter().chain(tabs).collect::<Vec<_>>()
                }),
            ))
            // Short of room, the meeting chip gives way with the tabs, never the buttons.
            .children(self.render_meeting_chip().map(|chip| chip.mr_1()))
            // New tab and split buttons: a new empty tab, or the picker for that split.
            .child(div().flex_none().ml_auto().mr_2().flex().gap_1().children(SPLITS.iter().enumerate().map(
                |(i, (name, icon, tip, axis, new_tab))| {
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
                        .child(self.controls.mark(format!("top:{name}")))
                        .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                        .on_click(cx.listener(move |this, _, window, cx| this.top_button(axis, new_tab, window, cx)))
                },
            )))
    }

    /// Bottom bar: who has the keyboard, and the focused pane.
    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let color = theme::accent(self.model.mode());
        // Name the return key only if it works: a wrong hint strands the user in the app.
        let app_hint = match RETURN_KEY.get() {
            Some(key) => format!("keys go to the app · {key} or a click on ottospace returns"),
            None => "keys go to the app · click ottospace to return".to_string(),
        };
        let peek = self.model.peek.as_ref().and_then(|peek| self.hosted.get(&peek.pane));
        let peek_hint = peek.map(|hosted| format!("{} · a click or a key enters it", hosted.app));
        let (mode, hint) = match self.model.mode() {
            // The app is up for a scroll: it has the keyboard until the peek ends.
            _ if peek_hint.is_some() => ("PEEK", peek_hint.as_deref().unwrap_or_default()),
            // One hint, not the key list: ⌘/ shows the keys of every mode. A mode you can be in
            // by accident also says how to leave. The picker and the new-task box show theirs.
            Mode::Normal => ("NORMAL", "⌘/ keys"),
            Mode::Pick => ("PICK", ""),
            Mode::Palette => ("COMMAND", ""),
            Mode::Inspect => ("INSPECT", "type to filter · ↑↓ select · esc leaves"),
            Mode::App => ("APP", app_hint.as_str()),
            Mode::Visual => ("VISUAL", "esc leaves · ⌘/ keys"),
            Mode::List => ("LIST", "esc leaves · ⌘/ keys"),
            Mode::Compose => ("TASK", ""),
            Mode::Sneak if self.model.sneak.as_ref().is_some_and(|sneak| sneak.hid_overlay) => {
                ("SNEAK", "type a label · esc leaves · the accessibility overlay is hidden (⌘⌃A shows it)")
            }
            Mode::Sneak => ("SNEAK", "type a label · esc leaves"),
            Mode::Confirm if self.model.confirm.as_ref().is_some_and(|c| c.dismiss.is_some()) => ("DISMISS?", ""),
            Mode::Confirm => ("QUIT?", ""),
            Mode::Annotate => ("ANNOTATE", "drag to circle · click for a note · ↵ title · esc leaves"),
            Mode::Talk => ("TALK", "keys go to the task's agent"),
            Mode::Mark => match self.model.mark_input {
                Some(MarkInput::Rename { .. }) => ("MARK", "type its name · ↵ keeps it · esc cancels"),
                Some(MarkInput::SetKey { .. }) => ("MARK", "a letter for the mark · ⌫ for none · esc cancels"),
                Some(MarkInput::Workbook { .. }) => ("WORKBOOK", "type its name · ↵ keeps it · esc cancels"),
                _ => ("MARK?", "a mark's letter · esc cancels"),
            },
        };
        let hint = match self.sneak_app_hint() {
            Some(app) => format!("{hint} · {app}"),
            None => hint.to_string(),
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
            // After a capture from a live app, with the draft gone: the way back to that app.
            .children(self.model.back_to.filter(|_| self.model.capture.is_none()).and_then(|pane| {
                let Kind::App { name, .. } = &self.model.pane(pane)?.kind else { return None };
                let back = div().id("status-back").relative().flex_none().cursor_pointer().text_color(rgb(theme::APP));
                Some(
                    back.child(format!("↩ back to {name}"))
                        .child(self.controls.mark("draft:back"))
                        .on_click(cx.listener(|this, _, window, cx| this.click_capture("back", window, cx))),
                )
            }))
            .children(self.meeting_soon())
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

/// A task's priority and title, as cells of a row.
fn task_title_cells(task: &Task) -> [Div; 2] {
    let urgent = if task.priority == 1 { theme::ERROR } else { theme::MUTED };
    let priority = div().flex_none().text_xs().text_color(rgb(urgent)).child(format!("P{}", task.priority));
    [priority, div().flex_1().min_w_0().truncate().child(task.title.clone())]
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
    edge.shadow(vec![BoxShadow {
        color: black().opacity(0.65),
        offset: point(px(if left { 8. } else { -8. }), px(0.)),
        blur_radius: px(16.),
        spread_radius: px(-6.),
    }])
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

/// A rail's open cell, raised: the panel's background, a soft shadow above and below it, a
/// lighter line along its top, and an accent bar at the window's edge (`right`: the right rail's).
/// The `first` cell has neither line nor shadow above: the title strip's border is the one line there.
pub(crate) fn raised_cell(cell: Stateful<Div>, accent: u32, right: bool, first: bool) -> Stateful<Div> {
    let shadow = |y: f32| BoxShadow {
        color: black().opacity(0.85),
        offset: point(px(0.), px(y)),
        blur_radius: px(9.),
        spread_radius: px(-6.),
    };
    let bar = div().absolute().top_0().w(px(2.)).h_full().bg(rgb(accent));
    let bar = if right { bar.right_0() } else { bar.left_0() };
    let shadows = if first { vec![shadow(7.)] } else { vec![shadow(-7.), shadow(7.)] };
    let line = (!first).then(|| div().absolute().top_0().left_0().w_full().h(px(1.)).bg(rgb(0x303036)));
    cell.relative().bg(rgb(theme::RAISED)).shadow(shadows).children(line).child(bar)
}

/// A rail cell with the keyboard (`zones`): a ring in list mode's colour, apart from "open".
pub(crate) fn focus_ring(cell: Stateful<Div>) -> Stateful<Div> {
    cell.child(div().absolute().inset(px(4.)).rounded(px(8.)).border_2().border_color(rgb(theme::LIST)))
}

/// A row under a heading (a repository's checkout): no outline at rest, so a long list stays
/// quiet. Hover and selection mark it.
fn list_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .relative()
        // Room on the left for the row's icon.
        .pl(px(26.))
        .pr_2()
        .py_0p5()
        .rounded_md()
        // Always there, so a row does not shift by a point when it is selected.
        .border_1()
        .border_color(transparent_black())
        .text_size(px(13.))
        .line_height(px(17.))
        .text_color(rgb(theme::MUTED))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
        .when(selected, |row| row.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)).border_color(rgb(theme::LIST)))
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

/// A key's modifiers as the core names them.
fn mods(m: gpui::Modifiers) -> Mods {
    Mods { ctrl: m.control, alt: m.alt, shift: m.shift, cmd: m.platform }
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        // Every task's letters, worked out once a frame for each badge drawn (`task_badge`).
        *self.badges.borrow_mut() = self.model.badges(&[]);
        // This frame draws every action so far: the replies held for it can go (`settle`).
        self.drawn_seq = self.seq;
        self.frame_start.set(Some(Instant::now()));
        for pane in &self.model.panes {
            self.cells.entry(pane.id).or_default();
            self.frames.entry(pane.id).or_default();
        }
        self.controls.begin();
        self.bars.begin();
        // The Repos list is noted again only while it is drawn.
        self.marks.borrow_mut().retain(|mark, _| !matches!(mark, Mark::Repo(_) | Mark::Checkout(_) | Mark::BarHeader));
        // The pane drag is over, however it ended (a drop, Esc, a release outside the window):
        // no drop zone may stay drawn.
        if !cx.has_active_drag() {
            self.drop_at = None;
        }
        let tree = self.model.layout.as_ref().map(|root| self.render_node(root, cx));
        // Only while the tab on show has no panes. Its lines are built once and kept: a frame
        // that moves nothing (a terminal printing in another tab) only paints them again.
        let keep_out = self.home.borrow().keep_out();
        let home = self.model.home(&keep_out).map(|home| self.render_home(home, cx));
        // The left column's width, and the drawer's: as dragged, or half the window when widened.
        let left = RAIL + if self.model.sidebar.is_some() { SECTION } else { 0. };
        let half = f32::from(window.viewport_size().width) / 2.;
        let drawer = if self.model.talk_wide { half } else { self.model.talk_width.min(half.max(MIN_DRAWER)) };
        self.drawer.width = drawer;
        // The inspector takes the drawer's place while it is open; the drawer keeps its state.
        let right = self.render_inspector(drawer, cx);
        let right = right.or_else(|| self.render_draft(cx)).or_else(|| self.render_drawer(drawer, cx));
        let right_width = talk::RAIL + if right.is_some() { drawer } else { 0. };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .map(|root| self.command_actions(root, cx))
            // On the root, so a selection drag keeps going over other panes.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                this.mouse_move(e, cx);
                this.annotate_move(e, window, cx);
            }))
            .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<DraggedPane>, window, cx| {
                this.pane_drag_move(e.drag(cx).0, e.event.position, window, cx);
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
                    this.annotate_release(e.position, window, cx);
                }),
            )
            // Any click leaves visual mode first, before the clicked element acts: visual mode has no
            // backdrop, and e.g. a click on an app pane must go live, not be replayed while blocked.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
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
            .on_action(cx.listener(|this, _: &CopySelection, window, cx| this.copy(window, cx)))
            .on_action(cx.listener(|this, _: &PasteText, window, cx| this.paste(window, cx)))
            .on_action(cx.listener(|this, _: &Cut, window, cx| this.field_menu(edit::Key::Cut, window, cx)))
            .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
                this.field_menu(edit::Key::Edit(Edit::SelectAll), window, cx)
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
            // The drawer's Stop, for the conversation it shows.
            .on_action(cx.listener(|this, _: &StopAgent, window, cx| {
                if let Some(id) = this.model.talk.as_ref().filter(|talk| talk.open).map(|talk| talk.task.clone()) {
                    this.dispatch(Action::TaskStop { id }, this.input, window, cx);
                }
            }))
            // ⌘↵ is the new-task box's "start" while it has the keyboard: the binding comes first.
            .on_action(cx.listener(|this, _: &TalkGo, window, cx| {
                if this.model.compose_keys() {
                    this.dispatch(Action::ComposeSubmit { open: true }, this.input, window, cx);
                } else {
                    this.dispatch(Action::TalkGo, this.input, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CapturePane, window, cx| {
                this.error = this.capture_pane(None, this.input, window, cx).err();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MarkPane, window, cx| this.toggle_mark(MarkOf::Pane(None), window, cx)))
            .on_action(cx.listener(|this, _: &MarkTab, window, cx| this.toggle_mark(MarkOf::Tab(None), window, cx)))
            .on_action(cx.listener(|this, _: &SaveWorkbook, window, cx| this.save_workbook(window, cx)))
            .on_action(cx.listener(|this, _: &AddTabToWorkbook, window, cx| this.pick_workbook(window, cx)))
            .on_action(cx.listener(|this, _: &SaveAllWorkbook, window, cx| this.save_all_workbook(window, cx)))
            .on_action(cx.listener(|this, _: &StartFresh, window, cx| {
                drop(this.dispatch(Action::StartFresh, this.input, window, cx))
            }))
            .on_action(cx.listener(|this, _: &MarkJump, window, cx| {
                this.dispatch(Action::MarkPrefix, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTask, window, cx| {
                this.dispatch(Action::Tick { now: now_ms() }, this.input, window, cx);
                this.dispatch(Action::TaskGo { id: None }, this.input, window, cx);
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
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
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
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.top_button(Axis::Right, true, window, cx)))
            .on_action(cx.listener(|this, _: &NewTabOutside, window, cx| {
                this.dispatch(Action::NewTabOf { task: None }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &RemoveTabFromTask, window, cx| {
                this.dispatch(Action::TabPlace { tab: None, at: TabAt::End(None) }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveTabToTask, window, cx| {
                this.dispatch(Action::PaletteTasks, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.dispatch(Action::CycleTab { delta: 1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                this.dispatch(Action::CycleTab { delta: -1 }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.dispatch(Action::CloseTab { index: None }, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ClosePane, window, cx| {
                this.dispatch(Action::Close { pane: None }, this.input, window, cx);
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
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .flex()
                                    .child(self.render_rail(cx))
                                    .children(self.model.sidebar.map(|section| self.render_bar(section, cx))),
                            ),
                    )
                    // Center: the tab strip over the workspace, from the section list to the drawer.
                    .child(
                        div().flex_1().min_w_0().h_full().flex().flex_col().child(self.render_top(left, cx)).child(
                            div()
                                .flex_1()
                                // Let the canvas shrink below its content, so a wide image cannot
                                // push it past the window; it scales down instead.
                                .min_w_0()
                                .min_h_0()
                                .w_full()
                                .flex()
                                .overflow_hidden()
                                .children(home)
                                .children(tree),
                        ),
                    )
                    // Right: the drawer (the draft being collected, a conversation, or a brief),
                    // then its rail.
                    .children(right)
                    .child(self.render_right_rail(cx))
                    // The side zones sit above the workspace: each casts a soft shadow onto it.
                    // Drawn last, so it lies over the panes. A live app's window is above it all.
                    .child(zone_shadow(left, true))
                    .child(zone_shadow(right_width, false))
                    // Highest, the title strip: its shadow falls on all three zones.
                    .child(title_shadow())
                    // Over the strip's top edge, above the window's buttons: the meeting timeline.
                    .children(self.render_hairline())
                    .children(self.render_meeting_banner(cx)),
            )
            .child(self.render_status(cx))
            // Only over a parked, drawn pane: a live window sits above ottospace and would cover it.
            .children(
                self.ax_overlay
                    .as_ref()
                    .filter(|o| self.model.ax_overlay && self.model.live != Some(o.pane))
                    .filter(|o| self.model.layout.as_ref().is_some_and(|l| l.leaves().contains(&o.pane)))
                    .map(ax_debug::Overlay::render),
            )
            .children(self.render_inspector_outline())
            // Over everything of ottospace's own. It is never on while an app's window is.
            .children(self.model.sneak.as_ref().map(sneak::labels))
            .children(self.model.picker.as_ref().map(|picker| self.render_picker(picker, cx)))
            .children(self.model.palette.as_ref().map(|palette| self.render_palette(palette, cx)))
            .children(self.render_pointing())
            .when(self.model.keys, |d| d.child(self.render_keys(cx)))
            .children(self.model.confirm.as_ref().map(|confirm| self.render_confirm(confirm, cx)))
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
    let reply = control::send(&control::path(), request).unwrap_or_else(|err| {
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
    trace::record("start", serde_json::json!({ "pid": std::process::id(), "hosting": hosting }));
    let control = control::serve().map_err(|err| eprintln!("control socket off: {err}")).ok();
    // For `osi`: which program the running app is.
    running::write();

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        // Run as a bare binary (`ottospace`, `cargo run`) there is no bundle to supply the icon.
        ottospace_mac::window::set_dock_icon(include_bytes!("../assets/ottospace.icns"));
        cx.bind_keys([
            KeyBinding::new("cmd-d", SplitRight, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-e", PickPane, None),
            KeyBinding::new("cmd-shift-e", PickPaneDown, None),
            KeyBinding::new("cmd-/", ShowKeys, None),
            KeyBinding::new("cmd-p", commands::CommandPalette, None),
            KeyBinding::new("cmd-w", ClosePane, None),
            KeyBinding::new("cmd-ctrl-left", FocusLeft, None),
            KeyBinding::new("cmd-ctrl-right", FocusRight, None),
            KeyBinding::new("cmd-ctrl-up", FocusUp, None),
            KeyBinding::new("cmd-ctrl-down", FocusDown, None),
            // The same moves on the home row, and a cycle through every pane.
            KeyBinding::new("cmd-ctrl-h", FocusLeft, None),
            KeyBinding::new("cmd-ctrl-l", FocusRight, None),
            KeyBinding::new("cmd-ctrl-k", FocusUp, None),
            KeyBinding::new("cmd-ctrl-j", FocusDown, None),
            KeyBinding::new("cmd-]", FocusNext, None),
            KeyBinding::new("cmd-[", FocusPrev, None),
            // Resize the focused pane's split, and even all splits out.
            KeyBinding::new("cmd-alt-left", ResizeLeft, None),
            KeyBinding::new("cmd-alt-right", ResizeRight, None),
            KeyBinding::new("cmd-alt-up", ResizeUp, None),
            KeyBinding::new("cmd-alt-down", ResizeDown, None),
            KeyBinding::new("cmd-alt-h", ResizeLeft, None),
            KeyBinding::new("cmd-alt-l", ResizeRight, None),
            KeyBinding::new("cmd-alt-k", ResizeUp, None),
            KeyBinding::new("cmd-alt-j", ResizeDown, None),
            KeyBinding::new("cmd-ctrl-=", Equalize, None),
            KeyBinding::new("cmd-ctrl-0", FitPane, None),
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
            KeyBinding::new("cmd-shift-n", NextTask, None),
            KeyBinding::new("cmd-l", TalkFocus, None),
            KeyBinding::new("cmd-shift-l", TalkToggle, None),
            KeyBinding::new("cmd-enter", TalkGo, None),
            // Marks: ⌘' waits for a mark's letter. ⌘⌥1..9 go to the first nine (`key_down`).
            KeyBinding::new("cmd-b", MarkPane, None),
            KeyBinding::new("cmd-shift-b", MarkTab, None),
            KeyBinding::new("cmd-'", MarkJump, None),
            KeyBinding::new("cmd-h", Hide, None),
            // Asks first while a terminal runs (`ask_quit`): ⌘Q sits next to ⌘W.
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        // The menu bar lists the same actions; the app-level ones are handled here.
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
        cx.on_window_closed(|cx| cx.quit()).detach();
        if hosting {
            cx.activate(true);
            // The return key works from any app, so it is only useful with hosting on.
            let name = std::env::var("OTTOSPACE_RETURN_KEY").unwrap_or_else(|_| "f18".into());
            let registered = hotkey::key_code(&name)
                .ok_or_else(|| "it is not one of f13-f20".to_string())
                .and_then(hotkey::register);
            // A key another app holds is said in the status bar too: stderr is seen by nobody.
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
        const TESTS_ONLY: [&str; 5] =
            ["capture_start", "repo_add", "inspector_fixture", "sneak_app_tree", "calendar_call"];
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
        for path in icons {
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
