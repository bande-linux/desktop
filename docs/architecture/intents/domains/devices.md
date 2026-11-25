# Devices

Part of [Bande Intents](../../intents.md). Main noun `device`; also `print` (`job`) and `scanners`
(`scanner`). Owner: `bande-system-intents`. Work package D13.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `devices.device` | name~ text, kind `sound` `input` `phone` `other`, connected, known bool, battery integer | all |
| `print.printer`, `print.job` | name~ text, state `ready` `printing` `stopped`, reasons list<text>, paper list<text>, ink list<integer>, default bool; title\* text, state enum | all |
| `scanners.scanner` | name~ text | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `devices.find` | `text`, `kind`, `nearby` | `read`; with `nearby` `disrupt` |  | ready (`libs/bluetooth`) | A |
| `devices.get` | `devices`, `properties` (kind, connected, battery) | `read` |  | ready | A |
| `print.find_printer` | `text` | `read` |  | ready (`libs/printers`) | A |
| `print.find_job` | `printer` | `read` |  | ready | A |
| `print.get_printer` | `printers`, `properties` | `read` |  | ready | A |
| `scanners.find` | `text` | `read` |  | ready | B |
| `devices.connect`, `devices.disconnect` | `device` (a paired one) | sound `change`, else `disrupt` | each other | ready | A |
| `print.start` | `file`, or a note, message, event or photo, `printer`, `copies`, `pages` (`text`, as `1-3,7`), `sides` (`one`, `two`), `paper`, `colour` (`bool`) | `send` |  | part: files | A |
| `print.cancel_job` | `job` | `disrupt` |  | ready | A |
| `devices.add` | `device`; not a keyboard or pointer, and the owner confirms the code | `critical` | `devices.remove` | ready | B |
| `print.add_printer`, `print.remove_printer` | `address`; `printer` | `critical` | each other | ready | B |
| `scanners.create` | `scanner`, `pages`, `colour`, `format`, `within` | `disrupt` | `files.trash` | ready | B |
| `print.set_default` | `printer` | `change` | the earlier one | ready | B |
| `devices.remove` | `device` | `disrupt` |  | ready | C |
| `print.pause_printer`, `print.resume_printer` | `printer` | `disrupt` | each other | ready | C |
