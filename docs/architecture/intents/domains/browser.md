# Browser

Part of [Bande Intents](../../intents.md). Main noun `tab`. Owner: a D-Bus object added to the
browser by a patch. Work package D18.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `browser.open` | `address`, `private` | `send`, the whole address is shown |  | ready | A |
| `browser.find` | `text` | foreign |  | new | C |
| `browser.get` | `tab`, `properties` (title, address, text) | foreign |  | new | C |
| `browser.create_bookmark` | `address`, `title` | `change` | removes it | new | C |
| `browser.find_bookmark` | `text` | foreign |  | new | C |

History, cookies, saved sign-ins, page scripts and form filling have no intent. Remote-control
protocols are synthetic input and are not used.
