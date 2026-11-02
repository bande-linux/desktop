# JavaScript

JavaScript is the single biggest item.

| Engine | Size | What it is | Fit |
|---|---|---|---|
| SpiderMonkey (mozjs) | `js/src` 1.2M lines of C++; a 175 MB crate | interpreter, baseline and optimising JITs, WASM | C++ and Firefox's monthly CVE cadence: excluded by "fully Rust" |
| V8 | over 1M lines of C++ | the fastest JITs | Google: excluded |
| Boa 0.22 | 154k + 30k parser, 290 `unsafe` | bytecode interpreter, mark-sweep stop-the-world GC, >90% of test262 | pure Rust, a plain dependency, about 10–50× slower than a JIT on heavy scripts |
| Nova 1.0 | 159k, 1122 `unsafe` | data-oriented heap (one vector per object type, index handles), exact GC with scoped handles, interpreter | elegant, young; a design to learn from |
| Our own | 200–300k to parity | see below | the end state the owner wants |

**What our own costs.**
1. A conformant interpreter (parser, bytecode, objects, built-ins, `Intl` over ICU4X, generational
   GC) at test262 parity with Boa: 150–200k lines, **4–6 engineer-years**.
2. A baseline JIT plus inline caches on arm64: **2–3 engineer-years**.
3. An optimising JIT that reaches JetStream and Speedometer parity: **6–10+ engineer-years**.
   V8's and SpiderMonkey's JIT teams each have dozens of engineers.

Google Docs, Figma and heavy sites need step 3.

**Recommendation.**
- Write `web-script` (WebIDL bindings generator, Web APIs, event loop) engine-neutral from day
  one, behind a narrow interface: realms, objects, functions, promises, GC roots, wrappers.
- Phase 2 ships on Boa as a plain dependency.
- `web-js` starts in phase 3 with a data-oriented heap that the DOM wrappers share (one heap, so
  no cross-heap cycles and no wrapper overhead). It replaces Boa when it passes Boa on test262
  and on the top-sites list.
- The JITs follow in phases 4–5.

Starting with our own engine on day one delays phase 2, the first scripted pages, by about 2–3
years and buys nothing, because the bindings don't change.

## Phase 2 design

**Layout.**
- `web-script` holds the WebIDL files (`webidl/*.webidl`), the generated bindings, the hand-written
  Web API implementations and the event loop.
- The engine sits behind one module, `engine`: realms, values, objects, functions, promises, the
  job queue and native-data wrappers, as concrete types over Boa (no trait generics, so no cost
  per call). `web-js` later replaces that module's insides; bindings and APIs don't change.
- The bindings generator is our own (a WebIDL parser for the subset we use, about 1k lines) and
  runs as `web-script`'s build script. Per interface it emits the prototype, constructor,
  attributes and operations, the argument conversions (DOMString, boolean, the integer types,
  double, nullable, interface types, sequences, callbacks, dictionaries) and a Rust trait that
  the hand-written implementation fills in. Implementations take the realm's `Host` (document,
  wrapper table, event-loop state) and return `Result<_, Exception>`.

**GC integration.**
- DOM nodes live in `web-dom`'s arena and belong to the document. Removed nodes stay allocated
  and ids are never reused while the document lives, so a wrapper holds just a `NodeId`; nothing
  in the DOM points into the JS heap.
- Each realm keeps a wrapper table indexed by `NodeId`, filled only for nodes script touches.
  Entries are strong: a wrapper (with its expandos and identity) lives as long as the document.
  Realm, wrappers and arena drop together on navigation, so nothing leaks past the document.
- JS values held from Rust (event listeners, timer and `requestAnimationFrame` callbacks,
  observers) sit in realm-owned tables, which Boa treats as roots. Cycles through them are
  bounded by the document's lifetime for the same reason.
- With `web-js`, wrappers and JS objects share one heap and these tables become traced fields.

**Running script.**
- Script runs on the renderer's main thread, inside its sandbox. A page with no script never
  creates a realm, so static pages keep their budgets.
- First milestones run parser-inserted classic scripts in document order after parsing (as if
  deferred); `document.write` and parser-blocking scripts come with the streaming parser.
- After each task and its microtasks, DOM changes mark the page dirty; one restyle and layout
  follows, then a display-list patch to the app.
- Timers are deadlines in the renderer's `poll` timeout; nothing wakes when none are due.
  `requestAnimationFrame` asks the app for one frame callback at a time and stops when the tab
  is hidden. Idle pages make zero wake-ups.
- Boa has no interrupt, so a looping script hangs only its own renderer; the host's
  unresponsive-renderer timeout kills it and shows "page unresponsive". Recursion and loop
  limits come from Boa's `RuntimeLimits`.

**Budgets.** A realm with the DOM bindings costs at most 3 MB; a wrapper at most 200 B; a DOM
call from script at most 1 µs on the mini excluding the work it does. Fuzz targets drive
arbitrary script against a small document through every binding.

**Milestones.**
- S1: the generator, the engine module, `window`, `document`, `console`, Node, Element, Text and
  Document basics, scripts run after parsing, mutations relaid out.
- S2: events (EventTarget, capture and bubble, clicks from the app), `querySelector(All)`,
  `classList`, `innerHTML` through `web-html`'s fragment parsing.
- S3: the event loop: tasks, microtasks, timers, `requestAnimationFrame`, zero idle wake-ups.
- S4: a testharness.js runner in `tools/conformance` for WPT `dom` and `html/webappapis`.
