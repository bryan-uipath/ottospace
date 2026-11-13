//! Tests only: a file or folder in the system temp dir that goes when the test ends, pass or fail.

use std::path::{Path, PathBuf};

/// `$TMPDIR/<name>`, emptied now and removed on drop. Use it as a `&Path`.
pub struct Scratch(PathBuf);

impl Scratch {
    /// An empty folder.
    pub fn dir(name: &str) -> Self {
        let scratch = Self::path(name);
        std::fs::create_dir_all(&scratch.0).unwrap();
        scratch
    }

    /// A path with nothing at it yet: for a file.
    pub fn path(name: &str) -> Self {
        let scratch = Self(std::env::temp_dir().join(name));
        scratch.remove();
        scratch
    }

    fn remove(&self) {
        let _ = std::fs::remove_dir_all(&self.0);
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.remove();
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}
