# Photos

Part of [Bande Intents](../../intents.md). Main noun `photo`; `photos.album`. Owner:
`crates/services/files-intents` for facts, the Photos app for albums. Work package A3, D1 for facts.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `photos.find` | `taken_after`, `taken_before`, `camera`, `within`, `near`, `order` | foreign; `near` needs the scope "places" |  | part: facts are read, not indexed | A |
| `photos.get` | `photos`, `properties` (taken, width, height, camera, lens, exposure, place) | foreign; `place` needs the scope "places" |  | part: facts are display text, not typed values | A |
| `photos.find_album` | `text` | `read` |  | new, with the Photos app | B |
| `photos.create_copy` | `photos`, `format` (a single PDF too), `width`, `crop`, `rotation`, `flip`, `within` | `change`; server: `send` | `files.trash` | part: PNG copy in Viewer, PDF writing in the toolkit | A |
| `photos.update` | `photos`, `rotation`, `flip` | `change`; server: `send` | the earlier values | app; a JPEG's orientation | B |
| `photos.create_album` | `name`, `photos` | `change` | removes it | new | B |
| `photos.update_album` | `album`, `name`, `add`, `remove` | `change` | the earlier album | new | B |
| `photos.set_favourite` | `photos`, `favourite` | `change` | the earlier value | new | B |

A photo is its file, so `files.move` and `files.trash` work on it. `create_copy` leaves out place
and camera. The scope "places" is part of a grant, off until the person adds it. Facts are read in
the confined importer and indexed as typed values: today the date has no zone and dimensions need a
decode. There is no photo library app, album or favourite: see the
[plan](../../../plans/apps/photos.md). Finding by what a picture shows needs a model the person
installs, and is not decided.
