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
        let socket = std::env::temp_dir().join(format!("ottospace-e2e-{name}-{}.sock", std::process::id()));
        let child = Command::new(env!("CARGO_BIN_EXE_ottospace"))
            .env("OTTOSPACE_E2E", "1")
            .env("OTTOSPACE_SOCKET", &socket)
            // A plain shell: the test must not depend on the user's shell profile.
            .env("OTTOSPACE_SHELL", "/bin/sh")
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
    let [one, four] = otto.bounds(&[1, 4])[..] else { panic!("two panes") };
    assert!(one[0] + one[2] <= four[0], "1 is left of 4: {one:?} {four:?}");

    for bad in [
        r#"{"cmd":"drop_pane","source":1,"target":1,"zone":"left"}"#,
        r#"{"cmd":"drop_pane","source":1,"target":9,"zone":"left"}"#,
        r#"{"cmd":"drop_pane","source":1,"target":4,"zone":"middle"}"#,
    ] {
        assert_eq!(otto.raw(bad)["ok"], false, "{bad}");
    }
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
fn list_mode_drives_the_bar_with_the_keyboard() {
    let otto = Otto::launch("list");
    let key = |keys: &str| otto.call(json!({"cmd": "key", "keys": keys}));
    for (title, state) in [("first", "needs_you"), ("second", "needs_you"), ("third", "running")] {
        otto.call(json!({"cmd": "task_add", "title": title, "priority": 2, "state": state}));
    }
    otto.call(json!({"cmd": "section", "section": "tasks"}));

    // ⌘⇧E gives the bar the keyboard, on its first row.
    let reply = key("cmd-shift-e");
    assert_eq!((reply["mode"].as_str(), reply["state"]["list"].as_u64()), (Some("list"), Some(0)));
    // j moves down; x dismisses that task; the selection stays on a row.
    key("j");
    let reply = key("x");
    let titles = |reply: &Value| -> Vec<String> {
        reply["inbox"].as_array().unwrap().iter().map(|r| r["title"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!((titles(&reply), reply["state"]["list"].as_u64()), (vec!["first".into(), "third".into()], Some(1)));
    // u takes it back; h holds the selected task; s snoozes it.
    assert_eq!(titles(&key("u")), ["first", "second", "third"]);
    let reply = key("h");
    assert_eq!(reply["state"]["tasks"]["list"][1]["state"], "parked");
    key("u");
    let reply = key("k s");
    assert!(reply["state"]["tasks"]["list"][0]["snoozed_until"].as_u64().is_some(), "the first task is snoozed");
    assert_eq!(titles(&reply), ["second", "third"]);

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
    // One repository to name with `@`.
    let root = std::env::temp_dir().join(format!("ottospace-e2e-compose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    fake_repo(&root.join("flow-app"));
    let otto = Otto::launch_with("compose", &[("OTTOSPACE_REPOS", root.to_str().unwrap())]);
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
    otto.call(json!({"cmd": "compose", "text": "fix zoom flicker #canvas p2 @flow"}));
    // A second draft while the first is on screen is refused, not dropped in silence.
    assert_eq!(otto.raw(r#"{"cmd":"compose","text":"another"}"#)["ok"], false);
    assert_eq!(key("tab")["state"]["compose"], "fix zoom flicker #canvas p2 @flow-app ");
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
    let start = Instant::now();
    while otto.child.try_wait().expect("wait").is_none() {
        assert!(start.elapsed() < Duration::from_secs(5), "quit did not exit the app");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!otto.socket.exists(), "socket file is removed on quit");
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
    // Keys that are not letters do nothing. Esc leaves, and nothing has changed.
    let before = otto.call(json!({"cmd": "key", "keys": "1 left enter tab"}));
    assert_eq!((&before["mode"], &before["sneak"]["typed"]), (&json!("sneak"), &json!("")), "not letters");
    // Asking again while it is on changes nothing: the same labels.
    assert_eq!(sneak(&otto).1, labels);
    let after = otto.call(json!({"cmd": "key", "keys": "escape"}));
    assert_eq!((&after["mode"], &after["sneak"], &after["state"]), (&json!("normal"), &Value::Null, &before["state"]));
    // A letter of no label leaves too, and nothing has changed.
    sneak(&otto);
    let after = otto.call(json!({"cmd": "key", "keys": "z"}));
    assert_eq!((&after["mode"], &after["sneak"], &after["state"]), (&json!("normal"), &Value::Null, &before["state"]));
    // Any ⌘ shortcut leaves and then runs, also one that changes no state (copy).
    sneak(&otto);
    assert_eq!(otto.call(json!({"cmd": "key", "keys": "cmd-c"}))["mode"], "normal");
    sneak(&otto);
    let reply = otto.call(json!({"cmd": "key", "keys": "cmd-/"}));
    assert_eq!(
        (&reply["mode"], &reply["sneak"], &reply["state"]["keys"]),
        (&json!("normal"), &Value::Null, &json!(true))
    );
    otto.call(json!({"cmd": "key", "keys": "escape"}));
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
    otto.call(json!({"cmd": "section", "section": "tasks"}));
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

/// Where the labelled control `id` is: its frame's left edge.
fn label_x(reply: &Value, id: &str) -> Option<f64> {
    reply["sneak"]["targets"].as_array()?.iter().find(|t| t["source"]["id"] == id)?["frame"]["x"].as_f64()
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
    let (was, is) = (label_x(&before, &third_close).unwrap(), label_x(&after, &third_close).unwrap());
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
