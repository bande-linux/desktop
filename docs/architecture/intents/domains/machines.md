# Machines and containers

Part of [Bande Intents](../../intents.md). Main nouns `machine` and `container`;
`machines.snapshot`, `containers.image`, `containers.volume`, `containers.project`. Owner: the Velox
app. Work package D20.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `machines.machine`, `machines.snapshot`, `machines.image` | name~ text, state `running` `paused` `stopped`, memory, disk integer, processors integer; name~, made date_time; name~, size integer | all |
| `containers.container`, `.image`, `.volume`, `.project` | name~ text, image\* text, state enum, engine text, remote bool; name\* text, size integer | name, state |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `machines.find`, `machines.get` | `text`; `machines`, `properties` (state, memory, processors, disk) | `read` |  | ready | B |
| `containers.find`, `containers.get` | `text`; `containers`, `properties` (name, image, state, engine) | `read` for names, foreign for images |  | app | B |
| `containers.find_image`, `containers.find_volume` | `text` | `read` for names, foreign for images |  | app | B |
| `machines.find_image` | `text` | `read` |  | ready | B |
| `machines.find_snapshot` | `machine` | `read` |  | ready | B |
| `containers.find_project` | `text` | `read` |  | app | B |
| `machines.start` | `machine` | `disrupt` (runs what is in it) |  | ready | B |
| `machines.create_snapshot` | `machine`, `name` | `change` | removes that snapshot | ready | B |
| `containers.start`, `containers.stop`, `containers.pause`, `containers.resume` | `container` | `disrupt`; remote engine: also `send` |  | app | B |
| `machines.pause`, `machines.resume` | `machine` | `disrupt` | each other | ready | B |
| `machines.stop` | `machine`, `suspend`, `force` | `disrupt`; with `force` `critical` |  | part | B |
| `machines.create` | `image` (`machines.image`), `name`, `processors`, `memory`, `disk` | `critical` | trash while unused | ready | B |
| `machines.update` | `machine`, `name`, `processors`, `memory`, `start_at_login` | `disrupt`; `start_at_login`: `critical` | the earlier values | app | B |
| `machines.restore_snapshot` | `snapshot` | `critical`, a new snapshot first |  | ready | B |
| `machines.delete` | `machine` | `critical` |  | ready | B |
| `containers.start_project`, `containers.stop_project` | `project`: its existing containers | `disrupt`; remote engine: also `send` | each other | app | B |
| `containers.delete` | `containers` | `critical` |  | app | B |
| `machines.copy` | `machine`, `name` | `disrupt` |  | ready | C |
| `machines.delete_snapshot` | `snapshot` | `critical` |  | ready | C |
| `containers.delete_unused` | `kinds` (`containers`, `images`, `volumes`, `networks`) | `critical` |  | app | C |
| `containers.delete_image`, `containers.delete_volume` | `image`; `volume` | `critical` |  | app | C |
| `containers.start_engine`, `containers.stop_engine` |  | `disrupt`, or `critical` as administrator | each other | app | C |

A machine is known by its manifest id, a container by its id and its engine. Environment, labels,
logs, consoles, addresses and first-boot settings are not returned. Running, executing, building and
compose files of the caller's choice have no intent, nor have pulling, shares, ports, networks and
USB. A machine whose start would ask for a password is refused.
