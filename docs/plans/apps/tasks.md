# Tasks

Domain: [tasks](../../architecture/intents/domains/tasks.md). Package A2, after D5. Size M.

**There is.** CalDAV in Calendar, which drops collections without events and has no VTODO model; the
branch's `bande-dav` and `bande-content-lines`; recurrence rules; notifications.

**Work.** A VTODO model (RFC 5545: due, start, priority, completion, repeats, alarms, subtasks by
`RELATED-TO`) in the store shared with Calendar; task lists found and made on the server; a local
list; the app (lists, today, scheduled, search, drag to order); due alerts through the same
background process as Calendar's; due tasks in Calendar when the person wants them.

**Done when** a list edited here and by another CalDAV client agree, offline and after conflicts,
and "add milk to my shopping list" needs no prompt on the person's own list.
