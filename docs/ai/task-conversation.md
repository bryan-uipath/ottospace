# The task conversation

Status: the core is built (`ottospace-core`: `agents.rs`, `talk.rs`, the model's `talk` state).
The shell does not host an agent's terminal yet: see "What is built".

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
- The right rail (48 pt) mirrors the left one. Its first button is the conversation, with a
  state dot. It is there whenever a conversation exists. No button that does nothing.
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

1. Core (done): the table, the prompt, `default_agent`; the model's `talk` (task, open, keys),
   `talk_width`, `talk_wide`, `agents`; `TaskTalk`, `ComposeEnter`, `AgentStarted`,
   `AgentExited`, `TaskStop`, `TalkToggle`, `TalkKeys`, `TalkResize`, `TalkWide`; `Mode::Talk`;
   `Effect::StartAgent`, `Effect::StopAgent`. Unit and property tests.
2. Shell (not done): today it answers every `StartAgent` with `AgentExited`, so nothing
   starts and the keyboard stays where it is. To build: the drawer and its rail, the agent's
   terminal in it, the header, the tab strip and rail cells, row clicks and ↵ sending
   `TaskTalk` and `ComposeEnter`, the composer in the drawer, the empty state, the Tasks
   groups, fake `claude` and `codex` for `cargo e2e`.

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

- Codex takes no session id when it starts, so a task started with Codex has no session to
  follow: it runs until its terminal ends. Find its session by directory and start time?
- `claude --resume` looks for the session by the directory it runs in. A task's directory is
  its repository; an adopted session may have started in a folder below it.
- A session that still runs in the user's own terminal: going on with it from the drawer
  starts a second program on it.
- Tasks groups: Needs you, Running, To review, Later, Done. "To review" needs a state the core
  does not have; until then it is part of Needs you.
- The quit question does not ask about agents in the drawer yet.
- A sessions row (no task) in the drawer: `Talk.task` becomes a subject that may be a session.
- Does a client ever start an agent? Not in stage 1.
