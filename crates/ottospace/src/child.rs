//! A program run to its end off the main thread, as the Repos section runs git and gh: nothing on
//! its standard input, its output read up to a cap, killed if it takes too long.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What it said: whether it succeeded, its standard output and error (each cut to `most` bytes).
pub struct Said {
    pub ok: bool,
    pub out: String,
    pub err: String,
}

/// Runs `command` (its argv and environment set by the caller) for at most `timeout`.
pub fn run(mut command: Command, timeout: Duration, most: u64) -> Result<Said, String> {
    let name = command.get_program().to_string_lossy().rsplit('/').next().unwrap_or("program").to_string();
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|err| format!("{name}: {err}"))?;
    // Read on threads: a full pipe would hold the program up while this waits.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(most).read_to_end(&mut bytes);
            }
            let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
        });
        rx
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} took longer than {} seconds", timeout.as_secs()));
            }
            Err(err) => return Err(format!("{name}: {err}")),
        }
    };
    // A process it left behind (a hook's `&`) may hold its pipes open: what came by then is enough.
    let done = |rx: mpsc::Receiver<String>| rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    Ok(Said { ok: status.success(), out: done(out), err: done(err) })
}

/// `git -C <dir>`: no prompt, and no environment that points it at another repository.
pub fn git(dir: &std::path::Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).env("GIT_TERMINAL_PROMPT", "0");
    for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY"] {
        command.env_remove(name);
    }
    command
}
