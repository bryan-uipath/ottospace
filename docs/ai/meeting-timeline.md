# Meeting timeline

Bryan, 2026-11-13: "an option for an ICS to show a 'timeline' at the top of the screen … i missed
two meetings today due to not paying attention … something non-intrusive but available." Of three
mock-ups he chose a hairline, "pulsing up to the meeting, and then a notification if zoom isn't
open when the meeting starts". Later: the hairline's left edge is now and the hours roll past it;
a Calendar section in the left rail (a stacked pull request).

User-facing description: [reference](../reference.md#meeting-timeline).

## As built

| Part | Where | What |
|---|---|---|
| Reader | `ottospace-core::ics` | A pure iCalendar reader: events, recurrence and time zones, for a span of time |
| Rules | `ottospace-core::calendar` | Stages, the hairline's blocks, ticks and lanes, the chip and the status line, the alert rule, the link allow-function, the feed-link check and curl's config |
| Model | `Calendar` in `Model` | Today's and tomorrow's events, whether a call is on, the alerts given. Actions `SetCalendar`, `CalendarRead`, `CallSeen`, `CalendarDismiss` and `CalendarJoin` (the last two the user's alone); effects `MeetingAlert` and `OpenLink` |
| macOS | `ottospace-mac::meeting` | Zone offsets (CoreFoundation), whether a call is on (CoreAudio), Reduce motion, the alert sound, opening a link |
| Shell | `ottospace::calendar` | The feed thread, the hairline, the chip, the banner, the state's `calendar` |

- **Stages** by time to the start: far (more than 10 minutes), approaching (10), imminent (2),
  running, past. The glow is a cosine ease (2.4 s a period approaching, 0.9 s imminent), steady
  while running; with Reduce motion a fixed brightness per stage. Redrawn 10 times a second only
  while a meeting pulses; otherwise every 20 seconds, which moves a block under a point on a wide
  window (8 hours across 1440 points is 1 point a minute).
- **The hairline** is absolute over the top 6 points of the window, inset 10 points from the
  rounded corners: the window's buttons start 7 points down, so it touches nothing and takes no
  height. Hour labels on hover: at 6 points a printed number does not read.
- **The chip** sits beside the new-tab and split buttons, the strip's one free place whatever
  the left column's width (beside the traffic lights there is no room with the section closed).
- **The alert** comes once at the start and once 2 minutes later, only within the meeting's first
  10 minutes, only if a call is known to be off (`in_call == Some(false)`), and not for a meeting
  dismissed or joined. `calendar_notify_for: "links"` limits it to meetings with a Zoom, Meet or
  Teams link; the default alerts every meeting (an open decision, made a setting).

## The Calendar section

A second pull request. `Section::Calendar`, second on the rail, exists only while a feed is set:
`Model::sections()` drops it otherwise, and every place that walks the sections asks it (the rail,
⌘⌃E's section cycling and the rails' focus, a restored layout, `ToggleSection`). A feed removed
between runs: the restored section is dropped; `SetCalendar { on: false }` closes it and moves the
rail's keyboard to Tasks.

- Rows (`Calendar::agenda`): today's all-day events, then the rest by start, then "Tomorrow · n"
  (`ListRow::MeetingsTomorrow`, folded; `CalendarTomorrow` unfolds it), then tomorrow's while
  unfolded. Today's past meetings stay, dimmed: only the hairline drops them.
- ↵ on a meeting is `CalendarJoin` through `ListChoose`: refused for a client while the Calendar is
  open (`calendar::USERS_ALONE`), so a client's key there opens nothing.
- The rail cell (`Calendar::rail`): a steady dot while a meeting is imminent or runs, a filled
  badge while the banner is up. No pulse there.
- The list refreshes with the feed (every `calendar_refresh_minutes`), so the header has no
  refresh button: the footer says when it was read.
- Fixed with it: a layout restored after the user had opened the drawer's task listed that task
  twice on the right rail (found by the property test once its makers changed; `layout_file`).

## Is the user in a call?

The alert wants "you are not in the meeting that just started". Signals tried, least invasive
first:

| Signal | Reads | Result |
|---|---|---|
| A process records: CoreAudio's `kAudioProcessPropertyIsRunningInput` on each audio process object (macOS 14.2) | One flag per audio process, no name. No window, title or log; no permission prompt | **Chosen.** Checked on this machine with `ffmpeg -f avfoundation -i ":1" -t 3 -f null -` (3 seconds of the built-in microphone, discarded): 0 processes running input before, 1 during; `afplay` of a system sound raised running *output* only. Works for Zoom, Meet in a browser and Teams alike |
| A microphone device running: `kAudioDevicePropertyDeviceIsRunningSomewhere` on every input device | One flag per audio device | Also flips with the recording above, but is device-wide: a headset that only plays music would read as in use. Kept only as the fallback before macOS 14.2 |
| The unified log: `log show --last 20s --predicate 'subsystem == "com.apple.controlcenter" AND eventMessage CONTAINS[c] "mic"'` | The user's system log | Readable without admin, and names the process (`Recent activity attributions changed to ["mic:ffmpeg"]`), but costs 2.2 s a query and reads far more than a flag. Not used |
| Zoom's windows: a standard window titled `Zoom Meeting` (see `1-initial-brainstorm/zoom-ax.md`) | Another app's window titles | Zoom only, and reads another app's text. Dropped |
| A Zoom helper process (`CptHost`, `aomhost`) | Process names | Not verifiable without a real meeting (Zoom was not running). Not used |

`ottospace_mac::meeting::call()` returns `InCall`, `NotInCall` or `Unknown` (no way to tell, or a
read failed and none said `InCall`); `Unknown` gives no alert. It is asked every 2 seconds from a minute
before a meeting until it ends, never otherwise, and not in a test run (`calendar_call` stands in).

Limits: a microphone in use for something else (dictation, a voice memo, a recording app) counts as
a call, so the alert is wrongly held back; and a user who joined without computer audio is "in no
call", so the alert comes. The first is the safe failure for "non-intrusive", the second costs one
Dismiss.

## The network

The feed is ottospace's first network access. `/usr/bin/curl` (which `install.sh` already uses)
instead of an HTTP and TLS crate: no new dependency, the system's TLS and proxy settings.

- `curl -q --config -`: the argument list is fixed. `-q` first, so no `~/.curlrc` applies. The
  config on standard input holds the link and the limits: `proto =https`, no redirects
  (`max-redirs 0`, no `location`), `max-filesize` 5 MB (also cut at 5 MB while reading),
  `connect-timeout 10`, `max-time 30`, `globoff`, `fail`, and `write-out "%{stderr}%{http_code}"`.
  No cookie or netrc options: curl uses neither unless asked. A proxy in the user's environment
  (`https_proxy`) applies; then the proxy looks the host up and the pin below does not hold.
- A redirect (a 3xx code) is no read: a redirect could point at this machine's network. The
  status bar says `calendar feed redirects: put its final https address in calendar_ics`.
- The host is looked up once in the shell; the first address that `calendar::public_address`
  accepts (not loopback, private, link-local, carrier NAT, multicast, documentation, unspecified,
  or such an address inside IPv6) is given to curl as `resolve = "host:port:address"`, so curl
  connects there and a second lookup cannot answer otherwise (no DNS-rebinding window). A host with
  no public address is refused with `calendar feed: its host is not on the internet`.
- The link is a secret (a private feed's address holds a token). It is in `settings.json` only;
  not in curl's arguments (so not in `ps`), not in the state, the trace or an error. curl is run
  `silent` without `show-error`, so its stderr holds only the status code, never a message that
  could quote the link.
- `calendar::feed_refusal` refuses anything but `https://` with a named host: no user name, no IP
  literal (any host whose last label is a number or `0x…`), no `localhost`, `.local`, `.internal`,
  `.home`, `.lan`, `.arpa`, no single-label host. A local file is the separate form (`/…`, `~/…`),
  read with the same 5 MB cap.
- Checked by hand against the system curl with a public page (not calendar data): the config is
  accepted, a pinned read answers `200`, a page that redirects answers `301` and is not followed.
- The feed is parsed once a read, and its events worked out again only then or when the day turns.
  The reader's work, zone tables included, is charged to one budget (`ics::expand_counted`), and
  only zones an event names are worked out.
- A test run may name a stand-in in `OTTOSPACE_CURL` (honoured only with `OTTOSPACE_E2E`); the e2e
  test checks its arguments and that the config carries the link.

## The notification

`osascript -e 'on run argv' -e 'display notification (item 1 of argv) with title "Meeting started"'
-e 'end run' 'Started 0m ago: <title>'`, then `NSBeep`. A binary started from a terminal has no
bundle identity for the UserNotifications API, so `osascript` it is; the notification is shown as
Script Editor's. The text is an argument, never part of the script, and starts with our own word,
so a title like `-e …` cannot be taken for an option. Not in a test run (the state counts
`alerts` instead). Posted once by hand with a made-up title to check it shows.

## Trust

Titles, places and descriptions are written by other people. They are cleaned
(`capture::plain`: one line, characters that show, at most 120), drawn, and go nowhere else: not
the state (counts, times and stages only), the trace (action names only), a brief, or an agent.
The guide says so. Links open only on the user's own Join (a click or a sneak label), only through `calendar::joinable` (https, a plain host, no user name, no
quote, space or control character), checked in the core and again in the shell.

## The reader

Hand-written: the core allows std, serde and unicode-segmentation, and a crate for this would need
the rule widened (the `icalendar` crate writes more than it reads; `ical` reads properties but
does no recurrence or zones). About 600 lines.

Followed: line unfolding, text escapes, `DTSTART`/`DTEND`/`DURATION` as UTC, floating, `TZID=` and
`VALUE=DATE`; `RRULE` with `FREQ` `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`, `INTERVAL`, `UNTIL`,
`COUNT`, `BYDAY` (with ordinals for monthly and yearly), `BYMONTHDAY` (negative too), `BYMONTH`,
`WKST`; `EXDATE`; `RECURRENCE-ID` overrides (moved or cancelled occurrences); `STATUS:CANCELLED`;
`VTIMEZONE` with `STANDARD`/`DAYLIGHT` rules (Outlook's Windows zone names work this way); other
`TZID`s from the system's zone database. Only today and tomorrow are expanded.

Not followed:

- `BYSETPOS`, `BYWEEKNO`, `BYYEARDAY`, `BYHOUR`, `BYMINUTE`, `BYSECOND`, `HOURLY` and finer: such a
  rule gives its first occurrence only.
- `RDATE`, `VTODO`, `VJOURNAL`, `VALARM`, `EXRULE`.
- Declined meetings: `PARTSTAT=DECLINED` is the user's only if the feed says which attendee is
  the user, and it does not. A declined meeting still shows and alerts.
- A `TZID` neither the feed nor the system knows is read as local time.

Caps: 5 MB, 20,000 components, 4,000 characters a value, 1,000 exception dates, 14 days an event,
500 occurrences, a budget on how far `COUNT` is followed. A fuzz test feeds arbitrary bytes and
cut-up feeds.

## Open questions

- Alert every meeting, or only those with a meeting link? Built: every one (`calendar_notify_for`).
- Declined meetings: read the user's address from somewhere (a setting, or Google's feed link,
  which holds it) to drop them?
- A call that has nothing to do with the meeting (an earlier one running over) holds the alert
  back. Should a meeting that starts during another call still alert?
- Camera in use (CoreMediaIO) as a second signal, for a user who joins muted with audio off.
