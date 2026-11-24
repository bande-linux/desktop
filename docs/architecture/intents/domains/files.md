# Files

Part of [Bande Intents](../../intents.md). Main noun `file`; `files.location` is a server, volume or
standard folder. `archives` has the main noun `archive`. Owner: `crates/services/files-intents`.
Work package D1.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `files.find_location` | `text` | `read` |  | ready; saved servers, mounted volumes and standard folders, no discovery | A |
| `archives.get` | `archive`, `properties` (items, size, `names`) | foreign |  | part: totals; names are new | A |
| `files.find` | `name`, `text`, `kind`, `within`, `recursive`, `modified_after`, `modified_before`, `used_after`, `larger_than`, `duplicates`, `trash`, `order` (`name`, `modified`, `used`, `size`) | `read`; foreign with `text` or where what is in it is |  | part: names and text within a folder, and only with words to find; Trash is listed in `libs/fs/src/trash.rs`; duplicates are new | A |
| `files.get` | `files`, `properties` (name, size, dates, kind, access, origin, digest) | as `find` |  | part: digest is new | A |
| `files.create_folder` | `within`, `name` | `change`; server: `send` | `files.trash` | part: `new_folder` chooses the name | A |
| `files.move`, `files.copy` | `files`, `to`, `new_folder`, `if_exists` (`keep-both`, `skip`) | `change`; server: `send` | move back, trash the copies | ready | A |
| `files.update` | `files`, `names` in the same order | `change`; server: `send` | the old names | ready | A |
| `files.trash`, `files.restore` | `files` | `change`; server: `send` | each other | ready | A |
| `files.reveal` | `file` | `show` |  | ready | A |
| `files.connect_location` | `location` (a saved server) | `send` | `files.disconnect_location` | ready, only with a remembered sign-in | A |
| `files.disconnect_location` | `location` (a server) | `disrupt` |  | ready | A |
| `archives.create` | `files`, `name`, `format` (`zip`, `tar-gz`, `tar-bz2`, `tar-xz`), `within` | `change`; server: `send` | `files.trash` | ready | A |
| `archives.extract` | `archive`, `within` | `change`; server: `send` | `files.trash` | part: one item gets no folder of its own, and hidden names, desktop entries and executable bits are kept | A |
| `files.open` | `files`, `app` | `show`; another app: `disrupt`, refused for an agent |  | ready | A |
| `files.create_link` | `file`, `within` | `change`; server: `send` | `files.trash` | ready (`libs/fs/src/ops.rs`) | B |
| `files.compute_usage` | `within`, `depth` | `read`; foreign for names outside own folders |  | app (`apps/settings/src/storage.rs`) | A |
| `files.compute_changes` | `within`, `since` | `read`; foreign where what is in it is |  | new, on the backup client | B |
| `files.set_access` | `files`, `class`, `access` | `disrupt`; wider for others: `authorise` | the earlier access | ready (`libs/fs/src/permissions.rs`) | C |
| `files.delete` | `files` | `authorise` |  | ready | B |
| `files.delete_trash` | `older_than` | `authorise` |  | ready | A |
| `files.add_location` | `address` | `send` | `files.remove_location` | part: saved servers | B |
| `files.remove_location` | `location` | `disrupt` |  | ready | B |

`kind` is `folder`, `picture`, `sound`, `video`, `document`, `spreadsheet`, `presentation`,
`archive` or `other`. There is no `replace`. `delete` and `delete_trash` name the count, the size
and the oldest and newest in the prompt, and an agent deletes only what it was granted. A move to a
server that has to be connected first is one plan and one prompt; a password or a new host key is
asked by Files, of the person. Moves between volumes are jobs, and cancelling leaves each file whole
on one side.

`extract` makes one new folder and what is in it is foreign. `bande-fs` already refuses paths and
links out of the folder, devices and bombs. A password has no parameter: the app asks. The Archives
app is never started with a file, because it then expands at once.
