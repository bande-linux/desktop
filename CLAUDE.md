# AI Agent Rules

**Pure Wayland only**: no X11 code, X libraries, X server or XWayland bridge anywhere in the desktop. Do not enable Smithay's `xwayland`/`xwm` features or add X11 dependencies.

## Core Directives
1. **Never write code comments**: Zero inline, block, or trailing comments. No AI watermarks or commented-out code. Make code self-documenting through clear naming and types.
2. **Never add useless tests**: No tautological tests, mock-only verifications, or tests of language/compiler guarantees. Every test must test real domain logic and fail if that logic breaks.
3. **Reuse code first**: Search the workspace before writing any helper, type, or utility. Never duplicate existing abstractions.
4. **Minimal surface area (YAGNI)**: Zero speculative code, unused builder patterns, or premature indirection. Solve problems with the fewest moving parts. Prefer standard library combinators and idiomatic composition over custom machinery.
5. **Purge code rot**: Delete unused imports, dead functions, unreferenced types, and stale tests in any file you touch. Leave code cleaner than you found it.

## Factory Verification
- **No panics**: No `.unwrap()` or `.expect()` in production code. Use proper error handling.
- **Strict gate**: Must pass `cargo check`, `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check` before finishing.

## Workflow
- Trunk-based: `main` is always green. Each milestone is a short branch from `main`.
- Merge a branch into `main` only when the local gate is fully green (fmt, workspace clippy `-D warnings`, workspace tests on the hermetic bus): fast-forward if possible, otherwise a merge commit. Never force-push, never merge red. GitHub CI runs only on version tags (`v*.*.*`) and manual dispatch.
- Then merge `main` into the next branch and push only that branch; never push several stacked branches at once.
- Commit messages: one brief line saying what the commit does. No body, no trailers (no `Co-Authored-By`, no session links), nothing special.
