# Backups

Part of [Bande Intents](../../intents.md). Main noun `backup`. Owner: `bande-system-intents`, in
front of the backup service. Work package D15.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `backups.backup` (`:only`) | disk~ text, last, next date_time, running bool, size integer | all |
| `backups.version` | file ref, made date_time, size integer | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `backups.get` | `:only`, `properties` | `read` |  | ready | A |
| `backups.find_version` | `files`, `before` | `read`; foreign where what is in it is |  | ready | A |
| `backups.start` | `at` | `change` | `backups.stop` | ready | A |
| `backups.restore` | `files`, `version`, `to`; keeps both | `change`; shared: `send` | `files.trash` | app | A |
| `backups.stop` | `:only` | `disrupt` |  | ready | B |
| `backups.set_disk` | `disk`, `erase` | `critical` |  | ready | B |
| `backups.pause`, `backups.resume` | `:only` | `disrupt`; `change` | each other | ready | C |
| `backups.remove_disk` | `disk` | `critical` |  | ready | C |
| `backups.update` | `excluded` | `critical` | the earlier list | ready | C |

Phrases in Search: "back up now".

A restore never overwrites. The password of a backup is asked by its owner.
