//! Outer loop: launches the real app and drives it over the control socket. Run with `cargo e2e`.
//! Ignored by default because each test opens a window. `OTTOSPACE_E2E` turns window hosting off,
//! so tests never touch other apps' windows.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How long a wait may take. Generous: a loaded machine can take over 10s to start a shell.
const TIMEOUT: Duration = Duration::from_secs(40);

/// A running ottospace on a private socket. Killed on drop, so a failed assert leaves no window.
struct Otto {
    child: Child,
    socket: PathBuf,
}

impl Otto {
    fn launch(name: &str) -> Self {
        Self::launch_with(name, &[])
    }

    /// Launches with extra environment, e.g. `[("OTTOSPACE_REPOS", "/tmp/repos")]`.
    fn launch_with(name: &str, env: &[(&str, &str)]) -> Self {
        // A plain shell: the test must not depend on the user's shell profile.
        Self::launch_shell(name, &[&[("OTTOSPACE_SHELL", "/bin/sh")], env].concat())
    }

    /// Launches the way a user's ottospace starts terminals: no `OTTOSPACE_SHELL`, so each runs
    /// `/usr/bin/login -flp <user> /bin/zsh -fc "exec -a -<name> $SHELL"`. `$SHELL` is a script
    /// in `dir` (which the test made) that runs `/bin/sh` with an empty environment and `dir`
    /// as its home. Nothing of the user's is read: `zsh -f` skips their rc files, and the
    /// `/bin/sh` is not a login shell and has no `ENV`, so it reads no profile.
    fn launch_login(name: &str, dir: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let shell = dir.join("shell");
        let home = dir.to_str().expect("utf-8 path");
        let script =
            format!("#!/bin/sh\nexec /usr/bin/env -i PATH=/usr/bin:/bin TERM=xterm-256color HOME='{home}' /bin/sh\n");
        std::fs::write(&shell, script).expect("write the shell script");
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        Self::launch_shell(name, &[("SHELL", shell.to_str().expect("utf-8 path"))])
    }

    /// Launches with `env`, and without `OTTOSPACE_SHELL` unless `env` names it.
    fn launch_shell(name: &str, env: &[(&str, &str)]) -> Self {
        let socket = std::env::temp_dir().join(format!("ottospace-e2e-{name}-{}.sock", std::process::id()));
        let child = Command::new(env!("CARGO_BIN_EXE_ottospace"))
            .env("OTTOSPACE_E2E", "1")
            .env("OTTOSPACE_SOCKET", &socket)
            .env_remove("OTTOSPACE_SHELL")
            // Never inherit hosting from the caller's shell: this suite must not move real windows.
            .env_remove("OTTOSPACE_HOST_ONLY")
            // Never list the user's own repositories unless a test names a directory, here or in
            // the settings of a folder it made.
            .env_remove("OTTOSPACE_REPOS")
            .envs((!env.iter().any(|(name, _)| *name == "OTTOSPACE_HOME")).then_some(("OTTOSPACE_REPOS", "")))
            // Never read the user's settings or tasks: `OTTOSPACE_E2E` without a folder keeps nothing.
            .env_remove("OTTOSPACE_HOME")
            .envs(env.iter().copied())
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
        stream.set_read_timeout(Some(TIMEOUT)).expect("set timeout");
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

    /// Polls `check` until it returns a value; panics after `TIMEOUT`.
    fn wait_for<T>(&self, what: &str, check: impl Fn(&Self) -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(value) = check(self) {
                return value;
            }
            assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Drawn bounds `[x, y, w, h]` of every pane in `ids`, once all are laid out.
    /// On timeout the panic shows the last state, so a failure explains itself.
    fn bounds(&self, ids: &[u64]) -> Vec<[f64; 4]> {
        let start = Instant::now();
        loop {
            let state = self.call(json!({"cmd": "state"}));
            let found: Option<Vec<[f64; 4]>> = ids
                .iter()
                .map(|id| {
                    let b = state["bounds"][id.to_string()].as_array()?;
                    let b: Vec<f64> = b.iter().filter_map(Value::as_f64).collect();
                    (b.len() == 4 && b[2] > 1.0 && b[3] > 1.0).then(|| [b[0], b[1], b[2], b[3]])
                })
                .collect();
            if let Some(found) = found {
                return found;
            }
            assert!(start.elapsed() < TIMEOUT, "no layout for panes {ids:?}; last state: {state}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Otto {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// A made-up repository at `at`: a folder whose `.git` names the branch `main`.
fn fake_repo(at: &Path) {
    std::fs::create_dir_all(at.join(".git")).unwrap();
    std::fs::write(at.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
}

/// Waits for the app's process to end.
fn exits(otto: &mut Otto) {
    let start = Instant::now();
    while otto.child.try_wait().expect("wait").is_none() {
        assert!(start.elapsed() < Duration::from_secs(5), "quit did not exit the app");
        std::thread::sleep(Duration::from_millis(50));
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
    assert_eq!(otto.call(json!({"cmd": "focus_cycle", "delta": 1}))["state"]["focused"], 1); // wraps
    assert_eq!(otto.call(json!({"cmd": "focus_cycle", "delta": -1}))["state"]["focused"], 3);

    // Closing 2 lets 3 take the whole right column.
    let before = otto.bounds(&[3])[0];
    assert_eq!(pane_ids(&otto.call(json!({"cmd": "close", "pane": 2}))), [1, 3]);
    let after = otto.wait_for("pane 3 to grow", |o| Some(o.bounds(&[3])[0]).filter(|b| b[3] > before[3] * 1.8));
    assert!((after[0] - before[0]).abs() < 2.0, "3 stays in the right column: {before:?} {after:?}");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn panes_resize_swap_and_change_tab_from_the_keyboard() {
    let otto = Otto::launch("arrange");
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    let [one, two] = otto.bounds(&[1, 2])[..] else { panic!("two panes") };
    let total = one[2] + two[2];

    // ⌘⌃← moves the split left: 1 gives 5% of the row to 2. The socket request does the same.
    let reply = otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-left"}));
    assert_eq!(reply["state"]["layout"]["split"]["ratio"], 0.45, "{reply}");
    otto.call(json!({"cmd": "resize", "dir": "left"}));
    let narrow = otto.wait_for("pane 1 to shrink", |o| Some(o.bounds(&[1])[0]).filter(|b| b[2] < one[2] - 10.0));
    assert!((narrow[2] / total - 0.4).abs() < 0.02, "1 has 40% of the row: {narrow:?} of {total}");

    // ⌘⌃= makes the halves equal again.
    otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-="}));
    otto.wait_for("equal halves", |o| Some(o.bounds(&[1])[0]).filter(|b| (b[2] - one[2]).abs() < 2.0));

    // ⌘⌃0 and `fit` are for an app pane whose window refused its size: these panes stay as they are.
    let reply = otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-0"}));
    assert_eq!(reply["state"]["layout"]["split"]["ratio"], 0.5, "{reply}");
    // The socket says why: an empty pane has no window to fit, and a pane must exist.
    let refused = otto.raw(&json!({"cmd": "fit", "pane": 1}).to_string());
    assert!(
        refused["error"].as_str().is_some_and(|e| e.contains("pane 1 is empty: fit is for an app pane")),
        "{refused}"
    );
    let refused = otto.raw(&json!({"cmd": "fit", "pane": 9}).to_string());
    assert_eq!(refused["error"], "no pane 9", "{refused}");
    assert_eq!(otto.call(json!({"cmd": "state"}))["state"]["layout"]["split"]["ratio"], 0.5);
    // The header's × stays inside its pane.
    let state = otto.call(json!({"cmd": "state"}));
    for id in ["1", "2"] {
        let (close, frame) = (&state["closes"][id], &state["frames"][id]);
        let right = |r: &Value| r[0].as_f64().unwrap_or(0.0) + r[2].as_f64().unwrap_or(-1.0);
        assert!(close[2].as_f64() > Some(1.0) && right(close) <= right(frame), "× {close} in pane {frame}");
    }

    // ⌘⇧⌥H swaps 2 with its left neighbour; 2 keeps focus.
    let reply = otto.call(json!({"cmd": "key", "keys": "cmd-shift-alt-h"}));
    assert_eq!(reply["state"]["focused"], 2);
    assert_eq!(reply["tab_panes"], json!([[2, 1]]));
    otto.wait_for("2 on the left", |o| Some(o.bounds(&[1, 2])).filter(|b| b[1][0] < b[0][0]));
    assert_eq!(otto.call(json!({"cmd": "swap", "dir": "right"}))["tab_panes"], json!([[1, 2]]));

    // ⌘⇧⌥2 moves the focused pane to the second tab, which comes forward.
    otto.call(json!({"cmd": "new_tab"}));
    otto.call(json!({"cmd": "select_tab", "index": 0}));
    let reply = otto.call(json!({"cmd": "key", "keys": "cmd-shift-alt-2"}));
    assert_eq!(reply["tab_panes"], json!([[1], [2]]));
    assert_eq!((reply["state"]["active"].as_u64(), reply["state"]["focused"].as_u64()), (Some(1), Some(2)));
    assert_eq!(otto.call(json!({"cmd": "key", "keys": "cmd-shift-alt-1"}))["tab_panes"], json!([[1, 2], []]));
    let reply = otto.call(json!({"cmd": "move_pane", "pane": 1, "tab": 1}));
    assert_eq!(reply["tab_panes"], json!([[2], [1]]));
    let full = otto.bounds(&[1])[0];
    assert!(full[2] >= total, "1 fills the other tab: {full:?}");
    assert_eq!(otto.raw(r#"{"cmd":"move_pane","tab":5}"#)["ok"], false);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_dropped_pane_splits_or_swaps_where_it_lands() {
    let otto = Otto::launch("drop");
    // 1 | 2 | 3
    for _ in 0..3 {
        otto.call(json!({"cmd": "split", "kind": "empty"}));
    }
    let wide = otto.bounds(&[1])[0];

    // 3 on the top of 1: 3 is drawn over 1, in 1's old column, and has focus.
    let reply = otto.call(json!({"cmd": "drop_pane", "source": 3, "target": 1, "zone": "top"}));
    assert_eq!((reply["tab_panes"].clone(), reply["state"]["focused"].as_u64()), (json!([[3, 1, 2]]), Some(3)));
    let [one, three] = otto.wait_for("3 over 1", |o| {
        let b = o.bounds(&[1, 3]);
        (b[1][1] + b[1][3] <= b[0][1]).then_some([b[0], b[1]])
    });
    assert_eq!((one[0], three[0]), (wide[0], wide[0]), "both in the left column: {one:?} {three:?}");

    // The centre swaps: 2 takes 3's place at the top left.
    let reply = otto.call(json!({"cmd": "drop_pane", "source": 2, "target": 3, "zone": "center"}));
    assert_eq!((reply["tab_panes"].clone(), reply["state"]["focused"].as_u64()), (json!([[2, 1, 3]]), Some(2)));
    otto.wait_for("2 at the top left", |o| Some(o.bounds(&[2])[0]).filter(|b| b[0] == three[0] && b[1] == three[1]));

    // A pane of a background tab lands in the tab on show.
    otto.call(json!({"cmd": "new_tab"}));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    let reply = otto.call(json!({"cmd": "drop_pane", "source": 1, "target": 4, "zone": "left"}));
    assert_eq!(reply["tab_panes"], json!([[2, 3], [1, 4]]));
    otto.wait_for("1 left of 4", |o| Some(o.bounds(&[1, 4])).filter(|b| b[0][0] + b[0][2] <= b[1][0]));

    for bad in [
        r#"{"cmd":"drop_pane","source":1,"target":1,"zone":"left"}"#,
        r#"{"cmd":"drop_pane","source":1,"target":9,"zone":"left"}"#,
        r#"{"cmd":"drop_pane","source":1,"target":4,"zone":"middle"}"#,
    ] {
        assert_eq!(otto.raw(bad)["ok"], false, "{bad}");
    }
}

/// A rect `[x, y, w, h]` of a reply: `rect(&reply["headers"]["2"])`.
fn rect(value: &Value) -> [f64; 4] {
    let r: Vec<f64> =
        value.as_array().unwrap_or_else(|| panic!("no rect: {value}")).iter().filter_map(Value::as_f64).collect();
    [r[0], r[1], r[2], r[3]]
}

/// The point at fractions `fx`, `fy` of a rect.
fn at(r: [f64; 4], fx: f64, fy: f64) -> [f64; 2] {
    [r[0] + r[2] * fx, r[1] + r[3] * fy]
}

impl Otto {
    /// One pointer event through the window's own handlers. The reply is drawn state.
    fn mouse(&self, action: &str, p: [f64; 2]) -> Value {
        self.call(json!({"cmd": "mouse", "action": action, "x": p[0], "y": p[1]}))
    }

    /// The state after a fresh frame: a pointer move outside the window draws one first.
    fn drawn(&self) -> Value {
        self.mouse("move", [-50.0, -50.0])
    }

    /// A whole drag: press, eight moves, release.
    fn drag(&self, from: [f64; 2], to: [f64; 2]) -> Value {
        self.call(json!({"cmd": "drag", "from": from, "to": to}))
    }

    /// Drags pane `source` by the middle of its header to fractions `fx`, `fy` of pane `target`.
    fn drag_pane(&self, source: u64, target: u64, fx: f64, fy: f64) -> Value {
        let state = self.drawn();
        let from = at(rect(&state["headers"][source.to_string()]), 0.5, 0.5);
        self.drag(from, at(rect(&state["frames"][target.to_string()]), fx, fy))
    }

    /// A press and a release at `p`.
    fn click(&self, p: [f64; 2], clicks: u32) -> Value {
        self.call(json!({"cmd": "mouse", "action": "down", "x": p[0], "y": p[1], "clicks": clicks}));
        self.call(json!({"cmd": "mouse", "action": "up", "x": p[0], "y": p[1], "clicks": clicks}))
    }
}

fn leaf(id: u64) -> Value {
    json!({"leaf": id})
}

fn split(axis: &str, ratio: &Value, a: Value, b: Value) -> Value {
    json!({"split": {"axis": axis, "ratio": ratio, "a": a, "b": b}})
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_header_dragged_with_the_mouse_splits_or_swaps_where_it_is_released() {
    let otto = Otto::launch("drag-zones");
    // 1 | 2 | 3, the outer split off centre, so a ratio that is kept can be told from a new one.
    for _ in 0..3 {
        otto.call(json!({"cmd": "split", "kind": "empty"}));
    }
    otto.call(json!({"cmd": "focus", "pane": 1}));
    let reply = otto.call(json!({"cmd": "resize", "dir": "right"}));
    let outer = reply["state"]["layout"]["split"]["ratio"].clone();
    let half = json!(0.5);
    assert_ne!(outer, half);
    assert_eq!(reply["drag"], Value::Null);
    let layout = |reply: &Value| reply["state"]["layout"].clone();

    // The left edge of 1. Step by step, to see the drop zone on the way.
    let state = otto.drawn();
    let (from, to) = (at(rect(&state["headers"]["3"]), 0.5, 0.5), at(rect(&state["frames"]["1"]), 0.1, 0.5));
    assert_eq!(otto.mouse("down", from)["drag"], Value::Null, "a press alone is no drag");
    let reply = otto.mouse("move", [from[0] - 1.0, from[1]]);
    assert_eq!(reply["drag"], Value::Null, "nor is a move of 1 pt");
    // Over itself there is nowhere to drop.
    let reply = otto.mouse("move", [from[0] - 10.0, from[1]]);
    assert_eq!(reply["drag"], json!({"target": null, "zone": null}), "{reply}");
    let reply = otto.mouse("move", at(rect(&state["headers"]["2"]), 0.5, 0.5));
    assert_eq!(reply["drag"], json!({"target": 2, "zone": "top"}), "over 2's header: {reply}");
    assert_eq!(otto.mouse("move", to)["drag"], json!({"target": 1, "zone": "left"}));
    let reply = otto.mouse("move", at(rect(&state["frames"]["1"]), 0.5, 0.5));
    assert_eq!(reply["drag"], json!({"target": 1, "zone": "center"}));
    let reply = otto.mouse("move", at(rect(&state["frames"]["3"]), 0.5, 0.5));
    assert_eq!(reply["drag"], json!({"target": null, "zone": null}));
    otto.mouse("move", to);
    let reply = otto.mouse("up", to);
    assert_eq!(reply["drag"], Value::Null);
    let inner = |a, b| split("right", &half, leaf(a), leaf(b));
    assert_eq!(layout(&reply), split("right", &outer, inner(3, 1), leaf(2)));
    assert_eq!((reply["state"]["focused"].as_u64(), reply["tab_panes"].clone()), (Some(3), json!([[3, 1, 2]])));
    // Drawn there too: 3, 1, 2 from the left.
    let b = otto.bounds(&[3, 1, 2]);
    assert!(b[0][0] < b[1][0] && b[1][0] < b[2][0], "{b:?}");

    // Where it already is: nothing moves.
    assert_eq!(layout(&otto.drag_pane(3, 1, 0.1, 0.5)), split("right", &outer, inner(3, 1), leaf(2)));

    // The right, top and bottom edges.
    assert_eq!(layout(&otto.drag_pane(3, 1, 0.9, 0.5)), split("right", &outer, inner(1, 3), leaf(2)));
    let over = |a, b| split("down", &half, leaf(a), leaf(b));
    assert_eq!(layout(&otto.drag_pane(3, 2, 0.5, 0.1)), split("right", &outer, leaf(1), over(3, 2)));
    let reply = otto.drag_pane(3, 1, 0.5, 0.9);
    assert_eq!(layout(&reply), split("right", &outer, over(1, 3), leaf(2)));
    assert_eq!(reply["state"]["focused"], 3);

    // The centre swaps the two.
    let reply = otto.drag_pane(2, 3, 0.5, 0.5);
    assert_eq!(layout(&reply), split("right", &outer, over(1, 2), leaf(3)));
    assert_eq!((reply["state"]["focused"].as_u64(), reply["drag"].clone()), (Some(2), Value::Null));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_drag_reports_its_drop_zone_from_the_move_that_starts_it() {
    let otto = Otto::launch("drag-first-move");
    for _ in 0..3 {
        otto.call(json!({"cmd": "split", "kind": "empty"}));
    }
    // 1 | 2 | 3. One move from 3's header to the centre of 1 starts the drag and is over 1.
    let state = otto.drawn();
    let to = at(rect(&state["frames"]["1"]), 0.5, 0.5);
    otto.mouse("down", at(rect(&state["headers"]["3"]), 0.5, 0.5));
    assert_eq!(otto.mouse("move", to)["drag"], json!({"target": 1, "zone": "center"}));
    assert_eq!(otto.mouse("up", to)["tab_panes"], json!([[3, 2, 1]]));
    // So does a whole drag of one move.
    let state = otto.drawn();
    let from = at(rect(&state["headers"]["1"]), 0.5, 0.5);
    let reply = otto.call(json!({"cmd": "drag", "from": from, "to": to, "steps": 1}));
    assert_eq!(reply["tab_panes"], json!([[1, 2, 3]]));

    // The zone of the last drop is not the next drag's: 3 pt along its own header, 2 is over nothing.
    let from = at(rect(&otto.drawn()["headers"]["2"]), 0.5, 0.5);
    otto.mouse("down", from);
    let reply = otto.mouse("move", [from[0] + 3.0, from[1]]);
    assert_eq!(reply["drag"], json!({"target": null, "zone": null}));
    otto.mouse("up", [from[0] + 3.0, from[1]]);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_header_dragged_onto_a_tab_moves_the_pane_there() {
    let otto = Otto::launch("drag-tabs");
    // Tab 0: 1 | 2. Tab 1: 3.
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "new_tab"}));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "select_tab", "index": 0}));
    let to_tab = |pane: u64, tab: usize| {
        let state = otto.drawn();
        assert_eq!(state["tabs"].as_array().map(Vec::len), Some(2), "{}", state["tabs"]);
        otto.drag(at(rect(&state["headers"][pane.to_string()]), 0.5, 0.5), at(rect(&state["tabs"][tab]), 0.5, 0.5))
    };
    let sorted = |reply: &Value| -> Vec<Vec<u64>> {
        let tabs = reply["tab_panes"].as_array().expect("tab_panes").iter();
        tabs.map(|t| {
            let mut ids: Vec<u64> = t.as_array().expect("tab").iter().filter_map(Value::as_u64).collect();
            ids.sort_unstable();
            ids
        })
        .collect()
    };

    // Its own tab: nothing moves.
    let before = otto.call(json!({"cmd": "state"}));
    let reply = to_tab(2, 0);
    assert_eq!((&reply["state"]["layout"], &reply["tab_panes"]), (&before["state"]["layout"], &before["tab_panes"]));
    assert_eq!(reply["drag"], Value::Null);

    // Another tab: the pane goes there, and that tab comes forward, as with ⌘⇧⌥1..9.
    let reply = to_tab(2, 1);
    assert_eq!(sorted(&reply), [vec![1], vec![2, 3]]);
    assert_eq!((reply["state"]["active"].as_u64(), reply["state"]["focused"].as_u64()), (Some(1), Some(2)));
    assert_eq!(reply["drag"], Value::Null);
    otto.wait_for("2 and 3 drawn", |o| Some(o.call(json!({"cmd": "state"}))).filter(|s| s["headers"]["2"].is_array()));

    // The only pane of a tab: the tab stays, empty, and shows the home screen.
    otto.call(json!({"cmd": "select_tab", "index": 0}));
    let reply = to_tab(1, 1);
    assert_eq!(sorted(&reply), [vec![], vec![1, 2, 3]]);
    assert_eq!((reply["state"]["active"].as_u64(), reply["drag"].clone()), (Some(1), Value::Null));
    let reply = otto.call(json!({"cmd": "select_tab", "index": 0}));
    assert_eq!(reply["state"]["layout"], Value::Null);
    assert_ne!(reply["home"], Value::Null, "the home screen");
    assert_eq!(otto.drawn()["headers"], json!({}));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_pane_drag_that_ends_nowhere_changes_nothing() {
    let otto = Otto::launch("drag-cancel");
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    let before = otto.drawn();
    let same = |what: &str, reply: &Value| {
        assert_eq!(reply["drag"], Value::Null, "{what}: no drop zone is left");
        assert_eq!(reply["state"]["layout"], before["state"]["layout"], "{what}");
        assert_eq!(reply["tab_panes"], before["tab_panes"], "{what}");
    };
    let (one, two) = (rect(&before["frames"]["1"]), rect(&before["frames"]["2"]));
    let from = at(rect(&before["headers"]["2"]), 0.5, 0.5);

    // Esc, then the release.
    otto.mouse("down", from);
    otto.mouse("move", at(two, 0.2, 0.3));
    assert_eq!(otto.mouse("move", at(one, 0.5, 0.9))["drag"], json!({"target": 1, "zone": "bottom"}));
    same("esc", &otto.call(json!({"cmd": "key", "keys": "escape"})));
    same("a move after esc", &otto.mouse("move", at(one, 0.5, 0.1)));
    same("the release after esc", &otto.mouse("up", at(one, 0.5, 0.1)));

    // The sidebar, left of the panes.
    assert!(one[0] > 40.0, "a sidebar left of pane 1: {one:?}");
    same("the sidebar", &otto.drag(from, [one[0] / 2.0, one[1] + one[3] / 2.0]));
    // The gap between the two panes.
    let gap = (one[0] + one[2] + two[0]) / 2.0;
    assert!(one[0] + one[2] < gap && gap < two[0], "a gap: {one:?} {two:?}");
    same("the gap", &otto.drag(from, [gap, one[1] + one[3] / 2.0]));
    // Outside the window.
    let window = rect(&before["window"]);
    same("outside", &otto.drag(from, [window[2] + 60.0, window[3] + 60.0]));
    same("above", &otto.drag(from, [one[0] + 20.0, -40.0]));
    // A press nobody releases: after 5 s the app lets go, outside the window.
    otto.mouse("down", from);
    assert_eq!(otto.mouse("move", at(one, 0.5, 0.9))["drag"], json!({"target": 1, "zone": "bottom"}));
    std::thread::sleep(Duration::from_millis(5500));
    same("an abandoned press", &otto.call(json!({"cmd": "state"})));
    same("a move after it", &otto.mouse("move", at(one, 0.5, 0.1)));

    // The pointer is free again: a click focuses, a new drag drops.
    assert_eq!(otto.click(at(rect(&before["bounds"]["1"]), 0.5, 0.5), 1)["state"]["focused"], 1);
    assert_eq!(otto.drag_pane(2, 1, 0.1, 0.5)["tab_panes"], json!([[2, 1]]));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_click_on_a_header_or_its_close_button_is_not_a_drag() {
    let otto = Otto::launch("drag-clicks");
    for _ in 0..3 {
        otto.call(json!({"cmd": "split", "kind": "empty"}));
    }
    let before = otto.drawn();
    assert_eq!(before["state"]["focused"], 3);
    let unmoved = |what: &str, reply: &Value| {
        assert_eq!(reply["drag"], Value::Null, "{what}");
        assert_eq!(reply["state"]["layout"], before["state"]["layout"], "{what}");
    };
    let header = |id: u64| at(rect(&before["headers"][id.to_string()]), 0.3, 0.5);

    // A click focuses.
    let reply = otto.click(header(1), 1);
    unmoved("a click", &reply);
    assert_eq!(reply["state"]["focused"], 1);
    // So does a double-click.
    otto.click(header(2), 1);
    let reply = otto.click(header(2), 2);
    unmoved("a double-click", &reply);
    assert_eq!(reply["state"]["focused"], 2);
    // A double-click on the tab bar zooms or minimises the user's window; an injected one does not.
    let tab = at(rect(&before["tabs"][0]), 0.5, 0.5);
    otto.click(tab, 1);
    otto.click(tab, 2);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(otto.call(json!({"cmd": "state"}))["window"], before["window"], "the window is as it was");
    // A press that moves 2 pt, GPUI's limit, is still a click.
    let p = header(3);
    otto.mouse("down", p);
    unmoved("2 pt", &otto.mouse("move", [p[0] + 2.0, p[1]]));
    let reply = otto.mouse("up", [p[0] + 2.0, p[1]]);
    unmoved("2 pt, released", &reply);
    assert_eq!(reply["state"]["focused"], 3);

    // A press on × that moves 5 pt inside it closes the pane; no drag starts.
    let close = rect(&before["closes"]["2"]);
    assert!(close[2] >= 8.0 && close[3] >= 4.0, "×: {close:?}");
    let (down, up) = ([close[0] + 1.5, close[1] + close[3] / 2.0], [close[0] + 6.5, close[1] + close[3] / 2.0]);
    otto.mouse("down", down);
    assert_eq!(otto.mouse("move", up)["drag"], Value::Null);
    let reply = otto.mouse("up", up);
    assert_eq!((reply["drag"].clone(), reply["tab_panes"].clone()), (Value::Null, json!([[1, 3]])));

    // A press on × that leaves it, over another pane: no close, no drag, no drop.
    let state = otto.drawn();
    let close = at(rect(&state["closes"]["3"]), 0.5, 0.5);
    let far = at(rect(&state["frames"]["1"]), 0.5, 0.5);
    otto.mouse("down", close);
    assert_eq!(otto.mouse("move", far)["drag"], Value::Null);
    let reply = otto.mouse("up", far);
    assert_eq!((reply["drag"].clone(), reply["tab_panes"].clone()), (Value::Null, json!([[1, 3]])));
    assert_eq!(reply["state"]["layout"], state["state"]["layout"]);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_selection_drag_across_a_header_moves_no_pane() {
    let otto = Otto::launch("drag-select");
    // 1 over 2, both terminals.
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "split", "axis": "down"}));
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo alpha beta\r"}));
    otto.wait_for("the output", |o| Some(term1(o)).filter(|t| shows(t, "alpha beta")));
    let before = otto.drawn();
    let (cells, header, below) =
        (rect(&before["bounds"]["1"]), rect(&before["headers"]["2"]), rect(&before["bounds"]["2"]));
    assert!(cells[1] < header[1] && header[1] < below[1], "1 over 2: {before}");

    // From the top-left cell of 1, down over 2's header, into 2.
    otto.mouse("down", [cells[0] + 2.0, cells[1] + 2.0]);
    for p in [at(cells, 0.5, 0.5), at(header, 0.5, 0.5), at(header, 0.2, 0.5), at(below, 0.5, 0.5)] {
        let reply = otto.mouse("move", p);
        assert_eq!(reply["drag"], Value::Null, "at {p:?}");
        assert_eq!(reply["state"]["layout"], before["state"]["layout"]);
    }
    let reply = otto.mouse("up", at(below, 0.5, 0.5));
    assert_eq!((reply["drag"].clone(), reply["tab_panes"].clone()), (Value::Null, json!([[1, 2]])));
    assert_eq!(reply["state"]["layout"], before["state"]["layout"]);
    let selected = reply["terminals"]["1"]["selection"].as_str().unwrap_or_default();
    assert!(selected.contains("alpha beta"), "the drag selected text in 1: {selected:?}");
    assert_eq!(reply["terminals"]["2"]["selection"], Value::Null);
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
fn capture_and_wait_read_one_pane() {
    let otto = Otto::launch("capture");
    otto.call(json!({"cmd": "split"}));

    // The shell knows its pane and tab. `^` skips the echoed command, which starts with the prompt.
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo pane-$OTTOSPACE_PANE-tab-$OTTOSPACE_TAB\r"}));
    let tab = otto.call(json!({"cmd": "state"}))["state"]["tabs"][0]["id"].as_u64().unwrap();
    let first = format!("pane-1-tab-{tab}");
    let found = otto.call(json!({"cmd": "wait", "pane": 1, "pattern": format!("^{first}$"), "timeout_ms": 40000}));
    assert_eq!(found["line"], first);
    assert!(found.get("state").is_none(), "wait replies with the line, not the state");

    // Output that left the screen is in the capture only with `history`.
    otto.call(json!({"cmd": "type", "pane": 1, "text": "seq 1 200; echo end-$((6*7))\r"}));
    otto.call(json!({"cmd": "wait", "pane": 1, "pattern": "^end-42$", "timeout_ms": 40000}));
    let text = |history: bool| {
        let reply = otto.call(json!({"cmd": "capture", "pane": 1, "history": history}));
        reply["text"].as_str().unwrap().to_string()
    };
    assert!(text(true).lines().any(|l| *l == first), "history holds the first command's output");
    assert!(!text(false).lines().any(|l| *l == first), "the screen alone does not");
    assert!(text(false).lines().any(|l| l == "end-42"));

    // No match: an error at the time limit. A bad pattern or a pane that is no terminal: refused.
    let late = otto.raw(r#"{"cmd":"wait","pane":1,"pattern":"^never$","timeout_ms":200}"#);
    assert_eq!(late["ok"], false);
    assert!(late["error"].as_str().unwrap().contains("200 ms"), "{late}");
    assert_eq!(otto.raw(r#"{"cmd":"wait","pane":1,"pattern":"("}"#)["ok"], false);
    assert_eq!(otto.raw(r#"{"cmd":"capture","pane":9}"#)["ok"], false);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn repos_are_listed_and_open_a_terminal_there() {
    // A repository with one linked worktree, written the way git lays them out, and a plain directory.
    let root = std::env::temp_dir().join(format!("ottospace-e2e-repos-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    fake_repo(&root.join("app"));
    let admin = root.join("app/.git/worktrees/app-2");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(root.join("app-2")).unwrap();
    std::fs::create_dir_all(root.join("plain")).unwrap();
    std::fs::create_dir_all(root.join("app/.git/worktrees/app-3")).unwrap();
    std::fs::create_dir_all(root.join("app-3")).unwrap();
    // As given: on macOS the temp dir is behind a symlink, so this is not the resolved path.
    let unresolved = root.to_string_lossy().into_owned();
    let root = root.canonicalize().unwrap(); // the path `pwd` prints
    let at = |rest: &str| root.join(rest).to_string_lossy().into_owned();
    std::fs::write(root.join("app/.git/worktrees/app-2/HEAD"), "ref: refs/heads/fix/zoom\n").unwrap();
    std::fs::write(root.join("app/.git/worktrees/app-2/gitdir"), format!("{}\n", at("app-2/.git"))).unwrap();
    std::fs::write(root.join("app-2/.git"), format!("gitdir: {}\n", at("app/.git/worktrees/app-2"))).unwrap();

    // A second worktree linked with relative paths (git's `worktree.useRelativePaths`).
    std::fs::write(root.join("app/.git/worktrees/app-3/HEAD"), "ref: refs/heads/feat/map\n").unwrap();
    std::fs::write(root.join("app/.git/worktrees/app-3/gitdir"), "../../../../app-3/.git\n").unwrap();
    std::fs::write(root.join("app-3/.git"), "gitdir: ../app/.git/worktrees/app-3\n").unwrap();

    let otto = Otto::launch_with("repos", &[("OTTOSPACE_REPOS", &unresolved)]);
    let repos = otto.call(json!({"cmd": "repos"}))["state"]["repos"].clone();
    // Each worktree is under its repository, not listed a second time, however it is linked.
    let worktrees = json!([
        {"name": "app-2", "path": at("app-2"), "branch": "fix/zoom"},
        {"name": "app-3", "path": at("app-3"), "branch": "feat/map"},
    ]);
    assert_eq!(repos, json!([{"name": "app", "path": at("app"), "branch": "main", "worktrees": worktrees}]));

    // Opening the worktree: a terminal whose shell starts there, and the empty tab takes its name.
    let reply = otto.call(json!({"cmd": "open_repo", "path": at("app-2")}));
    assert_eq!(reply["state"]["tabs"][0]["title"], "app-2");
    assert_eq!(reply["state"]["panes"][0]["cwd"], at("app-2"));
    otto.call(json!({"cmd": "type", "pane": 1, "text": "pwd\r"}));
    let pattern = format!("^{}$", regex::escape(&at("app-2")));
    otto.call(json!({"cmd": "wait", "pane": 1, "pattern": pattern, "timeout_ms": 40000}));

    // A path that is not in the list is refused: `open_repo` is not "open any directory".
    assert_eq!(otto.raw(&json!({"cmd": "open_repo", "path": at("plain")}).to_string())["ok"], false);
    // So is a listed directory that has gone since the list was read.
    std::fs::remove_dir_all(root.join("app-3")).unwrap();
    let gone = otto.raw(&json!({"cmd": "open_repo", "path": at("app-3")}).to_string());
    assert!(gone["error"].as_str().is_some_and(|e| e.contains("not a directory")), "{gone}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn tasks_are_listed_as_an_inbox_and_pop_opens_the_next() {
    let otto = Otto::launch("tasks");
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "new_tab"}));
    let tabs = otto.call(json!({"cmd": "state"}))["state"]["tabs"].clone();
    let (first, second) = (tabs[0]["id"].clone(), tabs[1]["id"].clone());

    assert_eq!(otto.call(json!({"cmd": "task_add", "title": "idea"}))["task"]["id"], 1);
    otto.call(json!({"cmd": "task_add", "title": "build", "state": "running", "tab": second}));
    otto.call(json!({"cmd": "task_add", "title": "question", "priority": 1, "state": "needs_you", "tab": first}));
    let inbox = |o: &Otto| -> Vec<(String, String)> {
        let rows = o.call(json!({"cmd": "task_list"}))["inbox"].as_array().unwrap().clone();
        rows.iter().map(|r| (r["group"].as_str().unwrap().into(), r["title"].as_str().unwrap().into())).collect()
    };
    let row = |group: &str, title: &str| (group.to_string(), title.to_string());
    assert_eq!(inbox(&otto), [row("needs_you", "question"), row("running", "build"), row("captured", "idea")]);
    assert_eq!(otto.call(json!({"cmd": "state"}))["needs_you"], 1);

    // Pop next: the tab of the task that needs the user comes forward.
    assert_eq!(otto.call(json!({"cmd": "state"}))["state"]["active"], 1);
    let popped = otto.call(json!({"cmd": "task_pop"}));
    assert_eq!((&popped["task"]["title"], &popped["opened"]), (&json!("question"), &json!(true)));
    assert_eq!(otto.call(json!({"cmd": "state"}))["state"]["active"], 0);

    // Snoozed: out of the inbox, and pop finds nothing. Undo brings it back.
    otto.call(json!({"cmd": "task_snooze", "id": 3, "seconds": 3600}));
    assert_eq!(inbox(&otto), [row("running", "build"), row("captured", "idea")]);
    let none = otto.call(json!({"cmd": "task_pop"}));
    assert!(none["task"].is_null() && none["opened"] == false, "{none}");
    otto.call(json!({"cmd": "task_undo"}));
    assert_eq!(inbox(&otto)[0], row("needs_you", "question"));

    // A snooze that has run out returns by itself.
    otto.call(json!({"cmd": "task_snooze", "id": 3, "seconds": 0}));
    otto.wait_for("the snoozed task to return", |o| {
        (o.call(json!({"cmd": "state"}))["state"]["tasks"]["list"][2]["snoozed_until"].is_null()).then_some(())
    });

    // Hold and dismiss; a state report from the agent moves the task.
    otto.call(json!({"cmd": "task_hold", "id": 3}));
    otto.call(json!({"cmd": "task_dismiss", "id": 1}));
    otto.call(json!({"cmd": "task_set_state", "id": 2, "state": "needs_you"}));
    assert_eq!(inbox(&otto), [row("needs_you", "build"), row("on_hold", "question")]);
    // A dismissed task that starts to need the user comes back. It had no tab: opening it gives it one.
    otto.call(json!({"cmd": "task_set_state", "id": 2, "state": "done"}));
    otto.call(json!({"cmd": "task_set_state", "id": 1, "state": "needs_you"}));
    let popped = otto.call(json!({"cmd": "task_pop"}));
    assert_eq!((&popped["task"]["title"], &popped["opened"]), (&json!("idea"), &json!(true)));
    assert_eq!(otto.raw(r#"{"cmd":"task_hold","id":9}"#)["ok"], false, "no such task");
    assert_eq!(otto.raw(r#"{"cmd":"task_add","title":"x","state":"sleeping"}"#)["ok"], false, "no such state");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_tasks_section_lists_what_needs_you_then_what_is_open() {
    let otto = Otto::launch("tasks-bar");
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    // The section as drawn: (heading, count, ids of its rows).
    let bar = |reply: &Value| -> Vec<(String, u64, Vec<u64>)> {
        let sections = reply["tasks_bar"]["sections"].as_array().unwrap();
        let ids = |s: &Value| s["rows"].as_array().unwrap().iter().map(|r| r["id"].as_u64().unwrap()).collect();
        sections
            .iter()
            .map(|s| (s["heading"].as_str().unwrap().to_string(), s["count"].as_u64().unwrap(), ids(s)))
            .collect()
    };
    let section = |heading: &str, ids: &[u64]| (heading.to_string(), ids.len() as u64, ids.to_vec());

    // No task: the section says how to add one.
    let reply = otto.call(json!({"cmd": "section", "section": "tasks"}));
    assert_eq!((&reply["tasks_bar"]["empty"], bar(&reply).len()), (&json!(true), 0));
    otto.call(json!({"cmd": "task_add", "title": "ask", "state": "needs_you"}));
    otto.call(json!({"cmd": "task_add", "title": "build", "state": "running"}));
    otto.call(json!({"cmd": "task_new", "line": "idea p1"}));

    // ⌘↵ in the new-task box adds the task and starts it: its tab is on show, and it is under Open.
    otto.call(json!({"cmd": "compose", "text": "fix zoom #canvas"}));
    let reply = key("cmd-enter");
    assert_eq!((reply["mode"].as_str(), &reply["state"]["current_task"]), (Some("normal"), &json!(4)));
    assert_eq!(
        bar(&reply),
        [section("needs_you", &[1]), section("open", &[4]), section("running", &[2]), section("captured", &[3])]
    );
    assert!(reply["state"]["tasks"]["list"][3]["created"].as_u64().is_some(), "{reply}");
    // The same from the socket.
    let filed = otto.call(json!({"cmd": "task_new", "line": "write the notes", "open": true}));
    assert_eq!((&filed["task"]["id"], &filed["opened"]), (&json!(5), &json!(true)));
    // A task that needs you stays under Needs you when it is open; its row says what it has open.
    let reply = otto.call(json!({"cmd": "task_open", "id": 1}));
    assert_eq!(bar(&reply)[..2], [section("needs_you", &[1]), section("open", &[4, 5])]);
    let row = &reply["tasks_bar"]["sections"][0]["rows"][0];
    assert_eq!((&row["tabs"], &row["current"]), (&json!(1), &json!(true)));
    assert!(row["detail"].as_str().unwrap().starts_with("1 tab, 0 panes"), "{row}");

    // Where a control is drawn: sneak mode's targets give each one's frame. Then a real click
    // there, through the window's own handlers.
    let press = |id: &str| -> Value {
        let reply = otto.call(json!({"cmd": "sneak"}));
        key("escape");
        let targets = reply["sneak"]["targets"].as_array().expect("targets");
        let drawn: Vec<&str> = targets.iter().filter_map(|t| t["source"]["id"].as_str()).collect();
        let target = targets.iter().find(|t| t["source"]["id"] == id).unwrap_or_else(|| panic!("no {id} in {drawn:?}"));
        let f = |key: &str| target["frame"][key].as_f64().unwrap();
        otto.click([f("x") + f("w") / 2.0, f("y") + f("h") / 2.0], 1)
    };
    let drawn = |id: &str| -> bool {
        let reply = otto.call(json!({"cmd": "sneak"}));
        key("escape");
        reply["sneak"]["targets"].as_array().unwrap().iter().any(|t| t["source"]["id"] == id)
    };

    // A folded heading keeps its count and draws no rows. By the socket, then by a click on it.
    let reply = otto.call(json!({"cmd": "task_group", "heading": "open"}));
    assert_eq!(bar(&reply)[1], ("open".to_string(), 2, vec![]));
    let reply = press("task-heading:open");
    assert_eq!(bar(&reply)[1], section("open", &[4, 5]));

    // The buttons of a row show on the row list mode has selected (and under the pointer).
    // Rows as list mode walks them: Needs you, 1, Open, 4, 5, Running, 2, Captured, 3.
    key("cmd-shift-e");
    let reply = key("j j j j j");
    assert_eq!(reply["state"]["list"], 6);
    // A click on Dismiss: the task leaves, and a line at the top offers to take it back.
    let reply = press("task-dismiss:2");
    assert_eq!((&reply["tasks_bar"]["undo"], bar(&reply).len()), (&json!("Dismissed"), 3));
    // A click on that line does what `u` does, and the offer goes.
    let reply = press("task-undo");
    assert_eq!((&reply["tasks_bar"]["undo"], bar(&reply)[2].clone()), (&json!(null), section("running", &[2])));
    // A click on Hold. Left alone, the offer goes after a few seconds; the move stays.
    let reply = press("task-hold:2");
    assert_eq!(reply["tasks_bar"]["undo"], "On hold");
    otto.wait_for("the offer to undo to go", |o| {
        o.call(json!({"cmd": "state"}))["tasks_bar"]["undo"].is_null().then_some(())
    });
    assert_eq!(bar(&otto.call(json!({"cmd": "state"}))).last(), Some(&section("on_hold", &[2])));
    // On the row of a task on hold there is no Hold button: it would change nothing.
    // Rows now: Needs you, 1, Open, 4, 5, Captured, 3, On hold, 2. The selection went with its task.
    assert_eq!(otto.call(json!({"cmd": "state"}))["state"]["list"], 8);
    assert!(drawn("task-dismiss:2") && !drawn("task-hold:2"));
    // The same moves from the socket (an agent's) offer nothing.
    assert_eq!(otto.call(json!({"cmd": "task_dismiss", "id": 3}))["tasks_bar"]["undo"], json!(null));
    // A click on Start shows the task's tabs.
    let reply = press("task-start:2");
    assert_eq!(reply["state"]["current_task"], 2);

    // `task_new` with `open` while the new-task box is open: the task is added, the tabs on show
    // stay, and the reply says so.
    otto.call(json!({"cmd": "compose", "text": "mine"}));
    let filed = otto.call(json!({"cmd": "task_new", "line": "theirs", "open": true}));
    assert_eq!((&filed["opened"], filed["task"]["title"].as_str()), (&json!(false), Some("theirs")));
    let state = otto.call(json!({"cmd": "state"}));
    assert_eq!((&state["state"]["current_task"], state["mode"].as_str()), (&json!(2), Some("compose")));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn list_mode_drives_the_bar_with_the_keyboard() {
    let otto = Otto::launch("list");
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    for (title, state) in [("first", "needs_you"), ("second", "needs_you"), ("third", "running")] {
        otto.call(json!({"cmd": "task_add", "title": title, "priority": 2, "state": state}));
    }
    otto.call(json!({"cmd": "section", "section": "tasks"}));

    // ⌘⇧E gives the bar the keyboard, on its first task: the heading above it is row 0.
    let reply = key("cmd-shift-e");
    assert_eq!((reply["mode"].as_str(), reply["state"]["list"].as_u64()), (Some("list"), Some(1)));
    // j moves down; x dismisses that task; the selection stays at its place.
    key("j");
    let reply = key("x");
    let titles = |reply: &Value| -> Vec<String> {
        reply["inbox"].as_array().unwrap().iter().map(|r| r["title"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!((titles(&reply), reply["state"]["list"].as_u64()), (vec!["first".into(), "third".into()], Some(2)));
    // u takes it back; h holds the selected task; s snoozes it.
    assert_eq!(titles(&key("u")), ["first", "second", "third"]);
    let reply = key("h");
    assert_eq!(reply["state"]["tasks"]["list"][1]["state"], "parked");
    key("u");
    let reply = key("k s");
    assert!(reply["state"]["tasks"]["list"][0]["snoozed_until"].as_u64().is_some(), "the first task is snoozed");
    assert_eq!(titles(&reply), ["second", "third"]);
    // A heading is a row: space folds it, ↵ unfolds it, and the selection stays on it.
    let reply = key("k space");
    assert_eq!((&reply["state"]["collapsed"], reply["state"]["list"].as_u64()), (&json!(["needs_you"]), Some(0)));
    assert_eq!(reply["tasks_bar"]["sections"][0]["rows"], json!([]));
    let reply = key("enter");
    assert_eq!((&reply["state"]["collapsed"], reply["mode"].as_str()), (&json!([]), Some("list")));

    // Esc gives the keyboard back. While the bar has it, keys do not reach the terminal.
    assert_eq!(key("escape")["mode"], "normal");
    otto.call(json!({"cmd": "split"}));
    assert_eq!(key("cmd-shift-e")["mode"], "list");
    key("j k");
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo typed-$((1+1))\r"}));
    otto.call(json!({"cmd": "wait", "pane": 1, "pattern": "^typed-2$", "timeout_ms": 40000}));
    let text = otto.call(json!({"cmd": "capture", "pane": 1}))["text"].as_str().unwrap().to_string();
    assert!(!text.contains("jk"), "list keys were typed into the shell: {text}");
    // A split takes the keyboard for its new pane.
    assert_eq!(otto.call(json!({"cmd": "split", "axis": "down"}))["mode"], "normal");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_new_task_box_files_a_line() {
    // One repository to name with `>`.
    let root = std::env::temp_dir().join(format!("ottospace-e2e-compose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("flow-app/.git")).unwrap();
    std::fs::write(root.join("flow-app/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
    // A path inside it for `/`, and a made-up skill for `$`: never the user's own skill folders.
    std::fs::create_dir_all(root.join("flow-app/src/export")).unwrap();
    let skills = root.join("skills");
    std::fs::create_dir_all(skills.join("code-review")).unwrap();
    std::fs::write(skills.join("code-review/SKILL.md"), "").unwrap();
    let env = [("OTTOSPACE_REPOS", root.to_str().unwrap()), ("OTTOSPACE_SKILLS", skills.to_str().unwrap())];
    let otto = Otto::launch_with("compose", &env);
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    otto.call(json!({"cmd": "split"}));

    // ⌘N opens the box. It takes the keys: nothing reaches the shell behind it, and ⌘D waits.
    let reply = key("cmd-n");
    assert_eq!((reply["mode"].as_str(), reply["state"]["compose"].as_str()), (Some("compose"), Some("")));
    assert_eq!(key("cmd-d")["state"]["panes"].as_array().map(Vec::len), Some(1));
    // ↵ on an empty line files nothing; Esc closes.
    assert_eq!(key("enter")["mode"], "compose");
    assert_eq!(key("escape")["mode"], "normal");

    // An agent offers a draft; Tab finishes the repository's name; ↵ files it.
    otto.call(json!({"cmd": "compose", "text": "fix zoom flicker #canvas p2 >flow"}));
    // A second draft while the first is on screen is refused, not dropped in silence.
    assert_eq!(otto.raw(r#"{"cmd":"compose","text":"another"}"#)["ok"], false);
    assert_eq!(key("tab")["state"]["compose"], "fix zoom flicker #canvas p2 >flow-app ");
    let reply = key("enter");
    let task = &reply["state"]["tasks"]["list"][0];
    assert_eq!((reply["mode"].as_str(), task["title"].as_str()), (Some("normal"), Some("fix zoom flicker")));
    assert_eq!(
        (task["project"].as_str(), task["priority"].as_u64(), task["state"].as_str()),
        (Some("canvas"), Some(2), Some("captured"))
    );
    let repo = root.canonicalize().unwrap().join("flow-app");
    assert_eq!(task["repo"].as_str(), repo.to_str());

    // The same from the socket, without the box. A line with no title is refused.
    let filed = otto.call(json!({"cmd": "task_new", "line": "ask Ana #support p1"}));
    assert_eq!((filed["task"]["id"].as_u64(), filed["task"]["project"].as_str()), (Some(2), Some("support")));
    assert_eq!(otto.raw(r##"{"cmd":"task_new","line":"#support p1"}"##)["ok"], false);

    // Opening the task the first time: its own tab, named after its repository, with a shell there.
    let reply = otto.call(json!({"cmd": "task_open", "id": 1}));
    let state = &reply["state"];
    assert_eq!(
        (&state["current_task"], &state["tabs"][1]["task"], &state["tabs"][1]["title"]),
        (&json!(1), &json!(1), &json!("flow-app"))
    );
    let pane = state["focused"].as_u64().unwrap();
    otto.call(json!({"cmd": "type", "pane": pane, "text": "pwd\r"}));
    let here = format!("^{}$", regex::escape(repo.to_str().unwrap()));
    otto.call(json!({"cmd": "wait", "pane": pane, "pattern": here, "timeout_ms": 40000}));
    // A new tab is the task's. ⌘1 and ⌘2 count the task's tabs, not all tabs.
    let reply = otto.call(json!({"cmd": "new_tab"}));
    assert_eq!((&reply["state"]["tabs"][2]["task"], &reply["state"]["active"]), (&json!(1), &json!(2)));
    let reply = key("cmd-1");
    assert_eq!((&reply["state"]["active"], &reply["state"]["focused"]), (&json!(1), &json!(pane)));
    assert_eq!(key("cmd-2")["state"]["active"], 2);
    assert_eq!(key("cmd-3")["state"]["active"], 2, "the task has two tabs: ⌘3 does nothing");
    // A follow-up filed from the task's tab cannot take the tab; a tab that is gone is refused too.
    let taken = otto.raw(&json!({"cmd": "task_add", "title": "follow-up", "tab": state["tabs"][1]["id"]}).to_string());
    assert!(taken["error"].as_str().is_some_and(|e| e.contains("another task")), "{taken}");
    assert_eq!(otto.raw(r#"{"cmd":"task_add","title":"x","tab":99}"#)["ok"], false);
    // Leaving shows the tabs of no task again; the task's shell goes on.
    let reply = otto.call(json!({"cmd": "task_leave"}));
    assert_eq!((&reply["state"]["current_task"], &reply["state"]["active"]), (&Value::Null, &json!(0)));
    assert!(reply["terminals"][pane.to_string()].is_object(), "the task's terminal is still there");

    // ⌘⇧N shows the tabs of the task that needs the user. It had none: it gets one.
    otto.call(json!({"cmd": "task_add", "title": "waits", "state": "needs_you"}));
    let reply = key("cmd-shift-n");
    assert_eq!(reply["state"]["current_task"], 3);
    otto.call(json!({"cmd": "task_leave"}));

    // What a line's tags may name is read off the window's thread: the box says when it is in.
    let compose = |text: &str| {
        otto.call(json!({"cmd": "compose", "text": text}));
        otto.wait_for("the line's tags to be read", |o| {
            (o.call(json!({"cmd": "state"}))["compose_read"] == true).then_some(())
        });
    };
    // Tab finishes each kind of tag from its own names. A space in a path gets a backslash.
    std::fs::create_dir_all(root.join("flow-app/My Notes")).unwrap();
    let finished = [
        ("a @co", "a @codex "),
        ("a $code", "a $code-review "),
        ("a >flow-app /src/ex", "a >flow-app /src/export/"),
        ("a >flow-app /My", r"a >flow-app /My\ Notes/"),
    ];
    for (typed, completed) in finished {
        compose(typed);
        assert_eq!(key("tab")["state"]["compose"], completed, "{typed}");
        key("escape");
    }
    // An amount of money is no skill being typed: Tab leaves the line alone.
    compose("costs $5");
    assert_eq!(key("tab")["state"]["compose"], "costs $5");
    key("escape");
    // Every tag through the box. The skill is spelled as its folder is, once; the path is kept
    // as typed, with where it was found: inside the repository.
    let line = r"review the export PR #canvas >flow-app p2, @codex $Code-Review $code-review /src/export, /My\ Notes";
    compose(line);
    let reply = key("enter");
    let task = &reply["state"]["tasks"]["list"][3];
    assert_eq!((task["title"].as_str(), task["project"].as_str()), (Some("review the export PR"), Some("canvas")));
    assert_eq!(
        (task["repo"].as_str(), task["agent"].as_str(), task["priority"].as_u64()),
        (repo.to_str(), Some("codex"), Some(2))
    );
    assert_eq!((&task["skills"], &task["paths"]), (&json!(["code-review"]), &json!(["/src/export", "/My Notes"])));
    assert_eq!(task["resolved"], json!([repo.join("src/export"), repo.join("My Notes")]));
    // A socket call while the box is open reads its own line's tags: the user's line keeps its own.
    compose("mine $code-review");
    let theirs = otto.call(json!({"cmd": "task_new", "line": "theirs /no-such-path-here"}));
    assert_eq!(theirs["task"]["title"], "theirs /no-such-path-here");
    let reply = key("enter");
    let mine = reply["state"]["tasks"]["list"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((&mine["title"], &mine["skills"]), (&json!("mine"), &json!(["code-review"])));
    // Names nobody knows, and prose, stay in the title. `@repo` no longer names a repository.
    let line = "ask @ana if $5 > 3 for @flow-app $nothing /no-such-path-here";
    let filed = otto.call(json!({"cmd": "task_new", "line": line}));
    assert_eq!((filed["task"]["title"].as_str(), &filed["task"]["repo"]), (Some(line), &Value::Null));
    // `task_add` takes an agent and skills that `task_new` would know, spelled as they are
    // read, and refuses any other by name. Paths are kept as given.
    let request =
        json!({"cmd": "task_add", "title": "t", "agent": "Claude", "paths": ["~/notes"], "skills": ["CODE-review"]});
    let filed = otto.call(request);
    let task = &filed["task"];
    assert_eq!(
        (&task["agent"], &task["paths"], &task["skills"], &task["resolved"]),
        (&json!("claude"), &json!(["~/notes"]), &json!(["code-review"]), &json!([]))
    );
    let before = otto.call(json!({"cmd": "state"}))["state"]["tasks"]["list"].as_array().unwrap().len();
    for unknown in [json!({"agent": "../../etc"}), json!({"skills": ["code-review", "deploy"]})] {
        let mut request = json!({"cmd": "task_add", "title": "t"});
        request.as_object_mut().unwrap().extend(unknown.as_object().unwrap().clone());
        let refused = otto.raw(&request.to_string());
        assert!(
            refused["ok"] == false && refused["error"].as_str().unwrap().contains("not an agent or a skill"),
            "{refused}"
        );
    }
    assert_eq!(otto.call(json!({"cmd": "state"}))["state"]["tasks"]["list"].as_array().unwrap().len(), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_home_screen_shows_tasks_as_nodes_until_a_pane_opens() {
    let otto = Otto::launch("home");
    let home = |o: &Otto| o.call(json!({"cmd": "state"}))["home"].clone();
    assert_eq!((&home(&otto)["nodes"], &home(&otto)["needs_you"]), (&json!([]), &json!([])), "no tasks, no nodes");

    for line in ["review the fix #canvas p1", "port the job #billing", "ask Ana #support p2", "tidy up"] {
        otto.call(json!({"cmd": "task_new", "line": line}));
    }
    for (id, state) in [(1, "needs_you"), (2, "running"), (3, "needs_you"), (4, "done")] {
        otto.call(json!({"cmd": "task_set_state", "id": id, "state": state}));
    }
    // Lanes are the projects by name, then the tasks of none. Depth 0 is the newest task.
    let shown = home(&otto);
    let mut nodes: Vec<Value> = shown["nodes"].as_array().expect("nodes").clone();
    nodes.sort_by_key(|n| n["task"].as_u64());
    let facts: Vec<Value> = nodes.iter().map(|n| json!([n["task"], n["lane"], n["depth"], n["group"]])).collect();
    assert_eq!(
        json!(facts),
        json!([[1, 1, 3, "needs_you"], [2, 0, 2, "running"], [3, 2, 1, "needs_you"], [4, 3, 0, "done"]])
    );
    let mut crossings: Vec<String> = nodes.iter().map(|n| n["crossing"].to_string()).collect();
    crossings.sort();
    crossings.dedup();
    assert_eq!(crossings.len(), 4, "each node has its own crossing: {shown}");
    // "Needs you": most urgent first, with what its row shows.
    let rows: Vec<Value> = shown["needs_you"].as_array().expect("rows").clone();
    let row = |r: &Value| json!([r["id"], r["priority"], r["title"], r["project"]]);
    assert_eq!(
        json!(rows.iter().map(row).collect::<Vec<_>>()),
        json!([[1, 1, "review the fix", "canvas"], [3, 2, "ask Ana", "support"]])
    );
    assert_eq!(
        (&shown["counts"]["needs_you"], &shown["counts"]["running"], &shown["counts"]["done"]),
        (&json!(2), &json!(1), &json!(1))
    );
    // What was drawn, not what the model says: a click target for every node, inside the
    // home screen and clear of the new-task line and its list.
    let drawn = |o: &Otto, nodes: usize| {
        o.wait_for("the home screen to be drawn", |o| {
            let drawn = o.call(json!({"cmd": "state"}))["home_drawn"].clone();
            (drawn["nodes"].as_object()?.len() == nodes && drawn["panel"][2].as_f64()? > 1.0).then_some(drawn)
        })
    };
    let rect = |v: &Value| -> [f64; 4] { std::array::from_fn(|i| v[i].as_f64().expect("a rectangle")) };
    let apart = |a: [f64; 4], b: [f64; 4]| {
        a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1]
    };
    let first = drawn(&otto, 4);
    let (area, panel) = (rect(&first["area"]), rect(&first["panel"]));
    for (task, node) in first["nodes"].as_object().expect("drawn nodes") {
        let node = rect(node);
        assert!(apart(node, panel), "the node of task {task} at {node:?} is under the text at {panel:?}");
        let inside = node[0] >= area[0] && node[1] >= area[1] && node[0] + node[2] <= area[0] + area[2];
        assert!(inside && node[1] + node[3] <= area[1] + area[3], "task {task} at {node:?} is outside {area:?}");
    }
    // A hidden task loses its node.
    otto.call(json!({"cmd": "task_dismiss", "id": 4}));
    assert_eq!(home(&otto)["nodes"].as_array().map(Vec::len), Some(3));
    drawn(&otto, 3);

    // The lattice's lines are built once and kept: frames that move nothing only paint them.
    let lattice = |o: &Otto| {
        let drawn = o.call(json!({"cmd": "state"}))["home_drawn"].clone();
        (drawn["lattice_builds"].as_u64().expect("builds"), drawn["lattice_paints"].as_u64().expect("paints"))
    };
    // Showing and hiding the key reference draws the window again.
    let frames = |o: &Otto| {
        let (_, before) = lattice(o);
        o.call(json!({"cmd": "keys"}));
        o.call(json!({"cmd": "keys"}));
        o.wait_for("the window to be drawn again", |o| Some(lattice(o)).filter(|(_, paints)| *paints > before))
    };
    frames(&otto); // lets a build that is still on its way land
    let (builds, _) = frames(&otto);
    assert_eq!(frames(&otto).0, builds, "a frame with nothing moved built the lines again");
    // A second row under "Needs you" moves the text, so the lines around it are built again.
    otto.call(json!({"cmd": "task_set_state", "id": 2, "state": "needs_you"}));
    otto.wait_for("the lines to be built again", |o| (lattice(o).0 > builds).then_some(()));

    // A click on a node or a row opens the task (the same action as `task_open`). Its new tab
    // has no pane yet, so the home screen stays until one opens.
    let reply = otto.call(json!({"cmd": "task_open", "id": 3}));
    assert_eq!(reply["state"]["current_task"], 3);
    assert!(reply["home"].is_object(), "an empty tab shows the home screen: {reply}");
    let reply = otto.call(json!({"cmd": "split", "kind": "empty"}));
    assert!(reply["home"].is_null(), "a pane takes the home screen's place");
    otto.bounds(&pane_ids(&reply));
    // The empty pane's patch is kept too: a terminal printing beside it does not build it again.
    otto.call(json!({"cmd": "split"}));
    let printed = |o: &Otto, word: &str| {
        o.call(json!({"cmd": "type", "pane": 2, "text": format!("echo {word}-$((6*7))\r")}));
        o.call(json!({"cmd": "wait", "pane": 2, "pattern": format!("^{word}-42$"), "timeout_ms": 40000}));
        lattice(o)
    };
    let (builds, paints) = printed(&otto, "one");
    let (again, later) = (printed(&otto, "two"), printed(&otto, "three"));
    assert_eq!((again.0, later.0), (builds, builds), "the terminal's frames built the patch again");
    assert!(later.1 > paints, "the patch was painted with the terminal's frames");
    // The tab of no task is still empty.
    assert!(otto.call(json!({"cmd": "task_leave"}))["home"].is_object());
}

/// A fresh folder to pass as `OTTOSPACE_HOME`, with `settings` as its `settings.json`.
fn user_folder(name: &str, settings: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("ottospace-e2e-home-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("settings.json"), settings).unwrap();
    home
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn tasks_are_kept_between_runs() {
    let home = user_folder("tasks", r#"{"repos": []}"#); // no repositories: not the user's own
    let env = [("OTTOSPACE_HOME", home.to_str().unwrap())];
    let titles = |o: &Otto| -> Vec<(String, String)> {
        let rows = o.call(json!({"cmd": "task_list"}))["inbox"].as_array().unwrap().clone();
        rows.iter().map(|r| (r["group"].as_str().unwrap().into(), r["title"].as_str().unwrap().into())).collect()
    };
    let before = {
        let otto = Otto::launch_with("keep-1", &env);
        otto.call(json!({"cmd": "task_add", "title": "question", "priority": 1, "state": "needs_you", "tab": 1}));
        otto.call(json!({"cmd": "task_add", "title": "idea"}));
        otto.call(json!({"cmd": "task_add", "title": "later", "state": "needs_you"}));
        otto.call(json!({"cmd": "task_hold", "id": 2}));
        otto.call(json!({"cmd": "task_snooze", "id": 3, "seconds": 3600}));
        // One file a task, written as it changes. The tab is not kept: it ends with the run.
        let file: Value = serde_json::from_str(&std::fs::read_to_string(home.join("tasks/1.json")).unwrap()).unwrap();
        assert_eq!(
            (&file["title"], &file["state"], &file["tab"]),
            (&json!("question"), &json!("needs_you"), &Value::Null)
        );
        titles(&otto)
    }; // the app is killed here, with no chance to save on the way out
    assert_eq!(before, [("needs_you".into(), "question".into()), ("on_hold".into(), "idea".into())]);

    // Files that are not a task of their number are passed over, and reported: text that is not
    // a task, a copy of task 1 under another name, and a task file too broken to read.
    std::fs::write(home.join("tasks/notes.json"), "not a task").unwrap();
    std::fs::copy(home.join("tasks/1.json"), home.join("tasks/1 copy.json")).unwrap();
    std::fs::write(home.join("tasks/9.json"), r#"{"id": 9, "title": "half written"#).unwrap();
    let otto = Otto::launch_with("keep-2", &env);
    assert_eq!(titles(&otto), before, "the same inbox, snooze and hold included");
    let state = otto.call(json!({"cmd": "state"}));
    assert!(state["notice"].as_str().is_some_and(|e| e.contains("could not be read")), "{}", state["notice"]);
    assert!(state["notice"].as_str().is_some_and(|e| e.contains("3 file(s)")), "{}", state["notice"]);
    // Ids go on from the highest number in the folder, so the broken 9.json is not overwritten.
    assert_eq!(otto.call(json!({"cmd": "task_add", "title": "new"}))["task"]["id"], 10);
    assert!(std::fs::read_to_string(home.join("tasks/9.json")).unwrap().contains("half written"));
    // The file is the user's alone.
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(home.join("tasks/10.json")).unwrap().permissions().mode() & 0o777, 0o600);
    // A title too long to read back is refused. The start-up notice outlives a later request.
    let long = otto.raw(&json!({"cmd": "task_add", "title": "x".repeat(501)}).to_string());
    assert_eq!(long["ok"], false);
    assert!(otto.call(json!({"cmd": "state"}))["notice"].as_str().is_some());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn settings_choose_the_font_the_repos_and_the_tasks_folder() {
    // Defaults: Menlo 13, rows 18 points high.
    let plain = Otto::launch("settings-none");
    let state = plain.call(json!({"cmd": "state"}));
    // The build, as the status bar shows it: "0.1.0 · local" here.
    let version = state["version"].as_str().unwrap_or_default();
    assert!(version.starts_with(concat!(env!("CARGO_PKG_VERSION"), " · ")), "{version}");
    assert_eq!(
        (&state["font"], &state["cell"][1], &state["notice"]),
        (&json!({"name": "Menlo", "size": 13.0}), &json!(18.0), &Value::Null)
    );
    drop(plain);

    let home = user_folder("settings", "");
    fake_repo(&home.join("src/app"));
    let settings = json!({
        // A test run never offers to start an app, whatever the settings say.
        "apps": ["Calculator"],
        "terminal": {"font": "Monaco", "font_size": 16},
        "repos": [home.join("src")],
        "tasks": "elsewhere",
    });
    std::fs::write(home.join("settings.json"), settings.to_string()).unwrap();
    let env = [("OTTOSPACE_HOME", home.to_str().unwrap())];
    let otto = Otto::launch_with("settings-set", &env);
    let state = otto.call(json!({"cmd": "state"}));
    assert_eq!((&state["font"], &state["cell"][1]), (&json!({"name": "Monaco", "size": 16.0}), &json!(22.0)));
    assert_eq!(state["state"]["repos"][0]["name"], "app");
    assert_eq!((&state["closed_apps"], &state["notice"]), (&json!([]), &Value::Null));
    otto.call(json!({"cmd": "task_add", "title": "kept elsewhere"}));
    assert!(home.join("elsewhere/1.json").is_file(), "a relative tasks folder is inside the user's folder");
    drop(otto);

    // A mistake in the file: the defaults apply and the status bar says what is wrong.
    std::fs::write(home.join("settings.json"), r#"{"terminal": {"font_family": "Monaco"}}"#).unwrap();
    let otto = Otto::launch_with("settings-bad", &env);
    let state = otto.call(json!({"cmd": "state"}));
    assert_eq!(state["font"]["name"], "Menlo");
    assert!(state["notice"].as_str().is_some_and(|e| e.contains("font_family")), "{}", state["notice"]);
    drop(otto);
    // A font that is not installed: Menlo, at the size asked for.
    std::fs::write(home.join("settings.json"), r#"{"terminal": {"font": "No Such Font", "font_size": 15}}"#).unwrap();
    let otto = Otto::launch_with("settings-font", &env);
    let state = otto.call(json!({"cmd": "state"}));
    assert_eq!(state["font"], json!({"name": "Menlo", "size": 15.0}));
    assert!(state["notice"].as_str().is_some_and(|e| e.contains("not installed")), "{}", state["notice"]);
    let _ = std::fs::remove_dir_all(&home);
}

/// Terminal 1's entry in the state dump.
fn term1(o: &Otto) -> Value {
    o.call(json!({"cmd": "state"}))["terminals"]["1"].clone()
}

fn shows(terminal: &Value, text: &str) -> bool {
    terminal["lines"].as_array().is_some_and(|lines| lines.iter().any(|l| l.as_str() == Some(text)))
}

/// A made-up session file from `ottospace-sessions`' fixtures, copied to `to` with its directory
/// (`/work/app`) rewritten to `cwd`, and last changed `age` ago. Never a real session file.
fn session_fixture(fixture: &str, to: &Path, cwd: &Path, age: Duration) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ottospace-sessions/tests/fixtures").join(fixture);
    let text = std::fs::read_to_string(&from).unwrap().replace("/work/app", cwd.to_str().unwrap());
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::write(to, text).unwrap();
    let file = std::fs::File::options().write(true).open(to).unwrap();
    file.set_modified(std::time::SystemTime::now() - age).unwrap();
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn sessions_that_are_no_tasks_are_listed_and_can_be_adopted() {
    let home = user_folder("sessions", "");
    fake_repo(&home.join("src/app"));
    let repo = home.canonicalize().unwrap().join("src/app");
    // Three made-up sessions: a Claude Code at work in the repository, a Codex idle for two hours
    // below it, and a Codex last active three days ago.
    let (claude, codex) = (home.join("claude"), home.join("codex"));
    let (busy, idle, old) = (
        "11111111-1111-4111-8111-111111111111",
        "c0de0001-0000-4000-8000-000000000001",
        "c0de0004-0000-4000-8000-000000000004",
    );
    let project = claude.join("projects").join(ottospace_sessions::claude::slug(&repo));
    let hours = |n: u64| Duration::from_secs(n * 3600);
    session_fixture(
        &format!("claude/projects/-work-app/{busy}.jsonl"),
        &project.join(format!("{busy}.jsonl")),
        &repo,
        hours(0),
    );
    let rollout = |day: &str, at: &str, id: &str| {
        format!("sessions/2026/{day}/rollout-2026-{}T{at}-{id}.jsonl", day.replace('/', "-"))
    };
    let (recent, long_ago) = (rollout("11/12", "09-30-00", idle), rollout("03/02", "15-00-00", old));
    session_fixture(&format!("codex/{recent}"), &codex.join(&recent), &repo, hours(2));
    session_fixture(&format!("codex/{long_ago}"), &codex.join(&long_ago), &repo.join("docs"), hours(72));
    let settings = |sessions: bool| {
        let dirs = json!({"claude": [claude], "codex": codex});
        let settings = json!({"repos": [home.join("src")], "sessions": sessions, "session_dirs": dirs});
        std::fs::write(home.join("settings.json"), settings.to_string()).unwrap();
    };
    let env = [("OTTOSPACE_HOME", home.to_str().unwrap())];
    // The Sessions group as (runner, id, name, active) of each row drawn.
    let rows = |reply: &Value| -> Vec<(String, String, String, bool)> {
        let text = |row: &Value, key: &str| row[key].as_str().unwrap().to_string();
        let rows = reply["sessions_bar"]["rows"].as_array().unwrap();
        rows.iter()
            .map(|r| (text(r, "runner"), text(r, "id"), text(r, "name"), r["active"].as_bool().unwrap()))
            .collect()
    };
    let row = |runner: &str, id: &str, active: bool| (runner.to_string(), id.to_string(), "app".to_string(), active);

    // `"sessions": false`: nothing is read, with the section open or not.
    settings(false);
    let off = Otto::launch_with("sessions-off", &env);
    off.call(json!({"cmd": "section", "section": "tasks"}));
    std::thread::sleep(Duration::from_millis(1500));
    let bar = off.call(json!({"cmd": "state"}))["sessions_bar"].clone();
    assert_eq!((&bar["enabled"], &bar["reads"], &bar["count"]), (&json!(false), &json!(0), &json!(0)));
    assert_eq!(bar["slow_reads"], 0, "no task follows a session: nothing to look at");
    drop(off);

    settings(true);
    let otto = Otto::launch_with("sessions", &env);
    // Nothing is read while the Tasks section is closed.
    std::thread::sleep(Duration::from_millis(1500));
    let bar = otto.call(json!({"cmd": "state"}))["sessions_bar"].clone();
    assert_eq!((&bar["enabled"], &bar["reads"], &bar["count"]), (&json!(true), &json!(0), &json!(0)));
    assert_eq!(bar["slow_reads"], 0, "no task follows a session: nothing to look at");
    // Open: read at once. The active session first, the one older than a day behind "1 older".
    otto.call(json!({"cmd": "section", "section": "tasks"}));
    let reply = otto.wait_for("the sessions to be read", |o| {
        Some(o.call(json!({"cmd": "state"}))).filter(|reply| reply["sessions_bar"]["count"] == 3)
    });
    assert_eq!(rows(&reply), [row("claude", busy, true), row("codex", idle, false)]);
    let bar = &reply["sessions_bar"];
    assert_eq!(
        (&bar["older"], &bar["rows"][0]["doing"], &bar["rows"][1]["doing"]),
        (&json!(1), &json!("network · git push"), &Value::Null)
    );
    assert_eq!((&bar["rows"][0]["age"], &bar["rows"][1]["age"]), (&json!("now"), &json!("2h")));
    let reply = otto.call(json!({"cmd": "sessions_older"}));
    assert_eq!(rows(&reply)[2], row("codex", old, false));
    otto.call(json!({"cmd": "sessions_older"}));
    // The words of the made-up conversation are nowhere in what the app says.
    let said = otto.call(json!({"cmd": "state"})).to_string();
    assert!(!said.contains("Add a greeting") && !said.contains("made-up"), "{said}");

    // One terminal opened in the session's directory, one session started there: it runs "in" that
    // pane, and a click on the row focuses it. Nothing is typed into it.
    otto.call(json!({"cmd": "repos"}));
    let pane = otto.call(json!({"cmd": "open_repo", "path": repo}))["state"]["focused"].clone();
    let other = otto.call(json!({"cmd": "split"}))["state"]["focused"].clone();
    assert_ne!(pane, other);
    let reply = otto.call(json!({"cmd": "session_click", "runner": "claude", "id": busy}));
    assert_eq!((&reply["sessions_bar"]["rows"][0]["pane"], &reply["state"]["focused"]), (&pane, &pane));
    // No pane to tell: the click selects the row.
    let reply = otto.call(json!({"cmd": "session_click", "runner": "codex", "id": idle}));
    assert_eq!(
        (reply["mode"].as_str(), &reply["state"]["list"], &reply["state"]["focused"]),
        (Some("list"), &json!(2), &pane)
    );
    otto.call(json!({"cmd": "key", "keys": "escape"}));

    // Adopt: a task named after the repository, in it, worked by the session's runner. The task
    // keeps which session it is, and no path from the session's own file.
    let reply = otto.call(json!({"cmd": "session_adopt", "runner": "claude", "id": busy}));
    let task = &reply["state"]["tasks"]["list"][0];
    assert_eq!((&task["title"], &task["state"], &task["agent"]), (&json!("app"), &json!("running"), &json!("claude")));
    assert_eq!((task["repo"].as_str(), task["session"]["id"].as_str()), (repo.to_str(), Some(busy)));
    assert!(task["session"].get("cwd").is_none(), "{task}");
    assert_eq!(reply["tasks_bar"]["sections"][0]["rows"][0]["detail"], ">app · @claude · now");
    assert_eq!(rows(&reply), [row("codex", idle, false)]);
    // One that started below the repository is the repository's task too: `>app`, not `>crates`.
    let reply = otto.call(json!({"cmd": "session_adopt", "runner": "codex", "id": idle}));
    let task = &reply["state"]["tasks"]["list"][1];
    assert_eq!(
        (task["repo"].as_str(), &task["state"], &task["agent"]),
        (repo.to_str(), &json!("captured"), &json!("codex"))
    );
    assert_eq!(rows(&reply), []);
    // Both stay tasks when the sessions are read again.
    let reads = reply["sessions_bar"]["reads"].as_u64().unwrap();
    let reply = otto.wait_for("the next read", |o| {
        Some(o.call(json!({"cmd": "state"}))).filter(|reply| reply["sessions_bar"]["reads"].as_u64() > Some(reads))
    });
    assert_eq!((rows(&reply), &reply["sessions_bar"]["count"]), (vec![], &json!(1)));
    // Hide takes the last one away, shown or not.
    let reply = otto.call(json!({"cmd": "session_hide", "runner": "codex", "id": old}));
    assert_eq!((&reply["sessions_bar"]["count"], &reply["sessions_bar"]["slow_reads"]), (&json!(0), &json!(0)));

    // The adopted session's file last changed `secs` ago.
    let transcript = project.join(format!("{busy}.jsonl"));
    let changed = |secs: u64| {
        let file = std::fs::File::options().write(true).open(&transcript).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(secs)).unwrap();
    };
    let task_state = |o: &Otto| o.call(json!({"cmd": "state"}))["state"]["tasks"]["list"][0]["state"].clone();
    // Idle for two minutes, as during a long build that prints nothing: its task runs on. The
    // threshold for "quiet" is three minutes (`session_quiet_minutes`).
    changed(120);
    let reads = otto.call(json!({"cmd": "state"}))["sessions_bar"]["reads"].as_u64().unwrap();
    otto.wait_for("two more reads", |o| {
        (o.call(json!({"cmd": "state"}))["sessions_bar"]["reads"].as_u64() > Some(reads + 1)).then_some(())
    });
    assert_eq!(task_state(&otto), "running");
    // The Tasks section closed, then quiet for four minutes: the task needs you all the same,
    // and the status bar's count with it. Only a look at the adopted sessions' files, after the
    // section closed, can have seen it: no listing was made.
    let closed = otto.call(json!({"cmd": "section", "section": "tasks"}));
    let reads = closed["sessions_bar"]["reads"].clone();
    let slow = otto.wait_for("a look at the adopted sessions' files", |o| {
        o.call(json!({"cmd": "state"}))["sessions_bar"]["slow_reads"].as_u64().filter(|n| *n >= 1)
    });
    assert_eq!(task_state(&otto), "running", "two minutes idle is not quiet, closed or open");
    changed(240);
    let reply = otto.wait_for("the task to follow its quiet session", |o| {
        let reply = o.call(json!({"cmd": "state"}));
        let followed = reply["state"]["tasks"]["list"][0]["state"] == "needs_you";
        (followed && reply["sessions_bar"]["slow_reads"].as_u64() > Some(slow)).then_some(reply)
    });
    assert_eq!((&reply["needs_you"], &reply["sessions_bar"]["count"]), (&json!(1), &json!(0)));
    assert_eq!(reply["sessions_bar"]["reads"], reads, "no listing: {reply}");
    drop(otto);

    // A new run, the section never opened: the task follows its session again once it works.
    // (Its file is found with one listing, as nothing kept where it is.)
    changed(0);
    let fresh = Otto::launch_with("sessions-close", &env);
    let reply = fresh.wait_for("the task to run again", |o| {
        Some(o.call(json!({"cmd": "state"}))).filter(|reply| reply["state"]["tasks"]["list"][0]["state"] == "running")
    });
    assert_eq!((&reply["sessions_bar"]["count"], &reply["sessions_bar"]["reads"]), (&json!(0), &json!(1)));

    // What was found is kept only while the section is open: closed, nothing is listed (so
    // nothing reads or adopts a session by what it did a while ago); opened, it is read again.
    fresh.call(json!({"cmd": "section", "section": "tasks"}));
    let listed = |o: &Otto, count: u64| {
        o.wait_for("the sessions bar", |o| {
            (o.call(json!({"cmd": "state"}))["sessions_bar"]["count"] == count).then_some(())
        })
    };
    listed(&fresh, 1); // the two adopted above are tasks on disk; the old one is left
    fresh.call(json!({"cmd": "section", "section": "tasks"}));
    listed(&fresh, 0);
    let refused = fresh.call(json!({"cmd": "session_adopt", "runner": "codex", "id": old}));
    assert_eq!(refused["state"]["tasks"]["list"].as_array().map(Vec::len), Some(2), "not listed: not adopted");
    fresh.call(json!({"cmd": "section", "section": "tasks"}));
    listed(&fresh, 1);
    drop(fresh);

    // `"sessions": false`: tasks adopted before are not followed either. Nothing is read.
    settings(false);
    let off = Otto::launch_with("sessions-off-adopted", &env);
    std::thread::sleep(Duration::from_millis(1500));
    let bar = off.call(json!({"cmd": "state"}))["sessions_bar"].clone();
    assert_eq!((&bar["reads"], &bar["slow_reads"]), (&json!(0), &json!(0)));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn terminal_scrolls_back_and_typing_returns_to_the_bottom() {
    let otto = Otto::launch("scroll");
    otto.call(json!({"cmd": "split"}));
    otto.bounds(&[1]);
    otto.call(json!({"cmd": "type", "text": "i=1; while [ $i -le 200 ]; do echo line-$i; i=$((i+1)); done\r"}));
    let t = otto.wait_for("200 lines", |o| Some(term1(o)).filter(|t| shows(t, "line-200")));
    let rows = t["rows"].as_u64().expect("rows");
    assert!(t["history"].as_u64() > Some(100), "history is kept: {t}");
    assert_eq!(t["display_offset"], 0);

    // The wheel scrolls back; an early line comes into view.
    let t = otto.call(json!({"cmd": "scroll", "lines": 150}))["terminals"]["1"].clone();
    let offset = t["display_offset"].as_u64().expect("offset");
    assert_eq!(offset, 150, "{t}");
    assert!(shows(&t, "line-50") && !shows(&t, "line-200"), "{t}");

    // ⇧End, ⇧Home, ⇧PageDown move the view; the keys do not reach the shell.
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}))["terminals"]["1"].clone();
    // A huge delta stops at the top instead of overflowing alacritty's offset.
    let t = otto.call(json!({"cmd": "scroll", "lines": 3e9}))["terminals"]["1"].clone();
    assert_eq!(t["display_offset"], t["history"]);
    assert_eq!(key("shift-end")["display_offset"], 0);
    let top = key("shift-home");
    assert_eq!(top["display_offset"], top["history"]);
    assert!(shows(&top, "line-1"), "{top}");
    let history = top["history"].as_u64().expect("history");
    assert_eq!(key("shift-pagedown")["display_offset"].as_u64(), Some(history - rows));

    // Typing jumps back to the bottom; new output while at the bottom stays there.
    let t = key("x");
    assert_eq!(t["display_offset"], 0, "{t}");
    otto.call(json!({"cmd": "type", "text": "\recho done\r"}));
    let t = otto.wait_for("output", |o| Some(term1(o)).filter(|t| shows(t, "done")));
    assert_eq!(t["display_offset"], 0, "{t}");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn mouse_selects_and_copy_paste_use_it() {
    let otto = Otto::launch("select");
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "split"})); // pane 2 has focus
    otto.bounds(&[1, 2]);
    // Fill the screen first, so the next output scrolls it.
    otto.call(json!({"cmd": "type", "pane": 1, "text": "i=1; while [ $i -le 100 ]; do echo $i; i=$((i+1)); done\r"}));
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo alpha beta gamma\r"}));
    let t = otto.wait_for("the output", |o| Some(term1(o)).filter(|t| shows(t, "alpha beta gamma")));
    let lines = t["lines"].as_array().expect("lines");
    let row = lines.iter().position(|l| l.as_str() == Some("alpha beta gamma")).expect("row") as f32 + 0.5;

    // A double-click selects a word, and focuses the pane it lands on.
    let down =
        |x: f32, clicks: u32| otto.call(json!({"cmd": "mouse_down", "pane": 1, "x": x, "y": row, "clicks": clicks}));
    let reply = down(7.5, 2);
    assert_eq!(reply["state"]["focused"], 1);
    assert_eq!(reply["terminals"]["1"]["selection"], "beta");
    assert_eq!(down(7.5, 3)["terminals"]["1"]["selection"], "alpha beta gamma\n", "a triple-click selects the line");

    // A drag selects from the press to the cell under the pointer; a click alone clears it.
    down(0.25, 1);
    let reply = otto.call(json!({"cmd": "mouse_drag", "pane": 1, "x": 9.75, "y": row}));
    assert_eq!(reply["terminals"]["1"]["selection"], "alpha beta");
    assert_eq!(down(3.5, 1)["terminals"]["1"]["selection"], Value::Null);

    // ⌘C with nothing selected copies nothing.
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    assert_eq!(key("cmd-c")["copied"], Value::Null);

    // The selection follows its text as new output scrolls the screen.
    down(7.5, 2);
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo one; echo two\r"}));
    let t = otto.wait_for("more output", |o| Some(term1(o)).filter(|t| shows(t, "two")));
    assert_eq!(t["selection"], "beta", "{t}");

    // ⌘C copies it; ⌘V pastes it at the prompt (e2e runs use a private clipboard).
    assert_eq!(key("cmd-c")["copied"], "beta");
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo pasted-"}));
    key("cmd-v");
    otto.call(json!({"cmd": "type", "pane": 1, "text": "\r"}));
    otto.wait_for("the pasted command's output", |o| Some(term1(o)).filter(|t| shows(t, "pasted-beta")));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn visual_mode_selects_with_the_keyboard() {
    let otto = Otto::launch("visual");
    otto.call(json!({"cmd": "split"}));
    otto.bounds(&[1]);
    otto.call(json!({"cmd": "type", "text": "echo alpha beta\r"}));
    otto.wait_for("the output", |o| Some(term1(o)).filter(|t| shows(t, "alpha beta")));
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));

    // ⌘⇧C puts a cursor at the terminal cursor and drops any mouse selection; pane actions wait.
    otto.call(json!({"cmd": "mouse_down", "x": 0.25, "y": 0.5, "clicks": 2}));
    let reply = key("cmd-shift-c");
    assert_eq!((reply["mode"].as_str(), reply["state"]["visual"].as_u64()), (Some("visual"), Some(1)));
    assert_eq!(reply["terminals"]["1"]["selection"], Value::Null);
    let line = reply["terminals"]["1"]["visual_cursor"][0].as_i64().expect("a cursor");
    assert_eq!(pane_ids(&otto.call(json!({"cmd": "split"}))), [1]);

    // Up to the output line, to its start, select to the end of the word, copy and leave.
    let reply = key("k 0 v e");
    assert_eq!(reply["terminals"]["1"]["visual_cursor"], json!([line - 1, 4]));
    assert_eq!(reply["terminals"]["1"]["selection"], "alpha");
    let reply = key("y");
    assert_eq!((reply["mode"].as_str(), reply["copied"].as_str()), (Some("normal"), Some("alpha")));
    assert_eq!(reply["terminals"]["1"]["selection"], Value::Null);
    assert_eq!(reply["terminals"]["1"]["visual_cursor"], Value::Null);

    // ⌘C copies in visual mode too, and stays in it. A click leaves it.
    let reply = key("cmd-shift-c k 0 w v e cmd-c");
    assert_eq!((reply["mode"].as_str(), reply["copied"].as_str()), (Some("visual"), Some("beta")));
    let reply = otto.call(json!({"cmd": "mouse_down", "x": 0.25, "y": 0.5}));
    assert_eq!((reply["mode"].as_str(), reply["state"]["visual"].as_u64()), (Some("normal"), None));

    // Esc leaves without copying; typed keys never reach the shell while in visual mode.
    let reply = key("cmd-shift-c w v $ escape");
    assert_eq!((reply["mode"].as_str(), reply["copied"].as_str()), (Some("normal"), Some("beta")));
    otto.call(json!({"cmd": "type", "text": "echo done\r"}));
    otto.wait_for("the shell to be untouched", |o| Some(term1(o)).filter(|t| shows(t, "done")));
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

/// The open text field as the state gives it: (text, cursor, selection start, selection end).
fn field(reply: &Value) -> (String, u64, u64, u64) {
    let f = &reply["field"];
    let n = |v: &Value| v.as_u64().unwrap_or(u64::MAX);
    (f["text"].as_str().unwrap_or("<none>").to_string(), n(&f["cursor"]), n(&f["selection"][0]), n(&f["selection"][1]))
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_text_field_moves_selects_and_undoes() {
    let otto = Otto::launch("field");
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    let at = |text: &str, cursor: u64, from: u64, to: u64| (text.to_string(), cursor, from, to);
    assert_eq!(key("cmd-n")["field"]["of"], "compose");
    // Typed text goes in at the cursor, which moves by character, word and line.
    assert_eq!(field(&key("f i x space z o o m")), at("fix zoom", 8, 8, 8));
    assert_eq!(field(&key("left left")), at("fix zoom", 6, 6, 6));
    assert_eq!(field(&key("shift-X")), at("fix zoXom", 7, 7, 7));
    assert_eq!(field(&key("alt-left")), at("fix zoXom", 4, 4, 4));
    assert_eq!(field(&key("cmd-left")), at("fix zoXom", 0, 0, 0));
    assert_eq!(field(&key("ctrl-e")), at("fix zoXom", 9, 9, 9));
    assert_eq!(field(&key("home alt-right ctrl-f")), at("fix zoXom", 4, 4, 4));
    assert_eq!(field(&key("end ctrl-a ctrl-b right")), at("fix zoXom", 1, 1, 1));
    // Shift extends the selection; typing replaces it. The state's `compose` is still the text.
    let reply = key("end shift-left shift-left shift-left");
    assert_eq!((field(&reply), reply["field"]["selected"].as_str()), (at("fix zoXom", 6, 6, 9), Some("Xom")));
    let reply = key("o m");
    assert_eq!((field(&reply), reply["state"]["compose"].as_str()), (at("fix zoom", 8, 8, 8), Some("fix zoom")));
    assert_eq!(field(&key("alt-shift-left")), at("fix zoom", 4, 4, 8));
    assert_eq!(field(&key("cmd-shift-left")), at("fix zoom", 0, 0, 8));
    // An arrow leaves a selection at that side. ⌘A then a key replaces everything.
    assert_eq!(field(&key("right")), at("fix zoom", 8, 8, 8));
    assert_eq!(field(&key("cmd-a")), at("fix zoom", 8, 0, 8));
    assert_eq!(field(&key("a")), at("a", 1, 1, 1));
    // ⌘Z takes back a step, ⇧⌘Z does it again. ⌥⌫ takes a word, ⌘⌫ the line, ⌦ what follows.
    assert_eq!(field(&key("cmd-z")), at("fix zoom", 8, 0, 8));
    assert_eq!(field(&key("cmd-shift-z")), at("a", 1, 1, 1));
    assert_eq!(field(&key("cmd-z right space n o w")), at("fix zoom now", 12, 12, 12));
    assert_eq!(field(&key("alt-backspace")), at("fix zoom ", 9, 9, 9));
    assert_eq!(field(&key("cmd-z")), at("fix zoom now", 12, 12, 12));
    assert_eq!(field(&key("alt-left alt-left alt-delete")), at("fix  now", 4, 4, 4));
    assert_eq!(field(&key("delete backspace")), at("fixnow", 3, 3, 3));
    assert_eq!(field(&key("cmd-backspace")), at("now", 0, 0, 0));
    assert_eq!(field(&key("ctrl-k")), at("", 0, 0, 0));
    // Each undo also puts the cursor back where it was.
    assert_eq!(field(&key("cmd-z cmd-z cmd-z cmd-z cmd-z")), at("fix zoom now", 4, 4, 4));
    // ⌘C, ⌘X and ⌘V: in a test run the clipboard is the app's own (`copied`), never the user's.
    let reply = key("end alt-shift-left cmd-c");
    assert_eq!((reply["copied"].as_str(), field(&reply)), (Some("now"), at("fix zoom now", 9, 9, 12)));
    assert_eq!(field(&key("left cmd-v")), at("fix zoom nownow", 12, 12, 12));
    let reply = key("cmd-a cmd-x");
    assert_eq!((reply["copied"].as_str(), field(&reply)), (Some("fix zoom nownow"), at("", 0, 0, 0)));
    assert_eq!(field(&key("cmd-v cmd-v")).0, "fix zoom nownowfix zoom nownow");
    // An emoji is one step, two UTF-16 units; ⌃X types nothing.
    key("cmd-a backspace");
    otto.call(json!({"cmd": "key", "keys": "a"}));
    assert_eq!(key("escape")["field"], Value::Null);
    otto.call(json!({"cmd": "compose", "text": "a👍🏽b"}));
    assert_eq!(field(&key("left left")), at("a👍🏽b", 1, 1, 1));
    assert_eq!(field(&key("shift-right ctrl-x")), at("a👍🏽b", 5, 1, 5));
    assert_eq!(field(&key("backspace")), at("ab", 1, 1, 1));
    key("escape");

    // The mouse: a click places the cursor, a double click takes a word, a triple click the
    // line, a drag selects, and goes on past the field's edge.
    otto.call(json!({"cmd": "compose", "text": "fix zoom flicker"}));
    let drawn = otto.drawn();
    let (b, end) = (rect(&drawn["field"]["bounds"]), drawn["field"]["cursor_x"].as_f64().unwrap());
    assert!(b[2] > 100.0 && end > b[0] + 40.0, "the field is drawn, the caret after the text: {}", drawn["field"]);
    let (y, width) = (b[1] + b[3] / 2.0, end - b[0]);
    // "zoom" is a quarter to half of the way along; "fix" starts the line.
    assert_eq!(field(&otto.click([b[0] + width * 0.4, y], 2)), at("fix zoom flicker", 8, 4, 8));
    assert_eq!(field(&otto.click([b[0] + width * 0.4, y], 3)), at("fix zoom flicker", 16, 0, 16));
    assert_eq!(field(&otto.click([b[0] + 1.0, y], 1)), at("fix zoom flicker", 0, 0, 0));
    assert_eq!(field(&otto.click([end + 30.0, y], 1)), at("fix zoom flicker", 16, 16, 16));
    let reply = otto.drag([b[0] + 1.0, y], [end + 40.0, y + 60.0]);
    assert_eq!((field(&reply), reply["mode"].as_str()), (at("fix zoom flicker", 16, 0, 16), Some("compose")));
    let reply = otto.drag([end + 30.0, y], [b[0] - 30.0, y]);
    assert_eq!(field(&reply), at("fix zoom flicker", 0, 0, 16));
    // ⇧-click selects from the cursor, and a drag after it goes on from there: not from the
    // press before (at the end of the line), which the keys have long left.
    otto.click([end + 30.0, y], 1);
    key("cmd-left right right right");
    let shift = |action: &str, x: f64| {
        otto.call(json!({"cmd": "mouse", "action": action, "x": x, "y": y, "modifiers": ["shift"]}))
    };
    let (text, cursor, from, to) = field(&shift("down", b[0] + width * 0.5));
    assert_eq!((text.as_str(), from, cursor == to), ("fix zoom flicker", 3, true));
    let (_, cursor, from, further) = field(&shift("move", b[0] + width * 0.7));
    assert!(from == 3 && further > to && cursor == further, "3..{to} grew to {from}..{further}");
    shift("up", b[0] + width * 0.7);
    // The text shrinks under a press: ⌘A and a letter, then ⇧-click and a move of two points.
    otto.click([end + 30.0, y], 1);
    key("cmd-a x");
    shift("down", end);
    let reply = shift("move", end + 2.0);
    shift("up", end + 2.0);
    assert_eq!(field(&reply), at("x", 1, 1, 1));
    key("cmd-z");
    // The selection is replaced by what is typed next, and ↵ adds the task.
    key("cmd-a");
    key("t i d y");
    let reply = key("enter");
    assert_eq!(
        (reply["mode"].as_str(), reply["state"]["tasks"]["list"][0]["title"].as_str()),
        (Some("normal"), Some("tidy"))
    );

    // A line wider than the box scrolls: the caret stays inside it.
    let long = "a long line of words that goes on and on ".repeat(4);
    otto.call(json!({"cmd": "compose", "text": long.trim()}));
    let drawn = otto.drawn();
    let (b, x) = (rect(&drawn["field"]["bounds"]), drawn["field"]["cursor_x"].as_f64().unwrap());
    assert!(x > b[0] + b[2] - 20.0 && x <= b[0] + b[2], "the caret is at the right edge: {}", drawn["field"]);
    key("cmd-left");
    assert_eq!(otto.drawn()["field"]["cursor_x"].as_f64(), Some(b[0]));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_picker_filter_is_a_field_and_offers_apps_that_are_not_running() {
    // Made-up apps in a folder of the test's own: never the user's, and none is ever started.
    let root = std::env::temp_dir().join(format!("ottospace-e2e-apps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for app in ["Sketchpad.app", "Skylark.app", "Lumen Desk.app", "notes.txt"] {
        std::fs::create_dir_all(root.join(app)).unwrap();
    }
    let otto = Otto::launch_with("picker-apps", &[("OTTOSPACE_APPS_DIRS", root.to_str().unwrap())]);
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    let rows = |reply: &Value| -> Vec<String> {
        let row = |row: &Value| format!("{} {}", row["type"].as_str().unwrap(), row["name"].as_str().unwrap_or(""));
        reply["picker_rows"].as_array().map(|rows| rows.iter().map(row).collect()).unwrap_or_default()
    };
    // Nothing typed: what the picker showed before. The folders are read on another thread.
    let reply = otto.call(json!({"cmd": "picker"}));
    assert_eq!((rows(&reply), reply["field"]["of"].as_str()), (vec!["terminal ".to_string()], Some("picker")));
    otto.wait_for("the scan of the apps folder", |o| {
        let reply = o.call(json!({"cmd": "picker_query", "query": "sk"}));
        (rows(&reply) == ["closed Sketchpad", "closed Skylark", "closed Lumen Desk"]).then_some(())
    });
    // The filter edits as a field does; names that start with it come before those that hold it.
    let reply = key("cmd-a d e s");
    assert_eq!((field(&reply).0.as_str(), rows(&reply)), ("des", vec!["closed Lumen Desk".to_string()]));
    let reply = key("shift-left shift-left shift-left k");
    assert_eq!(rows(&reply), ["closed Lumen Desk", "closed Sketchpad", "closed Skylark"]);
    let reply = key("cmd-z");
    assert_eq!((field(&reply), rows(&reply).len()), (("des".to_string(), 0, 0, 3), 1));
    // The terminal and running apps come first. ↓ moves as before; Esc closes.
    let reply = key("t");
    assert_eq!(rows(&reply), ["terminal ", "closed Sketchpad"]);
    assert_eq!(key("down")["state"]["picker"]["selected"], 1);
    let reply = key("escape");
    assert_eq!(
        (reply["mode"].as_str(), &reply["field"], &reply["picker_rows"]),
        (Some("normal"), &Value::Null, &Value::Null)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn picker_opens_a_terminal_in_the_chosen_direction() {
    let otto = Otto::launch("picker");
    // An empty pane is filled by the pick, not split.
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "picker"}));
    let reply = otto.call(json!({"cmd": "picker_choose"}));
    assert_eq!(pane_ids(&reply), [1]);
    assert_eq!(reply["state"]["panes"][0]["kind"]["type"], "terminal");

    let reply = otto.call(json!({"cmd": "picker", "axis": "down"}));
    assert_eq!(reply["mode"], "pick");
    assert_eq!(reply["state"]["picker"]["items"][0]["type"], "terminal");

    // A query with no match keeps the picker open; a matching one opens the pane and closes it.
    // Pane requests wait while the picker is open; any move delta is safe.
    assert_eq!(pane_ids(&otto.call(json!({"cmd": "close"}))), [1]);
    otto.call(json!({"cmd": "picker_move", "delta": 2147483647}));
    otto.call(json!({"cmd": "picker_query", "query": "zzz"}));
    assert_eq!(otto.call(json!({"cmd": "picker_choose"}))["mode"], "pick");
    otto.call(json!({"cmd": "picker_query", "query": "term"}));
    let reply = otto.call(json!({"cmd": "picker_choose"}));
    assert_eq!((reply["mode"].as_str(), reply["state"]["picker"].is_null()), (Some("normal"), true));
    assert_eq!(reply["state"]["panes"][1]["kind"]["type"], "terminal");

    let [first, second] = otto.bounds(&[1, 2])[..] else { panic!("two panes") };
    assert!(first[1] + first[3] <= second[1], "the picked pane is below: {first:?} {second:?}");

    otto.call(json!({"cmd": "picker"}));
    assert_eq!(otto.call(json!({"cmd": "picker_close"}))["mode"], "normal");

    // The key reference shows and hides.
    assert_eq!(otto.call(json!({"cmd": "keys"}))["state"]["keys"], true);
    assert_eq!(otto.call(json!({"cmd": "keys"}))["state"]["keys"], false);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn tabs_keep_their_own_panes() {
    let otto = Otto::launch("tabs");
    otto.call(json!({"cmd": "split"})); // pane 1, a terminal, in tab 1
    let first = otto.bounds(&[1])[0];

    let reply = otto.call(json!({"cmd": "new_tab"}));
    assert_eq!((reply["state"]["tabs"].as_array().unwrap().len(), reply["state"]["active"].as_u64()), (2, Some(1)));
    assert!(reply["bounds"].get("1").is_none(), "a background pane reports no bounds: {reply}");
    assert_eq!(reply["tab_panes"], json!([[1], []]), "the dump says which tab holds each pane");
    otto.call(json!({"cmd": "split", "kind": "empty"})); // pane 2 in tab 2
    let second = otto.bounds(&[2])[0];
    assert!((first[2] - second[2]).abs() < 2.0, "a lone pane fills the canvas in either tab");

    // Back to tab 1: its pane and focus return.
    let reply = otto.call(json!({"cmd": "select_tab", "index": 0}));
    assert_eq!(reply["state"]["focused"], 1);
    otto.bounds(&[1]);

    // The picker can open its pick in a new tab.
    otto.call(json!({"cmd": "picker", "new_tab": true}));
    let reply = otto.call(json!({"cmd": "picker_choose"}));
    assert_eq!((reply["state"]["tabs"].as_array().unwrap().len(), reply["state"]["active"].as_u64()), (3, Some(2)));
    assert_eq!(reply["state"]["focused"], 3);

    // Closing a tab closes its panes and ends their shells.
    let reply = otto.call(json!({"cmd": "close_tab"}));
    assert_eq!(pane_ids(&reply), [1, 2]);
    assert!(reply["terminals"].get("3").is_none(), "{reply}");
    let reply = otto.call(json!({"cmd": "cycle_tab", "delta": 1}));
    assert_eq!(reply["state"]["tabs"].as_array().unwrap().len(), 2);
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

    // Accessibility requests need an app pane: others are refused, and nothing toggles.
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    for bad in [r#"{"cmd":"ax_tree"}"#, r#"{"cmd":"ax_tree","pane":1}"#, r#"{"cmd":"ax_overlay"}"#] {
        let reply = otto.raw(bad);
        assert_eq!(reply["ok"], false, "{bad} -> {reply}");
        assert!(reply["error"].as_str().is_some_and(|e| e.contains("app pane")), "{bad} -> {reply}");
    }
    assert_eq!(otto.raw(r#"{"cmd":"ax_tree","limit":1}"#)["ok"], false, "unknown field");
    let reply = otto.call(json!({"cmd": "state"}));
    assert_eq!((&reply["state"]["ax_overlay"], &reply["ax_overlay"]), (&json!(false), &Value::Null), "{reply}");
    otto.call(json!({"cmd": "close"}));

    // A second app whose socket path names an ordinary file must leave that file alone.
    let file = std::env::temp_dir().join(format!("ottospace-e2e-notasocket-{}", std::process::id()));
    std::fs::write(&file, "keep me").expect("write");
    let mut other = Command::new(env!("CARGO_BIN_EXE_ottospace"))
        .env("OTTOSPACE_E2E", "1")
        .env("OTTOSPACE_SOCKET", &file)
        .env_remove("OTTOSPACE_HOME") // never the user's settings or tasks
        .env("OTTOSPACE_REPOS", "")
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
    exits(&mut otto);
    assert!(!otto.socket.exists(), "socket file is removed on quit");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn cmd_q_quits_at_once_but_asks_while_a_terminal_runs() {
    let key = |o: &Otto, keys: &str| o.call(json!({"cmd": "key", "keys": keys}));
    // Only a shell at its prompt: no question.
    let mut idle = Otto::launch("quit-idle");
    idle.call(json!({"cmd": "split"}));
    idle.call(json!({"cmd": "type", "text": "echo up-$((1+1))\r"}));
    idle.call(json!({"cmd": "wait", "pattern": "^up-2$", "timeout_ms": 30000}));
    key(&idle, "cmd-q");
    exits(&mut idle);
    assert!(!idle.socket.exists(), "socket file is removed on quit");

    // Pane 1 at its prompt, pane 2 running `sleep`: the question names pane 2 only.
    let mut otto = Otto::launch("quit-busy");
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "type", "text": "echo up-$((1+1))\r"}));
    otto.call(json!({"cmd": "wait", "pattern": "^up-2$", "timeout_ms": 30000}));
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "type", "pane": 2, "text": "sh -c 'echo go; exec sleep 30'\r"}));
    otto.call(json!({"cmd": "wait", "pane": 2, "pattern": "^go$", "timeout_ms": 30000}));
    let sleep = json!([{"pane": 2, "program": "sleep", "background": false}]);
    // `go` came from the inner `sh`, which is in front; a moment later it is `sleep`.
    let asked = asks(&otto, &sleep);
    assert_eq!(asked["mode"], "confirm", "{asked}");

    // Esc: no quit, nothing closed, and the key did not reach the terminal.
    let before = otto.call(json!({"cmd": "capture", "pane": 2}))["text"].to_string();
    let reply = key(&otto, "escape");
    assert_eq!((&reply["mode"], &reply["state"]["confirm"]), (&json!("normal"), &Value::Null), "{reply}");
    // The question is the user's. A client's key answers it (↵, Q, Esc) or is refused: not
    // typed, not pasted, no pane closed, and the question stays. So is a client's click, and
    // a request that would take the keyboard.
    assert_eq!(key(&otto, "cmd-q")["state"]["confirm"]["running"], sleep);
    let refused = |request: Value| {
        let reply = otto.raw(&request.to_string());
        assert_eq!(reply["error"], "the quit question is up", "{request}: {reply}");
        let state = otto.call(json!({"cmd": "state"}));
        assert_eq!((&state["mode"], &state["state"]["confirm"]["running"]), (&json!("confirm"), &sleep), "{request}");
        assert_eq!(pane_ids(&state), vec![1, 2], "{request}: {state}");
    };
    for other in ["x", "cmd-v", "cmd-c", "cmd-w", "cmd-h", "cmd-ctrl-f", "cmd-shift-q", "shift-enter", "cmd-q"] {
        refused(json!({"cmd": "key", "keys": other}));
    }
    refused(json!({"cmd": "mouse_down", "pane": 2, "x": 1, "y": 1}));
    refused(json!({"cmd": "keys"}));
    refused(json!({"cmd": "picker"}));
    std::thread::sleep(Duration::from_millis(300));
    let after = otto.call(json!({"cmd": "capture", "pane": 2}))["text"].to_string();
    assert!(otto.child.try_wait().expect("wait").is_none(), "a refused key quit");
    assert_eq!(before, after, "a key reached the terminal under the question");
    // A client's pane request runs underneath, and the question stays.
    let reply = otto.call(json!({"cmd": "focus", "pane": 1}));
    assert_eq!((&reply["mode"], &reply["state"]["focused"]), (&json!("confirm"), &json!(1)), "{reply}");
    assert_eq!(key(&otto, "escape")["mode"], "normal");

    // A background job is ended by a quit too: the shell is in front, and is asked about.
    otto.call(json!({"cmd": "type", "pane": 1, "text": "sleep 100 &\r"}));
    otto.call(json!({"cmd": "type", "pane": 1, "text": "echo bg-$((3+3))\r"}));
    otto.call(json!({"cmd": "wait", "pane": 1, "pattern": "^bg-6$", "timeout_ms": 30000}));
    let both = json!([
        {"pane": 1, "program": "sleep", "background": true},
        {"pane": 2, "program": "sleep", "background": false},
    ]);
    asks(&otto, &both);
    // A pane that closes while the question is up is no longer asked about.
    let reply = otto.call(json!({"cmd": "close", "pane": 1}));
    assert_eq!((&reply["mode"], &reply["state"]["confirm"]["running"]), (&json!("confirm"), &sleep), "{reply}");
    // The last one closing ends the question, and does not quit.
    let reply = otto.call(json!({"cmd": "close", "pane": 2}));
    assert_eq!((&reply["mode"], &reply["state"]["confirm"]), (&json!("normal"), &Value::Null), "{reply}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(otto.child.try_wait().expect("wait").is_none(), "the question going away quit");

    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "type", "text": "sh -c 'echo go; exec sleep 30'\r"}));
    otto.call(json!({"cmd": "wait", "pattern": "^go$", "timeout_ms": 30000}));
    asks(&otto, &json!([{"pane": 3, "program": "sleep", "background": false}]));
    key(&otto, "enter");
    exits(&mut otto);
    assert!(!otto.socket.exists(), "socket file is removed on quit");
}

/// Presses ⌘Q until the question lists exactly `running`, cancelling a question that lists
/// something else (a program still starting). Something must run already, or this quits.
fn asks(otto: &Otto, running: &Value) -> Value {
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    otto.wait_for("the quit question", |_| {
        let reply = key("cmd-q");
        let found = &reply["state"]["confirm"]["running"] == running;
        if !found {
            key("escape");
        }
        found.then_some(reply)
    })
}

/// The default path: no `OTTOSPACE_SHELL`, so the terminal's child is `login` and the shell is
/// its child, in its own process group. An idle prompt there must not count as a program.
#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn cmd_q_knows_an_idle_shell_behind_login() {
    let dir = std::env::temp_dir().join(format!("ottospace-e2e-login-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("make the home");
    // Up, and really behind login. The second line is a builtin: when it has printed, `ps` is
    // gone and the shell is in front again.
    let ready = |o: &Otto| {
        o.call(json!({"cmd": "split"}));
        o.call(json!({"cmd": "type", "text": "ps -o comm= -p $PPID; echo up-$((1+1))\r"}));
        o.call(json!({"cmd": "wait", "pattern": "^/usr/bin/login$", "timeout_ms": 30000}));
        o.call(json!({"cmd": "wait", "pattern": "^up-2$", "timeout_ms": 30000}));
    };
    let mut idle = Otto::launch_login("quit-login-idle", &dir);
    ready(&idle);
    idle.call(json!({"cmd": "key", "keys": "cmd-q"}));
    exits(&mut idle);

    let mut busy = Otto::launch_login("quit-login-busy", &dir);
    ready(&busy);
    busy.call(json!({"cmd": "type", "text": "sh -c 'echo go; exec sleep 30'\r"}));
    busy.call(json!({"cmd": "wait", "pattern": "^go$", "timeout_ms": 30000}));
    asks(&busy, &json!([{"pane": 1, "program": "sleep", "background": false}]));
    busy.call(json!({"cmd": "key", "keys": "enter"}));
    exits(&mut busy);
    let _ = std::fs::remove_dir_all(&dir);
}

type Labels = std::collections::BTreeMap<String, String>;

/// Each labelled control's label, by the control's id, in a reply taken in sneak mode.
fn sneak_labels(reply: &Value) -> Labels {
    assert_eq!(reply["mode"], "sneak", "{reply}");
    let targets = reply["sneak"]["targets"].as_array().expect("targets");
    let label = |t: &Value| Some((t["source"]["id"].as_str()?.to_string(), t["label"].as_str()?.to_string()));
    targets.iter().map(|t| label(t).expect("an id and a label")).collect()
}

/// Enters sneak mode: the reply, and each control's label by the control's id.
fn sneak(o: &Otto) -> (Value, Labels) {
    let reply = o.call(json!({"cmd": "sneak"}));
    let labels = sneak_labels(&reply);
    (reply, labels)
}

/// A label as `key` takes it: a keystroke a letter.
fn letters(label: &str) -> String {
    label.chars().map(String::from).collect::<Vec<_>>().join(" ")
}

/// Enters sneak mode and types the label of the control `id`, a key at a time.
fn sneak_to(o: &Otto, id: &str) -> Value {
    let (_, labels) = sneak(o);
    let label = labels.get(id).unwrap_or_else(|| panic!("no label for {id}: {labels:?}"));
    o.call(json!({"cmd": "key", "keys": letters(label)}))
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn sneak_mode_labels_the_controls_and_a_typed_label_clicks_one() {
    let root = std::env::temp_dir().join(format!("ottospace-e2e-sneak-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    fake_repo(&root.join("app"));
    let repo = root.canonicalize().unwrap().join("app").to_string_lossy().into_owned();
    let otto = Otto::launch_with("sneak", &[("OTTOSPACE_REPOS", &root.to_string_lossy())]);
    otto.call(json!({"cmd": "task_add", "title": "review the fix", "state": "needs_you"}));

    // The home screen with the Repos bar open: every control drawn has a label, and one each.
    otto.call(json!({"cmd": "section", "section": "repos"}));
    let (reply, labels) = sneak(&otto);
    let ids: Vec<&str> = labels.keys().map(String::as_str).collect();
    let row = format!("repo:{repo}");
    for id in ["rail:apps", "rail:workbook", "rail:tasks", "rail:repos", "bar:refresh", "bar:close", row.as_str()] {
        assert!(ids.contains(&id), "no label for {id}: {ids:?}");
    }
    for id in ["tab:1", "tab-close:1", "top:new_tab", "top:split_right", "top:split_down"] {
        assert!(ids.contains(&id), "no label for {id}: {ids:?}");
    }
    for id in ["home:new_task", "home-row:1", "home-node:1"] {
        assert!(ids.contains(&id), "no label for {id}: {ids:?}");
    }
    let mut unique: Vec<&String> = labels.values().collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), labels.len(), "a label names one control: {labels:?}");
    // Each label is on its control, inside the window.
    let window = reply["window"].as_array().expect("window");
    for t in reply["sneak"]["targets"].as_array().unwrap() {
        let f = &t["frame"];
        let (x, y, w, h) = (f["x"].as_f64(), f["y"].as_f64(), f["w"].as_f64(), f["h"].as_f64());
        let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else { panic!("frame of {t}") };
        assert!(x >= 0.0 && y >= 0.0 && w > 1.0 && h > 1.0, "{t}");
        assert!(x + w <= window[2].as_f64().unwrap() + 1.0 && y + h <= window[3].as_f64().unwrap() + 1.0, "{t}");
    }

    // A × never has a one-letter label: one stray key closes nothing.
    assert_eq!(labels["tab-close:1"].len(), 2, "{labels:?}");
    // While the labels are up, a client may type a label, ⌫, Esc or the mode's key. Any other
    // key, a ⌘ shortcut too, and the pointer are refused and change nothing: they would end the
    // user's mode, or do something under it.
    let refused = |request: Value| {
        let reply = otto.raw(&request.to_string());
        assert_eq!((&reply["ok"], &reply["error"]), (&json!(false), &json!("sneak mode is on")), "{request}");
    };
    for keys in ["1", "left", "enter tab", "cmd-c", "cmd-/", "cmd-w", "ctrl-a"] {
        refused(json!({"cmd": "key", "keys": keys}));
    }
    refused(json!({"cmd": "mouse_down", "x": 1, "y": 1}));
    refused(json!({"cmd": "mouse", "action": "down", "x": 5, "y": 5}));
    refused(json!({"cmd": "drag", "from": [5, 5], "to": [50, 50]}));
    let before = otto.call(json!({"cmd": "state"}));
    assert_eq!((&before["mode"], &before["sneak"]["typed"]), (&json!("sneak"), &json!("")));
    assert_eq!((&before["state"]["keys"], sneak_labels(&before)), (&json!(false), labels.clone()));
    // Asking again while it is on changes nothing: the same labels.
    assert_eq!(sneak(&otto).1, labels);
    let after = otto.call(json!({"cmd": "key", "keys": "escape"}));
    assert_eq!((&after["mode"], &after["sneak"], &after["state"]), (&json!("normal"), &Value::Null, &before["state"]));
    // A letter of no label leaves too, and nothing has changed.
    sneak(&otto);
    let after = otto.call(json!({"cmd": "key", "keys": "z"}));
    assert_eq!((&after["mode"], &after["sneak"], &after["state"]), (&json!("normal"), &Value::Null, &before["state"]));
    // The mode's own key enters, and leaves.
    assert_eq!(otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-g"}))["mode"], "sneak");
    assert_eq!(otto.call(json!({"cmd": "key", "keys": "cmd-ctrl-g"}))["mode"], "normal");

    // A rail icon opens its section, where the task has a row; the bar's × closes it.
    let reply = sneak_to(&otto, "rail:tasks");
    assert_eq!((&reply["mode"], &reply["state"]["sidebar"]), (&json!("normal"), &json!("tasks")), "{reply}");
    assert!(sneak(&otto).1.contains_key("task:1"));
    assert_eq!(sneak_to(&otto, "bar:close")["state"]["sidebar"], Value::Null);

    // The task's row on the home screen shows its tabs; the chip's × goes back. Its node does the same.
    assert_eq!(sneak_to(&otto, "home-row:1")["state"]["current_task"], 1);
    assert_eq!(sneak_to(&otto, "top:task_leave")["state"]["current_task"], Value::Null);
    assert_eq!(sneak_to(&otto, "home-node:1")["state"]["current_task"], 1);
    otto.call(json!({"cmd": "task_leave"}));
    // The new-task line opens the box, a top-bar button the picker. Neither lets sneak mode start.
    assert_eq!(sneak_to(&otto, "home:new_task")["mode"], "compose");
    let refused = otto.raw(r#"{"cmd":"sneak"}"#);
    assert!(refused["error"].as_str().is_some_and(|e| e.contains("normal or list")), "{refused}");
    otto.call(json!({"cmd": "key", "keys": "escape"}));
    assert_eq!(sneak_to(&otto, "top:split_down")["state"]["picker"]["axis"], "down");
    assert_eq!(otto.raw(r#"{"cmd":"sneak"}"#)["ok"], false);
    otto.call(json!({"cmd": "picker_close"}));

    // A repository's row opens a terminal there.
    otto.call(json!({"cmd": "section", "section": "repos"}));
    assert_eq!(sneak_to(&otto, &row)["state"]["panes"][0]["cwd"], repo);
    otto.call(json!({"cmd": "section", "section": "repos"}));

    // 1 | 2    and a second tab.
    //   | 3
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "down", "kind": "empty"}));
    // The task has a tab too, off the bar: the new tab is the last of three.
    let tabs = otto.call(json!({"cmd": "new_tab"}))["state"]["tabs"].clone();
    let (place, tab) = (tabs.as_array().unwrap().len() - 1, tabs.as_array().unwrap().last().unwrap()["id"].clone());
    let (tab, tab_close) = (format!("tab:{tab}"), format!("tab-close:{tab}"));
    otto.call(json!({"cmd": "select_tab", "index": 0}));
    let drawn = otto.bounds(&[1, 2, 3]);
    let (reply, labels) = sneak(&otto);
    for id in ["header:1", "close:1", "pane:1", "header:3", "close:3", "pane:3", tab.as_str(), tab_close.as_str()] {
        assert!(labels.contains_key(id), "no label for {id}: {labels:?}");
    }
    assert!(!labels.contains_key("home:new_task"), "the home screen is gone: {labels:?}");
    // A pane's own label covers its body.
    let body = reply["sneak"]["targets"].as_array().unwrap().iter().find(|t| t["source"]["id"] == "pane:2").unwrap();
    assert!((body["frame"]["x"].as_f64().unwrap() - drawn[1][0]).abs() < 1.0, "{body} {drawn:?}");
    assert!((body["frame"]["w"].as_f64().unwrap() - drawn[1][2]).abs() < 1.0, "{body} {drawn:?}");
    otto.call(json!({"cmd": "key", "keys": "escape"}));

    // A pane's label, or its header's, moves focus there. Its × closes it.
    assert_eq!(otto.call(json!({"cmd": "state"}))["state"]["focused"], 3);
    assert_eq!(sneak_to(&otto, "pane:1")["state"]["focused"], 1);
    assert_eq!(sneak_to(&otto, "header:2")["state"]["focused"], 2);
    let reply = sneak_to(&otto, "close:3");
    assert_eq!((pane_ids(&reply), &reply["mode"]), (vec![1, 2], &json!("normal")));
    // A tab's label selects it; its × closes it.
    assert_eq!(sneak_to(&otto, &tab)["state"]["active"], place);
    let reply = sneak_to(&otto, &tab_close);
    assert_eq!((reply["state"]["tabs"].as_array().unwrap().len(), pane_ids(&reply)), (place, vec![1, 2]));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_two_letter_sneak_label_narrows_then_activates() {
    let otto = Otto::launch("sneak-two");
    // Three panes, two tabs and the bar: more controls than the alphabet has letters.
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    otto.call(json!({"cmd": "split", "axis": "down", "kind": "empty"}));
    otto.call(json!({"cmd": "new_tab"}));
    otto.call(json!({"cmd": "select_tab", "index": 0}));
    otto.call(json!({"cmd": "section", "section": "marks"})); // one control: the rail has a fifth icon now
    otto.bounds(&[1, 2, 3]);
    otto.call(json!({"cmd": "focus", "pane": 1}));

    let (_, labels) = sneak(&otto);
    assert!(labels.len() > 19, "{labels:?}");
    assert!(labels.values().any(|l| l.len() == 1) && labels.values().all(|l| l.len() <= 2), "{labels:?}");
    for (a, b) in labels.values().flat_map(|a| labels.values().map(move |b| (a, b))) {
        assert!(a == b || !b.starts_with(a.as_str()), "{a} is a prefix of {b}");
    }
    // Each × has two letters, however many one-letter labels are left over; no other control does here.
    for (id, label) in &labels {
        let destructive = id.starts_with("close:") || id.starts_with("tab-close:");
        assert_eq!(label.len(), if destructive { 2 } else { 1 }, "{id} in {labels:?}");
    }
    let (first, second) = labels["close:3"].split_at(1);

    // The first letter narrows and waits; ⌫ takes it back.
    let reply = otto.call(json!({"cmd": "key", "keys": first}));
    let narrowed = (&reply["mode"], &reply["sneak"]["typed"], pane_ids(&reply));
    assert_eq!(narrowed, (&json!("sneak"), &json!(first), vec![1, 2, 3]));
    let reply = otto.call(json!({"cmd": "key", "keys": "backspace"}));
    assert_eq!((&reply["mode"], &reply["sneak"]["typed"]), (&json!("sneak"), &json!("")));
    // The second letter activates.
    let reply = otto.call(json!({"cmd": "key", "keys": format!("{first} {second}")}));
    assert_eq!((&reply["mode"], &reply["sneak"], pane_ids(&reply)), (&json!("normal"), &Value::Null, vec![1, 2]));
}

/// One side of the labelled control `id`'s frame: `x`, `y`, `w` or `h`.
fn label_frame(reply: &Value, id: &str, side: &str) -> Option<f64> {
    reply["sneak"]["targets"].as_array()?.iter().find(|t| t["source"]["id"] == id)?["frame"][side].as_f64()
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_sneak_label_follows_its_control_and_never_reaches_the_one_that_took_its_place() {
    let otto = Otto::launch("sneak-drawn");
    // Three tabs of no task; the second has a pane to lose.
    otto.call(json!({"cmd": "new_tab"}));
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    let state = otto.call(json!({"cmd": "new_tab"}));
    let ids: Vec<u64> = state["state"]["tabs"].as_array().unwrap().iter().filter_map(|t| t["id"].as_u64()).collect();
    let [_, second, third] = ids[..] else { panic!("three tabs: {ids:?}") };
    let (second_close, third_close) = (format!("tab-close:{second}"), format!("tab-close:{third}"));

    let (before, labels) = sneak(&otto);
    let (gone, kept) = (labels[&second_close].clone(), labels[&third_close].clone());
    assert_eq!((gone.len(), kept.len()), (2, 2), "a × has two letters: {labels:?}");

    // An agent files a task with the second tab: news, so the mode stays on. The tab leaves the
    // bar, and the third slides under the pill that was on the second's ×.
    otto.call(json!({"cmd": "task_add", "title": "an agent's task", "tab": second}));
    let after = otto.wait_for("the labels to follow the new frame", |o| {
        let reply = o.call(json!({"cmd": "state"}));
        (!sneak_labels(&reply).contains_key(&second_close)).then_some(reply)
    });
    let now = sneak_labels(&after);
    assert_eq!(now.get(&third_close), Some(&kept), "the same letters on the same control");
    let x = |reply: &Value| label_frame(reply, &third_close, "x").unwrap();
    let (was, is) = (x(&before), x(&after));
    assert!(is < was - 10.0, "its pill moved left with it: {was} -> {is}");
    assert!(now.len() < labels.len() && now.iter().all(|(id, label)| labels.get(id) == Some(label)), "{now:?}");

    // The letters that were on the second tab's × close nothing: not the hidden tab, not the
    // third. They are no label now, so the mode ends.
    let reply = otto.call(json!({"cmd": "key", "keys": letters(&gone)}));
    assert_eq!((&reply["mode"], &reply["sneak"]), (&json!("normal"), &Value::Null));
    assert_eq!((&reply["state"]["tabs"], pane_ids(&reply)), (&after["state"]["tabs"], vec![1]), "{reply}");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn news_does_not_end_sneak_mode_and_no_key_of_a_label_reaches_the_terminal() {
    let otto = Otto::launch("sneak-news");
    let wait = |pattern: &str| otto.call(json!({"cmd": "wait", "pane": 2, "pattern": pattern, "timeout_ms": 40000}));
    // 1 | 2    two shells; the second has the keyboard.
    otto.call(json!({"cmd": "split", "kind": "terminal"}));
    otto.call(json!({"cmd": "split", "axis": "right", "kind": "terminal"}));
    otto.bounds(&[1, 2]);
    otto.call(json!({"cmd": "type", "pane": 2, "text": "echo ready-$((40+2))\r"}));
    wait("^ready-42$");

    // The first shell exits by itself while the labels are up: its pane closes, the mode stays.
    let (_, labels) = sneak(&otto);
    assert!(labels.contains_key("pane:1") && labels.contains_key("close:1"), "{labels:?}");
    otto.call(json!({"cmd": "type", "pane": 1, "text": "exit\r"}));
    let after = otto.wait_for("the exited shell's labels to go", |o| {
        let reply = o.call(json!({"cmd": "state"}));
        (pane_ids(&reply) == [2] && !sneak_labels(&reply).contains_key("pane:1")).then_some(reply)
    });
    let now = sneak_labels(&after);
    assert!(now.iter().all(|(id, label)| labels.get(id) == Some(label)), "no label changed its letters: {now:?}");
    assert!(!["pane:1", "header:1", "close:1"].iter().any(|id| now.contains_key(*id)), "{now:?}");

    // So the next letters are still a label's: the Tasks icon opens its section.
    let reply = otto.call(json!({"cmd": "key", "keys": letters(&now["rail:tasks"])}));
    assert_eq!((&reply["mode"], &reply["state"]["sidebar"]), (&json!("normal"), &json!("tasks")));

    // A letter of no label ends the mode, and the keys sent with it are dropped: none is typed
    // into the shell that has the keyboard again.
    sneak(&otto);
    assert_eq!(otto.call(json!({"cmd": "key", "keys": "z x x x"}))["mode"], "normal");
    otto.call(json!({"cmd": "type", "pane": 2, "text": "echo clean-$((40+2))\r"}));
    wait("^clean-42$");
    let text = otto.call(json!({"cmd": "capture", "pane": 2}))["text"].as_str().unwrap().to_string();
    assert!(!text.contains("xxx"), "{text}");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_request_from_the_socket_does_not_end_sneak_mode() {
    let otto = Otto::launch("sneak-socket");
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.bounds(&[1]);
    let (before, labels) = sneak(&otto);

    // An agent's requests run, and the user's labels stay up.
    let reply = otto.call(json!({"cmd": "split", "axis": "right", "kind": "empty"}));
    assert_eq!((&reply["mode"], pane_ids(&reply)), (&json!("sneak"), vec![1, 2]));
    let reply = otto.call(json!({"cmd": "focus", "pane": 1}));
    assert_eq!((&reply["mode"], &reply["state"]["focused"]), (&json!("sneak"), &json!(1)));
    otto.call(json!({"cmd": "task_add", "title": "an agent's task"}));
    // One that would take the keyboard from under the labels is refused, and says why.
    for request in [json!({"cmd": "picker"}), json!({"cmd": "keys"}), json!({"cmd": "enter", "pane": 1})] {
        let reply = otto.raw(&request.to_string());
        assert_eq!((&reply["ok"], &reply["error"]), (&json!(false), &json!("sneak mode is on")), "{request}");
    }
    assert_eq!(otto.raw(r#"{"cmd":"compose","text":"a draft"}"#)["ok"], false);
    // No picker opened, so there is nothing to choose from: that fails too, it does not pass in silence.
    assert_eq!(otto.raw(r#"{"cmd":"picker_choose"}"#)["error"], "the picker is not open");
    let reply = otto.call(json!({"cmd": "state"}));
    let untouched = (&reply["mode"], &reply["state"]["picker"], &reply["state"]["keys"], &reply["state"]["compose"]);
    assert_eq!(untouched, (&json!("sneak"), &Value::Null, &json!(false), &Value::Null));

    // The labels follow the new frame: pane 1 is half as wide and keeps its letters; pane 2 has none.
    let was = label_frame(&before, "pane:1", "w").unwrap();
    let after = otto.wait_for("the labels to follow the split", |o| {
        let reply = o.call(json!({"cmd": "state"}));
        label_frame(&reply, "pane:1", "w").is_some_and(|now| now < was * 0.7).then_some(reply)
    });
    let now = sneak_labels(&after);
    assert_eq!(now.get("pane:1"), labels.get("pane:1"));
    assert!(!now.contains_key("pane:2") && now.iter().all(|(id, label)| labels.get(id) == Some(label)), "{now:?}");

    // A label typed with `key` is input to the mode: it activates, and the mode ends.
    let reply = otto.call(json!({"cmd": "key", "keys": letters(&now["rail:tasks"])}));
    let chosen = (&reply["mode"], &reply["sneak"], &reply["state"]["sidebar"]);
    assert_eq!(chosen, (&json!("normal"), &Value::Null, &json!("tasks")));
    let (_, labels) = sneak(&otto);
    let reply = otto.call(json!({"cmd": "key", "keys": letters(&labels["close:2"])}));
    assert_eq!((&reply["mode"], pane_ids(&reply)), (&json!("normal"), vec![1]));
}

/// Each mark's name, in the list's order.
fn mark_names(reply: &Value) -> Vec<String> {
    let marks = reply["marks"].as_array().cloned().unwrap_or_default();
    marks.iter().filter_map(|mark| mark["name"].as_str().map(str::to_string)).collect()
}

/// The mark called `name`, as the state has it.
fn mark_named(reply: &Value, name: &str) -> Value {
    let marks = reply["marks"].as_array().cloned().unwrap_or_default();
    marks.into_iter().find(|mark| mark["name"] == name).unwrap_or_else(|| panic!("no mark {name}: {}", reply["marks"]))
}

/// Types `pwd` into `pane` and waits for a line that is exactly `dir`.
fn shell_is_in(otto: &Otto, pane: u64, dir: &Path) {
    otto.call(json!({"cmd": "type", "pane": pane, "text": "pwd\r"}));
    let pattern = format!("^{}$", dir.display().to_string().replace('.', "\\."));
    otto.call(json!({"cmd": "wait", "pane": pane, "pattern": pattern}));
}

/// `marks.json` in `home`, as JSON.
fn kept_marks(home: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(home.join("marks.json")).unwrap()).unwrap()
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn marks_are_a_way_back_to_a_pane_and_a_tab_and_are_kept_between_runs() {
    use std::os::unix::fs::PermissionsExt;
    let home = user_folder("marks", r#"{"repos": []}"#);
    std::fs::create_dir_all(home.join("notes")).unwrap();
    let notes = home.join("notes").canonicalize().unwrap();
    let env = [("OTTOSPACE_HOME", home.to_str().unwrap())];
    {
        let otto = Otto::launch_with("marks-1", &env);
        let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
        otto.call(json!({"cmd": "split"}));
        // The shell moves: a mark records where it is now, not where it started.
        let cd = format!("cd '{}' && echo in-$((6*7))\r", notes.display());
        otto.call(json!({"cmd": "type", "pane": 1, "text": cd}));
        otto.call(json!({"cmd": "wait", "pane": 1, "pattern": "^in-42$"}));

        // ⌘B marks the focused pane, named for its directory. ⌘B again takes the mark off.
        let marked = key("cmd-b");
        let mark = mark_named(&marked, "notes");
        assert_eq!(
            (&mark["kind"], &mark["key"], &mark["open"], &mark["pane"], &mark["available"]),
            (&json!("terminal"), &Value::Null, &json!(true), &json!(1), &json!(true)),
            "{mark}"
        );
        assert_eq!(key("cmd-b")["marks"], json!([]));
        // By request, with a letter. The reply is the mark.
        let added = otto.call(json!({"cmd": "mark_add", "pane": 1, "key": "n"}));
        assert_eq!((&added["mark"]["name"], &added["mark"]["key"]), (&json!("notes"), &json!("n")));
        // ⌘⇧B marks the tab on show, with both its panes.
        otto.call(json!({"cmd": "split", "axis": "down"}));
        let both = key("cmd-shift-b");
        assert_eq!(mark_names(&both), ["notes", "Tab 1"]);
        let tab = mark_named(&both, "Tab 1");
        assert_eq!((&tab["kind"], &tab["open"], &tab["tab"]), (&json!("tab"), &json!(true), &json!(1)));

        // ⌘' waits for a letter (MARK? in the status bar), then goes to that mark: its tab and pane.
        otto.call(json!({"cmd": "new_tab"}));
        let waiting = key("cmd-'");
        assert_eq!((&waiting["mode"], &waiting["state"]["mark_input"]), (&json!("mark"), &json!({"type": "jump"})));
        let there = key("n");
        assert_eq!(
            (&there["mode"], &there["state"]["active"], &there["state"]["focused"]),
            (&json!("normal"), &json!(0), &json!(1))
        );
        // Esc ends the wait, and so does a letter of no mark: neither reaches the shell.
        assert_eq!((key("cmd-'")["mode"].clone(), key("escape")["mode"].clone()), (json!("mark"), json!("normal")));
        assert_eq!((key("cmd-'")["mode"].clone(), key("z")["mode"].clone()), (json!("mark"), json!("normal")));
        let typed = otto.call(json!({"cmd": "capture", "pane": 1}))["text"].as_str().unwrap_or_default().to_string();
        assert!(!typed.lines().any(|line| line.ends_with('z') || line.ends_with('n')), "{typed}");
        // ⌘⌥2 goes to the second mark: the tab, from another one.
        otto.call(json!({"cmd": "select_tab", "index": 1}));
        assert_eq!(key("cmd-alt-2")["state"]["active"], 0);

        // The pane closes: the mark stays, not open. Going to it opens a shell in its directory.
        let closed = otto.call(json!({"cmd": "close", "pane": 1}));
        assert_eq!(mark_named(&closed, "notes")["open"], false);
        let back = otto.call(json!({"cmd": "mark_go", "name": "notes"}));
        assert_eq!((&back["state"]["focused"], &mark_named(&back, "notes")["pane"]), (&json!(3), &json!(3)));
        shell_is_in(&otto, 3, &notes);
        // Again: it is open, so nothing new opens.
        assert_eq!(pane_ids(&otto.call(json!({"cmd": "mark_go", "key": "n"}))), [2, 3]);

        // What cannot be marked or found says why. So does a move that waits behind the picker.
        otto.call(json!({"cmd": "select_tab", "index": 1}));
        let refused = |request: Value| otto.raw(&request.to_string())["error"].as_str().unwrap_or_default().to_string();
        assert_eq!(refused(json!({"cmd": "mark_add", "tab": 2})), "a tab with nothing in it has nothing to mark");
        assert_eq!(refused(json!({"cmd": "mark_add", "pane": 99})), "no pane 99");
        assert_eq!(refused(json!({"cmd": "mark_add", "pane": 2, "key": "1"})), "a mark's key is one letter, a to z");
        assert_eq!(refused(json!({"cmd": "mark_go", "name": "nope"})), "no mark named \"nope\"");
        assert_eq!(refused(json!({"cmd": "mark_go"})), "name the mark by `name` or by `key`: one of them");
        // No mark takes a command: the field is not one of the request's.
        assert!(refused(json!({"cmd": "mark_add", "pane": 2, "command": "ls"})).contains("unknown field `command`"));
        otto.call(json!({"cmd": "picker"}));
        assert_eq!(refused(json!({"cmd": "mark_go", "key": "n"})), "the picker is open");
        assert_eq!(otto.call(json!({"cmd": "picker_close"}))["state"]["active"], 1, "and it did not move");
        otto.call(json!({"cmd": "keys"}));
        assert_eq!(refused(json!({"cmd": "mark_go", "key": "n"})), "the key reference is open");
        otto.call(json!({"cmd": "keys"}));

        // The file: the user's alone, with the directory, an id for the mark, and no pane id or pid.
        let file = kept_marks(&home);
        let first = &file["marks"][0];
        assert_eq!((&file["version"], &first["name"], &first["key"]), (&json!(1), &json!("notes"), &json!("n")));
        assert_eq!(first["id"].as_str().map(str::len), Some(16), "{first}");
        let pane = &first["target"]["pane"];
        assert_eq!((&pane["kind"], pane["cwd"].as_str()), (&json!({"type": "terminal"}), notes.to_str()));
        assert_eq!(file["marks"][1]["target"]["tab"]["layout"]["split"]["axis"], "down");
        assert_eq!(std::fs::metadata(home.join("marks.json")).unwrap().permissions().mode() & 0o777, 0o600);
    } // the app is killed here

    // A new run on the same folder: the marks are there, nothing is open, and they still work.
    let otto = Otto::launch_with("marks-2", &env);
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    let start = otto.call(json!({"cmd": "state"}));
    assert_eq!((mark_names(&start), pane_ids(&start)), (vec!["notes".to_string(), "Tab 1".into()], vec![]));
    assert!(start["marks"].as_array().unwrap().iter().all(|mark| mark["open"] == false && mark["available"] == true));
    key("cmd-'");
    assert_eq!(key("n")["state"]["focused"], 1);
    shell_is_in(&otto, 1, &notes);

    // The Marks section lists them. A click on a row goes to the mark: the tab is built again,
    // split as it was, the upper shell in its directory.
    otto.call(json!({"cmd": "section", "section": "marks"}));
    let listed = otto.wait_for("the rows to be drawn", |o| Some(o.drawn()).filter(|s| s["marks"][1]["row"].is_array()));
    let built = otto.click(at(rect(&mark_named(&listed, "Tab 1")["row"]), 0.3, 0.5), 1);
    assert_eq!(built["tab_panes"], json!([[1], [2, 3]]), "{}", built["state"]["layout"]);
    assert_eq!(built["state"]["layout"]["split"]["axis"], "down");
    assert_eq!((&built["state"]["active"], &mark_named(&built, "Tab 1")["tab"]), (&json!(1), &json!(2)));
    assert_eq!(built["state"]["tabs"][1]["title"], "Tab 1");
    shell_is_in(&otto, 2, &notes);

    // List mode: ⇧J moves the selected mark down, M gives it a letter, R renames it in its row.
    assert_eq!(key("cmd-shift-e")["mode"], "list");
    assert_eq!(mark_names(&key("shift-j")), ["Tab 1", "notes"]);
    assert_eq!(key("shift-k")["state"]["list"], 0);
    assert_eq!(key("j")["state"]["list"], 1);
    assert_eq!(key("m")["state"]["mark_input"]["type"], "set_key");
    let lettered = key("t");
    assert_eq!((&lettered["mode"], &mark_named(&lettered, "Tab 1")["key"]), (&json!("list"), &json!("t")));
    assert_eq!(key("r")["state"]["mark_input"]["text"], "Tab 1");
    let renamed = key("backspace backspace backspace backspace backspace w o r k enter");
    assert_eq!(
        (mark_names(&renamed), renamed["mode"].as_str()),
        (vec!["notes".to_string(), "work".into()], Some("list"))
    );
    // X takes the selected mark off and offers it back; U, in this section, puts it back in its place.
    let off = key("x");
    assert_eq!(
        (mark_names(&off), &off["mark_removed"], &off["tasks_bar"]["undo"]),
        (vec!["notes".into()], &json!("work"), &Value::Null)
    );
    let on = key("u");
    assert_eq!((mark_names(&on), &on["mark_removed"]), (vec!["notes".to_string(), "work".into()], &Value::Null));
    assert_eq!(mark_named(&on, "work")["key"], "t");
    key("escape");

    // The row's own buttons: the pencil starts a rename (Esc keeps the name), × removes the mark.
    let rows = otto.drawn();
    let pencil = otto.click(at(rect(&mark_named(&rows, "notes")["buttons"]["rename"]), 0.5, 0.5), 1);
    assert_eq!((&pencil["mode"], &pencil["state"]["mark_input"]["type"]), (&json!("mark"), &json!("rename")));
    assert_eq!(pane_ids(&pencil), [1, 2, 3], "the click did not also go to the mark");
    assert_eq!(key("x escape")["mode"], "normal");
    let removed = otto.click(at(rect(&mark_named(&rows, "notes")["buttons"]["remove"]), 0.5, 0.5), 1);
    assert_eq!((mark_names(&removed), &removed["mark_removed"]), (vec!["work".to_string()], &json!("notes")));
    assert_eq!(kept_marks(&home)["marks"].as_array().map(Vec::len), Some(1), "the file follows");

    // One of the tab's two directories goes. The mark still opens: what can open does, and the
    // other pane is empty and says why.
    otto.call(json!({"cmd": "close_tab", "index": 1}));
    otto.call(json!({"cmd": "close", "pane": 1}));
    std::fs::remove_dir_all(&notes).unwrap();
    let gone = format!("its directory is gone: {}", notes.display());
    let partly = otto.wait_for("the missing directory to be noticed", |o| {
        Some(mark_named(&o.call(json!({"cmd": "state"})), "work")).filter(|work| work["reason"].is_string())
    });
    assert_eq!(
        (&partly["open"], &partly["available"], partly["reason"].as_str()),
        (&json!(false), &json!(true), Some(&*format!("1 of 2 panes cannot open: {gone}")))
    );
    let built = otto.call(json!({"cmd": "mark_go", "key": "t"}));
    let kinds: Vec<&str> =
        built["state"]["panes"].as_array().unwrap().iter().filter_map(|p| p["kind"]["type"].as_str()).collect();
    assert_eq!((pane_ids(&built), kinds), (vec![4, 5], vec!["empty", "terminal"]));
    assert_eq!(mark_named(&built, "work")["unfilled"], json!([{"pane": 4, "why": gone}]));
    // The directory is back: going to the open tab again starts the shell that was missing.
    std::fs::create_dir_all(&notes).unwrap();
    otto.wait_for("the empty pane to be filled", |o| {
        let again = o.call(json!({"cmd": "mark_go", "key": "t"}));
        (mark_named(&again, "work")["unfilled"] == json!([])).then_some(())
    });
    shell_is_in(&otto, 4, &notes);
    drop(otto);

    // A file that does not parse is said so, and left as it is.
    std::fs::write(home.join("marks.json"), "{ not marks").unwrap();
    let otto = Otto::launch_with("marks-3", &env);
    let state = otto.call(json!({"cmd": "split"}));
    assert!(state["notice"].as_str().is_some_and(|notice| notice.starts_with("marks.json: ")), "{}", state["notice"]);
    assert_eq!(otto.call(json!({"cmd": "mark_add", "pane": 1}))["mark"]["kind"], "terminal");
    assert_eq!(std::fs::read_to_string(home.join("marks.json")).unwrap(), "{ not marks");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_marks_wait_for_a_letter_is_the_users_and_a_mark_taken_off_by_mistake_comes_back() {
    let otto = Otto::launch("marks-quiet");
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    // No marks yet.
    let empty = otto.call(json!({"cmd": "section", "section": "marks"}));
    assert_eq!((&empty["marks"], &empty["state"]["sidebar"]), (&json!([]), &json!("marks")));
    otto.call(json!({"cmd": "split"}));
    otto.call(json!({"cmd": "mark_add", "pane": 1, "name": "build", "key": "b"}));
    // A second ⌘B by mistake takes the mark off, and offers it back; a third puts the same mark
    // back, with its name and letter.
    let off = key("cmd-b");
    assert_eq!((&off["marks"], &off["mark_removed"]), (&json!([]), &json!("build")));
    let on = key("cmd-b");
    let back = mark_named(&on, "build");
    assert_eq!((&back["key"], &back["pane"], &on["mark_removed"]), (&json!("b"), &json!(1), &Value::Null));
    key("cmd-shift-b");
    assert_eq!(mark_names(&otto.drawn()), ["build", "Tab 1"]);

    // While the prefix waits, a client's request runs underneath, but none takes the keyboard,
    // and no pointer event ends the wait.
    key("cmd-'");
    let split = otto.call(json!({"cmd": "split", "kind": "empty"}));
    assert_eq!((split["mode"].as_str(), pane_ids(&split)), (Some("mark"), vec![1, 2]));
    for taker in [json!({"cmd": "picker"}), json!({"cmd": "keys"}), json!({"cmd": "compose"})] {
        let refused = otto.raw(&taker.to_string());
        assert_eq!(refused["error"], "a mark is waiting for a letter or a name", "{taker}");
    }
    let click = otto.raw(r#"{"cmd":"mouse","action":"down","x":300,"y":300}"#);
    assert_eq!(click["error"], "a mark is waiting for a letter or a name");
    assert_eq!(key("b")["state"]["focused"], 1);

    // A pane mark whose directory is gone cannot be opened, and says why. No shell opens elsewhere.
    let dir = std::env::temp_dir().join(format!("ottospace-e2e-mark-gone-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    otto.call(json!({"cmd": "type", "pane": 1, "text": format!("cd '{}' && echo in-$((6*7))\r", dir.display())}));
    otto.call(json!({"cmd": "wait", "pane": 1, "pattern": "^in-42$"}));
    assert_eq!(otto.raw(r#"{"cmd":"mark_add","pane":2}"#)["error"], "an empty pane has nothing to mark");
    otto.call(json!({"cmd": "mark_remove", "name": "build"}));
    otto.call(json!({"cmd": "mark_add", "pane": 1, "name": "there"}));
    otto.call(json!({"cmd": "close", "pane": 1}));
    std::fs::remove_dir_all(&dir).unwrap();
    let reason = format!("its directory is gone: {}", dir.display());
    otto.wait_for("the missing directory to be noticed", |o| {
        let there = mark_named(&o.call(json!({"cmd": "state"})), "there");
        (there["available"] == false && there["reason"] == json!(reason)).then_some(())
    });
    let refused = otto.raw(r#"{"cmd":"mark_go","name":"there"}"#);
    assert_eq!(refused["error"].as_str(), Some(&*format!("there: {reason}")));
    assert_eq!(pane_ids(&otto.call(json!({"cmd": "state"}))), [2], "no shell somewhere else");
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_hand_written_marks_file_is_checked_and_commands_in_it_are_never_typed() {
    // A wrong key in the settings too: the two problems are both said.
    let home = user_folder("marks-file", r#"{"repos": [], "nope": 1}"#);
    std::fs::create_dir_all(home.join("work")).unwrap();
    let work = home.join("work").canonicalize().unwrap();
    let env = [("OTTOSPACE_HOME", home.to_str().unwrap())];
    let shell = |cwd: &str, rest: &str| json!({"id": 1, "kind": {"type": "terminal"}, "cwd": cwd, "command": rest});
    let chat = json!({"id": 2, "kind": {"type": "app", "bundle": "com.example.acmechat", "name": "Acme Chat", "title": "general", "pid": 811}});
    let layout = json!({"split": {"axis": "right", "ratio": 0.6, "a": {"leaf": 1}, "b": {"leaf": 2}}});
    let tab = json!({"title": "review", "layout": layout, "panes": [shell(work.to_str().unwrap(), "echo ran-$((6*7))"), chat], "display": 2});
    let written = json!({"version": 1, "theme": "dark", "marks": [{"name": "review", "key": "r", "pinned": true, "target": {"tab": tab}}]});
    std::fs::write(home.join("marks.json"), written.to_string()).unwrap();

    let otto = Otto::launch_with("marks-file-1", &env);
    let start = otto.call(json!({"cmd": "state"}));
    let notice = start["notice"].as_str().unwrap_or_default();
    assert!(
        notice.contains("settings.json: ") && notice.contains("marks.json: commands are not supported; ignored"),
        "{notice}"
    );
    // The app is not installed: the tab opens with the shell, and that pane empty, saying why.
    let partly = otto.wait_for("the missing app to be noticed", |o| {
        Some(mark_named(&o.call(json!({"cmd": "state"})), "review")).filter(|mark| mark["reason"].is_string())
    });
    assert_eq!(partly["reason"], "1 of 2 panes cannot open: Acme Chat is not installed");
    let built = otto.call(json!({"cmd": "key", "keys": "cmd-' r"}));
    assert_eq!(built["tab_panes"], json!([[], [1, 2]]));
    assert_eq!(mark_named(&built, "review")["unfilled"], json!([{"pane": 2, "why": "Acme Chat is not installed"}]));
    // The shell is in its directory, and nothing was typed into it.
    shell_is_in(&otto, 1, &work);
    let text = otto.call(json!({"cmd": "capture", "pane": 1, "history": true}))["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!text.contains("ran-"), "{text}");
    // Written again: what this build does not know is kept, the commands are not.
    otto.call(json!({"cmd": "mark_rename", "key": "r", "to": "pr review"}));
    let file = kept_marks(&home);
    let mark = &file["marks"][0];
    assert_eq!(
        (&file["theme"], &mark["pinned"], &mark["name"], &mark["id"]),
        (&json!("dark"), &json!(true), &json!("pr review"), &json!("name:review"))
    );
    let kept = &mark["target"]["tab"];
    assert_eq!((&kept["display"], &kept["panes"][1]["kind"]["pid"]), (&json!(2), &json!(811)));
    assert!(!file.to_string().contains("command"), "{file}");
    drop(otto);

    // A record that is not sound is left out, and said; the file is then left as it is.
    let unsound = json!({"version": 1, "marks": [
        {"name": "good", "target": {"pane": {"kind": {"type": "terminal"}, "cwd": work.to_str().unwrap()}}},
        {"name": "relative", "target": {"pane": {"kind": {"type": "terminal"}, "cwd": "work"}}},
        {"name": "escape", "target": {"pane": {"kind": {"type": "app", "name": "Chat", "title": "a\u{1b}[31mb"}}}}]});
    std::fs::write(home.join("marks.json"), unsound.to_string()).unwrap();
    let otto = Otto::launch_with("marks-file-2", &env);
    let state = otto.call(json!({"cmd": "split"}));
    assert_eq!(mark_names(&state), ["good"]);
    let notice = state["notice"].as_str().unwrap_or_default();
    for part in [
        "settings.json: ",
        "mark \"relative\": its directory is not an absolute path",
        "mark \"escape\": ",
        "not saved this run",
    ] {
        assert!(notice.contains(part), "{part}: {notice}");
    }
    otto.call(json!({"cmd": "mark_add", "pane": 1, "name": "new"}));
    assert_eq!(std::fs::read_to_string(home.join("marks.json")).unwrap(), unsound.to_string());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn two_ottospaces_on_one_folder_keep_each_others_marks() {
    let home = user_folder("marks-two", r#"{"repos": []}"#);
    let env = [("OTTOSPACE_HOME", home.to_str().unwrap())];
    let (a, b) = (Otto::launch_with("marks-two-a", &env), Otto::launch_with("marks-two-b", &env));
    let on_disk = || -> Vec<String> {
        let marks = kept_marks(&home)["marks"].as_array().cloned().unwrap_or_default();
        marks.iter().filter_map(|mark| mark["name"].as_str().map(str::to_string)).collect()
    };
    // Each marks a pane of its own: the second to write takes the first's mark in.
    for (otto, name) in [(&a, "from a"), (&b, "from b")] {
        otto.call(json!({"cmd": "split"}));
        otto.call(json!({"cmd": "mark_add", "pane": 1, "name": name}));
    }
    assert_eq!(on_disk(), ["from a", "from b"]);
    let seen = b.call(json!({"cmd": "state"}));
    assert_eq!(
        (mark_names(&seen), &mark_named(&seen, "from a")["open"]),
        (vec!["from a".to_string(), "from b".into()], &json!(false))
    );
    // A learns of B's mark when it next writes; its own is still of its own pane.
    let third = a.call(json!({"cmd": "mark_add", "tab": 1, "name": "tab of a"}));
    assert_eq!(third["mark"]["name"], "tab of a");
    let seen = a.call(json!({"cmd": "state"}));
    assert_eq!(mark_names(&seen), ["from a", "from b", "tab of a"]);
    assert_eq!(
        (&mark_named(&seen, "from a")["pane"], &mark_named(&seen, "from b")["open"]),
        (&json!(1), &json!(false))
    );
    // B renames its mark, A removes its own: neither undoes the other.
    b.call(json!({"cmd": "mark_rename", "name": "from b", "to": "bee"}));
    assert_eq!(on_disk(), ["from a", "bee", "tab of a"]);
    assert_eq!(mark_names(&a.call(json!({"cmd": "mark_remove", "name": "from a"}))), ["bee", "tab of a"]);
    assert_eq!(on_disk(), ["bee", "tab of a"]);
    // B sees the removal at its next write, and does not write the mark back.
    assert_eq!(b.call(json!({"cmd": "mark_add", "pane": 1, "key": "q"}))["mark"]["name"], "bee");
    assert_eq!(mark_names(&b.call(json!({"cmd": "state"}))), ["bee", "tab of a"]);
    assert_eq!(on_disk(), ["bee", "tab of a"]);
    let _ = std::fs::remove_dir_all(&home);
}

/// A scroll area's bar as the state gives it: (track, thumb, opacity). `None`: its content fits.
fn scrollbar(reply: &Value, id: &str) -> Option<([f64; 4], [f64; 4], f64)> {
    let bar = reply["scrollbars"].get(id)?;
    Some((rect(&bar["track"]), rect(&bar["thumb"]), bar["opacity"].as_f64()?))
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn a_long_list_has_a_scrollbar_that_drags_and_pages() {
    let otto = Otto::launch("scrollbar");
    otto.call(json!({"cmd": "split", "kind": "empty"}));
    otto.call(json!({"cmd": "section", "section": "tasks"}));
    otto.call(json!({"cmd": "task_add", "title": "task 0"}));
    otto.bounds(&[1]);
    // A list that fits has no bar.
    assert_eq!(otto.drawn()["scrollbars"], json!({}));

    for i in 1..60 {
        otto.call(json!({"cmd": "task_add", "title": format!("task {i}")}));
    }
    let bar = |reply: &Value| scrollbar(reply, "tasks");
    // At the top, faded out: the pointer is elsewhere and nothing scrolled.
    let (track, thumb, opacity) = otto.wait_for("the bar", |o| bar(&o.drawn()));
    assert_eq!((thumb[1], opacity), (track[1], 0.0), "{track:?} {thumb:?}");
    assert!(track[2] == 8.0 && thumb[2] == 6.0, "a 6 point thumb on an 8 point track: {track:?} {thumb:?}");
    assert!(thumb[3] >= 24.0 && thumb[3] < track[3] / 2.0, "60 rows, a short thumb: {track:?} {thumb:?}");

    // The pointer over the list shows it; over the bar, the thumb widens.
    let (_, _, opacity) = bar(&otto.mouse("move", [track[0] - 100.0, track[1] + 200.0])).unwrap();
    assert_eq!(opacity, 1.0);
    let (_, wide, _) = bar(&otto.mouse("move", at(thumb, 0.5, 0.5))).unwrap();
    assert_eq!((wide[0], wide[2]), (track[0], 8.0));

    // A drag of the thumb moves it with the pointer, and stops at the bottom. No row is clicked.
    let reply = otto.drag(at(thumb, 0.5, 0.5), [thumb[0], thumb[1] + thumb[3] / 2.0 + 100.0]);
    let (_, dragged, _) = bar(&reply).unwrap();
    assert!((dragged[1] - (thumb[1] + 100.0)).abs() < 1.0, "{thumb:?} {dragged:?}");
    let reply = otto.drag(at(dragged, 0.5, 0.5), [dragged[0], track[1] + track[3] + 500.0]);
    let (_, bottom, _) = bar(&reply).unwrap();
    assert!((bottom[1] + bottom[3] - (track[1] + track[3])).abs() < 0.5, "{track:?} {bottom:?}");
    assert_eq!((pane_ids(&reply), reply["state"]["tabs"].as_array().map(Vec::len)), (vec![1], Some(1)));

    // A click on the track above the thumb pages up: the thumb moves by its own length.
    let (_, paged, _) = bar(&otto.click(at(track, 0.5, 0.02), 1)).unwrap();
    assert!((bottom[1] - paged[1] - thumb[3]).abs() < 1.0, "{bottom:?} {paged:?}");
    // A moment after the pointer leaves, the bar fades.
    otto.drawn();
    otto.wait_for("the bar to fade", |o| bar(&o.call(json!({"cmd": "state"}))).filter(|bar| bar.2 == 0.0));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_terminal_scrollbar_shows_the_scrollback_and_drags_it() {
    let otto = Otto::launch("scrollbar-term");
    otto.call(json!({"cmd": "split"}));
    otto.bounds(&[1]);
    // An empty history has no bar.
    assert_eq!(otto.drawn()["scrollbars"], json!({}));
    otto.call(json!({"cmd": "type", "text": "i=1; while [ $i -le 200 ]; do echo line-$i; i=$((i+1)); done\r"}));
    otto.wait_for("200 lines", |o| Some(term1(o)).filter(|t| shows(t, "line-200")));

    let reply = otto.drawn();
    let (t, (track, thumb, _)) = (&reply["terminals"]["1"], scrollbar(&reply, "terminal:1").unwrap());
    let (rows, history) = (t["rows"].as_f64().unwrap(), t["history"].as_f64().unwrap());
    // The thumb is the screen's share of all the lines, at the bottom.
    assert!((thumb[3] / track[3] - rows / (rows + history)).abs() < 0.01, "{track:?} {thumb:?} {rows} {history}");
    assert!((thumb[1] + thumb[3] - (track[1] + track[3])).abs() < 0.5, "{track:?} {thumb:?}");

    // Dragged to the top, the first line shows. The press selects nothing.
    let reply = otto.drag(at(thumb, 0.5, 0.5), [thumb[0], track[1] - 50.0]);
    let t = &reply["terminals"]["1"];
    assert_eq!(t["display_offset"], t["history"], "{t}");
    assert!(shows(t, "line-1") && t["selection"].is_null(), "{t}");
    // A click on the track below the thumb pages down one screen.
    let reply = otto.click(at(track, 0.5, 0.98), 1);
    assert_eq!(reply["terminals"]["1"]["display_offset"].as_f64(), Some(history - rows));
}

#[test]
#[ignore = "opens a window; run with `cargo e2e`"]
fn the_keys_overlay_scrolls_inside_the_window() {
    let otto = Otto::launch("scrollbar-keys");
    let reply = otto.call(json!({"cmd": "keys"}));
    let window = rect(&reply["window"]);
    let (track, thumb, _) = otto.wait_for("the bar", |o| scrollbar(&o.drawn(), "keys"));
    assert!(track[1] > 0.0 && track[1] + track[3] < window[3], "the panel is inside the window: {track:?} {window:?}");
    // Dragging the bar scrolls the list and does not close it, as a press beside the panel does.
    let reply = otto.drag(at(thumb, 0.5, 0.5), [thumb[0], track[1] + track[3]]);
    let (_, bottom, _) = scrollbar(&reply, "keys").unwrap();
    assert!((bottom[1] + bottom[3] - (track[1] + track[3])).abs() < 0.5, "{track:?} {bottom:?}");
    assert_eq!(reply["state"]["keys"], true);
    // Closed, its bar is gone.
    assert_eq!(otto.call(json!({"cmd": "keys"}))["state"]["keys"], false);
    assert_eq!(otto.drawn()["scrollbars"], json!({}));
}
