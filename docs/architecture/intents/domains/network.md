# Network

Part of [Bande Intents](../../intents.md). Main noun `connection`. Owner: `bande-system-intents`.
Work package D12.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `network.connection` | name~ text, kind enum, state `connected` `connecting` `disconnected`, addresses, router, dns list<text>, metered `yes` `no` `unknown`, sign_in_needed, known bool, security `open` `wpa2` `wpa3` `enterprise` `none` | name, kind, state, known |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `network.find` | `text`, `kind` (`wifi`, `ethernet`, `vpn`, `hotspot`), `known` | `read` |  | ready (`libs/network`) | A |
| `network.get` | `connections`, `properties` (state, addresses, router, DNS, metered, sign-in needed) | `read` |  | ready | A |
| `network.connect`, `network.disconnect` | `connection` (a known one) | `disrupt` | each other | ready | A |
| `network.add` | `name`, `hidden`; the key is asked by the owner | `critical` | `network.remove` | ready | A |
| `network.remove` | `connection` | `critical` |  | ready | B |
| `network.start_hotspot`, `network.stop_hotspot` | `:only` | `critical`; `disrupt` | each other | ready | B |
| `network.open_sign_in` | `connection` | `send` |  | ready | B |
| `network.import` | `file` (VPN) | `critical` | `network.remove` | ready | C |
| `network.update` | `connection`, `addresses`, `dns`, `automatic`, `metered` | `critical` | the earlier values | ready | C |

Addresses are this computer's own. A public address or a speed test needs someone else's server and
has no intent. Losing the connection an agent works over is said in the plan. Proxies are settings.
