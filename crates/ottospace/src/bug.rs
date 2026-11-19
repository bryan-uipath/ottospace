//! The bug reporter, the shell half: what `developer.ottospace_repo` names on disk. Whether the
//! button shows, the marks and the brief are the core's (`ottospace_core::bug`). See
//! `docs/ai/bug-reporter.md`.

use ottospace_core::bug::RepoFacts;
use ottospace_core::settings::{Settings, expand_home};

use crate::config;

/// What the setting names: its folder (`~` expanded), whether it is there, and whether it is a
/// git checkout (`.git`, a folder or a worktree's file). `None`: not set.
pub(crate) fn repo_facts(settings: &Settings) -> Option<RepoFacts> {
    let named = settings.developer.ottospace_repo.as_deref()?;
    let path = expand_home(named, &config::user_home());
    let folder = std::path::Path::new(&path);
    let (exists, git) = (folder.is_dir(), folder.join(".git").exists());
    Some(RepoFacts { path, exists, git })
}
