//! Finds the agent sessions (Claude Code, Codex) of the listed repositories, for the Sessions
//! group of the Tasks section. Reading them is file I/O, so a thread of its own does it: the
//! window asks (`Poller::ask`), and takes what was found when it is there (`Poller::take`).
//!
//! Read-only. Nothing attaches to a session, types into it or signals it. Only what
//! `ottospace-sessions` derives leaves this file: a runner, an id, a directory, times, a station
//! and a label from that crate's fixed tables.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use ottospace_core::repos::Repo;
use ottospace_core::sessions::Found;
use ottospace_sessions::claude::Claude;
use ottospace_sessions::codex::Codex;
use ottospace_sessions::report::IDLE_AFTER_MS;
use ottospace_sessions::snapshot::Snapshot;
use ottospace_sessions::{Cursor, Kind, Runner, SessionRef};

use crate::{config, repos};

/// How often the sessions are read while the Tasks section is open.
pub const EVERY_MS: u64 = 5_000;

/// The most bytes of one session read in one round. A long transcript is caught up over several.
const BUDGET: u64 = 4 << 20;

/// The most sessions whose transcripts are followed at once: the most recently active.
const FOLLOWED: usize = 8;

/// How far a session's transcript has been read, and what it added up to.
#[derive(Default)]
struct Follow {
    cursor: Cursor,
    snapshot: Snapshot,
    /// Read to the end at the last round: what the snapshot says is current.
    caught_up: bool,
}

pub struct Poller {
    ask: Sender<u64>,
    found: Receiver<Vec<Found>>,
    /// A round was asked for and has not come back.
    waiting: bool,
    /// When the last round was asked for. `None`: the section was closed since.
    asked: Option<u64>,
    /// The reading thread is gone: nothing more will be found.
    stopped: bool,
    reads: Arc<AtomicU64>,
}

impl Poller {
    /// `None` if `"sessions": false`, or no session directory is named (a test run names none
    /// by itself): then nothing is ever read.
    pub fn start() -> Option<Self> {
        let dirs = config::session_dirs()?;
        let (ask, asked) = channel::<u64>();
        let (send, found) = channel();
        let reads = Arc::new(AtomicU64::new(0));
        let count = reads.clone();
        let spawned = std::thread::Builder::new().name("sessions".into()).spawn(move || {
            let claude = Claude::new(dirs.claude);
            let codex = dirs.codex.map(Codex::new);
            let mut runners: Vec<&dyn Runner> = vec![&claude];
            runners.extend(codex.as_ref().map(|codex| codex as &dyn Runner));
            let mut follows: HashMap<PathBuf, Follow> = HashMap::new();
            // Ends when the window is gone.
            while let Ok(now) = asked.recv() {
                // The repositories as they are on disk now, read here: not on the window's thread.
                let found = read(&runners, &repos::scan(), now, &mut follows);
                count.fetch_add(1, Ordering::Relaxed);
                if send.send(found).is_err() {
                    break;
                }
            }
        });
        spawned.ok().map(|_| Self { ask, found, waiting: false, asked: None, stopped: false, reads })
    }

    /// Asks for a round if the Tasks section is open and it is time: at once when the section
    /// opens, then every `EVERY_MS`. Never while it is closed.
    pub fn ask(&mut self, open: bool, now: u64) {
        if !open {
            self.asked = None;
            return;
        }
        let due = self.asked.is_none_or(|at| now.saturating_sub(at) >= EVERY_MS);
        if due && !self.waiting && !self.stopped {
            match self.ask.send(now) {
                Ok(()) => (self.waiting, self.asked) = (true, Some(now)),
                Err(_) => self.stopped = true,
            }
        }
    }

    /// What the last round found, once.
    pub fn take(&mut self) -> Option<Vec<Found>> {
        let mut last = None;
        loop {
            match self.found.try_recv() {
                Ok(found) => last = Some(found),
                Err(TryRecvError::Empty) => break,
                // The thread ended (it panicked): no round is on its way any more.
                Err(TryRecvError::Disconnected) => {
                    (self.stopped, self.waiting) = (true, false);
                    break;
                }
            }
        }
        self.waiting &= last.is_none();
        last
    }

    /// The reading thread is gone. Sessions cannot be listed until the app starts again.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// How many rounds were read since the app started.
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }
}

/// One round: the sessions of every listed repository and worktree, and, for the few most
/// recently active, what they are doing now.
fn read(runners: &[&dyn Runner], listed: &[Repo], now: u64, follows: &mut HashMap<PathBuf, Follow>) -> Vec<Found> {
    let checkouts: Vec<PathBuf> = listed
        .iter()
        .flat_map(|repo| std::iter::once(&repo.path).chain(repo.worktrees.iter().map(|w| &w.path)))
        .map(PathBuf::from)
        .collect();
    if checkouts.is_empty() {
        follows.clear();
        return Vec::new();
    }
    let mut sessions: Vec<(&dyn Runner, SessionRef)> = Vec::new();
    for runner in runners {
        // A runner whose folder cannot be read has no sessions to show.
        for session in runner.sessions(&checkouts).unwrap_or_default() {
            // A sub-agent's own file is part of its parent's session, not one more.
            if session.parent.is_none() && !sessions.iter().any(|(_, s)| s.path == session.path) {
                sessions.push((*runner, session));
            }
        }
    }
    sessions.sort_by_key(|(_, session)| std::cmp::Reverse(session.last_activity));
    let active = |session: &SessionRef| session.last_activity.is_some_and(|at| now.saturating_sub(at) <= IDLE_AFTER_MS);
    // How far a transcript was read is kept while its session is listed, at work or not: after a
    // pause the reading goes on from there, not from the start of a long file. (A file replaced
    // or cut short starts over by itself: the cursor knows which file it read.)
    follows.retain(|path, _| sessions.iter().any(|(_, session)| session.path == *path));
    let mut found = Vec::new();
    for (index, (runner, session)) in sessions.iter().enumerate() {
        let runner_name = match session.kind {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::Other(_) => continue,
        };
        // The longest listed checkout the session started in or below: a worktree inside its
        // repository's folder is the worktree's.
        let inside = checkouts.iter().filter(|dir| session.cwd.starts_with(dir));
        let Some(repo) = inside.max_by_key(|dir| dir.as_os_str().len()) else { continue };
        let (mut station, mut label) = (None, None);
        if active(session) && index < FOLLOWED {
            let follow = follows.entry(session.path.clone()).or_default();
            advance(*runner, session, follow);
            if follow.caught_up {
                let name = |station| serde_json::to_value(station).ok().and_then(|v| v.as_str().map(str::to_string));
                station = follow.snapshot.station().and_then(name);
                label = follow.snapshot.running.last().map(|call| call.summary.clone());
            }
        }
        found.push(Found {
            runner: runner_name.to_string(),
            id: session.id.clone(),
            cwd: session.cwd.to_string_lossy().into_owned(),
            repo: repo.to_string_lossy().into_owned(),
            started: session.started,
            last_activity: session.last_activity,
            active: active(session),
            station,
            label,
        });
    }
    found
}

/// Reads on from where the last round stopped, at most `BUDGET` bytes.
fn advance(runner: &dyn Runner, session: &SessionRef, follow: &mut Follow) {
    match runner.read(session, std::mem::take(&mut follow.cursor), BUDGET) {
        Ok(batch) => {
            if batch.restarted {
                follow.snapshot = Snapshot::default();
            }
            batch.events.iter().for_each(|event| follow.snapshot.apply(event));
            (follow.cursor, follow.caught_up) = (batch.cursor, !batch.more);
        }
        // Gone or unreadable since it was listed: start again if it comes back.
        Err(_) => *follow = Follow::default(),
    }
}
