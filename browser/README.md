# Bande web engine

A web engine written from scratch in Rust for the Bande Browser app: HTML, CSS, layout, DOM, rendering through Bande's own wgpu/vello stack, a sandboxed multi-process model, and JavaScript. The goal is full compatibility with today's web while staying far leaner than existing engines.

The plan and milestones are in `NOTES/plans/browser.md` (the owner's brief is `NOTES/browser.md`). The Browser app lives in `bande-linux/apps`; this repository is the engine it embeds.

## Tools

Builds go through the build queue, which picks a free build machine: `build-run <worktree> cargo ...` (`build-run --here` to stay on the mini, for a benchmark or a binary you run next).

- **Conformance:** `cargo run --release -p web-conformance -- [SUITE...]` runs html5lib-tests
  (`html5lib-tokenizer`, and `html5lib-tree` over WPT's `html/syntax/parsing` `.dat` files),
  css-parsing-tests (`css-parsing`) and the WPT reftest subset (`wpt-reftests`, two renders
  compared at run time; `.xht` and `.xhtml` files parse as XML). Suites are fetched at the commits pinned in
  `tools/conformance/src/source.rs` into `~/.cache/bande-web/suites` (or `$WEB_SUITES`), never
  into the repo. It prints a pass rate per suite; `--failures` lists failing cases,
  `--summary NOTES/plans/browser/roadmap.md` rewrites the progress table there, and
  `--baseline FILE` exits 1 when a suite passes fewer cases than that table records.
- **Fuzzing:** each parser crate has a cargo-fuzz project in `crates/<crate>/fuzz` with its own
  `[workspace]` table. `tools/fuzz MINUTES` builds them all through the build queue, then
  shares MINUTES across the targets; corpora and crash inputs stay in `~/.cache/bande-web/fuzz`.
  Needs `cargo install cargo-fuzz`; it runs on the stable toolchain with `--sanitizer none`.
- **Benchmarks:** `cargo run --release -p web-bench -- [FILE.html|FILE.css...]` prints HTML
  parse and parse-style-layout throughput, bytes per element for the DOM, styles, boxes,
  fragments and display list (and the process's anonymous memory growth), and CSS parse
  throughput. Without arguments it uses a generated article and stylesheet.
- **Processes:** `cargo build --release -p web-renderer -p web-netd` and then `cargo run --release
  -p web-host --example load -- URL [OUT.png]` load a page through the network process and a
  sandboxed renderer, print each frame, then each helper's memory and wake-ups over 10 idle
  seconds, and rasterise the last display list.
- **CI:** `.github/workflows/ci.yml` runs fmt, clippy and the conformance runners on arm64 and
  amd64, only for `web-vX.Y.Z` tags and on dispatch.

Licensed under either of Apache-2.0 or MIT, at your option.
