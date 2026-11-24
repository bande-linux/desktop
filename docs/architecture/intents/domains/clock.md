# Clock

Part of [Bande Intents](../../intents.md). Things `clock.alarm`, `clock.timer`, `clock.stopwatch`,
`clock.city`. Owner: the Clock process. Work package D6.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `clock.find_alarm`, `clock.find_timer` |  | `read`; foreign for a label an agent wrote |  | app; the bus gives no ids | B |
| `clock.get_stopwatch` | `properties` (running, elapsed, laps) | `read` |  | app | B |
| `clock.update_alarm` | `alarm`, `time`, `date`, `label`, `enabled`, `repeats` | `change` | the earlier values | app | B |
| `clock.trash_alarm`, `clock.restore_alarm` | `alarm` | `change` | each other | app | B |
| `clock.start_timer` | `length`, `label` | `change` | `clock.stop_timer` | app | B |
| `clock.update_timer` | `timer`, `length`, `label` | `change` | the earlier values | app | B |
| `clock.pause_timer`, `clock.resume_timer`, `clock.stop_timer` | `timer` | `change` | each other; a stopped timer starts again with what was left | app | B |
| `clock.snooze_alarm`, `clock.dismiss_alarm` | `alarm` | `disrupt` |  | app | C |
| `clock.create_alarm` | `time`, `date`, `label`, `repeats`, `sound` | `change` | `clock.trash_alarm` | app; has no date | A |
| `clock.compute_time` | `place`, `at` | `read` |  | app (`cities.rs`) | A |
| `clock.add_city`, `clock.remove_city` | `city` | `change` | each other | app | C |
| `clock.start_stopwatch`, `clock.pause_stopwatch`, `clock.stop_stopwatch` | `:only` | `change` | each other | app | B |
