# AI Agent Rules

The workspace `../CLAUDE.md` holds the rules shared by every repository. Never enable Smithay's `xwayland`/`xwm` features.

## Tests
- **Few, purposeful tests only**: tests are end-to-end scenarios of what a user does and sees (real compositor headless, real test bus, real apps), or a regression test for a real bug or security boundary that fails without its fix. At most a couple per app or feature; the whole repo stays around 1,500 lines of tests. No unit tests of wiring or helpers (only genuinely tricky pure logic such as parsers or date maths), no tests that mirror the implementation, no mocks-only checks. **No screenshot/golden tests**: never add or regenerate image comparisons; assert behaviour (e.g. through the accessibility tree) instead.

## Apps
- Every app (`crates/apps/*` except the file chooser portal and the keyring prompt, and every app in `bande-linux/apps`) is a `bande-app` app: `bande_app::run` in `src/main.rs`, windows through `Cx::open`. Windows, menus, standard keys, appearance, Settings… and single instance come from `bande-app`, never from the app; `ci/check-apps.sh` checks it. See `docs/architecture/apps.md`.

## Factory Verification
- **Build first, test once**: each change only needs to compile (`desktop-check <worktree> -p <crate>`: fmt and `cargo check`). Anything else heavy goes through the build queue: `build-run <worktree> cargo ...`, on whichever build machine is free (`build-run --here` to stay on this machine, for a benchmark or a binary you run next). The full gate (`ci/release/verify.sh gate`, the same release checks `release.yml` runs: fmt, clippy `-D warnings`, the layering rules, the tests on the hermetic bus, the debs with check-debs, smoke and lintian) runs once per merge-train batch; see `docs/architecture/ci.md`.

## Workflow
- Trunk-based: `main` always builds. Each milestone is a short branch from `main`.
- A finished branch has `origin/main` merged in, compiles, and is pushed and queued with `desktop-enqueue <branch>`; the merge train gates the batch and pushes `main`. GitHub CI (`release.yml`) runs on version tags (`vX.Y.Z`) and on dispatch; a green run for a tag on `main` publishes to the APT repository. A release is: bump `[workspace.package] version`, merge to `main` through the train, tag.
- Then merge `main` into the next branch and push only that branch; never push several stacked branches at once.
