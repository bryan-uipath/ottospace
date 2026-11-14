# Scripting SDK

Design for the TypeScript SDK. Builds on `1-initial-brainstorm/scripting.md` (TypeScript, a `bun`
sidecar, the action deck) and Bryan's requests of 2026-11-11 and 12: window rules "a faculty in
our TS SDK", per-app persistence "that could be overridden", the Zoom layout and spinner, quick
actions, and "a playwright style api (like selectors for the a11y tree)". Not agreed: first
design to react to. Built so far: only the typed client in `sdk/` (stage 0 below).

## Decisions, in short

| Question | Decision | Main reason |
|---|---|---|
| Where scripts run | A separate process (bun), started and watched by the app, talking over the control socket. No engine in the app now | A script cannot stall or crash the app, and it is the one protocol agents and tests already use |
| Transport | Keep request/reply. Add `subscribe`: the connection stays open and the app writes events and, for hooks, questions with a deadline and a default | A hook must be asked and answered in milliseconds without the app ever waiting on a script |
| `Request` closed set | Stays closed. Scripts register data (an action's title, a selector); they never add a request. Events and questions are two more closed enums | The trust rule in `AGENTS.md` does not change |
| Types | Hand-written TypeScript, held to the Rust code by tests | The reply is put together with `json!` in the shell, so there is no Rust type to generate it from |
| Accessibility | Playwright-style locators, evaluated inside the app, step by step, with a wait. Reads first. Proposed, not decided: presses behind a per-app setting | Zoom builds parts of its tree only when asked, so a snapshot misses them |

## 1. Where scripts run

| | Separate process (bun or node) | Engine in the app (QuickJS, boa, deno_core) |
|---|---|---|
| Start | Once, when the app starts. Measured here: bun 15 ms, node 26 59 ms to run a one-line `.ts` | None |
| A hook's answer | One round trip on a Unix socket to a process that is already running. Not measured | A function call |
| A script that loops forever or crashes | The app goes on: the question times out and the default applies | Stalls the UI unless the engine has its own thread and an interrupt; a crash in the engine is a crash of the app |
| "Core has no I/O" | Untouched. Scripts are outside the app | The engine would live in the shell crate, never the core. Possible, but every API is then written twice (engine binding and socket) |
| TypeScript, no build step | bun runs `.ts`. Node 22.18+ strips types, but refuses files under `node_modules` (checked: `ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING`) | QuickJS and boa run JavaScript only: a TypeScript stripper (swc or oxc) must be built in too. deno_core is V8: tens of MB and a long build |
| npm packages, `fetch`, files, child processes | Yes | Each must be written and bound by hand |
| Debugging | `bun --inspect`, stack traces with file and line | Little or none |
| Size | bun is 58 MB (measured, 1.3.10). The app is one bare binary today | QuickJS adds about 1 MB (not measured) |
| Sandbox | None. A script can do what the user can | Possible: a script could be given only `otto` |

**Decision: a separate process, and nothing embedded until a measurement asks for it.** One
"script host" process loads every script. An embedded engine buys two things: no 58 MB, and a
sandbox. Neither is needed yet. Scripts are the user's own code, and what the owner asked for
(Chrome tabs through AppleScript, opening URLs, the spinner) needs files, processes and the
network, which a sandbox would take away.

Staged, so option (c) stays open: if a `windows.on` answer proves too slow, or a no-download
install matters more than npm, the pure decision hooks (`windows.on`) can move to an embedded
QuickJS later without changing the scripts, because the hook API is data in, data out.

**Where bun comes from.** `scripting.md` records "bun, bundled" in the `.app`. The installer
(`osi`) ships a bare binary today, so there is nothing to bundle into. Proposal:

1. Lookup order: `$OTTOSPACE_BUN`, a bun beside the app binary, `bun` on `PATH`.
2. None found and the scripts folder is not empty: the status bar says
   `scripts: bun not found (osi scripts, or install bun)`, and the app runs without scripts.
3. `osi scripts` fetches one pinned bun (version and SHA-256 in the repository) into
   `~/.ottospace/bin`. Opt-in: a user with no scripts never downloads 58 MB.
4. Node is not a script runtime: it cannot load the SDK's `.ts` from `node_modules` without a
   build step. The client library in `sdk/` does run under Node, for tests and for a user's own
   project.

**Scripts an agent writes.** A script in the folder runs at every start with the user's powers,
so writing one is as strong as editing `~/.zshrc`. A same-user agent can already do both; the
app cannot stop it. What the app can do is make it visible: a script file that is new or changed
since the user last ran it shows one status-bar notice (`scripts: zoom.ts changed`) the first
time it loads, and the guide tells agents not to write to the scripts folder unless asked. No
socket request writes or enables a script. Whether a changed script should wait for the user's
approval before it runs is open question 2.

## 2. Transport

Today: one JSON request per line, one JSON reply per line, answered in order on a connection.
That stays, byte for byte. Three additions.

**a. A protocol number.** Every state reply gains `"protocol": 1`, beside `version` (the
build, which is there today). It goes up when a reply
loses or changes a field. The SDK refuses to run against a number it does not know.

**b. `subscribe`.** A new request. Its connection becomes a stream:

```
→ {"cmd":"subscribe","events":["task","tab","pane","mode","notice"],"client":"scripts"}
← {"ok":true,"protocol":1,"subscription":3}
← {"event":"task_changed","seq":1,"task":{"id":4,"state":"needs_you",..}}
← {"event":"mode_changed","seq":2,"mode":"app"}
```

- `events` names groups; an unknown name is refused. Leaving it out subscribes to nothing but
  questions (below).
- Each event is one line with `event` (the kind) and `seq` (counts up by one on this
  subscription). An event carries the changed thing, not the whole state. After a gap, call `state`.
- First set: `task_changed`, `task_removed`, `tab_opened`, `tab_closed`, `tab_selected`,
  `pane_opened`, `pane_closed`, `focus_changed`, `mode_changed`, `notice_changed`. Later:
  `window_appeared`, `window_closed`, `app_launched`, `app_quit` (need the window watch, stage 3),
  `action_pressed` (stage 4).
- **Pane output is not streamed.** `pane_output {pane}` says only that a pane has new text, at
  most 10 times a second per pane, and only for panes the subscription names
  (`"panes":[3]`). The text is read with `capture`. This keeps other panes' text (which may
  show secrets) out of a standing feed, and keeps the stream small.
- The other direction on this connection takes two requests only: `answer` (below) and
  `subscribe` again to change the groups. Everything else goes on a second, ordinary connection,
  so a slow `wait` never delays an event. The SDK opens both.
- `ottospace ctl` prints one reply and exits, so it is no use here. Add `ottospace events
  [groups]`, which prints the stream until stopped. Useful by itself for e2e tests and agents.

**c. Questions (`ask` / `answer`).** For `windows.on`, and later `apps.define`:

```
→ {"cmd":"subscribe","hooks":[{"hook":"window","id":"w1","select":{"app":"us.zoom.xos","title":"Zoom Meeting"}}]}
← {"ok":true,"protocol":1,"subscription":3}
← {"ask":17,"hook":"w1","deadline_ms":150,"window":{"app":"zoom.us","bundle":"us.zoom.xos","title":"Zoom Meeting","subrole":"AXStandardWindow","size":[880,496],"display":2}}
→ {"cmd":"answer","ask":17,"value":{"do":"ignore"}}
```

- The **selector is matched in the app**, with `rules::Rule::selects` (the same code the
  `windows` setting uses). The script is asked only about windows its selector selects.
- The app waits `deadline_ms` (150 for a window; 2000 for `capture` and `restore`). No answer
  in time, an answer for an unknown `ask`, or a value that does not parse: the **default**
  applies, which is what the data rules alone would have done, and a late answer is dropped. The
  window is left where it is while the question is open.
- Three missed deadlines in a row from one subscription: the app stops asking it and shows
  `scripts: zoom.ts does not answer`.
- Core: the shell sends `Action::WindowSeen { facts }`. The core answers with
  `Effect::Ask { .. }` if a hook selects it, else decides from the rules. The answer comes back
  as `Action::WindowAnswered`, the deadline as `Action::AskExpired` (the shell runs the timer:
  the core has no clock). Each has a unit test. The decision stays in the core.

**Back-pressure.** The UI thread never writes to a socket. It puts events on a bounded queue
(256) per subscription, with `try_send`; a writer thread per connection drains it. If the queue
is full, the subscriber is slow or stuck: the app drops the subscription and closes the
connection. The client sees the end of the stream, connects again, reads `state` and carries
on: events are hints, the state is the truth. When a connection ends for any reason, everything
it registered (hooks, actions) is removed. So a crashed script leaves nothing behind.

**The closed-set rule.** `subscribe` and `answer` are two more variants of `Request`; the set
stays closed and still `deny_unknown_fields`. Events are a closed `Event` enum and questions a
closed `Ask` enum, both in `ottospace-core` (they are data the core produces). A script can
register an action's title or a selector. It cannot add a request, and the app runs no code a
client sends.

**The guide's unit test.** `the_agent_guide_covers_every_request` keeps working: `subscribe`
and `answer` need examples. Add its twin: every `Event` and `Ask` variant has an example line
in the guide, and each example deserializes into the enum. The SDK's fixture test (section 6)
grows the same way.

Found while testing, not fixed here: `deny_unknown_fields` does not apply to requests with no
fields. `{"cmd":"state","extra":1}` is accepted. The guide says unknown fields are refused.

## 3. API

Marks: **[socket]** works on today's socket. **[request]** needs a new request. **[events]**
needs `subscribe`. **[app]** needs new behaviour in the app.

In `sdk/` now: `state`, `terminal(pane)` (`type`, `wait`, `capture`, `run`) and `tasks`, as
below, and every request as a method of the plain client (`otto.client.split(..)`,
`otto.client.openRepo(..)`). Not in `sdk/` yet, though the socket can do them: the `panes`,
`tabs` and `repos` groups.

```ts
declare const otto: Otto;

interface Otto {
  state(): Promise<Snapshot>;                                  // [socket]
  version(): Promise<{ app: string; protocol: number }>;       // [socket] app: `version` in the state. [request] protocol
  notify(text: string): Promise<void>;                         // [request] a status-bar notice
  open(target: string, options?: OpenOptions): Promise<Pane>;  // [request][app] a URL or an app, by bundle id

  panes: {
    list(): Promise<Pane[]>;                                   // [socket] from state
    split(options?: { axis?: Axis; kind?: "terminal" | "empty" }): Promise<Pane>;  // [socket]
    focus(pane: PaneId): Promise<void>;                        // [socket]
    close(pane: PaneId): Promise<void>;                        // [socket]
  };
  tabs: {
    list(): Promise<Tab[]>;                                    // [socket]
    create(): Promise<Tab>;                                    // [socket] new_tab
    select(tab: Tab): Promise<void>;                           // [socket] by index today; by id needs [request]
    close(tab: Tab): Promise<void>;                            // [socket]
  };
  terminal(pane: PaneId): {
    type(text: string): Promise<void>;                         // [socket]
    wait(pattern: string, timeoutMs?: number): Promise<string>;  // [socket]
    capture(history?: boolean): Promise<string>;               // [socket]
    run(command: string): Promise<{ status: number; output: string }>;  // [socket] type + wait + capture
    onOutput(handler: () => void): Unsubscribe;                // [events] pane_output
  };
  tasks: {
    add(title: string, options?: TaskOptions): Promise<Task>;  // [socket]
    list(): Promise<InboxRow[]>;                               // [socket]
    setState(id: TaskId, state: TaskState): Promise<void>;     // [socket]
    open(id: TaskId): Promise<void>;                           // [socket]
    leave(): Promise<void>;                                    // [socket]
  };
  repos: {
    list(): Promise<Repo[]>;                                   // [socket]
    open(path: string, axis?: Axis): Promise<Pane>;            // [socket]
  };
  settings(): Promise<Settings>;                               // [request] read-only; no request returns them today

  on<E extends keyof Events>(event: E, handler: (e: Events[E]) => void): Unsubscribe;  // [events]

  windows: {
    /** Asked when a window the selector selects appears. Return nothing to let the rules decide. */
    on(selector: WindowSelector, handler: (w: SeenWindow) => WindowVerdict | void): Unsubscribe;  // [events][app]
    list(selector?: WindowSelector): Promise<WindowFacts[]>;   // [request][app] also windows not in a pane
  };
  apps: {
    /** Per-app session restore. Last definition for a bundle id wins. */
    define<S>(bundleId: string, adapter: {
      capture(win: HostedWindow): S | Promise<S>;              // [events][app] asked when the session is saved
      restore(state: S): void | Promise<void>;                 // [events][app] asked when it is restored
    }): void;
  };
  actions: {
    /** A tile on the action deck. Shown while `when` is true; `when` is checked again on events. */
    define(action: {
      id: string; title: string; icon?: string; key?: string;
      when?: () => boolean | Promise<boolean>;
      run(): void | Promise<void>;
    }): Unsubscribe;                                           // [request][events][app] the deck does not exist
  };
  app(bundleIdOrName: string): AppHandle;                      // [request][app] section 4
}

/** The fields of a `windows` rule in settings.json. A string is a glob (`*`), letters in any case. */
interface WindowSelector {
  app?: string;            // name or bundle id
  title?: string | RegExp; // a RegExp is checked in the script, after the app's own match on the rest
  subrole?: string;
}
interface WindowFacts { app: string; bundle: string | null; title: string; subrole: string;
                        size: { width: number; height: number }; display: number }
interface SeenWindow extends WindowFacts {
  ignore(): WindowVerdict;             // never in a pane (the rule action `ignore`)
  manage(): WindowVerdict;             // as any other window (`manage`)
  slot(name: string): WindowVerdict;   // [app] into a named slot of a layout; stage 6
}
```

`size` and `display` are new facts: `rules::Facts` has `app`, `bundle`, `title`, `subrole`
today. They are what tells Zoom's two `Zoom Meeting` windows apart (880 × 496 against about
1400 × 940, measured). The owner's example then reads as written:

```ts
otto.windows.on({ app: "us.zoom.xos", title: /Zoom Meeting/ },
  (w) => w.size.width < 900 ? w.ignore() : w.slot("main"));
```

A handler is synchronous on purpose: it must answer inside the deadline from the facts it was
given. One that needs to look further (read the window's controls) returns nothing and acts later.

The selectors are `ottospace_core::rules` ("settings first, then SDK"): stage 3 adds to them.

## 4. Locators: a Playwright-style API over an app's accessibility tree

Bryan, 2026-11-12: "a playwright style api (like selectors for the a11y tree)".

```ts
interface AppHandle {
  windows(selector?: WindowSelector): Promise<WindowFacts[]>;
  /** Every window the selector selects. A locator searches all of them. */
  window(selector?: WindowSelector): Locator;
}
interface Locator {
  // Narrowing. Lazy: nothing is read until a read or an action is called.
  getByRole(role: Role, options?: { name?: string | RegExp; description?: string | RegExp;
                                    value?: string | RegExp; exact?: boolean }): Locator;
  getByText(text: string | RegExp, options?: { exact?: boolean }): Locator;
  locator(step: { role?: Role; subrole?: string; name?: string | RegExp }): Locator;
  filter(options: { hasText?: string | RegExp; has?: Locator }): Locator;
  first(): Locator;
  nth(index: number): Locator;

  // Reads. Each waits until the locator matches, up to `timeout` (default 5000 ms).
  count(): Promise<number>;            // does not wait: 0 is an answer
  all(): Promise<Locator[]>;
  text(): Promise<string>;             // the name (rule below). More than one match is an error
  allTexts(): Promise<string[]>;
  describe(): Promise<string | null>;  // AXDescription
  value(): Promise<string | null>;     // AXValue
  attribute(name: string): Promise<string | null>;  // any AX attribute, as text: "AXHelp"
  frame(): Promise<Frame | null>;      // global screen points
  isVisible(): Promise<boolean>;       // does not wait
  waitFor(options?: { state?: "attached" | "visible" | "detached"; timeout?: number }): Promise<void>;

  // Actions on another app. Refused unless the user allowed that app: "Read and act" below.
  click(): Promise<void>;              // AXPress
  press(action: string): Promise<void>;  // another AX action: "AXShowMenu"
  fill(text: string): Promise<void>;   // sets AXValue
}
```

**Where it follows Playwright:** `getByRole`, `getByText`, `locator`, `filter`, `first`, `nth`,
`count`, `all`, `waitFor`, `click`, `fill`, `isVisible`, laziness, waiting, and strictness (a
single read on several matches is an error naming the count; use `first()` or `allTexts()`).

**Where macOS differs:**

- **Roles: both forms.** A name starting `AX` is the raw role and passes through
  (`"AXOutline"`). A lowercase name is looked up in a fixed table and anything else is an error
  that lists the table: `button` AXButton, `checkbox` AXCheckBox, `radio` AXRadioButton, `link`
  AXLink, `textbox` AXTextField or AXTextArea, `text` AXStaticText, `image` AXImage, `row`
  AXRow, `cell` AXCell, `table` AXTable, `tree` AXOutline, `list` AXList, `group` AXGroup,
  `menuitem` AXMenuItem, `tablist` AXTabGroup, `slider` AXSlider, `combobox` AXComboBox or
  AXPopUpButton, `window` AXWindow. Raw roles are needed because apps use roles the table does
  not have, and they are what `ax_tree` prints.
- **Name.** ARIA computes one "accessible name"; macOS has three attributes and apps choose
  differently. The rule: the first that is not empty of `AXTitle`, `AXDescription`, and, only
  for `AXStaticText`, `AXTextField`, `AXTextArea` and `AXCell`, `AXValue`. Measured in Zoom: its
  buttons have an empty title and say everything in the description; a participant's name is the
  static text's value. `description` and `value` options match one attribute alone.
- **Text matching.** A string matches as a substring in any case; `exact: true` asks for the
  whole text. A RegExp is sent as its source and matched by the app with the Rust `regex` crate
  (already a dependency, for `wait`). Only the `i` flag is taken; look-around and
  back-references are refused with an error, not ignored.
- **Names not in Playwright:** `text()` (its `textContent`), `allTexts()` (`allTextContents`),
  `value()` (`inputValue`), `attribute()` (`getAttribute`), `frame()` (`boundingBox`). Recent
  Playwright has a `describe()` with another meaning (a label for traces; from memory, not
  checked): open question 6.
- **Visible** means: has a frame with area inside its window's frame. Accessibility has no
  "hidden". A parked window's controls count as visible.

**Lazy trees, so no snapshot.** Measured in Zoom (`zoom-ax.md`, branch `zoom-ax`): a
whole-window read returned the participant rows with empty cells; the name appeared only when
each cell was asked for its children. So a locator is not matched against a tree the script
holds. The chain is sent to the app and resolved there, step by step: each step asks the
current matches for their children and searches down from them. One new request:

```
→ {"cmd":"ax_query",
   "target":{"app":"us.zoom.xos","title":"Zoom Meeting"},
   "steps":[{"role":"AXOutline"},{"role":"AXRow"},{"role":"AXStaticText"}],
   "read":["name","description","value","frame"],
   "wait":"attached","timeout_ms":5000,"limit":200}
← {"ok":true,"count":11,"matches":[{"window":4411,"path":[0,3,0,0,2,0,0],"role":"AXStaticText","name":"Ana Example (me)","value":"Ana Example (me)","frame":{..}}, ..],
   "truncated":{"depth":false,"nodes":false,"timeout":false},"millis":940}
```

- `steps` is the chain; a step may carry `role`, `subrole`, `name`, `description`, `value`
  (each `{"contains":".."}`, `{"exact":".."}` or `{"regex":".."}`), `has_text`, and `nth`.
- It reuses the bounded reader's limits (`ax_tree.rs`): depth 40, 3000 nodes per query, 0.5 s
  per element, breadth first, off the UI thread. `truncated` means the same as in `ax_tree`.
- **Waiting is in the app:** it evaluates again every 100 ms until the `wait` state holds or
  `timeout_ms` passes (capped at 600000, as `wait` is). One request, no polling from the script.
- **Why not ship trees to the script:** a 3000-node tree takes about 2 s to read and is mostly
  not wanted; it holds other people's text the script did not ask for; and it would still miss
  the children Zoom makes on request. `ax_tree` stays as the debugging view.
- Core and shell: a step's test (role table, name rule, text match) is a pure function in
  `ottospace_core::ax`, unit-tested against recorded trees. The walk is in `ottospace-mac`. A
  Zoom fixture with names replaced is needed; only a Calculator fixture exists.

**Examples, from the Zoom measurements:**

```ts
const zoom = otto.app("us.zoom.xos");
const meeting = zoom.window({ title: "Zoom Meeting", subrole: "AXStandardWindow" });

// Is the user sharing? A window that exists only while sharing.
const sharing = (await zoom.windows({ title: "zoom share toolbar window" })).length > 0;

// How many participants, with the panel closed: the toolbar button's description is
// "Open participants panel, closed, 10 participants".
const button = meeting.getByRole("button", { description: /participants panel/i });
const count = Number((await button.describe())?.match(/(\d+) participants/)?.[1] ?? 0);

// Names, with the panel open: outline > row > cell > static text.
const rows = meeting.getByRole("tree").getByRole("row");
const names = await rows.getByRole("text").allTexts();          // ["Ana Example (me)", ...]

// Who is muted: each row's images are described "Computer audio muted", "Video off".
const muted = await rows.filter({ has: meeting.getByRole("image", { description: "Computer audio muted" }) })
  .getByRole("text").allTexts();
```

Both `Zoom Meeting` windows match `meeting`; the search runs in both and finds the outline in
the one that has it. That is why `window()` is "every window selected", not "exactly one".

**Read and act.** Reading is what is agreed today (`ax_tree` reads). `click`, `press` and
`fill` act in another app, which is the separate decision in `accessibility.md`: ottospace would
lend its Accessibility grant to any client of its socket. Proposal:

- The methods exist, and are refused unless `settings.json` allows that app:
  `"ax_actions": ["us.zoom.xos"]`. Default: none. The refusal names the setting.
- The two guards `accessibility.md` proposes stay, whatever the setting says:
  1. **Only hosted panes, never other windows.** An action is refused on a window that is not
     in a pane. Reads may reach windows outside a pane (Scope, below); actions may not. So a
     script cannot press in an app the user has not brought into ottospace.
  2. **Refused while the target app is frontmost and the user is typing**, so a script's press
     or `fill` cannot land in the middle of the user's own input.
- An action re-finds the element, checks its role still matches, then acts (as
  `accessibility.md` plans for `ax_press`).
- While a script acts, the status bar shows it for 2 seconds: `SCRIPT zoom.ts pressed "Open
  participants panel" in zoom.us`. Every action is one line in `~/.ottospace/logs/ax.log`.
- Until the owner decides, the Zoom flow needs no press: the user opens the participants panel,
  the script reads it.
- Electron and Chrome show their page content only after a client **writes**
  `AXManualAccessibility` / `AXEnhancedUserInterface`. That is a write with side effects
  (`accessibility.md`), so it belongs with actions, not reads.

**Scope: apps that are not in a pane.** `ax_tree` needs a hosted pane today. Zoom's panel
during a meeting is not hosted. Needed: find a running app by bundle id or name, list its
windows with their facts (the same facts the window rules use), and read by pid and window
without a pane. `mac_ax::Window::find(pid, id)` already reads by pid; the app list and window
facts are new. The Accessibility grant is the one ottospace already has.

**Privacy.** A locator returns other people's names and message text (Zoom's chat panel is
readable while open). The rule the guide gives for pane text applies: use only what the task
needs, do not store or quote the rest. Queries are logged by app and step, never by result.
Whether reading apps outside a pane should also be a per-app setting is open question 5.

## 5. Loading

- **Folder:** `$OTTOSPACE_HOME/scripts/*.ts` (`~/.ottospace/scripts`). One file per app or
  scenario (answers the open question in `scripting.md`). An e2e run with no `OTTOSPACE_HOME`
  starts no scripts.
- **One host process** loads every file, in name order, each in its own `try`: a file that
  throws while loading is reported and skipped; the others load. One process per file would
  isolate better and cost about 30 MB each (not measured).
- **`otto` is a global** the host sets, as in the owner's examples. The app unpacks the SDK it
  was built with to `~/.ottospace/sdk/` (the `.ts` sources, embedded with `include_str!`) and
  writes `scripts/ottospace.d.ts`, so an editor has the types and they match the running app.
- **Start:** after the socket is up, if the folder holds a `.ts` file and scripts are not off.
  **Restart:** when a file in the folder changes (the app watches the folder), and after a
  crash, waiting 1, 2, 4, 8 seconds; after five crashes in a minute it stops until a file
  changes. A restart is always the whole host: the connection ends, so its hooks and actions are
  removed, and the new host registers again. No half-reloaded state.
- **Errors reach the user** through `notify`, shown as the status-bar notice:
  `scripts: zoom.ts: TypeError: w.size is undefined (zoom.ts:4)`. The host catches errors per
  file and per handler and sends them. The host's stderr goes to `~/.ottospace/logs/scripts.log`.
  If the host dies, the app shows the last line it wrote.
- **Turning off:** `"scripts": { "enabled": false }` in `settings.json` for all;
  `"scripts": { "disabled": ["zoom.ts", "builtin:chrome"] }` for one.
- **Built-in defaults.** Data where data can say it: the built-in window rules are already
  data in `rules::built_in()`. Scripts where code is needed: per-app adapters
  (`builtin:chrome`, `builtin:slack`, `builtin:zoom`) are `.ts` files embedded in the binary and
  loaded first.
- **Overriding.** Built-ins load first, the user's files after, in name order.
  `apps.define(id)`: the last definition wins, so a user's Chrome adapter replaces the built-in.
  `windows.on`: the user's handlers are asked before built-in ones and the first answer wins,
  which is the order `rules::manages` already uses (user's rules, then built-in).

## 6. Trust

- **Scripts are the user's own code** and run as the user, outside the app. The app gives them
  no power the user lacks.
- **What a script may do that the socket does not offer today:** hold a standing feed of events
  (task titles, window titles, which pane has new output); decide what happens to a new window
  (`windows.on`); put tiles in the app's own UI (`actions.define`); be asked to save and bring
  back an app's state (`apps.define`); show a notice; open a URL or app; read the controls of
  apps that are not in a pane (locators).
- **Not without the separate decision:** anything that presses or types into another app.
  `click`, `press`, `fill` are refused by default (section 4). `type` into a terminal pane is
  as today.
- **Agents use the same socket** and may use the same SDK. What differs:
  1. **Standing hooks are for the script host.** The app starts the host with a random token
     in its environment (`OTTOSPACE_HOST_TOKEN`); `subscribe` with `hooks`, and
     `actions.define`, need it. An agent can subscribe to events and call every other request.
     This is a guard against accidents, not a wall: a same-user process can read another's
     environment.
  2. **`client` on `subscribe`** (`"scripts"`, `"agent:claude"`) is a label for logs and for
     saying who registered what. It proves nothing.
  3. **Guide rules for agents:** subscribe to `pane_output` only for panes you opened; do not
     write to the scripts folder unless the user asked; locator reads follow the pane-text rule.
- The socket stays same-user, mode 0600.

## 7. Keeping the types true

**Decision: hand-written types, held by tests.** Considered: generating them (`ts-rs` or
`specta` derives, or `schemars` and a JSON-schema converter).

- A generator covers `Request` and the `Model`. It does not cover the reply: `snapshot()` builds
  it with `json!`, and `task_add`, `capture`, `wait`, `ax_tree` each build their own. Half the
  top-level keys (`bounds`, `terminals`, `inbox`, `apps`, ...) have no Rust type.
- It puts a derive and a dependency in `ottospace-core`, which is `std` and `serde` only.
- Revisit when the reply becomes a struct. Then generation is the better answer.

What holds the hand-written types today (all in `sdk/test`, and one Rust test):

1. `requests.jsonl` holds one request per client method, written by the client itself with
   every field set. A Rust unit test parses each line with the real `Request` and checks every
   variant is there. A new variant, a renamed field or a wrong type fails `cargo test`.
2. A TypeScript test fails if the file is stale, if a method is missing from it, or if a variant
   in `control.rs` has no method.
3. Replies are parsed with runtime checks, not casts. Against a real app, the parsed snapshot
   must equal the raw reply, key for key: a key the app adds or drops fails the test.

**Versions.** The SDK is not published; it has the app's version and ships inside the app
(section 5), so a script always gets the SDK of the app that runs it. `ottospace --version`
names the build. `protocol` in the state (section 2a) is for clients that do not come with the
app: an agent's own copy, a test.

## 8. Stages

Each is one small PR that works by itself.

| # | PR | Unlocks |
|---|---|---|
| 0 | Typed client for today's socket (`sdk/`). This PR | Tests and tools in TypeScript; a checked list of what the socket says |
| 1 | `protocol` in the state; `subscribe` with the first event set; `ottospace events`; `otto.on` | E2E tests and agents stop polling; an agent can wait for `needs_you` |
| 2 | Script host: the folder, start and restart, `notify`, errors in the status bar, the `scripts` setting, `otto` as a global | A user's script that reacts to tasks and terminals |
| 3 | Window watch (an AX observer for apps named by a rule or hook), `size` and `display` in the facts, `ask` / `answer`, `otto.windows.on` | **Window rules by callback**: the two Zoom windows told apart by size |
| 4 | Action deck: actions in the core, the deck in the shell, `actions.define`, `action_pressed` | **The action deck**; "start a task" and "pop next" tiles need no script |
| 5 | `ax_query` and locator reads, on hosted panes and by app for windows not in a pane; `otto.open` | Participant names and count; "is the user sharing?" |
| 6 | Layout slots (`w.slot`), a layout template in the core, `builtin:zoom` with the Spin action | **The Zoom layout** and names to the spinner (the user opens the panel) |
| 7 | `apps.define`, `capture` on save and `restore` on start, `builtin:chrome`, `builtin:slack`. Needs layout and terminal-history restore first, which needs no scripting | **Per-app capture and restore** with defaults a user can replace |
| 8 | Locator actions behind `ax_actions`, the status-bar line and the log. Needs the owner's decision in `accessibility.md` | A script that opens the participants panel itself |

Stages 4 and 5 do not depend on each other; either can go first.

## Not verified

- No part of sections 2 to 6 is built. The protocol is a specification, not tested code.
- The round-trip time of a question on the socket. The 150 ms deadline is a guess.
- That a bounded queue of 256 and dropping the subscriber is right under real event rates.
- The memory of a bun host, and the size of QuickJS in the binary.
- bun under the hardened runtime in a signed `.app` (the JIT entitlement), noted in `scripting.md`.
- That the tree reader sees inside Zoom's cells: `zoom-ax.md` says it should and that it was
  not run against Zoom. The Zoom examples follow that note's measurements; none was run.
- The role table and the name rule against apps other than Zoom and Calculator.
- An AX observer for window-created on apps that are not hosted: not prototyped.
- That JavaScript RegExp sources people write are mostly valid for the Rust `regex` crate.
- Chrome tabs and profile, Slack's channel, Outlook's view: how each is captured and restored
  is not researched here (`plan.md` names AppleScript, deep links and the `code` CLI).
- Measured here: bun and node start time, bun's size, Node refusing `.ts` under `node_modules`,
  and everything in `sdk/` against a real private instance.

## Open questions

1. **bun:** fetched by `osi scripts` on request (proposed), or always shipped beside the binary?
2. **A changed script:** runs at once with a notice (proposed), or waits for you to approve it?
   The second protects against a script an agent wrote and costs a key press on every edit.
3. **`otto` as a global** (your examples) or an import (`scripting.md`)? The global needs no
   module resolution. An import from the bare name `ottospace` is not safe until the package is
   published by us: the name on npm is not ours (not checked who has it, if anyone).
4. **Field names:** the socket is snake_case (`tab_panes`, `timeout_ms`) and the client in
   `sdk/` keeps it, so a request object is what is sent. Should the script API be camelCase
   instead (`w.size.width` already is)?
5. **Reading apps that are not in a pane:** any app, or only apps the settings name?
6. **`describe()`** as you named it, or `description()` to stay clear of Playwright's own
   `describe()`?
7. **`ax_actions`:** is a per-app list in the settings enough for you to allow presses, or do
   you want it per script as well?
8. **Events for agents:** may any socket client subscribe, or only the script host at first?
