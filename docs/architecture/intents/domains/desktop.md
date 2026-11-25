# Desktop

Part of [Bande Intents](../../intents.md). One thing, `desktop.current`. Owner:
`bande-system-intents`. Work package D8.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `desktop.current` (`:only`) | app ref, window ref, title\* text, selection list<ref>, document ref<files.file>, workspace ref, time date_time | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `desktop.get` | `:only`, `properties` | `read`, foreign for the title |  | new: asks the focused app's owner, by its app id, its `find` with `current` | A |

What is in front of the person, for "this" and "here". It needs the permission Apps and Windows, and
of the selection only references the caller may read are given.
