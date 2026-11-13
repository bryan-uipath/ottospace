//! Canary: a tiny app whose windows ottospace can host in tests, so the tests never touch the
//! user's real windows. Each window records the keys and clicks it receives. A control socket
//! reports that record and opens or closes windows on request.
//!
//! One JSON request per line on `$CANARY_SOCKET`, one JSON reply per line:
//! `{"cmd":"state"}`, `{"cmd":"open_window"}`, `{"cmd":"close_window","index":1}`, `{"cmd":"quit"}`.
//! A window that refuses a pane's size: `{"cmd":"open_window","min":[900,700]}` cannot be made
//! smaller than that; `{"cmd":"open_window","fixed":[240,180]}` cannot be resized at all.
//! `{"cmd":"retitle","index":0,"title":"Private"}` gives a window another title.
//! `{"cmd":"move","index":0,"to":[x,y]}` and `{"cmd":"recreate","index":0,"at":[x,y]}` put a
//! window, or a new one in its place, anywhere, off screen too. `{"cmd":"hang","ms":8000}` makes
//! the app too busy to answer Accessibility calls for that long.
//!
//! With `CANARY_BUTTON` set, the first window has a native button titled "Canary button" in
//! its content: a control its accessibility tree names. Below it, a checkbox "Canary press"
//! (`pressed` in the state: whether it is on) and a disabled one, "Canary disabled".

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, div, prelude::*, px, rgb, size,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    State,
    OpenWindow {
        #[serde(default)]
        min: Option<[f32; 2]>,
        #[serde(default)]
        fixed: Option<[f32; 2]>,
    },
    CloseWindow {
        index: usize,
    },
    Retitle {
        index: usize,
        title: String,
    },
    /// Moves a window's top left to `to`, in Accessibility's coordinates, off screen if asked.
    Move {
        index: usize,
        to: [f64; 2],
    },
    /// Closes a window and opens a new one at `at`, as an app that re-makes its window where
    /// the old one stood.
    Recreate {
        index: usize,
        at: [f64; 2],
    },
    /// Blocks the main thread for `ms` after replying: an app too busy for Accessibility calls.
    Hang {
        ms: u64,
    },
    Quit,
}

/// The window's `NSView`.
fn ns_view(window: &Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr()),
        _ => None,
    }
}

fn move_window(handle: WindowHandle<Canary>, to: [f64; 2], cx: &mut App) {
    let _ = handle.update(cx, |_, window, _| {
        if let Some(view) = ns_view(window) {
            unsafe { ottospace_mac::window::move_to(view, to[0], to[1]) };
        }
    });
}

/// One canary window: shows and records what it receives.
struct Canary {
    title: String,
    keys: Vec<String>,
    /// Click positions in window coordinates.
    clicks: Vec<[f32; 2]>,
    focus: FocusHandle,
}

impl Render for Canary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(rgb(0x1f2937))
            .text_color(rgb(0xf9fafb))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                this.keys.push(e.keystroke.key.clone());
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.clicks.push([e.position.x.into(), e.position.y.into()]);
                    cx.notify();
                }),
            )
            .child(div().text_xl().child(self.title.clone()))
            .child(format!("keys: {}", self.keys.join(" ")))
            .child(format!("clicks: {}", self.clicks.len()))
    }
}

type Windows = Rc<RefCell<Vec<WindowHandle<Canary>>>>;

thread_local! {
    /// The first window's checkboxes, with `CANARY_BUTTON`: "Canary press", "Canary disabled".
    static CHECKBOXES: RefCell<Vec<ottospace_mac::window::Checkbox>> = const { RefCell::new(Vec::new()) };
}

/// `at`: where its top left goes, as `move` puts it.
fn open(windows: &Windows, min: Option<[f32; 2]>, fixed: Option<[f32; 2]>, at: Option<[f64; 2]>, cx: &mut App) {
    let number = windows.borrow().len() + 1;
    let title = format!("Canary {number}");
    // Stagger the windows so each one is partly visible.
    let offset = px(40. * number as f32);
    let [width, height] = fixed.or(min).unwrap_or([640., 420.]);
    let mut bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    bounds.origin.x += offset;
    bounds.origin.y += offset;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(title.clone().into()), ..Default::default() }),
        is_resizable: fixed.is_none(),
        window_min_size: min.map(|[width, height]| size(px(width), px(height))),
        // Placed at `at` before it is first seen: nothing reads it where GPUI put it.
        show: at.is_none(),
        ..Default::default()
    };
    let handle = cx.open_window(options, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus);
            Canary { title, keys: Vec::new(), clicks: Vec::new(), focus }
        })
    });
    if let Ok(handle) = handle {
        if number == 1 && std::env::var_os("CANARY_BUTTON").is_some() {
            let _ = handle.update(cx, |_, window, _| {
                if let Some(view) = ns_view(window) {
                    let at = [16., 150., 170., 30.];
                    unsafe { ottospace_mac::window::add_button(view, "Canary button", at) };
                    let add = |title, y, on| unsafe {
                        ottospace_mac::window::add_checkbox(view, title, [16., y, 170., 24.], on)
                    };
                    CHECKBOXES.with_borrow_mut(|boxes| {
                        boxes.extend(add("Canary press", 190., true));
                        boxes.extend(add("Canary disabled", 224., false));
                    });
                }
            });
        }
        windows.borrow_mut().push(handle);
        // A moment later, once GPUI is done placing it: moved, shown, and moved again in case
        // showing it put it on screen.
        if let Some(at) = at {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let _ = handle.update(cx, |_, window, _| {
                    if let Some(view) = ns_view(window) {
                        unsafe {
                            ottospace_mac::window::move_to(view, at[0], at[1]);
                            ottospace_mac::window::order_front(view);
                            ottospace_mac::window::move_to(view, at[0], at[1]);
                        }
                    }
                });
            })
            .detach();
        }
    }
}

/// Each open window's record and frame `[x, y, w, h]`. Closed windows are dropped from the list.
fn state(windows: &Windows, cx: &mut App) -> String {
    let mut open_windows = Vec::new();
    windows.borrow_mut().retain(|handle| {
        let entry = handle.update(cx, |canary, window, _| {
            let b = window.bounds();
            let frame: [f32; 4] = [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()];
            // `active`: this window has the keyboard. A test waits for it before typing.
            let active = window.is_window_active();
            // `frame`: as Accessibility has it, global, so a test can tell whether it is on a screen.
            let global = ns_view(window).and_then(|view| unsafe { ottospace_mac::window::frame(view) });
            json!({ "title": canary.title, "keys": canary.keys, "clicks": canary.clicks, "bounds": frame, "frame": global, "active": active })
        });
        entry.map(|entry| open_windows.push(entry)).is_ok()
    });
    // Whether "Canary press" is on: a press reached it.
    let pressed = CHECKBOXES.with_borrow(|boxes| boxes.first().map(|b| b.on()));
    json!({ "ok": true, "windows": open_windows, "pressed": pressed }).to_string()
}

fn serve(calls: &Receiver<(Request, Sender<String>)>, windows: &Windows, cx: &mut App) {
    for (request, reply) in calls.try_iter() {
        match request {
            Request::State => {}
            Request::OpenWindow { min, fixed } => open(windows, min, fixed, None, cx),
            Request::CloseWindow { index } => {
                let handle = windows.borrow().get(index).copied();
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }
            Request::Retitle { index, title } => {
                let handle = windows.borrow().get(index).copied();
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |canary, window, cx| {
                        window.set_window_title(&title);
                        canary.title = title;
                        cx.notify();
                    });
                }
            }
            Request::Move { index, to } => {
                let handle = windows.borrow().get(index).copied();
                if let Some(handle) = handle {
                    move_window(handle, to, cx);
                }
            }
            Request::Recreate { index, at } => {
                let handle = windows.borrow().get(index).copied();
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    open(windows, None, None, Some(at), cx);
                }
            }
            Request::Hang { ms } => {
                let _ = reply.send(state(windows, cx));
                std::thread::sleep(Duration::from_millis(ms.min(30_000)));
                continue;
            }
            Request::Quit => {
                let _ = reply.send(json!({ "ok": true }).to_string());
                // Quit a moment later, so the connection thread can write the reply first.
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
                return;
            }
        }
        let _ = reply.send(state(windows, cx));
    }
}

fn listen(path: String, calls: Sender<(Request, Sender<String>)>) {
    // Remove only a stale socket, never some other file the path names.
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_socket()) {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path).expect("bind $CANARY_SOCKET");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let calls = calls.clone();
            std::thread::spawn(move || handle(stream, calls));
        }
    });
}

fn handle(stream: UnixStream, calls: Sender<(Request, Sender<String>)>) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let reply = match serde_json::from_str(&line) {
            Ok(request) => {
                let (reply, answer) = channel();
                if calls.send((request, reply)).is_err() {
                    return;
                }
                answer.recv().unwrap_or_else(|_| json!({ "ok": false }).to_string())
            }
            Err(err) => json!({ "ok": false, "error": err.to_string() }).to_string(),
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

fn main() {
    let (calls, requests) = channel();
    if let Ok(path) = std::env::var("CANARY_SOCKET") {
        listen(path, calls);
    }
    Application::new().run(move |cx: &mut App| {
        let windows: Windows = Rc::default();
        open(&windows, None, None, None, cx);
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                if cx.update(|cx| serve(&requests, &windows, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}
