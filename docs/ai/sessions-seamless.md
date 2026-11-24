# One conversation, wherever it runs

Status: design note, no code. Written 2026-11-24 against `main` 70ac51c9, with #482, #486,
#488 and #490 open (the fixes that come first). Each open point has a recommendation; the owner
picks before anything is built.

The owner: "we should make it seamless to go between terminal sessions and ottospace
tasks/sessions", and "so i can build confidence that no matter where i have my conversation,
ottospace can help me". So the order is: first, detection, live updates and counts must be
correct; switching features come after.

## Principle (proposed for AGENTS.md)

> **No matter where the conversation happens, ottospace can help.** A conversation started
> anywhere gets the same help as one started in ottospace: its transcript, its place in the
> city, its decisions, and its turn in needs-you.

The test: take any feature that helps a task's conversation. Does it work, read-only if it must
be, for a `claude` or `codex` the person started in a terminal of their own? If not, either that
is a bug or the note says why it cannot.

## Decided (owner, 2026-11-24)

1. **One writer at a time.** The switch between a terminal and ottospace is offered only when
   the other side is idle or has ended. ottospace never kills or signals the user's process.
2. **Read-only says so.** While ottospace reads a session that another process runs, the view
   says plainly that it is read-only, in the header's chip and banner and where the composer
   would be (#482, #488).
3. Correctness of detection, live updates and counts comes before switching features.

## 1. One identity

**Goal:** a session is one item with one row, one title, one colour, one city presence, one
place in decisions and one history, whether a terminal or ottospace runs it now.

What splits it today:

| Split | Where | Effect |
|---|---|---|
| Elsewhere vs the task list | `Heading::Sessions`, cards (`terminal_tasks`) | A live session is a card among the tasks; an idle or past one is a row in Elsewhere, folded below Done. The same session moves between two lists as it goes idle. |
| Adopted vs free | `Tasks::adopted`, `Model::session` | Track or Take over makes a task with a `Link`. From then on the session's row, its card and the city's `Who::Session` give way to the task, with a new title (the repository's), a new colour, and a new history (the task's). |
| A separate tab on open | `open_space(Owner::Session)` | Fixed for `terminal_tasks` by #490: it opens in place. A Resume still makes `Owner::Session`, a space of its own that is no task's. |
| Two subjects | `Subject::Session` vs `Subject::Task` | Chats, views, dials and the drawer key by subject, so adopting a session leaves its watched chat behind under the old key. |
| Titles | `Found::title` (quoted) vs `Task::title` | A task is never named from the runner's title, so the row changes its name when tracked. |

**Recommendation:** one key per conversation, `(runner, session id)`, which a task points to.
Leave the `Subject` and `Task` types as they are, and fix the four places where the split shows:

- **a.** When a session is tracked, its chat moves to the task's subject. Today the watched
  transcript is dropped and read again.
- **b.** The task's title starts as the session's quoted title, marked as the runner's, until
  the user renames it. **Owner's pick:** this changes the rule "never a task's title" from
  sessions-as-tasks.md (Identity).
- **c.** An idle session that was live this run stays a card ("idle in a terminal"), and does not
  drop to Elsewhere until it ends. Elsewhere remains the archive of sessions that ended.
- **d.** The city and decisions key by session too, so a session tracked mid-turn keeps its icon
  and glow.

## 2. Terminal → ottospace

What exists: Take over (#490 makes the banner's the same as the card's). It runs `SessionAdopt`
for the user: a task named after the repository, linked to the session, and then
`<runner> --resume <id>` in the drawer (structured mode, `RunnerSpec::terminal.resume` /
`agents::Start::Resume`). The full history is in the session file, so the CLI has it. The drawer
shows what the watch built until the resumed agent's events replace it.

What should change:

- **Resume the same session id with its history shown.** This holds today for Claude Code; the
  CLI does it. Missing: the drawer keeps the watched transcript and appends the live one to it,
  rather than starting over (see 1a).
- **Offered only when the other side is idle or ended** (decided 1). How that is told, by
  capability:
  - `disk.held` (Claude Code): no live holder in the process records, or its record says idle
    (not `busy`, not `waiting`), and the file has been quiet for `IDLE_AFTER_MS`.
  - Without `disk.held` (Codex): only when no pane of ours runs it (`in_panes`) and its file has
    been quiet. Take over asks first: "Codex cannot tell if it is still open in another terminal.
    Close it there first." (sessions-as-tasks.md, Two writers).
  - Otherwise the button is dim, its hover says why ("Possible once it is idle in the other
    terminal"), and nothing is queued.

**Recommendation:** keep Take over as the one path, add 1a, and never auto-switch.

## 3. ottospace → terminal

What exists: `Chat | Terminal` in the drawer's header switches one session between the
structured view and the CLI's own TUI in a pane of ottospace's, one process at a time.

Missing: "continue in a terminal" for a person who wants the session in a terminal they keep,
with ottospace following it as it follows any terminal session.

**Recommendation:**

- A menu item and palette command, "Continue in a terminal", on a task's conversation whose agent
  is idle or ended. It opens a terminal pane in the task's checkout and types
  `<spec.terminal.program> <spec.terminal.resume> <id>`. Both come from the `RunnerSpec`, never
  from a `match` on the runner's name. Only the user can run it: a client's request is refused,
  because typing into a terminal is as strong as a shell.
- From then on the chat is the watched, read-only view of that session (`watch_target`, the
  in-pane `InPane` facts make it `in_use`), with the same transcript and the same row.
- Not while a turn runs or a card waits (decided 1). Stop first, as `Chat | Terminal` already
  requires.
- **Owner's pick:** a pane of ottospace's (recommended: it is detected at once, the session's
  waits are known through `disk.waits`, and ⌘J reaches it), or the user's own terminal app (it
  needs a URL scheme or AppleScript per terminal app, and is only detected by file).

## 4. Two processes on one session

Hazard: `claude --resume` of a session that is live does not refuse; it branches the same file,
and two writers interleave (agent-runners.md, Probes). Codex may refuse, or join through its
daemon.

**Rule (decided 1):** one writer at a time.

- **Detected from:**
  - Claude Code: the process records (`sessions/<pid>.json`, `holds`: the pid lives and wrote
    the record), and `status`.
  - Any runner: one of our panes running it (`pane_runner`), or our own agent (`agent_runs`).
  - A file written in the last 30 s, for runners with no records.
- **Shown otherwise:** the read-only banner, the "Watching" chip (or "Not updating"), and a dim
  Take over with its reason. No composer field.
- **Never:** kill or signal the user's process, queue a message for when it ends, or resume a
  session `in_use` (the property check covers `Subject::Session` starts. Extend it to a task's
  linked session: sessions-as-tasks.md lists this as not yet covered).
- **Race:** the user types in the terminal at the same instant Take over starts. The resume is
  refused if the facts read for the click say `in_use` (sessions-as-tasks.md). The window left
  is the few ms between that read and the CLI's start. Accepted, and stated in the hover.

## 5. What cannot carry across

| Thing | Terminal → ottospace | ottospace → terminal | How it is shown (recommended) |
|---|---|---|---|
| Permission mode | The terminal's mode (e.g. `bypassPermissions`) is in the file's records, not on the CLI line; the resumed agent starts in ottospace's default | `--permission-mode` is not passed to the typed command | The foot shows the mode the CLI reports (`auto`, `bypass` in the warning colour), as today; the switch's hover says "starts in the default permission mode" |
| ottospace's MCP server, pass, `OTTOSPACE_TASK` | Only ottospace-started runs get them (`agents::run_args`, `talk::resumed` adds them only for a task) | Lost: the typed command has none | The context dot lists what was added; after a switch, none. The hover names it: "ottospace's tools are not attached in a terminal" |
| Model | The session's model is in its records; a resume keeps the CLI's own default unless `--model` is passed | The same | The foot shows the model the CLI reports, never the one asked for |
| Folder and worktree | `--resume` looks the session up by the folder it started in (`resume_dir`); a session outside the listed checkouts is not offered | The pane opens in the session's start folder; if the agent moved into a worktree (`What::Cwd`) it resumes from the start folder, and the CLI follows | The foot's place line (#490) says repository, worktree and folder on both sides |
| Sub-agents mid-run | Background sub-agents of the terminal's process end with it; a resume does not reattach them | The same | Offered only when idle: no sub-agent is at work (`Found::works`, #486) |
| Hide, queue, drafts | ottospace's own state, kept by session key | A draft in the composer is not typed into the terminal | A draft left warns before the switch, as a quit does (`Confirm::unsent`) |

## Order (recommended)

1. #482, #486, #488, #490: correct detection, live updates, counts and words.
2. 1a and 1c (one transcript, one row through idle), then 2 (Take over keeps the history).
3. 3 ("Continue in a terminal") once the owner picks between a pane of ottospace's and the
   user's own terminal app.
4. 1b and 1d, by the owner's pick.
