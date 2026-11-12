// Where the app's socket is, and whether it is safe to talk to. `type` sends text that a shell
// runs, so the client must not be led to a socket some other user made.

import { execFileSync } from "node:child_process";
import { statSync } from "node:fs";
import { join } from "node:path";

/** macOS's per-user temp dir, which the app uses when `TMPDIR` is not set. Never `/tmp`. */
function darwinUserTempDir(): string {
  try {
    const dir = execFileSync("/usr/bin/getconf", ["DARWIN_USER_TEMP_DIR"], { encoding: "utf8" }).trim();
    if (dir.startsWith("/")) return dir;
  } catch {
    // Not macOS, or no getconf: fall through to the error.
  }
  throw new Error("cannot find the per-user temp dir: set OTTOSPACE_SOCKET");
}

/**
 * `$OTTOSPACE_SOCKET`, else `ottospace.sock` in `$TMPDIR`, else in macOS's per-user temp dir: the
 * app's own order. It does not fall back to `/tmp`, where any user could put a socket.
 */
export function defaultSocketPath(
  env: NodeJS.ProcessEnv = process.env,
  userTempDir: () => string = darwinUserTempDir,
): string {
  if (env.OTTOSPACE_SOCKET) return env.OTTOSPACE_SOCKET;
  return join(env.TMPDIR || userTempDir(), "ottospace.sock");
}

/** Throws unless `socketPath` is a socket that `uid` (default: this process's user) owns. */
export function checkOwned(socketPath: string, uid: number | undefined = process.getuid?.()): void {
  const found = statSync(socketPath);
  if (!found.isSocket()) throw new Error(`${socketPath} is not a socket`);
  if (uid !== undefined && found.uid !== uid) {
    throw new Error(`${socketPath} belongs to another user (uid ${found.uid}): not connecting`);
  }
}
