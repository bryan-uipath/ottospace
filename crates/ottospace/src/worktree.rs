//! `git worktree add` for the Repos section's +: the one git the app runs. A fixed argv, the branch
//! checked by the core (`repos::check_branch`), the folder checked here to be new and inside
//! `worktrees_dir`. Runs off the main thread; nothing is ever deleted but a folder made here, empty.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
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

/// Makes the worktree: a local branch is checked out; one only a remote has (`origin/feat`, of one
/// remote) is made to track it; any other name is a new branch from the repository's HEAD.
fn add(repo: &Path, branch: &str, path: &Path, root: &Path) -> Result<PathBuf, String> {
    let (target, made) = target(path, root)?;
    let local = format!("refs/heads/{branch}");
    let exists = git(repo, &["rev-parse", "--verify", "--quiet", &local])?.0;
    let remote = if exists { None } else { remote_branch(repo, branch) };
    let target = target.as_os_str();
    let args: Vec<&OsStr> = match (exists, &remote) {
        (true, _) => {
            ["worktree", "add", "--"].map(OsStr::new).into_iter().chain([target, OsStr::new(branch)]).collect()
        }
        (false, Some(remote)) => ["worktree", "add", "--track", "-b", branch, "--"]
            .map(OsStr::new)
            .into_iter()
            .chain([target, OsStr::new(remote)])
            .collect(),
        (false, None) => ["worktree", "add", "-b", branch, "--"]
            .map(OsStr::new)
            .into_iter()
            .chain([target, OsStr::new("HEAD")])
            .collect(),
    };
    let target = PathBuf::from(target);
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

/// The one remote's branch of that name (`origin/feat/map`), if exactly one remote has it.
fn remote_branch(repo: &Path, branch: &str) -> Option<String> {
    let pattern = format!("refs/remotes/*/{branch}");
    let mut command = crate::child::git(repo);
    command.args(["for-each-ref", "--format=%(refname)", &pattern]);
    let said = crate::child::run(command, TIMEOUT, MAX_OUTPUT).ok().filter(|said| said.ok)?;
    let refs: Vec<&str> = said.out.lines().filter_map(|line| line.strip_prefix("refs/remotes/")).collect();
    match refs[..] {
        [one] => Some(one.to_string()),
        _ => None,
    }
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
        // `worktrees_dir` inside the repositories' folder: the worktree would be in a checkout.
        Ok(meta) if meta.is_dir() && folder.join(".git").exists() => {
            return Err(format!("{} is a repository: set worktrees_dir to a folder of its own", folder.display()));
        }
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

/// Runs `git -C <repo> <args>`: no prompt, no environment that points it at another repository.
/// Whether it succeeded, and what it said.
fn git<S: AsRef<OsStr>>(repo: &Path, args: &[S]) -> Result<(bool, String), String> {
    let mut command = crate::child::git(repo);
    command.args(args);
    let said = crate::child::run(command, TIMEOUT, MAX_OUTPUT)?;
    // Its message, on standard error, else on standard output.
    Ok((said.ok, if said.err.trim().is_empty() { said.out } else { said.err }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;
    use std::process::{Command, Stdio};

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
        // A branch only the remote has: tracked, not made anew from HEAD.
        let clone = scratch.join("clone");
        git_in(&scratch, &["clone", "-q", repo.to_str().unwrap(), clone.to_str().unwrap()]);
        add(&clone, "feat/map", &root.join("clone/feat-map"), &root).unwrap();
        let mut tracks = Command::new("git");
        tracks.arg("-C").arg(root.join("clone/feat-map")).args(["rev-parse", "--abbrev-ref", "@{u}"]);
        assert_eq!(String::from_utf8_lossy(&tracks.output().unwrap().stdout).trim(), "origin/feat/map");
        // A worktrees folder that is a checkout itself is not put into.
        let err = add(&repo, "x", &scratch.join("app/x"), &scratch).unwrap_err();
        assert!(err.ends_with("set worktrees_dir to a folder of its own"), "{err}");
        // Not a repository: said, and the empty folder made for it goes.
        let err = add(&scratch.join("none"), "x", &root.join("none/x"), &root).unwrap_err();
        assert!(err.starts_with("git"), "{err}");
        assert!(!root.join("none").exists());
    }
}
