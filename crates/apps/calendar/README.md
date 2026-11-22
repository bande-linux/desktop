# Bande Calendar

A calendar for the Bande desktop: CalDAV accounts that work offline,
local calendars, and read-only subscriptions, shown in day, week, month and year
views. Packaged as `bande-calendar`.

## What it does

- **Accounts.** The account sheet first asks for the type: CalDAV account (RFC 4791) or
  calendar subscription. For CalDAV it takes an address and a password and looks up the
  server of the address's own domain (RFC 6764: `_caldavs._tcp` and `_caldav._tcp` SRV records
  with their TXT `path`, then `/.well-known/caldav`), without sending the password. The server
  settings step shows what was found, with server URL and user name, to confirm or change;
  Enter Server Settings Manually goes there directly. Sign In follows `current-user-principal`
  → `calendar-home-set` → the calendars in the home set, trying the URL, its
  `/.well-known/caldav` and its root. Credentials go only in answer to a server's challenge,
  as HTTP Basic or Digest (RFC 7616, SHA-256 or MD5), and only over HTTPS (or to this computer,
  for a local server). A failure says what went wrong (unreachable, certificate not valid, user
  name or password refused, unsupported sign-in, no calendar there, server error) and keeps
  every field editable. Passwords go to the Secret Service, never to disk. Several accounts,
  plus a local calendar. Settings ▸ Accounts shows each account's server and user name, and
  Server Settings… changes them; the sheet opens in the Settings window. When the server
  refuses the password, Calendar asks for it again, with Server Settings… in the same sheet,
  and keeps every offline change meanwhile; with its window closed it posts one notification
  instead, which opens the window at the password sheet. DNS lookups use `bande-discovery`,
  shared with Mail.
- **Subscriptions.** `webcal:` / `webcals:` links and plain `https:` `.ics` URLs, read-only,
  refreshed on a wall-clock deadline kept across restarts (every 5 minutes to every week, or
  never; for all of them or one by one in Settings); their alerts are dropped unless turned on
  for that subscription; the name comes from RFC 7986 `NAME` or `X-WR-CALNAME`, the colour from
  `COLOR` (a CSS colour name) or the de-facto `X-APPLE-CALENDAR-COLOR`. The subscription sheet
  also opens for a `webcal:` link given to Calendar.
- **Offline sync.** Every calendar is cached on disk with its pending changes, so it works
  without a network and pushes changes as soon as the network is back (`bande_network::follow_online`).
  Sync is incremental with `sync-token` (RFC 6578), falling back to ctag + etag listings on servers without it. Creates use
  `If-None-Match: *`, edits and deletes `If-Match`; when an edit meets a 412, the server's
  version stays and the local edit is kept beside it as a new event, and the sidebar says so.
  Undo (Ctrl+Z) takes back a delete, a move or a change of length, and Redo (Shift+Ctrl+Z) does
  it again: a deleted event comes back
  with its own UID at its own address (`If-None-Match: *`), so other clients see the same event
  return, and a moved one is written back with its earlier times.
- **iCalendar.** Its own parser and writer: line folding, escaping, unknown properties and
  components kept on round trip, `RRULE` (every frequency and `BY*` part, `COUNT`, `UNTIL`,
  `BYSETPOS`, week numbers), `EXDATE`, `RDATE`, `RECURRENCE-ID` overrides, all-day events,
  `VALARM`, and time zones: `TZID`s resolve through the system tz database (also common
  non-IANA zone names and prefixed names), unknown ones through their `VTIMEZONE` (converted to
  a POSIX rule). Written events carry a `VTIMEZONE` generated from the tz database.
- **Views.** Day, week, month and year, a mini month and the calendar list in the sidebar,
  per-calendar colours and visibility, Today and previous/next. The view comes back at the next
  start. File ▸ Print… prints the view that is shown, with every hour that has an event (and the
  working day) in the day and week views, followed, except for the year, by a list of every
  event in it with its times and location.
- **Calendars.** New, rename and delete, on this computer or on a CalDAV server (MKCALENDAR,
  PROPPATCH of the name and colour, DELETE): File ▸ New Calendar…, or right-click a calendar or
  an account heading.
- **Busy hours.** Overlapping events share the column and widen into free lanes; when there are
  more than fit readably, a "+N" chip opens that day in the Day view.
- **Editing.** Drag on empty time to create, drag an event to move it (also between days in the
  month view and into the all-day row), drag its bottom edge to change its length, and an
  inspector popover with title, location, all-day, start and end, repeat (the usual choices
  or Custom…: every N days, weeks on chosen weekdays, months on a day or on the first…last
  weekday, or years) and when it ends (never, after a number of times, or on a date), alert,
  calendar and notes. Changing a recurring event asks whether it is for this event only (an
  override) or all events.
- **Invitations** through the server (RFC 6638 implicit scheduling): the inspector lists the organizer and invitees with their
  answers, whether one is a room, resource or group (`CUTYPE`), when the server could not
  deliver to them (`SCHEDULE-STATUS` 3.x or 5.x) or leaves sending to someone else
  (`SCHEDULE-AGENT` other than `SERVER`), and who sent an invitation for its organizer
  (`SENT-BY`). Accept, Maybe and Decline update your own `PARTSTAT` (an invitation's title and
  times stay read-only), and on an event you organise in a CalDAV calendar you can add
  invitees by address; the server sends the mail, and a change of time, repeat or exceptions
  raises `SEQUENCE` so it goes out as an update; your own changes to an invitation, such as an
  alert, keep the organizer's `SEQUENCE`. Your addresses come from
  `calendar-user-address-set`.
- **People and rooms.** People and Rooms… in the inspector searches the server's directory
  (RFC 3744 `principal-property-search` on `displayname` and `calendar-user-address-set`) for
  colleagues, rooms and resources; one picked is added as an invitee, a room or resource with
  its `CUTYPE`, and its answer shows like anyone's.
- **Availability.** Availability… in the inspector of a timed event you organise shows, for
  you and every invitee, when they are busy during the event's working day, with the event
  outlined, and offers the next three times within working hours (the day's start and end from
  Settings, on the locale's working days) in the next two weeks when everyone is free; picking
  one sets the event's times. It asks the account's scheduling outbox (RFC 6638 `VFREEBUSY`
  `REQUEST`), and for anyone the outbox does not answer for, or on a server without one, a
  RFC 4791 `free-busy-query` on that person's calendars found through the directory. Someone
  the server shares nothing about is shown as not known.
- **What goes to the server.** Directory searches and availability are asked only when you
  press Search in People and Rooms or open Availability, only of the CalDAV server of the
  event's own account, and carry only the typed text, or the invitees' addresses and the two
  weeks asked about; answers are kept only while the sheet is open.
- **Invitations by mail** (iMIP, RFC 6047), opened from Mail or as a file: a `REQUEST` that
  invites one of your addresses asks Accept, Maybe or Decline, keeps the event with your
  answer and your own alerts (Decline removes it; for one occurrence, only that occurrence is
  updated) and opens a message to the organizer in Mail through a `mailto:`
  link with the answer in words; the message carries no `REPLY` part, since `mailto:` cannot
  attach one. A `REPLY` with one invitee updates that invitee's answer in an event you organise,
  and a `CANCEL` asks to remove the event or the one cancelled occurrence (undoable). An update or
  `CANCEL` from someone other than the stored event's organizer, or with an older `SEQUENCE`, is
  refused. `METHOD` is never stored.
- **Search** over title, location and notes, nearest occurrence first.
- **Alerts** through `org.freedesktop.Notifications`, also while the window is closed, with
  Snooze (5 minutes) and a click that opens Calendar on the event's day. Calendar also shows them when it was never opened or was quit: the autostart
  entry `/etc/xdg/autostart/org.bande.CalendarAgent.desktop` starts `bande-calendar
  --background` at login, a process with no window that is not a running app until Calendar is
  opened, and Quit returns there; both follow "Show alerts when Calendar is closed" (on by
  default), and with it off `--background` exits at once. Alerts,
  the clock and refreshes wait on one `CLOCK_REALTIME` timer, so they fire on time after a
  suspend or a clock change, and the view follows a change of the system time zone at once.
  Alerts that fell due while Calendar was not running, up to a day back, show when it starts.
- **Settings** (Calendar ▸ Settings…, stored in `~/.config/bande/calendar.conf`): default
  calendar, start of the week, the hours the day starts and ends (the rest is shaded and the
  week opens there), hours shown at a time, week numbers, default alerts for events and
  all-day events, alerts when Calendar is closed, refresh intervals, adding, re-signing and removing accounts, and time zone
  support. With it, the inspector shows each timed event's zone, its times are shown and typed
  in that zone, a new zone keeps the wall times of the whole series (exceptions and end date
  move with it), and View ▸ Time Zone shows the calendar in a zone its events use.
- **Files and links.** `bande-calendar file.ics`, File ▸ Import… or a dropped file offers to add
  its events (files are read in the background, several are offered one after another, and one
  that can't be read is named in the sidebar);
  File ▸ Export… (or Export… on a calendar's menu) saves a calendar as one `.ics` file, through
  the desktop's file chooser portal; `bande-calendar webcal://…` offers to subscribe. The
  desktop file registers `text/calendar` and the `webcal` schemes. With the window closed,
  Refresh Calendars syncs in the background and Import… and Export… show only the file chooser;
  the window opens when there is something to confirm.

Shortcuts: Ctrl+1…4 for the views, Ctrl+T today, Page Up/Page Down previous/next, Ctrl+N new
event, Ctrl+F search, Ctrl+R refresh, Delete to delete, Ctrl+Z to undo, Shift+Ctrl+Z to redo, Enter to open the selected event (or the
focused day, in the month and year views), Ctrl+W close and Ctrl+Q quit. The arrow keys move the
focused day (Up and Down by a week in the month and year views, and scroll the hours in the day
and week views), Home and End go to the first and last day shown, and Tab and Shift+Tab select
the focused day's events in turn. F6 moves between the calendar, the calendar list (Up and Down
to choose, Space to show or hide, Menu for its menu) and the toolbar. Every day, event and
calendar is in the accessibility tree.

Dates and times are typed and shown the way the locale writes them; a date that starts with a
four-digit year is read as year-month-day. A date or time that can't be read keeps the inspector
open on that field and says what to type.

## Layout

| Path | What it is |
|---|---|
| `src/ical/` | iCalendar: content lines, values and durations, time zones, recurrence, events, free/busy |
| `src/caldav/` | HTTP, WebDAV XML, DNS SRV, discovery, the CalDAV requests, directory search and free/busy |
| `src/store.rs` | accounts, calendars and the offline cache in `~/.local/share/bande-calendar` |
| `src/sync.rs` | push pending changes, pull with sync-token or ctag/etag, apply the outcome |
| `src/edit.rs` | the edits the UI makes: create, move a series or one occurrence, delete, details |
| `src/schedule.rs` | free times for a meeting within working hours |
| `src/alerts.rs` | which alarms fall due |
| `src/settings.rs` | the settings and the messages the Settings window sends the app |
| `src/wake.rs` | the wall-clock timer for alerts, the clock and refreshes |
| `src/worker.rs` | the network thread the window talks to; passwords through `bande-keyring` |
| `src/ui/` | the window: toolbar, sidebar, the four views, inspector, account sheet, input, Settings |
| `src/main.rs` | the app: window, menus, sync timers, alerts, files and links |

It is a `bande-app` app on the desktop's toolkit, design tokens and theme, like Files and
Settings.

## Tests

`tests/untrusted.rs` opens a hostile `.ics` file without panicking and refuses a FIFO or a file
over 64 MiB. `tests/accounts.rs` finds a CalDAV server from a domain's records and signs in with
Digest after a wrong password, and never sends a password over plain HTTP to anything but this
computer. `tests/time_zone.rs` moves a repeating event to another zone and checks that its times,
exceptions and end follow. `tests/undo.rs` undoes a delete while a sync pushes it and checks
the event comes back at its own address. The whole test list and the reason for each is in
`docs/quality/tests.md`.
