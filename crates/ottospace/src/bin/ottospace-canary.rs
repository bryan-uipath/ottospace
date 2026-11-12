//! Canary: a tiny app whose windows ottospace can host in tests, so the tests never touch the
//! user's real windows. Each window records the keys and clicks it receives. A control socket
//! reports that record and opens or closes windows on request.
//!
//! One JSON request per line on `$CANARY_SOCKET`, one JSON reply per line:
//! `{"cmd":"state"}`, `{"cmd":"open_window"}`, `{"cmd":"close_window","index":1}`, `{"cmd":"quit"}`.

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
    OpenWindow,
    CloseWindow { index: usize },
    Quit,
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

fn open(windows: &Windows, cx: &mut App) {
    let number = windows.borrow().len() + 1;
    let title = format!("Canary {number}");
    // Stagger the windows so each one is partly visible.
    let offset = px(40. * number as f32);
    let mut bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
    bounds.origin.x += offset;
    bounds.origin.y += offset;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(title.clone().into()), ..Default::default() }),
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
        windows.borrow_mut().push(handle);
    }
}

/// Each open window's record and frame `[x, y, w, h]`. Closed windows are dropped from the list.
fn state(windows: &Windows, cx: &mut App) -> String {
    let mut open_windows = Vec::new();
    windows.borrow_mut().retain(|handle| {
        let entry = handle.update(cx, |canary, window, _| {
            let b = window.bounds();
            let frame: [f32; 4] = [b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into()];
            json!({ "title": canary.title, "keys": canary.keys, "clicks": canary.clicks, "bounds": frame })
        });
        entry.map(|entry| open_windows.push(entry)).is_ok()
    });
    json!({ "ok": true, "windows": open_windows }).to_string()
}

fn serve(calls: &Receiver<(Request, Sender<String>)>, windows: &Windows, cx: &mut App) {
    for (request, reply) in calls.try_iter() {
        match request {
            Request::State => {}
            Request::OpenWindow => open(windows, cx),
            Request::CloseWindow { index } => {
                let handle = windows.borrow().get(index).copied();
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
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
        open(&windows, cx);
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
