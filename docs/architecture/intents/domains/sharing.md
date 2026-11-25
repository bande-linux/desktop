# Sharing and screen

Part of [Bande Intents](../../intents.md). `sharing` has `sharing.status` and `sharing.folder`;
`screen` the main noun `picture`. Owner: `crates/services/screen-sharing`, the screenshot tool. Work
package D19.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `sharing.status` (`:only`), `sharing.folder` | on bool, viewers\* list<text>; folder ref<files.file> | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `sharing.find_folder` | `text` | `read` |  | app | B |
| `sharing.get_status` | `:only`, `properties` (on, viewers) | `read`, foreign for viewers' names |  | new; the service has no interface | C |
| `sharing.stop` | `:only` | `change` for what the caller started, otherwise `disrupt` for an agent |  | part | A |
| `screen.create_screenshot` | `display`, `window`, `delay`; saved in Pictures, not returned | `disrupt` | `files.trash` | app | A |
| `sharing.start` | `view_only` (always), `until` | `critical` | `sharing.stop` | new: today a viewer may control | B |
| `sharing.start_folder` | `folder` | `critical` | `sharing.stop_folder` | app | B |
| `sharing.stop_folder` | `folder` | `disrupt`; with an administrator `critical` | `sharing.start_folder`, authorised | app | B |
| `screen.create` | a window the person picks in the prompt, returned | `disrupt`, foreign |  | app; the portal does not ask again once allowed | B |
| `screen.start_recording` | `target`, `sound` | `critical` | `screen.stop_recording` | app | B |
| `screen.stop_recording` |  | `change` for what the caller started, otherwise `disrupt` for an agent |  | app | B |
| `sharing.start_login` | remote login | `critical` | `sharing.stop_login` | app | C |
| `sharing.stop_login` |  | `disrupt`; with an administrator `critical` |  | app | C |
| `sharing.open` | `computer` (a saved one) | `disrupt` and `send` |  | app | C |

Sharing is always view only, and accepting a viewer, passwords, another computer's picture,
clipboard and any input have no intent. What an agent recorded or captured it cannot read.
