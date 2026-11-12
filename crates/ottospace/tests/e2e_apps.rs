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
        let mut command = Command::new(bin);
        // Never the user's settings, tasks or repositories.
        command.env(socket_var, &socket).env_remove("OTTOSPACE_HOME").env("OTTOSPACE_REPOS", "");
        let child = command.envs(env.iter().copied()).spawn().expect("launch");
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
    wait_or(what, String::new, check)
}

/// The one polling loop. `held` is what the panic shows after `TIMEOUT`: the state that never
/// became right.
fn wait_or<T>(what: &str, held: impl Fn() -> String, check: impl Fn() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}: {}", held());
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Sends a key press to whichever app is frontmost, as the user's keyboard would.
fn press(what: &str) {
    let script = format!("tell application \"System Events\" to {what}");
    let status = Command::new("osascript").args(["-e", &script]).status().expect("run osascript");
    assert!(status.success(), "osascript could not send: {what}");
}

/// The front app: its process id and name.
fn front_app() -> (u32, String) {
    let script =
        "tell application \"System Events\" to tell (first process whose frontmost is true) to get {unix id, name}";
    let out = Command::new("osascript").args(["-e", script]).output().expect("run osascript");
    let out = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let (pid, name) = out.split_once(", ").unwrap_or(("0", &out));
    (pid.parse().unwrap_or(0), name.to_string())
}

/// Fails unless the process is the front app, naming the app that was.
fn assert_front(pid: u32, who: &str) {
    let (front, name) = front_app();
    assert!(front == pid, "{who} is the front app: in front was {name} (pid {front})");
}

/// What a wait for a front app shows when it times out: the app that was in front.
fn in_front() -> String {
    let (front, name) = front_app();
    format!("in front was {name} (pid {front})")
}

/// Asks once for the process to come to the front. macOS may refuse.
fn ask_forward(pid: u32) {
    press(&format!("set frontmost of (first process whose unix id is {pid}) to true"));
}

/// Brings the process to the front: asked until it holds.
fn bring_forward(pid: u32, who: &str) {
    wait_or(&format!("{who} to be the front app"), in_front, || {
        ask_forward(pid);
        (front_app().0 == pid).then_some(())
    });
}

/// A real click at a global screen point, or with `double` a real double-click.
fn click_at(x: f64, y: f64, double: bool) {
    let kind = if double { "dc" } else { "c" };
    let status = Command::new("cliclick").arg(format!("{kind}:{x:.0},{y:.0}")).status().expect("run cliclick");
    assert!(status.success(), "cliclick failed");
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
fn a_window_the_rules_ignore_never_becomes_a_pane() {
    // A folder of the test's own, with one rule: leave the canary's second window alone.
    let home = std::env::temp_dir().join(format!("otto-apps-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("make the folder");
    let settings = json!({"windows": [{"app": CANARY, "title": "Canary 2", "do": "ignore"}]});
    std::fs::write(home.join("settings.json"), settings.to_string()).expect("write settings");

    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-rules", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-rules",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
        ],
    );
    let titles = || -> Vec<String> {
        let state = otto.call(json!({"cmd": "state"}));
        let panes = state["state"]["panes"].as_array().cloned().unwrap_or_default();
        panes.iter().filter_map(|p| p["kind"]["title"].as_str().map(str::to_string)).collect()
    };
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(titles(), ["Canary 1"]);

    // The app opens two more windows. The third gets a pane; the second, which the rule names, does not.
    canary.call(json!({"cmd": "open_window"}));
    canary.call(json!({"cmd": "open_window"}));
    wait_for("the third window's pane", || titles().contains(&"Canary 3".to_string()).then_some(()));
    assert_eq!(titles(), ["Canary 1", "Canary 3"]);
    // It was left where the app put it. A window in a pane is parked at the far corner of the
    // screens; the hosted frames show the two panes' windows there and no third.
    let state = otto.call(json!({"cmd": "state"}));
    assert_eq!(state["hosted"].as_object().map(|hosted| hosted.len()), Some(2), "{}", state["hosted"]);
    let _ = std::fs::remove_dir_all(&home);
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
    // macOS keeps a title bar on screen and above the Dock, so the window stops a title bar's
    // height (plus the Dock's, if it shows at the bottom) short of the spot vertically.
    // Horizontally it reaches it: a 1pt sliver stays visible.
    let at_spot = |f: &[f64; 4]| close(f[0], spot[0]) && f[1] <= spot[1] && f[1] > spot[1] - 200.0;
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

    // The app comes to the front by itself, as on a click on its Dock icon. Its window is off
    // screen, so ottospace goes to its pane and gives it the keyboard.
    // First the user is elsewhere: ottospace in front, long enough for it to see that the app
    // is not. It acts when an app comes forward, not while one stays in front.
    bring_forward(otto.child.id(), "ottospace");
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(state()["mode"], "normal", "nothing happens while the app is not in front");
    ask_forward(canary.child.id());
    wait_for("the activated app's pane to go live", || (state()["mode"] == "app").then_some(()));
    wait_for("its window to cover the pane", || {
        frame(0).filter(|f| close(f[0], window[0] + body[0]) && close(f[1], window[1] + body[1] - crop))
    });
    // It stays so: the pane does not bounce back, and the app keeps the keyboard.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(state()["mode"], "app", "still live after a moment");
    assert_front(canary.child.id(), "the canary");
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait_for("the window to park once more", || hosted().filter(|f| at_spot(f) && close(f[3], body[3] + crop)));

    // Sneak mode. Starting it hides the accessibility overlay: both draw chips from one alphabet.
    // Every label is on a control of ottospace's own; none is inside the app's window.
    // The user who types a label is in ottospace: it is the front app.
    bring_forward(otto.child.id(), "ottospace");
    otto.call(json!({"cmd": "ax_overlay"}));
    wait_for("the overlay to show", || (!state()["ax_overlay"].is_null()).then_some(()));
    let labelled = otto.call(json!({"cmd": "sneak"}));
    assert_eq!((&labelled["mode"], &labelled["ax_overlay"]), (&json!("sneak"), &Value::Null), "{labelled}");
    let targets = labelled["sneak"]["targets"].as_array().cloned().unwrap_or_default();
    assert!(targets.iter().all(|t| t["source"]["type"] == "own"), "{targets:?}");
    // The app pane's label does what a click on the pane does, less the click replayed into the
    // app: the app gets the keyboard, which raises its window over the pane.
    let label = targets.iter().find(|t| t["source"]["id"] == "pane:1").and_then(|t| t["label"].as_str());
    let keys: Vec<String> = label.expect("a label on the app pane").chars().map(String::from).collect();
    assert_eq!(otto.call(json!({"cmd": "key", "keys": keys.join(" ")}))["mode"], "app");
    wait_for("the labelled pane's window to cover it", || {
        frame(0).filter(|f| close(f[0], window[0] + body[0]) && close(f[1], window[1] + body[1] - crop))
    });
    assert_eq!(state()["state"]["live"], 1);
    // Not asked for through System Events: the label alone must bring the app forward.
    wait_or("the canary to be the front app after its pane's label", in_front, || {
        (front_app().0 == canary.child.id()).then_some(())
    });
    assert_eq!(canary.call(json!({"cmd": "state"}))["windows"][0]["clicks"], json!([]), "no click is replayed");
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait_for("the window to park after the label", || hosted().filter(|f| at_spot(f) && close(f[3], body[3] + crop)));

    // A real click on the parked pane makes it live and is replayed at the same content point.
    // Needs `cliclick`; clicks only while ottospace is frontmost, so no other app gets it.
    if Command::new("cliclick").arg("-V").output().is_ok() {
        // Leave reactivates ottospace only if the canary had the keyboard; ask until it holds.
        bring_forward(otto.child.id(), "ottospace");
        click_at(global[0] + 100.0, global[1] + crop + 60.0, false);
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
    wait_or("the canary to have the keyboard", in_front, || {
        ask_forward(canary.child.id());
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

/// The middle of a rect.
fn middle(r: [f64; 4]) -> [f64; 2] {
    [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_hosted_pane_is_dragged_by_its_header() {
    // A folder of the test's own, with one rule: the canary's second window is never a pane.
    // It is the other app's window over ottospace at the end.
    let home = std::env::temp_dir().join(format!("otto-apps-drag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("make the folder");
    let settings = json!({"windows": [{"app": CANARY, "title": "Canary 2", "do": "ignore"}]});
    std::fs::write(home.join("settings.json"), settings.to_string()).expect("write settings");

    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-drag", "CANARY_SOCKET", &[]);
    let canary_window = |index: usize| canary.call(json!({"cmd": "state"}))["windows"][index].clone();
    let frame = |index: usize| rect(&canary_window(index)["bounds"]);
    wait_for("the canary window", || frame(0));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-drag",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let mouse = |action: &str, p: [f64; 2]| otto.call(json!({"cmd": "mouse", "action": action, "x": p[0], "y": p[1]}));
    // The state after a fresh frame: a pointer move outside the window draws one first.
    let drawn = || mouse("move", [-50.0, -50.0]);
    let of = |state: &Value, key: &str, id: u64| rect(&state[key][id.to_string()]).filter(|b| b[2] > 1.0);
    let part = |r: [f64; 4], fx: f64, fy: f64| [r[0] + r[2] * fx, r[1] + r[3] * fy];
    // Drags pane `source` by its header to fractions `fx`, `fy` of pane `target`. Injected: see `mouse`.
    let drag_pane = |source: u64, target: u64, fx: f64, fy: f64| {
        let state = drawn();
        let from = middle(of(&state, "headers", source).expect("the header"));
        let to = part(of(&state, "frames", target).expect("the target"), fx, fy);
        otto.call(json!({"cmd": "drag", "from": from, "to": to}))
    };
    let leaf = |id: u64| json!({"leaf": id});
    let split = |axis: &str, a: u64, b: u64| json!({"split": {"axis": axis, "ratio": 0.5, "a": leaf(a), "b": leaf(b)}});
    let spot = point(&state()["parking"]).expect("parking spot");
    let at_spot = |f: &[f64; 4]| close(f[0], spot[0]) && f[1] <= spot[1] && f[1] > spot[1] - 200.0;
    let crop = || state()["crop"]["1"].as_f64().filter(|c| (16.0..=40.0).contains(c));
    // Parked: the real window is at the far corner, as large as the pane's content plus its title
    // bar. Both ottospace's reading of the window and the canary's own agree on the size.
    let parked = |what: &str| {
        wait_for(what, || {
            let state = state();
            let (body, c) = (of(&state, "bounds", 1)?, crop()?);
            let fits = |f: &[f64; 4]| close(f[2], body[2]) && close(f[3], body[3] + c);
            rect(&state["hosted"]["1"]).filter(|f| fits(f) && at_spot(f))?;
            frame(0).filter(fits).map(|_| body)
        })
    };
    // Live: the real window is over the pane, its title bar over the header.
    let live = |what: &str| {
        wait_for(what, || {
            let state = state();
            let (window, body, c) = (rect(&state["window"])?, of(&state, "bounds", 1)?, crop()?);
            frame(0).filter(|f| close(f[0], window[0] + body[0]) && close(f[1], window[1] + body[1] - c))
        })
    };
    let no_clicks = |what: &str| assert_eq!(canary_window(0)["clicks"], json!([]), "{what}: a click reached the app");

    // 1 (the canary) | 2 (empty).
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    let narrow = parked("the window to park at pane size");

    // (a) Not live. Dropped on the bottom of 2, the pane is as wide as both were, and the real
    // window is sized again for it, still parked.
    let reply = drag_pane(1, 2, 0.5, 0.9);
    assert_eq!(reply["state"]["layout"], split("down", 2, 1), "{reply}");
    assert_eq!((reply["mode"].clone(), reply["state"]["live"].clone()), (json!("normal"), Value::Null));
    let wide = parked("the window to park at the new pane's size");
    assert!(wide[2] > narrow[2] * 1.5 && wide[3] < narrow[3] * 0.7, "{narrow:?} -> {wide:?}");
    no_clicks("a drag of a parked pane");

    // (c) Live, the app's own title bar covers the pane's header.
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    let over = live("the window to cover its pane");
    let before = drawn();
    let window = rect(&before["window"]).expect("ottospace window frame");
    let header = of(&before, "headers", 1).expect("the header");
    let header = [window[0] + header[0], window[1] + header[1], header[2], header[3]];
    let bar = crop().expect("a title bar");
    assert!(
        over[0] <= header[0] + 2.0
            && over[1] <= header[1] + 2.0
            && over[0] + over[2] >= header[0] + header[2] - 2.0
            && over[1] + bar >= header[1] + header[3] - 2.0,
        "the title bar {over:?} (top {bar}) covers the header {header:?}"
    );
    // So the user's press there goes to the app. A real click shows it: ottospace gets nothing
    // (a click on ottospace would take the keyboard back and park the app). Needs `cliclick`.
    if Command::new("cliclick").arg("-V").output().is_ok() {
        wait_or("the canary to be in front", in_front, || {
            ask_forward(canary.child.id());
            (canary_window(0)["active"] == true).then_some(())
        });
        let p = middle(header);
        click_at(p[0], p[1], false);
        std::thread::sleep(Duration::from_millis(700));
        let after = state();
        assert_eq!((after["mode"].clone(), after["state"]["live"].clone()), (json!("app"), json!(1)), "still live");
        assert_eq!((&after["drag"], &after["state"]["layout"]), (&Value::Null, &before["state"]["layout"]));
    } else {
        eprintln!("cliclick not found: a real press on a live app's title bar not checked");
    }

    // (b) A drag that does start while the app is live (injected: it passes the real window by)
    // parks the app, and the app stays parked after the drop.
    let from = middle(of(&before, "headers", 1).expect("the header"));
    let to = part(of(&before, "frames", 2).expect("pane 2"), 0.1, 0.5);
    mouse("down", from);
    mouse("move", [from[0] + 8.0, from[1] + 8.0]);
    let reply = mouse("move", to);
    assert_eq!(reply["drag"], json!({"target": 2, "zone": "left"}), "{reply}");
    assert_eq!((reply["mode"].clone(), reply["state"]["live"].clone()), (json!("normal"), Value::Null), "parked");
    let reply = mouse("up", to);
    assert_eq!(reply["state"]["layout"], split("right", 1, 2), "{reply}");
    assert_eq!((reply["mode"].clone(), reply["state"]["live"].clone()), (json!("normal"), Value::Null));
    parked("the window to park at the dropped pane's size");
    std::thread::sleep(Duration::from_millis(800));
    let after = state();
    assert_eq!((after["mode"].clone(), after["state"]["live"].clone()), (json!("normal"), Value::Null), "it stays");
    // One click on the pane goes in again, at its new place. Injected, so nothing is clicked in the app.
    let body = of(&drawn(), "bounds", 1).expect("pane 1");
    mouse("down", middle(body));
    let reply = mouse("up", middle(body));
    assert_eq!((reply["mode"].clone(), reply["state"]["live"].clone()), (json!("app"), json!(1)));
    live("the window to cover the pane's new place");
    // A replayed click would come 80 ms after the window is live: give it time.
    std::thread::sleep(Duration::from_millis(300));
    no_clicks("an injected click on an app pane");

    // (c) After leaving, the header is ottospace's again: the window is at the corner, and a drag works.
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    parked("the window to park after leaving");
    no_clicks("after leaving");
    assert_eq!(drag_pane(1, 2, 0.5, 0.5)["state"]["layout"], split("right", 2, 1));
    parked("the window to park after the swap");

    // Another app's window over ottospace: the canary's second, which is no pane.
    canary.call(json!({"cmd": "open_window"}));
    let other = wait_for("the second canary window", || frame(1));
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(state()["tab_panes"], json!([[2, 1]]), "the second window is no pane");
    if Command::new("cliclick").arg("-V").output().is_ok() {
        wait_or("the canary to be in front", in_front, || {
            ask_forward(canary.child.id());
            (canary_window(0)["active"] == true || canary_window(1)["active"] == true).then_some(())
        });
        // The app came forward: ottospace may give it pane 1 (its first window, live). Let that settle.
        std::thread::sleep(Duration::from_millis(1500));
        let before = drawn();
        let window = rect(&before["window"]).expect("ottospace window frame");
        let on_screen = |p: [f64; 2]| [window[0] + p[0], window[1] + p[1]];
        let inside = |r: [f64; 4], p: [f64; 2]| r[0] < p[0] && p[0] < r[0] + r[2] && r[1] < p[1] && p[1] < r[1] + r[3];
        // From 2's header, which the other window leaves free, to a point of pane 1 under it.
        let from = on_screen(middle(of(&before, "headers", 2).expect("the header")));
        let target = of(&before, "frames", 1).expect("pane 1");
        let to = on_screen(part(target, 0.35, 0.5));
        assert!(!inside(other, from) && inside(other, to), "{other:?} covers {to:?}, not {from:?}");
        // The one real drag of the suite: the pointer itself, through the window server.
        let step = |verb: &str, p: [f64; 2]| format!("{verb}:{:.0},{:.0}", p[0], p[1]);
        let via = [from[0] + 12.0, from[1] + 12.0];
        let half = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
        let moves = [step("dd", from), step("dm", via), step("dm", half), step("dm", to), step("du", to)];
        let status = Command::new("cliclick").args(["-w", "120"]).args(&moves).status().expect("run cliclick");
        assert!(status.success(), "cliclick failed");
        // A press on ottospace brings its window forward, and the pointer is its own until the
        // release: the pane is dropped where the other window was (the centre of 1: a swap), and
        // that window gets nothing.
        let held = || {
            let s = state();
            let held = json!({
                "mode": s["mode"], "live": s["state"]["live"], "focused": s["state"]["focused"],
                "drag": s["drag"], "layout": s["state"]["layout"], "counts": s["counts"],
            });
            format!("{held}; {}; canary windows {}", in_front(), canary.call(json!({"cmd": "state"}))["windows"])
        };
        // Not fatal for now: a real release over another app's window can be lost, on `main` too
        // (about one run in ten on the hosted runner). The drag then never ends. See #56.
        let dropped = (0..100).find_map(|_| {
            std::thread::sleep(Duration::from_millis(100));
            Some(state()).filter(|s| s["state"]["layout"] == split("right", 1, 2))
        });
        match dropped {
            Some(dropped) => {
                assert_eq!((dropped["drag"].clone(), dropped["state"]["focused"].clone()), (Value::Null, json!(2)));
                assert_eq!(canary_window(1)["clicks"], json!([]), "the release did not reach the other window");
            }
            None => {
                eprintln!(
                    "known: a real release over another app's window can be lost, see #56. No drop in 10 s: {}",
                    held()
                );
                // Esc ends the drag, so the app is not left holding one.
                otto.call(json!({"cmd": "key", "keys": "escape"}));
            }
        }
    } else {
        eprintln!("cliclick not found: a real drag released where another app's window was not checked");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_window_that_refuses_the_pane_size_is_shown_unscaled_at_the_pane_top_left() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-size", "CANARY_SOCKET", &[]);
    let windows = || canary.call(json!({"cmd": "state"}))["windows"].clone();
    let frame = |index: usize| rect(&windows()[index]["bounds"]);
    wait_for("the canary window", || frame(0));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-size",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let of = |field: &str, id: u64| rect(&state()[field][id.to_string()]);
    let pane = |id: u64| of("bounds", id).filter(|b| b[2] > 1.0);
    let crop = |id: u64| state()["crop"][id.to_string()].as_f64();
    // A wait that prints what ottospace holds when it times out.
    let held = || {
        let s = state();
        json!({"bounds": s["bounds"], "shown": s["shown"], "hosted": s["hosted"], "crop": s["crop"]}).to_string()
    };
    let wait = |what: &str, check: &dyn Fn() -> Option<[f64; 4]>| wait_or(what, held, check);
    let spot = rect(&json!([state()["parking"][0], state()["parking"][1], 0, 0])).expect("parking spot");
    let at_spot = |f: &[f64; 4]| close(f[0], spot[0]) && f[1] <= spot[1] && f[1] > spot[1] - 200.0;

    // Three panes side by side: an ordinary window, one of a fixed size, one with a minimum width.
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    let whole = wait_for("the pane layout", || pane(1));
    // The fixed window's pane will be about a quarter of this one: on a small screen (a CI
    // runner's) that is under 180 pt, so the window's width follows it, to leave a gap beside it.
    let fixed_size = [(whole[2] / 4.0 - 70.0).round().clamp(90.0, 160.0), 120.0];
    canary.call(json!({"cmd": "open_window", "fixed": fixed_size}));
    wait_for("the fixed window's pane", || (state()["tab_panes"] == json!([[1, 2]])).then_some(()));
    let min_width = (whole[2] * 0.75).round();
    canary.call(json!({"cmd": "open_window", "min": [min_width, 200.0]}));
    wait_for("the minimum-size window's pane", || (state()["tab_panes"] == json!([[1, 2, 3]])).then_some(()));
    // The layout has settled when the three panes share the width the first one had.
    let settled = || {
        let (a, b, c) = (pane(1)?, pane(2)?, pane(3)?);
        (a[2] + b[2] + c[2] <= whole[2] && c[0] > b[0] && b[0] > a[0]).then_some([a, b, c])
    };
    let [ordinary, small, narrow] = wait_for("three panes", settled);
    let window = rect(&state()["window"]).expect("ottospace window frame");
    // Where a pane's content is on the canary's screen, and whether a window's content starts there.
    let over = |f: &[f64; 4], body: &[f64; 4], crop: f64| {
        close(f[0], window[0] + body[0]) && close(f[1], window[1] + body[1] - crop)
    };

    // (c) An ordinary window takes the pane's size, and its preview fills the pane.
    wait("the ordinary window to park at pane size", &|| {
        let c = crop(1)?;
        of("hosted", 1).filter(|f| close(f[2], ordinary[2]) && close(f[3], ordinary[3] + c) && at_spot(f))
    });
    wait("its preview to fill the pane", &|| of("shown", 1).filter(|s| (0..4).all(|i| close(s[i], ordinary[i]))));
    assert!(crop(1) > Some(0.0), "the canary's title bar is cropped");

    // (a) A fixed-size window smaller than its pane keeps its size while parked.
    let own = frame(1).expect("the fixed window");
    assert!(close(own[2], fixed_size[0]), "the fixed window {own:?} has the width asked for, {fixed_size:?}");
    assert!(own[2] + 40.0 < small[2] && own[3] + 160.0 < small[3], "window {own:?} is smaller than pane {small:?}");
    let parked = wait("the fixed window to park", &|| of("hosted", 2).filter(at_spot));
    assert!(close(parked[2], own[2]) && close(parked[3], own[3]), "parked at its own size: {parked:?} vs {own:?}");
    // The preview is the window's content at its own size, at the pane's top left: not the
    // pane's size (stretched), not in the middle (where the live window is not).
    let content = [own[2], own[3] - crop(2).expect("crop")];
    let preview = wait("the fixed window's preview", &|| {
        of("shown", 2).filter(|s| close(s[2], content[0]) && close(s[3], content[1]))
    });
    assert!(close(preview[0], small[0]) && close(preview[1], small[1]), "preview {preview:?} in pane {small:?}");

    // Entering places the real window at the pane's origin, at its own size: where the preview was.
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 2}))["mode"], "app");
    let bar = crop(2).expect("crop");
    let live = wait("the fixed window to sit at its pane's top left", &|| frame(1).filter(|f| over(f, &small, bar)));
    assert!(close(live[2], own[2]) && close(live[3], own[3]), "live at its own size: {live:?} vs {own:?}");
    let preview = of("shown", 2).expect("preview");
    assert!(over(&live, &preview, bar), "the preview {preview:?} is where the live window {live:?} sits");
    let global = of("hosted", 2).expect("live window frame");
    // It stays there: the refused size does not make ottospace move or park it again.
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(state()["mode"], "app");
    assert_eq!(of("hosted", 2), Some(global), "the live window did not move");
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait("the fixed window to park again", &|| of("hosted", 2).filter(at_spot));
    assert_eq!(of("shown", 2), Some(preview), "the preview is unchanged after a visit");

    // (b) A window with a minimum width larger than its pane is cut off, not scaled.
    let wide = wait("the minimum-size window to park", &|| {
        let c = crop(3)?;
        of("hosted", 3).filter(|f| at_spot(f) && close(f[3], narrow[3] + c))
    });
    assert!(wide[2] > narrow[2] + 50.0, "the window {wide:?} is wider than its pane {narrow:?}");
    wait("its preview to fill the pane", &|| of("shown", 3).filter(|s| (0..4).all(|i| close(s[i], narrow[i]))));
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 3}))["mode"], "app");
    let bar = crop(3).expect("crop");
    let live =
        wait("the minimum-size window to sit at its pane's top left", &|| frame(2).filter(|f| over(f, &narrow, bar)));
    assert!(close(live[2], wide[2]) && close(live[3], narrow[3] + bar), "live {live:?}: cut off on the right only");
    let placed = of("hosted", 3);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(state()["mode"], "app");
    assert_eq!(of("hosted", 3), placed, "the live window did not move");
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait("the minimum-size window to park again", &|| of("hosted", 3).filter(at_spot));

    // Real clicks on the fixed window's pane. Needs `cliclick`; clicks only while ottospace is
    // frontmost, so no other app gets one.
    if Command::new("cliclick").arg("-V").output().is_err() {
        eprintln!("cliclick not found: clicks on a small window's pane not checked");
        return;
    }
    let bar = crop(2).expect("crop");
    let clicks = || windows()[1]["clicks"].as_array().cloned().unwrap_or_default();
    let canary_pid = canary.child.id();
    let still_live = || {
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(state()["mode"], "app", "still live after a moment");
    };
    // On the preview: the pane goes live and the click arrives at the same point of the window.
    bring_forward(otto.child.id(), "ottospace");
    click_at(global[0] + 40.0, global[1] + bar + 30.0, false);
    let click = wait_for("the replayed click", || clicks().first().and_then(point));
    assert!(close(click[0], 40.0) && close(click[1], 30.0), "click landed at {click:?}, not (40, 30)");
    still_live();
    // The replayed click is a click on the canary's window, so the canary is in front.
    assert_front(canary_pid, "the canary");
    otto.call(json!({"cmd": "leave"}));
    wait("the fixed window to park after the click", &|| of("hosted", 2).filter(at_spot));

    // In the gap below it: the pane goes live, and no click is replayed onto ottospace.
    bring_forward(otto.child.id(), "ottospace");
    let gap = (global[0] + 40.0, global[1] + global[3] + 100.0);
    click_at(gap.0, gap.1, false);
    wait_for("the gap click to enter the app", || (state()["mode"] == "app").then_some(()));
    wait("the window to sit at its pane's top left", &|| frame(1).filter(|f| over(f, &small, bar)));
    still_live();
    // No click reached the canary, and macOS may refuse an app's own request to come forward
    // while the user works elsewhere: ask through System Events until it holds.
    bring_forward(canary_pid, "the canary");
    assert_eq!(state()["mode"], "app", "still live with the canary in front");
    assert_eq!(clicks().len(), 1, "the gap click did not reach the window");
    // A click in the gap while live is a click on ottospace: it takes the keyboard back.
    click_at(gap.0, gap.1, false);
    wait_for("the second gap click to leave app mode", || (state()["mode"] == "normal").then_some(()));
    wait("the fixed window to park after it", &|| of("hosted", 2).filter(at_spot));
    assert_eq!(clicks().len(), 1, "nor did the second");

    // A double-click in the gap: the second click comes under 400 ms after the pane went live.
    // It leaves all the same: NORMAL, the window parked, the keyboard with ottospace.
    bring_forward(otto.child.id(), "ottospace");
    let went_live = || state()["counts"]["live"].as_u64().expect("the count of panes gone live");
    let before = went_live();
    click_at(gap.0, gap.1, true);
    wait_for("the first click of the two to enter the app", || (went_live() > before).then_some(()));
    wait_for("the second to leave app mode", || (state()["mode"] == "normal").then_some(()));
    wait("the fixed window to park after the double-click", &|| of("hosted", 2).filter(at_spot));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(state()["mode"], "normal", "still NORMAL after a moment");
    assert_eq!(went_live(), before + 1, "the pane went live once");
    assert!(of("hosted", 2).is_some_and(|f| at_spot(&f)), "the window is still parked: {}", held());
    assert_front(otto.child.id(), "ottospace");
    assert_eq!(clicks().len(), 1, "no click of the two reached the window");
}
