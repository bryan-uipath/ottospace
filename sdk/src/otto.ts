// A thin layer over `Client` for the common jobs: tasks, and running a command in a terminal pane.
// Anything not here is on `otto.client`.

import { randomBytes } from "node:crypto";

import { connect } from "./client.ts";
import type { Client } from "./client.ts";
import type { InboxRow, PaneId, Snapshot, Task, TaskAddArgs, TaskId, TaskState } from "./types.ts";

export interface RunResult {
  /** The command's exit status. */
  status: number;
  /** What the command printed, as the pane shows it. */
  output: string;
}

/** One terminal pane. Every request names the pane, because focus is shared with the user. */
export class TerminalPane {
  private readonly client: Client;
  readonly pane: PaneId;

  constructor(client: Client, pane: PaneId) {
    this.client = client;
    this.pane = pane;
  }

  type(text: string): Promise<Snapshot> {
    return this.client.type({ pane: this.pane, text });
  }

  /** The first line that matches `pattern` (Rust `regex` syntax; each line is matched alone). */
  async wait(pattern: string, timeoutMs?: number): Promise<string> {
    const limit = timeoutMs === undefined ? {} : { timeout_ms: timeoutMs };
    return (await this.client.wait({ pane: this.pane, pattern, ...limit })).line;
  }

  async capture(history = false): Promise<string> {
    return (await this.client.capture({ pane: this.pane, history })).text;
  }

  /**
   * Types `command` into the shell and waits for it to end, as the guide teaches: a marker made
   * up for this run follows the command, e.g. `cargo test; echo done-7f3a-$?`. Needs a POSIX
   * shell at its prompt (`;` and `$?`). The output is read from the pane, so a full-screen
   * program, or more output than the history keeps, is not all there.
   */
  async run(command: string, timeoutMs = 600_000): Promise<RunResult> {
    const marker = `done-${randomBytes(4).toString("hex")}-`;
    await this.type(`${command}; echo ${marker}$?\r`);
    const end = await this.wait(`^${marker}[0-9]+$`, timeoutMs);
    const lines = (await this.capture(true)).split("\n");
    // The echoed command line holds the marker too, followed by `$?`: the output is between the two.
    const from = lines.findIndex((line) => line.includes(`${marker}$?`));
    const to = lines.lastIndexOf(end);
    return { status: Number(end.slice(marker.length)), output: lines.slice(from + 1, to).join("\n") };
  }
}

export class Otto {
  readonly client: Client;

  constructor(client: Client) {
    this.client = client;
  }

  readonly tasks = {
    add: async (title: string, options: Omit<TaskAddArgs, "title"> = {}): Promise<Task> =>
      (await this.client.taskAdd({ title, ...options })).task,
    /** The inbox, in the order the Tasks section lists it. */
    list: async (): Promise<InboxRow[]> => (await this.client.taskList()).inbox,
    setState: (id: TaskId, state: TaskState): Promise<Snapshot> => this.client.taskSetState({ id, state }),
    /** Puts the task's tabs on the user's tab bar. */
    open: (id: TaskId): Promise<Snapshot> => this.client.taskOpen({ id }),
    leave: (): Promise<Snapshot> => this.client.taskLeave(),
  };

  state(): Promise<Snapshot> {
    return this.client.state();
  }

  terminal(pane: PaneId): TerminalPane {
    return new TerminalPane(this.client, pane);
  }

  disconnect(): void {
    this.client.disconnect();
  }
}

/** Connects to the running ottospace at `socketPath` (default: `$OTTOSPACE_SOCKET`, else the temp dir). */
export async function open(socketPath?: string): Promise<Otto> {
  return new Otto(await connect(socketPath));
}
