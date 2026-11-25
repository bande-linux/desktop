# Music

Part of [Bande Intents](../../intents.md). Main noun `track`; also `music.album`, `music.artist`,
`music.playlist`. Owner: the Music app. Work package A4.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `music.track` | title\*, artist\*, album\* text, length duration, favourite bool | title, artist |
| `music.album` | name~, artist~ text | all |
| `music.artist` | name~ text | all |
| `music.playlist` | name~ text, count integer | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `music.find` | `text`, `artist`, `album`, `genre`, `playlist`, `order` | foreign (tags) |  | new | B |
| `music.find_album`, `music.find_artist`, `music.find_playlist` | `text` | `read` (names) |  | new | B |
| `music.start` | `tracks`, `album`, `artist` or `playlist`, `shuffle` | `change` | the earlier queue | new | B |
| `music.create_playlist`, `music.update_playlist` | `name`, `tracks`; `playlist`, `add`, `remove`, `name` | `change` | removes it; the earlier one | new | B |
| `music.append` | `tracks` | `change` | the earlier queue | new | B |
| `music.set_favourite` | `tracks`, `favourite` | `change` | the earlier value | new | C |

Phrases in Search: "play {album}".

Playing, pausing, position and volume are [`media`](media.md), which any player has.
