# Photos

Part of [Bande Intents](../../intents.md). Main noun `photo`; `photos.album`. Owner:
`bande-files-intents`; the Photos app for albums and favourites. Work package A3, D1 for facts.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `photos.photo` (a view of a file) | taken date_time, width, height integer, camera~, lens~ text, exposure\* text, place place (scope "places"), rating integer, favourite bool, albums list<ref>, picture picture | taken, camera, favourite |
| `photos.album` | name~ text, count integer | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `photos.find` | `taken_after`, `taken_before`, `camera`, `lens`, `rating`, `favourite`, `album`, `within`, `near` (`place`), `order` | foreign; `near` needs the scope "places" |  | part: facts are read, not indexed | A |
| `photos.get` | `photos`, `properties` (taken, width, height, camera, lens, exposure, place) | foreign; `place` needs the scope "places" |  | part: facts are display text, not typed values | A |
| `photos.find_album` | `text` | `read` |  | new, with the Photos app | B |
| `photos.create_copy` | `photos`, `format` (a single PDF too), `width`, `crop`, `rotation`, `flip`, `within` | `change`; shared: `send` | `files.trash` | part: PNG copy in Viewer, PDF writing in the toolkit | A |
| `photos.update` | `photos`, `rotation`, `flip` | `change`; shared: `send` | the earlier values | app; a JPEG's orientation | B |
| `photos.create_album` | `name`, `photos` | `change` | removes it | new | B |
| `photos.update_album` | `album`, `name`, `add`, `remove` | `change` | the earlier album | new | B |
| `photos.set_favourite` | `photos`, `favourite` | `change` | the earlier value | new | B |

A photo is its file, so `files.move` and `files.trash` work on it. `create_copy` leaves out place
and camera. The scope "places" is a permission of its own. Facts are read in the confined importer
and indexed as typed values: today the date has no zone and dimensions need a decode. There is no
photo library app, album or favourite: see the [plan](../../../plans/apps/photos.md). Finding by
what a picture shows needs a model the person installs, and is not decided.
