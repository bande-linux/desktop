# Bande Contacts

The address book of the Bande desktop: contacts on this computer and on CardDAV servers, kept
offline and in sync. Packaged as `bande-contacts`.

## What it does

- **Accounts.** File ▸ Add Account… asks for an address and a password and looks up the CardDAV
  server of the address's own domain (RFC 6764: `_carddavs._tcp` and `_carddav._tcp` SRV records
  with their TXT `path`, then `/.well-known/carddav`) without sending the password, and shows it to
  confirm or change; Enter Server Settings Manually goes there directly. Sign In follows
  `current-user-principal` → `addressbook-home-set` → the address books there (RFC 6352). Sign-in
  is Basic or Digest only in answer to the server's challenge and only over HTTPS (or to this
  computer), through `bande-dav`, the client Calendar uses. A failure says what went wrong and keeps
  every field. Passwords go to the Secret Service. "Also use this account for Calendar" offers the
  same server and sign-in to Calendar (`bande_dav::offer`), which finds its calendars there the next
  time it syncs; Calendar's account sheet offers the same for Contacts. The app that takes an offer
  asks the keyring once to read a password another app saved. An account's row in the sidebar
  opens its server settings; its menu also removes it. When the server refuses the saved password,
  Contacts asks for it again.
- **On this computer.** A local address book that never leaves the machine.
- **Offline sync.** Every address book is kept on disk with its pending changes in
  `~/.local/share/bande-contacts` and pushed when the network is back, every 15 minutes and on
  View ▸ Refresh. Sync uses `sync-collection` (RFC 6578), else ctag and etags; contacts are
  fetched with `addressbook-multiget`. Creates use `If-None-Match: *`, edits and deletes
  `If-Match`: when an edit meets a contact changed elsewhere, the server's version stays and the
  edit is kept as a second contact to merge. The title bar says Offline, or Not Synced with the
  reason, Try Again and Server Settings….
- **vCard.** 3.0 and 4.0 (and 2.1 with quoted-printable) through `bande-address-book`: folding,
  escaping, groups, and every property it does not edit kept as it was; an edit rewrites only the
  fields that changed. New contacts are vCard 3.0, which every CardDAV server takes. A card over
  8 MB, with more than 4096 properties or a photo over 4 MB is left out; a file may hold up to
  100 000 cards and 64 MB. Photo links are never fetched.
- **The window.** The address books and their groups in the sidebar, the list with a search field
  (names, organizations, email addresses, numbers and notes) and a letter index, and the contact. Edit
  changes names, organization and title, email addresses, phone numbers and addresses with their
  labels, birthday (with or without a year), groups, the address book of a new contact, notes and
  the photo (made 512 pixels at most). Groups are group cards (`KIND:group`, or
  `X-ADDRESSBOOKSERVER-KIND` on vCard 3.0 servers) where the address book has them, else
  `CATEGORIES`. Clicking an email address writes to it in Mail.
- **Duplicates.** Contacts in one address book with the same name or an email address in common
  are offered for merging (Merge Duplicates… in the title bar and the Edit menu): every address,
  number and note comes together in one.
- **Undo.** Ctrl+Z and Shift+Ctrl+Z take back and redo adding, changing, deleting, importing and
  merging; a deleted contact comes back at its own address.
- **Files.** File ▸ Import…, a dropped `.vcf` file or `bande-contacts file.vcf` offers to import
  its contacts into an address book (contacts already there are updated); File ▸ Export… saves
  the contacts the list shows, a contact's menu saves that one. Both use the file chooser portal.
  The desktop file registers `text/vcard` and `text/x-vcard`.
- **Other apps.** Mail completes addresses from contacts before the senders it knows, shows
  contacts' names for their addresses and adds an address with Add to Contacts
  (`bande-contacts --add "Name <address>"`). Calendar completes invitees from contacts and can
  show their birthdays. Both read the offline copy through `bande_address_book::Directory`, never
  the network. Search lists contacts (`contact:<uid>` opens one).

Shortcuts: Ctrl+N new contact, Ctrl+E edit, Delete deletes the selected contact, Ctrl+F search,
Ctrl+O import, Shift+Ctrl+E export, Ctrl+R refresh, Ctrl+Z undo, Shift+Ctrl+Z redo. Typing a
name in the list goes to it; Return edits it. Every control is in the accessibility tree, the
letter index as one button per letter.

## Layout

| Path | What it is |
|---|---|
| `crates/libs/address-book` | vCard, the offline address books, CardDAV sync and the `Directory` other apps read |
| `src/view.rs` | the window: sidebar, list, contact, undo, import, merging, account sheets |
| `src/editor.rs` | the contact editor |
| `src/setup.rs` | the CardDAV account sheet |
| `src/worker.rs` | the sync thread; passwords through `bande-keyring` |
| `src/widgets.rs` | the panes, the photo and the letter index |
| `src/offer.rs` | contacts offered to search |
| `src/main.rs` | the app: window, menus, sync timer, files, the file chooser and arguments |

## Tests

`tests/contacts.rs` adds a contact, undoes and redoes it, imports a file and merges the
duplicate through the accessibility tree. `crates/libs/address-book/tests` reads tricky vCards
and syncs with Radicale. The whole list is in `docs/quality/tests.md`.
