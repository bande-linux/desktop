# Calendar

Part of [Bande Intents](../../intents.md). Main noun `event`; `calendar.calendar`. Owner: the
Calendar app. Work package D5.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `calendar.event` | title text (foreign with guests or from outside), starts, ends date_time, all_day bool, calendar ref, place\*, notes\* text, url\* url, repeats text, alerts list<duration>, attendees\* list<address>, answer enum, shared bool | title, starts, ends, calendar |
| `calendar.calendar` | name~ text, colour text, shown, read_only, shared bool, account ref | all |
| `calendar.person` | name\*, text; address address | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `calendar.find` | `text`, `starts_after`, `starts_before`, `calendar`, `answer`, `trash` | `read`; foreign where what is in it is, and for invitations |  | app | A |
| `calendar.get` | `events` | as `find` |  | app | A |
| `calendar.find_calendar` | `text` | `read` |  | app | A |
| `calendar.find_person` | `text` (the server's directory) | `send`, foreign |  | app | B |
| `calendar.create` | `title`, `starts`, `ends`, `all_day`, `calendar`, `place`, `url`, `notes`, `repeats`, `alerts`, `attendees` | `change`; attendees or shared: `send` | `calendar.trash` | app; create has no undo | A |
| `calendar.update` | `event`, `span` (`this`, `this-and-following`, `all`) and the same | as `create` | the earlier event | app | A |
| `calendar.trash`, `calendar.restore` | `event`, `span` | as `create` | each other | app | A |
| `calendar.set_answer` | `event`, `answer` (`accept`, `tentative`, `decline`) | `send` |  | app | A |
| `calendar.open` | `event` or `date` | `show` |  | app | A |
| `calendar.compute_free_time` | `starts`, `until`, `length`; gives starts and ends | `read` |  | app; `free_slots` is UI-free inside the app | B |
| `calendar.compute_busy_time` | `people`, `starts`, `until` (RFC 6638) | `send`, foreign |  | app | B |
| `calendar.create_calendar` | `name`, `colour`, `account` | `change`; shared: `send` | removes it while empty | app (`worker.rs`) | B |
| `calendar.update_calendar` | `calendar`, `name`, `colour`, `shown` | as `create_calendar` | the earlier values | app | B |
| `calendar.import` | `file`, `calendar` | `change`; items with `ORGANIZER` or `ATTENDEE`, or shared: `send` | removes those, with the same effect | app | B |
| `calendar.snooze` | `event`, `length` | `change` | `calendar.dismiss` | app | B |
| `calendar.dismiss` | `event` | `change` | shows it again | app | B |
| `calendar.delete_calendar` | `calendar` | `critical`; shared: also `send` |  | app | C |
| `calendar.add_subscription` | `url` | `send` | `calendar.remove_subscription` | app | C |
| `calendar.remove_subscription` | `calendar` | `change` | adds it again | app | C |
| `calendar.create_export` | `calendar`, `within` | `change`; shared: `send` | `files.trash` | app | C |

`repeats` is an RFC 5545 rule, `alerts` are durations before. `compute_free_time` uses the person's
calendars only.
