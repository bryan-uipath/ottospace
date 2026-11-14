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
    focus: "nothing",
    state: {
      panes: [],
      tabs: [{ id: 1, title: "Tab 1", task: null }],
      active: 0,
      layout: null,
      focused: null,
      live: null,
      sidebar: null,
      compose: null,
      list: null,
      rail_focus: null,
      repos: [],
      repo_dirs: { dirs: [], fixed: false },
      tasks: { list: [] },
      collapsed: ["done"],
      tasks_view: "state",
      folded_projects: [],
      open_done: [],
      picker: null,
      palette: null,
      inspector: null,
      confirm: null,
      keys: false,
      visual: null,
      ax_overlay: overlay !== null,
      mark_input: null,
      capture: null,
      back_to: null,
      brief: null,
      talk: null,
      talk_width: 480,
      talk_wide: false,
      agents: [],
    },
    bounds: {},
    frames: {},
    headers: {},
    closes: {},
    tabs: [],
    strip: [0, 0, 316, 28],
    bar_header: null,
    drag: null,
    shown: {},
    tab_panes: [[]],
    window: [1907, 985, 1400, 900],
    parking: [1919, 1079],
    return_key: null,
    version: "0.1.0 · local",
    font: { name: "Menlo", size: 13 },
    cell: [7.82666015625, 18],
    notice: null,
    hosted: {},
    crop: {},
    terminals: {},
    copied: null,
    picker_rows: null,
    palette_rows: null,
    inspector: null,
    field: null,
    inbox: [],
    needs_you: 0,
    compose_read: false,
    compose_host: null,
    compose_plan: null,
    compose_picks: { wheres: ["app"], whos: ["claude", "codex"] },
    welcome: false,
    compose_menu: null,
    compose_pointing: null,
    compose_note: null,
    tasks_bar: { empty: true, undo: null, sections: [] },
    tasks_progress: {
      view: "state",
      projects: [],
      today: { done: 0, running: 0, agent_ms: 0, line: "0 done · 0 running · 0s" },
      cards: {},
    },
    sessions_bar: { count: 0, total: 0, range: "today", collapsed: true, rows: [], enabled: false, reads: 0, slow_reads: 0 },
    repos_bar: [],
    home: { nodes: [], needs_you: [], counts: { needs_you: 0, running: 0, captured: 0, on_hold: 0, done: 0 } },
    home_drawn: { area: [0, 30, 1400, 846], panel: [80, 300, 520, 120], nodes: {}, lattice_builds: 1, lattice_paints: 2 },
    closed_apps: [],
    apps_bar: { not_running: [], more: 0, more_open: false, all_count: 0, all_open: false, all: [], filter: "", filtering: false, undo: null },
    apps: [],
    ax_overlay: overlay,
    sneak: null,
    counts: { live: 0 },
    app_presses: null,
    marks: [],
    mark_removed: null,
    workbooks: [],
    workbook_offer: null,
    scrollbars: {},
    capture: { key: null, bar: null, picture: null, encoded: [], pending: 0, dir: null, brief: null },
    groups: [{ task: 1, tabs: [0] }, { task: null, tabs: [1] }],
    badges: { "1": "FL" },
    calendar: {
      on: true, events_today: 3, all_day_today: 1, events_tomorrow: 2, next_start: 1794560400000, stage: "approaching",
      updated: 1794556800000, stale: false, in_call: false, soon: true, bars: 3, chip: true,
      banner: { join: [0, 28, 40, 20], dismiss: [44, 28, 60, 20] }, alerts: 1, joins: null,
    },
    drawer: {
      body: [880, 40, 470, 820],
      width: 470,
      agents: { "1": { model: "claude", ended: false, cols: 60, rows: 45, lines: ["fake claude ready"] } },
      goes_to: null,
      note: null,
      rail: [
        { task: 1, mono: "FL", group: "running", working: true, shown: true, tip: "fix login\nRunning\napp · @claude · now" },
        // A session no task owns, by its runner and id.
        { task: { runner: "claude", id: "8f14e45f-ceea-467f-a8d3-0242ac130003" }, mono: "GT", group: null, working: false, shown: false, tip: "Greet the user\nSession\napp · @claude · 2h" },
      ],
    },
    frame: { number: 3, seq: 2, drawn: 2, waited_ms: 0, window: 41 },
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

test("the field of each owner parses: the Apps section's filter too", () => {
  const field = (of: string) => ({
    of, text: "sk", cursor: 2, selection: [2, 2], selected: "", marked: null, bounds: [0, 0, 100, 20], cursor_x: 12,
  });
  // `note`: a note being typed on a capture. Every reply has it while one is.
  for (const owner of ["compose", "picker", "palette", "inspector", "apps", "note"]) {
    assert.equal(snapshot({ ...replyWith(null), field: field(owner) }).field?.of, owner);
  }
  assert.throws(() => snapshot({ ...replyWith(null), field: field("elsewhere") }), /field\.of should be/);
});
