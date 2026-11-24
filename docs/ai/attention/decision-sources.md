# Decisions from any source

Status: design note, nothing built. First version merged in #424; revised 2026-11-23 for the
owner's answers to its three weakest points (triage is a decision; priority from the week's focus;
socket first). Written against `main` 5dd1fc71. Extends [decisions.md](decisions.md) (the
Decisions workspace, slice 1 built behind `decisions`) and §3 and §5 of [plan.md](plan.md). Every
example is made up.

The owner: "automatically bring in other 'sources' for decision points - ie, new slack threads
[...] categorized by the secretary agent: do they correspond to one or more projects; are there
additional helpful evidence; and then recommend some next actions. And have these also presented
as 'decisions'. We might want to tag decisions based on their source." Then: "triage makes sense
to fit into decisions - it's a decision itself (is this a priority? how does it slot in to existing
work?)", and "I already call out 4 focus areas for the week, and would like to track progress
against that."

Goals served (PR #421): a command center (every decision that is the captain's, from any source,
in one queue, ranked by the captain's own focus); flow and joy (one clear thing to act on per card;
sorting in one key each). The secretary is a later role (plan.md §2–3); this note lays the path it
will file through, so it is just another source.

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
| Where it shows | The Decisions workspace (⌘J under `decisions`; the palette): one card pane (`crates/ottospace/src/decisions.rs`), header "P2 · waiting 12m · its agent asks", the drawer's ask card, Skip and Later (S and L caps). "Decisions · 2 of 5" over a progress bar |
| Order | The card on show first; then priority; then waiting longest; then task id (`decision_queue`) |
| How it is answered | 1–9 pick, ↵ sends (`DecisionsAsk` → `ask_act` → `Effect::AgentAnswers`), the agent resumes. Skip: out of this run. Later: the task snoozed one hour (`LATER_MS`). Only `Origin::User`; a client or timer is refused (`USERS_ALONE`) |
| Persistence | None of its own. The run and `waiting` live in memory; Later is the task's `snoozed_until`, kept with the task |
| Socket state | `decisions`: null, or `tab`, `pane`, `task`, `ticket`, `at`, `of`, `queue` (`task`, `ticket`, `priority`, `since`). Ids and times only; the question is under `drawer.native` as a shape, never words |
| Counted | The status bar's `needs you` counts tasks and terminal sessions that wait (in-pane ones under `session_waits`; every waiting session card under `terminal_tasks`, `status_bar.rs`); the decision itself is not counted apart |

The gap to "any source":

- No record: nothing outside a task's agent can be a decision, and nothing is kept across a quit.
- One source, implied: no badge, no filter, no way to say where it came from.
- No project, core/ancillary, deadline, what it unblocks, expiry, evidence or "why recommended"
  (decisions.md slices 3, 4 and 7 plan some, for agent questions only).
- No filing path, no dedupe (a key is `(task, ticket)`), no caps, no triage.
- One kind of answer: words back to the asking agent.
- No notion of the week's focus: order knows the task's priority alone. Projects (#407, #409)
  has an order its section alone reads; task `#project` tags are free strings, and Projects rows
  are folders or lone `.md` files. Nothing links them yet.

## 2. The record

A `Decision` in `ottospace-core::decisions` (new module, pure). The agent's own question stays
derived; the queue holds every kind as one `Card`:

```
enum Card { Asked { task, ticket }, Waits { runner, id }, Filed { id } }
// a task's agent asks | a terminal session waits (#423) | kept on disk
```

A `Card::Asked` gets the fields below by rule (source `agent_session`, its task's project, core,
"an agent waits"); a `Card::Waits` likewise (source `terminal_session`, no project, ancillary).
Both keep today's keys only (answer or go to the pane, Skip, Later): no record to dismiss or
forget. Everything else is a `Filed` decision, which starts in triage unless its source skips it
(§4).

| Field | Type, limits | Notes |
|---|---|---|
| `id` | `u64`, ottospace's | |
| `key` | string ≤ 128, `[A-Za-z0-9:._-]` | The dedupe key, the filer's: `slack:C0MADEUP:1763500000.000100`, `jira:ABC-123`, `routine:dri:availability`. One open decision per `(source, key)` |
| `source` | `Source`, closed set (below) | Drawn as a badge |
| `filed_by` | `socket`, `inbox` (§3) or `ottospace` | Provenance of the filing, not a claim the filer makes |
| `revision`, `filed`, `updated` | `u32`, ms | Each update of the same key is a revision |
| `summary` | one line ≤ 160 chars | What came in, in the filer's words: the triage card's line |
| `question` | one line ≤ 160 chars | The one thing to decide once it is work |
| `options` | 0–6 | Each: `label` ≤ 60, `consequence` ≤ 160 ("the thread waits until Monday"), `act` (§7). None: slotting it files a next action (§4) |
| `recommend` | option index or none | |
| `why` | ≤ 300 chars | Why that option |
| `evidence` | ≤ 8 | Each: `kind` (`quote`, `link`, `file`), `text` ≤ 500 (a quote) or a label, `at` (an `https:` URL, or a file and line range), and `provenance`: `source`, `when` (ms), `who` (`a person`, `a bot`, `the filer`). Quotes are other people's text: §8 |
| `suggest` | `projects` (≤ 3 names), `role` (`core`/`ancillary`) with `role_why` ≤ 120 | The filer's proposal. In triage: shown, never used to rank |
| `claims` | `due` (ms), `person_waits` (bool), `blocks` (task ids) | The filer's word. `blocks` is checked by the core; `due` and `person_waits` add urgency only once accepted (§4) |
| `triage` | `pending`, or `sorted { area or project, at }`, or `skipped` (a trusted source) | §4 |
| `accepted` | `projects` (or an `area`), `role`, `due`, `person_waits`, each with `by` (`user` or `trusted source`); `now { until, displaced }` after `t` | What ranks it. The user's choice sticks for the key, across revisions |
| `expires` | ms | Tied to its asker where that can be checked (§3); else 7 days after its last update |
| `task` | task id or none | The task it belongs to, when an agent files for its own task |
| `status` | `open`, `snoozed { until }`, `answered { option, at }`, `dismissed`, `not_mine`, `withdrawn`, `expired` | Only the user moves it out of `open`, except `withdrawn` (the filer) and `expired` (the clock or the asker's end) |

### Sources

A closed set, each a `SourceSpec`, asked by capability (as a runner is its spec): never a `match`
on its name outside the table.

| Source | Badge | Filed by | Triage by default | Acts its options may take (§7) |
|---|---|---|---|---|
| `agent_session` | the runner's glyph | derived (`Card::Asked`); or over the socket naming a live `task` (a TUI agent cannot use the structured ask). The socket cannot tell the caller: the task is a claim, and the card says "filed over the socket" | derived: no. Over the socket: promoted by default (work the user already took on), revocable as any promotion; Open 8 | answer, start task, next action |
| `terminal_session` | the runner's glyph, dim | derived (`Card::Waits`), with sessions-as-tasks slice 5 (#423) | no | go to pane |
| `slack` | # | socket (inbox later) | yes | start task, draft reply, next action, open link |
| `jira` | ◆ | socket (inbox later) | yes | start task, next action, open link |
| `pr_review` | ⑂ | socket (inbox later); deriving it from `prs.rs` needs review requests added to its fixed field list (it reads `reviewDecision`, not who is asked) | yes | start task, next action, open link |
| `calendar` | ◷ | socket (inbox later) | yes | start task, next action |
| `routine` | ↻ | socket (inbox later) | yes | start task, next action, open link |

A new source: a variant and its `SourceSpec` row (badge, who may file, triage by default, allowed
acts, whether its text is external), one core test that the table refuses the rest, the guide's
list, the SDK's type. No other code names it.

### Dedupe and updates

- Same `(source, key)` while `open` or `snoozed`: an update. New revision; fields replaced, except
  the user's sort, acceptances and snooze, which stay. In triage it stays in triage. The card on
  show stays on show (decisions.md rule 1).
- The card on show updated: its text changes in place with a quiet "updated" line; a pick made on
  the older revision is cleared; ↵ names the revision it answers, and a stale one does nothing.
- Same key after `answered`: a new revision opens again, marked "again", already sorted where the
  user sorted it (the thread moved on; its place is known).
- Same key after `dismissed`, `not_mine` or `expired`: kept closed, updated silently, unless the
  update brings a sooner `due` or a new `blocks`; then back to triage. As ottobot's alerts: a
  higher level brings it back, the same level does not.
- `decision_withdraw { source, key }`: the filer says it is moot. Closes it; the card on show
  leaves as an answered-elsewhere one does.

## 3. Ingest

ottospace never fetches. A filer reads its source with its own tools (the user's read-only Slack
CLI, `gh`, a Jira client) and hands ottospace the record.

**One door first: the control socket** (working default, reversible). `decision_file` from an agent
in one of our panes, or a routine the user runs by hand (`Origin::Socket`). The inbox folder
(`<home>/decisions/inbox/*.json`, `Origin::System`) comes only when a real unattended routine
needs it (slice 5): routines.md §6 keeps the socket away from unattended runs, since it can `type`
into a terminal, and a drop folder gives a run one power only, filing. Both doors share one
validation and one core action, `Action::DecisionFile { record, via, now }`.

Requests (closed set, same-user socket as today):

```
ottospace ctl '{"cmd":"decision_file","source":"slack","key":"slack:C0MADEUP:1763500000.000100",
  "summary":"Asked in #made-up-rollout whether the flag can flip before 5",
  "question":"Reply to the rollout thread today, or hand it to Sam?",
  "options":[{"label":"Draft a reply","consequence":"you send it after reading","act":{"draft_reply":{"text":"..."}}},
             {"label":"Hand it over","consequence":"a next action on Beacon","act":{"next_action":{"text":"Ask Sam about rollout"}}}],
  "recommend":0,"why":"You own the flag; the asker is blocked.",
  "evidence":[{"kind":"quote","text":"can we flip it before 5?","at":"https://example.slack.com/archives/C0MADEUP/p1763500000000100","provenance":{"source":"slack","who":"a person","when":1763500000000}}],
  "suggest":{"projects":["beacon"],"role":"core","role_why":"the flag is Beacon's"},
  "claims":{"due":1763535600000,"person_waits":true}}'
ottospace ctl '{"cmd":"decision_withdraw","source":"slack","key":"slack:C0MADEUP:1763500000.000100"}'
ottospace ctl '{"cmd":"decision","id":12}'        # read-only: the record and its status, labelled untrusted
ottospace ctl '{"cmd":"decision_wait","id":12}'   # replies when the user sorts, answers, snoozes or closes it
```

The reply to `decision_file` is `{"ok":true,"id":12,"revision":1,"status":"open","triage":"pending"}`.

Who may do what:

| Act | User | Socket | System (inbox, clock, derived) |
|---|---|---|---|
| File, update, withdraw | — | yes, within the caps | yes, within the caps |
| Read a record (`decision`, `decision_wait`) | — | yes | — |
| Sort, accept a claim, answer, skip, snooze, dismiss, not mine, forget | yes | refused (`USERS_ALONE`) | refused |
| Set the week's focus, promote or demote a source | yes | refused; a client proposes focus only (§5) | refused |
| Open Decisions, move the queue, take the keyboard | yes | refused | refused |
| Expire | — | — | the clock (`Tick`), or the asker's end |

**Expiry tied to its asker** (working default, reversible), checked where ottospace can see the
asker; else 7 days after the last update:

| Asker | Expires when |
|---|---|
| An agent filing for its own `task` | The task is done or gone, or its agent ended (the decision would be answered to no one) |
| A waiting terminal session | Derived: it leaves when the session stops waiting |
| `decision_wait` open on it | Not by itself: an asker still waiting keeps it |
| Slack, Jira, PR, calendar, routine | Not checkable: the filer's `expires`, at most 90 days, default 7 days; a filer withdraws when it sees the thread resolved |

Caps, in the core, with `now` passed in (each a refusal that names the cap):

- At most 20 open per source and 100 open filed in all; past a source's 20, refused with "20
  open from slack: withdraw or wait". At most 30 filings (updates count) per source per hour; an
  update that changes nothing is accepted and not counted.
- The field limits above; control characters dropped; text cut, never refused, for length.
- Refused: an unknown source; the socket filing `terminal_session`, or `agent_session` without a
  live `task`; an act the source's spec does not allow; a link not `https:`; a `file` outside the
  listed repositories and their worktrees; a `blocks` task that does not exist; `due` or
  `expires` more than 90 days off.

One path, three filers:

- **A coding agent's own question.** Native: `Card::Asked`, as today. In a terminal (TUI, Codex
  without asks): `decision_file` with its `task`, then `decision_wait`; no triage (its task is
  work already taken on). It expires with its task.
- **The secretary (later).** Reads Slack and others through the user's read-only tools, files each
  thread once by key, updates it as the thread moves, withdraws it when resolved. Its suggestions
  carry the categorisation the owner described; the user's sort decides.
- **A DRI routine.** An ottobot skill: reads dashboards and bug lists with its own tools, files
  `routine` decisions keyed by incident (`routine:dri:<incident>`), as ottobot's `incident_key`
  dedupes alerts.

## 4. Triage: sorting is a decision

An item from a source that has not earned trust does not enter the queue as a claimed-urgent
decision. It comes in as a triage item: "Is this a priority, and where does it slot in?" The
filer's project, role and claims are suggestions on the card. Only the user's sort makes it work
or a ranked decision.

### Sort mode

Sorting and deciding are two modes. Deciding is one card at a time, read, then answered. Sorting
is a stack gone through at one key each, the evidence folded unless asked for.

While items wait, the Decisions run's first card is one card for all of them, "Sort 6 new items"
(after cards where an agent waits: an idle agent costs more). ↵ enters Sort mode; `s` skips to
deciding, and the sort card waits for the next run. The items are never counted in `needs you`;
they show as `· 6 to sort` beside it (Decided 2).

```
┌──────────────────────────────────────────────────────────────────┐
│ Sort · 2 of 6                              # Slack · 20 min ago  │
│                                                                  │
│  Asked in #made-up-rollout whether the flag can flip before 5    │
│  Suggested: Beacon · core          Claims: someone waits · 5 pm  │
│  ▸ Evidence (1)                    (not counted until you say)   │
│                                                                  │
│  1 Login rewrite   2 Beacon launch   3 Hiring   4 DRI week       │
│  t take now · first of the unplanned, before cobalt: "cache"     │
│  o other project · l later · n not mine · d dismiss              │
│  u count its claims · e evidence · z undo · Esc leave            │
└──────────────────────────────────────────────────────────────────┘
```

| Key | The item becomes | Kept |
|---|---|---|
| `1`–`4` | Slotted under that focus area (§5): a ranked decision there, role core. With no options, a next action of the area's first project, or of the area itself when it links none (kept in `focus.json`) | `triage: sorted`, `accepted.area` |
| `o` | A picker of projects: slotted under that project, role ancillary unless `c` | the same |
| `t` | Take now: first of its tier for today (until midnight), its claims counted, role ancillary unless `c`. The card says what it goes before, before the key is pressed. Outside focus it is first of the unplanned, and counted as unplanned work that displaced focus (§6) | `accepted.now { until, displaced }` |
| `l` | Back in the next sort run, tomorrow | snoozed |
| `n` | Not mine: closed; the filer reads it | `not_mine` |
| `d` | Dismiss: closed | `dismissed` |
| `u` | Toggles: its claims (due, someone waits) count once it is slotted. Off by default | `accepted.due`, `accepted.person_waits` |
| `c` | Toggles core / ancillary before the slot key | `accepted.role` |
| `z` | Undo the last sort | — |

The next item takes keys at once; the sorted one folds into its area's chip at the bottom (motion
under the next key, at once under Reduce Motion). The run ends on one line: "6 sorted · 2 to
Beacon · 1 taken now · 3 closed".

The suggested area is preselected when the filer's suggestion names a project linked to one, so
↵ takes the suggestion, as a recommended option is (§6).

### Earning the right to skip triage

- **Only the user promotes, and only a named scope.** Each source's key has a closed format
  (`slack:<channel>:<ts>`, `jira:<PROJECT>-<n>`, `pr_review:<owner>:<repo>:<n>`,
  `routine:<name>:<incident>`; `agent_session:<id>` and `calendar:<id>` have no scope); a filing
  whose key is not its source's is refused. P promotes the scope of the item on show, a field of
  that format (the channel, the project, the repository, the routine), which the card names before
  the key ("p skip triage for Slack channel C0MADEUP"). Never a free prefix, never a whole source:
  there is no way to promote one (built, slice 1b). Its items then rank at once with the filer's
  suggestions and claims accepted.
- **What a promotion means.** The socket cannot prove who filed: any same-user client can file
  under a promoted channel, and its claims count at once. So promotion trusts the *filers* of that
  scope, not only the people in it. It stays bounded: the caps hold for a promoted scope (20 open
  per source, 30 filings per source an hour), and in slice 2 the three-in-a-row rule spaces its
  cards like any source's.
- **New words are new triage.** An update that changes what a sorted item says (summary, question,
  options, why, evidence, suggestion, claims, task) puts it back in triage, takes back what the
  user accepted (its claims, its role), and the card says what changed ("Changed since you sorted
  it: question, claims"). Only a new time (`expires`) leaves it sorted. Under a promoted scope the
  filer's word stands for each new revision. Every acceptance names the revision it was given to,
  and a property check holds that none stands for other words (built, slice 1b).
- **ottospace may offer, never decide**: at the end of a sort run where the last 10 items from one
  prefix were all slotted as suggested, one line: "↻ dri: 10 of 10 as suggested. Let it skip
  triage?" ↵ promotes; anything else and it is not asked again for 10 more.
- **Revocable at once**: the same menu, or `!` on any card from it ("back to triage"), which puts
  its open items back in triage too.
- Kept in `<home>/decisions/trust.json` (the scopes promoted). A test run writes none
  without `OTTOSPACE_HOME`. Not in the state as text: counts only.

## 5. The week's focus

The owner names four focus areas each week. They, not the Projects drag order, decide priority.
The drag order stays the Projects section's own order and feeds nothing else (replaces Decided 4
of #424).

### A focus area

`Area { name, links, set_by, from }`: the user's name for it (≤ 40 characters), and links to zero
or more projects (by Projects entry name, Decided 3) and repositories (listed ones, by path).
Picking a project makes an area named after it, linked to it: the common case. A free-text area
("Hiring") may link to nothing; then only items sorted into it by hand belong to it.

What belongs to an area: a task whose `#tag` or `>repo` is linked (a repository linked to a
linked project through Projects slice 3 counts too, once built); a decision whose accepted
project is linked or which was sorted into it; a pull request of a linked repository.

At most four, in the user's order; the order is for display (and the number keys), not rank: all
four rank alike.

### Where it comes from

ottospace itself never reads or writes the weekly note. Two ways in:

1. **By hand, in ottospace**: ⌘P › This week's focus…, or the Week strip's ✎. Built (slice 2a): the
   editor lists the Projects entries; the user ticks up to four, each an area named by the
   project's title and linked by its name. An area of no entry (a name a proposal brought, "Hiring")
   is listed above them, ticked, kept as it is. Typing a free name by hand is not built.
2. **Proposed by an agent**: a routine (later the secretary) reads the note with its own tools
   and files the week's focus:

```
ottospace ctl '{"cmd":"focus_set","week":"2026-W48","areas":[
  {"name":"Login rewrite","projects":["atlas"]},
  {"name":"Beacon launch","projects":["beacon"],"repos":["/Users/me/code/beacon"]},
  {"name":"Hiring"},
  {"name":"DRI week","projects":["dri"]}],
  "provenance":{"path":"~/notes/weekly/2026-w48.md","lines":[5,9],"quote":"- Login rewrite\n- Beacon launch\n..."}}'
```

A `focus_set` never sets focus. It files a confirm card, a decision of the triage kind, first in
the next Decisions run (after cards where an agent waits):

```
┌──────────────────────────────────────────────────────────────────┐
│ ◎ This week's focus · proposed by a routine from your note       │
│                                                                  │
│   1 Login rewrite      → atlas                                   │
│   2 Beacon launch      → beacon, ~/code/beacon                   │
│   3 Hiring             (no link)                                 │
│   4 DRI week           → dri                                     │
│                                                                  │
│   Now: Login rewrite, Beacon launch, Docs, DRI week              │
│   Changes: − Docs  + Hiring                                      │
│   ▸ From weekly/2026-w48.md, lines 5–9 (quoted)                  │
│                                                                  │
│   ↵ set it · e edit · Esc keep the current focus                 │
└──────────────────────────────────────────────────────────────────┘
```

The card shows exactly what will be set: each name and link as they will be kept, and what
changes from the current focus. The note's words are the user's own, but they arrive through an
agent that may have misread or been steered, so nothing is applied until ↵ (or after an edit).

- **When it runs**: the routine's to decide; recommended at the start of the week (Monday
  morning) and when the note changes (the routine watches the file; ottospace does not).
- **One proposal at a time**: a newer `focus_set` for the same week replaces the pending one (a
  revision). Identical to the current focus: no card. At most 6 an hour.
- **The note and ottospace disagree** (the user changed focus by hand, then the note changed):
  the next proposal shows the difference and waits; Esc keeps ottospace's. ottospace's focus is
  the one that ranks; the note is never written back. A proposal the user declined is not shown
  again until the routine files a different one.
- Refused: more than four areas, a name over 40 characters, a week other than this one (a project
  or repository that is not listed is taken, marked "not listed" on the card: no existence oracle) (built, slice 2b: next week is refused for now; `week` may be
  left out for this one). On the card: ↵ sets the revision shown (a newer proposal replaces the
  card, and a ↵ for an older one sets nothing), E opens the editor with it ticked, N keeps the focus
  as it is, Esc closes Decisions and the card waits.

### The week

Weeks are ISO weeks, starting Monday 00:00 local time (the shell passes the offset; the core gets
the time). On the first `Tick` of a new week, last week's focus becomes last week's, and nothing
is in focus until the user acts: the first Decisions run of the week starts with a carry-over card
("Keep last week's focus? ↵ keep · e edit · Esc none"), unless a proposal for this week is already
waiting, which comes first instead. Kept in `<home>/focus.json`: each week's areas and links, who
set them and when, and the week's tallies at rollover (§6). Never in the notes.

## 6. Priority and progress

### The rule

The queue is a pure function of named inputs (the cards, this week's focus, the tasks, `now`). Each
card shows the inputs that placed it.

Sort key, in order:

1. The card on show (unchanged: nothing moves under the person).
2. The focus card, then the sort card, when waiting: after the in-focus cards where an agent
   waits, before everything else (a new focus changes where items sort to, so it comes first).
3. **Focus**: in one of this week's areas first; the rest after, all equal.
4. **Taken now** (`t` today), first of its tier.
5. **An agent waits** on it: a `Card::Asked`, or a `blocks` task whose agent waits now (checked by
   the core, never taken from a claim). An idle agent costs more than any filed card.
6. **C × U**: C is 2 for core, 1 for ancillary. U is 1, plus 1 if a person waits (accepted), plus
   1 if due within 24 h or 2 if within 2 h or overdue (accepted). Range 1 to 8.
7. The task's priority (P1 first; a card of no task after every task's), so today's order holds
   when nothing else differs.
8. Waiting longest, then id.

Then one pass over that list: **never more than three in a row from one source** while another
source has a card in the same focus tier; a card with an agent waiting is exempt.

Triage items are not in this list: they are the sort card's. With no focus set, step 3 is a tie
and the rule is today's (priority, then age) for agent questions, since all have C × U = 6.

Worked examples, Wednesday at noon. Focus: 1 Login rewrite (atlas), 2 Beacon launch (beacon),
3 Hiring, 4 DRI week (dri). Not in focus: cobalt, dune.

| Card | Focus | Agent waits | C × U | Why line |
|---|---|---|---|---|
| A. Task "login" (atlas, P3), its agent asks | yes | yes | 2 × 1 = 2 | `Login rewrite · an agent waits` |
| B. Slack thread sorted to Beacon launch, claims accepted (someone waits, due 5 pm) | yes | no | 2 × 3 = 6 | `Beacon launch · core · someone waits · due 5 pm` |
| C. Routine: dri error rate up, trusted source, core | yes | no | 2 × 1 = 2 | `DRI week · core` |
| D. Jira bug on dune, sorted with `o`, ancillary, due 1 pm accepted | no | no | 1 × 3 = 3 | `not in focus · due 1 pm` |
| E. Task "cache" (cobalt, P1), its agent asks | no | yes | 2 × 1 = 2 | `not in focus · an agent waits` |
| F. Slack thread on dune, taken now with `t` | no | no | 1 × 1 = 1 | `taken now · before "cache"` |

Order: A (an agent waits), B, C, then F (taken now), E (an agent waits), D. B's claims counted only because
the user pressed `u` when sorting; unsorted, B was in the sort card and ranked nowhere.

Hover (or `?`) on the why line gives the sum: "in focus (Beacon launch) · core, the filer's,
you kept it × urgency 3 (someone waits, due 5 pm: you counted these) = 6".

The user re-ranks through the inputs: set the week's focus, sort, `c` on a card, `u` to count
claims, `t`, Later, Skip. No per-card drag (Decided 7).

The Tasks section's Needs you group and ⌘J's next task order by focus first, then as today.

Core tests it implies:

- The worked table, and that each input alone moves a card.
- A triage item is in no ranked queue; sorting it puts it in its area's tier.
- An unaccepted claim adds nothing; accepting it adds exactly its U; a trusted source's counts.
- A claimed `blocks` task whose agent is not waiting adds nothing.
- With no focus set, the order of agent questions is today's (priority, then age).
- Drag order changes nothing in the queue.
- `t` puts a card first of its tier until midnight, and records the card it displaced.
- A focus proposal changes nothing until the user's ↵; a client's ↵ is refused.
- Week rollover: nothing in focus until the user keeps, edits or sets it.
- Four non-exempt cards from one source ranked ahead of one from another, in one tier: the other
  comes fourth; an agent-waiting card is exempt.
- A filed card never ranks above a card of its tier where an agent waits.
- Property `check`: the queue is a permutation of the open, sorted (or trusted) cards not dealt
  with in this run and inside the filter; focus tier never inverted; the card on show first.

### Progress against focus

Per area, this week, from what ottospace already holds (pure: counted from records with times):

| Counts | From |
|---|---|
| Tasks done | tasks of the area with `finished_at` this week |
| Pull requests merged | linked repositories' pull requests merged this week: a new read-only `gh` read (Decided 16) |
| Decisions answered | a per-week counter in `focus.json`, counted when the user answers (records are deleted, and a key's revisions reuse one record) |
| Agent time | the area's tasks' agent time this week: a per-week counter beside `progress::Clock`'s day total (new: it keeps a lifetime and one day's time) |
| Waiting on you | its open decisions and its tasks under Needs you, now |

Each area's state, one word, in this order:

- **waiting**: anything of it waits on the person now;
- **moved**: something done, merged or answered in the last 2 working days;
- **stalled**: nothing in the last 2 working days (counted in hours of weekdays);
- **new**: less than 2 working days of the week have passed and nothing moved yet.

Unplanned work: what belongs to no area is counted as one line, "Unplanned". What displaced focus
is named apart: items taken now (`t`) outside focus, and agent time on unplanned tasks while an
area was stalled.

Shown as the Week strip on the Decisions pane (between cards, and at the end of a run); the status
bar's `needs you` hover adds one line per area to its tooltip. Never a notice.

```
Week 48 · Wed
 1 Login rewrite   ● moved     3 done · 1 merged · 4 decided · 6 h agents
 2 Beacon launch   ● waiting   2 wait on you
 3 Hiring          ○ stalled   nothing since Mon
 4 DRI week        ● moved     5 decided
   Unplanned                   2 done · 3 h agents · 2 taken now
```

Facts, no score. A reward marks an outcome (Joy follows the user): an area's first move of the
day lights its dot once, when the user is looking at the strip, never by itself.

At rollover the week's tallies go into `focus.json` (counts and area names only), so the next
week's carry-over card can say "Hiring stalled all week".

**The day plan** (the owner's idea, not designed): a plan of the day lists what the user means to
do today, each item linked to an area. This note gives it what it needs: areas, membership, and the
tallies; the day's line reads "4 of 6 planned were focus · 2 unplanned took 3 h".

## 7. Actions

An option carries one act, from a closed set; the source's spec says which it may carry. Every
act runs on the user's key only (`Origin::User`).

| Act | What ↵ on it does | Kept where |
|---|---|---|
| `answer` | An agent's question: words back to its agent (`AgentAnswers`, today's path). A filed one from an agent: its status `answered { option }`, read by `decision_wait` | The task's transcript; the record |
| `start_task { title, brief }` | Files a task (as `task_add`), its project from the decision, its brief the question, the chosen option and the evidence, quoted and labelled untrusted (as a capture's brief). ↵ files; ⌘↵ files and starts it, as the quick note box (⌘⇧N) does | The task file |
| `draft_reply { text }` | Shows the draft in the card, selectable; Copy (⌘C) and Open thread. ottospace never sends | The record, until it closes |
| `next_action { text }` | Adds a next action to the decision's project | `<home>/projects.json`, keyed by the project's path, beside #409's order: never the notes |
| `go_to_pane` | Shows the pane a terminal session runs in (sessions-as-tasks) | — |
| `open_link` | Opens the `https:` link in the browser | — |

On every filed card: Skip (this run), Later (`l`: one hour; `L`: until a time), Dismiss (`d`),
Not mine (`n`: the filer reads it), Forget (the card's menu: deleted now), back to triage (`!`).
An agent's question or a waiting session keeps today's Skip and Later only.

The card of a ranked decision is #424's: badge and area, the why line, the one question, the
recommended option preselected (↵ takes it), evidence folded (`e`).

Never done automatically, by anyone but the user's key: sending, posting, reacting or editing
anywhere; starting an agent; opening a link or a file; writing to the notes; changing a Jira
issue, a pull request or a review; answering an agent's question (except decisions.md slice 5's
default, which the user sets per card); sorting; setting focus; promoting a source.

## 8. Trust and privacy

- **External text is data.** Summaries, quotes, link labels, drafts, a filer's `why` and a focus
  proposal's names came from other people or from an agent that read them. Drawn as plain text: no
  markup, no links made from text, no shell. Wherever an agent can read them (`decision`, a
  started task's brief) they come in a quoted block as a capture's brief does (`capture::quoted`:
  a fence word in its first and last lines, every line of content prefixed `| `), with
  `"untrusted":true` and a `warning`.
- **A filing cannot act, rank or set.** Its options are a closed set of acts, each on the user's
  key; its suggestions and claims rank nothing until the user sorts and accepts them (or promoted
  its source); a focus proposal sets nothing until confirmed.
- **What the state carries** (every reply that carries it): under `decisions.queue`, each card's
  `id` or `task`/`ticket`, `source`, `filed_by`, `triage`, `focus` (an area's index or null),
  `c`, `u`, `now`, `due`, `since`, `revision`, `status`, counts of options and evidence; counts to
  sort and to decide; per area its index, state and counts. Never a question, summary, option,
  why, evidence, draft, channel, person, project title or area name.
- **What only a read-only request gives**: a decision's text, `decision { id }`; the week's areas
  by name with their links and state, `focus`; each labelled as `note` is.
- **Logs**: the event log counts filings, sorts, answers and closes by source, never text.
- **Retention**: `<home>/decisions/<id>.json`, 0600, deleted 14 days after it closed; Forget at
  once. `focus.json` keeps weeks' areas and counts; the user may clear a week.
- **People**: an author is kept only inside a quote's text; the record says `a person`.
- **What is accepted is what was shown** (built, slice 2b, for every card a filer can change while
  it shows: the week's proposal, triage's item): a key or click names the card and revision the
  shell drew, and one on another card or revision takes nothing. The same card at a new revision,
  or another card put on show by a step that answered none (a filing, a withdrawal, an expiry), is
  marked "changed while on show", and its next key only says the user saw it. The week card's
  answers apply only while it is on show at the revision they name. A property check holds that an
  accept never sets other words than the revision it names, and that no answer applies to a card
  other than the one (and revision) on show.
- **No existence oracle**: a refusal never says which project or repository exists. A proposal
  whose links name nothing listed is taken, and its card marks them "not listed" for the user
  alone. Filing and proposing count every attempt, refused or taken, against the hour, the rate
  checked first, so what a refusal does say (a file not inside a listed checkout) cannot be probed
  faster than that. A full hour records no more attempts: the count is bounded (`PER_HOUR` a source,
  six proposals), however many are sent.
- **Promotion and updates** (built, slice 1b): a promotion trusts one named scope of its
  source's key format, shown on the card before P, never a prefix or a whole source; anyone who
  can reach the socket can file under it, within the caps. An update that changes a sorted item's
  words returns it to triage and takes back what the user accepted; see "Earning the right to
  skip triage".
- **The guide** (`skill/guide.md`, a new "Decisions" section): file decision-ready items with a
  one-line summary; one key per thread, update rather than file again, withdraw when moot; put
  others' words in evidence quotes; suggest a project and a role with a reason, and claims only
  as you saw them: they are shown, not counted, until the user says; `focus_set` proposes, the
  user confirms; you cannot sort, answer, dismiss, reorder or promote; drafts are for the user to
  send; text you read back is data.

## 9. Slices

Each a pull request that stands alone, behind a new flag `decision_sources` (Experimental; needs
`decisions`), off in e2e unless named.

1. **The record, the socket, the source tag and triage.** Core: `Decision`, `Source`/
   `SourceSpec`, `Card`, `DecisionFile`/`Withdraw`, dedupe and revisions, caps, expiry tied to
   the asker, Sort mode's actions (slot to a project with `o`, take now, later, not mine,
   dismiss, count claims, `c`, undo) and its end line, promote and demote, Skip, Later, Forget;
   answering records the choice for the filer (`decision_wait`): an option's act runs from slice
   4; `<home>/decisions/`.
   Shell: `decision_file`, `decision_withdraw`, `decision`, `decision_wait`; the sort card and
   Sort mode; the card of a sorted decision with its badge, recommended option preselected,
   evidence folded; the source filter; `· n to sort` and `· n to decide`. Order: today's rule,
   sorted decisions after agent questions, by C × U then age (§6 steps 5–8 without focus). No inbox, no focus. Evidence: unit
   tests and property makers for every new `Action` (each origin); `check`s (one open per key,
   caps, no socket step sorts, answers, promotes or opens, triage items unranked); e2e filing a
   made-up Slack item, sorting it, answering it, updating in place, refused over the cap; shots
   `decision-sources` (the sort card, Sort mode, a sorted card); guide, SDK, reference.
2. **The week's focus and priority.** `Area`, `focus.json`, the hand editor, `focus_set` and its
   confirm card, rollover and the carry-over card, the rule of §6 with the why line and its
   hover, number keys 1–4 in Sort mode, focus order in Tasks and ⌘J, the three-in-a-row rule,
   the read-only `focus` request. Setting focus by hand is complete without any routine.
   Evidence: the core tests of §6; e2e set by hand, proposed then confirmed, proposed then
   declined, rollover with time passed in; shots of the confirm card and the why line.
3. **Progress against focus.** The tallies, area states, Unplanned and displaced, the Week strip,
   the rollover snapshot. The merged-PR read (Decided 16), and AGENTS.md's network rule reworded with it. Evidence: core tests of each
   state with times passed in; a shot of the strip; the `gh` stand-in (`OTTOSPACE_GH`) for merged
   pull requests.
4. **Acts.** `start_task` with the quoted brief, `draft_reply` with Copy and Open thread,
   `next_action` in `projects.json`, `open_link`, the decide run's outcome line; `Card::Waits` and
   `go_to_pane` once #423's slice 5 is in. Evidence: e2e per act (a test run opens no link:
   counted, as `counts.pr_opens`).
5. **A reference triage routine**, and with it the inbox folder (an unattended run needs it): an
   ottobot skill that reads new Slack threads with the user's read-only CLI and files one keyed
   item per thread, updates and withdraws; and a weekly-note routine that files `focus_set`.
   Shipped as examples, not installed. Evidence: dry runs over made-up fixture messages and a
   made-up note, the files they wrote.
6. **A DRI routine** on the same path (`routine` source, incident keys), dashboards once
   `present` lands.
7. **The secretary**, a role (plan.md §2–3): files through the inbox, proposes focus, and
   throttles itself on Not mine and Dismiss counts from a status file ottospace writes beside the
   inbox (`<home>/decisions/status.json`: each `(source, key)` and its status, no text).

## 10. Decided

From #424 (owner, 2026-11-23), standing unless marked:

1. **Unattended routines file through the inbox folder**, never the socket. Now with a working
   default (reversible): the socket first; the inbox only when a real unattended routine needs it
   (slice 5).
2. **What blocks nobody is not in `needs you`**: `· n to sort` and `· n to decide` beside it,
   reached by ⌘J. Only an agent waiting or an accepted deadline within 2 h counts.
3. **A project's identity** is the Projects entry's name (a folder's, or a lone file's without
   `.md`), case-insensitive; a task's `#tag` that matches one is that project.
4. ~~W from the drag only~~. **Replaced**: priority comes from the week's focus (§5, §6); the
   drag order feeds nothing but the Projects section.
5. **Nothing interrupts** in these slices (an on-call rule comes after slice 2: Decided 15).
6. **Retention**: closed decisions deleted after 14 days; Forget for sooner.
7. **No per-card "first" pin**; `t` (take now) in triage is a sort, for today only.
8. **A dismissed thread comes back** only when more urgent, and then to triage.
9. **Slack authors' names** stay inside quotes, never a field.
10. **Next actions** in `projects.json`, never the notes.

Added (owner, 2026-11-23):

11. **Triage is a decision**: items from external sources not promoted arrive as "Is this a priority, and
    where does it slot in?"; the user's sort makes them work. Claims are shown and count only once
    accepted. A source skips triage only by the user's promotion, revocable.
12. **Priority from the week's four focus areas**: in focus first, the rest equal.
13. **The week's focus comes from the user**: set by hand in ottospace, or proposed by an agent
    that read the weekly note (`focus_set`) and confirmed by the user. ottospace never reads or
    writes the note.
14. **Expiry tied to its asker** where ottospace can check it (working default, reversible).
15. **An "urgent" rule for an on-call week** (owner, 2026-11-23): a rule the user sets may raise
    `needs you`. By source and scope only (a routine such as `dri`, a pager channel; the scopes of
    the key formats), never by keyword (a keyword matches untrusted text, so anyone in a channel
    could raise it); it ends on a date the user sets; it raises the count and puts the card first,
    with no sound, notice or focus change. Built after slice 2.
16. **A second read-only `gh` call** (owner, 2026-11-23): pull requests merged this week in the
    listed repositories, for progress against focus. And the network rule is too narrow as one
    named command: in the slice that adds this read (progress against focus), AGENTS.md's rule is
    reworded to the principle it protects. The shell may read, through the user's own `gh`, with a
    fixed argv and read-only subcommands, data about repositories the user listed (open and merged
    pull requests today; a new read is added by naming it in the rule and in `prs.rs`); never a
    write, never another repository, never a shell; `OTTOSPACE_GH` stands in for it in tests. The
    calendar clause stays as it is. AGENTS.md is not changed before that slice.

## 11. Open questions (the owner's)

1. **An idle agent on unplanned work waits behind every focus card** (and behind an item taken
   now)? Recommend yes: it leads the rest of the unplanned, and its why line says "an agent waits".
2. **Where the sort card sits**: first of every run while items wait, or only at set times (morning,
   after lunch)? Recommend first of every run; `s` skips it.
3. **Promotion scope**: a whole source, or a source and key prefix (a channel, a routine)?
   Recommend the prefix: one noisy channel should not ride on a trusted one.
4. **Stalled after 2 working days with nothing moved?** Recommend yes; a setting only if asked.
5. *Decided: see Decided 15.*
6. *Decided: see Decided 16.*
7. **Carry-over at a new week**: offer to keep last week's focus, or start empty and wait for a
   proposal? Recommend the offer, unless a proposal is already waiting.
8. **An agent's own question filed over the socket** skips triage by default (its task is work
   the user took on), though the socket cannot prove the caller? Recommend yes, revocable as any
   promotion; a derived question always skips.
9. **Two Projects entries of one name** (`beacon/` and `beacon.md`): Decided 3's name is then
   ambiguous. Recommend refusing the name and taking the entry's path instead.

## Not verified

- That ottobot's runs can write to `<home>/decisions/inbox/`.
- The cost of reading the inbox folder on each pass of an existing shell thread.
