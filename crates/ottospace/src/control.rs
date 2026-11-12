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

use ottospace_core::{Axis, Dir, PaneId, Section};
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
    /// Next (positive) or previous tab.
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
    /// Turns the scroll wheel over a terminal pane (default: the focused one). Positive `lines`
    /// scrolls back.
    Scroll {
        #[serde(default)]
        pane: Option<PaneId>,
        lines: f32,
    },
    Quit,
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
