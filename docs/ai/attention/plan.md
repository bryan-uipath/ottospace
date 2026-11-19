# Human attention: deciding, not hunting

A design note. 2026-11-19, from a brainstorm with the owner. **Nothing here is built.** The
owner has not picked among the proposals yet.

The principle (in `AGENTS.md` under Principles, `README.md` and the site):

> Human attention is the scarcest resource. ottospace interrupts only for what needs the person,
> brings the context with the question, batches the rest, and never makes the person hunt for
> what's next.

Its companion (wording proposed, not yet approved by the owner):

> ottospace stays out of your way. Nothing moves, covers what you are working in, or takes the
> keyboard unless you asked for it; every mode has an obvious way out; and nothing decorative ever
> makes the fast path slower.

When the two conflict (tiebreaker, wording proposed): "When the two conflict, ottospace offers
and waits. It may show that something needs you; it never acts on your screen, focus or keyboard
to make you look."

For this note: a Decisions card never moves under the pointer, its animations never hold a key
back, Esc always leaves the session, and focus mode changes nothing the person did not ask for.

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
| The rail of open tasks; after a Done the most urgent remaining is in front | Built | `model/rail.rs`, `Model::front` (`model/spaces.rs`) |
| Next task that needs you: ⌘⇧N, and a click on the status bar's `● 1 needs you` | Built | reference "Status bar", `status_bar.rs` |
| ⌘J | **Not in the repository.** No code or doc names it; ⌘⇧N is the "what's next" key today | grep, 2026-11-19 |
| Priority (1–9, `P1` drawn), snooze (`snoozed_until`), hold | Built | `tasks.rs` |
| A task's deadline | **None.** Tasks have no due time (they keep when they started and finished, and a snooze's end) | `tasks.rs` |
| Ask cards (`AskUserQuestion`): options, Other…, number keys 1–9 pick a row, ↵ answers, Esc declines | Built (Claude Code) | `ottospace-core/src/ask.rs` |
| btw: a side question to the running agent in a column beside the conversation; "Use as my answer" fills the card's Other… | Built (Claude Code, not Codex) | `model/btw.rs`; agent-runners.md "Side questions" |
| Done and Later; Done's seal, fold and land: a ghost of the drawer plays over a model that has already moved on, and Undo stops it at once | Built (#278) | `model/finish.rs`, `ottospace-core/src/seal.rs`; [done.md](../done.md) |
| Calendar feed: busy/free/tentative per entry; focus blocks (a block with no call link whose title starts with the word `Focus` or `Deep Work`) green by default; the next meeting in the status bar | Built | `ottospace-core/src/calendar.rs`; [meeting-timeline.md](../meeting-timeline.md) |
| Session readers: Claude Code and Codex session files, read-only; for listing, names, counts and times leave the crate, never content (exceptions: the title; the drawer's watch view, `Cursor::with_edits`, reads more) | Built | `ottospace-sessions`; [sessions.md](../sessions.md) |
| Elsewhere: sessions of **listed repositories** no task has adopted; Track makes one a task; a Running task that follows its session moves to Needs you after `session_quiet_minutes` quiet (0: never) | Built | reference "Sessions" |
| Capture (⌃⌥⌘Space): a pane frozen, annotated, its text read once, a task's brief | Built | [capture.md](../capture.md) |
| Pick on screen: task context picked in panes | Designed, not built | [pick-and-app-profiles.md](../pick-and-app-profiles.md) |
| Notice bar: one app-level message with Undo; a system notification and sound for a meeting that started | Built | reference "Notice bar", `ottospace/src/calendar.rs` |
| A toast of ottospace's own: notices as cards over the panes' top right, so no pane moves | **Not on main.** Open in #299. "Toast" on main is only a kind of other apps' window that hosting skips | `rules.rs`; #299 |
| App-window hosting: an inactive window is parked off-screen with one pixel on a display, its pane an SCK stream | Built | [hosting-windows.md](../hosting-windows.md) |
| Focus blocks that hide Slack, with an agent that can break through | Designed (roadmap line), no note of its own | reference roadmap; [plan.md](../1-initial-brainstorm/plan.md): Phases row F, Constraints (Notifications) |

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

### 2. A secretary agent

The owner: "a 'superfan' or a 'secretary' agent that monitors the tasks, sessions, etc and
oversees that things are moving. Having a concept of a priority or due date for tasks could help
make good decisions, and the secretary could dispatch tasks that help. One challenge in the past
was 'noise': if you automatically delegate tasks, you end up with a giant wall of slop and text
that you are mentally unprepared to go through."

- **Role**: one overseer. It reads the tasks, sessions, calendar and due dates and keeps things
  moving. It does not do the work itself and never answers for the person.
- **Priority and due date** on tasks: set by the person, or inferred with its reason shown
  ("due today: from the Slack message"). The Decisions queue orders by them. Today a task has a
  priority (1–9) and no due date.
- **The output contract is the answer to noise.** A speculative attempt stays invisible until it
  ends. Failed: one line, what was tried and what stopped it. Succeeded: two or three lines on
  the approach, plus the thing itself (a draft, a PR, a decision card). Never a transcript;
  detail only on request. Speculative work sits on a separate shelf of offers, not in Tasks,
  until the person takes it.
- **Levels the person sets**: Watch (notices only: "idle 2 h, blocked on you", "its agent
  stopped", "needs a rebase"); Prepare (writes the recap and a recommendation for each pending
  decision, researches ahead); Act (dispatches work inside rules: reversible, nothing
  outward-facing, a spend cap).
- **Chores it would take**, each seen when coordinating many agents by hand: restart an agent
  that died or lost its helpers; notice a PR waiting on checks or conflicting; rebase; chase a
  stale review; the end-of-block digest; focus mode's break-through watch (§5).
- **Self-throttle**: it counts how many of its offers the person takes. A low rate, fewer offers.
- **Trust**: its delegated authority is explicit and narrow. A relayed instruction is not the
  person's consent for an outward-facing or irreversible act (merge, send, delete): those go to
  the Decisions queue. Everything it reads from tasks, sessions and messages is untrusted data.

### 3. Stray-sessions inbox

Elsewhere lists only sessions of listed repositories. Widen it: every Claude Code and Codex
session on the machine that is waiting on the person, wherever it was started. Track one, and it
joins the Decisions queue.

- The readers already walk the runners' session folders and keep the sessions under the given
  directories; this widens discovery beyond the listed repositories, still read-only, still
  names, never content.
- "Waiting on the person" must be read from the file (an open ask, a permission request, a turn
  that ended on a question), not guessed. Resume stays limited to listed repositories, as today.

### 4. Slack ping → draft reply

The owner's case: "A random Slack ping comes in that will need to be addressed by the end of the
day. I want to spin up an agent to research, it needs some clarification/decisions, I address
those, and then have a draft reply ready to go."

1. The person picks the message on screen (Pick, once built; Capture today), or a watcher
   surfaces it (§5).
2. "Handle this" starts a quick task with a deadline of today.
3. The agent researches. Its questions land in the Decisions queue, ranked by that deadline.
4. The result is a draft-reply card: Copy, or open the thread in Slack.

ottospace never sends a message. The person sends it.

### 5. Focus mode

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
    Whether it hides Dock badges is to verify. The first brainstorm's plan already proposed this
    (a macOS Focus through Shortcuts, and covering the Dock). See "macOS Focus" below for how
    ottospace could read, follow or set one.
  - **(c) Slack's own "pause notifications".** A write to Slack, seen by others. It conflicts
    with ottospace never writing to Slack: rejected unless that rule changes.
  - **Lean**: (a) for the apps the person names, with (b) alongside for everything else.
  - The phone still buzzes unless its own Focus is on: out of ottospace's reach, unless the
    Focus is shared across devices (below).
- **Notices held.** ottospace's own notice bar and notifications wait for the block's end,
  except what breaks through.
- **A break-through watcher.** An agent reads Slack on a timer against rules the person sets (an
  incident, their manager, "urgent"), so nothing needs the person's mental energy. A match
  breaks through with its reason. Everything else becomes one digest when the block ends.
- **Immersion.** The unit is the shown workspace, whole: all its panes stay fully on screen,
  on every display it spans (tabs folded out with ⌘⇧O, behind `multi_monitor`: built in #261 and
  #262; the folded-out window's own drawer and the arrows across are not). What recedes is
  everything outside it: other workspaces' tabs, the drawer and sidebar (collapsed into their rails, as ⌘⇧R and ⌘⇧L do today),
  notices, other apps. A quiet timer in the status bar.
- **Decided (owner, 2026-11-19)**:
  - Displays the shown workspace does not use are left alone: no blanking, no dimming.
  - The person may switch workspace during a block. Switching does not end the block, and
    ottospace does not ask. The block belongs to the person's time, not to one workspace:
    immersion applies to whichever workspace is shown. What stays held for the whole block is
    the outside world: notices, the named apps quit, the watcher.

#### macOS Focus

Known: `/usr/bin/shortcuts run <name>` exists (checked on macOS 26.7.3); it runs a Shortcut the
person made, and cannot make one. Nothing else below is checked against Apple's documentation
yet. **To verify in a spike on the owner's macOS version**:

- **Read whether a Focus is on**: `INFocusStatusCenter`, after the person grants permission.
  Whether it says which Focus, or only on or off.
- **Follow a Focus**: a Focus filter (`SetFocusFilterIntent`, App Intents), so macOS is the one
  switch: the person's Work Focus turns on, and ottospace enters focus mode. With Focus sharing
  the phone follows too. Whether a binary started from a terminal, with no bundle, can offer one
  (the app bundle may be needed).
- **Set a Focus from ottospace**: the Shortcuts "Set Focus" action through `shortcuts run`
  (a Shortcut the person makes once), against a public API (the owner believes one exists: the
  spike confirms or rules it out). Private interfaces are not acceptable.

### 6. Smaller ideas

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
