# Bande Notes

Notes for the Bande desktop: folders in a sidebar, the notes of the chosen folder sorted by when
they last changed, and the editor, in one window. Notes are kept on this computer or in the
Notes folder of an IMAP mail account, in the IMAP notes format other clients use, so the same
notes open on other devices.

```
apt install bande-notes
```

## What it does

- **Where notes live.** "On This Computer" always exists. A mail account is used for notes when
  its Notes switch is on: in Settings… (Ctrl+,) for each of Mail's accounts, in Mail's own
  account setup ("Also use this account for Notes", on by default), or by adding the account
  from Notes with File ▸ Add Account…, which is Mail's setup sheet (account type, the servers
  found for the address, or entered by hand) and adds the account to Mail too. Passwords stay in
  the keyring under `application=org.bande.Mail`. A wrong password asks for it again. Nothing
  contacts a server unless an account is used for notes.
- **The account's folders.** Notes uses the account's `Notes` folder (or `INBOX.Notes` where the
  server keeps every folder under the Inbox) and makes it when it is missing; each of its
  subfolders is a note folder. A server that refuses to make one turns the account off for
  Notes and says why.
- **The format.** Each note is a message with `X-Uniform-Type-Identifier: com.apple.mail-note`,
  a `X-Universally-Unique-Identifier`, `X-Mail-Created-Date`, `Date` (the last change), the
  first line as `Subject`, `From` the account, and a quoted-printable `text/html; charset=utf-8`
  body of `div`, `br`, `b`, `i`, `u`, `strike`, `h1`–`h3`, `ul`, `ol`, `li` and `http`, `https`
  or `mailto` links. Notes from other clients are read with the same HTML reader as Text Editor
  (`bande-rich-text`) and reduced to that subset; a note with pictures or attachments is shown
  read-only with a banner until Edit Anyway, because saving it keeps only its text.
- **Editing.** The editor saves a second after typing stops, when another note or folder is
  chosen, and when the window loses focus or closes. A saved edit appends the new version to the
  folder and expunges the old one (`UID EXPUNGE` with `UIDPLUS`), keeping the note's identifier
  and creation date. If the same note changed elsewhere since it was last synced, both versions
  are kept: Notes' version gets a new identifier. Format ▸ Title, Heading, Subheading and Body
  style the paragraph; Bold (Ctrl+B), Italic (Ctrl+I), Underline (Ctrl+U), Strikethrough
  (Shift+Ctrl+X) and the lists style the text. A new note left empty is not kept.
- **Notes and folders.** File ▸ New Note (Ctrl+N, or the New Note action of the app's icon)
  starts a note in the shown folder; Delete Note (Delete in the list) moves a note to the
  account's Trash, and Undo Delete (Ctrl+Z outside the editor) brings it back, up to 20 steps;
  Move To files the note in any other folder, of any account. Find (Ctrl+F) searches the shown
  folder's notes.
- **Sync.** Each account has a worker thread with its own connection; edits, deletions and moves
  are kept in the offline store (`~/.local/share/bande-notes/`) until the server has them, so
  they survive going offline or quitting. A second connection waits with IMAP IDLE on the Notes
  folder; servers without IDLE are asked every five minutes. A folder whose `UIDVALIDITY`,
  `UIDNEXT`, message count and, with `CONDSTORE`, `HIGHESTMODSEQ` are unchanged is not read
  again; with `CONDSTORE` a note another client marked deleted goes at once. A changed
  `UIDVALIDITY` starts the folder over, keeping unsynced edits. Connections, TLS and timeouts are
  Mail's (`bande-mail-core`).
- **Keyboard and screen readers.** Tab moves between the folders, the search field, the list and
  the editor; Enter in the list opens the note in the editor and Escape goes back to the list.
  The folders and the notes are tables a screen reader names "Folders" and "Notes", with each
  note's title, text and date. The sidebar names each account and whether it is offline or not
  signed in.

## Tests

`tests/notes.rs` syncs a note through Debian's `dovecot` (Mail's test server from
`bande-mail-core`'s `testing` feature): a note written by another client arrives, an edit
replaces it keeping its identifier and creation date, and a conflicting edit keeps both
versions. It skips itself when `/usr/sbin/dovecot` is not installed. `src/note.rs` has the
format's parser and writer test. The whole test list is in `docs/quality/tests.md`.

## Licence

MIT OR Apache-2.0.
