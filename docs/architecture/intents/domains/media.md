# Media

Part of [Bande Intents](../../intents.md). Main noun `player`. Owner: `bande-system-intents`. Work
package D7.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `media.player` | name~ text, app ref, state `playing` `paused` `stopped`, title\*, artist\* text, position, length duration, level integer | all but position |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `media.find` |  | `read` for players, foreign for what they play |  | app | A |
| `media.start`, `media.pause` | `player` | `change`; each is the other's undo |  | app | A |
| `media.set_track` | `player`, `track` (`next`, `previous`) | `change` |  | app | A |
| `media.set_volume` | `player`, `level` | `change` | the earlier level | ready (`libs/audio`) | A |
| `media.set_position` | `player`, `position` | `change`; the earlier position is the undo |  | app | B |
| `media.set_speed` | `player`, `speed` | `change` | the earlier speed | app | B |
| `media.append` | `files`, sound and video without playlists; Bande's Player only | `change` | removes them | app | B |
| `media.set_stream` | `player`, `kind` (`sound`, `subtitles`), `stream` | `change` | the earlier stream | app (`libs/media`) | C |
| `media.open` | `player` | `show` |  | app | C |

Any MPRIS player, also of other applications. The code is in
`crates/shell/quick-settings/src/media.rs`.

Playing a file is `files.open`; a library is [`music`](music.md).
