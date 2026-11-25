# The agent dial

> Since canvas J6 (slice 2) the open dial is drawn as the header's lights (`dial.rs`,
> `docs/reference.md`, "The agent's lights"); the model below is unchanged. The motion
> (springs, pulse, turning glyph, `counts.dial_frames`) is gone with the dial's face.

The MEDIUM (96 pt, in a face of 180 by 120 with its stations' words) and SMALL (18 pt) sizes of the "Agent activity (radial)" mock, as a section
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
drawn by weight against the heaviest. `Transcript::dial(now)` gives the `Dial`: per station
calls, failed, failing, lit and how long; the centre's state (running, needs you, idle, ended,
error); the context fraction and its level (amber ≥ 0.7, red ≥ 0.9); the lines (what it
does now, turns and time) and the timer under the centre; the hovers (the ring's: the context
used and the tokens). The shell draws only that.

A watched session (read from its file a message at a time) has no times: lit stations have no
arcs and no timer. The collapse is the model's: `Model::dial_collapsed` is the
user's pick (`DialToggle`, the user's alone), else open while the agent runs or is watched.
Picks are kept with the layout (`Drawer::dials`). The property test checks that no action but
an agent's events, a message, or a start or end moves any dial.

## Stations

One table classifies calls: `ottospace_sessions::station::classify` (the session list's too);
the enum moved to the core so the dial can name it. Each station can be fed.

| Station | From |
|---|---|
| You | a user message; `AskUserQuestion`; lit while a permission request waits |
| Model | a model step; lit while the model has the turn and no call runs |
| Files | `Read`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `Grep`, `Glob`, `LS`, `apply_patch`, `view_image`; never a shell line |
| Shell | `Bash`, `BashOutput`, `KillShell`, `PowerShell`, `Monitor`, Codex's `exec`/`shell`: any command, `ls`, `curl` and `git push` alike. It says a command ran; ottospace does not guess what the command did |
| Network | `WebFetch`, `WebSearch`, Codex's web search. Never a shell line or an MCP call |
| MCP | every MCP tool call, whatever its server: Claude Code's `mcp__<server>__<tool>`, Codex's `mcpToolCall` and calls in an `mcp__<server>` namespace. Never guessed from the server's name. It took Browser's place on the dial, which only browser-named MCP servers fed |
| Agents | `Task`, `Agent`, `SendMessage`, `TaskOutput`, `TaskStop`, `Workflow`; Codex's agent tools |
| Skills | `Skill`, `ToolSearch` |
| Memory | file tools on a runner's memory files |
| Hooks | `system.hook_started` lines (Claude Code) |
| Tools | any other tool |

Differences from the mock: MCP calls have their own station, not Tools, and there is no Browser
station. Colours are theme tokens (`theme::station`).

## Drawing

`crates/ottospace/src/dial.rs`: the header's lights row (canvas J6): the picture, the status
sentence, a light per tool station with its use bar, the context's number; the context bar on
the header's edge; the 18-point marker when collapsed. Nothing moves: the app paints on a
change, and on a 1 s tick while a station is lit.

## Sub-agents

Each `Agent`/`Task` call is a `transcript::Face` (its type, description, start and end, and the
sub-agent that started it). `Transcript::faces(level)` gives a level's in order (running first,
then the most recently active). Under the picture: up to three faces, then "+n", which opens the
list of them all. A face's click (`TalkDrill`) shows that sub-agent's own calls (each call's
`parent`, from `parent_tool_use_id`) in the same drawer; Back or Esc (`TalkBack`) goes up a
level. Only the user drills. The state says how deep (`dial.drilled`) and whether the list is
open (`dial.switcher`), never a description.
