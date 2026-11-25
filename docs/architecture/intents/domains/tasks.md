# Tasks

Part of [Bande Intents](../../intents.md). Main noun `task`; `tasks.list`. Owner:
`crates/services/calendar-intents`, the one writer. Work package A2.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tasks.find` | `text`, `list`, `due_after`, `due_before`, `done` | `read`; foreign where what is in it is |  | new | B |
| `tasks.get` | `tasks` | as `find` |  | new | B |
| `tasks.find_list` | `text` | `read` |  | new | B |
| `tasks.update` | `tasks`, `done` and the same | as `create` | the earlier tasks | new | B |
| `tasks.trash`, `tasks.restore` | `tasks` | as `create` | each other | new | B |
| `tasks.create_list` | `name` | `change`; server: `send` | removes it while empty | new | B |
| `tasks.create` | `title`, `list`, `parent`, `due`, `starts`, `priority`, `notes`, `repeats`, `alerts` | `change`; server: `send` | `tasks.trash` | new | A |
| `tasks.open` | `task` | `show` |  | new | A |
| `tasks.update_list` | `list`, `name`, `colour` | `change`; server: `send` | the earlier values | new | B |
| `tasks.delete_list` | `list` | `critical` |  | new | C |

Things: `tasks.task`, `tasks.list`. VTODO (RFC 5545) over CalDAV, and a local list.

There is no Tasks app and no VTODO model: see the [plan](../../../plans/apps/tasks.md). Calendar
drops collections without events, and a resource with only a VTODO gives it nothing; one beside an
event is kept for round trips. `bande-dav` and `bande-content-lines` on the `accounts` branch are
what it builds on.
