//! What an e2e test keeps of its run, shared by `e2e.rs` and `e2e_apps.rs`: a timeline of the
//! requests it sent, how long each wait took, and on a failure a folder that explains it.
//!
//! Everything goes under `$OTTOSPACE_E2E_OUT` (default `$TMPDIR/ottospace-e2e`):
//! - `results.jsonl`: one line per app a test launched: its test, pass or fail, times, waits.
//! - `failures/<test>-<pid>/`: `summary.txt`, `state.json` (the last full state), `timeline.jsonl`
//!   (request names and reply numbers, never their text), `trace.jsonl` (the app's own),
//!   `sample.txt` (`sample <pid> 3`, on a timeout) and `screenshot.png` (the app's window, if
//!   the window server allows it).
//!
//! Each test runs on a thread of its own named after it, so the log is the thread's.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[derive(Default)]
struct Log {
    /// The app the test drives: its pid and socket.
    app: Option<(u32, PathBuf)>,
    launched: Option<Instant>,
    first_frame_ms: Option<u64>,
    timeline: Vec<Value>,
    waits: Vec<Value>,
    /// The failure folder, once written.
    folder: Option<PathBuf>,
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::default();
}

fn ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

fn test_name() -> String {
    std::thread::current().name().unwrap_or("unnamed").replace("::", "-")
}

/// Where the run's results and failure folders go.
pub fn out() -> PathBuf {
    std::env::var_os("OTTOSPACE_E2E_OUT").map_or_else(|| std::env::temp_dir().join("ottospace-e2e"), PathBuf::from)
}

/// The app is up on `socket`. The timeline counts from the first launch of the test.
pub fn launched(pid: u32, socket: &Path) {
    LOG.with_borrow_mut(|log| {
        log.app = Some((pid, socket.to_path_buf()));
        let start = *log.launched.get_or_insert_with(Instant::now);
        log.timeline.push(json!({ "ms": ms(start), "event": "launch", "pid": pid }));
    });
}

/// One request and its reply: the request's `cmd`, how long the reply took, and the reply's
/// frame numbers. `who`: `otto`, or another process the test drives.
pub fn request(who: &str, request: &str, started: Instant, reply: Option<&Value>) {
    let cmd = serde_json::from_str::<Value>(request).ok().and_then(|r| r["cmd"].as_str().map(str::to_string));
    let mut entry = json!({ "who": who, "cmd": cmd, "took_ms": ms(started) });
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
    } else {
        entry["ok"] = json!(null);
    }
    LOG.with_borrow_mut(|log| {
        let at = log.launched.map_or(0, ms);
        entry["ms"] = json!(at.saturating_sub(entry["took_ms"].as_u64().unwrap_or(0)));
        // Launch to the first reply that shows a drawn frame.
        if who == "otto" && log.first_frame_ms.is_none() && entry["frame"].as_u64() > Some(0) {
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

/// A wait or a reply timed out: writes the failure folder with a stack sample of the app, and
/// returns the panic message, which names the folder.
pub fn timed_out(what: &str, started: Instant, detail: &str) -> String {
    waited(what, started, false);
    let headline = format!("timed out after {} ms waiting for {what}", ms(started));
    let summary = write_folder(&headline, detail, true);
    format!("{headline}\n{summary}")
}

/// A test that fails without a timeout (an assert): the folder, without a stack sample. Call it
/// while the app still runs, before it is killed.
pub fn failed_without_timeout() {
    if LOG.with_borrow(|log| log.folder.is_none() && log.app.is_some()) {
        let summary = write_folder("the test failed (see its panic above)", "", false);
        eprintln!("{summary}");
    }
}

/// The app is gone (killed at the end of the test, or on a failure): its result goes in
/// `results.jsonl`.
pub fn finish(ok: bool) {
    let line = LOG.with_borrow_mut(|log| {
        let (pid, _) = log.app.take()?;
        let calls = log.timeline.iter().filter(|e| e.get("cmd").is_some()).count();
        Some(json!({
            "test": test_name(), "pid": pid, "ok": ok,
            "total_ms": log.launched.map(ms), "first_frame_ms": log.first_frame_ms,
            "requests": calls, "waits": std::mem::take(&mut log.waits), "folder": log.folder,
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

/// Writes the failure folder; returns its summary: where it is, why, the last three requests.
fn write_folder(headline: &str, detail: &str, sample: bool) -> String {
    let (app, timeline) = LOG.with_borrow(|log| (log.app.clone(), log.timeline.clone()));
    let Some((pid, socket)) = app else { return format!("{headline} (no app was launched)") };
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
    let state = ask(&socket, r#"{"cmd":"state"}"#);
    match &state {
        Some(state) => drop(std::fs::write(dir.join("state.json"), state.to_string())),
        None => notes.push("state: no reply within 8 s".to_string()),
    }
    match ask(&socket, r#"{"cmd":"trace"}"#).and_then(|r| r["trace"].as_str().map(str::to_string)) {
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
    let mut summary = vec![format!("e2e failure folder: {}", dir.display()), format!("  {headline}")];
    if !detail.is_empty() {
        summary.push(format!("  detail: {}", detail.chars().take(300).collect::<String>()));
    }
    if let Some(frame) = state.as_ref().map(|s| &s["frame"]) {
        summary.push(format!("  frame {} drew seq {} of {}", frame["number"], frame["drawn"], frame["seq"]));
    }
    summary.push("  last requests:".to_string());
    let calls: Vec<&Value> = timeline.iter().filter(|e| e.get("cmd").is_some()).collect();
    for entry in calls.iter().rev().take(3).rev() {
        summary.push(format!("    {entry}"));
    }
    summary.extend(notes.into_iter().map(|note| format!("  {note}")));
    let summary = summary.join("\n");
    let _ = std::fs::write(dir.join("summary.txt"), format!("{summary}\n"));
    LOG.with_borrow_mut(|log| log.folder = Some(dir));
    summary
}
