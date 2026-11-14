// Launches a private ottospace the way `crates/ottospace/tests/e2e.rs` does: `OTTOSPACE_E2E`
// turns window hosting off and keeps the app in the background, the socket is the test's own,
// and with no `OTTOSPACE_HOME` the app reads no settings and keeps no tasks on disk.

import { spawn } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

import { connect } from "../src/client.ts";
import type { Client } from "../src/client.ts";

const BINARY = process.env.OTTOSPACE_BIN || join(import.meta.dirname, "..", "..", "target", "debug", "ottospace");
/** A loaded machine can take over 10 s to start the app. */
const LIMIT_MS = 40_000;
let launched = 0;

/**
 * Runs `body` against a fresh private instance, and kills the instance whatever happens.
 * `features`: the flags on (`OTTOSPACE_FEATURES`), else each as by default.
 */
export async function withOttospace(
  body: (client: Client, socketPath: string) => Promise<void>,
  features: string[] = [],
): Promise<void> {
  if (!existsSync(BINARY)) throw new Error(`${BINARY} is not built: run \`cargo build -p ottospace\``);
  // Short: a Unix socket path holds about 100 characters.
  const socketPath = join(tmpdir(), `osdk-${process.pid}-${launched++}.sock`);
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    OTTOSPACE_E2E: "1",
    OTTOSPACE_SOCKET: socketPath,
    // A plain shell: the tests must not depend on the user's shell profile.
    OTTOSPACE_SHELL: "/bin/sh",
    OTTOSPACE_REPOS: "",
    OTTOSPACE_FEATURES: features.join(","),
  };
  // Never the user's `~/.ottospace`, and never hosting inherited from the caller's shell.
  delete env.OTTOSPACE_HOME;
  delete env.OTTOSPACE_HOST_ONLY;
  const app = spawn(BINARY, [], { env, stdio: "ignore" });
  const kill = () => app.kill("SIGKILL");
  process.once("exit", kill); // also if the test process dies before `finally`
  try {
    const client = await dialWhenUp(socketPath);
    try {
      await body(client, socketPath);
    } finally {
      client.disconnect();
    }
  } finally {
    kill();
    process.off("exit", kill);
    rmSync(socketPath, { force: true });
    rmSync(socketPath.replace(/\.sock$/, ".lock"), { force: true });
  }
}

async function dialWhenUp(socketPath: string): Promise<Client> {
  const deadline = Date.now() + LIMIT_MS;
  for (;;) {
    try {
      return await connect(socketPath);
    } catch (error) {
      if (Date.now() > deadline) throw new Error(`ottospace did not open ${socketPath}: ${String(error)}`);
      await sleep(50);
    }
  }
}

/** Polls `check` until it returns a value. */
export async function eventually<T>(what: string, check: () => Promise<T | undefined>): Promise<T> {
  const deadline = Date.now() + LIMIT_MS;
  for (;;) {
    const value = await check();
    if (value !== undefined) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await sleep(50);
  }
}
