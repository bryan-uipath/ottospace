// Turns a reply (`unknown` from JSON.parse) into the types in `types.ts` with runtime checks:
// no casts, so a reply the types do not describe is an error here, not a wrong value later.
// Keys the types do not name are dropped, so a test can compare the result with the raw reply.

import type {
  AppEntry,
  AxNode,
  AxOverlay,
  AxSource,
  AxTarget,
  AxTreeReply,
  Axis,
  CaptureReply,
  Frame,
  Group,
  InboxReply,
  InboxRow,
  Layout,
  Mode,
  Model,
  Pane,
  PaneKind,
  PickItem,
  Picker,
  Rect,
  Repo,
  Section,
  Snapshot,
  Tab,
  Task,
  TaskPopReply,
  TaskReply,
  TaskState,
  Terminal,
  Truncated,
  WaitReply,
  Worktree,
} from "./types.ts";

type Fields = Record<string, unknown>;
type Parse<T> = (value: unknown, at: string) => T;

function wrong(at: string, expected: string, value: unknown): Error {
  return new Error(`ottospace reply: ${at} should be ${expected}, got ${JSON.stringify(value)}`);
}

function isFields(value: unknown): value is Fields {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function fields(value: unknown, at: string): Fields {
  if (!isFields(value)) throw wrong(at, "an object", value);
  return value;
}

function str(value: unknown, at: string): string {
  if (typeof value !== "string") throw wrong(at, "a string", value);
  return value;
}

function num(value: unknown, at: string): number {
  if (typeof value !== "number") throw wrong(at, "a number", value);
  return value;
}

function bool(value: unknown, at: string): boolean {
  if (typeof value !== "boolean") throw wrong(at, "true or false", value);
  return value;
}

function orNull<T>(parse: Parse<T>): Parse<T | null> {
  return (value, at) => (value === null ? null : parse(value, at));
}

function list<T>(parse: Parse<T>): Parse<T[]> {
  return (value, at) => {
    if (!Array.isArray(value)) throw wrong(at, "an array", value);
    return value.map((item: unknown, i) => parse(item, `${at}[${i}]`));
  };
}

/** An object used as a map, e.g. `terminals` by pane id. */
function map<T>(parse: Parse<T>): Parse<Record<string, T>> {
  return (value, at) => {
    const out: Record<string, T> = {};
    for (const [key, item] of Object.entries(fields(value, at))) out[key] = parse(item, `${at}.${key}`);
    return out;
  };
}

function pair(value: unknown, at: string): [number, number] {
  if (!Array.isArray(value) || value.length !== 2) throw wrong(at, "two numbers", value);
  return [num(value[0], at), num(value[1], at)];
}

function rect(value: unknown, at: string): Rect {
  if (!Array.isArray(value) || value.length !== 4) throw wrong(at, "[x, y, w, h]", value);
  return [num(value[0], at), num(value[1], at), num(value[2], at), num(value[3], at)];
}

function axis(value: unknown, at: string): Axis {
  if (value === "right" || value === "down") return value;
  throw wrong(at, "an axis", value);
}

function mode(value: unknown, at: string): Mode {
  if (value === "normal" || value === "app" || value === "pick") return value;
  if (value === "visual" || value === "list" || value === "compose") return value;
  throw wrong(at, "a mode", value);
}

function section(value: unknown, at: string): Section {
  if (value === "apps" || value === "workbook" || value === "tasks" || value === "repos") return value;
  throw wrong(at, "a section", value);
}

function taskState(value: unknown, at: string): TaskState {
  if (value === "captured" || value === "triaged" || value === "running") return value;
  if (value === "needs_you" || value === "review" || value === "stale") return value;
  if (value === "parked" || value === "done" || value === "cleaned") return value;
  throw wrong(at, "a task state", value);
}

function group(value: unknown, at: string): Group {
  if (value === "needs_you" || value === "running" || value === "captured") return value;
  if (value === "on_hold" || value === "done") return value;
  throw wrong(at, "an inbox group", value);
}

function paneKind(value: unknown, at: string): PaneKind {
  const o = fields(value, at);
  if (o.type === "empty") return { type: "empty" };
  if (o.type === "terminal") return { type: "terminal" };
  if (o.type === "app") {
    return { type: "app", window: num(o.window, `${at}.window`), title: str(o.title, `${at}.title`) };
  }
  throw wrong(`${at}.type`, "a pane kind", o.type);
}

function pane(value: unknown, at: string): Pane {
  const o = fields(value, at);
  const pane: Pane = { id: num(o.id, `${at}.id`), kind: paneKind(o.kind, `${at}.kind`) };
  if (o.cwd !== undefined) pane.cwd = str(o.cwd, `${at}.cwd`);
  return pane;
}

function tab(value: unknown, at: string): Tab {
  const o = fields(value, at);
  return { id: num(o.id, `${at}.id`), title: str(o.title, `${at}.title`), task: orNull(num)(o.task, `${at}.task`) };
}

function layout(value: unknown, at: string): Layout {
  const o = fields(value, at);
  if (o.leaf !== undefined) return { leaf: num(o.leaf, `${at}.leaf`) };
  const split = fields(o.split, `${at}.split`);
  return {
    split: {
      axis: axis(split.axis, `${at}.split.axis`),
      a: layout(split.a, `${at}.split.a`),
      b: layout(split.b, `${at}.split.b`),
    },
  };
}

function task(value: unknown, at: string): Task {
  const o = fields(value, at);
  return {
    id: num(o.id, `${at}.id`),
    title: str(o.title, `${at}.title`),
    state: taskState(o.state, `${at}.state`),
    priority: num(o.priority, `${at}.priority`),
    snoozed_until: orNull(num)(o.snoozed_until, `${at}.snoozed_until`),
    dismissed: bool(o.dismissed, `${at}.dismissed`),
    project: orNull(str)(o.project, `${at}.project`),
    repo: orNull(str)(o.repo, `${at}.repo`),
  };
}

function inboxRow(value: unknown, at: string): InboxRow {
  return { ...task(value, at), group: group(fields(value, at).group, `${at}.group`) };
}

function worktree(value: unknown, at: string): Worktree {
  const o = fields(value, at);
  return {
    name: str(o.name, `${at}.name`),
    path: str(o.path, `${at}.path`),
    branch: orNull(str)(o.branch, `${at}.branch`),
  };
}

function repo(value: unknown, at: string): Repo {
  return { ...worktree(value, at), worktrees: list(worktree)(fields(value, at).worktrees, `${at}.worktrees`) };
}

function pickItem(value: unknown, at: string): PickItem {
  const o = fields(value, at);
  if (o.type === "terminal") return { type: "terminal" };
  if (o.type === "app") return { type: "app", name: str(o.name, `${at}.name`) };
  throw wrong(`${at}.type`, "a picker item", o.type);
}

function picker(value: unknown, at: string): Picker {
  const o = fields(value, at);
  return {
    axis: axis(o.axis, `${at}.axis`),
    new_tab: bool(o.new_tab, `${at}.new_tab`),
    items: list(pickItem)(o.items, `${at}.items`),
    query: str(o.query, `${at}.query`),
    selected: num(o.selected, `${at}.selected`),
  };
}

function model(value: unknown, at: string): Model {
  const o = fields(value, at);
  return {
    panes: list(pane)(o.panes, `${at}.panes`),
    tabs: list(tab)(o.tabs, `${at}.tabs`),
    active: num(o.active, `${at}.active`),
    layout: orNull(layout)(o.layout, `${at}.layout`),
    focused: orNull(num)(o.focused, `${at}.focused`),
    live: orNull(num)(o.live, `${at}.live`),
    current_task: orNull(num)(o.current_task, `${at}.current_task`),
    sidebar: orNull(section)(o.sidebar, `${at}.sidebar`),
    compose: orNull(str)(o.compose, `${at}.compose`),
    list: orNull(num)(o.list, `${at}.list`),
    repos: list(repo)(o.repos, `${at}.repos`),
    tasks: { list: list(task)(fields(o.tasks, `${at}.tasks`).list, `${at}.tasks.list`) },
    picker: orNull(picker)(o.picker, `${at}.picker`),
    keys: bool(o.keys, `${at}.keys`),
    visual: orNull(num)(o.visual, `${at}.visual`),
    ax_overlay: bool(o.ax_overlay, `${at}.ax_overlay`),
  };
}

function terminal(value: unknown, at: string): Terminal {
  const o = fields(value, at);
  return {
    cols: num(o.cols, `${at}.cols`),
    rows: num(o.rows, `${at}.rows`),
    lines: list(str)(o.lines, `${at}.lines`),
    display_offset: num(o.display_offset, `${at}.display_offset`),
    history: num(o.history, `${at}.history`),
    selection: orNull(str)(o.selection, `${at}.selection`),
    visual_cursor: orNull(pair)(o.visual_cursor, `${at}.visual_cursor`),
  };
}

function frame(value: unknown, at: string): Frame {
  const o = fields(value, at);
  return { x: num(o.x, `${at}.x`), y: num(o.y, `${at}.y`), w: num(o.w, `${at}.w`), h: num(o.h, `${at}.h`) };
}

function truncated(value: unknown, at: string): Truncated {
  const o = fields(value, at);
  return {
    depth: bool(o.depth, `${at}.depth`),
    nodes: bool(o.nodes, `${at}.nodes`),
    timeout: bool(o.timeout, `${at}.timeout`),
  };
}

function axSource(value: unknown, at: string): AxSource {
  const o = fields(value, at);
  if (o.type === "own") return { type: "own", id: str(o.id, `${at}.id`) };
  if (o.type === "app") {
    return {
      type: "app",
      pane: num(o.pane, `${at}.pane`),
      path: list(num)(o.path, `${at}.path`),
      role: str(o.role, `${at}.role`),
    };
  }
  throw wrong(`${at}.type`, "a label source", o.type);
}

function axTarget(value: unknown, at: string): AxTarget {
  const o = fields(value, at);
  return {
    label: str(o.label, `${at}.label`),
    frame: frame(o.frame, `${at}.frame`),
    source: axSource(o.source, `${at}.source`),
  };
}

function axOverlay(value: unknown, at: string): AxOverlay {
  const o = fields(value, at);
  return {
    pane: num(o.pane, `${at}.pane`),
    targets: list(axTarget)(o.targets, `${at}.targets`),
    count: num(o.count, `${at}.count`),
    truncated: truncated(o.truncated, `${at}.truncated`),
    millis: num(o.millis, `${at}.millis`),
    error: orNull(str)(o.error, `${at}.error`),
  };
}

function appEntry(value: unknown, at: string): AppEntry {
  const o = fields(value, at);
  return { name: str(o.name, `${at}.name`), windows: num(o.windows, `${at}.windows`), icon: bool(o.icon, `${at}.icon`) };
}

/** The app leaves out what a node does not have. */
function axNode(value: unknown, at: string): AxNode {
  const o = fields(value, at);
  const node: AxNode = { role: str(o.role, `${at}.role`) };
  if (o.subrole !== undefined) node.subrole = str(o.subrole, `${at}.subrole`);
  if (o.title !== undefined) node.title = str(o.title, `${at}.title`);
  if (o.description !== undefined) node.description = str(o.description, `${at}.description`);
  if (o.value !== undefined) node.value = str(o.value, `${at}.value`);
  if (o.frame !== undefined) node.frame = frame(o.frame, `${at}.frame`);
  if (o.enabled !== undefined) node.enabled = bool(o.enabled, `${at}.enabled`);
  if (o.actions !== undefined) node.actions = list(str)(o.actions, `${at}.actions`);
  if (o.children !== undefined) node.children = list(axNode)(o.children, `${at}.children`);
  return node;
}

export function snapshot(value: unknown): Snapshot {
  const o = fields(value, "reply");
  return {
    ok: true,
    mode: mode(o.mode, "mode"),
    state: model(o.state, "state"),
    bounds: map(rect)(o.bounds, "bounds"),
    frames: map(rect)(o.frames, "frames"),
    tab_panes: list(list(num))(o.tab_panes, "tab_panes"),
    window: rect(o.window, "window"),
    parking: pair(o.parking, "parking"),
    return_key: orNull(str)(o.return_key, "return_key"),
    version: str(o.version, "version"),
    font: { name: str(fields(o.font, "font").name, "font.name"), size: num(fields(o.font, "font").size, "font.size") },
    cell: pair(o.cell, "cell"),
    notice: orNull(str)(o.notice, "notice"),
    hosted: map(rect)(o.hosted, "hosted"),
    crop: map(num)(o.crop, "crop"),
    terminals: map(terminal)(o.terminals, "terminals"),
    copied: orNull(str)(o.copied, "copied"),
    inbox: list(inboxRow)(o.inbox, "inbox"),
    needs_you: num(o.needs_you, "needs_you"),
    closed_apps: list(str)(o.closed_apps, "closed_apps"),
    apps: list(appEntry)(o.apps, "apps"),
    ax_overlay: orNull(axOverlay)(o.ax_overlay, "ax_overlay"),
  };
}

export function taskReply(value: unknown): TaskReply {
  return { ok: true, task: task(fields(value, "reply").task, "task") };
}

export function inboxReply(value: unknown): InboxReply {
  return { ok: true, inbox: list(inboxRow)(fields(value, "reply").inbox, "inbox") };
}

export function taskPopReply(value: unknown): TaskPopReply {
  const o = fields(value, "reply");
  return { ok: true, task: orNull(task)(o.task, "task"), opened: bool(o.opened, "opened") };
}

export function captureReply(value: unknown): CaptureReply {
  const o = fields(value, "reply");
  return { ok: true, pane: num(o.pane, "pane"), text: str(o.text, "text") };
}

export function waitReply(value: unknown): WaitReply {
  const o = fields(value, "reply");
  return { ok: true, pane: num(o.pane, "pane"), line: str(o.line, "line") };
}

export function axTreeReply(value: unknown): AxTreeReply {
  const o = fields(value, "reply");
  const tree = fields(o.tree, "tree");
  return {
    ok: true,
    pane: num(o.pane, "pane"),
    window: frame(o.window, "window"),
    millis: num(o.millis, "millis"),
    tree: {
      root: axNode(tree.root, "tree.root"),
      count: num(tree.count, "tree.count"),
      truncated: truncated(tree.truncated, "tree.truncated"),
    },
  };
}

/** The message of a refused request (`{"ok":false,"error":".."}`), or null for any other reply. */
export function refusal(value: unknown): string | null {
  if (!isFields(value) || value.ok !== false) return null;
  return typeof value.error === "string" ? value.error : "the request was refused";
}
