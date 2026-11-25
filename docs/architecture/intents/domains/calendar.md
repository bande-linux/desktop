# Calendar

Part of [Bande Intents](../../intents.md). Main noun `event`; `calendar.calendar`. Owner:
`crates/services/calendar-intents`. Work package D5.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `calendar.find` | `text`, `from`, `until`, `calendar` | `read`; foreign where what is in it is, and for invitations |  | app | B |
| `calendar.get` | `events` | as `find` |  | app | B |
| `calendar.find_calendar` | `text` | `read` |  | app | B |
| `calendar.find_person` | `text` (the server's directory) | `send`, foreign |  | app | C |
| `calendar.compute_free_time` | `from`, `until`, `length` | `read` |  | app; `free_slots` is UI-free inside the app | B |
| `calendar.compute_busy_time` | `people`, `from`, `until` (RFC 6638) | `send`, foreign |  | app | C |
| `calendar.create` | `title`, `starts`, `ends`, `all_day`, `calendar`, `place`, `url`, `notes`, `repeats`, `alerts`, `attendees` | `change`; attendees or server: `send` | `calendar.trash` | app; create has no undo | B |
| `calendar.update` | `event`, `span` (`this`, `this-and-following`, `all`) and the same | as `create` | the earlier event | app | B |
| `calendar.trash`, `calendar.restore` | `event`, `span` | as `create` | each other | app | B |
| `calendar.set_answer` | `event`, `answer` (`accept`, `tentative`, `decline`) | `send` |  | app | B |
| `calendar.open` | `event` or `date` | `show` |  | app | B |
| `calendar.create_calendar` | `name`, `colour`, `account` | `change`; server: `send` | removes it while empty | app (`worker.rs`) | B |
| `calendar.update_calendar` | `calendar`, `name`, `colour`, `shown` | as `create_calendar` | the earlier values | app | B |
| `calendar.delete_calendar` | `calendar` | `critical` |  | app | C |
| `calendar.add_subscription` | `url` | `send` | `calendar.remove_subscription` | app | B |
| `calendar.remove_subscription` | `calendar` | `change` | adds it again | app | B |
| `calendar.import` | `file`, `calendar` | `change`; items with `ORGANIZER` or `ATTENDEE`, or server: `send` | removes those, with the same effect | app | B |
| `calendar.create_export` | `calendar`, `within` | `change`; server: `send` | `files.trash` | app | C |
| `calendar.snooze` | `event`, `length` | `change` | `calendar.dismiss` | app | B |
| `calendar.dismiss` | `event` | `change` | shows it again | app | B |

Things: `calendar.event`, `calendar.calendar`.

`repeats` is an RFC 5545 rule, `alerts` are durations before. `compute_free_time` uses the person's
calendars only.
