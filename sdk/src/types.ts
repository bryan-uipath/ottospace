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
export type Mode = "normal" | "app" | "pick" | "visual" | "list" | "compose";

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

/** The active tab's split tree. */
export type Layout = { leaf: PaneId } | { split: { axis: Axis; a: Layout; b: Layout } };

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

/** `source` is flattened in: its `type` and fields sit beside `label` and `frame`. */
export type AxTarget = { label: string; frame: Frame } & AxSource;

export interface AxOverlay {
  pane: PaneId;
  targets: AxTarget[];
  count: number;
  truncated: Truncated;
  millis: number;
  error: string | null;
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
  /** Pane ids of each tab, in tab order. */
  tab_panes: PaneId[][];
  /** ottospace's own window frame. */
  window: Rect;
  /** Where parked windows go: `[x, y]`. */
  parking: [number, number];
  return_key: string | null;
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
  closed_apps: string[];
  apps: AppEntry[];
  ax_overlay: AxOverlay | null;
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
