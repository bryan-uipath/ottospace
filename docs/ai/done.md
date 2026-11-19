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

## The model (`model/finish.rs`)

- `Finishing::away`: the tasks the user put away. Never in front (`Model::front` skips them: with
  only those, a workspace has none, so the drawer has no conversation), never on the rail. Kept
  until the user opens the task again (`TaskTalk`, `revive`), Undo, or the task goes.
- What its stopping agent reports last moves a task put away nowhere (`chat_state`).
- `Model::finish(subject)`: the header's buttons. `done_offered`, `close_key` (⌘W), `done_key` (⌘↵).
- `Effect::Seal`: the user's Done in the drawer on that task. The shell plays the animation.
- A click on a done task's row makes it a task again and goes on with its session, if it has one.
- Who: the user alone. `TaskDone`, `TaskLater`, `DoneUndo` and the answer to Later's question
  from any other origin are refused (`finish::USERS_ALONE`).
- Property rules: a task the user marked done has no running agent, or its stop was asked; one put
  away is not on the rail nor in front; the seal plays only for the user's Done.
