# Users

Part of [Bande Intents](../../intents.md). Main noun `user`; also `reports` (`report`). Owner:
`crates/services/users-intents`. Work package D17.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `users.find`, `users.get` | `text`; `users`, `properties` (name, administrator, signed in) | `read` |  | app | C |
| `users.create` | `name`; a standard user, and the password is asked by the owner | `authorise` |  | app | C |
| `users.update` | own `name`, `picture` | `authorise` | the earlier values | app | C |
| `users.delete` | `user`, `keep_home` | `authorise`; never the last administrator or oneself |  | app | C |
| `reports.find`, `reports.create` | `text`; `summary` | foreign; `change` |  | app | C |
| `reports.send` | `report` | `send` |  | app | C |
