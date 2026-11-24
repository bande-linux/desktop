# Camera

Part of [Bande Intents](../../intents.md). Main noun `camera`. Owner: the Camera app. Work package
A6.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `camera.open` | `mode` (`photo`, `video`) | `disrupt` |  | new | C |
| `camera.create_photo` | `camera`, `delay`, `within` | `authorise`, with the indicator | `files.trash` | new | C |
| `camera.start_recording`, `camera.stop_recording` | `camera`, `sound` | `authorise`; `change` | each other | new | C |

For an agent a picture it took cannot be read, converted or attached.
