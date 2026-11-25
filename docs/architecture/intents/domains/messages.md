# Messages

Part of [Bande Intents](../../intents.md). Main noun `message`; also `messages.conversation`,
`messages.draft`. Owner: the Messages app. Work package A8.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `messages.conversation`, `messages.message`, `messages.draft` | name~ text, unread integer, muted bool; from\* address, body\* rich_text, sent date_time, attachments list<ref>; conversation ref, body rich_text, attachments list<ref> | name; from, sent |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `messages.find_conversation` | `text`, `unread` | `read` (names) |  | new | B |
| `messages.find` | `text`, `conversation`, `from`, `after`, `before` | foreign |  | new | B |
| `messages.get` | `messages`, `properties` | foreign; does not mark read |  | new | B |
| `messages.find_draft`, `messages.get_draft` | `conversation`; `drafts` | `read` |  | new | B |
| `messages.create_draft` | `conversation`, `body`, `attachments` | `change` | `messages.trash_draft` | new | B |
| `messages.update_draft`, `messages.trash_draft` | `draft` and the same | `change` | the earlier draft | new | B |
| `messages.send` | `draft` | `send` |  | new | B |
| `messages.update` | `conversation`, `read`, `muted` | `change`; a read receipt: `send` | the earlier values | new | B |
| `messages.open` | `conversation` | `show`, without marking read |  | new | B |
| `messages.create_files` | `attachments`, `within` | `change`; shared: `send` | `files.trash` | new | B |
| `messages.delete` | `messages`, `for_everyone` | `critical`; for everyone also `send` |  | new | C |

Joining rooms, verifying devices, keys and accounts are the person's.
