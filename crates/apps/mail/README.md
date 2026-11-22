# Bande Mail

Mail for the Bande desktop: mailboxes in a sidebar with a unified
Inbox, the message list, and the reader, in one window. It reads mail over IMAP, sends it over
SMTP, keeps an offline copy of every account and follows new mail with IMAP IDLE.

```
apt install bande-mail
```

## What it does

- **Accounts.** The setup sheet first asks for the account type (IMAP and SMTP), then for a
  name, an address and a password. Mail looks up the servers of the address's domain and shows
  them, with user name, hosts, ports and security (TLS, STARTTLS), in the server settings step
  to confirm or change; Enter Server Settings Manually goes there directly. Sign In checks that
  both IMAP and SMTP accept the password before the account is added. A failure says what went
  wrong (server unreachable, certificate not valid, no encryption offered, user name or password
  refused, server error) and keeps every field editable. When a saved password stops working,
  the password sheet also leads to Server Settings…. Several accounts work side by side.
- **Settings** (Ctrl+,) has five tabs. General: notifications for new mail and their sound.
  Accounts: whether each account is used, its full name, signature, OpenPGP key, S/MIME
  certificate and whether new messages are signed, and server settings (user
  name, hosts, ports, TLS or STARTTLS), Add Account… and Remove…; changed servers are checked
  with the saved password before they are kept. Viewing: list preview lines, conversations,
  plain text over HTML, a fixed-width font for plain text, whether remote content loads by
  itself (off by default), and Forget for the senders Mail loads it for because you chose Load
  Remote Content in one of their messages, whether decrypted messages can be found in search
  (off by default), and Forget for the S/MIME issuers you chose to trust. Composing: the account new messages are sent from and whether replies
  quote the original. Rules (each can be edited or removed): if any or all of several conditions hold (From, To or Cc, or
  Subject contains a text), move new unread Inbox mail to a mailbox, flag it or mark it read,
  while Mail is open; Message ▸ Apply Rules runs them on the chosen messages. They are kept in
  `~/.config/bande/mail.conf`; signatures, servers and rules in the offline store.
- **Finding servers** asks only the address's own domain, in this order: DNS SRV records (RFC
  6186: `_imaps`, `_imap`, `_submissions`, `_submission`), the domain's published autoconfig
  (`autoconfig.<domain>`, then `<domain>/.well-known/autoconfig`, over HTTPS, password sign-in
  only), and finally `imap.`/`mail.`/`smtp.` guesses that answer on the usual ports. The DNS
  lookups use `bande-discovery`, shared with Calendar.
- **Sign-in** uses SASL PLAIN, else LOGIN, as the server's capabilities offer, and only over TLS
  or STARTTLS. After sending, Mail keeps a copy in Sent unless the server already filed one
  with the same `Message-ID`.
- **Security.** IMAP and SMTP use TLS or STARTTLS, verified against the system's certificate
  store. Plain text is refused for anything but this computer (`localhost`, `127.0.0.1`, `::1`),
  so a local bridge or test server still works. Passwords live in the Secret Service,
  under `application=org.bande.Mail`, never in a file.
- **Window.** A window that was open at log out comes back in its place, showing the mailbox it showed. The sidebar lists Inbox, Flagged, Drafts, Sent, Archive, Junk and Trash across all
  accounts, then each account's folders with unread counts. The list shows sender, subject, a
  preview (two lines unless Settings says otherwise), the date, an unread dot, a flag and a
  paperclip. Messages of one conversation (the first `References` entry, else the
  `Message-ID`; a reply that has neither joins the latest message of the account with the same
  subject in the 30 days before it) share one row with a count; clicking the count, or Right and Left, shows and
  hides the earlier ones, and acting on a closed conversation acts on all of it (Settings ▸
  Viewing turns this off). The reader shows the whole conversation, oldest first, with your
  replies from Sent and the account's other mailboxes (not Trash or Junk): the chosen message and
  unread ones open, the others as a line that opens on a click. Each open message shows its
  headers, the plain text or sanitised HTML, quoted text folded behind Show Quoted Text, and the
  attachments; Reply, Reply All and Forward (in the message or the toolbar) act on the message
  last clicked, marked with a bar in the accent colour while the reader has focus. Delete, Flag,
  Mark as Read or Unread, Move, Junk, Archive and Apply Rules act on that message too when the
  reader has focus (after a click in it), and on the selected rows when the list has focus; a
  message moved out of the reader leaves the rest of the conversation open. Clicking an attachment opens a private copy in `~/.cache/bande-mail/attachments/`,
  removed when Mail quits unless it was edited; its menu also saves it where you choose (the
  desktop's file chooser) or to the Downloads folder.
  The reader's text can be selected with the pointer (a double-click selects a word) or, with
  the reader focused, Select All (Ctrl+A) for the chosen message, and copied with Copy (Ctrl+C)
  or its context menu; a name or address offers New Message and Copy Address, a link Open Link
  and Copy Link. Pointing at a link shows where it goes, and a link whose text names another
  host than the one it leads to asks before it opens.
- **Signed and encrypted mail**, with OpenPGP (PGP/MIME, RFC 3156, and inline PGP, read only)
  and S/MIME (RFC 8551). The keys are GnuPG's: secret keys stay in `gpg-agent`, which asks for
  their passphrase (pinentry) and uses smartcards; OpenPGP keys come from the GnuPG keyring,
  S/MIME certificates and their keys from `gpgsm`'s. Messages themselves are parsed and checked
  by Mail (Sequoia for OpenPGP, RustCrypto's `cms` and `rustls-webpki` for S/MIME), never by a
  GnuPG program, and the agent only unwraps a session key or signs a hash. A banner above an
  opened message says what was proven: signed by a verified key or certificate (an OpenPGP key
  GnuPG holds as fully valid, or an S/MIME certificate that chains to the system's or a trusted
  issuer and names the sender), signed by a key you have not verified or by a certificate from an
  issuer you don't trust, signed by someone who is not the sender, signed with a key you don't
  have, an expired, revoked or unusable key, an algorithm Mail does not accept (SHA-1), a
  signature that doesn't match, and whether the message was encrypted or why it can't be opened.
  Only a signature over the whole message counts: a signed part inside an unsigned message shows
  no signature banner. Headers protected inside the signed or encrypted part (RFC 9788 `hp`, and
  `protected-headers="v1"`) replace the outer ones, so a hidden subject shows. Look Up Key fetches
  a missing OpenPGP key from the sender's domain by WKD (Web Key Directory), only when clicked,
  and adds it to GnuPG; Trust Issuer… trusts an S/MIME issuer after showing its fingerprint.
  Keys are never fetched by themselves. A verified S/MIME sender's certificate is kept for
  encrypting to them. Checking and decrypting run off the window's thread, so a passphrase prompt
  never freezes Mail. The text of an encrypted message is not kept in the search index unless
  Settings ▸ Viewing allows it; turning that off removes it again. Autocrypt is not used: its
  keys arrive unverified in every message's headers, and a key found that way would be one more
  "unverified" signature, not proof; WKD on request gives the same keys from the domain itself.
- **HTML** is never rendered as a web page: it is turned into paragraphs, headings, lists,
  quotes, links and pictures. Scripts, styles, forms and frames are dropped, and only `http`,
  `https` and `mailto` links survive. Pictures sent inside the message (`cid:`) show at once;
  remote pictures are not fetched until **Load Remote Content** is clicked in the reader's
  banner, which Mail then remembers for that sender (or by themselves when Settings ▸ Viewing
  allows it), and then only over HTTPS, at
  most 20 per message, 10 MB each, decoded with limits on their size.
- **Toolbar and keys.** Get Mail (F5), New Message (Ctrl+N), Archive (Shift+Ctrl+A),
  Delete (Delete or Backspace), Junk, Reply (Ctrl+R), Reply All (Shift+Ctrl+R),
  Forward (Shift+Ctrl+F), Flag (Message ▸ Flag picks one of seven colours, kept
  on the server), Mark as Read or Unread, Send
  (Ctrl+Enter), Close Window (Ctrl+W), Quit (Ctrl+Q) and Search (Ctrl+F), which finds messages
  with every word in their sender, recipients, subject or text; `from:`, `to:` and `subject:`
  keep a word to that field, and quotes keep words together. Words of three or more characters are
  looked up in an SQLite FTS5 trigram index, so a search stays fast in a large mailbox; the list
  shows the first 500 messages and loads more as you scroll. View sorts the list by date, sender, subject, size or unread first,
  either way. Mailbox ▸ Mark All as Read marks the shown mailbox read, and Message ▸ Move To files the
  message in any mailbox of its account. Ctrl+click, Shift+click, Shift+Up/Down and Select All
  (Ctrl+A, in the list) select several messages; Delete, Archive, Junk, Move To, Flag and Mark as Read then
  act on all of them, and dragging them onto a mailbox in the sidebar moves them.
  Tab and F6 (Shift for backwards) move between the sidebar, the list, the reader and the
  toolbar; Up, Down, Home, End, Page Up and Page Down choose a mailbox in the sidebar, move the
  selection in the list and scroll the reader, and the Menu key (or Shift+F10) opens the
  mailbox's or the message's menu. In the reader, Tab also steps through its links, addresses,
  attachments and buttons such as Load Remote Content; Enter or Space presses the one with the
  focus ring, the Menu key opens its menu and Escape leaves it. Screen readers see every mailbox,
  message row, message text, link (named by its text), attachment and button, and can focus and
  press them. Dates and times follow the locale. Undo (Ctrl+Z)
  puts moved or deleted messages back, up to 20 steps, and a mailbox that messages were moved
  to is synced right after the move. Mailbox ▸ New Mailbox… and a mailbox's menu in the sidebar
  create, rename and delete mailboxes on the server; the special mailboxes keep their names.
- **Compose** opens its own window, with From when there are several accounts, and Bcc when
  View ▸ Bcc Address Field is on or the message has Bcc recipients. To, Cc and Bcc complete an address as you type it, from the senders of
  your mail and the recipients of mail you sent, as found in the offline store; Tab, a comma or
  End takes the suggestion. Replies quote the
  message and thread it (`In-Reply-To`, `References`); forwards keep the attachments. The account's
  signature goes above the quoted text and replaces the previous account's signature when From
  changes. Files are
  attached through the desktop's file chooser (the portal), up to 25 MB each. Each attachment's × button
  removes it and Undo (Ctrl+Z, outside a text field) brings it back; attachments that don't fit
  on the line are listed behind “N more”. Sign and Encrypt in the compose window's header sign
  the message with the account's key and encrypt it to every recipient and to yourself. Sign
  starts on when the account's Sign new messages is on, Encrypt when replying to or forwarding an
  encrypted message. OpenPGP is used when every recipient has a key in GnuPG, else S/MIME when
  every recipient has a trusted certificate; if neither, or with Bcc recipients, the message is
  not sent and the window says which addresses lack a key, so a message meant to be encrypted is
  never sent in plain text. The protected headers (From, To, Cc, Subject, Date, Message-ID) are
  copied into the signed or encrypted part; the outer ones stay readable. A sent message is kept in Sent and the original is marked answered. If
  sending fails, the message comes back in a window with the error. A message sent or saved from
  an account that is turned off, or whose servers change before it goes out, stays queued: Mail
  says so and sends it when the account is on again.
- **Drafts.** Closing a message that was changed and not sent keeps it in the account's Drafts
  mailbox (also on quit). While a message is written, a copy is kept every few seconds in
  `~/.local/share/bande-mail/unsent/` until the server has it, so a message survives a crash or
  quitting offline: it opens again the next time Mail starts, still replacing the draft it came
  from and answering the message it replies to. Both copies keep the Bcc recipients. A message
  marked Encrypt is never put in the server's Drafts in plain text: it stays only in that local
  copy and opens again the next time Mail starts. Double-click a draft, or press Return, to carry on; saving it again or
  sending it removes the older copy. Message ▸ Discard Draft closes the message without keeping it, after asking, and moves the draft it came from to the Trash.
- **Offline store.** One SQLite database in `~/.local/share/bande-mail/` holds the accounts,
  folders, headers, flags and message bodies (bodies up to 2 MB are fetched ahead, larger ones
  when opened). Flag changes, moves and deletions apply at once and are queued until the
  server has them, so they survive going offline or quitting.
- **New mail.** A second connection IDLEs on the Inbox; servers without IDLE are polled every
  five minutes. New unread mail shows a notification through `org.freedesktop.Notifications`; clicking it opens the message. The Taskbar icon shows how many messages in the Inbox are unread (`com.canonical.Unity.LauncherEntry`).
- **Integration.** `org.bande.Mail.desktop` registers Mail for `mailto:` links (choose it in
  Settings ▸ Apps). A second `bande-mail mailto:…` hands the link to the running Mail.

## Robustness

Every connection has a 15 s connect and 60 s read timeout, IMAP responses are parsed by a
small parser that rejects malformed input instead of panicking (deep nesting, short literals,
over-long lines and 128 MB responses are refused), and a lost connection is retried with
backoff from 5 s to 5 min. An account that can't connect is marked Offline in the sidebar; clicking that, or a Get Mail that fails again, says why, with Try Again and Server Settings…. A changed `UIDVALIDITY` starts the mailbox over, a server with `CONDSTORE` (RFC 7162) is asked only for the flags
that changed since the last sync, a server without
`MOVE` gets `COPY` + `EXPUNGE`, and one without `SPECIAL-USE` has its folders recognised by
their usual English names.

## Tests

`tests/mail.rs` starts Debian's `dovecot` as the current user on a free loopback port, with a
small SMTP listener beside it, and checks that a message sent after the IMAP session went stale
goes out once, without injected headers or its Bcc, and is kept in Sent. It skips itself when
`/usr/sbin/dovecot` is not installed. It also checks that a `mailto:` link cannot add Bcc
recipients or attachments. `tests/crypto.rs` opens real signed and encrypted messages made by
GnuPG, gpgsm and OpenSSL (`tests/crypto/make-fixtures.py` makes them) with a real `gpg-agent` in a
temporary GnuPG home, and sends signed and encrypted messages to itself in both standards; it
skips itself when `gpgsm` is not installed. The whole test list is in `docs/quality/tests.md`.

## Dependencies

Each crate is packaged in Debian, some at an older version than the one used here, except where
noted:

| Crate | Why |
|---|---|
| `rustls`, `rustls-platform-verifier` | TLS 1.2 and 1.3 for IMAP, SMTP and HTTPS, checked against the system's CA store, on the ring crypto provider, the one Debian builds rustls with; the same TLS stack as the rest of Bande |
| `mailparse` | MIME parsing, charsets and encoded words of incoming mail |
| `rusqlite` | the offline store, on the system `libsqlite3` |
| `html5ever` | a correct HTML tokenizer for turning mail HTML into safe text |
| `image` | PNG, JPEG and GIF pictures in mail, with the same version and features as the toolkit |
| `data-encoding` | base64 for MIME, `AUTH PLAIN` and modified UTF-7 (already pulled in by `mailparse`) |
| `zeroize` | keeps the typed password out of freed memory (already used by the toolkit) |
| `sequoia-openpgp` | OpenPGP parsing, verification, decryption, signing and encryption, on Debian's crypto backend, Nettle |
| `sequoia-gpg-agent` | secret-key operations in `gpg-agent`, so GnuPG keys, smartcards and pinentry work; its build needs `capnproto` |
| `cms` (not in Debian), `x509-cert` | S/MIME's CMS and X.509 structures |
| `rustls-webpki`, `rustls-pki-types`, `ring` | S/MIME certificate chains and signatures, with the TLS stack's own verifier |
| `nettle` | AES-CBC and RSA encryption for S/MIME (already linked through Sequoia) |
| `jiff`, `roxmltree`, `zbus`, `accesskit`, `tracing` | shared with the desktop |

No IMAP, SMTP, DNS or HTTP crate is used: the maintained IMAP crates are async-only or not in
Debian, and the desktop has no async runtime. The IMAP and SMTP clients, the SRV/MX lookup and
the HTTPS autoconfig fetch are small, synchronous and bounded by timeouts.

Running: `gpg`, `gpg-agent` and `gpgsm` (Recommends) for signed and encrypted mail, `scdaemon`
(Suggests) for smartcards. Without them, S/MIME signatures are still checked, OpenPGP signatures
show as made with a key you don't have, and encrypted mail as mail none of your keys can open.
A key with a passphrase needs a pinentry that works in the session.

## Licence

MIT OR Apache-2.0. The built program also contains Sequoia (LGPL-2.0-or-later) and Nettle's
bindings (LGPL-3.0, GPL-2.0 or GPL-3.0); `debian/copyright` lists each.
