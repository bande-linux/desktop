# Processes

Part of [Bande Intents](../../intents.md). Main noun `process`. Owner: the System Monitor app. Work
package D11.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `processes.process` | name~ text, app ref, processor, memory, disk, network, energy number, started date_time | name, app, the order's value |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `processes.find` | `text`, `order` (`processor`, `memory`, `disk`, `network`, `energy`); returns the ordered value | `read` |  | app; no energy | A |
| `processes.get` | `processes`, `properties` | `read` |  | app | A |
| `processes.compute_load` | `properties`; gives them | `read` |  | app | A |
| `processes.stop` | `process`, `force` | `disrupt`; with `force` `critical` |  | app | A |

A process is known by its id, start time and boot. Command lines, environments, open files and
connections are not returned: they hold secrets. `stop` asks it to quit and refuses the session's
own; `force` does not ask.
