# Terminal

Part of [Bande Intents](../../intents.md). Main noun `folder`. Owner: the Terminal app. Work package
D9.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `terminal.open_folder` | `folder` | `show`; for an agent `disrupt` |  | app; `--working-directory` | B |

Opening a terminal runs the person's shell startup files, and a folder may hold more of them, so it
is asked every time. Commands, input, paste, titles, the working folder and the scrollback have no
intent. `org.freedesktop.Application.Open` on the terminal forwards `-e`, so it is never how an
intent opens it.
