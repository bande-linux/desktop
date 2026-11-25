# Recordings and speech

Part of [Bande Intents](../../intents.md). Main nouns `recording` and `reading`. Owner: the Recorder
app; `bande-system-intents` for speech. Work package A7, D22.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `recordings.recording` (a view of a file) | length duration, kind `sound` `video`, recording bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `recordings.find` | `text`, `after` | `read` |  | new | B |
| `recordings.get` | `recording`, `properties` (length, kind, size of the picture) | foreign |  | app | C |
| `recordings.start` | `source`, `name`; never unattended | `critical` |  | new | B |
| `recordings.stop` | `recording` | `change` for what the caller started, otherwise `disrupt` for an agent |  | new | B |
| `recordings.create_export` | `recording`, `format`, `from`, `until`, `audio_only`, `within` | `change`; shared: `send` | `files.trash` | new | B |
| `speech.start` | `text`, or a thing with its read grant | `show`; for an agent see the [rules](../../intents.md#intents-that-would-add-up-to-one-of-these) | `speech.stop` | new (Speech Dispatcher) | B |
| `speech.stop` |  | `show` |  | new | B |
| `recordings.pause`, `recordings.resume` | `recording` | `change` | each other | new | B |
