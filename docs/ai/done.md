# Done, Later

The owner's design (the canvas "Later vs Done", 2026-11-17): a clear way to end a task's
conversation.

- **Later** (header button, the dim × on a rail item's hover, ⌘W with the keyboard in the drawer):
  the agent stops, the task is `parked` (Later in Tasks), off the rail. Asks first while a turn
  runs: "Stop "<task>" and put it away?". Its row brings it back (`NeedsYou`, in front).
- **While a turn runs** the header has no Done (the owner: one stop button): the composer's Stop
  (`TalkInterrupt`, Esc Esc) stops that turn alone. Done shows once the agent idles.
- **✓ Done** (header, ⌘↵ in the conversation, the offer under the last reply): the task is
  `done`, its idle agent stopped, the drawer folds into its rail, the task leaves the rail. Its
  workspace stays on the top row. The notice bar says `Done · <task>` with Undo for 6 s
  (`finish::DONE_MS`, as every other Undo): Undo gives its state back and puts it in front. Not while a turn runs.
- **The offer**: "Got what you needed? ✓ Done · Later" under the last reply, while the agent is
  alive and idle, no card waits, the draft is empty and its last word is a reply.

## Its worktree, and the workspace after

The owner (2026-11-18): Done "deletes the task's own worktree if the task made one, without
asking"; the workspace stays, "we might want to start a new task in that workspace".

- `Task::made`: the worktree the new-task box made for it (`WorktreeAdded` of its `making`).
  Done asks `Effect::RemoveWorktree` while it is listed, unless another live task works there
  (`KEPT_TASK`) or a terminal is in it (`KEPT_TERMINAL`: decided in the core from the terminals'
  directories the shell tells first, so the worktree is kept, never pulled from under a shell).
- The shell runs `git worktree remove -- <path>` (no `--force`): git refuses a dirty one, and
  the shell counts `git status --porcelain` for why. `WorktreeRemoved` (the shell's alone; one
  the core asked for, else nothing): kept → the Done line says why; removed → `made.removed`,
  `ReadRepos`, and a workspace whose default it was falls back to its repository
  (`Workbooks::default_gone`).
- Only its own: listed at its path *on its branch* (one made there since, or switched, stays).
  A terminal counts by its shell's folder, its front program's and the folder it started in.
- Undo: `AddWorktree { fresh: false }` on its branch at the folder first asked for, in the
  worktrees folder it was made in (`Made::root`); the answer (`readded`) opens nothing and gives
  the workspaces that fell back their default again. Undo while git still removes it: added back
  once the removal is answered. A done task opened again (Done today) gets it back the same way,
  and its session goes on once it is (`Finishing::resume`).
- Done while git still makes it: removed once made (`late_worktree`).
- A new task in that workspace: with no task in front, it shares the newest put-away task's
  place, and a removed worktree's repository in its place (`shared_place`); so while one is
  being removed.
- New task here shows only while the done task's workspace is on show (`done_here`).
- Tasks: `Heading::DoneToday`, done since the local day began (`LocalDay`, the shell's, on its
  tick), above Done.

## The model (`model/finish.rs`)

- `Finishing::away`: the tasks the user put away. Never in front (`Model::front` skips them: with
  only those, a workspace has none, so the drawer has no conversation), never on the rail. Kept
  in `layout.json` (`drawer.away`) until the user opens the task again (a click, ↵ or ⌘J on
  it: `talk_to` revives it), Undo, a state reported since, or the task goes.
- What its stopping agent reports last moves a task put away nowhere (`chat_state`).
- `Model::finish(subject)`: the header's buttons. `done_offered`, `close_key` (⌘W), `done_key` (⌘↵).
- `Effect::Seal`: the user's Done in the drawer on that task (the core says it plays). Where each
  part is at `t` ms is `seal::frame(t, calm)` (pure, tested): seal 0–250 ms (ring overshoots
  and settles, tick draws, four sparks to 400 ms), fold 250–550 (into the rail), land 550–800
  (the item pops, a ✓ flashes), then `None`. Calm: tick and ring still, the drawer fades, no
  sparks, no pop. The shell (`seal.rs`) draws a ghost of the drawer and the rail item where
  they were last drawn; the drawer's own header and conversation stay drawn in it and fade out
  over the first 150 ms (`Frame::content`) while the ring and tick draw over them: a crossfade,
  as GPUI cannot scale an element's picture, over everything, and asks for frames only while it plays.
- A click on a done task's row makes it a task again and goes on with its session, if it has one.
- Who: the user alone. `TaskDone`, `TaskLater`, `DoneUndo` and the answer to Later's question
  from any other origin are refused (`finish::USERS_ALONE`).
- Property rules: a task the user marked done has no running agent, or its stop was asked; one put
  away is not on the rail nor in front; the seal plays only for the user's Done.
