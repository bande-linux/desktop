# Accounts

Part of [Bande Intents](../../intents.md). Main noun `account`. Owner: `bande-system-intents`, on
the accounts store. Work package D3.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `accounts.account` | name~ text, address address, services list<enum>, state `ok` `offline` `sign-in`, synced date_time, shared bool | name, services |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `accounts.find` | `text`, `service` (`mail`, `notes`, `calendar`, `contacts`, `tasks`) | `read` |  | branch, and uncommitted work on it | A |
| `accounts.get` | `accounts`, `properties` | `read` |  | branch | A |
| `accounts.start_sync` | `account`, `service` | `change` |  | app | A |
| `accounts.add` | `address`, `kind`, `services`; opens the filled-in sheet, which shows the host found before it asks for the password | `critical` | `accounts.remove` | branch | B |
| `accounts.update` | `account`, `name`, `services` | `disrupt` | the earlier values | branch | B |
| `accounts.remove` | `account` | `critical` |  | branch | B |
| `accounts.open` | `account` | `show` |  | branch | B |

Server and key settings are the person's, on the account's page.
