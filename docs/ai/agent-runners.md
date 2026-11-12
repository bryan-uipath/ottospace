# Agent runners: what the agents in the panes are doing

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
| Where it lives | A new crate, `crates/ottospace-agents`: std, serde, serde_json. No network, no async, no writes |
| Source of truth | The runner's own session files, read incrementally |
| Live updates | Tail the files first. Hooks later, for the pane link and for what files lack |
| What leaves the crate | Names, counts, times. Never prompts, file contents, command lines or outputs |
| Session ↔ directory | The cwd recorded in the session is the directory or below it. Worktrees: ask per checkout |
| Session ↔ pane | Not built. Plan: runner pid → the pane's shell is an ancestor. Fallback: a hook reports `OTTOSPACE_PANE` |
| Wiring | None in this slice: no UI, no model, no socket. A hidden `ottospace agents [dir]` prints JSON |

## The abstraction

```rust
trait Runner {
    fn kind(&self) -> Kind;                                             // Claude | Codex | Other(String)
    fn sessions(&self, dir: &Path) -> io::Result<Vec<SessionRef>>;      // started in dir or below, newest first
    fn read(&self, session: &SessionRef, after: Cursor) -> io::Result<(Vec<Event>, Cursor)>;
}
```

- `SessionRef { kind, id, cwd, path, started, last_activity, model, pid, busy, parent }`. Times
  are Unix milliseconds. `pid` and `busy` are the runner's own record of a live process, when it
  keeps one. `parent` marks a sub-agent's own session file.
- `Cursor` is opaque: a byte offset plus the little state a reader needs across calls (last
  model, last request id, open sub-agents). `Cursor::default()` reads from the start.
- `read` returns only complete lines. A last line without its newline is left for the next call.
  A line that is not JSON, or a record of an unknown type, is skipped. A file shorter than the
  offset was replaced and is read again from the start.
- `Event { at, what }`, where `What` is: `UserMessage`, `ModelStep { model, usage, window }`,
  `ToolStart { id, tool, station, summary }`, `ToolEnd { id, failed, station, summary }`,
  `Error { kind }`, `SubAgentStart { id, label }`, `SubAgentEnd { id }`,
  `Compaction { tokens_after }`, `ModelChange { from, to }`.
- `station::classify(tool, arg) -> Station` is pure and table-tested. For a shell tool `arg` is
  the command line, and the station goes by the names of the commands in it: Network if any
  reaches the network (`curl`, `gh`, `ssh`, `git push`, `npm install`, …), Files if every one
  only works on files (`cat`, `grep`, `rm`, `sed`, …), else Shell. Wrappers (`sudo`, `env`,
  `timeout`, `xargs`), `$(…)`, pipes, redirections, heredoc bodies and `find -exec` are handled.
- `Snapshot::fold(events)` is pure: running calls, the last 16 finished calls, uses per station,
  running sub-agents, errors since the last user message, the model, context tokens and window.
  `activity(now_ms, idle_after_ms)` takes the clock as an argument.

Changes from the shape first sketched, and why:

- `Event` is a struct with a time around the enum `What`: every variant has a time.
- `SessionRef` gained `pid`, `busy` and `parent`: Claude Code records the first two, and they
  are the strongest signals found. Codex writes each sub-agent as a session of its own.
- `ToolEnd` may carry a station and a summary. Codex's current tool is `exec`, whose input is a
  script; the command it ran is only known when it ends. The snapshot then moves the call's
  count to the station it turned out to use.
- `git::checkouts(dir)` and `report::report(…)` were added: the first lists a repository's
  worktrees from `.git` without running git, the second is the JSON the subcommand prints.

`Hooks` is a station nothing lights yet. `You` counts user messages, `Model` model requests.

## Claude Code

Where: `<config dir>/projects/<slug>/<session id>.jsonl`. The config dir is `$CLAUDE_CONFIG_DIR`
or `~/.claude`; `Claude::new` takes a list, since one machine may have several (`~/.claude`,
`~/.claude-personal`). The slug is the cwd with every character but ASCII letters and digits
turned into `-`, so it is lossy: `/a/b-c` and `/a/b/c` share one. `sessions` therefore takes the
folders whose name is the slug or begins with it and `-`, and then checks the `cwd` recorded
inside each file.

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
| Model | `message.model` of the last request | solid |
| Process and busy/idle | `sessions/<pid>.json` | solid while it runs; a crashed process may leave the file: check the pid |
| Tool call start | `tool_use` block | solid. One request is written as one record per content block, same `message.id` |
| Tool call end, failed | `tool_result` block, `is_error` | solid |
| Call in progress | a start with no end yet | solid: the start is on disk while the call runs (checked once) |
| Tokens in context | `usage`: input + cache read + cache creation + output | solid |
| Context window size | not recorded | **estimate**: 200k, or 1M once a request exceeds 200k |
| Compaction | `compact_boundary`, `compactMetadata.postTokens` | solid, 2 samples seen |
| API error | `assistant` record with `error` (`rate_limit`, `server_error`, …) | solid |
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
| What a call was | current: only at its end, from `CommandExecution`, matched by order | best effort. While it runs the call shows as `exec` on Shell |
| Failed | `status: failed` or a non-zero `exit_code` | solid for commands; other tools never fail here |
| Call in progress | a start with no output yet | **unverified**: whether the start is on disk before the call ends was not measured |
| Tokens in context | `event_msg/token_count`, `last_token_usage`: input + output | solid |
| Context window size | `token_count.info.model_context_window` | solid |
| Compaction | top-level `compacted` | best effort: no token count after; `event_msg/context_compacted` is ignored so one compaction is not counted twice |
| Errors | `event_msg/error` (`codex_error_info`), `turn_aborted` unless `interrupted` | best effort, one sample |
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
already. One elsewhere does not, so the caller asks once per checkout. The model already lists
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
changes; `read` costs one `stat` when nothing was appended. Add a hook only for what files
cannot give: the exact pane, and a start signal if a runner turns out to buffer its writes.
Never make the view depend on hooks alone.

First read of a long session is the cost to watch: a 48 MB Claude transcript took about 15 s in
a debug build and 1 to 3 s in a release build, on one thread. The shell must read off the main
thread and keep the cursor.

## Privacy

Session files hold prompts, source code, command output and whatever secrets passed through.

- Read-only. The crate opens files for reading and writes nothing, anywhere.
- Local only. No network code and no dependency that has any.
- Only derived facts leave the crate. A label is a name: a command's name and subcommand
  (`git push`, never its arguments), the last two parts of a path, a host without path or query,
  a skill's or agent type's name, an error code. Control characters are removed and labels are
  cut at 60 characters. The fixture test checks that no prompt, output or argument reaches the
  printed JSON.
- Known soft spots: a file's name and a subcommand are shown and can themselves be telling;
  `SessionRef.cwd` and `path` are full paths.
- Nothing is cached or copied. A snapshot lives in memory.
- Off switch, to build with the first consumer: a setting `agents: false` in `settings.json`
  that stops all reading, checked by the shell before it constructs a runner. Today nothing
  reads unless someone runs `ottospace agents`.
- Tests read only `tests/fixtures`: hand-written records with made-up content.

## Unknown or unverified

- Whether Codex writes a tool call's start record before the call finishes. This decides whether
  "running now" can come from files alone. Claude Code does: a running command found its own
  `tool_use` record on disk (2.1.322, a sub-agent's transcript).
- Claude's context window size. The estimate is wrong for a 1M session until it passes 200k.
- Whether Claude Code removes `sessions/<pid>.json` on a crash, and whether pids are reused
  fast enough to matter. `procStart` in that file could settle it.
- Whether hook commands inherit `OTTOSPACE_PANE` in both runners. Codex's `hooks.json` contract
  was not read.
- Claude's slug for very long paths, and for non-ASCII ones.
- A user message queued while a tool runs: the snapshot drops running calls on a user message,
  so such a call would vanish from `running` early.
- Codex versions between the two generations may write both forms of a user message or a
  sub-agent event, which would count them twice.
- Codex MCP calls, `local_shell_call`, and refusals in either runner: mapped from names, no
  samples.
- Other runners (Gemini CLI, opencode, Cursor agent): not looked at. `Kind::Other` is the slot.

## Staged plan

1. **This slice.** The crate, both runners, stations, snapshot, fixtures, `ottospace agents`.
2. **Measure.** Watch a live Codex session to settle the flush question; decide the poll interval.
3. **Pane link.** Parent-pid lookup in `ottospace-mac`; pane → Claude session by process tree.
4. **Core.** `Action::AgentsChanged { … }` carrying snapshots, so the model owns what is shown.
   The shell polls off the main thread and dispatches. The `agents` setting and its off switch.
5. **Markers.** Active, waiting, failed per pane and per repository row. Snapshot in the control
   socket's state, so E2E can read it.
6. **Radial view.** Stations, running calls, sub-agents, context ring.
7. **Later.** Sub-agents' own calls; hooks for the pane link and the Hooks station; Codex's
   SQLite index; more runners.
