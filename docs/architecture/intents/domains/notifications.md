# Notifications

Part of [Bande Intents](../../intents.md). Main noun `notification`; also `clipboard`. Owner:
`crates/services/notify`, `crates/services/clipboard`. Work package D16.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `notifications.notification` | app ref, title\*, body\* text, made date_time | app, made |
| `notifications.rule` | app ref, rule `on` `quiet` `off` | all |
| `clipboard.item` | title\* text, made date_time, types list<text> | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `notifications.find` | `text`, `app` | foreign |  | ready | B |
| `notifications.find_rule` | `app` | `read` |  | ready | B |
| `clipboard.find`, `clipboard.get` | `text`; `items`; concealed items do not exist | `critical`, foreign |  | ready | C |
| `notifications.create` | `title`, `body`; no actions, and the agent is named | `show` | `notifications.dismiss` | ready | A |
| `notifications.set_rule` | `app`, `rule` (`on`, `quiet`, `off`) | `change`; for an agent `quiet` or `off`: `disrupt` | the earlier rule | ready | A |
| `notifications.dismiss` | `notifications`, `app` | `change` |  | ready | B |
| `clipboard.set` | `text` or a file | `change` or `disrupt`, see the [rules](../../intents.md#intents-that-would-add-up-to-one-of-these) | the earlier item, unless concealed | ready | B |
| `clipboard.delete` | `items`, `older_than` | `disrupt` |  | ready | B |

Activating another program's notification has no intent.
