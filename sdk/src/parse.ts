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
  Confirm,
  Frame,
  Group,
  Heading,
  Home,
  HomeDrawn,
  HomeNode,
  InboxReply,
  InboxRow,
  Layout,
  Mark,
  MarkInput,
  MarkReply,
  MarksReply,
  Mode,
  Model,
  Pane,
  PaneId,
  PaneKind,
  PickItem,
  Picker,
  Rect,
  Repo,
  Running,
  Section,
  SessionRow,
  SessionRunner,
  SessionsBar,
  Sneak,
  Snapshot,
  Tab,
  Task,
  TaskPopReply,
  TaskNewReply,
  TaskReply,
  TaskRow,
  TaskSection,
  TaskState,
  TasksBar,
  Terminal,
  Truncated,
  WaitReply,
  Worktree,
  Zone,
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
  if (value === "visual" || value === "list" || value === "compose" || value === "sneak") return value;
  if (value === "confirm" || value === "mark") return value;
  throw wrong(at, "a mode", value);
}

function section(value: unknown, at: string): Section {
  if (value === "apps" || value === "workbook" || value === "tasks" || value === "repos") return value;
  if (value === "marks") return value;
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

function heading(value: unknown, at: string): Heading {
  return value === "open" || value === "sessions" ? value : group(value, at);
}

function taskRow(value: unknown, at: string): TaskRow {
  const o = fields(value, at);
  return {
    id: num(o.id, `${at}.id`),
    group: group(o.group, `${at}.group`),
    tabs: num(o.tabs, `${at}.tabs`),
    panes: num(o.panes, `${at}.panes`),
    current: bool(o.current, `${at}.current`),
    detail: str(o.detail, `${at}.detail`),
  };
}

function taskSection(value: unknown, at: string): TaskSection {
  const o = fields(value, at);
  return {
    heading: heading(o.heading, `${at}.heading`),
    count: num(o.count, `${at}.count`),
    collapsed: bool(o.collapsed, `${at}.collapsed`),
    rows: list(taskRow)(o.rows, `${at}.rows`),
  };
}

function tasksBar(value: unknown, at: string): TasksBar {
  const o = fields(value, at);
  return {
    empty: bool(o.empty, `${at}.empty`),
    undo: orNull(str)(o.undo, `${at}.undo`),
    sections: list(taskSection)(o.sections, `${at}.sections`),
  };
}

function paneKind(value: unknown, at: string): PaneKind {
  const o = fields(value, at);
  if (o.type === "empty") return { type: "empty" };
  if (o.type === "terminal") return { type: "terminal" };
  if (o.type === "app") {
    const app: PaneKind = {
      type: "app",
      window: num(o.window, `${at}.window`),
      title: str(o.title, `${at}.title`),
      name: str(o.name, `${at}.name`),
    };
    // An app with no bundle (a bare binary) has no `bundle`.
    if (o.bundle !== undefined) app.bundle = str(o.bundle, `${at}.bundle`);
    return app;
  }
  throw wrong(`${at}.type`, "a pane kind", o.type);
}

function pane(value: unknown, at: string): Pane {
  const o = fields(value, at);
  const pane: Pane = { id: num(o.id, `${at}.id`), kind: paneKind(o.kind, `${at}.kind`) };
  if (o.cwd !== undefined) pane.cwd = str(o.cwd, `${at}.cwd`);
  if (o.fit !== undefined) {
    const fit = fields(o.fit, `${at}.fit`);
    pane.fit = { wants: pair(fit.wants, `${at}.fit.wants`), hint: str(fit.hint, `${at}.fit.hint`) };
    if (fit.stuck !== undefined) {
      if (fit.stuck !== "no_split" && fit.stuck !== "at_limit") throw wrong(`${at}.fit.stuck`, "a reason", fit.stuck);
      pane.fit.stuck = fit.stuck;
    }
  }
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
      ratio: num(split.ratio, `${at}.split.ratio`),
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
    agent: orNull(str)(o.agent, `${at}.agent`),
    paths: list(str)(o.paths, `${at}.paths`),
    resolved: list(str)(o.resolved, `${at}.resolved`),
    skills: list(str)(o.skills, `${at}.skills`),
    created: orNull(num)(o.created, `${at}.created`),
    session: orNull(sessionLink)(o.session, `${at}.session`),
  };
}

function sessionLink(value: unknown, at: string): { runner: string; id: string } {
  const o = fields(value, at);
  return { runner: str(o.runner, `${at}.runner`), id: str(o.id, `${at}.id`) };
}

function sessionRunner(value: unknown, at: string): SessionRunner {
  if (value === "claude" || value === "codex") return value;
  throw wrong(at, "a session runner", value);
}

function sessionRow(value: unknown, at: string): SessionRow {
  const o = fields(value, at);
  return {
    runner: sessionRunner(o.runner, `${at}.runner`),
    id: str(o.id, `${at}.id`),
    name: str(o.name, `${at}.name`),
    active: bool(o.active, `${at}.active`),
    doing: orNull(str)(o.doing, `${at}.doing`),
    age: orNull(str)(o.age, `${at}.age`),
    pane: orNull(num)(o.pane, `${at}.pane`),
  };
}

function sessionsBar(value: unknown, at: string): SessionsBar {
  const o = fields(value, at);
  return {
    count: num(o.count, `${at}.count`),
    collapsed: bool(o.collapsed, `${at}.collapsed`),
    rows: list(sessionRow)(o.rows, `${at}.rows`),
    older: num(o.older, `${at}.older`),
    older_open: bool(o.older_open, `${at}.older_open`),
    enabled: bool(o.enabled, `${at}.enabled`),
    reads: num(o.reads, `${at}.reads`),
    slow_reads: num(o.slow_reads, `${at}.slow_reads`),
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

function running(value: unknown, at: string): Running {
  const o = fields(value, at);
  return {
    pane: num(o.pane, `${at}.pane`),
    program: orNull(str)(o.program, `${at}.program`),
    background: bool(o.background, `${at}.background`),
  };
}

function confirm(value: unknown, at: string): Confirm {
  return { running: list(running)(fields(value, at).running, `${at}.running`) };
}

function markInput(value: unknown, at: string): MarkInput {
  const o = fields(value, at);
  if (o.type === "jump") return { type: "jump" };
  if (o.type === "rename") return { type: "rename", mark: num(o.mark, `${at}.mark`), text: str(o.text, `${at}.text`) };
  if (o.type === "set_key") return { type: "set_key", mark: num(o.mark, `${at}.mark`) };
  throw wrong(`${at}.type`, "what a mark waits for", o.type);
}

function markKind(value: unknown, at: string): "terminal" | "app" | "tab" {
  if (value === "terminal" || value === "app" || value === "tab") return value;
  throw wrong(at, "a mark's kind", value);
}

function mark(value: unknown, at: string): Mark {
  const o = fields(value, at);
  const buttons = fields(o.buttons, `${at}.buttons`);
  return {
    id: num(o.id, `${at}.id`),
    name: str(o.name, `${at}.name`),
    key: orNull(str)(o.key, `${at}.key`),
    kind: markKind(o.kind, `${at}.kind`),
    open: bool(o.open, `${at}.open`),
    pane: orNull(num)(o.pane, `${at}.pane`),
    tab: orNull(num)(o.tab, `${at}.tab`),
    available: bool(o.available, `${at}.available`),
    reason: orNull(str)(o.reason, `${at}.reason`),
    unfilled: list((value, at) => {
      const empty = fields(value, at);
      return { pane: num(empty.pane, `${at}.pane`), why: str(empty.why, `${at}.why`) };
    })(o.unfilled, `${at}.unfilled`),
    row: orNull(rect)(o.row, `${at}.row`),
    buttons: {
      key: orNull(rect)(buttons.key, `${at}.buttons.key`),
      rename: orNull(rect)(buttons.rename, `${at}.buttons.rename`),
      remove: orNull(rect)(buttons.remove, `${at}.buttons.remove`),
    },
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
    collapsed: list(heading)(o.collapsed, `${at}.collapsed`),
    picker: orNull(picker)(o.picker, `${at}.picker`),
    confirm: orNull(confirm)(o.confirm, `${at}.confirm`),
    keys: bool(o.keys, `${at}.keys`),
    visual: orNull(num)(o.visual, `${at}.visual`),
    ax_overlay: bool(o.ax_overlay, `${at}.ax_overlay`),
    mark_input: orNull(markInput)(o.mark_input, `${at}.mark_input`),
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

function sneak(value: unknown, at: string): Sneak {
  const o = fields(value, at);
  return { targets: list(axTarget)(o.targets, `${at}.targets`), typed: str(o.typed, `${at}.typed`) };
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

function homeNode(value: unknown, at: string): HomeNode {
  const o = fields(value, at);
  return {
    task: num(o.task, `${at}.task`),
    group: group(o.group, `${at}.group`),
    lane: num(o.lane, `${at}.lane`),
    depth: num(o.depth, `${at}.depth`),
    crossing: pair(o.crossing, `${at}.crossing`),
  };
}

function home(value: unknown, at: string): Home {
  const o = fields(value, at);
  const counts = fields(o.counts, `${at}.counts`);
  return {
    nodes: list(homeNode)(o.nodes, `${at}.nodes`),
    needs_you: list(task)(o.needs_you, `${at}.needs_you`),
    counts: {
      needs_you: num(counts.needs_you, `${at}.counts.needs_you`),
      running: num(counts.running, `${at}.counts.running`),
      captured: num(counts.captured, `${at}.counts.captured`),
      on_hold: num(counts.on_hold, `${at}.counts.on_hold`),
      done: num(counts.done, `${at}.counts.done`),
    },
  };
}

function homeDrawn(value: unknown, at: string): HomeDrawn {
  const o = fields(value, at);
  return {
    area: rect(o.area, `${at}.area`),
    panel: rect(o.panel, `${at}.panel`),
    nodes: map(rect)(o.nodes, `${at}.nodes`),
    lattice_builds: num(o.lattice_builds, `${at}.lattice_builds`),
    lattice_paints: num(o.lattice_paints, `${at}.lattice_paints`),
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

function zone(value: unknown, at: string): Zone {
  if (value === "left" || value === "right" || value === "top" || value === "bottom" || value === "center") return value;
  throw wrong(at, "a zone", value);
}

function drag(value: unknown, at: string): { target: PaneId | null; zone: Zone | null } {
  const o = fields(value, at);
  return { target: orNull(num)(o.target, `${at}.target`), zone: orNull(zone)(o.zone, `${at}.zone`) };
}

export function snapshot(value: unknown): Snapshot {
  const o = fields(value, "reply");
  return {
    ok: true,
    mode: mode(o.mode, "mode"),
    state: model(o.state, "state"),
    bounds: map(rect)(o.bounds, "bounds"),
    frames: map(rect)(o.frames, "frames"),
    headers: map(rect)(o.headers, "headers"),
    closes: map(rect)(o.closes, "closes"),
    tabs: list(rect)(o.tabs, "tabs"),
    drag: orNull(drag)(o.drag, "drag"),
    shown: map(rect)(o.shown, "shown"),
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
    compose_read: bool(o.compose_read, "compose_read"),
    tasks_bar: tasksBar(o.tasks_bar, "tasks_bar"),
    sessions_bar: sessionsBar(o.sessions_bar, "sessions_bar"),
    home: orNull(home)(o.home, "home"),
    home_drawn: homeDrawn(o.home_drawn, "home_drawn"),
    closed_apps: list(str)(o.closed_apps, "closed_apps"),
    apps: list(appEntry)(o.apps, "apps"),
    ax_overlay: orNull(axOverlay)(o.ax_overlay, "ax_overlay"),
    sneak: orNull(sneak)(o.sneak, "sneak"),
    counts: orNull(map(num))(o.counts, "counts"),
    marks: list(mark)(o.marks, "marks"),
    mark_removed: orNull(str)(o.mark_removed, "mark_removed"),
  };
}

export function taskNewReply(value: unknown): TaskNewReply {
  const opened = fields(value, "reply").opened;
  return { ...taskReply(value), ...(opened === undefined ? {} : { opened: bool(opened, "opened") }) };
}

export function marksReply(value: unknown): MarksReply {
  return { ok: true, marks: list(mark)(fields(value, "reply").marks, "marks") };
}

export function markReply(value: unknown): MarkReply {
  return { ok: true, mark: mark(fields(value, "reply").mark, "mark") };
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
