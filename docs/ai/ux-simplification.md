# UX simplification review

Read at `origin/main` 80fab88 (2026-11-14). `file:line` is at that commit. Paths are under
`crates/ottospace/src/` unless they start with `core/` (`crates/ottospace-core/src/`).
`conversation.rs`, `talk.rs`, feature flags (#134, open) and the calendar request (#138, open) are
moving; their lines may shift.

Seen: the real UI. A private test instance (`OTTOSPACE_E2E`, private home and socket, fake agent,
fixture calendar) was driven into 18 states and captured by window id; images are not committed.
What they showed that the code alone did not:
- One task's badge (`FL`) is drawn six times at once: tab group, both pane headers, chat header,
  right rail, task row.
- The hovered task row's words (Snooze, Hold, Dismiss) paint over its detail line (`#auth · >`),
  and its title is cut to make room for `open`.
- The chat header at default width reads `Fix login re… claude-o… 0:04 mode: default Chat Terminal
  Stop ×`: the title loses to six other facts. `mode: default` shows when nothing was chosen.
  Stop is drawn twice (header and the working line).
- With a permission card up, three lines repeat its keys: the buttons `Allow (a)`, `Deny (d)`, the
  footer `a allow · d deny · s show all · m deny with a message`, and `esc esc stops`.
- WHO in the new-task drawer is seven chips (every runner × model), three rows tall.
- ⌘/ shows `off` as the key of two rows (return key, capture key) when they are not set: a row
  whose key is "off" teaches nothing.
- The home screen shows Needs-you while the Tasks section beside it shows the same task.
- After Allow, the call's line shows a red ✗ beside "You allowed Bash · git status." (seen once,
  with the fake agent: check whether the real result was an error or the mark is wrong).
- The Apps section with nothing listed is blank: header and ×, no empty state.
- Sneak labels sit on the first letters of what they label (`S ab 1`, `Q ERM`).


Counts: about 420 user-visible strings reviewed (shell literals, core labels that reach the
screen, 85 key-reference rows, 63 menu items, 59 palette names). 74 proposed for removal or
shortening (table in §2). 38 nouns a new user meets (§3). 65 static key bindings (46 actions;
arrows and HJKL both bound), 4 numbered families (⌘1–9, ⌘⌃1–9, ⌘⌥1–9, ⌘⇧⌥1–9: 36 chords), 2
global keys (return, capture), about 60 keys local to a mode. 14 modes (`core/model.rs:76`), 18
status-bar chip labels (`main.rs:4973-5000`).

## 1. The ten changes that simplify the most

Ranked by confusion removed over effort. S/M/L is effort.

### 1. One word per thing (S, strings only)

Now: the same thing has up to five names. The agent's pane is "conversation" (palette
`core/commands.rs:100-101`), "Chat" (`conversation.rs:1137`), mode `TALK` (`main.rs:4994`),
"drawer" (cheat sheet), "session" (`talk.rs:569`, `conversation.rs:1314`). A waiting task is
"Later" in Tasks (`core/tasks.rs:70`) and "Captured"/"On hold" on the home legend
(`home.rs:457-458`). A captured picture is "material" (`core/model.rs:5309`), "MATERIALS"
(`collect.rs:1027`), "capture" and "Brief" (`talk.rs:478`). "Workspace" is the home heading
(`home.rs:370`), a hover-card fact meaning "2 tabs · 3 panes" (`progress.rs:93`), and (decided) the
first top-row tab.
Why: every second name reads as a second concept. This is most of "I don't understand the
hierarchy".
Change: **Task** (the user's unit), **Agent** (what runs on it), **Chat** (the agent's pane; mode
chip `CHAT`), **Tab**, **Pane**, **Sidebar** (rail plus section panel), **Capture** (never
material/brief). **Later** everywhere a task waits (drop "Captured" and "On hold" from screen;
Hold becomes a Later reason in the hover card). **Workspace** only for the top-row tab.
Lost: the distinction captured-vs-held at a glance (still in the hover card and the `p`/`h` keys).

### 2. Status bar says state, not instructions (S)

Now: `NORMAL · shell · ⌘/ keys … 1 task needs you · 0.1.0 · a4a2582` (`main.rs:4978`, `:5009`,
`:5043`, `:5061`). The mode chip is drawn in every mode, including NORMAL. "shell" restates the
pane header. The version is always there.
Why: four facts on every frame, none of which changes what the user does next in NORMAL.
Change: no chip in NORMAL (the focus border colour already says the mode, `main.rs:4724`); no
focus text; version moves to the palette ("About ottospace") and the hover of an empty area of the
bar; "⌘/ keys" becomes hint-once (§4). Keep: chip in any other mode, needs-you count, copy text,
errors, back-to-app.
Lost: build tag at a glance when filing a bug (one palette command away).

### 3. Pane header: drop `TERM shell` and `EMPTY` (S)

Now: every terminal header reads `TERM shell` (`main.rs:4473`); an empty pane `EMPTY`
(`main.rs:4472`); an app `APP Window title` (`main.rs:4474`).
Why: "shell" is the same on every terminal; TERM/APP is what the pane's content already shows.
Change: terminal header shows its directory's last segment and the running program when not the
shell (`app-api · cargo test`); app header shows its icon and window title; empty pane no header
text. Keep ◆, fit, ×, copy text.
Lost: a word to tell a terminal from an app at a squint (the icon does it).

### 4. Remove the standard key footers and repeated prompts (S)

Now: picker `↑↓ move · ↵ open · esc close` (`main.rs:3734`), palette `↑↓ move · ↵ run · esc
close` (`commands.rs:234-235`), composer `↵ send · ⇧↵ new line · esc leave` (`conversation.rs:1097`),
new-task `↵ start · ⇧↵ file for later · esc leaves` (`main.rs:659`) and two longer variants
(`new_task.rs:286-287`), INSPECT/VISUAL/LIST/SNEAK/MARK hints (`main.rs:4981-4999`).
Why: these are how every Mac list and text box works; the user reads them once and then they are
noise on every frame.
Change: remove footers where the key is standard (↑↓, ↵, esc, ⇧↵ new line). Keep only the one
non-standard key per surface, as a key cap and no verb: new-task `⇧↵ later`; composer nothing.
Lost: the line that tells a first-time user esc closes (they try it anyway).

### 5. New-task drawer: one prompt, no paragraph (S)

Now: "NEW TASK" header (`new_task.rs:513`), "What needs doing?" three times (heading `:488`,
placeholder `:361`, idle box `:463`), "Say it in a line: an agent starts on it here, beside your
workspace." (`:493`), WHERE and WHO headings (`:495-497`), the plan line "↵ will start claude in
~/code/app on main" (`:451`), the tag hint `#project · >repo · @agent · /path · $skill · ^open ·
p1–p9` (`main.rs:658`) and the keys line.
Why: seven lines of text before the field. The user asked to avoid exactly this paragraph.
Change: the header and the field (placeholder "What needs doing?") and the WHERE/WHO chips with no
headings (`>` and `@` say which is which). Plan line only when it differs from the default
(another repo or a saved agent). Tag hint only while the line is empty, then gone; the dropdown
teaches the rest when a sigil is typed.
Lost: the explanation that an agent starts (the plan line and the drawer opening show it).

### 6. One meeting signal, not six (S)

Now: a meeting is in the tab-bar chip `Design review · in 30m`
(`core/calendar.rs:612`), the now-tick's card `Next: Design review at 14:00 (in 2h 3m)`
(`core/calendar.rs:619`), the status-bar `Design review in 4m` (`calendar.rs:381`), the band, the
rail dot (`main.rs:3847`), the banner `Meeting started · … · you are in no call`
(`calendar.rs:429-431`) and a notification.
Why: the band was added to be the simple form; the chip and the status line now repeat it.
Change: drop the tab-bar chip and the status-bar line; the band's block pulses from 10 minutes
(it does) and hover gives the details. Banner keeps "Meeting started", drops "you are in no call".
Rail dot only when the Calendar section is hidden by the sidebar being closed.
Lost: the meeting title readable without hover while the band is off (`calendar_ics` unset means
there is nothing to show anyway).

### 7. With two-level tabs, delete the right rail and its satellites (L, depends on the tab work)

Now: an agent appears in four places: right-rail badge (`talk.rs:672`), tab-group badge
(`main.rs:4828`), pane-header badge (`main.rs:4561-4566`), tab underline in its colour
(`main.rs:4896`). Linking tabs to tasks needs "Claim tab" (`main.rs:4341`), "Claim this tab" /
"Go to its tabs ⌘↵" / "New tab" (`core/talk.rs:59-61`), "New Tab outside the Task", "Remove Tab
from Task", "Move Tab to Task…" (`core/commands.rs:95-97`), the `open` chip on a row
(`main.rs:4374`), ⌘⌃1–9, ⌘↵, and ⌘⇧L "Close to the rail".
Why: this is the "tab agent hierarchy" confusion. The top row makes the task the parent, visibly.
Change: delete all of the above. A task's top tab carries its badge once; its tabs are the second
row; ⌘1–9 go along the top row, ⌘⌥… or ⌃Tab within the second. Moving a tab is a drag onto a
top tab (and one palette command).
Lost: switching agents without switching tabs (the chat drawer can still be pinned open on another
task: keep ⌘L).

### 8. Task row on one line (M)

Now: two lines, up to eight facts: badge, `P2`, title, `open`, tags, "1 material", "2 tabs, 3
panes", age (`core/model.rs:5305-5311`), live activity, model (`main.rs:4385-4390`), plus four
hover words (Snooze, Hold, Claim tab, Dismiss).
Why: the user called these rows dense. The second line is a different fact each time.
Change: one line: state dot, title, `P1` only when p1. The second line only while the agent is at
work (its live activity, muted). Everything else is in the hover card, which already has it
(`progress.rs:68-103`). Hover words: Snooze, Done (Hold and Claim go: Hold is `h`, Claim goes with
§7).
Lost: tags at a glance (Project view groups by `#project`, which is the tag that matters).

### 9. Other sessions: one line a row, no range switch (M)

Now: heading "Other sessions" with `Today | 7 days | 30 days | All` (`progress.rs:168-177`);
rows of three lines: name, `in pane 3`, runner; the model's title; state, age, model
(`main.rs:4204-4256`); hover Open, Adopt, Hide.
Why: densest list in the app, for agents the user did not start here. "Session" and "Adopt" are
implementation words.
Change: heading "Elsewhere"; rows: dot, title (the model's, else repo), runner icon. Range fixed at
7 days (setting for others). Hover words: Track (was Adopt), Hide. A click opens it as now.
Lost: model and age at a glance (hover card).

### 10. ⌘/ shows what you have not learned (M)

Now: 11 groups, 85 rows in three columns (`main.rs:3603-3680`), including a debug key ("⌘⌃A
Accessibility overlay (debug)", `:3650`), prose for mouse drags (`:442-447`), and misfiled rows
(⌘N, ⌘⇧N, ⌘H, ⌘Q, ⌘⌃E, ⌘; under TERMINAL, `:470-475`). No row for ⌘L, ⌘⇧L, ⌘↵, ⌘⌃1–9 or ⌘'.
Why: a wall the user scans once. The user asked for usage bars that hide what they know.
Change: the use-driven reference in §4.4; groups by what the user is doing (Panes, Tabs, Tasks,
Text, Modes), debug keys only in the palette, mouse gestures as one line each. Generated from the
keymap (`commands::key_of`), not a second hand-kept table, so it cannot drift.
Lost: nothing; the full list stays one click down ("Known").

## 2. Strings

`remove` means the element goes; position, icon or state carries it.

### Always-visible chrome

| Where | Now | Proposed |
|---|---|---|
| `main.rs:4978` | `NORMAL` chip | remove in NORMAL |
| `main.rs:4978` | `⌘/ keys` | hint-once (§4) |
| `main.rs:5008-5010` | `shell` / `empty pane` / `no pane` | remove |
| `main.rs:5061` | `0.1.0 · a4a2582` | remove (palette: About) |
| `main.rs:5043` | `1 task needs you` / `2 tasks need you` | `● 2` in the needs-you colour, tooltip "Needs you" |
| `main.rs:4473` | `TERM shell` | directory · program |
| `main.rs:4472` | `EMPTY` | remove |
| `main.rs:4474` | `APP` | app icon |
| `core/hosted.rs:120-125` | `needs 120 pt more width` | `fit` alone; the number on hover |
| `core/hosted.rs:74-75` | `no split to move` / `splits at their limit` | hover of a dim `fit` |
| `main.rs:4374` | `open` chip on a task row | remove (§7) |
| `main.rs:5069` | `P2`…`P9` on every row | only `P1` |
| `progress.rs:305` | `Today` + `2 done · 1 running · 1h 05m` footer | remove; in the Tasks header's hover |
| `progress.rs:160-161` | `State | Project` switch | keep; it is the one view choice |
| `core/calendar.rs:612` | `Design review · in 30m` (tab-bar chip) | remove the chip (§1.6) |
| `core/calendar.rs:618-620` | `Next: X at 14:00 (in 2h 3m)` (now-tick hover) | `X · 14:00` |
| `calendar.rs:381` | `Design review in 4m` | remove |
| `calendar.rs:431` | `you are in no call` | remove |
| `calendar.rs:574` | read status footer in Calendar | hover of the header |
| `home.rs:370-371` | `WORKSPACE` / `Your windows, apps and terminals go here` | remove both; the two key buttons stay |
| `home.rs:405` | `⌘⇧N next` | `⌘⇧N` |
| `home.rs:437-459` | legend of five states with counts | remove; the nodes have hover |
| `conversation.rs:596` | `Watching · updates per message` | `Read-only` |
| `conversation.rs:567` | `Another terminal runs it. Read-only: its calls and edits, never its words.` | remove (chip tooltip) |
| `talk.rs:527-528` | model and elapsed time in the chat header | hover of the title |
| `talk.rs:478` | `Brief` / `Agent` toggle | `Captures` / `Chat` |
| `conversation.rs:1136` | `mode: default` / `mode: plan` | hide `default`; keep others (agent state) |
| `conversation.rs:959-961` | `esc esc stops` + second `Stop` in the working line | remove both; header Stop stays |
| `new_task.rs:440-443` | a WHO chip per runner and model (7) | runners only (`@claude`, `@codex`); models from the `@` dropdown |
| `main.rs:3604-3605` | key `off` in ⌘/ for an unset return or capture key | hide the row |

### Empty states

| Where | Now | Proposed |
|---|---|---|
| `main.rs:4291` | `No tasks yet` / `Nothing in the inbox` + `New task ⌘N` | keep the button only; good otherwise |
| `main.rs:4453` | `No repositories in: …. Set OTTOSPACE_REPOS to the directories that hold yours.` | `No repositories` + `+ Add` |
| `marks.rs:457` | `No marks. ⌘B marks the focused pane, ⌘⇧B the tab on show.` | `⌘B marks a pane` |
| `marks.rs:461` | `⌘' then its letter · ⌘⌥1…9` | hint-once |
| `workbook.rs:251` | `No workbooks yet —` `Save this tab` | `Save this tab ⌘⇧S` |
| `calendar.rs:534` | `Nothing today` | keep |
| `main.rs:4673` | `⌘E to pick what goes here` | keep; good |
| `main.rs:4625` | `waiting for frames…` | remove (blank) |
| `talk.rs:569` | `Not running. A click on its task goes on with its session.` | `Ended · Resume` (a button) |
| `talk.rs:570` | `no agent runs for this conversation` | remove |
| `talk.rs:589-590` | `The agent ended. A click on its task/row goes on with its session.` | `Ended · Resume` |
| `conversation.rs:1096` | `The agent ended` / `a click on its task goes on with its session` | placeholder `Resume…`: sending resumes |
| `talk.rs:559` | `Ending the other view's process before this one starts…` | spinner, no text |
| `apps.rs:407` | `No app has that in its name` | `No match` |

### Hints

| Where | Now | Proposed |
|---|---|---|
| `main.rs:658` | `#project · >repo · @agent · /path · $skill · ^open · p1–p9` | only on an empty line |
| `main.rs:659` | `↵ start · ⇧↵ file for later · esc leaves` | `⇧↵ later` |
| `new_task.rs:286-287` | `↑↓ move · ↵ or tab take · esc close the list · ⌘↵ start` | remove |
| `new_task.rs:493` | `Say it in a line: an agent starts on it here, beside your workspace.` | remove |
| `new_task.rs:495-497` | `WHERE`, `WHO` | remove |
| `new_task.rs:463`, `:488` | `What needs doing?` (twice more) | remove; the placeholder stays |
| `new_task.rs:452` | `@x is a saved agent: ↵ files the task and starts nothing yet` | `↵ files it; nothing starts` |
| `new_task.rs:303` | `+ file  ⌘O` | `+ ⌘O` |
| `main.rs:3734` | `↑↓ move · ↵ open · esc close` | remove |
| `commands.rs:234-235` | `↑↓ move · ↵ run · esc close` / `… ↵ move the tab …` | remove |
| `conversation.rs:1097` | `Message the agent` / `↵ send · ⇧↵ new line · esc leave` | placeholder `Message`; no hint |
| `conversation.rs:1094` | `↵ sends once the turn ends · esc esc stops` | `esc esc stops` |
| `conversation.rs:959` | `esc esc stops` beside Stop | remove (the button is there) |
| `conversation.rs:1092` | `a allow · d deny · s show all · m deny with a message` | remove; the buttons show their key |
| `main.rs:4968` | `keys go to the app · F18 or a click on ottospace returns` | `F18 returns` |
| `main.rs:4969` | `keys go to the app · click ottospace to return` | `click to return` |
| `main.rs:4972` | `Slack · a click or a key enters it` | `Slack` |
| `main.rs:4981` | `type to filter · ↑↓ select · esc leaves` | remove |
| `main.rs:4983-4984` | `esc leaves · ⌘/ keys` | `esc` |
| `main.rs:4987` | `type a label · esc leaves · the accessibility overlay is hidden (⌘⌃A shows it)` | `esc` |
| `main.rs:4989` | `type a label · esc leaves` | `esc` |
| `main.rs:4993`, `collect.rs:1009` | annotate keys, twice | once, in the strip; drop from status |
| `main.rs:4994` | `keys go to the task's agent` | remove |
| `main.rs:4996-4999` | `type its name · ↵ keeps it · esc cancels` (×3 variants) | remove |
| `collect.rs:1010` | `click a capture to draw on it · esc cancels the draft` | remove |
| `collect.rs:1038` | `+ capture the pane in use ⌃⌥⌘Space` | `+ Capture ⌃⌥⌘Space` |
| `collect.rs:1106-1107` | `An agent in this task's terminals has OTTOSPACE_TASK set, and reads the brief with:` + command | remove (agents learn it from the skill) |

### Tooltips

| Where | Now | Proposed |
|---|---|---|
| `main.rs:3893` | `New task (⌘N)` | `New task ⌘N` (no parentheses, everywhere) |
| `main.rs:3895` | `Save this tab as a workbook (⌘⇧S)` | `Save tab ⌘⇧S` |
| `main.rs:4215-4217` | `Its conversation, in the drawer` / `Make a task of this session` / `Hide until ottospace starts again` | remove / `Track as a task` / `Hide` |
| `main.rs:4332-4343` | `Snooze 1 hour (S)`, `Hold (H)`, `Make the tab on show this task's`, `Dismiss (X)` | `1 hour · S`, `H`, remove, `X` |
| `talk.rs:468-470` | `Show the tab of this task shown last` / … / `A new tab of this task, with a terminal in its repository` | remove (§7) |
| `talk.rs:500` | `End this conversation's agent` | `Stop the agent` |
| `talk.rs:506` | `Close to the rail (⌘⇧L); the agent goes on` | `Close · agent keeps running` |
| `conversation.rs:544` | `Possible once it is idle in the other terminal` | `When idle` |
| `conversation.rs:580` | `Another process holds this session: each reply shows here once that message is complete` | `Runs elsewhere; read-only` |
| `conversation.rs:787`, `:793` | `Copy as plain text` / `Copy the message as its author wrote it` | remove (the word says it) |
| `conversation.rs:961` | `Stop this turn; the session goes on` | `Stop this turn` |
| `conversation.rs:1123` | `Show this conversation natively, or in the CLI's own terminal` | remove |
| `workbook.rs:294-331` | seven tooltips that repeat the word plus `(↵)`, `(A)`… | the key only |
| `apps.rs:291-294` | `Take off the pinned apps (P)` … | the key only |
| `main.rs:4911` | `Workbook: name` | `name` |
| `main.rs:4186` | `N in all` on Other sessions | keep |

### Notices and errors

| Where | Now | Proposed |
|---|---|---|
| `conversation.rs:1314` | `Session resumed. Earlier turns are in its file: Terminal shows them.` | `Resumed` |
| `conversation.rs:1307` | `Stopped. The session goes on.` | `Stopped` |
| `conversation.rs:1311` | `The context was compacted.` | `Compacted` |
| `conversation.rs:1312-1313` | `You allowed Bash · git.` | keep (audit trail) |
| `conversation.rs:1315` | `A message, in the other terminal.` | keep |
| `conversation.rs:1031` | `Cut: 1234 bytes in all. Allow opens once you have seen all of it.` | `Shortened · Show all to allow` |
| `main.rs:1066` | `tasks: 2 file(s) in the tasks folder could not be read` | keep (error, rare) |
| `workbook.rs:136` | `pick a workbook: A adds this tab to it` | keep |
| `core/workbook.rs:568` | `Saved "x": 3 tabs, 120 lines, 2 commands` | `Saved "x"` |
| errors in general | lower-case `area: what` form | good; keep the form |

### Palette and menu names

| Where | Now | Proposed |
|---|---|---|
| `core/commands.rs:100` | `Type in the Conversation` | `Focus Chat` |
| `core/commands.rs:101` | `Open or Close the Conversation` | `Toggle Chat` |
| `core/commands.rs:102` | `Go to Its Tabs` | remove (§7) |
| `core/commands.rs:95-97` | `New Tab outside the Task`, `Remove Tab from Task`, `Move Tab to Task…` | `Move Tab to…` only |
| `core/commands.rs:111` | `Drive the Bar with the Keyboard` | `Focus Sidebar` |
| `core/commands.rs:112` | `Label the Controls` | `Click by Letter` |
| `core/commands.rs:113` | `Select with the Keyboard` | `Select Text with Keys` |
| `core/commands.rs:115` | `Keys` | `Keyboard Shortcuts` (people type "shortcut") |
| `core/commands.rs:83-84` | `Even Out the Splits`, `Fit to the App's Window` | `Equalize Panes`, `Fit Pane to App` |
| `core/commands.rs:99` | `Capture the Pane` | `Capture` |
| missing | sidebar sections, Settings errors, About | `Show Tasks`, `Show Calendar`, `Show Repos`, `About ottospace` |
| `menus.rs:16-127` | names typed a second time beside `core/commands.rs` | build menus from `Command::name()`: one source |

### Dialogs

`confirm.rs:127-137` ("Quit?", "Quitting ends these:", "1 task's agent will be stopped"), the
paste ask ("Paste 4 lines?", `core/clip.rs:144`) and the draft discard (`collect.rs:1005`) are
already short and carry consequences: keep. `core/model.rs:152` `Stop its agent and dismiss?`:
keep.

### Vocabulary

| Thing | Names today | One word |
|---|---|---|
| The user's unit of work | task, inbox item, draft | **task** |
| What runs on it | agent, session, conversation, the CLI | **agent** |
| The agent's pane | conversation, chat, drawer, talk (`TALK`) | **chat** |
| An agent not started here | other session, session | **elsewhere** (heading) / agent |
| Make it a task | Adopt | **Track** |
| A waiting task | Later, Captured, On hold, Snoozed | **Later** |
| Out of the list | Dismiss, Done, Hide | **Done** for tasks; **Hide** for suggestions and sessions |
| What a capture keeps | material, materials, capture, brief | **capture(s)** |
| Left side | rail, bar, section, section list, secondary bar, list mode | **sidebar** |
| Its parts | section, pane (cheat sheet title "Apps pane") | **section** |
| The thing that splits | pane, split, window | **pane** |
| The thing in the top strip | tab, group, task's tabs, workbook tab | **tab** (top row: task / workbook / Workspace) |
| A tab's home | workspace, home screen | **Workspace** (top-row tab only) |
| "Needs you" | needs you, waiting, a request waits | **needs you** |

Labels that restate position or icon: `TERM`, `APP`, `EMPTY`, `shell`, `NORMAL`, `WORKSPACE`,
`NEW TASK` inside the new-task drawer whose header already says it (`new_task.rs:382`), `TASKS`
header over the Tasks list (the rail icon is lit beside it), `WHERE`, `WHO`, `Workbook:` in a tab
tooltip next to the workbook icon.

## 3. Concepts and hierarchy

Nouns a new user meets today (38): tab, pane, split, terminal, app, empty pane, workspace/home,
task, agent, conversation, session, drawer, right rail, left rail, section, inbox, project, repo,
skill, reference (`^`), path tag, priority, brief, material, capture, annotation, workbook, mark,
palette, picker, sneak label, list mode, visual mode, peek, parked window, return key, meeting
band, snooze/hold/later.

Smallest set that does the job (8): **task**, **agent/chat**, **tab**, **pane**, **sidebar**,
**palette**, **capture**, **workbook**. The rest is either a tag (`#`, `>`, `@`, `/`, `$`, `^`,
`p1`) learned from the dropdown when typed, or a power feature behind a key that the user meets in
the palette.

Two concepts that could be one:
- **Workbook and task**: both are a named set of tabs on the top row. A workbook is a task with no
  agent that does not end. Under two-level tabs, consider "pin" on a task's top tab in place of a
  separate Workbook section. Not now; note it.
- **Mark and pinned top tab**: marks (flagged off) are a way back to a pane or tab. The top row
  makes tabs reachable by ⌘1–9 already; leave marks off and do not invest more.
- **Right rail and tab group badge**: the same thing twice; §1.7.
- **Captured and On hold**: both mean later; §1.1.

Implementation concepts on screen: `session` (Claude Code's file), `Adopt`, `in pane 3`, `tab on
show`, `Claim`, `material`, `OTTOSPACE_TASK` in the brief view, `OTTOSPACE_REPOS` in the Repos
empty state, `mode: plan` (fine: the user set it), token in/out/cached columns in the hover card
(fine on hover), version tag.

The two-level tab model changes the hierarchy from "tabs that may belong to a task, shown as
badges in four places" to "a task is a tab; its tabs are under it". With it, delete: the right
rail; ⌘⌃1–9; ⌘⇧L "close to the rail"; `Go to its tabs`, `Claim this tab`, `New tab` in the chat
header; "Claim tab" on rows; three tab/task palette commands; the pane-header task badge; tab
underlines; the `open` chip; `talk::Go` itself. It also makes `⌘⇧N` ("next task that needs you")
a jump along the top row, and the home screen's Needs-you list redundant with the top row's
needs-you badges (keep the home buttons only).

## 4. Incremental discovery

### What the user faces

- **Minute 1**: an empty tab with "WORKSPACE / Your windows, apps and terminals go here", two
  buttons (Open an app ⌘E, Terminal ⌘D), the lattice and a five-state legend; the Repos section
  (cheat sheet: "opens on the Repos list"); the new-task drawer on the right (`Model::welcome`)
  with its seven lines; the status bar `NORMAL shell ⌘/ keys`. Must know: click a repo or press
  ⌘N. Too much at once: three surfaces each asking for the first action.
- **Hour 1**: ⌘N, ⌘D/⌘E, ⌘W, ⌘T, clicking a task, the chat, ⌘L, ⌘C/⌘V, the return key when an
  app has the keyboard. Meets: APP mode (and how to get out), permission cards, needs-you.
- **Week 1**: ⌘P, ⌘/, ⌘1–9, splits and resize, capture, `#`/`>`/`@` tags, snooze, sessions
  elsewhere, calendar. Can ignore for a week: visual mode, sneak, list mode, marks, workbooks,
  swap/move-to-tab chords, ⌘⌃0 fit, ⌘⌃F, inspector, ax overlay, ⌘⇧⌥1–9, all `^`/`$`/`/` tags.
  That is roughly 70 of the ~100 global chords and 11 of the 14 modes.

### Layering

1. **Zero knowledge (always visible)**: the field "What needs doing?", the top row, `+` on the top
   row, panes with their ×, the sidebar icons. One key shown on each empty state, as a key cap.
2. **In context, once**: a key cap that appears on the thing it acts on the first time it would
   help, then never again (hint-once, below).
3. **On demand**: ⌘P (every action, with its key) and ⌘/ (keys you have not learned).

### Mechanisms, judged

- **Palette as the single front door — recommend, primary.** It already shows each command's key
  (`commands.rs:100`, `key_of`). Gaps to close: every sidebar section, every list action
  (snooze, hold, track, pin), About, Reset Key Counts; aliases ("shortcut" finds Keys, "agent"
  finds chat). The menu bar built from the same `Command` list. Rule: an action with no palette
  entry does not ship.
- **Empty states offer the one next action as a key — recommend.** Already right in Tasks and the
  empty pane. Fix Repos, Marks, Workbook (§2). One action, one key cap, no sentence.
- **Hint-once with a persisted seen set — recommend, sparingly.** Candidates, each shown on the
  element it concerns, as a key cap that fades after 4 s or on use: ⌘L on the chat header the
  first time a chat opens; the return key on the APP chip the first time an app takes the keyboard
  (this is the one that strands people); ⌘P in the status bar after the first menu-bar use; ⇧↵ in
  the new-task field on first open; ⌘B on a pane header only if marks are on. Retire a hint
  forever when its key is used once (shared with §4.4's counts) or after it was shown three times.
  At most one hint on screen. Store: `<home>/seen.json`. No hint text longer than a key cap and
  one word.
- **Progressive status bar — do not.** The status bar is the densest strip already; making it a
  teacher adds text the user asked to avoid. It should shrink (§1.2).
- **Feature flags as stages — recommend.** Keep `marks` and `apps_section` off; add flags for
  sneak (and its ⌘;/⌘⌃G), visual mode, the inspector/ax overlay (developer), Other sessions, and
  workbooks until the top row absorbs them. A flagged-off feature has no key, no palette entry, no
  ⌘/ row, no menu item. Enable from the palette ("Enable Sneak…") so turning one on is itself
  discoverable. This is the strongest lever on the 38 nouns.
- **Short guided first task — do not.** It is a paragraph by another name, it runs once, and the
  welcome drawer already is the first task. Make that drawer quiet (§1.5) instead.

### 4.4 Key usage bars in ⌘/ (wanted)

Behaviour:
- **What counts as a use**: a binding fired by a key the user pressed: `Origin::User` and the
  event came from the keyboard. Not the palette, not a menu click, not a socket client (including
  `"as":"user"` in tests), not a sneak label. Count per command, not per chord: arrows and HJKL
  are one row; a numbered family (⌘1–9) is one row. Mode-local keys (list `s`/`h`/`x`, visual `v`
  `y`, permission `a`/`d`) count as their own rows, second PR.
- **Recently**: a decaying score, half-life 7 days (≈ two weeks of memory). Stored as `(score,
  at)`; on each use `score = score · 0.5^((now - at)/7d) + 1`. The clock comes in the `Action`,
  so the rule stays in the core with a unit test.
- **Learned**: score ≥ 5 (five uses in a few days, or a steady habit). Hysteresis: it leaves
  "learned" when it decays below 2, which is about nine idle days. So a key unused for a long time
  returns to the main list by itself, with its bar mostly empty again: the reminder the user
  wants, no extra rule.
- **Layout**: each group lists unlearned keys first, ordered by score ascending (never-used at the
  top). Learned keys of all groups go into one folded group at the bottom, `Known · 12`, opened by
  click or space. No numbers on rows.
- **The bar**: 2 pt high, 24 pt wide, under the key cap, muted track, fill in the primary colour
  at 60% opacity, filled to `score / 5`. No bar at score 0 (an unused key looks like a plain row,
  which is the point). No label, no tooltip.
- **A palette run of a command that has a key**: does not count as a use; it marks the row
  "looked for": it sorts to the top of its group and shows the hint-once key cap next time the
  command's context comes up. The palette itself shows no bar: it already shows the key, and a
  bar per row would make it dense.
- **Hint-once**: the same store. A key with score > 0 never gets a hint-once.
- **Feature flags**: a disabled feature's keys are neither listed nor counted.
- **Storage**: `<home>/keys.json`, `{command: {score, at}}`, written with a debounce, never in the
  socket state, never synced. A test run writes none unless it names `OTTOSPACE_HOME`. Palette
  command "Reset Key Counts". Tension: the e2e loop wants to read the state; keep the decision in
  core unit tests, and have e2e check only that ⌘/ draws a `Known` group after keys are sent, via
  the key-reference bounds the snapshot already allows, not the counts.

## 5. Density

| Element | Lines / facts now | Move |
|---|---|---|
| Task row (`main.rs:4319-4398`) | 2 lines, up to 8 facts, 4 hover words | 1 line (dot, title, P1); live line while working; rest to the hover card |
| Session row (`main.rs:4204-4264`) | 3 lines, 7 facts, 3 hover words | 1 line (dot, title, runner icon) |
| Task hover card (`progress.rs:68-103`) | title, state, tags, 3 facts, token table, context bar | fine for hover; drop the `cached` column |
| Chat header (`talk.rs:509-535`) | badge, title, model, elapsed, watching, Chat/Terminal, Go, Brief, Adopt, Stop, × | badge, title, Chat/Terminal, Stop, ×; model and time on title hover; Go goes (§7); Brief only when captures exist (already) |
| Status bar (`main.rs:5012-5062`) | chip, focus, hint, back, meeting, needs-you, clip, error, version | state only (§1.2) |
| Top bar (`main.rs:4796-4961`) | task badges, tabs with ◆ and workbook icons, meeting chip, 3 buttons | tabs, + and the two split buttons; meeting chip goes |
| New-task drawer (`new_task.rs:403-527`) | 7 text lines over the field | header, chips, field |
| Home screen (`home.rs:333-499`) | heading, sentence, 2 buttons, Needs-you list, Running line, legend, lattice nodes | 2 buttons and the lattice |
| ⌘/ (`main.rs:3603`) | 85 rows, 3 columns | §4.4 |
| Calendar row (`calendar.rs:631-640`) | time, title, place line | fine |

## 6. What not to simplify

- **Permission cards** (`conversation.rs:1015-1080`): tool, detail, "The agent says:", Allow /
  Deny / Deny with a message / Show all, the gate that Allow opens only after Show all on a cut
  card. Keep every part; only the footer of keys (repeating the buttons) can go.
- **"You allowed Bash · git."** notices: the audit trail of what was let through.
- **Untrusted-content marking** in briefs (`docs/ai/capture.md`): keep the quoting and label.
- **A task's badge in one place per row of the hierarchy** (top tab, chat header, task row): the
  identity colour is how the user maps chat to tabs. Remove the extra copies (pane headers, right
  rail), not the badge.
- **Read-only watch state**: shorten to `Read-only`, never remove: typing into a watched session
  would mislead.
- **Needs-you**: the count, the amber rail cell and dots. It is the reason to look.
- **Copy/cut/paste feedback**: the pulse and "Copied 3 lines" in the pane header, and the paste
  ask for multi-line text. The user asked for this.
- **APP mode chip and the return key**: the one mode a user gets stuck in. Keep the chip, shorten
  the text (§2).
- **Quit and dismiss confirmations** that name what will stop.
- **"runs in ~/notes/"** on the new-task line when a folder is picked: it says where an agent will
  write.
- **`mode: plan`** and the agent's model where the user chose it: agent state, not decoration.
- **Settings errors** in the status bar (`settings.json:3:1 trailing comma`): short already, and
  the only place they show.

## 7. Suggested PR sequence

Independent unless noted. Strings first.

1. **Vocabulary pass**: task / agent / chat / later / capture / sidebar in labels, palette names,
   tooltips, notices (§2 vocabulary). Strings only, plus `TALK`→`CHAT`.
2. **Hint footers out**: remove standard key footers in picker, palette, composer, new-task;
   shorten mode hints. Strings only.
3. **Status bar to state**: no NORMAL chip, no focus text, version to an About command.
4. **Pane header**: drop TERM/EMPTY/shell; directory · program for terminals; app icon.
5. **New-task drawer quiet**: one prompt, no paragraph or WHERE/WHO, tag hint on empty line only.
6. **Meeting surfaces**: drop tab-bar chip and status-bar line; banner text shorter.
7. **Empty states**: Repos, Marks, Workbook, ended agent (`Resume` button; sending resumes).
8. **Menus from `Command`**: one source for menu and palette names; add missing palette entries
   (sections, About, aliases).
9. **Task row one line**: facts move to the hover card; hover words Snooze and Done.
10. **Elsewhere**: Other sessions rows one line, Track/Hide, fixed 7-day range.
11. **Key usage counts (core)**: `Action::KeyUsed`, decaying score, learned/known rule, unit
    tests, `keys.json`, "Reset Key Counts". No UI.
12. **⌘/ by use**: generated from the keymap, unlearned first, bars, `Known · n` group, flag-aware.
    Depends on 11.
13. **Hint-once**: `seen.json`, at most one hint, the five hints of §4; shares 11's store.
    Depends on 11.
14. **More flags**: sneak, visual mode, developer tools, Other sessions; enable from the palette.
    After #134.
15. **Two-level tabs cleanup**: delete right rail, ⌘⌃1–9, ⌘⇧L, Go/Claim, tab/task commands,
    header badges, underlines, `open` chip. Depends on the two-level tab work.
16. **Home screen**: two buttons and the lattice; Needs-you list goes. After 15.
17. **Site and docs in step**: `site/shots/*.png` show `@repo` tags and a "Captured" heading
    (stale against `>repo` and "Later"); retake after 1–5. `docs/cheat-sheet.md` follows each PR.
