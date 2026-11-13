# Sessions: what the agents in the panes are doing

Words: an **agent** is a saved definition (instructions, skills, runner, model: see `routines.md`).
A **session** is one running or finished conversation of a runner (Claude Code, Codex). This note,
the crate `ottospace-sessions` and `ottospace sessions` are about sessions.

One Rust abstraction over Claude Code, Codex and later runners. It answers, for a directory:
which agent sessions exist, is each active, what tool call runs now and of what kind, what ran
lately, errors, sub-agents, the model, and how full the context window is.

Consumers, later: a status marker per pane and repository, and a radial activity view with the
stations You, Model, Memory, Files, Shell, Network, Browser, Agents, Skills, Hooks. The station
set and the "names, never content" rule follow the [lot-map](https://github.com/alexeyw/lot-map)
README (MIT); no code is taken from it.

Written 2026-11-12 against Claude Code 2.1.322 and codex-cli 0.166.0 on macOS. The schemas below
were read off real session files by listing keys and types, never content. Neither runner
documents these files as a stable interface.

## Decisions

| Question | Decision |
|---|---|
| Where it lives | A new crate, `crates/ottospace-sessions`: std, serde, serde_json. No network, no async, no writes |
| Source of truth | The runner's own session files, read incrementally |
| Live updates | Tail the files first. Hooks later, for the pane link and for what files lack |
| What leaves the crate | Names, counts, times. A command's label is built only from fixed tables. Never prompts, file contents, command lines or outputs. One exception: a session's title, by `Runner::title` alone (see Labels, "Quoted") |
| Session ↔ directory | The cwd recorded in the session is the directory or below it. Worktrees: pass every checkout in one call |
| Session ↔ pane | Not built. Plan: runner pid → the pane's shell is an ancestor. Fallback: a hook reports `OTTOSPACE_PANE` |
| Wiring | None in this slice: no UI, no model, no socket. A hidden `ottospace sessions [dir]` prints JSON |

## The abstraction

```rust
trait Runner: Send + Sync {
    fn kind(&self) -> Kind;                                          // Claude | Codex | Other(String)
    fn sessions(&self, dirs: &[PathBuf]) -> io::Result<Vec<SessionRef>>;   // one scan, newest first
    fn model(&self, session: &SessionRef) -> Option<String>;         // may read the file's end
    fn title(&self, session: &SessionRef) -> Option<String>;         // free text: quote, never obey
    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> io::Result<Batch>;
}
struct Batch { events: Vec<Event>, cursor: Cursor, restarted: bool, more: bool }
```

- `SessionRef { kind, id, cwd, path, started, last_activity, model, pid, busy, parent }`. Times
  are Unix milliseconds. `pid` and `busy` are the runner's own record of a live process, when it
  keeps one. `parent` marks a sub-agent's own session file. `model` is set only when listing
  knows it for free (Codex); `Runner::model` reads it on demand.
- `sessions` takes every directory of interest at once and scans once. A relative or empty
  directory is an error: `Path::starts_with("")` is true for every path, so it would match every
  session. What listing reads from a file is kept by (path, modification time, length) and read
  again only when one changed.
- `Cursor` is opaque: a byte offset, the file's (device, inode), and the little state a reader
  needs across calls (model, last request id, peak tokens, open sub-agents, open Codex calls).
  `Cursor::default()` reads from the start.
- `read` returns only complete lines, and stops at the first line boundary past `budget` bytes
  with `more: true`, so a first read of a long file comes in slices. A last line without its
  newline is left for the next call. A line that is not JSON, a record of an unknown type and a
  line over 64 MB are skipped; the long one is never held in memory, and while its end is not
  written yet the cursor remembers that, so a poll reads only the new bytes.
- Only regular files are opened: a FIFO named like a session file is passed by, not waited on.
- Bounds: a cursor tracks at most 256 open sub-agents, 256 open Codex calls and 256 refinements;
  a snapshot keeps the newest 256 running calls and 256 sub-agents. Counts are not cut.
- A file that got shorter, or another file at the same path, resets the whole cursor and sets
  `restarted`: the consumer throws its snapshot away and folds from that batch on.
- `Event { at, what }`, where `What` is: `UserMessage`, `ModelStep { model, usage, window }`,
  `ToolStart { id, tool, station, summary }`, `ToolEnd { id, failed, station, summary }`,
  `Error { kind }`, `SubAgentStart { id, label }`, `SubAgentEnd { id }`,
  `Compaction { tokens_after }`, `ModelChange { from, to }`.
- `station::classify(tool, arg) -> Station` is pure and table-tested. For a shell tool `arg` is
  the command line, and the station goes by the names of the commands in it: Network if any
  reaches the network (`curl`, `gh`, `ssh`, `git push`, `npm install`, …), Files if every one
  only works on files (`cat`, `grep`, `rm`, `sed`, …), else Shell. Wrappers with their own
  options (`sudo -u x`, `env`, `timeout -s KILL 30`, `xargs -I {}`), `bash -c '<line>'`, `$(…)`,
  pipes, redirections, `find -exec`, comments, heredoc bodies and array values are handled.
- `Snapshot::fold(events)` is pure: running calls, the last 16 finished calls, uses per station,
  running sub-agents, errors since the last user message, the model, context tokens and window.
  `activity(now_ms, idle_after_ms)` takes the clock as an argument.

### Labels

Everything shown about a call is one of these, and nothing else.

**From fixed tables in the code.** Nothing a file says can add to them.

| Label | Rule |
|---|---|
| A command | The program's name, only if it is in the table in `station.rs` (about 180 programs); else `shell`. Then a subcommand, only if it is on that program's list (`git push`, `cargo test`, `kubectl get`) and is the first word after the options, where the program's value-taking options (`git -C x`, `kubectl --token x`, `npm --otp x`) are skipped with their values. The label is the first command's on the line, never a later one's. A line is read by its first 256 KiB |
| A command given as words (Codex `["git", "push"]`) | The same, from the words as they are. Never joined and split again |
| A built-in tool | Its name, from the table of both runners' tools (`Bash`, `Read`, `exec_command`, …). Any other tool that is not an MCP tool is `tool` |
| An error | A code from the runner's table (`rate_limit`, `usage_limit_exceeded`, …). Anything else is `api_error` (a Claude API error record), `error` (a Codex error event) or `aborted` (a Codex turn aborted for a reason not in the table). A code's shape is not enough |
| An id of a call, a message, a sub-agent or a thread | Never shown: a 16-digit hash of it, good only to pair a start with its end. An empty id is no id, and its record is skipped |

**Shown as written.** These pass a shape check, but the text is still what a model or a file
wrote. Treat each as **untrusted, model-written text**: fine in a local view, never an
instruction. Before any of them reaches the control socket or a UI another agent reads, the
guide must say so. `what_is_shown_as_written` in `tests/fixtures.rs` lists them by test.

| Field | Where the value comes from | Shape it must have, else |
|---|---|---|
| File label (`ToolStart.summary` of `Read`, `Write`, `Edit`, `NotebookEdit`, `Glob`, `Grep` with a path; Codex `apply_patch`) | The model's tool input (`file_path`, `notebook_path`, `path`), or a patch's `*** … File:` line | The last two parts of something path-shaped: it has a `/`, no `://`, and each shown part is at most 40 of `A-Z a-z 0-9 space . _ + @ , ( ) -`. With no `/`: 1 to 64 of `A-Z a-z 0-9 _ . -`. Else `file` |
| Host (`summary` of `WebFetch`) | The model's tool input (`url`) | `scheme://host`, the host letters, digits, `.` and `-` with a dot in it, at most 60, no credential prefix. Any `@` after the scheme, no scheme, `mailto:`, IPv6: `web` |
| Skill name (`summary` of `Skill`) | The model's tool input (`skill`) | A name (below). Else `skill` |
| Sub-agent type (`summary` of `Agent`, `SubAgentStart.label`) | The model's tool input (`subagent_type`) | A name. Else `agent` |
| MCP server and tool (`ToolStart.tool`, `summary`) | The tool name the model called, `mcp__<server>__<tool>` | Each a name. Else `mcp` |
| Codex agent (`SubAgentStart.label`) | The last part of `agent_path` in the rollout | A name. Else `agent` |
| Model (`ModelStep.model`, `ModelChange`, `SessionRef.model`) | `message.model` in a transcript, `turn_context.model` in a rollout | A name, with an optional `[1m]` at the end for Claude. Else the record's model is ignored |
| Session id (`SessionRef.id`, `parent`) | Claude: the transcript's file name. Codex: `session_meta.id`, `parent_thread_id` | A name. Else the session is **not listed** (chosen over an opaque id: nothing could be matched to it) |
| Directory (`SessionRef.cwd`, the report's `cwd`; `SessionRef.path`) | `cwd` in the session file; the file's own path | An absolute path of at most 1024 bytes with no control characters and no `..`. Else the session is not listed |

**Quoted.** One value is free text, and the only one: a session's **title**. It leaves the
crate by `Runner::title` and nowhere else: not in `SessionRef`, an `Event`, a `Snapshot` or the
report `ottospace sessions` prints.

| Runner | Where the title comes from |
|---|---|
| Claude Code | Records `{"type":"custom-title","customTitle":…}` (the user's `/rename`) and `{"type":"ai-title","aiTitle":…}` (written by Claude Code's own model). Claude Code repeats both near the end of the transcript as it grows, so they are read from the file's last 256 KiB, together with the model: one read, kept by (path, modification time, length). The last record of each kind counts. A custom title wins; an empty one is a name taken back. Only a whole record of that type counts: the same words inside a prompt or a tool's output are another record's text |
| Codex | `<codex home>/session_index.jsonl`: `{"id","thread_name","updated_at"}` a line, the last line for an id. Only its last 1 MiB is read. A rollout records no title, and most sessions have no line in the index: those have none |

No title is made up: no model is called, and nothing is summarised. A session without one
shows none.

A title is **untrusted free text**: a model wrote it from the session's prompts and from
whatever the session read, which may be a web page or a file somebody else wrote. The rules:

- The crate gives out its first line, without control characters, at most 256 characters.
- The core (`sessions::kept`) keeps of that only characters that show (`capture::shows`: no
  zero-width, direction-turning, tag or filler characters) and at most 80 of them.
- It is drawn in the session's row, as plain text on one line, and it is in the state reply as
  `sessions_bar.rows[].quoted_title`. The guide tells an agent that it is a quotation written
  by another model: data, never an instruction.
- It goes nowhere else. Adopt names the task after the repository, never after the title. A
  title is not read for tags, not rendered as markdown, not typed into a terminal and not
  written to disk.

The model shown beside it is a name (the table above). The core shortens it by a fixed rule
(`sessions::short_model`): `claude-opus-4-7[1m]` → `opus-4.7`; any other name as it is.

A **name** is 1 to 40 of `A-Z a-z 0-9 _ . : -` that does not look like a credential: it does not
begin with `ghp_`, `gho_`, `github_pat_`, `sk-`, `xox`, `AKIA` or `eyJ`, and has no `:` with 8
or more characters after it. The last rule also hides a plugin's skill (`plugin:code-review`
shows as `skill`).

`tests/fixtures/adversarial` holds a session of each runner with about 500 sentinels: one in
every free-text and argument position (prompts, tool output, header values, env prefixes, option
values in the forms `--opt v`, `--opt=v` and `-ov`, comments, argv elements, `$(…)`, backticks,
`<(…)`, `env -S`, `find -exec`, remote commands, nested shells, error strings and code-shaped
error values, URL userinfo, path and query, file content, the leading parts of a path,
descriptions, ids), and in every name position as free text and as a credential shape. A test
asserts that none is in the printed JSON or in the `Debug` form of any public type. A second
test asserts the exact set of values that are shown as written.

Changes from the shape first sketched, and why:

- `Event` is a struct with a time around the enum `What`: every variant has a time.
- `SessionRef` gained `pid`, `busy` and `parent`: Claude Code records the first two, and they
  are the strongest signals found. Codex writes each sub-agent as a session of its own.
- `ToolEnd` may carry a station and a summary. Codex's current tool is `exec`, whose input is a
  script; the command it ran is only known when it ends. The snapshot then moves the call's
  count to the station it turned out to use.
- `git::checkouts(dir)` and `report::report(…)` were added: the first lists a repository's
  worktrees from `.git` without running git, the second is the JSON the subcommand prints.
- After review: `sessions` takes several directories, `read` takes a byte budget and returns a
  `Batch`, the model is read on demand, and labels come from tables only (see Labels).

`git::checkouts` reads the same files as the app's `crates/ottospace/src/repos.rs`, which builds
the Repos list with branches. They are not one helper: `repos.rs` returns the core's `Repo` and
lives in the shell crate. The rules were brought in line instead: resolved paths, pointers
relative to where they are written, small regular files only. If a third reader appears, move
the shared part here and have `repos.rs` call it.

`Hooks` is a station nothing lights yet. `You` counts user messages, `Model` model requests.

## Claude Code

Where: `<config dir>/projects/<slug>/<session id>.jsonl`. The config dir is `$CLAUDE_CONFIG_DIR`
or `~/.claude`; `Claude::new` takes a list, since one machine may have several (`~/.claude`,
`~/.claude-personal`). The slug is the cwd, part by part, with every character but ASCII letters
and digits turned into `-`, so it is lossy: `/a/b-c` and `/a/b/c` share one. `sessions`
therefore takes the folders whose name is the slug or begins with it and `-`, and then checks
the `cwd` recorded inside each file. The `cwd` is usually in the first few records; if not, the
whole file is searched for it once, and the result kept.

Also read: `<config dir>/sessions/<pid>.json` (one per running process: `pid`, `sessionId`,
`cwd`, `status` = `busy` | `idle`, `updatedAt`), and
`<project>/<session id>/subagents/agent-<id>.meta.json` (`toolUseId`, `agentType`, `finished`,
`stoppedByUser`).

Records used: `user` (a prompt, or `tool_result` blocks), `assistant` (`message.model`,
`message.usage`, `tool_use` blocks, `error`), `system` with `subtype: compact_boundary`. About
twenty other record types are skipped. `api-request*` lines are most of a file's bytes and are
skipped by their prefix, without parsing.

| Fact | Source | Reliability |
|---|---|---|
| Session exists for a directory | folder slug + recorded `cwd` | solid |
| Started, last activity | first `timestamp`; file mtime | solid |
| Model | `message.model` of the last request, read from the file's end on demand | solid |
| Process and busy/idle | `sessions/<pid>.json` | solid while it runs; a crashed process may leave the file: check the pid |
| Tool call start | `tool_use` block | solid. One request is written as one record per content block, same `message.id` |
| Tool call end, failed | `tool_result` block, `is_error` | solid |
| Call in progress | a start with no end yet | solid: the start is on disk while the call runs (checked once) |
| Tokens in context | `usage`: input + cache read + cache creation + output | solid |
| Context window size | not recorded | **estimate**: 1M once any request of the session exceeded 200k, and it stays 1M after a compaction. Below that: 200k for a model with no 1M window (`haiku`), **unknown** for the others, so no percentage |
| Compaction | `compact_boundary`, `compactMetadata.postTokens` | solid, 2 samples seen |
| API error | `assistant` record with `error` | solid. Shown from a table: `authentication_failed`, `invalid_request`, `model_not_found`, `rate_limit`, `server_error` were seen; the table's other entries are the API's error types, not seen here. Else `api_error` |
| Refusal | `stop_reason: refusal` | best effort, no sample seen |
| Sub-agent start | `Agent` (`Task`) tool call | solid |
| Sub-agent end | the call's result, or `finished` in the meta file for a background agent | best effort: every sample was a background launch; an agent that crashed never ends |
| Sub-agent's own calls | `subagents/agent-<id>.jsonl`, same schema, `isSidechain: true` | not read yet |
| User message | `user` record that is not `isMeta`, not a compaction summary, not a tool result | best effort: some injected text may count |
| Hook refusals | `system/stop_hook_summary`, `hook_*` attachments | not read yet |

## Codex

Where: `$CODEX_HOME` or `~/.codex`, `sessions/<yyyy>/<mm>/<dd>/rollout-<time>-<id>.jsonl`. The
first line is `session_meta` with `cwd` and `id`. There is no folder per directory, so `sessions`
opens the newest rollouts (1000 by default) and reads each one's head. `archived_sessions/` is
not listed.

Records are `{timestamp, type, payload}`. Two generations are on this machine and both are read:

| | Current (0.166) | Earlier 2026 |
|---|---|---|
| Shell tool | `custom_tool_call` `exec`, input is a script | `function_call` `exec_command` with `cmd` |
| What ran | `event_msg/item_completed`, item `CommandExecution` (`command`, `status`, `exit_code`) | `event_msg/exec_command_end` |
| User message | item `UserMessage` | `event_msg/user_message` |
| Sub-agents | item `SubAgentActivity` (`started`, `completed`), each agent a rollout with `parent_thread_id` | `event_msg/sub_agent_activity` |
| File edits | through `exec` | `custom_tool_call` `apply_patch` |

| Fact | Source | Reliability |
|---|---|---|
| Session exists for a directory | `session_meta.cwd` | solid, but only among the newest rollouts scanned |
| Model | `turn_context.model` | solid |
| Process, busy/idle | none found | **unavailable** from files |
| Tool call start, end | `function_call` / `custom_tool_call` and their `*_output`, by `call_id` | solid |
| What a call was | earlier: `exec_command_end` names its call. Current: `CommandExecution` names none, so it is given to the open `exec` call only when exactly one is open | best effort. While it runs the call shows as `exec` on Shell; with parallel `exec` calls each stays `exec` |
| Failed | `status: failed` or a non-zero `exit_code` | solid for commands; other tools never fail here |
| Call in progress | a start with no output yet | **unverified**: whether the start is on disk before the call ends was not measured |
| Tokens in context | `event_msg/token_count`, `last_token_usage`: input + output | solid |
| Context window size | `token_count.info.model_context_window` | solid |
| Compaction | top-level `compacted` | best effort: no token count after; `event_msg/context_compacted` is ignored so one compaction is not counted twice |
| Errors | `event_msg/error` (`codex_error_info`), `turn_aborted` unless `interrupted` | best effort, one sample. Shown from a table of Codex's error kinds, written from memory of its source and not checked against 0.166; an unlisted kind shows as `error` |
| Sub-agent start, end | `SubAgentActivity` | best effort, few samples |
| MCP and web calls | `web_search_call` is read; MCP call records were too rare to map | best effort |

`~/.codex/state_5.sqlite` has a `threads` table with `cwd`, `rollout_path`, `model` and
`tokens_used`: an index that would make `sessions` exact and cheap. Not used: it needs a SQLite
dependency, and the file name carries a schema version.

## Matching

**Directory.** A session matches `dir` when its recorded cwd is `dir` or below it, compared by
path components. A session started in a subdirectory matches the repository. A later `cd`
inside the session does not move it.

**Worktree.** A worktree inside the repository (`.claude/worktrees/x`) is below it and matches
already. One elsewhere does not, so the caller passes every checkout in one call. The model already lists
each repository's worktrees; `git::checkouts(dir)` gives the same from any directory inside one
by reading `.git`, `commondir` and `worktrees/*/gitdir`. Not handled: a repository moved after
its sessions were written, submodules, bare repositories.

**Pane.** Not built. Three ways, strongest first:

1. **Process tree.** A terminal pane knows its shell's pid. Claude Code's `sessions/<pid>.json`
   gives pid → session id. The session is the pane's when the pane's shell is an ancestor of
   that pid. It needs a parent-pid lookup in `ottospace-mac` and no cooperation from the user.
   For Codex there is no pid file: find the `codex` process below the shell, then the rollout it
   holds open, or fall back to way 3.
2. **Hook.** ottospace sets `OTTOSPACE_PANE` in its terminals. A runner hook that runs at session
   start can post `{session id, pane}` to the control socket. Exact, but it needs a hook
   installed in the user's runner settings, and a new `Request`, which is a trust decision.
3. **cwd and recency.** The pane's cwd equals the session's cwd and the session is the newest
   active one there. Wrong when two agents share a directory.

The CLI is strict about its directory: `ottospace sessions` takes an absolute path that exists
(default: the current directory) and fails otherwise. It never falls back to "every session".

## Live updates

| | Tail the session file | Runner hooks → control socket |
|---|---|---|
| Setup | none | a hook entry in each runner's settings, kept in step with ottospace |
| Covers | everything written: calls, usage, errors, sub-agents, model | what the runner offers hooks for; context use is not among them for a plain settings hook |
| Latency | the poll interval, or a file watcher | immediate |
| Sessions started before ottospace | yes | no |
| Sessions outside ottospace's terminals | yes | yes, without a pane |
| Breaks when | the file format changes | the hook contract changes, or the hook is removed |
| Trust | read-only | another process writes to the socket: a new `Request` |

Recommendation: **tail first, hooks second**. Tailing needs no setup, sees past sessions and
gives every field above. Poll active sessions about once a second and others when their mtime
changes. A tick costs one directory scan and one `stat` per session file for the listing (heads
are cached), and one `open` per session read when nothing was appended. Add a hook only for what files
cannot give: the exact pane, and a start signal if a runner turns out to buffer its writes.
Never make the view depend on hooks alone.

First read of a long session is the cost to watch: a 48 MB Claude transcript took about 15 s in
a debug build and 1 to 3 s in a release build, on one thread. The shell must read off the main
thread (`Runner` is `Send + Sync`), give `read` a byte budget so the first read comes in slices,
and keep the cursor.

## Conversations

A click on a session's row opens its conversation in the right drawer (see
`task-conversation.md`); Adopt makes it a task's. Rules, each with a core test and a property
check on every `StartAgent`:

- **The user alone.** `SessionOpen` from a client or a timer is refused (`Model::refusal`). A
  client's `session_adopt` makes the task and starts nothing.
- **The command** is the launcher's table (`agents::resume`): `claude --resume <id>`, `codex
  resume <id>`, the id a UUID, as argv. Nothing else from a session file; never the title.
- **The directory** is where the session started, if it is inside a listed repository or
  worktree with no `..` (`sessions::below`); the shell checks again with links resolved
  before it starts anything.
- **No second program.** A session at work, or held (`Found.held`: Claude Code's
  `sessions/<pid>.json` names a pid that exists), is not resumed. One whose agent runs in the
  drawer is shown, not started again.

Known gaps: Codex records no process, so an idle Codex session open in another terminal is
resumed. For up to 30 s after a drawer agent ends its session reads "at work", and "held" is
as of the last read, so opening it again may say it is live elsewhere until the next read.

## Privacy

Session files hold prompts, source code, command output and whatever secrets passed through.

- Read-only. The crate opens files for reading and writes nothing, anywhere.
- Local only. No network code and no dependency that has any.
- Only derived facts leave the crate. The rules are in Labels above: a command's label is made
  of table entries, never of text from the command line, and every other label must have a
  name's shape or is replaced by a fixed word. The adversarial fixture test checks it.
- A session's title is the one free text that leaves the crate: see "Quoted" under Labels. It
  is shown only in the app's Sessions rows and its state reply, never by `ottospace sessions`.
- Known soft spots, accepted for the local CLI: the fields in "Shown as written" above. A file
  label shows the last two parts of a path; a name-shaped skill, agent type, MCP name or model
  is shown as the model or file wrote it; `SessionRef.cwd` and `path`, and the report's `cwd`,
  are full paths. Revisit before any label reaches the control socket: there the guide must mark
  them as untrusted data, not instructions.
- Nothing is cached or copied. A snapshot lives in memory.
- Off switch, to build with the first consumer: a setting `sessions: false` in `settings.json`
  that stops all reading, checked by the shell before it constructs a runner. Today nothing
  reads unless someone runs `ottospace sessions`.
- Tests read only `tests/fixtures`: hand-written records with made-up content.

## Unknown or unverified

- Whether Codex writes a tool call's start record before the call finishes. This decides whether
  "running now" can come from files alone. Claude Code does: a running command found its own
  `tool_use` record on disk (2.1.322, a sub-agent's transcript).
- Claude's context window size. Unknown for most models until a request passes 200k; which
  models have no 1M window (`haiku`) is a guess written into the code.
- Whether Claude Code removes `sessions/<pid>.json` on a crash, and whether pids are reused
  fast enough to matter. `procStart` in that file could settle it.
- Whether hook commands inherit `OTTOSPACE_PANE` in both runners. Codex's `hooks.json` contract
  was not read.
- Claude's slug for very long paths, and for non-ASCII ones (emoji, accented letters). If Claude
  Code shortens or hashes a long slug, those sessions are not found.
- `UserMessage` clears running calls: a call with no end by the next prompt is taken as
  interrupted. A message that arrives in the middle of a turn (queued while a tool runs) would
  drop that call from `running` early, and its end would then be ignored.
- Config dirs when ottospace is launched from the Dock: no `CLAUDE_CONFIG_DIR` in the
  environment, so only `~/.claude` is read, and this user has a second one. To become a setting
  (a list of config dirs) with the first consumer.
- An empty `CODEX_HOME` is treated as unset. A relative one, or no `HOME` at all, means no
  Codex sessions rather than reading below the current directory.
- Two parallel Codex `exec` calls: their commands cannot be told apart, so neither is named.
- The error tables. Claude's beyond the five seen, and all of Codex's but one, are not from
  samples. A real code missing from a table shows as `api_error` or `error`: safe, less telling.
- The built-in tool table. A tool a runner adds later shows as `tool` until it is listed.
- Codex versions between the two generations may write both forms of a user message or a
  sub-agent event, which would count them twice.
- Codex MCP calls, `local_shell_call`, and refusals in either runner: mapped from names, no
  samples.
- Other runners (Gemini CLI, opencode, Cursor agent): not looked at. `Kind::Other` is the slot.
- Titles. Whether Claude Code always repeats a title within the last 256 KiB (on this machine
  the last one was within 34 KiB of the end in every transcript that had any). `custom-title`
  is from Claude Code 2.1.323's own code, not from a sample: no transcript here had one. Who
  writes Codex's `thread_name` (the user, or Codex) was not found out; it is quoted either way.
  A title for the Codex sessions that have none would need a summarising model: not built.

## Staged plan

1. **This slice.** The crate, both runners, stations, snapshot, fixtures, `ottospace sessions`.
2. **Measure.** Watch a live Codex session to settle the flush question; decide the poll interval.
3. **Pane link.** Parent-pid lookup in `ottospace-mac`; pane → Claude session by process tree.
4. **Core.** `Action::AgentsChanged { … }` carrying snapshots, so the model owns what is shown.
   The shell polls off the main thread and dispatches. The `sessions` setting and its off switch.
5. **Markers.** Active, waiting, failed per pane and per repository row. Snapshot in the control
   socket's state, so E2E can read it.
6. **Radial view.** Stations, running calls, sub-agents, context ring.
7. **Later.** Sub-agents' own calls; hooks for the pane link and the Hooks station; Codex's
   SQLite index; more runners.
