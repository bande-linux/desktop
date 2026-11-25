# Routines

Part of [Bande Intents](../../intents.md). Main noun `routine`. Owner: `bande-routines`. Work
package C8.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `routines.routine` | name~ text, trigger text, steps list<text>, enabled bool, last date_time, by_agent bool | name, enabled |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `routines.find`, `routines.get` | `text`; `routines` | `read` |  | new | B |
| `routines.create` | `name`, `at`, `every`, `event`, `intents`, `arguments` | the strongest effect in it | `routines.trash` | new | B |
| `routines.update` | `routine` and the same | as `create` | the earlier routine | new | B |
| `routines.trash`, `routines.restore` | `routine` | `change`; as `create` | each other | new | B |
| `routines.start` | `routine` | its effects |  | new | B |
| `routines.set_enabled` | `routine`, `enabled` | on: as `create`, off: `change` | the earlier value | new | B |

See [Later and by itself](../../intents.md#later-and-by-itself). Search runs them by name and by a
short alias the person gives.
