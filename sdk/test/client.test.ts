// Drives a real, private ottospace over its socket. Needs the debug binary:
// `cargo build -p ottospace`. Each test launches its own instance and kills it.

import assert from "node:assert/strict";
import { test } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";

import { connect } from "../src/client.ts";
import type { Client } from "../src/client.ts";
import { Otto } from "../src/otto.ts";
import { snapshot } from "../src/parse.ts";
import type { Snapshot } from "../src/types.ts";
import { eventually, withOttospace } from "./harness.ts";

/** The root split's ratio, or null for a single pane or none. */
function layoutRatio(snapshot: Snapshot): number | null {
  const layout = snapshot.state.layout;
  return layout !== null && "split" in layout ? layout.split.ratio : null;
}

/** The typed snapshot holds every key of the real reply and nothing else: the types are true. */
async function exactState(client: Client): Promise<void> {
  const raw = await client.send({ cmd: "state" });
  assert.deepEqual(snapshot(raw), raw);
}

test("state: the typed snapshot is the whole reply", async () => {
  await withOttospace(async (client) => {
    const start = await client.state();
    assert.equal(start.mode, "normal");
    assert.deepEqual(start.state.panes, []);
    assert.deepEqual(start.state.tabs, [{ id: 1, title: "Tab 1", task: null }]);
    assert.equal(start.state.current_task, null);
    assert.equal(start.notice, null);
    // No pane: the home screen, with nothing on it yet.
    assert.deepEqual(start.home?.nodes, []);
    assert.deepEqual(start.home?.counts, { needs_you: 0, running: 0, captured: 0, on_hold: 0, done: 0 });
    assert.match(start.version, /^\d+\.\d+\.\d+ · \w+$/);
    await exactState(client);

    // The home screen with tasks on it: nodes, the "Needs you" rows and the counts.
    const ask = (await client.taskAdd({ title: "Answer the agent", state: "needs_you", priority: 1 })).task;
    await client.taskNew({ line: "tidy up #canvas" });
    const home = await eventually("both tasks as nodes on the home screen", async () => {
      const now = (await client.state()).home;
      return now !== null && now.nodes.length === 2 ? now : undefined;
    });
    assert.deepEqual(home.needs_you, [ask]);
    assert.deepEqual(home.counts, { needs_you: 1, running: 0, captured: 1, on_hold: 0, done: 0 });
    assert.deepEqual(home.nodes.map((node) => node.task).toSorted(), [1, 2]);
    await exactState(client);

    // With a split tree, a terminal, a task, an inbox row and the picker in it.
    assert.equal((await client.split({ kind: "terminal" })).home, null, "a pane takes the home screen's place");
    await client.split({ axis: "down", kind: "empty" });
    await client.taskAdd({ title: "Check the types", state: "needs_you" });
    const picking = await client.picker({ axis: "down" });
    assert.equal(picking.mode, "pick");
    assert.deepEqual(picking.state.picker?.items[0], { type: "terminal" });
    await exactState(client);
  });
});

test("split and close", async () => {
  await withOttospace(async (client) => {
    await client.split({ kind: "empty" });
    const two = await client.split({ axis: "down", kind: "empty" });
    assert.deepEqual(two.state.panes.map((pane) => pane.id), [1, 2]);
    assert.equal(two.state.focused, 2);
    assert.deepEqual(two.state.layout, { split: { axis: "down", ratio: 0.5, a: { leaf: 1 }, b: { leaf: 2 } } });
    assert.deepEqual(two.tab_panes, [[1, 2]]);

    const one = await client.close({ pane: 1 });
    assert.deepEqual(one.state.panes, [{ id: 2, kind: { type: "empty" } }]);
    assert.deepEqual(one.state.layout, { leaf: 2 });
  });
});

test("resize, equalize, swap and move_pane", async () => {
  await withOttospace(async (client) => {
    await client.split({ kind: "empty" });
    await client.split({ axis: "right", kind: "empty" }); // 1 | 2, focus on 2
    const narrower = await client.resize({ dir: "right" });
    assert.notEqual(layoutRatio(narrower), 0.5, "the border moved");
    assert.equal(layoutRatio(await client.equalize()), 0.5);

    const swapped = await client.swap({ dir: "left" });
    assert.deepEqual(swapped.state.layout, { split: { axis: "right", ratio: 0.5, a: { leaf: 2 }, b: { leaf: 1 } } });

    await client.newTab();
    await client.selectTab({ index: 0 });
    const moved = await client.movePane({ pane: 2, tab: 1 });
    assert.deepEqual(moved.tab_panes.map((panes) => panes.toSorted()), [[1], [2]]);
    await exactState(client);
  });
});

test("type, wait and capture; and run on top of them", async () => {
  await withOttospace(async (client) => {
    const pane = (await client.split({ kind: "terminal" })).state.focused;
    assert.equal(pane, 1);

    await client.type({ pane: 1, text: "echo sdk-$((20+3))\r" });
    const waited = await client.wait({ pane: 1, pattern: "^sdk-23$", timeout_ms: 30_000 });
    assert.deepEqual(waited, { ok: true, pane: 1, line: "sdk-23" });
    const captured = await client.capture({ pane: 1, history: true });
    assert.equal(captured.pane, 1);
    assert.ok(captured.text.split("\n").includes("sdk-23"), captured.text);
    const terminal = (await client.state()).terminals["1"];
    assert.ok(terminal?.lines.includes("sdk-23"), "the snapshot holds the pane's lines");

    const shell = new Otto(client).terminal(1);
    assert.deepEqual(await shell.run("printf 'one\\ntwo\\n'"), { status: 0, output: "one\ntwo" });
    assert.deepEqual(await shell.run("sh -c 'exit 3'"), { status: 3, output: "" });

    // A wait that runs out is a refusal, and other requests are answered while it waits.
    const never = client.wait({ pane: 1, pattern: "^never-printed$", timeout_ms: 300 });
    assert.equal((await client.state()).state.focused, 1);
    await assert.rejects(never, /no match for the pattern in 300 ms/);
  });
});

test("run: the command and the marker cannot break each other", async () => {
  await withOttospace(async (client) => {
    await client.split({ kind: "terminal" });
    const shell = new Otto(client).terminal(1);
    const quick = 20_000; // each of these hung until the time limit before
    // Output with no newline at its end: the marker still starts a line.
    assert.deepEqual(await shell.run("printf foo", quick), { status: 0, output: "foo" });
    // A comment at the end does not swallow the marker, and the status is the command's.
    assert.deepEqual(await shell.run("false # a note", quick), { status: 1, output: "" });
    assert.deepEqual(await shell.run("echo a;", quick), { status: 0, output: "a" });
    // A job in the background: the shell may print a job line, so only that it ends, with 0.
    assert.equal((await shell.run("sleep 0.2 &", quick)).status, 0);
    // Quotes in the command survive the quoting, and an empty line in the output is kept.
    assert.deepEqual(await shell.run(`echo "it's"; echo; echo 'two  words'`, quick), {
      status: 0,
      output: "it's\n\ntwo  words",
    });
    assert.deepEqual(await shell.run("echo last; (exit 7)", quick), { status: 7, output: "last" });
  });
});

test("disconnect ends a pending wait, and later calls reject", async () => {
  await withOttospace(async (client, socketPath) => {
    await client.split({ kind: "terminal" });
    const other = await connect(socketPath);
    const started = Date.now();
    const pending = other.wait({ pane: 1, pattern: "^never-printed$", timeout_ms: 60_000 });
    await sleep(300); // the wait has its own connection by now
    other.disconnect();
    await assert.rejects(pending, /the client was closed|ottospace closed the connection/);
    assert.ok(Date.now() - started < 5_000, `settled after ${Date.now() - started} ms, not at the time limit`);
    await assert.rejects(other.state(), /the client was disconnected/);
    await assert.rejects(other.wait({ pane: 1, pattern: "x" }), /the client was disconnected/);

    // Disconnecting before the wait's own connection is up: still no call left hanging.
    const early = await connect(socketPath);
    const racing = early.wait({ pane: 1, pattern: "^never-printed$", timeout_ms: 60_000 });
    early.disconnect();
    await assert.rejects(racing, /the client was disconnected|the client was closed/);
    // The app goes on answering others.
    assert.equal((await client.state()).state.focused, 1);
  });
});

test("tasks: add, list, open, leave", async () => {
  await withOttospace(async (client) => {
    const otto = new Otto(client);
    const later = await otto.tasks.add("Write the note");
    assert.equal(typeof later.created, "number", "filed now");
    assert.deepEqual({ ...later, created: null }, {
      id: 1,
      title: "Write the note",
      state: "captured",
      priority: 3,
      snoozed_until: null,
      dismissed: false,
      project: null,
      repo: null,
      agent: null,
      paths: [],
      resolved: [],
      skills: [],
      created: null,
      session: null,
    });
    const urgent = await otto.tasks.add("Review the client", { priority: 1, state: "needs_you" });
    const filed = (await client.taskNew({ line: "fix the flicker #canvas p2" })).task;
    assert.deepEqual([filed.title, filed.project, filed.priority], ["fix the flicker", "canvas", 2]);

    const inbox = await otto.tasks.list();
    assert.deepEqual(inbox.map((row) => [row.id, row.group]), [[2, "needs_you"], [3, "captured"], [1, "captured"]]);
    assert.equal((await client.state()).needs_you, 1);

    const opened = await otto.tasks.open(urgent.id);
    assert.equal(opened.state.current_task, urgent.id);
    assert.deepEqual(opened.state.tabs[opened.state.active], { id: 2, title: "Review the client", task: urgent.id });

    const left = await otto.tasks.leave();
    assert.equal(left.state.current_task, null);
    assert.equal(left.state.tabs[left.state.active]?.task, null);
  });
});

test("sneak mode labels the controls, and a typed label acts as a click", async () => {
  await withOttospace(async (client) => {
    const labelled = await client.sneak();
    assert.equal(labelled.mode, "sneak");
    const tasks = labelled.sneak?.targets.find((t) => t.source.type === "own" && t.source.id === "rail:tasks");
    assert.ok(tasks, "the Tasks icon has a label");
    const after = await client.key({ keys: tasks.label.split("").join(" ") });
    assert.deepEqual([after.mode, after.sneak, after.state.sidebar], ["normal", null, "tasks"]);
  });
});

test("marks: add, list, go, rename, remove", async () => {
  await withOttospace(async (client) => {
    const pane = (await client.split()).state.focused;
    assert.equal(pane, 1);
    const { mark } = await client.markAdd({ pane: 1, name: "notes", key: "n" });
    assert.deepEqual(
      [mark.name, mark.key, mark.kind, mark.open, mark.pane, mark.available],
      ["notes", "n", "terminal", true, 1, true],
    );
    const tab = (await client.markAdd({ tab: 1 })).mark;
    assert.deepEqual([tab.name, tab.kind, tab.tab], ["Tab 1", "tab", 1]);
    assert.deepEqual((await client.marks()).marks.map((m) => m.name), ["notes", "Tab 1"]);
    await exactState(client);

    // The jump prefix waits for a letter: the state says so, and the types hold it.
    const waiting = await client.key({ keys: "cmd-'" });
    assert.deepEqual([waiting.mode, waiting.state.mark_input], ["mark", { type: "jump" }]);
    await exactState(client);
    assert.equal((await client.key({ keys: "escape" })).mode, "normal");

    // Closed, the mark opens a terminal again, under a new pane id.
    await client.close({ pane: 1 });
    const gone = await client.markGo({ key: "n" });
    assert.deepEqual([gone.state.focused, gone.marks[0]?.pane], [2, 2]);
    const renamed = await client.markRename({ name: "notes", to: "journal" });
    assert.deepEqual(renamed.marks.map((m) => m.name), ["journal", "Tab 1"]);
    const removed = await client.markRemove({ key: "n" });
    assert.deepEqual([removed.marks.map((m) => m.name), removed.mark_removed], [["Tab 1"], "journal"]);
    await exactState(client);
    await assert.rejects(client.markGo({ name: "journal" }), { message: 'no mark named "journal"' });
  });
});

test("a refused request rejects with the app's message", async () => {
  await withOttospace(async (client) => {
    await assert.rejects(client.taskOpen({ id: 99 }), { name: "Error", message: "no task 99" });
    await assert.rejects(client.type({ pane: 99, text: "x" }), { message: "no terminal pane to type into" });
    await assert.rejects(client.axTree(), { message: "not an app pane" });
    await assert.rejects(client.send({ cmd: "no_such_request" }), /unknown variant `no_such_request`/);
    await assert.rejects(client.send({ cmd: "focus", pane: 1, extra: 1 }), /unknown field `extra`/);
    // The connection is still good after a refusal.
    assert.equal((await client.state()).mode, "normal");
  });
});
