# Browser

Part of [Bande Intents](../../intents.md). Main noun `tab`. Owner: a D-Bus object added to the
browser by a patch. Work package D18.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `browser.tab` | title\*, text\* text, url\* url, private bool | title, url |
| `browser.bookmark` | title\* text, url\* url | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `browser.find` | `text` | foreign |  | new | B |
| `browser.get` | `tab`, `properties` | foreign |  | new | B |
| `browser.find_bookmark` | `text` | foreign |  | new | C |
| `browser.open` | `url`, `private` | `show`; for an agent `send`, the whole address is shown |  | ready | A |
| `browser.open_tab` | `tab` | `show` |  | new | B |
| `browser.create_bookmark` | `url`, `title` | `change`; `send` while the browser syncs | removes it | new | C |

History, cookies, saved sign-ins, page scripts and form filling have no intent. Remote-control
protocols are synthetic input and are not used.
