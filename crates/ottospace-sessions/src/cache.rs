//! What listing learned from each session file, kept until the file changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Per file: (modification time, length, what was read from it).
type Entries<T> = HashMap<PathBuf, (u64, u64, Option<T>)>;

pub struct Cache<T>(Mutex<Entries<T>>);

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

impl<T: Clone> Cache<T> {
    /// What `load` read from `path`, with the file's modification time in Unix milliseconds.
    /// `load` runs only when the file's time or length changed since the last call, and never
    /// with the lock held. `None` for anything but a regular file (a FIFO would block a reader).
    pub fn get(&self, path: &Path, load: impl FnOnce() -> Option<T>) -> Option<(T, u64)> {
        let meta = path.metadata().ok().filter(|m| m.is_file())?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
        let stamp = (modified, meta.len());
        let kept = self.entries(|entries| entries.get(path).filter(|e| (e.0, e.1) == stamp).map(|e| e.2.clone()));
        let value = match kept {
            Some(value) => value,
            None => {
                let value = load();
                self.entries(|entries| entries.insert(path.to_path_buf(), (stamp.0, stamp.1, value.clone())));
                value
            }
        };
        value.map(|v| (v, modified))
    }

    /// Forgets every file not in `seen`, e.g. deleted sessions.
    pub fn keep(&self, seen: &HashSet<PathBuf>) {
        self.entries(|entries| entries.retain(|path, _| seen.contains(path)));
    }

    fn entries<R>(&self, with: impl FnOnce(&mut Entries<T>) -> R) -> R {
        with(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
