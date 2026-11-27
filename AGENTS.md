# ottospace

macOS agent multiplexer: terminals and real app windows tiled in one workspace. Rust + GPUI.

## Architecture: functional core, imperative shell

| Crate | Role | May use |
|---|---|---|
| `ottospace-core` | State and rules. `Model::update(Envelope { action, origin }) -> Vec<Effect>` | std, serde, unicode-segmentation. No I/O, UI, clock, threads |
| `ottospace-mac` | macOS calls: AX, ScreenCaptureKit, displays, input | system APIs |
| `ottospace-sessions` | Reads Claude Code and Codex session files: sessions of a directory, events, snapshot. A `SessionReader` per `RunnerId` | std, serde, read-only files, `ottospace-core` (pure: for `runner::RunnerId`, `activity::Station`, `sessions::WaitsFor`). No network, clock, writes |
| `ottospace-agents` | Speaks to agent CLIs in their structured modes: starts, sends, reads events. A `Wire` per `RunnerId` | std, serde_json, the two crates above; spawns child processes. No network of its own, clock decisions, or reading other apps |
| `ottospace` | Shell: GPUI views, control socket. Turns input into `Action`s, carries out `Effect`s | everything; the network only to read the calendar feed (`calendar_ics`), through `/usr/bin/curl`, `https:` only, and the open pull requests of listed GitHub repositories, through the user's `gh` (`repos_prs`; `prs.rs`), read-only, a fixed argv; and, started only by the user, listing the user's repositories and cloning one the user chose, through the user's `gh` (`clone.rs`), each a fixed argv |

Rules:

- **A runner is its spec.** What differs between Claude Code and Codex is a field of its
  `RunnerSpec` (`ottospace-core::runner`), asked by capability; never a `match` on its name.
- **Decisions go in the core.** If code chooses what happens next (which pane gets focus, where a
  split goes, what closes), it belongs in `ottospace-core` with a unit test.
- **The shell only translates.** Input → `Action`; `Effect` → system call or draw. All state
  changes go through `Workspace::dispatch`. No second copy of model state in the shell.
- **New behaviour starts as a core test.** Add the `Action`/`Effect`, test it, then wire the shell.
- **Core stays pure.** If the core needs time, randomness or system data, pass it in the `Action`.
- **Every action says who sent it.** `Workspace::dispatch(action, origin, ..)`: `Origin::User` (the
  person's keys, clicks and menu items: pass `self.input`), `Socket` (a control-socket request, also
  a key or click it injects) or `System` (timers, a shell that exited, lists read again). A rule
  about what may interrupt the user asks the origin, not the action.

## Principles

Goals, which the rules below serve:

- **A command center for your agents.** You are the captain: ottospace makes it easy to see what
  the agents are doing and to digest the decisions that are yours.
- **Flow and joy.** The loop is now delegate, wait, decide, verify: ottospace gives quick wins,
  shows progress, and makes the right things satisfying to do.

Design rules for every feature. Each change answers to all four.

### Human attention

> Human attention is the scarcest resource. ottospace interrupts only for what needs the person,
> brings the context with the question, batches the rest, and never makes the person hunt for
> what's next.

- A new interruption (notice, notification, badge, sound, focus change) says why it needs the
  person now. If it can wait, it does not interrupt.
- A question to the person carries its context and a recommendation: the answer must not need a
  hunt through the task.
- What can wait is batched: one digest, one queue, never one ping each.
- The person never searches for what needs them: one key and one place say what's next (⌘J,
  the status bar's `needs you`). A new kind of waiting joins them, never a place of its own.

### Out of the way (wording proposed)

> ottospace stays out of your way. Nothing moves, covers what you are working in, or takes the
> keyboard unless you asked for it; every mode has an obvious way out; and nothing decorative ever
> makes the fast path slower.

- Layout never shifts for a message: the notice bar moved every pane, so notices become a toast
  over them (#299, open).
- Nothing rearranges itself: a new app window never splits a pane by itself; its pane offers it
  (#266).
- Explicit over magical: the person says where a window goes (slots: only the user makes, fills
  or clears one; putting a window in a pane is the user's act, and that pane stays, a slot, when
  the window closes, `docs/ai/slots.md`).
- Animation never delays input: the next thing is usable while it plays (Done's seal
  plays over a model that has already moved on, #278); under Reduce Motion it is at once.
- A test run or an agent never takes the person's keyboard or front window (see Tests).

### Joy follows the user (wording proposed)

> Delight attaches only to something you just did or asked to see. It never interrupts, never
> delays the next key, and never decorates the fast path.

- What ottospace presents is a pleasure to take in: never a wall of text with decisions buried
  in it. Each decision is one clear thing to act on, and acting on it leaves the person feeling
  in control, in command, and that things are getting done.
- A reward marks an outcome (a decision made, a stack landed, main green), never activity.
- It plays over a model that has already moved on; under Reduce Motion it is at once.
- Nothing celebrates by itself: no animation, sound or badge arrives uninvited.

### Wherever the conversation happens (wording proposed)

> No matter where the conversation happens, ottospace can help.

- A conversation started anywhere (a `claude` or `codex` in a terminal of the person's own) gets
  the same help as one started in ottospace: its transcript, its place in the city, its
  decisions, its turn in needs-you. Read-only where it must be, and saying so.
- Correct detection, live updates and counts come before any switching feature.
- One writer at a time: the switch between a terminal and ottospace is offered only when the
  other side is idle or ended; ottospace never kills or signals the person's process.

Design note: `docs/ai/sessions-seamless.md`.

When they conflict (wording proposed): ottospace offers and waits. It may show that something
needs you; it never acts on your screen, focus or keyboard to make you look.

Design note: `docs/ai/attention/plan.md`.

## Tests

- **Inner loop:** `cargo test`. Core unit tests. Fast, no window.
- **Property tests**, part of `cargo test`: seeded random sequences, with what must hold after every
  step. The model's (`crates/ottospace-core/src/model/tests/prop.rs`) sends every `Action` from every
  `Origin`; `layout.rs`, `compose.rs` and `edit.rs` have their own. A new `Action` needs a maker in
  `MAKERS` (a test fails until it has one), and a new rule about the model belongs in `check`. A
  failure prints its seed and the least steps that still fail, as a test: paste it under
  "Regressions" and fix the bug. `OTTOSPACE_PROP_SEED=<n>` runs one sequence again, and
  `OTTOSPACE_PROP_WORKERS=<n>` caps each property test's threads (every core by default);
  `cargo test -p ottospace-core --release -- --ignored random_actions` is the long run.
- **Outer loop:** `cargo e2e`. Launches the real app and drives it over the control socket
  (`crates/ottospace/tests/e2e.rs`). Use it for anything the core cannot prove: drawing, layout
  bounds, key bindings, socket protocol.
- In an E2E run a reply with the state waits for the frame that draws its request: read bounds
  from it, no poll needed. A failed test writes a folder that explains it (its panic names it),
  and `scripts/e2e-repeat N [filter]` tallies flaky tests. See `docs/ai/e2e-reliability.md`.
- `OTTOSPACE_E2E_WINDOW=1024x674` (read only under `OTTOSPACE_E2E`) sizes a test run's window:
  CI's screen is 1024 wide, so there the default 1400x900 is 1024x674. macOS keeps it no larger
  than the screen.
- `scripts/e2e-for [<base>]` lists the e2e tests a change can affect, each with why: tests it
  changes in `e2e.rs` (and those using a helper it changes), the areas of its paths
  (`scripts/e2e-for.areas`), a smoke set. A path the table does not know prints the full suite
  instead. `--run` runs them under nextest, `--ci-size` at CI's window. It refuses a locked
  screen or another e2e run, and never runs `cargo e2e-apps`. `scripts/test-e2e-for.sh` tests it;
  `scripts/e2e-for --check` fails on a source file the table does not know.
- **A pull request merged on local evidence** first runs `scripts/e2e-for --run --ci-size` and
  pastes its summary in the pull request.
- A test leaves nothing in `$TMPDIR`, pass or fail. An e2e test makes its paths with
  `support::temp`, removed when it ends; a unit test holds a guard that removes its folder on
  drop. `a_test_leaves_nothing_in_the_temp_dir` checks it.
- An E2E run reads no `settings.json` and keeps no tasks on disk, unless the test names its own
  folder with `OTTOSPACE_HOME`. Tests must never read or write the user's `~/.ottospace`.
  The event log (`event_log.rs`, `<home>/log`) follows the same rule, and a test run writes none
  of it to the unified log.
- E2E runs set `OTTOSPACE_E2E`, which turns window hosting off. Tests must never move the
  user's real windows.
- A `cargo e2e` run stays out of the user's way: the app is a background app (no Dock icon,
  it does not come to the front or take the keyboard by itself), and its window sits at a free
  corner of the screens with a sliver showing (about 12 points wide, as tall as the title bar).
  The sliver floats above other windows on every Space and lets clicks through. It is there
  because GPUI does not draw a window that nobody can see, and a test waiting for layout would
  time out. A layout timeout prints the last state. To look at the window, set
  `OTTOSPACE_E2E_SHOW`: it opens in the middle of the screen and takes clicks, still without
  taking the keyboard by itself.
- **App-window loop:** `cargo e2e-apps`. Hosts real windows of `ottospace-canary`, a test app
  built from this package (`src/bin`). `OTTOSPACE_HOST_ONLY=<pid>` limits hosting (and `ottospace rescue`) to that process. A test
  run keeps the record of hosted windows (`hosted.json`) only in a folder it names with
  `OTTOSPACE_HOME`, and starts the guardian only with `OTTOSPACE_GUARD` set.
  `OTTOSPACE_E2E_FINDER` lets a test host a Finder window it opens (CI only: there Finder has no
  windows of anyone's); without it, and while Finder has any window open, the test hosts none.
  The run takes keyboard focus while it lasts. Needs
  Accessibility and Screen Recording for the terminal. Use it for anything that moves, parks,
  adopts or restores a real window.
- `OTTOSPACE_APPS_DIRS` (folders, `:` between them) replaces the folders scanned for installed apps.
  An E2E run that names none lists none, and never starts an app. `OTTOSPACE_DOCK_APPS` (bundle
  paths, `:` between them) stands for the Dock's apps; a test run that names none has none, and
  never reads the user's Dock.
- `OTTOSPACE_CLIPBOARD_IMAGE` (a made-up image file) is what the test run's stand-in clipboard
  holds as an image (⌘V attaches it), read only under `OTTOSPACE_E2E`; a test run never reads the
  user's clipboard. Pasted images are kept in `<home>/images` (a test names `OTTOSPACE_HOME`).
- `OTTOSPACE_LISTEN_APPS` (a JSON file of made-up apps) stands for the apps the transcript's picker
  lists, read only under `OTTOSPACE_E2E`; a test run that names none lists none, never the user's.
- `OTTOSPACE_GH` (a program) stands for `gh` in a test run: a stand-in script with fixture JSON. A test run
  that names none runs no gh, and never the user's. The state's `counts.pr_opens` (test runs only) counts
  the pull requests the user's click opened; a test run opens none. `counts.link_opens` (test runs only) counts the
  terminal links the user's ⌘-click opened (`Effect::OpenWeb`); a test run opens none, and no URL is in the state.
- Feature flags (`ottospace-core/src/features.rs`) are off in an E2E run, as for a new user. A test
  of a feature behind one names it: `OTTOSPACE_FEATURES=marks,apps_section` (read only under
  `OTTOSPACE_E2E`; it wins over `settings.json`), `MARKS`, `APPS_SECTION` or `BOTH` in `e2e.rs`. In
  the core's unit tests every flag is on (`Model::default`); a test of one off sends
  `SetFeatures`. A new action names its feature in `Action::feature`.
- Requests and fields that answer only under `OTTOSPACE_E2E`, by one gate (`control::test_refusal`).
  They are documented here and on `Request` in `control.rs`, never in `skill/guide.md`: an agent
  must not learn them.
  - `mouse` (`down`, `move`, `up`, `wheel`, `exit`, `enter`) and `drag` hand events to the window's
    own AppKit handlers, never posted. `wheel` takes `dx`, `dy`: lines, or with `precise` points,
    both rounded to whole numbers. `"button":"right"` makes a `down` or `up` the right button's
    (a right click: a Repos worktree row's menu), never a drag. `"button":"middle"` makes a `down` or
    `up` the middle button's (a mouse's wheel, pressed), and every `move` between them its drag (the
    code city's pan).
  - `"as":"user"` on `key`, `mouse` and `drag`: the event is `Origin::User`, for what a socket
    client is refused (sneak mode, a mark's wait, the quit question, a pin in the Apps section).
    Every key and pointer event a client sends is otherwise the client's. Zoom-out (`zoom_out`)
    is the user's alone: a test zooms with `cmd-|` and clicks a tile (`zoom_tiles`) this way.
    Outside a test run a client that passed for the user could quit over running programs, end the
    mode the user is typing in, or write their settings. A test run hosts no window and is the test's own
    instance, on its own socket.
  - `repo_add` (`"path"`): the folder the user's picker would give the Repos section's +, as the
    user's own act. Outside a test run only the user adds a directory to their settings, and a
    test run that names no `OTTOSPACE_HOME` writes none.
  - `capture_start`: the user's capture key, pressed by the test (`"pane"` optional). Outside a
    test run only the user starts a capture; the core also refuses a `CaptureStart` of any other
    origin, so a client's key, click or sneak label starts none in a test run either.
  - `bug_start`: the user's bug key, pressed by the test, while `developer.ottospace_repo` is a
    checkout. Outside a test run only the user starts a bug report (it pictures the window); the
    core refuses a `BugStart` of any other origin. The state's `bug.controls` (where the overlay's
    controls are drawn, by id, while a report is open) is null outside a test run.
  - `city_spin`: the Developer command "Spin the City and Time Its Frames", as the user's: the city
    on show turns once round (`CameraStep::Spin`, a step a frame), its camera then back; the numbers
    in the state's `city.spin` (`scripts/bench`'s spin). Outside a test run only the user moves the
    city's camera; the core refuses a `CityCamera` of any other origin.
  - `typing_time` (`"full"`, `"busy"`, both optional): the Developer command "Time Typing in This
    Terminal", as the user's: made-up keys into a throwaway `cat` in a new pane (`full`: the made-up
    full-screen `ottospace typing-echo`; `busy`: a pane printing without end beside it), each timed
    from its key (written to that terminal as its key handler writes one) to the frame that paints
    its echo, by stage; the panes then closed. The
    numbers in the state's `typing` (`running`, `result`: `keys`, `missed`, and per stage `n`,
    `p50`, `p95`, `p99`, `max` in ms: `key_write`, `write_echo`, `echo_taken`, `taken_painted`,
    `app` (the app's own share), `total`; `hz` the display's refresh). Null outside a test run.
    `scripts/bench`'s typing. Outside a test run only the user's command types into the window.
  - `city_under` (`"pane"`, `"opacity"`): with `city_live_layer`, that terminal pane's folder's city
    drawn on its live layer under it, the pane's background at `opacity` over it (the
    `city-under` shot: text over the city); `"pane": null` ends it. Outside a test run the city
    behind panes is not built.
  - `sneak_app_tree`: a fixture tree as if read from the focused pane's window while sneak mode
    is on (a test run hosts no window), so its elements get labels. The state's `app_presses`
    (null outside a test run) lists each press a label asked for.
  - State field `goto_rows`: go-to's rows (⌘P) while it lists places or files, each place's
    `place` (`workspace`, `checkout`, `task`), `name` and `detail`, or each `file` (a path in the
    place's folder); `palette_rows` is null meanwhile. Null outside a test run: a file's path is in
    no other state.
  - `compose_drop` with `"pane"`: files dropped on that pane as the user's own drop (`Action::PaneDrop`:
    a terminal types the quoted paths). Outside a test run only the user drops files; `type` is a
    client's way to type.
  - State field `clip_lights`: what a copy or paste lights while its notice shows (the pulse is its first
    600 ms): the copied `cells` (row, first column, last column) or the pane's border (`ring`). `counts.clip_frames` counts the frames its pulse asked for. Null outside a test run.
  - State field `overflow`: each scroll area laid out (not a terminal), also one that fits, and which of its edges
    fade (`top`/`bottom`, the tab row's `left`/`right`). Null outside a test run.
  - State field `terminals[n].styled`: each span on show drawn with a style (`row`, `text`, `fg` with
    dim already in it, `bg`, `bold`, `italic`, `strike`, `underline` `{curly, color}`). Null outside a test run.
  - State field `counts.cell_art`: terminal cells drawn as shapes (box drawing, blocks), each frame's
    added up. Null outside a test run, as all of `counts`.
  - State field `clone`: the clone dialog (From GitHub… of the Repos section's +) while open, its `rows`
    (`owner/name`), `selected`, `destination` (in full) and `said`; and the clone on its way (`repo`,
    `dest`, gh's last `line`, `cancelled`). `counts.folder_picks` counts Local folder…'s folder pickers
    (a test run shows none). Null outside a test run: a repository's name is the user's data (the
    dialog's line is a hidden field, `field_hidden`; a clone's toasts name no repository). Tests
    drive the dialog with the user's keys and clicks, `clone_dir` from their own `OTTOSPACE_HOME`,
    `OTTOSPACE_GH` a stand-in that makes a local repository; no request clones.
  - State field `repos_bar[n].name_cut`: a repository's name as drawn in its heading (`cut_text`), its box
    (`room`) and its shaped width (`drawn`). Null outside a test run.
  - State field `compose_rects`: where the open new-task box's controls were drawn, by id
    (`compose-attach`, `compose-sheet`, `compose-edge` its bottom row, `compose-start`, `picker:#`,
    `picker:>`, `picker:@`, `picker-list` the open picker's list, `compose-pen` its ✎): sneak mode labels none while its
    line has the keyboard.
    Null outside a test run and while the box is closed.
  - State field `offer_rects`: where an offer chip's words (`words:<pane>`) and their tooltip (`tip`,
    while it shows) were drawn; never the words. Null outside a test run.
  - State field `viewers[n].marked_rect` (drawn): where a viewer's first marked line is drawn, while
    its pane shows. Null outside a test run.
  - State field `city`: the code city of the first empty place on show that draws one (`code_city`):
    `on` (`"tab"`, `{"pane": id}` or `{"out": tab}`), `pictures` (kept, one per folder, lit files and
    size), `small` (the places too small for a city, drawing their lattice), `places`, `groups`,
    `picture` (its size in pixels), `bounds` and `scale` (where it was painted), `lit` (each lit
    building's rect), `lit_tops` (the middle of each one's top), `labels`, `failed` (why there is
    no picture), `reads`, `renders`, `paints`, `render_us`, `hovered` (the beam of the box under
    the user's pointer: `file`, `lit`, a file's `beam` its foot and dot `[x0, y0, x1, y1]`, a folder's `outline`
    (its top face's four corners), `beacon` its label's rect; null with none), `names` (each name drawn by
    `city_labels`: its `tier`, `wall`, `folder` or `file`, and its `rect`; never the name; with
    `city_wall_names` also each one the renderer drew on a plinth, `plinth`, its em box's `rect` and
    `hidden`, a standing box in front of its middle: solid hides it, glass dims it), `spots` (a point
    over an outer district's own ground, `folder`, and over a building, `file`, for a test's pointer), `lit_beams` (each lit building's beam drawn, as `beam`), `glass` (its buildings see-through:
    `developer.city_opaque` off), `lower` (how far below the picture's middle the fitted city's is,
    a share of its height: `developer.city_frame`), `folder_colors` (how many `city_folder_colors` the
    picture's colours had), `first_ms` (the place laid out to its first picture painted),
    `read_us` (the last file list's read), `agents` (each agent's icon drawn: `task` (null: a session no
    task has), `rect`, `cone` (it shines on a building), `fan` `[k, n]`, `hovered`, `pulsing` (a call of it runs), `subagents` (the sub-agents it runs), `station` (its call's station, `shell`; its glyph drawn at its corner where it fits), `caption` (where its call's line, the sidebar's, was drawn; never the words)), `glimmer` (the reads' flashes, searches' sweeps and
    found files' lights playing now: `flash`, `sweep`, `found`, counts), `behind` (drawn behind its
    tab's panes, `city_behind`: each pane that lets it show, `pane` and `opacity`; null in front), `spin` (`running`, and the last
    spin's `result`: `steps`, `done`, `frames`, each frame's time from the one before as `mean`, `p50`,
    `p95`, `p99`, `max`, `over_16`, `over_33` (ms), `repeats` (frames with no new picture since the
    frame before: no new image painted, or with `city_live_layer` no present on the live layer), a
    new picture from made to painted `wait_p50`, `wait_p95`, `wait_max` (0 on the live layer), a step's
    picture from its place asking to it shown (painted, or presented) `shown_p50`, `shown_p95`,
    `shown_p99`, each picture's render `render_p50`, `render_p95`, `live`, the pictures made for its place `made`, its place's paints `paints`, the new
    pictures seen there (painted, or presented) `new`, `boxes`). `counts.city_reads`: the reads, a city on show or not;
    `counts.city_beam_frames`: the frames the hovered beam's fade asked for (none under Reduce Motion);
    `counts.city_fade_frames`: the frames a first picture's fade-in over the lattice asked for, and
    `counts.city_rise_frames` those its rise painted (none at rest, none under Reduce Motion);
    `counts.city_agent_frames`: the frames an icon gliding to another file asked for (none at rest
    or under Reduce Motion); `counts.city_pulse_frames` those a busy agent's pulse asked for (none
    once its calls end, none under Reduce Motion); `counts.city_glimmer_frames` those the reads' and
    searches' effects asked for (none once they end, none under Reduce Motion). `view` (its camera: `yaw`, `pitch`, `zoom`, `pan`), `grabbed` (a drag of a
    city's camera is in flight); `counts.city_frames`: frames a camera's reset asked for while easing
    back (none at rest, none under Reduce Motion). Never a path or a name; a task's title is drawn, never in the state.
    Null outside a test run and while none is on show. `OTTOSPACE_CITY_NO_GPU` (read only under
    `OTTOSPACE_E2E`) stands for a machine with no GPU: every picture fails, the lattice stays.
  - State fields `keys_known` (the ids of the key bindings the user knows) and `key_reference`
    (while ⌘/ shows: its groups' row ids, the known ids, whether Known is open, its header's
    rect). Null outside a test run: what the user's keys are used for is never on the socket.
  - `window_fixture` (`"pane"`, `"size"`, optional `"title"`, `"subrole"`, `"modal"`): a new window of
    that app pane's app, as if just seen (a test run hosts none), judged, logged and offered as a real
    one (to a slot that selects it first); its id (800001 on) is in `offers`. Put in a pane on it opens a pane with no window.
    `window_fixture_close` (`"window"`): it closed (also an `app_fixture` pane's own, as its app runs on). Outside a test run only ottospace sees an app's windows.
    `window_fixture` also takes `"identifier"` (its `AXIdentifier`).
  - `slot_windows` (`"look"`: an app's windows as a slot's look reads them) and `slot_make` (`"pane"`,
    `"slot"`: the slot dialog's Keep, as the user's act), for slots (`slots` flag) in a test run, which
    hosts no window. A slot's Bring it here then fills with a pane of no window.
  - `record_start` (`"dir"`) and `record_stop`: ScreenCaptureKit films ottospace's own window
    into `dir`, a raw BGRA file a change and `frames.jsonl` (`record.rs`), for
    `scripts/shot --record`. Outside a test run no client films the window.
  - `audio_probe_start` and `audio_probe_stop`: ScreenCaptureKit hears the app
    `OTTOSPACE_HOST_ONLY` names (the canary), with no recogniser; the stop's reply lists each
    buffer's `pts_ms`, `samples` and `rms`, to see whether silence comes as buffers. Outside a test
    run no client hears an app.
  - `pinch` (`"x"`, `"y"`, `"magnification"`, `"as"`): a trackpad pinch as the native hook
    (`ottospace_mac::magnify`) hands it over: the zoomed-out canvas or a code city zooms about it.
    Outside a test run only the user's trackpad pinches; a real pinch needs a trackpad.
  - `zoom_press` (`"tab"` optional: a tab out's window): the window's green button, pressed as the
    user would (`performZoom:`). The zoom fills that window's display instead
    (`ottospace_mac::window::zoom_asks`); refused if that hook is not in, so it never zooms the
    test's window. Outside a test run only the user's hand fills the display.
  - `modifier_flags` (`"changes": [{"at", "cmd", "others", "input_at"}]`): modifier changes as ⌘⌘'s
    listener would hear them in another app (`double_cmd`; a test run listens to no keyboard); a
    double tap returns as the return key, without bringing the window forward. The state's
    `counts.double_cmds` counts them. Outside a test run only the user's own ⌘⌘ returns.
  - `display_change`: the displays changed, as macOS's reconfiguration callback says (a test
    cannot unplug one). The shell then kicks GPUI to draw again, as after a real change
    (`stall.rs`, `ottospace_mac::window::redraw`); the state's `counts.redraws` counts the kicks.
    Outside a test run only macOS says the displays changed.
  - `ui_gallery` (`"on"`, `"page"`): every component of `ui/` in every state over the window
    (`docs/ai/ui.md`), for `scripts/shot ui-gallery`; `"page":"dialogs"` or `"examples"`: the
    dialogs' parts, or a question and a form whole (`ui-dialogs`). Outside a test run no client covers the
    user's workspace.
  - `displays_set` (`"displays": [{"uid", "frame": [x, y, w, h], "scale"}]`, `"main"`: a uid, default
    the first): made-up displays in place of the real ones from then on, sent to the core as the
    display watcher would (`Action::SetDisplays`). Leaving one out unplugs it, putting it back
    plugs it in again (`model/displays.rs`). Outside a test run only ottospace reads the displays.
  - `"repeat":true` on `key`: each key as one still held down. A key that types is a real repeat
    to `keyDown:`: `enter`, `tab`, `space`, or a character the layout types with no modifier on
    key codes 0 to 50 (`a`, `7`, `/`; not `A`). A `cmd-` shortcut, by the flag alone, is pressed as
    without `repeat` while `key_repeating()` says held. `escape`, `backspace` and arrows answer
    "no key to hold down".
- `OTTOSPACE_LISTEN_FIXTURE=<file.jsonl>` (read only under `OTTOSPACE_E2E`) is a transcript's sound:
  timed lines (`{"after_ms":300,"from":"them","text":"made-up words","final":true}`) replayed as
  what was heard, and a terminal pane may then be a source (a test run hosts no app). With none, a
  test run's start fails with "no audio source in a test run": never SCK, the mic or real audio.
  `listen_end` (`"why"`: `app_quit`, `stream_failed`) ends the one on as the shell would; the state's
  `listen_lines` (the drawer's rows) and `listen_rects` (where its controls are drawn) are null
  outside a test run. See `docs/ai/transcripts.md`.
- Transcripts in a test run: the sound is `OTTOSPACE_LISTEN_FIXTURE` (made-up lines and levels;
  read only under `OTTOSPACE_E2E`), never SCK or the mic. The one exception is
  `OTTOSPACE_HOST_ONLY=<pid>`: then only that app (the canary, `{"cmd":"play"}` on its socket) is
  heard by SCK and the real recogniser. A test run never downloads the speech model and shows no
  menu-bar dot. The meter and its label are draw-only: never in the state.
- `app_fixture` (`"name"`, `"user"`): an app pane with no window (a test run hosts none), as the
  user's pick (`user`) or a client's `open_app`, for agent mode's robot and its grants. A read of it,
  once granted, finds no window. In an app-window run (`OTTOSPACE_HOST_ONLY`, no user to grant a
  level) `ax_tree` reads the canary's pane as before. See `docs/ai/agent-mode.md`.
- `OTTOSPACE_REDUCE_MOTION` (read only under `OTTOSPACE_E2E`) stands for the system's Reduce Motion
  in Done's seal. `counts.seal_frames` and `counts.seal_spark_frames` count the frames Done's seal,
  fold and land (`seal.rs`), and `counts.repo_pulse_frames` for a Repos
  dot's commit pulse; `counts.repo_pulses` counts the pulses (none under Reduce Motion);
  `counts.overlay_frames` for a side overlay's slide out of its rail and back (`overlays.rs`);
  `counts.zoom_frames` for zoom-out's zoom both ways (`zoom.rs`);
  `counts.shimmer_frames` for a working line's sweep (`ui::shimmer`): none at rest;
  `counts.space_pulse_frames` for a working space's mark breathing on the top row (none while none
  works).
- A feature is done when both loops cover it. Extend `Request` and the state snapshot when a test
  needs to drive or read something new.

## CI

- A pull request takes no macOS runner by default (an installer change still runs `scripts.yml` on
  one): they are scarce. `.github/workflows/ci.yml`'s
  `check`, on pull requests, `merge_group` and pushes to `main`, on `ubuntu-latest`: `cargo fmt -p
  ottospace -p ottospace-core -p ottospace-sessions -p ottospace-agents --check`, `cargo clippy --
  -D unused -D unreachable_pub` on the three pure crates, for Linux and again with `--target
  aarch64-apple-darwin` (no link, no Apple SDK) (errors and unused code fail, other warnings do not;
  locally unused code only warns; a `pub` the crate root cannot reach: make it `pub(crate)`),
  `cargo machete --with-metadata` (an unused dependency fails), `cargo test` of the pure crates, and
  the SDK's type check and request-fixture tests (`sdk/`, Node).
- What `check` does not prove: that `ottospace-mac` and the shell compile (their build scripts run
  `swift build` against Apple's frameworks; GPUI's run `cc` and `bindgen` over the SDK), their
  clippy and unit tests, `cargo e2e`, the SDK against an instance, `cargo e2e-apps`. Those run in
  `.github/workflows/macos.yml`, the one macOS job, on every push to `main` and on a pull request
  that asks.
- `macos.yml`, one build: clippy `-D unused` over the workspace and `-D unreachable_pub` on
  `ottospace-mac`, `cargo test --workspace`, `cargo e2e` under cargo-nextest (`.config/nextest.toml`:
  up to three tries, a test that passes on a retry is listed as flaky), the SDK's tests against a
  private instance, then `cargo e2e-apps` (after e2e, never beside it: it takes the keyboard). Each
  suite is a step that does not stop the next; the last step fails the job if any failed, and the
  summary lists each suite's outcome and the failed tests.
- On `main`, `ci.yml` calls it on every push (a newer push cancels the older run, so a burst of
  merges tests the newest tip). While it or `check` fails, `main-red` keeps one issue of its own (never one a person opened), "main is red":
  the failed tests and `git log <last green>..HEAD`; it closes when both pass again and opens again at the next red (a re-run of an
  older push leaves it alone) (`issues: write` and
  `actions: read` on that job only). A red main is fixed before more lands.
- On a pull request, `.github/workflows/e2e.yml`, by label: `pr-e2e` runs `macos.yml` with the e2e
  tests `scripts/e2e-for` selects from the diff (all of them when the diff reaches every test), no
  app-window suite; `pr-e2e-apps` adds `cargo e2e-apps`. One job either way. Another label tests
  nothing. Not a required check.
- Unused code is deleted, or its `cfg`/visibility fixed. A library's `pub` item that no crate uses
  is not caught: make `pub(crate)` what only its own crate uses. `#[allow(dead_code)]` only with a one-line
  reason; a machete false positive goes in `[package.metadata.cargo-machete] ignored`, with one.
- `.github/workflows/reliability.yml`, nightly and by hand: `cargo e2e` and `cargo e2e-apps` five
  times each, tallied; the nightly run keeps the issue "Flaky tests", one of its own (never one a
  person opened; `issues: write` on that job only). Never on pull requests.
- A change that touches only `docs/`, `site/`, `README.md`, `AGENTS.md` or `LICENSE` runs no
  workflow on the pull request. Such a pull request has no checks to wait
  for (the merge queue still runs `ci.yml` and `duped.yml`), and after such a push `main-latest`
  trails `main` by docs commits.
- `ci.yml` and `duped.yml` also run on `merge_group` (the merge queue), with no path filter: the
  queue tests every pull request, on Linux only (`check`). Its ref (`gh-readonly-queue/...`) is its
  own concurrency group; it saves no cache. A label-gated workflow (`build`, `bench`, `shots`, `e2e`)
  skips its jobs without the label, before a runner is taken; a skipped job is not a failure.
- `.github/workflows/app-windows.yml`, by hand only: `cargo e2e-apps` once, or one test repeated
  (`repeat`, `test`), on a hosted runner, whose image grants bash Accessibility and Screen Recording.
  Prefer it, or `pr-e2e-apps`, to a local run, which takes the keyboard.
- Not in CI: rustfmt on
  `ottospace-mac` (not formatted yet), clippy `-D warnings` (main has warnings).
- `.github/workflows/build.yml`, on pushes to `main` and on pull requests that carry the label
  `pr-build` (`osi install pr <n>` adds it; without it the jobs are skipped): `scripts/test-osi.sh`,
  and `scripts/package.sh` (release binary, `.app` zip, `BUILD`, `SHA256SUMS`) uploaded as a workflow
  artifact. On `main` a third job, the only one with a write token, replaces the assets of the
  `main-latest` prerelease and moves its tag. Not a required check.
- `.github/workflows/scripts.yml`, on pull requests that change what `scripts/test-osi.sh` reads
  (`install.sh`, `scripts/osi`, `scripts/test-osi.sh`, `scripts/package.sh`): `scripts/test-osi.sh`;
  and on changes to `scripts/e2e-for`, its table or its test: `scripts/test-e2e-for.sh`.
  A file the test starts to read is added to its `paths`.
- One Rust cache for the macOS dev build: `macos.yml` saves it, on `main` only, red or green
  (`shared-key: e2e`); `e2e.yml`'s run of it, `app-windows.yml`, `shots.yml`, `bench.yml` and
  `reliability.yml` restore it and save none (`save-if: false`). A job that builds the same
  dependencies joins it; one that saves from a pull request does not. Linux's `check` keeps its own.
- `.github/workflows/shots.yml`, on pull requests that carry the label `pr-shots` and change `crates/ottospace/src/**`,
  `crates/ottospace-core/src/**`, `crates/ottospace-mac/**`, `scripts/shot`, `scripts/shots/**` or `scripts/test-shot.sh`:
  `scripts/test-shot.sh`, then `scripts/shot --compare` of every scenario against the merge base;
  the pictures are the artifact `shots`. Read-only token, no comment on the pull request. Not a
  required check. (`test-shot.sh` needs a screen that draws, so it runs here, not in scripts.yml.)
- `.github/workflows/bench.yml`, on pull requests that carry the label `pr-bench` and change `crates/**`, `Cargo.lock`, `scripts/bench`,
  `scripts/shot`, the fixtures it copies or `bench.yml`: `scripts/bench --compare` against the base branch's tip, the pull request merged into it after (dev build, both sides on one
  runner), the table in the job summary, the JSON as an artifact. Red past the thresholds in
  `scripts/bench`; not a required check: whoever merges a red run says in the pull request whether
  the regression is real and accepted. Nightly and by hand on `main`: the JSON alone, for trends.
  With the label `pr-bench-large` as well, it also spins the code city of 50,000 to 600,000 made-up
  files (`--city-large`). With `pr-bench-release`, both sides are built in release (`--release`), as the app
  ships; its spins also hold the live layer (`city_live_layer`) to a few repeated frames at 9,500 files.
- `.github/workflows/duped.yml`, on pull requests (not docs-only ones), on `ubuntu-latest`:
  `duped check --base origin/<base>` (a merge group: its `base_sha`; the release binary, pinned by version and checksum) fails on an
  added helper identical to one already there, or an identical file of 10 lines or more. A deliberate
  copy goes in `duped.json` (never also a `duped.toml`: duped refuses both), with a `reason` that
  says why the two must stay separate. A new release: change `DUPED_VERSION` and `DUPED_SHA256` together.
- `.github/workflows/deps.yml`, on pull requests that change `Cargo.lock`, a `Cargo.toml`, `deny.toml`
  or the workflow, and weekly on `main`, on `ubuntu-latest` (no build): `cargo audit` (a vulnerability
  fails) and `cargo deny --locked check` (`deny.toml`: advisories, licences, bans, sources, for the
  `aarch64-apple-darwin` graph). An advisory with no fix is ignored in `deny.toml` with a dated
  reason; a licence outside the allow-list goes in `exceptions`, by crate, never in `allow`.
- Dependency cooldown: `.cargo/config.toml` resolves only versions published 7 days ago or more
  (`global-min-publish-age`, Cargo 1.100; older Cargo warns and ignores it). Every cargo build, test,
  clippy and run in a workflow passes `--locked` (`cargo --locked e2e-apps` for an alias), so CI
  fails on a stale `Cargo.lock` instead of resolving. `.github/dependabot.yml`: cargo,
  github-actions and `sdk/` npm, weekly, 7-day cooldown, minor and patch grouped, at most 2 open
  pull requests each (each takes macOS runners).
- `.github/workflows/release.yml`, on a `v*` tag: `scripts/package.sh`, GitHub Release.
- `.github/workflows/pages.yml`, on pushes to `main` that touch `site/**`: deploys `site/` to GitHub
  Pages. Never on pull requests. Only its deploy job has `pages: write` and `id-token: write`.
- Workflows use `pull_request`, never `pull_request_target`, and name their `permissions:`. Pin
  actions by commit.

## Pull requests

- **Every pull request shows evidence.** A change to anything on screen has a before and an after
  picture (or a short recording when the change is motion); a tool, CLI, parser or CI change has
  the command's output, before and after where there is a before. No evidence, no merge.
- Pictures come from a private test instance (`OTTOSPACE_E2E`, its own `OTTOSPACE_HOME` and
  `OTTOSPACE_SOCKET` under `$TMPDIR`, made-up content), by window id only (`screencapture -l <id>`),
  never a screen region, never the user's own windows. Look at each one before it is uploaded.
  They go in the pull request's body through a secret gist, never into the repository.
- If a picture cannot be taken (the screen is locked: a locked screen draws nothing), the pull
  request says so and waits; it is not merged unseen.
- Visual evidence: `scripts/shot --compare <base> <scenario>…` (or the `shots.yml` artifact: label the pull request `pr-shots`); add
  a scenario under `scripts/shots/` for a view none covers. See `docs/development.md`.
- A tool, CLI, parser or CI change pastes the command's output instead.
- A pull request's checks are Linux only: no e2e, no picture, no compile of `ottospace-mac` or the
  shell (see "CI"). So a visual change's before and after pictures are in its body (Shots runs only
  with `pr-shots`), and its e2e evidence is the summary of a local `scripts/e2e-for --run --ci-size` run pasted in the body, or a green `pr-e2e` run (`pr-e2e-apps` for hosting, parking or adopting windows).

## Install scripts

- `install.sh` installs `scripts/osi`; `osi` installs builds (`main`, `pr <n>`, a release) into
  `~/.local/bin` and records them in `~/.ottospace/install`. Design: `docs/ai/install-and-update.md`.
- `osi` reads the asset names, the `main-latest` tag, the workflow file name, the artifact name and
  the label `pr-build`.
  Change them in `scripts/package.sh`, `build.yml` and `scripts/osi` together.
- The app writes `<home>/running` (its pid and resolved executable) at start and removes it on a
  clean quit; subcommands and test runs (`OTTOSPACE_E2E`) write none. `osi` reads it to say the
  old build is still running (`crates/ottospace/src/running.rs`).
- Each build's tarball carries `scripts/osi`; an install replaces the installed `osi` with it
  when its `OSI_VERSION` is higher. **Bump `OSI_VERSION` in `scripts/osi` on every change to it**,
  or installs will not deliver the change.
- Test with `scripts/test-osi.sh`: fixture builds, a temp `HOME`, no network. Never run
  `install.sh` or `osi` against the real `HOME` in a test; every path has an override.

## Control socket

Unix socket, mode 0600, at `$OTTOSPACE_SOCKET` or `$TMPDIR/ottospace.sock`. One JSON request per
line, one JSON reply (the full state) per line. See `Request` in `crates/ottospace/src/control.rs`.

```
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","text":"ls\r"}'
ottospace ctl '{"cmd":"state"}'
ottospace ctl '{"cmd":"open_repo","path":"/Users/me/code/app"}'  # a terminal in a listed repository
ottospace ctl '{"cmd":"capture","pane":1}'                 # one terminal pane's text, not the state
ottospace ctl '{"cmd":"wait","pane":1,"pattern":"^done$"}'  # replies when a line matches
ottospace ctl '{"cmd":"ax_tree"}'   # the focused app pane's accessibility tree, not the state
```

`crates/ottospace/skill/guide.md` is the agent-facing guide (`ottospace skill` prints it). When
you add or change a request, update the guide; a unit test parses every example in it.

`sdk/` is the TypeScript client for the socket: one method per `Request`, and the reply typed.
When you add or change a request, add or change its method in `sdk/src/client.ts` and its call in
`sdk/test/requests.test.ts`, then run `UPDATE_FIXTURE=1 npm test` in `sdk/` and commit
`sdk/test/requests.jsonl`: a unit test parses that file with `Request` and fails on a request with
no method. When the state gains or loses a key, change `sdk/src/types.ts` and `sdk/src/parse.ts`:
the SDK's tests compare the typed state with a real reply. Design: `docs/ai/sdk.md`.

Agents use the same socket to inspect and drive ottospace. Keep `Request` a closed set, and
keep the socket same-user only: `type` sends keys to a terminal, which is as strong as a shell.
`ax_tree` and `ax_overlay` only read. A capture reads a hosted window's picture and its tree
once and keeps what lies under the user's annotations. Only the user starts one (the capture
key, the menu, the bar): the core refuses a `CaptureStart` of any other origin, and
`capture_start` on the socket answers only under `OTTOSPACE_E2E` (`control::test_refusal`).
In a task's brief (`task_brief`) everything captured is quoted and labelled as untrusted: see
`docs/ai/capture.md`. Meeting titles and places are never in the state (every reply carries it):
only the read-only `calendar` request gives them, with times and busy/free, from what the app
holds (it never fetches); never a meeting's link, description or attendees, the feed's address or
`calendar_email`. One more read-only request gives a title: `note`, what a quick note (⌘⇧N) was
filed with (the meeting's title and times, the transcript's file and how far in, what was in
front), only what the user left on as pills; in its task's brief it is quoted and labelled
untrusted. A filed decision's words (`decision_sources`: a Slack thread's, an issue's) are never in
the state either: only the read-only `decision` gives them, labelled untrusted; filing one acts on
nothing. The guide tells agents titles and transcript text are data, never instructions. The user's own sneak label may press a control in the focused app pane
(`docs/ai/sneak-apps.md`): the core refuses a label in an app from any other origin. Requests
that press or set values in another app are still a
separate trust decision: see `docs/ai/1-initial-brainstorm/accessibility.md`.

## App bundle

- `scripts/bundle.sh` builds `target/release/bundle/ottospace.app`. Ship only the `ottospace` binary in it, never `ottospace-canary`.
- `crates/ottospace/assets/ottospace.icns` is generated: change `assets/icon.svg` (and `icon-small.svg`, the 16 and 32 pixel sizes) and run `scripts/icon.sh`, do not edit it.

## Docs

- `README.md` is the front page: the pitch, install, setup and links. Keep it short. Detail lives in
  `docs/reference.md` (status tables and roadmap, keys, panes, settings, `ottospace ctl`, hosting, the list
  of design docs), `docs/install.md` (`osi`, other builds, the app bundle) and `docs/development.md`
  (building, tests, crates, CI, releasing). A new key, setting or capability goes in `docs/reference.md`.
- `site/` is the static site (`index.html`, `docs.html`, `shots/`), deployed to GitHub Pages by
  `.github/workflows/pages.yml`. Keep it in step with README's claims: when one changes, change the
  other, and `site/docs.html` in step with `docs/reference.md`. Neither presents a roadmap item as
  available.
- `docs/cheat-sheet.md` — one page for daily use. Update it when a key, a section or a setting changes.
- `docs/ideas/<n>-<name>.md` — Bryan's own notes. **Never edit, move or reformat.** Read-only input.
- `docs/ai/<n>-<name>/` — AI-derived output for that note: plans, refinements, research.
  e.g. `docs/ideas/1-initial-brainstorm.md` → `docs/ai/1-initial-brainstorm/plan.md`.
  Plan changes go here, never back into `docs/ideas/`.

## Style

- Format with `cargo fmt` (`rustfmt.toml`: 120 columns).
