# Apps and windows

Part of [Bande Intents](../../intents.md). Main nouns `app` and `window`. Owner:
`crates/services/apps-intents`, the compositor for windows. Work package D8.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `windows.find` | `app`, `workspace` | `read` for the app, foreign for titles |  | app | C |
| `apps.find` | `text`, `running`, `responding` | `read` |  | part | A |
| `apps.find_permission` | `app` | `read` |  | app | B |
| `apps.open` | `app` | `show` or `disrupt`, as above |  | ready | B |
| `windows.open` | `window` | `show` |  | app | C |
| `apps.stop` | `app`, `force` | `disrupt`; with `force` `authorise` |  | app | A |
| `apps.remove_permission` | `app`, `permission`; giving one back is the person's | `disrupt` |  | app (`apps/settings/src/permissions.rs`) | B |
| `windows.move` | `windows`, `workspace`, `tile`, `display`, `on_top`, `minimised`, `full_screen` | `change` | the earlier places | app; only a client's own window | A |
| `windows.stop` | `window`; its app may ask | `disrupt` |  | app | B |

`apps.open` is `show` for Bande's apps that neither connect nor restore anything when opened,
otherwise `disrupt`. It takes no arguments or files and does not open a terminal.
