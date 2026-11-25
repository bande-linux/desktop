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
| [Books](apps/books.md) | an EPUB reader |
| [Camera](apps/camera.md) | pictures and video from a camera |
| [Recorder](apps/recorder.md) | voice memos |
| [Messages](apps/messages.md) | chat over an open protocol |
| [Maps](apps/maps.md) | maps that work without a server |
| [PDF editing in Viewer](apps/pdf-editing.md) | pages, forms, notes |
| [Fingerprint and security keys](apps/fingerprint.md) | enrolment, and use at sign-in, lock and polkit |
| [On-screen keyboard and other input](apps/assistive-input.md) | without which the prompt is not for everyone |
| [Feeds](apps/feeds.md) | news and podcasts the person subscribes to |
| [Passwords](apps/passwords.md) | generator, one-time codes, export, in Settings |
| [Synced folders](apps/synced-folders.md) | a folder here and on the person's server, kept the same |
| [Help](apps/help.md) | offline help and a first start |

Not planned: phone calls, whiteboard, journal (a Notes folder does it), weather (no open protocol
without a fixed provider), and an office suite of Bande's own (any application serves `documents`,
`spreadsheets` and `presentations` as a [provider](../architecture/intents/providers.md)).

Every app starts with `ci/new-app.sh`, starts without a window when asked, is the only writer of its
store, and declares its domain from the first commit, so its menus, Search, the command line and
agents use one implementation.
