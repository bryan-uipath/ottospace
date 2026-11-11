# Hiding panes from screen sharing

Refines a request Bryan gave in conversation on 2026-11-11. Not agreed. Nothing here is tested.

## Source (Bryan's words)

> is it possible to block out / black out certain windows for screen sharing? thinking like for my
> terminal in the meeting pane i might not want to share it

## Short answer

Yes for anything ottospace draws itself (terminals, and app panes while they show a stream).
Not for an app window while it is live, because that is the other app's real window.

## Why it is not one switch

macOS can exclude a *window* from capture (`NSWindow.sharingType = .none`). But all panes are
drawn inside one ottospace window, so that switch is all or nothing. And we cannot set it on
another app's window at all.

## Options

| | How | Hidden from the share | You still see it | Risk |
|---|---|---|---|---|
| A. Curtain | While sharing, ottospace draws a placeholder instead of the pane | Always. The pixels are not there | No | None. Works with every capture method |
| B. Excluded overlay | The private pane is drawn in a child window with `sharingType = .none`, placed over the pane. The main window shows the curtain underneath | If the OS honours the flag | Yes | Must be tested on this macOS version and with Zoom. Newer capture paths may ignore the flag |
| C. Both | B on top of A | Always: if B fails, the share shows the curtain | Yes when B works | Most code |

Recommended: **C**, built as A first. A is safe on its own; B is an upgrade that cannot leak,
because the curtain is always what the main window draws.

## Rules

- A pane (or a pane kind, or a task) is marked **private**.
- While a share is active, private panes use the curtain (and the overlay, when available).
- **App panes:** a private app pane cannot go live during a share. It stays parked and curtained.
  If it is live when the share starts, it is parked first.
- **Fail closed.** If ottospace cannot tell whether a share is active, private panes stay curtained
  until the user says otherwise.
- The status bar shows `SHARING` and how many panes are hidden.

## Knowing that a share is active

- A manual toggle (action deck tile, key). Always available.
- From a script: `zoom.sharing()` read from Zoom's accessibility state, so the curtain drops in on
  its own. See `scripting.md`.

```ts
otto.when(() => zoom.sharing(), { curtain: { kinds: ["terminal"], panes: ["notes"] } });
```

## Where it lives

- Core: `private` flag per pane, `sharing` flag in the model, and one rule: a pane draws content
  only if `!(sharing && private)`. Enter is refused for a private app pane while sharing.
- Shell: the placeholder, the overlay window, the share detection.

## To test before building B

1. Set `sharingType = .none` on a test window. Share the whole screen in Zoom. Is it hidden?
2. Same with a single-window share of ottospace.
3. Same in a QuickTime screen recording and a screenshot.

## Open questions

1. Curtain look: blank, blurred last frame, or Otto?
2. Default: are terminals private unless marked shareable, or shareable unless marked private?
