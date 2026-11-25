# Network

Part of [Bande Intents](../../intents.md). Main noun `connection`. Owner:
`crates/services/network-intents`. Work package D12.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `network.find` | `text`, `kind` (`wifi`, `ethernet`, `vpn`, `hotspot`), `known` | `read` |  | ready (`libs/network`) | A |
| `network.get` | `connections`, `properties` (state, addresses, router, DNS, metered, sign-in needed) | `read` |  | ready | A |
| `network.connect`, `network.disconnect` | `connection` (a known one) | `disrupt` | each other | ready | A |
| `network.add` | `name`, `hidden`; the key is asked by the owner | `critical` | `network.remove` | ready | A |
| `network.remove` | `connection` | `critical` |  | ready | B |
| `network.import` | `file` (VPN) | `critical` | `network.remove` | ready | B |
| `network.update` | `connection`, `addresses`, `dns`, `automatic`, `metered` | `critical` | the earlier values | ready | B |
| `network.start_hotspot`, `network.stop_hotspot` | `:only` | `critical`; `disrupt` | each other | ready | B |
| `network.open_sign_in` | `connection` | `send` |  | ready | B |

Addresses are this computer's own. A public address or a speed test needs someone else's server and
has no intent. Losing the connection an agent works over is said in the plan. Proxies are settings.
