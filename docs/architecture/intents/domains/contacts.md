# Contacts

Part of [Bande Intents](../../intents.md). Main noun `contact`; `contacts.address_book`. Owner: the
Contacts app. Work package A1c.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `contacts.contact` | name~, nickname~, related~ text, emails list<address>, phones, addresses list<text>, organisation~, job_title~ text, birthday date, note\* text, address_book ref, groups list<ref>, photo picture | name, emails |
| `contacts.address_book` | name~ text, account ref, shared bool | all |
| `contacts.group` | name~ text, address_book ref | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `contacts.get` | `contacts`, `properties` | as `find` |  | branch | A |
| `contacts.find_address_book` | `text` | `read` |  | branch | A |
| `contacts.find` | `text`, `address_book`, `group`, `birthday_after`, `birthday_before`, `duplicates`, `trash` | `read`; foreign where what is in it is |  | branch | A |
| `contacts.find_group` | `text` | `read` |  | branch | B |
| `contacts.trash`, `contacts.restore` | `contacts` | as `create` | each other | branch; there is no Trash, the removed card has to be kept | A |
| `contacts.open` | `contact` | `show` |  | branch | A |
| `contacts.create` | `name`, `nickname`, `related`, `emails`, `phones`, `addresses`, `organisation`, `job_title`, `birthday`, `note`, `photo`, `address_book` | `change`; shared: `send`; for an agent see [Broker](../agents.md#broker) | `contacts.trash` | branch; nickname and related are new | A |
| `contacts.update` | `contact` and the same | as `create` | the earlier card | branch | A |
| `contacts.merge` | `contacts`, `into` | `disrupt`; shared: `send` | the earlier cards | branch | B |
| `contacts.import` | `file`, `address_book` | `disrupt`; shared: `send` | removes those | branch | B |
| `contacts.create_group` | `name`, `address_book` | `change`; shared: `send` | removes it | branch | B |
| `contacts.update_group` | `group`, `add`, `remove`, `name` | `change`; shared: `send` | the earlier group | branch | B |
| `contacts.create_export` | `contacts`, `within` | `change`; shared: `send` | `files.trash` | branch | C |

Phrases in Search: "open {contact}".

`find` matches names, organisations, addresses and numbers, and `NICKNAME` and `RELATED` (RFC 6350),
so "mom" can be found; the branch does not read those two. Only Contacts writes the store, so the
app is the owner.
