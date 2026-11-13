//! Control socket: drives and inspects the running app. One JSON request per line in, one JSON
//! reply per line out. Used by `ottospace ctl`, the e2e tests, and (later) agents.
//!
//! Trust: a Unix socket, mode 0600, in the per-user temp dir. Only the same user can connect.
//! Requests are a closed set (`Request`). `type` sends keys to a terminal pane: the same power the
//! user has at the keyboard, and no more than a same-user process already has.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::{fs, thread};

use ottospace_core::tasks::{Heading, State, TaskId};
use ottospace_core::{Axis, Dir, PaneId, Section, Zone};
use serde::Deserialize;

/// e.g. `{"cmd":"split","axis":"down"}`, `{"cmd":"type","text":"ls\r"}`, `{"cmd":"state"}`.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    State,
    Split {
        #[serde(default)]
        axis: Axis,
        #[serde(default)]
        kind: PaneKind,
    },
    /// Sends `text` to a terminal pane (default: the focused one), as if typed.
    Type {
        #[serde(default)]
        pane: Option<PaneId>,
        text: String,
    },
    Close {
        #[serde(default)]
        pane: Option<PaneId>,
    },
    Focus {
        pane: PaneId,
    },
    FocusDir {
        dir: Dir,
    },
    /// Next (positive) or previous pane in reading order.
    FocusCycle {
        delta: i32,
    },
    /// Moves the boundary of the nearest split around the focused pane one step toward `dir`.
    Resize {
        dir: Dir,
    },
    /// The focused pane trades places with its neighbour on that side.
    Swap {
        dir: Dir,
    },
    /// Moves a pane (default: the focused one) to the tab at place `tab` on the tab bar, from 0.
    MovePane {
        #[serde(default)]
        pane: Option<PaneId>,
        tab: usize,
    },
    /// Evens out the splits of the active tab.
    Equalize,
    /// Drops pane `source` on pane `target`, as a mouse drag does. `zone`: `left`, `right`, `top`
    /// or `bottom` splits `target` and puts `source` on that side; `center` swaps the two.
    DropPane {
        source: PaneId,
        target: PaneId,
        zone: Zone,
    },
    /// Moves the splits around an app pane (default: the focused one) so the pane has the size
    /// its window kept; see `state.panes[].fit`.
    Fit {
        #[serde(default)]
        pane: Option<PaneId>,
    },
    OpenApp {
        name: String,
        #[serde(default)]
        axis: Axis,
    },
    Enter {
        pane: PaneId,
    },
    Leave,
    /// Toggles a rail section: `{"cmd":"section","section":"apps"}`.
    Section {
        section: Section,
    },
    /// Opens the pane picker: `{"cmd":"picker","axis":"down"}`.
    Picker {
        #[serde(default)]
        axis: Axis,
        /// Open the pick in a new tab.
        #[serde(default)]
        new_tab: bool,
    },
    NewTab,
    SelectTab {
        index: usize,
    },
    /// Next (positive) or previous tab on the bar: the tabs of the task on show.
    CycleTab {
        delta: i32,
    },
    /// Closes a tab (default: the active one) and its panes.
    CloseTab {
        #[serde(default)]
        index: Option<usize>,
    },
    PickerQuery {
        query: String,
    },
    PickerMove {
        delta: i32,
    },
    /// Opens the selected match, or the match at `index`.
    PickerChoose {
        #[serde(default)]
        index: Option<usize>,
    },
    PickerClose,
    /// Shows or hides the key reference.
    Keys,
    /// Presses keys as if typed, through the key bindings: `{"cmd":"key","keys":"shift-pageup"}`.
    /// Space-separated keystrokes in GPUI's syntax.
    Key {
        keys: String,
    },
    /// Presses the mouse on a terminal pane (default: the focused one) at `x`, `y` in cells from its
    /// top-left. A fraction picks a side of a cell. `clicks`: 2 selects a word, 3 a line.
    MouseDown {
        #[serde(default)]
        pane: Option<PaneId>,
        x: f32,
        y: f32,
        #[serde(default = "one")]
        clicks: usize,
    },
    /// Drags the pressed mouse to `x`, `y` in cells, measured in that pane.
    MouseDrag {
        #[serde(default)]
        pane: Option<PaneId>,
        x: f32,
        y: f32,
    },
    /// Turns the scroll wheel over a terminal pane (default: the focused one). Positive `lines`
    /// scrolls back.
    Scroll {
        #[serde(default)]
        pane: Option<PaneId>,
        lines: f32,
    },
    /// For tests (`pointer_refusal`). One left-button pointer event at `x`, `y`, in points from
    /// the top-left of ottospace's window, as the state gives rects:
    /// `{"cmd":"mouse","action":"down","x":300,"y":60}`. A `move` between a `down` and its `up`
    /// is a drag; every `down` needs its `up`. The event never leaves ottospace: it is not
    /// posted, the real pointer does not move, and no other app's window gets it. But a click
    /// does what the control under it does, with the outside effects of `open_app`, `enter`,
    /// `leave` or `close`: a click on a parked app pane raises that app's window.
    Mouse {
        action: MouseAction,
        x: f32,
        y: f32,
        #[serde(default = "one")]
        clicks: usize,
        #[serde(default)]
        modifiers: Vec<Modifier>,
    },
    /// A whole left-button drag: a press at `from`, `steps` moves along the line, a release at
    /// `to`. For tests, as `mouse`.
    Drag {
        from: [f32; 2],
        to: [f32; 2],
        #[serde(default = "eight")]
        steps: usize,
    },
    /// Reads the repository list from disk again. It is `state.repos` in the reply.
    Repos,
    /// Opens a terminal in a listed repository or worktree, by its `path` from `state.repos`.
    OpenRepo {
        path: String,
        #[serde(default)]
        axis: Axis,
    },
    /// Files a task: `{"cmd":"task_add","title":"Fix the zoom flicker","priority":2}`. `priority`:
    /// 1 (most urgent) to 9, default 3. `state`: default `captured`. `tab`: the `id` of an open
    /// tab that becomes the task's (`$OTTOSPACE_TAB` in a shell). The reply is
    /// `{"ok":true,"task":{..}}`: the new task, with its `id`.
    TaskAdd {
        title: String,
        #[serde(default = "three")]
        priority: u8,
        #[serde(default = "captured")]
        state: State,
        #[serde(default)]
        tab: Option<u32>,
        /// Who works it and the skills it loads: names `task_new` would know (`@agent`, `$skill`),
        /// else the request is refused. `paths`: what it is about, kept as given, not checked.
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default)]
        skills: Vec<String>,
    },
    /// An agent reports where its task is: `running`, `needs_you`, `done`, ...
    TaskSetState {
        id: TaskId,
        state: State,
    },
    /// Hides a task from the inbox for `seconds`, or until it needs the user for a new reason.
    TaskSnooze {
        id: TaskId,
        seconds: u64,
    },
    TaskHold {
        id: TaskId,
    },
    /// Takes a task out of the inbox. It goes on running.
    TaskDismiss {
        id: TaskId,
    },
    /// Takes back the last snooze, hold or dismiss.
    TaskUndo,
    /// The inbox, in order: `{"ok":true,"inbox":[{"group":"needs_you","id":1,..}]}`, not the state.
    TaskList,
    /// Files a task from one line, as the new-task box does: `#project`, `>repo` (a listed
    /// repository), `@agent`, `/path`, `$skill`, `p1`..`p9`; the rest is the title. The reply is
    /// `{"ok":true,"task":{..}}`. `open`: also show the task's tabs ("add and start").
    TaskNew {
        line: String,
        #[serde(default)]
        open: bool,
    },
    /// Collapses or expands a heading of the Tasks section: `needs_you`, `open`, `running`,
    /// `captured`, `on_hold`, `done`, `sessions`.
    TaskGroup {
        heading: Heading,
    },
    /// Makes a task of a session the Tasks section lists under Sessions: `runner` is `claude` or
    /// `codex`, `id` the session's id, both as `sessions_bar.rows` gives them. The task is named
    /// after the session's repository, works in its directory, and the session leaves the group.
    SessionAdopt {
        runner: String,
        id: String,
    },
    /// Stops listing a session until the app starts again.
    SessionHide {
        runner: String,
        id: String,
    },
    /// A click on a session's row: focuses the pane it runs in if that is known, else selects
    /// the row. Nothing is ever sent to the session.
    SessionClick {
        runner: String,
        id: String,
    },
    /// Shows or hides the idle sessions older than a day ("N older").
    SessionsOlder,
    /// Opens the new-task box for the user, with `text` in it to edit: a draft, not a filed task.
    Compose {
        #[serde(default)]
        text: String,
    },
    /// For tests (`pointer_refusal`): files or folders dropped on the open new-task box, as a
    /// drag from Finder drops them: `{"cmd":"compose_drop","paths":["/tmp/a.txt"]}`. Each goes
    /// into the line at the cursor as a `/path` word.
    ComposeDrop {
        paths: Vec<String>,
    },
    /// Shows the tabs of the first task that needs the user. The reply is
    /// `{"ok":true,"task":{..},"opened":true}`; `"task":null` if nothing needs the user, and
    /// `"opened":false` if the view could not change (the picker or key reference is open).
    TaskPop,
    /// Shows a task's tabs. A task opened for the first time gets a tab, with a terminal in its
    /// repository if it names one.
    TaskOpen {
        id: TaskId,
    },
    /// Shows the tabs of no task again. The task goes on.
    TaskLeave,
    /// A terminal pane's text (default: the focused one): the bottom screen, and with `history`
    /// everything scrolled off above it. The reply is `{"ok":true,"pane":3,"text":".."}`, not the state.
    Capture {
        #[serde(default)]
        pane: Option<PaneId>,
        #[serde(default)]
        history: bool,
    },
    /// Replies when a line of a terminal pane's text (history included) matches the regex
    /// `pattern`: `{"ok":true,"pane":3,"line":".."}`. Fails after `timeout_ms` (default 10000,
    /// at most 600000).
    Wait {
        #[serde(default)]
        pane: Option<PaneId>,
        pattern: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// An app pane's accessibility tree (default: the focused pane). Read-only. The reply is
    /// `{"ok":true,"pane":1,"window":{..},"millis":..,"tree":{..}}`, not the state.
    AxTree {
        #[serde(default)]
        pane: Option<PaneId>,
        #[serde(default)]
        depth: Option<usize>,
        #[serde(default)]
        nodes: Option<usize>,
    },
    /// Shows or hides the accessibility overlay on the focused app pane. `refresh`: show it and
    /// read the tree again.
    AxOverlay {
        #[serde(default)]
        refresh: bool,
    },
    /// Sneak mode: a letter label on each of ottospace's own controls. They are `sneak.targets` in
    /// the reply, each with its control's `id` and `frame`, as a frame painted after the request
    /// drew them; type one with `key`. Already on: it stays on. Fails outside normal and list
    /// mode, and if the window is not drawn.
    Sneak,
    /// The marks, in the user's order: `{"ok":true,"marks":[{"name":"notes","key":"n",
    /// "kind":"terminal","open":true,"available":true,..}]}`, not the state.
    Marks,
    /// Marks a pane (default: the focused one), or with `tab` (a tab's `id`) a whole tab. `name`:
    /// default the app, the terminal's directory, or the tab's title. `key`: one letter, `a` to
    /// `z`; a letter another mark has moves here. A pane or tab that is marked already keeps its
    /// mark, which takes what is given. The reply is `{"ok":true,"mark":{..}}`.
    MarkAdd {
        #[serde(default)]
        pane: Option<PaneId>,
        #[serde(default)]
        tab: Option<u32>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        key: Option<char>,
    },
    /// Goes to a mark, by its `name` or its `key`: to its pane or tab if that is open now, else
    /// opens it (`axis`: where a new pane goes).
    MarkGo {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        key: Option<char>,
        #[serde(default)]
        axis: Axis,
    },
    /// Removes a mark, by its `name` or its `key`. What it marks stays open.
    MarkRemove {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        key: Option<char>,
    },
    /// Renames a mark (by its `name` or its `key`) to `to`.
    MarkRename {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        key: Option<char>,
        to: String,
    },
    Quit,
}

fn one() -> usize {
    1
}

fn eight() -> usize {
    8
}

fn three() -> u8 {
    3
}

fn captured() -> State {
    State::Captured
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Down,
    Move,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

/// Why a request is refused, if it is: `mouse` and `drag` answer only in a test run (`e2e`:
/// `OTTOSPACE_E2E` is set). Coordinates would reach every control with no request of its own.
pub fn pointer_refusal(request: &Request, e2e: bool) -> Option<&'static str> {
    let pointer = matches!(request, Request::Mouse { .. } | Request::Drag { .. } | Request::ComposeDrop { .. });
    (pointer && !e2e)
        .then_some("pointer requests are for tests (OTTOSPACE_E2E); use drop_pane, focus, close, enter and the like")
}

/// Pane kinds a client may create. App panes come from `open_app`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneKind {
    #[default]
    Terminal,
    Empty,
}

/// A request waiting for the UI thread, and where its reply goes.
pub struct Call {
    pub request: Request,
    pub reply: Sender<String>,
}

/// `$OTTOSPACE_SOCKET`, else `ottospace.sock` in the per-user temp dir.
pub fn path() -> PathBuf {
    std::env::var_os("OTTOSPACE_SOCKET").map_or_else(|| std::env::temp_dir().join("ottospace.sock"), PathBuf::from)
}

/// Listens on `path()`. Calls arrive on the returned channel; the UI thread polls it.
pub fn serve() -> io::Result<Receiver<Call>> {
    let path = path();
    // One owner per socket path: a lock file held for the life of the process. It closes the
    // gap where two instances starting together each take the other's socket for a stale one.
    let lock = fs::File::create(path.with_extension("lock"))?;
    if lock.try_lock().is_err() || UnixStream::connect(&path).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AddrInUse, "another ottospace owns the socket"));
    }
    // Remove only a stale socket. Never delete some other file the path happens to name.
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() => fs::remove_file(&path)?,
        Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "socket path is an existing file")),
        Err(_) => {}
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    let (calls, receiver) = channel();
    thread::spawn(move || {
        let _lock = lock; // held until the process exits
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let calls = calls.clone();
                    thread::spawn(move || handle(stream, calls));
                }
                // e.g. out of file descriptors: wait, do not spin.
                Err(_) => thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
    });
    Ok(receiver)
}

fn handle(stream: UnixStream, calls: Sender<Call>) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let reply = match serde_json::from_str(&line) {
            Ok(request) => {
                let (reply, answer) = channel();
                if calls.send(Call { request, reply }).is_err() {
                    return;
                }
                answer.recv().unwrap_or_else(|_| error("ottospace is closing"))
            }
            Err(err) => error(&err.to_string()),
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

pub fn error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

/// Client side: sends one request line, returns the reply line.
pub fn send(path: &Path, request: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    writeln!(stream, "{request}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_requests_answer_only_in_a_test_run() {
        let parse = |line: &str| serde_json::from_str::<Request>(line).expect(line);
        let mouse = parse(r#"{"cmd":"mouse","action":"down","x":300,"y":60}"#);
        let drag = parse(r#"{"cmd":"drag","from":[300,60],"to":[900,400]}"#);
        let drop = parse(r#"{"cmd":"drop_pane","source":3,"target":1,"zone":"left"}"#);
        for pointer in [&mouse, &drag] {
            assert!(pointer_refusal(pointer, false).expect("refused").contains("drop_pane"));
            assert_eq!(pointer_refusal(pointer, true), None);
        }
        assert_eq!(pointer_refusal(&drop, false), None);
        assert_eq!(pointer_refusal(&parse(r#"{"cmd":"state"}"#), false), None);
    }
}
