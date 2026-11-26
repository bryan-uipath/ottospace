// Turns a reply (`unknown` from JSON.parse) into the types in `types.ts` with runtime checks:
// no casts, so a reply the types do not describe is an error here, not a wrong value later.
// Keys the types do not name are dropped, so a test can compare the result with the raw reply.

import type {
  CopyAsk,
  ExplorerShape,
  AppWindow,
  CloneDrawn,
  Display,
  TabOut,
  BugMark,
  BugReport,
  AgentChip,
  AgentDial,
  ChatSelection,
  ChatWorking,
  AgentLine,
  Btw,
  CalendarFetch,
  Toast,
  ToastCard,
  NoticeLevel,
  AppEntry,
  Feature,
  Finish,
  Listen,
  ListenLine,
  Slot,
  Offer,
  Decisions,
  DecisionSorting,
  SlotDialog,
  Viewer,
  ViewerDrawn,
  ViewerRead,
  CalendarReply,
  Captured,
  Note,
  AgentGrant,
  NoteReply,
  DecisionAccepted,
  FocusArea,
  FocusEdit,
  FocusSetReply,
  WeekFocusReply,
  WeekFocusState,
  DecisionAct,
  DecisionAt,
  DecisionChoice,
  DecisionEvidence,
  DecisionFileReply,
  DecisionProvenance,
  DecisionReply,
  Filed,
  FiledRow,
  WorkspaceReply,
  Meeting,
  TimeSpan,
  NativeChat,
  MeetingShow,
  MeetingStage,
  CalendarDayView,
  AxNode,
  AxOverlay,
  AxSource,
  Editor,
  AxTarget,
  AgentMode,
  AppLookReply,
  AppReadReply,
  Untrusted,
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
  Amount,
  Clip,
  Confirm,
  Unsent,
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
  RailItem,
  Home,
  Glyph,
  HomeDrawn,
  CityDrawn,
  CityAgent,
  CityBehind,
  CityGlimmer,
  CitySpin,
  CitySpun,
  Typed,
  TypingDist,
  TypingState,
  CityHovered,
  CityName,
  CitySpots,
  CityOn,
  CityView,
  HomeNode,
  InboxReply,
  InboxRow,
  Layout,
  Mark,
  MarkInput,
  MarkReply,
  MarksReply,
  RecordReply,
  ProbeBuffer,
  AudioProbeReply,
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
  GoToRow,
  PaletteFeature,
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
  ProjectsBar,
  ProjectRow,
  RepoDirs,
  CheckoutRow,
  PullRequest,
  AppsBar,
  ClosedApp,
  Sneak,
  SneakApp,
  SneakTarget,
  KeyReference,
  Snapshot,
  Station,
  SpaceRow,
  RowMenu,
  Owner,
  ReorderItem,
  Tab,
  Task,
  TaskPopReply,
  TaskNewReply,
  TaskReply,
  TaskRow,
  TaskSection,
  SessionCard,
  TaskState,
  TasksBar,
  Terminal,
  StyledSpan,
  Truncated,
  WaitReply,
  Worktree,
  Zone,
  Zoom,
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

function triple(value: unknown, at: string): [number, number, number] {
  if (!Array.isArray(value) || value.length !== 3) throw wrong(at, "three numbers", value);
  return [num(value[0], at), num(value[1], at), num(value[2], at)];
}

function range(value: unknown, at: string): [[number, number], [number, number]] {
  if (!Array.isArray(value) || value.length !== 2) throw wrong(at, "two [line, column] pairs", value);
  return [pair(value[0], at), pair(value[1], at)];
}

function rect(value: unknown, at: string): Rect {
  if (!Array.isArray(value) || value.length !== 4) throw wrong(at, "[x, y, w, h]", value);
  return [num(value[0], at), num(value[1], at), num(value[2], at), num(value[3], at)];
}

function display(value: unknown, at: string): Display {
  const o = fields(value, at);
  return {
    id: num(o.id, `${at}.id`),
    frame: rect(o.frame, `${at}.frame`),
    scale: num(o.scale, `${at}.scale`),
    main: bool(o.main, `${at}.main`),
  };
}

function appWindow(value: unknown, at: string): AppWindow {
  const o = fields(value, at);
  const out: AppWindow = {
    main: bool(o.main, `${at}.main`),
    tab: orNull(num)(o.tab, `${at}.tab`),
    display: orNull(num)(o.display, `${at}.display`),
    key: bool(o.key, `${at}.key`),
    fill: bool(o.fill, `${at}.fill`),
  };
  if ("frame" in o) out.frame = orNull(rect)(o.frame, `${at}.frame`);
  if ("frames" in o) out.frames = orNull(num)(o.frames, `${at}.frames`);
  if ("number" in o) out.number = orNull(num)(o.number, `${at}.number`);
  return out;
}

function cutText(value: unknown, at: string): { room: Rect; drawn: number } {
  const o = fields(value, at);
  return { room: rect(o.room, `${at}.room`), drawn: num(o.drawn, `${at}.drawn`) };
}

function tabOut(value: unknown, at: string): TabOut {
  const o = fields(value, at);
  return { tab: num(o.tab, `${at}.tab`), display: num(o.display, `${at}.display`) };
}

function axis(value: unknown, at: string): Axis {
  if (value === "right" || value === "down") return value;
  throw wrong(at, "an axis", value);
}

function mode(value: unknown, at: string): Mode {
  if (value === "normal" || value === "app" || value === "pick" || value === "palette") return value;
  if (value === "clone") return value;
  if (value === "inspect") return value;
  if (value === "visual" || value === "list" || value === "compose" || value === "sneak") return value;
  if (value === "confirm" || value === "mark" || value === "annotate" || value === "talk") return value;
  throw wrong(at, "a mode", value);
}

function section(value: unknown, at: string): Section {
  if (value === "apps" || value === "workbook" || value === "tasks" || value === "repos") return value;
  if (value === "marks" || value === "calendar" || value === "projects") return value;
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
  if (o.drawer_rail !== undefined) return { drawer_rail: railItem(o.drawer_rail, `${at}.drawer_rail`) };
  return { rail: section(o.rail, `${at}.rail`) };
}

function railItem(value: unknown, at: string): RailItem {
  const o = fields(value, at);
  if (o.kind === "transcript") return { kind: "transcript" };
  if (o.kind === "task") return { kind: "task", id: num(o.id, `${at}.id`) };
  throw wrong(`${at}.kind`, "transcript or task", o.kind);
}

function railFocus(value: unknown, at: string): RailFocus {
  const o = fields(value, at);
  if (o.right !== undefined) return { right: railItem(o.right, `${at}.right`) };
  return { left: section(o.left, `${at}.left`) };
}

function railRow(value: unknown, at: string): Snapshot["drawer"]["rail"][number] {
  const o = fields(value, at);
  const kind = o.kind;
  if (kind !== "transcript" && kind !== "task") throw wrong(`${at}.kind`, "transcript or task", kind);
  const section = o.section;
  if (section !== "workspace" && section !== "everywhere") {
    throw wrong(`${at}.section`, "workspace or everywhere", section);
  }
  return {
    kind,
    id: orNull(num)(o.id, `${at}.id`),
    section,
    selected: bool(o.selected, `${at}.selected`),
    rect: orNull(rect)(o.rect, `${at}.rect`),
  };
}

function heading(value: unknown, at: string): Heading {
  if (value === "needs_you" || value === "running" || value === "later") return value;
  if (value === "done_today" || value === "done" || value === "sessions") return value;
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
    priority: num(o.priority, `${at}.priority`),
    filed: orNull(str)(o.filed, `${at}.filed`),
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
    current: bool(o.current, `${at}.current`),
    fact: orNull(str)(o.fact, `${at}.fact`),
    live: orNull(str)(o.live, `${at}.live`),
    thinking: orNull(num)(o.thinking, `${at}.thinking`),
    shimmer: bool(o.shimmer, `${at}.shimmer`),
    worktree: orNull(str)(o.worktree, `${at}.worktree`),
    pending: list(pending)(o.pending, `${at}.pending`),
    more: num(o.more, `${at}.more`),
  };
}

function pending(value: unknown, at: string): TaskRow["pending"][number] {
  const o = fields(value, at);
  if (o.kind === "permission") return { kind: "permission" };
  if (o.kind === "question") return { kind: "question", shown: num(o.shown, `${at}.shown`), of: num(o.of, `${at}.of`) };
  throw wrong(`${at}.kind`, "question or permission", o.kind);
}

function taskSection(value: unknown, at: string): TaskSection {
  const o = fields(value, at);
  return {
    heading: heading(o.heading, `${at}.heading`),
    count: num(o.count, `${at}.count`),
    collapsed: bool(o.collapsed, `${at}.collapsed`),
    rows: list(taskRow)(o.rows, `${at}.rows`),
    cards: list(sessionCard)(o.cards, `${at}.cards`),
  };
}

function sessionCard(value: unknown, at: string): SessionCard {
  const o = fields(value, at);
  return {
    ...sessionRow(value, at),
    state: oneOf(["needs_you", "running", "ended"] as const, "a card's state")(o.state, `${at}.state`),
    take_over: bool(o.take_over, `${at}.take_over`),
    // An older app's card has none.
    hidden: o.hidden === undefined ? false : bool(o.hidden, `${at}.hidden`),
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
  if (o.type === "slot") {
    const slot: PaneKind = { type: "slot", name: str(o.name, `${at}.name`) };
    if (o.bundle !== undefined) slot.bundle = str(o.bundle, `${at}.bundle`);
    return slot;
  }
  if (o.type === "file") return { type: "file", path: str(o.path, `${at}.path`) };
  if (o.type === "explorer") return { type: "explorer", root: str(o.root, `${at}.root`) };
  if (o.type === "decision") return { type: "decision" };
  throw wrong(`${at}.type`, "a pane kind", o.type);
}

function viewerRead(value: unknown, at: string): ViewerRead {
  const o = fields(value, at);
  if (o.status === "shown") return { status: "shown", lines: num(o.lines, `${at}.lines`) };
  if (o.status === "too_large") return { status: "too_large", bytes: num(o.bytes, `${at}.bytes`) };
  const size = () => ({ width: num(o.width, `${at}.width`), height: num(o.height, `${at}.height`) });
  if (o.status === "picture") {
    return { status: "picture", ...size(), frames: num(o.frames, `${at}.frames`), bytes: num(o.bytes, `${at}.bytes`) };
  }
  if (o.status === "too_many_pixels") return { status: "too_many_pixels", ...size(), frames: num(o.frames, `${at}.frames`) };
  if (o.status === "diff") {
    return {
      status: "diff",
      rows: num(o.rows, `${at}.rows`),
      hunks: list(num)(o.hunks, `${at}.hunks`),
      added: num(o.added, `${at}.added`),
      removed: num(o.removed, `${at}.removed`),
      cut: bool(o.cut, `${at}.cut`),
    };
  }
  const bare = ["reading", "missing", "not_file", "binary", "unreadable", "broken", "no_change"] as const;
  return { status: oneOf(bare, "what the viewer read")(o.status, `${at}.status`) };
}

function viewer(value: unknown, at: string): Viewer {
  const o = fields(value, at);
  const view: Viewer = {
    shows: oneOf(["markdown", "code", "text", "image"] as const, "what a file shows as")(o.shows, `${at}.shows`),
    read: viewerRead(o.read, `${at}.read`),
    top: num(o.top, `${at}.top`),
    most: orNull(num)(o.most, `${at}.most`),
    page: num(o.page, `${at}.page`),
    pending: orNull(str)(o.pending, `${at}.pending`),
    zoom: orNull(num)(o.zoom, `${at}.zoom`),
    pan: pair(o.pan, `${at}.pan`),
    playing: orNull(bool)(o.playing, `${at}.playing`),
    diff: bool(o.diff, `${at}.diff`),
  };
  if (o.opener !== undefined) view.opener = num(o.opener, `${at}.opener`);
  if (o.marked !== undefined) {
    const marked = fields(o.marked, `${at}.marked`);
    view.marked = { first: num(marked.first, `${at}.marked.first`), last: num(marked.last, `${at}.marked.last`) };
  }
  return view;
}

function explorerShape(value: unknown, at: string): ExplorerShape {
  const o = fields(value, at);
  return {
    rows: num(o.rows, `${at}.rows`),
    selected: orNull(num)(o.selected, `${at}.selected`),
    open: num(o.open, `${at}.open`),
    reading: num(o.reading, `${at}.reading`),
    marked: num(o.marked, `${at}.marked`),
    show_ignored: bool(o.show_ignored, `${at}.show_ignored`),
    paths: orNull(list(str))(o.paths, `${at}.paths`),
  };
}

function viewerDrawn(value: unknown, at: string): ViewerDrawn {
  const o = fields(value, at);
  const drawn: ViewerDrawn = {
    body: orNull(oneOf(["markdown", "lines", "image", "diff"] as const, "a viewer's body"))(o.body, `${at}.body`),
  };
  for (const key of ["rows", "hunks", "added", "removed"] as const) {
    if (o[key] !== undefined) drawn[key] = num(o[key], `${at}.${key}`);
  }
  if (o.shape !== undefined) {
    const shape = fields(o.shape, `${at}.shape`);
    const links = fields(shape.links, `${at}.shape.links`);
    drawn.shape = {
      blocks: map(num)(shape.blocks, `${at}.shape.blocks`),
      links: { web: num(links.web, `${at}.shape.links.web`), file: num(links.file, `${at}.shape.links.file`) },
    };
  }
  if (o.lines !== undefined) drawn.lines = num(o.lines, `${at}.lines`);
  if (o.frames !== undefined) drawn.frames = num(o.frames, `${at}.frames`);
  if (o.frame !== undefined) drawn.frame = num(o.frame, `${at}.frame`);
  if (o.syntax !== undefined) {
    const langs = [
      "rust", "typescript", "tsx", "javascript", "json", "toml", "python", "bash", "go",
      "c", "cpp", "java", "csharp", "ruby", "html", "css", "yaml", "sql",
      "php", "lua", "make", "xml", "markdown"
    ] as const;
    drawn.syntax = orNull(oneOf(langs, "a grammar"))(o.syntax, `${at}.syntax`);
  }
  if (o.marked_rect !== undefined) drawn.marked_rect = orNull(rect)(o.marked_rect, `${at}.marked_rect`);
  return drawn;
}

function pane(value: unknown, at: string): Pane {
  const o = fields(value, at);
  const pane: Pane = { id: num(o.id, `${at}.id`), kind: paneKind(o.kind, `${at}.kind`) };
  if (o.cwd !== undefined) pane.cwd = str(o.cwd, `${at}.cwd`);
  if (o.owned !== undefined) {
    if (o.owned !== true) throw wrong(`${at}.owned`, "true", o.owned);
    pane.owned = true;
  }
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
  return { id: num(o.id, `${at}.id`), title: str(o.title, `${at}.title`), space: owner(o.space, `${at}.space`) };
}

function owner(value: unknown, at: string): Owner {
  const o = fields(value, at);
  switch (o.kind) {
    case "home":
      return { kind: "home" };
    case "task":
      return { kind: "task", id: num(o.id, `${at}.id`) };
    case "session":
      return { kind: "session", runner: str(o.runner, `${at}.runner`), id: str(o.id, `${at}.id`) };
    case "workbook":
      return { kind: "workbook", id: num(o.id, `${at}.id`) };
    case "decisions":
      return { kind: "decisions" };
    default:
      throw new Error(`${at}.kind: not a space: ${JSON.stringify(o.kind)}`);
  }
}

function reorder(value: unknown, at: string): { item: ReorderItem; to: number } {
  const o = fields(value, at);
  const item = fields(o.item, `${at}.item`);
  let carried: ReorderItem;
  if (item.kind === "workbook") carried = { kind: "workbook", id: num(item.id, `${at}.item.id`) };
  else if (item.kind === "space") carried = { kind: "space", of: owner(item.of, `${at}.item.of`) };
  else if (item.kind === "repo") carried = { kind: "repo", path: str(item.path, `${at}.item.path`) };
  else if (item.kind === "project") carried = { kind: "project" };
  else throw wrong(`${at}.item.kind`, "workbook, space, repo or project", item.kind);
  return { item: carried, to: num(o.to, `${at}.to`) };
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
    ...(o.folder === undefined ? {} : { folder: str(o.folder, `${at}.folder`) }),
    ...(o.worktree === undefined ? {} : { worktree: str(o.worktree, `${at}.worktree`) }),
    ...(o.made === undefined ? {} : { made: made(o.made, `${at}.made`) }),
    agent: orNull(str)(o.agent, `${at}.agent`),
    paths: list(str)(o.paths, `${at}.paths`),
    resolved: list(str)(o.resolved, `${at}.resolved`),
    skills: list(str)(o.skills, `${at}.skills`),
    ...(o.line === undefined ? {} : { line: str(o.line, `${at}.line`) }),
    ...(o.body === undefined ? {} : { body: str(o.body, `${at}.body`) }),
    ...(o.queued === undefined ? {} : { queued: num(o.queued, `${at}.queued`) }),
    ...(o.images === undefined ? {} : { images: num(o.images, `${at}.images`) }),
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

function bugReport(value: unknown, at: string): BugReport {
  const o = fields(value, at);
  const mark = (value: unknown, at: string): BugMark => {
    const m = fields(value, at);
    return { n: num(m.n, `${at}.n`), rect: orNull(rect)(m.rect, `${at}.rect`), comment: str(m.comment, `${at}.comment`) };
  };
  return {
    size: pair(o.size, `${at}.size`),
    marks: list(mark)(o.marks, `${at}.marks`),
    typing: orNull(num)(o.typing, `${at}.typing`),
    drag: orNull(rect)(o.drag, `${at}.drag`),
    apps: bool(o.apps, `${at}.apps`),
    picture: bool(o.picture, `${at}.picture`),
  };
}

function bugDrawn(value: unknown, at: string): Snapshot["bug"] {
  const o = fields(value, at);
  return {
    icon: bool(o.icon, `${at}.icon`),
    report: orNull(bugReport)(o.report, `${at}.report`),
    blanked: orNull(list(rect))(o.blanked, `${at}.blanked`),
    pixels: orNull(pair)(o.pixels, `${at}.pixels`),
    controls: orNull(map(rect))(o.controls, `${at}.controls`),
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
    ...(o.crop === undefined ? {} : { crop: rect(o.crop, `${at}.crop`) }),
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
    ...(o.note === undefined ? {} : { note: note(o.note, `${at}.note`) }),
  };
}

function note(value: unknown, at: string): Captured<Note> {
  const o = fields(value, at);
  const meeting = (v: unknown, at: string) => {
    const m = fields(v, at);
    return { title: str(m.title, `${at}.title`), start: num(m.start, `${at}.start`), end: num(m.end, `${at}.end`) };
  };
  const transcript = (v: unknown, at: string) => {
    const t = fields(v, at);
    return { file: str(t.file, `${at}.file`), at_ms: num(t.at_ms, `${at}.at_ms`) };
  };
  return {
    ...labelled(o, at),
    filed: num(o.filed, `${at}.filed`),
    meeting: orNull(meeting)(o.meeting, `${at}.meeting`),
    transcript: orNull(transcript)(o.transcript, `${at}.transcript`),
    front: orNull(str)(o.front, `${at}.front`),
  };
}

export function workspaceReply(value: unknown): WorkspaceReply {
  const o = fields(value, "reply");
  const pane = (value: unknown, at: string) => {
    const p = fields(value, at);
    return {
      id: num(p.id, `${at}.id`),
      kind: paneKind(p.kind, `${at}.kind`),
      cwd: orNull(str)(p.cwd, `${at}.cwd`),
      grant: orNull(grant)(p.grant, `${at}.grant`),
    };
  };
  const tab = (value: unknown, at: string) => {
    const t = fields(value, at);
    return { id: num(t.id, `${at}.id`), title: str(t.title, `${at}.title`), panes: list(pane)(t.panes, `${at}.panes`) };
  };
  return {
    ok: true,
    task: orNull(num)(o.task, "task"),
    space: owner(o.space, "space"),
    name: str(o.name, "name"),
    tabs: list(tab)(o.tabs, "tabs"),
    agent_mode: bool(o.agent_mode, "agent_mode"),
  };
}

export function noteReply(value: unknown): NoteReply {
  const o = fields(value, "reply");
  return { ok: true, task: num(o.task, "task"), note: note(o.note, "note") };
}

const decisionSource = oneOf(
  ["agent_session", "terminal_session", "slack", "jira", "pr_review", "calendar", "routine"] as const,
  "a decision's source",
);
const decisionStatus = oneOf(["open", "withdrawn", "expired", "dismissed", "not_mine"] as const, "a decision's status");
const decisionTriage = oneOf(["pending", "skipped", "sorted"] as const, "triage");
const via = oneOf(["socket"] as const, "how a decision came");

export function decisionFileReply(value: unknown): DecisionFileReply {
  const o = fields(value, "reply");
  return {
    ok: true,
    id: num(o.id, "id"),
    revision: num(o.revision, "revision"),
    status: decisionStatus(o.status, "status"),
    triage: decisionTriage(o.triage, "triage"),
  };
}

function decisionAct(value: unknown, at: string): DecisionAct {
  if (value === "answer" || value === "go_to_pane") return value;
  const o = fields(value, at);
  if (o.start_task !== undefined) {
    const t = fields(o.start_task, `${at}.start_task`);
    return { start_task: { title: str(t.title, `${at}.start_task.title`), brief: str(t.brief, `${at}.start_task.brief`) } };
  }
  if (o.draft_reply !== undefined) {
    return { draft_reply: { text: str(fields(o.draft_reply, `${at}.draft_reply`).text, `${at}.draft_reply.text`) } };
  }
  if (o.next_action !== undefined) {
    return { next_action: { text: str(fields(o.next_action, `${at}.next_action`).text, `${at}.next_action.text`) } };
  }
  if (o.open_link !== undefined) {
    return { open_link: { url: str(fields(o.open_link, `${at}.open_link`).url, `${at}.open_link.url`) } };
  }
  throw wrong(at, "an act", value);
}

function decisionChoice(value: unknown, at: string): DecisionChoice {
  const o = fields(value, at);
  return {
    label: str(o.label, `${at}.label`),
    consequence: str(o.consequence, `${at}.consequence`),
    act: orNull(decisionAct)(o.act, `${at}.act`),
  };
}

function decisionAt(value: unknown, at: string): DecisionAt {
  if (typeof value === "string") return value;
  const o = fields(value, at);
  return { path: str(o.path, `${at}.path`), lines: orNull(pair)(o.lines, `${at}.lines`) };
}

function decisionEvidence(value: unknown, at: string): DecisionEvidence {
  const o = fields(value, at);
  const provenance = (v: unknown, p: string): DecisionProvenance => {
    const f = fields(v, p);
    return {
      source: decisionSource(f.source, `${p}.source`),
      when: orNull(num)(f.when, `${p}.when`),
      who: oneOf(["person", "bot", "filer"] as const, "who")(f.who, `${p}.who`),
    };
  };
  return {
    kind: oneOf(["quote", "link", "file"] as const, "an evidence kind")(o.kind, `${at}.kind`),
    text: str(o.text, `${at}.text`),
    at: orNull(decisionAt)(o.at, `${at}.at`),
    provenance: orNull(provenance)(o.provenance, `${at}.provenance`),
  };
}

export function decisionReply(value: unknown): DecisionReply {
  const d = fields(fields(value, "reply").decision, "decision");
  const at = (key: string) => `decision.${key}`;
  const suggest = fields(d.suggest, at("suggest"));
  const claims = fields(d.claims, at("claims"));
  return {
    ok: true,
    decision: {
      ...labelled(d, "decision"),
      id: num(d.id, at("id")),
      source: decisionSource(d.source, at("source")),
      key: str(d.key, at("key")),
      via: via(d.via, at("via")),
      revision: num(d.revision, at("revision")),
      filed: num(d.filed, at("filed")),
      updated: num(d.updated, at("updated")),
      summary: str(d.summary, at("summary")),
      question: str(d.question, at("question")),
      options: list(decisionChoice)(d.options, at("options")),
      recommend: orNull(num)(d.recommend, at("recommend")),
      why: str(d.why, at("why")),
      evidence: list(decisionEvidence)(d.evidence, at("evidence")),
      suggest: {
        projects: list(str)(suggest.projects, at("suggest.projects")),
        role: orNull(oneOf(["core", "ancillary"] as const, "a role"))(suggest.role, at("suggest.role")),
        role_why: str(suggest.role_why, at("suggest.role_why")),
      },
      claims: {
        due: orNull(num)(claims.due, at("claims.due")),
        person_waits: bool(claims.person_waits, at("claims.person_waits")),
        blocks: list(num)(claims.blocks, at("claims.blocks")),
      },
      triage: decisionTriage(d.triage, at("triage")),
      expires: num(d.expires, at("expires")),
      task: orNull(num)(d.task, at("task")),
      status: decisionStatus(d.status, at("status")),
      closed: orNull(num)(d.closed, at("closed")),
      later: orNull(num)(d.later, at("later")),
      accepted: decisionAccepted(d.accepted, at("accepted")),
      revised: list(str)(d.revised, at("revised")),
    },
  };
}

function decisionAccepted(value: unknown, at: string): DecisionAccepted {
  const o = fields(value, at);
  return {
    project: orNull(str)(o.project, `${at}.project`),
    role: orNull(oneOf(["core", "ancillary"] as const, "a role"))(o.role, `${at}.role`),
    claims: bool(o.claims, `${at}.claims`),
    now_until: orNull(num)(o.now_until, `${at}.now_until`),
    revision: num(o.revision, `${at}.revision`),
  };
}

const setBy = oneOf(["user", "proposal"] as const, "who set the focus");

function weekCard(value: unknown, at: string): { proposal: number } | "carry" {
  if (value === "carry") return value;
  return { proposal: num(fields(value, at).proposal, `${at}.proposal`) };
}

function seed(value: unknown, at: string): FocusEdit["from"] {
  if (value === "current" || value === "last") return value;
  return { proposal: num(fields(value, at).proposal, `${at}.proposal`) };
}

function focusEdit(value: unknown, at: string): FocusEdit {
  const o = fields(value, at);
  return {
    count: num(o.count, `${at}.count`),
    ticked: list(num)(o.ticked, `${at}.ticked`),
    cursor: num(o.cursor, `${at}.cursor`),
    // Absent from an app before proposals: the current week's.
    from: o.from === undefined ? "current" : seed(o.from, `${at}.from`),
  };
}

export function focusSetReply(value: unknown): FocusSetReply {
  const o = fields(value, "reply");
  return {
    ok: true,
    status: oneOf(["pending", "same", "declined"] as const, "what a proposal came to")(o.status, "status"),
    revision: orNull(num)(o.revision, "revision"),
  };
}

function weekFocusState(value: unknown, at: string): WeekFocusState {
  const o = fields(value, at);
  return {
    week: str(o.week, `${at}.week`),
    areas: num(o.areas, `${at}.areas`),
    set_by: orNull(setBy)(o.set_by, `${at}.set_by`),
    last: orNull(str)(o.last, `${at}.last`),
    // Absent from an app before proposals: none, and no carry question.
    proposal: o.proposal === undefined ? null : orNull((v: unknown, p: string) => {
      const f = fields(v, p);
      return { revision: num(f.revision, `${p}.revision`), areas: num(f.areas, `${p}.areas`), filed: num(f.filed, `${p}.filed`) };
    })(o.proposal, `${at}.proposal`),
    carry: o.carry === undefined ? false : bool(o.carry, `${at}.carry`),
  };
}

function focusArea(value: unknown, at: string): FocusArea {
  const o = fields(value, at);
  return { name: str(o.name, `${at}.name`), projects: list(str)(o.projects, `${at}.projects`), repos: list(str)(o.repos, `${at}.repos`) };
}

export function weekFocusReply(value: unknown): WeekFocusReply {
  const o = fields(value, "reply");
  const f = fields(o.focus, "focus");
  return {
    ok: true,
    week: str(o.week, "week"),
    focus: { ...labelled(f, "focus"), areas: list(focusArea)(f.areas, "focus.areas"), set_by: orNull(setBy)(f.set_by, "focus.set_by") },
  };
}

function filedRow(value: unknown, at: string): FiledRow {
  const o = fields(value, at);
  return {
    id: num(o.id, `${at}.id`),
    source: decisionSource(o.source, `${at}.source`),
    via: via(o.via, `${at}.via`),
    triage: decisionTriage(o.triage, `${at}.triage`),
    status: decisionStatus(o.status, `${at}.status`),
    revision: num(o.revision, `${at}.revision`),
    filed: num(o.filed, `${at}.filed`),
    updated: num(o.updated, `${at}.updated`),
    expires: num(o.expires, `${at}.expires`),
    task: orNull(num)(o.task, `${at}.task`),
    options: num(o.options, `${at}.options`),
    evidence: num(o.evidence, `${at}.evidence`),
  };
}

function filed(value: unknown, at: string): Filed {
  const o = fields(value, at);
  return {
    open: num(o.open, `${at}.open`),
    to_sort: num(o.to_sort, `${at}.to_sort`),
    trusted: num(o.trusted, `${at}.trusted`),
    rows: list(filedRow)(o.rows, `${at}.rows`),
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

function pillKind(value: unknown, at: string): "meeting" | "transcript" | "front" {
  if (value === "meeting" || value === "transcript" || value === "front") return value;
  throw wrong(at, "meeting, transcript or front", value);
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

function composeChoices(value: unknown, at: string): Snapshot["compose_choices"] & object {
  const o = fields(value, at);
  return {
    project: orNull(str)(o.project, `${at}.project`),
    place: orNull(str)(o.place, `${at}.place`),
    agent: str(o.agent, `${at}.agent`),
    default: bool(o.default, `${at}.default`),
  };
}

function composeWorktree(value: unknown, at: string): Snapshot["compose_worktree"] & object {
  const o = fields(value, at);
  return { new: bool(o.new, `${at}.new`), repo: str(o.repo, `${at}.repo`), branch: str(o.branch, `${at}.branch`) };
}

function composeTarget(value: unknown, at: string): Snapshot["compose_target"] & object {
  const o = fields(value, at);
  return {
    space: owner(o.space, `${at}.space`),
    name: orNull(str)(o.name, `${at}.name`),
    elsewhere: bool(o.elsewhere, `${at}.elsewhere`),
    switch: orNull(rect)(o.switch, `${at}.switch`),
  };
}

function noticeLevel(value: unknown, at: string): NoticeLevel {
  if (value === "info" || value === "warning" || value === "meeting" || value === "error") return value;
  throw new Error(`${at}: not a level: ${JSON.stringify(value)}`);
}

function noticeUndo(value: unknown, at: string): { text: string; rect: Rect | null } {
  const o = fields(value, at);
  return { text: str(o.text, `${at}.text`), rect: orNull(rect)(o.rect, `${at}.rect`) };
}

function toastCard(value: unknown, at: string): ToastCard {
  const o = fields(value, at);
  return {
    kind: str(o.kind, `${at}.kind`),
    level: noticeLevel(o.level, `${at}.level`),
    text: str(o.text, `${at}.text`),
    offer: orNull(str)(o.offer, `${at}.offer`),
    offer_rect: orNull(rect)(o.offer_rect, `${at}.offer_rect`),
    close: orNull(rect)(o.close, `${at}.close`),
    new_task: orNull(rect)(o.new_task, `${at}.new_task`),
    rect: orNull(rect)(o.rect, `${at}.rect`),
  };
}

function toast(value: unknown, at: string): Toast {
  const o = fields(value, at);
  return {
    cards: list(toastCard)(o.cards, `${at}.cards`),
    more: num(o.more, `${at}.more`),
    rect: orNull(rect)(o.rect, `${at}.rect`),
    shown: num(o.shown, `${at}.shown`),
    panel: orNull(rect)(o.panel, `${at}.panel`),
  };
}

function statusBar(value: unknown, at: string): Snapshot["status_bar"] {
  const o = fields(value, at);
  const meeting = (v: unknown, p: string) => {
    const m = fields(v, p);
    return { rect: orNull(rect)(m.rect, `${p}.rect`), join: orNull(rect)(m.join, `${p}.join`) };
  };
  return {
    mode: orNull(rect)(o.mode, `${at}.mode`),
    resting: orNull(rect)(o.resting, `${at}.resting`),
    place: orNull(noticeUndo)(o.place, `${at}.place`),
    meeting: orNull(meeting)(o.meeting, `${at}.meeting`),
    agents: agentsCount(o.agents, `${at}.agents`),
    shrink: num(o.shrink, `${at}.shrink`),
  };
}

function agentsCount(value: unknown, at: string): Snapshot["status_bar"]["agents"] {
  const o = fields(value, at);
  return {
    ...noticeUndo(value, at),
    working: num(o.working, `${at}.working`),
    in_terminal: num(o.in_terminal, `${at}.in_terminal`),
    needs_you: num(o.needs_you, `${at}.needs_you`),
    terminal: orNull(rect)(o.terminal, `${at}.terminal`),
  };
}

function agentLine(value: unknown, at: string): AgentLine {
  const o = fields(value, at);
  const pair = (v: unknown, p: string): [string, string] => {
    const [name, text] = list(str)(v, p);
    if (name === undefined || text === undefined) throw new Error(`${p}: not a [name, value] pair`);
    return [name, text];
  };
  return {
    foot: orNull(str)(o.foot, `${at}.foot`),
    bypass: bool(o.bypass, `${at}.bypass`),
    folder: orNull(str)(o.folder, `${at}.folder`),
    facts: list(pair)(o.facts, `${at}.facts`),
  };
}

function spaceRow(value: unknown, at: string): SpaceRow {
  const o = fields(value, at);
  return {
    of: owner(o.of, `${at}.of`),
    // An older app's card has none.
    hidden: o.hidden === undefined ? false : bool(o.hidden, `${at}.hidden`),
    selected: bool(o.selected, `${at}.selected`),
    tabs: list(num)(o.tabs, `${at}.tabs`),
    tasks: list(num)(o.tasks, `${at}.tasks`),
    front: orNull(num)(o.front, `${at}.front`),
    worktree: orNull(str)(o.worktree, `${at}.worktree`),
    glyph: glyph(o.glyph, `${at}.glyph`),
    indicator: oneOf(["idle", "working", "needs_you"] as const, "a space's indicator")(o.indicator, `${at}.indicator`),
    kept: bool(o.kept, `${at}.kept`),
  };
}

function glyph(value: unknown, at: string): Glyph {
  const o = fields(value, at);
  switch (o.kind) {
    case "home":
    case "tasks":
    case "name":
      return { kind: o.kind };
    case "app":
      return { kind: "app", name: str(o.name, `${at}.name`) };
    default:
      throw new Error(`${at}.kind: not a glyph: ${JSON.stringify(o.kind)}`);
  }
}

function nativeChat(value: unknown, at: string): NativeChat {
  const o = fields(value, at);
  const t = fields(o.transcript, `${at}.transcript`);
  const entry = (value: unknown, at: string) => {
    const e = fields(value, at);
    return { ...e, kind: str(e.kind, `${at}.kind`) };
  };
  const form = (value: unknown, at: string) => {
    const f = fields(value, at);
    const question = (value: unknown, at: string) => {
      const q = fields(value, at);
      return {
        options: num(q.options, `${at}.options`),
        multi: bool(q.multi, `${at}.multi`),
        picked: list(num)(q.picked, `${at}.picked`),
        answered: bool(q.answered, `${at}.answered`),
      };
    };
    return {
      questions: list(question)(f.questions, `${at}.questions`),
      cursor: pair(f.cursor, `${at}.cursor`),
      tabs: num(f.tabs, `${at}.tabs`),
    };
  };
  const card = (value: unknown, at: string) => {
    const c = fields(value, at);
    return {
      tool: str(c.tool, `${at}.tool`),
      label: str(c.label, `${at}.label`),
      cut: bool(c.cut, `${at}.cut`),
      expanded: bool(c.expanded, `${at}.expanded`),
      len: num(c.len, `${at}.len`),
      ask: orNull(form)(c.ask, `${at}.ask`),
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
      not_understood: num(t.not_understood, `${at}.transcript.not_understood`),
    },
    replies: list(reply)(o.replies, `${at}.replies`),
    draft_chars: num(o.draft_chars, `${at}.draft_chars`),
    queued: num(o.queued, `${at}.queued`),
    images: num(o.images, `${at}.images`),
    held: bool(o.held, `${at}.held`),
    editing: bool(o.editing, `${at}.editing`),
    deny: bool(o.deny, `${at}.deny`),
    armed: bool(o.armed, `${at}.armed`),
    away: bool(o.away, `${at}.away`),
    details: bool(o.details, `${at}.details`),
    details_open: bool(o.details_open, `${at}.details_open`),
    unread_open: bool(o.unread_open, `${at}.unread_open`),
    dial: agentDial(o.dial, `${at}.dial`),
    working: orNull(chatWorking)(o.working, `${at}.working`),
    selection: chatSelection(o.selection, `${at}.selection`),
    btw: btw(o.btw, `${at}.btw`),
    asked_tags: num(o.asked_tags, `${at}.asked_tags`),
    context: orNull((value: unknown, at: string) => {
      const c = fields(value, at);
      return {
        items: num(c.items, `${at}.items`),
        via: oneOf(["server", "system_prompt", "nowhere"] as const, "where the guidance went")(c.via, `${at}.via`),
        tools: num(c.tools, `${at}.tools`),
        briefed: bool(c.briefed, `${at}.briefed`),
        folder: bool(c.folder, `${at}.folder`),
        open: bool(c.open, `${at}.open`),
      };
    })(o.context, `${at}.context`),
  };
}

function chatWorking(value: unknown, at: string): ChatWorking {
  const o = fields(value, at);
  return { verb: orNull(str)(o.verb, `${at}.verb`), shimmer: bool(o.shimmer, `${at}.shimmer`) };
}

function btw(value: unknown, at: string): Btw {
  const o = fields(value, at);
  return {
    offered: bool(o.offered, `${at}.offered`),
    open: bool(o.open, `${at}.open`),
    keys: bool(o.keys, `${at}.keys`),
    asked: num(o.asked, `${at}.asked`),
    answered: num(o.answered, `${at}.answered`),
    synthetic: num(o.synthetic, `${at}.synthetic`),
    failed: num(o.failed, `${at}.failed`),
    waiting: bool(o.waiting, `${at}.waiting`),
  };
}

function chatSelection(value: unknown, at: string): ChatSelection {
  const o = fields(value, at);
  const extent = orNull((v, a) => {
    const e = fields(v, a);
    return { rows: num(e.rows, `${a}.rows`), chars: num(e.chars, `${a}.chars`) };
  })(o.extent, `${at}.extent`);
  const text = (v: unknown, a: string): [string, Rect] => {
    if (!Array.isArray(v) || v.length !== 2) throw wrong(a, "[key, rect]", v);
    return [str(v[0], a), rect(v[1], a)];
  };
  return { extent, keys: bool(o.keys, `${at}.keys`), texts: orNull(list(text))(o.texts, `${at}.texts`) };
}

function station(value: unknown, at: string): Station {
  if (value === "you" || value === "model" || value === "memory" || value === "files") return value;
  if (value === "shell" || value === "network" || value === "mcp" || value === "agents") return value;
  if (value === "skills" || value === "hooks" || value === "other" || value === "apps") return value;
  throw wrong(at, "a station", value);
}

function pulse(value: unknown, at: string): AgentDial["pulse"] {
  if (value === "running" || value === "needs_you" || value === "idle" || value === "ended") return value;
  if (value === "error") return value;
  throw wrong(at, "a dial's state", value);
}

function agentChip(value: unknown, at: string): AgentChip {
  const o = fields(value, at);
  return {
    task: subject(o.task, `${at}.task`),
    space: num(o.space, `${at}.space`),
    state: pulse(o.state, `${at}.state`),
    line: str(o.line, `${at}.line`),
    rect: orNull(rect)(o.rect, `${at}.rect`),
    short: bool(o.short, `${at}.short`),
  };
}

function agentDial(value: unknown, at: string): AgentDial {
  const o = fields(value, at);
  const level = (value: unknown, at: string): AgentDial["level"] => {
    if (value === "calm" || value === "amber" || value === "red") return value;
    throw wrong(at, "calm, amber or red", value);
  };
  const spot = (value: unknown, at: string) => {
    const s = fields(value, at);
    return {
      station: station(s.station, `${at}.station`),
      calls: num(s.calls, `${at}.calls`),
      failed: num(s.failed, `${at}.failed`),
      failing: num(s.failing, `${at}.failing`),
      lit: bool(s.lit, `${at}.lit`),
      running: num(s.running, `${at}.running`),
    };
  };
  return {
    pulse: pulse(o.pulse, `${at}.pulse`),
    stations: list(spot)(o.stations, `${at}.stations`),
    context: orNull(num)(o.context, `${at}.context`),
    level: level(o.level, `${at}.level`),
    failed: bool(o.failed, `${at}.failed`),
    lit: bool(o.lit, `${at}.lit`),
    timed: bool(o.timed, `${at}.timed`),
    collapsed: bool(o.collapsed, `${at}.collapsed`),
    faces: num(o.faces, `${at}.faces`),
    faces_more: num(o.faces_more, `${at}.faces_more`),
    drilled: num(o.drilled, `${at}.drilled`),
    switcher: bool(o.switcher, `${at}.switcher`),
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
    rail: list(railRow)(o.rail, `${at}.rail`),
    width: num(o.width, `${at}.width`),
    collapsed: bool(o.collapsed, `${at}.collapsed`),
    agents: map(agent)(o.agents, `${at}.agents`),
    native: native(o.native, `${at}.native`),
    line: orNull(agentLine)(o.line, `${at}.line`),
    note: orNull(str)(o.note, `${at}.note`),
    finish: orNull(finish)(o.finish, `${at}.finish`),
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
    waits_for: orNull(oneOf(["permission", "question", "input", "turn"] as const, "what a session waits for"))(
      o.waits_for,
      `${at}.waits_for`,
    ),
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
    group: orNull(str)(o.group, `${at}.group`),
    header: orNull(rect)(o.header, `${at}.header`),
    folded: bool(o.folded, `${at}.folded`),
    dot: orNull(oneOf(["working", "needs_you"] as const, "a folded repository's dot"))(o.dot, `${at}.dot`),
    rows: list(checkoutRow)(o.rows, `${at}.rows`),
    name_cut: orNull(cutText)(o.name_cut, `${at}.name_cut`),
  };
}

function checkoutRow(value: unknown, at: string): CheckoutRow {
  const o = fields(value, at);
  return {
    path: str(o.path, `${at}.path`),
    branch: str(o.branch, `${at}.branch`),
    default: bool(o.default, `${at}.default`),
    label: str(o.label, `${at}.label`),
    pr: orNull(pullRequest)(o.pr, `${at}.pr`),
    activity: oneOf(["idle", "working", "needs_you"] as const, "a checkout's activity")(o.activity, `${at}.activity`),
    pulsing: bool(o.pulsing, `${at}.pulsing`),
    row: orNull(rect)(o.row, `${at}.row`),
  };
}

/** One of `kinds`, or a parse error. */
function oneOf<T extends string>(kinds: readonly T[], what: string): Parse<T> {
  return (value, at) => {
    if (typeof value === "string" && (kinds as readonly string[]).includes(value)) return value as T;
    throw wrong(at, what, value);
  };
}

function pullRequest(value: unknown, at: string): PullRequest {
  const o = fields(value, at);
  return {
    number: num(o.number, `${at}.number`),
    draft: bool(o.draft, `${at}.draft`),
    look: oneOf(["failing", "draft", "pending", "ready", "open"] as const, "a pull request's look")(o.look, `${at}.look`),
    checks: oneOf(["none", "passing", "running", "failing"] as const, "checks")(o.checks, `${at}.checks`),
    failed: num(o.failed, `${at}.failed`),
    review: oneOf(["none", "approved", "changes_requested", "required"] as const, "a review")(o.review, `${at}.review`),
    merge: oneOf(["clean", "conflicts", "behind", "blocked", "unknown"] as const, "a merge state")(o.merge, `${at}.merge`),
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

function projectsBar(value: unknown, at: string): ProjectsBar {
  const o = fields(value, at);
  return { on: bool(o.on, `${at}.on`), rows: list(projectRow)(o.rows, `${at}.rows`) };
}

function projectRow(value: unknown, at: string): ProjectRow {
  const o = fields(value, at);
  return {
    placed: bool(o.placed, `${at}.placed`),
    file: bool(o.file, `${at}.file`),
    note: bool(o.note, `${at}.note`),
    modified: num(o.modified, `${at}.modified`),
    used: orNull(num)(o.used, `${at}.used`),
    row: orNull(rect)(o.row, `${at}.row`),
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
    return { text: str(d.text, `${at}.text`), path: str(d.path, `${at}.path`), pinned: bool(d.pinned, `${at}.pinned`) };
  };
  return { dirs: list(dir)(o.dirs, `${at}.dirs`), fixed: bool(o.fixed, `${at}.fixed`) };
}

function pickItem(value: unknown, at: string): PickItem {
  const o = fields(value, at);
  if (o.type === "terminal") return { type: "terminal" };
  if (o.type === "file") return { type: "file" };
  if (o.type === "app") return { type: "app", name: str(o.name, `${at}.name`) };
  if (o.type === "new_window") {
    const bundle = orNull(str)(o.bundle, `${at}.bundle`);
    return { type: "new_window", app: str(o.app, `${at}.app`), bundle, menu_item: bool(o.menu_item, `${at}.menu_item`) };
  }
  if (o.type === "window") {
    const pane = orNull(num)(o.pane, `${at}.pane`);
    return { type: "window", app: str(o.app, `${at}.app`), window: num(o.window, `${at}.window`), pane };
  }
  if (o.type === "closed") {
    return { type: "closed", name: str(o.name, `${at}.name`), path: str(o.path, `${at}.path`) };
  }
  if (o.type === "sound") {
    return { type: "sound", name: str(o.name, `${at}.name`), playing: bool(o.playing, `${at}.playing`) };
  }
  throw wrong(`${at}.type`, "a picker item", o.type);
}

function field(value: unknown, at: string): Field {
  const o = fields(value, at);
  const owners = ["compose", "picker", "palette", "clone", "inspector", "apps", "mark", "workbook", "tab", "worktree", "note", "talk", "btw", "bug", "slot"];
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

function newWindow(value: unknown, at: string): { app: string; since: number } {
  const o = fields(value, at);
  return { app: str(o.app, `${at}.app`), since: num(o.since, `${at}.since`) };
}

function picker(value: unknown, at: string): Picker {
  const o = fields(value, at);
  return {
    axis: axis(o.axis, `${at}.axis`),
    new_tab: bool(o.new_tab, `${at}.new_tab`),
    items: list(pickItem)(o.items, `${at}.items`),
    query: str(o.query, `${at}.query`),
    selected: num(o.selected, `${at}.selected`),
    transcribe: bool(o.transcribe, `${at}.transcribe`),
    app: orNull(str)(o.app, `${at}.app`),
  };
}

function palette(value: unknown, at: string): Palette {
  const o = fields(value, at);
  return {
    query: str(o.query, `${at}.query`),
    selected: num(o.selected, `${at}.selected`),
    tasks: orNull(list(paletteTask))(o.tasks, `${at}.tasks`),
    features: orNull(list(paletteFeature))(o.features, `${at}.features`),
  };
}

function paletteFeature(value: unknown, at: string): PaletteFeature {
  const o = fields(value, at);
  return { feature: feature(o.feature, `${at}.feature`), on: bool(o.on, `${at}.on`) };
}

function feature(value: unknown, at: string): Feature {
  if (value === "marks" || value === "apps_section" || value === "transcripts" || value === "slots" || value === "agent_mode") return value;
  if (value === "multi_monitor" || value === "review" || value === "city_behind" || value === "city_wall_names") return value;
  if (value === "overlay_panels" || value === "decisions" || value === "code_city" || value === "city_live_layer") return value;
  if (value === "session_waits" || value === "projects" || value === "terminal_tasks" || value === "zoom_out" || value === "decision_sources") return value;
  throw wrong(at, "a feature", value);
}

function paletteTask(value: unknown, at: string): PaletteTask {
  const o = fields(value, at);
  return { to: owner(o.to, `${at}.to`), title: str(o.title, `${at}.title`) };
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

function editor(value: unknown, at: string): Editor {
  const o = fields(value, at);
  return {
    target: str(o.target, `${at}.target`),
    keys: bool(o.keys, `${at}.keys`),
    shown: bool(o.shown, `${at}.shown`),
    running: bool(o.running, `${at}.running`),
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
    description: orNull(str)(o.description, `${at}.description`),
    stage: orNull(str)(o.stage, `${at}.stage`),
    on: orNull(bool)(o.on, `${at}.on`),
    rect: orNull(rect)(o.rect, `${at}.rect`),
    group_rect: orNull(rect)(o.group_rect, `${at}.group_rect`),
    stage_rect: orNull(rect)(o.stage_rect, `${at}.stage_rect`),
    text_rect: orNull(rect)(o.text_rect, `${at}.text_rect`),
  };
}

function goToRow(value: unknown, at: string): GoToRow {
  const o = fields(value, at);
  return {
    place: orNull(str)(o.place, `${at}.place`),
    name: orNull(str)(o.name, `${at}.name`),
    detail: orNull(str)(o.detail, `${at}.detail`),
    file: orNull(str)(o.file, `${at}.file`),
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

function amount(value: unknown, at: string): Amount {
  const o = fields(value, at);
  return { lines: num(o.lines, `${at}.lines`), chars: num(o.chars, `${at}.chars`), newline: bool(o.newline, `${at}.newline`) };
}

function clip(value: unknown, at: string): Clip {
  const o = fields(value, at);
  const verb = o.verb;
  if (verb !== "copied" && verb !== "cut" && verb !== "pasted") throw wrong(`${at}.verb`, "copied, cut or pasted", verb);
  return {
    pane: orNull(num)(o.pane, `${at}.pane`),
    verb,
    amount: amount(o.amount, `${at}.amount`),
    text: str(o.text, `${at}.text`),
    at: num(o.at, `${at}.at`),
  };
}

function confirm(value: unknown, at: string): Confirm {
  const o = fields(value, at);
  return {
    running: list(running)(o.running, `${at}.running`),
    dismiss: orNull(num)(o.dismiss, `${at}.dismiss`),
    unlist: orNull(str)(o.unlist, `${at}.unlist`),
    close: orNull(num)(o.close, `${at}.close`),
    later: orNull(later)(o.later, `${at}.later`),
    edited: orNull(num)(o.edited, `${at}.edited`),
    delete: orNull(str)(o.delete, `${at}.delete`),
    agents: list(str)(o.agents, `${at}.agents`),
    unsent: list(unsent)(o.unsent, `${at}.unsent`),
  };
}

function unsent(value: unknown, at: string): Unsent {
  const o = fields(value, at);
  const kinds = ["message", "answer", "btw", "new_task", "capture", "editor"] as const;
  return { kind: oneOf(kinds, "a draft's kind")(o.kind, `${at}.kind`), place: str(o.place, `${at}.place`) };
}

function made(value: unknown, at: string): { repo: string; branch: string; path: string; asked: string; root: string; removed: boolean } {
  const o = fields(value, at);
  return {
    repo: str(o.repo, `${at}.repo`),
    branch: str(o.branch, `${at}.branch`),
    path: str(o.path, `${at}.path`),
    asked: str(o.asked, `${at}.asked`),
    root: str(o.root, `${at}.root`),
    removed: bool(o.removed, `${at}.removed`),
  };
}

function later(value: unknown, at: string): [number, string] {
  if (!Array.isArray(value) || value.length !== 2) throw wrong(at, "[id, title]", value);
  return [num(value[0], `${at}[0]`), str(value[1], `${at}[1]`)];
}

function finish(value: unknown, at: string): Finish {
  const o = fields(value, at);
  const r = fields(o.rects, `${at}.rects`);
  const at2 = `${at}.rects`;
  return {
    later: bool(o.later, `${at}.later`),
    done: bool(o.done, `${at}.done`),
    offered: bool(o.offered, `${at}.offered`),
    rects: {
      later: orNull(rect)(r.later, `${at2}.later`),
      done: orNull(rect)(r.done, `${at2}.done`),
      offer_done: orNull(rect)(r.offer_done, `${at2}.offer_done`),
      offer_later: orNull(rect)(r.offer_later, `${at2}.offer_later`),
    },
  };
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
  if (o.type === "tab") return { type: "tab", tab: num(o.tab, `${at}.tab`), text: str(o.text, `${at}.text`) };
  if (o.type === "worktree") return { type: "worktree", repo: str(o.repo, `${at}.repo`), text: str(o.text, `${at}.text`) };
  throw wrong(`${at}.type`, "what a mark waits for", o.type);
}

function markKind(value: unknown, at: string): "terminal" | "app" | "tab" | "file" | "explorer" {
  if (value === "terminal" || value === "app" || value === "tab" || value === "file" || value === "explorer") return value;
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

function closingWorkbook(value: unknown, at: string): { id: number; name: string; undo: Rect | null } {
  const o = fields(value, at);
  return { id: num(o.id, `${at}.id`), name: str(o.name, `${at}.name`), undo: orNull(rect)(o.undo, `${at}.undo`) };
}

function workbook(value: unknown, at: string): Workbook {
  const o = fields(value, at);
  const buttons = fields(o.buttons, `${at}.buttons`);
  const pane = (value: unknown, at: string): WorkbookPane => {
    const p = fields(value, at);
    const kind = p.kind;
    if (kind !== "terminal" && kind !== "app" && kind !== "empty" && kind !== "file" && kind !== "explorer") {
      throw wrong(`${at}.kind`, "a pane's kind", kind);
    }
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
      buttons: { menu: orNull(rect)(words.menu, `${at}.buttons.menu`) },
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
    task: orNull(num)(o.task, `${at}.task`),
    row: orNull(rect)(o.row, `${at}.row`),
    buttons: {
      fold: orNull(rect)(buttons.fold, `${at}.buttons.fold`),
      menu: orNull(rect)(buttons.menu, `${at}.buttons.menu`),
    },
  };
}

function cloneDrawn(value: unknown, at: string): CloneDrawn {
  const o = fields(value, at);
  const dialog = (value: unknown, at: string): CloneDrawn["dialog"] => {
    const d = fields(value, at);
    return {
      rows: list(str)(d.rows, `${at}.rows`),
      selected: num(d.selected, `${at}.selected`),
      destination: orNull(str)(d.destination, `${at}.destination`),
      said: orNull(str)(d.said, `${at}.said`),
    };
  };
  const job = (value: unknown, at: string): CloneDrawn["job"] => {
    const j = fields(value, at);
    return {
      repo: str(j.repo, `${at}.repo`),
      dest: str(j.dest, `${at}.dest`),
      line: str(j.line, `${at}.line`),
      cancelled: bool(j.cancelled, `${at}.cancelled`),
    };
  };
  return { dialog: orNull(dialog)(o.dialog, `${at}.dialog`), job: orNull(job)(o.job, `${at}.job`) };
}

function rowMenu(value: unknown, at: string): RowMenu {
  const o = fields(value, at);
  const item = (value: unknown, at: string): RowMenu["items"][number] => {
    const i = fields(value, at);
    const kind = i.item;
    const kinds = [
      "rename", "delete", "remove", "keep", "pause", "resume", "delete_worktree", "local_folder", "from_github",
      "slot_keep", "slot_edit", "slot_look_again", "slot_give_back", "slot_swap", "slot_resort", "slot_clear",
      "close", "fold_out", "explorer_here", "open", "open_diff", "open_below", "reveal", "copy_path", "terminal_here",
    ] as const;
    const known = kinds.find((k) => k === kind);
    if (known === undefined) throw wrong(`${at}.item`, "a menu item", kind);
    return {
      item: known,
      label: str(i.label, `${at}.label`),
      why: orNull(str)(i.why, `${at}.why`),
      rect: orNull(rect)(i.rect, `${at}.rect`),
    };
  };
  return {
    workbook: orNull(num)(o.workbook, `${at}.workbook`),
    tab: orNull(num)(o.tab, `${at}.tab`),
    space: bool(o.space, `${at}.space`),
    transcript: bool(o.transcript, `${at}.transcript`),
    worktree: orNull(str)(o.worktree, `${at}.worktree`),
    pane: orNull(num)(o.pane, `${at}.pane`),
    open_tab: orNull(num)(o.open_tab, `${at}.open_tab`),
    explorer: orNull(num)(o.explorer, `${at}.explorer`),
    selected: orNull(num)(o.selected, `${at}.selected`),
    items: list(item)(o.items, `${at}.items`),
  };
}

function workbookOffer(value: unknown, at: string): "deleted" | "updated" | "closed" {
  if (value === "deleted" || value === "updated" || value === "closed") return value;
  throw wrong(at, "deleted, updated or closed", value);
}

function copyAsk(value: unknown, at: string): CopyAsk {
  const o = fields(value, at);
  return {
    id: num(o.id, `${at}.id`),
    at: num(o.at, `${at}.at`),
    chars: num(o.chars, `${at}.chars`),
    too_large: bool(o.too_large, `${at}.too_large`),
  };
}

function model(value: unknown, at: string): Model {
  const o = fields(value, at);
  return {
    panes: list(pane)(o.panes, `${at}.panes`),
    copy_asks: map(copyAsk)(o.copy_asks, `${at}.copy_asks`),
    copy_always: list(num)(o.copy_always, `${at}.copy_always`),
    tabs: list(tab)(o.tabs, `${at}.tabs`),
    active: orNull(num)(o.active, `${at}.active`),
    layout: orNull(layout)(o.layout, `${at}.layout`),
    focused: orNull(num)(o.focused, `${at}.focused`),
    live: orNull(num)(o.live, `${at}.live`),
    fill_display: bool(o.fill_display, `${at}.fill_display`),
    sidebar: orNull(section)(o.sidebar, `${at}.sidebar`),
    compose: orNull(str)(o.compose, `${at}.compose`),
    list: orNull(num)(o.list, `${at}.list`),
    rail_focus: orNull(railFocus)(o.rail_focus, `${at}.rail_focus`),
    repos: list(repo)(o.repos, `${at}.repos`),
    repo_dirs: repoDirs(o.repo_dirs, `${at}.repo_dirs`),
    repos_open: list(str)(o.repos_open, `${at}.repos_open`),
    tasks: { list: list(task)(fields(o.tasks, `${at}.tasks`).list, `${at}.tasks.list`) },
    collapsed: list(heading)(o.collapsed, `${at}.collapsed`),
    tasks_view: tasksView(o.tasks_view, `${at}.tasks_view`),
    folded_projects: list(orNull(str))(o.folded_projects, `${at}.folded_projects`),
    open_done: list(orNull(str))(o.open_done, `${at}.open_done`),
    picker: orNull(picker)(o.picker, `${at}.picker`),
    new_window: orNull(newWindow)(o.new_window, `${at}.new_window`),
    edited: list(num)(o.edited, `${at}.edited`),
    palette: orNull(palette)(o.palette, `${at}.palette`),
    inspector: orNull(inspectorState)(o.inspector, `${at}.inspector`),
    confirm: orNull(confirm)(o.confirm, `${at}.confirm`),
    reorder: orNull(reorder)(o.reorder, `${at}.reorder`),
    clip: orNull(clip)(o.clip, `${at}.clip`),
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
    features: list(feature)(o.features, `${at}.features`),
    listen: orNull(listen)(o.listen, `${at}.listen`),
    transcripts: transcripts(o.transcripts, `${at}.transcripts`),
    agent_mode: agentMode(o.agent_mode, `${at}.agent_mode`),
    slots: map(slot)(o.slots, `${at}.slots`),
    offers: list(offer)(o.offers, `${at}.offers`),
    offer_list: orNull(num)(o.offer_list, `${at}.offer_list`),
    slot_dialog: orNull(slotDialog)(o.slot_dialog, `${at}.slot_dialog`),
    viewers: map(viewer)(o.viewers, `${at}.viewers`),
    // Absent from an older build's reply: none open.
    decisions: o.decisions === undefined ? null : orNull(decisions)(o.decisions, `${at}.decisions`),
    // Absent from an older build's reply: not open.
    focus_edit: o.focus_edit === undefined ? null : orNull(focusEdit)(o.focus_edit, `${at}.focus_edit`),
    // Absent from an older build's reply: not zoomed out.
    zoom: o.zoom === undefined ? null : orNull(zoom)(o.zoom, `${at}.zoom`),
  };
}

function zoom(value: unknown, at: string): Zoom {
  const o = fields(value, at);
  const camera = fields(o.camera, `${at}.camera`);
  const [x, y, ...more] = list(num)(camera.center, `${at}.camera.center`);
  if (x === undefined || y === undefined || more.length > 0) throw wrong(`${at}.camera.center`, "two numbers", camera.center);
  return {
    from: num(o.from, `${at}.from`),
    selected: num(o.selected, `${at}.selected`),
    camera: { scale: num(camera.scale, `${at}.camera.scale`), center: [x, y] },
  };
}

function decisions(value: unknown, at: string): Decisions {
  const o = fields(value, at);
  const card = (value: unknown, at: string) => {
    const c = fields(value, at);
    const n = (key: string) => num(c[key], `${at}.${key}`);
    // `area`: absent from an app before the rank: none.
    const area = c.area === undefined ? null : orNull(num)(c.area, `${at}.area`);
    return { task: n("task"), ticket: n("ticket"), priority: n("priority"), since: n("since"), area };
  };
  return {
    tab: num(o.tab, `${at}.tab`),
    pane: num(o.pane, `${at}.pane`),
    task: orNull(num)(o.task, `${at}.task`),
    ticket: orNull(num)(o.ticket, `${at}.ticket`),
    sort: orNull(decisionSorting)(o.sort, `${at}.sort`),
    week: o.week === undefined ? null : orNull(weekCard)(o.week, `${at}.week`),
    changed: o.changed === undefined ? false : bool(o.changed, `${at}.changed`),
    at: num(o.at, `${at}.at`),
    of: num(o.of, `${at}.of`),
    queue: list(card)(o.queue, `${at}.queue`),
  };
}

function decisionSorting(value: unknown, at: string): DecisionSorting {
  const o = fields(value, at);
  return {
    item: orNull(num)(o.item, `${at}.item`),
    claims: bool(o.claims, `${at}.claims`),
    role: orNull(oneOf(["core", "ancillary"] as const, "a role"))(o.role, `${at}.role`),
    done: num(o.done, `${at}.done`),
    left: num(o.left, `${at}.left`),
    revision: num(o.revision, `${at}.revision`),
  };
}

function slotDialog(value: unknown, at: string): SlotDialog {
  const o = fields(value, at);
  const pick = (value: unknown, at: string): SlotDialog["pick"] => {
    if (value === "largest" || value === "smallest" || value === "any") return value;
    return { display: num(fields(value, at).display, `${at}.display`) };
  };
  return {
    pane: num(o.pane, `${at}.pane`),
    has_id: bool(o.has_id, `${at}.has_id`),
    by: oneOf(["any", "id", "title"] as const, "a slot rule's window")(o.by, `${at}.by`),
    test: oneOf(["is", "contains", "starts_with", "pattern"] as const, "a title test")(o.test, `${at}.test`),
    has_kind: bool(o.has_kind, `${at}.has_kind`),
    pick: pick(o.pick, `${at}.pick`),
    matching: oneOf(["manual", "automatic"] as const, "a slot's matching")(o.matching, `${at}.matching`),
    editing: bool(o.editing, `${at}.editing`),
  };
}

function offer(value: unknown, at: string): Offer {
  const o = fields(value, at);
  return { pane: num(o.pane, `${at}.pane`), window: num(o.window, `${at}.window`) };
}

function slot(value: unknown, at: string): Slot {
  const o = fields(value, at);
  return {
    status: oneOf(["absent", "empty", "offered", "filled", "away"] as const, "a slot's status")(o.status, `${at}.status`),
    matches: num(o.matches, `${at}.matches`),
    matching: oneOf(["manual", "automatic"] as const, "a slot's matching")(o.matching, `${at}.matching`),
  };
}

const agentLevel = oneOf(["off", "see", "act"] as const, "an agent mode level");

/** An app pane's agent mode grant: its level and the task it is given to. */
function grant(value: unknown, at: string): AgentGrant {
  const g = fields(value, at);
  return { level: agentLevel(g.level, `${at}.level`), task: orNull(num)(g.task, `${at}.task`) };
}

function agentMode(value: unknown, at: string): AgentMode {
  const o = fields(value, at);
  const panes: AgentMode["panes"] = {};
  for (const [id, given] of Object.entries(fields(o.panes, `${at}.panes`))) {
    panes[id] = grant(given, `${at}.panes.${id}`);
  }
  const menu = (value: unknown, at: string) => {
    const m = fields(value, at);
    return { pane: num(m.pane, `${at}.pane`), selected: orNull(num)(m.selected, `${at}.selected`) };
  };
  const ahead: AgentMode["ahead"] = {};
  for (const [id, picked] of Object.entries(fields(o.ahead, `${at}.ahead`))) {
    ahead[id] = { level: agentLevel(fields(picked, `${at}.ahead.${id}`).level, `${at}.ahead.${id}.level`) };
  }
  const open = orNull(menu)(o.menu, `${at}.menu`);
  return { panes, default: agentLevel(o.default, `${at}.default`), menu: open, ahead };
}

function transcripts(value: unknown, at: string): Snapshot["state"]["transcripts"] {
  const o = fields(value, at);
  const past = (value: unknown, at: string) => {
    const p = fields(value, at);
    return {
      name: str(p.name, `${at}.name`),
      started_ms: num(p.started_ms, `${at}.started_ms`),
      length_ms: num(p.length_ms, `${at}.length_ms`),
    };
  };
  return { today: list(past)(o.today, `${at}.today`), open: orNull(str)(o.open, `${at}.open`) };
}

function listen(value: unknown, at: string): Listen {
  const o = fields(value, at);
  const source = fields(o.source, `${at}.source`);
  const from = source.app === true ? { app: true as const } : { pane: num(source.pane, `${at}.source.pane`) };
  return {
    status: oneOf(["starting", "on", "paused"] as const, "a transcript's status")(o.status, `${at}.status`),
    source: from,
    mic: bool(o.mic, `${at}.mic`),
    auto: bool(o.auto, `${at}.auto`),
    started_ms: num(o.started_ms, `${at}.started_ms`),
    paused_ms: num(o.paused_ms, `${at}.paused_ms`),
    paused_at_ms: orNull(num)(o.paused_at_ms, `${at}.paused_at_ms`),
  };
}

function listenLine(value: unknown, at: string): ListenLine {
  const o = fields(value, at);
  if (o.paused_ms !== undefined) return { paused_ms: num(o.paused_ms, `${at}.paused_ms`) };
  return {
    from: oneOf(["them", "me"] as const, "a speaker")(o.from, `${at}.from`),
    final: bool(o.final, `${at}.final`),
    text: str(o.text, `${at}.text`),
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
    selection_range: orNull(range)(o.selection_range, `${at}.selection_range`),
    visual_cursor: orNull(pair)(o.visual_cursor, `${at}.visual_cursor`),
    styled: orNull(list(styledSpan))(o.styled, `${at}.styled`),
  };
}

function styledSpan(value: unknown, at: string): StyledSpan {
  const o = fields(value, at);
  return {
    row: num(o.row, `${at}.row`),
    text: str(o.text, `${at}.text`),
    fg: num(o.fg, `${at}.fg`),
    bg: orNull(num)(o.bg, `${at}.bg`),
    bold: bool(o.bold, `${at}.bold`),
    italic: bool(o.italic, `${at}.italic`),
    strike: bool(o.strike, `${at}.strike`),
    underline: orNull(underline)(o.underline, `${at}.underline`),
  };
}

function underline(value: unknown, at: string): { curly: boolean; color: number | null } {
  const o = fields(value, at);
  return { curly: bool(o.curly, `${at}.curly`), color: orNull(num)(o.color, `${at}.color`) };
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

function sneakTarget(value: unknown, at: string): SneakTarget {
  return { ...axTarget(value, at), pill: frame(fields(value, at).pill, `${at}.pill`) };
}

function sneak(value: unknown, at: string): Sneak {
  const o = fields(value, at);
  return {
    targets: list(sneakTarget)(o.targets, `${at}.targets`),
    typed: str(o.typed, `${at}.typed`),
    app: orNull(sneakApp)(o.app, `${at}.app`),
  };
}

function appPress(value: unknown, at: string): [PaneId, number[]] {
  if (!Array.isArray(value) || value.length !== 2) throw wrong(at, "[pane, path]", value);
  return [num(value[0], `${at}[0]`), list(num)(value[1], `${at}[1]`)];
}

function keyReference(value: unknown, at: string): KeyReference {
  const o = fields(value, at);
  const group = (g: unknown, where: string) => {
    const f = fields(g, where);
    return { group: str(f.group, `${where}.group`), rows: list(str)(f.rows, `${where}.rows`) };
  };
  return {
    groups: list(group)(o.groups, `${at}.groups`),
    known: list(str)(o.known, `${at}.known`),
    known_open: bool(o.known_open, `${at}.known_open`),
    known_header: orNull(rect)(o.known_header, `${at}.known_header`),
  };
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

function overlays(value: unknown, at: string): { sidebar: Rect | null; drawer: Rect | null; box: Rect | null } {
  const o = fields(value, at);
  return {
    sidebar: orNull(rect)(o.sidebar, `${at}.sidebar`),
    drawer: orNull(rect)(o.drawer, `${at}.drawer`),
    box: orNull(rect)(o.box, `${at}.box`),
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

function cityOn(value: unknown, at: string): CityOn {
  if (value === "tab") return value;
  const o = fields(value, at);
  if ("pane" in o) return { pane: num(o.pane, `${at}.pane`) };
  return { out: num(o.out, `${at}.out`) };
}

function cityDrawn(value: unknown, at: string): CityDrawn {
  const o = fields(value, at);
  return {
    on: cityOn(o.on, `${at}.on`),
    pictures: num(o.pictures, `${at}.pictures`),
    small: list(cityOn)(o.small, `${at}.small`),
    places: orNull(num)(o.places, `${at}.places`),
    groups: orNull(num)(o.groups, `${at}.groups`),
    picture: orNull(pair)(o.picture, `${at}.picture`),
    bounds: rect(o.bounds, `${at}.bounds`),
    scale: num(o.scale, `${at}.scale`),
    lit: list(rect)(o.lit, `${at}.lit`),
    lit_tops: list(point)(o.lit_tops, `${at}.lit_tops`),
    lit_beams: list(segment)(o.lit_beams, `${at}.lit_beams`),
    labels: num(o.labels, `${at}.labels`),
    names: list(cityName)(o.names, `${at}.names`),
    spots: orNull(citySpots)(o.spots, `${at}.spots`),
    failed: orNull(str)(o.failed, `${at}.failed`),
    reads: num(o.reads, `${at}.reads`),
    renders: num(o.renders, `${at}.renders`),
    paints: num(o.paints, `${at}.paints`),
    render_us: orNull(num)(o.render_us, `${at}.render_us`),
    glass: orNull(bool)(o.glass, `${at}.glass`),
    lower: orNull(num)(o.lower, `${at}.lower`),
    folder_colors: orNull(num)(o.folder_colors, `${at}.folder_colors`),
    hovered: orNull(cityHovered)(o.hovered, `${at}.hovered`),
    first_ms: orNull(num)(o.first_ms, `${at}.first_ms`),
    read_us: orNull(num)(o.read_us, `${at}.read_us`),
    view: cityView(o.view, `${at}.view`),
    grabbed: bool(o.grabbed, `${at}.grabbed`),
    spin: citySpin(o.spin, `${at}.spin`),
    agents: list(cityAgent)(o.agents, `${at}.agents`),
    glimmer: orNull(cityGlimmer)(o.glimmer, `${at}.glimmer`),
    behind: orNull(list(cityBehind))(o.behind, `${at}.behind`),
  };
}

function cityBehind(value: unknown, at: string): CityBehind {
  const o = fields(value, at);
  return { pane: num(o.pane, `${at}.pane`), opacity: num(o.opacity, `${at}.opacity`) };
}

function citySpin(value: unknown, at: string): CitySpin {
  const o = fields(value, at);
  return { running: bool(o.running, `${at}.running`), result: orNull(citySpun)(o.result, `${at}.result`) };
}

function citySpun(value: unknown, at: string): CitySpun {
  const o = fields(value, at);
  const n = (key: string) => num(o[key], `${at}.${key}`);
  return {
    steps: n("steps"),
    done: bool(o.done, `${at}.done`),
    frames: n("frames"),
    mean: n("mean"),
    p50: n("p50"),
    p95: n("p95"),
    p99: n("p99"),
    max: n("max"),
    over_16: n("over_16"),
    over_33: n("over_33"),
    repeats: n("repeats"),
    wait_p50: n("wait_p50"),
    wait_p95: n("wait_p95"),
    wait_max: n("wait_max"),
    shown_p50: n("shown_p50"),
    shown_p95: n("shown_p95"),
    shown_p99: n("shown_p99"),
    render_p50: n("render_p50"),
    render_p95: n("render_p95"),
    live: bool(o.live, `${at}.live`),
    boxes: n("boxes"),
  };
}

function typingState(value: unknown, at: string): TypingState {
  const o = fields(value, at);
  return { running: bool(o.running, `${at}.running`), result: orNull(typed)(o.result, `${at}.result`) };
}

function typingDist(value: unknown, at: string): TypingDist {
  const o = fields(value, at);
  const n = (key: string) => num(o[key], `${at}.${key}`);
  return { n: n("n"), p50: n("p50"), p95: n("p95"), p99: n("p99"), max: n("max") };
}

function typed(value: unknown, at: string): Typed {
  const o = fields(value, at);
  const d = (key: string) => typingDist(o[key], `${at}.${key}`);
  return {
    keys: num(o.keys, `${at}.keys`),
    missed: num(o.missed, `${at}.missed`),
    key_write: d("key_write"),
    write_echo: d("write_echo"),
    echo_taken: d("echo_taken"),
    taken_painted: d("taken_painted"),
    app: d("app"),
    total: d("total"),
    full: bool(o.full, `${at}.full`),
    busy: bool(o.busy, `${at}.busy`),
    done: bool(o.done, `${at}.done`),
    hz: num(o.hz, `${at}.hz`),
  };
}

function cityGlimmer(value: unknown, at: string): CityGlimmer {
  const o = fields(value, at);
  return { flash: num(o.flash, `${at}.flash`), sweep: num(o.sweep, `${at}.sweep`), found: num(o.found, `${at}.found`) };
}

function cityAgent(value: unknown, at: string): CityAgent {
  const o = fields(value, at);
  return {
    task: orNull(num)(o.task, `${at}.task`),
    rect: rect(o.rect, `${at}.rect`),
    cone: bool(o.cone, `${at}.cone`),
    fan: pair(o.fan, `${at}.fan`),
    hovered: bool(o.hovered, `${at}.hovered`),
    pulsing: bool(o.pulsing, `${at}.pulsing`),
    subagents: num(o.subagents, `${at}.subagents`),
    station: orNull(station)(o.station, `${at}.station`),
    caption: orNull(rect)(o.caption, `${at}.caption`),
  };
}

function cityView(value: unknown, at: string): CityView {
  const o = fields(value, at);
  return {
    yaw: num(o.yaw, `${at}.yaw`),
    pitch: num(o.pitch, `${at}.pitch`),
    zoom: num(o.zoom, `${at}.zoom`),
    pan: pair(o.pan, `${at}.pan`),
    lift: num(o.lift, `${at}.lift`),
  };
}

function cityHovered(value: unknown, at: string): CityHovered {
  const o = fields(value, at);
  return {
    file: bool(o.file, `${at}.file`),
    lit: bool(o.lit, `${at}.lit`),
    beam: orNull(segment)(o.beam, `${at}.beam`),
    outline: orNull(list(point))(o.outline, `${at}.outline`),
    beacon: rect(o.beacon, `${at}.beacon`),
  };
}

function citySpots(value: unknown, at: string): CitySpots {
  const o = fields(value, at);
  return { folder: orNull(point)(o.folder, `${at}.folder`), file: orNull(point)(o.file, `${at}.file`) };
}

function cityName(value: unknown, at: string): CityName {
  const o = fields(value, at);
  return {
    tier: oneOf(["wall", "folder", "file"] as const, "a city name's tier")(o.tier, `${at}.tier`),
    rect: rect(o.rect, `${at}.rect`),
    plinth: bool(o.plinth, `${at}.plinth`),
    hidden: bool(o.hidden, `${at}.hidden`),
  };
}

/** A line from one point to another: `[x0, y0, x1, y1]`. */
function segment(value: unknown, at: string): [number, number, number, number] {
  if (!Array.isArray(value) || value.length !== 4) throw wrong(at, "[x0, y0, x1, y1]", value);
  return [num(value[0], at), num(value[1], at), num(value[2], at), num(value[3], at)];
}

function homeDrawn(value: unknown, at: string): HomeDrawn {
  const o = fields(value, at);
  return {
    area: rect(o.area, `${at}.area`),
    panel: rect(o.panel, `${at}.panel`),
    nodes: map(rect)(o.nodes, `${at}.nodes`),
    heading: orNull(str)(o.heading, `${at}.heading`),
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
    titles: map(str)(o.titles, "titles"),
    tabs: list(orNull(rect))(o.tabs, "tabs"),
    space_rects: list(orNull(rect))(o.space_rects, "space_rects"),
    reorder_bar: orNull(rect)(o.reorder_bar, "reorder_bar"),
    agent_chip: orNull(agentChip)(o.agent_chip, "agent_chip"),
    strip: orNull(rect)(o.strip, "strip"),
    bar_header: orNull(rect)(o.bar_header, "bar_header"),
    overlays: overlays(o.overlays, "overlays"),
    tasks_dot: orNull(rect)(o.tasks_dot, "tasks_dot"),
    zoom_tiles: o.zoom_tiles === undefined ? null : orNull(map(rect))(o.zoom_tiles, "zoom_tiles"),
    drag: orNull(drag)(o.drag, "drag"),
    shown: map(rect)(o.shown, "shown"),
    tab_panes: list(list(num))(o.tab_panes, "tab_panes"),
    window: rect(o.window, "window"),
    window_active: bool(o.window_active, "window_active"),
    displays: list(display)(o.displays, "displays"),
    out: list(tabOut)(o.out, "out"),
    windows: list(appWindow)(o.windows, "windows"),
    parking: pair(o.parking, "parking"),
    return_key: orNull(str)(o.return_key, "return_key"),
    version: str(o.version, "version"),
    font: { name: str(fields(o.font, "font").name, "font.name"), size: num(fields(o.font, "font").size, "font.size") },
    cell: pair(o.cell, "cell"),
    notice: orNull(str)(o.notice, "notice"),
    notice_level: orNull(noticeLevel)(o.notice_level, "notice_level"),
    toast: orNull(toast)(o.toast, "toast"),
    quit_rows: list(rect)(o.quit_rows, "quit_rows"),
    confirm_buttons: list(rect)(o.confirm_buttons, "confirm_buttons"),
    settings_errors: num(o.settings_errors, "settings_errors"),
    editor: orNull(editor)(o.editor, "editor"),
    hosted: map(rect)(o.hosted, "hosted"),
    crop: map(num)(o.crop, "crop"),
    terminals: map(terminal)(o.terminals, "terminals"),
    viewers: map(viewerDrawn)(o.viewers, "viewers"),
    explorers: map(explorerShape)(o.explorers, "explorers"),
    copied: orNull(str)(o.copied, "copied"),
    picker_rows: orNull(list(pickItem))(o.picker_rows, "picker_rows"),
    palette_rows: orNull(list(paletteRow))(o.palette_rows, "palette_rows"),
    goto_rows: orNull(list(goToRow))(o.goto_rows, "goto_rows"),
    clone: orNull(cloneDrawn)(o.clone, "clone"),
    inspector: orNull(inspector)(o.inspector, "inspector"),
    field: orNull(field)(o.field, "field"),
    inbox: list(inboxRow)(o.inbox, "inbox"),
    needs_you: num(o.needs_you, "needs_you"),
    compose_read: bool(o.compose_read, "compose_read"),
    compose_host: orNull(composeHost)(o.compose_host, "compose_host"),
    compose_centred: bool(o.compose_centred, "compose_centred"),
    quick_pills: orNull(list(pillKind))(o.quick_pills, "quick_pills"),
    compose_plan: orNull(composePlan)(o.compose_plan, "compose_plan"),
    compose_status: orNull(str)(o.compose_status, "compose_status"),
    compose_images: num(o.compose_images, "compose_images"),
    compose_hint: orNull(str)(o.compose_hint, "compose_hint"),
    compose_choices: orNull(composeChoices)(o.compose_choices, "compose_choices"),
    compose_picker: orNull(composeMenu)(o.compose_picker, "compose_picker"),
    quick_project: orNull(str)(o.quick_project, "quick_project"),
    compose_worktree: orNull(composeWorktree)(o.compose_worktree, "compose_worktree"),
    compose_target: orNull(composeTarget)(o.compose_target, "compose_target"),
    welcome: bool(o.welcome, "welcome"),
    compose_menu: orNull(composeMenu)(o.compose_menu, "compose_menu"),
    compose_menu_bounds: orNull(rect)(o.compose_menu_bounds, "compose_menu_bounds"),
    compose_tags: orNull(map(rect))(o.compose_tags, "compose_tags"),
    compose_rects: orNull(map(rect))(o.compose_rects, "compose_rects"),
    compose_folder: orNull(str)(o.compose_folder, "compose_folder"),
    compose_pointing: orNull(rect)(o.compose_pointing, "compose_pointing"),
    compose_note: orNull(str)(o.compose_note, "compose_note"),
    tasks_bar: tasksBar(o.tasks_bar, "tasks_bar"),
    tasks_progress: tasksProgress(o.tasks_progress, "tasks_progress"),
    sessions_bar: sessionsBar(o.sessions_bar, "sessions_bar"),
    repos_bar: list(reposGroup)(o.repos_bar, "repos_bar"),
    projects_bar: projectsBar(o.projects_bar, "projects_bar"),
    filed: orNull(filed)(o.filed, "filed"),
    // Absent from an older build's reply: no focus.
    week_focus: o.week_focus === undefined ? null : orNull(weekFocusState)(o.week_focus, "week_focus"),
    home: orNull(home)(o.home, "home"),
    home_drawn: homeDrawn(o.home_drawn, "home_drawn"),
    city: orNull(cityDrawn)(o.city, "city"),
    // Absent from an older build's reply.
    typing: o.typing === undefined ? null : orNull(typingState)(o.typing, "typing"),
    closed_apps: list(str)(o.closed_apps, "closed_apps"),
    apps_bar: appsBar(o.apps_bar, "apps_bar"),
    apps: list(appEntry)(o.apps, "apps"),
    ax_overlay: orNull(axOverlay)(o.ax_overlay, "ax_overlay"),
    sneak: orNull(sneak)(o.sneak, "sneak"),
    counts: orNull(map(num))(o.counts, "counts"),
    app_presses: orNull(list(appPress))(o.app_presses, "app_presses"),
    keys_known: orNull(list(str))(o.keys_known, "keys_known"),
    key_reference: orNull(keyReference)(o.key_reference, "key_reference"),
    listen_lines: orNull(list(listenLine))(o.listen_lines, "listen_lines"),
    listen_rects: orNull(map(rect))(o.listen_rects, "listen_rects"),
    offer_rects: orNull(map(rect))(o.offer_rects, "offer_rects"),
    heading_rects: orNull(map(rect))(o.heading_rects, "heading_rects"),
    clip_lights: orNull((value, at) => {
      const lit = fields(value, at);
      return { cells: list(triple)(lit.cells, `${at}.cells`), ring: bool(lit.ring, `${at}.ring`) };
    })(o.clip_lights, "clip_lights"),
    marks: list(mark)(o.marks, "marks"),
    mark_removed: orNull(str)(o.mark_removed, "mark_removed"),
    unfilled: list((value, at) => {
      const empty = fields(value, at);
      const open = orNull(rect)(empty.open, `${at}.open`);
      return { pane: num(empty.pane, `${at}.pane`), why: str(empty.why, `${at}.why`), open };
    })(o.unfilled, "unfilled"),
    workbooks: list(workbook)(o.workbooks, "workbooks"),
    workbooks_closing: list(closingWorkbook)(o.workbooks_closing, "workbooks_closing"),
    workbook_offer: orNull(workbookOffer)(o.workbook_offer, "workbook_offer"),
    row_menu: orNull(rowMenu)(o.row_menu, "row_menu"),
    scrollbars: map(scrollbar)(o.scrollbars, "scrollbars"),
    overflow: orNull(map(map(bool)))(o.overflow, "overflow"),
    capture: captureDrawn(o.capture, "capture"),
    bug: bugDrawn(o.bug, "bug"),
    spaces: list(spaceRow)(o.spaces, "spaces"),
    badges: map(str)(o.badges, "badges"),
    drawer: drawerDrawn(o.drawer, "drawer"),
    calendar: calendar(o.calendar, "calendar"),
    status_bar: statusBar(o.status_bar, "status_bar"),
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

function blockKind(value: unknown, at: string): "call" | "block" {
  if (value === "call" || value === "block") return value;
  throw wrong(at, "call or block", value);
}

function meetingShow(value: unknown, at: string): MeetingShow {
  if (value === "busy" || value === "tentative" || value === "free" || value === "away") return value;
  throw wrong(at, "busy, tentative, free or away", value);
}

function dayView(value: unknown, at: string): CalendarDayView {
  const o = fields(value, at);
  const block = (v: unknown, at: string) => {
    const b = fields(v, at);
    return {
      start: num(b.start, `${at}.start`),
      end: num(b.end, `${at}.end`),
      rect: orNull(rect)(b.rect, `${at}.rect`),
      column: num(b.column, `${at}.column`),
      columns: num(b.columns, `${at}.columns`),
      stage: meetingStage(b.stage, `${at}.stage`),
      selected: bool(b.selected, `${at}.selected`),
      more: num(b.more, `${at}.more`),
      join: orNull(rect)(b.join, `${at}.join`),
    };
  };
  const gap = (v: unknown, at: string) => {
    const g = fields(v, at);
    return {
      from: num(g.from, `${at}.from`),
      to: num(g.to, `${at}.to`),
      minutes: num(g.minutes, `${at}.minutes`),
      rect: orNull(rect)(g.rect, `${at}.rect`),
    };
  };
  const now = (v: unknown, at: string) => {
    const n = fields(v, at);
    return { rect: rect(n.rect, `${at}.rect`), label: orNull(rect)(n.label, `${at}.label`) };
  };
  return {
    rect: rect(o.rect, `${at}.rect`),
    viewport: orNull(rect)(o.viewport, `${at}.viewport`),
    scroll: num(o.scroll, `${at}.scroll`),
    span: pair(o.span, `${at}.span`),
    hours: num(o.hours, `${at}.hours`),
    blocks: list(block)(o.blocks, `${at}.blocks`),
    gaps: list(gap)(o.gaps, `${at}.gaps`),
    now: orNull(now)(o.now, `${at}.now`),
  };
}

function band(value: unknown, at: string): NonNullable<Snapshot["calendar"]["band"]> {
  const o = fields(value, at);
  const block = (v: unknown, at: string) => {
    const b = fields(v, at);
    return {
      start: num(b.start, `${at}.start`),
      end: num(b.end, `${at}.end`),
      rect: orNull(rect)(b.rect, `${at}.rect`),
      kind: blockKind(b.kind, `${at}.kind`),
      show: meetingShow(b.show, `${at}.show`),
      color: str(b.color, `${at}.color`),
      focus: bool(b.focus, `${at}.focus`),
      lane: orNull(num)(b.lane, `${at}.lane`),
      lanes: orNull(num)(b.lanes, `${at}.lanes`),
      more: num(b.more, `${at}.more`),
    };
  };
  const outside = (v: unknown, at: string) => {
    const b = fields(v, at);
    return { count: num(b.count, `${at}.count`), rect: orNull(rect)(b.rect, `${at}.rect`) };
  };
  return {
    rect: orNull(rect)(o.rect, `${at}.rect`),
    span: pair(o.span, `${at}.span`),
    workday: pair(o.workday, `${at}.workday`),
    marks: list(orNull(rect))(o.marks, `${at}.marks`),
    now: orNull(rect)(o.now, `${at}.now`),
    blocks: list(block)(o.blocks, `${at}.blocks`),
    outside: list(outside)(o.outside, `${at}.outside`),
    status_bar: orNull(rect)(o.status_bar, `${at}.status_bar`),
  };
}

function nav(value: unknown, at: string): Snapshot["calendar"]["nav"] {
  const o = fields(value, at);
  return {
    back: orNull(rect)(o.back, `${at}.back`),
    forward: orNull(rect)(o.forward, `${at}.forward`),
    today: orNull(rect)(o.today, `${at}.today`),
  };
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
    bars: num(o.bars, `${at}.bars`),
    band: orNull(band)(o.band, `${at}.band`),
    band_state: oneOf(["absent", "loading", "failed", "ready"] as const, "a band's state")(o.band_state, `${at}.band_state`),
    band_line: orNull(str)(o.band_line, `${at}.band_line`),
    band_rect: orNull(rect)(o.band_rect, `${at}.band_rect`),
    band_retry: orNull(rect)(o.band_retry, `${at}.band_retry`),
    banner: orNull(banner)(o.banner, `${at}.banner`),
    alerts: num(o.alerts, `${at}.alerts`),
    joins: orNull(num)(o.joins, `${at}.joins`),
    rows: num(o.rows, `${at}.rows`),
    rail: railMark(o.rail, `${at}.rail`),
    day: num(o.day, `${at}.day`),
    day_start: num(o.day_start, `${at}.day_start`),
    all_day_line: orNull(rect)(o.all_day_line, `${at}.all_day_line`),
    first_row: orNull(rect)(o.first_row, `${at}.first_row`),
    day_view: orNull(dayView)(o.day_view, `${at}.day_view`),
    nav: nav(o.nav, `${at}.nav`),
    fetch: calendarFetch(o.fetch, `${at}.fetch`),
    fetch_line: str(o.fetch_line, `${at}.fetch_line`),
    fetch_rect: orNull(rect)(o.fetch_rect, `${at}.fetch_rect`),
    retry: orNull(rect)(o.retry, `${at}.retry`),
    retries: orNull(num)(o.retries, `${at}.retries`),
  };
}

function calendarFetch(value: unknown, at: string): CalendarFetch {
  const o = fields(value, at);
  const state = oneOf(["idle", "loading", "read", "failed"] as const, "a fetch's state")(o.state, `${at}.state`);
  switch (state) {
    case "idle":
      return { state };
    case "loading":
      return { state, since: num(o.since, `${at}.since`), bytes: num(o.bytes, `${at}.bytes`) };
    case "read":
      return { state, at: num(o.at, `${at}.at`), events: num(o.events, `${at}.events`) };
    case "failed":
      return { state, at: num(o.at, `${at}.at`), error: str(o.error, `${at}.error`), code: orNull(num)(o.code, `${at}.code`) };
  }
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

export function recordReply(value: unknown): RecordReply {
  return { ok: true, frames: num(fields(value, "reply").frames, "frames") };
}

function probeBuffer(value: unknown, at: string): ProbeBuffer {
  const o = fields(value, at);
  return {
    pts_ms: orNull(num)(o.pts_ms, `${at}.pts_ms`),
    samples: num(o.samples, `${at}.samples`),
    rms: num(o.rms, `${at}.rms`),
  };
}

export function audioProbeReply(value: unknown): AudioProbeReply {
  const o = fields(value, "reply");
  return { ok: true, buffers: list(probeBuffer)(o.buffers, "buffers"), failed: orNull(str)(o.failed, "failed") };
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

function untrusted(o: Record<string, unknown>, at: string): Untrusted {
  if (o.untrusted !== true) throw new Error(`${at}.untrusted: expected true`);
  return { untrusted: true, warning: str(o.warning, `${at}.warning`) };
}

export function appReadReply(value: unknown): AppReadReply {
  const o = fields(value, "reply");
  const captured = fields(o.captured, "captured");
  return {
    ok: true,
    pane: num(o.pane, "pane"),
    task: orNull(num)(o.task, "task"),
    window: frame(o.window, "window"),
    millis: num(o.millis, "millis"),
    captured: {
      ...untrusted(captured, "captured"),
      count: num(captured.count, "captured.count"),
      truncated: truncated(captured.truncated, "captured.truncated"),
      tree: axNode(captured.tree, "captured.tree"),
    },
  };
}

export function appLookReply(value: unknown): AppLookReply {
  const o = fields(value, "reply");
  const captured = fields(o.captured, "captured");
  return {
    ok: true,
    pane: num(o.pane, "pane"),
    task: num(o.task, "task"),
    captured: { ...untrusted(captured, "captured"), path: str(captured.path, "captured.path") },
  };
}

function timeSpan(value: unknown, at: string): TimeSpan {
  const o = fields(value, at);
  return { start: str(o.start, `${at}.start`), end: str(o.end, `${at}.end`) };
}

function meeting(value: unknown, at: string): Meeting {
  const o = fields(value, at);
  const status = o.status === undefined ? undefined : meetingShow(o.status, `${at}.status`);
  if (status === "busy") throw wrong(`${at}.status`, "tentative, free or away", o.status);
  return {
    ...timeSpan(value, at),
    all_day: bool(o.all_day, `${at}.all_day`),
    title: str(o.title, `${at}.title`),
    location: orNull(str)(o.location, `${at}.location`),
    has_call_link: bool(o.has_call_link, `${at}.has_call_link`),
    ...(status === undefined ? {} : { status }),
  };
}

export function calendarReply(value: unknown): CalendarReply {
  const o = fields(value, "reply");
  if (bool(o.configured, "configured") === false) return { ok: true, configured: false };
  return {
    ok: true,
    configured: true,
    updated: orNull(str)(o.updated, "updated"),
    stale: bool(o.stale, "stale"),
    in_call: orNull(bool)(o.in_call, "in_call"),
    from: str(o.from, "from"),
    to: str(o.to, "to"),
    meetings: list(meeting)(o.meetings, "meetings"),
    more: num(o.more, "more"),
    known_until: orNull(str)(o.known_until, "known_until"),
    workday: str(o.workday, "workday"),
    busy: list(timeSpan)(o.busy, "busy"),
    free: list(timeSpan)(o.free, "free"),
  };
}

/** The message of a refused request (`{"ok":false,"error":".."}`), or null for any other reply. */
export function refusal(value: unknown): string | null {
  if (!isFields(value) || value.ok !== false) return null;
  return typeof value.error === "string" ? value.error : "the request was refused";
}
