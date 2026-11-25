# Session

Part of [Bande Intents](../../intents.md). Main noun `session`; power and staying awake. Owner:
`bande-system-intents`. Work package D14.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `session.session` (`:only`) | battery integer, charging bool, uptime duration, version text, blockers list<text>, stops date_time, computer, processor, graphics~ text, memory integer, battery_left duration | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `session.get` | `:only`, `properties` | `read` |  | part | A |
| `session.pause` | `how` (`lock`, `sleep`) | lock `change`, sleep `disrupt` |  | ready | A |
| `session.stop` | `how` (`log-out`, `restart`, `shut-down`), `at` | `disrupt` | `session.cancel` while it waits | ready; `at` is new | A |
| `session.cancel` | `:only` | `change` for what the caller started, otherwise `disrupt` for an agent |  | new | A |
| `session.start_awake`, `session.stop_awake` | `until` | `change`, at most 4 hours; else `critical` | each other | ready (`services/power`) | A |

Phrases in Search: "lock", "sleep", "stay awake {length}".

Open work is never forced: an app that asks about it keeps the session, and the plan ends
`declined`.
