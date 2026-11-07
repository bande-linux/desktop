# Extensions: running the real uBlock Origin

The owner's decision: Browser runs **uBlock Origin type extensions, unlike modern Chrome**. There is
no built-in blocker. We implement the WebExtensions surface of **uBO's Firefox build**, so it runs at
full strength: Manifest V2 semantics with blocking `webRequest`, plus enough Manifest V3 for ordinary
extensions to install. `browser.*` with promises is the reference; `chrome.*` is the same object with
callbacks accepted.

## What uBO needs

| Area | API |
|---|---|
| Manifest | persistent `background.page`; `content_scripts` (`all_frames`, `match_about_blank`, `run_at: document_start`); `browser_action` popup; `options_ui` dashboard; `web_accessible_resources` (the `redirect=` resources); `commands`; `unlimitedStorage` |
| Network | `webRequest` with `webRequestBlocking`: `onBeforeRequest` (cancel, `redirectUrl`), `onBeforeSendHeaders`, `onHeadersReceived` (add CSP, remove headers), `onCompleted`, `onErrorOccurred`; `webRequest.filterResponseData` (HTML filtering, `replace=`); `dns.resolve` (CNAME uncloaking) |
| Pages | `tabs` (query, update, `insertCSS` with `cssOrigin: "user"`, `executeScript`, `sendMessage`), `webNavigation`, `menus`, `privacy` (prefetch, WebRTC IP policy) |
| Plumbing | `runtime` (messages, ports, `getURL`), `storage.local` (large), `i18n`, `alarms`, `permissions`, `management` (self) |
| MV3, for uBO Lite and others | `declarativeNetRequest`, `scripting`, `background.scripts` or a service worker |

## Process model

- **One extension process per extension**: a renderer binary started for the site
  `extension://<uuid>`, with the extension bindings on. It holds the background page and the
  extension's other pages (popup, dashboard) as documents of one agent, so `getBackgroundPage` keeps
  working. The uuid is random per install, as in Firefox, so pages cannot fingerprint extensions.
- **The sandbox is the renderer's**: no sockets but its inherited ones, no exec, not dumpable, in its
  own memory-capped scope. Landlock adds one read-only tree, that extension's unpacked package.
  It cannot read other extensions, their data or any profile file.
- **Its sockets**, all created and bound by the host: control to the host (as a renderer has), a
  network client socket bound to `extension://<uuid>`, and the **decision socket** to the network
  process described below.
- **Content scripts** run in the page's renderer, in an isolated world (a separate V8 context in the
  renderer's isolate) over the same DOM, as `web-script`'s several-worlds design provides. So an
  extension process never touches renderer memory. Everything between the two crosses the host,
  which stamps the sender (extension, tab, frame, URL) from its own records and hands a renderer only
  the scripts whose `matches` hit the document it committed.
- **Extension pages in the UI**: a popup is a view in a popover, the dashboard a tab, both drawn from
  display lists like any page, validated by the host.
- **Crash**: the host restarts the process, at once or after 5 s if the last start was recent. Pending
  decisions resolve as Continue, and the decision socket is replaced.
- **Idle**: zero wake-ups. `alarms` are deadlines in the process's poll timeout. uBO's background page
  is persistent, so it stays resident (budget below); MV3 service-worker extensions run as a
  background document with no eviction for now, which is what Firefox's event pages cost at most.

## Security boundary

- Extensions never reach renderer memory or each other. The kernel enforces it: separate processes,
  no shared mappings but sealed read-only memfds, no path to another package.
- The host and the network process treat every extension and renderer message as hostile. The network
  process accepts a verdict only for an event it sent to that extension, and only if the grant allows
  it: `webRequestBlocking` for pausing, host permission on the URL for seeing or changing a request,
  a redirect target that is http(s), `data:` or that extension's own web-accessible resource.
  A protocol violation kills the extension process, never the network process.
- A renderer cannot forge a sender, a tab, a frame's site or a request's session; the host binds
  those when it creates the socket.
- `.xpi` and `.crx` files are hostile input: bounded sizes, no path escape, no symlinks, fuzzed.
- Extensions do not see: their own requests or another extension's, the host's update checks, and
  private windows unless the user allowed it for that extension (off by default, as in Firefox).

## The network path

Decisions happen in `web-netd`, on its one event loop, and never block it.

1. **web-net emits stages.** A fetch with hooks on stops at `BeforeRequest` (after the HSTS upgrade,
   so listeners see the final scheme and no DNS has happened), `BeforeSendHeaders` (the real header
   list, cookies included), `HeadersReceived` (also for cache hits, so `csp=` and HTML filters apply
   to cached documents) and again for every redirect hop. Each stage is an event on the session's
   completion channel; the fetch task awaits a decision. With no extension installed no hook runs,
   so the cost is zero.
2. **declarativeNetRequest first, in process.** Rule sets, compiled at install, are matched natively
   in `web-netd`: no IPC, budget below. A block ends the request there.
3. **Native filter before crossing.** An extension registers listeners with their `RequestFilter`
   (match patterns, types, tab). `web-netd` evaluates filter and host permission itself, so a request
   crosses only to extensions that asked for it.
4. **Direct socket, no host hop.** Each extension has one `SOCK_SEQPACKET` decision socket straight to
   `web-netd`. A blocking stage sends `Event { event, listeners, details }` to each matching extension
   and parks the request in a table. Requests are pipelined; one paused request never holds another.
   Observing stages (`onSendHeaders`, `onBeforeRedirect`, `onResponseStarted`, `onCompleted`,
   `onErrorOccurred`, and non-blocking listeners) are fire and forget.
5. **Merging**, as Firefox: results apply in install order; a cancel wins at once, then the first
   redirect; header lists apply one after another.
6. **Slow and dead extensions.** A parked stage has a 10 s deadline and then continues (fail-open), on
   one timerfd armed only while something is parked. An extension that missed a deadline is marked
   stalled: further requests do not wait for it until it answers again, so a hung extension costs one
   10 s stall, not one per request. A queue over the socket limit, or a closed socket, means the same.
7. **Startup.** Requests wait for an extension with blocking listeners until it reports `Ready` (its
   background page's first run has finished), bounded by the same deadline, so nothing leaks at browser
   start, like Firefox's startup blocking. Listeners may answer later through a promise; uBO relies on
   that while it loads its filter lists.
8. **Details.** `requestId` (stable across redirects), `url`, `method`, `type`, `tabId`, `frameId`,
   `parentFrameId`, `originUrl`, `documentUrl`, `thirdParty`, `incognito`, `timeStamp`, `fromCache`,
   `statusCode`, `statusLine`, headers, `redirectUrl`, `error`, `ip`. No detail requires a DNS lookup.
   The host binds `tabId` to each client socket; a renderer reports its documents' frames and a
   document URL must be same-site with the socket's binding. An event over the 64 KiB message limit
   travels in a sealed memfd.
9. **`filterResponseData`.** A stage after the body arrives: the decoded body goes to the extension in
   a sealed memfd and a replacement comes back. To the extension it is `ondata` and `onstop`. It
   becomes chunked with the streaming response path.
10. **`dns.resolve`** is answered by `web-netd`, on a helper thread as dials are, and returns the
    canonical name and addresses.
11. **Resources.** `extension://<uuid>/…` fetches are served by `web-netd` from the unpacked package,
    read-only by Landlock. A client bound to that origin may fetch all its files, any other client only
    those the manifest lists as web accessible (and `matches` allows), so uBO's `redirect=` needs no host
    hop.

**Budgets, IPC only** (measured on the mini with a native peer): round trip 33 µs p50, 41 µs p99 for a
single request and 6.5 µs per request in a burst of 200, on `SOCK_SEQPACKET`, so shared-memory rings
would save too little to justify them. So:

| What | Budget |
|---|---|
| `web-netd` work per stage with one listener, excluding the listener | ≤ 50 µs |
| DNR match against uBO Lite's default rule sets (about 19,000 rules) | ≤ 20 µs p50, ≤ 200 µs p99 |
| DNR rule memory, default rule sets | ≤ 10 MB |
| A `webRequest` decision through V8 (listener included), uBO's default lists | ≤ 200 µs p50, ≤ 2 ms p99 |
| Page load with uBO, against no extension, ignoring what it blocks | ≤ 5% slower |
| uBO's process, idle with default lists | ≤ Firefox's uBO heap |
| Idle wake-ups | zero |

## Isolated worlds, CSS and scriptlets

- **User-origin CSS.** `web-style` has a user origin in the cascade: `insertCSS` with `cssOrigin: "user"`
  beats author rules with `!important`, and uBO's `display:none!important` holds against the page.
- **Early CSS.** The host sends a committing renderer the manifest's matching `css` with the commit,
  before parsing starts.
- **First-paint gate.** uBO asks its background page for cosmetic filters from `document_start`. While
  a document has `document_start` content scripts with unanswered calls to the extension, its first
  paint waits, bounded by 100 ms, so ads never flash.
- **Scriptlets.** uBO injects them with `executeScript` from `webNavigation.onCommitted`. The host
  sends `onCommitted` and holds the renderer's first page script until pending `document_start`
  injections for that document have arrived, bounded by 100 ms; in Firefox this is a race that uBO
  sometimes loses. Injected code runs in the isolated world and adds its `<script>` to the main world
  synchronously, as it does in Firefox.
- **Budget.** A world costs what a realm costs (`js.md`), and exists only in frames with matching scripts.

## Install, signatures, updates

- **Formats.** `.xpi`/`.zip` and CRX3 `.crx`, read by our own bounded ZIP reader (fuzzed), unpacked to
  `$XDG_DATA_HOME/org.bande.Browser/extensions/packages/<uuid>/<version>/`. Package files are never
  written after install. Per-extension data is under `extensions/data/<uuid>/`. The network process's
  writable folder is `network/` beside them.
- **Signatures, verified at install and at update, never at startup**, so an expired certificate can
  never disable an installed extension (Firefox's 2019 outage):
  - AMO `.xpi`: `META-INF/cose.sig`, a COSE signature (PS256 or ES256) over `cose.manifest`, chained
    to Mozilla's add-on root, code-signing EKU, certificate CN equal to the extension id, and every file
    hashed by SHA-256 in the manifest, with none missing or extra. PKCS#7-only add-ons count as unsigned.
  - CRX3: every proof verified against the file; the id comes from the author key. It counts as store
    signed only with Google's publisher proof.
  - Anything else counts as unsigned and installs only with Settings → Advanced → "Allow unsigned
    extensions" (off), with a warning. No remote kill switch.
- **Identity.** `browser_specific_settings.gecko.id`, or the CRX id, or for unsigned packages a hash
  of the name. Same id means an update, not a second copy. Settings-import of uBO works since the gecko
  id is kept.
- **Updates.** A daily deadline while the browser runs, from the host, in a session with no cookies:
  AMO's version check (or the manifest's `update_url`) for signed packages. Same verification. A new
  version asking for more permissions waits for the user's approval and keeps running the old one until
  then. Nothing else leaves the machine: no telemetry, no blocklist ping. The Web Store is not
  polled, as its protocol identifies the browser as Chrome.

## Permissions and UI

- **Model.** From the manifest: API permissions (`storage`, `tabs`, …), origins (`<all_urls>`, match
  patterns; MV3's `host_permissions`, `content_scripts` matches), optional ones. Per extension, the
  user's state: enabled, granted API permissions and origins, allowed in private windows. Unknown API
  permissions are ignored with a note in the UI, as Firefox does. MV3 host permissions are shown like
  MV2's and granted at install, as requested by the extension, with per-site removal in Settings.
- **Prompt at install and on escalation**: name, version, signature state, what it may do in plain words
  ("Read and change all your data on all websites", "Block and change network requests").
- **Settings → Extensions**: list, enable, remove, permissions, site access, private windows, a popup
  button per extension in the toolbar overflow.
- **Installing**: a navigation to `application/x-xpinstall` or `.crx`, or File → Install Extension….

## declarativeNetRequest

`web_extensions::dnr` compiles `rule_resources` at install into a compact table stored beside the
package; `web-netd` loads the enabled sets. Rules are indexed by the longest literal token of
`urlFilter`, or by request domain. `regexFilter` uses the `regex` crate (RE2 syntax, Chrome's and
Firefox's contract, linear time on hostile patterns, compiled on first use). Priority semantics are
Chrome's. Dynamic and session rules arrive through the decision socket with the API.

## Storage

`storage.local` keeps one append-only log per extension under `extensions/data/<uuid>/`, compacted by
rewrite and rename when half of it is garbage, served by a storage thread in the host so disk work never
stalls the broker. Values are opaque bytes (the bindings encode them); values over 32 KiB cross in a
sealed memfd. Quota 10 MiB, or none with `unlimitedStorage`, enforced there. `storage.sync` is the same
store with no sync.

## New dependencies and why

- `rustls-webpki`, `aws-lc-rs` (both already linked through rustls): certificate chains and signature
  checks for add-ons, SHA-256 of package files.
- `regex`: `regexFilter`; RE2 semantics are the API's contract and linear time is a security property.
- `serde_json`: manifests and rule files are JSON from hostile packages; it runs at install only.

## Status and order

1. Now, without script: stages in web-net and `web-netd`, DNR, manifests, install, signatures,
   storage and permissions, user-origin CSS, Settings.
2. With V8 bindings (`web-core`): the extension process, isolated worlds, then the API surface.
3. **Done when** real uBO from AMO installs, its default lists block on the top-sites list, cosmetic and
   HTML filters apply and CNAME uncloaking works, measured for load time and blocking against Firefox with
   uBO.

Until then privacy rests on the defaults: third-party cookies blocked, partitioned storage, HTTPS-only.
