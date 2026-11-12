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
   * Types `command` into the shell and waits for it to end, as the guide teaches, with a marker
   * made up for this run: `eval '<command>'; printf '\ndone-7f3a-%d\n' $?`.
   * - `eval` of the quoted command keeps the two apart: a command that ends in a comment, `;` or
   *   `&` cannot swallow or break the marker, and `$?` is the command's own exit status.
   * - The marker starts on a new line, so output with no newline at its end is still seen.
   * Needs a POSIX shell at its prompt. `output` is the command's output without its last
   * newline, read from the pane: a full-screen program, or more than the history keeps, is not
   * all there.
   */
  async run(command: string, timeoutMs = 600_000): Promise<RunResult> {
    const marker = `done-${randomBytes(4).toString("hex")}-`;
    const quoted = `'${command.replaceAll("'", "'\\''")}'`;
    await this.type(`eval ${quoted}; printf '\\n${marker}%d\\n' $?\r`);
    const end = await this.wait(`^${marker}[0-9]+$`, timeoutMs);
    const lines = (await this.capture(true)).split("\n");
    // The echoed command line holds the marker too, followed by `%d`: the output is between the two.
    const from = lines.findLastIndex((line) => line.includes(`${marker}%d`));
    const to = lines.indexOf(end, from + 1);
    const output = lines.slice(from + 1, to);
    // The marker's own leading newline: an empty line, unless the output ended without one.
    if (output.at(-1) === "") output.pop();
    return { status: Number(end.slice(marker.length)), output: output.join("\n") };
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

/** Connects to the running ottospace at `socketPath` (default: see `defaultSocketPath`). */
export async function open(socketPath?: string): Promise<Otto> {
  return new Otto(await connect(socketPath));
}
