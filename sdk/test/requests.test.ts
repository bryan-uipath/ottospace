// Keeps the client in step with `Request` in `crates/ottospace/src/control.rs`, without the app.
// `requests.jsonl` holds one request per client method, as the client sends it, with every field
// set. A Rust unit test (`the_sdk_fixture_holds_every_request`) parses each line with the real
// `Request`, so a wrong field name or a request with no method fails `cargo test`.
// To write the file again after a change: `UPDATE_FIXTURE=1 npm test`.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { Client } from "../src/client.ts";
import type { Transport } from "../src/client.ts";

const FIXTURE = join(import.meta.dirname, "requests.jsonl");
const CONTROL_RS = join(import.meta.dirname, "..", "..", "crates", "ottospace", "src", "control.rs");

/** Every method, with every field the request has. */
const CALLS: Array<(client: Client) => Promise<unknown>> = [
  (c) => c.state(),
  (c) => c.split({ axis: "down", kind: "empty" }),
  (c) => c.type({ pane: 1, text: "ls\r" }),
  (c) => c.close({ pane: 1 }),
  (c) => c.focus({ pane: 1 }),
  (c) => c.focusDir({ dir: "left" }),
  (c) => c.focusCycle({ delta: -1 }),
  (c) => c.resize({ dir: "left" }),
  (c) => c.swap({ dir: "right" }),
  (c) => c.movePane({ pane: 3, tab: 0 }),
  (c) => c.equalize(),
  (c) => c.dropPane({ source: 3, target: 1, zone: "left" }),
  (c) => c.fit({ pane: 2 }),
  (c) => c.openApp({ name: "Safari", axis: "right" }),
  (c) => c.enter({ pane: 1 }),
  (c) => c.leave(),
  (c) => c.section({ section: "tasks" }),
  (c) => c.picker({ axis: "down", new_tab: true }),
  (c) => c.newTab(),
  (c) => c.selectTab({ index: 0 }),
  (c) => c.cycleTab({ delta: 1 }),
  (c) => c.closeTab({ index: 1 }),
  (c) => c.pickerQuery({ query: "term" }),
  (c) => c.pickerMove({ delta: 1 }),
  (c) => c.pickerChoose({ index: 0 }),
  (c) => c.pickerClose(),
  (c) => c.keys(),
  (c) => c.key({ keys: "cmd-c" }),
  (c) => c.key({ keys: "cmd-c", as: "user", repeat: true }),
  (c) => c.mouseDown({ pane: 1, x: 2.5, y: 5, clicks: 2 }),
  (c) => c.mouseDrag({ pane: 1, x: 20, y: 5 }),
  (c) => c.scroll({ pane: 1, lines: 40 }),
  (c) => c.mouse({ action: "down", x: 300, y: 60, clicks: 1, modifiers: ["shift"] }),
  (c) =>
    c.mouse({
      action: "wheel",
      x: 300,
      y: 60,
      clicks: 1,
      modifiers: ["shift"],
      dx: 0,
      dy: -3,
      precise: true,
      as: "user",
    }),
  (c) => c.drag({ from: [300, 60], to: [900, 400], steps: 8 }),
  (c) => c.drag({ from: [300, 60], to: [900, 400], steps: 8, as: "user" }),
  (c) => c.repos(),
  (c) => c.openRepo({ path: "/Users/me/code/app", axis: "down" }),
  (c) => c.repoAdd({ path: "/Users/me/code/app" }),
  (c) =>
    c.taskAdd({
      title: "Fix the zoom flicker",
      priority: 1,
      state: "needs_you",
      tab: 2,
      agent: "codex",
      paths: ["/src/export"],
      skills: ["code-review"],
    }),
  (c) => c.taskSetState({ id: 1, state: "running" }),
  (c) => c.taskSnooze({ id: 1, seconds: 3600 }),
  (c) => c.taskHold({ id: 1 }),
  (c) => c.taskDismiss({ id: 1 }),
  (c) => c.taskUndo(),
  (c) => c.taskList(),
  (c) => c.taskNew({ line: "fix zoom flicker #canvas >app @claude p1" }),
  (c) => c.taskGroup({ heading: "later" }),
  (c) => c.sessionAdopt({ runner: "claude", id: "11111111-1111-4111-8111-111111111111" }),
  (c) => c.sessionHide({ runner: "codex", id: "c0de0001-0000-4000-8000-000000000001" }),
  (c) => c.sessionClick({ runner: "claude", id: "11111111-1111-4111-8111-111111111111" }),
  (c) => c.sessionsRange({ range: "week" }),
  (c) => c.compose({ text: "a draft" }),
  (c) => c.composeDrop({ paths: ["/tmp/a.txt"] }),
  (c) => c.taskPop(),
  (c) => c.taskOpen({ id: 1 }),
  (c) => c.taskLeave(),
  (c) => c.capture({ pane: 1, history: true }),
  (c) => c.wait({ pane: 1, pattern: "^done$", timeout_ms: 1000 }),
  (c) => c.captureStart({ pane: 1 }),
  (c) =>
    c.inspectorFixture({
      app: "Fixture",
      tree: { root: { role: "AXWindow", children: [{ role: "AXButton", actions: ["AXPress"] }] }, count: 2, truncated: { depth: false, nodes: false, timeout: false } },
    }),
  (c) => c.taskBrief({ id: 1 }),
  (c) => c.axTree({ pane: 1, depth: 20, nodes: 500 }),
  (c) => c.axOverlay({ refresh: true }),
  (c) => c.sneak(),
  (c) =>
    c.sneakAppTree({
      window: { x: 0, y: 0, w: 400, h: 300 },
      crop: 28,
      content: { x: 60, y: 40, w: 400, h: 272 },
      tree: { role: "AXWindow", children: [{ role: "AXButton", actions: ["AXPress"], frame: { x: 10, y: 40, w: 60, h: 24 } }] },
    }),
  (c) => c.marks(),
  (c) => c.markAdd({ pane: 1, name: "notes", key: "n" }),
  (c) => c.markGo({ name: "notes", axis: "down" }),
  (c) => c.markRemove({ key: "n" }),
  (c) => c.markRename({ name: "notes", to: "journal" }),
  (c) => c.trace(),
  (c) => c.quit(),
];

/** Not requests: the connection's own methods. */
const NOT_REQUESTS = ["constructor", "disconnect", "send", "over", "snapshot"];

/** Keeps each line the client sends, and refuses it. */
async function record(): Promise<string[]> {
  const lines: string[] = [];
  const transport: Transport = {
    send: async (line) => {
      lines.push(line);
      return '{"ok":false,"error":"recorded"}';
    },
    close: () => {},
  };
  const client = new Client(transport, async () => transport);
  for (const call of CALLS) await assert.rejects(call(client), /recorded/);
  return lines;
}

function command(line: string): string {
  const request: unknown = JSON.parse(line);
  if (typeof request === "object" && request !== null && "cmd" in request && typeof request.cmd === "string") {
    return request.cmd;
  }
  throw new Error(`no cmd in ${line}`);
}

/** `taskAdd` → `task_add`. */
function snake(name: string): string {
  return name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

test("the fixture holds what the client sends", async () => {
  const sent = `${(await record()).join("\n")}\n`;
  if (process.env.UPDATE_FIXTURE) writeFileSync(FIXTURE, sent);
  assert.equal(sent, readFileSync(FIXTURE, "utf8"), "run `UPDATE_FIXTURE=1 npm test` and commit requests.jsonl");
});

test("every client method is in the fixture", async () => {
  const methods = Object.getOwnPropertyNames(Client.prototype).filter((name) => !NOT_REQUESTS.includes(name));
  // `key`, `mouse` and `drag` are there twice: plain, and with the fields for tests.
  const sent = [...new Set((await record()).map(command))];
  assert.deepEqual(sent.toSorted(), methods.map(snake).toSorted());
});

test("every Request variant in control.rs has a client method", () => {
  const source = readFileSync(CONTROL_RS, "utf8");
  const body = source.slice(source.indexOf("pub enum Request {"), source.indexOf("\n}\n", source.indexOf("pub enum Request {")));
  // Variants sit at one level of indentation: `    TaskAdd {` or `    Leave,`.
  const variants = [...body.matchAll(/^ {4}([A-Z][A-Za-z]*)\b/gm)].map((match) => match[1] ?? "");
  assert.ok(variants.length > 20, `read ${variants.length} variants from control.rs`);
  const methods = Object.getOwnPropertyNames(Client.prototype).filter((name) => !NOT_REQUESTS.includes(name));
  const pascal = (name: string) => name.charAt(0).toUpperCase() + name.slice(1);
  assert.deepEqual(methods.map(pascal).toSorted(), variants.toSorted());
});
