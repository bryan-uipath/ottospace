//! A ring of the app's recent events, for tests and bug reports: `{"cmd":"trace"}` reads it.
//! Names, counts, durations and ids only: never terminal text, titles, typed text or paths.
//! Times are milliseconds since the app started, from a monotonic clock.
//! Under `OTTOSPACE_E2E` or `OTTOSPACE_TRACE=1` it is also written to a file on a panic or on
//! SIGUSR1 (a test's timeout): beside the socket in a test run, else `<home>/trace.jsonl`.

use std::collections::VecDeque;
use std::fmt::{self, Debug, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Events kept: about a minute of a busy test.
const CAPACITY: usize = 2_000;

struct Ring {
    start: Instant,
    events: VecDeque<String>,
}

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Ring { start: Instant::now(), events: VecDeque::with_capacity(CAPACITY) }))
}

/// Adds `{"ms":..,"ev":event, ..fields}`. `fields`: names, numbers and ids only.
pub fn record(event: &str, fields: Value) {
    let Ok(mut ring) = ring().lock() else { return };
    let mut line = json!({ "ms": ring.start.elapsed().as_millis() as u64, "ev": event });
    if let (Some(line), Value::Object(fields)) = (line.as_object_mut(), fields) {
        line.extend(fields);
    }
    if ring.events.len() == CAPACITY {
        ring.events.pop_front();
    }
    ring.events.push_back(line.to_string());
}

/// The ring as JSONL, oldest first.
pub fn jsonl() -> String {
    ring().lock().map(|ring| ring.events.iter().map(|line| format!("{line}\n")).collect()).unwrap_or_default()
}

/// An enum value's variant name, never its payload: `Split { .. }` gives `Split`. The payload is
/// not even formatted: the writer refuses the first character after the name.
pub fn name(value: &dyn Debug) -> String {
    struct Name(String);
    impl fmt::Write for Name {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for c in s.chars() {
                if !(c.is_ascii_alphanumeric() || c == '_') {
                    return Err(fmt::Error);
                }
                self.0.push(c);
            }
            Ok(())
        }
    }
    let mut name = Name(String::new());
    let _ = write!(name, "{value:?}");
    name.0
}

/// Where the ring is written: in a test run beside its socket, where the harness looks; else
/// `<home>/trace.jsonl`.
fn file() -> PathBuf {
    if std::env::var_os("OTTOSPACE_E2E").is_some() {
        return crate::control::path().with_extension("trace.jsonl");
    }
    match std::env::var_os("OTTOSPACE_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home).join("trace.jsonl"),
        None => PathBuf::from(crate::config::user_home()).join(".ottospace/trace.jsonl"),
    }
}

/// Writes the ring to its file, with `why` as the last event.
fn dump(why: &str) {
    record("dump", json!({ "why": why }));
    let _ = std::fs::write(file(), jsonl());
}

static SIGNALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    // Only an atomic store: a signal handler may not lock or allocate.
    SIGNALLED.store(true, Relaxed);
}

/// Test runs, or `OTTOSPACE_TRACE=1`: write the ring on a panic and on SIGUSR1. A thread does the
/// writing, so it works while the UI thread is stuck.
pub fn arm() {
    let traced = std::env::var("OTTOSPACE_TRACE").is_ok_and(|v| v == "1");
    if std::env::var_os("OTTOSPACE_E2E").is_none() && !traced {
        return;
    }
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dump("panic");
        before(info);
    }));
    unsafe { libc::signal(libc::SIGUSR1, on_signal as *const () as libc::sighandler_t) };
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if SIGNALLED.swap(false, Relaxed) {
                dump("signal");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_never_carries_its_payload() {
        #[derive(Debug)]
        #[allow(dead_code)]
        enum Sample {
            Quit,
            Type { text: String },
            Opened(u32),
        }
        assert_eq!(name(&Sample::Quit), "Quit");
        assert_eq!(name(&Sample::Type { text: "secret".into() }), "Type");
        assert_eq!(name(&Sample::Opened(3)), "Opened");
    }

    #[test]
    fn the_ring_keeps_the_newest() {
        for n in 0..CAPACITY + 5 {
            record("t", json!({ "n": n }));
        }
        let text = jsonl();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() <= CAPACITY);
        assert!(lines.last().is_some_and(|l| l.contains(&format!("\"n\":{}", CAPACITY + 4))), "{:?}", lines.last());
    }
}
