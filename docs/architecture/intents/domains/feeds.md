# Feeds

Part of [Bande Intents](../../intents.md). Main noun `item`; `feeds.feed`. Owner: the Feeds app.
Work package A13.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `feeds.item` | title\*, text\* text, url\* url, published date_time, read, kept bool, feed ref | title, published |
| `feeds.feed` | name~ text, url url, unread integer | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `feeds.find` | `text`, `feed`, `unread`, `after` | foreign |  | new | B |
| `feeds.get` | `items`, `properties` | foreign |  | new | B |
| `feeds.find_feed` | `text` | `read` |  | new | B |
| `feeds.update` | `items`, `read`, `kept` | `change` | the earlier values | new | B |
| `feeds.add_feed` | `url` | `send` | `feeds.remove_feed` | new | B |
| `feeds.remove_feed` | `feed` | `change` | adds it again | new | B |
| `feeds.open` | `item` | `show` |  | new | B |
