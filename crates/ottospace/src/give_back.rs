//! Hosted windows go back on every way out. `<home>/hosted.json` records each hosted window's
//! original frame. A quit (⌘Q, the close button, `ctl quit`, a TERM, INT or HUP signal) puts
//! them back; a guardian process does when ottospace dies without a quit (a crash, `kill -9`);
//! the next start does if neither could. `ottospace rescue` brings back any stranded window.

use std::ffi::{c_char, c_int};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::time::Duration;

use ottospace_core::restore::{self, Entry, Frame};
use ottospace_mac::recovery::{self, Screens};
use ottospace_mac::{ax, process};

use crate::config;

const NAME: &str = "hosted.json";

/// The user's folder; `None` in a test run that named none, which keeps no record.
fn home() -> Option<&'static Path> {
    config::get().home.as_deref()
}

fn read() -> Vec<Entry> {
    let text = home().and_then(|home| fs::read_to_string(home.join(NAME)).ok());
    text.map(|text| restore::parse(&text)).unwrap_or_default()
}

/// Whole or not at all (temp file and rename), 0600.
fn write(entries: &[Entry]) {
    if let Some(home) = home() {
        let _ = config::write_private(home, NAME, restore::to_text(entries).as_bytes());
    }
}

fn me() -> u32 {
    std::process::id()
}

/// Records a window just hosted, and starts the guardian if none watches this run.
pub fn remember(pid: i32, window: u32, frame: Frame) {
    if home().is_none() {
        return;
    }
    let mut entries = read();
    entries.retain(|e| !(e.owner == me() && e.window == window));
    entries.push(Entry { owner: me(), pid, window, frame });
    write(&entries);
    start_guard();
}

pub fn forget(window: u32) {
    if home().is_none() {
        return;
    }
    let mut entries = read();
    let before = entries.len();
    entries.retain(|e| !(e.owner == me() && e.window == window));
    if entries.len() != before {
        write(&entries);
    }
}

/// True while the ottospace that wrote an entry runs (this one, or a live process named ottospace).
fn running(owner: u32) -> bool {
    let named = |pid: i32| process::name(pid).is_some_and(|name| name.starts_with("ottospace"));
    owner == me() || i32::try_from(owner).is_ok_and(|pid| process::alive(pid) && named(pid))
}

/// Windows a running ottospace hosts: never moved here.
fn hosted_elsewhere(entries: &[Entry]) -> Vec<(i32, u32)> {
    entries.iter().filter(|e| running(e.owner)).map(|e| (e.pid, e.window)).collect()
}

/// Puts each entry's window back, verified. Returns the entries to keep (a window found and not
/// yet where the user can reach it) and how many windows it moved. Only `only`'s, if named.
fn give_back(entries: Vec<Entry>, only: Option<i32>, screens: &Screens, skip: &[(i32, u32)]) -> (Vec<Entry>, usize) {
    let (mut kept, mut moved) = (Vec::new(), 0);
    for e in entries {
        if only.is_some_and(|only| only != e.pid) || e.pid == me() as i32 {
            kept.push(e);
            continue;
        }
        // Gone with its app.
        if !process::alive(e.pid) {
            continue;
        }
        // Only a standard window of that pid with that id: the file's numbers are untrusted.
        let back = match ax::Window::find(e.pid, e.window) {
            Some(window) => {
                let now = window.frame().map(recovery::to_frame);
                // Put back at the quit, or moved by the user since: left alone.
                if now.is_some_and(|f| restore::reachable(f, &screens.visible)) {
                    true
                } else {
                    let back = recovery::put_back(&window, e.frame, screens).is_some();
                    moved += usize::from(back);
                    back
                }
            }
            // The app replaced its window (a new id): its windows left parked come back instead.
            None => {
                let stranded = recovery::stranded_of(e.pid, screens);
                let stranded: Vec<_> = stranded
                    .into_iter()
                    .filter(|(w, _)| w.id().is_some_and(|id| !skip.contains(&(e.pid, id))))
                    .collect();
                let back = stranded.iter().filter(|(w, _)| recovery::put_back(w, e.frame, screens).is_some()).count();
                moved += back;
                back > 0 && back == stranded.len()
            }
        };
        if !back {
            kept.push(e);
        }
    }
    (kept, moved)
}

/// Where the record was before it moved to the user's folder: read once, then removed.
fn old_record() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let test_run = std::env::var_os("OTTOSPACE_E2E").is_some();
    (!test_run).then(|| PathBuf::from(home).join("Library/Application Support/ottospace/hosted.txt"))
}

/// At start: puts back the windows an ottospace that is no longer running left hosted. Returns
/// how many it moved. Entries whose window could not be verified back stay for the next try.
pub fn leftovers(only: Option<i32>) -> usize {
    if home().is_none() {
        return 0;
    }
    let mut entries = read();
    let old = old_record();
    let old_text = old.as_ref().and_then(|path| fs::read_to_string(path).ok());
    entries.extend(old_text.as_deref().map(restore::parse_old).unwrap_or_default());
    let (live, dead): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| running(e.owner));
    if dead.is_empty() && old_text.is_none() {
        return 0;
    }
    let (kept, moved) = give_back(dead, only, &recovery::screens(), &hosted_elsewhere(&live));
    write(&[live, kept].concat());
    if let Some(path) = old.filter(|_| old_text.is_some()) {
        let _ = fs::remove_file(path);
    }
    moved
}

static GUARD: Mutex<Option<Child>> = Mutex::new(None);

/// Starts `ottospace guard <pid>` unless one is running. Not in a test run, unless it sets
/// `OTTOSPACE_GUARD`.
fn start_guard() {
    if std::env::var_os("OTTOSPACE_E2E").is_some() && std::env::var_os("OTTOSPACE_GUARD").is_none() {
        return;
    }
    let Ok(mut guard) = GUARD.lock() else { return };
    if guard.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None))) {
        return;
    }
    let Ok(program) = std::env::current_exe() else { return };
    let mut command = Command::new(program);
    command.arg("guard").arg(me().to_string()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    unsafe extern "C" {
        fn setsid() -> c_int;
    }
    // Its own session: the ^C or hangup from the terminal that ends ottospace must not end it.
    // SAFETY: `setsid` is async-signal-safe, as `pre_exec` requires.
    unsafe {
        command.pre_exec(|| {
            setsid();
            Ok(())
        })
    };
    *guard = command.spawn().map_err(|err| eprintln!("guardian off: {err}")).ok();
}

/// `ottospace guard <pid>`: waits for ottospace `<pid>`, its parent, to end, then puts back the
/// windows it left hosted. Ends early once the record holds none of that run's windows.
pub fn guard(parent: Option<&String>) -> ! {
    let Some(parent) = parent.and_then(|p| p.parse::<u32>().ok()) else { std::process::exit(2) };
    while std::os::unix::process::parent_id() == parent {
        if !read().iter().any(|e| e.owner == parent) {
            std::process::exit(0);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // A moment for an app that moves its window again once ottospace lets go: checked after it.
    std::thread::sleep(Duration::from_secs(1));
    // Never a permission prompt from a process the user did not start: the next start retries.
    if !ax::is_trusted(false) {
        std::process::exit(1);
    }
    let (theirs, rest): (Vec<Entry>, Vec<Entry>) = read().into_iter().partition(|e| e.owner == parent);
    let (kept, _) = give_back(theirs, host_only(), &recovery::screens(), &hosted_elsewhere(&rest));
    write(&[rest, kept].concat());
    std::process::exit(0);
}

fn host_only() -> Option<i32> {
    std::env::var("OTTOSPACE_HOST_ONLY").ok().and_then(|pid| pid.parse().ok())
}

/// Moves every stranded window back on screen: first those a run that ended left hosted, to
/// their own frames, then any other off every screen or in the parking spot, to the main
/// screen. Never one a running ottospace hosts (`hosted`: this one's). Returns how many.
pub fn rescue(only: Option<i32>, hosted: &[u32]) -> usize {
    let back = leftovers(only);
    let elsewhere = hosted_elsewhere(&read());
    back + recovery::rescue(only, |pid, id| hosted.contains(&id) || elsewhere.contains(&(pid, id)))
}

/// `ottospace rescue`, from the user's terminal.
pub fn rescue_command() -> ! {
    // A test run moves no window but its canary's.
    if std::env::var_os("OTTOSPACE_E2E").is_some() && host_only().is_none() {
        eprintln!("rescue: a test run names its app with OTTOSPACE_HOST_ONLY");
        std::process::exit(2);
    }
    if !ax::is_trusted(false) {
        eprintln!("grant Accessibility access to your terminal, then rerun");
        std::process::exit(1);
    }
    let moved = rescue(host_only(), &[]);
    println!("moved {moved} window{} back on screen", if moved == 1 { "" } else { "s" });
    std::process::exit(0);
}

static SIGNALS: AtomicUsize = AtomicUsize::new(0);
static QUITTING: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(signal: c_int) {
    unsafe extern "C" {
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn dup2(from: c_int, to: c_int) -> c_int;
        fn _exit(status: c_int) -> !;
    }
    // Hung up: the terminal is gone, and `eprintln!` to it would panic. Writes go nowhere instead.
    if signal == SIGHUP {
        let fd = unsafe { open(c"/dev/null".as_ptr(), 1) }; // O_WRONLY
        if fd >= 0 {
            unsafe { dup2(fd, 1) };
            unsafe { dup2(fd, 2) };
        }
    }
    // A second signal while the quit hangs ends the app now; the guardian puts the windows back.
    if SIGNALS.fetch_add(1, SeqCst) > 0 {
        unsafe { _exit(128 + signal) };
    }
}

const SIGHUP: c_int = 1;

/// TERM, INT and HUP quit as ⌘Q does, so hosted windows go back. The handler only counts (only
/// async-signal-safe calls there); the app's tick sees it and quits on the main thread.
pub fn on_signals() {
    unsafe extern "C" {
        fn signal(signal: c_int, handler: extern "C" fn(c_int)) -> usize;
    }
    for sig in [SIGHUP, 2, 15] {
        unsafe { signal(sig, on_signal) };
    }
}

/// True once, after the first TERM, INT or HUP.
pub fn signalled() -> bool {
    SIGNALS.load(SeqCst) > 0 && !QUITTING.swap(true, SeqCst)
}
