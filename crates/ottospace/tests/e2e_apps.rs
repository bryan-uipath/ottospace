//! App-window loop: hosts real windows of the canary app. Run with `cargo e2e-apps`.
//! Needs Accessibility and Screen Recording permission for the terminal that runs it.
//! `OTTOSPACE_HOST_ONLY=<canary pid>` limits hosting to that process, so no other window is moved.
//! The run takes keyboard focus while it lasts.

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
        stream.set_read_timeout(Some(TIMEOUT)).expect("set timeout");
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

/// Sends a key press to whichever app is frontmost, as the user's keyboard would.
fn press(what: &str) {
    let script = format!("tell application \"System Events\" to {what}");
    let status = Command::new("osascript").args(["-e", &script]).status().expect("run osascript");
    assert!(status.success(), "osascript could not send: {what}");
}

fn rect(value: &Value) -> Option<[f64; 4]> {
    let v: Vec<f64> = value.as_array()?.iter().filter_map(Value::as_f64).collect();
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

fn point(value: &Value) -> Option<[f64; 2]> {
    let v: Vec<f64> = value.as_array()?.iter().filter_map(Value::as_f64).collect();
    (v.len() == 2).then(|| [v[0], v[1]])
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
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            // Not F18: the user's own ottospace may hold that key, and registration is exclusive.
            ("OTTOSPACE_RETURN_KEY", "f19"),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let pane = |id: u64| rect(&state()["bounds"][id.to_string()]).filter(|b| b[2] > 1.0);

    // The Apps list holds the canary, with its window count and an icon.
    let apps = state()["apps"].clone();
    assert_eq!(apps, json!([{"name": CANARY, "windows": 1, "icon": true}]));

    // Host: the pane appears, and the real window is resized to the pane and moved away.
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "app", "{reply}");
    let body = wait_for("the pane layout", || pane(1));
    // Checked in ottospace's own global coordinates: with two displays, each app's window
    // frame is relative to its own display and cannot be compared with the parking spot.
    let spot = rect(&json!([state()["parking"][0], state()["parking"][1], 0, 0])).expect("parking spot");
    let hosted = || rect(&state()["hosted"]["1"]);
    // macOS keeps a title bar on screen, so the window stops a title bar's height short of the
    // spot vertically. Horizontally it reaches it: a 1pt sliver stays visible.
    let at_spot = |f: &[f64; 4]| close(f[0], spot[0]) && f[1] <= spot[1] && f[1] > spot[1] - 60.0;
    // The canary has a plain native title bar (32pt on macOS 26), so it is cropped: the window
    // parks that much taller, and the stream shows only its content, at pane size.
    let crop = || state()["crop"]["1"].as_f64().filter(|c| (16.0..=40.0).contains(c));
    let crop = wait_for("the window to park at pane size plus its title bar", || {
        let c = crop()?;
        hosted().filter(|f| close(f[2], body[2]) && close(f[3], body[3] + c) && at_spot(f)).map(|_| c)
    });

    // Accessibility: GPUI exposes no controls, so the canary's tree is its window chrome: three
    // pressable traffic lights, all in the title bar the parked stream crops off.
    let tree = otto.call(json!({"cmd": "ax_tree"}));
    assert_eq!(tree["tree"]["root"]["role"], "AXWindow", "{tree}");
    let presses =
        |n: &&Value| n["role"] == "AXButton" && n["actions"].as_array().is_some_and(|a| a.contains(&json!("AXPress")));
    let top = tree["window"]["y"].as_f64().expect("window frame");
    let buttons: Vec<Value> =
        tree["tree"]["root"]["children"].as_array().map_or(vec![], |c| c.iter().filter(presses).cloned().collect());
    assert!(buttons.len() >= 3, "close, minimise and zoom: {tree}");
    assert!(buttons.iter().all(|b| b["frame"]["y"].as_f64().is_some_and(|y| y - top < crop)), "in the bar: {tree}");
    // So the overlay reads the tree but labels nothing: controls the picture does not show are dropped.
    otto.call(json!({"cmd": "ax_overlay"}));
    let overlay =
        wait_for("the overlay to read the tree", || Some(state()["ax_overlay"].clone()).filter(|o| o["pane"] == 1));
    assert!(overlay["error"].is_null() && overlay["count"].as_u64() >= Some(5), "{overlay}");
    assert_eq!(overlay["targets"], json!([]), "{overlay}");
    otto.call(json!({"cmd": "ax_overlay"}));
    assert!(state()["ax_overlay"].is_null(), "off again");

    // Enter: the window is raised by its title bar, so its content sits exactly over the pane's
    // content and its own title bar covers the pane header.
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    let window = rect(&state()["window"]).expect("ottospace window frame");
    let live = wait_for("the window to cover its pane", || {
        frame(0).filter(|f| close(f[0], window[0] + body[0]) && close(f[1], window[1] + body[1] - crop))
    });
    assert!(close(live[2], body[2]) && close(live[3], body[3] + crop), "live size {live:?} vs pane {body:?}");
    // The pane border stays visible: a 2pt gap inside it on all four sides, the top included.
    let inside = rect(&state()["frames"]["1"]).expect("pane frame");
    let (left, top) = (window[0] + inside[0], window[1] + inside[1]);
    let gap = |d: f64| (d - 2.0).abs() <= 0.5;
    assert!(live[1] >= top && gap(live[1] - top), "top gap: live {live:?}, inside border {inside:?}");
    assert!(gap(live[0] - left), "left gap: live {live:?}, inside border {inside:?}");
    assert!(gap(left + inside[2] - live[0] - live[2]), "right gap: live {live:?}, inside border {inside:?}");
    assert!(gap(top + inside[3] - live[1] - live[3]), "bottom gap: live {live:?}, inside border {inside:?}");
    // The live window in global coordinates, to click the pane at a known screen point below.
    let global = hosted().expect("live window frame");

    // Leave: parked again, mode back to normal.
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait_for("the window to park again", || hosted().filter(|f| at_spot(f) && close(f[3], body[3] + crop)));

    // A real click on the parked pane makes it live and is replayed at the same content point.
    // Needs `cliclick`; clicks only while ottospace is frontmost, so no other app gets it.
    if Command::new("cliclick").arg("-V").output().is_ok() {
        let front = || {
            let script = "tell application \"System Events\" to unix id of first process whose frontmost is true";
            let out = Command::new("osascript").args(["-e", script]).output().expect("run osascript");
            String::from_utf8_lossy(&out.stdout).trim() == otto.child.id().to_string()
        };
        // Leave reactivates ottospace only if the canary had the keyboard; ask until it holds.
        let raise = format!("set frontmost of (first process whose unix id is {}) to true", otto.child.id());
        wait_for("ottospace to be frontmost", || {
            press(&raise);
            front().then_some(())
        });
        let (x, y) = (global[0] + 100.0, global[1] + crop + 60.0);
        let status = Command::new("cliclick").arg(format!("c:{x:.0},{y:.0}")).status().expect("run cliclick");
        assert!(status.success(), "cliclick failed");
        let clicks = || canary.call(json!({"cmd": "state"}))["windows"][0]["clicks"].as_array().cloned();
        let click = wait_for("the replayed click", || clicks()?.first().and_then(point));
        assert!(close(click[0], 100.0) && close(click[1], 60.0), "click landed at {click:?}, not (100, 60)");
        assert_eq!(state()["mode"], "app");
        otto.call(json!({"cmd": "leave"}));
        wait_for("the window to park after the click", || hosted().filter(at_spot));
    } else {
        eprintln!("cliclick not found: click replay not checked");
    }

    // While live, the keyboard belongs to the app: a typed key arrives in the canary.
    otto.call(json!({"cmd": "enter", "pane": 1}));
    wait_for("the window to cover its pane", || frame(0).filter(|f| close(f[0], live[0]) && close(f[1], live[1])));
    // Typed keys go to whichever app is frontmost. macOS may refuse the app's own request to come
    // forward while the user works elsewhere, so ask through System Events until it holds.
    let front = format!("set frontmost of (first process whose unix id is {}) to true", canary.child.id());
    wait_for("the canary to have the keyboard", || {
        press(&front);
        (canary.call(json!({"cmd": "state"}))["windows"][0]["active"] == true).then_some(())
    });
    press("keystroke \"k\"");
    wait_for("the key to reach the canary", || {
        let keys = canary.call(json!({"cmd": "state"}))["windows"][0]["keys"].clone();
        keys.as_array()?.iter().any(|k| k == "k").then_some(())
    });

    // The global return key takes the keyboard back from the app, with no click.
    assert_eq!(state()["return_key"], "F19", "the return key is registered");
    press("key code 80"); // F19
    wait_for("the return key to leave app mode", || (state()["mode"] == "normal").then_some(()));
    wait_for("the window to park after the return key", || hosted().filter(at_spot));

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
    wait_for("the window to return to its frame", || frame(0).filter(|f| (0..4).all(|i| close(f[i], original[i]))));
}
