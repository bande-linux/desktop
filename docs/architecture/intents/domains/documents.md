# Documents, spreadsheets and presentations

Part of [Bande Intents](../../intents.md). Main nouns `document`, `workbook`, `presentation`. Owner:
`bande-intents-providers`; reading text in `crates/services/files-intents`. Work package C7, D1 for
text, A10 for PDF.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `documents.get` | `document` or a `mail.attachment`, `properties` (text, outline, pages, words, title, author) | foreign |  | part: text, in the confined importer (`bande-content`) | A |
| `spreadsheets.get` | `workbook`, `sheet`, `range` | foreign |  | part: text only, in the confined importer | B |
| `spreadsheets.find_sheet` | `workbook` | foreign |  | new | B |
| `presentations.get` | `presentation`, `properties` (outline, text, notes) | foreign |  | part: text only, in the confined importer | B |
| `documents.create` | `name`, `within`, `format`, `body` | `change`; server: `send` | `files.trash` | app | B |
| `documents.append` | `document`, `body` | as `create` | the earlier content | new | B |
| `documents.update` | `document`, `find`, `replace` | as `create`, and foreign | the earlier content | new | B |
| `documents.create_export` | `document`, `format`, `within`, `recognise_text`, `language` | as `create` | `files.trash` | new | B |
| `spreadsheets.create` | `name`, `within`, `format`, `values` | `change`; server: `send` | `files.trash` | new | B |
| `spreadsheets.update_range` | `workbook`, `sheet`, `range`, `values`, `formulas` | as `create` | the earlier content | new | B |
| `spreadsheets.append` | `workbook`, `sheet`, `values` | as `create` | the earlier content | new | B |
| `spreadsheets.create_export` | `workbook`, `sheet`, `format`, `within` | as `create` | `files.trash` | new | B |
| `presentations.create` | `name`, `within`, `format`, `outline` | `change`; server: `send` | `files.trash` | new | B |
| `presentations.append` | `presentation`, `title`, `body` | as `create` | the earlier content | new | B |
| `presentations.create_export` | `presentation`, `format`, `within` | as `create` | `files.trash` | new | B |
| `documents.create_from_pages` | `documents`, `pages`, `rotations`, `name`, `within` | as `create` | `files.trash` | part: the toolkit's PDF writing keeps pages | A |
| `documents.open` | `document`, `page`, `text`; as `files.open` | `show` |  | app | B |
| `documents.update_form` | `document`, `names`, `values` | as `create`, and foreign | the earlier content | new | B |
| `documents.append_note` | `document`, `page`, `text` | as `create` | the earlier content | new | B |

Main nouns `document`, `workbook`, `presentation`; files, served by whichever application the person
chose.

`body` and `outline` are `rich_text`; a heading starts a section or a slide. `values` are `rows`:
without `formulas` a value that begins with `=`, `+`, `-` or `@` is text. A native provider may edit
in place, and then its own undo is the undo; an adapter makes a copy. The person opens one with
`files.reveal` unless a Bande viewer shows it.
