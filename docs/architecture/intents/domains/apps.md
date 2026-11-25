# Apps and windows

Part of [Bande Intents](../../intents.md). Main nouns `app` and `window`. Owner:
`bande-system-intents`, over a `bande_windows_v1` the compositor serves to it alone. Work package
D8.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `apps.app` | name~ text, running, responding, pinned bool | all |
| `apps.permission` | app ref, name text, given bool | all |
| `windows.window` | app ref, title\* text, workspace ref, display text, states list<`minimised` `maximised` `full-screen` `on-top`>, focused bool | app, title, workspace |
| `windows.workspace` | name~ text, active bool, display text | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `windows.find` | `app`, `workspace` | `read` for the app, foreign for titles |  | app | A |
| `apps.find` | `text`, `running`, `responding` | `read` |  | part | A |
| `windows.find_workspace` | `text` | `read` |  | app | A |
| `apps.find_permission` | `app` | `read` |  | app | B |
| `apps.open` | `app` | `show`; for an agent `disrupt` |  | ready | A |
| `windows.open` | `window` | `show` |  | app | A |
| `apps.stop` | `app`, `force` | `disrupt`; with `force` `critical` |  | app | A |
| `windows.move` | `windows`, `workspace`, `tile`, `display`, `on_top`, `minimised`, `full_screen` | `change` | the earlier places | app; only a client's own window | A |
| `apps.remove_permission` | `app`, `permission`; giving one back is the person's | `disrupt` |  | app (`apps/settings/src/permissions.rs`) | B |
| `windows.stop` | `window`; its app may ask | `disrupt` |  | app | B |
| `apps.set_pinned` | `app`, `pinned` | `change` | the earlier value | app | B |
| `windows.open_workspace` | `workspace` | `show` |  | app | B |

Phrases in Search: "quit {app}", "tile left", "tile right", "workspace {workspace}".

`apps.open` is `show` for Bande's apps that neither connect nor restore anything when opened,
otherwise `disrupt`. It takes no arguments or files and does not open a terminal.
