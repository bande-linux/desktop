# Machines and containers

Part of [Bande Intents](../../intents.md). Main nouns `machine` and `container`;
`machines.snapshot`, `containers.image`, `containers.volume`, `containers.project`. Owner:
`crates/services/velox-intents`. Work package D20.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `machines.find`, `machines.get` | `text`; `machines`, `properties` (state, memory, processors, disk) | `read` |  | ready | B |
| `containers.find`, `containers.get` | `text`; `containers`, `properties` (name, image, state, engine) | `read` for names, foreign for images |  | app | B |
| `containers.find_image`, `containers.find_volume` | `text` | `read` for names, foreign for images |  | app | B |
| `machines.start` | `machine` | `disrupt` (runs what is in it) |  | ready | C |
| `machines.create_snapshot` | `machine`, `name` | `change` | removes that snapshot | ready | C |
| `containers.start`, `containers.stop`, `containers.pause`, `containers.resume` | `container` | `disrupt`; remote engine: also `send` |  | app | C |
| `machines.pause`, `machines.resume` | `machine` | `disrupt` | each other | ready | B |
| `machines.stop` | `machine`, `suspend`, `force` | `disrupt`; with `force` `authorise` |  | part | B |
| `machines.create` | `image` (catalogue), `name`, `processors`, `memory`, `disk` | `authorise` | trash while unused | ready | B |
| `machines.update` | `machine`, `name`, `processors`, `memory`, `start_at_login` | `disrupt` | the earlier values | app | B |
| `machines.copy` | `machine`, `name` | `disrupt` |  | ready | C |
| `machines.restore_snapshot` | `snapshot` | `authorise`, a new snapshot first |  | ready | B |
| `machines.delete` | `machine` | `authorise` |  | ready | B |
| `machines.delete_snapshot` | `snapshot` | `authorise` |  | ready | C |
| `containers.start_project`, `containers.stop_project` | `project`: its existing containers | `disrupt`; remote engine: also `send` | each other | app | B |
| `containers.delete` | `containers` | `authorise` |  | app | B |
| `containers.delete_unused` | `kinds` (`containers`, `images`, `volumes`, `networks`) | `authorise` |  | app | A |
| `containers.delete_image`, `containers.delete_volume` | `image`; `volume` | `authorise` |  | app | C |
| `containers.start_engine`, `containers.stop_engine` |  | `disrupt`, or `authorise` as administrator | each other | app | C |

A machine is known by its manifest id, a container by its id and its engine. Environment, labels,
logs, consoles, addresses and first-boot settings are not returned. Running, executing, building and
compose files of the caller's choice have no intent, nor have pulling, shares, ports, networks and
USB. A machine whose start would ask for a password is refused.
