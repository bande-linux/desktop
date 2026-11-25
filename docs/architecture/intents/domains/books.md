# Books

Part of [Bande Intents](../../intents.md). Main noun `book`. Owner: the Books app. Work package A5.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `books.book` | title\*, author\* text, contents\* list<text>, position text, unread bool | title, author |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `books.find` | `text`, `author`, `unread` | foreign |  | new | B |
| `books.get` | `books`, `properties` | foreign |  | new | B |
| `books.open` | `book`, `place` | `show` |  | new | B |
