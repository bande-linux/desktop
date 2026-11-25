# Recordings and speech

Part of [Bande Intents](../../intents.md). Main nouns `recording` and `reading`. Owner: the Recorder
app; `crates/services/speech-intents`. Work package A7, D22.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `recordings.get` | `recording`, `properties` (length, kind, size of the picture) | foreign |  | app | C |
| `recordings.start` | `source`, `name`; never unattended | `critical` |  | new | B |
| `recordings.stop` | `recording` | `change` |  | new | B |
| `recordings.create_export` | `recording`, `format`, `from`, `until`, `audio_only`, `within` | `change`; server: `send` | `files.trash` | new | B |
| `speech.start` | `text`, or a thing with its read grant | `disrupt`; for an agent `send` | `speech.stop` | new (Speech Dispatcher) | B |
| `speech.stop` |  | `show` |  | new | B |
