# Maps

Part of [Bande Intents](../../intents.md). Main noun `place`. Owner: the Maps app. Work package A9.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `maps.place`, `maps.route`, `maps.location` (`:only`) | name\* text, address\* text, at place; length duration, distance number; at place | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `maps.find` | `text`, `near` | `read` in installed regions; a server the person set: `send` |  | new | B |
| `maps.get_location` | `:only` | `critical` (where the person is) |  | new | C |
| `maps.open` | `place` or `route` | `show`; a server the person set: `send` |  | new | B |
| `maps.compute_route` | `from`, `to`, `by` (`foot`, `bicycle`, `car`) | `read` in installed regions |  | new | C |
| `maps.create_bookmark` | `place`, `name` | `change` | removes it | new | C |

No region is downloaded and no server asked unless the person set it up.
