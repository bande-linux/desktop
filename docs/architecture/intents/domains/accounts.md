# Accounts

Part of [Bande Intents](../../intents.md). Main noun `account`. Owner:
`crates/services/accounts-intents`. Work package D3.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `accounts.find` | `text`, `service` (`mail`, `notes`, `calendar`, `contacts`, `tasks`) | `read` |  | branch, and uncommitted work on it | A |
| `accounts.get` | `accounts`, `properties` (services, state, last synced) | `read` |  | branch | A |
| `accounts.add` | `address`, `kind`, `services`; opens the filled-in sheet, which shows the host found before it asks for the password | `critical` | `accounts.remove` | branch | A |
| `accounts.update` | `account`, `name`, `services` | `disrupt` | the earlier values | branch | B |
| `accounts.remove` | `account` | `critical` |  | branch | B |
| `accounts.start_sync` | `account`, `service` | `send` |  | app | A |
| `accounts.open` | `account` | `show` |  | branch | B |

Server and key settings are the person's, on the account's page.
