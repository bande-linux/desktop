# Code quality, testing, security

The web repo gets a `CLAUDE.md` with Bande's rules (no comments, no panics, YAGNI, small current
docs, one-line commits, trunk-based, never force-push), plus these.

## Boundaries

- One responsibility per crate, as in `engine.md`, with an acyclic dependency graph checked by a
  lint script.
- `web-dom` knows nothing of layout. Layout reads the DOM and style; paint reads fragments.
- Processes share only `web-ipc` message types.

## `unsafe`

- Every crate is `#![forbid(unsafe_code)]` except four audited modules:
  - `web-av1`, the only path to rav1d, whose Rust crate exposes nothing but dav1d's C ABI. It
    decodes one AVIF still behind a safe `decode(obus, max_pixels, colour)`; the container is
    parsed by safe code in `web-image`, the same size caps apply, and `web-image`'s `decode` fuzz
    target reaches it. rav1d 1.1.0 still panics inside `dav1d_send_data` on some malformed
    streams (an `unwrap` in `rav1d_submit_frame`'s error path), and a panic cannot unwind out of
    a C ABI call, so it aborts the process. AVIF is therefore behind `web-image`'s `avif`
    feature, which only the sandboxed renderer process turns on: an abort crashes that one tab;
  - `web-ipc`: mapping sealed memfds and `unshare` (fd passing, seccomp and Landlock go through
    rustix, seccompiler and landlock's safe APIs);
  - the GC and JIT code in `web-js`;
  - SIMD scanning in `web-html` (or `std::simd` once stable, and then none).
- Each holds its invariants in `docs/unsafe.md` and runs under Miri where Miri can run it.
- Dependencies' `unsafe` is reviewed when they are added (`cargo geiger` counts in the PR).

## Errors and allocation

- No `unwrap`, `expect` or `panic!`: workspace clippy denies them.
- Parsers are total: malformed input recovers as the specs say and never errors.
- Errors are typed per crate. A renderer failure becomes a "page crashed" sheet, never an app
  crash.
- Arenas and per-pass bump allocation; no `Rc<RefCell<…>>` graphs; ids instead of pointers.

## Testing

Conformance suites are the tests, with one expectations file per suite, pinned to a commit:
- **WPT:** testharness tests, plus reftests that compare two of our own renders at run time,
  with no stored images.
- **html5lib-tests** for the parser, **test262** for JavaScript.

Pass rates are the progress metric; a regression fails the check. Our own tests are few:
- tricky pure logic;
- regressions for real bugs;
- the sandbox boundary: a renderer can't open a file or a socket, or exec; the network process
  writes only its storage, opens no UDP socket, reaches no other Unix socket and can't exec
  (`web-ipc`'s `sandbox` test);
- the process model end to end: the paused-request hook, a killed renderer reported as crashed,
  a reload after it, and loads working again after the network process is killed (`web-host`'s
  `processes` test, which re-runs its own binary as the helpers).

The app has one end-to-end test (about 40 lines, within the apps guide of 80–100 lines per app). It opens a local page on the headless
compositor and finds its heading and a link over AT-SPI.

## Fuzzing and security

- **Fuzzing:** cargo-fuzz targets for the HTML tokenizer and tree builder, the XML parser, the CSS
  parser, the selector matcher, HTTP response handling, cookies, image decoding through our cache,
  WOFF and WOFF2 font decoding, and every IPC message. They run on the mini between merges and
  aren't counted as tests.
- **Advisories and upstreams:** `cargo audit` and `cargo deny` (licences, advisories, duplicate
  crates) on every merge. An agent watches RustSec and the dependencies' releases, and WPT and
  the specs for changes.
- **Review:** security-relevant changes (Fetch, CORS, origins, sandbox, IPC validation, the
  password filler) need a second independent review and the owner's approval before release, as
  the note asks.

## CI and verification

- Builds and checks run on the mini. Heavy builds go under `gentle` with the 6 GB cap; Rust-only
  dependencies keep that within reach.
- GitHub CI runs only for release tags (`web-vX.Y.Z` in the web repo, `browser-vX.Y.Z` here) and
  on dispatch.
- The WPT, html5lib-tests and test262 runs and the fuzzers run on the mini; the tag CI also runs
  the conformance runners, checking the parser suites against the plan's baseline table.
