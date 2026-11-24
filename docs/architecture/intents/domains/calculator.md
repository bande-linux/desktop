# Calculator and dictionary

Part of [Bande Intents](../../intents.md). `calculator` has no things; `dictionary` has the main
noun `entry`. Owner: `crates/services/lookup-intents`. Work package D21.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `dictionary.find` | `word`, `language` | `read`; foreign from a dictionary the person added |  | ready | A |
| `calculator.compute` | `expression` | `read` |  | ready (`bande_search::calculate`, units included, no network) | A |
| `dictionary.open` | `word` | `show`; refused while an online source is on |  | ready | A |

`dictionary.find` uses installed dictionaries only and returns the entries' text. The calculator's
tape, adding dictionaries and online sources have no intent.
