//! Finds the agent sessions (Claude Code, Codex) of the listed repositories, for the Sessions
//! group of the Tasks section. Reading them is file I/O, so a thread of its own does it: the
//! window asks (`Poller::ask`), and takes what was found when it is there (`Poller::take`).
//!
//! While the section is closed nothing is listed, but tasks adopted from sessions still follow
//! them: a slow round looks only at when those sessions' files last changed.
//!
//! Read-only. Nothing attaches to a session, types into it or signals it. Only what
//! `ottospace-sessions` derives leaves this file: a runner, an id, a directory, times, a station
//! and a label from that crate's fixed tables.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use ottospace_core::repos::Repo;
use ottospace_core::sessions::{Found, Seen};
use ottospace_sessions::claude::Claude;
use ottospace_sessions::codex::Codex;
use ottospace_sessions::report::IDLE_AFTER_MS;
use ottospace_sessions::snapshot::Snapshot;
use ottospace_sessions::{Cursor, Kind, Runner, SessionRef};

use crate::{config, repos};

/// How often the sessions are read while the Tasks section is open.
pub const EVERY_MS: u64 = 5_000;

/// How often the files of adopted sessions are looked at while it is closed. Shorter than the
/// time after which a session reads idle, so work between two looks is not missed.
pub const SLOW_MS: u64 = 20_000;

/// A session whose file last changed at `at` is at work at `now`: for the rows and for the
/// tasks that follow sessions alike.
fn recent(at: u64, now: u64) -> bool {
    now.saturating_sub(at) <= IDLE_AFTER_MS
}

/// A session, by runner and id.
type Key = (String, String);

/// What the window asks the reading thread for.
enum Ask {
    /// Every session of the listed repositories, at this time.
    All(u64),
    /// Only when the files of these sessions last changed.
    Adopted(u64, Vec<Key>),
}

/// What a round read.
pub enum Read {
    Found(Vec<Found>),
    Seen(Vec<Seen>),
}

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
    ask: Sender<Ask>,
    found: Receiver<Read>,
    /// The last round asked for was for the open section.
    open: bool,
    slow_reads: Arc<AtomicU64>,
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
        let (ask, asked) = channel::<Ask>();
        let (send, found) = channel();
        let (reads, slow_reads) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (count, slow_count) = (reads.clone(), slow_reads.clone());
        let spawned = std::thread::Builder::new().name("sessions".into()).spawn(move || {
            let claude = Claude::new(dirs.claude);
            let codex = dirs.codex.map(Codex::new);
            let mut runners: Vec<&dyn Runner> = vec![&claude];
            runners.extend(codex.as_ref().map(|codex| codex as &dyn Runner));
            let mut follows: HashMap<PathBuf, Follow> = HashMap::new();
            // Where each session's file is, as the listing found it: an adopted session is
            // looked at by this path, with no listing. Kept in memory only.
            let mut files: HashMap<Key, PathBuf> = HashMap::new();
            // Adopted sessions already looked for with a listing, found or not. One that a
            // listing did not find is not looked for again until the Tasks section is opened.
            let mut sought: HashSet<Key> = HashSet::new();
            // When each adopted session's file had last changed, at the last slow round.
            let mut last: HashMap<Key, u64> = HashMap::new();
            // Ends when the window is gone.
            while let Ok(ask) = asked.recv() {
                let read = match ask {
                    Ask::All(now) => {
                        // The repositories as they are on disk now, read here: not on the window's thread.
                        let found = read(&runners, &repos::scan(), now, &mut follows, &mut files);
                        count.fetch_add(1, Ordering::Relaxed);
                        // The section is open: what was not found before may be looked for again.
                        sought.clear();
                        Read::Found(found)
                    }
                    Ask::Adopted(now, keys) => {
                        // A session adopted in an earlier run has no known file yet. One listing
                        // for all of those at once finds where their files are: it lists the
                        // session folders and reads each file's first records, as any listing
                        // does, and follows no transcript.
                        let missing: Vec<&Key> =
                            keys.iter().filter(|key| !files.contains_key(*key) && !sought.contains(*key)).collect();
                        if !missing.is_empty() {
                            sought.extend(missing.into_iter().cloned());
                            locate(&runners, &repos::scan(), &mut files);
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                        slow_count.fetch_add(1, Ordering::Relaxed);
                        Read::Seen(seen(&keys, &files, &mut last, now))
                    }
                };
                if send.send(read).is_err() {
                    break;
                }
            }
        });
        spawned.ok().map(|_| Self {
            ask,
            found,
            open: false,
            slow_reads,
            waiting: false,
            asked: None,
            stopped: false,
            reads,
        })
    }

    /// Asks for a round if it is time. With the Tasks section open: every session, at once when
    /// the section opens, then every `EVERY_MS`. With it closed: only the files of the sessions
    /// tasks still follow, at once when it closes, then every `SLOW_MS`; and nothing at all
    /// when there are none. `adopted` says which those are: asked only when a slow round is due.
    pub fn ask(&mut self, open: bool, adopted: impl FnOnce() -> Vec<Key>, now: u64) {
        if open != self.open {
            (self.open, self.asked) = (open, None);
        }
        let every = if open { EVERY_MS } else { SLOW_MS };
        let due = self.asked.is_none_or(|at| now.saturating_sub(at) >= every);
        if due && !self.waiting && !self.stopped {
            let ask = if open { Ask::All(now) } else { Ask::Adopted(now, adopted()) };
            // No task follows a session: nothing to look at until the next slow round is due.
            if matches!(&ask, Ask::Adopted(_, keys) if keys.is_empty()) {
                self.asked = Some(now);
                return;
            }
            match self.ask.send(ask) {
                Ok(()) => (self.waiting, self.asked) = (true, Some(now)),
                Err(_) => self.stopped = true,
            }
        }
    }

    /// What the rounds since read, oldest first.
    pub fn take(&mut self) -> Vec<Read> {
        let mut last = Vec::new();
        loop {
            match self.found.try_recv() {
                Ok(read) => last.push(read),
                Err(TryRecvError::Empty) => break,
                // The thread ended (it panicked): no round is on its way any more.
                Err(TryRecvError::Disconnected) => {
                    (self.stopped, self.waiting) = (true, false);
                    break;
                }
            }
        }
        self.waiting &= last.is_empty();
        last
    }

    /// How many rounds looked only at adopted sessions' files.
    pub fn slow_reads(&self) -> u64 {
        self.slow_reads.load(Ordering::Relaxed)
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

/// When the file of each of `keys` last changed, for those whose file is known and still there.
/// One `stat` each: no directory is listed and no file is opened. A session is at work if its
/// file changed lately, or changed since the round before (`last`): a write just after one
/// round, then a long silent command, is still work at the next.
fn seen(keys: &[Key], files: &HashMap<Key, PathBuf>, last: &mut HashMap<Key, u64>, now: u64) -> Vec<Seen> {
    last.retain(|key, _| keys.contains(key));
    let mut changed = |key: &Key| {
        let modified = std::fs::metadata(files.get(key)?).ok()?.modified().ok()?;
        let at = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
        let advanced = last.insert(key.clone(), at).is_some_and(|before| at > before);
        let (runner, id) = key.clone();
        Some(Seen { runner, id, last_activity: at, active: recent(at, now) || advanced })
    };
    keys.iter().filter_map(|key| changed(key)).collect()
}

/// A runner's name as tasks and rows keep it.
fn runner_name(kind: &Kind) -> Option<&'static str> {
    match kind {
        Kind::Claude => Some("claude"),
        Kind::Codex => Some("codex"),
        Kind::Other(_) => None,
    }
}

/// The listed repositories and worktrees, and every session started in or below one, the most
/// recently active first. The runners list their folders and read each session file's first
/// records (cached) to learn where it started.
fn list<'a>(runners: &[&'a dyn Runner], listed: &[Repo]) -> (Vec<PathBuf>, Vec<(&'a dyn Runner, SessionRef)>) {
    let checkouts: Vec<PathBuf> = listed
        .iter()
        .flat_map(|repo| std::iter::once(&repo.path).chain(repo.worktrees.iter().map(|w| &w.path)))
        .map(PathBuf::from)
        .collect();
    let mut sessions: Vec<(&dyn Runner, SessionRef)> = Vec::new();
    for runner in runners.iter().filter(|_| !checkouts.is_empty()) {
        // A runner whose folder cannot be read has no sessions to show.
        for session in runner.sessions(&checkouts).unwrap_or_default() {
            // A sub-agent's own file is part of its parent's session, not one more.
            if session.parent.is_none() && !sessions.iter().any(|(_, s)| s.path == session.path) {
                sessions.push((*runner, session));
            }
        }
    }
    sessions.sort_by_key(|(_, session)| std::cmp::Reverse(session.last_activity));
    (checkouts, sessions)
}

/// Where each listed session's file is, and no more: no transcript is followed.
fn locate(runners: &[&dyn Runner], listed: &[Repo], files: &mut HashMap<Key, PathBuf>) {
    for (_, session) in list(runners, listed).1 {
        if let Some(runner) = runner_name(&session.kind) {
            files.insert((runner.to_string(), session.id), session.path);
        }
    }
}

/// One round: the sessions of every listed repository and worktree, and, for the few most
/// recently active, what they are doing now. `files`: where each one's file is, for `seen`.
fn read(
    runners: &[&dyn Runner],
    listed: &[Repo],
    now: u64,
    follows: &mut HashMap<PathBuf, Follow>,
    files: &mut HashMap<Key, PathBuf>,
) -> Vec<Found> {
    let (checkouts, sessions) = list(runners, listed);
    let active = |session: &SessionRef| session.last_activity.is_some_and(|at| recent(at, now));
    // How far a transcript was read is kept while its session is listed, at work or not: after a
    // pause the reading goes on from there, not from the start of a long file. (A file replaced
    // or cut short starts over by itself: the cursor knows which file it read.)
    follows.retain(|path, _| sessions.iter().any(|(_, session)| session.path == *path));
    let mut found = Vec::new();
    for (index, (runner, session)) in sessions.iter().enumerate() {
        let Some(runner_name) = runner_name(&session.kind) else { continue };
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
        files.insert((runner_name.to_string(), session.id.clone()), session.path.clone());
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn an_adopted_sessions_file_is_at_work_if_it_changed_lately_or_since_the_last_look() {
        let file = std::env::temp_dir().join(format!("ottospace-seen-{}.jsonl", std::process::id()));
        std::fs::write(&file, "").unwrap();
        let ms = |time: SystemTime| time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let (now_time, minute) = (SystemTime::now(), Duration::from_secs(60));
        let now = ms(now_time);
        let changed = |ago: Duration| {
            let handle = std::fs::File::options().write(true).open(&file).unwrap();
            handle.set_modified(now_time - ago).unwrap();
        };
        let key: Key = ("claude".into(), "s1".into());
        let gone: Key = ("codex".into(), "s2".into());
        let files = HashMap::from([(key.clone(), file.clone()), (gone.clone(), file.with_extension("gone"))]);
        let mut last = HashMap::new();
        let mut look = |keys: &[Key]| seen(keys, &files, &mut last, now);
        let all = [key.clone(), gone.clone(), ("claude".into(), "unknown".into())];
        // Changed just now: at work. A file that is gone, or not known, is not reported.
        changed(Duration::ZERO);
        let first = look(&all);
        assert_eq!((first.len(), first[0].id.as_str(), first[0].active), (1, "s1", true));
        // Last changed ten minutes ago, as at the look before: not at work.
        changed(10 * minute);
        assert!(!look(&all)[0].active);
        assert!(!look(&all)[0].active);
        // Changed since the look before, though more than 30 seconds ago: a write, then a long
        // silent command. That is work.
        changed(2 * minute);
        let after = look(&all);
        assert_eq!((after[0].active, now - after[0].last_activity), (true, 120_000));
        assert!(!look(&all)[0].active, "and not again at the next look");
        let _ = std::fs::remove_file(&file);
    }
}
