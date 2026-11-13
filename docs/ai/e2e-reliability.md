# E2E reliability

Bryan's question of 2026-11-13: CI goes red on tests that pass locally. Can we run every test
several times to find the flaky ones, see why the suite is slow, and get failures that explain
themselves? Should we invest in logging?

## What was wrong

Most red runs were one class of bug: a reply whose state was a frame old. A request changed the
model and replied at once, with the `bounds` of the last frame drawn, which was of the layout
before. A test that read bounds after a change saw the old layout, or the first of two changes
(`panes_resize_swap_and_change_tab_from_the_keyboard` on main's own CI: it waited for pane 1 to
shrink, saw the shrink of the first resize, and asserted the second). A fast machine draws
between two requests and hides it; a loaded CI runner does not.

## Settled replies

The shell counts actions (`seq`, bumped in `dispatch`) and records in `render` how many the frame
drew (`drawn_seq`). In a test run (`OTTOSPACE_E2E`) every reply that carries the state, `state`
included, waits until `drawn_seq >= seq`: the reply carries the current state, so an action
dispatched after its own must be drawn too. The 60 Hz tick sends what a frame has drawn for;
`quit` sends what is held before the app exits. After 5 s with no frame the reply goes anyway, marked
`"stale": true`, so a hidden or stuck window fails a test with a reason instead of hanging it.
Replies that carry no state (`capture`, `wait`, `task_list`, `trace`, ..) never wait.

Only in test runs: a window that is not drawn (minimised, on another Space) would make every
reply wait 5 s. The reply's `frame` (`number`, `seq`, `drawn`, `waited_ms`) is there always, so a
client outside a test can see when what is drawn is a frame behind.

## Failures that explain themselves

The harness (`crates/ottospace/tests/support/mod.rs`) keeps, per test, a timeline of every request
it sent: when, how long the reply took, the reply's frame numbers. Never the request's text. On a
failure it writes `failures/<test>-<pid>/` under `$OTTOSPACE_E2E_OUT`, while the app still runs:

| File | What |
|---|---|
| `summary.txt` | Also the panic message: what timed out and after how long, the frame numbers, the last three requests |
| `state.json` | The last full state |
| `timeline.jsonl` | The test's requests and waits |
| `trace.jsonl` | The app's own trace (below); by `kill -USR1` if the app does not answer |
| `sample.txt` | Timeouts only: `sample <pid> 3`. A main thread in `Window::draw` or layout is stuck drawing; one in `CFRunLoopRun` is idle, never asked to draw |
| `screenshot.png` | `screencapture -l <window>` of the test's own window, when the window server allows it |

## The trace (the logging answer: yes, small and structured)

`trace.rs`: a ring of the last 2,000 events in memory, read by `{"cmd":"trace"}`. Each is a line of
JSON with `ms` (monotonic, since start) and `ev`: `action` (variant name, origin, effect names,
seq), `frame` (number, seq, ms from render to paint), `request`/`reply` (cmd, ms), terminal
spawn and exit, `host`/`park`/`unpark`/`window_gone` (pane and window ids), `timeout`. The rules
of `sessions.md`: names, counts, durations, ids. A variant's name is read by formatting it into a
writer that refuses the first character after the name, so its payload (typed text, a title, a
path) is never even formatted. The core is untouched: the shell records.

Under `OTTOSPACE_E2E` or `OTTOSPACE_TRACE=1` the ring is written to `trace.jsonl` (the home, or
beside the socket in a test run with no home) on a panic and on SIGUSR1; a thread writes it, so
it works while the UI thread is stuck.

## Timing

Each test's `results.jsonl` line has launch to first frame, every wait's time, and the total.
`scripts/e2e-report` sorts them: slowest tests, slowest waits, first-frame median and maximum, and
writes `timings.json`. CI puts the tables in the job summary. That is the level we can act on: a
wait that is slow in every run is a test waiting on the wrong thing; one that is slow in one run
is the machine. To go deeper: the trace's `frame.took_ms` gives each frame's layout and paint
time, its `reply.took_ms` each request's time in the app, and `sample <pid> 3` (or Instruments'
Time Profiler attached to a test's instance by its pid) shows where a slow
frame goes.

## Reliability runs

- CI's e2e job runs the suite under cargo-nextest (`.config/nextest.toml`, profile `e2e`): each
  test in its own process, one at a time (each window takes the same free corner), a failed test
  tried twice more. One that then passes is flaky: listed in the summary, the job stays green.
  One that fails three times fails the job. nextest fits: the tests are `#[ignore]`d
  (`--run-ignored only`), each makes its own socket from its process id, and the harness's
  thread-local log works the same in one process per test.
- `scripts/e2e-repeat N [filter] [--apps]`: N runs with plain `cargo test`, then the tally. No
  nextest needed locally.
- `reliability.yml`: nightly and by hand, `cargo e2e` and `cargo e2e-apps` five times each on
  macOS runners, tallied in the summary. The nightly run opens or updates one issue, "Flaky tests".

## Nothing left in `$TMPDIR`

Runs had left about 12,000 files there: a lock per launch, settings folders, unit tests' folders.

- The app owns its socket and the `.lock` beside it: it removes both when it quits, and the
  lock when it took it but could not listen. A killed app removes nothing, so the harness
  removes its socket, lock and trace dump after the kill.
- An e2e test makes every path with `support::temp(name)`: registered in the test's log and
  removed when its thread ends, pass or fail. The failure folders are under
  `$OTTOSPACE_E2E_OUT`, never registered: they stay.
- A unit test that makes a folder holds a guard that removes it on drop (`Scratch`).
- The first launch of a run sweeps this suite's own leftovers older than a day:
  `ottospace-e2e-<words>-<numbers>[.sock|.lock|.trace.jsonl]`, plain files and folders right in
  `$TMPDIR`, never through a link. `ottospace.sock`, the user's, never matches.
- `a_test_leaves_nothing_in_the_temp_dir` runs two tests again with a `$TMPDIR` of their own (an
  app killed, one that quits, settings folders, a file that is no socket) and asserts it is empty.
