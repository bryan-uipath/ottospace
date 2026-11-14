// Turns a reply (`unknown` from JSON.parse) into the types in `types.ts` with runtime checks:
// no casts, so a reply the types do not describe is an error here, not a wrong value later.
// Keys the types do not name are dropped, so a test can compare the result with the raw reply.

import type {
  AppEntry,
  NativeChat,
  MeetingStage,
  AxNode,
  AxOverlay,
  AxSource,
  AxTarget,
  AxTreeReply,
  Axis,
  Brief,
  BriefMaterial,
  CapturedNode,
  CaptureReply,
  CaptureSource,
  Draft,
  Material,
  Shape,
  Subject,
  TaskBriefReply,
  Confirm,
  ComposeMenu,
  Field,
  MenuRow,
  Ref,
  Frame,
  Group,
  Heading,
  Card,
  ProjectGroup,
  TasksProgress,
  TasksView,
  SessionsRange,
  Tokens,
  Work,
  Focus,
  RailFocus,
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
  TraceReply,
  Workbook,
  WorkbookPane,
  WorkbookTab,
  Mode,
  Model,
  Pane,
  PaneId,
  PaneKind,
  PickItem,
  Picker,
  Palette,
  PaletteRow,
  PaletteTask,
  Inspector,
  InspectorRow,
  InspectorState,
  Rect,
  Repo,
  Running,
  Scrollbar,
  Section,
  SessionRow,
  SessionRunner,
  SessionsBar,
  ReposGroup,
  RepoDirs,
  CheckoutRow,
  AppsBar,
  ClosedApp,
  Sneak,
  SneakApp,
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
  if (value === "normal" || value === "app" || value === "pick" || value === "palette") return value;
  if (value === "inspect") return value;
  if (value === "visual" || value === "list" || value === "compose" || value === "sneak") return value;
  if (value === "confirm" || value === "mark" || value === "annotate" || value === "talk") return value;
  throw wrong(at, "a mode", value);
}

function section(value: unknown, at: string): Section {
  if (value === "apps" || value === "workbook" || value === "tasks" || value === "repos") return value;
  if (value === "marks" || value === "calendar") return value;
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

function focus(value: unknown, at: string): Focus {
  if (value === "drawer" || value === "inspector" || value === "nothing") return value;
  const o = fields(value, at);
  if (o.pane !== undefined) return { pane: num(o.pane, `${at}.pane`) };
  if (o.list !== undefined) return { list: num(o.list, `${at}.list`) };
  if (o.rail !== undefined) return { rail: section(o.rail, `${at}.rail`) };
  return { right: num(o.right, `${at}.right`) };
}

function railFocus(value: unknown, at: string): RailFocus {
  const o = fields(value, at);
  if (o.left !== undefined) return { left: section(o.left, `${at}.left`) };
  return { right: num(o.right, `${at}.right`) };
}

function heading(value: unknown, at: string): Heading {
  if (value === "needs_you" || value === "running" || value === "later") return value;
  if (value === "done" || value === "sessions") return value;
  throw wrong(at, "a heading", value);
}

function tasksView(value: unknown, at: string): TasksView {
  if (value === "state" || value === "project") return value;
  throw wrong(at, "state or project", value);
}

function sessionsRange(value: unknown, at: string): SessionsRange {
  if (value === "today" || value === "week" || value === "month" || value === "all") return value;
  throw wrong(at, "today, week, month or all", value);
}

function tokens(value: unknown, at: string): Tokens {
  const o = fields(value, at);
  return {
    model: str(o.model, `${at}.model`),
    sub: bool(o.sub, `${at}.sub`),
    input: num(o.input, `${at}.input`),
    output: num(o.output, `${at}.output`),
    cached: num(o.cached, `${at}.cached`),
  };
}

function work(value: unknown, at: string): Work {
  const o = fields(value, at);
  const c = fields(o.clock, `${at}.clock`);
  return {
    runner: str(o.runner, `${at}.runner`),
    session: str(o.session, `${at}.session`),
    clock: {
      ms: num(c.ms, `${at}.clock.ms`),
      last: orNull(num)(c.last, `${at}.clock.last`),
      day: num(c.day, `${at}.clock.day`),
      day_ms: num(c.day_ms, `${at}.clock.day_ms`),
    },
    tokens: list(tokens)(o.tokens, `${at}.tokens`),
    context: orNull(num)(o.context, `${at}.context`),
    window: orNull(num)(o.window, `${at}.window`),
  };
}

function projectGroup(value: unknown, at: string): ProjectGroup {
  const o = fields(value, at);
  const b = fields(o.bar, `${at}.bar`);
  return {
    project: orNull(str)(o.project, `${at}.project`),
    folded: bool(o.folded, `${at}.folded`),
    bar: {
      needs_you: num(b.needs_you, `${at}.bar.needs_you`),
      running: num(b.running, `${at}.bar.running`),
      later: num(b.later, `${at}.bar.later`),
      done: num(b.done, `${at}.bar.done`),
    },
    rows: list(taskRow)(o.rows, `${at}.rows`),
    done: num(o.done, `${at}.done`),
    done_open: bool(o.done_open, `${at}.done_open`),
    done_rows: list(taskRow)(o.done_rows, `${at}.done_rows`),
  };
}

function card(value: unknown, at: string): Card {
  const o = fields(value, at);
  return {
    id: num(o.id, `${at}.id`),
    title: str(o.title, `${at}.title`),
    state: str(o.state, `${at}.state`),
    group: orNull(group)(o.group, `${at}.group`),
    live: orNull(str)(o.live, `${at}.live`),
    model: orNull(str)(o.model, `${at}.model`),
    tags: list(str)(o.tags, `${at}.tags`),
    started: orNull(str)(o.started, `${at}.started`),
    agent_time: str(o.agent_time, `${at}.agent_time`),
    workspace: str(o.workspace, `${at}.workspace`),
    tokens: list(tokens)(o.tokens, `${at}.tokens`),
    context: orNull(num)(o.context, `${at}.context`),
    window: orNull(num)(o.window, `${at}.window`),
  };
}

function tasksProgress(value: unknown, at: string): TasksProgress {
  const o = fields(value, at);
  const t = fields(o.today, `${at}.today`);
  return {
    view: tasksView(o.view, `${at}.view`),
    projects: list(projectGroup)(o.projects, `${at}.projects`),
    today: {
      done: num(t.done, `${at}.today.done`),
      running: num(t.running, `${at}.today.running`),
      agent_ms: num(t.agent_ms, `${at}.today.agent_ms`),
      line: str(t.line, `${at}.today.line`),
    },
    cards: map(card)(o.cards, `${at}.cards`),
  };
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
    live: orNull(str)(o.live, `${at}.live`),
    model: orNull(str)(o.model, `${at}.model`),
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
    refs: list(ref)(o.refs, `${at}.refs`),
    title_generated: bool(o.title_generated, `${at}.title_generated`),
    materials: list(str)(o.materials, `${at}.materials`),
    // Written only when known.
    ...(o.started_at === undefined ? {} : { started_at: num(o.started_at, `${at}.started_at`) }),
    ...(o.finished_at === undefined ? {} : { finished_at: num(o.finished_at, `${at}.finished_at`) }),
    ...(o.work === undefined ? {} : { work: list(work)(o.work, `${at}.work`) }),
    ...(o.colour === undefined ? {} : { colour: num(o.colour, `${at}.colour`) }),
  };
}

function point(value: unknown, at: string): [number, number] {
  return pair(value, at);
}

function shape(value: unknown, at: string): Shape {
  const o = fields(value, at);
  if (o.kind === "ellipse" || o.kind === "box") return { kind: o.kind, rect: rect(o.rect, `${at}.rect`) };
  if (o.kind === "stroke") return { kind: "stroke", points: list(point)(o.points, `${at}.points`) };
  if (o.kind === "note") return { kind: "note", at: point(o.at, `${at}.at`), text: str(o.text, `${at}.text`) };
  throw wrong(`${at}.kind`, "a shape", o.kind);
}

function captureSource(value: unknown, at: string): CaptureSource {
  const o = fields(value, at);
  if (o.type === "terminal") {
    return { type: "terminal", cwd: orNull(str)(o.cwd, `${at}.cwd`), program: orNull(str)(o.program, `${at}.program`) };
  }
  if (o.type === "app") {
    const bundle = orNull(str)(o.bundle, `${at}.bundle`);
    return { type: "app", name: str(o.name, `${at}.name`), bundle, title: str(o.title, `${at}.title`) };
  }
  throw wrong(`${at}.type`, "a capture's source", o.type);
}

function material(value: unknown, at: string): Material {
  const o = fields(value, at);
  const marks = fields(o.marks, `${at}.marks`);
  const tool = marks.tool;
  if (tool !== "ellipse" && tool !== "box" && tool !== "stroke") throw wrong(`${at}.marks.tool`, "a tool", tool);
  const content = o.content;
  if (content !== null && content !== "app" && content !== "terminal" && content !== "missing") {
    throw wrong(`${at}.content`, "a kind of content", content);
  }
  const note = (value: unknown, at: string): [[number, number], string] => {
    if (!Array.isArray(value) || value.length !== 2) throw wrong(at, "a point and its text", value);
    return [point(value[0], `${at}[0]`), str(value[1], `${at}[1]`)];
  };
  return {
    n: num(o.n, `${at}.n`),
    pane: num(o.pane, `${at}.pane`),
    source: captureSource(o.source, `${at}.source`),
    marks: {
      shapes: list(shape)(marks.shapes, `${at}.marks.shapes`),
      selected: orNull(num)(marks.selected, `${at}.marks.selected`),
      tool,
      note: orNull(note)(marks.note, `${at}.marks.note`),
    },
    content,
    picture: orNull(bool)(o.picture, `${at}.picture`),
    stale: bool(o.stale, `${at}.stale`),
    at: num(o.at, `${at}.at`),
  };
}

function draft(value: unknown, at: string): Draft {
  const o = fields(value, at);
  return {
    draft: str(o.draft, `${at}.draft`),
    typing: bool(o.typing, `${at}.typing`),
    materials: list(material)(o.materials, `${at}.materials`),
    annotating: orNull(num)(o.annotating, `${at}.annotating`),
    asked: bool(o.asked, `${at}.asked`),
    images: bool(o.images, `${at}.images`),
  };
}

function captureDrawn(value: unknown, at: string): Snapshot["capture"] {
  const o = fields(value, at);
  const shown = (value: unknown, at: string) => {
    const b = fields(value, at);
    return { task: num(b.task, `${at}.task`), dir: orNull(str)(b.dir, `${at}.dir`) };
  };
  return {
    key: orNull(str)(o.key, `${at}.key`),
    pending: num(o.pending, `${at}.pending`),
    bar: orNull(rect)(o.bar, `${at}.bar`),
    picture: orNull(rect)(o.picture, `${at}.picture`),
    encoded: list(num)(o.encoded, `${at}.encoded`),
    dir: orNull(str)(o.dir, `${at}.dir`),
    brief: orNull(shown)(o.brief, `${at}.brief`),
  };
}

/** The label every captured thing carries: without it the content is not handed on. */
function labelled(o: Fields, at: string): { untrusted: true; warning: string } {
  if (o.untrusted !== true) throw wrong(`${at}.untrusted`, "true", o.untrusted);
  return { untrusted: true, warning: str(o.warning, `${at}.warning`) };
}

function capturedNode(value: unknown, at: string): CapturedNode {
  const o = fields(value, at);
  return {
    role: str(o.role, `${at}.role`),
    text: str(o.text, `${at}.text`),
    frame: rect(o.frame, `${at}.frame`),
    path: list(num)(o.path, `${at}.path`),
    context: list(str)(o.context ?? [], `${at}.context`),
  };
}

function briefMaterial(value: unknown, at: string): BriefMaterial {
  const o = fields(value, at);
  const c = fields(o.captured, `${at}.captured`);
  const under = (value: unknown, at: string) => {
    const u = fields(value, at);
    return {
      annotation: num(u.annotation, `${at}.annotation`),
      nodes: list(capturedNode)(u.nodes ?? [], `${at}.nodes`),
      lines: list(str)(u.lines ?? [], `${at}.lines`),
    };
  };
  return {
    image: orNull(str)(o.image, `${at}.image`),
    annotated: orNull(str)(o.annotated, `${at}.annotated`),
    n: num(o.n, `${at}.n`),
    stale: bool(o.stale ?? false, `${at}.stale`),
    annotations: list(shape)(o.annotations, `${at}.annotations`),
    captured: {
      ...labelled(c, `${at}.captured`),
      source: captureSource(c.source, `${at}.captured.source`),
      under: list(under)(c.under, `${at}.captured.under`),
      cut: bool(c.cut, `${at}.captured.cut`),
      missing: orNull(str)(c.missing, `${at}.captured.missing`),
    },
    at: num(o.at, `${at}.at`),
  };
}

function brief(value: unknown, at: string): Brief {
  const o = fields(value, at);
  const ask = fields(o.ask, `${at}.ask`);
  const open = fields(o.open_windows, `${at}.open_windows`);
  return {
    task: num(o.task, `${at}.task`),
    fence: str(o.fence ?? "", `${at}.fence`),
    ask: {
      title: str(ask.title, `${at}.ask.title`),
      title_generated: bool(ask.title_generated ?? false, `${at}.ask.title_generated`),
      priority: num(ask.priority, `${at}.ask.priority`),
      project: orNull(str)(ask.project, `${at}.ask.project`),
      repo: orNull(str)(ask.repo, `${at}.ask.repo`),
      agent: orNull(str)(ask.agent, `${at}.ask.agent`),
      paths: list(str)(ask.paths, `${at}.ask.paths`),
      skills: list(str)(ask.skills, `${at}.ask.skills`),
    },
    materials: list(briefMaterial)(o.materials, `${at}.materials`),
    open_windows: {
      ...labelled(open, `${at}.open_windows`),
      tabs: list(str)(open.tabs, `${at}.open_windows.tabs`),
      panes: list(captureSource)(open.panes, `${at}.open_windows.panes`),
    },
  };
}

export function taskBriefReply(value: unknown): TaskBriefReply {
  const o = fields(value, "reply");
  return {
    ok: true,
    task: num(o.task, "task"),
    dir: orNull(str)(o.dir, "dir"),
    path: orNull(str)(o.path, "path"),
    brief: brief(o.brief, "brief"),
  };
}

function ref(value: unknown, at: string): Ref {
  const o = fields(value, at);
  const kind = o.kind;
  if (kind !== "tab" && kind !== "pane" && kind !== "mark" && kind !== "app") throw wrong(`${at}.kind`, "a reference kind", kind);
  const out: Ref = { kind, name: str(o.name, `${at}.name`) };
  // Keys that are not there stay out: the typed value is the reply, key for key.
  if (o.id !== undefined) out.id = num(o.id, `${at}.id`);
  for (const key of ["title", "app", "cwd", "program", "bundle"] as const) {
    if (o[key] !== undefined) out[key] = str(o[key], `${at}.${key}`);
  }
  return out;
}

function composeMenu(value: unknown, at: string): ComposeMenu {
  const o = fields(value, at);
  const row = (value: unknown, at: string): MenuRow => {
    const r = fields(value, at);
    return {
      name: str(r.name, `${at}.name`),
      kind: str(r.kind, `${at}.kind`),
      detail: str(r.detail, `${at}.detail`),
      matched: list(num)(r.matched, `${at}.matched`),
    };
  };
  return {
    mark: str(o.mark, `${at}.mark`),
    typed: str(o.typed, `${at}.typed`),
    rows: list(row)(o.rows, `${at}.rows`),
    selected: orNull(num)(o.selected, `${at}.selected`),
    note: orNull(str)(o.note, `${at}.note`),
  };
}

function composeHost(value: unknown, at: string): "drawer" | "bar" {
  if (value === "drawer" || value === "bar") return value;
  throw wrong(at, "drawer or bar", value);
}

function composePlan(value: unknown, at: string): Snapshot["compose_plan"] & object {
  const o = fields(value, at);
  return {
    agent: str(o.agent, `${at}.agent`),
    starts: bool(o.starts, `${at}.starts`),
    dir: orNull(str)(o.dir, `${at}.dir`),
    branch: orNull(str)(o.branch, `${at}.branch`),
  };
}

function composePicks(value: unknown, at: string): Snapshot["compose_picks"] {
  const o = fields(value, at);
  return { wheres: list(str)(o.wheres, `${at}.wheres`), whos: list(str)(o.whos, `${at}.whos`) };
}

function railCell(value: unknown, at: string): Snapshot["drawer"]["rail"][number] {
  const o = fields(value, at);
  return {
    task: subject(o.task, `${at}.task`),
    mono: str(o.mono, `${at}.mono`),
    group: orNull(group)(o.group, `${at}.group`),
    working: bool(o.working, `${at}.working`),
    shown: bool(o.shown, `${at}.shown`),
    tip: str(o.tip, `${at}.tip`),
  };
}

function goWord(value: unknown, at: string): "tabs" | "claim" | "new" {
  if (value === "tabs" || value === "claim" || value === "new") return value;
  throw new Error(`${at}: not a go word: ${JSON.stringify(value)}`);
}

function tabGroup(value: unknown, at: string): Snapshot["groups"][number] {
  const o = fields(value, at);
  return { task: orNull(num)(o.task, `${at}.task`), tabs: list(num)(o.tabs, `${at}.tabs`) };
}

function nativeChat(value: unknown, at: string): NativeChat {
  const o = fields(value, at);
  const t = fields(o.transcript, `${at}.transcript`);
  const entry = (value: unknown, at: string) => {
    const e = fields(value, at);
    return { ...e, kind: str(e.kind, `${at}.kind`) };
  };
  const card = (value: unknown, at: string) => {
    const c = fields(value, at);
    return {
      tool: str(c.tool, `${at}.tool`),
      label: str(c.label, `${at}.label`),
      cut: bool(c.cut, `${at}.cut`),
      expanded: bool(c.expanded, `${at}.expanded`),
      len: num(c.len, `${at}.len`),
    };
  };
  const reply = (value: unknown, at: string) => {
    const r = fields(value, at);
    const links = fields(r.links, `${at}.links`);
    return {
      blocks: map(num)(r.blocks, `${at}.blocks`),
      links: { web: num(links.web, `${at}.links.web`), file: num(links.file, `${at}.links.file`) },
    };
  };
  const view = (value: unknown, at: string): "native" | "terminal" => {
    if (value === "native" || value === "terminal") return value;
    throw wrong(at, "native or terminal", value);
  };
  return {
    view: view(o.view, `${at}.view`),
    native: bool(o.native, `${at}.native`),
    transcript: {
      entries: list(entry)(t.entries, `${at}.transcript.entries`),
      dropped: num(t.dropped, `${at}.transcript.dropped`),
      cards: list(card)(t.cards, `${at}.transcript.cards`),
      turn: orNull(str)(t.turn, `${at}.transcript.turn`),
      model: orNull(str)(t.model, `${at}.transcript.model`),
      mode: orNull(str)(t.mode, `${at}.transcript.mode`),
      session: orNull(str)(t.session, `${at}.transcript.session`),
      context: orNull(num)(t.context, `${at}.transcript.context`),
      window: orNull(num)(t.window, `${at}.transcript.window`),
      live: bool(t.live, `${at}.transcript.live`),
      watched: bool(t.watched, `${at}.transcript.watched`),
    },
    replies: list(reply)(o.replies, `${at}.replies`),
    draft_chars: num(o.draft_chars, `${at}.draft_chars`),
    deny: bool(o.deny, `${at}.deny`),
    armed: bool(o.armed, `${at}.armed`),
    away: bool(o.away, `${at}.away`),
  };
}

function native(value: unknown, at: string): Snapshot["drawer"]["native"] {
  const o = fields(value, at);
  const kind = (value: unknown, at: string): "web" | "file" => {
    if (value === "web" || value === "file") return value;
    throw wrong(at, "web or file", value);
  };
  return { chats: map(nativeChat)(o.chats, `${at}.chats`), opened: list(kind)(o.opened, `${at}.opened`) };
}

function drawerDrawn(value: unknown, at: string): Snapshot["drawer"] {
  const o = fields(value, at);
  const agent = (value: unknown, at: string) => {
    const a = fields(value, at);
    return {
      model: str(a.model, `${at}.model`),
      ended: bool(a.ended, `${at}.ended`),
      cols: num(a.cols, `${at}.cols`),
      rows: num(a.rows, `${at}.rows`),
      lines: list(str)(a.lines, `${at}.lines`),
    };
  };
  return {
    body: orNull(rect)(o.body, `${at}.body`),
    width: num(o.width, `${at}.width`),
    agents: map(agent)(o.agents, `${at}.agents`),
    native: native(o.native, `${at}.native`),
    goes_to: orNull(goWord)(o.goes_to, `${at}.goes_to`),
    note: orNull(str)(o.note, `${at}.note`),
    rail: list(railCell)(o.rail, `${at}.rail`),
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
    quoted_title: orNull(str)(o.quoted_title, `${at}.quoted_title`),
    model: orNull(str)(o.model, `${at}.model`),
  };
}

function sessionsBar(value: unknown, at: string): SessionsBar {
  const o = fields(value, at);
  return {
    count: num(o.count, `${at}.count`),
    total: num(o.total, `${at}.total`),
    range: sessionsRange(o.range, `${at}.range`),
    collapsed: bool(o.collapsed, `${at}.collapsed`),
    rows: list(sessionRow)(o.rows, `${at}.rows`),
    enabled: bool(o.enabled, `${at}.enabled`),
    reads: num(o.reads, `${at}.reads`),
    slow_reads: num(o.slow_reads, `${at}.slow_reads`),
  };
}

function reposGroup(value: unknown, at: string): ReposGroup {
  const o = fields(value, at);
  return {
    name: str(o.name, `${at}.name`),
    path: str(o.path, `${at}.path`),
    header: orNull(rect)(o.header, `${at}.header`),
    rows: list(checkoutRow)(o.rows, `${at}.rows`),
  };
}

function checkoutRow(value: unknown, at: string): CheckoutRow {
  const o = fields(value, at);
  return {
    path: str(o.path, `${at}.path`),
    branch: str(o.branch, `${at}.branch`),
    default: bool(o.default, `${at}.default`),
    label: str(o.label, `${at}.label`),
    row: orNull(rect)(o.row, `${at}.row`),
  };
}

function closedApp(value: unknown, at: string): ClosedApp {
  const o = fields(value, at);
  return {
    name: str(o.name, `${at}.name`),
    path: str(o.path, `${at}.path`),
    pinned: bool(o.pinned, `${at}.pinned`),
    suggested: bool(o.suggested, `${at}.suggested`),
  };
}

function appsBar(value: unknown, at: string): AppsBar {
  const o = fields(value, at);
  return {
    not_running: list(closedApp)(o.not_running, `${at}.not_running`),
    more: num(o.more, `${at}.more`),
    more_open: bool(o.more_open, `${at}.more_open`),
    all_count: num(o.all_count, `${at}.all_count`),
    all_open: bool(o.all_open, `${at}.all_open`),
    all: list(closedApp)(o.all, `${at}.all`),
    filter: str(o.filter, `${at}.filter`),
    filtering: bool(o.filtering, `${at}.filtering`),
    undo: orNull(str)(o.undo, `${at}.undo`),
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

function repoDirs(value: unknown, at: string): RepoDirs {
  const o = fields(value, at);
  const dir = (value: unknown, at: string) => {
    const d = fields(value, at);
    return { text: str(d.text, `${at}.text`), path: str(d.path, `${at}.path`) };
  };
  return { dirs: list(dir)(o.dirs, `${at}.dirs`), fixed: bool(o.fixed, `${at}.fixed`) };
}

function pickItem(value: unknown, at: string): PickItem {
  const o = fields(value, at);
  if (o.type === "terminal") return { type: "terminal" };
  if (o.type === "app") return { type: "app", name: str(o.name, `${at}.name`) };
  if (o.type === "closed") {
    return { type: "closed", name: str(o.name, `${at}.name`), path: str(o.path, `${at}.path`) };
  }
  throw wrong(`${at}.type`, "a picker item", o.type);
}

function field(value: unknown, at: string): Field {
  const o = fields(value, at);
  const owners = ["compose", "picker", "palette", "inspector", "apps", "mark", "workbook", "note", "talk"];
  if (typeof o.of !== "string" || !owners.includes(o.of)) {
    throw wrong(`${at}.of`, "compose, picker, apps, mark, workbook, note or talk", o.of);
  }
  return {
    of: o.of as Field["of"],
    text: str(o.text, `${at}.text`),
    cursor: num(o.cursor, `${at}.cursor`),
    selection: pair(o.selection, `${at}.selection`),
    selected: str(o.selected, `${at}.selected`),
    marked: orNull(pair)(o.marked, `${at}.marked`),
    bounds: rect(o.bounds, `${at}.bounds`),
    cursor_x: num(o.cursor_x, `${at}.cursor_x`),
  };
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

function palette(value: unknown, at: string): Palette {
  const o = fields(value, at);
  return {
    query: str(o.query, `${at}.query`),
    selected: num(o.selected, `${at}.selected`),
    tasks: orNull(list(paletteTask))(o.tasks, `${at}.tasks`),
  };
}

function paletteTask(value: unknown, at: string): PaletteTask {
  const o = fields(value, at);
  return { task: orNull(num)(o.task, `${at}.task`), title: str(o.title, `${at}.title`) };
}

function inspectorState(value: unknown, at: string): InspectorState {
  const o = fields(value, at);
  return {
    pane: num(o.pane, `${at}.pane`),
    app: str(o.app, `${at}.app`),
    error: orNull(str)(o.error, `${at}.error`),
    filter: str(o.filter, `${at}.filter`),
    filtering: bool(o.filtering, `${at}.filtering`),
  };
}

function inspectorRow(value: unknown, at: string): InspectorRow {
  const o = fields(value, at);
  return {
    path: list(num)(o.path, `${at}.path`),
    depth: num(o.depth, `${at}.depth`),
    role: str(o.role, `${at}.role`),
    pressable: bool(o.pressable, `${at}.pressable`),
    settable: bool(o.settable, `${at}.settable`),
    focused: bool(o.focused, `${at}.focused`),
    children: num(o.children, `${at}.children`),
    folded: bool(o.folded, `${at}.folded`),
  };
}

function inspector(value: unknown, at: string): Inspector {
  const o = fields(value, at);
  const c = fields(o.counts, `${at}.counts`);
  const role = (v: unknown, where: string): [string, number] => {
    if (!Array.isArray(v) || v.length !== 2) throw wrong(where, "[role, count]", v);
    return [str(v[0], `${where}[0]`), num(v[1], `${where}[1]`)];
  };
  return {
    pane: num(o.pane, `${at}.pane`),
    counts: {
      nodes: num(c.nodes, `${at}.counts.nodes`),
      pressable: num(c.pressable, `${at}.counts.pressable`),
      settable: num(c.settable, `${at}.counts.settable`),
      roles: list(role)(c.roles, `${at}.counts.roles`),
      truncated: truncated(c.truncated, `${at}.counts.truncated`),
      sparse: bool(c.sparse, `${at}.counts.sparse`),
    },
    millis: orNull(num)(o.millis, `${at}.millis`),
    rows: orNull(list(inspectorRow))(o.rows, `${at}.rows`),
    row_count: orNull(num)(o.row_count, `${at}.row_count`),
    selected: orNull(num)(o.selected, `${at}.selected`),
    outline: orNull(frame)(o.outline, `${at}.outline`),
  };
}

function paletteRow(value: unknown, at: string): PaletteRow {
  const o = fields(value, at);
  return {
    command: str(o.command, `${at}.command`),
    name: str(o.name, `${at}.name`),
    group: str(o.group, `${at}.group`),
    key: orNull(str)(o.key, `${at}.key`),
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
  const o = fields(value, at);
  return { running: list(running)(o.running, `${at}.running`), dismiss: orNull(num)(o.dismiss, `${at}.dismiss`) };
}

function markInput(value: unknown, at: string): MarkInput {
  const o = fields(value, at);
  if (o.type === "jump") return { type: "jump" };
  if (o.type === "rename") return { type: "rename", mark: num(o.mark, `${at}.mark`), text: str(o.text, `${at}.text`) };
  if (o.type === "set_key") return { type: "set_key", mark: num(o.mark, `${at}.mark`) };
  if (o.type === "workbook") {
    const typing = { type: "workbook" as const, workbook: num(o.workbook, `${at}.workbook`), text: str(o.text, `${at}.text`) };
    return o.tab === undefined ? typing : { ...typing, tab: num(o.tab, `${at}.tab`) };
  }
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

function workbook(value: unknown, at: string): Workbook {
  const o = fields(value, at);
  const buttons = fields(o.buttons, `${at}.buttons`);
  const pane = (value: unknown, at: string): WorkbookPane => {
    const p = fields(value, at);
    const kind = p.kind;
    if (kind !== "terminal" && kind !== "app" && kind !== "empty") throw wrong(`${at}.kind`, "a pane's kind", kind);
    const read: WorkbookPane = p.app === undefined ? { kind } : { kind, app: str(p.app, `${at}.app`) };
    if (p.lines !== undefined) read.lines = num(p.lines, `${at}.lines`);
    if (p.has_command !== undefined) read.has_command = bool(p.has_command, `${at}.has_command`);
    return read;
  };
  const tab = (value: unknown, at: string): WorkbookTab => {
    const t = fields(value, at);
    const words = fields(t.buttons, `${at}.buttons`);
    return {
      title: str(t.title, `${at}.title`),
      panes: list(pane)(t.panes, `${at}.panes`),
      summary: str(t.summary, `${at}.summary`),
      open: bool(t.open, `${at}.open`),
      tab: orNull(num)(t.tab, `${at}.tab`),
      row: orNull(rect)(t.row, `${at}.row`),
      buttons: {
        open: orNull(rect)(words.open, `${at}.buttons.open`),
        rename: orNull(rect)(words.rename, `${at}.buttons.rename`),
        remove: orNull(rect)(words.remove, `${at}.buttons.remove`),
      },
    };
  };
  return {
    id: num(o.id, `${at}.id`),
    name: str(o.name, `${at}.name`),
    panes: list(pane)(o.panes, `${at}.panes`),
    summary: str(o.summary, `${at}.summary`),
    open: bool(o.open, `${at}.open`),
    tab: orNull(num)(o.tab, `${at}.tab`),
    tabs: list(tab)(o.tabs, `${at}.tabs`),
    unfolded: bool(o.unfolded, `${at}.unfolded`),
    row: orNull(rect)(o.row, `${at}.row`),
    buttons: {
      open: orNull(rect)(buttons.open, `${at}.buttons.open`),
      add: orNull(rect)(buttons.add, `${at}.buttons.add`),
      fold: orNull(rect)(buttons.fold, `${at}.buttons.fold`),
      rename: orNull(rect)(buttons.rename, `${at}.buttons.rename`),
      delete: orNull(rect)(buttons.delete, `${at}.buttons.delete`),
    },
  };
}

function workbookOffer(value: unknown, at: string): "deleted" | "updated" {
  if (value === "deleted" || value === "updated") return value;
  throw wrong(at, "deleted or updated", value);
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
    sidebar: orNull(section)(o.sidebar, `${at}.sidebar`),
    compose: orNull(str)(o.compose, `${at}.compose`),
    list: orNull(num)(o.list, `${at}.list`),
    rail_focus: orNull(railFocus)(o.rail_focus, `${at}.rail_focus`),
    repos: list(repo)(o.repos, `${at}.repos`),
    repo_dirs: repoDirs(o.repo_dirs, `${at}.repo_dirs`),
    tasks: { list: list(task)(fields(o.tasks, `${at}.tasks`).list, `${at}.tasks.list`) },
    collapsed: list(heading)(o.collapsed, `${at}.collapsed`),
    tasks_view: tasksView(o.tasks_view, `${at}.tasks_view`),
    folded_projects: list(orNull(str))(o.folded_projects, `${at}.folded_projects`),
    open_done: list(orNull(str))(o.open_done, `${at}.open_done`),
    picker: orNull(picker)(o.picker, `${at}.picker`),
    palette: orNull(palette)(o.palette, `${at}.palette`),
    inspector: orNull(inspectorState)(o.inspector, `${at}.inspector`),
    confirm: orNull(confirm)(o.confirm, `${at}.confirm`),
    keys: bool(o.keys, `${at}.keys`),
    visual: orNull(num)(o.visual, `${at}.visual`),
    ax_overlay: bool(o.ax_overlay, `${at}.ax_overlay`),
    mark_input: orNull(markInput)(o.mark_input, `${at}.mark_input`),
    capture: orNull(draft)(o.capture, `${at}.capture`),
    back_to: orNull(num)(o.back_to, `${at}.back_to`),
    brief: orNull(num)(o.brief, `${at}.brief`),
    talk: orNull(talk)(o.talk, `${at}.talk`),
    talk_width: num(o.talk_width, `${at}.talk_width`),
    talk_wide: bool(o.talk_wide, `${at}.talk_wide`),
    agents: list(subject)(o.agents, `${at}.agents`),
  };
}

function talk(value: unknown, at: string): { task: Subject; open: boolean; keys: boolean } {
  const o = fields(value, at);
  return { task: subject(o.task, `${at}.task`), open: bool(o.open, `${at}.open`), keys: bool(o.keys, `${at}.keys`) };
}

function subject(value: unknown, at: string): Subject {
  if (typeof value === "number") return num(value, at);
  const o = fields(value, at);
  return { runner: str(o.runner, `${at}.runner`), id: str(o.id, `${at}.id`) };
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
  return {
    targets: list(axTarget)(o.targets, `${at}.targets`),
    typed: str(o.typed, `${at}.typed`),
    app: orNull(sneakApp)(o.app, `${at}.app`),
  };
}

function appPress(value: unknown, at: string): [PaneId, number[]] {
  if (!Array.isArray(value) || value.length !== 2) throw wrong(at, "[pane, path]", value);
  return [num(value[0], `${at}[0]`), list(num)(value[1], `${at}[1]`)];
}

function sneakApp(value: unknown, at: string): SneakApp {
  const o = fields(value, at);
  return { pane: num(o.pane, `${at}.pane`), controls: num(o.controls, `${at}.controls`), found: num(o.found, `${at}.found`) };
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
    focus: focus(o.focus, "focus"),
    state: model(o.state, "state"),
    bounds: map(rect)(o.bounds, "bounds"),
    frames: map(rect)(o.frames, "frames"),
    headers: map(rect)(o.headers, "headers"),
    closes: map(rect)(o.closes, "closes"),
    tabs: list(rect)(o.tabs, "tabs"),
    strip: orNull(rect)(o.strip, "strip"),
    bar_header: orNull(rect)(o.bar_header, "bar_header"),
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
    picker_rows: orNull(list(pickItem))(o.picker_rows, "picker_rows"),
    palette_rows: orNull(list(paletteRow))(o.palette_rows, "palette_rows"),
    inspector: orNull(inspector)(o.inspector, "inspector"),
    field: orNull(field)(o.field, "field"),
    inbox: list(inboxRow)(o.inbox, "inbox"),
    needs_you: num(o.needs_you, "needs_you"),
    compose_read: bool(o.compose_read, "compose_read"),
    compose_host: orNull(composeHost)(o.compose_host, "compose_host"),
    compose_plan: orNull(composePlan)(o.compose_plan, "compose_plan"),
    compose_picks: composePicks(o.compose_picks, "compose_picks"),
    welcome: bool(o.welcome, "welcome"),
    compose_menu: orNull(composeMenu)(o.compose_menu, "compose_menu"),
    compose_pointing: orNull(rect)(o.compose_pointing, "compose_pointing"),
    compose_note: orNull(str)(o.compose_note, "compose_note"),
    tasks_bar: tasksBar(o.tasks_bar, "tasks_bar"),
    tasks_progress: tasksProgress(o.tasks_progress, "tasks_progress"),
    sessions_bar: sessionsBar(o.sessions_bar, "sessions_bar"),
    repos_bar: list(reposGroup)(o.repos_bar, "repos_bar"),
    home: orNull(home)(o.home, "home"),
    home_drawn: homeDrawn(o.home_drawn, "home_drawn"),
    closed_apps: list(str)(o.closed_apps, "closed_apps"),
    apps_bar: appsBar(o.apps_bar, "apps_bar"),
    apps: list(appEntry)(o.apps, "apps"),
    ax_overlay: orNull(axOverlay)(o.ax_overlay, "ax_overlay"),
    sneak: orNull(sneak)(o.sneak, "sneak"),
    counts: orNull(map(num))(o.counts, "counts"),
    app_presses: orNull(list(appPress))(o.app_presses, "app_presses"),
    marks: list(mark)(o.marks, "marks"),
    mark_removed: orNull(str)(o.mark_removed, "mark_removed"),
    workbooks: list(workbook)(o.workbooks, "workbooks"),
    workbook_offer: orNull(workbookOffer)(o.workbook_offer, "workbook_offer"),
    scrollbars: map(scrollbar)(o.scrollbars, "scrollbars"),
    capture: captureDrawn(o.capture, "capture"),
    groups: list(tabGroup)(o.groups, "groups"),
    badges: map(str)(o.badges, "badges"),
    drawer: drawerDrawn(o.drawer, "drawer"),
    calendar: calendar(o.calendar, "calendar"),
    frame: frameNumbers(o.frame, "frame"),
    ...(o.stale === undefined ? {} : { stale: staleMark(o.stale, "stale") }),
  };
}

function meetingStage(value: unknown, at: string): MeetingStage {
  if (value === "far" || value === "approaching" || value === "imminent" || value === "running" || value === "past") {
    return value;
  }
  throw wrong(at, "a meeting stage", value);
}

function railMark(value: unknown, at: string): Snapshot["calendar"]["rail"] {
  if (value === "none" || value === "near" || value === "alert") return value;
  throw wrong(at, "none, near or alert", value);
}

function calendar(value: unknown, at: string): Snapshot["calendar"] {
  const o = fields(value, at);
  const banner = (v: unknown, at: string) => {
    const b = fields(v, at);
    return { join: orNull(rect)(b.join, `${at}.join`), dismiss: orNull(rect)(b.dismiss, `${at}.dismiss`) };
  };
  return {
    on: bool(o.on, `${at}.on`),
    events_today: num(o.events_today, `${at}.events_today`),
    all_day_today: num(o.all_day_today, `${at}.all_day_today`),
    events_tomorrow: num(o.events_tomorrow, `${at}.events_tomorrow`),
    next_start: orNull(num)(o.next_start, `${at}.next_start`),
    stage: orNull(meetingStage)(o.stage, `${at}.stage`),
    updated: orNull(num)(o.updated, `${at}.updated`),
    stale: bool(o.stale, `${at}.stale`),
    in_call: orNull(bool)(o.in_call, `${at}.in_call`),
    soon: bool(o.soon, `${at}.soon`),
    bars: num(o.bars, `${at}.bars`),
    chip: bool(o.chip, `${at}.chip`),
    banner: orNull(banner)(o.banner, `${at}.banner`),
    alerts: num(o.alerts, `${at}.alerts`),
    joins: orNull(num)(o.joins, `${at}.joins`),
    rows: num(o.rows, `${at}.rows`),
    rail: railMark(o.rail, `${at}.rail`),
    tomorrow_open: bool(o.tomorrow_open, `${at}.tomorrow_open`),
  };
}

function frameNumbers(value: unknown, at: string): Snapshot["frame"] {
  const o = fields(value, at);
  return {
    number: num(o.number, `${at}.number`),
    seq: num(o.seq, `${at}.seq`),
    drawn: num(o.drawn, `${at}.drawn`),
    waited_ms: num(o.waited_ms, `${at}.waited_ms`),
    window: orNull(num)(o.window, `${at}.window`),
  };
}

function staleMark(value: unknown, at: string): true {
  if (value === true) return true;
  throw wrong(at, "true", value);
}

export function traceReply(value: unknown): TraceReply {
  return { ok: true, trace: str(fields(value, "reply").trace, "trace") };
}

function scrollbar(value: unknown, at: string): Scrollbar {
  const o = fields(value, at);
  return {
    track: rect(o.track, `${at}.track`),
    thumb: rect(o.thumb, `${at}.thumb`),
    opacity: num(o.opacity, `${at}.opacity`),
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
