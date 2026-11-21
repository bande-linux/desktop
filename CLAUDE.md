# AI Agent Rules

The workspace `../CLAUDE.md` holds the rules shared by every repository. Never enable Smithay's `xwayland`/`xwm` features.

## Tests
- **Few, purposeful tests only**: tests are end-to-end scenarios of what a user does and sees (real compositor headless, real test bus, real apps), or a regression test for a real bug or security boundary that fails without its fix. At most a couple per app or feature; the desktop stays around 1,500 lines of tests and each app around 80–100 (a guide, not a hard cap). `docs/quality/tests.md` lists every test with its reason; update it with the tests. No unit tests of wiring or helpers (only genuinely tricky pure logic such as parsers or date maths), no tests that mirror the implementation, no mocks-only checks. **No screenshot/golden tests**: never add or regenerate image comparisons; assert behaviour (e.g. through the accessibility tree) instead.

## Apps
- Every app (`crates/apps/*` except the file chooser portal and the keyring prompt) is a `bande-app` app: `bande_app::run` in `src/main.rs`, windows through `Cx::open`. Windows, menus, standard keys, appearance, Settings… and single instance come from `bande-app`, never from the app; `ci/check-apps.sh` checks it. See `docs/architecture/apps.md`. Start a new app with `ci/new-app.sh`; code two apps share is a crate in `crates/libs`.
- Strings are English keys (`tr!("Open File…")`, `trn!`, `text!`), translated in each crate's `locales/<lang>.po`: `docs/architecture/localisation.md`. App shortcuts use Ctrl (`Ctrl+N`, `Shift+Ctrl+N`); Super is reserved for the system.

## Browser (browser/)
Bande's Browser is a patch queue on Debian's `firefox-esr` source: `browser/` holds no Firefox tree, only our patches, branding, prefs, theme, the scripts that build Debian's source with them and the `bande-firefox-esr-settings` source (`firefox-esr-bande/`). It is not part of the Cargo workspace or the desktop's source package, and builds on its own rhythm (`browser-follow-debian.yml`). Removing something means it can't send anything or be switched back on, not merely that a pref is off.
- A fresh profile makes no unsolicited connection, and there are never ads or sponsored content. `scripts/capture.sh` proves it and blocks a release.
- Every source change is a patch in `patches/<esr>/`, listed in `series`, applied after Debian's with no fuzz. Keep patches small and to one purpose, so a new Debian upload applies without conflict. Deleting the many inert Glean and Telemetry calls is out of scope for that reason: remove what sends, not every caller.
- Debian's packaging and LibreWolf's patches before our own. Patches from LibreWolf stay in `patches/<esr>/librewolf/`, close to their original.
- Security updates come first: a new Debian upload must reach users fast. Never hold one for a feature.
- Patches to MPL files are MPL 2.0, keeping their headers. Never ship Firefox's name or logo as ours; the branding is `branding/`, with "based on Mozilla Firefox source code" kept. `Firefox` stays in the User-Agent for site compatibility.
- `cairo-gtk3-wayland-only`; GTK is a dependency we don't extend. Prefer portals over GTK where a seam exists.
- Behaviour and looks follow `docs/design/datum.md`. Theme CSS stays in `theme/` and is small enough to throw away.
- Scripts are event-driven where anything waits.
- Build with `scripts/local-build.sh` (see `browser/README.md`). Use a temporary `HOME` for anything that runs the browser.

## Factory Verification
- **Build first, test once**: each change only needs to compile (`desktop-check <worktree> -p <crate>`: fmt and `cargo check`). Anything else heavy goes through the build queue: `build-run <worktree> cargo ...`, on whichever build machine is free (`build-run --here` to stay on this machine, for a benchmark or a binary you run next). The full gate (`ci/release/verify.sh gate`, the same release checks `release.yml` runs: fmt, clippy `-D warnings`, the layering rules, the tests on the hermetic bus, the debs with check-debs, smoke and lintian) runs once per merge-train batch; see `docs/architecture/ci.md`.

## Workflow
- Trunk-based: `main` always builds. Each milestone is a short branch from `main`.
- A finished branch has `origin/main` merged in, compiles, and is pushed and queued with `desktop-enqueue <branch>`; the merge train gates the batch and pushes `main`. GitHub CI (`release.yml`) runs on version tags (`vX.Y.Z`) and on dispatch; a green run for a tag on `main` publishes to the APT repository. A release is: bump `[workspace.package] version` (one version for every package, the apps included), merge to `main` through the train, tag.
- Then merge `main` into the next branch and push only that branch; never push several stacked branches at once.
