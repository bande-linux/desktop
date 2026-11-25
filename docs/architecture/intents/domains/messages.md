# Messages

Part of [Bande Intents](../../intents.md). Main noun `message`; also `messages.conversation`,
`messages.draft`. Owner: the Messages app's engine. Work package A8.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `messages.find_conversation` | `text`, `unread` | `read` (names) |  | new | B |
| `messages.find` | `text`, `conversation`, `from`, `after`, `before` | foreign |  | new | B |
| `messages.get` | `messages`, `properties` | foreign; does not mark read |  | new | B |
| `messages.create_draft` | `conversation`, `body`, `attachments` | `change` | `messages.trash_draft` | new | B |
| `messages.update_draft`, `messages.trash_draft` | `draft` and the same | `change` | the earlier draft | new | B |
| `messages.send` | `draft` | `send` |  | new | B |
| `messages.update` | `conversation`, `read`, `muted` | `change`; a read receipt: `send` | the earlier values | new | B |
| `messages.delete` | `messages`, `for_everyone` | `critical`; for everyone also `send` |  | new | C |
| `messages.open` | `conversation` | `show` |  | new | B |

Joining rooms, verifying devices, keys and accounts are the person's.
