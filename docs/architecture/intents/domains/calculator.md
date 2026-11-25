# Calculator and dictionary

Part of [Bande Intents](../../intents.md). `calculator` has no things; `dictionary` has the main
noun `entry`. Owner: `bande-system-intents`. Work package D21.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `dictionary.entry` | word, text text, language text, source~ text | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `dictionary.find` | `word`, `language` | `read`; foreign from a dictionary the person added |  | ready | A |
| `calculator.compute` | `expression`; gives a text | `read` |  | ready (`bande_search::calculate`, units included, no network) | A |
| `dictionary.open` | `word` | `show`; refused while an online source is on |  | ready | B |

Phrases in Search: "{expression}", "define {word}".

`dictionary.find` uses installed dictionaries only and returns the entries' text. The calculator's
tape, adding dictionaries and online sources have no intent.
