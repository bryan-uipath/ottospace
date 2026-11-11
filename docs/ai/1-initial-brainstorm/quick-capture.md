# Quick capture and task lifecycle

Refines two scenarios Bryan gave in conversation on 2026-11-11 (not yet in `docs/ideas/`).
Not agreed. This is a first design to react to.

## Source (Bryan's words)

> a 'quick capture' - ie, someone reports a bug in slack. i can press a shortcut key, circle it,
> maybe add some context, and dispatch to an agent. i can add or specify a repo (project) hint, a
> priority, but i should also just be able to quickly offload it too. i'd also like to think about
> a lifecycle - i end up with tons of worktrees littering me in supacode

> the command+command might be a good shortcut for that

## Quick capture

Goal: from any app, turn what is on screen into a task in under 5 seconds, then go back to work.

```
⌘⌘ (double-tap, global)
  → screen freezes; drag to circle a region        (Esc cancels)
  → capture card opens at the cursor:
       [ screenshot ]
       note:     ____________      (optional)
       repo:     auto ▾            (optional hint)
       priority: P2 ▾              (optional)
       ↵ dispatch     ⌘↵ dispatch and open
  → card closes. Focus returns to where it was.
```

- **Fast path:** ⌘⌘, drag, ↵. No typing. The agent works out repo and priority.
- **What is captured:** the image, plus context we can read without asking: source app, window
  title, and the text under the region from the accessibility tree. For Slack: the message text,
  author, channel, and a permalink if AX exposes one. Text beats OCR, so AX first.
- **What dispatch does:** creates a task in state `captured`. A triage agent then fills in what
  the user skipped (repo, priority, title), and starts work or asks one question.
- **Does not need ottospace focused.** Same global event tap as the return key. Caps Lock (F18)
  returns to ottospace; ⌘⌘ captures. Risk: ⌘⌘ can misfire on fast ⌘ shortcuts. Fire only when
  both taps are bare ⌘ (no other key between), inside 300 ms.
- **Where it shows up:** the task list, under "Running" or "Needs you". Nothing opens unless the
  user chose ⌘↵. A capture must not pull attention.

## Task lifecycle

Problem: tasks start easily and never end. Worktrees pile up.

Rule: **a task owns its resources. Ending the task removes them.** Nothing outlives its task.

```
captured → triaged → running ⇄ needs-you → review → done → cleaned
                         └──────────────→ parked ──(14 days)──→ stale
```

| State | Meaning | Resources held |
|---|---|---|
| captured | Offloaded. No one has looked | Capture only |
| triaged | Has repo, priority, title | None yet |
| running | Agent is working | Worktree, panes, processes |
| needs-you | Agent waits for the user | Same |
| review | PR is open | Worktree. Panes and processes closed |
| done | Merged, or dropped by the user | Being removed |
| cleaned | Nothing left but the record | None |
| parked | Paused on purpose | Worktree only |
| stale | Parked or idle too long | Worktree. Flagged for a decision |

- **Worktrees are created late.** Not at capture. Only when an agent starts to change code.
  A task that ends at triage ("duplicate", "not ours") never makes one.
- **Cleanup is one action**, and automatic on `done`: close panes, stop processes (dev servers),
  remove the worktree, delete the merged local branch, write one line to the weekly notes.
- **Cleanup is safe by rule, not by prompt.** Remove a worktree only if it has no uncommitted
  changes and no unpushed commits. Otherwise the task goes to `needs-you` with the reason.
- **Done is detected, not declared.** PR merged or closed → `done`. The user can also drop a task.
- **Stale sweep.** A task with no activity for 14 days (number to tune) is listed once in the
  weekly review: keep, park, or drop. No silent deletion.
- **Adoption.** On first run, list existing worktrees with no task. For each: make a task, or
  remove (same safety rule). This clears today's litter.
- **Limit work in progress.** Show the count of `running` + `needs-you`. Captures always succeed;
  the limit only delays when agents start.

## Fit with the architecture

- Core: `Task`, its state machine, and the cleanup rule (given facts like `dirty`, `unpushed`,
  `pr_state`, decide the next state and effects). All unit-tested.
- Shell: git, `gh`, processes, screenshot, AX reads. It reports facts to the core as actions.
- Control socket: `capture`, `task_list`, `task_done`. Agents use the same calls to file
  follow-up tasks.

## Open questions

1. Triage agent: runs inside ottospace, or does ottobot own it?
2. Task store: files in `~/notes` (readable by skills), or ottospace's own state directory?
3. Is "circle" a freehand lasso, or is a rectangle enough? Rectangle is much cheaper.
4. Review state: keep the worktree until merge (fast fixes), or remove at PR open and recreate?
