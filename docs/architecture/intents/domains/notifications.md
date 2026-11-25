# Notifications

Part of [Bande Intents](../../intents.md). Main noun `notification`; also `clipboard`. Owner:
`crates/services/notify`, `crates/services/clipboard`. Work package D16.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `notifications.find` | `text`, `app` | foreign |  | ready | B |
| `notifications.create` | `title`, `body`; no actions, and the agent is named | `show` | `notifications.dismiss` | ready | A |
| `notifications.dismiss` | `notifications`, `app` | `disrupt` |  | ready | B |
| `notifications.find_rule` | `app` | `read` |  | ready | B |
| `notifications.set_rule` | `app`, `rule` (`on`, `quiet`, `off`) | `change` | the earlier rule | ready | A |
| `clipboard.set` | `text` or a file | `change` or `disrupt`, see the [rules](../../intents.md#intents-that-would-add-up-to-one-of-these) | the earlier item, unless concealed | ready | B |
| `clipboard.find`, `clipboard.get` | `text`; `items`; concealed items do not exist | `critical`, foreign |  | ready | C |
| `clipboard.delete` | `items`, `older_than` | `disrupt` |  | ready | B |

Activating another program's notification has no intent.
