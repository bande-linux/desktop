# Mail

Part of [Bande Intents](../../intents.md). Main noun `message`; `mail.thread`, `mail.draft`,
`mail.attachment`, `mail.mailbox`, `mail.rule`. Owner: `crates/services/mail-intents`, on the engine
split from the app. Work package D4.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `mail.find` | `text`, `from`, `to`, `mailbox`, `after`, `before`, `unread`, `flagged`, `attachments` | foreign |  | app | A |
| `mail.find_thread` | `text`, `message` | foreign |  | app | A |
| `mail.get` | `messages`, `properties` (from, to, cc, date, subject, body, attachments, thread) | foreign; does not mark read |  | app | A |
| `mail.find_mailbox` | `text`, `account` | `read` |  | app | A |
| `mail.find_rule` | `text` | `read` |  | app (`store.rs`) | B |
| `mail.create_draft` | `account`, `to`, `cc`, `bcc`, `subject`, `body`, `attachments` | `change` | `mail.trash_draft` | app | A |
| `mail.update_draft` | `draft` and the same | `change` | the earlier draft | app | A |
| `mail.trash_draft`, `mail.restore_draft` | `draft` | `change` | each other | app | A |
| `mail.create_reply` | `message`, `to_all`, `cc`, `bcc`, `body`, `attachments` | `change` | `mail.trash_draft` | app | A |
| `mail.create_forward` | `message`, `to`, `cc`, `bcc`, `body`, `attachments` | `change` | `mail.trash_draft` | app | A |
| `mail.archive`, `mail.move`, `mail.trash` | `messages`, `mailbox` | as `update` | move back | app; one account only | A |
| `mail.open` | `message` | `show`, without remote images |  | app | A |
| `mail.send` | `draft`, `at` | `send` | within the delay, as `change` | app; no delay and no outbox | A |
| `mail.create_files` | `attachments`, `within` | `change`; server: `send` | `files.trash` | app | A |
| `mail.update` | `messages`, `read`, `flagged`, `colour`, `junk` | `change`; shared mailbox: `send` | the earlier values | app; has no undo | A |
| `mail.stop_subscription` | `message` (RFC 2369, RFC 8058; the target is shown) | `send`, foreign |  | new | A |
| `mail.create_rule` | `from`, `to`, `subject`, `move_to`, `read`, `flag` | `disrupt` | `mail.delete_rule` | app; no forwarding action | B |
| `mail.update_rule` | `rule` and the same | `disrupt` | the earlier rule | app | B |
| `mail.delete_rule` | `rule` | `disrupt` | the earlier rule | app | B |
| `mail.create_mailbox` | `name`, `account` | `change`; shared: `send` | removes it while empty | app (`worker.rs`) | B |
| `mail.update_mailbox` | `mailbox`, `name` | as `create_mailbox` | the earlier name | app | B |
| `mail.delete_mailbox` | `mailbox` | `critical` |  | app | C |
| `mail.delete` | `messages` | `critical` |  | app | C |
| `mail.set_signature` | `account`, `body` | `send` | the earlier one | app | C |
| `mail.set_away` | `account`, `from`, `until`, `body` (Sieve, RFC 5230 and RFC 5804) | `send` | turns it off | new | B |

Things: `mail.message`, `mail.thread`, `mail.draft`, `mail.attachment`, `mail.mailbox`,
`mail.account`.

A draft an intent makes stays on this computer until the person saves or sends it. `mail.send` takes
a draft and nothing else. From approval on the draft is frozen with copies of its attachments; it is
held for a short delay and never retried by itself. `mail.get` returns what is on the machine;
fetching a missing body contacts the person's own server and needs the grant to say so. "Block this
sender" is a rule that moves to Junk; no rule forwards. Accounts are [`accounts`](accounts.md);
server settings, keys and trusted senders have no intent. Snooze and templates do not exist.
