# Plans

Work that is planned and not built. A plan leaves when it is done; what it built is described in
`architecture/`.

| Plan | What |
| :--- | :--- |
| [Bande Intents](intents.md) | the interface, the prompt, the broker, the MCP server, every domain, and how many agents do it at once |
| [Contacts and shared accounts](apps/contacts.md) | address book and one place for accounts; on a branch |
| [Tasks](apps/tasks.md) | to-do lists over CalDAV |
| [Photos](apps/photos.md) | a library over plain folders |
| [Music](apps/music.md) | a library for Player |
| [Camera](apps/camera.md) | pictures and video from a camera |
| [Recorder](apps/recorder.md) | voice memos |
| [PDF editing in Viewer](apps/pdf-editing.md) | pages, forms, notes |
| [Fingerprint and security keys](apps/fingerprint.md) | enrolment, and use at sign-in, lock and polkit |
| [Mouse keys, dwell and colour filters](apps/assistive-input.md) | pointing without a mouse, and colours told apart |
| [Passwords](apps/passwords.md) | generator, one-time codes, export, in Settings |
| [Synced folders](apps/synced-folders.md) | a folder here and on the person's server, kept the same |

Not planned: an on-screen keyboard, chat, maps, a book reader, news feeds, help pages, phone calls,
whiteboard, journal (a Notes folder does it), weather (no open protocol without a fixed provider),
and an office suite of Bande's own (any application serves `documents`, `spreadsheets` and
`presentations` as a [provider](../architecture/intents/providers.md)).

Every app starts with `ci/new-app.sh`, starts without a window when asked, is the only writer of its
store, and declares its domain from the first commit, so its menus, Search, the command line and
agents use one implementation.
