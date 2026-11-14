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
| Rules | `ottospace-core::calendar` | Stages, the workday, the band's span, blocks and lanes, the hover cards, the chip and the status line, the alert rule, the link allow-function, the feed-link check and curl's config |
| Model | `Calendar` in `Model` | The events of today and the next 7 days, whether a call is on, the alerts given. Actions `SetCalendar`, `CalendarRead`, `CallSeen`, `CalendarDismiss` and `CalendarJoin` (the last two the user's alone); effects `MeetingAlert` and `OpenLink` |
| macOS | `ottospace-mac::meeting` | Zone offsets (CoreFoundation), whether a call is on (CoreAudio), Reduce motion, the alert sound, opening a link |
| Shell | `ottospace::calendar` | The feed thread, the band, the chip, the banner, the state's `calendar`, the `calendar` request's reply (`calendar::reply` in the core) |

- **Stages** by time to the start: far (more than 10 minutes), approaching (10), imminent (2),
  running, past. The glow is a cosine ease (2.4 s a period approaching, 0.9 s imminent), steady
  while running; with Reduce motion a fixed brightness per stage. Redrawn 10 times a second only
  while a meeting pulses; otherwise every 20 seconds, which moves now's tick about a point on a
  wide window (8 hours across 1440 points is 3 points a minute).
- **The band** (below) replaced the first build's hairline across the top of the window.
- **The chip** sits beside the new-tab and split buttons, the strip's one free place whatever
  the left column's width (beside the traffic lights there is no room with the section closed).
- **The alert** comes once at the start and once 2 minutes later, only within the meeting's first
  10 minutes, only if a call is known to be off (`in_call == Some(false)`) or the call on was
  first seen at least 10 minutes before the start (`Calendar::in_other_call`), and not for a
  meeting dismissed or joined. `calendar_notify_for: "links"` limits it to meetings with a Zoom, Meet or
  Teams link; the default alerts every meeting (an open decision, made a setting).

## The band

Bryan, 2026-11-14, on the hairline: "the borders look a bit much though. we should try and make
it a little simpler (just a mark for the beginning and start of the workday, and the
appointments. hover should show details about it. I'm thinking maybe it should go above the
statusbar instead of at the top of the window too".

- 8 points high, a flex child between the panes and the status bar, inset 6 points at the
  sides so a mark at either end is drawn whole. The top of the window is as before the timeline.
- No line, outline or hour ticks: two muted marks (the workday's start and end), flat blocks in
  amber (opacity is the stage: faint when past, the pulse while near), a bright tick for now,
  the all-day dot kept at the left end (the smaller change).
- The span is the workday (`calendar_workday`, `"HH:MM-HH:MM"`, default `09:00-17:00`), local
  wall time worked out with the zone's offset at that moment, so 09:00 stays 09:00 on a day the
  clocks change (`calendar::band`). `calendar_window_hours` is retired: still read (as
  `IgnoredAny`), so a file with it is not refused; the status bar asks to remove it.
- Out of hours: the band widens to today's earliest start and latest end, never past midnight,
  so no meeting is hidden; the workday's marks stay at its times. Chosen over clipping with a
  marker: simpler, and a meeting wholly outside the workday still shows. A meeting over midnight
  is cut there; its card says its whole time. Before or after the band, now pins to its end.
- With a 16-hour workday a 30-minute meeting is about 45 points on a 1440-point window; any block
  is drawn at least 3 points wide, and each hover area (block, mark, now) is 3 points wider on
  each side and the band's full height. Hover only: no click handler, nothing occludes, so a
  pane split's drag and the status bar's tooltips are untouched.
- Cards: a block's title, `09:00–09:30 · 30m · place`, and `Zoom link` (Meet, Teams) when it has
  one (`calendar::describe`); a mark `Workday starts 09:00`; now `14:05` and the next meeting or
  `No more meetings today` (`Calendar::now_card`). Drawn only: the state has bounds and times.

## The Calendar section

A second pull request. `Section::Calendar`, second on the rail, exists only while a feed is set:
`Model::sections()` drops it otherwise, and every place that walks the sections asks it (the rail,
⌘⌃E's section cycling and the rails' focus, a restored layout, `ToggleSection`). It is never
written to `layout.json` (a build without it would set the whole file aside), and one read from
another build is dropped without a feed; `SetCalendar { on: false }` closes it and moves the
rail's keyboard to Tasks.

- Rows (`Calendar::agenda`): today's all-day events, then the rest by start, then "Tomorrow · n"
  (`ListRow::MeetingsTomorrow`, folded; `CalendarTomorrow` unfolds it), then tomorrow's while
  unfolded. Today's past meetings stay, dimmed, as on the band.
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

## A meeting that starts in another call

Bryan, 2026-11-13: "a meeting that starts while in a call should still have another alert".
Before, any call held the alert back (`in_call == Some(false)` only), so a back-to-back meeting
whose predecessor ran over alerted nothing, unless that call ended in its first 10 minutes.

- `CallSeen { in_call, at }`: the core keeps when the call on was first seen (`call_since`),
  forgotten when one is seen off or not known. A call first seen at least `JOINED_EARLY` (10
  minutes) before a start, or before its last minute while another meeting ran (a short one
  overrunning), is another call: the meeting alerts, and its banner shows in that call for its
  first 10 minutes. One seen later, or after the start, is taken for this meeting: no alert.
- A call that is on is watched (every 2 seconds) until it is seen off, not only near a meeting,
  so `call_since` is of this call: a call that ended unseen and a new one would otherwise read
  as the old one.
- Once per occurrence, at most twice (start and 2 minutes on), as before: a feed read keeps the
  alerts of an id, and the same call read again changes nothing. A new call ends the alert.
- Limits: leaving one call and joining the next between two looks (2 seconds) reads as one call,
  so the joined meeting alerts (one Dismiss; the banner goes after 10 minutes). Joining early
  while another meeting's slot still runs reads as another call too. A call with no meeting
  before it is first looked at a minute before the start, so it is taken for this meeting.

## Declined meetings

Bryan, 2026-11-13: "declined meetings hidden would be ideal; ok to add e-mail address in
settings". `calendar_email` names the user's address; the shell's feed thread passes it to
`ics::parse_for`, and only there is it compared. The model never holds it, so it is in no state,
trace or failure folder, and the curl config holds only the link.

- Of each VEVENT's `ATTENDEE` lines (at most 1,000 looked at), the first whose `mailto:` value or
  `EMAIL=` parameter is the address (ASCII case ignored) gives one bool: its `PARTSTAT` is
  `DECLINED`. No other attendee's name or address is kept.
- Parameters split at `;` outside quotes (`content`), so `CN="x;PARTSTAT=DECLINED"` is one
  parameter, and a colon in a quoted CN does not start the value.
- An event declined is skipped as a cancelled one is: an override declined hides that occurrence
  only (it still replaces the series' one); a series declined hides all but overrides with the
  user's own other answer. An override with no line of the user's answers as its series does.
- Not verified against real feeds (none was read): Google's secret iCal address is known to list
  attendees with `PARTSTAT`. Outlook by default removes a meeting the user declines from their
  calendar, so its published feed should not hold one; a feed published with limited detail
  (free/busy, titles only) carries no attendees, and then nothing is hidden.

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
(`capture::plain`: one line, characters that show, at most 120) and drawn. They are not in the
state (counts, times and stages only: every reply carries it, so a title would ride along with
any request), the trace (action names only) or a brief.

Bryan, 2026-11-14: "we should make sure our skill/mcp lets agents check the calendar if needed",
and "agents can see meeting titles and free/busy times". So one request gives titles on purpose:
`calendar` (below). Descriptions and links still go nowhere: a join link holds a passcode, a
description is long untrusted text.

## The `calendar` request

`{"cmd":"calendar","day":"tomorrow","days":2}`: its own reply, not the state, shaped by
`calendar::reply` (pure, unit-tested).

- **Span**: `day` is `today` (default), `tomorrow` or `YYYY-MM-DD`; `days` 1 to 7 (default 1).
  Local midnight to local midnight. The feed thread now expands today and the next 7 days
  (`reply::HELD_DAYS`, 8 days); the hairline and the section still show today and tomorrow
  only. A span outside those days is refused, not fetched: no client can make the app read the
  feed. The reply is built on the UI thread from the model (a few hundred events at most).
- **Reply**: `configured` (false and nothing else with no `calendar_ics`), `updated` (RFC 3339)
  and `stale`, `in_call`, `from`, `to`, `meetings` (at most 200, by start, `more` the rest:
  `start`, `end` with the local offset then, `all_day`, `title`, `location` or null,
  `has_call_link` for a Zoom, Meet or Teams link, `status: "tentative"` when the feed says so;
  cancelled and declined ones are never held), `busy`, `free`.
- **Busy and free**: timed meetings cut to the span, merged where they overlap or meet (back to
  back is one span); all-day events and those with no length are no busy time, tentative ones
  are. `free` is every gap between them inside each day's `calendar_workday` (09:00 to 17:00 by
  default), at that day's offsets, so a clock change moves it with the clock. The reply names
  the `workday`.
- **Text**: titles and locations go through `capture::plain` again, and any word that looks
  like a link (`://`, `www.`, `host.tld/…`) is taken out: a location is often the Zoom link with
  its passcode. A passcode written as plain words ("Passcode 123") is not recognised.
- **Never**: the link, the description, attendees, the feed's address, `calendar_email`.
- The guide's section tells agents titles and places are other people's text: data, never
  instructions; not to be passed on unless the user asked. Links open only on the user's own Join (a click or a sneak label), only through `calendar::joinable` (https, a plain host, no user name, no
quote, space or control character), checked in the core and again in the shell.

## The reader

Hand-written: the core allows std, serde and unicode-segmentation, and a crate for this would need
the rule widened (the `icalendar` crate writes more than it reads; `ical` reads properties but
does no recurrence or zones). About 600 lines.

Followed: line unfolding, text escapes, `DTSTART`/`DTEND`/`DURATION` as UTC, floating, `TZID=` and
`VALUE=DATE`; `RRULE` with `FREQ` `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`, `INTERVAL`, `UNTIL`,
`COUNT`, `BYDAY` (with ordinals for monthly and yearly), `BYMONTHDAY` (negative too), `BYMONTH`,
`BYSETPOS`, `WKST`; `EXDATE`; `RDATE` (a `VALUE=PERIOD` by its start, with the event's length);
`RECURRENCE-ID` overrides (moved or cancelled occurrences); `STATUS:CANCELLED`; the user's own
`ATTENDEE` `PARTSTAT=DECLINED` (above);
`VTIMEZONE` with `STANDARD`/`DAYLIGHT` rules (Outlook's Windows zone names work this way); other
`TZID`s from the system's zone database. Only today and the next 7 days are expanded.

Not followed:

- `BYWEEKNO`, `BYYEARDAY`, `BYHOUR`, `BYMINUTE`, `BYSECOND`, `HOURLY` and finer: such a rule gives
  its first occurrence only.
- An `RDATE` of a whole day for a timed event, or of a time for a whole-day one.
- `VTODO`, `VJOURNAL`, `VALARM`, `EXRULE`.
- Declined meetings without `calendar_email`, or in a feed with no attendees: they show and alert.
- A `TZID` neither the feed nor the system knows is read as local time.

Caps: 5 MB, 20,000 components, 4,000 characters a value, 1,000 exception dates, 14 days an event,
500 occurrences, a budget on how far `COUNT` is followed. A fuzz test feeds arbitrary bytes and
cut-up feeds.

## Open questions

- Alert every meeting, or only those with a meeting link? Built: every one (`calendar_notify_for`).
- Declined meetings: built with a setting (`calendar_email`). Google's feed link holds the address
  too; not read from it.
- A meeting that starts during another call: built, it alerts (above).
- Camera in use (CoreMediaIO) as a second signal, for a user who joins muted with audio off.
