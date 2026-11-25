# Clock

Part of [Bande Intents](../../intents.md). Things `clock.alarm`, `clock.timer`, `clock.city`. Owner:
the Clock app. Work package D6.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `clock.alarm` | time time, date date, label text, enabled bool, repeats text, sound enum, ringing bool | all |
| `clock.timer` | length, left duration, state `running` `paused` `done`, label text | all |
| `clock.city` | name~ text, offset duration | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `clock.find_alarm`, `clock.find_timer` |  | `read`; foreign for a label an agent wrote |  | app; the bus gives no ids | A |
| `clock.update_alarm` | `alarm`, `time`, `date`, `label`, `enabled`, `repeats` | `change` | the earlier values | app | A |
| `clock.trash_alarm`, `clock.restore_alarm` | `alarm` | `change` | each other | app | A |
| `clock.start_timer` | `length`, `label` | `change` | `clock.stop_timer` | app | A |
| `clock.pause_timer`, `clock.resume_timer`, `clock.stop_timer` | `timer` | `change` | each other; a stopped timer starts again with what was left | app | A |
| `clock.snooze_alarm`, `clock.dismiss_alarm` | `alarm` | `change` |  | app | A |
| `clock.create_alarm` | `time`, `date`, `label`, `repeats`, `sound` (`default`, `gentle`, `loud`) | `change` | `clock.trash_alarm` | app; has no date | A |
| `clock.compute_time` | `place`, `at` | `read` |  | app (`cities.rs`) | A |
| `clock.update_timer` | `timer`, `length`, `label` | `change` | the earlier values | app | B |
| `clock.add_city`, `clock.remove_city` | `city` | `change` | each other | app | B |

Phrases in Search: "timer {length}", "alarm {time}".
