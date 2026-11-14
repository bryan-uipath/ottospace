# Agent runners: a native conversation in the drawer

Stage 1 (`task-conversation.md`, PR #84) puts the agent's own TUI in a docked terminal on the
right. The user asks for more: "a non-terminal" conversation, "easy to copy and paste,
potentially visualize images, make links clickable, render markdown or code snippets properly",
an activity view, and "a Rust abstraction over agent runners". This note picks how, and
`crates/ottospace-agents` is the non-UI part of it. Stages 2a and 2b are built for Claude Code: see
"Native view: what is built".

Written 2026-11-13 against Claude Code 2.1.323 and codex-cli 0.166.0 on macOS.

## Decision

**Drive each CLI's structured mode ourselves and render natively (option a). Keep the terminal
as a tab for what only the TUI does (option c's fallback).**

| | Claude Code | Codex |
|---|---|---|
| Process | `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio` and `--session-id <uuid>` or `--resume <uuid>` | `codex app-server` (stdio, JSON-RPC, experimental) |
| Messages in | a `user` line on stdin | `turn/start` |
| Text out | `stream_event` text deltas, then the whole `assistant` message | `item/agentMessage/delta`, then `item/completed` |
| Approvals | `control_request` `can_use_tool` → our `control_response` | server requests `item/commandExecution/requestApproval`, `item/fileChange/requestApproval` → our response |
| Interrupt | `control_request` `interrupt` | `turn/interrupt` |
| Resume | `--resume <uuid>` | `thread/resume` |

Not `codex exec --json`: it has no approval channel. Its docs offer a read-only sandbox,
`--sandbox workspace-write` or `--approve-for-me` (automatic review). The drawer's crux is that
the user answers, so Codex goes through the app server. `exec --json` stays a fallback for a
read-only run if the app server proves too unstable.

### The options

| | (a) Structured mode, native view | (b) Hidden TUI in a PTY, mirror the session file | (c) Hybrid |
|---|---|---|---|
| Send a message | a JSON line; text and images exact | type into the PTY: bracketed paste, races the TUI's own editor | (a) |
| Streaming text | token deltas | the file has whole messages only, written late | (a) |
| Tool calls, results | in the stream, with ids | in the file (`ottospace-sessions` reads them) | both |
| **Permission prompts** | a request we answer; the drawer draws it | drawn by the TUI in a hidden terminal: we would have to screen-scrape it and type keys | (a) |
| Interrupt | a control message, the session stays | Esc into the PTY | (a) |
| Resume | flag or request | the TUI's flag | (a) |
| Images | base64 (Claude), a path (Codex) | paste or a path in text | (a) |
| Sub-agents | Claude: `parent_tool_use_id`; Codex: other threads' items | session file and sub-agent files | (a), plus files for history |
| Slash commands | Claude: skills and `/model` etc. work in `-p`, TUI-only ones (`/login`, `/config` UI, `/resume` picker) do not. Codex: no slash commands in the protocol | all | terminal tab |
| Cost, context | Claude: `result.total_cost_usd`, `modelUsage[*].contextWindow`, per-message `usage`. Codex: `thread/tokenUsage/updated` with `modelContextWindow` | from the file, window estimated for Claude | (a) |
| Breaks when | the stream format changes (documented for Claude; experimental for Codex) | the TUI's drawing changes (not an interface at all) | |

(b) fails at the crux: answering a permission prompt means reading a hidden TUI's screen. It
is a worse terminal, not a better conversation.

**When the drawer falls back to the terminal tab:** the user asks for it (a "Terminal" toggle
in the drawer header), a TUI-only command (`/login`, interactive pickers, `/config` UI,
plugin/MCP setup flows), a request the native client refuses (Codex
`item/tool/requestUserInput`, MCP elicitation, auth refresh; Claude control requests other than
`can_use_tool`), or the structured process exits with a startup error. The fallback runs the
stage 1 command with `--resume <id>`: the same session, in the TUI. Never both at once on one
session: the drawer stops its structured process first.

### Who answers a permission, and what an answer grants

- Only the user answers: `Origin::User` in the core, as stage 1 does for `TaskTalk`. A socket
  client's request to answer is refused (`agents::USERS_ALONE`), and `capture_start`'s
  test-only gate is the precedent for an E2E-only way in. A `Ticket` is made only by a parser,
  so an answer always names a request the CLI really asked; the shell maps it to an index.
- **An answer is Allow (this one request, exactly as displayed) or Deny (with an optional
  message to the agent). Nothing else: no "always", no session rule, no policy or amendment.**
  Claude: `allow` with the input sent back as it came, or `deny`; never `updatedPermissions`,
  and the CLI's `permission_suggestions` are never read. Codex: `accept` or `decline`; never
  `acceptForSession`, `acceptWithExecpolicyAmendment` or `applyNetworkPolicyAmendment`.
- **What the user approves is derived from the request's payload, never from text the model
  describes it with. What cannot be shown in full is not approvable in one step. A grant is
  never wider than what was displayed.**
  - The payload is one line per field, `label: value`, built only by `Payload::field`
    (`payload.rs`), which escapes every value: `\` `\n` `\r` `\t` written out, other control
    characters, Unicode line separators and bidirectional controls as `\u{…}`. So no value can
    forge a line or hide text (`rm x\u{202e}gnp.y`). The labels are fixed words. A test asserts
    the runners never write a payload any other way; property tests assert that for any strings
    the payload has exactly one line per field, each starting with its label.
  - Claude Bash/PowerShell: `command:`, then `option: key = value` for every input field but
    `description`; any other Claude tool: `input:` its whole input as JSON (a `Write`'s content
    included); then `blocked path:` if the request has one. A `blocked_path` that is not text is
    denied. Codex command: `command:`, `cwd:`, then `field: key = value` for every other field
    of the request but its ids (`approvalId`, `itemId`, `threadId`, `turnId`, `startedAtMs`)
    and `reason`, `unknown field: …` for one not in the schema. Codex file change: a line per
    change, `update|add|delete|change: <path>` with a move's destination (`-> <path>`) and
    `[outside the working directory]`; `grant root:` if asked; the other fields; then a `diff:`
    line per change (kept from `item/started` and `item/fileChange/patchUpdated`).
  - Allowed only when Allow provably grants this one action, per the protocol schema. Codex
    `accept` is "User approved the command" / "the file changes"; proposed amendments apply
    only through decisions we never send, so a request that merely proposes one may be allowed
    (and shows it). Declined, shown, with a reason pointing to the terminal view
    (`Event::AutoDenied`): a field the client does not know (not in the 0.166 schema); a
    `networkApprovalContext` (a managed-network approval may let the host in past this one
    command); a `grantRoot` (writes under a root for the rest of the session).
  - `Event::Permission.detail` is the payload whole, or its first 4 KB and `… [N more bytes]`
    with `cut: true`; `ticket.full()` holds all of it. **UI rule: no one-click Allow on a cut
    request.** The drawer shows the full text (scrollable) first; Allow is enabled after it.
  - Over 1 MB (`MAX_APPROVABLE`) a request is denied (`Event::AutoDenied`, its start shown), as
    is a Codex approval with no command or for changes never announced.
  - The model's description (Claude's `description`, Codex's `reason`) is `claim`: shown apart,
    smaller, labelled as the agent's words, never as what is approved.
- The drawer shows the request in the transcript where the call is: tool, label, the payload as
  untrusted text, the claim, and two answers: **Allow** (↵) and **Deny** (⎋, with an optional
  message). Stopping the turn is the separate Stop. A request the CLI withdraws
  (`control_cancel_request`, `serverRequest/resolved`) disappears. Granting more than one call
  stays a TUI or settings act until the stage below.
- Neither CLI times out a prompt: the task shows "Needs you" while one waits.
- The permission mode and approval policy are the user's own settings, as in the TUI. ottospace
  does not pass `--dangerously-skip-permissions`, `--permission-mode` or `approvalPolicy`.

### Later: session grants ("always allow")

Not built, on purpose: three review rounds found a grant wider than what was shown (forwarded
CLI suggestions that could `setMode` or add directories; file rules that are gitignore patterns
covering a directory and every edit tool; host rules covering every URL; a Codex
`acceptForSession` summary built beside the payload that understated a `grantRoot` or a path
with a newline, and an exec "always" labelled with the command that granted network access).
A later stage may add them only under these requirements:

- A grant is never wider than what was displayed.
- The grant's text is generated from the exact payload sent back (the rule or decision
  itself), never from a parallel summary.
- Runner-proposed rules and policies (Claude's `permission_suggestions`, Codex's
  `proposedExecpolicyAmendment`, `proposedNetworkPolicyAmendments`) are never forwarded.
- Network/host and directory-root grants each need their own explicit prompt, never folded
  into a command or file approval.
- Each grant kind proven exact from the runner's documented rule grammar, with a test that a
  near-miss (a directory, a pattern character, an alias, a wrapper) is refused.

### The session file still matters

Both CLIs still write their session files in these modes. `ottospace-sessions` stays the source
for history (a session started elsewhere, or before ottospace), the activity view's counts
after a restart, and sub-agents' own calls. The live stream is the source for the drawer while
ottospace runs the process. One rule: a live session's view folds stream events; on reopen it
folds the file, then follows the stream.

## One description per runner (`ottospace-core::runner`)

The runners are a closed enum, `RunnerId { Claude, Codex }`, and each has one `RunnerSpec`: a
`const` struct of plain data. Code asks the spec, never the name: no `match "claude"` outside
the spec. On the wire (task files, the socket, the settings) a runner stays its name, a string,
read with `RunnerId::named` where a decision needs it.

- **Identity.** `name` (the `@` tag and the wire name), `display`, the default `models`, and
  `ids_named` (Claude's model ids start with its name, so `claude-opus-5-5` is tag and id both).
- **Terminal.** `program`, `model_flag`, `resume` (a flag or a subcommand), and `session`:
  `Chosen { flag }` (we pick the id: `--session-id`) or `Found { how: NewInFolder }` (the CLI
  names it; `agents::find_sessions` finds it). `chooses_session()` asks which.
- **Structured.** `status` (`Verified | Experimental | None`), `answers` (`AllowOnce`, `Deny`;
  never a session grant), `interrupt`.
- **Disk.** `held` (the files name the live process) and `cwd` (the files record where it started).
- **Attach.** `watch`, `take_over_when_idle` (`Yes | No`) and `attach_live`
  (`No | ViaOwnTerminal | ViaDaemon`), stated even where the answer is no.

The id type lives in the core (pure). `ottospace-sessions` and `ottospace-agents` depend on the
core for it and are looked up by it: `ottospace_sessions::reader(id, &dirs)` gives a
`SessionReader`; `ottospace_agents::structured(id, ..)` gives the argv and a `Wire`.

The table, written by a test from the specs (`runner::tests`): every capability of every runner.

|  | Claude Code | Codex |
| --- | --- | --- |
| name | `claude` | `codex` |
| model ids carry the name | true | false |
| session id at start | chosen (`--session-id`) | found: new in its folder |
| resume | `claude --resume <id>` | `codex resume <id>` |
| structured mode | Verified | Experimental |
| permission answers | [AllowOnce, Deny] | [AllowOnce, Deny] |
| interrupt | true | true |
| held (live holder on disk) | true | false |
| cwd on disk | true | true |
| watch | Yes | Yes |
| take over when idle | Yes | Yes |
| attach live | ViaOwnTerminal | ViaDaemon |

What is verified, and how:

- Session ids, resume and model flags: `claude --help` (2.1.323) and `codex --help` (0.166.0),
  and the stage 1 terminal mode, which uses them (`task-conversation.md`).
- Structured mode: Claude's permission envelope, allow and interrupt seen in the real probe
  (below); Codex's app server is experimental by its own `--help`, and its command approval and
  interrupt were seen in the probe, and a file-change approval in the attach probes (below);
  `grantRoot` and `networkApprovalContext` were not.
- Held: Claude Code's `sessions/<pid>.json` (`sessions.md`); Codex records no process, though a
  held `thread-writer-locks/<id>.lock` refuses a `flock` (Probes, 1; not read yet).
- Take over when idle: both continue the same id once the holder is gone (Probes, 5). A Codex
  thread the daemon holds is refused to a private `codex app-server` for minutes after its
  clients leave; through the daemon (the TUI's `codex resume`) it is not.
- Watch: both files grow a record at a time while the session runs, and a tool call's start
  is on disk before its result (Probes, 3 and 4). Neither file has text deltas.
- Attach live: Claude: an interactive or `-p` session is live only in the terminal running it;
  `--resume` from elsewhere does not join it, it branches the same file (Probes, 4). Only a
  `--bg` session is joined, by `claude attach <id>`. Codex: a thread on the shared app-server
  daemon (where the TUI runs by default) is joined by a second daemon client, live (Probes, 1).

Where the UI asks the spec today: a new session's id (`chooses_session`: the task is linked at
once, or its session is sought), the sessions sought and read (`reader`), the held check
(`disk.held`), and a resume (`take_over_when_idle`). Seams not wired, on purpose:

- `attach.attach_live`: joining a running session (Claude's background attach, Codex's daemon).
- `structured.answers` and `interrupt`: the card always offers Allow (once) and Deny, and Stop; both
  runners' specs say so.
- `display`: the drawer header and rows still show `name`, as before.

Wired with the native view: `structured.status` (`Verified` → the drawer may draw it natively,
`Model::drawable`; Codex is `Experimental` and stays in a terminal), `attach.watch` (`Yes` → a
session at work elsewhere is watched read-only, `Model::watch_target`) and
`take_over_when_idle` (`Yes` → Take over once it is idle, `Model::take_over_ready`).

### Adding a third runner

1. A variant in `RunnerId`, in `ALL`, and its `RunnerSpec` const in `runner.rs`; every field
   stated, from its `--help`, with `No` where nothing was seen to work.
2. The per-runner fields of the settings: `settings::AgentModels`, `settings::SessionDirs`,
   `ottospace_sessions::ConfigDirs`, `agents::Models` (and `Models::of`), `Action::SetAgents`,
   and `config::session_dirs` for its default folder.
3. A `SessionReader` in `ottospace-sessions` and its arm in `reader`; fixtures for it.
4. If it has a structured mode: a `Wire` in `ottospace-agents` and its arm in `structured`.
5. The table above (the test prints the rows to paste), `sdk/src/types.ts` `SessionRunner`
   and `sdk/src/parse.ts`, and the guide's list of runners.

## The common model (`crates/ottospace-agents`)

```rust
pub trait Wire: Send + 'static {          // pure: lines in, lines and events out
    fn opening(&mut self) -> Vec<String>;
    fn read(&mut self, line: &str) -> Output;          // events + replies to write at once
    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused>;
    fn awaits_answer(&self, head: &str) -> bool;       // may an unreadable line be a request?
}
pub trait Process {                         // driver::Driver implements it
    fn send(&self, input: Input) -> io::Result<()>;
    fn events(&self) -> &Receiver<Event>;               // ends with Exited
    fn stop(&mut self);                                  // close stdin
    fn kill(&mut self) -> io::Result<()>;
}
enum Input { Send { text, images }, Answer { ticket, answer }, Interrupt }
enum Answer { Allow, Deny { message } }                // this one request, or not
enum Event {
    Started { session, model, mode }, TurnStarted,
    TextDelta { text: Untrusted }, Text { text: Untrusted },
    ToolStart { id, parent, tool, station, label, detail: Untrusted }, ToolEnd { id, failed },
    Permission { ticket, tool, station, label, detail: Untrusted, len, cut, claim: Untrusted },
    PermissionClosed { ticket }, AutoDenied { tool, label, reason, detail: Untrusted },
    Usage { context, window, cost_micro_usd }, Compaction,
    TurnFinished { outcome }, Error { kind, retrying }, Unknown { kind },
    Exited { code, stderr: Untrusted },
}
// By runner id: `claude -p …` or `codex app-server`, and its wire.
fn structured(runner: RunnerId, cwd: &Path, model: Option<&str>, start: &Start) -> Option<(Argv, Box<dyn Wire>)>;
```

- `claude::argv(model, start)` and `codex::argv()` are fixed tables. A model id passes the core's
  `agents::is_name`, a session id is a UUID (`is_uuid`, mirrored from `task-start`'s
  `agents.rs`: whichever PR merges second calls the core's). The prompt is never in argv.
- Text deltas and `Text`: deltas build the live block; `Text` is that block whole and replaces
  it. Both runners fit this without ids.
- Labels reuse `ottospace-sessions`: `claude::describe` (now `pub`) and `station::*`. A command's
  label comes from the program tables; a path shows its last two parts; ids are FNV hashes.
- A control or server request nobody will answer gets an error reply at once, so a CLI never
  waits on a question the drawer cannot show.
- Lines are capped at 32 MB (Claude's tool results carry images as base64); a longer one is
  dropped as it streams and reported as `Unknown { kind: "oversized" }`. If its first 1 KB says
  it may be a request the CLI waits on (`awaits_answer`), it cannot be answered, so the driver
  kills the session with `Error { kind: "oversized_request" }` rather than leave it waiting.
- The driver: piped stdio, the CLI in a process group of its own. One thread reads stdout
  (→ protocol → events, replies), one writes stdin from a queue (a CLI that stops reading
  stdin never blocks a send or a reply), one keeps stderr's last 4 KB, one waits for the exit
  (`waitid` with `WNOWAIT`), kills what the CLI left in its group (a background command, an MCP
  server) while the group's id is still held, then reaps it. `kill` and dropping a driver kill
  the whole group. No clock decisions: the one sleep gives stderr up to half a second to finish.
- Turns: Claude names no turn start, so its first output after a `result` is `TurnStarted`. An
  interrupt with no turn running sends nothing; a Codex interrupt before the turn has an id is
  sent when the id comes. A Codex thread or turn that fails to start ends as a failed turn.

Tests: 32 unit (incl. the approval rules: an answer grants this call alone, a blocked path
shown and an unreadable one denied, one escaped line per field for any strings, cut and
auto-deny, every changed path, move and escaped newline, every exec field, and the declines of
an unknown field, a network approval and a `grantRoot`), 6 driver tests against fake
`claude` and `codex` scripts (`tests/fakes`) that replay hand-written fixtures and record argv
and stdin (a turn with a permission asked and allowed, a turn interrupted, stdin closed; a
background command ended with its CLI; an unreadable request; a send to a CLI that reads
nothing), and 4 property tests (every fixture value swapped
for 15 other values and types, every line cut at every character, random bytes; a sentinel never
reaches a label or `Debug`). Two ignored probes of the real CLIs (`tests/real.rs`, below).

## Trust rules

Everything in the stream is model- or tool-written, including what looks like the CLI's own
words (a tool result, a file the model read, a web page it fetched). The four that sum them up,
each held by code named here:

- **A grant is never wider than what was displayed.** The card shows the payload `Payload::field`
  built, and Allow sends that one request back as it came (`Answer::Allow`). A request cut at
  4 KB cannot be allowed until it was shown whole (`Card::allowable`, `TalkExpand`); past 1 MB it
  is denied unseen. No "always", no session rule.
- **Nothing is fetched.** No `img()` of model text: a web image is a link, a local one a thumbnail
  read by us (`conversation::read_thumb`). The watch view reads only the session's own file.
- **Nothing opens without the user's click.** `links::target` is the one judge; the shell opens
  only what it gave, only for `Origin::User` (`open_link`), and checks a file again with its links
  resolved. A test run opens nothing (`drawer.native.opened` records it).
- **Only the user answers.** Sends, answers, Show all, the Deny's message, Esc, interrupts and the
  Chat | Terminal switch are `Origin::User` only (`chat::USERS_ALONE`), and only for the
  conversation shown; the composer's text too (`FieldEdit` on `talk`). `AgentEvents` comes from
  the shell's runner (`System`); a client's is refused (`chat::RUNNERS_ALONE`). The property test
  checks each.

1. **Untrusted is a type.** Text, command lines, paths, tool input and stderr are `Untrusted`:
   no `Serialize`, and `Debug` prints a length. Only labels (`tool`, `label`, `station`, `kind`,
   `model`, `session`) may reach the control socket's state, by `sessions.md`'s rules, and the
   guide marks the name-shaped ones as untrusted data. Transcript text never goes into the
   socket's state; an agent reading the socket sees that a call ran, not what it said.
2. **The markdown renderer** (stage 2a) builds GPUI elements from a parsed tree, never HTML.
   - No remote fetch, ever: an image whose source is a URL is drawn as a link, not loaded
     (an image URL is an exfiltration channel: the model can encode secrets into it). GPUI's
     `img()` fetches any string that parses as a URI, so the renderer never passes it model text:
     only `ImageSource::Image` from bytes we read.
   - Local images only from the task's repository or from what the user attached, read by us
     (`Image::load`: regular file, 20 MB, sniffed PNG/JPEG/GIF/WebP; no SVG).
   - Links: shown with their real target on hover; opened only on the user's click; only
     `https` and `http` (scheme checked case-insensitively, as written and after
     percent-decoding; a plain host, no user name; no control, space or bidi character),
     everything else drawn as text (`mailto` too, for now). A link's text that looks like a URL
     but differs from its target shows both.
   - A link to a path becomes a link only inside the task's repository (`..` resolved, then
     links resolved again before the click acts); a click reveals it in Finder (`reveal_path`),
     never opens or runs it. Bare paths in text are not made links.
   - No HTML blocks or inline HTML rendered: shown as text. No autolinking of bare hosts.
   - Code blocks: monospace, a copy button, no execution affordance. Terminal escapes in text
     are stripped before drawing.
3. **Answers and sends are the user's.** Permission answers, interrupts, stop, and the message
   text come only from `Origin::User`. A socket client may read a conversation's labels; whether
   it may ever send a message is a separate trust decision (as `type` is today) and not in this
   plan.
4. **Argv.** A fixed table plus a name-shaped model and a UUID. The shell runs it through the
   login shell for `PATH` as stage 1 does (`"$SHELL" -lic 'exec "$@"' -- claude …`).
5. **Data exposure.** stderr may hold tokens: it is `Untrusted`, kept in the driver, never
   logged, never in the core or the state (not shown in the drawer yet either).
   Copy of a message or block goes to the clipboard, never to the state's `copied` (a test
   run's clipboard is `copied` itself). Images the user attaches are read once and sent; nothing
   is cached on disk by ottospace.
6. **Prompt injection** cannot be prevented here; the defence is the approval step being a real
   human act with the real command shown, and nothing auto-approved by ottospace.

## Native view: what is built

- **Core** (`transcript.rs`, `model/chat.rs`, `links.rs`): a transcript per conversation folded
  from the runner's events (user messages, streamed text replaced by the whole block, tool rows
  with label and state, sub-agent calls marked, auto-denials, notices, usage), capped at 400
  entries or 4 MB (256 KB an entry); the oldest are counted and dropped from memory, the session
  file keeps them; scroll-back does not read them again. A draft, a Deny's message and Esc's
  state per conversation, so the conversation shown can change at any time. Cards queue in
  order and are answered once. Task state follows the stream: Running while a turn runs, Needs
  you while a card waits or once it ends; the rail's live line is the stream's current call.
  The view (`Model::talk_view`): the user's pick, else `conversation_view` where the spec's
  structured mode is verified, else Terminal; kept in `layout.json` (`drawer.views`).
- **Shell** (`conversation.rs`, `markdown.rs`): `structured(...)` from the spec, through the
  login shell, the first prompt on stdin; events translated, tickets numbered; markdown blocks
  (pulldown-cmark 0.13.6, no `html` feature) drawn with `StyledText`/`InteractiveText`; code
  blocks with Copy; Copy and Copy as Markdown per message; a working row; the card; a multi-line
  composer (`field.rs`, up to 8 lines). Keys (`chat::key`): on a card ↵ does nothing, `a` `d`
  `s` `m`; in the composer ↵ sends (or denies with the message), ⇧↵ a new line, Esc leaves, Esc
  Esc interrupts. While a turn runs ↵ sends nothing (the draft waits).
- **Switch.** Chat → Terminal is refused while a turn runs or a card waits; Terminal → Chat at
  any time (the TUI is hung up on, as Stop). The core marks the switch and stops the agent; the
  shell kills the native group or hangs up on the TUI (SIGKILL after 3 s) and says
  `AgentExited` only once the process is gone; then the core resumes the session in the other
  view. Never two processes on one session.
- **Watch.** A session at work or held elsewhere, whose spec says `watch: Yes`, is followed from
  its file (`ottospace-sessions`, read each second): its calls, sub-agents, errors and a notice
  per message, never its text; no turn, no card. Take over, when `take_over_when_idle` and it is
  idle, is the user's open of it (a resume).
- **Not built:** Codex native (its `Wire` exists; the drawer keeps it in a terminal until its
  app server is probed), images attached from the composer (the runner takes them; no picker
  yet), stderr on request, syntax colouring, drag selection, rich tool results (diffs), the
  radial activity view (2c), mailto links, bare paths as links.
- **Unverified against the real CLI:** that `claude -p --session-id` writes the session file
  as the TUI does (the docs' `--no-session-persistence`, "only works with --print", implies it
  does; the driver tests use a fake), so time and tokens per model, the resume after a switch
  and the watch view depend on it; the shape of `permissionMode` in `system/init` beyond the
  probe's `default`/`auto`.

### The stages as planned

- **2a. Transcript and composer.** Markdown via `pulldown-cmark` (pure Rust, no I/O; already in
  the local cargo registry as 0.13.x, not yet a dependency) into GPUI elements. GPUI 0.2.2 (the
  repo's version) has `StyledText` (runs of styles), `InteractiveText` (click and hover on
  character ranges: links), `img` with `ImageSource`, `list`/`uniform_list` for long transcripts,
  and `App::write_to_clipboard` / `App::open_url`. It has **no markdown element and no text
  selection** on `StyledText`: Zed's `markdown` crate is not published on crates.io. So: copy
  buttons per message and per code block first; drag-selection is its own piece of work later.
  Syntax colouring: none at first (monospace), a later choice.
- **2b. Permissions and quick answers.** The request card in the transcript, keys for the four
  answers, "Needs you" on the task while one waits; the core gets `Action::AgentEvent` (labels
  only) and `Effect::AnswerPermission`, both from `Origin::User` only.
- **2c. Activity view.** The radial view from the same events: `station` per call, sub-agents
  (`parent`), context ring from `Usage { context, window }`. The same fold as
  `ottospace-sessions`'s `Snapshot`, fed by stream events while live.

## Verified and UNVERIFIED

Sources: `claude --help` (2.1.323); code.claude.com/docs `headless`, `cli-reference`,
`agent-sdk/typescript` (message types, `CanUseTool`, `PermissionResult`, `PermissionUpdate`,
`ModelUsage`), `permissions` (rule grammar: exact Bash rules, gitignore file rules, `Edit` covering
every edit tool, `domain:` rules); `codex exec --help`, `codex exec resume --help`, `codex app-server --help`, and
`codex app-server generate-json-schema --out <dir>` (0.166.0, offline); learn.chatgpt.com/docs
`non-interactive-mode` (exec's JSONL events).

Verified from those sources: every flag above exists; stream-json output needs `--verbose`;
the shapes of `system/init`, `stream_event`, `assistant`, `user`, `result` (incl.
`total_cost_usd`, `modelUsage[*].contextWindow`), `system/api_retry` error codes; every Codex
request, notification and approval shape and decision value used (from the generated schema).

Verified by the real probes (`tests/real.rs`, run 2026-11-13 with the user's approval, Claude
Code 2.1.323 on `haiku`, codex-cli 0.166.0 on the user's default model, each in a new empty
folder; they print protocol names and key names, never text):

| What | Seen |
|---|---|
| Claude's permission envelope | stdout `{type: control_request, request_id, request: {subtype: can_use_tool, tool_name, input, tool_use_id, permission_suggestions, description, display_name, blocked_path}}`; our `{type: control_response, response: {subtype: success, request_id, response: {behavior: allow, updatedInput}}}` is accepted: the file did not exist when the request came and existed after Allow |
| `--permission-prompt-tool stdio` | accepted by 2.1.323: prompts come to us as above |
| Why the first Claude run asked nothing | the user's settings set the permission mode to `auto` (`system/init.permissionMode`): the classifier approved the `touch`. With `--setting-sources project` (probe only: the user's settings not loaded, in an empty folder) the mode is `default` and the request came. ottospace keeps the user's mode: the drawer sees only what falls to a prompt |
| `system/init` | sent after each user message, after any `SessionStart` hook events: `Started` comes once per turn |
| Claude interrupt | our `control_request {subtype: interrupt}` is answered `control_response {subtype: success}`; the turn ends `result {subtype: error_during_execution, is_error: true, stop_reason: null, terminal_reason: aborted_streaming}` with no text after. The parser now reads `terminal_reason` (`aborted*`); its own flag counts only when the turn did not succeed |
| Codex order and JSON-RPC | `initialize` → result → `initialized` + `thread/start` → result works without `"jsonrpc"`; in a folder that is not a git repository |
| Codex approval | asked under the user's config. `item/started` (commandExecution) comes before `item/commandExecution/requestApproval`, whose params carry `availableDecisions` (not in the plain schema; in `--experimental`'s: "decisions the client may present") — that alone made the first run decline. It grants nothing, so it is now known; Allow is offered only when it lists `accept`. `additionalPermissions` (also experimental) declines as wider. After Allow: `serverRequest/resolved`, the item completes, the file exists |
| Codex interrupt | `turn/interrupt` → `turn/completed` with `turn.status: interrupted` |

Lines no rule reads (`Unknown`), and what becomes of them:

| Runner | Kind | Decision |
|---|---|---|
| Claude | `system.hook_started`, `system.hook_response` (the user's hooks) | stay `Unknown`; later the activity view's Hooks station |
| Claude | `system.status` | stay `Unknown` (turn events say the same) |
| Claude | `rate_limit_event` | later an event: rate-limit state for the header (stage 2a) |
| Claude | `system.thinking_tokens` | later an event: a "thinking" indicator (stage 2a) |
| Codex | `thread.status.changed`, `account.updated`, `remoteControl.status.changed` | stay `Unknown` |
| Codex | `mcpServer.startupStatus.updated` | later an event: MCP servers on the activity view (stage 2c) |
| Codex | `account.rateLimits.updated` | later an event, with Claude's rate limits |

Still UNVERIFIED:

| What | Why it matters |
|---|---|
| Whether `updatedInput` is required on allow | We send the original input back, which works either way |
| Codex `grantRoot`, `networkApprovalContext` | Not triggered by any probe; declines are built from the schema |
| Which client a daemon thread's approval goes to when two are subscribed | Not triggered: the daemon probe ran no tool |
| The Claude `control_cancel_request` | Not seen; `PermissionClosed` relies on the SDK's shape |

The probes, each one short model session on the user's account (approval needed each time):

```
OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture claude_real
OTTOSPACE_PROBE_ISOLATED=1 OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture claude_real
OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture codex_real
```

## Probes: attach, watch and take-over (2026-11-13)

Run with the user's approval, each in a new `mktemp -d` folder (a `git init` with one file), on
sessions the probe made: Claude Code 2.1.325 (`--model haiku`), codex-cli 0.166.0 (the user's
default model, `effort: low`), whose shared daemon was app-server 0.169.0 (already running, the
user's; not stopped). Python drivers over the CLIs' stdio, a PTY for the TUIs. 14 model turns.
The Claude TUI runs were started without the `CLAUDE*` variables of the Claude Code session
running the probe: inherited, they turn session persistence off.

**1. Codex daemon and attach.** `codex agents` is a TUI browser of the daemon's sessions;
`codex queue --thread <id> --message <text> [--resume]` queues a message (neither run). The TUI
uses the daemon unless `--no-daemon`. The daemon's socket
(`~/.codex/app-server-control/app-server-control.sock`) speaks JSON-RPC over WebSocket;
`codex app-server proxy` pipes stdio to it (send an HTTP upgrade first: `101 Switching Protocols`).

| Run | Seen |
|---|---|
| Private `codex app-server` A holds thread T (idle); a second private server B: `thread/resume T` | `-32600 thread T already has an active writer`; B's `thread/loaded/list` lacks T |
| A killed (`SIGKILL`); B: `thread/resume T` | ok: `~/.codex/thread-writer-locks/T.lock` stays on disk but the lock is released. A file there is not "held"; a held one refuses a non-blocking `flock` (`EAGAIN`) |
| Daemon clients P1, P2 (`app-server proxy`). P1 `thread/start`, `turn/start`; P2 `thread/resume T` mid-turn | ok, `status: {type: active}`; P2 got the rest of P1's turn live (65 of its 79 deltas, then `turn/completed`) |
| P2 `turn/start` | ok; P1 got every event of P2's turn. Both clients saw `thread/status/changed` (active / idle) even before P2 resumed |
| Both `thread/unsubscribe` (`unsubscribed`), clients gone; a private server: `thread/resume T` | 3 s later: `already has an active writer` (the daemon keeps it loaded); about 12 minutes later: ok |

So: a session on the daemon can be watched and joined live by another daemon client; a session
in a private `codex app-server` (what `ottospace-agents` starts) cannot be joined by anyone, and a
resume in another server is refused, not forked (through the same daemon it joins). Live/held: `thread/resume` is refused; on disk, the lock
file's `flock`; on the daemon, `thread/loaded/list` and `thread/status/changed`.

**2. Codex file-change approval** (private server, `thread/start {sandbox: read-only,
approvalPolicy: on-request}`, prompt "create a.txt containing 1 with apply_patch"):

```
item/started   {item: {type: fileChange, id: "exec-<uuid>", changes: [{path: "<cwd>/a.txt",
                kind: {type: add}, diff: "1\n"}], status: inProgress, startedAtMs, completedAtMs: null},
                threadId, turnId, startedAtMs}
item/fileChange/requestApproval  id 0  {threadId, turnId, itemId: "exec-<uuid>", startedAtMs,
                reason: null, grantRoot: null}
← {"id":0,"result":{"decision":"accept"}}
serverRequest/resolved {requestId: 0}; item/completed {status: completed}; turn/diff/updated; a.txt written
```

No `availableDecisions` and no `item/fileChange/patchUpdated`. The schema's answers: `accept`,
`acceptForSession`, `decline`, `cancel`; ottospace sends only the first and third. It matches
`codex.rs`: every field is an id, `reason` or a null `grantRoot`, the change was announced by
`item/started`, so the request is offered. The real lines are now a unit test
(`a_real_file_change_approval_is_offered_and_accepted`).

**3. Codex watch.** The rollout (`thread.path`, `~/.codex/sessions/YYYY/MM/DD/rollout-*-<id>.jsonl`)
is created at the first turn and grows a record or a few at a time, within ~20 ms of the matching
event: `response_item/custom_tool_call` (the patch) was on disk 170 ms *before* `item/started` and
the approval, `custom_tool_call_output` after `item/completed`. Agent text is on disk as the whole
message at `item/completed`; deltas are never written. The first write is large (~175 KB of
instructions and tool lists).

**4. Claude.** `claude --help` (2.1.325): `--bg` ("Start the session in the background … With
--resume <session-id>, continues that session in the background under the same ID, or starts a
copy and says so when the session is already running"), `claude agents [--json] [--cwd <path>]`,
`claude attach <id>`, `logs`, `stop`, `rm`, `respawn`, `--fork-session`, `--remote-control`,
`--cloud`, `--teleport`.

| Run | Seen |
|---|---|
| A: `claude -p --input-format stream-json … --session-id U`, one turn, idle, alive | `sessions/<A pid>.json` with `sessionId: U`, `kind: interactive`, `status: idle`; `claude agents --json --cwd <dir>` lists it |
| B: `claude -p --resume U "Reply with the word two."` while A lives | not refused: exit 0, `session_id` U, appends to U's file |
| A's next turn: "the word you replied before this" | `one`, not `two`: A never saw B. In the file B's turn and A's both hang off A's first reply: a silent branch in one file |
| A exits; C: `claude -p --resume U` | same id, same file, continues A's branch (B's is left aside) |
| Interactive TUI A (PTY, `--session-id V`), idle; `-p --resume V` | as above: not refused, same id |
| A second TUI `claude --resume V` while A lives | opens normally with both turns; `claude agents` lists two `interactive` holders of V |
| `claude --bg --model haiku "Reply with the word one."` | ignores `--session-id` ("--bg manages the session id"), prints `backgrounded · <short id>`; starts `claude daemon run --origin transient`, which exited by itself once the sessions were stopped |
| `claude -p --resume <bg id>` while it runs | refused: `That session is running in the background (<id>). Run claude attach <id> to open it, or claude stop <id> first to resume it here. Add --fork-session to branch off a copy instead.` |
| `claude --bg --resume <bg id>` | `session <id> is already running in the background, so this started a copy as <new id>` |
| `claude attach <bg id>` (PTY) | the live session's TUI: transcript and composer |

Watch: the file grows a record at a time: the `tool_use` message ~1 s after the stream showed
it and before its `tool_result`; text as whole messages ~30 ms after the stream's. No deltas.

**5. Take over when idle.** Codex: after the holder exits (cleanly or killed) `thread/resume`
continues the same id and file. A thread the daemon holds stays refused to a private server for
a while after its clients leave: take it over through the daemon. Claude: after the holder exits
`--resume` continues the same id; while one lives, `--resume` (but of a `--bg` session) is *not*
refused and branches the file, so the `held` check (`sessions/<pid>.json`) must come first.

What watch mode can rely on:

| | Claude Code | Codex |
|---|---|---|
| Tail the file live | yes, a record at a time; a call's start before its end | yes, a record at a time; a call's start before its end |
| Text deltas | no (stream only) | no (stream only) |
| Live holder | `sessions/<pid>.json`, `claude agents --json --cwd` | lock file `flock`; daemon `thread/loaded/list`, `thread/status/changed` |
| Join live | only `--bg` sessions, by `claude attach` in a terminal | daemon sessions, by a daemon client; never a private `app-server`'s |
| Resume while held | not refused (branches the file) but for `--bg` (refused) | refused by another server (`active writer`); joined through the same daemon |

Left on disk: the probe's session files (Claude: `projects/<scratch>/…jsonl`; Codex: two rollouts
under `sessions/2026/11/13/`), the scratch folder's trust entry in Claude's `.claude.json`, and
the three stopped, removed `--bg` sessions' records (`claude rm` keeps the conversation).

## Open questions

1. **Codex through the experimental app server, or Claude-only native first?** Recommend:
   both behind the same trait, Claude first in the UI; Codex's native view ships once the probe
   passes, with the terminal tab as its fallback.
2. **Should a task started natively be resumable in the terminal tab, and vice versa?**
   Recommend yes, one at a time: the drawer stops one process before starting the other.
3. **Session grants ("always allow") in the drawer at all?** Recommend not until the later stage
   above, and then one grant kind at a time (an exact Bash command first), each with its own
   prompt; meanwhile the terminal view or the user's settings grant more.
4. **Set the permission mode / approval policy for drawer sessions**, or keep the user's? The
   probe found this user's Claude mode is `auto`, so most calls never reach the drawer.
   Recommend the user's own, shown in the drawer header (`auto`, `never`) so it is visible.
5. **May a socket client send a message to a task's agent?** Recommend no for now, as stage 1;
   revisit with the agent-to-agent work.
6. **Text selection.** Copy buttons first, or build drag-selection over `StyledText` before 2a
   ships? Recommend copy buttons first; selection as 2a's follow-up.
7. **Sub-agent text.** Claude's `--forward-subagent-text` would show sub-agents' prose too.
   Recommend off: the activity view shows their calls; their text stays in the files.
