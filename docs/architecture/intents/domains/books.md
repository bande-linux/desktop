# Books

Part of [Bande Intents](../../intents.md). Main noun `book`. Owner: the Books app. Work package A5.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `books.find` | `text`, `author`, `unread` | foreign |  | new | C |
| `books.get` | `books`, `properties` (title, author, contents, position) | foreign |  | new | C |
| `books.open` | `book`, `place` | `show` |  | new | C |
| `books.add` | `files` | `change` | `books.remove` | new | C |
| `books.remove` | `books` | `change` | `books.add` | new | C |
