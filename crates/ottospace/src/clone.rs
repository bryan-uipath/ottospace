//! A repository the user chose, cloned with their gh (`Effect::Clone`) on a thread of its own:
//! a fixed argv, no shell, no prompt, no hook, into a folder this clone makes and only that one
//! removes. What the clone means and what is said of it are the core's (`ottospace_core::clone`).

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{Context, KeyDownEvent, Window, div, prelude::*, px, rgb};
use ottospace_core::clone::{Dialog, Failure, Provider, Remote};
use ottospace_core::keys::FilterKey;
use ottospace_core::{Action, Origin, theme};

use crate::{Workspace, dialog};

/// The longest a clone may run before it is stopped as a failure.
const TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How often gh's last line is passed on, at most.
const LINE_EVERY: Duration = Duration::from_millis(200);
/// How much of gh's error output is kept for the failure.
const MOST_SAID: usize = 4096;

/// What a clone says: its last line, or its end (the folder it made, or why not).
pub enum News {
    Line { id: u64, line: String },
    Done { id: u64, result: Result<String, Failure> },
}

/// What a provider runs to clone a repository: a second provider (GitLab, a plain URL) is a
/// second one of these, its variant in `Provider`.
trait Source {
    /// The argv that clones `remote` into `dest`, an empty folder this clone made.
    fn clone_argv(&self, gh: &Path, remote: &Remote, dest: &Path) -> Command;
}

struct GitHub;

impl Source for GitHub {
    /// `gh repo clone <owner>/<name> <dest> -- --progress --no-recurse-submodules`: both
    /// positionals checked (no `-` first, `dest` absolute); after `--` only git's flags.
    fn clone_argv(&self, gh: &Path, remote: &Remote, dest: &Path) -> Command {
        let mut command = crate::prs::command(gh);
        command.args(["repo", "clone"]).arg(remote.slug()).arg(dest);
        command.args(["--", "--progress", "--no-recurse-submodules"]);
        command
    }
}

fn source(provider: Provider) -> &'static dyn Source {
    match provider {
        Provider::GitHub => &GitHub,
    }
}

/// The clones on their way, each with its stop flag, and their news.
pub struct Cloner {
    news_in: Sender<News>,
    pub news: Receiver<News>,
    stops: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
}

impl Cloner {
    pub fn new() -> Self {
        let (news_in, news) = channel();
        Self { news_in, news, stops: Arc::default() }
    }

    /// Starts clone `id` of `remote` into `dest`, on a thread of its own.
    pub fn start(&self, id: u64, provider: Provider, remote: Remote, dest: String, make_dir: bool) {
        let stop = Arc::new(AtomicBool::new(false));
        if let Ok(mut stops) = self.stops.lock() {
            stops.insert(id, stop.clone());
        }
        let (news, stops) = (self.news_in.clone(), self.stops.clone());
        std::thread::spawn(move || {
            let line = |line: String| drop(news.send(News::Line { id, line }));
            let result = run(source(provider), &remote, Path::new(&dest), make_dir, &stop, line);
            if let Ok(mut stops) = stops.lock() {
                stops.remove(&id);
            }
            let _ = news.send(News::Done { id, result: result.map(|()| dest) });
            ottospace_mac::wake::post();
        });
    }

    /// The user's Esc: clone `id` stops, and its folder goes.
    pub fn cancel(&self, id: u64) {
        if let Some(stop) = self.stops.lock().ok().and_then(|stops| stops.get(&id).cloned()) {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

/// Which folder this clone made: its device and inode, so only that one is ever removed.
type Made = (u64, u64);

/// One clone: the checks, the folder, gh. Nothing is made before the checks pass, and what
/// this run made goes again if the clone does not end with a repository there.
fn run(
    source: &dyn Source,
    remote: &Remote,
    dest: &Path,
    make_dir: bool,
    stop: &AtomicBool,
    line: impl Fn(String),
) -> Result<(), Failure> {
    let dir = dest.parent().filter(|dir| dir.is_absolute()).ok_or(Failure::DirMissing)?;
    // The core checked the name; checked again at the door: one plain name inside `dir`.
    if dest.file_name().and_then(|name| name.to_str()) != Some(remote.name.as_str()) || !dest.is_absolute() {
        return Err(Failure::Said("the destination is not one name inside clone_dir".into()));
    }
    match std::fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(Failure::Said(format!("{} is not a folder", dir.display()))),
        Err(_) if make_dir => {
            std::fs::create_dir_all(dir).map_err(|err| Failure::Said(format!("{}: {err}", dir.display())))?
        }
        Err(_) => return Err(Failure::DirMissing),
    }
    // Anything there, a link too, even to nowhere: refused before any process runs.
    if std::fs::symlink_metadata(dest).is_ok() {
        return Err(Failure::Exists);
    }
    let gh = crate::prs::gh().map_err(|why| match why {
        crate::prs::NO_GH => Failure::NoGh,
        why => Failure::Said(why.into()),
    })?;
    // Made here, atomically: one made meanwhile by anything else is refused, never cloned into.
    let made = make(dest)?;
    let result = clone(source, &gh, remote, dest, stop, line);
    let result = match result {
        Ok(()) if dest.join(".git").is_dir() => return Ok(()),
        Ok(()) => Err(Failure::Said("gh ended with no repository there".into())),
        Err(failure) => Err(failure),
    };
    remove_made(dest, made);
    result
}

/// Makes `dest`, empty; `Exists` if anything is there.
fn make(dest: &Path) -> Result<Made, Failure> {
    match std::fs::create_dir(dest) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Err(Failure::Exists),
        Err(err) => return Err(Failure::Said(format!("{}: {err}", dest.display()))),
    }
    let meta = std::fs::symlink_metadata(dest).map_err(|err| Failure::Said(err.to_string()))?;
    Ok((meta.dev(), meta.ino()))
}

/// Removes `dest` only if it is still the folder this clone made: not a link, the same inode.
fn remove_made(dest: &Path, made: Made) {
    let same = std::fs::symlink_metadata(dest).is_ok_and(|m| m.is_dir() && (m.dev(), m.ino()) == made);
    if same {
        let _ = std::fs::remove_dir_all(dest);
    }
}

/// gh, run to its end in a process group of its own (so a stop ends git too), its error output
/// read line by line: `\r` ends git's progress lines.
fn clone(
    source: &dyn Source,
    gh: &Path,
    remote: &Remote,
    dest: &Path,
    stop: &AtomicBool,
    line: impl Fn(String),
) -> Result<(), Failure> {
    let mut command = source.clone_argv(gh, remote, dest);
    // No prompt, no other repository, no hook of the user's or the clone's: for this run only,
    // never written into the new repository's config.
    command.env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_COUNT", "1");
    command.env("GIT_CONFIG_KEY_0", "core.hooksPath").env("GIT_CONFIG_VALUE_0", "/dev/null");
    for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY"] {
        command.env_remove(name);
    }
    command.current_dir(dest).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).process_group(0);
    let mut child = command.spawn().map_err(|err| Failure::Said(format!("gh: {err}")))?;
    let said = Arc::new(Mutex::new(String::new()));
    let reader = child.stderr.take().map(|mut pipe| {
        let said = said.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n @ 1..) = pipe.read(&mut buf) {
                if let Ok(mut said) = said.lock() {
                    said.push_str(&String::from_utf8_lossy(&buf[..n]));
                    let cut = said.len().saturating_sub(MOST_SAID);
                    let cut = (cut..said.len()).find(|at| said.is_char_boundary(*at)).unwrap_or(said.len());
                    said.drain(..cut);
                }
            }
        })
    });
    let (start, mut shown) = (Instant::now(), Instant::now());
    let pid = child.id() as i32;
    let status = loop {
        if stop.load(Ordering::Relaxed) || start.elapsed() > TIMEOUT {
            // The group: gh and the git it started.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = child.wait();
            let _ = reader.map(std::thread::JoinHandle::join);
            return Err(if stop.load(Ordering::Relaxed) {
                Failure::Cancelled
            } else {
                Failure::Said("the clone took longer than 30 minutes".into())
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => return Err(Failure::Said(format!("gh: {err}"))),
        }
        if shown.elapsed() >= LINE_EVERY {
            shown = Instant::now();
            let last = said.lock().ok().and_then(|said| last_line(&said));
            if let Some(last) = last {
                line(last);
                ottospace_mac::wake::post();
            }
        }
    };
    let _ = reader.map(std::thread::JoinHandle::join);
    let said = said.lock().map(|said| said.clone()).unwrap_or_default();
    if status.success() { Ok(()) } else { Err(Failure::Said(said)) }
}

/// The last line git or gh wrote, by `\r` or `\n`.
fn last_line(said: &str) -> Option<String> {
    said.split(['\r', '\n']).map(str::trim).rfind(|line| !line.is_empty()).map(str::to_string)
}

impl Workspace {
    /// News of the clones, to the core as the shell's: a made one is read and listed as Local's
    /// add is (`change_repos`).
    pub(crate) fn take_clones(&mut self, window: &Window, cx: &mut Context<Self>) {
        let news: Vec<News> = self.cloner.news.try_iter().collect();
        for news in news {
            match news {
                News::Line { id, line } => {
                    drop(self.dispatch(Action::CloneProgress { id, line }, Origin::System, window, cx))
                }
                News::Done { id, result: Ok(dest) } => {
                    let found = crate::repos::found_in(Path::new(&dest));
                    self.change_repos(Action::CloneDone { id, result: Ok(found) }, Origin::System, window, cx);
                }
                News::Done { id, result: Err(failure) } => {
                    drop(self.dispatch(Action::CloneDone { id, result: Err(failure) }, Origin::System, window, cx))
                }
            }
            cx.notify();
        }
    }

    /// A key while the dialog is open: Esc closes it (and stops a clone on its way), ↵ clones the
    /// row the keys are on, ↑↓ move; the rest types.
    pub(crate) fn clone_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let action = |key| match key {
            FilterKey::Close => Action::CloneClose { cancel: true },
            FilterKey::Choose => Action::CloneChoose { index: None },
            FilterKey::Move(delta) => Action::CloneMove { delta },
        };
        self.filtered_list_key(event, action, window, cx);
    }

    /// The dialog: the line, the repository it names, where it goes in full, what it says, and
    /// gh's last line while it clones. A click outside hides it; a clone on its way goes on.
    pub(crate) fn render_clone(&self, dialog: &Dialog, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        for (i, remote) in dialog.rows().into_iter().enumerate() {
            let row = dialog::filter_row(("clone", i), i == dialog.selected)
                .child(crate::cut_text(remote.slug()).flex_1())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::CloneChoose { index: Some(i) }, this.input, window, cx);
                }));
            rows.push(row.into_any_element());
        }
        let header = div()
            .h(px(40.))
            .child(div().flex_none().text_xs().text_color(rgb(theme::COMMAND)).child("Clone"))
            .child(self.render_field("owner/name, or a GitHub link", cx));
        let panel = dialog::floating(560.).max_w_full().max_h(px(440.));
        let panel = self.list_panel(panel, header, "clone", rows, "Paste owner/name, or a GitHub link");
        let line = |text: String, color: u32| div().px_3().py_1().text_xs().text_color(rgb(color)).child(text);
        let job = self.model.cloning.as_ref();
        let dest = match (self.model.clone_destination(), self.model.repo_dirs.clone.is_some()) {
            (Some(dest), _) => Some(format!("Clones into {dest}")),
            (None, false) => Some(ottospace_core::clone::NO_DIR.to_string()),
            _ => None,
        };
        let busy = job.map(|job| {
            let said = if job.line.is_empty() { "starting gh…" } else { job.line.as_str() };
            line(format!("◌ Cloning {} · {said}", job.remote.slug()), theme::TEXT)
        });
        let said = dialog.said.clone().map(|said| line(said, theme::ERROR));
        let hint =
            if job.is_some() { "esc stops the clone · click outside hides it" } else { "↵ clone · esc close" };
        let panel = panel
            .children(dest.filter(|_| job.is_none()).map(|dest| line(dest, theme::MUTED)))
            .children(busy)
            .children(said)
            .child(dialog::hint(hint));
        dialog::backdrop("clone-backdrop", 0.35, Action::CloneClose { cancel: false }, cx)
            .pt(px(72.))
            .px_4()
            .flex()
            .flex_col()
            .items_center()
            .child(panel)
    }

    /// The dialog and the clone on its way, for a test run's state: names are the user's data,
    /// never in an ordinary reply.
    pub(crate) fn clone_json(&self) -> serde_json::Value {
        let dialog = self.model.clone.as_ref().map(|dialog| {
            let rows: Vec<String> = dialog.rows().iter().map(Remote::slug).collect();
            serde_json::json!({
                "rows": rows, "selected": dialog.selected, "destination": self.model.clone_destination(),
                "said": dialog.said,
            })
        });
        let job = self.model.cloning.as_ref().map(|job| {
            serde_json::json!({"repo": job.remote.slug(), "dest": job.dest, "line": job.line, "cancelled": job.cancelled})
        });
        serde_json::json!({"dialog": dialog, "job": job})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A folder of the test's own, removed when the test ends.
    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp(name: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("ottospace-clone-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir.canonicalize().unwrap())
    }

    /// A stand-in that runs `body`, as if gh.
    struct Script(PathBuf);

    impl Source for Script {
        fn clone_argv(&self, _: &Path, _: &Remote, _: &Path) -> Command {
            Command::new(&self.0)
        }
    }

    fn script(dir: &Path, body: &str) -> Script {
        let path = dir.join("stand-in");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Script(path)
    }

    fn hello() -> Remote {
        Remote::new("octo", "hello").unwrap()
    }

    #[test]
    fn a_failed_or_stopped_clone_removes_only_the_folder_it_made() {
        let root = temp("remove");
        let code = root.0.join("code");
        std::fs::create_dir(&code).unwrap();
        let dest = code.join("hello");
        let gh = Path::new("/usr/bin/true");
        // It wrote into its folder, then failed: the folder goes, and what is beside it stays.
        std::fs::write(code.join("beside"), "x").unwrap();
        let failing = script(&root.0, "mkdir .git; echo 'fatal: oops' >&2; exit 1");
        let made = make(&dest).unwrap();
        let ended = clone(&failing, gh, &hello(), &dest, &AtomicBool::new(false), |_| {});
        assert_eq!(ended, Err(Failure::Said("fatal: oops\n".into())));
        remove_made(&dest, made);
        assert!(!dest.exists() && code.join("beside").exists());
        // Stopped: the program is killed, and says Cancelled.
        let slow = script(&root.0, "sleep 30");
        std::fs::create_dir(&dest).unwrap();
        let stop = AtomicBool::new(true);
        let started = Instant::now();
        assert_eq!(clone(&slow, gh, &hello(), &dest, &stop, |_| {}), Err(Failure::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
        // A folder put in its place (not the one made) is never removed.
        std::fs::remove_dir(&dest).unwrap();
        let other = make(&dest).unwrap();
        std::fs::remove_dir(&dest).unwrap();
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("theirs"), "x").unwrap();
        let replaced = std::fs::symlink_metadata(&dest).map(|m| (m.dev(), m.ino())).unwrap();
        if replaced != other {
            remove_made(&dest, other);
            assert!(dest.join("theirs").exists());
        }
        // Nor a link where it was: the link stays, and what it points at.
        std::fs::remove_dir_all(&dest).unwrap();
        let made = make(&dest).unwrap();
        std::fs::remove_dir(&dest).unwrap();
        std::os::unix::fs::symlink(&code, &dest).unwrap();
        remove_made(&dest, made);
        assert!(std::fs::symlink_metadata(&dest).is_ok() && code.join("beside").exists());
    }

    #[test]
    fn nothing_runs_where_something_is_already_or_the_folder_is_missing() {
        let root = temp("exists");
        let code = root.0.join("code");
        let dest = code.join("hello");
        let never = script(&root.0, &format!("touch '{}'", root.0.join("ran").display()));
        let stop = AtomicBool::new(false);
        assert_eq!(run(&never, &hello(), &dest, false, &stop, |_| {}), Err(Failure::DirMissing));
        std::fs::create_dir(&code).unwrap();
        std::os::unix::fs::symlink("/nowhere", &dest).unwrap();
        assert_eq!(run(&never, &hello(), &dest, false, &stop, |_| {}), Err(Failure::Exists));
        // A destination that is not the remote's name is refused at the door.
        let elsewhere = code.join("other");
        assert!(matches!(run(&never, &hello(), &elsewhere, false, &stop, |_| {}), Err(Failure::Said(_))));
        assert!(!root.0.join("ran").exists());
        assert_eq!(make(&dest), Err(Failure::Exists));
    }

    #[test]
    fn the_last_line_is_gits_progress_or_its_last_message() {
        assert_eq!(
            last_line("Cloning into 'x'...\nReceiving objects:  10%\rReceiving objects:  42%\r").unwrap(),
            "Receiving objects:  42%"
        );
        assert_eq!(last_line("\n\r \n"), None);
    }
}
