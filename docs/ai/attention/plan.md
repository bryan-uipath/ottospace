# Human attention: deciding, not hunting

A design note. 2026-11-19, from a brainstorm with the owner. **Nothing here is built.** The
owner has not picked among the proposals yet.

The principle (in `AGENTS.md`, `README.md` and the site):

> Human attention is the scarcest resource. ottospace interrupts only for what needs the person,
> brings the context with the question, batches the rest, and never makes the person hunt for
> what's next.

## The problem

The owner: "in my workday I end up with lots of random sessions, scattered across my computer,
and it's just going through each one to unblock or think through the next decision. One energy
sink is just figuring out what to do next."

Two costs: finding what waits (the hunt), and loading each one back into the head before
deciding (the switch). The proposals below cut both.

## What exists on main

Checked against the code on 2026-11-19. Each proposal builds on these.

| Building block | State | Where |
|---|---|---|
| Needs you: a task state; the Tasks section groups by it, needs-you first | Built | `ottospace-core/src/tasks.rs`; reference "Tasks inbox" |
| The rail of open tasks, the most urgent in front | Built | `model/rail.rs`, `Model::front` (`model/spaces.rs`) |
| Next task that needs you: ⌘⇧N, and a click on the status bar's `● 1 needs you` | Built | reference "Status bar", `status_bar.rs` |
| ⌘J | **Not in the repository.** No code or doc names it; ⌘⇧N is the "what's next" key today | grep, 2026-11-19 |
| Priority (1–9, `P1` drawn), snooze (`snoozed_until`), hold | Built | `tasks.rs` |
| A task's deadline | **None.** Tasks have no due time; snooze is the only time on a task | `tasks.rs` |
| Ask cards (`AskUserQuestion`): options, Other…, number keys 1–9 pick a row, ↵ answers, Esc declines | Built (Claude Code) | `ottospace-core/src/ask.rs` |
| btw: a side question to the running agent in a column beside the conversation; "Use as my answer" fills the card's Other… | Built (Claude Code, not Codex) | `model/btw.rs`; agent-runners.md "Side questions" |
| Done and Later; the user's Done emits `Effect::Seal` | Model built. **The shell draws no seal yet** (`main.rs`: `Effect::Seal { .. } => {}`) | `model/finish.rs`; [done.md](../done.md) |
| Calendar feed: busy/free/tentative per entry; focus blocks (title starts `Focus` or `Deep Work`) coloured green; the next meeting in the status bar | Built | `ottospace-core/src/calendar.rs`; [meeting-timeline.md](../meeting-timeline.md) |
| Session readers: Claude Code and Codex session files, read-only; names, counts and times leave the crate, never content (one exception: the title) | Built | `ottospace-sessions`; [sessions.md](../sessions.md) |
| Elsewhere: sessions of **listed repositories** no task has adopted; Track makes one a task; an adopted task moves to Needs you after `session_quiet_minutes` quiet | Built | reference "Sessions" |
| Capture (⌃⌥⌘Space): a pane frozen, annotated, its text read once, a task's brief | Built | [capture.md](../capture.md) |
| Pick on screen: task context picked in panes | Designed, not built | [pick-and-app-profiles.md](../pick-and-app-profiles.md) |
| Notice bar: one app-level message with Undo; a system notification and sound for a meeting that started | Built | reference "Notice bar", `ottospace/src/calendar.rs` |
| A toast of ottospace's own | **None.** "Toast" in the code is a kind of other apps' window that hosting skips (`rules.rs`) | `rules.rs` |
| App-window hosting: an inactive window is parked off-screen, its pane an SCK stream | Built | [hosting-windows.md](../hosting-windows.md) |
| Focus blocks that hide Slack, with an agent that can break through | Designed (roadmap line), no note of its own | reference roadmap; [plan.md](../1-initial-brainstorm/plan.md) §F |

## Proposals, in build order

All **proposed**.

### 1. Decisions session

An ephemeral tab, full window, that walks the person through everything waiting on them, one
card at a time. Made for a block: "15 minutes of decisions".

- **A card**: the question; a three-line recap of where the task stands; the options; the
  agent's recommendation, marked. Keys 1–4 answer (the ask card's number keys already exist), and
  the agent resumes at once.
- **Order**: an agent blocked right now first, then by deadline, then by age. Each card says why
  it is where it is (see Open questions).
- **Talk it over**: the btw column, attached to the card, asks the agent about an option without
  answering. "Use as my answer" works as it does today.
- **Flow that never slows**: "3 of 7" progress; on an answer the Done seal plays while the next
  card is already in place; the options' details animate in. No animation holds a key back.
- **Defer with a default**, for low-stakes ones: "if I haven't answered by 3pm, take the
  recommended option". The default is shown on the card and on the task until it fires.
- **New requirement on agents**: hand over decision-ready questions (question, options,
  recommendation, why). The guide says so. For a session that just stopped without asking, a
  cheap model writes the recap and the question; that needs a model call ottospace does not make
  today.
- Needs from the core: a deadline on a task (none today), the queue's order as a pure function
  with tests, the deferred default as a timer passed in as time.

### 2. Stray-sessions inbox

Elsewhere lists only sessions of listed repositories. Widen it: every Claude Code and Codex
session on the machine that is waiting on the person, wherever it was started. Track one, and it
joins the Decisions queue.

- The readers already scan by directory; this asks for a scan of the runners' whole session
  folders, still read-only and still names, never content.
- "Waiting on the person" must be read from the file (an open ask, a permission request, a turn
  that ended on a question), not guessed. Resume stays limited to listed repositories, as today.

### 3. Slack ping → draft reply

The owner's case: "A random Slack ping comes in that will need to be addressed by the end of the
day. I want to spin up an agent to research, it needs some clarification/decisions, I address
those, and then have a draft reply ready to go."

1. The person picks the message on screen (Pick, once built; Capture today), or a watcher
   surfaces it (§4).
2. "Handle this" starts a quick task with a deadline of today.
3. The agent researches. Its questions land in the Decisions queue, ranked by that deadline.
4. The result is a draft-reply card: Copy, or open the thread in Slack.

ottospace never sends a message. The person sends it.

### 4. Focus mode

Knows focus time from the calendar (focus blocks are already recognised and coloured). During a
block:

- **Distractions removed.** The owner: "the problem with Slack is that it also can make noise on
  notifications and show a red dot in the dock, both of which can pull me away mentally." So
  parking or hiding Slack's window is not enough: the sound and the Dock badge still reach the
  person. Hiding is cheap and instant, and fails for that reason. The options:
  - **(a) Quit the app for the block, reopen it at the end.** Complete: no sound, no badge. The
    watcher reads Slack through its API or CLI, read-only, so it needs no app running. Costs: the
    person shows as away in Slack; a call or huddle rings nowhere; relaunch time at the end; the
    app's own state (a half-typed message) is the app's to keep.
  - **(b) Turn on a macOS Focus for the block.** Silences sounds and banners for all apps.
    Whether it hides Dock badges is **not verified** on macOS. ottospace cannot set a Focus
    itself; `/usr/bin/shortcuts run <name>` exists (checked on macOS 26.7), so a Shortcut the
    person makes once, with the Set Focus action, could. To verify: that action from the CLI, and
    turning it off again.
  - **(c) Slack's own "pause notifications".** A write to Slack, seen by others. It conflicts
    with ottospace never writing to Slack: rejected unless that rule changes.
  - **Lean**: (a) for the apps the person names, with (b) alongside for everything else.
  - The phone still buzzes unless its own Focus is on: out of ottospace's reach.
- **Notices held.** ottospace's own notice bar and notifications wait for the block's end,
  except what breaks through.
- **A break-through watcher.** An agent reads Slack on a timer against rules the person sets (an
  incident, their manager, "urgent"), so nothing needs the person's mental energy. A match
  breaks through with its reason. Everything else becomes one digest when the block ends.
- **Immersion.** One task on screen, other panes dimmed, a quiet timer in the status bar.

### 5. Smaller ideas

- Hold agents' questions for the next Decisions block, unless one has blocked an agent longer
  than a set time.
- A two-line "where we were" on returning to a task.
- Standing rules: "always take the recommendation for this kind of question".
- Attention metrics: how long agents sat blocked on the person; decisions per day.
- A meeting transcript's action items become tasks (transcripts: [transcripts.md](../transcripts.md)).

## Open questions

- **What a watcher may read.** Reading Slack is read-only, always: no send, react or mark-read.
  Which channels and DMs, through which token, and where its rules live. The watcher's own
  reads are listed for the person.
- **Priority must be explainable.** The queue's order is computed from named inputs (blocked
  now, deadline, age, the task's priority), and each card says which put it there. Never a
  model's guess.
- **Recaps.** Where they are made (which model, local or not) and what they may read. Task
  content and session text are untrusted data: a recap quotes and labels it, as a capture's
  brief does, and nothing in it is obeyed.
- **Privacy.** The rules already in `AGENTS.md` hold: meeting titles and places are never in the
  state; a watcher's Slack text and a recap are no more public than a session's title.
- **Who acts.** Answers, defers, adopting a session and quitting an app should be the person's
  alone (`Origin::User`), refused from any other origin, as Done and Later are today.
- **Deadlines.** Typed by the person, read from the calendar, or set by the watcher ("by end of
  day")? Each needs the field on a task first.
