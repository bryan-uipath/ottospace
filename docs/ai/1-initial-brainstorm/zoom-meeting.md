# Zoom: meeting layout and participants → spinner

Refines a scenario Bryan gave in conversation on 2026-11-11 (extends #5 window rules and
"interesting scenario" 2, agentic AX). Not agreed. First design to react to.

## Source (Bryan's words)

> in zoom, it spins up multiple windows when a meeting starts (~3) - i wonder if we can
> automatically create a layout that manages the zoom windows. in addition, a really cool thing
> would be if we could wire up the ax picker to see who is in the meeting and feed to the scrum
> spinner

## Checked so far

- Zoom idle (no meeting) has one window: `Zoom Workplace`, `AXStandardWindow`. Read through
  accessibility on 2026-11-11.
- **Not checked:** the windows a meeting opens, their subroles, and whether the participant list is
  readable. This needs a real meeting. Everything below marked (?) depends on it.
- `~/code/spinner` keeps names only in `localStorage`. It has no way to take names from outside.

## 1. Meeting layout

This is the general window-rules feature (#5), with Zoom as the first rule set.

```
window appears  →  rule matches (app, title, subrole)  →  placed in a named slot of a layout
last match gone →  layout closes, previous tab comes back
```

Example rule set (titles are guesses (?)):

| Match | Slot |
|---|---|
| `zoom.us`, title `Zoom Meeting` | main, 65% |
| `zoom.us`, title `Participants*` | right, top |
| `zoom.us`, title `*Chat*` | right, bottom |
| `zoom.us`, anything else while in a meeting | leave floating, do not host |

- **Trigger.** Today ottospace only polls apps it already hosts. Rules need a watch on apps that
  are *not* hosted yet: an AX observer for window-created on each app named in a rule.
- **Meeting tab.** The first match creates a "Meeting" tab. A notes terminal can be part of the
  layout. When the meeting window closes, the tab closes and the previous tab returns.
- **Where the logic lives.** Core: `rules.match(window facts) -> slot` and the layout template.
  Unit-tested against recorded window facts. Shell: the AX observer and window moves.

Risks:

- **Non-standard windows.** Zoom's floating controls, thumbnail strip and share toolbar are likely
  not `AXStandardWindow` (?). We host only standard windows today. Rule: host what is standard,
  leave the rest floating. Do not fight Zoom for them.
- **Video while parked.** A parked Zoom window must keep rendering video. Measured for Chrome and
  Slack, not for Zoom. Run `probe` on it.
- **Minimum sizes.** Zoom windows may refuse small panes. The slot sizes must respect what the
  window accepts; read the size back after placing.
- **Screen sharing.** Sharing changes Zoom's windows (meeting window hides, toolbar appears) (?).
  The rule set needs a "sharing" state, or the layout must stand down while sharing.
- **Focus mode.** A meeting should be allowed during a focus block, or end it. To decide.

## 2. Participants → spinner

```
Zoom participants panel (AX)  →  names  →  clean  →  spinner opens in a pane with those names
```

- **Read.** Walk the AX tree of the participants panel and take each row's name (?). The panel
  probably has rows only while it is open; if so, open it first with an AX action on the
  Participants button. No screenshot, no OCR.
- **Clean (core, unit-tested).** Strip suffixes like `(Host, me)`, `(Guest)`, `(Co-host)`; drop
  yourself if wanted; drop duplicates and room systems; keep order stable.
- **Feed.** Add one input to the spinner: `index.html#names=Ana,Ben,Chen`. A URL fragment is not
  sent to the server, so names stay on the machine even on the GitHub Pages copy. Names from the
  fragment replace the stored list for that visit only.
- **Trigger.** A scripted quick action, shown only while Zoom is in a meeting. See `scripting.md`.
- **No calendar fallback.** Bryan, 2026-11-11: attendee lists are not up to date. If the panel
  is not readable, the command says so and does nothing.

Privacy: names are other people's data. Keep them in memory. Do not write them to the usage
metrics or to notes unless asked.

## Steps

1. **Record a real meeting.** Dump Zoom's windows (title, subrole, size) and the participants
   panel's AX tree to a fixture file. This removes every (?) above. 15 minutes, needs you in a
   meeting.
2. **Rules in the core.** Matching and layout template, tested against the fixture.
3. **AX observer** for window created and destroyed, in `ottospace-mac`.
4. **Name cleaning in the core**, tested against the fixture.
5. **Spinner `#names=`** (small change in `~/code/spinner`).
6. **E2E** with a fake app that opens three windows, so the test does not need Zoom.

## Open questions

1. Which Zoom windows do you want in panes, and which left floating?
2. Should the spin exclude you, or last time's winner?
3. Meeting during a focus block: allowed, or does it end the block?
