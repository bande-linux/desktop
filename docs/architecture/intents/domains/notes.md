# Notes

Part of [Bande Intents](../../intents.md). Main noun `note`; `notes.folder`. Owner: the Notes app.
Work package D2.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `notes.note` | title text, body rich_text, folder ref, created, modified date_time, by_agent bool (then title and body\*) | title, folder, modified |
| `notes.folder` | name~ text, account ref<accounts.account>, parent ref | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `notes.find` | `text`, `folder`, `modified_after`, `modified_before`, `order` (`modified`, `title`), `trash` | `read`; foreign where what is in it is |  | app; not in the search index | A |
| `notes.get` | `notes` | as `find` |  | app | A |
| `notes.find_folder` | `text` | `read` |  | app | A |
| `notes.create` | `title`, `body`, `folder` | `change`; shared: `send` | `notes.trash` | app | A |
| `notes.append` | `note`, `body`, `at` (`start`, `end`) | as `create` | the earlier text | app; a note keeps no history | A |
| `notes.update` | `note`, `title`, `body` | as `create` | the earlier text | app | A |
| `notes.move` | `notes`, `folder` | as `create` | move back | app | A |
| `notes.trash`, `notes.restore` | `notes` | as `create` | each other | app | A |
| `notes.open` | `note` | `show` |  | app | A |
| `notes.create_export` | `note`, `format` (`markdown`, `html`, `pdf`), `within` | `change`; shared: `send` | `files.trash` | new | B |
| `notes.create_folder` | `name`, `account` | `change`; shared: `send` | removes it while empty | new | B |
| `notes.update_folder` | `folder`, `name` | `change`; shared: `send` | the earlier name | new | C |
| `notes.delete` | `notes` | `critical`; shared: also `send` |  | new | C |

Phrases in Search: "new note {text}", "open note {note}".

The folder may be in the local account, so `notes.create` works offline. Pins, tags and locks do not
exist.
