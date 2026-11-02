# Engine architecture

## Crates in `bande-linux/web`

One Cargo workspace with one lock. Dependencies run only downward in this list.

| Crate | Owns |
|---|---|
| `web-atoms` | interned strings: static atoms generated at build time, per-document dynamic tables, no global mutex |
| `web-html` | tokenizer, tree builder, serialiser, preload scanner |
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
| `web-compositor` | in the app process: layers, async scroll and animation, drawing into the toolkit `Scene` |
| `web-content`, `web-netd`, `web-mediad`, `web-cdmd`, `web-gpu` | the helper processes' `main`s |
| `tools/wpt`, `fuzz/` | the conformance runner and the fuzz targets |

## Processes

- **The app process** (`bande-browser`, a bande-app app) runs the UI, the compositor and the GPU
  device. It holds no page data beyond display lists and accessibility trees, which it validates
  as untrusted.
- **The network process** does all sockets, TLS, cookies, cache and the webRequest hook.
- **One renderer per site** (scheme plus registrable domain; cross-site iframes get their own)
  runs HTML, style, layout, paint and script. It has no files (landlock: read-only fonts and its
  own executable), no sockets (seccomp), no GPU, no Wayland, `PR_SET_NO_NEW_PRIVS`, and a memory
  cgroup at its tab budget.
- **The media process** (phase 4) demuxes and decodes into NV12 dmabufs (`media.md`).
- **The CDM process** (phase 4, per site) hosts Widevine once licensed.
- **The extension process** (phase 4, one per extension) runs the extension's pages.
- **The GPU process** (phase 4) runs WebGL and WebGPU through wgpu and hands frames over as
  dmabuf, so untrusted shaders never run in the app.

IPC uses Unix sockets with fd passing and fixed-layout messages read without copying. Large data
(images, glyph outlines, display lists) travels in sealed memfds. Every message is validated by
its receiver.

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
elements) and "Cat" (13.4k), without author styles, at 1000 px:

| Stage | Budget per element | Now |
|---|---|---|
| Node core (`web-dom`) | ≤ 64 B | 36 B |
| DOM with attributes and text | ≤ 350 B | 236–321 B |
| Computed styles (shared, 4 B index per node) | ≤ 16 B | 10 B |
| Box tree with shaped text | ≤ 520 B | 408–507 B |
| Fragment tree | ≤ 380 B | 283–367 B |
| Display list | ≤ 340 B | 225–328 B |

Shaped glyphs (28 B each) dominate the box tree; moving font and run to per-run spans (20 B) and dropping the
per-paragraph text copy are the next savings.

## `web-css` selectors for the matcher

`SelectorParser::parse_list` turns a prelude into a `SelectorList` of `Selector`s. Each selector's
`components` run in matching order: the subject compound first, then `Combinator(c)` and the
compound to its left, and so on. Inside a compound every component must match.
- Names are `Atom`s from the document's `AtomTable`. `LocalName` carries the name as written and
  its ASCII lowercase (HTML elements in HTML documents compare `lower`); attributes likewise.
- `Namespace(Any)` alone stands for `*`. `Namespace(Url)` or `Namespace(None)` precede the local
  name when a namespace applies.
- `Is`, `Where`, `Not` hold lists; `&` in nested rules is already replaced by `Is(parent list)`
  (or `:scope` at the top level). `Has` holds relative selectors whose last components are
  `Combinator(c), RelativeAnchor`; the anchor matches only the `:has` element.
- `Nth` has `a`, `b` and an optional `of` list; `NthSelector::matches_index` takes the 1-based index.
- `specificity` is packed `a << 20 | b << 10 | c`; `pseudo_element` is the trailing pseudo-element
  (`WebkitUnknown` never matches); `ancestor_atoms` are up to four ids, classes and lowercase
  local names from ancestor compounds, for the bloom filter.
