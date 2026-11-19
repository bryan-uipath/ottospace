# Hosting other apps' windows: approach, measurements, risks

Moved out of the README on 2026-11-11. Technical background for `ottospace-mac` and the shell.

## Approach

macOS cannot reparent another process's window, so ottospace fakes embedding:

- **Live pane:** the real app window, placed over its pane through the Accessibility (AX) API
  (the AeroSpace model). Native input, no capture.
- **Parked pane:** a ScreenCaptureKit (SCK) stream of the window, with the real window parked on a
  1px off-screen sliver.
- **Fallback, not built:** park on a private `CGVirtualDisplay` if sliver-parked windows stop
  rendering.

Input forwarding to parked windows is a dead end: Chrome ignores `CGEventPostToPid` clicks even
on screen (tried window-under-pointer fields, private window-local location, HID-state source).
So interaction means: place the real window over the pane, then replay the click at HID level.

## One title bar per pane

An app pane shows one bar. Parked: the stream starts below the app's title bar
(`SCStreamConfiguration::with_source_rect`), so only ottospace's header shows; the window is
parked that much taller, so its content is pane-sized. Live: the window is raised by the bar's
height, so its own bar covers the pane header and its content lines up with the pane's. Apps
without a plain title bar keep the old behaviour: no crop, no raise.

Rule (`ottospace_core::titlebar::height`): crop only when AX reports a close button and an
`AXTitleUIElement`, no `AXToolbar` child, a title no taller than the button + 4pt, and the title
centred in the strip the button implies (bar = 2 × button offset + button height, 16–40pt).
Any failed read means no crop.

Measured 2026-11-11, macOS 26, attributes read with System Events (frames relative to window top):

| Window | Close button | Title element | Toolbar child | Result |
|---|---|---|---|---|
| ottospace-canary (GPUI) | y+8, 16pt | y+8, 16pt | no | crop 32 |
| Terminal | y+8, 16pt | y+8, 16pt | no | crop 32 |
| Finder (unified toolbar) | y+18, 16pt | y+0, 52pt | yes, 52pt | none |
| Activity Monitor (toolbar) | y+18, 16pt | y+0, 52pt | yes, 52pt | none |
| Chrome (tabs in title bar) | y+12, 16pt | none | no | none |
| VS Code (Electron) | y+9, 16pt | none | no | none |
| Spotify (CEF) | y+24, 16pt | none | no | none |

Safari and Slack were not running. An app pane with a bar gets a header as tall as the bar
(`titlebar::header`), inside the same 2pt gap as the content: the raised live window's bar covers
it exactly, and the pane's border stays visible on all four sides. The header does not shrink in
a short pane. A pane whose raised window would reach above ottospace's top bar gets no crop
(`titlebar::crop`); the layout keeps that from happening, and the top bar is below the menu bar,
so a raised window cannot go under it either. Not hidden: while live, the app's real title bar shows,
including the purple pill macOS draws in it while the window is captured.

## A sharp picture

A parked pane's picture is drawn 1:1: the stream captures the pane's points × the scale of
ottospace's display (`hosted::capture_pixels`), and GPUI draws it into the same pixels (its
sampler is linear; at 1:1 on whole pixels that changes nothing). Three things made it blurry:

- The scale was read at each re-park but the stream was reconfigured only for a new size or
  crop: ottospace moved from a 1× display to a 2× one kept its 1× picture, drawn up 2×.
  The check (every ~500ms) now also compares the scale.
- Windows were parked at the first free corner, whatever its display: with a 2× laptop and a
  1× external, often the external's. A window is drawn at its display's scale, so the app drew
  1× and no capture setting could add pixels. Parking now takes a free corner of a display with
  ottospace's scale (`hosted::parking_corner`): not more, or each park and enter would move the
  window between scales and the app would draw itself again. A window re-parks when the spot
  moves (displays changed, ottospace moved). Recovery counts every free corner as a parking spot
  (`hosted::parking_corners`). With no free corner on a 2× display (an external to the right
  that reaches lower than the laptop), the picture stays 1×.
- Memory: a picture is at most a 5K display's pixels (`hosted::MOST_PIXELS`, 14.7 M, 22 MB a
  4:2:0 frame); a larger pane is captured at a lower scale.

Measured 2026-11-17: a mostly off-screen window on a 1× display, captured at 1:1, equals
`screencapture -l` of it exactly in BGRA, and in 420f 92.7% of pixels within 2 levels (chroma at
half resolution). `e2e_apps.rs` (`a_parked_panes_picture_is_its_window_pixel_for_pixel`) compares
the canary's one-pixel stripes as ottospace draws them with the live window. CI's runner has one
1024×768 display at 1× and no HiDPI modes, so the 2× cases are checked by hand.

## Probe results

Measured with `probe`, a CLI since removed (`d483a37:crates/probe` in git history).
`cargo run -p probe -- "Google Chrome" 5` printed frames/sec by SCK status for on-screen → sliver →
restored. Play a video or animation in the target app first. Live means "complete/s" stays near
the on-screen rate while parked.

- 2026-11-10 Chrome + YouTube (with audio), 5s/phase: ~53 complete fps on-screen and parked. Live.
- 2026-11-10 Slack (Electron) + animated GIF, unfocused, 10s/phase: ~51 on-screen, ~50 parked. Live.
- 2026-11-10 Chrome + YouTube muted (site + player), 30s/phase: 39 on-screen, 51 parked, 56 restored. Live.

## Pop-ups: a new window that does not split

A new window of a hosted app (`adopt_new_windows`, every tick) used to split its app's pane if it
was standard and not small: a Zoom dialog halved Zoom's pane, and in a real call the owner ended up
with "3 windows and 1 stays a popup while 2 end up in splits… i just don't understand the
criteria". The size rule judged each window once, at first sight; nothing on screen said why.

Now **no window goes in a pane unless the user puts it there**: the offer's chip now, or (slots)
a slot they made. `rules::decide` judges it once, on facts the shell reads (`AXSubrole`,
`AXModal`, `AXRole`/`AXParent` for a sheet, its size, the app pane's size), and says why:

- **Split**: only a window a `"do": "split"` rule selects.
- **Float with an offer**: any other window over 100×100, whatever its size (`Why::NoSlot`), and a
  dialog, a panel or a modal window too (`Why::Dialog`, `Why::Modal`: the owner's Zoom meeting
  window was floated as a panel and could not be brought in). The app pane's header shows a chip, `Zoom opened “Zoom Meeting” · no slot
  for this window · ◫ split right ⊟ split down ×` (`Action::WindowOffered`, the shell's alone).
  ◫ or ⊟ (`OfferAccept { axis }`, the user's alone: a client's click is refused with
  `offers::USERS_ALONE`) asks the shell to host it beside that pane (`Effect::HostOffer`, then
  `Adopt`, which splits the way the offer was taken); × (`OfferDismiss`, the user's) leaves it
  floating. Several offers of one pane group: `Zoom opened 3 windows ▾`; ▾ (`OfferList`, the
  user's) opens their list over the pane's top right, each with the three choices, while two or
  more are left (`Model::offer_list`). The offer goes when the window closes (`OfferGone`: the shell checks
  `recovery::window_exists` each tick), when it is in a pane any other way, or when its pane is no
  app pane. Its title is drawn in the chip and never in the state (`offers` holds pane and window
  id).
  **Known limit:** while the app's pane is live its window covers the header, so the chip is not
  seen until the pane parks (the user leaves it). The window floats in front meanwhile.
- **A window that closed.** The owner's call, 2026-11-19 ("explicit, not magical"): a shared
  window's pane (one it found open) waits, empty, while its app runs: `Zoom's window closed ·
  waiting for its next window ×` (`offers::offer_keeps`, on `WindowGone`, sent as soon as the
  window is gone: the 1.5 s wait for a remade window is gone). The app's next window is offered
  in its body, `Zoom opened a window · Bring it here ↵ ×`; several waiting panes of one app take
  its windows in reading order (`Model::waiting_pane`), and a waiting pane with no app pane left
  beside it still hears of them (`adopt_new_windows`). An owned window's pane (New window) closes
  with it; so does a pane whose app quit. The user's click or ↵ fills a waiting pane
  (`Action::Fill`); × closes it, and a window it offered floats on. No offer vanishes while its app has a
  pane: one whose pane closes moves to the newest other pane of its app (`settle_offers`).
- **Float, no offer**: only a `"do": "float"` rule. A floating window has no
  pane, no stream: the window is `AXRaise`d once without
  activating the app. It is moved first only if the user cannot reach it (`rules::reachable`):
  its top has no 120×60 on a screen's visible frame, or, while no pane of the app is up and the
  app is not active (ottospace is in front), no 120 of it beside ottospace's window and its middle
  not over its app's pane, or it lies over a parking spot (put by its parked parent). A cover counts where it lies over the title bar's middle row. Then `rules::float_to` puts it, at its own size, in the middle of the pane, wholly on
  the visible frame of the pane's screen (a pane whose middle is on no screen: the main one's).
  A window that floats with an offer is placed the same way.
- **Leave**: a sheet or a child window (macOS keeps it on its parent), an `ignore` rule (a built-in
  `ignore` beats a user's `split` or `float`; only `manage` undoes it), a window
  under 100×100 with no rule (the canary, a GPUI app, has a 66×20 one off screen). Any other
  subrole (a toast, a popover) is not judged at all, a `split` rule or not, and is looked at again
  each tick in case it becomes a window.

**The decision log.** Each judgement is one `place` line in `<home>/log` (`event_log`, a test run
writes only its own `OTTOSPACE_HOME`): `bundle`, `subrole`, `modal`, `sheet`, `size`, `pane`,
`min_width`, `place` (`split`, `offer`, `float`, `leave`), `why`, and `rule`: the user's
`windows[n]` or a built-in rule that decided or let it through (`manage`), with its action. Never
the title. The chip names the rule too: `no slot for this window (windows[1]: manage)`.

Each window is judged once, at first sight. A pop-up of an app with no pane left is not touched.

Later, not built: per-app memory of where the user put a window (a pane, floating), so the next
one of the same kind is offered there, or put there once the user says so. Slots (slots.md) come
first: a slot that selects the window fills it under its rules before any offer.

AX has no minimum size. The width a window kept when its pane asked for less (read on the
half-second check) stands for one; a window never squeezed has none. It no longer decides a split
(none splits by itself); the log keeps it, to say why an accepted window squeezed its app's.

A floating pop-up is placed once, then left to its app. Live, the app is in front and its pop-up
with it. Parked, ottospace is in front of it, as of every window of the app; entering the pane
brings the app, and its key window, forward again. A child window or sheet moves with its parent,
by macOS. ottospace does not follow, park, give back or close a pop-up.

## The picker's second step: which window

Choosing a running app in ⌘E lists "New window", then its windows (`model/pick.rs`): the shell
reads those the user's pick may put in a pane (`rules::pickable`), front first: every window but
a sheet or child window, a helper under 100×100, or one a rule ignores. A pop-up `place` floated
(Zoom's meeting window, an `AXFloatingWindow`) is listed too, and so is one a `float` rule names:
the user asked for it. A minimised one is not (its stream would draw nothing). and the core lists them, each
with the pane that holds it already. Choosing a held one shows its pane; any other is hosted, shared: given
back when its pane closes, as before. Titles are drawn, never in the state. A test run lists the
made-up apps of `OTTOSPACE_PICK_APPS` (a JSON file) and hosts none: `counts.window_picks` counts the asks.

"New window" asks the app for a window of its own by its recipe, from one table by bundle id
(`new_window::RECIPES`): Chromium browsers `open -n -g -b <id> --args --new-window`, VS Code and
its forks `-n`, Safari, Finder, Terminal and iTerm2 an Apple Event (`osascript`, fixed text; the
first one asks for Automation permission). An app the table does not name is offered "New
window" only if a menu of its has an item titled exactly "New Window" (`new_window::MENU_ITEM`,
English for now; a submenu's counts, File ▸ New ▸ Window), which is pressed over Accessibility
(the app need not be in front). Never ⌘N by guess: in Notes it makes a note. Only the user's pick asks:
it presses in another app. One ask at a time: a second while one waits is refused. The shell notes the app's windows first; the first new one that may
go in a pane is hosted, owned (`Pane::owned`), in the tab it was asked from beside the pane
focused then, even if the user moved on (the tab on show stays; a tab that closed: the one on
show), as the new-task box pins its workspace; before `adopt_new_windows` could split it in as a
pop-up. An app that does not say which windows it has is not asked: an old window would pass for
the new one. An ask that fails (`NewWindowFailed`) is said as it failed, and ends the wait: no
timeout notice after it. No window within `new_window::WAIT_MS` (3 s, from the pick: a tick is sent with it):
the core says so on a tick, and one that comes later (looked for 30 s) stays outside.

Closing an owned pane closes its window (`Effect::CloseWindow`): it goes back where it was first,
then its close button is pressed, so a save question of the app's shows where the user can
answer it; one that does not come back on a screen stays open, and the bar says so. A shared
window goes back. The shell reads `AXEdited` (on the window, or its close
button) of each owned window about twice a second, and again at the user's close (one that does
not answer counts as unsaved); the user's close of one that has unsaved changes asks first, "Close <title>? It has unsaved changes". A client's close, a tab closed and
a quit do not ask: the app's own question is the backstop (a quit gives every window back,
owned or not). The canary marks its windows edited (`edited`) and has File ▸ New Window (⌘N),
so `cargo e2e-apps` checks the flag and the menu press on CI.

Later, not built: ⌘⇧T reopens a closed pane; a per-app "always new window" setting.

## Risks

- Capture costs input: while ScreenCaptureKit streams a Finder window, macOS loses a release of
  most quick double-clicks on it (CI, 2026-11-17: 28 of 48 captured, 0 of 48 not), so Finder opens
  nothing. The stream stops while the pane is live and starts again as it parks.
- Capturable ≠ still rendering. Apps may throttle parked/occluded windows; must be measured per app.
- Parked windows must not overlap each other, or they occlude and throttle.
- AX moves/resizes are synchronous IPC to the target app's main thread: busy apps stall; drag-follow lags.
- Min-size / non-resizable windows won't fit small panes. Sheets, popovers and child windows escape panes.
- Z-order: clicking a real window can bury the ottospace window. Fixing that needs private SkyLight APIs.
- Screen Recording: persistent indicator, periodic re-prompts; DRM / `sharingType = .none` windows capture black.
- TCC: ad-hoc rebuilds churn grants. Needs a signed `.app` with a stable identity.
- N streams cost IOSurface memory (~95 MiB at 4K); capture at pane size, adapt frame rate.
- Lifecycle: sleep/wake, display hotplug, Spaces, Stage Manager, crashes leaving windows parked.

Prior art: AeroSpace (sliver parking, AX), yabai (SkyLight), DeskPad / BetterDisplay
(`CGVirtualDisplay`), AltTab (SCK previews).
