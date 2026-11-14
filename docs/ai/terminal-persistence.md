# Terminal persistence

Status: design, not built. Backlog item 10. Written 2026-11-13. Its first piece, session
restore's layout file, is built (`layout.json`, `session-restore.md`); the holders are not.

Goal: quit, update or reopen ottospace and lose nothing that runs. Each terminal (a workspace
tab's, a task's, the agent's in the conversation drawer, a workbook tab's) goes on running and
comes back with its screen and scrollback. With session restore's layer 1, each task comes
back as it was.

This is a layer under everything, not a workbook feature. A workbook is a template: what to open
again, later, with no process in it (`workbook.md`). Session restore puts the layout back
(`session-restore.md`). This note keeps the processes alive under both.

## Today

- `Terminal::spawn` (`terminal.rs`) forks the shell on a PTY inside the app, through
  `alacritty_terminal::tty`. The app's own `Term` parses the output; its `EventLoop` thread owns
  the master. The app ending closes the master, and the shell gets SIGHUP.
- Quit (`Effect::Quit`) ends everything. `confirm.rs` asks first, listing what runs: "Quitting
  ends these". The task-start branch notes that the question does not cover drawer agents yet.
- Tabs are kept in `layout.json` (session restore layer 1): after quit a task comes back with
  its tabs and splits, and each terminal as a new shell in its folder under its last lines. What
  ran in it is gone.
- `osi` says after an install: "Quit puts hosted windows back; terminal panes and what runs in
  them close."

## Prior art

| Tool | How | Quit | Crash | Update | Logout / reboot |
|---|---|---|---|---|---|
| SupaCode | Sessions run in zmx, "not as children of the app"; "quit and relaunch, and every session reattaches exactly where you left it, scrollback included". A quit option tears all down | Live | Live | Live | Lost |
| zmx | One daemon and one Unix socket per session. The daemon feeds PTY output to the client and to a headless `libghostty-vt`, which holds state and scrollback; on attach it sends a snapshot. Socket dir 0750 by default; logs on by default; an IPC change in an upgrade "will kill all your sessions" | Live | Live | Lost on IPC change | Lost |
| tmux / screen | A server owns the PTYs and its own grid; clients draw tmux's rendering, not the program's (zmx's table: no 1:1 emulator features, no native scrollback). Client and server of other versions refuse each other | Live | Live | Old server lives on | Lost (tmux-resurrect saves layout) |
| iTerm2 | Jobs run in "long-lived servers"; crash and upgrade restore them. "Quitting with Cmd-Q will terminate your jobs" unless a setting says otherwise. macOS window restoration keeps scrollback | Lost (default) | Live | Live | Text only |
| Zellij | Serialises layout, the command in each pane and optionally scrollback; commands wait behind "Press ENTER to run" | Text, command | Text, command | n/a | Text, command |
| cmux | Layout, cwd, scrollback "best effort", agent resume by captured session id; "does not checkpoint arbitrary live process state" | Text, agent resume | Same | Same | Same |
| Ghostty | `window-save-state`: position, size, tabs, splits; cwd with shell integration. No scrollback, no processes | Layout | Layout | Layout | Layout |
| Terminal.app | macOS Resume: windows and their text; per-window shell state in `~/.zsh_sessions` | Text | Text | n/a | Text |

Two families: hold the process (SupaCode/zmx, tmux, iTerm2) or save and resurrect (Zellij,
cmux, Ghostty, Terminal). Only the first keeps an agent mid-task. Nobody survives a reboot.

## Options

**(a) Our own PTY holder.** An `ottospace` subcommand holds PTY masters outside the app. The
app attaches, detaches and replays. Survives quit, update and an app crash. A holder crash,
logout and reboot are lost.

**(b) Bundle zmx or depend on tmux.**
- zmx is the right design, but: a Zig binary to ship and sign beside ours; an internal
  socket protocol we would speak without a contract, where its own upgrade kills sessions;
  group-readable socket dir and logs on by default.
- tmux puts a second emulator between the program and ours (modes, mouse, scrollback all
  re-emulated). It depends on the user's version and `~/.tmux.conf`, and refuses
  mismatched versions.

**(c) Hand the PTY fds across an `exec`.** Clear `FD_CLOEXEC`, exec the new binary, serialise
the grids. No new process, but it survives only an update: not quit, not a crash. Hosted
windows would have to survive the exec too.

**Decision: (a), built the way zmx is.** One holder per session (zmx and abduco do this), not
one daemon for all:
- After an update, new sessions run the new build and old ones keep theirs until they end.
  Nothing is ever restarted to upgrade.
- A holder that crashes loses one terminal, not all.
- No idle-exit logic: a holder exits when its program has exited and an app has seen the end.

### The holder

`ottospace hold` (hidden subcommand), started by the app with `posix_spawn`.
- **Start.** It calls `setsid` and ignores SIGHUP, so closing the terminal ottospace was started
  from no longer ends it. stdio goes to `/dev/null`, cwd is `/`, `RLIMIT_CORE` is 0. It then
  spawns the child with `alacritty_terminal::tty::new`, as `Terminal::spawn` does today:
  same login shell, same `$OTTOSPACE_SHELL`, the same fallback for a cwd that is gone.
- **Env and cwd** come in the create request: the app's environment plus the pane's variables,
  as today. The holder's own environment is not passed on.
- **State.** A headless `alacritty_terminal::Term` (already a dependency) is fed every byte
  read from the master: the shadow. It holds the screen, the alternate screen, the modes and
  history up to the app's scrollback limit. The app keeps its own `Term` for drawing,
  selection and visual mode, fed the same bytes (zmx's model). No raw byte ring: a ring that
  wraps loses the escape that entered the alternate screen or set a mode.
- **Attach.** The holder sends a snapshot as VT bytes: history lines, the primary screen,
  `CSI ?1049h` and the alternate screen if it is up, cursor position and shape, modes (app
  cursor, bracketed paste, mouse, focus), title. Then live bytes. VT bytes, not a serde dump of
  the grid, so a newer app reads an older holder. About 300 lines of serialiser. Not restored:
  inline images, a half-received escape.
- **Size.** One client at a time; a second attach takes over (the old one is told). The
  client sets the size. While detached the size stays and nothing sends SIGWINCH. On reattach
  the pane's size is applied once, and the program redraws.
- **Never block the child.** The holder always reads the master. Detached, bytes go only to
  the shadow; history beyond the limit drops its oldest lines. To an attached client, a bounded
  queue (8 MiB): a client that falls behind is dropped and reattaches to a fresh snapshot.
  Nothing is lost that the shadow holds.
- **Memory.** One shadow per session. alacritty allocates history rows as they fill, so a quiet
  shell costs little. The worst case is about `lines × cols × 24 B`: 10 000 × 200 is 48 MB. The
  app's own copy is the same again. The status bar names the limit when a session nears it.
- **Ends.** The child exits: the holder keeps the final screen and the exit status until an
  app attaches and reads them, then exits. With no app it waits 7 days, then exits.

### Permissions (TCC)

macOS charges a privacy prompt to the responsible process. That process is found by launch
lineage and kept across fork and exec. Today: Terminal.app starts ottospace, ottospace starts
the shell, the shell starts the agent, and Accessibility and Screen Recording are Terminal's.
- A holder spawned by the app keeps that lineage after `setsid` and after the app exits. So a
  program in a pane prompts as before, under the terminal app, or under `ottospace.app` when
  started from the Dock.
- The holder itself needs no permission: it only reads and writes a PTY.
- Rejected: starting holders from a LaunchAgent. launchd would make each holder responsible
  for itself, an ad hoc identity per build, so prompts would come per build (`install-and-update.md`).
- **To verify:** that the attribution holds after the starting terminal app quits.

## The model

- **`SessionId`**: 16 hex digits, made by the shell's random source, passed in through an
  `Action` (core stays pure), checked in the core and again where a path is made. 16, not 32:
  the socket path must fit macOS's 104-byte `sun_path` under `$TMPDIR`.
- **A terminal pane names its session**: `Pane.session: Option<SessionId>` for `Kind::Terminal`.
  Distinct from the pane's `key`: the holder owns the session, the model owns the pane.
- **Layout file.** `session.json` (session restore layer 1) gains `"session": "<id>"` on each
  terminal pane, and `"talk": [{ "task": 12, "session": "<id>" }]` for drawer agents. Workbooks
  and marks never carry one: a workbook opened twice must not share a process.
- **Lifecycle.**
  - Create: `Effect::SpawnTerminal { pane, session, cwd, argv }`. The agent's command (`agents.rs`)
    is one more argv.
  - Detach: on quit, and on a crash (the socket closes). Never a kill.
  - Reattach on start: `Action::Restore(Session)` with `Action::SessionsListed`, the live
    holders. The core decides per pane:
    - live: `Effect::Attach`;
    - ended: its final screen, then "exited 0 while ottospace was closed";
    - gone (reboot, holder crash): layer 2's path, a new shell in the pane's cwd under the saved
      scrollback.
  - Kill: closing the pane (⌘W, ⌘⇧W, the ×) ends the session, as today.
- **Orphans**: a live holder no pane names (a crash before the layout was written, or a
  `session.json` turned off). Listed in a "Detached terminals" row in the Tasks section, with
  attach (into a new tab) and kill. An orphan that is at a prompt with no jobs and was not
  attached for 7 days is ended; one that runs a program never is.
- **Agents in the drawer** are sessions like any other. An update no longer kills a running
  agent. If the holder is gone (reboot), the task's agent session link (`Task.session`, or
  Claude's `--session-id`) offers "Go on" (`claude --resume <id>`, `codex resume <id>`) as
  `task-conversation.md` already does. Never on its own at start: it costs tokens and acts.
- **Quit.** ⌘Q quits without a question: nothing is lost. A capture draft still asks
  (`quit_discards`). "Quit and End Everything" (File menu, ⌘⌥Q) is the explicit choice. It asks
  with today's list, now including drawer agents. It is `Origin::User` only; a socket
  `quit` detaches.
- **Origins.** Attaching an orphan, killing from the list and ending everything are User only
  (`Model::refusal`). A pane's close stays what it is today.
- **Control socket** is unchanged: `type`, `capture`, `wait` reach a session through the app.
  Holder sockets are internal and not in `skill/guide.md`.

## Close and reopen, piece by piece

| Piece | Provides | Gone when |
|---|---|---|
| 1. `layout.json` (session restore layer 1, built) | Tabs with their tasks, splits, ratios, focus, the tab on show, the drawer's conversation. Each terminal's session id when holders come | Never; written a second after a change and at quit |
| 2. Holders (this note) | The live processes, screen, scrollback, modes | Logout, reboot, holder crash |
| 3. The agent's own session file | `--resume` when 2 is gone | Agent's own retention |
| 4. Hosted app panes | App identity (bundle, name, pid, window id). On quit windows go back to the desktop (today). Reopening re-hosts them by layer 2's matching; documents and URLs are layer 4 | n/a |

What the user gets, with layer 1 and stages 1 to 3 built (today: nothing, a fresh workspace):

| | Terminals | Agents | Task tabs | Hosted apps |
|---|---|---|---|---|
| After quit | Live, as left | Running | As left | Re-hosted (layer 2), else a placeholder |
| After an app crash | Live; panes from the last write, others as orphans | Running | As of ≤1 s before | Given back by `hosted.txt`, then re-hosted |
| After update | Live, held by the old build | Running | As left | As after quit |
| After reboot | New shells in their cwd; scrollback text with layer 2 | "Go on" resumes by id | As left | Started again (layer 3), else a placeholder |

## Trust

A holder's socket is as strong as a shell: it writes keys to the program and reads everything
it printed. That is what the control socket's `type` and `capture` already give, with one
difference: holders outlive the app.
- **Where.** `$TMPDIR/ottospace-terminals/<id>.sock`. macOS's `$TMPDIR` is per user, 0700, and
  emptied at boot. The folder is made 0700. Before every use it is checked with `lstat`: owned
  by the user, mode 0700, not a link. The socket is bound under `umask 077`, so it is never
  wider than 0600, not even between bind and chmod.
- **Who.** On accept, `getpeereid` must give the holder's own uid, or the connection closes.
  Another user cannot reach the folder. A sandboxed app cannot connect to a socket outside its
  container. A same-user, unsandboxed process can, and could already type through the control
  socket or edit `~/.zshrc`: nothing new is opened to it.
- **What.** A closed request set: `hello` (protocol version), `create` (argv, cwd, env, size),
  `attach`, `detach`, `input`, `resize`, `info` (child pid, foreground pgid, cwd, exited), `kill`.
  Frames are length-prefixed and capped (1 MiB). `create` takes an argv list, never a shell
  string. No network listener, ever.
- **Secrets.** The shadow holds terminal output, so it can hold secrets. It lives in memory only:
  no log of content (zmx logs by default; we do not), no core dumps, no swap file of our own.
  Writing scrollback to disk is layer 2's, with its own 0600 files and switch.

## Updates

- `osi` replaces the file by rename. A running holder keeps its old inode and goes on; `osi`
  never stops one.
- `hello` carries a protocol number. The app speaks the current one and the one before. A
  holder too old for the app is listed as "held by an older build", kept running, and offered
  only kill. It is never ended by the update.
- Holders are not restarted; they end with their program. A new session always runs the new
  build.
- The `running` file (`osi-simple-output` branch) names the app, not holders. `osi`'s line
  becomes "quit and start it again: your terminals keep running"; `uninstall` names live
  holders and leaves them.

## Stages

Each ships alone. Session restore layer 1 comes first: without a layout there is nowhere to
reattach.

1. **Plain terminals survive quit and update.** `ottospace hold`, the protocol, the shadow and
   its snapshot, `Pane.session`, `session.json` carrying ids, attach at start.
2. **Agents and the quit question.** Drawer agents become sessions; ⌘Q stops asking; "Quit and
   End Everything" lists running programs and agents.
3. **Crash.** Orphans adopted or listed, ended sessions show their last screen, holder-gone
   falls back to layer 2.
4. **The list.** "Detached terminals": see, attach, kill, the 7-day rule.

Not attempted: logout and reboot (processes die; that is session restore's resurrect path:
layer 2 scrollback, agent resume), checkpointing a process, two clients on one session,
attaching from another terminal app, remote hosts.

## Tests

- **Core**: `Pane.session` survives splits, moves and tab drags; no two panes share an id; closing
  a pane emits `Kill`; `Quit` emits none; `QuitEndAll` kills every session and only from
  `Origin::User`; restore's attach, ended and gone cases; orphan rule by injected time. New actions get
  `MAKERS` in `prop.rs`; `check` gains "no shared session, no kill on plain quit".
- **Holder** (unit, in `ottospace`): snapshot round trip. Feed bytes to a shadow, replay its snapshot
  into a fresh `Term`, compare grids and modes (alternate screen, bracketed paste, cursor).
  Peer check and folder checks refuse a wrong mode, owner or link. Frame cap.
- **`cargo e2e`**: holders in a test folder (`OTTOSPACE_TERMINALS_DIR` under `$TMPDIR`; an E2E run
  without it makes a private one and ends its holders when the app exits), never the user's.
  - Stage 1: start `sleep 999` and text in a pane, quit over the socket, relaunch on the same
    home and folder, `capture` shows the text and `state` the program running.
  - Stage 2: a stand-in agent (`OTTOSPACE_AGENTS_DIR`) survives the same quit and relaunch.
  - Stage 3: kill the app with SIGKILL, relaunch: the session comes back or is listed.
  - Stage 4: the list's attach and kill (`"as":"user"`).
- **Not by test**: TCC lineage, checked by hand once per macOS release and noted here.

## Open questions

1. **⌘Q with nothing lost: no question at all?** Recommend yes; a capture draft still asks.
2. **Orphan rule: 7 days, idle only?** Recommend yes; never end one that runs a program.
3. **Should the drawer resume an agent on its own after a reboot?** Recommend no: "Go on" stays a
   click.
4. **A setting to turn holders off** (terminals end with the app, as today)? Recommend yes,
   `"terminal": {"persist": false}`, for anyone who wants quit to mean quit.
5. **`ottospace attach <id>` from another terminal?** Recommend not now: a second client means
   size arbitration.
6. **`OTTOSPACE_PANE` in a shell's environment goes stale** when ids are renumbered at restore.
   Recommend adding `OTTOSPACE_SESSION`, stable, and documenting `OTTOSPACE_PANE` as "at start".
7. **Ship before session restore layer 1?** Recommend no: without a layout every terminal would
   come back as an orphan.

Sources: [SupaCode README](https://github.com/supabitapp/supacode),
[zmx README](https://github.com/neurosnap/zmx), zmx `docs/posts/architecture.md` and
`configuration.md` in that repository, [iTerm2 restoration](https://iterm2.com/documentation-restoration.html),
[Zellij session resurrection](https://zellij.dev/documentation/session-resurrection.html),
[Ghostty `window-save-state`](https://ghostty.org/docs/config/reference#window-save-state),
[cmux session restore](https://cmux.com/docs/session-restore), Terminal.app from
community sources only (no Apple document found).
