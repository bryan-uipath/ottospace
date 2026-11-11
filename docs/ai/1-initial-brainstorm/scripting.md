# Scripting in TypeScript

Refines #5 (scripting / window handling) with Bryan's direction from 2026-11-11. Not agreed.
This replaces the earlier decision "declarative rules before a scripting language".

## Source (Bryan's words)

> opening the spinner could be a scriptable command - ie, something we could declare in our TS
> language, like "if zoom() is open and in a meeting, add a quick action button to open spinner"

## Shape

A user script declares **conditions**, and what exists while a condition is true: quick actions,
layouts, window rules. ottospace re-checks conditions when facts change.

```ts
// ~/.config/ottospace/config.ts
import { otto, zoom } from "ottospace";

otto.when(() => zoom.inMeeting(), {
  // Windows of a meeting go to named slots.
  layout: {
    tab: "Meeting",
    slots: { main: "65%", side: ["top", "bottom"] },
    place: [
      { window: zoom.window("Zoom Meeting"), slot: "main" },
      { window: zoom.window(/^Participants/), slot: "side.top" },
    ],
  },
  // A button in the status bar and an entry in the switcher, only during a meeting.
  quickActions: [
    {
      title: "Spin",
      icon: "dice",
      key: "s",
      async run() {
        const names = await zoom.participants(); // read from the accessibility tree
        if (names.length === 0) return otto.notify("Cannot read the participant list");
        const list = names.map(encodeURIComponent).join(",");
        await otto.open(`https://bryan-uipath.github.io/spinner/#names=${list}`, { split: "right" });
      },
    },
  ],
});
```

`zoom` is not built in. It is a small adapter script, like the per-app adapters for persistence
(#6): `inMeeting()`, `participants()`, `window(title)`, written with generic primitives
(`otto.app(name).windows()`, `.ax.find(role, title)`, `.ax.press()`).

## Action deck (Bryan, 2026-11-11)

> a right pane to host quick actions (like a streamdeck sort of thinking) when available
> an icon + small title, contextually available. tab should smoothly come in and out when
> contextual command is available. i guess it could also be where we have an action to start a task

- A narrow column on the right edge. Each action is a tile: icon above a short title.
- **Contextual.** The deck slides in when at least one contextual action exists and slides out
  when none do. No empty deck.
- **Two groups.** Contextual actions on top (from scripts, e.g. "Spin" during a meeting). Below
  them, a few always-there actions: start a task, capture. If only the always-there group has
  entries, the deck stays hidden and those live in the switcher. To confirm.
- **Keys.** Each tile shows its key. In sneak mode the tiles get labels like any other control.
- **Script side.** A quick action gains an `icon` name. Nothing else changes.
- **Core.** `Model` holds the visible actions in order. Deck open or closed is derived:
  `!contextual.is_empty()`. The slide is shell-only animation.

## How scripts run

| Option | For | Against |
|---|---|---|
| **Sidecar process** (bun or node) that talks to the control socket | No JS engine in the app. Real TypeScript, npm, async. A script crash cannot crash the app. Same API agents use | One more process. Needs bun or node installed |
| Embedded engine (QuickJS / V8 in the Rust binary) | Single binary | Large dependency or weak TS support. A bad script can stall the UI |

Recommended: **sidecar**. It fits what exists: the control socket is already the one way to drive
ottospace, so scripts, agents and tests share one protocol. The `ottospace` TS package is a typed
client for that socket.

**Runtime: bun, bundled** (Bryan, 2026-11-11).

- Ship one pinned bun binary inside the app bundle (`Contents/Resources/bun`). Users install
  nothing, and every install runs scripts on the same version.
- Lookup order: `$OTTOSPACE_BUN`, then the bundled binary, then `bun` on `PATH`. Until there is a
  signed `.app`, development uses `PATH`.
- Pin by version and SHA-256. The build downloads that exact release and fails on a mismatch.
  Never fetch "latest".
- Signing: the bundled binary is signed with the app. bun uses a JIT, so under the hardened runtime
  it needs the `allow-jit` entitlement. Check this when the `.app` bundle exists.
- Cost: the app grows by roughly the size of the bun binary (tens of MB).
- Include bun's licence notices in the bundle.

What the socket needs for this:

1. **Events.** A `subscribe` call that streams facts as they change: window appeared or closed,
   app launched or quit, pane focused, mode changed. Conditions re-run on events, not on a timer.
2. **Registration.** `quick_action_add` / `quick_action_remove`, `layout_apply`. ottospace draws the
   button; pressing it sends an event back to the script, which runs `run()`.
3. **AX queries.** `ax_find`, `ax_press`, `ax_read` scoped to an app. This is also backlog item 2
   (accessibility tree as a first-class surface).

## Where logic lives

- **Core:** quick actions and layouts are data in the `Model` (who registered it, title, key, when
  it shows). Applying a layout template to a set of windows is a pure function. Unit-tested.
- **Shell:** the socket, the AX calls, drawing the buttons.
- **Script:** the conditions and what to do. Not trusted with anything the socket does not offer.

## Trust

Scripts are the user's own code and run as the user, so they add no new power. The socket stays
same-user only. AX access to other apps is strong: keep AX calls scoped to a named app per call,
and log them.

## Steps

1. `subscribe` events on the socket (useful for e2e too: no more polling).
2. Quick actions in the core + status bar and switcher.
3. `ottospace` TS client package and the sidecar launcher (starts `config.ts`, restarts on change).
4. AX query calls.
5. Zoom adapter + the spinner action, after the meeting recording.

## Open questions

1. One `config.ts`, or a folder of scripts with one per app or scenario?
