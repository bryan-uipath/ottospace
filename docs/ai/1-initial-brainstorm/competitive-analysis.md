# Competitive analysis

Research by an agent on 2026-11-11, at Bryan's request. Read directly: the Atuin threads post, its
Hacker News comments, the T3 Code README, Superlogical's Rex docs. Most other peer details come
from third-party summaries: medium confidence. Not researched: Zed's agent panel, Cursor's
background agents. VS Code's inbox mode is covered in `tasks-inbox.md`.

## Peers

| Product | What it is | Attention | Persistence | Worktrees | Other apps' windows | Scripting |
|---|---|---|---|---|---|---|
| Supacode | Command center for parallel agents (macOS) | unknown | Terminals survive app restarts (bundled daemon) | One per task | No | unknown |
| Herdr | Agent multiplexer (terminal) | Sidebar: blocked, working, done, idle | Multiplexer sessions | Workspaces → tabs → panes | No | API agents can drive |
| cmux | Native terminal (macOS) | Pane ring, sidebar badge, jump to latest unread | unknown | Shows branch and dir | unverified | `notify` CLI and hooks |
| Conductor | One worktree per agent (macOS) | unknown | unknown | One workspace per agent | Preview, diff | unknown |
| T3 Code | GUI over existing agent CLIs | PR state icon | unknown | New-worktree option, setup command | Terminal, diff view | MCP |
| Claude Squad | TUI over tmux | unknown | tmux keeps agents alive | One per agent | No | unknown |
| Warp | Terminal with agent features | Tabs, approval notifications | unknown | unknown | No | unknown |
| Rex (Superlogical, beta) | Multiplexer: server, macOS app, CLI | Programs report idle / working / blocked / done / error by escape sequence; tabs show a spinner or a blocked mark | Sessions live on a server | Sessions, tabs, blocks | No | Lua config, CLI, an agent skill |
| AeroSpace | Tiling window manager (macOS) | n/a | n/a | n/a | Tiles any app | TOML, CLI |

Sources: github.com/supabitapp/supacode, herdr.dev/docs, cmux.com/agents,
github.com/pingdotgg/t3code, superlogical.com/rex, github.com/nikitabobko/AeroSpace,
blog.atuin.sh/threads-agent-sessions.

## Where ottospace stands

- **Different:** it puts real app windows in the same tiles as agent terminals; it has a control
  socket that agents and tests drive; it plans focus blocks. No peer found does the first or the
  third.
- **Behind:** no attention status, no session record or resume, no worktree handling, no diff
  view, no persistence across restarts.

## Ideas to bring back

| # | Idea | From | For ottospace | Size |
|---|---|---|---|---|
| 1 | Attention status per pane | Rex, Herdr, cmux | A program reports working / blocked / done. Tab shows a spinner or a blocked mark. Pop next jumps to the first blocked tab | Medium |
| 2 | Persistent terminals | Supacode, Claude Squad, Rex | Shells run under a detached daemon; a restart reattaches with scrollback. Layout and cwd are saved | Medium to large |
| 3 | Session history and resume | Atuin | A picker lists agent sessions from their on-disk logs, by directory; Enter reopens one in a new tab | Small to medium |
| 4 | `capture` and `wait` requests | Rex | Read one pane's text; block until text appears or the prompt returns | Small |
| 5 | Pane and tab identity in the environment | Rex | `OTTOSPACE_PANE` and `OTTOSPACE_TAB` in each shell, so a program can address its own pane | Small |
| 6 | An agent skill | Rex | `ottospace skill` prints a guide for agents. Built | Small |
| 7 | Worktree per task with a setup command | Supacode, Conductor, T3 | "New task" makes a worktree and a tab and runs a per-repo setup command | Medium |
| 8 | Richer task rows | cmux, Warp | Branch, directory and last output line on each Tasks row | Small |
| 9 | Diff pane | Conductor, Claude Squad, T3 | A read-only `git diff` pane | Medium |

## Decisions (Bryan, 2026-11-11)

- **Attention status has two inputs, one model field.** The terminal escape sequence Rex uses is
  the main path for programs in terminal panes: the pane it arrives in identifies it, it needs
  no ottospace-specific hook, and it works through tmux. `ottospace ctl` is the second path:
  for tasks with no terminal, app panes, and structured changes. A status ring must not gate
  anything important, since any text printed in a pane can set it.
- **Show it:** a spinner while working, a distinct mark when blocked.
- **Session recording, if built, is local and opt-in.** Commenters on the Atuin thread worried
  about secrets in synced history.

## Order

1. `capture`, `wait` on a pattern, and the environment variables (4, 5).
2. Attention status with the spinner and blocked mark (1), then the tasks inbox on top of it.
3. Session restore as its own track (2). Session history (3) after the inbox.
