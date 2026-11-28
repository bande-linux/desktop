# Contacts and shared accounts

Domain: [contacts](../../architecture/intents/domains/contacts.md),
[accounts](../../architecture/intents/domains/accounts.md). Packages A1a, A1b, A1c. Size L.

**There is.** vCard 2.1 to 4.0, CardDAV with offline sync, the Contacts app with
groups, import, export, merge and undo, completion in Mail and Calendar, keyring sharing between
named programs, tests. On branch `accounts-wip`: one accounts store and a Settings pane.

**Work.**
1. A1b: the accounts store written again on `main`'s setup guide; Mail, Notes, Calendar and Contacts
   read it.
2. A1c: Contacts' process owns its store, with a lock; `NICKNAME` and `RELATED`; a kept card for
   Trash.
3. Whether others can read a collection: D5 for `DAV:owner` and sharees in `bande-dav`, D2 for IMAP
   `NAMESPACE` and ACL in `bande-mail-core`.

**Done when** one account set up once serves mail, notes, calendar, contacts and tasks, and "email
mom" finds her.
