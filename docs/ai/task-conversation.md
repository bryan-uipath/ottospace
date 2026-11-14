# The task conversation

Status: stage 1 is built: the drawer hosts the agent's terminal, the composer lives in it, the
right rail lists open conversations, Codex sessions are found; the Tasks groups, the Project
view, time and tokens on the task, the hover card. See "What is built".

## Three zones

| Zone | Holds | Rule |
|---|---|---|
| Left: rail and section list | Things to pick: apps, tasks, repositories, marks | A click picks; it never rearranges the center by surprise |
| Center: the workspace | Tabs and panes of windows, apps and the user's own terminals | The user's. Starting or showing a conversation does not switch it |
| Right: the drawer, and its rail | One conversation at a time: a task's agent | In no tab and no split. Switching tabs keeps it |

- The tab strip runs from the workspace's left edge to the drawer's left edge. The strip above
  the rail and the section list holds the traffic lights only.
- The drawer's header names the task: state dot, title, model, elapsed, `Brief`, Stop, ×. The
  task label in the tab strip goes.
- The right rail (48 pt): a cell per open conversation (`Model::rail`: needs you, then running
  in start order, then the last opened; at most 8), each a two-letter badge (`talk::monograms`,
  told apart when two collide) ringed in the task's state colour, a steady dot while it works,
  a tooltip (title, what it does now, where, who, how old). Then a divider and `+`. A click
  shows that conversation; on the one shown, closes the drawer. ⌘⌃1…9: ⌥ is Meta in terminals.
  Nothing on the rail moves.
- Left rail cells are square and as wide as the rail; the active one has the panel's
  background, so it reads as joined to the panel.

## One verb

There is no Show, Open or Start. A click on a task, or ↵ on it, always does something a
person can see:

| The task | A click or ↵ does |
|---|---|
| Never started | Starts its agent; the conversation opens with the keyboard |
| Has a session, no live terminal | Goes on with that session; the conversation opens |
| Its agent runs | The conversation opens with the keyboard |
| `@agent` is a saved agent | The conversation opens and says a saved agent cannot be started yet |

The center does not change. When it shows another task's tabs, the header offers
`Go to <workspace>` (⌘↵).

↵ in the composer files the task and starts it. ⇧↵ files it for later. × on the drawer, or
⌘⇧L, closes it to the rail; the agent goes on. ⌘L moves the keyboard into the conversation and
back.

## The composer

One composer, in the drawer. ⌘N, the Tasks section's +, and the rail's + open the drawer in
its new-task state: the field, tags, dropdown, file chips and `^` references of today's box
(`new_task.rs`, `field.rs`), and a line that says what ↵ will do. On ↵ the drawer becomes that
task's conversation. No modal over the workspace, and no line on the home screen.

With nothing open (no pane, no task) the drawer is open by itself on the composer, under
"What needs doing?", with one-click picks: WHERE (the first listed repositories, "no
repository") and WHO (the default agent, then each runner and model). The center says
"WORKSPACE · Your windows, apps and terminals go here" with "Open an app ⌘E" and "Terminal ⌘D".
Esc there moves the keyboard to the workspace and leaves the drawer showing.

The capture draft is the same drawer in its collecting state.

## Starting an agent

- **Who.** `agents::read` reads the task's `@agent`; with none, the `default_agent` setting,
  else `claude`. A saved agent names no runner: nothing starts.
- **The command** is a fixed table in `agents.rs`, read from `claude --help` and `codex --help`:

  | Runner | New session | Go on with a session |
  |---|---|---|
  | `claude` | `claude [--model <id>] --session-id <uuid> -- <prompt>` | `claude --resume <uuid>` |
  | `codex` | `codex [--model <id>] -- <prompt>` | `codex resume <uuid>` |

  Only three things are put into it: a model id that is a name (`agents::is_name`), a session
  id that is a UUID, and the prompt, as one argv element after `--`. Never a shell string.
  The shell is to run it through the user's login shell for its `PATH`, with the program and
  arguments as positional parameters (`"$SHELL" -lic 'exec "$@"' -- claude …`): the script is
  fixed text, and nothing of a task is ever part of it.
- **The prompt** is `Task: ` and the task's title (the fixed start keeps a title such as
  `/logout fails` from being read as a command of the runner's own) and, if the task has a brief, the path of `brief.md` and
  fixed words that tell the agent to read it and to treat what it quotes as data. The
  brief's text, which holds what was captured from the screen, never goes into argv. A task
  filed from captures alone has a title made of fixed words.
- **Who may.** Only the user (`Origin::User`): `Model::refusal` refuses `TaskTalk` and
  `TaskStop` from the socket, and a client's ↵ or `open` files the task and shows its tabs, as
  before, with no agent. The property test fails on a `StartAgent` any other origin caused.
- **Following.** Claude takes the session's id, so the task is linked to its session from the
  start and follows it by the rule Adopt uses: Running while it works, Needs you when quiet.
  When the agent's terminal ends while the task runs, the task needs you.

## What is built

1. Core: the table, the prompt, `default_agent`; the model's `talk` (task, open, keys),
   `talk_width`, `talk_wide`, `agents`; `TaskTalk`, `TalkPick`, `ComposeEnter`, `ComposePick`,
   `AgentStarted`, `AgentSessions`, `AgentExited`, `TaskStop`, `TalkToggle`, `TalkKeys`,
   `TalkResize`, `TalkWide`; `Mode::Talk`; `Effect::StartAgent`, `Effect::StopAgent`;
   `rail`, `compose_picks`, `compose_plan`, `welcome`. Unit and property tests.
2. Shell (`talk.rs`, `new_task.rs`): the drawer and the right rail; the agent's terminal
   (`Terminal::run`, through the login shell, program and arguments as positional
   parameters); the header; the tab strip from the section list to the drawer; square rail
   cells joined to their list; row clicks and ↵ as `TaskTalk`; the composer in the drawer (no
   modal, no home-screen line); the empty state; Codex's session read on a thread of its own.
   `cargo e2e` runs stand-in `claude` and `codex` (`OTTOSPACE_AGENTS_DIR`, test runs only;
   without it a test run starts no agent).
3. Progress: the State view (Needs you, Running, Later, Done folded) and a Project view
   (`tasks::by_project`, `Segments`), rows with the session's live line and model, a Today
   footer (`Model::today`, local midnight passed in), a hover card (`Model::card`). The task's
   file keeps `started_at`, `finished_at` and `work`: per session its time at work
   (`progress::Clock`: activity until a two-minute quiet gap) and tokens by model, read on by
   `ottospace-sessions::tally` and replaced whole each read, so a restart counts nothing twice.
   Unknown keys of the file are kept (`Task::extra`): the session-restore layer can add a task's
   tabs and layout there with no format break.

One seam: the drawer asks one function what to show for a conversation. Stage 1 answers "the
agent's terminal".

## Later stages (not built)

- **Stage 2: a native conversation.** Drawn from `ottospace-sessions` events: text that copies
  and pastes, links that open, markdown and code drawn as such, pictures shown. The terminal
  stays as a fallback tab.
- **Stage 3: activity.** The agent at the center, spokes to files, shell, network, MCP,
  ottospace and sub-agents, and context as a ring; a strip of the same at the top of the
  drawer. Quick answers. Read access to workspace panes for the agent.

## Open questions

- **Codex's session** (built): `codex` takes no id at start, and nothing we could set at start
  comes back in the rollout. 0.166 `--help`: no option names a session; `-c` keys are config,
  not written to `session_meta`; `originator` is an internal override also sent to OpenAI as a
  header; the prompt would put the id into the model's context; and the process that has the
  rollout open may be the shared app-server daemon, not ours. So after starting it the shell
  reads the rollouts in the task's folder (`ottospace-sessions`), and the core
  (`agents::find_sessions`) links the task only to the one session that may be its: Codex's,
  that folder, `session_meta.timestamp` at or after the start (taken before the program
  starts; a time written to the second, in that second) and before the next agent there
  started, within a minute, a UUID, and that no task or open conversation has; and no other
  agent's it may be. Else it waits; after a minute the task follows none. Two agents in one
  folder started one after the other take one session each; two started before either wrote
  take none.
- **Where a session is gone on with** (built): `claude --resume <id>` looks in the directory it
  runs in first (since 2.1.223 then in every project), and the agent works there, so a session
  is resumed in the folder it started in (`Found.cwd`), when
  that is inside a listed repository or worktree (`sessions::below`, then again in the shell
  with links resolved). Not listed (the Tasks section is closed): that one session is read
  first (`ReadSession`), and the resume waits on it; read and not found: the task's repository.
- **A session in use elsewhere** (built): one at work (its file changed in the last 30 s), or
  held (Claude Code records a live process for it: `sessions/<pid>.json` and a pid that
  exists), is not gone on with: the drawer opens on it and says it is live in another
  terminal. Claude Code's docs (code.claude.com/docs/en/sessions): "If you resume the same
  session in two terminals without forking, messages from both interleave into one
  transcript"; a running *background* session is attached to instead (since 2.1.285). Not
  tried against a live session. Codex records no process: an idle Codex session open in
  another terminal is gone on with.
- "To review" needs a state the core does not have; until then a review is under Running.
- The quit question does not ask about agents in the drawer yet.
- **A session that is no task's** (built): `talk::Subject` is a task's id or a session's
  runner and id. A click on its row opens it as `TaskTalk` does, with no task; it has a rail
  cell (badge from its title, else its repository; ring only while at work) and Adopt in the
  header. Adopt moves its conversation to the new task (`Effect::AgentMoved`): the same program
  if it runs. Only the user's `SessionOpen` and Adopt start one; a client's `session_adopt`
  makes the task only.
- Does a client ever start an agent? Not in stage 1.
