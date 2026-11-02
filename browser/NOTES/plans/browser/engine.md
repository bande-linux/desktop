# Engine architecture

## Crates in `bande-linux/web`

One Cargo workspace with one lock. Dependencies run only downward in this list.

| Crate | Owns |
|---|---|
| `web-atoms` | interned strings: static atoms generated at build time, per-document dynamic tables, no global mutex |
| `web-html` | tokenizer, tree builder, XML parser for XHTML, serialiser, preload scanner |
| `web-css` | syntax, property table and generated value types, stylesheets, media queries |
| `web-dom` | arena, node kinds, mutation journal, events, ranges, selection |
| `web-style` | selector matching, invalidation sets, cascade, computed style, style sharing |
| `web-text` | inline formatting context over harfrust, skrifa and ICU4X |
| `web-layout` | box tree, fragment trees, every formatting context |
| `web-paint` | display lists, stacking contexts, hit testing |
| `web-net` | Fetch, cookies, cache, HSTS, the webRequest hook, the epoll executor, HTTP over hyper, h2, rustls |
| `web-script` | WebIDL bindings generator and Web APIs over a narrow engine interface |
| `web-js` | our JavaScript engine (from phase 3) |
| `web-media` | demuxers, MSE, AVD bitstream parsers over V4L2, Opus, A/V sync |
| `web-cdm` | EME and the CDM host |
| `web-extensions` | the WebExtensions APIs |
| `web-ipc` | message schema, shared memory, process launch and sandbox |
| `web-host` | in the app process: launches the helpers, routes navigations to renderers, validates what they send, reports crashes |
| `web-renderer`, `web-netd` | the renderer and network processes (libraries with a `main`; the app ships them as `bande-browser-renderer` and `bande-browser-network`) |
| `web-compositor` | in the app process: layers, async scroll and animation, drawing into the toolkit `Scene` |
| `web-mediad`, `web-cdmd`, `web-gpu` | the later helper processes' `main`s |
| `tools/wpt`, `fuzz/` | the conformance runner and the fuzz targets |

## Processes

- **The app process** (`bande-browser`, a bande-app app) runs the UI, the compositor and the GPU
  device. It holds no page data beyond display lists and accessibility trees. `web-host`'s broker
  thread there validates them as untrusted: every count and index is bounded, coordinates are
  finite, link targets are http(s), and a malformed message kills the renderer.
- **The network process** (`web-netd`) does all sockets and TLS, with one `Fetcher` per browsing
  session (`Session::Default` or `Private(n)`, ended when its last view closes). Cookies, the cache
  and HSTS live in web-net and so run here. The host fetches navigations; each renderer has its own
  socket to it, which the host binds to the renderer's session and site, so a renderer cannot
  forge either and cannot fetch documents. Its sandbox:
  - Landlock allows reading only the certificate stores, the resolver and NSS configuration,
    `/usr/lib` (NSS modules), Debian's public suffix list and `/run/systemd/resolve`;
  - Landlock allows writing only beneath its `--storage` folder (the app's
    `$XDG_DATA_HOME/org.bande.Browser`), where web-net keeps cookies, HSTS and the cache;
  - Unix sockets may connect only to systemd-resolved, nscd and avahi;
  - it cannot exec, and it is not dumpable;
  - a seccomp allowlist adds sockets for Unix, IPv4, IPv6 and route netlink only, `ioctl` only for
    `FIONBIO` and `FIONREAD`, and `clone` only for threads (`clone3` returns `ENOSYS`, so glibc falls
    back to `clone`, as in Chromium's policy);
  - TCP ports stay open: Landlock's network rules work per port, and pages use any port.
- **After a network-process crash** the host fails the pending navigations. It restarts the process
  at once, or at the next command if the last start was under 5 s ago, so a crash loop never spins.
  It then hands every live renderer a new network socket; the renderers keep their pages and treat
  their in-flight fetches as failed.
- **Hung renderers.** A renderer answers every batch of commands with `Done { commands }`, the
  number it has handled so far. The host arms a deadline only while some commands are unanswered,
  on one timerfd for all renderers, which is disarmed when nothing is outstanding. When the deadline
  passes (`Helpers::unresponsive_after`, 15 s in the app), the view gets `Outcome::Unresponsive`.
  `Command::Wait` re-arms the deadline, `Command::Kill` kills the renderer (`Outcome::Crashed`
  follows), and progress after a report sends `Outcome::Responsive`. Script (phase 2) runs as
  tasks in the renderer's poll loop and has to keep acknowledging between tasks.
- **Site data** goes through the host as well. `Command::ClearData { session, clear }` runs
  web-net's `Fetcher::clear` (cookies and cache by time and site) in that session.
  `Command::SiteData { view, session }` is answered to that view with `Outcome::SiteData`; the list
  travels in a sealed memfd, and comes back empty if the network process dies first.
- **The webRequest hook** is in the network process too. With interception on, each request pauses
  before it starts (the `onBeforeRequest` stage) and the host gets `Outcome::Paused` for the view
  that made it; `Command::Resume` continues, cancels or redirects it. A timerfd armed only while
  something is paused continues a request after 10 s.
- **One renderer per tab and site** runs HTML, style, layout, paint and image decoding (AVIF
  included). The site is the scheme plus the registrable domain, from `web_net::Site`, which reads
  Debian's public suffix list (the `publicsuffix` package). Without the list, a site is the full
  origin, which is stricter. A navigation to another site replaces
  the process, chosen after redirects. Its sandbox follows Chromium's layers, and bubblewrap's
  for namespaces:
  - new user, network and IPC namespaces (best effort, before anything else);
  - fonts and fontconfig initialised;
  - Landlock (ABI up to 9, best effort, but refusing to run unenforced) allowing only reading
    under the font and fontconfig folders: no other reads, no writes, no exec, no TCP, no
    pathname or abstract Unix sockets, no signals out;
  - not dumpable;
  - a seccomp allowlist (EPERM for the rest): no `socket`, `connect`, `clone`, `execve`,
    `ptrace` or `ioctl`, and `tgkill` only to itself.

  It has no GPU and no Wayland. The app puts each renderer in a transient systemd user scope
  (`app-bande-org.bande.Browser.Renderer-PID.scope` in `app.slice`, `MemoryMax` 512 MB, no swap),
  as the desktop places apps. It waits for the scope job's `JobRemoved` signal before the host hands
  the renderer its first page. A crash or an OOM kill reaches the host as a hang-up and becomes
  `Outcome::Crashed`.
- **The media process** (phase 4) demuxes and decodes into NV12 dmabufs (`media.md`).
- **The CDM process** (phase 4, per site) hosts Widevine once licensed.
- **The extension process** (phase 4, one per extension) runs the extension's pages.
- **The GPU process** (phase 4) runs WebGL and WebGPU through wgpu and hands frames over as
  dmabuf, so untrusted shaders never run in the app.

IPC is `SOCK_SEQPACKET` Unix sockets with fd passing. A message is at most 64 KiB, encoded by
hand-written `Wire` impls with no versioning. Display lists, documents, images and font files
travel in sealed memfds that the receiver maps read-only. Senders queue and never block, so a peer
that stops reading cannot stall the host or the network process. Fonts and images go once per
renderer (images once per load) and are referred to by id afterwards. A display list goes as a
patch against the previous one of the same load: the common prefix and suffix of its items and
glyphs are kept, and the link table is sent only when it changes. A first frame is a patch against
nothing. The host rebuilds the list, validates it whole, and reports the damaged rectangle with
`Outcome::Relaid`: the union of the changed items, or `None` when the size, the background or a
clip or opacity group changed. The app then rebuilds only the tiles that rectangle touches. Helpers start with a cleared
environment, the control socket on stdin and the renderer's network socket on stdout.

**Gaps:**
- Font files cross into the app as bytes and are parsed there. Glyph outlines (D1) remove that.
- Image pixels are copied once into the app's `Arc`s and the app keeps them for the load.
- Only the `onBeforeRequest` stage exists.

**Measured** (release, the mini, Debian's fonts):
- An idle renderer is 1.7 MB anonymous and 8.4 MB resident (the rest is shared files). It starts,
  initialises fonts and enters the sandbox in 14–15 ms warm.
- The Rust article's renderer is 14–15 MB anonymous, with 21 images; Cat's is 18–19 MB with 52.
  The renderer keeps images encoded and drops their pixels once sent, keeps no fragment tree
  between layouts, and caps shaped words at 2 MB. Helpers run with glibc's mmap and trim
  thresholds fixed (`GLIBC_TUNABLES`), so freed buffers go back to the kernel.
- The idle network process is 1–7 MB anonymous.
- The app with one window on that article is 29.8 MiB anonymous.
- Every process makes zero wake-ups when idle.

## Pipeline and efficiency

- **Compact data.**
  - Arena DOM with 32-bit ids.
  - The element core is 64 B or less; attributes and layout data sit in side tables.
  - Atoms for every name.
  - Computed styles shared by `Arc` between equal elements (sharing cache) and split into inherited
    and reset groups.
  - Fragments are immutable and reused across layouts.
- **Incremental everything.**
  - A DOM mutation journal feeds style invalidation sets.
  - Restyle damage marks only the boxes that need layout.
  - Layout caches fragments per formatting context by their constraints.
  - Paint rebuilds only the dirty stacking contexts.
  - The compositor redraws only damaged layers.
- **Parallel where it pays.**
  - Style traversal and layout of independent formatting contexts run on a work-stealing pool,
    only above a size threshold. Small pages run on one thread.
  - Pool threads park; they never spin.
- **Zero idle wake-ups.**
  - Every process blocks in `epoll` with deadlines only.
  - Page timers and `requestAnimationFrame` follow the window's frame callbacks and stop when the
    tab is hidden.
  - The network pools close idle connections by deadline.
- **A strict budget per tab.**
  - Each renderer has a memory cgroup limit (default 512 MB, raised per site by the user).
  - Decoded images are capped per tab and decoded at display size.
  - A tab hidden for 10 minutes, or under system memory pressure (desktop decision 0004), is
    discarded to its URL, state and snapshot.
- **Off the critical path.**
  - Parsing streams as bytes arrive, and the preload scanner fetches ahead.
  - Fonts and images decode on helper threads.
  - Scrolling and compositor animations never wait on a renderer.
  - Parse, style and layout run in the renderer process, never in the app; the UI only draws
    display lists.

## Measurable targets

Each target is measured on the M2 mini against Debian's `chromium` and `firefox-esr` with the
same pages, profile-free, reported in release notes, not stored in the repo.

| Metric | Target |
|---|---|
| Browser window, cold start | ≤ 150 ms (Chromium and Firefox: 500 ms or more) |
| App process, one tab | ≤ 40 MB private memory |
| Renderer, static article (a Wikipedia page) | ≤ 25 MB, at most 40% of Chromium's renderer |
| Renderer, Google Docs document (phase 5) | ≤ 70% of Chromium's |
| Element core | ≤ 64 B; whole DOM plus style plus layout per element ≤ 50% of Chromium's heap snapshot |
| Article: response to first paint | ≤ 50% of Chromium's time from a warm cache |
| Frame time while scrolling | p99 ≤ 6 ms at 120 Hz, never waiting on a renderer |
| Idle | zero wake-ups in all processes on a static page (`bande-background` and `perf trace`) |
| No window open | ≤ 6 MB, helpers exited |
| Speedometer 3 | phase 2–3: tracked only; phase 5: ≥ Firefox, then ≥ Chromium |
| JetStream 2 | phase 5, with our JIT: ≥ 80% of Chromium |

### Per-element budgets (W1 core)

`Page::memory()` in `web-content` reports heap bytes per stage; the `render-png` example prints
them per element. Measured on the English Wikipedia articles "Rust (programming language)" (9.7k
elements, with its own stylesheets) and "Cat" (13.4k, without), at 1000 px:

| Stage | Budget per element | Now |
|---|---|---|
| Node core (`web-dom`) | ≤ 64 B | 40 B |
| DOM with attributes, text and the document's atoms | ≤ 420 B | 293–400 B |
| Computed styles (shared, 4 B index per node) | ≤ 40 B | 14–18 B |
| Box tree with shaped text | ≤ 450 B | 210–238 B |
| Fragment tree | ≤ 300 B | 225–256 B |
| Display list | ≤ 280 B | 185–264 B |

Shaped glyphs are 12 B, with the rare mark offsets in a side table, and laid-out glyphs 8 B (a
`u16` id, the y offset in 1/16 px, x). Line breaks and spaces are flags on the shaped glyphs, so no
paragraph text is kept after shaping, and the shaped vectors are trimmed to length. Shape plans and
shaped words are cached per font, size and direction. `tools/bench` measures the same stages on its
generated article.

Web fonts decode in `web-text` (`decode_font`: WOFF2 with the glyf, loca and hmtx transforms,
WOFF, TTF and OTF; fuzzed). Each page registers its `@font-face` faces in the shared `Fonts` under
private family aliases, one per family and `unicode-range`, so pages never see each other's fonts,
and `release` unregisters them. Only faces the page uses are fetched, and layout never waits for
them.

Styles are shared three ways: siblings with the same name and classes reuse a style after
revalidating the selectors that depend on attributes, position or siblings; elements with the same
parent style and the same matched rules reuse one computed style (the Rust article's 9.7k elements
need 861 styles); equal computed styles are interned by value; and inherited and reset groups are shared `Arc`s. Fragments keep borders, layers and images in an optional side record, so a plain box is 56 B. Its 220 KB of CSS styles in
about 12 ms.

## `web-css` for the cascade

- **Entry points:** `parse_stylesheet(css, atoms)` gives a `Stylesheet` of `CssRule`s (style rules
  with nested rules, `@media`, `@supports` evaluated at parse time, `@container`, `@layer`,
  `@import`, `@font-face` as a parsed `FontFace`, `@keyframes`); `@namespace` fills `namespaces`.
  `parse_style_attribute` gives a `DeclarationBlock`; `decode_stylesheet` turns bytes into text
  by BOM, protocol label, `@charset` and environment encoding.
- **Properties:** `properties.rs` (crate root) is the table; `build.rs` generates `LonghandId`
  (dense `u16`), `ShorthandId`, `PropertyId::from_name` (with aliases), a keyword enum per keyword
  type, `PropertyValue` (one variant per value type) and the parse and serialisation dispatch.
  A block holds `Declaration { id, value, important }` with `DeclaredValue::{Value, CssWide,
  Unparsed}`; shorthands expand to longhands (unset parts as `CssWide(Initial)`), values with
  `var()` stay `Unparsed` for `parse_value_str` after substitution, and custom properties sit in
  `custom` as raw text. Lengths keep their unit; `calc()` simplifies at parse time and
  `LengthPercentage::resolve` finishes it; `Color::resolve` gives `Rgba`.
- **Media:** `MediaList::matches(&MediaEnvironment::screen(width, height))`.

## `web-css` selectors for the matcher

`SelectorParser::parse_list` turns a prelude into a `SelectorList` of `Selector`s. Each selector's
`components` run in matching order: the subject compound first, then `Combinator(c)` and the
compound to its left, and so on. Inside a compound every component must match.
- Names are `Atom`s from the document's `AtomTable`. `LocalName` carries the name as written and
  its ASCII lowercase (HTML elements in HTML documents compare `lower`); attributes likewise.
- `Namespace(Any)` alone stands for `*`. `Namespace(Url)` or `Namespace(None)` precede the local
  name when a namespace applies.
- `Is`, `Where`, `Not` hold lists (`Arc`-shared); `&` in nested rules is already replaced by
  `Is(parent list)` (or `:scope` at the top level), and a nested rule without `&` gets
  `Is(parent) <descendant>` prepended, so nested rules flatten as they are.
- `Has` holds relative selectors whose last components are `Combinator(c), RelativeAnchor`; the anchor matches only the `:has` element.
- `Nth` has `a`, `b` and an optional `of` list; `NthSelector::matches_index` takes the 1-based index.
- `specificity` is packed `a << 20 | b << 10 | c`; `pseudo_element` is the trailing pseudo-element
  (`WebkitUnknown` never matches); `ancestor_atoms` are up to four ids, classes and lowercase
  local names from ancestor compounds, for the bloom filter.
