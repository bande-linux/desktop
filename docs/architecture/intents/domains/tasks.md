# Tasks

Part of [Bande Intents](../../intents.md). Main noun `task`; `tasks.list`. Owner: the Tasks app.
Work package A2.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `tasks.task` | title\* (own lists: not foreign) text, list ref, parent ref, due, starts, completed date_time, done bool, priority enum, notes\* rich_text, url\* url, message ref<mail.message>, repeats text, alerts list<duration> | title, list, due, done |
| `tasks.list` | name~ text, account ref, colour text, shared bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tasks.find` | `text`, `list`, `due_after`, `due_before`, `done` | `read`; foreign where what is in it is |  | new | A |
| `tasks.get` | `tasks` | as `find` |  | new | A |
| `tasks.find_list` | `text` | `read` |  | new | A |
| `tasks.update` | `task` and the same | as `create` | the earlier tasks | new | A |
| `tasks.trash`, `tasks.restore` | `tasks` | as `create` | each other | new | A |
| `tasks.create` | `title`, `list`, `parent`, `due`, `starts`, `priority`, `notes`, `repeats`, `alerts`, `url`, `message` | `change`; shared: `send` | `tasks.trash` | new | A |
| `tasks.open` | `task` | `show` |  | new | A |
| `tasks.set_done` | `tasks`, `done` | `change`; shared: `send` | the earlier values | new | A |
| `tasks.create_list` | `name` | `change`; shared: `send` | removes it while empty | new | B |
| `tasks.update_list` | `list`, `name`, `colour` | `change`; shared: `send` | the earlier values | new | B |
| `tasks.delete_list` | `list` | `critical`; shared: also `send` |  | new | C |

There is no Tasks app and no VTODO model: see the [plan](../../../plans/apps/tasks.md). Calendar
drops collections without events, and a resource with only a VTODO gives it nothing; one beside an
event is kept for round trips. `bande-dav` and `bande-content-lines` on the `accounts` branch are
what it builds on.
