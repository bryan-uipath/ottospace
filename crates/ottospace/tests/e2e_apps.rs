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

mod support;

/// How long a wait may take. Generous: a loaded machine can take over 10s to start a shell.
const TIMEOUT: Duration = Duration::from_secs(40);

const CANARY: &str = "ottospace-canary";

/// A child process with a control socket. Killed on drop, so a failed assert leaves no window.
struct Proc {
    child: Child,
    socket: PathBuf,
    /// `otto` or `canary`, for the timeline.
    who: &'static str,
}

impl Proc {
    fn launch(bin: &str, name: &str, socket_var: &str, env: &[(&str, &str)]) -> Self {
        let socket = support::temp(&format!("otto-apps-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut command = Command::new(bin);
        // Never the user's settings, tasks or repositories.
        command.env(socket_var, &socket).env_remove("OTTOSPACE_HOME").env("OTTOSPACE_REPOS", "");
        // Nor a key taken from every app, nor flags, unless the test names them.
        command.env_remove("OTTOSPACE_CAPTURE_KEY").env_remove("OTTOSPACE_FEATURES");
        let child = command.envs(env.iter().copied()).spawn().expect("launch");
        let who = if socket_var == "OTTOSPACE_SOCKET" { "otto" } else { "canary" };
        if who == "otto" {
            support::launched(child.id(), &socket);
        }
        let proc = Self { child, socket, who };
        wait_for(&format!("{name}'s socket"), || UnixStream::connect(&proc.socket).ok().map(|_| ()));
        proc
    }

    fn call(&self, request: Value) -> Value {
        let (line, started) = (request.to_string(), Instant::now());
        let mut stream = UnixStream::connect(&self.socket).expect("connect");
        stream.set_read_timeout(Some(TIMEOUT)).expect("set timeout");
        writeln!(stream, "{request}").expect("send");
        let mut reply = String::new();
        if let Err(err) = BufReader::new(stream).read_line(&mut reply) {
            panic!("{}", support::no_reply(self.who, &self.socket, &line, started, &err));
        }
        let reply: Value = serde_json::from_str(&reply).expect("reply is JSON");
        support::request(self.who, &self.socket, &line, started, Some(&reply));
        assert_eq!(reply["ok"], true, "{request} failed: {reply}");
        reply
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if self.who == "otto" {
            support::exiting(self.child.id());
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A killed app removes nothing: its socket, the lock beside it, a trace it dumped, the
        // folder of its agents' pass files.
        let _ = std::fs::remove_dir_all(self.socket.with_extension("passes"));
        for end in ["sock", "lock", "trace.jsonl"] {
            let _ = std::fs::remove_file(self.socket.with_extension(end));
        }
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
            support::waited(what, start, true);
            return value;
        }
        if start.elapsed() >= TIMEOUT {
            panic!("{}", support::timed_out(what, start, &held()));
        }
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
    released(x, y);
}

/// Moves the pointer to `x, y` in many small steps, as a hand does: a panel of a background app
/// hears its moves by a tracking area, which one jump may cross unheard.
fn glide(x: f64, y: f64) {
    let status =
        Command::new("cliclick").args(["-e", "100", &format!("m:{x:.0},{y:.0}")]).status().expect("run cliclick");
    assert!(status.success(), "cliclick failed");
}

/// Waits for the system to read the left button as up, releasing again at `x, y` until it does.
/// About one `cliclick` release in six is not taken on a hosted runner; the button then reads as
/// down for the tests after, and a wheel over a parked pane (a drag, to ottospace) enters nothing.
fn released(x: f64, y: f64) {
    let up = (0..20).any(|_| {
        std::thread::sleep(Duration::from_millis(150));
        if !ottospace_mac::input::left_button_down() {
            return true;
        }
        eprintln!("the system still reads the left button as down: releasing again");
        let again = Command::new("cliclick").arg(format!("du:{x:.0},{y:.0}")).status().expect("run cliclick");
        assert!(again.success(), "cliclick failed");
        false
    });
    assert!(up, "the system still reads the left button as down after 20 releases");
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
    let home = support::temp(&format!("otto-apps-rules-{}", std::process::id()));
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

    // The app opens two more windows. The third is offered, and the user's click puts it in a
    // pane; the second, which the rule names, is not offered.
    canary.call(json!({"cmd": "open_window"}));
    canary.call(json!({"cmd": "open_window"}));
    let offered = wait_offers(&otto, 1);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        (wait_offers(&otto, 1), titles()),
        (offered.clone(), vec!["Canary 1".to_string()]),
        "no split by itself"
    );
    take_offer(&otto, offered[0]);
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
fn a_pop_up_floats_over_its_apps_pane_at_its_own_size_and_a_big_window_is_offered_not_split() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-popup", "CANARY_SOCKET", &[]);
    let frame = |index: usize| rect(&canary.call(json!({"cmd": "state"}))["windows"][index]["bounds"]);
    wait_for("the canary window", || frame(0));
    let otto = otto_for(&canary, "otto-popup", &[]);
    let state = || otto.call(json!({"cmd": "state"}));
    let pane = |id: u64| rect(&state()["bounds"][id.to_string()]).filter(|b| b[2] > 1.0);
    let hosted = || state()["hosted"].as_object().map_or(0, |hosted| hosted.len());
    let spot = host_and_park(&otto);
    let body = wait_for("the pane layout", || pane(1));
    // The pane's content on the canary's screen.
    let window = rect(&state()["window"]).expect("ottospace window frame");
    let over_pane = |f: [f64; 4]| {
        let [x, y] = middle(f);
        x > window[0] + body[0]
            && x < window[0] + body[0] + body[2]
            && y > window[1] + body[1]
            && y < window[1] + body[1] + body[3]
    };

    // A dialog the app opens over its parked window, off screen, as macOS places one by its
    // parent: no pane, the app's pane as it was, the dialog over it at its own size. 700×450 is
    // no small window: only its subrole makes it a pop-up.
    canary.call(json!({"cmd": "dialog", "at": spot}));
    // Settled: where it is now is where it was a moment ago (the canary and ottospace both move it).
    let dialog = wait_for("the dialog to settle over the pane", || {
        let before = frame(1).filter(|f| over_pane(*f))?;
        std::thread::sleep(Duration::from_millis(800));
        (frame(1) == Some(before)).then_some(before)
    });
    // GPUI's bounds hold the title bar too.
    assert!(close(dialog[2], 700.0) && (450.0..520.0).contains(&dialog[3]), "its own size: {dialog:?}");
    let screens = ottospace_mac::recovery::screens().visible;
    let whole = |f: [f64; 4]| {
        screens.iter().any(|s| f[0] >= s.x && f[1] >= s.y && f[0] + f[2] <= s.x + s.w && f[1] + f[3] <= s.y + s.h)
    };
    assert!(whole(dialog), "moved wholly onto a screen: {dialog:?} {screens:?}");
    // A small standard window: the same.
    canary.call(json!({"cmd": "open_window", "fixed": [420.0, 300.0]}));
    wait_for("the small window", || frame(2));
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(state()["tab_panes"], json!([[1]]), "no pane for either");
    assert_eq!(hosted(), 1);
    assert_eq!(pane(1), Some(body), "the app's pane kept its rect");
    assert_eq!(frame(1), Some(dialog), "the dialog stays where it was put");
    let small = frame(2).expect("the small window");
    assert!(close(small[2], 420.0) && reachable(small), "the small one, at its size, on screen: {small:?}");

    // A dialog the app opens on screen beside ottospace's window, its title bar in sight: the
    // user can reach it, so it is raised where it is, not moved over the pane.
    // It needs 140 beside ottospace's window: none on a laptop's screen, where it is skipped.
    let main = screens[0];
    let (left, right) = (window[0] - main.x, main.x + main.w - (window[0] + window[2]));
    let room = left.max(right) >= 140.0;
    if !room {
        eprintln!("skipped the dialog beside ottospace: no room beside {window:?} on {main:?}");
    } else {
        let x = if right >= left { window[0] + window[2] + 10.0 } else { window[0] - 710.0 };
        let at = [x, window[1] + 100.0];
        // Judged: a `float` newer than the trace's last line now. By time, not count: the ring drops old lines.
        let trace = || otto.call(json!({"cmd": "trace"}))["trace"].as_str().unwrap_or_default().to_string();
        let ms = |line: &str| serde_json::from_str::<Value>(line).ok().and_then(|line| line["ms"].as_u64());
        let since = trace().lines().last().and_then(ms);
        canary.call(json!({"cmd": "dialog", "at": at}));
        let judged = wait_for("the dialog beside to be judged", || {
            trace().lines().find(|l| l.contains(r#""ev":"float""#) && ms(l) > since).map(str::to_string)
        });
        assert!(judged.contains(r#""moved":false"#), "{judged}");
        let beside = frame(3).expect("the dialog beside");
        assert!(close(beside[0], at[0]) && close(beside[1], at[1]), "left where the app put it: {beside:?} {at:?}");
        assert!(!over_pane(beside) && reachable(beside), "{beside:?}");
    }

    // A big standard window floats too, offered (as the small one and the dialogs are): no size splits a pane.
    // The user's click on the offer puts it in a pane of its own, to the right.
    // The dialogs and the small window are offered too: one more for the big one.
    let before = state()["state"]["offers"].as_array().map_or(0, Vec::len);
    canary.call(json!({"cmd": "open_window", "min": [body[2] * 0.6, 420.0]}));
    let offered = wait_offers(&otto, before + 1);
    assert_eq!((state()["tab_panes"].clone(), hosted()), (json!([[1]]), 1), "no split by itself");
    take_offer(&otto, offered[before]);
    wait_for("the big window's pane", || (state()["tab_panes"] == json!([[1, 2]])).then_some(()));
    assert_eq!(hosted(), 2);
    assert_eq!(state()["state"]["panes"][1]["kind"]["title"], if room { "Canary 5" } else { "Canary 4" });
}

/// The owner's Zoom call: a popped-out chat (an `AXSystemDialog`) could not be put in a pane. A
/// system dialog of the hosted canary is offered and listed by the picker; the user's click on
/// the offer puts it in a pane of its own.
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_system_dialog_is_offered_and_listed_and_the_users_click_puts_it_in_a_pane() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-sysdialog", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = otto_for(&canary, "otto-sysdialog", &[]);
    let state = || otto.call(json!({"cmd": "state"}));
    let spot = host_and_park(&otto);
    canary.call(json!({"cmd": "dialog", "at": spot, "subrole": "AXSystemDialog"}));
    let offered = wait_offers(&otto, 1);
    // The window Accessibility reads is the system dialog asked for.
    let pid = canary.child.id() as i32;
    let read = ottospace_mac::ax::Window::all_of(pid).into_iter().find(|w| w.id() == Some(offered[0] as u32));
    assert_eq!(read.and_then(|w| w.subrole()).as_deref(), Some("AXSystemDialog"));
    // ⌘E, the canary: its two windows listed, the system dialog too.
    otto.call(json!({"cmd": "picker"}));
    otto.call(json!({"cmd": "picker_query", "query": CANARY}));
    let reply = otto.call(json!({"cmd": "picker_choose"}));
    let rows = reply["picker_rows"].as_array().cloned().unwrap_or_default();
    let listed: Vec<u64> = rows.iter().filter(|r| r["type"] == "window").filter_map(|r| r["window"].as_u64()).collect();
    assert!(listed.contains(&offered[0]) && listed.len() == 2, "{reply}");
    otto.call(json!({"cmd": "key", "keys": "escape", "as": "user"}));
    take_offer(&otto, offered[0]);
    wait_for("the dialog's pane", || (state()["tab_panes"] == json!([[1, 2]])).then_some(()));
    assert_eq!(state()["hosted"].as_object().map(|hosted| hosted.len()), Some(2));
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
    assert_eq!(tree["captured"]["tree"]["role"], "AXWindow", "{tree}");
    let presses =
        |n: &&Value| n["role"] == "AXButton" && n["actions"].as_array().is_some_and(|a| a.contains(&json!("AXPress")));
    let top = tree["window"]["y"].as_f64().expect("window frame");
    let buttons: Vec<Value> =
        tree["captured"]["tree"]["children"].as_array().map_or(vec![], |c| c.iter().filter(presses).cloned().collect());
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

    // A window the app opens itself is offered; the user's click gives it its own pane, to the
    // right, without taking focus.
    canary.call(json!({"cmd": "open_window"}));
    take_offer(&otto, wait_offers(&otto, 1)[0]);
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

/// A leave right after going live stays: the app's own activation from going live, first seen by
/// the front-app check (every 500 ms) after the leave, is not the user bringing it forward.
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_quick_leave_is_not_undone_by_the_apps_own_activation() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-quick", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-quick", &[]);
    host_and_park(&otto);
    // Three times: each leave lands before that check only by chance.
    for _ in 0..3 {
        assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
        bring_forward(canary.child.id(), "the canary");
        assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
        // Past two front-app checks.
        std::thread::sleep(Duration::from_millis(1200));
        assert_eq!(otto.call(json!({"cmd": "state"}))["mode"], "normal", "the leave stays");
        assert_eq!(front_app().0, otto.child.id(), "and ottospace has the keyboard: {}", in_front());
    }
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
    let home = support::temp(&format!("otto-apps-drag-{}", std::process::id()));
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
        // A release not taken would make the drag rightly go on: a finger cannot do that.
        released(to[0], to[1]);
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
        let dropped =
            wait_or("the drop", held, || Some(state()).filter(|s| s["state"]["layout"] == split("right", 1, 2)));
        assert_eq!((dropped["drag"].clone(), dropped["state"]["focused"].clone()), (Value::Null, json!(2)));
        assert_eq!(canary_window(1)["clicks"], json!([]), "the release did not reach the other window");
    } else {
        eprintln!("cliclick not found: a real drag released where another app's window was not checked");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_dragged_divider_sizes_the_hosted_window_once_on_release() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-divider", "CANARY_SOCKET", &[]);
    let window = || canary.call(json!({"cmd": "state"}))["windows"][0].clone();
    let frame = || rect(&window()["bounds"]);
    wait_for("the canary window", frame);
    let otto = otto_for(&canary, "otto-divider", &[]);
    let state = || otto.call(json!({"cmd": "state"}));
    let mouse = |action: &str, x: f64, y: f64| {
        otto.call(json!({"cmd": "mouse", "action": action, "x": x, "y": y, "as": "user"}));
    };
    let body = |s: &Value| rect(&s["bounds"]["1"]).filter(|b| b[2] > 1.0);
    // Parked as wide as its pane's content.
    let fits = |what: &str| {
        wait_for(what, || {
            let b = body(&state())?;
            frame().filter(|f| close(f[2], b[2])).map(|_| b)
        })
    };
    // 1 (the canary) | 2 (empty).
    host_and_park(&otto);
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    let before = fits("parked at its pane's width");
    let s = state();
    let (one, two) = (rect(&s["frames"]["1"]).expect("pane 1"), rect(&s["frames"]["2"]).expect("pane 2"));
    let (x, y) = ((one[0] + one[2] + two[0]) / 2.0, one[1] + one[3] / 2.0);

    // Held: the pane is narrower at once, the window is not asked to follow each move.
    mouse("down", x, y);
    mouse("move", x - 20.0, y);
    mouse("move", x - 80.0, y);
    let held = wait_for("the pane narrower", || body(&state()).filter(|b| b[2] < before[2] - 60.0));
    std::thread::sleep(Duration::from_millis(600));
    assert!(frame().is_some_and(|f| close(f[2], before[2])), "resized while held: {:?} for {held:?}", frame());

    // Released: sized to its pane once.
    mouse("up", x - 80.0, y);
    let after = fits("sized to the pane on release");
    assert!(close(after[2], before[2] - 80.0), "{before:?} -> {after:?}");
    assert_eq!(window()["clicks"], json!([]), "the drag reached the app");
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_window_that_refuses_the_pane_size_is_shown_unscaled_at_the_pane_top_left() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-size", "CANARY_SOCKET", &[]);
    let windows = || canary.call(json!({"cmd": "state"}))["windows"].clone();
    let frame = |index: usize| rect(&windows()[index]["bounds"]);
    wait_for("the canary window", || frame(0));
    let home = splitting_home("size");
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-size",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
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

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_refused_size_is_named_in_the_pane_header_and_fit_gives_the_window_room() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-fit", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let home = splitting_home("fit");
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-fit",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let of = |field: &str, id: u64| rect(&state()[field][id.to_string()]);
    let pane = |id: u64| of("bounds", id).filter(|b| b[2] > 1.0);
    // A pane's `fit`: null when its window has the pane's size.
    let fit = |id: usize| state()["state"]["panes"][id - 1]["fit"].clone();
    // A wait that prints the panes when it times out.
    let held = || {
        let s = state();
        json!({"bounds": s["bounds"], "shown": s["shown"], "panes": s["state"]["panes"]}).to_string()
    };
    let wait = |what: &str, check: &dyn Fn() -> Option<[f64; 4]>| wait_or(what, held, check);
    let refused = |request: Value| {
        let mut stream = UnixStream::connect(&otto.socket).expect("connect");
        writeln!(stream, "{request}").expect("send");
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).expect("reply");
        let reply: Value = serde_json::from_str(&reply).expect("reply is JSON");
        reply["error"].as_str().unwrap_or_else(|| panic!("{request} was not refused: {reply}")).to_string()
    };
    let ratio = |path: &[&str]| path.iter().fold(state()["state"]["layout"].clone(), |at, key| at[*key].clone());
    // The Repos section, open at start, closes: on a CI runner's small screen the panes need
    // its room.
    otto.call(json!({"cmd": "section", "section": "repos"}));

    // Three panes side by side, 1 | (2 | 3): ordinary, fixed size, a minimum width.
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    let whole = wait_for("the pane layout", || pane(1));
    // The fixed window's pane will be about a quarter of the first: on a small screen (a CI
    // runner's) that is about 170 pt. The window is at least 24 pt narrower, or it would count
    // as fitting (an app that snaps its size), and wide enough for a header with its ×.
    let fixed = (whole[2] / 4.0 - 50.0).round().clamp(120.0, 160.0);
    let fixed_hint = format!("fixed size {fixed}×120");
    canary.call(json!({"cmd": "open_window", "fixed": [fixed, 120.0]}));
    wait_for("the fixed window's pane", || (state()["tab_panes"] == json!([[1, 2]])).then_some(()));
    // A little wider than its quarter: fitted, it must leave pane 2 more than 24 pt wider than
    // the fixed window, on a small screen too.
    let min_width = (whole[2] * 0.27).round();
    canary.call(json!({"cmd": "open_window", "min": [min_width, 200.0]}));
    wait_for("the minimum-size window's pane", || (state()["tab_panes"] == json!([[1, 2, 3]])).then_some(()));
    let narrow = wait("three panes", &|| pane(3).filter(|c| c[2] < whole[2] * 0.26));
    let half = wait("pane 1", &|| pane(1))[2];

    // The hints: what each window kept. The ordinary window took its pane's size: no hint.
    let more = (min_width - narrow[2]).round();
    wait("the cut-off window's hint", &|| (fit(3)["hint"] == format!("needs {more} pt more width")).then_some(narrow));
    let wants = |id: usize| point(&fit(id)["wants"]).expect("wants");
    assert!(close(wants(3)[0], min_width) && close(wants(3)[1], narrow[3]), "{}", fit(3));
    wait("the fixed window's hint", &|| (fit(2)["hint"] == fixed_hint.as_str()).then_some(narrow));
    assert!(close(wants(2)[0], fixed) && close(wants(2)[1], 120.0), "{}", fit(2));
    assert!(fit(2)["stuck"].is_null(), "its width can be fitted: {}", fit(2));
    assert!(fit(1).is_null(), "{}", fit(1));
    assert_eq!(ratio(&["split", "b", "split", "ratio"]), 0.5, "nothing moved by itself");

    // Window frames are read on the half-second check, not on every 16 ms tick: three windows,
    // two or three checks a second. A read each tick would be over 180 a second.
    let reads = || state()["counts"]["frame_reads"].as_u64().expect("the count of frame reads");
    let before = reads();
    std::thread::sleep(Duration::from_millis(2000));
    let read = reads() - before;
    assert!((3..=30).contains(&read), "{read} window frame reads in 2 s");

    // A pane that does not exist, and one whose window fits: said, or nothing to do.
    assert_eq!(refused(json!({"cmd": "fit", "pane": 9})), "no pane 9");
    otto.call(json!({"cmd": "fit", "pane": 1}));
    assert_eq!(ratio(&["split", "ratio"]), 0.5, "a window that fits moves nothing");

    // Fit the cut-off pane: it grows to its window's width, its preview fills it, the hint goes.
    // Asked twice at once, as a double-click on `fit` would: the second finds no fresh measure
    // and moves nothing more.
    otto.call(json!({"cmd": "fit", "pane": 3}));
    otto.call(json!({"cmd": "fit", "pane": 3}));
    let grown = wait("pane 3 to have its window's width", &|| pane(3).filter(|b| close(b[2], min_width)));
    wait("its preview to fill it", &|| of("shown", 3).filter(|s| (0..4).all(|i| close(s[i], grown[i]))));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(fit(3).is_null(), "its hint went: {}", fit(3));
    assert!(close(wait("pane 3", &|| pane(3))[2], min_width), "it kept the window's width: {}", held());
    assert!(close(wait("pane 1", &|| pane(1))[2], half), "pane 1 kept its half");

    // Fit the fixed-size pane, once it was measured in the new layout: it shrinks to the
    // window's width. No split is above it on the other axis, so its height stays.
    wait("the fixed window's hint again", &|| (fit(2)["hint"] == fixed_hint.as_str()).then_some(narrow));
    otto.call(json!({"cmd": "fit", "pane": 2}));
    let shrunk = wait("pane 2 to have its window's width", &|| pane(2).filter(|b| close(b[2], fixed)));
    assert!(close(shrunk[3], narrow[3]), "its height stayed: {shrunk:?}");
    // The hint stays for the height and says why no `fit` is offered; the socket says it too.
    wait("the hint to say nothing can move", &|| (fit(2)["stuck"] == "no_split").then_some(shrunk));
    assert_eq!(fit(2)["hint"], fixed_hint.as_str());
    assert_eq!(refused(json!({"cmd": "fit", "pane": 2})), "pane 2 cannot be fitted: no split to move");
    // In a pane that narrow (120 to 160 pt) the header still holds its ×.
    let s = state();
    let (x, frame) = (rect(&s["closes"]["2"]).expect("pane 2's ×"), rect(&s["frames"]["2"]).expect("pane 2"));
    assert!(x[2] > 1.0 && x[0] > frame[0] && x[0] + x[2] <= frame[0] + frame[2], "× {x:?} in pane {frame:?}");
    // Pane 3 took the room; its window can grow, so it fits still.
    assert!(fit(3).is_null(), "{}", fit(3));

    // A pane in a background tab is fitted where it is. Its measure is the last one taken while
    // it was on show: pane 3 is made 100 pt too narrow first.
    otto.call(json!({"cmd": "focus", "pane": 3}));
    let before = wait("pane 3", &|| pane(3));
    // A step at a time until its window is cut off: how far a step goes depends on the screen.
    let hinted = || fit(3)["hint"].as_str().is_some_and(|h| h.starts_with("needs "));
    let mut cut = before;
    for _ in 0..6 {
        otto.call(json!({"cmd": "key", "keys": "cmd-shift-alt-l"}));
        let was = cut[2];
        cut = wait("pane 3 to be narrower", &|| pane(3).filter(|b| b[2] < was - 1.0));
        std::thread::sleep(Duration::from_millis(1200));
        if hinted() {
            break;
        }
    }
    wait("its hint", &|| hinted().then_some(cut));
    let inner = ratio(&["split", "b", "split", "ratio"]);
    otto.call(json!({"cmd": "new_tab"}));
    assert_eq!(state()["state"]["active"], 1);
    otto.call(json!({"cmd": "fit", "pane": 3}));
    assert_eq!(state()["state"]["active"], 1, "its tab stays in the background");
    otto.call(json!({"cmd": "select_tab", "index": 0}));
    assert!(ratio(&["split", "b", "split", "ratio"]).as_f64() < inner.as_f64(), "the split moved: was {inner}");
    wait("pane 3 to have its window's width again", &|| pane(3).filter(|b| close(b[2], min_width)));
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_mark_hosts_its_canary_window_again_and_of_two_windows_the_one_with_its_title() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-marks", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-marks",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_FEATURES", "marks"),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    // Each app pane's id and window title, in pane order.
    let titled = |state: &Value| -> Vec<(u64, String)> {
        let panes = state["state"]["panes"].as_array().cloned().unwrap_or_default();
        let app = |p: &Value| Some((p["id"].as_u64()?, p["kind"]["title"].as_str()?.to_string()));
        panes.iter().filter_map(app).collect()
    };
    let mark = |state: &Value, name: &str| -> Value {
        let marks = state["marks"].as_array().cloned().unwrap_or_default();
        marks.into_iter().find(|m| m["name"] == name).unwrap_or_else(|| panic!("no mark {name}: {}", state["marks"]))
    };

    // One window in a pane, marked; the app opens a second, which gets its own pane and mark.
    let hosted = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(titled(&hosted), [(1, "Canary 1".to_string())]);
    assert_eq!(hosted["state"]["panes"][0]["kind"]["name"], CANARY, "the pane says whose window it is");
    let one = otto.call(json!({"cmd": "mark_add", "pane": 1, "name": "one", "key": "a"}));
    assert_eq!(
        (&one["mark"]["kind"], &one["mark"]["open"], &one["mark"]["pane"]),
        (&json!("app"), &json!(true), &json!(1))
    );
    canary.call(json!({"cmd": "open_window"}));
    take_offer(&otto, wait_offers(&otto, 1)[0]);
    wait_for("the second window's pane", || (titled(&state()).len() == 2).then_some(()));
    assert_eq!(titled(&state())[1], (2, "Canary 2".to_string()));
    otto.call(json!({"cmd": "mark_add", "pane": 2, "name": "two", "key": "b"}));

    // Both panes close: the windows go back to the app, and the marks stay, not open.
    otto.call(json!({"cmd": "close", "pane": 1}));
    let closed = otto.call(json!({"cmd": "close", "pane": 2}));
    assert_eq!((titled(&closed), closed["hosted"].as_object().map(|h| h.len())), (vec![], Some(0)));
    for name in ["one", "two"] {
        let row = mark(&closed, name);
        assert_eq!((&row["open"], &row["available"]), (&json!(false), &json!(true)), "{row}");
    }

    // Going to a mark hosts its window again, in a new pane: of the two free windows, the one
    // with its title, whichever the app calls its main window.
    let second = otto.call(json!({"cmd": "mark_go", "name": "two"}));
    assert_eq!(titled(&second), [(3, "Canary 2".to_string())], "{}", second["state"]["panes"]);
    assert_eq!(
        (&mark(&second, "two")["pane"], &second["state"]["focused"], &second["mode"]),
        (&json!(3), &json!(3), &json!("normal"))
    );
    let first = otto.call(json!({"cmd": "key", "keys": "cmd-' a"}));
    assert_eq!(titled(&first), [(3, "Canary 2".to_string()), (4, "Canary 1".to_string())]);
    assert_eq!((&mark(&first, "one")["pane"], &first["state"]["focused"]), (&json!(4), &json!(4)));
    assert_eq!(first["hosted"].as_object().map(|h| h.len()), Some(2), "each window is in its own pane");
    // Open now: going again only focuses, and hosts nothing twice.
    let again = otto.call(json!({"cmd": "mark_go", "key": "b"}));
    assert_eq!((titled(&again).len(), &again["state"]["focused"]), (2, &json!(3)));

    // A mark of the tab: closed, it is built again with each window in its own pane, by title.
    otto.call(json!({"cmd": "mark_add", "tab": 1, "name": "both"}));
    let emptied = otto.call(json!({"cmd": "close_tab", "index": 0}));
    assert_eq!((titled(&emptied), mark(&emptied, "both")["open"].clone()), (vec![], json!(false)));
    otto.call(json!({"cmd": "mark_go", "name": "both"}));
    let built = wait_for("both windows in the tab's panes", || Some(state()).filter(|s| titled(s).len() == 2));
    assert_eq!(titled(&built), [(5, "Canary 2".to_string()), (6, "Canary 1".to_string())]);
    // Home keeps no tab: the built one is a workspace's.
    assert_eq!(built["tab_panes"], json!([[5, 6]]), "{}", built["state"]["tabs"]);
    assert_eq!(built["hosted"].as_object().map(|h| h.len()), Some(2));
    // The pane marks are of the same windows, under their new ids.
    assert_eq!((&mark(&built, "two")["pane"], &mark(&built, "one")["pane"]), (&json!(5), &json!(6)));
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_hosted_window_is_captured_with_the_part_of_its_tree_under_the_circle() {
    // A folder of the test's own: the captures are kept there, and one rule names a title the
    // canary's window takes later.
    let home = support::temp(&format!("otto-apps-capture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("make the folder");
    let settings = json!({"repos": [], "windows": [{"app": CANARY, "title": "Private*", "do": "ignore"}]});
    std::fs::write(home.join("settings.json"), settings.to_string()).expect("write settings");

    // The canary's window has one native control with a name: a button in its content.
    let env = [("CANARY_BUTTON", "1")];
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-capture", "CANARY_SOCKET", &env);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-capture",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys, "as": "user"}));
    // The Repos section closes: the draft takes the drawer's width, and on a CI runner's small
    // screen the pane needs the room.
    otto.call(json!({"cmd": "section", "section": "repos"}));
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    wait_for("the window to park with its title bar cropped", || {
        state()["crop"]["1"].as_f64().filter(|c| (16.0..=40.0).contains(c))
    });
    wait_for("a frame of its stream", || rect(&state()["shown"]["1"]).filter(|shown| shown[2] > 100.0));

    // From the live app, as the capture key does it: the app is parked so the picture can be
    // drawn on, and the way back is kept.
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    let reply = otto.call(json!({"cmd": "capture_start"}));
    assert_eq!(
        (&reply["mode"], &reply["state"]["live"], &reply["state"]["back_to"]),
        (&json!("annotate"), &Value::Null, &json!(1))
    );
    let source = &reply["state"]["capture"]["materials"][0]["source"];
    assert_eq!(
        (&source["type"], &source["name"], &source["title"]),
        (&json!("app"), &json!(CANARY), &json!("Canary 1"))
    );
    // The bar opens first; the window is then parked at the pane's size beside it, and the
    // picture is a frame of that size. Its tree is read with it, off the main thread.
    wait_or(
        "the picture to be taken",
        || state().to_string(),
        || (state()["state"]["capture"]["materials"][0]["picture"] == true).then_some(()),
    );
    wait_or(
        "the window's tree to be read",
        || state().to_string(),
        || (state()["state"]["capture"]["materials"][0]["content"] == "app").then_some(()),
    );
    let picture = wait_for("the picture to be drawn", || rect(&state()["capture"]["picture"]).filter(|p| p[2] > 100.0));
    let dir = PathBuf::from(state()["capture"]["dir"].as_str().expect("the draft's folder"));
    wait_for("the picture to be written", || dir.join("1.png").is_file().then_some(()));

    // Where the button is, by the canary's own tree: in the window, so below the title bar that
    // the picture crops off. The circle goes round it there.
    let tree = otto.call(json!({"cmd": "ax_tree", "pane": 1}));
    let children = tree["captured"]["tree"]["children"].as_array().cloned().unwrap_or_default();
    let named = children.iter().find(|node| node["title"] == "Canary button").unwrap_or_else(|| panic!("{tree}"));
    let part = |of: &Value, key: &str| of[key].as_f64().expect("a number");
    let crop = state()["crop"]["1"].as_f64().expect("the crop");
    let (bx, by) = (
        part(&named["frame"], "x") - part(&tree["window"], "x"),
        part(&named["frame"], "y") - part(&tree["window"], "y") - crop,
    );
    let (bw, bh) = (part(&named["frame"], "w"), part(&named["frame"], "h"));
    assert!(by > 0.0 && by + bh < picture[3], "the button is in the picture: {named}");
    let (from, to) =
        ([picture[0] + bx - 10.0, picture[1] + by - 10.0], [picture[0] + bx + bw + 10.0, picture[1] + by + bh + 10.0]);
    let reply = otto.call(json!({"cmd": "drag", "from": from, "to": to, "as": "user"}));
    assert_eq!(reply["state"]["capture"]["materials"][0]["marks"]["shapes"][0]["kind"], "ellipse", "{reply}");
    assert_eq!(key("enter")["mode"], "compose");
    key("w h a t space i s space t h i s");
    let reply = key("enter");
    let file = reply["state"]["tasks"]["list"][0]["materials"][0].as_str().expect("its capture").to_string();
    let name = file.strip_suffix("/1.json").filter(|name| name.starts_with("1-")).expect("a folder of its own");
    let kept = home.join("materials").join(name);
    // Written on another thread: the pictures, the brief, and the capture's own file last.
    wait_for("the task's files", || kept.join("1.json").is_file().then_some(()));

    // The brief names the button, as quoted data from the canary, with its window as context.
    let brief = otto.call(json!({"cmd": "task_brief", "id": 1}))["brief"].clone();
    let captured = &brief["materials"][0]["captured"];
    assert_eq!(captured["untrusted"], true);
    // The label is the brief's own words: the app's name is data, inside what it labels.
    assert!(captured["warning"].as_str().is_some_and(|w| w.contains("untrusted") && !w.contains(CANARY)), "{captured}");
    assert_eq!(captured["source"]["name"], CANARY);
    let nodes = captured["under"][0]["nodes"].as_array().cloned().unwrap_or_default();
    let button = nodes.iter().find(|node| node["text"].as_str().is_some_and(|text| text.contains("Canary button")));
    let tree = || otto.call(json!({"cmd": "ax_tree", "pane": 1}));
    let button = button.unwrap_or_else(|| panic!("the button is under the circle: {captured}\nthe tree: {}", tree()));
    assert_eq!(button["role"], "AXButton", "{button}");
    assert!(button["context"].as_array().is_some_and(|c| c.contains(&json!("Canary 1"))), "{button}");
    // Its frame in the picture, 0..1 of it: where the circle was drawn.
    let frame = rect(&button["frame"]).expect("its frame");
    let near = |part: f64, points: f64, of: f64| (part * of - points).abs() < 3.0;
    assert!(near(frame[0], bx, picture[2]) && near(frame[1], by, picture[3]), "{frame:?} of {picture:?}");
    assert!(near(frame[2], bw, picture[2]) && near(frame[3], bh, picture[3]), "{frame:?} of {picture:?}");
    // No traffic light is listed: the title bar is not in the picture.
    assert!(nodes.iter().all(|node| node["frame"][1].as_f64() >= Some(0.0)), "{nodes:?}");

    // Both pictures are kept, in the task's folder, and are pictures: the reply gives their whole paths.
    let path = |key: &str| brief["materials"][0][key].as_str().map(PathBuf::from);
    assert_eq!((path("image"), path("annotated")), (Some(kept.join("1.png")), Some(kept.join("1.annotated.png"))));
    for name in ["1.png", "1.annotated.png"] {
        let png = std::fs::read(kept.join(name)).expect(name);
        assert!(png.starts_with(b"\x89PNG") && png.len() > 1000, "{name}: {} bytes", png.len());
    }
    assert!(std::fs::read_to_string(kept.join("brief.md")).is_ok_and(|text| text.contains("Canary button")));
    // The way back to the app is still offered, and takes it.
    assert_eq!(state()["state"]["back_to"], 1);

    // The window takes a title the rules say to leave alone: it is not captured again. (Back to
    // the pane's space, after Home: the workspace made for the app.)
    key("cmd-ctrl-2");
    canary.call(json!({"cmd": "retitle", "index": 0, "title": "Private notes"}));
    let refused = wait_for("the capture to be refused", || {
        let mut stream = UnixStream::connect(&otto.socket).ok()?;
        writeln!(stream, "{}", json!({"cmd": "capture_start", "pane": 1})).ok()?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).ok()?;
        let reply: Value = serde_json::from_str(&reply).ok()?;
        (reply["ok"] == false).then_some(reply)
    });
    assert!(refused["error"].as_str().is_some_and(|why| why.contains("rules")), "{refused}");
    let after = state();
    assert_eq!(
        (&after["state"]["capture"], std::fs::read_dir(home.join("materials")).map(Iterator::count).ok()),
        (&Value::Null, Some(1))
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_workbook_hosts_its_apps_window_again_beside_its_terminal() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-workbook", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-workbook",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_SHELL", "/bin/sh"),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let user_key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys, "as": "user"}));
    let kinds = |state: &Value| -> Vec<String> {
        let panes = state["state"]["panes"].as_array().cloned().unwrap_or_default();
        panes.iter().filter_map(|pane| pane["kind"]["type"].as_str().map(str::to_string)).collect()
    };

    // The app's window beside a terminal, saved by the user's key under the tab's name. The
    // workspace made for the app alone shows the app on the top row; with the terminal, its name.
    let alone = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(alone["spaces"][1]["glyph"], json!({"kind": "app", "name": CANARY}), "{}", alone["spaces"]);
    let beside = otto.call(json!({"cmd": "split"}));
    assert_eq!(beside["spaces"][1]["glyph"], json!({"kind": "name"}));
    user_key("cmd-shift-s");
    let saved = user_key("enter");
    let book = &saved["workbooks"][0];
    // Kinds and the app; a terminal's kept history (`lines`, `· history`) depends on what its shell printed.
    let saved_panes: Vec<(&Value, &Value)> =
        book["panes"].as_array().unwrap().iter().map(|p| (&p["kind"], &p["app"])).collect();
    assert_eq!(saved_panes, [(&json!("app"), &json!(CANARY)), (&json!("terminal"), &Value::Null)], "{book}");
    let summary = book["summary"].as_str().unwrap_or_default();
    assert!(summary.starts_with(&format!("1 terminal · {CANARY}")) && book["open"] == json!(true), "{book}");

    // Its tab closes: the window goes back to the app. The row opens it again: the same
    // split, the app's window in its pane (no title is kept to find it by) and a new shell.
    let closed = otto.call(json!({"cmd": "close_tab", "index": 0}));
    assert_eq!((kinds(&closed), closed["hosted"].as_object().map(|h| h.len())), (vec![], Some(0)));
    let row = wait_for("the workbook's row", || rect(&state()["workbooks"][0]["row"]));
    for action in ["down", "up"] {
        let [x, y] = middle(row);
        otto.call(json!({"cmd": "mouse", "action": action, "x": x, "y": y, "as": "user"}));
    }
    let built =
        wait_for("the window in the workbook's pane", || Some(state()).filter(|s| kinds(s) == ["app", "terminal"]));
    assert_eq!(built["hosted"].as_object().map(|h| h.len()), Some(1), "{}", built["state"]["panes"]);
    assert_eq!(built["state"]["panes"][0]["kind"]["title"], "Canary 1");
    // The save's note may still show: information now, no longer an error.
    let note = built["notice"].as_str().unwrap_or("Saved ");
    assert!(built["workbooks"][0]["open"] == json!(true) && note.starts_with("Saved "), "{}", built["notice"]);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_restored_layout_hosts_its_running_apps_window_again_beside_its_terminal() {
    let home = support::temp(&format!("otto-apps-layout-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    let home_str = home.to_str().expect("a plain path");
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-layout", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let env = [("OTTOSPACE_HOME", home_str), ("OTTOSPACE_SHELL", "/bin/sh")];
    let kinds = |state: &Value| -> Vec<String> {
        let panes = state["state"]["panes"].as_array().cloned().unwrap_or_default();
        panes.iter().filter_map(|pane| pane["kind"]["type"].as_str().map(str::to_string)).collect()
    };

    // The app's window beside a terminal; the quit gives the window back and keeps the layout.
    let mut otto = otto_for(&canary, "otto-layout-1", &env);
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "quit"}));
    let quit = Instant::now();
    while otto.child.try_wait().expect("wait").is_none() {
        assert!(quit.elapsed() < Duration::from_secs(10), "quit did not exit the app");
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = std::fs::read_to_string(home.join("layout.json")).expect("layout.json written at quit");
    // The app by name, and no window title.
    assert!(text.contains(CANARY) && !text.contains("Canary 1"), "{text}");
    drop(otto);

    // The next start hosts the running app's window in its pane again: started by nobody.
    let otto = otto_for(&canary, "otto-layout-2", &env);
    let state = || otto.call(json!({"cmd": "state"}));
    let built = wait_for("the window in its pane", || {
        Some(state()).filter(|s| kinds(s) == ["app", "terminal"] && s["hosted"].as_object().map(|h| h.len()) == Some(1))
    });
    assert_eq!(built["state"]["panes"][0]["kind"]["title"], "Canary 1", "{}", built["state"]["panes"]);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_restored_pane_whose_app_runs_with_its_window_minimised_says_so_and_the_users_open_hosts_it() {
    let home = support::temp(&format!("otto-apps-minimised-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    let home_str = home.to_str().expect("a plain path");
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-minimised", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let pid = canary.child.id() as i32;
    let env = [("OTTOSPACE_HOME", home_str), ("OTTOSPACE_SHELL", "/bin/sh")];
    let kinds = |state: &Value| -> Vec<String> {
        let panes = state["state"]["panes"].as_array().cloned().unwrap_or_default();
        panes.iter().filter_map(|pane| pane["kind"]["type"].as_str().map(str::to_string)).collect()
    };
    let hosted = |state: &Value| state["hosted"].as_object().map(|h| h.len());
    let quit = |mut otto: Proc| {
        otto.call(json!({"cmd": "quit"}));
        wait_exit(&mut otto, "quit");
    };

    // The app's window beside a terminal, kept at quit; the next start hosts it again.
    let otto = otto_for(&canary, "otto-minimised-1", &env);
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    otto.call(json!({"cmd": "split"}));
    quit(otto);
    let otto = otto_for(&canary, "otto-minimised-2", &env);
    let built = wait_for("the window in its pane", || {
        Some(otto.call(json!({"cmd": "state"}))).filter(|s| kinds(s) == ["app", "terminal"] && hosted(s) == Some(1))
    });
    assert_eq!((&built["unfilled"], &built["notice"]), (&json!([]), &Value::Null));
    quit(otto);

    // Its window goes to the Dock: the app runs, with no window on screen (as Spotify often does).
    canary.call(json!({"cmd": "minimize", "index": 0}));
    wait_for("the window in the Dock", || {
        let windows = ottospace_mac::ax::Window::standard_of(pid);
        (!windows.is_empty() && windows.iter().all(|w| w.minimized())).then_some(())
    });

    // The next start says so in the pane, not "not running", and raises no bar: nobody asked.
    let otto = otto_for(&canary, "otto-minimised-3", &env);
    let state = || otto.call(json!({"cmd": "state"}));
    let why = format!("{CANARY} is running with no window on this Space");
    let waiting = wait_or(
        "the pane to say why",
        || state().to_string(),
        || Some(state()).filter(|s| s["unfilled"][0]["why"].is_string() && rect(&s["unfilled"][0]["open"]).is_some()),
    );
    let said = (&waiting["unfilled"][0]["why"], &waiting["notice"], hosted(&waiting));
    assert_eq!(said, (&json!(why), &Value::Null, Some(0)));

    // A window again, and the user's "Open" hosts it in that pane.
    canary.call(json!({"cmd": "open_window"}));
    wait_for("a window out of the Dock", || {
        ottospace_mac::ax::Window::standard_of(pid).iter().any(|w| !w.minimized()).then_some(())
    });
    let [x, y] = middle(rect(&waiting["unfilled"][0]["open"]).expect("the Open button"));
    for action in ["down", "up"] {
        otto.call(json!({"cmd": "mouse", "action": action, "x": x, "y": y, "as": "user"}));
    }
    let filled = wait_or(
        "the window in the pane",
        || state().to_string(),
        || Some(state()).filter(|s| kinds(s) == ["app", "terminal"] && hosted(s) == Some(1)),
    );
    assert_eq!((&filled["unfilled"], &filled["notice"]), (&json!([]), &Value::Null), "{filled}");
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn the_inspector_reads_a_hosted_canary_window_and_counts_its_roles() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto",
        "OTTOSPACE_SOCKET",
        &[("OTTOSPACE_E2E", "1"), ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string())],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    wait_for("the pane layout", || rect(&state()["bounds"]["1"]).filter(|b| b[2] > 1.0));
    // The user's palette opens it on the focused app pane; it reads the window off the UI thread.
    otto.call(json!({"cmd": "key", "keys": "cmd-shift-p i n s p e c t o r enter", "as": "user"}));
    let read = wait_for("the inspector's read", || {
        Some(state()["inspector"].clone()).filter(|i| i["counts"]["nodes"].as_u64() > Some(0))
    });
    // GPUI exposes no controls: the window and its three traffic lights, all it has.
    let counts = &read["counts"];
    assert!(counts["pressable"].as_u64() >= Some(3), "{read}");
    assert_eq!((&read["pane"], &counts["sparse"]), (&json!(1), &json!(true)), "{read}");
    let roles: Vec<&str> = counts["roles"].as_array().unwrap().iter().filter_map(|r| r[0].as_str()).collect();
    assert!(roles.contains(&"AXWindow") && roles.contains(&"AXButton"), "{roles:?}");
    // `ax_tree` is as it was: what the inspector reads more of is not in its reply.
    let tree = otto.call(json!({"cmd": "ax_tree"})).to_string();
    assert!(!tree.contains("\"focused\"") && !tree.contains("\"settable\"") && !tree.contains("\"identifier\""));
    // It goes with its pane.
    assert!(otto.call(json!({"cmd": "close"}))["inspector"].is_null());
}

/// A tab with a hosted pane folds out to another display: entered there, the canary's window
/// covers its pane in the window out, not where the main window would have drawn it.
#[test]
#[ignore = "hosts real windows; run with `cargo e2e-apps`"]
fn a_hosted_panes_tab_folds_out_and_its_window_goes_live_over_the_window_out() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary", "CANARY_SOCKET", &[]);
    let frame = |index: usize| rect(&canary.call(json!({"cmd": "state"}))["windows"][index]["bounds"]);
    wait_for("the canary window", || frame(0));
    let otto = otto_for(&canary, "otto-out", &[("OTTOSPACE_FEATURES", "multi_monitor")]);
    let state = || otto.call(json!({"cmd": "state"}));
    let main = json!({"uid": "main", "frame": [0, 0, 1728, 1117]});
    let other = json!({"uid": "other", "frame": [1728, 0, 1920, 1080], "scale": 1});
    otto.call(json!({"cmd": "displays_set", "displays": [main, other], "main": "main"}));
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "new_tab"}));
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    let pane = reply["state"]["focused"].as_u64().expect("the canary's pane");
    let tab = reply["state"]["tabs"][1]["id"].as_u64().expect("its tab");
    let folded = otto.call(json!({"cmd": "key", "keys": "cmd-shift-o", "as": "user"}));
    assert_eq!(folded["out"], json!([{"tab": tab, "display": 1}]), "{folded}");
    // The window out draws the pane: its frame and the pane's bounds in it.
    let (out, body) = wait_for("the pane laid out in the window out", || {
        let s = state();
        let out = rect(&s["windows"][1]["frame"]).filter(|_| s["windows"][1]["frames"].as_u64() > Some(0))?;
        Some((out, rect(&s["bounds"][pane.to_string()]).filter(|b| b[2] > 1.0)?))
    });
    let crop = wait_for("the title bar's crop", || state()["crop"][pane.to_string()].as_f64().filter(|c| *c > 0.0));
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": pane}))["mode"], "app");
    let live = wait_for("the window to cover its pane in the window out", || {
        frame(0).filter(|f| close(f[0], out[0] + body[0]) && close(f[1], out[1] + body[1] - crop))
    });
    assert!(close(live[2], body[2]) && close(live[3], body[3] + crop), "live {live:?} vs pane {body:?}");
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
}

/// The canary window's frame as Accessibility has it: global, top-left origin.
fn global_frame(canary: &Proc, index: usize) -> Option<[f64; 4]> {
    rect(&canary.call(json!({"cmd": "state"}))["windows"][index]["frame"])
}

/// True when the user can take hold of the window: its title bar on a screen.
fn reachable(f: [f64; 4]) -> bool {
    let screens = ottospace_mac::recovery::screens();
    ottospace_core::restore::reachable(ottospace_core::ax::Frame::new(f[0], f[1], f[2], f[3]), &screens.visible)
}

/// An ottospace that hosts only the canary's windows. `env`: more of its environment.
fn otto_for(canary: &Proc, name: &str, env: &[(&str, &str)]) -> Proc {
    let pid = canary.child.id().to_string();
    let mut all = vec![("OTTOSPACE_E2E", "1"), ("OTTOSPACE_HOST_ONLY", pid.as_str()), ("OTTOSPACE_RETURN_KEY", "f19")];
    all.extend_from_slice(env);
    Proc::launch(env!("CARGO_BIN_EXE_ottospace"), name, "OTTOSPACE_SOCKET", &all)
}

/// A home of the test's own whose one rule splits every new canary window, however small.
fn splitting_home(name: &str) -> PathBuf {
    let home = support::temp(&format!("otto-apps-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    // Standard windows only: GPUI's own small panels would get panes too.
    let settings = json!({"windows": [{"app": CANARY, "subrole": "AXStandardWindow", "do": "split"}]});
    std::fs::write(home.join("settings.json"), settings.to_string()).expect("write settings");
    home
}

/// The windows offered (`offers`) once there are `n`.
fn wait_offers(otto: &Proc, n: usize) -> Vec<u64> {
    wait_for(&format!("{n} offers"), || {
        let offers = otto.call(json!({"cmd": "state"}))["state"]["offers"].as_array().cloned().unwrap_or_default();
        (offers.len() == n).then(|| offers.iter().filter_map(|o| o["window"].as_u64()).collect())
    })
}

/// The user's click on ◫ (split right) for `window`, found by its sneak label. Several offers
/// group in one chip: its ▾ opens their list first.
fn take_offer(otto: &Proc, window: u64) {
    let targets = || {
        let reply = otto.call(json!({"cmd": "sneak"}));
        otto.call(json!({"cmd": "key", "keys": "escape"}));
        reply["sneak"]["targets"].as_array().cloned().unwrap_or_default()
    };
    let click = |target: &Value| {
        let [x, y] = middle(["x", "y", "w", "h"].map(|key| target["frame"][key].as_f64().unwrap_or_default()));
        for action in ["down", "up"] {
            otto.call(json!({"cmd": "mouse", "action": action, "x": x, "y": y, "as": "user"}));
        }
    };
    let id = format!("offer:{window}");
    let named = |targets: &[Value], id: &str| targets.iter().find(|t| t["source"]["id"] == id).cloned();
    let mut found = targets();
    if named(&found, &id).is_none()
        && let Some(list) =
            found.iter().find(|t| t["source"]["id"].as_str().is_some_and(|s| s.starts_with("offer-list:")))
    {
        click(list);
        found = targets();
    }
    click(&named(&found, &id).unwrap_or_else(|| panic!("no {id}: {found:?}")));
}

/// The user's click on the control named `id`, found by its sneak label.
fn user_click_control(otto: &Proc, id: &str) {
    let reply = otto.call(json!({"cmd": "sneak"}));
    otto.call(json!({"cmd": "key", "keys": "escape"}));
    let targets = reply["sneak"]["targets"].as_array().cloned().unwrap_or_default();
    let target = targets.iter().find(|t| t["source"]["id"] == id).unwrap_or_else(|| panic!("no {id}: {targets:?}"));
    let [x, y] = middle(["x", "y", "w", "h"].map(|key| target["frame"][key].as_f64().unwrap_or_default()));
    for action in ["down", "up"] {
        otto.call(json!({"cmd": "mouse", "action": action, "x": x, "y": y, "as": "user"}));
    }
}

/// Hosts the canary's first window and waits for it to park. Returns the parking spot.
fn host_and_park(otto: &Proc) -> [f64; 2] {
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "app", "{reply}");
    let spot = point(&otto.call(json!({"cmd": "state"}))["parking"]).expect("parking spot");
    wait_for("the window to park", || {
        rect(&otto.call(json!({"cmd": "state"}))["hosted"]["1"]).filter(|f| close(f[0], spot[0]))
    });
    spot
}

/// Waits for the process to end; panics after `TIMEOUT`.
fn wait_exit(proc: &mut Proc, what: &str) {
    let start = Instant::now();
    while proc.child.try_wait().expect("wait").is_none() {
        assert!(start.elapsed() < TIMEOUT, "{what}: ottospace did not end");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn signal(proc: &Proc, name: &str) {
    let status = Command::new("kill").args([format!("-{name}"), proc.child.id().to_string()]).status().expect("kill");
    assert!(status.success(), "kill -{name}");
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn every_way_out_gives_the_window_back_where_it_was() {
    for way in ["ctl quit", "cmd-q", "close button", "TERM", "INT", "HUP"] {
        let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-out", "CANARY_SOCKET", &[]);
        wait_for("the canary window", || global_frame(&canary, 0));
        // Near the main screen's bottom-right corner: put back size first, from the parking
        // spot, macOS would clamp the window to what fits there.
        let main = ottospace_mac::display::all()[0];
        let to = [main.origin.x + main.size.width - 700.0, main.origin.y + main.size.height - 520.0];
        canary.call(json!({"cmd": "move", "index": 0, "to": to}));
        let original = wait_for("the canary to move", || global_frame(&canary, 0).filter(|f| close(f[0], to[0])));
        let mut otto = otto_for(&canary, "otto-out", &[]);
        host_and_park(&otto);
        match way {
            "ctl quit" => drop(otto.call(json!({"cmd": "quit"}))),
            "cmd-q" => drop(otto.call(json!({"cmd": "key", "keys": "cmd-q"}))),
            "close button" => {
                let pid = otto.child.id();
                press(&format!(
                    "tell (first process whose unix id is {pid}) to click (first button of window 1 whose subrole is \"AXCloseButton\")"
                ));
            }
            name => signal(&otto, name),
        }
        wait_exit(&mut otto, way);
        let back = global_frame(&canary, 0).expect("canary frame");
        let within = (0..4).all(|i| (back[i] - original[i]).abs() <= 1.0);
        assert!(within, "{way}: the window is at {back:?}, it was at {original:?}");
    }
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_killed_ottospace_has_its_guardian_give_the_window_back() {
    let home = support::temp(&format!("otto-apps-guard-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-guard", "CANARY_SOCKET", &[]);
    let original = wait_for("the canary window", || global_frame(&canary, 0));
    let home_str = home.to_str().expect("a plain path");
    let mut otto = otto_for(&canary, "otto-guard", &[("OTTOSPACE_HOME", home_str), ("OTTOSPACE_GUARD", "1")]);
    host_and_park(&otto);
    let record = || std::fs::read_to_string(home.join("hosted.json")).unwrap_or_default();
    let pid = otto.child.id();
    assert!(record().contains(&format!("\"owner\":{pid}")), "the record: {}", record());
    let guardian = || {
        let out = Command::new("pgrep").args(["-f", &format!("ottospace guard {pid}$")]).output().expect("pgrep");
        !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    };
    assert!(guardian(), "a guardian watches the run");

    // No quit runs: SIGKILL.
    otto.child.kill().expect("kill -9");
    let _ = otto.child.wait();
    let start = Instant::now();
    let back = wait_for("the guardian to put the window back", || {
        global_frame(&canary, 0).filter(|f| (0..4).all(|i| (f[i] - original[i]).abs() <= 1.0))
    });
    assert!(start.elapsed() < Duration::from_secs(10), "back after {:?}: {back:?}", start.elapsed());
    wait_for("the record to be cleared", || (record() == "[]").then_some(()));
    wait_for("the guardian to end", || (!guardian()).then_some(()));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_window_found_off_screen_comes_back_on_screen_and_rescue_brings_one_back() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-off", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-off", &[]);
    let spot = point(&otto.call(json!({"cmd": "state"}))["parking"]).expect("parking spot");
    // Where a crashed run would have left it: parked.
    let parked = [spot[0], spot[1] - 200.0];
    canary.call(json!({"cmd": "move", "index": 0, "to": parked}));
    wait_for("the canary to be off screen", || global_frame(&canary, 0).filter(|f| !reachable(*f)));
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    let notice = reply["notice"].as_str().unwrap_or_default().to_string();
    assert!(notice.contains("off screen"), "the status bar says so: {reply}");
    otto.call(json!({"cmd": "quit"}));
    wait_for("the window on screen after the quit", || global_frame(&canary, 0).filter(|f| reachable(*f)));
    drop(otto);

    // `ottospace rescue`, no app running: a window parked by nobody comes back.
    canary.call(json!({"cmd": "move", "index": 0, "to": parked}));
    wait_for("the canary to be off screen", || global_frame(&canary, 0).filter(|f| !reachable(*f)));
    let out = Command::new(env!("CARGO_BIN_EXE_ottospace"))
        .arg("rescue")
        .env("OTTOSPACE_E2E", "1")
        .env("OTTOSPACE_HOST_ONLY", canary.child.id().to_string())
        .env_remove("OTTOSPACE_HOME")
        .output()
        .expect("run rescue");
    let said = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && said.contains("moved 1 window back"), "{said} {err}");
    let back = global_frame(&canary, 0).expect("canary frame");
    assert!(reachable(back), "rescued to {back:?}");
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_window_the_app_remade_while_hosted_is_not_stranded() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-remade", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-remade", &[]);
    let spot = host_and_park(&otto);
    // The app closes its window and opens a new one where the old one stood: parked, offered.
    canary.call(json!({"cmd": "recreate", "index": 0, "at": [spot[0], spot[1] - 200.0]}));
    let titles = || -> Vec<String> {
        let panes = otto.call(json!({"cmd": "state"}))["state"]["panes"].as_array().cloned().unwrap_or_default();
        panes.iter().filter_map(|p| p["kind"]["title"].as_str().map(str::to_string)).collect()
    };
    // Its pane waits, empty, offering the new window: the user's Bring it here puts it there.
    let offered = wait_offers(&otto, 1);
    wait_for("the pane to wait for it", || {
        (otto.call(json!({"cmd": "state"}))["state"]["panes"][0]["kind"]["type"] == "empty").then_some(())
    });
    take_offer(&otto, offered[0]);
    wait_for("the new window's pane alone", || (titles() == ["Canary 2"]).then_some(()));
    // It floated onto the screen over its pane before it was offered: what is recorded is on screen.
    otto.call(json!({"cmd": "quit"}));
    wait_for("the new window on screen", || {
        let windows = canary.call(json!({"cmd": "state"}))["windows"].clone();
        let frame = rect(&windows.as_array()?.iter().find(|w| w["title"] == "Canary 2")?["frame"])?;
        reachable(frame).then_some(frame)
    });
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_busy_app_keeps_its_pane_and_gets_its_window_back() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-busy", "CANARY_SOCKET", &[]);
    let original = wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-busy", &[]);
    host_and_park(&otto);
    // Longer than Accessibility's 6s wait: a frame read times out while the app is busy.
    canary.call(json!({"cmd": "hang", "ms": 9000}));
    std::thread::sleep(Duration::from_secs(11));
    let state = otto.call(json!({"cmd": "state"}));
    assert_eq!(state["state"]["panes"][0]["kind"]["type"], "app", "the pane stays: {}", state["state"]["panes"]);
    otto.call(json!({"cmd": "quit"}));
    wait_for("the window back where it was", || {
        global_frame(&canary, 0).filter(|f| (0..4).all(|i| (f[i] - original[i]).abs() <= 1.0))
    });
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_sneak_label_presses_a_control_in_the_hosted_app_and_a_disabled_one_has_none() {
    // The canary's window has a button, a checkbox whose state it reports, and a disabled one.
    let env = [("CANARY_BUTTON", "1")];
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-sneak", "CANARY_SOCKET", &env);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    assert_eq!(canary.call(json!({"cmd": "state"}))["pressed"], false);
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-sneak",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let user_key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys, "as": "user"}));
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    wait_for("the window to park with its title bar cropped", || {
        state()["crop"]["1"].as_f64().filter(|c| (16.0..=40.0).contains(c))
    });
    wait_for("a frame of its stream", || rect(&state()["shown"]["1"]).filter(|shown| shown[2] > 100.0));

    // The tree, to tell the controls apart by their paths: the labels carry no text.
    let tree = otto.call(json!({"cmd": "ax_tree", "pane": 1}));
    let children = tree["captured"]["tree"]["children"].as_array().cloned().unwrap_or_default();
    let path_of = |title: &str| {
        let at = children.iter().position(|node| node["title"] == title);
        json!([at.unwrap_or_else(|| panic!("no {title} in {tree}"))])
    };
    let (press, disabled, button) = (path_of("Canary press"), path_of("Canary disabled"), path_of("Canary button"));

    // The user's sneak key: ottospace's own labels at once, the app's when its tree is read.
    assert_eq!(user_key("cmd-ctrl-g")["mode"], "sneak");
    let labelled = wait_or(
        "the app's labels",
        || state().to_string(),
        || Some(state()).filter(|s| s["sneak"]["app"]["controls"].as_u64().is_some_and(|n| n >= 2)),
    );
    let targets = labelled["sneak"]["targets"].as_array().cloned().unwrap_or_default();
    let label_of = |path: &Value| {
        let found = targets.iter().find(|t| t["source"]["type"] == "app" && &t["source"]["path"] == path);
        found.and_then(|t| t["label"].as_str().map(str::to_string))
    };
    assert!(label_of(&button).is_some() && label_of(&disabled).is_none(), "{targets:?}");
    let label = label_of(&press).unwrap_or_else(|| panic!("no label on the checkbox: {targets:?}"));
    // The label lies over the pane, where the checkbox shows in its stream.
    let shown = rect(&labelled["shown"]["1"]).expect("the stream");
    let on = targets.iter().find(|t| t["label"] == label.as_str()).expect("its target")["frame"].clone();
    let (x, y) = (on["x"].as_f64().unwrap_or(-1.0), on["y"].as_f64().unwrap_or(-1.0));
    assert!(x >= shown[0] && y >= shown[1] && x < shown[0] + shown[2] && y < shown[1] + shown[3], "{on} in {shown:?}");

    // Typing it presses the checkbox through accessibility: the canary reports it, the pane
    // stays parked and ottospace keeps the keyboard.
    // Two letters: the first only narrows, and presses nothing yet.
    assert!(label.len() >= 2, "{label}");
    let first = user_key(&label[..1]);
    assert_eq!((&first["mode"], &first["sneak"]["typed"]), (&json!("sneak"), &json!(&label[..1])), "{first}");
    assert_eq!((&first["app_presses"], &canary.call(json!({"cmd": "state"}))["pressed"]), (&json!([]), &json!(false)));
    let reply = user_key(&label[1..].chars().map(String::from).collect::<Vec<_>>().join(" "));
    assert_eq!((&reply["mode"], &reply["state"]["live"]), (&json!("normal"), &Value::Null), "{reply}");
    wait_or(
        "the canary to report the press",
        || format!("{} / {}", canary.call(json!({"cmd": "state"})), state()["notice"]),
        || (canary.call(json!({"cmd": "state"}))["pressed"] == true).then_some(()),
    );
    assert_eq!(state()["app_presses"], json!([[1, press]]));
}

/// Real (HID-level) input at a global point, as the user's mouse sends it: the pointer moves
/// there, and with `lines` the wheel turns. A scroll goes to the window under the pointer.
fn hid(x: f64, y: f64, lines: Option<i32>) {
    use objc2_core_graphics::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, CGScrollEventUnit};
    let at = objc2_core_foundation::CGPoint::new(x, y);
    let moved = CGEvent::new_mouse_event(None, CGEventType::MouseMoved, at, CGMouseButton::Left);
    CGEvent::post(CGEventTapLocation::HIDEventTap, moved.as_deref());
    if let Some(lines) = lines {
        let scroll = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 1, lines, 0, 0);
        CGEvent::set_location(scroll.as_deref(), at);
        CGEvent::post(CGEventTapLocation::HIDEventTap, scroll.as_deref());
    }
}

/// The user's wheel at `x, y` over a parked app pane, once what an entry needs holds: ottospace's
/// window has the keyboard (the front app's may not yet) and the left button is up (else a drag).
fn user_wheel(otto: &Proc, x: f64, y: f64) {
    let state = || otto.call(json!({"cmd": "state"}));
    let held = || {
        format!(
            "window_active {}, button down {}, {}",
            state()["window_active"],
            ottospace_mac::input::left_button_down(),
            in_front()
        )
    };
    wait_or("ottospace's window to be active", held, || {
        let active = state()["window_active"] == true;
        if !active {
            ask_forward(otto.child.id());
        }
        active.then_some(())
    });
    assert!(!ottospace_mac::input::left_button_down(), "the system reads the left button as down");
    hid(x, y, None);
    std::thread::sleep(Duration::from_millis(300));
    hid(x, y, Some(-3));
}

/// What a test saw and does not assert yet: on stderr, and in the CI job's summary, which a
/// passing test's captured output never reaches.
fn report(line: &str) {
    eprintln!("{line}");
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY")
        && let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path)
    {
        let _ = writeln!(file, "\n`{}`", line.replace('`', "'"));
    }
}

/// A global point of pane 1's picture on the main screen, once its stream shows.
fn on_picture(otto: &Proc) -> [f64; 2] {
    let state = || otto.call(json!({"cmd": "state"}));
    let shown =
        wait_for("a frame of its stream", || rect(&state()["shown"]["1"]).filter(|s| s[2] > 100.0 && s[3] > 200.0));
    let window = rect(&state()["window"]).expect("ottospace window frame");
    // Clear of the menu bar and the Dock.
    let main = ottospace_mac::display::all()[0];
    let left = (window[0] + shown[0]).max(main.origin.x);
    let top = (window[1] + shown[1]).max(main.origin.y + 40.0);
    let right = (window[0] + shown[0] + shown[2]).min(main.origin.x + main.size.width);
    let bottom = (window[1] + shown[1] + shown[3]).min(main.origin.y + main.size.height - 80.0);
    assert!(left + 40.0 < right && top + 40.0 < bottom, "no part of the picture on screen: {shown:?} in {window:?}");
    [(left + right) / 2.0, (top + bottom) / 2.0]
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn the_users_wheel_enters_a_parked_app_pane_and_its_first_scroll_reaches_the_app() {
    // The canary's window counts the scrolls it receives, and adds them up.
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-wheel", "CANARY_SOCKET", &[]);
    let canary_state = || canary.call(json!({"cmd": "state"}))["windows"][0].clone();
    let received = || canary_state()["wheels"].as_u64().unwrap_or(0);
    let scrolled = || canary_state()["wheel"].as_f64().unwrap_or(0.0);
    let original = wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-wheel", &[]);
    let state = || otto.call(json!({"cmd": "state"}));
    let spot = host_and_park(&otto);
    let parked = || rect(&state()["hosted"]["1"]).filter(|f| close(f[0], spot[0]));
    let [x, y] = on_picture(&otto);
    let window = rect(&state()["window"]).expect("ottospace window frame");
    let counts = || state()["counts"].clone();

    // A client's wheel enters nothing, and says why.
    let request = json!({"cmd": "mouse", "action": "wheel", "x": x - window[0], "y": y - window[1], "dy": -3});
    otto.call(request);
    let told = (0..30).any(|_| {
        std::thread::sleep(Duration::from_millis(100));
        state()["notice"].as_str().is_some_and(|n| n.contains("user's alone"))
    });
    let notice = format!("{} ({})", state()["notice"], counts());
    assert_eq!((counts()["wheel_entries"].as_u64(), received()), (Some(0), 0), "a client's wheel entered the app");
    assert_eq!(state()["mode"], "normal");
    assert!(parked().is_some(), "the window left its parking spot");

    // The user's wheel enters the pane, as a click does: APP mode, the window over its pane.
    user_wheel(&otto, x, y);
    let wheels = || {
        let trace = otto.call(json!({"cmd": "trace"}))["trace"].as_str().unwrap_or_default().to_string();
        let lines: Vec<&str> = trace.lines().filter(|l| l.contains("wheel")).collect();
        format!("{} {}; canary {}; {}", counts(), lines.join(" "), canary_state(), in_front())
    };
    wait_or("the entry", wheels, || (state()["mode"] == "app").then_some(()));
    assert_eq!((state()["state"]["live"].clone(), state()["state"]["focused"].clone()), (json!(1), json!(1)));
    let up = wait_for("the window over its pane", || rect(&state()["hosted"]["1"]).filter(|f| !close(f[0], spot[0])));
    assert!(up[0] <= x && x < up[0] + up[2] && up[1] <= y && y < up[1] + up[3], "{up:?} is not under the pointer");
    wait_or("the canary in front", in_front, || (front_app().0 == canary.child.id()).then_some(()));

    // The scroll that entered reaches the app, with no other sent: down, as the user's was.
    wait_or("the first scroll in the app", wheels, || (received() > 0).then_some(()));
    report(&format!("wheel: the first scroll reached the app: {}", wheels()));
    assert!(scrolled() < 0.0, "the scroll reached the app the wrong way: {}", wheels());
    assert_eq!(counts()["wheel_entries"], 1, "{}", counts());

    // Leaving is as ever: the leave key parks the window.
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    wait_for("the window parked again", parked);
    otto.call(json!({"cmd": "quit"}));
    wait_for("the window put back at quit", || {
        global_frame(&canary, 0).filter(|f| (0..4).all(|i| close(f[i], original[i])))
    });
    report(&format!("wheel: the client's wheel told why {told} ({notice})"));
}

/// A notice moves no hosted window. Over a live app's window the toast floats above it, a panel
/// a card, at its place in ottospace's window: in view, and the user's click on a card acts. The
/// app keeps the keyboard and gets no click, but one between two cards. A long card pointed at
/// shows its whole text, its panel as tall.
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_notice_moves_no_hosted_window_and_floats_over_a_live_one() {
    const RIGHT: &str = r#"{"repos": [], "restore_layout": false}"#;
    let home = support::temp(&format!("otto-apps-notice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("make the folder");
    std::fs::write(home.join("settings.json"), RIGHT).expect("write settings");
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-notice", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = otto_for(&canary, "otto-notice", &[("OTTOSPACE_HOME", home.to_str().expect("a plain path"))]);
    let state = || otto.call(json!({"cmd": "state"}));
    host_and_park(&otto);
    let crop = || state()["crop"]["1"].as_f64().filter(|c| (16.0..=40.0).contains(c));
    let cropped = wait_for("the title bar's crop", crop);
    let pane = || rect(&state()["bounds"]["1"]).filter(|b| b[2] > 1.0);
    let hosted = || rect(&state()["hosted"]["1"]);
    // Settled over its pane: two reads the same, and as tall as the pane and its title bar.
    let settled = || {
        let (a, b) = (hosted()?, hosted()?);
        (a == b && close(a[3], pane()?[3] + cropped)).then_some(a)
    };
    let enter = || {
        assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
        wait_for("the window over its pane", settled)
    };
    let live = enter();
    let body = wait_for("the pane layout", pane);
    let clicks = || canary.call(json!({"cmd": "state"}))["windows"][0]["clicks"].as_array().map(Vec::len);
    let clicked = clicks();

    // An error: a card in the panel over the live window, at the window's bottom right. The pane and
    // the window stay as they were.
    std::fs::write(home.join("settings.json"), "{ not json").expect("write settings");
    let floats =
        |s: &Value| (s["toast"]["panel"].is_array() && s["toast"]["cards"][0]["close"].is_array()).then(|| s.clone());
    let shown = wait_for("the toast's panel", || floats(&state()));
    let panel = rect(&shown["toast"]["panel"]).expect("the panel");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((pane(), settled()), (Some(body), Some(live)), "nothing moved");
    let inside = panel[0] >= live[0]
        && panel[0] + panel[2] <= live[0] + live[2]
        && panel[1] >= live[1]
        && panel[1] + panel[3] <= live[1] + live[3];
    assert!(inside, "the panel {panel:?} over the live window {live:?}");
    assert_eq!(crop(), Some(cropped), "the title bar is still cropped");
    assert_eq!(state()["mode"], "app", "the app keeps the keyboard");

    // Real clicks, over the app's window. Needs `cliclick`.
    if Command::new("cliclick").arg("-V").output().is_err() {
        eprintln!("cliclick not found: clicks on the toast's panel not checked");
        return;
    }
    // A rect of the toast's (in ottospace's window) on the screens: the panel's top left is the stack's.
    let global = |s: &Value, r: [f64; 4]| {
        let (panel, stack) =
            (rect(&s["toast"]["panel"]).expect("the panel"), rect(&s["toast"]["rect"]).expect("the stack"));
        (panel[0] + r[0] - stack[0] + r[2] / 2., panel[1] + r[1] - stack[1] + r[3] / 2.)
    };
    let card = &shown["toast"]["cards"][0];
    // A click on the card's words does nothing, and the keys still go to the app. The pointer glides
    // there first: a jump into a panel is heard as an entry, not a move, and points at nothing.
    let (x, y) = global(&shown, rect(&card["rect"]).expect("the card"));
    glide(x - 60., y);
    click_at(x - 60., y, false);
    let keys = || canary.call(json!({"cmd": "state"}))["windows"][0]["keys"].clone();
    press("keystroke \"q\"");
    wait_or("the key in the app", || keys().to_string(), || keys().as_array()?.iter().any(|k| k == "q").then_some(()));
    // Its ×, where it is with the pointer over the card: the card goes, the app gets no click and stays in front.
    // Read once the card pointed at has grown and its panel followed: two reads the same.
    let cross = || {
        let s = state();
        Some(global(&s, rect(&s["toast"]["cards"][0]["close"])?))
    };
    let (x, y) = wait_for("its × settled", || {
        let a = cross()?;
        std::thread::sleep(Duration::from_millis(200));
        (cross()? == a).then_some(a)
    });
    click_at(x, y, false);
    wait_for("the toast to go", || Some(state()).filter(|s| s["toast"].is_null()));
    assert_eq!((clicks(), state()["mode"].clone()), (clicked, json!("app")), "the app got no click, kept the keys");
    assert_front(canary.child.id(), "the canary");

    // Two cards (a folder that is not there, its path long; a settings file that does not read),
    // a panel each: a click between them is the app's.
    let long = format!("/made-up/{}", "a-long-folder-name/".repeat(10));
    otto.call(json!({"cmd": "repo_add", "path": long}));
    std::fs::write(home.join("settings.json"), "[ not json").expect("write settings");
    let two = |s: &Value| {
        let (first, second) = (rect(&s["toast"]["cards"][0]["rect"])?, rect(&s["toast"]["cards"][1]["rect"])?);
        rect(&s["toast"]["panel"])?;
        Some((s.clone(), first, second))
    };
    // The second card over the first.
    let (s, first, second) = wait_for("two cards in their panels", || two(&state()));
    assert!(first[1] - (second[1] + second[3]) >= 4., "a gap between {second:?} and {first:?}");
    let (x, y) = global(&s, [first[0], second[1] + second[3], first[2], first[1] - second[1] - second[3]]);
    click_at(x, y, false);
    wait_or(
        "the click between the cards in the app",
        || format!("{:?}", clicks()),
        || (clicks() == clicked.map(|n| n + 1)).then_some(()),
    );
    assert_eq!(state()["toast"]["cards"].as_array().map(Vec::len), Some(2), "both cards stay");
    report("toast: a click between two cards reached the app");

    // The pointer over the long card: its whole text, upward, the card and the panels' frame taller by as much.
    let long_card = |s: &Value| {
        let cards = s["toast"]["cards"].as_array()?;
        let at = cards.iter().position(|card| card["kind"] == "failed")?;
        Some((rect(&cards[at]["rect"])?, rect(&s["toast"]["rect"])?, rect(&s["toast"]["panel"])?))
    };
    let (card, stack, panel) = long_card(&state()).expect("the long card");
    let (x, y) = global(&s, card);
    glide(x - 60., y);
    // Taller, and its panel moved to fit by the next tick.
    let grown = |s: &Value| {
        long_card(s).filter(|(tall, now, _)| tall[3] > card[3] + 10. && close(now[3], stack[3] + tall[3] - card[3]))
    };
    let (tall, stack_now, panel_now) =
        wait_or("the long card taller", || format!("{:?}", long_card(&state())), || grown(&state()));
    let more = tall[3] - card[3];
    let whole = tall[1] >= stack_now[1] && tall[1] + tall[3] <= stack_now[1] + stack_now[3] + 0.5;
    assert!(whole, "the card {tall:?} is cut by its stack {stack_now:?}");
    assert!(close(tall[1] + tall[3], card[1] + card[3]), "the card {tall:?} grew down from {card:?}");
    assert!(close(panel_now[3], panel[3] + more), "the panels {panel_now:?}, were {panel:?}");
    assert_eq!((pane(), state()["mode"].clone()), (Some(body), json!("app")), "nothing moved, the app has the keys");
    report(&format!("toast: a long card pointed at grew by {more} pt in its panel"));
    // The pointer away: one line again.
    glide(panel[0] - 100., y);
    wait_or(
        "the long card at one line",
        || state()["toast"].to_string(),
        || long_card(&state()).filter(|(now, ..)| close(now[3], card[3])),
    );
    let clicked = clicked.map(|n| n + 1);

    // A workspace closed, then back in the app: its Undo floats too, and the user's click on it
    // takes the workspace back.
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    let sneak = otto.call(json!({"cmd": "sneak"}));
    otto.call(json!({"cmd": "key", "keys": "escape"}));
    let targets = sneak["sneak"]["targets"].as_array().cloned().unwrap_or_default();
    let plus = targets.iter().find(|t| t["source"]["id"] == "top:new_workspace").expect("the top row's +");
    let (px, py) = (plus["frame"]["x"].as_f64().unwrap() + 4., plus["frame"]["y"].as_f64().unwrap() + 4.);
    for action in ["down", "up"] {
        otto.call(json!({"cmd": "mouse", "action": action, "x": px, "y": py, "as": "user"}));
    }
    // The spaces on the top row: a closed one is off it while its Undo stands.
    let spaces =
        |s: &Value| s["spaces"].as_array().map(|all| all.iter().filter(|space| space["hidden"] != true).count());
    let open = spaces(&state());
    let closed = otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-w", "as": "user"}));
    assert_eq!(closed["workbook_offer"], "closed", "{closed}");
    let fewer = spaces(&state());
    assert!(fewer < open, "closed: {fewer:?} of {open:?}");
    enter();
    let undo = |s: &Value| {
        let card = s["toast"]["cards"].as_array()?.iter().find(|c| c["offer"] == "undo")?.clone();
        rect(&s["toast"]["panel"])?;
        Some((s.clone(), rect(&card["offer_rect"])?))
    };
    let (s, offer) = wait_for("the Undo in the panel", || undo(&state()));
    let (x, y) = global(&s, offer);
    click_at(x, y, false);
    // Back: as many workspaces as before it closed (an Undo that did nothing leaves one fewer).
    wait_for("the workspace back", || Some(state()).filter(|s| s["workbook_offer"].is_null() && spaces(s) == open));
    assert_eq!(clicks(), clicked, "the app got no click");
    let _ = std::fs::remove_dir_all(&home);
}

/// Made-up words the canary says, as `say` makes them: no person's voice, no one's meeting.
const CANARY_WORDS: &str = "The orange canary sings a quiet song about purple tables. \
    The orange canary sings again. The orange canary is done.";

/// The canary's own sound, heard and transcribed on this Mac: `say` into a file, played by the
/// canary (in its process, or a child's: `child`), its pane transcribed by the user's ⌘⌃T, and a
/// line of it in the transcript's Markdown file. None: why it was skipped (no macOS 26, or the
/// speech model not installed: a test run downloads none).
fn canary_transcribed(name: &str, child: bool) -> Option<String> {
    // Below macOS 26 no recogniser answers: its notice and one not yet asked read the same.
    let os = Command::new("/usr/bin/sw_vers").arg("-productVersion").output().expect("sw_vers");
    let major: u32 =
        String::from_utf8_lossy(&os.stdout).split('.').next().and_then(|m| m.trim().parse().ok()).unwrap_or(0);
    if major < 26 {
        return Some(format!("macOS {major}: transcripts need macOS 26"));
    }
    let home = support::temp(&format!("otto-apps-listen-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    let dir = home.join("transcripts");
    let settings = json!({"restore_layout": false, "transcripts_dir": dir});
    std::fs::write(home.join("settings.json"), settings.to_string()).expect("write settings");
    let words = home.join("words.aiff");
    let said = Command::new("/usr/bin/say").arg("-o").arg(&words).arg(CANARY_WORDS).status().expect("say");
    assert!(said.success(), "say -o");

    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), &format!("canary-{name}"), "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        &format!("otto-{name}"),
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
            ("OTTOSPACE_FEATURES", "transcripts"),
        ],
    );
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "app", "{reply}");
    // The recogniser is asked for at start: the app pane offers a start once it answers.
    let started = wait_or("the transcript to start, or say why not", String::new, || {
        let reply = otto.call(json!({"cmd": "state"}));
        if reply["state"]["listen"]["status"] == "on" {
            return Some(Ok(()));
        }
        if let Some(why) = reply["notice"].as_str().filter(|why| why.contains("Transcript")) {
            return Some(Err(why.to_string()));
        }
        if reply["state"]["listen"].is_null() {
            otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-t", "as": "user"}));
        }
        None
    });
    if let Err(why) = started {
        if why.contains("macOS 26") || why.contains("not installed") || why.contains("cannot run") {
            let _ = std::fs::remove_dir_all(&home);
            return Some(why);
        }
        panic!("{why}");
    }
    canary.call(json!({"cmd": "play", "path": words, "child": child}));
    let file = || std::fs::read_dir(&dir).ok()?.flatten().map(|entry| entry.path()).next();
    let heard =
        |text: &str| text.lines().any(|line| line.contains("**them**") && line.to_lowercase().contains("canary"));
    let start = Instant::now();
    let text = loop {
        let text = file().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        if heard(&text) {
            break text;
        }
        assert!(start.elapsed() < Duration::from_secs(90), "no line of the canary's words in 90 s:\n{text}");
        std::thread::sleep(Duration::from_millis(250));
    };
    eprintln!("the canary's transcript:\n{text}");
    let stopped = otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-t", "as": "user"}));
    assert_eq!(stopped["state"]["listen"], Value::Null, "{stopped}");
    let _ = std::fs::remove_dir_all(&home);
    None
}

#[test]
#[ignore = "hosts the canary and hears its own made-up speech; run with `cargo e2e-apps`"]
fn the_canarys_own_speech_is_transcribed_into_its_file() {
    if let Some(why) = canary_transcribed("speech", false) {
        eprintln!("skipped: {why}");
    }
}

/// Chrome and Electron apps play sound from helper processes, which ScreenCaptureKit does not
/// offer as apps of their own: this says whether the app's filter hears its child's sound.
#[test]
#[ignore = "hosts the canary and hears its own made-up speech; run with `cargo e2e-apps`"]
fn a_sound_the_canarys_child_plays_is_heard_as_the_canarys() {
    if let Some(why) = canary_transcribed("child", true) {
        eprintln!("skipped: {why}");
    }
}

/// A 16-bit mono 44.1 kHz WAV of `parts`: (seconds, tone or silence), each tone 440 Hz. Made by
/// the test, not `say`: CI's `say -o` often wrote seconds too few.
fn wav(file: &std::path::Path, parts: &[(f64, bool)]) {
    const RATE: u32 = 44_100;
    let mut samples: Vec<i16> = Vec::new();
    for &(secs, tone) in parts {
        let n = (secs * f64::from(RATE)) as usize;
        let at = |i: usize| (i as f64 * 440. * std::f64::consts::TAU / f64::from(RATE)).sin() * 8_000.;
        samples.extend((0..n).map(|i| if tone { at(i) as i16 } else { 0 }));
    }
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    // fmt: length 16, PCM, one channel, rate, bytes a second, bytes a frame, bits a sample.
    for (value, bytes) in [(16, 4), (1, 2), (1, 2), (RATE, 4), (RATE * 2, 4), (2, 2), (16, 2)] {
        out.extend(&u32::to_le_bytes(value)[..bytes]);
    }
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    std::fs::write(file, out).expect("write the sound");
}

/// An app's helpers (Teams's `com.microsoft.teams2.helper`): the stream of an app names each
/// child of its own bundle id, or one under it, that ScreenCaptureKit offers as an app (one with
/// a window), and hears it. The canary runs from a bundle the test makes, so it and its child
/// have a bundle id; the child has a window never shown. The log says how many pids were heard.
#[test]
#[ignore = "hears the canary's child's tone; run with `cargo e2e-apps`"]
fn a_tone_the_canarys_app_child_plays_is_heard() {
    let home = support::temp(&format!("otto-apps-child-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    std::fs::write(home.join("settings.json"), json!({"restore_layout": false}).to_string()).expect("settings");
    let tone = home.join("tone.wav");
    wav(&tone, &[(20., true)]);
    let contents = home.join("Canary.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).expect("make the bundle");
    let exe = contents.join("MacOS/ottospace-canary");
    std::fs::copy(env!("CARGO_BIN_EXE_ottospace-canary"), &exe).expect("copy the canary");
    let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
        <key>CFBundleIdentifier</key><string>com.ottospace.canary.test</string>\
        <key>CFBundleExecutable</key><string>ottospace-canary</string>\
        <key>CFBundlePackageType</key><string>APPL</string></dict></plist>";
    std::fs::write(contents.join("Info.plist"), plist).expect("write the bundle's plist");

    let canary = Proc::launch(exe.to_str().expect("a plain path"), "canary-child", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-child",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
        ],
    );
    canary.call(json!({"cmd": "play", "path": tone, "app": true}));
    // The pids each start heard, from the log.
    let starts = || -> Vec<Value> {
        let log = std::fs::read_to_string(home.join("log")).unwrap_or_default();
        let lines = log.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok());
        lines.filter(|line| line["ev"] == "listen_audio").map(|line| line["pids"].clone()).collect()
    };
    // Started again until SCK offers the child (its window reaches the window server).
    let begun = Instant::now();
    let reply = loop {
        otto.call(json!({"cmd": "audio_probe_start"}));
        if starts().last() == Some(&json!(2)) {
            std::thread::sleep(Duration::from_secs(4));
            break Some(otto.call(json!({"cmd": "audio_probe_stop"})));
        }
        otto.call(json!({"cmd": "audio_probe_stop"}));
        if begun.elapsed() > Duration::from_secs(12) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let started = starts();
    eprintln!("the log's starts, pids: {started:?}");
    let _ = std::fs::remove_dir_all(&home);
    let reply = reply.unwrap_or_else(|| panic!("no start heard the canary's child: {started:?}"));

    let buffers = reply["buffers"].as_array().expect("buffers");
    let loud = buffers.iter().filter(|b| b["rms"].as_f64().is_some_and(|rms| rms >= 0.01)).count();
    assert!(loud > 0, "no sound in {} buffers; the stream said {}", buffers.len(), reply["failed"]);
}

/// Whether ScreenCaptureKit sends buffers while an app is silent, or none: then the samples fed
/// to the recogniser lose each silence and a line's time shrinks by it. The canary plays a tone
/// with 8 s of silence inside the file, then plays nothing for 8 s, then a tone again; each
/// buffer's presentation time is kept with no recogniser (CI's virtual machine has none). The
/// time from the first buffer to the last is what the samples must cover.
#[test]
#[ignore = "hears the canary's own tones; run with `cargo e2e-apps`"]
fn the_canarys_silence_comes_as_samples_not_as_a_gap_in_time() {
    let home = support::temp(&format!("otto-apps-probe-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("make the folder");
    std::fs::write(home.join("settings.json"), json!({"restore_layout": false}).to_string()).expect("settings");
    // Tone, 8 s of zeros, tone; then a short tone.
    let (gap, gap_s) = (home.join("gap.wav"), 2. + 8. + 2.);
    wav(&gap, &[(2., true), (8., false), (2., true)]);
    let (short, short_s) = (home.join("short.wav"), 1.5);
    wav(&short, &[(short_s, true)]);

    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-probe", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-probe",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_HOME", home.to_str().expect("a plain path")),
        ],
    );
    otto.call(json!({"cmd": "audio_probe_start"}));
    std::thread::sleep(Duration::from_secs(1));
    canary.call(json!({"cmd": "play", "path": gap}));
    std::thread::sleep(Duration::from_secs_f64(gap_s + 0.5));
    // Nothing playing.
    std::thread::sleep(Duration::from_secs(8));
    canary.call(json!({"cmd": "play", "path": short}));
    std::thread::sleep(Duration::from_secs_f64(short_s + 1.0));
    let reply = otto.call(json!({"cmd": "audio_probe_stop"}));
    let _ = std::fs::remove_dir_all(&home);

    // (presentation ms, length ms, rms) of each buffer with a time.
    let buffers: Vec<(f64, f64, f64)> = reply["buffers"]
        .as_array()
        .expect("buffers")
        .iter()
        .filter_map(|b| Some((b["pts_ms"].as_f64()?, b["samples"].as_f64()? / 16., b["rms"].as_f64()?)))
        .collect();
    let given = reply["buffers"].as_array().map_or(0, Vec::len);
    assert!(!buffers.is_empty(), "no buffer with a time of {given}; the stream said {}", reply["failed"]);
    let (first, last) = (buffers[0], buffers[buffers.len() - 1]);
    let span = last.0 + last.1 - first.0;
    let sampled: f64 = buffers.iter().map(|b| b.1).sum();
    // Where time jumps past the samples: (at ms from the first, ms missing).
    let gaps: Vec<(f64, f64)> =
        buffers.windows(2).map(|w| (w[1].0 - first.0, w[1].0 - (w[0].0 + w[0].1))).filter(|gap| gap.1 > 100.).collect();
    let quiet = buffers.iter().filter(|b| b.2 < 0.001).count();
    eprintln!("audio probe: the stream said {}", reply["failed"]);
    eprintln!(
        "audio probe: {given} buffers ({} timed, {quiet} silent), {span:.0} ms presented, {sampled:.0} ms of samples, \
         gaps over 100 ms (at, missing): {gaps:.0?}",
        buffers.len()
    );
    // The largest step from one buffer's time to the next: needs no sample rate.
    let step = buffers.windows(2).map(|w| w[1].0 - w[0].0).fold(0., f64::max);
    let played = (gap_s + 8. + short_s - 1.) * 1_000.;
    // Words heard before the silences and after them: the first sound to the last spans both.
    let loud: Vec<f64> = buffers.iter().filter(|b| b.2 >= 0.001).map(|b| b.0).collect();
    let heard = loud.last().zip(loud.first()).map_or(0., |(last, first)| last - first);
    assert!(heard > (gap_s + 8. - 1.) * 1_000., "sound heard over {heard:.0} ms only, {} loud buffers", loud.len());
    assert!(span > played, "the canary played for {played:.0} ms, buffers span {span:.0} ms");
    assert!(step < 1_000., "{step:.0} ms between two buffers: {gaps:.0?}");
    assert!((span - sampled).abs() < 500., "{:.0} ms presented, not sampled: {gaps:.0?}", span - sampled);
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn the_window_fills_its_display_an_apps_window_stops_at_the_dock_and_every_end_brings_the_bars_back() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-fill", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || global_frame(&canary, 0));
    let mut otto = otto_for(&canary, "otto-fill", &[]);
    let state = || otto.call(json!({"cmd": "state"}));
    let fill = || drop(otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-f", "as": "user"})));
    // As another app sees the system: whether the bars are away, and what they leave when they show.
    let bars_hidden = |hidden: bool| (canary.call(json!({"cmd": "state"}))["bars_hidden"] == hidden).then_some(());
    let visible = rect(&canary.call(json!({"cmd": "state"}))["visible"]).expect("the visible frame");
    host_and_park(&otto);
    bring_forward(otto.child.id(), "ottospace");
    let window = || rect(&state()["window"]);
    let before = window().expect("ottospace's window frame");
    let small = rect(&state()["bounds"]["1"]).expect("the pane");
    // The display the window's middle is on.
    let [x, y] = middle(before);
    let on = |d: &&objc2_core_foundation::CGRect| {
        x >= d.origin.x && x < d.origin.x + d.size.width && y >= d.origin.y && y < d.origin.y + d.size.height
    };
    let displays = ottospace_mac::display::all();
    let d = displays.iter().find(on).expect("the window's display");
    let display = [d.origin.x, d.origin.y, d.size.width, d.size.height];
    let at = |to: [f64; 4]| move |w: &[f64; 4]| (0..4).all(|i| close(w[i], to[i]));

    // On: the display's whole frame, the bars away, and the layout as large as the window.
    fill();
    assert_eq!(state()["state"]["fill_display"], true);
    wait_for("the window to fill its display", || window().filter(at(display)));
    wait_for("the bars to go", || bars_hidden(true));
    let body = wait_for("the layout to follow", || rect(&state()["bounds"]["1"]).filter(|b| b[3] > small[3] + 20.0));
    let frames = || {
        let s = state();
        let fit = &s["state"]["panes"][0]["fit"];
        format!("window {}, pane {}, app's window {}, misfit {fit}", s["window"], s["bounds"]["1"], s["hosted"]["1"])
    };
    // ottospace in front, the Dock away: the app's picture goes below the Dock's top, as far as
    // macOS lets a window grow (no taller than what the bars leave, measured: 679 of 700 asked),
    // and no misfit is named.
    let crop = state()["crop"]["1"].as_f64().expect("the crop");
    let covers = |pane: [f64; 4]| move |f: &[f64; 4]| f[3] - crop >= pane[3].min(visible[3] - crop) - 4.0;
    let filled = || {
        let s = state();
        let parked = rect(&s["hosted"]["1"]).filter(covers(body)).filter(|_| s["state"]["panes"][0]["fit"].is_null());
        parked.filter(|_| rect(&s["window"]).as_ref().is_some_and(at(display)))
    };
    let parked = wait_or("the app's picture to fill its pane", frames, filled);
    std::thread::sleep(Duration::from_millis(1200));
    report(&format!("fill, ottospace in front, 1.2 s on: {}", frames()));
    assert!(filled().is_some(), "after a second: {}", frames());

    // In the app: its bars come back over ottospace's edges, and ottospace's window stays. The
    // app's window covers its pane down to the Dock's top, no further, and is not named a misfit.
    let bottom = (display[1] + body[1] + body[3]).min(visible[1] + visible[3]);
    let fits = |f: &[f64; 4]| (f[1] + f[3] - bottom).abs() <= 4.0;
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    let live = wait_for("the app's window over its pane, above the Dock", || {
        rect(&state()["hosted"]["1"]).filter(|f| close(f[1], display[1] + body[1] - crop) && fits(f))
    });
    wait_or("the canary to be the front app", in_front, || {
        ask_forward(canary.child.id());
        (front_app().0 == canary.child.id()).then_some(())
    });
    wait_for("the bars to come back for the app", || bars_hidden(false));
    // Past a check of the window's size (every 500 ms).
    std::thread::sleep(Duration::from_millis(1200));
    report(&format!("fill, in the app pane: {}", frames()));
    let now = state();
    assert_eq!(
        (&now["state"]["panes"][0]["fit"], &now["state"]["fill_display"]),
        (&Value::Null, &json!(true)),
        "{now}"
    );
    assert_eq!(rect(&now["window"]).filter(at(display)).is_some(), true, "ottospace's window moved: {}", now["window"]);
    assert_eq!(rect(&now["bounds"]["1"]), Some(body), "the layout moved under the app");
    // Back in ottospace: parked as before, the picture to the pane's foot again.
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    bring_forward(otto.child.id(), "ottospace");
    wait_for("the bars to go again", || bars_hidden(true));
    let again =
        wait_or("the window to park", frames, || rect(&state()["hosted"]["1"]).filter(|f| !close(f[0], live[0])));
    assert!(close(again[2], parked[2]) && close(again[3], parked[3]), "parked {again:?}, first {parked:?}");
    std::thread::sleep(Duration::from_millis(1200));
    report(&format!("fill, back in ottospace: {}", frames()));
    assert!(filled().is_some(), "back in ottospace: {}", frames());

    // The pane resized (a split beside it, then closed): the window keeps the display's frame.
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    let narrow =
        wait_or("the pane to narrow", frames, || rect(&state()["bounds"]["1"]).filter(|b| b[2] < body[2] - 20.0));
    wait_or("the narrow pane's picture to its foot", frames, || {
        rect(&state()["hosted"]["1"]).filter(|f| close(f[2], narrow[2]) && covers(narrow)(f))
    });
    assert!(window().as_ref().is_some_and(at(display)), "split: {}", frames());
    otto.call(json!({"cmd": "close"}));
    wait_or("the pane to widen", frames, || rect(&state()["bounds"]["1"]).filter(|b| close(b[2], body[2])));
    wait_or("the picture to fill its pane again", frames, filled);
    std::thread::sleep(Duration::from_millis(1200));
    report(&format!("fill, after the split closed: {}", frames()));
    assert!(filled().is_some(), "after the split closed: {}", frames());

    // Off: the window where it was, the bars as the system has them.
    fill();
    let held =
        || format!("the window is at {:?}, it was at {before:?}; {}", window(), state()["state"]["fill_display"]);
    wait_or("the window back where it was", held, || window().filter(at(before)));
    wait_for("the bars back", || bars_hidden(false));
    assert_eq!(state()["state"]["fill_display"], false);

    // On, and ottospace is killed: nothing of the mode outlives it.
    fill();
    wait_for("the bars to go once more", || bars_hidden(true));
    otto.child.kill().expect("kill -9");
    let _ = otto.child.wait();
    wait_for("the bars back after the kill", || bars_hidden(false));
}

/// Closes Finder's windows of these names on drop, pass or fail: Finder is the user's, and the
/// test opened them. Declared before ottospace, so it runs after ottospace is gone.
struct CloseFinderWindows(Vec<String>);

impl Drop for CloseFinderWindows {
    fn drop(&mut self) {
        for name in &self.0 {
            let script = format!(
                "tell application \"System Events\" to tell process \"Finder\" to repeat with w in \
                 (every window whose name is \"{name}\")\nclick (first button of w whose subrole is \
                 \"AXCloseButton\")\nend repeat"
            );
            let _ = Command::new("osascript").args(["-e", &script]).status();
        }
    }
}

/// Finder's pid: it always runs.
fn finder_pid() -> i32 {
    let finder =
        ottospace_mac::ax::running().into_iter().find(|(_, bundle, _)| bundle.as_deref() == Some("com.apple.finder"));
    finder.expect("Finder runs").0
}

/// The first node named `name` (title, value or description) with a frame: `[x, y, w, h]`.
fn named_frame(node: &Value, name: &str) -> Option<[f64; 4]> {
    let named = ["title", "value", "description"].iter().any(|k| node[*k] == name);
    let f = &node["frame"];
    match [f["x"].as_f64(), f["y"].as_f64(), f["w"].as_f64(), f["h"].as_f64()] {
        [Some(x), Some(y), Some(w), Some(h)] if named && w > 0.0 && h > 0.0 => Some([x, y, w, h]),
        _ => node["children"].as_array()?.iter().find_map(|child| named_frame(child, name)),
    }
}

#[test]
#[ignore = "moves a Finder window it opens; run with `cargo e2e-apps` and OTTOSPACE_E2E_FINDER"]
fn a_hosted_finder_window_takes_the_users_double_clicks() {
    // Finder is the user's own app: only where nobody's Finder windows are (CI asks for it).
    if std::env::var_os("OTTOSPACE_E2E_FINDER").is_none() {
        return eprintln!("OTTOSPACE_E2E_FINDER not set: Finder not hosted");
    }
    let pid = finder_pid();
    // Hosting picks Finder's main window, and adopts every new one: none of them may be the user's.
    if !ottospace_mac::ax::Window::standard_of(pid).is_empty() {
        return eprintln!("Finder has windows open: not hosted, so none of them moves");
    }
    let me = std::process::id();
    let dir = support::temp(&format!("otto-apps-finder-{me}"));
    // `inner`: opened by a double-click while hosted. `first`: given back, the control.
    let (first, inner) = (format!("otto-first-{me}"), format!("otto-inner-{me}"));
    for name in [&first, &inner] {
        std::fs::create_dir_all(dir.join(name)).expect("make folders");
    }
    let outer = dir.file_name().and_then(|n| n.to_str()).expect("folder name").to_string();
    let _close = CloseFinderWindows(vec![outer.clone(), first.clone(), inner.clone()]);
    let opened = Command::new("open").args(["-a", "Finder"]).arg(&dir).status().expect("run open");
    assert!(opened.success(), "open -a Finder failed");
    let ours = || {
        let windows = ottospace_mac::ax::Window::standard_of(pid);
        windows.into_iter().find(|w| w.title().is_some_and(|t| t == outer))
    };
    let finder_window = wait_for("Finder's window of the folder", || ours().and_then(|w| w.id()));
    let frame = || {
        let w = ottospace_mac::ax::Window::find(pid, finder_window)?;
        w.frame().map(|f| [f.origin.x, f.origin.y, f.size.width, f.size.height])
    };
    let title = || ottospace_mac::ax::Window::find(pid, finder_window).and_then(|w| w.title());
    // Where Finder draws a folder's icon, read from its own tree.
    let icon_of = |name: &str| {
        let window = ottospace_mac::ax::Window::find(pid, finder_window)?;
        let limits = ottospace_core::ax::Limits { depth: 30, nodes: 3000, millis: None };
        let tree = ottospace_mac::ax_tree::read(&window, limits, ottospace_mac::ax_tree::Fetch::Batched);
        named_frame(&serde_json::to_value(&tree.root).ok()?, name)
    };
    let double_click = |icon: [f64; 4]| click_at(icon[0] + icon[2] / 2.0, icon[1] + icon[3] / 2.0, true);
    let original = wait_for("its frame", frame);

    let pid_text = pid.to_string();
    let env = [("OTTOSPACE_E2E", "1"), ("OTTOSPACE_HOST_ONLY", pid_text.as_str()), ("OTTOSPACE_RETURN_KEY", "f19")];
    let otto = Proc::launch(env!("CARGO_BIN_EXE_ottospace"), "otto-finder", "OTTOSPACE_SOCKET", &env);
    let state = || otto.call(json!({"cmd": "state"}));
    let reply = otto.call(json!({"cmd": "open_app", "name": "Finder"}));
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "app", "{reply}");
    let spot = point(&state()["parking"]).expect("parking spot");
    wait_for("the Finder window to park", || frame().filter(|f| close(f[0], spot[0])));
    let seen = || format!("front {:?}; window at {:?}; mode {}", front_app(), frame(), state()["mode"]);

    // The user's way in: a real click on the parked pane makes it live.
    bring_forward(otto.child.id(), "ottospace");
    let at = on_picture(&otto);
    click_at(at[0], at[1], false);
    wait_or("the pane to go live", seen, || (state()["mode"] == "app").then_some(()));
    let live = wait_or("the window over its pane", seen, || frame().filter(|f| !close(f[0], spot[0])));
    std::thread::sleep(Duration::from_millis(800));
    // A real double-click on the folder inside opens it: Finder took both clicks. With the
    // window captured while live, macOS lost a release of most double-clicks.
    let icon = wait_for("the inner folder's icon", || icon_of(&inner));
    let (x, y) = (icon[0] + icon[2] / 2.0, icon[1] + icon[3] / 2.0);
    assert!(x > live[0] && x < live[0] + live[2] && y > live[1] && y < live[1] + live[3], "{icon:?} in {live:?}");
    double_click(icon);
    let opened = || format!("title {:?}; icon {icon:?}; {}", title(), seen());
    wait_or("the double-click to open the folder", opened, || (title()? == inner).then_some(()));

    // Quit: the window goes back where it was, and only the test's windows close.
    otto.call(json!({"cmd": "quit"}));
    wait_for("the Finder window back", || frame().filter(|f| (0..4).all(|i| close(f[i], original[i]))));
    drop(otto);
    // The control: given back, a double-click on a folder opens it.
    bring_forward(pid as u32, "Finder");
    press("keystroke \"[\" using command down");
    wait_for("the window back at the folder", || (title()? == outer).then_some(()));
    let icon = wait_for("the first folder's icon", || icon_of(&first));
    double_click(icon);
    let control = || format!("title {:?}, icon {icon:?}, window {:?}, front {:?}", title(), frame(), front_app());
    wait_or("the control double-click to open its folder", control, || (title()? == first).then_some(()));
}

/// A global point in the middle of pane `pane`'s picture, once its stream shows.
fn picture_of(otto: &Proc, pane: u64) -> [f64; 2] {
    let state = || otto.call(json!({"cmd": "state"}));
    let key = pane.to_string();
    let shown = wait_for("a frame of its stream", || rect(&state()["shown"][&key]).filter(|s| s[2] > 100.0));
    let window = rect(&state()["window"]).expect("ottospace window frame");
    [window[0] + shown[0] + shown[2] / 2.0, window[1] + shown[1] + shown[3] / 2.0]
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn clicks_reach_hosted_windows_as_the_user_switches_between_their_panes() {
    // Two canary windows in two panes. Each round: a click on a parked pane's picture (replayed
    // into its window), then a click on the live window itself; then the other pane.
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-switch", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-switch", &[]);
    let state = || otto.call(json!({"cmd": "state"}));
    let spot = host_and_park(&otto);
    canary.call(json!({"cmd": "open_window"}));
    take_offer(&otto, wait_offers(&otto, 1)[0]);
    wait_for("the second window's pane", || (state()["tab_panes"] == json!([[1, 2]])).then_some(()));
    let clicks = |i: usize| canary.call(json!({"cmd": "state"}))["windows"][i]["clicks"].as_array().map_or(0, Vec::len);
    // Waits up to 2 s for window `i` to have more clicks than `before`.
    let reached = |i: usize, before: usize| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if clicks(i) > before {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    };
    // What pane `pane`'s stream has sent: none while live, more once parked again.
    let samples = |pane: u64| state()["counts"][format!("stream_samples_{pane}")].as_u64().unwrap_or(0);
    let mut parked: Option<(u64, u64)> = None;
    let rounds = std::env::var("OTTOSPACE_E2E_ROUNDS").ok().and_then(|n| n.parse().ok()).unwrap_or(6);
    let (mut replays, mut directs, mut tries) = (0, 0, 0);
    let mut missed = Vec::new();
    for round in 0..rounds {
        for (pane, index) in [(1u64, 0usize), (2, 1)] {
            tries += 1;
            // The first from ottospace in front; the others straight from the other pane's live
            // window, as the user goes from one app to the next.
            if tries == 1 {
                bring_forward(otto.child.id(), "ottospace");
            }
            let at = picture_of(&otto, pane);
            let before = clicks(index);
            click_at(at[0], at[1], false);
            let live_now = || (state()["state"]["live"] == pane).then_some(());
            wait_or("the pane to go live", || format!("{}", state()["state"]), live_now);
            // The pane left for this one parked: its stream sends again.
            if let Some((other, sent)) = parked.take() {
                wait_for("the parked pane's stream to send again", || (samples(other) > sent).then_some(()));
            }
            if reached(index, before) {
                replays += 1;
            } else {
                missed.push(format!("round {round} pane {pane}: replay; front {:?}", front_app()));
            }
            // The live window, clicked where the user clicks next: its own content.
            let live =
                wait_for("the window over its pane", || global_frame(&canary, index).filter(|f| !close(f[0], spot[0])));
            std::thread::sleep(Duration::from_millis(300));
            let before = clicks(index);
            click_at(live[0] + live[2] / 2.0, live[1] + live[3] - 40.0, false);
            // Live: its stream sends nothing (one sample may be on its way as it stops).
            let sent = samples(pane);
            std::thread::sleep(Duration::from_millis(500));
            assert!(samples(pane) <= sent + 1, "pane {pane}'s stream sent {} while live", samples(pane) - sent);
            parked = Some((pane, samples(pane)));
            if reached(index, before) {
                directs += 1;
            } else {
                missed.push(format!(
                    "round {round} pane {pane}: direct at {live:?}; front {:?}; mode {}",
                    front_app(),
                    state()["mode"]
                ));
            }
        }
    }
    report(&format!("switching: replayed {replays}/{tries}, direct {directs}/{tries}; missed {missed:?}"));
    assert!(missed.is_empty(), "{missed:?}");
    otto.call(json!({"cmd": "quit"}));
}

/// Agent mode on a hosted canary: its task's agent (the stand-in `claude`, its pass from a real
/// start) reads the window's tree and a picture of it only once the user set See; the user's Off
/// refuses the next read and removes the pictures. A client's `open_app` starts at Off.
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_task_reads_and_looks_at_the_canary_only_while_the_user_lets_it() {
    use std::os::unix::fs::PermissionsExt;
    let root = support::temp(&format!("otto-apps-agent-{}", std::process::id()));
    let (bin, record, home, sessions) =
        (root.join("bin"), root.join("record"), root.join("home"), root.join("sessions"));
    let repo = root.join("app");
    for dir in [&bin, &record, &home, &sessions, &repo.join(".git")] {
        std::fs::create_dir_all(dir).expect("make the folder");
    }
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
    let fake = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ottospace-agents/tests/fakes/claude");
    std::fs::copy(&fake, bin.join("claude")).expect("copy the stand-in");
    let settings = json!({"conversation_view": "native", "session_dirs": {"claude": [sessions]}});
    std::fs::write(home.join("settings.json"), settings.to_string()).expect("write settings");
    let path = |p: &PathBuf| p.to_str().expect("a plain path").to_string();
    let (bin_s, record_s, home_s, repo_s) = (path(&bin), path(&record), path(&home), path(&repo));

    let env = [("CANARY_BUTTON", "1")];
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-agent", "CANARY_SOCKET", &env);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace"),
        "otto-agent",
        "OTTOSPACE_SOCKET",
        &[
            ("OTTOSPACE_E2E", "1"),
            ("OTTOSPACE_HOST_ONLY", &canary.child.id().to_string()),
            ("OTTOSPACE_RETURN_KEY", "f19"),
            ("OTTOSPACE_HOME", &home_s),
            ("OTTOSPACE_REPOS", &repo_s),
            ("OTTOSPACE_FEATURES", "agent_mode"),
            ("OTTOSPACE_AGENTS_DIR", &bin_s),
            ("FAKE_DIR", &record_s),
            ("FAKE_FIXTURES", &record_s),
            ("FAKE_SAY_PASS", "1"),
        ],
    );
    let state = || otto.call(json!({"cmd": "state"}));
    let user_key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys, "as": "user"}));
    // A point of a labelled control (`fx`, `fy` of its frame), from one pass of sneak mode.
    let control = |id: &str, fx: f64, fy: f64| -> [f64; 2] {
        let reply = otto.call(json!({"cmd": "sneak"}));
        otto.call(json!({"cmd": "key", "keys": "escape"}));
        let targets = reply["sneak"]["targets"].as_array().cloned().unwrap_or_default();
        let found = targets.iter().find(|t| t["source"]["id"] == id).unwrap_or_else(|| panic!("no {id}: {reply}"));
        let f = |key: &str| found["frame"][key].as_f64().expect("a frame");
        [f("x") + f("w") * fx, f("y") + f("h") * fy]
    };
    let user_click = |p: [f64; 2]| {
        for action in ["down", "up"] {
            otto.call(json!({"cmd": "mouse", "action": action, "x": p[0], "y": p[1], "as": "user"}));
        }
    };
    // Asked with a pass: the reply, refused or not (`call` asserts it is not).
    let ask = |cmd: &str, pane: u64, pass: &str| -> Value {
        let mut stream = UnixStream::connect(&otto.socket).expect("connect");
        stream.set_read_timeout(Some(TIMEOUT)).expect("set timeout");
        writeln!(stream, "{}", json!({"cmd": cmd, "pane": pane, "pass": pass})).expect("send");
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).expect("a reply");
        serde_json::from_str(&reply).expect("reply is JSON")
    };

    // The user starts task 1's agent: its workspace on show, its pass in the stand-in's record.
    otto.call(json!({"cmd": "task_new", "line": "canary >app"}));
    otto.call(json!({"cmd": "section", "section": "tasks"}));
    user_click(control("task:1", 0.3, 0.3));
    user_click(control("talk:talk-start", 0.5, 0.5));
    let pass = wait_for("its pass", || {
        std::fs::read_to_string(record.join("pass")).ok().filter(|t| t.ends_with('\n')).map(|t| t.trim().to_string())
    });
    // A client's open_app: Off, refused.
    let reply = otto.call(json!({"cmd": "open_app", "name": CANARY}));
    let panes = reply["state"]["panes"].as_array().cloned().unwrap_or_default();
    let pane = panes.iter().find(|p| p["kind"]["type"] == "app").and_then(|p| p["id"].as_u64()).expect("its pane");
    let id = pane.to_string();
    wait_for("a frame of its stream", || rect(&state()["shown"][&id]).filter(|shown| shown[2] > 100.0));
    assert_eq!(state()["state"]["agent_mode"]["panes"][&id], json!({"level": "off", "task": 1}));
    let off = ask("app_read", pane, &pass);
    assert!(off["error"].as_str().is_some_and(|e| e.contains("agent mode is off on pane")), "{off}");

    // The user's See (⌘⌃V, ↓, ↵): the tree, labelled untrusted, and a picture only the user reads.
    user_key("cmd-ctrl-v");
    user_key("down");
    user_key("enter");
    assert_eq!(state()["state"]["agent_mode"]["panes"][&id], json!({"level": "see", "task": 1}));
    let tree = ask("app_read", pane, &pass);
    assert_eq!((&tree["task"], &tree["captured"]["untrusted"]), (&json!(1), &json!(true)), "{tree}");
    assert_eq!(tree["captured"]["tree"]["role"], "AXWindow", "{tree}");
    let children = tree["captured"]["tree"]["children"].as_array().cloned().unwrap_or_default();
    assert!(children.iter().any(|node| node["title"] == "Canary button"), "{tree}");
    let look = ask("app_look", pane, &pass);
    let png = PathBuf::from(look["captured"]["path"].as_str().unwrap_or_else(|| panic!("{look}")));
    let mode = std::fs::metadata(&png).expect("the picture").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "{look}");
    assert!(std::fs::read(&png).expect("the picture").starts_with(b"\x89PNG"));

    // The user's Off (⌘⌃V, ↵ on Off): the next read refused, the pictures gone.
    user_key("cmd-ctrl-v");
    user_key("enter");
    assert_eq!(state()["state"]["agent_mode"]["panes"][&id]["level"], "off");
    let again = ask("app_read", pane, &pass);
    assert!(again["error"].as_str().is_some_and(|e| e.contains("agent mode is off on pane")), "{again}");
    let folder = png.parent().expect("its folder").to_path_buf();
    wait_for("the pictures gone", || (!folder.exists()).then_some(()));
    drop(otto);
    let _ = std::fs::remove_dir_all(&root);
}

/// A window's picture by `screencapture -l`, no shadow: width, height and RGBA, top row first.
fn window_pixels(window: u64, path: &std::path::Path) -> (usize, usize, Vec<u8>) {
    let status = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-o", "-t", "bmp", "-l", &window.to_string()])
        .arg(path)
        .status()
        .expect("screencapture");
    assert!(status.success(), "screencapture -l {window}");
    let bmp = std::fs::read(path).expect("the picture");
    let le = |at: usize| u32::from_le_bytes(bmp[at..at + 4].try_into().expect("4 bytes"));
    let (start, width, height) = (le(10) as usize, le(18) as usize, le(22) as i32);
    assert_eq!(u16::from_le_bytes([bmp[28], bmp[29]]), 32, "32 bits a pixel");
    // Bit fields red, green, blue: bytes B, G, R, A.
    assert_eq!((le(30), le(54), le(58), le(62)), (3, 0xff0000, 0xff00, 0xff), "BGRA");
    let rows = height.unsigned_abs() as usize;
    let mut rgba = Vec::with_capacity(width * rows * 4);
    for row in 0..rows {
        // Negative: top row first.
        let from = if height < 0 { row } else { rows - 1 - row };
        for bgra in bmp[start + from * width * 4..][..width * 4].chunks_exact(4) {
            rgba.extend([bgra[2], bgra[1], bgra[0], 255]);
        }
    }
    (width, rows, rgba)
}

/// `w`×`h` pixels of a picture from `(x, y)`, as grey levels.
fn grey(picture: &(usize, usize, Vec<u8>), x: usize, y: usize, w: usize, h: usize) -> Vec<f64> {
    let (width, height, rgba) = picture;
    assert!(x + w <= *width && y + h <= *height, "{w}×{h} at {x},{y} in {width}×{height}");
    let mut out = Vec::with_capacity(w * h);
    for row in y..y + h {
        for p in rgba[(row * width + x) * 4..][..w * 4].chunks_exact(4) {
            out.push(0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64);
        }
    }
    out
}

/// Mean difference of neighbouring pixels, across and down: high for a sharp picture of stripes
/// or text, lower once it is resampled.
fn sharpness(g: &[f64], w: usize) -> f64 {
    let h = g.len() / w;
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            sum += (g[y * w + x] - g[y * w + x + 1]).abs() + (g[y * w + x] - g[(y + 1) * w + x]).abs();
            n += 2.0;
        }
    }
    sum / n
}

/// The box of the canary's stripes in `g`: rows where 32 neighbouring pairs differ white to black.
fn stripes_box(g: &[f64], w: usize) -> Option<[usize; 4]> {
    let h = g.len() / w;
    let alternates = |x: usize, y: usize| (g[y * w + x] - g[y * w + x + 1]).abs() > 160.0;
    let rows: Vec<(usize, usize, usize)> = (0..h)
        .filter_map(|y| {
            let xs: Vec<usize> = (0..w - 1).filter(|&x| alternates(x, y)).collect();
            (xs.len() >= 32).then(|| (y, xs[0], xs[xs.len() - 1] + 1))
        })
        .collect();
    let (top, bottom) = (rows.first()?.0, rows.last()?.0 + 1);
    let (left, right) = (rows.iter().map(|r| r.1).min()?, rows.iter().map(|r| r.2).max()?);
    Some([left, top, right - left, bottom - top])
}

/// `OTTOSPACE_SHARP_OUT`: where CI keeps the crops it compared, as PNGs.
fn keep_crop(name: &str, picture: &(usize, usize, Vec<u8>), at: [usize; 4]) {
    let Some(dir) = std::env::var_os("OTTOSPACE_SHARP_OUT").map(PathBuf::from) else { return };
    let (width, height, rgba) = picture;
    let [x, y] = [at[0].min(width - 1), at[1].min(height - 1)];
    let [w, h] = [at[2].min(width - x), at[3].min(height - y)];
    let crop: Vec<u8> = (y..y + h).flat_map(|row| rgba[(row * width + x) * 4..][..w * 4].to_vec()).collect();
    let _ = std::fs::create_dir_all(&dir);
    if let Some(png) = ottospace_mac::image::png_from_rgba(w, h, &crop) {
        let _ = std::fs::write(dir.join(format!("{name}.png")), png);
    }
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_parked_panes_picture_is_its_window_pixel_for_pixel() {
    let canary = Proc::launch(
        env!("CARGO_BIN_EXE_ottospace-canary"),
        "canary-sharp",
        "CANARY_SOCKET",
        &[("CANARY_PATTERN", "1")],
    );
    wait_for("the canary window", || global_frame(&canary, 0));
    let otto = otto_for(&canary, "otto-sharp", &[]);
    host_and_park(&otto);
    let state = || otto.call(json!({"cmd": "state"}));
    let count = |s: &Value, key: &str| s["counts"][key].as_f64().unwrap_or(0.0);
    // The held frame has the pixels the pane is drawn with: points × the display's scale.
    let s = wait_or(
        "a frame of the pane's size in pixels",
        || state()["counts"].to_string(),
        || {
            let s = state();
            let shown = rect(&s["shown"]["1"]).filter(|r| r[2] > 100.0)?;
            let scale = count(&s, "scale");
            let size = [count(&s, "stream_width_1"), count(&s, "stream_height_1")];
            (size == [(shown[2] * scale).round(), (shown[3] * scale).round()]).then_some(s)
        },
    );
    let (scale, shown) = (count(&s, "scale"), rect(&s["shown"]["1"]).expect("shown"));
    let canary_scale = canary.call(json!({"cmd": "state"}))["windows"][0]["scale"].as_f64().unwrap_or(0.0);
    report(&format!(
        "sharp: ottospace scale {scale}, parked display {}, canary window {canary_scale}, stream {}×{} px for {}×{} pt",
        count(&s, "parking_scale"),
        count(&s, "stream_width_1"),
        count(&s, "stream_height_1"),
        shown[2],
        shown[3]
    ));
    assert_eq!(count(&s, "stream_scale_1"), scale, "the stream captures at ottospace's scale");
    // No free corner of ottospace's scale (a 2× laptop, a 1× external to its right, lower): the
    // window draws at another scale parked, and no picture of it matches its pixels one to one.
    if count(&s, "parking_scale") != scale {
        report("sharp: parked on a display of another scale, the pixels are not compared");
        return;
    }

    // What ottospace draws: its own window, the preview's rect in pixels. This reply waits for a
    // frame drawn after the one held came.
    state();
    let otto_window = s["frame"]["window"].as_u64().expect("ottospace's window number");
    let drawn = window_pixels(otto_window, &support::temp(&format!("otto-sharp-{}.bmp", std::process::id())));
    let px = |v: f64| (v * scale).round() as usize;
    let (w, h) = (px(shown[2]), px(shown[3]));
    let picture = grey(&drawn, px(shown[0]), px(shown[1]), w, h);

    // The window itself, still parked: what the pane must show, whatever the app drew in it. One
    // ScreenCaptureKit picture: `screencapture -l` gives none of a window parked off screen.
    let crop = s["crop"]["1"].as_f64().unwrap_or(0.0);
    let canary_window = s["state"]["panes"][0]["kind"]["window"].as_u64().expect("the canary's window number");
    let frame = rect(&s["hosted"]["1"]).expect("the parked window's frame");
    let (pw, ph) = (px(frame[2]), px(frame[3]));
    ottospace_mac::capture::init();
    let parked =
        ottospace_mac::capture::window_rgba(canary_window as u32, pw, ph).expect("a picture of the parked window");
    let parked = (pw, ph, parked);
    let real = grey(&parked, 0, px(crop), w, h);

    let parked_box = stripes_box(&real, w).expect("the canary's stripes in its window");
    let [x, y, sw, sh] = parked_box;
    // The rows below the columns too.
    let sh = (2 * sh).min(h - y);
    let part = |g: &[f64]| -> Vec<f64> { (y..y + sh).flat_map(|row| g[row * w + x..][..sw].to_vec()).collect() };
    let within =
        |a: &[f64], b: &[f64]| a.iter().zip(b).filter(|(a, b)| (*a - *b).abs() <= 8.0).count() as f64 / b.len() as f64;
    let (stripes_picture, stripes_real) = (part(&picture), part(&real));
    let same = within(&stripes_picture, &stripes_real);
    let (sharp_picture, sharp_real) = (sharpness(&stripes_picture, sw), sharpness(&stripes_real, sw));
    report(&format!(
        "sharp: stripes {sw}×{sh} px at {x},{y}; sharpness picture {sharp_picture:.1}, window {sharp_real:.1}; \
         {:.1}% of the stripes' pixels within 8 levels of the window's",
        same * 100.0
    ));
    // The stripes and the text above them, from each.
    let (left, top) = (x.saturating_sub(40), y.saturating_sub(120));
    keep_crop("picture", &drawn, [px(shown[0]) + left, px(shown[1]) + top, sw + 80, sh + 140]);
    keep_crop("window", &parked, [left, px(crop) + top, sw + 80, sh + 140]);
    assert!(
        sharp_picture >= 0.95 * sharp_real,
        "the picture is as sharp as the window: {sharp_picture} vs {sharp_real}"
    );
    assert!(same >= 0.98, "the picture is the window's pixels: {:.1}% within 8 levels", same * 100.0);

    // Soft signal, never a failure: the window live over its pane against itself parked. The app
    // may keep old-size content stretched while parked (GPUI missing a resize that changes display).
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    // Over its pane and drawn at ottospace's scale: it may have been parked on another display.
    let spot = point(&s["parking"]).expect("parking spot");
    wait_for("the window to go live at ottospace's scale", || {
        let away = rect(&state()["hosted"]["1"]).is_some_and(|f| !close(f[0], spot[0]));
        let at = canary.call(json!({"cmd": "state"}))["windows"][0]["scale"].as_f64() == Some(scale);
        (away && at).then_some(())
    });
    std::thread::sleep(Duration::from_millis(500));
    let live = window_pixels(canary_window, &support::temp(&format!("canary-sharp-{}.bmp", std::process::id())));
    assert_eq!(otto.call(json!({"cmd": "leave"}))["mode"], "normal");
    keep_crop("live", &live, [left, px(crop) + top, sw + 80, sh + 140]);
    if live.0 < w || live.1 < px(crop) + h {
        report(&format!("sharp: the live window is {}×{} px, not compared with the parked one", live.0, live.1));
        return;
    }
    let real_live = grey(&live, 0, px(crop), w, h);
    let same_live = within(&stripes_real, &part(&real_live));
    if same_live < 0.98 {
        let line = format!(
            "soft signal, parked window not redrawn: its stripes {parked_box:?} (x, y, w, h), live {:?}; {:.1}% of \
             the parked stripes' pixels within 8 levels of the live window's",
            stripes_box(&real_live, w),
            same_live * 100.0
        );
        report(&line);
        // Kept beside the failures' folders, though nothing failed.
        let dir = support::out().join("failures");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("parked-window-not-redrawn-{}.txt", std::process::id())), line);
    }
}

#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn new_window_hosts_an_owned_window_that_closes_with_its_pane_and_asks_if_unsaved() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-new", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = otto_for(&canary, "otto-new", &[]);
    let windows = || canary.call(json!({"cmd": "state"}))["windows"].as_array().map_or(0, Vec::len);
    let state = || otto.call(json!({"cmd": "state"}));
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys, "as": "user"}));
    // ⌘E, the canary: "New window" first (it has a "New Window" menu item), then its window.
    // New window presses that item.
    otto.call(json!({"cmd": "picker"}));
    otto.call(json!({"cmd": "picker_query", "query": CANARY}));
    let reply = otto.call(json!({"cmd": "picker_choose"}));
    assert_eq!(
        (reply["picker_rows"][0]["type"].as_str(), reply["picker_rows"][1]["type"].as_str()),
        (Some("new_window"), Some("window")),
        "{reply}"
    );
    let reply = key("enter");
    assert_eq!(reply["counts"]["new_windows_menu_item"].as_u64(), Some(1), "{reply}");
    let owned = || {
        let panes = state()["state"]["panes"].as_array().cloned().unwrap_or_default();
        panes.iter().find(|pane| pane["owned"] == true).and_then(|pane| pane["id"].as_u64())
    };
    let pane = wait_for("the new window's pane, owned", owned);
    assert_eq!((windows(), state()["state"]["new_window"].clone()), (2, Value::Null));
    // Its app says it has unsaved changes: the user's close asks first, and Esc keeps it.
    canary.call(json!({"cmd": "edited", "index": 1, "on": true}));
    wait_for("its unsaved changes read", || (state()["state"]["edited"] == json!([pane])).then_some(()));
    let reply = key("cmd-w");
    assert_eq!(reply["state"]["confirm"]["edited"].as_u64(), Some(pane), "{reply}");
    let reply = key("escape");
    assert_eq!((reply["state"]["confirm"].clone(), owned()), (Value::Null, Some(pane)));
    // Yes: the pane closes and the window with it. The canary's first window stays open.
    key("cmd-w");
    key("enter");
    wait_for("the owned window closed", || (windows() == 1).then_some(()));
    assert_eq!(owned(), None);
}

/// The owner's report: "when Zoom closed, I lost all my slots". Its windows were picked into its
/// slots with ⌘E, which ended each slot, so its quit closed the panes. A window of the slot's own
/// app picked into it fills it; the app quitting leaves it waiting for the app (absent).
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn a_window_picked_into_its_apps_slot_fills_it_and_the_apps_quit_leaves_it_waiting() {
    let mut canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-slot", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = otto_for(&canary, "otto-slot", &[("OTTOSPACE_FEATURES", "slots")]);
    let state = || otto.call(json!({"cmd": "state"}));
    let slot = || state()["state"]["slots"]["1"]["status"].clone();
    host_and_park(&otto);
    otto.call(json!({"cmd": "slot_make", "pane": 1, "slot": {"select": {}}}));
    assert_eq!(slot(), "filled");
    // Give Back: it waits.
    user_click_control(&otto, "slot-dots:1");
    // ⋯'s Give back the window (Edit slot…, Give back the window, Clear slot).
    user_click_control(&otto, "row-menu:1");
    assert_eq!(state()["state"]["panes"][0]["kind"]["type"], "slot");
    // ⌘E, the canary, its window: the user's pick fills the slot, which stays one.
    otto.call(json!({"cmd": "picker"}));
    otto.call(json!({"cmd": "picker_query", "query": CANARY}));
    let reply = otto.call(json!({"cmd": "picker_choose"}));
    let rows = reply["picker_rows"].as_array().cloned().unwrap_or_default();
    let at = rows.iter().position(|row| row["type"] == "window").unwrap_or_else(|| panic!("no window row: {reply}"));
    for _ in 0..at {
        otto.call(json!({"cmd": "key", "keys": "down", "as": "user"}));
    }
    otto.call(json!({"cmd": "key", "keys": "enter", "as": "user"}));
    wait_for("the slot filled by the pick", || (slot() == "filled").then_some(()));
    assert_eq!(state()["state"]["panes"].as_array().map(Vec::len), Some(1), "{}", state());
    // The canary quits: the pane stays, the slot waits for its app.
    canary.child.kill().expect("kill the canary");
    wait_for("the slot absent", || (slot() == "absent").then_some(()));
    assert_eq!(state()["state"]["panes"][0]["kind"]["type"], "slot", "{}", state());
}

/// Swap with a real window: the canary's hosted window in slot 1, a second slot of the canary
/// waiting beside it. The user's Swap moves the hosted window into slot 2 (parked there), and
/// slot 1 waits, saying its window is in another pane.
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn swap_moves_a_hosted_window_to_another_slot_of_its_app() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary-swap", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let otto = otto_for(&canary, "otto-swap", &[("OTTOSPACE_FEATURES", "slots")]);
    let state = || otto.call(json!({"cmd": "state"}));
    let spot = host_and_park(&otto);
    otto.call(json!({"cmd": "slot_make", "pane": 1, "slot": {"select": {}}}));
    // A second pane of the canary (no window of its own), made a slot and given back: it waits.
    let reply = otto.call(json!({"cmd": "app_fixture", "name": CANARY, "user": true}));
    let two = reply["state"]["focused"].as_u64().unwrap_or_else(|| panic!("no second pane: {reply}"));
    otto.call(json!({"cmd": "slot_make", "pane": two, "slot": {"select": {}}}));
    user_click_control(&otto, &format!("slot-dots:{two}"));
    let menu = state()["row_menu"]["items"].as_array().cloned().unwrap_or_default();
    let give = menu.iter().position(|i| i["item"] == "slot_give_back").unwrap_or_else(|| panic!("{menu:?}"));
    user_click_control(&otto, &format!("row-menu:{give}"));
    wait_for("slot 2 waits", || (state()["state"]["panes"][1]["kind"]["type"] == "slot").then_some(()));
    // The user's Swap on slot 1: its window goes to slot 2, parked there.
    user_click_control(&otto, "slot-dots:1");
    let menu = state()["row_menu"]["items"].as_array().cloned().unwrap_or_default();
    let swap = menu.iter().position(|i| i["item"] == "slot_swap").unwrap_or_else(|| panic!("no swap: {menu:?}"));
    user_click_control(&otto, &format!("row-menu:{swap}"));
    let moved = wait_for("the window hosted in slot 2", || {
        let now = state();
        (now["hosted"]["1"].is_null()).then(|| rect(&now["hosted"][two.to_string()])).flatten()
    });
    assert!(close(moved[0], spot[0]), "parked at the spot: {moved:?}");
    let now = state();
    assert_eq!(now["state"]["panes"][0]["kind"]["type"], "slot", "{now}");
    assert_eq!(now["state"]["slots"]["1"]["status"], "empty", "{now}");
}

/// Zoom-out (`zoom_out`): no real window floats over the tiles. A parked canary window stays parked
/// while zoomed out; one the user is in is parked as the overview opens, and is live over its
/// pane again, where it was, on the way back.
#[test]
#[ignore = "moves real windows of the canary app; run with `cargo e2e-apps`"]
fn zooming_out_parks_the_live_window_and_going_back_makes_it_live_again() {
    let canary = Proc::launch(env!("CARGO_BIN_EXE_ottospace-canary"), "canary", "CANARY_SOCKET", &[]);
    wait_for("the canary window", || rect(&canary.call(json!({"cmd": "state"}))["windows"][0]["bounds"]));
    let env = [("OTTOSPACE_FEATURES", "zoom_out"), ("OTTOSPACE_REDUCE_MOTION", "1")];
    let otto = otto_for(&canary, "zoom", &env);
    let state = || otto.call(json!({"cmd": "state"}));
    otto.call(json!({"cmd": "open_app", "name": CANARY}));
    wait_for("the pane layout", || rect(&state()["bounds"]["1"]).filter(|b| b[2] > 1.0));
    let spot = rect(&json!([state()["parking"][0], state()["parking"][1], 0, 0])).expect("parking spot");
    let hosted = || rect(&state()["hosted"]["1"]);
    let at_spot = |f: &[f64; 4]| close(f[0], spot[0]) && f[1] <= spot[1] && f[1] > spot[1] - 200.0;
    wait_for("the window parked", || hosted().filter(at_spot));
    // Parked: it stays parked while zoomed out, and its tile is drawn.
    let zoomed = otto.call(json!({"cmd": "key", "keys": "cmd-|", "as": "user"}));
    assert!(zoomed["state"]["zoom"].is_object(), "{}", zoomed["state"]["zoom"]);
    assert!(zoomed["zoom_tiles"].is_object(), "{}", zoomed["zoom_tiles"]);
    std::thread::sleep(Duration::from_millis(600));
    assert!(hosted().is_some_and(|f| at_spot(&f)), "still parked: {:?}", hosted());
    otto.call(json!({"cmd": "key", "keys": "escape", "as": "user"}));
    // Live: parked as it opens, live again on the way back, over its pane where it was.
    assert_eq!(otto.call(json!({"cmd": "enter", "pane": 1}))["mode"], "app");
    let live = wait_for("the window over its pane", || hosted().filter(|f| !at_spot(f)));
    let zoomed = otto.call(json!({"cmd": "key", "keys": "cmd-|", "as": "user"}));
    assert_eq!((&zoomed["mode"], &zoomed["state"]["live"]), (&json!("normal"), &Value::Null), "{}", zoomed["state"]);
    wait_for("the window to park as it zooms out", || hosted().filter(at_spot));
    let back = otto.call(json!({"cmd": "key", "keys": "cmd-|", "as": "user"}));
    assert_eq!((&back["mode"], &back["state"]["live"]), (&json!("app"), &json!(1)), "{}", back["state"]);
    wait_for("the window live where it was", || hosted().filter(|f| (0..4).all(|i| close(f[i], live[i]))));
    otto.call(json!({"cmd": "leave"}));
}
