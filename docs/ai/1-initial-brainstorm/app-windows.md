# App windows in the picker, and moving panes

Refines requests Bryan gave in conversation on 2026-11-11. Not agreed. Not built.

## Source (Bryan's words)

> for multi-instance apps - like chrome, i'm thinking enter by default should open a new window.
> maybe shift-enter bindings to existing. if shift-enter and multiple, we might want a chooser

> is there a way to detect which apps allow multiple instances? maybe configurable per-app
> like slack is single instance iirc

> can we also drag and drop windows - like a visual pane manager

## Picker: new window or existing window

Today the picker hosts the app's main window. Proposed:

| Key | App has many windows (Chrome) | App has one window (Slack) |
|---|---|---|
| ↵ | Open a **new** window and host it | Host its window |
| ⇧↵ | Host an **existing** window. If there are several, show them as a second list | Host its window |

- The second list is the same picker with new items: one row per window, by title. Windows
  already in a pane are marked, and choosing one moves focus to that pane.
- A new window appears a moment after we ask for it. The adopt path already sees new windows of a
  hosted app; the change is to put that window in the pane the picker was aimed at, not beside
  the app's other pane.

## Which apps can open more windows

There is no system flag for this. `LSMultipleInstancesProhibited` in an app's Info.plist is about
processes, not windows. Three sources, in order:

1. **Per-app setting** in the user's config (see `scripting.md`). Always wins.
   ```ts
   otto.app("Google Chrome", { newWindow: { menu: ["File", "New Window"] } });
   otto.app("Slack", { windows: "single" });
   ```
2. **The app's menu bar**, read through accessibility: an enabled item named "New Window" (or
   "New … Window") under File means it can. This covers Chrome, Safari, Finder, VS Code, iTerm
   and Terminal with no setup. Pressing that menu item is also how we ask for the window.
3. **Default:** single window. ↵ and ⇧↵ do the same thing.

Not tested yet: how long the menu read takes on a large app, and apps whose "New Window" opens a
window in another Space.

## Moving panes by drag and drop

- Drag a pane by its header. While dragging, the pane under the pointer shows five drop zones:
  left, right, top, bottom (split that pane on that side) and centre (swap the two panes).
- Drop on a tab in the top bar to move the pane to that tab.
- Core: `Action::MovePane { pane, target, zone }` rewrites the split tree. It is a remove followed
  by an insert on a side, so `insert` needs "before" as well as "after". Unit-tested.
- Shell: the drag gesture, the zone highlight, and re-parking app windows after the move. An app
  pane being dragged shows its stream, never the live window.
- Keyboard equivalent, so it is not mouse-only: ⌘⌥⇧ + arrow moves the focused pane in that
  direction.

## Order

1. Tabs (⌘T, the + button, ⌘1-9).
2. New window by default, existing with ⇧↵, per-app detection.
3. Move panes: keyboard first, then drag and drop.
