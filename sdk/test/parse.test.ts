// Reply shapes the private test instance cannot produce (window hosting is off there), checked
// against fixtures that a Rust unit test also holds to the real serialization.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { snapshot } from "../src/parse.ts";

/** A real `state` reply of an empty app, with `ax_overlay` put in. */
function replyWith(overlay: unknown): Record<string, unknown> {
  return {
    ok: true,
    mode: "normal",
    state: {
      panes: [],
      tabs: [{ id: 1, title: "Tab 1", task: null }],
      active: 0,
      layout: null,
      focused: null,
      live: null,
      current_task: null,
      sidebar: null,
      compose: null,
      list: null,
      repos: [],
      tasks: { list: [] },
      picker: null,
      keys: false,
      visual: null,
      ax_overlay: overlay !== null,
    },
    bounds: {},
    frames: {},
    tab_panes: [[]],
    window: [1907, 985, 1400, 900],
    parking: [1919, 1079],
    return_key: null,
    font: { name: "Menlo", size: 13 },
    cell: [7.82666015625, 18],
    notice: null,
    hosted: {},
    crop: {},
    terminals: {},
    copied: null,
    inbox: [],
    needs_you: 0,
    closed_apps: [],
    apps: [],
    ax_overlay: overlay,
  };
}

test("an overlay with targets parses: `source` is nested, as the app writes it", () => {
  // `a_labelled_target_serializes_as_the_sdk_fixture_says` (ottospace-core) holds this file's
  // targets to what the Rust types serialize to.
  const overlay: unknown = JSON.parse(readFileSync(join(import.meta.dirname, "ax-overlay.json"), "utf8"));
  const raw = replyWith(overlay);
  const parsed = snapshot(raw);
  assert.deepEqual(parsed, raw);
  assert.deepEqual(parsed.ax_overlay?.targets[0]?.source, { type: "app", pane: 4, path: [0, 2], role: "AXButton" });
  assert.deepEqual(parsed.ax_overlay?.targets[1]?.source, { type: "own", id: "rail:apps" });
});

test("a reply the types do not describe is an error that names the place", () => {
  const flattened = { label: "a", frame: { x: 0, y: 0, w: 1, h: 1 }, type: "own", id: "rail:apps" };
  const overlay = { pane: 4, targets: [flattened], count: 1, truncated: { depth: false, nodes: false, timeout: false }, millis: 1, error: null };
  assert.throws(() => snapshot(replyWith(overlay)), /ax_overlay\.targets\[0\]\.source should be an object/);
  assert.throws(() => snapshot({ ...replyWith(null), mode: "zen" }), /mode should be a mode, got "zen"/);
});
