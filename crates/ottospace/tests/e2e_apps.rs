//! App-window loop: hosts real windows of the canary app. Run with `cargo e2e-apps`.
//! Needs Accessibility and Screen Recording permission for the terminal that runs it.
//! `OTTOSPACE_HOST_ONLY` limits hosting to the canary, so no other window is ever moved.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How long a wait may take. Generous: a loaded machine can take over 10s to start a shell.
const TIMEOUT: Duration = Duration::from_secs(40);

const CANARY: &str = "ottospace-canary";

/// A child process with a control socket. Killed on drop, so a failed assert leaves no window.
struct Proc {
    child: Child,
    socket: PathBuf,
}

impl Proc {
    fn launch(bin: &str, name: &str, socket_var: &str, env: &[(&str, &str)]) -> Self {
        let socket = std::env::temp_dir().join(format!("otto-apps-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let child = Command::new(bin).env(socket_var, &socket).envs(env.iter().copied()).spawn().expect("launch");
        let proc = Self { child, socket };
        wait_for(&format!("{name}'s socket"), || UnixStream::connect(&proc.socket).ok().map(|_| ()));
        proc
    }

    fn call(&self, request: Value) -> Value {
        let mut stream = UnixStream::connect(&self.socket).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("set timeout");
        writeln!(stream, "{request}").expect("send");
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).expect("reply");
        let reply: Value = serde_json::from_str(&reply).expect("reply is JSON");
        assert_eq!(reply["ok"], true, "{request} failed: {reply}");
        reply
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Polls `check` until it returns a value; panics after `TIMEOUT`.
fn wait_for<T>(what: &str, check: impl Fn() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn rect(value: &Value) -> Option<[f64; 4]> {
    let v: Vec<f64> = value.as_array()?.iter().filter_map(Value::as_f64).collect();
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 2.0
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_canary_window_is_hosted_entered_left_and_restored() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary", "CANARY_SOCKET", &[]);
    let frame = |index: usize| rect(&canary.call(json!({"cmd": "state"}))["windows"][index]["bounds"]);
    let original = wait_for("the canary window", || frame(0));

    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto",
        "OTTOSPACE_SOCKET",
        &[("OTTOSPACE_E2E", "1"), ("OTTOSPACE_HOST_ONLY", CANARY)],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let pane = |id: u64| rect(&state()["bounds"][id.to_string()]).filter(|b| b[2] > 1.0);

    // Host: the pane appears, and the real window is resized to the pane and moved away.
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "app", "{reply}");
    let body = wait_for("the pane layout", || pane(1));
    let parked = wait_for("the window to park at pane size", || {
        frame(0).filter(|f| close(f[2], body[2]) && close(f[3], body[3]) && !close(f[0], original[0]))
    });

    // Enter: the real window sits exactly over the pane.
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    let window = rect(&state()["window"]).expect("ottospace window frame");
    let live = wait_for("the window to cover its pane", || {
        frame(0).filter(|f| close(f[0], window[0] + body[0]) && close(f[1], window[1] + body[1]))
    });
    assert!(close(live[2], body[2]) && close(live[3], body[3]), "live size {live:?} vs pane {body:?}");

    // Leave: parked again, mode back to normal.
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait_for("the window to park again", || frame(0).filter(|f| close(f[0], parked[0]) && close(f[1], parked[1])));

    // A window the app opens itself gets its own pane, to the right, without taking focus.
    canary.call(json!({"cmd": "open_window"}));
    wait_for("the new window's pane", || (state()["tab_panes"] == json!([[1, 2]])).then_some(()));
    assert_eq!(state()["state"]["focused"], 1);
    let (first, second) = (wait_for("pane 1", || pane(1)), wait_for("pane 2", || pane(2)));
    assert!(first[0] + first[2] <= second[0] + 2.0, "pane 2 is to the right: {first:?} {second:?}");

    // Closing that window in the app closes its pane.
    canary.call(json!({"cmd": "close_window", "index": 1}));
    wait_for("the pane to close", || (state()["tab_panes"] == json!([[1]])).then_some(()));

    // Quit: the window goes back where it was.
    otto.call(json!({"cmd": "quit"}));
    wait_for("the window to return to its frame", || {
        frame(0).filter(|f| close(f[0], original[0]) && close(f[1], original[1]) && close(f[2], original[2]))
    });
}
