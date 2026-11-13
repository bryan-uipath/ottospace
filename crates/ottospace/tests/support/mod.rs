//! What an e2e test keeps of its run, shared by `e2e.rs` and `e2e_apps.rs`: a timeline of the
//! requests it sent, how long each wait took, and on a failure a folder that explains it.
//!
//! Everything goes under `$OTTOSPACE_E2E_OUT` (default `$TMPDIR/ottospace-e2e`):
//! - `results.jsonl`: one line per app a test launched: its test, pass or fail, times, waits.
//! - `failures/<test>-<pid>/`, one per app still running: `summary.txt`, `state.json` (the last
//!   full state), `timeline.jsonl` (request names and reply numbers, never their text),
//!   `trace.jsonl` (the app's own), `sample.txt` (`sample <pid> 3`, on a timeout) and
//!   `screenshot.png` (the app's window, if the window server allows it).
//!
//! Each test runs on a thread of its own named after it, so the log is the thread's.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// An ottospace the test launched and has not killed yet.
struct App {
    pid: u32,
    socket: PathBuf,
    /// The last full state it replied with: kept for a folder if it stops answering.
    last_state: Option<Value>,
}

#[derive(Default)]
struct Log {
    apps: Vec<App>,
    launched: Option<Instant>,
    first_frame_ms: Option<u64>,
    timeline: Vec<Value>,
    waits: Vec<Value>,
    /// The failure folders, once written.
    folders: Vec<PathBuf>,
    /// What the test made under `$TMPDIR` (`temp`): removed when it ends.
    made: Vec<PathBuf>,
}

/// The test's thread ends, pass or fail: what it made in `$TMPDIR` goes. The failure folders
/// stay: they are under `out()`, never in `made`.
impl Drop for Log {
    fn drop(&mut self) {
        for path in &self.made {
            let _ = std::fs::remove_dir_all(path);
            let _ = std::fs::remove_file(path);
        }
    }
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::default();
}

/// `$TMPDIR/<name>` for the test to make something at, removed when the test ends, pass or
/// fail. Starts empty: what an earlier run left there is removed now.
pub fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_file(&path);
    LOG.with_borrow_mut(|log| log.made.push(path.clone()));
    path
}

/// A leftover of this suite in `$TMPDIR`: `ottospace-e2e-<words>-<pid>[-<n>]`, a folder or one
/// with `.sock`, `.lock` or `.trace.jsonl`. Not `ottospace-e2e` (the failure folders), nor the
/// user's own `ottospace.sock`.
pub fn is_leftover(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("ottospace-e2e-") else { return false };
    let stem = [".sock", ".lock", ".trace.jsonl"].iter().find_map(|end| rest.strip_suffix(end)).unwrap_or(rest);
    let words: Vec<&str> = stem.split('-').collect();
    let plain = |w: &&str| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    // A name, then one to three numbers: `keep-1-4242`, `marks-4242-2`.
    let numbers = words.iter().rev().take_while(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit())).count();
    words.iter().all(plain) && (1..=3).contains(&numbers) && numbers < words.len()
}

/// Once per run: removes this suite's leftovers in `$TMPDIR` older than a day. Only names
/// `is_leftover` takes, only plain files and folders right there; a link is never followed.
pub fn sweep() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
        let day = Duration::from_secs(24 * 3600);
        for entry in entries.flatten() {
            let (path, name) = (entry.path(), entry.file_name());
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            let old = meta.modified().ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age > day);
            if !old || !name.to_str().is_some_and(is_leftover) || meta.file_type().is_symlink() {
                continue;
            }
            let _ = if meta.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    });
}

#[test]
fn only_this_suites_own_leftovers_are_swept() {
    for name in [
        "ottospace-e2e-splits-4242.sock",
        "ottospace-e2e-splits-4242.lock",
        "ottospace-e2e-keep-1-4242.lock",
        "ottospace-e2e-home-tasks-4242",
        "ottospace-e2e-repos-4242",
        "ottospace-e2e-marks-4242-2.lock",
        "ottospace-e2e-zz-4242.trace.jsonl",
    ] {
        assert!(is_leftover(name), "{name}");
    }
    for name in [
        "ottospace.sock",
        "ottospace.lock",
        "ottospace-e2e",
        "ottospace-e2e-",
        "ottospace-e2e-splits",
        "ottospace-e2e-splits-4242.txt",
        "ottospace-e2e-Splits-4242",
        "ottospace-e2e-a/b-4242",
        "ottospace-e2e-..-4242",
        "ottospace-e2e-a--4242",
        "ottospace-e2e-a-4242-1-2-3",
        "ottospace-sessions-4242-git",
        "my-ottospace-e2e-splits-4242",
    ] {
        assert!(!is_leftover(name), "{name}");
    }
}

fn ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

fn test_name() -> String {
    std::thread::current().name().unwrap_or("unnamed").replace("::", "-")
}

/// A request line's `cmd`, never the rest: a label must not carry what a test types.
fn cmd(request: &str) -> String {
    let parsed = serde_json::from_str::<Value>(request).ok();
    parsed.and_then(|r| r["cmd"].as_str().map(str::to_string)).unwrap_or_else(|| "?".into())
}

/// Where the run's results and failure folders go.
pub fn out() -> PathBuf {
    std::env::var_os("OTTOSPACE_E2E_OUT").map_or_else(|| std::env::temp_dir().join("ottospace-e2e"), PathBuf::from)
}

/// An app is up on `socket`. The timeline counts from the first launch of the test.
pub fn launched(pid: u32, socket: &Path) {
    sweep();
    LOG.with_borrow_mut(|log| {
        log.apps.push(App { pid, socket: socket.to_path_buf(), last_state: None });
        let start = *log.launched.get_or_insert_with(Instant::now);
        log.timeline.push(json!({ "ms": ms(start), "event": "launch", "pid": pid }));
    });
}

/// One request to `socket` and its reply: the request's `cmd`, how long the reply took, and the
/// reply's frame numbers. `who`: `otto`, or another process the test drives.
pub fn request(who: &str, socket: &Path, request: &str, started: Instant, reply: Option<&Value>) {
    let mut entry = json!({ "who": who, "cmd": cmd(request), "took_ms": ms(started), "ok": null });
    if let Some(reply) = reply {
        entry["ok"] = reply["ok"].clone();
        if let Some(frame) = reply.get("frame") {
            entry["frame"] = frame["number"].clone();
            entry["seq"] = frame["seq"].clone();
            entry["drawn"] = frame["drawn"].clone();
            entry["settle_ms"] = frame["waited_ms"].clone();
        }
        if reply.get("stale").is_some() {
            entry["stale"] = json!(true);
        }
    }
    LOG.with_borrow_mut(|log| {
        let at = log.launched.map_or(0, ms);
        entry["ms"] = json!(at.saturating_sub(entry["took_ms"].as_u64().unwrap_or(0)));
        if let Some(app) = log.apps.iter_mut().find(|app| app.socket == socket) {
            entry["pid"] = json!(app.pid);
            if reply.is_some_and(|r| r.get("frame").is_some()) {
                app.last_state = reply.cloned();
            }
        }
        // Launch to the first reply that shows a drawn frame.
        if log.first_frame_ms.is_none() && entry["frame"].as_u64() > Some(0) {
            log.first_frame_ms = Some(at);
        }
        log.timeline.push(entry);
    });
}

/// A wait for `what` ended, met or not.
pub fn waited(what: &str, started: Instant, met: bool) {
    let entry = json!({ "what": what, "ms": ms(started), "met": met });
    LOG.with_borrow_mut(|log| {
        let at = log.launched.map_or(0, ms);
        log.timeline.push(json!({ "ms": at, "event": "wait", "what": what, "took_ms": ms(started), "met": met }));
        log.waits.push(entry);
    });
}

/// A wait or a reply timed out: writes the failure folders with a stack sample of each app,
/// and returns the panic message, which names them.
pub fn timed_out(what: &str, started: Instant, detail: &str) -> String {
    waited(what, started, false);
    let headline = format!("timed out after {} ms waiting for {what}", ms(started));
    let summary = write_folders(&headline, detail, true);
    format!("{headline}\n{summary}")
}

/// No reply came from `socket` to `line`: records it, writes the folders, and returns the panic
/// message.
pub fn no_reply(who: &str, socket: &Path, line: &str, started: Instant, err: &std::io::Error) -> String {
    request(who, socket, line, started, None);
    timed_out(&format!("{who}'s reply to {}", cmd(line)), started, &err.to_string())
}

/// The app `pid` is about to be killed, at the end of the test or on a failure. A failure
/// writes the folders first, while it still runs. Its result goes in `results.jsonl`.
pub fn exiting(pid: u32) {
    let failed = std::thread::panicking();
    if failed && LOG.with_borrow(|log| log.folders.is_empty()) {
        eprintln!("{}", write_folders("the test failed (see its panic above)", "", false));
    }
    let line = LOG.with_borrow_mut(|log| {
        let at = log.apps.iter().position(|app| app.pid == pid)?;
        log.apps.remove(at);
        let calls = log.timeline.iter().filter(|e| e.get("cmd").is_some()).count();
        Some(json!({
            "test": test_name(), "pid": pid, "ok": !failed,
            "total_ms": log.launched.map(ms), "first_frame_ms": log.first_frame_ms,
            "requests": calls, "waits": std::mem::take(&mut log.waits), "folders": log.folders,
        }))
    });
    let Some(line) = line else { return };
    let out = out();
    let _ = std::fs::create_dir_all(&out);
    // One write per line: tests in other processes append to the same file.
    let file = std::fs::OpenOptions::new().create(true).append(true).open(out.join("results.jsonl"));
    if let Ok(mut file) = file {
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}

/// One request with a short wait: the app may be the thing that is stuck.
fn ask(socket: &Path, request: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(8))).ok()?;
    writeln!(stream, "{request}").ok()?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    serde_json::from_str(&reply).ok()
}

/// A folder for each app still running; returns the summary: where they are, why, the last
/// three requests.
fn write_folders(headline: &str, detail: &str, sample: bool) -> String {
    let (apps, timeline) = LOG.with_borrow(|log| {
        let apps: Vec<(u32, PathBuf, Option<Value>)> =
            log.apps.iter().map(|a| (a.pid, a.socket.clone(), a.last_state.clone())).collect();
        (apps, log.timeline.clone())
    });
    if apps.is_empty() {
        return format!("{headline} (no app was running)");
    }
    let mut summary = vec![format!("  {headline}")];
    if !detail.is_empty() {
        summary.push(format!("  detail: {}", detail.chars().take(300).collect::<String>()));
    }
    let mut folders = Vec::new();
    for (pid, socket, last_state) in apps {
        let (dir, notes, frame) = write_folder(pid, &socket, last_state, &timeline, sample);
        summary.insert(folders.len(), format!("e2e failure folder: {}", dir.display()));
        summary.extend(
            frame.map(|f| format!("  pid {pid}: frame {} drew seq {} of {}", f["number"], f["drawn"], f["seq"])),
        );
        summary.extend(notes.into_iter().map(|note| format!("  pid {pid}: {note}")));
        folders.push(dir);
    }
    summary.push("  last requests:".to_string());
    let calls: Vec<&Value> = timeline.iter().filter(|e| e.get("cmd").is_some()).collect();
    summary.extend(calls.iter().rev().take(3).rev().map(|entry| format!("    {entry}")));
    let summary = summary.join("\n");
    for dir in &folders {
        let _ = std::fs::write(dir.join("summary.txt"), format!("{summary}\n"));
    }
    LOG.with_borrow_mut(|log| log.folders = folders);
    summary
}

/// One app's folder. Returns it, what could not be captured, and the state's frame numbers.
fn write_folder(
    pid: u32,
    socket: &Path,
    last_state: Option<Value>,
    timeline: &[Value],
    sample: bool,
) -> (PathBuf, Vec<String>, Option<Value>) {
    let dir = out().join("failures").join(format!("{}-{pid}", test_name()));
    let _ = std::fs::create_dir_all(&dir);
    let mut notes = Vec::new();
    // The stack first: it is the app at the moment of the timeout, before the requests below.
    if sample {
        let path = dir.join("sample.txt");
        let ran = Command::new("/usr/bin/sample").arg(pid.to_string()).arg("3").arg("-file").arg(&path).output();
        if !ran.is_ok_and(|out| out.status.success()) {
            notes.push("sample: could not sample the app".to_string());
        }
    }
    let state = ask(socket, r#"{"cmd":"state"}"#).or_else(|| {
        notes.push("state: no reply within 8 s; state.json is the last state it sent".to_string());
        last_state
    });
    if let Some(state) = &state {
        let _ = std::fs::write(dir.join("state.json"), state.to_string());
    }
    match ask(socket, r#"{"cmd":"trace"}"#).and_then(|r| r["trace"].as_str().map(str::to_string)) {
        Some(trace) => drop(std::fs::write(dir.join("trace.jsonl"), trace)),
        // A stuck UI thread answers nothing: the app writes its trace beside the socket on SIGUSR1.
        None => {
            let _ = Command::new("/bin/kill").args(["-USR1", &pid.to_string()]).status();
            std::thread::sleep(Duration::from_millis(300));
            let dumped = socket.with_extension("trace.jsonl");
            if std::fs::copy(&dumped, dir.join("trace.jsonl")).is_err() {
                notes.push("trace: no reply, and no file after SIGUSR1".to_string());
            }
            let _ = std::fs::remove_file(dumped);
        }
    }
    let lines: String = timeline.iter().map(|entry| format!("{entry}\n")).collect();
    let _ = std::fs::write(dir.join("timeline.jsonl"), lines);
    // The app's own window, by its number: never the screen.
    let window = state.as_ref().and_then(|s| s["frame"]["window"].as_i64()).filter(|n| *n > 0);
    if let Some(window) = window {
        let shot = dir.join("screenshot.png");
        let _ =
            Command::new("/usr/sbin/screencapture").args(["-x", "-o", "-l", &window.to_string()]).arg(&shot).status();
        if !shot.exists() {
            notes.push("screenshot: the window server refused".to_string());
        }
    }
    (dir, notes, state.map(|s| s["frame"].clone()))
}
