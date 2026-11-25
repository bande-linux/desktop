# Documents, spreadsheets and presentations

Part of [Bande Intents](../../intents.md). Main nouns `document`, `workbook`, `presentation`. Owner:
`documents.get` `bande-files-intents`; `create`, `append` and `update` the Text Editor app;
`create_from_pages`, `update_form`, `create_export` and `open` the Viewer app;
[providers](../providers.md) for the rest. Work package D1c, D23, A10, C7.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `documents.document` (a view of a file) | text\*, title\*, author\* text, outline\* rich_text, pages, words integer, fields\* list<text>, picture picture | title |
| `spreadsheets.workbook` (a view of a file) | sheets\* list<text>, values\* rows | all |
| `presentations.presentation` (a view of a file) | outline\*, text\*, notes\* rich_text, slides integer | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `documents.get` | `document` or a `mail.attachment`, `properties` | foreign |  | part: text, in the confined importer (`bande-content`) | A |
| `spreadsheets.get` | `workbook`, `sheet`, `range` | foreign |  | part: text only, in the confined importer | B |
| `spreadsheets.find_sheet` | `workbook` | foreign |  | new | B |
| `presentations.get` | `presentation`, `properties` | foreign |  | part: text only, in the confined importer | B |
| `documents.create` | `name`, `within`, `format` (`txt`, `md`, `rtf`, `odt`, `docx`), `body` | `change`; shared: `send` | `files.trash` | app | A |
| `documents.append` | `document`, `body` | as `create` | the earlier content | new | A |
| `documents.update` | `document`, `find`, `replace` | as `create`, and foreign | the earlier content | new | A |
| `documents.create_export` | `document`, `format` (`pdf`, `docx`, `odt`, `html`, `txt`), `within`, `recognise_text`, `language` | as `create` | `files.trash` | new | A |
| `documents.create_from_pages` | `documents`, `pages`, `rotations`, `name`, `within` | as `create` | `files.trash` | part: the toolkit's PDF writing keeps pages | A |
| `spreadsheets.create` | `name`, `within`, `format`, `values` | `change`; shared: `send` | `files.trash` | new | B |
| `spreadsheets.update_range` | `workbook`, `sheet`, `range`, `values`, `formulas` | as `create` | the earlier content | new | B |
| `spreadsheets.append` | `workbook`, `sheet`, `values` | as `create` | the earlier content | new | B |
| `spreadsheets.create_export` | `workbook`, `sheet`, `format`, `within` | as `create` | `files.trash` | new | B |
| `presentations.create` | `name`, `within`, `format`, `outline` | `change`; shared: `send` | `files.trash` | new | B |
| `presentations.create_export` | `presentation`, `format`, `within` | as `create` | `files.trash` | new | B |
| `documents.open` | `document`, `page`, `text`; as `files.open` | `show` |  | app | B |
| `documents.update_form` | `document`, `names`, `values` | as `create`, and foreign | the earlier content | new | B |
| `presentations.append` | `presentation`, `title`, `body` | as `create` | the earlier content | new | C |

Main nouns `document`, `workbook`, `presentation`; files, served by whichever application the person
chose.

`body` and `outline` are `rich_text`; a heading starts a section or a slide. `values` are `rows`:
without `formulas` a value that begins with `=`, `+`, `-` or `@` is text. Text Editor keeps the
earlier content and replaces the file, so the undo puts it back; a provider that cannot makes a
copy. The person opens one with `documents.open` or `files.reveal`.
