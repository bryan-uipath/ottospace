// Reply shapes the private test instance cannot produce (window hosting is off there), checked
// against fixtures that a Rust unit test also holds to the real serialization.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { calendarReply, decisionReply, snapshot } from "../src/parse.ts";

/** A real `state` reply of an empty app, with `ax_overlay` put in. */
function replyWith(overlay: unknown): Record<string, unknown> {
  return {
    ok: true,
    mode: "normal",
    focus: "nothing",
    state: {
      panes: [],
      copy_asks: {},
      copy_always: [],
      tabs: [{ id: 1, title: "Tab 1", space: { kind: "workbook", id: 1 } }],
      active: 0,
      layout: null,
      focused: null,
      live: null,
      fill_display: false,
      sidebar: null,
      compose: null,
      list: null,
      rail_focus: null,
      repos: [],
      repo_dirs: { dirs: [], fixed: false },
      repos_open: [],
      tasks: { list: [] },
      collapsed: ["done"],
      tasks_view: "state",
      folded_projects: [],
      open_done: [],
      picker: null,
      new_window: null,
      edited: [],
      palette: null,
      inspector: null,
      confirm: null,
      reorder: null,
      clip: null,
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
      features: [],
      listen: null,
      transcripts: { today: [{ name: "2026-11-17-0900-zoom.md", started_ms: 1794819600000, length_ms: 2520000 }], open: null },
      agent_mode: { panes: { "3": { level: "see", task: 2 } }, default: "act", menu: null, ahead: { "4": { level: "see" } } },
      slots: {},
      offers: [],
      offer_list: null,
      slot_dialog: null,
      viewers: {},
      decisions: null,
      focus_edit: null,
      zoom: null,
    },
    bounds: {},
    frames: {},
    headers: {},
    closes: {},
    titles: {},
    tabs: [],
    space_rects: [],
    reorder_bar: null,
    agent_chip: null,
    strip: [0, 0, 316, 28],
    bar_header: null,
    overlays: { sidebar: null, drawer: null, box: null },
    tasks_dot: null,
    zoom_tiles: null,
    drag: null,
    shown: {},
    tab_panes: [[]],
    window: [1907, 985, 1400, 900],
    window_active: false,
    displays: [{ id: 0, frame: [0, 0, 1728, 1117], scale: 2, main: true }],
    out: [],
    windows: [{ main: true, tab: null, display: 0, key: true, fill: false }],
    parking: [1919, 1079],
    return_key: null,
    version: "0.1.0 · local",
    font: { name: "Menlo", size: 13 },
    cell: [7.82666015625, 18],
    notice: null,
    notice_level: null,
    toast: null,
    quit_rows: [],
    confirm_buttons: [],
    settings_errors: 0,
    editor: null,
    hosted: {},
    crop: {},
    terminals: {},
    viewers: {},
    explorers: {},
    copied: null,
    picker_rows: null,
    palette_rows: null,
    goto_rows: null,
    clone: null,
    inspector: null,
    field: null,
    inbox: [],
    needs_you: 0,
    compose_read: false,
    compose_host: null,
    compose_centred: false,
    quick_pills: null,
    compose_plan: null,
    compose_status: null,
    compose_images: 0,
    compose_hint: null,
    compose_choices: { project: null, place: "app · main", agent: "claude", default: true },
    compose_picker: null,
    quick_project: null,
    compose_worktree: null,
    compose_target: null,
    welcome: false,
    compose_menu: null,
    compose_menu_bounds: null,
    compose_tags: null,
    compose_rects: null,
    compose_folder: null,
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
    projects_bar: { on: false, rows: [] },
    filed: null,
    week_focus: null,
    home: { nodes: [], needs_you: [], counts: { needs_you: 0, running: 0, captured: 0, on_hold: 0, done: 0 } },
    home_drawn: { area: [0, 30, 1400, 846], panel: [80, 300, 520, 120], nodes: {}, heading: "Home", lattice_builds: 1, lattice_paints: 2 },
    city: {
      on: { pane: 3 }, pictures: 1, small: ["tab"], places: 10, groups: 3, picture: [1700, 1300], bounds: [340, 42, 850, 650], scale: 2, lit: [[620, 42, 17, 53]],
      lit_tops: [[628, 45]], lit_beams: [[628, 45, 629, 9]], labels: 1, names: [{ tier: "wall", rect: [400, 300, 60, 16] }], spots: { folder: [500, 400], file: null }, failed: null, reads: 2, renders: 1, paints: 9, render_us: 9100, glass: true, lower: 0.1, folder_colors: 0,
      hovered: { file: true, lit: true, beam: [628, 45, 629, 9], outline: null, beacon: [637, 0, 80, 18] },
      first_ms: 140, read_us: 6200, view: { yaw: 34, pitch: 52, zoom: 1, pan: [0, 0], lift: 0 }, grabbed: false, spin: { running: false, result: null },
      agents: [{ task: 1, rect: [618, 0, 22, 22], cone: true, fan: [0, 1], hovered: false, pulsing: false, subagents: 0 }],
      glimmer: { flash: 0, sweep: 0, found: 0 },
    },
    typing: {
      running: false,
      result: { keys: 250, missed: 0, key_write: { n: 250, p50: 0.3, p95: 0.6, p99: 0.9, max: 1.2 }, write_echo: { n: 250, p50: 0.3, p95: 0.6, p99: 0.9, max: 1.2 }, echo_taken: { n: 250, p50: 0.3, p95: 0.6, p99: 0.9, max: 1.2 }, taken_painted: { n: 250, p50: 0.3, p95: 0.6, p99: 0.9, max: 1.2 }, app: { n: 250, p50: 0.3, p95: 0.6, p99: 0.9, max: 1.2 }, total: { n: 250, p50: 0.3, p95: 0.6, p99: 0.9, max: 1.2 }, full: false, busy: false, done: true, hz: 75 },
    },
    closed_apps: [],
    apps_bar: { not_running: [], more: 0, more_open: false, all_count: 0, all_open: false, all: [], filter: "", filtering: false, undo: null },
    apps: [],
    ax_overlay: overlay,
    sneak: null,
    counts: { live: 0 },
    app_presses: null,
    keys_known: null,
    key_reference: null,
    listen_lines: null,
    listen_rects: null,
    offer_rects: null,
    clip_lights: null,
    heading_rects: null,
    marks: [],
    mark_removed: null,
    unfilled: [],
    workbooks: [],
    workbooks_closing: [],
    workbook_offer: null,
    row_menu: null,
    scrollbars: {},
    overflow: null,
    capture: { key: null, bar: null, picture: null, encoded: [], pending: 0, dir: null, brief: null },
    bug: { icon: false, report: null, blanked: null, pixels: null, controls: null },
    spaces: [
      { of: { kind: "home" }, hidden: false, selected: false, tabs: [], tasks: [], front: null, worktree: null, glyph: { kind: "home" }, indicator: "idle", kept: true },
      { of: { kind: "workbook", id: 1 }, hidden: false, selected: true, tabs: [0], tasks: [], front: null, worktree: "/code/app", glyph: { kind: "app", name: "Notes" }, indicator: "working", kept: true },
      { of: { kind: "task", id: 1 }, hidden: true, selected: false, tabs: [1], tasks: [1], front: 1, worktree: null, glyph: { kind: "tasks" }, indicator: "needs_you", kept: true },
    ],
    badges: { "1": "FL" },
    status_bar: {
      mode: [8, 880, 63, 16],
      resting: null,
      place: { text: "~/code/app · main ●", rect: [111, 879, 180, 19] },
      meeting: { rect: [900, 879, 150, 19], join: null },
      agents: { text: "● 2 working · 1 in a terminal · ● 1 needs you", rect: [1200, 879, 180, 19], working: 2, in_terminal: 1, needs_you: 1, terminal: [1290, 879, 90, 19] },
      shrink: 0,
    },
    calendar: {
      on: true, events_today: 3, all_day_today: 1, events_tomorrow: 2, next_start: 1794560400000, stage: "approaching",
      updated: 1794556800000, stale: false, in_call: false, bars: 3,
      band: {
        rect: [6, 868, 1388, 8], span: [1794556800000, 1794585600000], workday: [1794556800000, 1794585600000],
        marks: [[6, 869.5, 1, 5], [1394, 869.5, 1, 5]], now: [659.5, 868, 1.5, 8],
        blocks: [{
          start: 1794560400000, end: 1794562200000, rect: [179.5, 869.5, 86.75, 5],
          kind: "call", show: "tentative", color: "teal", focus: false, lane: 0, lanes: 3, more: 0,
        }],
        outside: [{ count: 1, rect: [1, 870.5, 4, 4] }, { count: 0, rect: null }], status_bar: [0, 877, 1400, 23],
      },
      banner: { join: [0, 28, 40, 20], dismiss: [44, 28, 60, 20] }, alerts: 1, joins: null,
      rows: 4, rail: "near", day: -1, day_start: 1794470400000,
      all_day_line: [70, 66, 236, 20], first_row: [106, 96, 98, 30],
      day_view: {
        rect: [70, 90, 236, 400], viewport: [70, 90, 236, 600], scroll: 120,
        span: [1794556800000, 1794585600000], hours: 6,
        blocks: [{
          start: 1794560400000, end: 1794562200000, rect: [106, 96, 98, 30], column: 0, columns: 2,
          stage: "running", selected: true, more: 0, join: [170, 97, 30, 16],
        }],
        gaps: [{ from: 1794564000000, to: 1794571200000, minutes: 120, rect: [106, 160, 196, 20] }],
        now: { rect: [104, 110, 202, 2], label: [71, 104.5, 28, 13] },
      },
      nav: { back: [140, 38, 14, 19], forward: [232, 38, 14, 19], today: [252, 38, 42, 19] },
      fetch: { state: "failed", at: 1794470400000, error: "http", code: 403 },
      fetch_line: "The calendar answered 403 · last read 09:40",
      fetch_rect: [70, 200, 236, 20], retry: [270, 200, 36, 18], retries: 1,
      band_state: "ready", band_line: null, band_rect: [6, 828, 1428, 11], band_retry: null,
    },
    drawer: {
      body: [880, 40, 470, 820],
      rail: [
        { kind: "task", id: 1, section: "workspace", selected: false, rect: [1360, 48, 36, 36] },
        { kind: "transcript", id: null, section: "everywhere", selected: true, rect: [1360, 800, 36, 36] },
      ],
      width: 470,
      collapsed: false,
      agents: { "1": { model: "claude", ended: false, cols: 60, rows: 45, lines: ["fake claude ready"] } },
      // A native conversation: its shape, never its text.
      native: {
        chats: {
          "2": {
            view: "native",
            native: true,
            transcript: {
              entries: [
                { kind: "user", bytes: 15 },
                { kind: "assistant", bytes: 120, streaming: false },
                { kind: "tool", tool: "Bash", label: "git", state: "running", sub: false },
                { kind: "notice", notice: "interrupted" },
              ],
              dropped: 0,
              cards: [
                { tool: "Bash", label: "git", cut: false, expanded: false, len: 22, ask: null },
                {
                  tool: "AskUserQuestion",
                  label: "AskUserQuestion",
                  cut: false,
                  expanded: false,
                  len: 0,
                  ask: { questions: [{ options: 2, multi: true, picked: [0, 2], answered: true }], cursor: [0, 2], tabs: 0 },
                },
              ],
              turn: "git",
              model: "claude-opus-5-5",
              mode: "default",
              session: "8f14e45f-ceea-467f-a8d3-0242ac130003",
              context: 2300,
              window: 1000000,
              live: true,
              watched: false,
              not_understood: 0,
            },
            replies: [{ blocks: { para: 2, code: 1 }, links: { web: 1, file: 0 } }],
            draft_chars: 0,
            queued: 0,
            images: 0,
            held: false,
            editing: false,
            deny: false,
            armed: false,
            away: false,
            details: false,
            details_open: false,
            unread_open: false,
            selection: { extent: { rows: 2, chars: 54 }, keys: true, texts: [["1:1.0", [933, 320, 407, 45]]] },
            dial: {
              pulse: "needs_you",
              stations: [
                { station: "you", calls: 1, failed: 0, failing: 0, lit: true, running: 1 },
                { station: "shell", calls: 2, failed: 1, failing: 1, lit: true, running: 2 },
              ],
              context: 0.75,
              level: "amber",
              failed: true,
              lit: true,
              timed: true,
              collapsed: false,
              faces: 0,
              faces_more: 0,
              drilled: 0,
              switcher: false,
            },
            working: { verb: "Pondering…", shimmer: false },
            btw: { offered: true, open: true, keys: false, asked: 2, answered: 1, synthetic: 0, failed: 0, waiting: true },
            asked_tags: 1,
            context: { items: 3, via: "server", tools: 7, briefed: false, folder: true, open: false },
          },
        },
        opened: ["web"],
      },
      line: null,
      note: null,
      finish: {
        later: true,
        done: true,
        offered: false,
        rects: { later: [0, 0, 40, 20], done: [44, 0, 50, 20], offer_done: null, offer_later: null },
      },
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
  // `talk`: a native conversation's composer.
  for (const owner of ["compose", "picker", "palette", "inspector", "apps", "note", "talk", "btw"]) {
    assert.equal(snapshot({ ...replyWith(null), field: field(owner) }).field?.of, owner);
  }
  assert.throws(() => snapshot({ ...replyWith(null), field: field("elsewhere") }), /field\.of should be/);
});

test("a calendar reply parses whole, and with no feed says only that", () => {
  // `a_reply_serializes_as_the_sdk_fixture_says` (ottospace-core) holds this file to the Rust types.
  const fixture: Record<string, unknown> = JSON.parse(readFileSync(join(import.meta.dirname, "calendar.json"), "utf8"));
  const raw = { ok: true, ...fixture };
  assert.deepEqual(calendarReply(raw), raw);
  assert.deepEqual(calendarReply({ ok: true, configured: false }), { ok: true, configured: false });
  const meetings = [{ start: "a", end: "b", all_day: false, title: "t", location: null, has_call_link: false, status: "maybe" }];
  assert.throws(() => calendarReply({ ...raw, meetings }), /meetings\[0\]\.status should be busy, tentative, free or away/);
});

test("a filed decision parses: the state's row, and its words labelled untrusted", () => {
  const row = {
    id: 9, source: "slack", via: "socket", triage: "pending", status: "open", revision: 2, filed: 1, updated: 2,
    expires: 3, task: null, options: 1, evidence: 1,
  };
  const filed = snapshot({ ...replyWith(null), filed: { open: 1, to_sort: 1, trusted: 0, rows: [row] } }).filed;
  assert.deepEqual(filed?.rows[0], row);
  const decision = {
    untrusted: true, warning: "w", id: 9, source: "slack", key: "slack:C0:1.0", via: "socket", revision: 2, filed: 1,
    updated: 2, summary: "s", question: "q?", recommend: 0, why: "", triage: "pending", expires: 3, task: null,
    status: "open", closed: null, later: null, revised: ["claims"],
    accepted: { project: null, role: null, claims: false, now_until: null, revision: 0 },
    options: [{ label: "Draft", consequence: "", act: { draft_reply: { text: "made-up" } } }],
    evidence: [{ kind: "file", text: "", at: { path: "/code/app/a.rs", lines: [3, 9] }, provenance: null }],
    suggest: { projects: ["beacon"], role: "core", role_why: "" },
    claims: { due: null, person_waits: true, blocks: [] },
  };
  assert.deepEqual(decisionReply({ ok: true, decision }).decision, decision);
  assert.throws(() => decisionReply({ ok: true, decision: { ...decision, untrusted: false } }), /untrusted/);
});
