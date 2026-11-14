//! Speaks to agent CLIs in their structured modes: starts one, sends it messages and answers,
//! reads its events. Claude Code: `claude -p` with stream-json in and out. Codex: `codex
//! app-server`, JSON-RPC over stdio. Both map into one `Event` stream.
//!
//! The parsers (`claude::Claude`, `codex::Codex`) are pure and total: any line gives events, never a
//! panic. `driver::Driver` runs the process. Everything a model or a tool wrote is `Untrusted`:
//! shown in the drawer, never in the control socket's state. Design: `docs/ai/agent-runners.md`.

pub mod claude;
pub mod codex;
pub mod driver;
pub mod lines;
pub mod payload;

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use ottospace_core::runner::RunnerId;
use ottospace_core::transcript::Outcome;
use ottospace_sessions::station::Station;
use serde_json::Value;

/// Text a model, a tool or the CLI wrote: a reply, a command line, a path, stderr. Shown to the
/// user as text, never as an instruction, never fetched or run. `Debug` prints its length only,
/// so a log or the socket's state cannot carry it by accident.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Untrusted(String);

impl Untrusted {
    /// At most `max` bytes of `text`, cut at a character boundary.
    pub(crate) fn new(text: &str, max: usize) -> Self {
        let mut end = text.len().min(max);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Self(text[..end].to_string())
    }

    pub fn text(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Untrusted {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Untrusted({} bytes)", self.0.len())
    }
}

/// The most of one tool call's or permission request's detail kept: a command line, a path.
pub const MAX_DETAIL: usize = 4096;
/// The most of one message's text kept.
pub const MAX_TEXT: usize = 1 << 20;

/// The longest payload a permission request may have: the most a user can be shown in full.
/// A longer request is denied unseen (`Event::AutoDenied`).
pub const MAX_APPROVABLE: usize = 1 << 20;

/// A permission request, to be answered once. Only a parser makes one, so an answer names a
/// request the CLI really asked. `Debug` shows nothing of it.
#[derive(Clone)]
pub struct Ticket {
    /// The CLI's own request id, echoed back as it came.
    id: Value,
    kind: TicketKind,
    /// Everything the request asks to do, sanitised. At most `MAX_APPROVABLE` bytes.
    full: Untrusted,
}

impl Ticket {
    fn new(id: Value, kind: TicketKind) -> Self {
        Self { id, kind, full: Untrusted::default() }
    }

    /// The whole payload the user approves, when `Event::Permission` said `cut`: a UI shows it
    /// before it offers Allow.
    pub fn full(&self) -> &Untrusted {
        &self.full
    }
}

#[derive(Clone)]
enum TicketKind {
    /// Claude's `can_use_tool`: the tool's input, sent back as it came.
    Claude { input: Value },
    /// Codex's command and file-change approvals: the decision is all the answer says.
    Codex,
    /// A request the CLI withdrew: equal to the one it closes, answered by nothing.
    Closed,
}

impl PartialEq for Ticket {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Ticket")
    }
}

/// The user's answer to a permission request: this one request, as shown, or not. No answer
/// grants more (no "always", no rule, no policy): see "Later: session grants" in the design note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Allow,
    /// Deny; the agent goes on without it. `message`: the user's words to the agent (Claude
    /// only; Codex's answer carries none).
    Deny {
        message: Option<String>,
    },
}

/// An image file's type, read from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Media {
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0x89, b'P', b'N', b'G', ..] => Some(Self::Png),
            [0xff, 0xd8, 0xff, ..] => Some(Self::Jpeg),
            [b'G', b'I', b'F', b'8', ..] => Some(Self::Gif),
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(Self::Webp),
            _ => None,
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

/// An image the user attached. Codex is sent its path, Claude its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub path: PathBuf,
    pub media: Media,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Image({:?}, {} bytes)", self.media, self.bytes.len())
    }
}

/// The largest image attached.
pub const MAX_IMAGE: u64 = 20 << 20;

impl Image {
    /// Reads an image the user picked: a regular file of at most `MAX_IMAGE` bytes whose first
    /// bytes say PNG, JPEG, GIF or WebP.
    pub fn load(path: &Path) -> io::Result<Self> {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_string());
        // Checked before opening too: opening a FIFO would wait for a writer.
        if !std::fs::metadata(path)?.is_file() {
            return Err(invalid("not a regular file"));
        }
        let file = std::fs::File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() > MAX_IMAGE {
            return Err(invalid("not a regular file of at most 20 MB"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_IMAGE + 1).read_to_end(&mut bytes)?;
        let media = Media::sniff(&bytes).ok_or_else(|| invalid("not a PNG, JPEG, GIF or WebP image"))?;
        Ok(Self { path: path.to_path_buf(), media, bytes })
    }
}

/// What goes into a runner.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A user message: text and images. Sent on stdin, never in argv.
    Send {
        text: String,
        images: Vec<Image>,
    },
    Answer {
        ticket: Ticket,
        answer: Answer,
    },
    /// End the current turn. The session goes on.
    Interrupt,
}

/// What comes out of a runner. Ids are opaque (a hash), good only to pair a start with its end.
/// `tool`, `label`, `kind`, `model` and `session` follow `docs/ai/sessions.md`'s label rules: may
/// reach the socket. Everything `Untrusted` is for the drawer alone.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The session is known. `session` is a UUID, `model` a name; else `None`. `mode`: the
    /// permission mode the CLI runs in (Claude's `permissionMode`), a name, the user's own.
    Started {
        session: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    },
    TurnStarted,
    /// More of the reply being written.
    TextDelta {
        text: Untrusted,
    },
    /// A reply block, whole: it replaces what the deltas since the last `Text` built.
    Text {
        text: Untrusted,
    },
    /// `parent`: the sub-agent call this call runs under.
    ToolStart {
        id: String,
        parent: Option<String>,
        tool: String,
        station: Station,
        label: String,
        detail: Untrusted,
    },
    ToolEnd {
        id: String,
        failed: bool,
    },
    /// The CLI waits for the user. Only `Origin::User` answers it. `detail` is what is approved,
    /// from the request's payload: whole, or when `cut`, its first `MAX_DETAIL` bytes and
    /// `… [N more bytes]` (`len`: the whole payload's bytes; `ticket.full()` has all of it).
    /// `claim` is the model's own description of why: secondary, never what is approved.
    Permission {
        ticket: Ticket,
        tool: String,
        station: Station,
        label: String,
        detail: Untrusted,
        len: usize,
        cut: bool,
        claim: Untrusted,
    },
    /// A request denied without asking (`reason`, fixed words): too long to show, nothing to
    /// show, or Allow would grant more than this one action. `detail` as in `Permission`.
    AutoDenied {
        tool: String,
        label: String,
        reason: &'static str,
        detail: Untrusted,
    },
    /// The CLI no longer waits for that answer.
    PermissionClosed {
        ticket: Ticket,
    },
    /// Tokens in the context now, the context window and the session's cost so far, when known.
    Usage {
        context: Option<u64>,
        window: Option<u64>,
        cost_micro_usd: Option<u64>,
    },
    Compaction,
    TurnFinished {
        outcome: Outcome,
    },
    /// `kind` from a table. `retrying`: the CLI tries again by itself.
    Error {
        kind: String,
        retrying: bool,
    },
    /// A record no rule reads: its type if it is a name, else `unknown`.
    Unknown {
        kind: String,
    },
    /// The process ended. `stderr`: its last few KB.
    Exited {
        code: Option<i32>,
        stderr: Untrusted,
    },
}

/// Why an input was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

/// What one line of output gave: events, and lines the protocol must write back at once.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    pub events: Vec<Event>,
    pub replies: Vec<String>,
}

/// One runner's wire protocol. Pure: lines in, lines and events out.
pub trait Wire: Send + 'static {
    /// Lines to write as soon as the process runs.
    fn opening(&mut self) -> Vec<String>;
    /// One line of the process's stdout, without its newline.
    fn read(&mut self, line: &str) -> Output;
    /// Lines that carry `input` to the process. May be none yet (queued until the session is ready).
    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused>;
    /// True if a line too long to read, of which `head` is the start, may be a request the CLI
    /// waits on: the driver then ends the session rather than leave it waiting.
    fn awaits_answer(&self, head: &str) -> bool;
}

/// A running agent CLI: what the shell talks to. `driver::Driver` is the one implementation.
pub trait Process {
    fn send(&self, input: Input) -> io::Result<()>;
    /// Events, in order, ending with `Exited`.
    fn events(&self) -> &Receiver<Event>;
    /// Closes stdin: the CLI ends its session and exits.
    fn stop(&mut self);
    fn kill(&mut self) -> io::Result<()>;
}

/// How a session starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// A new session. Claude takes `id` as its session id; Codex names its own and ignores it.
    New {
        id: String,
    },
    Resume {
        id: String,
    },
}

/// A program and its arguments, each one element of argv. Never text for a shell to read, never a prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argv {
    pub program: &'static str,
    pub args: Vec<String>,
}

/// A runner's structured mode: what to run, and the wire to speak to it. `None`: the model is
/// not a name, the id not a UUID, or `cwd` not a plain absolute path (Codex).
pub fn structured(runner: RunnerId, cwd: &Path, model: Option<&str>, start: &Start) -> Option<(Argv, Box<dyn Wire>)> {
    match runner {
        RunnerId::Claude => Some((claude::argv(model, start)?, Box::new(claude::Claude::new()))),
        RunnerId::Codex => Some((codex::argv(), Box::new(codex::Codex::new(cwd, model, start)?))),
    }
}

/// A wire found by `structured` is driven as any other.
impl<W: Wire + ?Sized> Wire for Box<W> {
    fn opening(&mut self) -> Vec<String> {
        (**self).opening()
    }
    fn read(&mut self, line: &str) -> Output {
        (**self).read(line)
    }
    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused> {
        (**self).write(input)
    }
    fn awaits_answer(&self, head: &str) -> bool {
        (**self).awaits_answer(head)
    }
}

/// True if `id` is written as a UUID. Mirrors `agents::is_uuid` on the `task-start` branch (PR
/// #84): whichever merges second makes this call the core's.
pub fn is_uuid(id: &str) -> bool {
    let dash = |at: usize| matches!(at, 8 | 13 | 18 | 23);
    id.len() == 36 && id.bytes().enumerate().all(|(at, c)| if dash(at) { c == b'-' } else { c.is_ascii_hexdigit() })
}

/// Why a request was denied unseen.
const TOO_LONG: &str = "too long to show in full";

/// The shown part of a payload: all of it, or its first `MAX_DETAIL` bytes and `… [N more bytes]`.
fn excerpt(full: &str) -> (Untrusted, bool) {
    let shown = Untrusted::new(full, MAX_DETAIL);
    match shown.0.len() < full.len() {
        true => (Untrusted(format!("{}… [{} more bytes]", shown.0, full.len() - shown.0.len())), true),
        false => (shown, false),
    }
}

/// A permission request, from the payload it asks to approve (built only by `Payload::field`,
/// so every value is escaped) and the model's claim about it.
struct Ask<'a> {
    tool: String,
    station: Station,
    label: String,
    payload: payload::Payload,
    claim: &'a str,
}

impl Ask<'_> {
    /// `Permission`, or `AutoDenied` when the payload is longer than `MAX_APPROVABLE`: the
    /// caller then denies it.
    fn event(self, mut ticket: Ticket) -> Event {
        if self.payload.len() > MAX_APPROVABLE {
            return self.denied(TOO_LONG);
        }
        let full = self.payload.text();
        let (detail, cut) = excerpt(full);
        let len = full.len();
        ticket.full = Untrusted(full.to_string());
        let claim = Untrusted::new(&payload::escaped(self.claim), MAX_DETAIL);
        let (tool, label, station) = (self.tool, self.label, self.station);
        Event::Permission { ticket, tool, station, label, detail, len, cut, claim }
    }

    /// The request, denied without asking for `reason`; what it asked is still shown.
    fn denied(self, reason: &'static str) -> Event {
        let shown = Untrusted::new(self.payload.text(), MAX_APPROVABLE);
        let (detail, _) = excerpt(shown.text());
        Event::AutoDenied { tool: self.tool, label: self.label, reason, detail }
    }
}

/// An id made opaque by `ottospace-sessions`' hash: the same id pairs the same in both. `None` for none.
fn opaque(id: &Value) -> Option<String> {
    ottospace_sessions::opaque(id.as_str()?)
}

/// A record's type as a label: a name, else `unknown`.
fn kind_of(text: Option<&str>) -> String {
    ottospace_sessions::station::name(text.unwrap_or(""), "unknown")
}

/// A model id as a label: a name in the core's sense (no option-like or credential-like text).
fn model_of(value: &Value) -> Option<String> {
    let id = value.as_str()?;
    // `[1m]` marks a 1M-token window, as in `ottospace-sessions`: `claude-opus-5-5[1m]`.
    let bare = id.strip_suffix("[1m]").unwrap_or(id);
    (ottospace_core::agents::is_name(bare) && ottospace_sessions::station::is_name(bare)).then(|| id.to_string())
}

/// A session id as a label: a UUID, else `None`.
fn session_of(value: &Value) -> Option<String> {
    value.as_str().filter(|id| is_uuid(id)).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ottospace_core::runner::Status;

    #[test]
    fn every_runner_with_a_structured_mode_has_a_wire_and_its_program() {
        let start = Start::New { id: "8f14e45f-ceea-467f-a8d3-0242ac130003".into() };
        for id in RunnerId::ALL {
            let found = structured(id, Path::new("/code/app"), Some("m-1"), &start);
            assert_eq!(found.is_some(), id.spec().structured.status != Status::None, "{id:?}");
            let program = found.map(|(argv, _)| argv.program);
            assert!(program.is_none_or(|program| program == id.spec().terminal.program), "{id:?}");
        }
    }

    #[test]
    fn untrusted_text_is_cut_at_a_character_and_never_debug_printed() {
        let text = Untrusted::new("héllo", 2);
        assert_eq!(text.text(), "h");
        assert_eq!(format!("{:?}", Untrusted::new("secret", 100)), "Untrusted(6 bytes)");
    }

    #[test]
    fn images_are_known_by_their_first_bytes() {
        assert_eq!(Media::sniff(b"\x89PNG\r\n"), Some(Media::Png));
        assert_eq!(Media::sniff(b"RIFF1234WEBPVP8"), Some(Media::Webp));
        assert_eq!(Media::sniff(b"<svg"), None);
        assert_eq!(Media::sniff(b""), None);
    }

    #[test]
    fn models_may_say_their_1m_window() {
        let model = |id: &str| model_of(&serde_json::json!(id));
        assert_eq!(model("claude-opus-5-5[1m]").as_deref(), Some("claude-opus-5-5[1m]"));
        assert_eq!((model("[1m]"), model("<synthetic>"), model("--yolo")), (None, None, None));
    }

    #[test]
    fn uuids() {
        assert!(is_uuid("8f14e45f-ceea-467f-a8d3-0242ac130003"));
        for bad in ["", "--resume", "8f14e45f-ceea-467f-a8d3-0242ac13000", "8f14e45f-ceea-467f-a8d3-0242ac13000g"] {
            assert!(!is_uuid(bad), "{bad}");
        }
    }
}
