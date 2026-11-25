//! `<home>/log`: what happened to the app, for its owner to read after a problem. Private (0600),
//! appended, one JSON line an event: `{"at":"2026-11-16T17:51:03-08:00","ev":"displays","count":1,..}`.
//! At 1 MB it becomes `log.1`, and the one before goes: 2 MB at most.
//! Events only, as `trace`: names, counts, sizes, ids and versions. Never terminal or transcript
//! text, titles, meetings, passes, paths or file contents.
//! Each line also goes to the unified log (`ottospace_mac::oslog`), outside a test run.
//! A test run writes the file only in the folder it names (`OTTOSPACE_HOME`).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ottospace_core::calendar::reply::rfc3339;
use ottospace_core::ics::Table;
use serde_json::{Value, json};

use crate::config;

const NAME: &str = "log";
const OLD: &str = "log.1";
/// The size at which the file is turned over.
const LIMIT: u64 = 1 << 20;

/// Writes `{"at":..,"ev":event, ..fields}`. `fields`: names, numbers and ids only.
pub(crate) fn event(event: &str, fields: Value) {
    write(event, fields, config::get().home.as_deref());
}

fn write(event: &str, fields: Value, home: Option<&Path>) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64);
    let line = line(now, ottospace_mac::time::utc_offset(now), event, fields);
    // Unit tests write only their own folder.
    if !cfg!(test) && std::env::var_os("OTTOSPACE_E2E").is_none() {
        ottospace_mac::oslog::write(&line);
    }
    if let Some(home) = home {
        let _ = append(home, &line);
    }
}

/// The log's last `most` lines as written (`log.1`'s before `log`'s), for a bug report's brief,
/// which keeps their names and numbers only. None kept (a test run with no folder): none.
pub(crate) fn last_lines(most: usize) -> Vec<String> {
    let Some(home) = config::get().home.clone() else { return Vec::new() };
    let read = |name: &str| crate::collect::read_plain(&home.join(name), LIMIT * 2).unwrap_or_default();
    let text = String::from_utf8_lossy(&[read(OLD), read(NAME)].concat()).into_owned();
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    lines[lines.len().saturating_sub(most)..].iter().map(|line| line.to_string()).collect()
}

/// Every panic, where it happened (`crate/src/file.rs:line`) and on which thread; never its
/// message, which may carry any text. And GPUI's own errors and warnings from its macOS layer
/// (`log_err`), which went nowhere: where, and what it said only when that is fixed words and a
/// code ("could not create display link, code: -6661").
pub(crate) fn arm() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|at| (at.file(), at.line()));
        // Not `config::get`: the panic may be in reading it.
        let home = config::loaded().and_then(|config| config.home.as_deref());
        write("panic", panic_fields(at, std::thread::current().name()), home);
        before(info);
    }));
    if log::set_logger(&Gpui).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

/// The part of a source path from its crate's folder: `/Users/me/.cargo/registry/src/../gpui-0.2.2/src/window.rs`
/// gives `gpui-0.2.2/src/window.rs`. Below `home` first, so `~/src/app/out/gen.rs` is `gen.rs`, not `me/src/..`.
fn short<'a>(file: &'a str, home: &str) -> &'a str {
    let home = home.trim_end_matches('/');
    let file = file.strip_prefix(home).filter(|rest| !home.is_empty() && rest.starts_with('/')).unwrap_or(file);
    let name = file.rsplit('/').next().unwrap_or(file);
    let Some(src) = file.rfind("/src/") else { return name };
    file[..src].rfind('/').map_or(name, |start| &file[start + 1..])
}

fn panic_fields(at: Option<(&str, u32)>, thread: Option<&str>) -> Value {
    let src = at.map(|(file, line)| format!("{}:{line}", short(file, &config::user_home())));
    json!({ "src": src, "thread": thread })
}

/// GPUI's words when they are fixed words and a code ("could not create display link, code: -6661"):
/// never text a newer GPUI might format into a message.
fn fixed(text: &str) -> Option<&str> {
    let first = text.lines().next()?;
    let (words, code) = first.split_once(", code: ")?;
    let plain = words.len() <= 60 && words.chars().all(|c| c.is_ascii_lowercase() || c == ' ');
    (plain && code.parse::<i64>().is_ok()).then_some(first)
}

/// A `gpui` event's fields for a record from GPUI's macOS layer; `None` for any other record.
fn gpui_fields(record: &log::Record) -> Option<Value> {
    let file = record.file().unwrap_or_default();
    if record.level() > log::Level::Warn || !(file.contains("/gpui-") && file.contains("/src/platform/mac/")) {
        return None;
    }
    let at = format!("{}:{}", short(file, &config::user_home()), record.line().unwrap_or(0));
    let said = fixed(&record.args().to_string()).map(String::from);
    Some(json!({ "level": record.level().as_str(), "src": at, "said": said }))
}

/// Counts a record from `src`: `Some(n)` at its 1st, 2nd, 4th, 8th.. time, so one that comes every
/// frame does not fill the log.
fn nth(seen: &mut BTreeMap<String, u64>, src: &str) -> Option<u64> {
    let n = seen.entry(src.to_string()).or_default();
    *n += 1;
    n.is_power_of_two().then_some(*n)
}

/// GPUI's records from its macOS layer, as `gpui` events.
struct Gpui;

impl log::Log for Gpui {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        static SEEN: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());
        let Some(mut fields) = gpui_fields(record) else { return };
        let mut seen = SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(n) = nth(&mut seen, fields["src"].as_str().unwrap_or_default()) {
            drop(seen);
            fields["n"] = json!(n);
            event("gpui", fields);
        }
    }

    fn flush(&self) {}
}

/// The line for `event` at `now` (seconds since 1970), in local time `offset` seconds east of UTC.
fn line(now: i64, offset: i64, event: &str, fields: Value) -> String {
    let (at, ev) = (json!(local_time(now, offset)), json!(event));
    let mut line = json!({ "at": at, "ev": ev });
    if let (Some(line), Value::Object(fields)) = (line.as_object_mut(), fields) {
        // Only the fields on the event's list: never `at` or `ev`, nor a field added without review.
        let listed = fields_of(event);
        line.extend(fields.into_iter().filter(|(name, _)| {
            let known = listed.contains(&name.as_str());
            debug_assert!(known, "{event}: field {name:?} is not on its list");
            known
        }));
    }
    line.to_string()
}

/// The fields each event may have: names, numbers, ids and sizes. A new one is a privacy review.
fn fields_of(event: &str) -> &'static [&'static str] {
    match event {
        "start" => &["pid", "version", "commit", "hosting", "displays"],
        "quit" => &["hosted"],
        "displays" => &["count", "sizes"],
        "window_display" => &["display", "size"],
        "stall" => &["stalled_ms", "n"],
        "redrawn" => &["kicks", "lasted_ms", "n"],
        "host_failed" | "give_back_failed" => &["window"],
        "gpui" => &["level", "src", "said", "n"],
        "panic" => &["src", "thread"],
        // The calendar's fetch: never its link, a title or curl's words.
        "calendar_fetch" => &["try"],
        "calendar_read" => &["bytes", "vevents", "held", "ms", "parse_ms"],
        "calendar_failed" => &["error", "curl_exit", "http", "ms", "try"],
        // A transcript's sound: the app's bundle id and counts, never a title or a word heard.
        "listen_audio" => &["bundle", "pids", "playing", "unoffered"],
        "listen_audio_filter" => &["bundle", "pids"],
        "listen_quiet" => &["bundle", "pids", "buffers"],
        "listen_counts" => &["bundle", "buffers", "loud", "lines"],
        // The picker's list of an app's windows: how many, and why each other was left out.
        "pick_list" => &["bundle", "listed", "skipped"],
        // The developer's timed spin of a code city: frame times and counts only.
        "city_spin" => &[
            "steps",
            "done",
            "frames",
            "mean",
            "p50",
            "p95",
            "p99",
            "max",
            "over_16",
            "over_33",
            "repeats",
            "wait_p50",
            "wait_p95",
            "wait_max",
            "shown_p50",
            "shown_p95",
            "shown_p99",
            "render_p50",
            "render_p95",
            "live",
            "boxes",
        ],
        // Why a waiting slot is offered nothing: counts by reason code, never a title or id.
        "slot_miss" => &["pane", "bundle", "why"],
        // The developer's timed typing: times by stage and counts only.
        "typing_time" => &[
            "keys",
            "missed",
            "key_write",
            "write_echo",
            "echo_taken",
            "taken_painted",
            "app",
            "total",
            "full",
            "busy",
            "done",
            "hz",
        ],
        // Where a new window of a hosted app went, and why: never its title.
        "place" => {
            &["bundle", "subrole", "modal", "sheet", "size", "pane", "min_width", "place", "why", "rule", "slot"]
        }
        _ => &[],
    }
}

/// How long frames must flow, with no stall, before a stall counts as the 1st again.
const CALM: Duration = Duration::from_secs(60);

/// Stalls in a row (a locked screen and a busy terminal: one a second), heard at the 1st, 2nd,
/// 4th, 8th..; a stall `CALM` after the last starts over.
#[derive(Default)]
pub(crate) struct Streak {
    n: u64,
    last: Option<Instant>,
    /// The current stall was written: so is its `redrawn`.
    heard: bool,
}

impl Streak {
    /// A stall starts at `now`: `Some(n)` when it is written.
    pub(crate) fn stall(&mut self, now: Instant) -> Option<u64> {
        if self.last.is_some_and(|last| now - last >= CALM) {
            self.n = 0;
        }
        (self.n, self.last) = (self.n + 1, Some(now));
        self.heard = self.n.is_power_of_two();
        self.heard.then_some(self.n)
    }

    /// The stall ended: `Some(n)` when its start was written.
    pub(crate) fn redrawn(&mut self) -> Option<u64> {
        std::mem::take(&mut self.heard).then_some(self.n)
    }
}

/// `2026-11-16T17:51:03-08:00`.
fn local_time(now: i64, offset: i64) -> String {
    rfc3339(u64::try_from(now).unwrap_or(0) * 1000, &Table::fixed(offset))
}

/// Appends `line` to `dir/log`, turning a full file over first. Never through a link, nor into
/// anything but a plain file (a pipe would hang). One at a time, across threads and processes (a
/// lock on the folder): two that turn a full file over at once would lose the first's `log.1`.
fn append(dir: &Path, line: &str) -> io::Result<()> {
    if !dir.is_dir() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    // Held until it closes, as this returns.
    let lock = fs::File::open(dir)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let path = dir.join(NAME);
    if fs::symlink_metadata(&path).is_ok_and(|meta| meta.len() >= LIMIT) {
        // A rename replaces `log.1` itself, a link there included.
        fs::rename(&path, dir.join(OLD))?;
    }
    let flags = libc::O_NOFOLLOW | libc::O_NONBLOCK;
    let mut file = fs::OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(flags).open(&path)?;
    // A hard link would have another file chmodded and written.
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 {
        return Err(io::Error::other("not a plain file of its own"));
    }
    // Private whatever made it.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(format!("{line}\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn temp(name: &str) -> Scratch {
        Scratch::path(&format!("ottospace-test-event-log-{name}-{}", std::process::id()))
    }

    #[test]
    fn a_line_is_the_local_time_the_event_and_its_fields() {
        // 2026-11-17T01:51:03Z, in a zone 8 hours west.
        let line = line(1_794_880_263, -8 * 3600, "displays", json!({ "count": 1 }));
        assert_eq!(line, r#"{"at":"2026-11-16T17:51:03-08:00","ev":"displays","count":1}"#);
        assert_eq!(local_time(0, 5 * 3600 + 30 * 60), "1970-01-01T05:30:00+05:30");
    }

    #[test]
    fn the_file_is_private_appended_and_turned_over_when_full() {
        let dir = temp("turn");
        append(&dir, "one").unwrap();
        append(&dir, "two").unwrap();
        let path = dir.join(NAME);
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Full: it becomes `log.1`, and the next line starts a new file.
        fs::write(&path, vec![b'x'; LIMIT as usize]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        append(&dir, "three").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "three\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(dir.join(OLD)).unwrap().len(), LIMIT);
        // Full again: the oldest goes.
        fs::write(&path, vec![b'y'; LIMIT as usize]).unwrap();
        append(&dir, "four").unwrap();
        assert_eq!(fs::read(dir.join(OLD)).unwrap()[0], b'y');
        assert_eq!(fs::read_to_string(&path).unwrap(), "four\n");
    }

    #[test]
    fn a_path_keeps_its_crate_and_never_the_folders_above() {
        let gpui = "/Users/me/.cargo/registry/src/index.crates.io-1949/gpui-0.2.2/src/platform/mac/window.rs";
        assert_eq!(short(gpui, "/Users/me"), "gpui-0.2.2/src/platform/mac/window.rs");
        assert_eq!(short("crates/ottospace/src/main.rs", "/Users/me"), "ottospace/src/main.rs");
        assert_eq!(short("/rustc/abc/library/core/src/option.rs", "/Users/me"), "core/src/option.rs");
        assert_eq!(short("/Users/me/build.rs", "/Users/me"), "build.rs");
        // A build script's output in a repository under `~/src`.
        assert_eq!(short("/Users/me/src/app/target/debug/build/x-1/out/gen.rs", "/Users/me/"), "gen.rs");
    }

    #[test]
    fn stalls_in_a_row_are_heard_at_their_first_second_fourth_time_and_again_after_calm() {
        let (mut streak, start) = (Streak::default(), Instant::now());
        let mut heard = Vec::new();
        // One a second for 9 seconds, each redrawn.
        for s in 0..9 {
            let n = streak.stall(start + Duration::from_secs(s));
            assert_eq!(streak.redrawn(), n, "a redrawn is written with its stall");
            heard.extend(n);
        }
        assert_eq!(heard, [1, 2, 4, 8]);
        assert_eq!(streak.redrawn(), None, "once");
        // Not calm long enough: the 10th.
        assert_eq!(streak.stall(start + Duration::from_secs(8) + CALM - Duration::from_secs(1)), None);
        // Calm: the 1st again.
        assert_eq!(streak.stall(start + Duration::from_secs(70) + CALM), Some(1));
    }

    #[test]
    fn a_field_off_its_events_list_is_dropped() {
        let fields = json!({ "window": 7, "title": "Fake Secret Title", "at": "then" });
        let line = std::panic::catch_unwind(|| line(0, 0, "host_failed", fields.clone()));
        if cfg!(debug_assertions) {
            assert!(line.is_err(), "a debug build stops at it");
        } else {
            assert_eq!(line.unwrap(), r#"{"at":"1970-01-01T00:00:00+00:00","ev":"host_failed","window":7}"#);
        }
    }

    #[test]
    fn every_event_kind_is_on_the_list_with_its_fields() {
        for (event, fields) in samples(&config::user_home()) {
            let listed = fields_of(event);
            assert!(!listed.is_empty(), "{event} has no list");
            for name in fields.as_object().unwrap().keys() {
                assert!(listed.contains(&name.as_str()), "{event}: {name}");
            }
        }
    }

    #[test]
    fn gpui_is_heard_at_its_first_second_fourth_time() {
        let mut seen = BTreeMap::new();
        let heard: Vec<u64> = (0..9).filter_map(|_| nth(&mut seen, "a.rs:1")).collect();
        assert_eq!(heard, [1, 2, 4, 8]);
        assert_eq!(nth(&mut seen, "b.rs:1"), Some(1));
    }

    const PASS: &str = "hunter2-fake-pass";
    const TITLE: &str = "Fake Secret Title";

    fn gpui_record(level: log::Level, file: &str, text: &str) -> Option<Value> {
        gpui_fields(
            &log::Record::builder().level(level).file(Some(file)).line(Some(489)).args(format_args!("{text}")).build(),
        )
    }

    /// Every kind of event, fed what could leak (a pass, a title, `home`), as the app builds it:
    /// none of that reaches the file.
    fn samples(home: &str) -> Vec<(&'static str, Value)> {
        let src = format!("{home}/.cargo/registry/src/index.crates.io-1949/gpui-0.2.2/src/platform/mac");
        let gpui = |file: &str, text: &str| gpui_record(log::Level::Error, &format!("{src}/{file}"), text).unwrap();
        let displays = json!({ "count": 2, "sizes": ["1728x1117", "2560x1440"] });
        let start =
            json!({ "pid": 42, "version": "0.1.0", "commit": "abc1234", "hosting": true, "displays": displays });
        let main = format!("{home}/code/ottospace/crates/ottospace/src/main.rs");
        vec![
            ("start", start),
            ("displays", displays.clone()),
            ("window_display", json!({ "display": 1, "size": "1728x1117" })),
            ("stall", json!({ "stalled_ms": 1500, "n": 1 })),
            ("redrawn", json!({ "kicks": 3, "lasted_ms": 2100, "n": 1 })),
            ("host_failed", json!({ "window": 77 })),
            ("give_back_failed", json!({ "window": 77 })),
            ("quit", json!({ "hosted": 1 })),
            ("calendar_fetch", json!({ "try": 1 })),
            ("calendar_read", json!({ "bytes": 1_557_023, "vevents": 1858, "held": 150, "ms": 4137, "parse_ms": 5 })),
            (
                "calendar_failed",
                json!({ "error": "timed_out", "curl_exit": null, "http": null, "ms": 30_002, "try": 1 }),
            ),
            ("listen_audio", json!({ "bundle": "us.zoom.xos", "pids": 2, "playing": true, "unoffered": 0 })),
            ("listen_audio_filter", json!({ "bundle": "us.zoom.xos", "pids": 3 })),
            ("listen_quiet", json!({ "bundle": "us.zoom.xos", "pids": 1, "buffers": 0 })),
            ("listen_counts", json!({ "bundle": "us.zoom.xos", "buffers": 500, "loud": 0, "lines": 0 })),
            (
                "place",
                json!({ "bundle": "us.zoom.xos", "subrole": "AXStandardWindow", "modal": false, "sheet": false,
                    "size": [1200.0, 800.0], "pane": [900.0, 700.0], "min_width": null, "place": "slot_offer",
                    "why": "slot", "rule": "windows[0]: manage", "slot": 3 }),
            ),
            (
                "pick_list",
                json!({ "bundle": "us.zoom.xos", "listed": 2,
                    "skipped": [{ "why": "built-in rule 0: ignore", "subrole": "AXSystemDialog" }] }),
            ),
            (
                "city_spin",
                json!({ "steps": 240, "done": true, "frames": 240, "mean": 16.7, "p50": 16.6, "p95": 17.1, "p99": 33.4,
                    "max": 50.1, "over_16": 9, "over_33": 2, "repeats": 4, "wait_p50": 5.2, "wait_p95": 12.0,
                    "wait_max": 20.4, "boxes": 10392, "shown_p50": 9.1, "shown_p95": 15.0, "shown_p99": 18.0, "render_p50": 3.1, "render_p95": 4.2,
                    "live": false }),
            ),
            ("slot_miss", json!({ "pane": 3, "bundle": "us.zoom.xos", "why": { "no_id": 1, "in_pane": 1 } })),
            (
                "typing_time",
                json!({ "keys": 250, "missed": 0, "app": { "n": 250, "p50": 0.4, "p95": 9.1, "p99": 15.8, "max": 16.3 },
                    "total": { "n": 250, "p50": 18.0, "p95": 29.2, "p99": 31.0, "max": 33.0 }, "full": false,
                    "busy": false, "done": true, "hz": 75.0 }),
            ),
            ("panic", panic_fields(Some((&main, 12)), Some("main"))),
            ("panic", panic_fields(Some((&format!("{home}/src/app/target/debug/build/x-1/out/gen.rs"), 3)), None)),
            // The first line only.
            ("gpui", gpui("display_link.rs", &format!("could not create display link, code: -6661\n{PASS} {TITLE}"))),
            (
                "gpui",
                gpui("window.rs", &format!("could not start display link, code: -6660 {}", "x".repeat(200) + PASS)),
            ),
            // No words but fixed ones and a code.
            ("gpui", gpui("text_system.rs", &format!("font {TITLE} for {home}"))),
        ]
    }

    #[test]
    fn no_event_writes_a_pass_a_title_or_the_home_path() {
        let dir = temp("private");
        let home = config::user_home();
        let (pass, title) = (PASS, TITLE);
        // A warning outside it is not an event at all.
        assert_eq!(gpui_record(log::Level::Warn, &format!("{home}/code/app/src/lib.rs"), pass), None);
        for (event, fields) in samples(&home) {
            write(event, fields, Some(&*dir));
        }
        let text = fs::read_to_string(dir.join(NAME)).unwrap();
        assert_eq!(text.lines().count(), 25, "{text}");
        // `at` is each line's time, never overwritten by a field.
        assert!(
            text.lines().all(|line| serde_json::from_str::<Value>(line).unwrap()["at"].as_str().unwrap().len() == 25)
        );
        assert!(!home.is_empty());
        let user = home.rsplit('/').next().unwrap_or_default();
        for forbidden in [pass, title, &home, user, "Caused by", "/Users/", "xxx"] {
            assert!(!text.contains(forbidden), "{forbidden:?} in\n{text}");
        }
        assert!(text.contains(r#""src":"ottospace/src/main.rs:12""#), "{text}");
        assert!(text.contains(r#""said":"could not create display link, code: -6661""#), "{text}");
        // Shown with `--nocapture`.
        println!("{text}");
    }

    #[test]
    fn the_log_is_never_written_through_a_hard_link() {
        let dir = temp("hard");
        fs::create_dir_all(&dir).unwrap();
        let elsewhere = dir.join("elsewhere");
        fs::write(&elsewhere, "").unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o644)).unwrap();
        fs::hard_link(&elsewhere, dir.join(NAME)).unwrap();
        assert!(append(&dir, "one").is_err());
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "");
        assert_eq!(fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[test]
    fn the_log_is_never_written_into_a_pipe() {
        let dir = temp("pipe");
        fs::create_dir_all(&dir).unwrap();
        let path = std::ffi::CString::new(dir.join(NAME).to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        // Returns at once, with no reader at the other end.
        assert!(append(&dir, "one").is_err());
    }

    #[test]
    fn the_log_is_never_written_through_a_link() {
        let dir = temp("link");
        fs::create_dir_all(&dir).unwrap();
        let elsewhere = dir.join("elsewhere");
        fs::write(&elsewhere, "").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(NAME)).unwrap();
        assert!(append(&dir, "one").is_err());
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "");
    }
}
