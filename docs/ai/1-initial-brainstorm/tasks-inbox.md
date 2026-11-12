# Tasks as an inbox

Refines #10 (tasks) with Bryan's direction from 2026-11-11: "the inbox mode did sound a lot like a
more refined version of my tasks idea". Builds on the lifecycle in `quick-capture.md`. Not built.

Source for the inbox idea: the VS Code 1.146 release notes ("Inbox mode for sessions"), read
through a summary, so only the general shape is taken from it.

## What the user sees

The Tasks rail section opens the inbox in the secondary bar. Tasks are grouped, in this order:

| Group | Holds | Lifecycle states |
|---|---|---|
| **Needs you** | An agent waits for an answer, an approval or a review | `needs-you`, `review` with open comments or failing checks |
| **Running** | An agent is working | `running`, `triaged` |
| **Captured** | Offloaded, not looked at yet | `captured` |
| **On hold** | Paused on purpose | `parked` |
| **Done** | Finished, waiting for cleanup | `done` |

Within a group: priority first, then oldest first.

Keys on the selected row:

| Key | Action |
|---|---|
| ↵ | Open the task: its tab comes forward |
| `s` | Snooze: hide until a time (1 hour, tomorrow morning) |
| `h` | Put on hold |
| `x` | Dismiss from the inbox. The task keeps running; it only leaves this list |
| `u` | Undo the last snooze, hold or dismiss |

- **Pop next** (a global key, and an action-deck tile): opens the first task under Needs you.
  Snoozed and dismissed tasks are skipped.
- **Status bar:** the Needs-you count. Nothing else from tasks is shown there.
- **A snoozed task comes back** on its own when its time arrives, or at once if its state changes
  to Needs you for a new reason.
- **Text capture.** The picker (⌘T / ⌘E) gets a "New task" entry: type one line, ↵ files it as
  `captured` and returns focus, ⌘↵ also opens it. It is quick capture without the screenshot.

## Core model

```
Task { id, title, state, priority, snoozed_until, dismissed, tab }
```

- The inbox order is a pure function of the task list and the current time. Time comes in with
  the action (the core has no clock).
- `PopNext`, `Snooze`, `Hold`, `Dismiss`, `Undo` are actions, each with a unit test.
- A task points at a tab; the tab's panes are the task's workspace.

## First PR

Core `Task` list, ordering and the five actions; socket calls `task_add`, `task_set_state`,
`task_list`, `task_pop`; a read-only Tasks section that draws the groups. No worktrees, no agents
yet: tasks are added and moved through the socket, which is enough to test the inbox itself.

## Later, in this order

1. Agents report their own state (running, needs-you) through the socket.
2. Review actions on the action deck: "Fix checks", "Address comments", sent to the task's agent
   without moving focus. Needs a `gh` poller for check and comment state.
3. `ottospace ctl adopt` from any terminal: make a task from the current directory and reopen its
   conversation in a new tab.
4. Worktree lifecycle and cleanup (`quick-capture.md`).

## Open questions

1. Snooze choices: are "1 hour" and "tomorrow morning" enough?
2. Should Done tasks clean up on their own after a delay, or wait for a key?
3. Where do tasks persist? (Still open from the plan: `~/notes` files or ottospace's own state.)
