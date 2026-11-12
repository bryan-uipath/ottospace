// Drives a real, private ottospace over its socket. Needs the debug binary:
// `cargo build -p ottospace`. Each test launches its own instance and kills it.

import assert from "node:assert/strict";
import { test } from "node:test";

import type { Client } from "../src/client.ts";
import { Otto } from "../src/otto.ts";
import { snapshot } from "../src/parse.ts";
import { withOttospace } from "./harness.ts";

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
    await exactState(client);

    // With a split tree, a terminal, a task, an inbox row and the picker in it.
    await client.split({ kind: "terminal" });
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
    assert.deepEqual(two.state.layout, { split: { axis: "down", a: { leaf: 1 }, b: { leaf: 2 } } });
    assert.deepEqual(two.tab_panes, [[1, 2]]);

    const one = await client.close({ pane: 1 });
    assert.deepEqual(one.state.panes, [{ id: 2, kind: { type: "empty" } }]);
    assert.deepEqual(one.state.layout, { leaf: 2 });
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

test("tasks: add, list, open, leave", async () => {
  await withOttospace(async (client) => {
    const otto = new Otto(client);
    const later = await otto.tasks.add("Write the note");
    assert.deepEqual(later, {
      id: 1,
      title: "Write the note",
      state: "captured",
      priority: 3,
      snoozed_until: null,
      dismissed: false,
      project: null,
      repo: null,
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
