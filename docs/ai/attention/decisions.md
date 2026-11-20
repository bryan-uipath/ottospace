# Decisions: the slices

A plan, 2026-11-19, for proposal 1 of [plan.md](plan.md) (#305), which the owner picked to build
first. Slice 1 is built behind the flag `decisions` (Experimental, off); the rest is planned.

The owner's problem: "lots of random sessions ... and it's just going through each one to unblock
or think through the next decision." Decisions is a special, ephemeral workspace that walks the
person through what waits on them, one card at a time, in a 15-minute block.

## Decided in design (owner)

- **A workspace like any other**, with one pane, the card, filling the window. Its tab reads
  "Decisions · 4 of 7" over a progress bar. It closes itself after the last card.
- **A card**: the task (badge, name, priority), a short recap, the question, its options with the
  agent's recommendation marked. A number key picks and ↵ sends, and the agent resumes; S skips; L
  puts it off. The next card takes keys at once while the answered one seals (Done's seal, #278).
- **Order**: an agent stopped on the person first, then due/priority, then age.
- **No conversation UI of its own**: talking a card over opens the right drawer on the card's task
  (its conversation and btw column, #284/#285; ⌘⇧R or its item on the right rail). The right rail
  shows the card's task. Splits and the ⌘E picker still add panes; they close when the card is
  answered, and the next card starts as one pane again.
- **Opening Decisions is the person's own act.** Nothing opens it for them.
- **⌘J under the flag, now** (owner, 2026-11-19): what needs you next. Cards waiting: Decisions,
  the first card; none: the next task (`TaskGo { id: None }`, as ⌘J does with the flag off); open:
  its card. The core decides which (`Model::next_needs_you`).
- **Evidence drill-in** (later): a card lists its evidence, each a source and a location; a click
  opens it in a pane with the ordinary viewer at the cited rows (open-at-line: #313).
- **⌘J** opens Decisions, and after a focus block on a "where next" card. **⌘⇧N** goes to the
  quick task and quick note (↵ files, ⌘↵ starts); the new-task box gains context pills. Neither of
  those is this plan's to build.

## What exists on main

| Building block | Where |
|---|---|
| Needs you: a task state; the Tasks section and the status bar's `● n needs you` | `ottospace-core/src/tasks.rs` (`Group::NeedsYou`), `status_bar.rs` |
| ⌘J: the next task that needs you (`Action::TaskGo { id: None }`). ⌘⇧N is bound to nothing (#340) | `ottospace/src/main.rs` `keymap()` (`cmd-j` → `NextTask`) |
| Ask cards: `AskUserQuestion` as a `Form` (1–8 questions, 1–8 options, Other…, multi-select, Review), answered through the runner's structured protocol (`Effect::AgentAnswers`). A runner offers them by `RunnerSpec::structured.asks`, never by its name | `ottospace-core/src/ask.rs`, `model/chat.rs` (`ask_key`, `Action::TalkAsk`), view `ottospace/src/conversation.rs` (`ask_card`) |
| Permission cards: Allow (a) / Deny (d) / Show all (s) / Deny with a message (m) | `model/chat.rs` (`key`, `TalkAnswer`), `conversation.rs` (`chat_card`) |
| Who answers: `TalkSend`, `TalkAnswer`, `TalkAsk`, `TalkExpand`, `TalkDenyNote` are refused from any origin but `User` (`chat::USERS_ALONE`); a client's key or click on a card picks and sends nothing (e2e `a_question_card_sends_the_users_pick`) | `Model::refusal` |
| Priority 1–9 (1 most urgent), snooze (`snoozed_until`), hold. No due date | `tasks.rs` |
| Workspaces: `Owner` (Home, Task, Session, Workbook); a temporary workspace is an unkept workbook; tabs titled "Tab n" or by the task's repository | `model/spaces.rs`, `workbook.rs`, `Model::push_tab_of` |
| The drawer follows the selected space's conversation (`space_subject`, `follow_space`); its right rail lists that space's tasks | `model/spaces.rs`, `model/rail.rs` |
| btw: side questions beside the conversation; "Use as my answer" fills a card's Other… | `model/btw.rs` |
| Done's seal, fold and land: drawn over a model that has already moved on | `ottospace-core/src/seal.rs`, `ottospace/src/seal.rs` |
| Feature flags: off in an e2e run unless named; every flag on in the core's unit tests | `ottospace-core/src/features.rs` |
| Property test: every `Action` from every `Origin`, `MAKERS`, `check` | `model/tests/prop.rs` |

## What a decision is

**Slice 1: a question card.** A task whose native agent waits on an `AskUserQuestion` card: its
conversation's transcript is live and its first card has a `Form`. That is the case the chat
already answers through the runner's protocol, with options to number. A task, not a session of
no task: a card belongs to a task (badge, priority, rail).

Out of slice 1, by kind, each joining the same queue later:

| Kind | Joins | Why not yet |
|---|---|---|
| Permission requests (Allow / Deny) | slice 2 | Their usual keys (owner, 2026-11-20); their `s` (Show all) is also the card's skip key, and Deny with a message types in the drawer's composer |
| A task in a terminal view (TUI) asking | later | ottospace cannot read its question: only its Needs you state |
| A session that stopped without asking (Needs you after `session_quiet_minutes`) | slice 6 | Needs a recap and a question written for it (a model call ottospace does not make today) |
| Pull requests waiting on review (`prs.rs`) | later | Other people's text; their answer is outward-facing (a review), the person's on GitHub |
| Messages held during a focus block | with focus mode (plan.md §6) | Focus mode is not built |

## The order

The queue is computed, never guessed: a pure function of named inputs.

1. **The card on show stays first** while it waits. Nothing moves under the person: a more urgent
   card that comes meanwhile is next, not now.
2. Then **blocked now**: an agent stopped on the person. In slice 1 every card is one; a session
   that merely stopped (slice 6) ranks after.
3. Then **due**, the sooner first (slice 4; no task has one today), then **priority**, P1 first.
4. Then **waiting longest**: when the card was first seen waiting (the core's clock, `Tick`).
5. Then the task's id, so two equal cards keep one order.

Each card says why it is where it is, from those inputs alone: "P1 · waiting 12 min".

A card is left out while its task is snoozed (Later), put away (Done, Later), dismissed or gone,
and once dealt with in this run (answered, skipped, put off). A new question of the same task (a
new ticket) is a new card.

## Who may do what

The rules today's ask cards have, unchanged: answering is the user's alone.

| Act | User | Socket | System |
|---|---|---|---|
| Open Decisions (`DecisionsOpen`) | yes | refused | refused |
| Answer, move, pick on its card (`DecisionsAsk`) | yes | refused, as `TalkAsk` is | refused |
| Skip, Later (`DecisionsSkip`, `DecisionsLater`) | yes | refused | refused |
| Close (`DecisionsClose`; ⌘W on the card, ⌘⇧W, its ×) | yes | refused | — |
| Move on after an answer; leave out a card answered elsewhere or a task gone; close after the last | — | — | the core, after every step |

A client may still read the state and split panes in the workspace, as in any other. No Action of
Socket or System opens Decisions, and none of theirs takes the keyboard to it.

## The state

`decisions`, null while none is open (and while the flag is off):

```json
{"tab": 5, "pane": 9, "task": 2, "ticket": 1, "at": 2, "of": 3,
 "queue": [{"task": 2, "priority": 1, "since": 1763560000000}, {"task": 4, "priority": 3, "since": 1763560100000}]}
```

`at`/`of`: "2 of 3". `queue`: the cards left, on show first. Ids, priorities and times only: the
question, its options and the recap are never in it (the card's shape already is, under its chat:
`drawer.native.chats`). Rects of the card's parts come with the shell (slice 1b), under the test
gate as other rects are.

## Slices

Each a pull request that stands alone, the flag off until the owner turns it on.

### 1a. The queue and the workspace (core)

Nothing on screen. `ottospace-core/src/model/decisions.rs`.

- `Feature::Decisions`; `Owner::Decisions`, a space of no task: never kept in `layout.json` or a
  workbook, never on the row once it ends; `Kind::Decision`, a pane only `DecisionsOpen` makes.
- `Model::decision_queue`: the order above. `DecisionsOpen` (with a non-empty queue: a tab of
  `Owner::Decisions` holding one `Kind::Decision` pane, on show; open already: shown),
  `DecisionsAsk { task, ticket, act }` (the card's `ask::Act`; a number picks; ↵
  sends), `DecisionsSkip { task, ticket }`, `DecisionsLater
  { task, ticket, until }` (the task snoozed until then: one hour, the shell's `SNOOZE_MS`),
  `DecisionsClose`. Each names the card it is for: one that moved on meanwhile does nothing.
- After every step (`settle_decisions`): the card shown is the first of the queue; one answered
  here or elsewhere, or whose task ended, leaves it and the next shows; after the last card it
  closes. Its tab reads "Decisions · n of m". Nothing moves under the person:
  - Panes the person added beside the card close on their own answer, skip or later; a card that
    moves on by another's step (answered elsewhere, its agent ended) leaves them be.
  - When it ends, those panes go with their tab to the card task's workspace, still on show; with
    none, the tab it was opened from shows again; in the background nothing on show changes.
  - A second tab made there goes to a workspace, as a stray tab does: Decisions has one tab.
  - While the person types in the drawer to the card's task, another's step moves no card: it is
    held until their own next step.
- The drawer follows the card's task (`space_subject`), so ⌘⇧R talks it over and the right rail
  shows it.
- Tests: unit tests for each rule; property makers for the five actions; `check`: the shown card
  is the queue's first; no Decisions space with an empty queue, a missing pane, or the flag off;
  one `Kind::Decision` pane at most, in its workspace; no Socket or System step opens it.

### 1b. The card on screen (shell)

- The card pane draws the existing ask card (`conversation.rs` `ask_card`, its actions as
  `DecisionsAsk` when hosted there) under a header of `ui::` parts: the task's badge, name and
  priority, why it is first. The tab's "Decisions · n of m" with a thin progress bar.
- Keys on the card pane: 1–9 pick, ↑↓ j k move, space picks, ↵ sends, ⇥ between questions, S
  skips, L later, Esc closes Decisions. Other… opens the drawer on the card's task with the keys
  there (its words are typed where the composer is).
- The seal (#278's) plays over the answered card while the next one already takes keys; at once
  under Reduce Motion.
- To open it: the palette's "Decisions" and **⌘J** under the flag. ⌘J is next task today
  (`keymap()`, #340); under the flag it opens Decisions first, else the next task: one rule. ⌘⇧N is
  untouched here (the quick task and note take it in their own work).
- e2e, under `OTTOSPACE_FEATURES=decisions`: two waiting tasks → open → answer → next → closes;
  skip; later; a client is refused. A `scripts/shots/` scenario with made-up tasks.
- Docs: `docs/reference.md`, the cheat sheet, `site/docs.html`, each as behind the flag.

### 2. Permission requests join

Allow / Deny cards in the queue, drawn by the permission card's view, with their usual keys
(owner, 2026-11-20). Core test: a permission card's place in the order; its Allow is never a number key's
reflex (a cut request still needs Show all first).

### 3. The recap and the recommendation

The card's "where the task stands" (three lines) and the recommended option marked. From the
agent's own words first: the guide asks agents for decision-ready questions (question, options,
recommendation, why), and an option the agent marks "(Recommended)" is drawn as such. Quoted and
labelled as the agent's (untrusted), as a capture's brief is.

### 4. Due dates on tasks

A `due` field (typed in the new-task box, `due:3pm`; shown on the row; kept). The order's step 3
reads it; a card says "due 3 pm".

### 5. Defer with a default

"If I haven't answered by 3 pm, take the recommendation": a timer fed by `Tick` (core: the
deferred answer as a pure rule of `now`), shown on the card and the task until it fires. Its answer
is the person's own (they set it), sent once; Undo while it shows.

### 6. Sessions that stopped

A task's session quiet on the person, with no question: a card whose recap and question a cheap
model writes. Its model, what it may read, and where it runs are the owner's to pick (plan.md
"Recaps").

### 7. One visual aid and the evidence list

At most one aid per card (the rows in dispute, a diff, before/after pictures), made before the
card shows by the task's agent or the presenter role (plan.md §2). Evidence: a list of source and
location; a click opens it beside the card in the ordinary viewer at the cited rows (open-at-line,
#313). Core: an `Evidence { source, location }` list on the card; the click is `OpenFile` with a
line, beside the card pane, closing with the card as any added pane does. Paths are untrusted: opened read-only, only inside what the task
may already open.

### 8. ⌘J as "what's next"

After a focus block (plan.md §6), ⌘J opens Decisions on a "where next" card: held messages and
waiting decisions in one ranked list, the suggested next first; ⌘J again takes it. Needs focus mode.

### Later

Hold agents' questions for the next block unless one waited too long; standing rules ("always take
the recommendation for this kind"); attention metrics (time blocked on the person); a secretary
role that prepares cards (plan.md §3).

## Answered by the owner (2026-11-20)

1. **Later** snoozes the task for an hour. Its agent keeps waiting; it is not put away.
2. **Skip** takes the card out of this run only; it is shown again next time.
3. **Permission cards** keep their usual keys in Decisions.
4. **Panes added to a card** close on the person's own answer, Skip or Later (not when a card
   leaves by another's step); when Decisions ends they go to the task's workspace.
5. **A number key picks, ↵ sends**, on every card: no answer is sent on a number alone (changed
   from "sends at once" on a single single-select question). Multi-select and multi-question
   cards are as before.
6. **The recommendation** is the asking agent's only: an option is recommended when the agent
   says so in its label.
