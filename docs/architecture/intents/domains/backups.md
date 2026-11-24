# Backups

Part of [Bande Intents](../../intents.md). Main noun `backup`. Owner:
`crates/services/backup-intents`, for the person, in front of the backup service. Work package D15.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `backups.get` | `:only`, `properties` (disk, last, next, running, size) | `read` |  | ready | A |
| `backups.start` | `at` | `disrupt` | `backups.stop` | ready | A |
| `backups.stop` | `:only` | `disrupt` |  | ready | A |
| `backups.pause`, `backups.resume` | `:only` | `disrupt`; `change` | each other | ready | B |
| `backups.find_version` | `files`, `before` | `read`; foreign where what is in it is |  | ready | A |
| `backups.restore` | `files`, `version`, `to`; keeps both | `change`; server: `send` | `files.trash` | app | A |
| `backups.set_disk` | `disk`, `erase` | `authorise` |  | ready | B |
| `backups.remove_disk` | `disk` | `authorise` |  | ready | C |
| `backups.update` | `excluded` | `authorise` | the earlier list | ready | C |

A restore never overwrites. The password of a backup is asked by its owner.
