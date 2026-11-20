# Human attention: deciding, not hunting

A design note, 2026-11-19, from a brainstorm with the owner. **Nothing here is built**; the owner
has not picked among the proposals. The principles (`AGENTS.md` Principles, `README.md`, the site):

> Human attention is the scarcest resource. ottospace interrupts only for what needs the person,
> brings the context with the question, batches the rest, and never makes the person hunt for
> what's next.

And (wording proposed, not yet approved):

> ottospace stays out of your way. Nothing moves, covers what you are working in, or takes the
> keyboard unless you asked for it; every mode has an obvious way out; and nothing decorative ever
> makes the fast path slower.

Tiebreaker (proposed): ottospace offers and waits. It may show that something needs you; it
never acts on your screen, focus or keyboard to make you look. So a Decisions card never moves
under the pointer, no animation holds a key back, and Esc always leaves.

## The problem

The owner: "in my workday I end up with lots of random sessions, scattered across my computer,
and it's just going through each one to unblock or think through the next decision. One energy
sink is just figuring out what to do next."

Two costs: finding what waits (the hunt), and loading each back into the head (the switch).

## What exists on main

Checked against the code, 2026-11-19.

| Building block | State | Where |
|---|---|---|
| Needs you: a task state; the Tasks section groups by it, needs-you first | Built | `ottospace-core/src/tasks.rs`; reference "Tasks inbox" |
| The rail of open tasks; after a Done the most urgent remaining is in front | Built | `model/rail.rs`, `Model::front` (`model/spaces.rs`) |
| Next task that needs you: ⌘J (⌘⇧N before), and a click on the status bar's `● 1 needs you` | Built | reference "Status bar", `status_bar.rs` |
| ⌘J, next thing needing you / "where next"; ⌘⇧N the note-or-quick-task box (§6) | ⌘J built as next task; ⌘⇧N unbound until the box (§6). ⌘⌥N is the new-task box's new-worktree key; no "resume a session" action | `main.rs` keymap |
| Priority (1–9, `P1` drawn), snooze (`snoozed_until`), hold | Built | `tasks.rs` |
| Quick note on ⌘⇧N with context pills (§6): ↵ later, ⌘↵ now in its own space; `note` gives what it was filed with | Built behind `overlay_panels` | `model/quick.rs`, `capture::Note`, `collect.rs` `keep_note` |
| New-task box centred over the dimmed panes; ↵ grows it into the drawer (canvas NewTaskFly) | Built behind `overlay_panels` | `model.rs` `compose_centred`, `ottospace-core/src/fly.rs`, `ottospace/src/fly.rs` |
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
| A toast of ottospace's own: notices as cards over the panes' bottom right, above the status bar, so no pane moves | **Not on main.** Open in #299. "Toast" on main is only a kind of other apps' window that hosting skips | `rules.rs`; #299 |
| App-window hosting: an inactive window is parked off-screen with one pixel on a display, its pane an SCK stream | Built | [hosting-windows.md](../hosting-windows.md) |
| Focus blocks that hide Slack, with an agent that can break through | Designed (roadmap line), no note of its own | reference roadmap; [plan.md](../1-initial-brainstorm/plan.md): Phases row F, Constraints (Notifications) |

## Proposals, in build order

All **proposed**.

### 1. Decisions session

A special, ephemeral workspace that walks the person through everything waiting on them, one card
at a time. Made for a block: "15 minutes of decisions".

- **A decision is a workspace.** The owner: "I picture the decision would be a special workspace
  with a single pane (that we could have side conversations or still spin up other panes for
  reference)." One pane fills the window: the exposition on one side, the question and options on
  the other. Shown in the full chrome. Mock:
  [Decision workspace](https://claude.ai/artifact/VvJWQgWzXazQhijmCJ9dcv#artboard-DecisionWorkspace).
- **It reuses the chrome.** The owner: "right rail / conversations / side conversations should go
  through that existing right pane ui". No conversation UI of its own: talking a card over opens
  the right drawer on the card's task (its conversation and btw column; ⌘⇧R, or the task on the
  right rail, which shows the card's task). Ordinary splits and the ⌘E picker add panes ("if I
  wanted to check something in the terminal"); they close with the card, and the next card starts
  as one pane again.
- **Evidence drill-in.** The owner: "I could click on them to see the raw data in a separate pane
  (using an existing or incoming file view)." A card carries a list of evidence, each item a
  source plus a location (a file and its rows or lines, a URL or app view, a record id, a session
  message), attached to the claim it supports. A click or a key opens it in a pane beside the
  card, in the ordinary viewer for its kind, never a one-off view. The framework is the list, not
  the viewers: any source that can name a location can offer evidence.
  - The file viewer on main ([file-viewer.md](../file-viewer.md)) shows a regular file of at
    most 5 MB read-only, markdown drawn, code with line numbers, other text (a CSV too) plain.
    `OpenFile` takes no line: opening at the cited rows, and marking them, is new.
  - The card's agent or presenter supplies the list. Paths in it are untrusted data: opened
    read-only, only inside what the person could already open from the task (its repository,
    listed repositories and worktrees, folders the agent was given). Today the viewer opens any
    readable absolute path; evidence would be narrower.
- **A card**: the question; a three-line recap of where the task stands; the options; the
  agent's recommendation, marked. Keys 1–4 pick and ↵ sends (owner, 2026-11-20); the
  agent then resumes.
- **Order**: an agent blocked now, then deadline, then age. Each card says why it is where it is.
- **Flow that never slows**: "3 of 7"; on an answer the seal plays while the next card is already
  usable; the options' details animate in.
- **Visual aids.** The owner: "a code snippet, or a before/after visual, an explanatory visual";
  on a data question, "show the disconnect so that there is context". At most one aid per card,
  chosen for its decision: the rows in dispute with the difference marked; a flow sketch and the
  diff being approved; before/after pictures as the options. A plain question gets none. Made
  before the card is shown, by the task's agent or the presenter role (§2), from task content
  (untrusted data). Mock:
  [Decision aids](https://claude.ai/artifact/VvJWQgWzXazQhijmCJ9dcv#artboard-DecisionAids).
- **Defer with a default**, for low-stakes ones: "if I haven't answered by 3pm, take the
  recommendation". Shown on the card and the task until it fires.
- **Where next** (⌘J): one card kind, the choice of what to focus on next (§7).
- **Agents hand over decision-ready questions** (question, options, recommendation, why); the
  guide says so. For a session that stopped without asking, a cheap model writes the recap and
  question: a model call ottospace does not make today.
- Core: a deadline on a task, the queue's order as a pure tested function, the deferred default
  as a timer fed time.

### 2. Agents and roles

The owner: "an agent as a combination of a runner, model, and any pre-baked context. ie: Bill:
claude code, opus 5.5, access to ~/notes/projects. [...] change Bill's model to fable 5.2, and
then everywhere Bill is used, it gets updated."

- **An agent** is a name, a runner, a model and pre-baked context (folders it may read, standing
  instructions). Defined once; referenced by name everywhere, so changing Bill's model changes
  every use of Bill.
- **It layers on the spec**: it names a `RunnerId` and gets what its `RunnerSpec`
  (`ottospace-core::runner`) allows. It adds no runner; "a runner is its spec" holds.
- **What exists.** `@agent` and the new-task box's who-pick name a runner, or a runner and a
  model (`default_agent`, `agent_models`). A saved agent (`~/.ottospace/agents/<name>.md`, the
  format in [routines.md](../routines.md) §2) is offered by name, but "↵ files it; nothing
  starts". Starting one, and its context, is not built.
- **A role** is a job ottospace itself knows and calls: the secretary (§3); the presenter, which
  prepares decision cards and their visual aids; perhaps the focus-block watcher and the recap
  writer. A closed, built-in set. Each has a default agent out of the box and can be assigned
  any agent.
- **Ordinary agents are not special**: tagged on a task or picked in the new-task box.
- **An agents pane** (later than the roles themselves). The owner: "an agents pane that can show
  the different agents, which ones are assigned as assistants, and what tasks/workspaces they are
  working on." Each agent's name, runner, model and context, its roles, and the tasks and
  workspaces it works in now. The place to change an agent's model or assign a role.
- **Audit and feedback.** The owner: "audit and review actions it took on my behalf (and give
  feedback good or bad...)". Every act of a role on the person's behalf goes in a log: what, when,
  why (the rule or trigger), what it read, the result, and Undo where the act can be undone. Each
  entry takes a good/bad mark and an optional note. Shown in the agents pane and as each role's
  "what it did" list; the shelf of offers is that list filtered to results.
  - Feedback to the agent: marked entries are handed to it as examples. A change to its standing
    instructions or skills is proposed and the person approves it; nothing rewrites itself.
  - Kept local, under `<home>`, private as the event log (`event_log.rs`, 0600). That log holds
    names, counts, ids and versions, never terminal or transcript text, titles, meetings, passes,
    paths or file contents. The audit log needs more (what it read, why): it may name a task, a
    repository and a PR; never message or transcript text, meeting titles, window titles or
    passes. Fed back to an agent, entries are data, never instructions.
- **Comparing two agents in a role** (the owner: "A/B test opus 5.5 vs gpt astra"):
  - *Side by side, blind*: both prepare the same card; the person picks the better. Costs double.
  - *Alternate by day or task*, with outcome measures: offers taken vs dismissed, decisions
    answered without opening the detail, how often the recommendation was picked, time blocked,
    spend; and the audit log's good/bad marks, a direct signal.
  - Few decisions a day make small samples: blind picks and marks beat the derived numbers. The
    measures stay on the Mac.

### 3. A secretary agent

A role (§2). The owner: "a 'superfan' or a 'secretary' agent that monitors the tasks, sessions,
etc and oversees that things are moving. [...] if you automatically delegate tasks, you end up with
a giant wall of slop and text that you are mentally unprepared to go through."

- **One overseer**: reads tasks, sessions, calendar and due dates, keeps things moving. Does not
  do the work, never answers for the person.
- **Priority and due date** on tasks: set by the person, or inferred with its reason shown ("due
  today: from the Slack message"). Today a task has a priority and no due date.
- **The output contract answers noise.** A speculative attempt is invisible until it ends.
  Failed: one line, what was tried and what stopped it. Succeeded: two or three lines on the
  approach, plus the thing (a draft, a PR, a decision card). Never a transcript. It sits on a
  shelf of offers, not in Tasks, until the person takes it.
- **Levels the person sets**: Watch (notices only: "idle 2 h, blocked on you", "needs a
  rebase"); Prepare (recaps and recommendations for pending decisions, research ahead); Act
  (dispatches work inside rules: reversible, nothing outward-facing, a spend cap).
- **Chores it would take**, each seen when coordinating many agents by hand: restart an agent
  that died or lost its helpers; notice a PR waiting on checks or conflicting; rebase; chase a
  stale review; rank the where-next card; focus mode's break-through watch (§7).
- **Self-throttle**: it counts how many of its offers the person takes. A low rate, fewer offers.
- **Trust**: its delegated authority is explicit and narrow. A relayed instruction is not the
  person's consent for an outward-facing or irreversible act (merge, send, delete): those go to
  the Decisions queue. Everything it reads from tasks, sessions and messages is untrusted data.

### 4. Stray-sessions inbox

Elsewhere lists only sessions of listed repositories. Widen it: every Claude Code and Codex
session on the machine that is waiting on the person, wherever it was started. Track one, and it
joins the Decisions queue.

- The readers already walk the runners' session folders and keep the sessions under the given
  directories; this widens discovery beyond the listed repositories, still read-only, still
  names, never content.
- "Waiting on the person" must be read from the file (an open ask, a permission request, a turn
  that ended on a question), not guessed. Resume stays limited to listed repositories, as today.

### 5. Slack ping → draft reply

The owner's case: "A random Slack ping comes in that will need to be addressed by the end of the
day. I want to spin up an agent to research, it needs some clarification/decisions, I address
those, and then have a draft reply ready to go."

1. The person picks the message on screen (Pick, once built; Capture today), or a watcher
   surfaces it (§7).
2. "Handle this" starts a quick task with a deadline of today.
3. The agent researches. Its questions land in the Decisions queue, ranked by that deadline.
4. The result is a draft-reply card: Copy, or open the thread in Slack.

ottospace never sends a message. The person sends it.

### 6. A quick note

The owner: "I was in a meeting, wanted to take a note of something to follow up on. Ideally, I
could press a shortcut (maybe command+shift+n), add the note, and it could also have a backpointer
to transcript and context if available (along with current meeting title)." (Not "capture": that
is the annotated screen capture, [capture.md](../capture.md).)

- **One key, wherever ottospace has the keyboard.** A one-line box; type, ↵, gone. Nothing else
  moves, no workspace switch. Inside a live app window (Zoom) ottospace's shortcuts do not fire:
  the person first leaves the app by the existing way out (the return key, F18), then presses it.
  Decided by the owner: "it's fine if this only works when not in a live app window".
- **Not planned**: a system-wide hot key for it (as the return key and the capture key are,
  `ottospace-mac/src/hotkey.rs`).
- **Saved**: the words, the time, and back-pointers when there are any: the transcript being
  written (its file in `transcripts_dir` and the time reached: each line there carries its local
  time; the file is built, behind `transcripts`), the meeting on the calendar now, the workspace
  and task in front.
- **Filed as a task not started**, as the new-task box's ⇧↵ does today ("adds it for later:
  nothing starts"; a saved agent's ↵ files the same way). It shows under Tasks and later in the
  where-next list, where one key starts an agent on it, context attached, or clears it.
- **Context pills.** The owner: "we should show pills when we create a task or note so it's clear
  what context it's bringing in." In the box, before ↵, each piece of context to be attached is a
  pill: "Meeting · <title>", "Transcript · 10:42", the workspace or task in front, picked items
  (Pick's chips, [pick-and-app-profiles.md](../pick-and-app-profiles.md)). Each has × to leave it
  out; nothing without a pill is attached. Every task made from the box, not only notes. Drawn
  from what the app holds; a pill's meeting title is draw-only, never in the state.
- **Privacy, decided (owner): on request, not broadcast.** A note's record holds the meeting title
  and the transcript pointer. They are given out by a read-only request and in the task's brief,
  quoted and marked untrusted, as the `calendar` request gives titles today; never in the state
  every reply carries, the event log or the unified log. This loosens "titles only via
  `calendar`" by one more read-only request: `AGENTS.md`'s Control socket section changes when it
  is built. Transcript text reached through the pointer is untrusted data for an agent.
- **The key, decided (owner)**: quick note and quick task are one box on ⌘⇧N. ↵ files a note for
  later, nothing starts; ⌘↵ starts it as a task now, in its own temporary workspace. Part of the
  decided remap: ⌘N new task, ⌘⇧N this box, ⌘⌥N resume a session, ⌘J next thing needing you. On
  main ⌘J is "next task that needs you" and ⌘⇧N is free.

### 7. Focus mode

Knows focus time from the calendar (focus blocks are already recognised and coloured).

- **Where it starts.** The secretary picks the place: the existing workspace of the task the block
  is for (the block's title, if it names one; else the top of the where-next ranking), or a new
  workspace when none fits. A new one is temporary as + makes one today, but becomes kept once it
  gets a task; closing it when its tasks are done and no panes are left is new. The pick is shown
  with its reason; one key picks another. Started by the person (their key, or their macOS Focus
  turning on), going there is their own act's consequence: it switches. Started by the calendar
  alone, it offers ("Focus time: go to Login loop?") and waits.

- **Distractions removed** during the block. The owner: Slack "can make noise on notifications and show a red dot
  in the dock, both of which can pull me away mentally." Hiding or parking its window is cheap and
  fails for that reason: the sound and the badge still reach the person. The options:
  - **(a) Quit the app for the block, reopen it at the end.** Complete: no sound, no badge. The
    watcher reads Slack through its API or CLI, read-only, so it needs no app running. Costs: the
    person shows as away in Slack; a call or huddle rings nowhere; relaunch time at the end; the
    app's own state (a half-typed message) is the app's to keep.
  - **(b) A macOS Focus for the block.** Silences sounds and banners for all apps; whether it
    hides Dock badges is to verify. The first brainstorm's plan proposed it too (through
    Shortcuts, and covering the Dock). See "macOS Focus" below.
  - **(c) Slack's own "pause notifications".** A write to Slack, seen by others. It conflicts
    with ottospace never writing to Slack: rejected unless that rule changes.
  - **Lean**: (a) for the apps the person names, with (b) alongside for everything else.
  - The phone buzzes unless its own Focus is on (shared Focus: below).
- **Notices held**, ottospace's own too, except what breaks through.
- **A break-through watcher.** An agent reads Slack on a timer against the person's rules (an
  incident, their manager, "urgent"). A match breaks through with its reason. The rest is held.
- **The block ends quietly** (status bar: "5 held"). The owner: "it's really a decision of where
  to focus next... I press command+j to jump to the next thing after a focus block." ⌘J opens the
  Decisions workspace on a where-next card: held messages and waiting decisions in one ranked
  list, the suggested next first. ⌘J again takes the first; the rest stay queued. During a block
  ⌘J stays inside the work and never surfaces what is held. This replaces a separate digest.
- **Immersion.** The shown workspace, whole, on every display it spans (⌘⇧O folds a tab out,
  behind `multi_monitor`, #261/#262). Everything outside it recedes: other workspaces' tabs, the
  drawer and sidebar (into their rails, as ⌘⇧R and ⌘⇧L do today), notices, other apps. A quiet
  timer in the status bar.
- **Decided (owner, 2026-11-19)**: displays the workspace does not use are left alone. Switching
  workspace neither ends the block nor asks: the block is the person's time, immersion follows
  the shown workspace, and the outside world (notices, the named apps, the watcher) stays held.

#### macOS Focus

Known: `/usr/bin/shortcuts run <name>` exists (macOS 26.7.3); it runs a Shortcut the person made.
The rest is unchecked against Apple's documentation: **to verify in a spike on the owner's
macOS version**.

- **Read** whether a Focus is on: `INFocusStatusCenter`, with permission; which one, or only on/off.
- **Follow** one: a Focus filter (`SetFocusFilterIntent`, App Intents), so macOS is the one
  switch (Work Focus on, focus mode on; the phone follows through Focus sharing). Whether a
  binary with no bundle can offer one.
- **Set** one: the Shortcuts "Set Focus" action via `shortcuts run`, against a public API (the
  owner believes one exists). Private interfaces are not acceptable.

### 8. Smaller ideas

- Hold agents' questions for the next Decisions block, unless one has blocked an agent too long.
- A two-line "where we were" on returning to a task.
- Standing rules: "always take the recommendation for this kind of question".
- Attention metrics: time agents sat blocked on the person; decisions per day.
- A meeting transcript's action items become tasks ([transcripts.md](../transcripts.md)).

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
- **Privacy.** `AGENTS.md`'s rules hold (meeting titles and places never in the state); a
  watcher's Slack text and a recap are no more public than a session's title.
- **Who acts.** Answers, defers, adopting a session and quitting an app should be the person's
  alone (`Origin::User`), refused from any other origin, as Done and Later are today.
- **Agents' definitions.** In `settings.json`, or the files `routines.md` already proposes
  (`~/.ottospace/agents/`)? What "may read a folder" means for each runner (Claude Code's
  `--add-dir`, Codex's sandbox), to check per runner. May a role be split between two agents?
- **Deadlines.** Typed by the person, read from the calendar, or set by the watcher ("by end of
  day")? Each needs the field on a task first.
