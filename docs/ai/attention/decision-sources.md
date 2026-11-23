# Decisions from any source

Status: design note, nothing built. Written 2026-11-23 against `main` a0c998d3. Extends
[decisions.md](decisions.md) (the Decisions workspace, slice 1 built behind `decisions`) and
§3 and §5 of [plan.md](plan.md). Every example is made up.

The owner: "automatically bring in other 'sources' for decision points - ie, new slack threads
[...] categorized by the secretary agent: do they correspond to one or more projects; are there
additional helpful evidence; and then recommend some next actions. And have these also presented
as 'decisions'. We might want to tag decisions based on their source." And: "sort by priority
(based on priority of the corresponding workstream [...] and are they core to the workstream or
ancillary)."

Goals served (PR #421, proposed): a command center (every decision that is the captain's, from
any source, in one queue); flow and joy (one clear thing to act on per card, never a wall of
text). The secretary is a later role (plan.md §2–3); this note lays the path it will file
through, so it is just another source.

Settled, not reopened here: ottospace's only network is the calendar feed and `gh`; Slack is read
by the user's read-only CLI and nothing ever sends, reacts or edits there; agent-written text never
goes into the notes; dashboards come later through a `present` request of fixed native blocks; no
MCP server.

## 1. Today

| | On `main` |
|---|---|
| What a decision is | Not a record. A derived view: a task whose native agent's live transcript has a question card (`AskUserQuestion`, a `Form`) first, and whose task is not done, snoozed, put away or gone (`Model::asking`, `model/decisions.rs`) |
| Who files one | Nobody. It appears when the agent asks through the runner's structured protocol; no request makes one |
| Its fields | `Waiting { task, ticket, priority, since }`: the task's priority (P1–P9), when first seen waiting (`Seen`, from `Tick`) |
| Where it shows | The Decisions workspace (⌘J under `decisions`; the palette): one card pane (`crates/ottospace/src/decisions.rs`), header "P2 · waiting 12m · its agent asks", the drawer's ask card, Skip (s) and Later (l). "Decisions · 2 of 5" over a progress bar |
| Order | The card on show first; then priority; then waiting longest; then task id (`decision_queue`) |
| How it is answered | 1–9 pick, ↵ sends (`DecisionsAsk` → `ask_act` → `Effect::AgentAnswers`), the agent resumes. Skip: out of this run. Later: the task snoozed one hour (`LATER_MS`). Only `Origin::User`; a client or timer is refused (`USERS_ALONE`) |
| Persistence | None of its own. The run and `waiting` live in memory; Later is the task's `snoozed_until`, kept with the task |
| Socket state | `decisions`: null, or `tab`, `pane`, `task`, `ticket`, `at`, `of`, `queue` (`task`, `ticket`, `priority`, `since`). Ids and times only; the question is under `drawer.native` as a shape, never words |
| Counted | The status bar's `needs you` counts tasks (and in-pane terminal sessions that wait, `session_waits`); the decision itself is not counted apart |

The gap to "any source":

- No record: nothing outside a task's agent can be a decision, and nothing is kept across a quit.
- One source, implied: no badge, no filter, no way to say where it came from.
- No project, core/ancillary, deadline, what it unblocks, expiry, evidence or "why recommended"
  (decisions.md slices 3, 4 and 7 plan some, for agent questions only).
- No filing path, no dedupe (a key is `(task, ticket)`), no caps.
- One kind of answer: words back to the asking agent. No start a task, draft a reply, next
  action, dismiss or "not mine".
- Order knows the task's priority alone; Projects (#407, #409) has an order only its section reads.
- Task `#project` tags are free strings; the Projects section's rows are folders or lone `.md` files. Nothing links
  the two yet.

## 2. The record

A `Decision` in `ottospace-core::decisions` (new module, pure). The agent's own question stays
derived; the queue holds both kinds as one `Card`:

```
enum Card { Asked { task, ticket }, Waits { runner, id }, Filed { id } }
// a task's agent asks | a terminal session waits (#423) | kept on disk
```

A `Card::Asked` gets the fields below by rule (source `agent_session`, its task's project, core,
"an agent waits"); a `Card::Waits` likewise (source `terminal_session`, no project, ancillary).
Both keep today's keys only (answer or go to the pane, Skip, Later): no record to dismiss or
forget. Everything else is a `Filed` decision.

| Field | Type, limits | Notes |
|---|---|---|
| `id` | `u64`, ottospace's | |
| `key` | string ≤ 128, `[A-Za-z0-9:._-]` | The dedupe key, the filer's: `slack:C0MADEUP:1763500000.000100`, `jira:ABC-123`, `routine:dri:availability`. One open decision per `(source, key)` |
| `source` | `Source`, closed set (below) | Drawn as a badge |
| `filed_by` | `socket` or `inbox` (§3) or `ottospace` | Provenance of the filing, not a claim the filer makes |
| `revision`, `filed`, `updated` | `u32`, ms | Each update of the same key is a revision |
| `question` | one line ≤ 160 chars | The one thing to decide |
| `options` | 1–6 | Each: `label` ≤ 60, `consequence` ≤ 160 ("the thread waits until Monday"), `act` (§6) |
| `recommend` | option index or none | |
| `why` | ≤ 300 chars | Why that option. Shown under it, folded on a narrow card |
| `evidence` | ≤ 8 | Each: `kind` (`quote`, `link`, `file`), `text` ≤ 500 (a quote) or a label, `at` (an `https:` URL, or a file and line range), and `provenance`: `source`, `when` (ms), `who` (`a person`, `a bot`, `the filer`). Quotes are other people's text: §7 |
| `projects` | ≤ 3 project refs | A Projects entry's name (§4, Q3). Unknown: kept, shown as unlinked |
| `role` | `core` / `ancillary`, `role_why` ≤ 120, `role_by` (`filer` / `user`) | The filer proposes with a reason; the user's flip sticks for the key, across revisions |
| `due` | ms or none | A deadline. "End of day" is the filer's to turn into a time |
| `blocks` | `task` ids and/or `person: true` | What it unblocks: an agent of ours (checked by the core, never believed) or someone outside (the filer's claim) |
| `expires` | ms or none | Past it the decision closes as `expired` unanswered: a meeting that started, a release that shipped. Default: 7 days after its last update |
| `task` | task id or none | The task it belongs to, when an agent files for its own task |
| `status` | `open`, `snoozed { until }`, `answered { option, at }`, `dismissed`, `not_mine`, `withdrawn`, `expired` | Only the user moves it out of `open`, except `withdrawn` (the filer) and `expired` (the clock) |

### Sources

A closed set, each a `SourceSpec`, asked by capability (as a runner is its spec): never a `match`
on its name outside the table.

| Source | Badge | Filed by | Acts its options may take (§6) |
|---|---|---|---|
| `agent_session` | the runner's glyph | derived (`Card::Asked`); or over the socket naming a live `task` (a TUI agent cannot use the structured ask). The socket cannot tell the caller: the task is a claim, and the card says "filed over the socket" | answer, start task, next action |
| `terminal_session` | the runner's glyph, dim | derived (`Card::Waits`), with sessions-as-tasks slice 5 (#423): "why it waits, go to the pane"; no answering from the card | go to pane |
| `slack` | # | socket or inbox | start task, draft reply, next action, open link |
| `jira` | ◆ | socket or inbox | start task, next action, open link |
| `pr_review` | ⑂ | socket or inbox; later derived from `prs.rs` | start task, next action, open link |
| `calendar` | ◷ | socket or inbox | start task, next action |
| `routine` | ↻ | inbox (an ottobot run), or socket | start task, next action, open link |

A new source: a variant and its `SourceSpec` row (badge, who may file, allowed acts, whether its
text is external), one core test that the table refuses the rest, the guide's list, the SDK's
type. No other code names it.

### Dedupe and updates

- Same `(source, key)` while `open` or `snoozed`: an update. New revision; fields replaced, except
  the user's `role` flip and snooze, which stay. Its place follows the new inputs, except that
  the card on show stays on show (decisions.md rule 1).
- The card on show updated: its text changes in place with a quiet "updated" line; a pick made on
  the older revision is cleared; ↵ names the revision it answers, and a stale one does nothing.
- Same key after `answered`: a new revision opens again, marked "again" (the thread moved on).
- Same key after `dismissed`, `not_mine` or `expired`: kept closed, updated silently, unless the
  update raises urgency (§4: a sooner `due`, or a new `blocks`). As ottobot's alerts: a higher
  level brings it back, the same level does not.
- `decision_withdraw { source, key }`: the filer says it is moot (resolved in the thread). Closes it; the
  card on show leaves as an answered-elsewhere one does.

## 3. Ingest

ottospace never fetches. A filer reads its source with its own tools (the user's read-only Slack
CLI, `gh`, a Jira client) and hands ottospace the record. Two doors, one validation, one core
action (`Action::DecisionFile { record, via, now }`):

| Door | Who | Origin |
|---|---|---|
| `decision_file` on the control socket | An agent in one of our panes, filing for its own task or a thread it was asked about; a routine the user runs by hand | `Socket` |
| The inbox folder `<home>/decisions/inbox/*.json`, one file per filing, read and removed by the shell | An unattended routine (ottobot), the future secretary | `System` |

Why two: routines.md §6 keeps the socket away from unattended runs ("an unattended run gets
neither ottospace's grants nor its socket"): the socket can `type` into a terminal. A drop folder
gives a run one power only, filing. Q1.

Requests (closed set, same-user socket as today):

```
ottospace ctl '{"cmd":"decision_file","source":"slack","key":"slack:C0MADEUP:1763500000.000100",
  "question":"Reply to the made-up rollout thread today, or hand it to Sam?",
  "options":[{"label":"Draft a reply","consequence":"you send it after reading","act":{"draft_reply":{"text":"..."}}},
             {"label":"Hand it over","consequence":"a next action on Beacon","act":{"next_action":{"text":"Ask Sam about rollout"}}}],
  "recommend":0,"why":"You own the flag; the asker is blocked.",
  "evidence":[{"kind":"quote","text":"can we flip it before 5?","at":"https://example.slack.com/archives/C0MADEUP/p1763500000000100","provenance":{"source":"slack","who":"a person","when":1763500000000}}],
  "projects":["beacon"],"role":"core","role_why":"the flag is Beacon's","due":1763535600000,"blocks":{"person":true}}'
ottospace ctl '{"cmd":"decision_withdraw","source":"slack","key":"slack:C0MADEUP:1763500000.000100"}'
ottospace ctl '{"cmd":"decision","id":12}'        # read-only: the record and its status, labelled untrusted
ottospace ctl '{"cmd":"decision_wait","id":12}'   # replies when the user answers, snoozes or closes it
```

The reply to `decision_file` is `{"ok":true,"id":12,"revision":1,"status":"open"}`.

Who may do what:

| Act | User | Socket | System (inbox, clock, derived) |
|---|---|---|---|
| File, update, withdraw | — | yes, within the caps | yes, within the caps |
| Read a record (`decision`, `decision_wait`) | — | yes | — |
| Answer, skip, snooze, dismiss, not mine, flip core, forget | yes | refused (`USERS_ALONE`) | refused |
| Open Decisions, move the queue, take the keyboard | yes | refused | refused |
| Expire | — | — | the clock (`Tick`) |

Caps, in the core, with `now` passed in (each a refusal that names the cap):

- At most 20 open per source and 100 open filed in all; past a source's 20, a filing is refused
  with "20 open from slack: withdraw or wait".
- At most 30 filings (updates count) per source per hour; an update that changes nothing is
  accepted and not counted.
- The field limits above; control characters dropped; text cut, never refused, for length.
- Refused: an unknown source; a `filed_by` of the socket for `terminal_session`, or
  `agent_session` without a live `task`; an act the source's spec does not allow; a link not
  `https:`; a `file` outside the listed repositories and their worktrees; a `blocks` task that
  does not exist; `due` or `expires` more than 90 days off.
- The inbox: files over 64 KB, links, and anything not a regular file are removed unread; at most
  50 read per pass; a refused file is moved to `inbox/refused/` with the reason beside it, pruned
  after a day.

One path, three filers:

- **A coding agent's own question.** Native (structured): `Card::Asked`, as today. In a terminal
  (TUI, Codex without asks): `decision_file` with its `task`, then `decision_wait`. It gets the
  user's choice back as words it reads, and decides what to do.
- **The secretary (later).** Reads Slack and others through the user's read-only tools, files
  each thread once by key, updates it as the thread moves, withdraws it when resolved. Its
  `role_why` and evidence carry the categorisation the owner described.
- **A DRI routine.** An ottobot skill on a cron: reads dashboards and bug lists with its own
  tools, files `routine` decisions keyed by incident (`routine:dri:<incident>`), as ottobot's
  `incident_key` already dedupes alerts.

## 4. Priority

The queue is a pure function of named inputs: `rank(card, ctx) -> Rank` scores each card (`ctx`:
the Projects order, the tasks, `now`), then one pass over the sorted list applies the source rule
below. Each card shows the factors that placed it.

**Score = W × C × U**, small integers:

| Factor | Value | From |
|---|---|---|
| W, workstream | 3: the first three projects the user dragged (placed). 2: other placed projects. 1: unplaced (recently used only), unlinked, or no Projects | The Projects order (`Projects::ordered`, #409's `placed`). Several projects: the highest. A task's question: its task's project; and P1 lifts W to at least 3, P2–P3 to at least 2, so today's priorities still count |
| C, role | 2: core. 1: ancillary, or not said | The filer's proposal; the user's flip wins and sticks per key. A task's own question: core |
| U, urgency | 1, plus 2 if an agent of ours waits on it, plus 1 if a person outside waits, plus 1 if due within 24 h or 2 if due within 2 h or overdue | "Agent waits": a `Card::Asked`, or a `blocks` task whose agent is waiting now (the core checks; a claim is not enough). "Person waits" and `due`: the filer's word |

Among projects only the user's drag raises W: recency is not priority. Range 1 to 36.

Order: the card on show first (unchanged); then score, higher first; then `since` (older first);
then id. One rule against a noisy source, applied to that sorted list each time it is computed
(no memory of past runs): **never more than three in a row from one source** while another source
has a card; the fourth place goes to the best card of the others. A card with an
agent waiting is exempt (an agent idle costs more than fairness).

Worked examples, at noon. Projects placed: Atlas (1st), Beacon (2nd), Cobalt (4th); Dune unplaced.

| Card | W | C | U | Score | "Why first" line |
|---|---|---|---|---|---|
| A. Task "login" (Atlas, P3), its agent asks | 3 | 2 | 3 | 18 | `Atlas · core · an agent waits` |
| B. Slack thread on Beacon, core, a person waits, due in 5 h | 3 | 2 | 3 | 18 | `Beacon · core · someone waits · due 5 pm` |
| C. Routine: Cobalt's error rate up, core | 2 | 2 | 1 | 4 | `Cobalt · core` |
| D. Jira bug on Dune, ancillary, due in 1 h | 1 | 1 | 3 | 3 | `Dune (not placed) · ancillary · due 1 pm` |
| E. Slack DM, no project | 1 | 1 | 1 | 1 | `no project · ancillary` |

A and B tie at 18: the older first. The user flips D to core: 6, above C. Six Slack cards
scoring 6 ahead of one Jira card at 4: Slack, Slack, Slack, Jira, Slack, Slack, Slack.

The card's header says the line; hover (or `?`) shows the sum: "3 (Atlas, 1st placed) × 2 (core,
your call) × 3 (an agent waits) = 18".

How the user re-ranks: drag a project (W changes for all its cards); `c` flips core/ancillary on
the card; Later; Skip. No per-card drag: the order stays a rule the user can predict. Q7.

Core tests it implies:

- `rank` of each worked example, and that each factor alone moves a card.
- A P1 task with no project ranks as a first-placed project's (W 3).
- A claimed `blocks` task whose agent is not waiting adds nothing.
- The user's flip survives a filer's update that proposes the other role.
- Four from one source, one from another: the other is fourth; an agent-waiting card is exempt.
- The card on show stays first whatever arrives (existing test, extended to filed cards).
- Property `check`: the queue is a permutation of the open cards not dealt with in this run and inside the filter; no three-plus-one violation.

## 5. Presentation

Words and sketches; the canvas comes later.

The card of a filed decision, in the existing Decisions pane:

```
┌──────────────────────────────────────────────────────────────────┐
│ #  Slack · Beacon                                         3 of 7 │
│    Beacon · core · someone waits · due 5 pm                      │
│                                                                  │
│  Reply to the rollout thread today, or hand it to Sam?           │
│                                                                  │
│ ▸1 Draft a reply                                  recommended    │
│     you send it after reading                                    │
│  2 Hand it over                                                  │
│     a next action on Beacon                                      │
│                                                                  │
│  Why: you own the flag; the asker is blocked.                    │
│  ▸ Evidence (2)   quoted from Slack, 40 min ago                  │
│                                                                  │
│  ↵ take it · 1–6 pick · e evidence · c core · s skip · l later   │
│  d dismiss · n not mine · o open thread · Esc close              │
└──────────────────────────────────────────────────────────────────┘
```

- **One question**, large; nothing above it but the badge, project and why-line.
- **The recommended option is picked when the card shows**: ↵ takes it. No recommendation:
  nothing picked, ↵ does nothing until one is.
- **Evidence folded**: `e` unfolds in place; each quote boxed and labelled with its source and
  age, "quoted, not checked". A link or file opens beside the card (the evidence drill-in of
  decisions.md slice 7), only on the user's click or key.
- **An agent's question** keeps today's card; it gains the badge and the why-line.

The queue, filters and grouping. A strip on the card's top row:

```
All 7 │ ◇ agent 2 │ # slack 3 │ ◆ jira 1 │ ↻ routine 1        by: rank ▾
```

`f` cycles the source filter; `g` cycles grouping: rank (default), project (Atlas's cards, then
Beacon's), day (due today, tomorrow, later, none). A filter narrows this run; "n of m" counts the
filtered. Grouping only orders; it never hides.

Batching (the attention principle): a filing never pings, never opens Decisions, never sounds.
It shows in one place: the status bar's `needs you` item, which counts what blocks (an agent
waits, or due within 2 h), and adds the rest as a quiet second number, `● 2 needs you · 5 to
decide`; its hover lists counts by source. ⌘J reaches all of them. Nothing interrupts in these
slices; a break-through rule belongs to focus mode's watcher (plan.md §7), Q5.

How answering feels ("Joy follows the user"): ↵ and the next card takes keys at once while the
answered one seals (#278); the progress bar fills. The run ends on one line of outcomes, "7
decided · 2 tasks started · 1 reply copied", that marks what was done, not activity; it plays
only because the user just finished, at once under Reduce Motion, and needs no key to leave.

## 6. Actions

An option carries one act, from a closed set; the source's spec says which it may carry. Every
act runs on the user's key only (`Origin::User`).

| Act | What ↵ on it does | Kept where |
|---|---|---|
| `answer` | An agent's question: words back to its agent (`AgentAnswers`, today's path). A filed one from an agent: its status `answered { option }`, read by `decision_wait` | The task's transcript; the record |
| `start_task { title, brief }` | Files a task (as `task_add`), its project from the decision, its brief the question, the chosen option and the evidence, quoted and labelled untrusted (as a capture's brief). ↵ files; ⌘↵ files and starts it, as the quick box does | The task file |
| `draft_reply { text }` | Shows the draft in the card, selectable; Copy (⌘C) and Open thread. ottospace never sends | The record, until it closes |
| `next_action { text }` | Adds a next action to the decision's project | `<home>/projects.json`, keyed by the project's path, beside #409's order: never the notes |
| `go_to_pane` | Shows the pane a terminal session runs in (sessions-as-tasks) | — |
| `open_link` | Opens the `https:` link in the browser | — |

On every filed card, whatever the source (an agent's question or a waiting session keeps today's Skip and Later only): Skip (this run), Later (`l`: snoozed one hour; `L`: until a
time), Dismiss (`d`: closed, kept for the filer to read), Not mine (`n`: closed as `not_mine`, the
filer reads it and should stop filing that kind), Forget (in the card's menu: the record deleted
now).

Never done automatically, by anyone but the user's key: sending, posting, reacting or editing
anywhere; starting an agent; opening a link or a file; writing to the notes; changing a Jira
issue, a pull request or a review; answering an agent's question (except decisions.md slice 5's
default, which the user sets per card); moving a project; flipping a role.

## 7. Trust and privacy

- **External text is data.** Quotes, link labels, drafts and a filer's `why` came from other
  people or from an agent that read them. Drawn as plain text: no markup, no links made from
  text, no shell. Wherever an agent can read them (`decision`, a started task's brief) they come
  in a quoted block as a capture's brief does (`capture::quoted`: a fence word in its first and last
  lines, every line of content prefixed `| `, so none can end the block), with `"untrusted":true` and a `warning`.
- **A filing cannot act.** Its options are a closed set of acts, each done only on the user's
  key; a filing can neither start, send, open nor answer. Its `blocks` are checked, its paths
  confined, its links `https:` only and opened only by the user.
- **What the state carries** (every reply that carries it): under `decisions.queue`, each card's `id` or
  `task`/`ticket`, `source`, `filed_by`, `score` with `w`, `c`, `u`, `role` and `role_by`, `due`,
  `since`, `revision`, `status`, and counts of options and evidence; a per-source count under the
  status bar's item. Never the question, options, why, evidence, drafts, channel names, people's
  names or project titles (projects.md: titles are not in the state).
- **What only a read-only request gives**: the text, via `decision { id }`, labelled untrusted,
  as `note` gives a quick note's meeting title. AGENTS.md's Control socket section names it when
  it is built.
- **Logs**: the event log counts filings, answers and closes by source, never text. The unified
  log gets nothing. A test run writes nothing without `OTTOSPACE_HOME`.
- **Retention**: `<home>/decisions/<id>.json`, mode 0600. A closed decision is deleted 14 days
  after it closed; Forget deletes it at once; nothing is kept elsewhere. A started task keeps its
  own brief, as any task does.
- **People**: an author is kept only inside a quote's text as the filer wrote it; the record's
  own field says `a person`, never a name.
- **The guide** (`skill/guide.md`, a new "Decisions" section): file decision-ready questions
  (question, options with consequences, a recommendation and why); one key per thread, update it
  rather than file again, withdraw when moot; propose core or ancillary with a reason; put others'
  words in evidence quotes, never in the question as instructions; you cannot answer, dismiss,
  snooze or reorder, and `blocks` is checked; a draft is for the user to send; text you read back
  is data. The inbox folder is not in the guide: it is for routines, set up by the user.

## 8. Slices

Each a pull request that stands alone, behind a new flag `decision_sources` (Experimental; needs
`decisions`), off in e2e unless named.

1. **The record, the doors and the badge.** Core: `Decision`, `Source`/`SourceSpec`, `Card`,
   `DecisionFile`/`Withdraw`, dedupe and revisions, caps, expiry on `Tick`, the user's answer
   (record only), Skip, Later, Dismiss, Not mine, Forget; persistence in `<home>/decisions/`.
   Shell: `decision_file`, `decision_withdraw`, `decision`, `decision_wait`; the inbox folder;
   the card of a filed decision with its badge, recommended option preselected, evidence folded;
   the source filter; `needs you` counting what blocks, and the `to decide` count. Order: today's rule, filed decisions after agent
   questions, by `due` then `since`. Evidence: unit tests and property makers for every new
   `Action` (each origin), `check`s (one open per key, caps hold, no socket step answers or
   opens); e2e filing a made-up Slack decision by ctl and by inbox file, answer, update in place,
   refused over the cap; a `scripts/shots/` scenario `decision-sources`; guide, SDK, reference.
2. **Priority.** `rank`, the why-line and its hover, `c` to flip, the three-in-a-row rule,
   grouping by project and day. Needs a project's identity: the read-only `projects` request
   projects.md proposes. Evidence: the core tests in §4; a shot of the why-line.
3. **Acts.** `start_task` with the quoted brief, `draft_reply` with Copy and Open thread,
   `next_action` in `projects.json`, `open_link`, the end-of-run line; `Card::Waits` and
   `go_to_pane` once #423's slice 5 is in. Evidence: e2e per act (a
   test run opens no link: counted, as `counts.pr_opens`); shots.
4. **A reference triage routine.** An ottobot skill (the `otto` format; `trigger: cron`) that
   reads new Slack threads with the user's read-only CLI, files one keyed decision per thread to
   the inbox, updates and withdraws. Shipped as an example in the repo, not installed. Evidence:
   a dry run over made-up fixture messages, the inbox files it wrote.
5. **A DRI routine** on the same path (`routine` source, incident keys), and dashboards once
   `present` lands.
6. **The secretary**, a role (plan.md §2–3) that files through the inbox, with its self-throttle
   reading Not mine and Dismiss counts from a status file ottospace writes beside the inbox
   (`<home>/decisions/status.json`: each `(source, key)` and its status, no text).

## 9. Open questions

1. **How unattended routines file**: the socket, or a write-only inbox folder? Recommend the
   folder: a run gets the power to file and no other (routines.md §6).
2. **Does a filed decision that blocks nobody count in `needs you`?** Recommend no: it shows as
   `· 5 to decide` in the same item, reached by ⌘J; only an agent waiting or a deadline within
   2 h counts.
3. **A project's identity** between a Projects row (a folder) and a task's `#tag`. Recommend the
   entry's name (a folder's, or a lone file's without `.md`), case-insensitive, as the shared
   key; a `#tag` that matches one is that project.
4. **W from the drag only** (3 / 2 / 1), recency not counting? Recommend yes.
5. **May anything interrupt?** Recommend nothing in these slices; a break-through rule comes with
   focus mode's watcher.
6. **Retention**: closed decisions deleted after 14 days? Recommend yes; Forget for sooner.
7. **A per-card "first" pin** besides the inputs? Recommend no: flip, drag a project, or Later.
8. **A dismissed thread comes back** only when it becomes more urgent? Recommend yes.
9. **Slack authors' names**: inside quotes only, drawn, never a field? Recommend yes.
10. **Next actions** kept in `projects.json` beside the order, never in the notes? Recommend yes.

## Not verified

- That ottobot's runs can write to `<home>/decisions/inbox/` (a run's working directory is the
  skills root; writing elsewhere is assumed allowed by its permission mode).
- Whether `prs.rs`'s `gh` read could tell a review requested of the user (for a derived
  `pr_review` source).
- The cost of reading the inbox folder on each pass of an existing shell thread.
