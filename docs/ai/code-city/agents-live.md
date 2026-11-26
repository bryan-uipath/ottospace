# Agents live on the city

Asked for after a task's agent ran a duped audit for minutes while its city showed nothing: "the
agent should at least be 'floating' over the city", "lerp between states", "a beam emanating
downward" for a shell command, "an antenna or pulse upward" for a network request, sub-agents
visualized too, and a text replay of a recorded session to debug a city that stayed quiet.

State: slice 1 **built** (`ottospace city-replay`); the rest proposed.

## What that session showed

Session `e4f4ee2a` (an ottospace chat, build `b1254a36`): 10 `Bash` calls and a `Skill`, no
`Read`, `Grep` or `Glob`. The reading happened inside `duped` and `jq`:

- `duped`, `jq`, `diff`, `ls`, `which` are not reads or searches (`glance.rs` `SEARCHES`, `READS`).
- `cd /tmp && jq …`: outside the checkout.
- `head -15 $p/…`: `$p` never expanded, so the path matches no building.
- `wc` inside `$(…)` inside an `echo` argument: never seen as a command.
- One `grep -rl … packages/*/package.json`: a glob scope (no place) and two found files (a brief flash).

Its icon should still have hovered over the middle of the city for the whole turn
(`Scene::marks`, no file). None showed. The replay says the core has the agent at every call
(mid-city 10 times, over `packages/ub-host/src` once): the loss is in the shell (`render_city`
drops a task with no `badges` entry), or in the real model differing from the replay's (a task
started fresh in the checkout).

Also seen: a search's `found` reads every stage's output on its line (`(eval)`, a `diff`'s
lines), not the search's alone. No place matches them: noise only.

Also seen: a watched session's `ToolStart` has an empty `detail` (`conversation.rs` `watched`), so
a watched session never produces a glimpse, even for a `Read`.

## Slices, each a pull request

1. **`ottospace city-replay <session.jsonl> [--dir <checkout>]`.** The session's lines through
   the live path: the runner's `Wire::read` (session lines have stream-json's shape), its events
   as `Action::AgentEvents` to a `Model` whose task works in the checkout, `city_at` after each
   event, labels and paths placed on the checkout's plan as `render_city` places them. Per call: its
   time and duration, the icon's place (file, near path, middle of the city, or none and why), lit
   files, glimpses, and why a call gave the city nothing. The agent's words are printed with control
   characters removed. Core logic in `ottospace-core` with tests; the subcommand reads the files.
2. **The missing icon**, on the side slice 1 shows, with a core test.
3. **Dwell and lerp.** A state the city shows holds a least time, then eases to the next: a read
   of 5 ms is seen. The icon's glide exists; a lit file, a beam and a glimpse fade in and out.
4. **Stations on the icon.** A shell call running: a beam down from the icon (to its near path's
   plinth, or the ground). A network call: a pulse up from it, an antenna. From the running calls'
   `Station`. Frames only while one runs; under Reduce Motion, drawn still.
5. **More is recognised.** `jq`, `diff` as reads; `p=x` assignments expanded in later words; `$(…)`
   read as commands; a path operand of any program (`duped report .`) a sweep of that place. The
   watch path passes the call's `detail`.
6. **Sub-agents as icons.** Each running sub-agent a small icon of its own, tied to its parent's by
   a line, with its own file, glimpses and stations (the transcript's `Face`s and each call's
   `parent`). A session read from its file: the reader gives each sub-agent's file, not one list.

## Effects by station (candidates)

Each from a running call's `Station` or a known outcome; nothing guessed from silence (`duped`
reading every file is not known, so it gets the shell's beam, not reads).

| Station / moment | Effect |
|---|---|
| Model (thinking) | the icon breathes slowly, calmer than a call's pulse |
| Files: read | the roof glows once in the agent's colour; reads in a row leave a fading trail |
| Files: write | the building rises by the change, then settles; new grows from the ground, deleted sinks |
| Search | the sweep and found lights (built); lines from the icon to found files |
| Shell | a beam down to its path or the ground; a failed exit flickers red once |
| Tests (`cargo test`, `npm test`) | the beam fans over test folders; a green or red ring at the end (an outcome) |
| Network | a pulse up, an antenna |
| MCP | as Network, tinted by server |
| Agents | a small icon splits off, glides to its file; at its end, back and merges |
| Skills | a short ring round the icon |
| Memory | the icon dips toward the city's edge |
| Hooks | a tick on the icon's rim |
| You (needs you) | the icon stops in the needs-you colour, a still marker; no pulse (⌘J leads) |
| Error | the icon desaturates; no motion |
| Turn done | the icon settles on its last building and fades |
| Commit / push | the files it touched pulse once (an outcome, as the Repos dot's) |

At most one effect per agent at a time, the newest: many sub-agents stay readable.

## Rules it keeps

- Ambient: nothing asks for frames at rest; nothing takes focus or moves layout.
- Every effect marks what an agent does now or just did; none plays uninvited after.
- Reduce Motion: effects at once and still.
