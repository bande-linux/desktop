# Notes

Part of [Bande Intents](../../intents.md). Main noun `note`; `notes.folder`. Owner:
`crates/services/notes-intents`, on the engine split from the app. Work package D2.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `notes.find` | `text`, `folder` | `read`; foreign where what is in it is |  | app; not in the search index | A |
| `notes.get` | `notes` | as `find` |  | app | A |
| `notes.find_folder` | `text` | `read` |  | app | A |
| `notes.create` | `title`, `body`, `folder` | `change`; server: `send` | `notes.trash` | app | A |
| `notes.append` | `note`, `body`, `at` (`start`, `end`) | as `create` | the earlier text | app; a note keeps no history | A |
| `notes.update` | `note`, `title`, `body` | as `create` | the earlier text | app | A |
| `notes.move` | `notes`, `folder` | as `create` | move back | app | A |
| `notes.trash`, `notes.restore` | `notes` | as `create` | each other | app | A |
| `notes.open` | `note` | `show` |  | app | A |
| `notes.create_export` | `note`, `format` (`markdown`, `html`, `pdf`), `within` | `change`; server: `send` | `files.trash` | new | B |
| `notes.create_folder` | `name`, `account` | `change`; server: `send` | removes it while empty | new | C |
| `notes.update_folder` | `folder`, `name` | `change`; server: `send` | the earlier name | new | C |
| `notes.delete` | `notes` | `authorise` |  | new | C |

Things: `notes.note`, `notes.folder`, `notes.account`.

The folder may be in the local account, so `notes.create` works offline. Pins, tags and locks do not
exist.
