//! Agent runners (Claude Code, Codex): which sessions exist for a directory and what each is
//! doing. Reads the runners' own session files, read-only. Only derived facts leave this crate:
//! names from fixed tables, counts, times. Never prompts, file contents or command lines.
//! Design: `docs/ai/sessions.md`.

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
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use station::Station;

/// Where the runners keep their files on this machine, by the environment.
pub struct ConfigDirs {
    /// Claude Code: `$CLAUDE_CONFIG_DIR` if set and not empty, and `~/.claude`.
    pub claude: Vec<PathBuf>,
    /// Codex: `$CODEX_HOME` if set and not empty, or `~/.codex`.
    pub codex: Option<PathBuf>,
}

/// The one place that says where the runners' folders are: sessions are under them, and so are
/// their skills.
pub fn config_dirs() -> ConfigDirs {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let home = set("HOME");
    let mut claude: Vec<PathBuf> = set("CLAUDE_CONFIG_DIR").into_iter().collect();
    if let Some(default) = home.as_ref().map(|home| home.join(".claude")).filter(|dir| !claude.contains(dir)) {
        claude.push(default);
    }
    ConfigDirs { claude, codex: set("CODEX_HOME").or(home.map(|home| home.join(".codex"))) }
}

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
    /// The runner's own session id, as written: a name (`station::is_name`), usually a UUID.
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
    /// The bytes at `offset` are the rest of an oversized line whose end is not written yet:
    /// the next read goes on skipping from there instead of starting the line again.
    skipping: bool,
    /// The file read so far, as (device, inode): another file at the same path starts over.
    file: Option<(u64, u64)>,
    model: Option<String>,
    /// Last model request seen (a Claude message id, made opaque): one request spans several records.
    step: Option<String>,
    /// The most tokens one request used so far: Claude's context window is estimated from it.
    peak: u64,
    /// Sub-agents started and not yet ended, by opaque id. At most `MAX_OPEN`.
    agents: BTreeSet<String>,
    /// Codex `exec` calls with no output yet, by opaque call id. At most `MAX_OPEN`.
    open: BTreeSet<String>,
    /// What a call's command turned out to be, by opaque call id, held until the call's output record.
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

/// The normalised event set. An `id` is opaque: a hash of the runner's id, good only for pairing
/// a start with its end. A `tool`, `summary`, `label`, `kind` or model follows the label rules in
/// `docs/ai/sessions.md`: table entries, a file's last two path parts, a host, or a name.
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
    /// An API error, a refusal or an aborted turn. `kind` is a code from a fixed table, never a message.
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

/// Calls, sub-agents and refinements a cursor tracks at once. Past it the oldest are dropped,
/// so a file cannot grow a cursor without bound.
const MAX_OPEN: usize = 256;

/// An error naming the first of `dirs` that is empty, relative or has a `..` in it.
fn absolute(dirs: &[PathBuf]) -> io::Result<()> {
    let bad = |dir: &&PathBuf| !dir.is_absolute() || dir.components().any(|c| c == Component::ParentDir);
    match dirs.iter().find(bad) {
        Some(dir) => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not an absolute directory: {dir:?}"))),
        None => Ok(()),
    }
}

/// A session's recorded cwd as a path, if it can be one: absolute, at most 1024 bytes, no control
/// characters, no `..`. Rebuilt from its components, so `/a//b/./c/` is `/a/b/c`.
fn recorded_cwd(text: &str) -> Option<PathBuf> {
    let path = Path::new(text);
    let sane = text.len() <= 1024 && !text.chars().any(char::is_control) && path.is_absolute();
    (sane && !path.components().any(|c| c == Component::ParentDir)).then(|| path.components().collect())
}

/// An id from a session file, made opaque: 16 hex digits of a hash of it (FNV-1a). Ids only pair
/// a start with its end, so the text a file put there is never passed on. `None` for an empty id.
fn opaque(id: &str) -> Option<String> {
    let step = |hash: u64, byte: u8| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    (!id.is_empty()).then(|| format!("{:016x}", id.bytes().fold(0xcbf2_9ce4_8422_2325, step)))
}

/// Adds `id` to a bounded set of open things. True if it was not there.
fn track(open: &mut BTreeSet<String>, id: String) -> bool {
    if open.len() >= MAX_OPEN {
        open.pop_first();
    }
    open.insert(id)
}

/// Opens a session file where `cursor` stopped. A truncated file, or another file at the same
/// path, resets the whole cursor: the flag says so.
fn open(path: &Path, cursor: Cursor) -> io::Result<(File, Cursor, bool)> {
    let file = jsonl::open(path)?;
    let meta = file.metadata()?;
    let id = (meta.dev(), meta.ino());
    let restarted = cursor.file.is_some_and(|seen| seen != id) || meta.len() < cursor.offset;
    let mut cursor = if restarted { Cursor::default() } else { cursor };
    cursor.file = Some(id);
    Ok((file, cursor, restarted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_cwd_must_be_a_plain_absolute_path() {
        assert_eq!(recorded_cwd("/work/app"), Some(PathBuf::from("/work/app")));
        assert_eq!(recorded_cwd("/work//app/./src/"), Some(PathBuf::from("/work/app/src")));
        assert!(recorded_cwd("/work/app").unwrap().starts_with("/work/app/"));
        assert!(!recorded_cwd("/work/app-other").unwrap().starts_with("/work/app"));
        for bad in ["", "work/app", "/work/app/../../etc", "/work/..", "/work/a\npp", "/work/\u{1b}[31m"] {
            assert_eq!(recorded_cwd(bad), None, "{bad:?}");
        }
        assert_eq!(recorded_cwd(&format!("/{}", "x".repeat(1024))), None);
        assert!(absolute(&[PathBuf::from("/work/app/../other")]).is_err());
    }

    #[test]
    fn ids_are_opaque() {
        assert_eq!(opaque(""), None);
        assert_eq!(opaque("toolu_01"), opaque("toolu_01"));
        assert_ne!(opaque("toolu_01"), opaque("toolu_02"));
        let id = opaque("an id with a secret in it").unwrap();
        assert!(id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    }

    #[test]
    fn open_things_are_bounded() {
        let mut open = BTreeSet::new();
        for i in 0..1000 {
            track(&mut open, format!("{i:04}"));
        }
        assert_eq!(open.len(), MAX_OPEN);
        assert!(open.contains("0999") && !open.contains("0000"));
    }
}
