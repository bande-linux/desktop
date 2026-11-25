# Mail

Part of [Bande Intents](../../intents.md). Main noun `message`; `mail.thread`, `mail.draft`,
`mail.attachment`, `mail.mailbox`, `mail.rule`. Owner: the Mail app. Work package D4.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `mail.message` | from\* address, to\*, cc\* list<address>, date date_time, subject\* text, body\* rich_text, attachments list<ref<mail.attachment>>, thread ref, mailbox ref, read, flagged, answered, junk bool, colour `red` `orange` `yellow` `none` | from, date, subject, read, mailbox |
| `mail.thread` | subject\* text, messages list<ref>, last date_time, people\* list<address> | subject, last |
| `mail.draft` | to, cc, bcc list<address>, subject text, body rich_text, attachments list<ref>, account ref, sends date_time, by_agent bool | to, subject, sends |
| `mail.attachment` | name\* text, size integer, media_type text | all |
| `mail.mailbox` | name~ text, account ref, role `inbox` `drafts` `sent` `archive` `junk` `trash` `none`, unread integer, shared bool | all |
| `mail.rule` | from, to, subject text, move_to ref, read, flagged bool | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `mail.find` | `text`, `from`, `to` (`text`), `mailbox`, `after`, `before`, `unread`, `flagged`, `answered`, `with_attachments`, `larger_than` | foreign |  | app | A |
| `mail.find_thread` | `text`, `message` | foreign |  | app | A |
| `mail.get` | `messages`, `properties` | foreign; does not mark read |  | app | A |
| `mail.find_mailbox` | `text`, `account` | `read` |  | app | A |
| `mail.find_draft` | `text`, `to`, `scheduled` | `read`; a draft the person made: foreign |  | app | A |
| `mail.get_draft` | `drafts`, `properties` | as `find_draft` |  | app | A |
| `mail.find_rule` | `text` | `read` |  | app (`store.rs`) | B |
| `mail.get_away`, `mail.stop_away` | `account` | `read`; `send` |  | new | B |
| `mail.get_signature` | `account` | `read` |  | app | B |
| `mail.create_draft` | `account`, `to`, `cc`, `bcc`, `subject`, `body`, `attachments` (`list<ref<files.file>, 50>`), `sign`, `encrypt` | `change` | `mail.trash_draft` | app | A |
| `mail.update_draft` | `draft` and the same | `change` | the earlier draft | app | A |
| `mail.trash_draft`, `mail.restore_draft` | `draft` | `change` | each other | app | A |
| `mail.create_reply` | `message`, `to_all`, `cc`, `bcc`, `body`, `attachments`, `sign`, `encrypt` | `change` | `mail.trash_draft` | app | A |
| `mail.create_forward` | `message`, `to`, `cc`, `bcc`, `body`, `attachments`, `sign`, `encrypt` | `change` | `mail.trash_draft` | app | A |
| `mail.archive`, `mail.move`, `mail.trash` | `messages`; `mailbox` for `mail.move` only | as `update` | move back | app; one account only | A |
| `mail.open` | `message` | `show`, without remote images |  | app | A |
| `mail.send` | `draft`, `at` | `send` | within the delay, as `change` | app; no delay and no outbox | A |
| `mail.create_files` | `attachments`, `within` | `change`; shared: `send` | `files.trash` | app | A |
| `mail.update` | `messages`, `read`, `flagged`, `colour`, `junk` | `change`; shared mailbox: `send` | the earlier values | app; has no undo | A |
| `mail.open_draft` | `draft` | `show` |  | app | A |
| `mail.cancel_send` | `draft` | `change` | `mail.send`, asked again | new | A |
| `mail.restore` | `messages` | `change`; shared: `send` | `mail.trash` | app | A |
| `mail.stop_subscription` | `message` (RFC 2369, RFC 8058; the target is shown) | `send`, foreign |  | new | B |
| `mail.create_rule` | `from`, `to`, `subject` (`text`), `move_to` (`mail.mailbox`), `read`, `flagged` | `disrupt` | `mail.delete_rule` | app; no forwarding action | B |
| `mail.update_rule` | `rule` and the same | `disrupt` | the earlier rule | app | B |
| `mail.delete_rule` | `rule` | `disrupt` | the earlier rule | app | B |
| `mail.create_mailbox` | `name`, `account` | `change`; shared: `send` | removes it while empty | app (`worker.rs`) | B |
| `mail.update_mailbox` | `mailbox`, `name` | as `create_mailbox` | the earlier name | app | B |
| `mail.set_signature` | `account`, `body` | `send` | the earlier one | app | B |
| `mail.set_away` | `account`, `starts`, `until`, `body` (Sieve, RFC 5230 and RFC 5804) | `send` | turns it off | new | B |
| `mail.create_export` | `messages`, `format` (`eml`, `pdf`), `within` | `change`; shared: `send` | `files.trash` | app | B |
| `mail.delete_mailbox` | `mailbox` | `critical`; shared: also `send` |  | app | C |
| `mail.delete` | `messages` | `critical`; shared: also `send` |  | app | C |

Phrases in Search: "new message to {contact}".

A draft an intent makes stays on this computer until the person saves or sends it. `mail.send` takes
a draft and nothing else. From approval on the draft is frozen with copies of its attachments; it is
held for a short delay and never retried by itself. From the start of sending a mail is marked as
being sent, with its `Message-ID`, before SMTP begins: after a crash it is never sent again or
reopened by itself, and the person is told it may have been sent. `mail.get` returns what is on the
machine and fetches a missing body from the person's own server. "Block this sender" is a rule that
moves to Junk; no rule forwards. Accounts are [`accounts`](accounts.md); server settings, keys and
trusted senders have no intent. Snooze and templates do not exist.
