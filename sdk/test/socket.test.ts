// Where the client looks for the socket, and that it only talks to one the user owns. No app.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { connect } from "../src/client.ts";
import { checkOwned, defaultSocketPath } from "../src/socket.ts";

test("the socket path: the variable, then TMPDIR, then macOS's per-user temp dir, never /tmp", () => {
  const never = () => {
    throw new Error("not asked");
  };
  assert.equal(defaultSocketPath({ OTTOSPACE_SOCKET: "/x/y.sock", TMPDIR: "/t" }, never), "/x/y.sock");
  assert.equal(defaultSocketPath({ TMPDIR: "/var/folders/ab/T/" }, never), "/var/folders/ab/T/ottospace.sock");
  assert.equal(defaultSocketPath({}, () => "/var/folders/cd/T/"), "/var/folders/cd/T/ottospace.sock");
  // An empty variable is no variable.
  assert.equal(defaultSocketPath({ OTTOSPACE_SOCKET: "", TMPDIR: "" }, () => "/u/T"), "/u/T/ottospace.sock");

  // With nothing set, this machine's own answer: what `getconf` says, as the app's `temp_dir` does.
  const real = execFileSync("/usr/bin/getconf", ["DARWIN_USER_TEMP_DIR"], { encoding: "utf8" }).trim();
  assert.equal(defaultSocketPath({}), join(real, "ottospace.sock"));
  assert.ok(!defaultSocketPath({}).startsWith("/tmp/"), defaultSocketPath({}));
});

test("a socket of another user, or a file that is no socket, is refused before anything is sent", async () => {
  // A folder of the test's own, with a short path: a socket path holds about 100 characters.
  const dir = mkdtempSync(join(tmpdir(), "osdk-own-"));
  const socketPath = join(dir, "s.sock");
  const server = createServer();
  const received: string[] = [];
  server.on("connection", (socket) => socket.on("data", (chunk) => received.push(String(chunk))));
  await new Promise<void>((resolve) => server.listen(socketPath, resolve));
  try {
    const uid = process.getuid?.() ?? 0;
    checkOwned(socketPath, uid);
    assert.throws(() => checkOwned(socketPath, uid + 1), /belongs to another user \(uid \d+\): not connecting/);

    const file = join(dir, "plain");
    writeFileSync(file, "");
    assert.throws(() => checkOwned(file), /is not a socket/);
    await assert.rejects(connect(file), /is not a socket/);
    await assert.rejects(connect(join(dir, "missing.sock")), /ENOENT/);
    assert.deepEqual(received, []);
  } finally {
    await new Promise((resolve) => server.close(resolve));
    rmSync(dir, { recursive: true, force: true });
  }
});
