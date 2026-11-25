# Recordings and speech

Part of [Bande Intents](../../intents.md). Main nouns `recording` and `reading`. Owner: the Recorder
app; `bande-system-intents` for speech. Work package A6, D22.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `recordings.recording` (a view of a file) | length duration, kind `sound` `video`, recording bool | all |
| `speech.reading` (`:only`) | running bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `recordings.find` | `text`, `after` | `read` |  | new | B |
| `recordings.get` | `recording`, `properties` | foreign |  | app | C |
| `recordings.start` | `source`, `name`; never unattended | `critical` |  | new | B |
| `recordings.stop` | `recording` | `change` for what the caller started, otherwise `disrupt` for an agent |  | new | B |
| `recordings.create_export` | `recording`, `format` (`opus`, `flac`, `mp4`), `begin`, `end` (`duration`), `audio_only`, `within` | `change`; shared: `send` | `files.trash` | new | B |
| `speech.start` | `text`, or a thing the caller may read | `show`; for an agent see the [rules](../../intents.md#intents-that-would-add-up-to-one-of-these) | `speech.stop` | new (Speech Dispatcher) | B |
| `speech.stop` |  | `show` |  | new | B |
| `recordings.pause`, `recordings.resume` | `recording` | `change` | each other | new | B |

Phrases in Search: "record".
