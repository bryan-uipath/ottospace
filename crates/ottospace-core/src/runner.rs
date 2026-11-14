//! The agent runners ottospace knows (Claude Code, Codex), and one description of each: what the
//! rest of the code asks of a runner instead of matching on its name. A closed set: a runner is
//! added here, never registered at run time. Pure data. Design: `docs/ai/agent-runners.md`.
//!
//! On the wire (task files, the socket, the settings) a runner is its `name`, a string, read
//! with `RunnerId::named`.

/// One runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RunnerId {
    Claude,
    Codex,
}

impl RunnerId {
    /// Every runner, in the order the `@` menu offers them.
    pub const ALL: [RunnerId; 2] = [RunnerId::Claude, RunnerId::Codex];

    /// The agent started when neither the task nor the settings name one.
    pub const DEFAULT: RunnerId = RunnerId::Claude;

    pub fn spec(self) -> &'static RunnerSpec {
        match self {
            RunnerId::Claude => &CLAUDE,
            RunnerId::Codex => &CODEX,
        }
    }

    /// As typed in an `@agent` tag and kept on the wire: `claude`.
    pub fn name(self) -> &'static str {
        self.spec().name
    }

    /// The runner of a name as kept on the wire: exactly its `name`, small letters.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.name() == name)
    }
}

/// Everything the core and the shell need to know of a runner.
#[derive(Debug, PartialEq)]
pub struct RunnerSpec {
    pub id: RunnerId,
    /// As typed in an `@agent` tag, and the start of a tag that names a model: `codex-gpt-6-astra`.
    pub name: &'static str,
    /// As a person reads it: `Claude Code`.
    pub display: &'static str,
    /// The models offered when the settings name none, as its CLI takes them.
    pub models: &'static [&'static str],
    /// Its model ids start with its name, so a tag is the id itself: `claude-opus-5-5`.
    pub ids_named: bool,
    pub terminal: Terminal,
    pub structured: Structured,
    pub disk: Disk,
    pub attach: Attach,
}

/// Its TUI in a terminal pane (stage 1). Read from the CLI's `--help`.
#[derive(Debug, PartialEq)]
pub struct Terminal {
    pub program: &'static str,
    /// Before the model's id.
    pub model_flag: &'static str,
    pub session: SessionId,
    /// Before the id of a session to go on with: a flag (`--resume`) or a subcommand (`resume`).
    pub resume: &'static str,
}

/// How a new session's id is known.
#[derive(Debug, PartialEq)]
pub enum SessionId {
    /// We choose it and pass it after `flag`: the task's session is known from the start.
    Chosen { flag: &'static str },
    /// The CLI names its session itself: it is found afterwards.
    Found { how: Finding },
}

/// How a session the CLI named is found.
#[derive(Debug, PartialEq)]
pub enum Finding {
    /// The one new session in the agent's folder after it started (`agents::find_sessions`).
    NewInFolder,
}

/// Its structured mode, driven by `ottospace-agents` (stage 2, not wired into the app yet).
#[derive(Debug, PartialEq)]
pub struct Structured {
    pub status: Status,
    /// The answers a permission request may get. Never a session grant: see the design note.
    pub answers: &'static [Reply],
    /// A running turn can be stopped with the session kept.
    pub interrupt: bool,
}

/// How far a structured mode is known to work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Documented, and its approval and interrupt paths seen in a probe of the real CLI.
    Verified,
    /// The CLI calls it experimental; probed in part.
    Experimental,
    /// No structured mode.
    None,
}

/// An answer to a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// This one request, exactly as displayed.
    AllowOnce,
    Deny,
}

/// What its session files on disk say.
#[derive(Debug, PartialEq)]
pub struct Disk {
    /// It records which live process holds a session: `sessions::Found::held` can be true.
    pub held: bool,
    /// A session's file records the folder it started in.
    pub cwd: bool,
}

/// What can be done with a session started elsewhere. Stated even where the answer is no or
/// not known, so the UI asks instead of assuming. Only `take_over_when_idle` is built.
#[derive(Debug, PartialEq)]
pub struct Attach {
    /// Its transcript can be followed read-only, live, from the session file.
    pub watch: Support,
    /// An idle session can be gone on with here (`resume`).
    pub take_over_when_idle: Support,
    pub attach_live: AttachLive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Yes,
    /// Thought to work, never seen to.
    Unverified,
    No,
}

/// How a session another program is running could be joined while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachLive {
    No,
    /// Only in the terminal that runs it: a pane of ours if we started it.
    ViaOwnTerminal,
    /// Through the runner's shared daemon.
    ViaDaemon {
        verified: bool,
    },
}

impl RunnerSpec {
    /// True if a new session's id is chosen here: only then is the task's session known from the start.
    pub fn chooses_session(&self) -> bool {
        matches!(self.terminal.session, SessionId::Chosen { .. })
    }
}

const CLAUDE: RunnerSpec = RunnerSpec {
    id: RunnerId::Claude,
    name: "claude",
    display: "Claude Code",
    models: &["claude-fable-5-2", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
    ids_named: true,
    terminal: Terminal {
        program: "claude",
        model_flag: "--model",
        session: SessionId::Chosen { flag: "--session-id" },
        resume: "--resume",
    },
    structured: Structured { status: Status::Verified, answers: &[Reply::AllowOnce, Reply::Deny], interrupt: true },
    // `sessions/<pid>.json` names the process; the file's records carry `cwd`.
    disk: Disk { held: true, cwd: true },
    attach: Attach {
        watch: Support::Yes,
        take_over_when_idle: Support::Yes,
        // Probe 2026-11-13: `--resume` of a live interactive or `-p` session is not refused, it
        // branches the same file. Only a `--bg` session is joined, by `claude attach <id>` (a TUI).
        attach_live: AttachLive::ViaOwnTerminal,
    },
};

const CODEX: RunnerSpec = RunnerSpec {
    id: RunnerId::Codex,
    name: "codex",
    display: "Codex",
    models: &["gpt-6-astra"],
    ids_named: false,
    terminal: Terminal {
        program: "codex",
        model_flag: "--model",
        session: SessionId::Found { how: Finding::NewInFolder },
        resume: "resume",
    },
    structured: Structured { status: Status::Experimental, answers: &[Reply::AllowOnce, Reply::Deny], interrupt: true },
    // No process is recorded; `session_meta` carries `cwd`.
    disk: Disk { held: false, cwd: true },
    attach: Attach {
        // Probe 2026-11-13: the rollout grows per item; a call's start is on disk before it ends.
        watch: Support::Yes,
        // Once the holder is gone. A daemon-held thread stays refused to a private `app-server` for
        // minutes after its clients leave; the TUI's `codex resume` goes through the daemon.
        take_over_when_idle: Support::Yes,
        // Probe 2026-11-13: a second client of the shared daemon (`codex app-server proxy`) resumed a
        // running thread and got its live events. A private `codex app-server` refuses it.
        attach_live: AttachLive::ViaDaemon { verified: true },
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_runner_is_its_spec_and_its_name() {
        for id in RunnerId::ALL {
            assert_eq!((id.spec().id, RunnerId::named(id.name())), (id, Some(id)));
            assert_eq!(id.spec().terminal.program, id.name(), "the program is the tag's name");
            assert!(id.spec().models.iter().all(|model| crate::agents::is_name(model)));
        }
        assert_eq!(RunnerId::ALL.map(RunnerId::name), ["claude", "codex"]);
        for other in ["Claude", "CODEX", "gemini", "", "claude "] {
            assert_eq!(RunnerId::named(other), None, "{other:?}");
        }
    }

    /// The table in `docs/ai/agent-runners.md`, written from the specs: every capability of every
    /// runner is stated there, and changes with the code.
    #[test]
    fn the_design_note_states_every_capability_of_every_runner() {
        let row = |what: &str, of: &dyn Fn(&RunnerSpec) -> String| {
            let cells: Vec<String> = RunnerId::ALL.iter().map(|id| of(id.spec())).collect();
            format!("| {what} | {} |\n", cells.join(" | "))
        };
        let session = |spec: &RunnerSpec| match &spec.terminal.session {
            SessionId::Chosen { flag } => format!("chosen (`{flag}`)"),
            SessionId::Found { how: Finding::NewInFolder } => "found: new in its folder".into(),
        };
        let answers = |spec: &RunnerSpec| format!("{:?}", spec.structured.answers);
        let mut table = row("", &|spec| spec.display.into());
        table += &row("---", &|_| "---".into());
        table += &row("name", &|spec| format!("`{}`", spec.name));
        table += &row("model ids carry the name", &|spec| spec.ids_named.to_string());
        table += &row("session id at start", &session);
        table += &row("resume", &|spec| format!("`{} {} <id>`", spec.terminal.program, spec.terminal.resume));
        table += &row("structured mode", &|spec| format!("{:?}", spec.structured.status));
        table += &row("permission answers", &answers);
        table += &row("interrupt", &|spec| spec.structured.interrupt.to_string());
        table += &row("held (live holder on disk)", &|spec| spec.disk.held.to_string());
        table += &row("cwd on disk", &|spec| spec.disk.cwd.to_string());
        table += &row("watch", &|spec| format!("{:?}", spec.attach.watch));
        table += &row("take over when idle", &|spec| format!("{:?}", spec.attach.take_over_when_idle));
        table += &row("attach live", &|spec| format!("{:?}", spec.attach.attach_live));
        let note = include_str!("../../../docs/ai/agent-runners.md");
        assert!(note.contains(&table), "docs/ai/agent-runners.md must hold:\n{table}");
    }
}
