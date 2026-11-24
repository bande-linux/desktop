# Software

Part of [Bande Intents](../../intents.md). Main noun `package`. Owner:
`crates/services/software-intents`. Work package D10.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `software.find` | `text`, `installed` | `read` for names, foreign for descriptions |  | app | B |
| `software.find_update` |  | `read`, from what is already known |  | ready | B |
| `software.get` | `packages`, `properties` (name, summary, version, size, installed, permissions, what is new, needs restart) | foreign |  | part: versions, sizes and what is installed are in `bande-packages`, catalogue text in the app | A |
| `software.find_unused` | `text` | `read` |  | part | B |
| `software.open` | `package` | `show`; pictures load after the person's click |  | new; Software takes no arguments | B |
| `software.install` | `packages` | `authorise` | `software.remove` | ready (`libs/packages`), the flow is in the app | A |
| `software.remove` | `packages`, `keep_data` | `authorise` | `software.install` | ready, as above | A |
| `software.start_check` |  | `disrupt` (contacts the sources) |  | app | A |
| `software.install_update` | `packages` or all, `at`, `then` (`nothing`, `restart`, `shut-down`) | `authorise` |  | part | A |
| `software.remove_unused` | `packages` | `authorise` |  | part | A |

A package is known by its AppStream id, package name or Flatpak reference. Installing is from
sources already set up, by name, with the size, the source and the permissions in the prompt: a
Flatpak installs without a password when asked directly, and a package's scripts run as root, so the
guard is Bande's. Adding a source and installing a package file have no intent. A removal names
everything that goes with it.
