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
    /// `load` runs only when the file's time or length changed since the last call.
    pub fn get(&self, path: &Path, load: impl FnOnce() -> Option<T>) -> Option<(T, u64)> {
        let meta = path.metadata().ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, len, value)) = entries.get(path)
            && (*at, *len) == (modified, meta.len())
        {
            return value.clone().map(|v| (v, modified));
        }
        let value = load();
        entries.insert(path.to_path_buf(), (modified, meta.len(), value.clone()));
        value.map(|v| (v, modified))
    }

    /// Forgets every file not in `seen`, e.g. deleted sessions.
    pub fn keep(&self, seen: &HashSet<PathBuf>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).retain(|path, _| seen.contains(path));
    }
}
