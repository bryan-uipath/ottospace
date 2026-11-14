//! Finds the agent sessions (Claude Code, Codex) of the listed repositories, for the Sessions
//! group of the Tasks section. Reading them is file I/O, so a thread of its own does it: the
//! window asks (`Poller::ask`), and takes what was found when it is there (`Poller::take`).
//!
//! While the section is closed nothing is listed, but tasks adopted from sessions still follow
//! them: a slow round looks only at when those sessions' files last changed.
//!
//! Read-only. Nothing attaches to a session, types into it or signals it. Only what
//! `ottospace-sessions` derives leaves this file: a runner, an id, a directory, times, a station
//! and a label from that crate's fixed tables. And two texts the core cuts before it keeps them:
//! the session's model, and its title, which is free text a model wrote: drawn and quoted only.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use ottospace_core::progress::{Clock, Tokens, Work};
use ottospace_core::repos::Repo;
use ottospace_core::runner::RunnerId;
use ottospace_core::sessions::{Found, Seen};
use ottospace_sessions::report::IDLE_AFTER_MS;
use ottospace_sessions::snapshot::Snapshot;
use ottospace_sessions::tally;
use ottospace_sessions::{Cursor, SessionReader, SessionRef, reader, readers};

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

/// A session whose work is read for its task: runner, id, and the task's repository (where a
/// Claude session's file is when no listing found it; none: the home folder).
pub type WorkKey = (String, String, Option<String>);

/// What the window asks the reading thread for. Each round also reads on in the transcripts of
/// the sessions given as work, counting time at work from `day` (the start of the local day).
enum Ask {
    /// Every session of the listed repositories, at this time.
    All(u64, Vec<WorkKey>, u64),
    /// Only when the files of these sessions last changed.
    Adopted(u64, Vec<Key>, Vec<WorkKey>, u64),
}

/// What a round read.
pub enum Read {
    Found(Vec<Found>),
    Seen(Vec<Seen>),
    Work(Vec<Work>),
}

/// The most bytes of one session read in one round. A long transcript is caught up over several.
const BUDGET: u64 = 4 << 20;

/// The most sessions whose transcripts are followed at once: the most recently active.
const FOLLOWED: usize = 8;

/// The most sessions whose title and model are read: the most recently active. More than the
/// rows listed, since a session that is a task's or hidden is no row.
const TITLED: usize = 2 * ottospace_core::sessions::MAX_FOUND;

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
            let readers = readers(&dirs);
            let runners: Vec<&dyn SessionReader> = readers.iter().map(|reader| reader.as_ref()).collect();
            let mut follows: HashMap<PathBuf, Follow> = HashMap::new();
            // Where each session's file is, as the listing found it: an adopted session is
            // looked at by this path, with no listing. Kept in memory only.
            let mut files: HashMap<Key, PathBuf> = HashMap::new();
            // Adopted sessions already looked for with a listing, found or not. One that a
            // listing did not find is not looked for again until the Tasks section is opened.
            let mut sought: HashSet<Key> = HashSet::new();
            // When each adopted session's file had last changed, at the last slow round.
            let mut last: HashMap<Key, u64> = HashMap::new();
            // How far each task's session was read for its work, and what it added up to.
            let mut tallies: HashMap<Key, (tally::Reader, Clock, Vec<u64>)> = HashMap::new();
            // Ends when the window is gone.
            while let Ok(ask) = asked.recv() {
                let (work, day) = match &ask {
                    Ask::All(_, work, day) | Ask::Adopted(_, _, work, day) => (work.clone(), *day),
                };
                let read = match ask {
                    Ask::All(now, ..) => {
                        // The repositories as they are on disk now, read here: not on the window's thread.
                        let found = read(&runners, &repos::scan(), now, &mut follows, &mut files);
                        count.fetch_add(1, Ordering::Relaxed);
                        // The section is open: what was not found before may be looked for again.
                        sought.clear();
                        Read::Found(found)
                    }
                    Ask::Adopted(now, keys, ..) => {
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
                let worked = works(&runners, &work, &files, &mut tallies, day);
                if !worked.is_empty() && send.send(Read::Work(worked)).is_err() {
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
    pub fn ask(&mut self, open: bool, adopted: impl FnOnce() -> Vec<Key>, work: Vec<WorkKey>, now: u64, day: u64) {
        if open != self.open {
            (self.open, self.asked) = (open, None);
        }
        let every = if open { EVERY_MS } else { SLOW_MS };
        let due = self.asked.is_none_or(|at| now.saturating_sub(at) >= every);
        if due && !self.waiting && !self.stopped {
            let ask = if open { Ask::All(now, work, day) } else { Ask::Adopted(now, adopted(), work, day) };
            // No task follows a session: nothing to look at until the next slow round is due.
            if matches!(&ask, Ask::Adopted(_, keys, work, _) if keys.is_empty() && work.is_empty()) {
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

/// The listed repositories and worktrees, and every session started in or below one, the most
/// recently active first. The runners list their folders and read each session file's first
/// records (cached) to learn where it started.
fn list<'a>(
    runners: &[&'a dyn SessionReader],
    listed: &[Repo],
) -> (Vec<PathBuf>, Vec<(&'a dyn SessionReader, SessionRef)>) {
    let checkouts: Vec<PathBuf> = listed
        .iter()
        .flat_map(|repo| std::iter::once(&repo.path).chain(repo.worktrees.iter().map(|w| &w.path)))
        .map(PathBuf::from)
        .collect();
    let mut sessions: Vec<(&dyn SessionReader, SessionRef)> = Vec::new();
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
fn locate(runners: &[&dyn SessionReader], listed: &[Repo], files: &mut HashMap<Key, PathBuf>) {
    for (_, session) in list(runners, listed).1 {
        files.insert((session.runner.name().to_string(), session.id), session.path);
    }
}

/// One round: the sessions of every listed repository and worktree, and, for the few most
/// recently active, what they are doing now. `files`: where each one's file is, for `seen`.
fn read(
    runners: &[&dyn SessionReader],
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
        let Some(repo) = repo_of(&checkouts, session) else { continue };
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
        files.insert((session.runner.name().to_string(), session.id.clone()), session.path.clone());
        // From the end of the file, read again only when the file changed: not a transcript followed.
        let (title, model) = if index < TITLED { (runner.title(session), runner.model(session)) } else { (None, None) };
        found.push(Found { station, label, title, model, ..as_found(session, repo, now) });
    }
    found
}

/// The longest listed checkout the session started in or below: a worktree inside its
/// repository's folder is the worktree's.
fn repo_of<'a>(checkouts: &'a [PathBuf], session: &SessionRef) -> Option<&'a PathBuf> {
    checkouts.iter().filter(|dir| session.cwd.starts_with(dir)).max_by_key(|dir| dir.as_os_str().len())
}

/// What a listing says of a session in `repo`: where and when, at work, held. Nothing it runs.
fn as_found(session: &SessionRef, repo: &std::path::Path, now: u64) -> Found {
    Found {
        runner: session.runner.name().to_string(),
        id: session.id.clone(),
        cwd: session.cwd.to_string_lossy().into_owned(),
        repo: repo.to_string_lossy().into_owned(),
        started: session.started,
        last_activity: session.last_activity,
        active: session.last_activity.is_some_and(|at| recent(at, now)),
        station: None,
        label: None,
        title: None,
        model: None,
        // Only a runner that records its process can say so (`disk.held`). A pid used again by
        // another program reads as alive: then it is not gone on with.
        held: session.runner.spec().disk.held && session.pid.is_some_and(crate::running::alive),
    }
}

/// One session, read for a resume the user asked for: where it started, if it is at work or a
/// process holds it, its title. One listing of the listed checkouts' session folders, as a round
/// makes; no transcript is followed. Called off the window's thread. `None`: no checkout has it.
pub fn read_one(runner: &str, id: &str, now: u64) -> Option<Found> {
    let dirs = config::session_dirs()?;
    // Only that runner's folders.
    let only = reader(RunnerId::named(runner)?, &dirs)?;
    let (checkouts, sessions) = list(&[only.as_ref()], &repos::scan());
    let (reader, session) = sessions.iter().find(|(_, s)| s.id == id)?;
    let repo = repo_of(&checkouts, session)?;
    Some(Found { title: reader.title(session), ..as_found(session, repo, now) })
}

/// What each task's session did, read on from where the last round stopped: tokens by model
/// and time at work (`progress::Clock`, from `day`). Only those read to their end are given:
/// half a transcript's tokens would replace the whole count the task keeps.
fn works(
    runners: &[&dyn SessionReader],
    keys: &[WorkKey],
    files: &HashMap<Key, PathBuf>,
    tallies: &mut HashMap<Key, (tally::Reader, Clock, Vec<u64>)>,
    day: u64,
) -> Vec<Work> {
    tallies.retain(|key, _| keys.iter().any(|(runner, id, _)| (runner, id) == (&key.0, &key.1)));
    let mut out = Vec::new();
    for (runner_name, id, repo) in keys {
        let key = (runner_name.clone(), id.clone());
        let Some(named) = RunnerId::named(runner_name) else { continue };
        let Some(runner) = runners.iter().find(|r| r.runner() == named) else { continue };
        // Where a listing found it; else where the runner keeps the sessions of the task's folder (Claude).
        let cwd = repo.clone().or_else(|| std::env::var("HOME").ok()).map(PathBuf::from).unwrap_or_default();
        let path = files.get(&key).cloned().or_else(|| runner.path_of(&cwd, id));
        let Some(path) = path else { continue };
        let session = SessionRef {
            runner: runner.runner(),
            id: id.clone(),
            cwd,
            path,
            started: None,
            last_activity: None,
            model: None,
            pid: None,
            busy: None,
            parent: None,
        };
        // Times wait until every file is read to its end: a long transcript takes several rounds,
        // and a sub-agent's later times must not come before the main loop's earlier ones.
        let (reader, clock, waiting) = tallies.entry(key).or_default();
        let Ok(read) = reader.advance(*runner, &session, BUDGET) else { continue };
        if read.restarted {
            (*clock, *waiting) = (Clock::default(), Vec::new());
        }
        waiting.extend(read.times);
        if !read.caught_up {
            continue;
        }
        waiting.sort_unstable();
        waiting.drain(..).for_each(|at| clock.tick(at, day));
        let tally = &reader.tally;
        let tokens = tally.models.iter().map(|((model, sub), usage)| Tokens {
            model: model.clone(),
            sub: *sub,
            input: usage.input,
            output: usage.output,
            cached: usage.cached,
        });
        out.push(Work {
            runner: runner_name.clone(),
            session: id.clone(),
            clock: *clock,
            tokens: tokens.collect(),
            context: tally.context,
            window: tally.window,
        });
    }
    out
}

/// Reads on from where the last round stopped, at most `BUDGET` bytes.
fn advance(runner: &dyn SessionReader, session: &SessionRef, follow: &mut Follow) {
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
        let file = crate::scratch::Scratch::path(&format!("ottospace-seen-{}.jsonl", std::process::id()));
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
        let files = HashMap::from([(key.clone(), file.to_path_buf()), (gone.clone(), file.with_extension("gone"))]);
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
    }
}
