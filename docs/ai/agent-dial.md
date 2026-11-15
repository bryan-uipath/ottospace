# The agent dial

The MEDIUM (96 pt) and SMALL (18 pt) sizes of the "Agent activity (radial)" mock, as a section
of each native conversation under its header. LARGE (a pane) is not built.

## The model

`ottospace-core::activity::Activity`, one per transcript (`Transcript::activity`), folded with
the transcript's own events and times (ms, passed in; no clock):

| Event | Activity |
|---|---|
| the user's message (`Transcript::send`), `MessageElsewhere` (watched) | You +1, a turn; failures since the last message cleared, and the error |
| `TextDelta`, `Text`, `Usage` with a context, a `ToolStart` | Model +1, once per step (a step ends with a call's end or a message) |
| `ToolStart { station, .. }` | that station +1; the call runs (lit) since then |
| `ToolEnd { failed }` | the call ends; a failure counts there, and is its red dot until the next message |
| `Hook` (`system.hook_started`) | Hooks +1 |
| `TurnFinished`, `Exited` | running calls end; a failed turn, or an end the user did not ask for (`exit 1`, `signal 9`), is the error |

Each use adds 1 to its station's weight, halved every 10 minutes (`HALF_LIFE_MS`); a spoke is
drawn by weight against the heaviest. The route keeps the last 6 stations (a station once in a
row) and shows those under a minute old. `Transcript::dial(now)` gives the `Dial`: per station
calls, failed, failing, lit and how long; the centre's state (running, needs you, idle, ended,
error); the context fraction and its level (amber ≥ 0.7, red ≥ 0.9); the route; the lines
(what it does now, the context, turns and time); the hovers. The shell draws only that.

A watched session (read from its file a message at a time) has no times: lit stations have no
arcs and the now line no clock. The collapse is the model's: `Model::dial_collapsed` is the
user's pick (`DialToggle`, the user's alone), else open while the agent runs or is watched.
Picks are kept with the layout (`Drawer::dials`). The property test checks that no action but
an agent's events, a message, or a start or end moves any dial.

## Stations

One table classifies calls: `ottospace_sessions::station::classify` (the session list's too);
the enum moved to the core so the dial can name it. All eleven are kept: each can be fed.

| Station | From |
|---|---|
| You | a user message; `AskUserQuestion`; lit while a permission request waits |
| Model | a model step; lit while the model has the turn and no call runs |
| Files | `Read`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `Grep`, `Glob`, `LS`, `apply_patch`, `view_image`; a shell line of file commands only |
| Shell | `Bash`, `BashOutput`, `KillShell`, `PowerShell`, `Monitor`, Codex's `exec`/`shell` |
| Network | `WebFetch`, `WebSearch`; a shell line that reaches the network (`git push`, `curl`); MCP servers other than browser and memory ones |
| Browser | MCP servers named `playwright`, `chrome`, `browser`, `puppeteer` |
| Agents | `Task`, `Agent`, `SendMessage`, `TaskOutput`, `TaskStop`, `Workflow`; Codex's agent tools |
| Skills | `Skill`, `ToolSearch` |
| Memory | memory MCP servers; file tools on a runner's memory files |
| Hooks | `system.hook_started` lines (Claude Code) |
| Tools | any other tool |

Differences from the mock: an MCP server that is not a browser or memory one goes to Network
(the shared table's rule), not to Tools. Colours are theme tokens (`theme::station`).

## Drawing

`crates/ottospace/src/dial.rs`: paths and quads in one canvas, built each paint. The app paints
on a change, and on a 1 s tick only while a station is lit or the route fades (`dial_moves`).
Reduce motion: the route keeps one opacity until it expires. Below 360 pt the lines are hidden.
