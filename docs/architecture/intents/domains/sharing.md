# Sharing and screen

Part of [Bande Intents](../../intents.md). `sharing` has `sharing.status` and `sharing.folder`;
`screen` the main noun `picture`. Owner: `crates/services/screen-sharing`, the screenshot tool. Work
package D19.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `sharing.get_status` | `:only`, `properties` (on, viewers) | `read`, foreign for viewers' names |  | new; the service has no interface | C |
| `sharing.start` | `view_only` (always), `until` | `authorise` | `sharing.stop` | new: today a viewer may control | B |
| `sharing.stop` | `:only` | `change` |  | part | A |
| `sharing.start_folder` | `folder` | `authorise` | `sharing.stop_folder` | app | B |
| `sharing.stop_folder` | `folder` | `disrupt` | `sharing.start_folder`, authorised | app | B |
| `sharing.start_login` | remote login | `authorise` | `sharing.stop_login` | app | C |
| `sharing.stop_login` |  | `disrupt` |  | app | C |
| `sharing.open` | `computer` (a saved one) | `disrupt` and `send` |  | app | C |
| `screen.create` | a window the person picks in the prompt, returned | `disrupt`, foreign |  | app; the portal does not ask again once allowed | B |
| `screen.create_screenshot` | `display`, `window`, `delay`; saved in Pictures, not returned | `disrupt` | `files.trash` | app | A |
| `screen.start_recording` | `target`, `sound` | `authorise` | `screen.stop_recording` | app | B |
| `screen.stop_recording` |  | `change` |  | app | B |

Stopping only protects and needs no approval. Sharing is always view only, and accepting a viewer,
passwords, another computer's picture, clipboard and any input have no intent. What an agent
recorded or captured it cannot read.
