# AI Agent Rules

**Our own engine, written from scratch**: the HTML parser, CSS, style system, layout, DOM, display list, scheduler, process model, networking policy and JS bindings are written here, in modern Rust. Never fork or vendor another engine (Blitz, Servo, Stylo, WebKit, Gecko, Ladybird). Plain dependencies are allowed only for commodity, security-sensitive building blocks listed in the plan (TLS, compression, media codecs, font parsing and shaping, wgpu and vello); adding a new one needs a written reason in the plan.

## Core Directives
1. **Never write code comments**: Zero inline, block, or trailing comments. No AI watermarks or commented-out code. Make code self-documenting through clear naming and types.
2. **Few, purposeful tests only**: conformance is measured with the Web Platform Tests and reftests run outside the repo's test count. Repo tests are end-to-end scenarios or regression tests for a real bug or security boundary that fail without the fix, plus fuzz targets for every parser. No unit tests of wiring or helpers (only genuinely tricky pure logic), no tests that mirror the implementation, no mocks-only checks, no stored screenshot or golden images.
3. **Reuse code first**: Search the workspace before writing any helper, type, or utility. Never duplicate existing abstractions.
4. **Minimal surface area (YAGNI)**: Zero speculative code, unused builder patterns, or premature indirection. Solve problems with the fewest moving parts. Prefer standard library combinators and idiomatic composition over custom machinery.
5. **Purge code rot**: Delete unused imports, dead functions, unreferenced types, and stale tests in any file you touch. Leave code cleaner than you found it.
6. **Small, current docs and notes**: Update the docs in the same change as the code they describe. Delete a plan or finding once it is finished (git keeps the history). No dated logs or measurement histories in the repo.

## Efficiency
- **Hyper-efficient by design**: compact data layouts (arena DOM, interned atoms, shared styles), incremental style and layout, no work for things that are not on screen. Every subsystem has a memory and time budget in the plan; a change that breaks a budget is a bug.
- **Event-driven, zero idle wake-ups**: no polling loops; an idle page with nothing animating costs nothing.
- **`unsafe`**: none outside the few audited modules the plan names; each such block is the smallest possible and justified by its safe wrapper's types.

## Factory Verification
- **No panics**: No `.unwrap()` or `.expect()` in production code. Untrusted input (HTML, CSS, JS, images, fonts, network data) never panics or allocates without a bound.
- **Build first, test once**: each change only needs to compile (fmt and `cargo check`, through the build queue: `build-run <worktree> cargo ...`, on whichever build machine is free; `build-run --here` to stay on the mini, for a benchmark or a binary you run next). The full gate (fmt, workspace clippy `-D warnings`, tests) runs once on `main` when a batch of work has merged, and `main` fixes whatever it finds then.

## Workflow
- Trunk-based: `main` always builds. Each milestone is a short branch from `main`, merged when it compiles and its gate passes. Never force-push.
- GitHub CI runs only on version tags and on dispatch.
- Commit messages: one brief line saying what the commit does. No body, no trailers (no `Co-Authored-By`, no session links), nothing special.
