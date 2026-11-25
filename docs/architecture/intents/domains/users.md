# Users

Part of [Bande Intents](../../intents.md). Main noun `user`; also `reports` (`report`). Owner:
`bande-system-intents`. Work package D17.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `users.user` | name~ text, administrator, signed_in bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `users.find`, `users.get` | `text`; `users`, `properties` (name, administrator, signed in) | `read` |  | app | B |
| `reports.find`, `reports.create` | `text`; `summary` | foreign; `change` |  | app | C |
| `users.create` | `name`; a standard user, and the password is asked by the owner | `critical` |  | app | B |
| `users.update` | own `name`, `picture` | `critical` | the earlier values | app | C |
| `users.delete` | `user`, `keep_home` | `critical`; never the last administrator or oneself |  | app | C |
| `reports.send` | `report` | `send` |  | app | C |
