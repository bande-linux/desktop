# Camera

Part of [Bande Intents](../../intents.md). Main noun `camera`. Owner: the Camera app. Work package
A6.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `camera.camera` | name~ text, in_use bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `camera.find` | `text` | `read` |  | new | B |
| `camera.open` | `mode` (`photo`, `video`) | `disrupt` |  | new | B |
| `camera.create_photo` | `camera`, `delay`, `within` | `critical`, with the indicator | `files.trash` | new | C |
| `camera.start_recording`, `camera.stop_recording` | `camera`, `sound` | `critical`; stopping as `recordings.stop` | each other | new | C |

For an agent a picture it took cannot be read, converted or attached.
