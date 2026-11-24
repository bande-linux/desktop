# Session

Part of [Bande Intents](../../intents.md). Main noun `session`; power and staying awake. Owner:
`crates/services/session-intents`. Work package D14.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `session.get` | `properties` (battery, uptime, system version, what blocks sleep) | `read` |  | part | A |
| `session.pause` | `how` (`lock`, `sleep`) | lock `change`, sleep `disrupt` |  | ready | A |
| `session.stop` | `how` (`log-out`, `restart`, `shut-down`), `at` | `disrupt` | `session.cancel` while it waits | ready; `at` is new | A |
| `session.cancel` | `:only` | `change` |  | new | A |
| `session.start_awake`, `session.stop_awake` | `until` | `change` | each other | ready (`services/power`) | A |

Open work is never forced: an app that asks about it keeps the session, and the plan ends
`declined`.
