//! Agent runners (Claude Code, Codex): which sessions exist for a directory and what each is
//! doing. Reads the runners' own session files, read-only. Only derived facts leave this crate:
//! names from fixed tables, counts, times. Never prompts, file contents or command lines.
//! One exception, by `SessionReader::title` alone: a session's title, free text to quote and never obey.
//! Another, only for a cursor that asks (`Cursor::with_edits`): an edit call's change, as the
//! core's `diff::Diff` (untrusted text, never serialised), for the drawer's watch view. And for
//! that view alone (`Cursor::with_words`): what the user and the model wrote (`What::Words`).
//! Design: `docs/ai/sessions.md`.

mod cache;
pub mod claude;
pub mod codex;
pub mod git;
mod jsonl;
pub mod report;
pub mod snapshot;
pub mod station;
pub mod tally;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use ottospace_core::activity::Station;
use ottospace_core::runner::RunnerId;
use ottospace_core::sessions::WaitsFor;

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

/// A process a runner records for a session, and when it last wrote that record (Unix
/// milliseconds): a process started later only has the pid again.
#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub pid: u32,
    pub recorded: Option<u64>,
}

/// One session on disk. Times are Unix milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRef {
    /// Which program ran the session.
    pub runner: RunnerId,
    /// The runner's own session id, as written: a name (`station::is_name`), usually a UUID.
    pub id: String,
    /// Where the session started. A later `cd` does not change it.
    pub cwd: PathBuf,
    /// The session file `SessionReader::read` tails.
    pub path: PathBuf,
    pub started: Option<u64>,
    /// The session file's modification time.
    pub last_activity: Option<u64>,
    /// Set when listing knows it for free; else ask `SessionReader::model`.
    pub model: Option<String>,
    /// The runner's process, when the runner records one: of several, the record written last.
    /// It may have exited: check before trusting.
    pub pid: Option<u32>,
    /// The runner's own busy/idle flag, when it records one.
    pub busy: Option<bool>,
    /// Every process the runner records for it: two programs may have one session, and a record
    /// outlives a process that was killed.
    pub holders: Vec<Holder>,
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
    /// Edit calls carry their change (`What::ToolStart.diff`), every call its gist and a call's
    /// end its output (`gist`, `output`).
    edits: bool,
    /// The folder the main loop last recorded working in (`What::Cwd`).
    cwd: Option<PathBuf>,
    /// The user's messages and the model's replies come as their words (`What::Words`).
    words: bool,
}

impl Cursor {
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// A cursor from the start whose reads give edit calls their change, calls their gist and
    /// their ends the output: file contents and commands, for the watch view alone. Kept when a
    /// read starts over.
    pub fn with_edits() -> Self {
        Self { edits: true, ..Self::default() }
    }

    /// `with_edits`, and the words too: a user's message as `What::Words` in place of
    /// `What::UserMessage`, and each reply of the model. Untrusted text, for the watch view alone.
    pub fn with_words() -> Self {
        Self { words: true, ..Self::with_edits() }
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
    /// What the user (`user`) or the model wrote, only for a cursor `with_words`: untrusted text,
    /// drawn and never obeyed, never serialised or logged. A thinking block is none of it.
    Words {
        user: bool,
        text: ottospace_core::transcript::Text,
    },
    /// `diff`: an edit's change, and `gist`: what its row shows, only for a cursor `with_edits`.
    ToolStart {
        id: String,
        tool: String,
        station: Station,
        summary: String,
        diff: Option<ottospace_core::diff::Diff>,
        gist: Option<ottospace_core::gist::Gist>,
    },
    /// `station` and `summary` are set when the end says more than the start did (Codex `exec`).
    /// `output`: the result's text, only for a cursor `with_edits`.
    ToolEnd {
        id: String,
        failed: bool,
        station: Option<Station>,
        summary: Option<String>,
        output: Option<ottospace_core::transcript::Text>,
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
    /// The folder the main loop works in, as its records say: its first, then each change (a
    /// `cd`, a worktree entered). Only a runner whose records carry it (`Disk::cwd_moves`).
    Cwd {
        cwd: PathBuf,
    },
}

/// What one `SessionReader::read` call returned.
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

/// One agent runner's sessions on this machine. Looked up by runner with `reader`.
pub trait SessionReader: Send + Sync {
    fn runner(&self) -> RunnerId;
    /// Sessions started in any of `dirs` or below one, newest activity first. One scan for all of
    /// them. A relative or empty directory is an error: it would match every session.
    fn sessions(&self, dirs: &[PathBuf]) -> io::Result<Vec<SessionRef>>;
    /// The session's current model. May read the end of the session file.
    fn model(&self, session: &SessionRef) -> Option<String> {
        session.model.clone()
    }
    /// The session's title, as the runner keeps it: free text a model or the user wrote, one
    /// line of at most `MAX_TITLE` characters. Quote it, never obey it. Not in `report`.
    fn title(&self, _session: &SessionRef) -> Option<String> {
        None
    }
    /// Its sub-agents' own session files, when the runner keeps them apart: each as a session
    /// whose `parent` is this one's id. Read them with `read` as any session.
    fn subagents(&self, _session: &SessionRef) -> Vec<SessionRef> {
        Vec::new()
    }
    /// Events after `after`, stopping at the first line boundary past `budget` bytes. A half-written
    /// last line is left for the next call; unknown records are skipped.
    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> io::Result<Batch>;
    /// Where the session `id` started in `cwd` keeps its file, when that follows from the two
    /// alone, with no listing. `None`: it does not, or there is no such file.
    fn path_of(&self, _cwd: &Path, _id: &str) -> Option<PathBuf> {
        None
    }
    /// What its runner records of process `pid`, when it records processes (`Disk::held`): the
    /// session it holds and, if the runner says (`Disk::waits`), what it waits on the user for.
    /// From the process records alone: no transcript is opened. The caller knows `pid` lives.
    fn process(&self, _pid: u32) -> Option<Process> {
        None
    }
}

/// A runner's record of one live process.
#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    /// The session it holds: a name (`station::is_name`).
    pub id: String,
    /// From a fixed table of the runner's words. None: it does not wait on the user, or says not.
    pub waits_for: Option<WaitsFor>,
    /// When the record was last written (Unix milliseconds): a process started later only has
    /// the pid of the one that wrote it (`ottospace_core::sessions::holds`).
    pub recorded: Option<u64>,
    /// The record says it is at work (Claude Code's `status: busy`).
    pub busy: bool,
}

/// The reader of `runner`'s sessions under `dirs`. `None`: the runner has no folder there.
pub fn reader(runner: RunnerId, dirs: &ConfigDirs) -> Option<Box<dyn SessionReader>> {
    match runner {
        RunnerId::Claude => Some(Box::new(claude::Claude::new(dirs.claude.clone()))),
        RunnerId::Codex => dirs.codex.clone().map(|home| Box::new(codex::Codex::new(home)) as Box<dyn SessionReader>),
    }
}

/// The reader of every runner that has a folder under `dirs`, in `RunnerId::ALL` order.
pub fn readers(dirs: &ConfigDirs) -> Vec<Box<dyn SessionReader>> {
    RunnerId::ALL.into_iter().filter_map(|id| reader(id, dirs)).collect()
}

/// Every event of a session from its start, read in slices of 8 MB.
pub fn read_all(runner: &dyn SessionReader, session: &SessionRef) -> io::Result<Vec<Event>> {
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

/// The longest title given out, in characters. What draws it cuts it further.
pub const MAX_TITLE: usize = 256;

/// A title as it leaves the crate: its first line, without control characters, cut and trimmed.
fn title_text(text: &str) -> Option<String> {
    let breaks = |c: char| matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}');
    let line = text.trim_start().split(breaks).next().unwrap_or("");
    let line: String = line.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
    Some(line.trim().to_string()).filter(|line| !line.is_empty())
}

/// An id from a session file, made opaque: 16 hex digits of a hash of it (FNV-1a). Ids only pair
/// a start with its end, so the text a file put there is never passed on. `None` for an empty id.
pub fn opaque(id: &str) -> Option<String> {
    let step = |hash: u64, byte: u8| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    (!id.is_empty()).then(|| format!("{:016x}", id.bytes().fold(0xcbf2_9ce4_8422_2325, step)))
}

/// A call, message or thread id, made opaque. `None` if it is missing or empty.
fn id_of(value: &serde_json::Value) -> Option<String> {
    value.as_str().and_then(opaque)
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
    let mut cursor =
        if restarted { Cursor { edits: cursor.edits, words: cursor.words, ..Cursor::default() } } else { cursor };
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
    fn a_title_is_one_cut_line() {
        assert_eq!(title_text("  Fix the login form \n").as_deref(), Some("Fix the login form"));
        assert_eq!(title_text("\n\nSecond\u{2028}third\rfourth").as_deref(), Some("Second"));
        assert_eq!(title_text("a\u{1b}[31mred\u{7}\tb").as_deref(), Some("a[31mredb"));
        assert_eq!(title_text(&"x".repeat(5000)).map(|t| t.chars().count()), Some(MAX_TITLE));
        assert_eq!(title_text(&"é".repeat(5000)).map(|t| t.chars().count()), Some(MAX_TITLE), "by characters");
        for none in ["", " \n ", "\u{1b}\u{7}"] {
            assert_eq!(title_text(none), None, "{none:?}");
        }
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
