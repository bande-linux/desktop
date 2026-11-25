# Contacts and shared accounts

Domain: [contacts](../../architecture/intents/domains/contacts.md),
[accounts](../../architecture/intents/domains/accounts.md). Packages A1a, A1b, A1c. Size L.

**There is.** On `accounts`: vCard 2.1 to 4.0, CardDAV with offline sync, the Contacts app with
groups, import, export, merge and undo, completion in Mail and Calendar, keyring sharing between
named programs, tests. Not committed there: one accounts store and a Settings pane.

**Work.**
1. Commit the uncommitted work to a branch of its own.
2. A1a: merge `main` into the committed `accounts` (13 files conflict), run the gate, merge to
   `main`.
3. A1b: the accounts store written again on `main`'s setup guide; Mail, Notes, Calendar and Contacts
   read it.
4. A1c: Contacts' process owns its store, with a lock; `NICKNAME` and `RELATED`; a kept card for
   Trash.
5. Whether others can read a collection: `DAV:owner`, sharees, IMAP `NAMESPACE` and ACL.

**Done when** one account set up once serves mail, notes, calendar, contacts and tasks, and "email
mom" finds her.
