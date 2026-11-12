//! Remembers hosted windows' original frames on disk, so a crash doesn't strand them parked.
//! Format: one `pid window-id x y w h` line per hosted window.

use std::path::PathBuf;

use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use crate::ax;

fn path() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/ottospace/hosted.txt")
}

fn read() -> Vec<(i32, u32, CGRect)> {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let v: Vec<f64> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            let [pid, id, x, y, w, h] = v[..] else { return None };
            Some((pid as i32, id as u32, CGRect { origin: CGPoint::new(x, y), size: CGSize::new(w, h) }))
        })
        .collect()
}

fn write(entries: &[(i32, u32, CGRect)]) {
    let text: String = entries
        .iter()
        .map(|(pid, id, r)| format!("{pid} {id} {} {} {} {}\n", r.origin.x, r.origin.y, r.size.width, r.size.height))
        .collect();
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

/// Set in test runs: they share this file with the user's real session and must not write it.
static DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Stops `remember` and `forget` from touching the file, for the life of the process.
pub fn disable() {
    DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn disabled() -> bool {
    DISABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn remember(pid: i32, id: u32, original: CGRect) {
    if disabled() {
        return;
    }
    let mut entries = read();
    entries.retain(|(_, i, _)| *i != id);
    entries.push((pid, id, original));
    write(&entries);
}

pub fn forget(id: u32) {
    if disabled() {
        return;
    }
    let mut entries = read();
    entries.retain(|(_, i, _)| *i != id);
    write(&entries);
}

/// Puts back any windows a previous run left parked, then clears the list.
pub fn restore_leftovers() {
    for (pid, id, frame) in read() {
        if let Some(w) = ax::Window::standard_of(pid).into_iter().find(|w| w.id() == Some(id)) {
            let _ = w.set_size(frame.size);
            let _ = w.set_position(frame.origin);
        }
    }
    write(&[]);
}
