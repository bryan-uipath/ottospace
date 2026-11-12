//! Agent runners (Claude Code, Codex): which sessions exist for a directory and what each is
//! doing. Reads the runners' own session files, read-only. Only derived facts leave this crate:
//! names, counts, times. Never prompts, file contents or command lines.
//! Design: `docs/ai/agent-runners.md`.

pub mod claude;
pub mod codex;
pub mod git;
mod jsonl;
pub mod report;
pub mod snapshot;
pub mod station;

use std::collections::BTreeSet;
use std::io;
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
    model: Option<String>,
    /// Last model request seen, e.g. a Claude message id: one request spans several records.
    step: Option<String>,
    /// Sub-agents started and not yet ended, by id.
    agents: BTreeSet<String>,
    /// What a finished command turned out to be, held until its tool call's output record.
    refined: Option<Refined>,
}

impl Cursor {
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Refined {
    /// The call it belongs to, when the record names one; else the next output record's.
    call: Option<String>,
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

/// The normalised event set. Summaries are names only, e.g. `git push` or `src/main.rs`.
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

/// One agent runner's sessions on this machine.
pub trait Runner {
    fn kind(&self) -> Kind;
    /// Sessions started in `dir` or below it, newest activity first.
    fn sessions(&self, dir: &Path) -> io::Result<Vec<SessionRef>>;
    /// Events after `after`. A half-written last line is left for the next call; unknown records are skipped.
    fn read(&self, session: &SessionRef, after: Cursor) -> io::Result<(Vec<Event>, Cursor)>;
}
