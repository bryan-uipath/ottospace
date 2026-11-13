# Agent runners: a native conversation in the drawer

Stage 1 (`task-conversation.md`, PR #84) puts the agent's own TUI in a docked terminal on the
right. The user asks for more: "a non-terminal" conversation, "easy to copy and paste,
potentially visualize images, make links clickable, render markdown or code snippets properly",
an activity view, and "a Rust abstraction over agent runners". This note picks how, and
`crates/ottospace-agents` is the non-UI part of it. Nothing is wired into the app yet.

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

### Who answers a permission, and what "always allow" means

- Only the user answers: `Origin::User` in the core, as stage 1 does for `TaskTalk`. A socket
  client's request to answer is refused (`agents::USERS_ALONE`), and `capture_start`'s
  test-only gate is the precedent for an E2E-only way in. A `Ticket` is made only by a parser,
  so an answer always names a request the CLI really asked; the shell maps it to an index.
- **What the user approves is derived from the request's payload, never from text the model
  describes it with. What cannot be shown in full is not approvable in one step.**
  - The payload: Claude Bash/PowerShell, the command and every option but `description`
    (`run_in_background`, `dangerouslyDisableSandbox`, …); any other Claude tool, its whole
    input as JSON (a `Write`'s content included). Codex command, the command and its `cwd`;
    Codex file change, every changed path (those outside the working directory flagged), a
    `grantRoot` if asked, then every diff (kept from `item/started` and
    `item/fileChange/patchUpdated`).
  - Shown sanitised: control characters (but newline, tab) and bidirectional controls become
    `�`, so `rm x\u{202e}gnp.y` cannot read as something else.
  - `Event::Permission.detail` is the payload whole, or its first 4 KB and `… [N more bytes]`
    with `cut: true`; `ticket.full()` holds all of it. **UI rule: no one-click Allow on a cut
    request.** The drawer shows the full text (scrollable) first; Allow is enabled after it.
  - Over 1 MB (`MAX_APPROVABLE`) a request is denied unseen (`Event::AutoDenied`), as is a Codex
    approval with no command or for changes never announced.
  - The model's description (Claude's `description`, Codex's `reason`) is `claim`: shown apart,
    smaller, labelled as the agent's words, never as what is approved.
- The drawer shows the request in the transcript where the call is: tool, label, the payload as
  untrusted text, the claim, and four answers: **Allow** (↵), **Always this session**,
  **Deny**, **Deny and stop** (⎋). A request the CLI withdraws (`control_cancel_request`,
  `serverRequest/resolved`) disappears.
- "Always" is **this session only**, and offered only with what it grants shown
  (`Event::Permission.always`). Claude: the CLI's `permission_suggestions` are never forwarded
  (they could `setMode`, add directories or replace rules). We build one `addRules`/`allow`/
  `session` rule from exactly what was shown: `Bash(<the command>)`, `Edit(//<absolute path>)`,
  `WebFetch(domain:<host>)`. None when no exact rule can be: a pattern character (`* ? [ ] { }
  ! \`), a relative path or `..`, options beyond `timeout`, any other tool; then only Allow is
  offered and "always" is refused. Codex: `acceptForSession`, which the server scopes to the
  same command or the same files; `always` shows that command or those paths. Writing a lasting
  rule stays a TUI or settings act.
- Neither CLI times out a prompt: the task shows "Needs you" while one waits.
- The permission mode and approval policy are the user's own settings, as in the TUI. ottospace
  does not pass `--dangerously-skip-permissions`, `--permission-mode` or `approvalPolicy`.

### The session file still matters

Both CLIs still write their session files in these modes. `ottospace-sessions` stays the source
for history (a session started elsewhere, or before ottospace), the activity view's counts
after a restart, and sub-agents' own calls. The live stream is the source for the drawer while
ottospace runs the process. One rule: a live session's view folds stream events; on reopen it
folds the file, then follows the stream.

## The common model (`crates/ottospace-agents`)

```rust
pub trait Protocol: Send + 'static {      // pure: lines in, lines and events out
    fn opening(&mut self) -> Vec<String>;
    fn read(&mut self, line: &str) -> Output;          // events + replies to write at once
    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused>;
}
pub trait Runner {                          // driver::Driver implements it
    fn send(&self, input: Input) -> io::Result<()>;
    fn events(&self) -> &Receiver<Event>;               // ends with Exited
    fn stop(&mut self);                                  // close stdin
    fn kill(&mut self) -> io::Result<()>;
}
enum Input { Send { text, images }, Answer { ticket, answer }, Interrupt }
enum Answer { Allow, AllowForSession, Deny, DenyAndStop }
enum Event {
    Started { session, model }, TurnStarted,
    TextDelta { text: Untrusted }, Text { text: Untrusted },
    ToolStart { id, parent, tool, station, label, detail: Untrusted }, ToolEnd { id, failed },
    Permission { ticket, tool, station, label, detail: Untrusted, len, cut, claim: Untrusted,
                 always: Option<Untrusted> }, PermissionClosed { ticket }, AutoDenied { tool, label, reason },
    Usage { context, window, cost_micro_usd }, Compaction,
    TurnFinished { outcome }, Error { kind, retrying }, Unknown { kind },
    Exited { code, stderr: Untrusted },
}
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
  dropped as it streams and reported as `Unknown { kind: "oversized" }`.
- The driver: piped stdio, the CLI in a process group of its own. One thread reads stdout
  (→ protocol → events, replies), one keeps stderr's last 4 KB, one waits for the exit
  (`waitid` with `WNOWAIT`), kills what the CLI left in its group (a background command, an MCP
  server) while the group's id is still held, then reaps it. `kill` and dropping a driver kill
  the whole group. No clock decisions: the one sleep gives stderr up to half a second to finish.
- Turns: Claude names no turn start, so its first output after a `result` is `TurnStarted`. An
  interrupt with no turn running sends nothing; a Codex interrupt before the turn has an id is
  sent when the id comes. A Codex thread or turn that fails to start ends as a failed turn.

Tests: 24 unit (incl. the three approval rules: exact session rule, cut and auto-deny, every changed path), 4 driver tests against fake `claude` and `codex` scripts (`tests/fakes`) that
replay hand-written fixtures and record argv and stdin (a turn with a permission asked and
allowed, a turn interrupted, stdin closed; and a background command ended with its CLI), and 4 property tests (every fixture value swapped
for 15 other values and types, every line cut at every character, random bytes; a sentinel never
reaches a label or `Debug`). Two ignored probes of the real CLIs (`tests/real.rs`, below).

## Trust rules

Everything in the stream is model- or tool-written, including what looks like the CLI's own
words (a tool result, a file the model read, a web page it fetched).

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
     `https`, `http` and `mailto` (scheme checked case-insensitively after percent-decoding),
     everything else drawn as text. A link's text that looks like a URL but differs from its
     target shows both.
   - File paths in text become links only inside the task's repository, after resolving
     symlinks and `..`; a click reveals or opens in the user's editor, never runs.
   - No HTML blocks or inline HTML rendered: shown as text. No autolinking of bare hosts.
   - Code blocks: monospace, a copy button, no execution affordance. Terminal escapes in text
     are stripped before drawing.
3. **Answers and sends are the user's.** Permission answers, interrupts, stop, and the message
   text come only from `Origin::User`. A socket client may read a conversation's labels; whether
   it may ever send a message is a separate trust decision (as `type` is today) and not in this
   plan.
4. **Argv.** A fixed table plus a name-shaped model and a UUID. The shell runs it through the
   login shell for `PATH` as stage 1 does (`"$SHELL" -lic 'exec "$@"' -- claude …`).
5. **Data exposure.** stderr may hold tokens: it is `Untrusted`, shown on request in the drawer,
   never logged or put in the state. Images the user attaches are read once and sent; nothing
   is cached on disk by ottospace.
6. **Prompt injection** cannot be prevented here; the defence is the approval step being a real
   human act with the real command shown, and nothing auto-approved by ottospace.

## Native view stages (not built)

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
`ModelUsage`); `codex exec --help`, `codex exec resume --help`, `codex app-server --help`, and
`codex app-server generate-json-schema --out <dir>` (0.166.0, offline); learn.chatgpt.com/docs
`non-interactive-mode` (exec's JSONL events).

Verified from those sources: every flag above exists; stream-json output needs `--verbose`;
the shapes of `system/init`, `stream_event`, `assistant`, `user`, `result` (incl.
`total_cost_usd`, `modelUsage[*].contextWindow`), `system/api_retry` error codes; every Codex
request, notification and approval shape and decision value used (from the generated schema).

UNVERIFIED, and what settles each:

| What | Why it matters |
|---|---|
| Claude's `control_request`/`control_response` wire shape (`request_id`, `request.subtype: can_use_tool`, `tool_name`, `input`; response `{subtype: success, request_id, response: PermissionResult}`) | The docs give `PermissionResult` and say a `control_response` must echo `request_id`, but not the envelope. Written from the SDKs' behaviour |
| `--permission-prompt-tool stdio` accepted by 2.1.323 | The docs describe `--permission-prompt-tool` as an MCP tool name; `stdio` is what the SDKs pass |
| `updatedInput` required or optional on allow | We send the original input back, which both accept |
| That our session rule (`addRules`, `destination: session`, `ruleContent` the exact command, `//path` or `domain:host`) is honoured for the request's tool | If not, "always" falls back to asking again: safe, less useful |
| When Claude prints `system/init` (at start or after the first message) | Only ordering; the parser does not care |
| The `result` after an interrupt, and the interrupt's `control_response` | The parser marks the turn interrupted itself |
| Codex app-server message order, and whether it needs `"jsonrpc":"2.0"` | The schema does not require the field; we omit it |
| Codex asks for approval at all under the user's default policy and sandbox | A `touch` in a workspace-write sandbox may need none |
| Codex app-server in a folder that is not a git repository | The probe uses an empty folder |

Commands that settle all of the above. Each starts **one short model session on your
account** (Claude with `haiku`, Codex with your default model) in a new empty temporary folder,
with the prompt "Run the shell command `touch ottospace-probe` …" and then "Count from 1 to
200", interrupted at once. They print event kinds only. Not run without your approval:

```
OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture claude_real
OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture codex_real
```

## Open questions

1. **Codex through the experimental app server, or Claude-only native first?** Recommend:
   both behind the same trait, Claude first in the UI; Codex's native view ships once the probe
   passes, with the terminal tab as its fallback.
2. **Should a task started natively be resumable in the terminal tab, and vice versa?**
   Recommend yes, one at a time: the drawer stops one process before starting the other.
3. **"Always this session" or also "always for this repository"?** Recommend session only in
   ottospace; a lasting rule is the user's edit of their own settings.
4. **Set Codex's approval policy for drawer sessions** (e.g. `on-request`), or keep the user's
   config? Recommend the user's config, shown in the drawer header so a `never` is visible.
5. **May a socket client send a message to a task's agent?** Recommend no for now, as stage 1;
   revisit with the agent-to-agent work.
6. **Text selection.** Copy buttons first, or build drag-selection over `StyledText` before 2a
   ships? Recommend copy buttons first; selection as 2a's follow-up.
7. **Sub-agent text.** Claude's `--forward-subagent-text` would show sub-agents' prose too.
   Recommend off: the activity view shows their calls; their text stays in the files.
