# AI Agent Rules

**Pure Wayland only**: no X11 code, X libraries, X server or XWayland bridge anywhere in the desktop. Do not enable Smithay's `xwayland`/`xwm` features or add X11 dependencies.

## Core Directives
1. **Never write code comments**: Zero inline, block, or trailing comments. No AI watermarks or commented-out code. Make code self-documenting through clear naming and types.
2. **Few, purposeful tests only**: tests are end-to-end scenarios of what a user does and sees (real compositor headless, real test bus, real apps), or a regression test for a real bug or security boundary that fails without its fix. At most a couple per app or feature; the whole repo stays around 1,500 lines of tests. No unit tests of wiring or helpers (only genuinely tricky pure logic such as parsers or date maths), no tests that mirror the implementation, no mocks-only checks. **No screenshot/golden tests**: never add or regenerate image comparisons; assert behaviour (e.g. through the accessibility tree) instead.
3. **Reuse code first**: Search the workspace before writing any helper, type, or utility. Never duplicate existing abstractions.
4. **Minimal surface area (YAGNI)**: Zero speculative code, unused builder patterns, or premature indirection. Solve problems with the fewest moving parts. Prefer standard library combinators and idiomatic composition over custom machinery.
5. **Purge code rot**: Delete unused imports, dead functions, unreferenced types, and stale tests in any file you touch. Leave code cleaner than you found it.

## Apps
- Every app (`crates/apps/*` except the file chooser portal, and every app in `bande-linux/apps`) is a `bande-app` app: `bande_app::run` in `src/main.rs`, windows through `Cx::open`. Windows, menus, standard keys, appearance, Settings… and single instance come from `bande-app`, never from the app; `ci/check-apps.sh` checks it. See `docs/architecture/apps.md`.

## Factory Verification
- **No panics**: No `.unwrap()` or `.expect()` in production code. Use proper error handling.
- **Build first, test once**: each change only needs to compile (`desktop-check <worktree> -p <crate>`: fmt and `cargo check`). The full gate (fmt, workspace clippy `-D warnings`, workspace tests on the hermetic bus) runs once on `main` when a batch of work has merged, and `main` fixes whatever it finds then.

## Workflow
- Trunk-based: `main` always builds. Each milestone is a short branch from `main`.
- A finished branch has `origin/main` merged in, compiles, and is pushed and queued with `desktop-enqueue <branch>`; the merge train compile-checks the batch and pushes `main`. Never force-push. GitHub CI (`release.yml`) runs on version tags (`vX.Y.Z`) and on dispatch; a green run for a tag on `main` publishes to the APT repository. A release is: bump `[workspace.package] version`, merge to `main`, tag.
- Then merge `main` into the next branch and push only that branch; never push several stacked branches at once.
- Commit messages: one brief line saying what the commit does. No body, no trailers (no `Co-Authored-By`, no session links), nothing special.
