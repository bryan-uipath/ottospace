//! `git worktree add` for the Repos section's +: the one git the app runs. A fixed argv, the branch
//! checked by the core (`repos::check_branch`), the folder checked here to be new and inside
//! `worktrees_dir`. Runs off the main thread; nothing is ever deleted but a folder made here, empty.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{fs, io};

use ottospace_core::repos::git_said;

/// How long git may take: a checkout of a large repository.
const TIMEOUT: Duration = Duration::from_secs(120);
/// The most of git's output read.
const MAX_OUTPUT: u64 = 64 * 1024;

/// What a try came to: the worktree's folder, resolved, or why there is none (for the status bar).
pub struct Made {
    pub repo: String,
    pub result: Result<String, String>,
}

/// Starts the worktree of the repository at `repo` on `branch` at `path`, inside `root`. Its end
/// comes on `done`.
pub fn start(repo: String, branch: String, path: String, root: String, done: mpsc::Sender<Made>) {
    std::thread::spawn(move || {
        let made = add(Path::new(&repo), &branch, Path::new(&path), Path::new(&root));
        let result = made.map(|target| target.to_string_lossy().into_owned());
        let _ = done.send(Made { repo, result });
    });
}

/// Makes the worktree: an existing branch is checked out, a new one made from the repository's HEAD.
fn add(repo: &Path, branch: &str, path: &Path, root: &Path) -> Result<PathBuf, String> {
    let (target, made) = target(path, root)?;
    let local = format!("refs/heads/{branch}");
    let exists = git(repo, &["rev-parse", "--verify", "--quiet", &local])?.0;
    let target_arg = target.as_os_str();
    let args: Vec<&OsStr> = match exists {
        true => ["worktree", "add", "--"].map(OsStr::new).into_iter().chain([target_arg, OsStr::new(branch)]).collect(),
        false => ["worktree", "add", "-b", branch, "--"]
            .map(OsStr::new)
            .into_iter()
            .chain([target_arg, OsStr::new("HEAD")])
            .collect(),
    };
    let (ok, stderr) = git(repo, &args)?;
    if ok {
        return Ok(target);
    }
    let said = git_said(&stderr);
    // What git made is left as it is, and said; an empty folder of ours goes.
    if fs::symlink_metadata(&target).is_ok() {
        return Err(format!("{said} (it left {})", target.display()));
    }
    if let Some(folder) = made {
        let _ = fs::remove_dir(folder);
    }
    Err(said)
}

/// The folder to make, resolved: `path` is `<root>/<repo>/<branch>`, two plain names under
/// `root`. `<repo>` may be there already as a real folder (not a link); the worktree's folder
/// must not be there at all. Also the `<repo>` folder if it is made here.
fn target(path: &Path, root: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
    let outside = || format!("{} is not a folder inside {}", path.display(), root.display());
    let rest = path.strip_prefix(root).map_err(|_| outside())?;
    let parts: Vec<&OsStr> = rest
        .components()
        .map(|part| match part {
            Component::Normal(name) => Ok(name),
            _ => Err(outside()),
        })
        .collect::<Result<_, _>>()?;
    let [folder, name] = parts[..] else { return Err(outside()) };
    let failed = |what: &Path, err: io::Error| format!("{}: {err}", what.display());
    fs::create_dir_all(root).map_err(|err| failed(root, err))?;
    let root = root.canonicalize().map_err(|err| failed(root, err))?;
    let folder = root.join(folder);
    let made = match fs::symlink_metadata(&folder) {
        Ok(meta) if meta.is_dir() => None,
        Ok(_) => return Err(format!("{} is a link or a file, not a folder", folder.display())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&folder).map_err(|err| failed(&folder, err))?;
            Some(folder.clone())
        }
        Err(err) => return Err(failed(&folder, err)),
    };
    let target = folder.join(name);
    match fs::symlink_metadata(&target) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok((target, made)),
        Ok(_) => Err(format!("{} is there already", target.display())),
        Err(err) => Err(failed(&target, err)),
    }
}

/// Runs `git -C <repo> <args>`: no prompt, no environment that points it at another repository,
/// nothing on its standard input. Whether it succeeded, and its standard error.
fn git<S: AsRef<OsStr>>(repo: &Path, args: &[S]) -> Result<(bool, String), String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY"] {
        command.env_remove(name);
    }
    command.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|err| format!("git: {err}"))?;
    // Read on a thread: a full pipe would hold git up while this waits.
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stderr) = stderr {
            let _ = stderr.take(MAX_OUTPUT).read_to_string(&mut text);
        }
        text
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git took longer than {} seconds", TIMEOUT.as_secs()));
            }
            Err(err) => return Err(format!("git: {err}")),
        }
    };
    Ok((status.success(), reader.join().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    /// git with none of the user's settings (a signing key, hooks): the test's own repository.
    fn git_in(dir: &Path, args: &[&str]) {
        let mut git = Command::new("git");
        git.arg("-C").arg(dir).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(git.stdin(Stdio::null()).status().unwrap().success(), "git {args:?}");
    }

    /// A repository with one commit on `main`, made with the real git: no network.
    fn repository(at: &Path) {
        fs::create_dir_all(at).unwrap();
        git_in(at, &["init", "-q", "-b", "main"]);
        git_in(at, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "one"]);
    }

    #[test]
    fn the_folder_is_new_and_inside_the_worktrees_folder_with_no_link_on_the_way() {
        let scratch = Scratch::dir(&format!("ottospace-worktree-target-{}", std::process::id()));
        let root = scratch.join("worktrees");
        let real = scratch.canonicalize().unwrap().join("worktrees");
        // Made: the repository's folder too, and said so; then there is one to go into.
        let made = target(&root.join("app/fix-zoom"), &root).unwrap();
        assert_eq!(made, (real.join("app/fix-zoom"), Some(real.join("app"))));
        assert_eq!(target(&root.join("app/two"), &root).unwrap().1, None);
        // Anything but two plain names under the folder.
        for path in ["app", "app/a/b", "app/../x", "../app/x"] {
            assert!(target(&root.join(path), &root).is_err(), "{path}");
        }
        assert!(target(&scratch.join("elsewhere/app/x"), &root).is_err());
        // Not where something is already.
        fs::create_dir_all(real.join("app/taken")).unwrap();
        assert!(target(&root.join("app/taken"), &root).unwrap_err().ends_with("is there already"));
        // A link as the repository's folder, or as the worktree's, is not followed out of it.
        let away = scratch.join("away");
        fs::create_dir_all(&away).unwrap();
        std::os::unix::fs::symlink(&away, real.join("linked")).unwrap();
        assert!(target(&root.join("linked/x"), &root).unwrap_err().contains("is a link"));
        std::os::unix::fs::symlink(&away, real.join("app/dangling")).unwrap();
        assert!(target(&root.join("app/dangling"), &root).unwrap_err().ends_with("is there already"));
        assert_eq!(fs::read_dir(&away).unwrap().count(), 0, "nothing was made through a link");
    }

    #[test]
    fn a_new_branch_is_made_from_head_and_an_existing_one_checked_out() {
        let scratch = Scratch::dir(&format!("ottospace-worktree-git-{}", std::process::id()));
        let repo = scratch.join("app");
        repository(&repo);
        git_in(&repo, &["branch", "feat/map"]);
        let root = scratch.join("worktrees");
        let head = |dir: &Path| fs::read_to_string(dir.join(".git")).is_ok();
        add(&repo, "fix/zoom", &root.join("app/fix-zoom"), &root).unwrap();
        add(&repo, "feat/map", &root.join("app/feat-map"), &root).unwrap();
        assert!(head(&root.join("app/fix-zoom")) && head(&root.join("app/feat-map")));
        // Checked out already: git's own words, and nothing is left behind.
        let err = add(&repo, "feat/map", &root.join("app/again"), &root).unwrap_err();
        assert!(err.starts_with("git: ") && err.contains("feat/map"), "{err}");
        assert!(!root.join("app/again").exists());
        // Not a repository: said, and the empty folder made for it goes.
        let err = add(&scratch.join("none"), "x", &root.join("none/x"), &root).unwrap_err();
        assert!(err.starts_with("git"), "{err}");
        assert!(!root.join("none").exists());
    }
}
