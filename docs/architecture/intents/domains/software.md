# Software

Part of [Bande Intents](../../intents.md). Main noun `package`. Owner: the Software app. Work
package D10.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `software.package` | name~ text, summary\*, notes\* text, version text, size integer, installed, restart bool, source~ text, permissions list<text> | name, installed |
| `software.update` | package ref, version text, size integer, restart bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `software.find` | `text`, `installed` | `read` for names, foreign for descriptions |  | app | A |
| `software.find_update` |  | `read`, from what is already known |  | ready | A |
| `software.get` | `packages`, `properties` | foreign |  | part: versions, sizes and what is installed are in `bande-packages`, catalogue text in the app | A |
| `software.find_unused` | `text` | `read` |  | part | B |
| `software.install` | `packages` | `critical` | `software.remove` | ready (`libs/packages`), the flow is in the app | A |
| `software.remove` | `packages`, `keep_data` | `critical` | `software.install` | ready, as above | A |
| `software.install_update` | `packages` or all, `at`, `then` (`nothing`, `restart`, `shut-down`) | `critical` |  | part | A |
| `software.open` | `package` | `show`; pictures load after the person's click |  | new; Software takes no arguments | B |
| `software.start_check` |  | `disrupt` (contacts the sources) |  | app | B |
| `software.remove_unused` | `packages` | `critical` |  | part | B |

A package is known by its AppStream id, package name or Flatpak reference. Installing is from
sources already set up, by name, with the size, the source and the permissions in the prompt: the
system asks an administrator for a package, and nothing for a Flatpak of one's own, so there Allow
is the only question. Adding a source and installing a package file have no intent. A removal names
everything that goes with it.
