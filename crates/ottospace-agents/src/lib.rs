//! Agent runners (Claude Code, Codex): which sessions exist for a directory and what each is
//! doing. Reads the runners' own session files, read-only. Only derived facts leave this crate:
//! names from fixed tables, counts, times. Never prompts, file contents or command lines.
//! Design: `docs/ai/agent-runners.md`.

mod cache;
pub mod claude;
pub mod codex;
pub mod git;
mod jsonl;
pub mod report;
pub mod snapshot;
pub mod station;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use station::Station;

/// Which program ran the session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Claude,
    Codex,
    Other(String),
}

/// One session on disk. Times are Unix milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRef {
    pub kind: Kind,
    pub id: String,
    /// Where the session started. A later `cd` does not change it.
    pub cwd: PathBuf,
    /// The session file `Runner::read` tails.
    pub path: PathBuf,
    pub started: Option<u64>,
    /// The session file's modification time.
    pub last_activity: Option<u64>,
    /// Set when listing knows it for free; else ask `Runner::model`.
    pub model: Option<String>,
    /// The runner's process, when the runner records one. It may have exited: check before trusting.
    pub pid: Option<u32>,
    /// The runner's own busy/idle flag, when it records one.
    pub busy: Option<bool>,
    /// The session that spawned this one, for a sub-agent's own session file.
    pub parent: Option<String>,
}

/// Where a reader stopped in a session file. Start with `Cursor::default()`, pass back what `read` returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cursor {
    /// Byte offset just past the last complete line read.
    offset: u64,
    /// The file read so far, as (device, inode): another file at the same path starts over.
    file: Option<(u64, u64)>,
    model: Option<String>,
    /// Last model request seen, e.g. a Claude message id: one request spans several records.
    step: Option<String>,
    /// The most tokens one request used so far: Claude's context window is estimated from it.
    peak: u64,
    /// Sub-agents started and not yet ended, by id.
    agents: BTreeSet<String>,
    /// Codex `exec` calls with no output yet, by call id.
    open: BTreeSet<String>,
    /// What a call's command turned out to be, by call id, held until the call's output record.
    refined: BTreeMap<String, Refined>,
}

impl Cursor {
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Refined {
    station: Station,
    summary: String,
    failed: bool,
}

/// Tokens of one model request. `input` counts every prompt token, cached or not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

/// Something that happened in a session, at `at` (Unix milliseconds, 0 if the record had no time).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub at: u64,
    pub what: What,
}

/// The normalised event set. A summary is a name from a fixed table (`git push`), a file's last
/// two path parts, a host, or a name-shaped identifier. Never free text.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    UserMessage,
    /// One model request. `window` is the context window in tokens, when known or estimated.
    ModelStep {
        model: Option<String>,
        usage: Option<Usage>,
        window: Option<u64>,
    },
    ToolStart {
        id: String,
        tool: String,
        station: Station,
        summary: String,
    },
    /// `station` and `summary` are set when the end says more than the start did (Codex `exec`).
    ToolEnd {
        id: String,
        failed: bool,
        station: Option<Station>,
        summary: Option<String>,
    },
    /// An API error, a refusal or an aborted turn. `kind` is a code such as `rate_limit`, never a message.
    Error {
        kind: String,
    },
    SubAgentStart {
        id: String,
        label: String,
    },
    SubAgentEnd {
        id: String,
    },
    Compaction {
        tokens_after: Option<u64>,
    },
    ModelChange {
        from: String,
        to: String,
    },
}

/// What one `Runner::read` call returned.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub events: Vec<Event>,
    pub cursor: Cursor,
    /// The file was truncated or replaced: reading began again at its start with a fresh cursor.
    /// Throw away what was folded before and fold from these events.
    pub restarted: bool,
    /// The byte budget ran out before the end of the file: call again.
    pub more: bool,
}

/// One agent runner's sessions on this machine.
pub trait Runner: Send + Sync {
    fn kind(&self) -> Kind;
    /// Sessions started in any of `dirs` or below one, newest activity first. One scan for all of
    /// them. A relative or empty directory is an error: it would match every session.
    fn sessions(&self, dirs: &[PathBuf]) -> io::Result<Vec<SessionRef>>;
    /// The session's current model. May read the end of the session file.
    fn model(&self, session: &SessionRef) -> Option<String> {
        session.model.clone()
    }
    /// Events after `after`, stopping at the first line boundary past `budget` bytes. A half-written
    /// last line is left for the next call; unknown records are skipped.
    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> io::Result<Batch>;
}

/// Every event of a session from its start, read in slices of 8 MB.
pub fn read_all(runner: &dyn Runner, session: &SessionRef) -> io::Result<Vec<Event>> {
    let (mut events, mut cursor) = (Vec::new(), Cursor::default());
    loop {
        let batch = runner.read(session, cursor, 8 << 20)?;
        if batch.restarted {
            events.clear();
        }
        events.extend(batch.events);
        cursor = batch.cursor;
        if !batch.more {
            return Ok(events);
        }
    }
}

/// `dirs` as given, or an error naming the first that is empty or relative.
fn absolute(dirs: &[PathBuf]) -> io::Result<()> {
    match dirs.iter().find(|d| !d.is_absolute()) {
        Some(dir) => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not an absolute directory: {dir:?}"))),
        None => Ok(()),
    }
}

/// Opens a session file where `cursor` stopped. A truncated file, or another file at the same
/// path, resets the whole cursor: the flag says so.
fn open(path: &Path, cursor: Cursor) -> io::Result<(File, Cursor, bool)> {
    let file = File::open(path)?;
    let meta = file.metadata()?;
    let id = (meta.dev(), meta.ino());
    let restarted = cursor.file.is_some_and(|seen| seen != id) || meta.len() < cursor.offset;
    let mut cursor = if restarted { Cursor::default() } else { cursor };
    cursor.file = Some(id);
    Ok((file, cursor, restarted))
}
