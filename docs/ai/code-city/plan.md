# The atlas, and the code city on it

A map where a thing is always in the same place, so the person learns where things are. The
first map is the code city: a repository's folders as districts, its files as buildings, drawn
behind a task's workspace in place of the lattice. The same map can hold other trees later
(Jira, Slack, tasks): the layout knows nothing of files.

Asked for as "a native version of code city, focused on just the file / folder / coupling
visualization ... leveraging human spatial recall", then "generalize the concept, so it could
apply to jira / slack / etc". The owner's own 3D city (`~/code/codebase-visualizer`, Three.js,
TS/JS only) was read for its rules, not its look: footprint and position never change across
modes, height may; coupling arcs only for the selected node; a folder with one child is no
district of its own. Its backlog asks for a "history-resistant layout": that is this note's
main subject.

State: **built** = in this pull request or the one stacked on it. Everything else is proposed.

## Principles this is held to

- **Spatial recall.** The same tree gives the same map on every launch. A change to one thing
  moves as little else as possible. A layout that looks better and reshuffles is a failure.
- **Attention is scarce; ottospace stays out of the way** (`docs/ai/attention/plan.md`, #305).
  The map is ambient: it never animates for attention, never takes focus, and asks for no
  frames while nothing changes (as the dial: `counts.dial_frames`).

## The model (built: `ottospace-core::atlas`)

A source gives a tree of `Node`s. The layout (`atlas(&root) -> Atlas`) gives each a `Place`: a
rectangle in fractions of the plan.

| Field | What | Rule |
|---|---|---|
| `id` | the node's own, for good | what a place, a link, an activity or a beacon names |
| `label` | what is written on it | never in the state or the logs |
| `footprint` | sizes a leaf on the plan | **slow measures only**: a change reflows. Already coarse (the source's size classes) |
| `height` | a second measure, 0 to 1 | drawn only, moves nothing: **fast measures go here** (or on shade), never on footprint |
| `children` | in layout order | a source that keeps old before new keeps old in place |
| links (beside the tree; proposed, not in the code) | two ids and a weight | the layout does not read them; drawn for one node at a time |

Two more channels are the caller's, as sets of ids, not part of the tree:

- **Activity**: "an agent is here", with the agent's colour. Built for files (below).
- **Beacon**: "something here is addressed to you". Proposed (below).

## The layout (built)

**Coarse ordered bisection.** A group's children, in the source's order, are cut in two at the
child boundary nearest the middle of the group's weight, each half again; the cut runs across
the longer side (as if the plan were 1.6 wide to 1 tall, whatever the window: a resize
stretches the map and never rearranges it). Two things make it stable:

1. **Coarse weights.** A leaf's footprint is a size class (a file: 1 unit up to 2 KB, doubling
   to 64 units at 128 KB). A group weighs its children's sum **rounded up a step** (4, 5, 6 or 7
   times a power of two: 100 → 112). So an edit seldom changes any weight, and a group that
   gains a child most times weighs the same from outside: nothing outside it moves.
2. **Free ground.** The room up to the step (at most a fifth of a group, a tenth on average) is
   left empty at the group's end. While the children do not reach a half, the cut is the half
   itself: ground is halved, not stretched. A new last child lands in the ground.

What holds, checked by `atlas::tests::prop` (seeded, 2,000 random repositories with eight
changes each, shrunk on failure):

- The same files in any order give the same plan, bit for bit.
- Every place is inside the plan and inside its group; no two leaves overlap; each child's area
  is its weight's share of its group.
- **Anchor rule.** After a file is added, removed or resized, find the deepest folder over it
  that weighs the same as before (the root if none). Every place outside that folder, and the
  folder's own district, is bit for bit where it was.

Measured (same test file, `cargo test -p ottospace-core atlas -- --nocapture`):

| One change | Other leaves that move | Same, with exact (unrounded) group weights |
|---|---|---|
| An edit: a file a tenth larger | none 99.3% of the time; 0.04% on average | 0.7% on average |
| A new file anywhere (about 400 files) | 16% on average | 100% |
| A new last thing that fits its group's ground | none 57% of the time; 3% of its own group on average, always the last few; none outside the group | — |

Compared:

| Layout | When a file is added | Verdict |
|---|---|---|
| Squarified treemap, stable order | Best shapes. Row breaks depend on every sibling's exact weight: one new file rewraps its folder, and the folder's new weight resizes every ancestor's siblings. All of it moves a little | The owner's 3D city uses it; fine for a picture made once, wrong for recall |
| Strip / ordered treemap (exact weights) | Order kept, neighbours stay neighbours. Still proportional: every ancestor's siblings resize. The "exact weights" column above | Half of what is needed |
| City blocks: power-of-two lots, buddy packed | A new lot moves nothing before it, exactly. Needs sizes sorted or leaves gaps: a third to a half of every district empty, compounding with depth | Too sparse; and sorting by size moves a file that grows |
| **Coarse ordered bisection** (chosen) | The ordered treemap, with weights rounded so most changes stop at one folder, and the spare as ground | Above |

Costs, known:

- Free ground: a deep tree is visibly not full (about 0.9 per level). It reads as room to grow.
- A file inserted by name in the middle of a folder shifts the files after it in that folder.
  Sources that only append (issues, threads) do not have this.
- Two leaves of very unlike weight alone in a folder make one a sliver. The shell draws nothing
  under a point wide.
- No memory: a group that crosses a step and comes back moves twice. Kept anchors per id
  (`<home>/atlas/…`, the EvoStreets idea) would fix this and the insert shift; later, if the
  numbers above are not enough in use.

## Drawing

**3D, in perspective, rendered on the GPU** (the owner's word, 2026-11-19: "I was expecting a
3D view … in the background"; and wgpu, so the renderer can follow ottospace to Linux and
Windows). It replaces the 2D plan this note first recommended.

- **The scene is the core's** (`ottospace-core::scene`, pure, unit-tested): the ground; each
  district a plate standing on its parent's, stepped as the owner's city steps its own
  (`TIER_LIFT` 13, 8, 4.6, 2.6, 1.7, then 1.1, in a tower's sixtieths, at three quarters); each
  file a box on its district, its height the owner's `buildingHeight` (`2 + 6·log2(1 + lines/10)`,
  2 to 60), lines guessed as bytes / 32 (lines would mean reading every file). Footprint stays
  the size class: the owner's city sizes a file by its lines too, and churn-based height is
  slice 6. In a city of few files (under about 400) the lots are large, so the tallest building
  is at least 2.5 lot sides and streets a tenth of one: towers on streets, not slabs.
- **Districts read as blocks** (the owner's word, 2026-11-20: "see folders as 'blocks' /
  elevated districts"). Rules, each tested: a folder of more than one child is a district (one
  child: none of its own, as the owner's city; `src/main/java` is one plate), a tier above the
  district it stands on; its plate is a street and a half in from its lot, so two siblings
  never touch; what stands on it is drawn a kerb in from its edge, the plan's shares scaled into
  it, never cut, so a district lies wholly inside its parent's with a margin. The plan is
  unchanged: districts are a drawing of it.
- **The camera is the core's**: perspective, 30° field of view, looking down 52° (38° before the
  districts: buildings hid the streets) from 34° off the plan's front, as near as keeps every
  box inside the picture with 4% to spare, at any shape of tab (tested). A point's projection
  and the ray through a point of the picture (`Scene::hit`: the nearest building or district)
  are pure functions.
- **A lit building is never hidden**: it stands above every terrace, and any building in the
  line from the camera to its top is cut down below that line (tested over many lit buildings,
  one and two at a time). Two lit buildings in line with each other may still overlap.
- **The shell renders it offscreen** (`city_gpu.rs`, wgpu, the platform's own backends): boxes
  instanced, a depth buffer, 4× multisampling, ambient light and one from above (sides darker
  than tops), fog into the background colour, a lit building in its task's colour, unlit by the
  light and the fog. **Edges** (the owner's city outlines its plates and buildings): drawn by the
  box shader itself, no second pass: each face finds its distance to its own edges in device
  pixels (`fwidth`) and draws about two thirds of a pixel of line, so two faces meeting make one or a
  little more, at any zoom
  and scale (tested at 1× and 2× offscreen); a face under about 4 pixels draws none, so a far
  city is not all lines. Four tones: the background, the ground (streets), districts (slate, a
  step lighter per tier, edged in a dim primary that fades with the tier) and buildings (grey,
  edged lighter). The picture is read back once and painted by GPUI as an image, under the
  hints, which get a dark backing that fades out at its edges (in a tab taller than the plan's
  shape, the picture takes only the part below them: the city is as large there); the outer two
  tiers of districts are named on their plates where the name fits, never behind the hints. On
  a thread of its own, made again only when the plan, the lit files or the tab's size in pixels
  change: a frame paints the kept image. No GPU, or a failed render: the lattice, and the state
  says why.
- **Hover: a beam** (the owner's word, 2026-11-20: "a straight-up beam with a dot that shows the
  filename"; the owner's city's PR beams, bright under the avatar and fading toward the ground).
  The user's pointer over a building raises a thin beam straight up from its top's middle, a
  fixed height in the world (the city's highest top, at least `scene::BEAM`, 0.2 of the plan's
  depth: a city of few, tall towers has tall beams; lower where the picture's top is nearer:
  `Scene::beam`, tested), so it stays vertical under any camera; a dot at its
  end, and the file's path beside the dot, cut in the middle if long. Over a district, the
  same from its plate with the folder's path. The hit is the core's ray; GPUI draws the beam
  between the two projected points, faint at the box and brightest at the dot, fading in over
  120 ms (at once under Reduce Motion), so hovering renders nothing again. Over the hints, the
  hints win; a beam is lowered until it clears them, else not drawn. A client's pointer shows
  none. Text colour, never amber: amber is the beacon of "needs you" (below). A lit building
  has a steady, thinner beam in its task's colour, no label.

- **Hierarchy: plinths** (the owner's word, 2026-11-21: "make folders and hierarchy more
  visible"; picked 2026-11-23 from five looks: terraces, plinths, tint, labels, blend). Each
  district is a plinth three times its terrace's lift (up to 4.5 beside a few tall towers), its
  top edge lit, its sides' edges dim (a per-instance factor in the shader: still one draw). A
  plinth's edge is where a folder's name goes, as in the original city.
- **Folders left out** (the owner, 2026-11-23: "node_modules, or an archives folder inside a
  notes checkout"). What the city read before: `git ls-files --cached --others
  --exclude-standard` (tracked and untracked files, `.gitignore` honoured), only in a checkout
  Repos lists (any other folder, a git one or not, has no city), links skipped, no folder
  skipped by name. Now `city_filter` (pure, tested) drops a file under a folder named by a
  built-in list (tools' output, other people's code) or by `city_exclude` (`*` and the
  checkout's own key): a name anywhere, or a path from the root; no globs, so an entry says
  plainly what goes, and nothing can reach outside the checkout. Dropped before the count and
  the layout: what stays is laid out as if the folder never was, and keeps its colours.
  Recommended next (not built): a folder's right-click "Hide from the city" (the user's click
  only) writes the checkout's entry; the place's corner says "2 folders hidden" beside Reset
  view, and its menu lists them, each with Show, so nothing is hidden without a way back.
- **Stable folder colours** (the owner, 2026-11-23: "a mapping, customisable"). A building's
  colour is its nearest folder's with a named colour (built in by what the folder is for: `docs`
  green, `tests` violet, `e2e` pink, `src` blue, `crates`/`packages` teal; `city_folder_colors`
  over them), else its top-level folder's own: a hue from an FNV-1a hash of its name, never its
  index or order, round the wheel less amber's band ("needs you"). Nearest, not top-level only:
  `crates/x/tests` reads as tests, and a monorepo's `packages/*/src` as source. A root file is
  grey; a lit building keeps its task's colour. Muted (lightness 0.34 top, 0.64 edges) so a lit
  building and amber stand out; districts keep their plinth tones. `city_colours`, tested: same
  name same colour in any tree, adding a file or folder changes no other colour.
- **Glass** (the owner's word, 2026-11-21: "a little more transparent"): buildings' faces are 80%
  opaque, their edges whole; a second pipeline blends them after the solid
  boxes, farthest first, writing no depth, so the order is fixed and nothing flickers as a camera
  moves. `developer.city_opaque` draws them solid, for comparing.

Why not paths (the "Oblique" alternative this note first named): GPUI has no depth buffer, so
every building would be three paths painted back to front each frame, 15,000 paths at 5,000
files; a rendered picture costs one image a frame however large the city.

**Level of detail**: none needed for the GPU (50,000 boxes are one draw). A city over 20,000
files, 4 MB of names or 10 s to read is not drawn (the lattice stays).

**Frames**: none at rest. A picture is made when the file list, the lit set or the tab's size
changes, and a paint happens then. A camera move (slice 7) would be the only animation.

**Load** (the owner's word, 2026-11-21: "a progressive load experience"; "improve the
performance"). Measured offscreen, release, on an Apple GPU, made-up trees (`a_city_is_rendered`,
`a_read_is_timed`): the GPU opens once (about 65 ms, on the picture's thread); a render and its read
back take 11 to 15 ms at 1600×1000 and 24 to 31 ms at 3200×2000, up to 20,000 files (half of that
the copy out, row by row now: it was byte by byte, 15 ms at 3200×2000); a file list's read about
57 ms at 30 or 2,000 files, of which most was waiting: the child was looked at every 50 ms, now
from 1 ms doubling. The biggest wait was the loop: reads and pictures were taken every 15th step
(about 250 ms) and a picture needs two hops, so up to half a second of nothing. Now each step
takes them, and the threads that make them wake the loop. The lattice shows at once; then the
first city of a place that waited rises (the owner's word: "why is it not feasible to
animate?"): its districts stand from the start, its buildings grow from the ground over 400 ms,
eased out, the district with most buildings first and the one with fewest a third of the way
in (`Scene::rise_delays`, `scene::risen`, tested). A frame of the rise is a render with the
rise's share as a uniform and each box's start as an instance value: nothing is rebuilt, and the
scene is made once (the picture's thread keeps the last). Frames measured offscreen: release at
most 6 ms at 2,000 files and 8 ms at 20,000; a dev build 6 and 23 ms (load average 18 to 40).
A frame of the rise rendered slower than 12 ms ends it for that city: its standing picture fades
in over 250 ms instead. Reduce Motion: at once. No frames at rest (`counts.city_rise_frames`,
`counts.city_fade_frames`). The newest two pictures no place shows are kept, so a tab shown
again paints its city at once, with no rise. `scripts/bench`: `city_2000_ms`, `city_20000_ms`.

## The camera (built by the user's pointer, slice 7; following navigation proposed)

Built (the owner, 2026-11-20: "rotate, zoom in / out, and move (i.e. hold the scroll wheel and
drag)"): the view is the core's (`scene::View`: yaw, pitch, zoom as a share of the fitted
distance, the point looked at moved over the plan), one per checkout (`Model::city_view`), kept
in the layout file (`layout.json`'s `cameras`, a moved one only) so a restart shows it as left,
moved by `Action::CityCamera` from the user only (a client's is refused), clamped (pitch 12° to
80°, zoom 0.05 to 1, the point looked at over the plan; checked by the property test). The shell
translates: a middle drag pans by the ground under the pointer (`Camera::ground`), the wheel's
lines zoom about it, a left drag past 4 points turns (0.4° a point), a trackpad's scroll pans,
⌥ turns, ⌘ zooms (GPUI 0.2.2 delivers no pinch or rotate). Esc during a drag cancels it to where
it began (`Grab`, `Cancel`); a moved camera shows a quiet "Reset view" button at the place's
corner, which brings back the fitted view, eased over 200 ms (at once under Reduce Motion;
`counts.city_frames` counts the frames). A double click is kept for going into a file. The fitted
view oversees the city (the owner, 2026-11-21: "'overseeing' vs 'looking straight down'"): 38°
down from 34° off its front, was 52°; 30° and 24° hid the back districts behind near towers. A move renders the picture again
at most once a frame, the newest only (the picture's thread takes the newest job; a place's old
key leaves the queue); input never waits for it.
The lit buildings: from a moved view nothing is cut down (they stay raised and coloured): the
user looks where they choose, and nothing changes height while they turn.

Proposed, as before:

The map has a view, a centre and a zoom, apart from the layout, which never changes. A stable
layout with a moving camera is how recall survives zoom: the district is where it was, the
person is only nearer.

- **It moves only because of something the person did**: their navigation or a change of
  focus. As the backdrop of the explorer it frames the folder being browsed, in as they go in
  and out as they leave; choosing a file centres it. In a workspace with one task it frames the
  folders that task has touched.
- **It never chases an agent.** "Follow this agent" may exist, as something the person turns
  on, and turns itself off at their next navigation.
- **The move** is short (about 200 ms, eased), never delays input (the next key retargets it),
  and with Reduce Motion it cuts (`OTTOSPACE_REDUCE_MOTION` stands for it in a test run). The
  target is the core's (`Action` in, a view out); the shell only eases toward it and counts the
  frames it asked for in `counts`.

## Code: the first source (built: `atlas::repo`; the shell in the stacked pull request)

- **Tree**: folders, then files, each by name: as a file explorer lists them, so the map and
  the explorer read alike. The id is the path from the repository's root.
- **Footprint: bytes**, not lines. Bytes come from one `stat` per file with no read; lines need
  every file read. Size classes cap at 128 KB, so one large asset is not the city.
- **Height: churn** (commits that touched the file, from one `git log --name-only`, capped).
  Proposed, not read yet.
- **The file list**: one `git ls-files -z --cached --others --exclude-standard` in the task's
  folder (tracked files and new ones, never an ignored one), then one `stat` each, on a thread of its own. The
  argv is fixed and hardened as the status bar's (`child::git_read`: no hooks, fsmonitor, pager,
  system config or prompt), and runs only in a checkout the person listed in Repos
  (`status_bar::reads_changes`: a repository's config can run programs). Nothing leaves the machine.
- **Limits**: 10 seconds, 4 MB of names, 20,000 files. Past any of them there is no city, and
  the lattice stays: half a city would put files in other places than the whole one.
- **Coupling** (proposed): files changed in the same commits, from the same one `git log`
  (`--name-only`, the last 500 commits, commits over 30 files skipped: a rename sweep couples
  nothing). Cheap, any language, and explainable in one line: "changed together 7 times".
  Imports would be truer and need a parser per language: not now. Shown for one building only,
  the hovered or chosen one: its partners lit faintly and a line to each of the strongest five.
  Never all at once.

### Agents on the map

What ottospace knows today, exactly:

- Each tool call of a conversation arrives as `ToolStart { station, summary, .. }` and
  `ToolEnd` (`ottospace-sessions`, `ottospace-agents`), folded into `Activity`
  (`core::activity`): the calls running now, and the last call, each a station and a label.
- For a `Files` call (`Read`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `Grep`, `Glob`, `LS`,
  `apply_patch`) the label is **the last two parts of the path** (`station::path_label`:
  `/Users/me/app/src/main.rs` → `src/main.rs`), on purpose: the full path never enters the
  model. An edit also carries its diff (`core::diff`), whose paths are not in the state.
- Freshness: a conversation ottospace runs (`Wire`) reports a call as it starts. A watched
  session is read from its file a message at a time: seconds late, and with no times.
- A shell command is `Shell`, never a guessed file. A search (`Grep`, `Glob`) names a folder.

So, built: a building is lit, in its task's identity colour (`Task::colour`, the colour of its
badge and tab), while its path ends with the label of a `Files` call that runs now, or that was
the last call while the agent still has the turn, in the conversation of any task working in
that checkout (two tasks in one checkout: each in its colour; a file in the root is labelled
`<folder>/<name>` and matched so). No
clock is read: the lit set changes only with an event. Two files that share their last two
parts are both lit: honest about what is known, and wrong for one of them (open question 3).

Built (the owner's word, 2026-11-21: "visualize agents as icons floating above shining over what
they are working on"): each agent working now in the city's checkout floats as its task's badge
(its letters in a square bordered in its identity colour) over the file it is at
(`Activity::at`: its newest call's file, else its newest running call on one), at that
building's beam's end, a translucent cone of its colour from the icon down onto the building's
top, its top washed in it. Several on one file fan out side by side; one at no file (a shell
line, between calls) hovers over the buildings, above the camera's middle as high as a beam, with no cone
(`Scene::marks`, tested). A session no task has, at work in the checkout (a `claude` the user
runs in a terminal: the Repos dot's `SessionsAtWork`), floats too, as its runner's glyph in the
colour of work going on, its file lit; its title on hover. Moving to another file, an icon
glides there over 280 ms (at once under Reduce Motion; `counts.city_agent_frames`, none at rest).
Icons and cones are drawn by GPUI over the picture at projected points, as the beam is: an agent
that moves renders nothing again, and a glide needs no render. The user's pointer on an icon
names its task (its title, drawn only) and wins over the building under it; the user's own
click goes to the task, as its row does (`Model::city_agent_click`: a client's click, or a
session's, does nothing: which pane a session runs in is not known until a terminal's agent is
matched to its session). An icon is lowered along its beam until clear of the hints, else not
drawn.

Proposed next: a marker (the task's badge, its identity colour) on the node the agent touched
last, with the last few as a fading trail (drawn at fixed strengths, no animation); two agents
in one building split it. An agent running a command, or between calls for long, is shown at
the district of its task's folder, or as a badge on the map's edge, "off the map": never at a
guessed file. For exact places the label must become an id: carry the path relative to the
task's folder beside the label (shell-side only, never in the state). Open question 3.

### The three uses

1. **Backdrop of every empty place** (built, behind the flag `code_city`; the owner, 2026-11-20:
   "why not the other empty experiences?"): an empty tab or pane draws the city of its folder
   where the lattice was, under the same key caps (`Model::city_at`, `Model::empty_folder`). The
   folder is the nearest: a pane's own (none keeps one), its tab's (opened in a checkout,
   `open_repo`; kept after its last pane closes), its space's (a task's checkout, a workspace's
   default). Lit: in a task's space that task's files; elsewhere every task working in the
   folder (`task_worktree`, as the Repos dots). A pane under 320 × 220 points keeps the lattice
   (a district is a few pixels there: chosen from offscreen pictures at 240, 320, 400 and 480
   wide). Each folder on show is read once; each picture (folder, lit files, size) is rendered
   once and painted by every place that asks for it. No folder, no listed checkout, too large,
   or not read yet: the lattice. Nothing in it takes a click or the keyboard.
2. **Beside the explorer** (proposed). The explorer is not built: `docs/ai/file-viewer.md`
   lists it as step D, "a tree pane of a folder". With it: the map is the explorer's backdrop
   or a pane beside it; the row under the cursor is the lit building; a click on a building
   moves the explorer there (the user's click only); the camera follows as above.
3. **Inside a file** (proposed). A file's own plan: its symbols as lots in reading order, top
   to bottom, footprint by lines (the owner's city found a treemap wrong here: "a function is
   not a district"). Needs an outline per language; the viewer's markdown headings are the
   cheap first case. The viewer's cursor line is the lit lot; `gg`/`G` and a lot's click move it.
   Inside a file, a symbol is coloured by its kind (function, class, interface, type, enum,
   const, a markdown heading), as the original city's `KIND_COLORS` did (the owner, 2026-11-23).

## Other sources (design only; none is built, and none adds a network path by this note)

The shell's network use today is two things (`CLAUDE.md`): the calendar feed and `gh` for
listed repositories. Each source below would be a third: an open question, not a plan.

### Jira

| | |
|---|---|
| Tree | project → epic → issue (issues with no epic: one group at the end) |
| Order | by issue number: new issues come last, into the group's ground |
| Footprint | the estimate in coarse classes (none: 1). Set once, seldom changed |
| Height | age without an update, or comments this month |
| Links | blocks / is blocked by, relates to |
| Activity | a task whose text names the issue key: its agent's colour on the issue |
| Beacon | assigned to you and waiting; blocked on you |
| Closed | stays in place, faded; dropped only when its whole epic is closed and old |
| From | no path today. Candidates: a `jira` CLI the person has signed in, read-only and a fixed argv, as `gh` is; or an agent writes a snapshot file ottospace reads. No token in ottospace |
| Private | issue titles, people, keys: drawn, never in the state, the event log or a task's brief. As meeting titles |

### Slack

| | |
|---|---|
| Tree | workspace → section (the person's own sidebar sections) → channel. Threads are not nodes: they churn too fast for a place |
| Order | by when the person joined: new channels last |
| Footprint | **members**, in classes (1-5, 6-20, 21-100, 100+): slow |
| Height | **messages in the past month**: fast, so drawn only |
| Links | people in common between two channels, for the hovered one |
| Beacon | you were mentioned there and have not read it |
| Archived | stays, faded |
| From | the owner reads Slack through a read-only CLI of their own. ottospace has no Slack path, and member counts and mentions are the person's private data. Open question 5 |
| Private | channel and people names: drawn only |

### Tasks

Home's lattice already places tasks (`core::home`: a lane per project, older is farther). On
the atlas: project → task, order by id, footprint 1, beacon = needs you. Then Home is one more
map, and a task's place is stable, which the lattice's is not (it re-ranks by age).

## The beacon (proposed)

A mark on a node that something there is addressed to the person: a mention in that channel; an
issue waiting on them; for code, the task working in that folder asks a question.

- It is the map's view of what ottospace already knows needs the person: the same state as
  the Tasks rail's "Needs you" and ⌘J. Not a second notification system: it has no sound, no
  badge of its own, no count.
- A steady light, not a pulse. It clears when the person has seen the thing (the task opened,
  the channel read).
- During a focus block beacons are held as any notice is, unless a break-through rule matches
  (`docs/ai/attention/plan.md`).
- For code it needs nothing new: `Pulse::NeedsYou` of the task's conversation, drawn on the
  district of the task's folder.

## Privacy and trust

- Paths and names are the person's. They are drawn, and kept in the shell's memory only. Not
  in the control socket's state, the event log, or anything written to disk.
- The state gains, under the test gate only (`OTTOSPACE_E2E`): `city`, with counts (places,
  groups, reads, renders, paints, names drawn), the picture's size, where it and the lit
  buildings are, the last render's time and why there is none (`failed`); `counts.city_reads`.
  No id, label or path. `OTTOSPACE_CITY_NO_GPU` (read only under `OTTOSPACE_E2E`) stands for a
  machine with no GPU.
- A source's text is data, never instructions: a file named like a command is a label.
- git is run with a fixed argv, in the task's folder, and never a hook, a pager or the network.

## Slices

| # | Slice | State |
|---|---|---|
| 1 | The layout and the repository source, in the core, with the property test | built (this pull request) |
| 2 | The backdrop behind `code_city`: the file list, the city in 3D rendered on the GPU (by the owner's word, 2026-11-19), lit buildings, a shot scenario, an e2e test | built (stacked) |
| 2b | The look (the owner's word, 2026-11-20): edges, districts as plates, four tones; hover: a building's name on a beacon | built |
| 2c | Every empty place with a folder draws its city: tabs and panes, nearest folder, shared pictures | built |
| 3 | The exact file of a call, shell-side, so a lit building is never a guess (question 3) | |
| 4 | Agent markers and trails; the task's "needs you" as a beacon on its district | |
| 5 | Coupling for the hovered building (one `git log`); hover itself is 2b | |
| 6 | Height from churn | |
| 7 | The camera in the core (a view from an `Action`), eased in the shell, Reduce Motion | built: the user's pointer (2026-11-20); following navigation proposed |
| 8 | The explorer pane (file-viewer.md, D), then the map beside it and the camera following it | needs D |
| 9 | Inside a file: markdown headings first | |
| 10 | Kept anchors, if drift is felt | |
| 11 | A second source (Tasks is the cheapest: no new data) | |

## Open questions for the owner

1. **Where is the backdrop?** Built: every empty tab and pane with a folder (2026-11-20). A
   workspace with panes shows it only in its empty panes. Should the map
   also be a pane of its own, or show through the gaps, or replace Home's lattice (question 6)?
2. **Bytes or lines** for the footprint? Bytes are free; lines read every file.
3. **Exact paths.** Lighting by the last two path parts can light two files of one name
   (`a/mod.rs`, `b/mod.rs` do not clash; `x/a/mod.rs` and `y/a/mod.rs` do). May the shell keep
   a call's path relative to the task's folder, in memory, never in the state?
4. **Free ground**: is a city a tenth empty per level fine, or should ground be hidden
   (children stretched over it: the same stability outside a folder, less inside)?
5. **Jira and Slack data**: which path, if any? A signed-in CLI with a fixed argv (as `gh`), a
   snapshot file an agent writes, or not in ottospace at all.
6. **Home**: should Home's lattice become the Tasks map, or stay?
7. **Oblique view**: answered 2026-11-19: 3D, in perspective, on the GPU (Drawing).
