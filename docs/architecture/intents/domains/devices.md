# Devices

Part of [Bande Intents](../../intents.md). Main noun `device`; also `print` (`job`) and `scanners`
(`scanner`). Owner: `crates/services/devices-intents`. Work package D13.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `devices.find` | `text`, `kind`, `nearby` | `read`; with `nearby` `disrupt` |  | ready (`libs/bluetooth`) | A |
| `devices.get` | `devices`, `properties` (kind, connected, battery) | `read` |  | ready | A |
| `devices.connect`, `devices.disconnect` | `device` (a paired one) | sound `change`, else `disrupt` | each other | ready | A |
| `devices.add` | `device`; not a keyboard or pointer, and the owner confirms the code | `authorise` | `devices.remove` | ready | B |
| `devices.remove` | `device` | `disrupt` |  | ready | C |
| `print.find_printer` | `text` | `read` |  | ready (`libs/printers`) | A |
| `print.find_job` | `printer` | `read` |  | ready | A |
| `print.start` | `file` or a thing, `printer`, `copies`, `pages`, `sides`, `colour` | `send` |  | part: files | A |
| `print.cancel_job` | `job` | `disrupt` |  | ready | A |
| `print.pause_printer`, `print.resume_printer` | `printer` | `disrupt` | each other | ready | B |
| `print.add_printer`, `print.remove_printer` | `address`; `printer` | `authorise` | each other | ready | B |
| `scanners.find` | `text` | `read` |  | ready | A |
| `scanners.create` | `scanner`, `pages`, `colour`, `format`, `within` | `disrupt` | `files.trash` | ready | A |
