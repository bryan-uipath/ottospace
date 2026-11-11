//! Outer loop: launches the real app and drives it over the control socket. Run with `cargo e2e`.
//! Ignored by default because each test opens a window. `OTTOSPACE_E2E` turns window hosting off,
//! so tests never touch other apps' windows.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// A running ottospace on a private socket. Killed on drop, so a failed assert leaves no window.
struct Otto {
    child: Child,
    socket: PathBuf,
}

impl Otto {
    fn launch(name: &str) -> Self {
        let socket = std::env::temp_dir().join(format!("ottospace-e2e-{name}-{}.sock", std::process::id()));
        let child = Command::new(env!("CARGO_BIN_EXE_ottospace"))
            .env("OTTOSPACE_E2E", "1")
            .env("OTTOSPACE_SOCKET", &socket)
            .spawn()
            .expect("launch ottospace");
        let otto = Self { child, socket };
        otto.wait_for("the control socket", |o| UnixStream::connect(&o.socket).ok().map(|_| ()));
        otto
    }

    /// Sends one raw request line; returns the parsed reply.
    fn raw(&self, line: &str) -> Value {
        let mut stream = UnixStream::connect(&self.socket).expect("connect");
        // A hung app fails the test instead of hanging it.
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("set timeout");
        writeln!(stream, "{line}").expect("send");
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).expect("reply");
        serde_json::from_str(&reply).expect("reply is JSON")
    }

    fn call(&self, request: Value) -> Value {
        let reply = self.raw(&request.to_string());
        assert_eq!(reply["ok"], true, "{request} failed: {reply}");
        reply
    }

    /// Polls `check` until it returns a value; panics after 10s.
    fn wait_for<T>(&self, what: &str, check: impl Fn(&Self) -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(value) = check(self) {
                return value;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Drawn bounds `[x, y, w, h]` of every pane in `ids`, once all are laid out.
    fn bounds(&self, ids: &[u64]) -> Vec<[f64; 4]> {
        self.wait_for("pane layout", |o| {
            let state = o.call(json!({"cmd": "state"}));
            ids.iter()
                .map(|id| {
                    let b = state["bounds"][id.to_string()].as_array()?;
                    let b: Vec<f64> = b.iter().filter_map(Value::as_f64).collect();
                    (b.len() == 4 && b[2] > 1.0 && b[3] > 1.0).then(|| [b[0], b[1], b[2], b[3]])
                })
                .collect()
        })
    }
}

impl Drop for Otto {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn pane_ids(reply: &Value) -> Vec<u64> {
    reply["state"]["panes"].as_array().unwrap().iter().map(|p| p["id"].as_u64().unwrap()).collect()
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn splits_are_drawn_where_the_layout_says() {
    let otto = Otto::launch("splits");
    assert_eq!(pane_ids(&otto.call(json!({"cmd": "state"}))), [] as [u64; 0]);

    // 1 | 2
    //   | 3
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    let reply = otto.call(json!({"cmd": "split", "axis": "down", "kind": "empty"}));
    assert_eq!(pane_ids(&reply), [1, 2, 3]);
    assert_eq!(reply["state"]["focused"], 3);
    assert_eq!(reply["mode"], "normal");

    let [one, two, three] = otto.bounds(&[1, 2, 3])[..] else { panic!("three panes") };
    assert!(one[0] + one[2] <= two[0], "2 is right of 1: {one:?} {two:?}");
    assert!(two[1] + two[3] <= three[1], "3 is below 2: {two:?} {three:?}");
    assert_eq!(two[0], three[0], "2 and 3 share a column");
    assert!((one[2] - two[2]).abs() < 4.0, "halves are equal: {one:?} {two:?}");
    assert!(one[3] > two[3] * 1.8, "1 is full height: {one:?} {two:?}");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn focus_and_close_follow_the_layout() {
    let otto = Otto::launch("focus");
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "down", "kind": "empty"}));

    assert_eq!(otto.call(json!({"cmd": "focus_dir", "dir": "left"}))["state"]["focused"], 1);
    assert_eq!(otto.call(json!({"cmd": "focus_dir", "dir": "left"}))["state"]["focused"], 1); // no pane there
    assert_eq!(otto.call(json!({"cmd": "focus", "pane": 3}))["state"]["focused"], 3);

    // Closing 2 lets 3 take the whole right column.
    let before = otto.bounds(&[3])[0];
    assert_eq!(pane_ids(&otto.call(json!({"cmd": "close", "pane": 2}))), [1, 3]);
    let after = otto.wait_for("pane 3 to grow", |o| Some(o.bounds(&[3])[0]).filter(|b| b[3] > before[3] * 1.8));
    assert!((after[0] - before[0]).abs() < 2.0, "3 stays in the right column: {before:?} {after:?}");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn terminal_runs_a_typed_command() {
    let otto = Otto::launch("terminal");
    let reply = otto.call(json!({"cmd": "split"}));
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "terminal");

    // The typed line holds `$((6*7))`; only the shell's output holds `42`.
    otto.call(json!({"cmd": "type", "text": "echo otto-$((6*7))\r"}));
    let lines = |o: &Otto| o.call(json!({"cmd": "state"}))["terminals"]["1"]["lines"].clone();
    otto.wait_for("the command's output", |o| {
        lines(o).as_array()?.iter().any(|l| l.as_str().is_some_and(|l| l.contains("otto-42"))).then_some(())
    });

    // A split halves the pane, so the grid loses columns.
    let cols = |o: &Otto| o.call(json!({"cmd": "state"}))["terminals"]["1"]["cols"].as_u64();
    let wide = otto.wait_for("the grid to fit the pane", |o| cols(o).filter(|c| *c > 80));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.wait_for("the grid to shrink", |o| cols(o).filter(|c| *c < wide * 2 / 3));

    // When the shell exits, its pane closes.
    otto.call(json!({"cmd": "type", "pane": 1, "text": "exit\r"}));
    otto.wait_for("the pane to close", |o| (pane_ids(&o.call(json!({"cmd": "state"}))) == [2]).then_some(()));
    assert_eq!(otto.raw(r#"{"cmd":"type","text":"x"}"#)["ok"], false, "no terminal is left");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn secondary_bar_opens_and_closes_beside_the_panes() {
    let otto = Otto::launch("bar");
    let reply = otto.call(json!({"cmd": "split", "kind": "empty"}));
    assert_eq!(reply["state"]["sidebar"], Value::Null, "bar starts closed when hosting is off");
    let closed = otto.bounds(&[1])[0];

    // The bar takes its width from the pane.
    assert_eq!(otto.call(json!({"cmd": "section", "section": "tasks"}))["state"]["sidebar"], "tasks");
    let open =
        otto.wait_for("the pane to move right", |o| Some(o.bounds(&[1])[0]).filter(|b| b[0] > closed[0] + 200.0));
    assert!((closed[2] - open[2] - (open[0] - closed[0])).abs() < 2.0, "{closed:?} {open:?}");

    // Another section replaces it; the same section closes it.
    assert_eq!(otto.call(json!({"cmd": "section", "section": "apps"}))["state"]["sidebar"], "apps");
    assert_eq!(otto.call(json!({"cmd": "section", "section": "apps"}))["state"]["sidebar"], Value::Null);
    otto.wait_for("the pane to move back", |o| Some(o.bounds(&[1])[0]).filter(|b| (b[0] - closed[0]).abs() < 2.0));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn bad_requests_are_refused_and_quit_exits() {
    let mut otto = Otto::launch("errors");
    for bad in ["not json", r#"{"cmd":"nope"}"#, r#"{"cmd":"focus"}"#, r#"{"cmd":"split","extra":1}"#] {
        let reply = otto.raw(bad);
        assert_eq!(reply["ok"], false, "{bad} -> {reply}");
        assert!(reply["error"].is_string());
    }
    // Hosting is off in e2e: the request is refused with the reason, and no pane appears.
    let reply = otto.raw(r#"{"cmd":"open_app","name":"Finder"}"#);
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(reply["error"].as_str().is_some_and(|e| e.contains("e2e")), "{reply}");
    // The failure does not ride on later replies.
    let reply = otto.call(json!({"cmd": "state"}));
    assert_eq!(reply.get("error"), None, "{reply}");
    assert_eq!(pane_ids(&reply), [] as [u64; 0]);

    // A second app whose socket path names an ordinary file must leave that file alone.
    let file = std::env::temp_dir().join(format!("ottospace-e2e-notasocket-{}", std::process::id()));
    std::fs::write(&file, "keep me").expect("write");
    let mut other = Command::new(env!("CARGO_BIN_EXE_ottospace"))
        .env("OTTOSPACE_E2E", "1")
        .env("OTTOSPACE_SOCKET", &file)
        .spawn()
        .expect("launch ottospace");
    std::thread::sleep(Duration::from_millis(1500));
    let kept = std::fs::read_to_string(&file);
    let _ = other.kill();
    let _ = other.wait();
    let _ = std::fs::remove_file(&file);
    assert_eq!(kept.expect("file still exists"), "keep me");

    // `ctl` exits non-zero when the app refuses a request, and zero when it accepts one.
    let ctl = |request: &str| {
        Command::new(env!("CARGO_BIN_EXE_ottospace"))
            .args(["ctl", request])
            .env("OTTOSPACE_SOCKET", &otto.socket)
            .output()
            .expect("run ctl")
            .status
            .success()
    };
    assert!(ctl(r#"{"cmd":"state"}"#));
    assert!(!ctl(r#"{"cmd":"nope"}"#));

    otto.call(json!({"cmd": "quit"}));
    let start = Instant::now();
    while otto.child.try_wait().expect("wait").is_none() {
        assert!(start.elapsed() < Duration::from_secs(5), "quit did not exit the app");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!otto.socket.exists(), "socket file is removed on quit");
}
