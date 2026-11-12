// The shapes on the control socket, as the Rust code writes them. Sources:
// `crates/ottospace/src/control.rs` (requests), `snapshot` in `crates/ottospace/src/main.rs`
// (the reply), `crates/ottospace-core/src/{model,tasks,repos,layout,ax}.rs` (what is in it).
// Field names are the wire's own (snake_case), so a request object is what is sent.

export type PaneId = number;
export type TaskId = number;

export type Axis = "right" | "down";
export type Dir = "left" | "right" | "up" | "down";
export type Section = "apps" | "workbook" | "tasks" | "repos";
/** Pane kinds `split` may create. App panes come from `open_app`. */
export type SplitKind = "terminal" | "empty";
/** Who has the keyboard. */
export type Mode = "normal" | "app" | "pick" | "visual" | "list" | "compose" | "sneak" | "confirm";

export type TaskState =
  | "captured"
  | "triaged"
  | "running"
  | "needs_you"
  | "review"
  | "stale"
  | "parked"
  | "done"
  | "cleaned";
/** A heading of the inbox, in the order shown. */
export type Group = "needs_you" | "running" | "captured" | "on_hold" | "done";

export type PaneKind =
  | { type: "empty" }
  | { type: "terminal" }
  /** `window`: the window-server id of the hosted window. */
  | { type: "app"; window: number; title: string };

export interface Pane {
  id: PaneId;
  kind: PaneKind;
  /** Only on a terminal opened by `open_repo`. */
  cwd?: string;
}

export interface Tab {
  /** Stable. Requests take the index in `tabs`, not this. */
  id: number;
  title: string;
  /** The task this tab belongs to, or null. */
  task: TaskId | null;
}

/** The active tab's split tree. `ratio`: the share of the split that `a` (left or top) takes. */
export type Layout = { leaf: PaneId } | { split: { axis: Axis; ratio: number; a: Layout; b: Layout } };

export interface Task {
  id: TaskId;
  title: string;
  state: TaskState;
  /** 1 is most urgent. */
  priority: number;
  /** Milliseconds since 1970, or null. */
  snoozed_until: number | null;
  dismissed: boolean;
  project: string | null;
  /** A repository path. */
  repo: string | null;
}

export interface InboxRow extends Task {
  group: Group;
}

export interface Worktree {
  name: string;
  path: string;
  branch: string | null;
}

export interface Repo extends Worktree {
  worktrees: Worktree[];
}

export type PickItem = { type: "terminal" } | { type: "app"; name: string };

export interface Picker {
  axis: Axis;
  new_tab: boolean;
  items: PickItem[];
  query: string;
  /** An index into the items that match `query`. */
  selected: number;
}

/** A terminal a quit would end something in. */
export interface Running {
  pane: PaneId;
  /** The program's name, or null when it could not be read. */
  program: string | null;
  /** True: a background or stopped job of the shell. False: the program in front of it. */
  background: boolean;
}

/** The quit question: the terminals that run something. */
export interface Confirm {
  running: Running[];
}

/** `state` in a reply: the core's model. */
export interface Model {
  /** Panes of every tab. */
  panes: Pane[];
  tabs: Tab[];
  /** Index into `tabs`. */
  active: number;
  layout: Layout | null;
  focused: PaneId | null;
  /** The app pane that has the keyboard. */
  live: PaneId | null;
  /** The task whose tabs are on the tab bar, or null for the tabs of no task. */
  current_task: TaskId | null;
  sidebar: Section | null;
  /** The text in the new-task box while it is open. */
  compose: string | null;
  /** The selected row while the secondary bar has the keyboard. */
  list: number | null;
  repos: Repo[];
  tasks: { list: Task[] };
  picker: Picker | null;
  /** The quit question while it is up. */
  confirm: Confirm | null;
  keys: boolean;
  /** The terminal pane in visual mode. */
  visual: PaneId | null;
  ax_overlay: boolean;
}

export interface Terminal {
  cols: number;
  rows: number;
  /** The visible text, one string a row. */
  lines: string[];
  /** Lines scrolled back from the bottom. 0: the bottom. */
  display_offset: number;
  /** Lines of history kept above the screen. */
  history: number;
  selection: string | null;
  /** `[line, column]` in visual mode. */
  visual_cursor: [number, number] | null;
}

/** `[x, y, w, h]` in points. */
export type Rect = [number, number, number, number];

/** A rect in points, top-left origin. */
export interface Frame {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Truncated {
  depth: boolean;
  nodes: boolean;
  timeout: boolean;
}

/** What an accessibility label activates. */
export type AxSource =
  | { type: "app"; pane: PaneId; path: number[]; role: string }
  | { type: "own"; id: string };

export interface AxTarget {
  label: string;
  /** In ottospace window coordinates. */
  frame: Frame;
  source: AxSource;
}

/** Sneak mode's labels on ottospace's own controls. Every `source` is `own`. */
export interface Sneak {
  targets: AxTarget[];
  /** The letters typed so far: labels that start with them are on show. */
  typed: string;
}

export interface AxOverlay {
  pane: PaneId;
  targets: AxTarget[];
  count: number;
  truncated: Truncated;
  millis: number;
  error: string | null;
}

/** One task on the home screen's lattice. */
export interface HomeNode {
  task: TaskId;
  /** Its inbox heading: the node's colour. */
  group: Group;
  /** Its project's place, from 0 on the left. Tasks of no project share the last lane. */
  lane: number;
  /** Its rank by age among the shown tasks: 0 is the newest and the nearest. */
  depth: number;
  /** The lattice crossing `[x, z]` it is drawn on. */
  crossing: [number, number];
}

/** The home screen: what a tab with no pane shows. */
export interface Home {
  /** One for each of the first 12 inbox tasks that has room. */
  nodes: HomeNode[];
  /** The first tasks that need the user, at most 3. */
  needs_you: Task[];
  /** How many listed tasks each inbox heading has. */
  counts: Record<Group, number>;
}

/** What the home screen drew, in window points. */
export interface HomeDrawn {
  area: Rect;
  /** The new-task line with its list. */
  panel: Rect;
  /** Each node's click target, by task id. */
  nodes: Record<string, Rect>;
  /** How often lattice lines were built and painted. */
  lattice_builds: number;
  lattice_paints: number;
}

export interface AppEntry {
  name: string;
  /** How many windows it has open. */
  windows: number;
  icon: boolean;
}

/** The reply to most requests: the full state after the request. */
export interface Snapshot {
  ok: true;
  mode: Mode;
  state: Model;
  /** Where each pane of the active tab is drawn, by pane id. */
  bounds: Record<string, Rect>;
  /** The same panes, inside their borders. */
  frames: Record<string, Rect>;
  /** Each drawn pane's header, the handle a drag moves it by, and the × in it. */
  headers: Record<string, Rect>;
  closes: Record<string, Rect>;
  /** Each tab on the bar, by place. */
  tabs: Rect[];
  /** While a pane is dragged: where a drop would land, both null over anything else. */
  drag: { target: PaneId | null; zone: Zone | null } | null;
  /** Pane ids of each tab, in tab order. */
  tab_panes: PaneId[][];
  /** ottospace's own window frame. */
  window: Rect;
  /** Where parked windows go: `[x, y]`. */
  parking: [number, number];
  return_key: string | null;
  /** The build, as the status bar shows it: `0.1.0 · a4a2582`, or `0.1.0 · local`. */
  version: string;
  font: { name: string; size: number };
  /** One character cell: `[w, h]` in points. */
  cell: [number, number];
  /** The problem the status bar shows. Not an error of this request. */
  notice: string | null;
  /** Each hosted window's real frame, by pane id. */
  hosted: Record<string, Rect>;
  /** Each hosted pane's title-bar crop in points, by pane id. */
  crop: Record<string, number>;
  /** By pane id. */
  terminals: Record<string, Terminal>;
  /** The text of the last copy. */
  copied: string | null;
  inbox: InboxRow[];
  needs_you: number;
  /** Null while the tab on show has a pane. */
  home: Home | null;
  home_drawn: HomeDrawn;
  closed_apps: string[];
  apps: AppEntry[];
  ax_overlay: AxOverlay | null;
  /** Null unless `mode` is `sneak`. */
  sneak: Sneak | null;
}

export interface TaskReply {
  ok: true;
  task: Task;
}

export interface InboxReply {
  ok: true;
  inbox: InboxRow[];
}

export interface TaskPopReply {
  ok: true;
  /** Null: nothing needs the user. */
  task: Task | null;
  /** False: the view could not change. */
  opened: boolean;
}

export interface CaptureReply {
  ok: true;
  pane: PaneId;
  text: string;
}

export interface WaitReply {
  ok: true;
  pane: PaneId;
  /** The first line that matched. */
  line: string;
}

export interface AxNode {
  role: string;
  subrole?: string;
  title?: string;
  description?: string;
  /** Cut to 120 characters. */
  value?: string;
  /** Global screen points. */
  frame?: Frame;
  enabled?: boolean;
  actions?: string[];
  children?: AxNode[];
}

export interface AxTreeReply {
  ok: true;
  pane: PaneId;
  window: Frame;
  millis: number;
  tree: { root: AxNode; count: number; truncated: Truncated };
}

// Request arguments: the request without its `cmd`.

export interface SplitArgs {
  axis?: Axis;
  kind?: SplitKind;
}
export interface TypeArgs {
  /** Default: the focused pane. Name it: focus is shared with the user. */
  pane?: PaneId;
  text: string;
}
export interface PaneArg {
  pane: PaneId;
}
export interface OptionalPaneArg {
  pane?: PaneId;
}
/** Where on a pane another is dropped: an edge splits it there, the centre swaps the two. */
export type Zone = "left" | "right" | "top" | "bottom" | "center";
export interface MovePaneArgs {
  /** Default: the focused pane. */
  pane?: PaneId;
  /** The tab's place on the tab bar, from 0. */
  tab: number;
}
export interface DropPaneArgs {
  source: PaneId;
  target: PaneId;
  zone: Zone;
}
export interface OpenAppArgs {
  name: string;
  axis?: Axis;
}
export interface PickerArgs {
  axis?: Axis;
  new_tab?: boolean;
}
export interface CellArgs {
  pane?: PaneId;
  /** In cells from the pane's top-left. A fraction picks a side of a cell. */
  x: number;
  y: number;
}
export interface MouseDownArgs extends CellArgs {
  /** 2 selects a word, 3 a line. */
  clicks?: number;
}
export interface MouseArgs {
  action: "down" | "move" | "up";
  /** In points from the top-left of ottospace's window, as the state's rects are. */
  x: number;
  y: number;
  /** At most 3. */
  clicks?: number;
  modifiers?: Array<"cmd" | "ctrl" | "alt" | "shift">;
}
export interface DragArgs {
  from: [number, number];
  to: [number, number];
  /** Moves between the press and the release, 1 to 256. Default: 8. */
  steps?: number;
}
export interface ScrollArgs {
  pane?: PaneId;
  /** Positive scrolls back. */
  lines: number;
}
export interface OpenRepoArgs {
  /** A `path` from `state.repos`. */
  path: string;
  axis?: Axis;
}
export interface TaskAddArgs {
  title: string;
  /** 1 (most urgent) to 9. Default 3. */
  priority?: number;
  /** Default `captured`. */
  state?: TaskState;
  /** The `id` of an open tab of no task, which becomes the task's. */
  tab?: number;
}
export interface CaptureArgs {
  pane?: PaneId;
  /** Also everything scrolled off above the screen. */
  history?: boolean;
}
export interface WaitArgs {
  pane?: PaneId;
  /** A regular expression (Rust `regex` syntax), matched against each line alone. */
  pattern: string;
  /** Default 10000, at most 600000. */
  timeout_ms?: number;
}
export interface AxTreeArgs {
  pane?: PaneId;
  depth?: number;
  nodes?: number;
}
