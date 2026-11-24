# Contacts

Part of [Bande Intents](../../intents.md). Main noun `contact`; `contacts.address_book`. Owner: the
Contacts process. Work package A1c.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `contacts.get` | `contacts`, `properties` (names, emails, phones, addresses, organisation, birthday, note) | as `find` |  | branch | A |
| `contacts.find_address_book` | `text` | `read` |  | branch | A |
| `contacts.find` | `text`, `address_book`, `duplicates` | `read`; foreign where what is in it is |  | branch | A |
| `contacts.trash`, `contacts.restore` | `contacts` | as `create` | each other | branch; there is no Trash, the removed card has to be kept | A |
| `contacts.open` | `contact` | `show` |  | branch | A |
| `contacts.create` | `name`, `nickname`, `related`, `emails`, `phones`, `addresses`, `organisation`, `job_title`, `birthday`, `note`, `photo`, `address_book` | `change`; server: `send`; for an agent see [Broker](../agents.md#broker) | `contacts.trash` | branch; nickname and related are new | A |
| `contacts.update` | `contact` and the same | as `create` | the earlier card | branch | A |
| `contacts.merge` | `contacts`, `into` | `disrupt` | the earlier cards | branch | B |
| `contacts.import` | `file`, `address_book` | `disrupt` | removes those | branch | B |
| `contacts.create_export` | `contacts`, `within` | `change`; server: `send` | `files.trash` | branch | C |

Things: `contacts.contact`, `contacts.address_book`.

`find` matches names, organisations, addresses and numbers, and `NICKNAME` and `RELATED` (RFC 6350),
so "mom" can be found; the branch does not read those two. Only Contacts writes the store, without a
lock, so its process becomes the owner. A note in a contact is foreign unless the person typed it
here. Groups have no intent.
