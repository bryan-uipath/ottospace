// A typed client for ottospace's control socket: one method per `Request` in
// `crates/ottospace/src/control.rs`, named as the request in camelCase (`task_add` → `taskAdd`).
// One JSON request per line out, one JSON reply per line back. A refused request rejects.

import { Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { axTreeReply, captureReply, inboxReply, refusal, snapshot, taskPopReply, taskReply, waitReply } from "./parse.ts";
import type {
  AxTreeArgs,
  AxTreeReply,
  CaptureArgs,
  CaptureReply,
  CellArgs,
  Dir,
  InboxReply,
  MouseDownArgs,
  OpenAppArgs,
  OpenRepoArgs,
  OptionalPaneArg,
  PaneArg,
  PickerArgs,
  ScrollArgs,
  Section,
  Snapshot,
  SplitArgs,
  TaskAddArgs,
  TaskId,
  TaskPopReply,
  TaskReply,
  TaskState,
  TypeArgs,
  WaitArgs,
  WaitReply,
} from "./types.ts";

/** Sends one request line and resolves with its reply line. */
export interface Transport {
  send(line: string): Promise<string>;
  close(): void;
}

/** `$OTTOSPACE_SOCKET`, else `ottospace.sock` in the per-user temp dir: where the app listens. */
export function defaultSocketPath(): string {
  return process.env.OTTOSPACE_SOCKET || join(tmpdir(), "ottospace.sock");
}

/** One connection. The app answers a connection's requests in order, so replies pair up by position. */
class SocketTransport implements Transport {
  private readonly waiting: Array<{ resolve: (line: string) => void; reject: (error: Error) => void }> = [];
  private buffer = "";
  private ended: Error | null = null;
  private readonly socket: Socket;

  constructor(socket: Socket) {
    this.socket = socket;
    socket.setEncoding("utf8");
    socket.on("data", (chunk: string) => this.read(chunk));
    socket.on("error", (error) => this.end(error));
    socket.on("close", () => this.end(new Error("ottospace closed the connection")));
  }

  send(line: string): Promise<string> {
    if (this.ended) return Promise.reject(this.ended);
    return new Promise((resolve, reject) => {
      this.waiting.push({ resolve, reject });
      this.socket.write(`${line}\n`);
    });
  }

  close(): void {
    this.end(new Error("the client was closed"));
    this.socket.destroy();
  }

  private read(chunk: string): void {
    this.buffer += chunk;
    for (let end = this.buffer.indexOf("\n"); end >= 0; end = this.buffer.indexOf("\n")) {
      const line = this.buffer.slice(0, end);
      this.buffer = this.buffer.slice(end + 1);
      this.waiting.shift()?.resolve(line);
    }
  }

  private end(error: Error): void {
    this.ended ??= error;
    for (const call of this.waiting.splice(0)) call.reject(this.ended);
  }
}

function dial(socketPath: string): Promise<Transport> {
  return new Promise((resolve, reject) => {
    // Listen before connecting: bun reports a missing socket inside `connect`, and an error
    // with no listener is thrown. For the same reason the listener stays on after a failure.
    const socket = new Socket();
    socket.on("error", reject);
    socket.once("connect", () => {
      const transport = new SocketTransport(socket);
      socket.off("error", reject);
      resolve(transport);
    });
    socket.connect(socketPath);
  });
}

export class Client {
  private readonly transport: Transport;
  private readonly dial: () => Promise<Transport>;

  /**
   * `dial` opens one more connection. `wait` uses it: the app answers a connection in order, so
   * on the shared one a long wait would hold up every request behind it.
   */
  constructor(transport: Transport, dial: () => Promise<Transport>) {
    this.transport = transport;
    this.dial = dial;
  }

  /** Ends the connection. `close` is the request that closes a pane. */
  disconnect(): void {
    this.transport.close();
  }

  /** Sends any request and resolves with the parsed reply. Rejects if the app refused it. */
  send(request: object): Promise<unknown> {
    return this.over(this.transport, request);
  }

  private async over(transport: Transport, request: object): Promise<unknown> {
    const reply: unknown = JSON.parse(await transport.send(JSON.stringify(request)));
    const refused = refusal(reply);
    if (refused !== null) throw new Error(refused);
    return reply;
  }

  private async snapshot(request: object): Promise<Snapshot> {
    return snapshot(await this.send(request));
  }

  // One method per request, in the order of `Request`.

  state(): Promise<Snapshot> {
    return this.snapshot({ cmd: "state" });
  }
  /** A new pane beside the focused one. */
  split(args: SplitArgs = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "split", ...args });
  }
  /** Keys to a terminal pane, as if typed. End a command with `\r`. */
  type(args: TypeArgs): Promise<Snapshot> {
    return this.snapshot({ cmd: "type", ...args });
  }
  /** Closes a pane (default: the focused one). Name it. Not the connection: see `disconnect`. */
  close(args: OptionalPaneArg = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "close", ...args });
  }
  focus(args: PaneArg): Promise<Snapshot> {
    return this.snapshot({ cmd: "focus", ...args });
  }
  focusDir(args: { dir: Dir }): Promise<Snapshot> {
    return this.snapshot({ cmd: "focus_dir", ...args });
  }
  /** Next (positive) or previous pane in reading order. */
  focusCycle(args: { delta: number }): Promise<Snapshot> {
    return this.snapshot({ cmd: "focus_cycle", ...args });
  }
  /** Hosts a running app's main window in a pane. It moves the user's real window. */
  openApp(args: OpenAppArgs): Promise<Snapshot> {
    return this.snapshot({ cmd: "open_app", ...args });
  }
  /** Gives an app pane the keyboard. */
  enter(args: PaneArg): Promise<Snapshot> {
    return this.snapshot({ cmd: "enter", ...args });
  }
  leave(): Promise<Snapshot> {
    return this.snapshot({ cmd: "leave" });
  }
  /** Toggles a rail section. */
  section(args: { section: Section }): Promise<Snapshot> {
    return this.snapshot({ cmd: "section", ...args });
  }
  picker(args: PickerArgs = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "picker", ...args });
  }
  newTab(): Promise<Snapshot> {
    return this.snapshot({ cmd: "new_tab" });
  }
  /** `index` is the position in `state.tabs`, not the tab's `id`. */
  selectTab(args: { index: number }): Promise<Snapshot> {
    return this.snapshot({ cmd: "select_tab", ...args });
  }
  cycleTab(args: { delta: number }): Promise<Snapshot> {
    return this.snapshot({ cmd: "cycle_tab", ...args });
  }
  /** Closes a tab (default: the one showing) and its panes. */
  closeTab(args: { index?: number } = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "close_tab", ...args });
  }
  pickerQuery(args: { query: string }): Promise<Snapshot> {
    return this.snapshot({ cmd: "picker_query", ...args });
  }
  pickerMove(args: { delta: number }): Promise<Snapshot> {
    return this.snapshot({ cmd: "picker_move", ...args });
  }
  /** Opens the selected match, or the match at `index`. */
  pickerChoose(args: { index?: number } = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "picker_choose", ...args });
  }
  pickerClose(): Promise<Snapshot> {
    return this.snapshot({ cmd: "picker_close" });
  }
  /** Shows or hides the key reference. */
  keys(): Promise<Snapshot> {
    return this.snapshot({ cmd: "keys" });
  }
  /** Presses keys through the app's bindings: `"cmd-c"`, `"shift-pageup"`. Space between keystrokes. */
  key(args: { keys: string }): Promise<Snapshot> {
    return this.snapshot({ cmd: "key", ...args });
  }
  mouseDown(args: MouseDownArgs): Promise<Snapshot> {
    return this.snapshot({ cmd: "mouse_down", ...args });
  }
  mouseDrag(args: CellArgs): Promise<Snapshot> {
    return this.snapshot({ cmd: "mouse_drag", ...args });
  }
  scroll(args: ScrollArgs): Promise<Snapshot> {
    return this.snapshot({ cmd: "scroll", ...args });
  }
  /** Reads the repository list from disk again: `state.repos` in the reply. */
  repos(): Promise<Snapshot> {
    return this.snapshot({ cmd: "repos" });
  }
  openRepo(args: OpenRepoArgs): Promise<Snapshot> {
    return this.snapshot({ cmd: "open_repo", ...args });
  }
  async taskAdd(args: TaskAddArgs): Promise<TaskReply> {
    return taskReply(await this.send({ cmd: "task_add", ...args }));
  }
  taskSetState(args: { id: TaskId; state: TaskState }): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_set_state", ...args });
  }
  taskSnooze(args: { id: TaskId; seconds: number }): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_snooze", ...args });
  }
  taskHold(args: { id: TaskId }): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_hold", ...args });
  }
  taskDismiss(args: { id: TaskId }): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_dismiss", ...args });
  }
  /** Takes back the last snooze, hold or dismiss. */
  taskUndo(): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_undo" });
  }
  async taskList(): Promise<InboxReply> {
    return inboxReply(await this.send({ cmd: "task_list" }));
  }
  /** Files a task from one line: `"fix zoom flicker #canvas @repo p1"`. */
  async taskNew(args: { line: string }): Promise<TaskReply> {
    return taskReply(await this.send({ cmd: "task_new", ...args }));
  }
  /** Opens the new-task box for the user with `text` in it. Nothing is filed. */
  compose(args: { text?: string } = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "compose", ...args });
  }
  /** Shows the tabs of the first task that needs the user. */
  async taskPop(): Promise<TaskPopReply> {
    return taskPopReply(await this.send({ cmd: "task_pop" }));
  }
  taskOpen(args: { id: TaskId }): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_open", ...args });
  }
  taskLeave(): Promise<Snapshot> {
    return this.snapshot({ cmd: "task_leave" });
  }
  /** One terminal pane's text. */
  async capture(args: CaptureArgs = {}): Promise<CaptureReply> {
    return captureReply(await this.send({ cmd: "capture", ...args }));
  }
  /** Resolves when a line of the pane matches `pattern`; rejects at the time limit. */
  async wait(args: WaitArgs): Promise<WaitReply> {
    const own = await this.dial();
    try {
      return waitReply(await this.over(own, { cmd: "wait", ...args }));
    } finally {
      own.close();
    }
  }
  /** An app pane's accessibility tree. Read-only. */
  async axTree(args: AxTreeArgs = {}): Promise<AxTreeReply> {
    return axTreeReply(await this.send({ cmd: "ax_tree", ...args }));
  }
  axOverlay(args: { refresh?: boolean } = {}): Promise<Snapshot> {
    return this.snapshot({ cmd: "ax_overlay", ...args });
  }
  /** Quits the app. */
  quit(): Promise<Snapshot> {
    return this.snapshot({ cmd: "quit" });
  }
}

/** Connects to a running ottospace. Rejects if none listens at `socketPath`. */
export async function connect(socketPath: string = defaultSocketPath()): Promise<Client> {
  return new Client(await dial(socketPath), () => dial(socketPath));
}
