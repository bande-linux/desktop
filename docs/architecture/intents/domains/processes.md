# Processes

Part of [Bande Intents](../../intents.md). Main noun `process`. Owner:
`crates/services/processes-intents`. Work package D11.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `processes.find` | `text`, `order` (`processor`, `memory`, `disk`, `network`, `energy`); returns the ordered value | `read` |  | app; no energy | B |
| `processes.get` | `processes`, `properties` (name, app, processor, memory, disk, network, started) | `read` |  | app | B |
| `processes.compute_load` | `properties` (processor, memory, disk, network, temperature) | `read` |  | app | B |
| `processes.stop` | `process`, `force` | `disrupt`; with `force` `authorise` |  | app | A |

A process is known by its id, start time and boot. Command lines, environments, open files and
connections are not returned: they hold secrets. `stop` asks it to quit and refuses the session's
own; `force` does not ask.
