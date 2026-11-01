# Moving Terminal, Activity Monitor, Software and Package Installer to bande-linux/apps

Approved by the owner on 2026-11-01. Files, Settings and the file chooser stay in desktop: the file chooser depends on bande-files, and Files is the system file manager. Apps consume desktop crates as git dependencies pinned to one desktop rev.

## Findings

### Dependencies of the moving crates
All are shared and stay in desktop unless noted.
- terminal (package bande-term): app-menu, app-settings, design, l10n, log, notify (client API), spring, toolkit, xdg; alacritty_terminal; dev: golden.
- monitor (bande-monitor): app-menu, l10n, log, status, toolkit, xdg; rustix; dev: golden, test-bus.
- software (bande-software): **catalog (only Software uses it, so it moves)**, inbox, l10n, log, packages, toolkit, xdg; dev: golden, test-bus (FakePackageKit).
- installer (bande-installer): fs, l10n, log, packages, toolkit, xdg; dev: golden, test-bus (FakePackageKit).
- catalog: search, xdg; flate2, roxmltree, serde_yaml_ng.
- bande-packages and the FakePackageKit in bande-test-bus stay: Settings (Software Update) and packages' own tests use them.
- Workspace dependencies only the moving crates use: alacritty_terminal, serde_yaml_ng. Remove them with the five moving crates' workspace entries.
- Past paths in history: crates/apps/{terminal,monitor,software,installer}, crates/libs/catalog, crates/unbound-term (Terminal's earlier home).
- No desktop crate depends on the four apps.

### Desktop code and CI that reference the moving apps (must change)
- **Force Quit:** the compositor (state/mod.rs:316 `Summoned::new(MONITOR)`, session.rs:33, data/session-clients) and the bar (menu_bar_client.rs:185) both launch `bande-monitor --force-quit`. The bar's low-memory dialog (low_memory.rs:26) launches `bande-monitor --memory`.
- xdg launch.rs:27 `TERMINAL = "bande-term"`, used by Files' "Open in Terminal" and by `Terminal=true` launches.
- Compositor Cargo.toml:48 Recommends includes bande-term.
- packaging/bande-desktop/debian/control Recommends lists bande-term, bande-monitor, bande-installer pinned `(= ${binary:Version})`; the store-remove work adds bande-software.
- bande-mimeapps.list maps `application/vnd.debian.binary-package` to org.bande.Installer. That default stays in desktop.
- ci/release/packages, check-debs.sh (moved-package checks, compositor-Recommends check), build-all.sh (`bande-monitor (= v)`, `bande-installer (= v)`), ci/deb/install-test/run.sh ("every Recommends is in Debian testing").
- ci/review/run.sh package list and the terminal, monitor-depth and apps scenarios; ci/vm/guest-tests.sh with compositor tests/guest/desktop.rs (types into bande-term); bench bande-session-bench (launches bande-term from target/).
- Tests that read files in the moving folders:
  - shell/bar tests/low_memory.rs reads `apps/monitor/data/*.svg` and `apps/terminal/data/*.svg`.
  - shell/desktop tests/icons.rs counts `.desktop` files under crates/apps and asserts 6.
  - monitor tests/view.rs reads `../../../data/icons/Bande`.
  - the installer test reads `../../../packaging/bande-desktop/bande-mimeapps.list`.
- data/icons/src/apps.py HICOLOR writes the hicolor SVGs into crates/apps/{terminal,monitor,installer}/data. Theme icons in data/icons/Bande/scalable/apps stay in desktop: desktop owns the look of every Bande icon.
- Test data only (no change): config, launcher, bar taskbar and rail, status, Settings goldens, compositor headless tests (app id strings).
- Docs: README.md, docs/architecture/{packaging,behaviour,performance}.md, docs/design/catalogue.md, docs/quality/userland.md, docs/roadmap.md, docs/decisions/0004.
- ~/bande-linux/bin/rebuild-desktop reads the control file's Depends block as the "pinned" list; rebuild-apps defaults to crates/apps.

### Apps repo today
- apps/mail and apps/calendar each have their own Cargo.lock; desktop crates are git dependencies at rev 3ff5f3e7 (about 140 commits behind desktop main).
- ci/build-app.sh requires the package in `apps/<dir>` to be named `bande-<dir>`, reads `cargo metadata --no-deps .packages[0]`, runs fmt, clippy `-D warnings` and tests, builds the deb, installs it with apt in the container and runs smoke.sh.
- `.github/workflows/build.yml` runs on `<app>-vX.Y.Z` tags and uploads `bande-debs-{arm64,amd64}`. apt's publish.yml takes each app's green tag run every 30 minutes, accepts only `bande-<app>` at exactly X.Y.Z, and refuses names the desktop owns. So the release pipeline into the archive already exists.
- No CLAUDE.md and no local gate script.

### apt and images
- **Live archive (checked 2026-11-01): `state/owners` lists only apt and kernel.** No desktop or app has published yet. The owners file locks a package name to the first source that publishes it, so the move must land before the first desktop `vX.Y.Z` tag. Otherwise apt needs a transfer list (e.g. conf/transfers, honoured in rebuild.sh when it builds `claims`) and the apps' versions must be higher than the desktop's.
- publish.yml's verify step installs bande-desktop with `--no-install-recommends`, so apps-repo packages don't trip its "every bande-* at the desktop version" check.
- images (prep/os-installable): fetch-desktop.sh downloads only packages owned by desktop or apt and requires each to equal the bande-desktop version; customize.sh runs `apt-get install --no-install-recommends /tmp/desktop/*.deb`. **Unless fetch-desktop.sh changes, the moved apps silently disappear from images.**

### Gates
- desktop: CLAUDE.md requires fmt, workspace clippy `-D warnings` and workspace tests on the hermetic bus (~/bande-linux/bin/desktop-gate) before merging to main. CI runs only on tags.
- apps: the per-app gate exists only inside build-app.sh; the README says CI runs only for releases.

## The default-apps file

Location: `packaging/bande-desktop/default-apps`, next to bande-mimeapps.list, inside the metapackage's source folder so dpkg-buildpackage stays self-contained. Line-based so awk and a short Rust test can read it.

It lists only the swappable apps: those from the apps repo and from Debian. Everything built in the desktop repo (Files, Settings, the file chooser and the shell components) is part of the desktop, always installed, and stays hand-written in the control file as today; it is not selectable, so it is not in this list (owner, 2026-11-01).

```
# The default apps a standard bande-desktop install gets. All are swappable: bande-desktop
# Recommends (or Suggests) them unversioned, so a user can remove or replace any of them.
# Desktop-repo packages are not listed: they are part of the desktop (see debian/control).
# [apps]    built in bande-linux/apps, released on their own.
# [debian]  from Debian.
# Columns: package, the icon the Bande theme must draw for it (- for none), recommends|suggests.
# Read by debian/rules (substvars), ci/release/build-all.sh, ci/deb/install-test/run.sh,
# crates/shell/desktop/tests/icons.rs and images fetch-desktop.sh.

[apps]
bande-terminal       org.bande.Terminal         recommends
bande-monitor        org.bande.ActivityMonitor  recommends
bande-software       org.bande.Software         recommends
bande-installer      org.bande.Installer        recommends
bande-mail           org.bande.Mail             recommends
bande-calendar       org.bande.Calendar         recommends

[debian]
firefox-esr          -                          recommends
gnome-text-editor    org.gnome.TextEditor       recommends
loupe                org.gnome.Loupe            recommends
papers               org.gnome.Papers           recommends
showtime             org.gnome.Showtime         recommends
gnome-calculator     org.gnome.Calculator       recommends
gnome-clocks         org.gnome.clocks           suggests
```

Mail and Calendar are added once each has a published release (D3), with their theme icons drawn first. Until the four apps have moved (M5), they stay in the control file as today and are not in this list. Non-app Recommends (fonts, cups, keyring, oomd and so on) stay hand-written. Since 2026-11-01 bande-desktop is the only metapackage (single-metapackage.md): its hand-written relations also carry the system around the desktop (Depends `bande-greeter (= ${binary:Version}) | x-display-manager`, `pipewire-audio`; Recommends `bande-greeter (= ${binary:Version})`, `network-manager`, `bande-apt-source`), and the default apps stay in its Recommends.

### Generation at build time
- `packaging/bande-desktop/debian/default-apps.sh <version>` (awk, about 20 lines) prints `bande:Recommends=...` and `bande:Suggests=...` and rejects unknown sections or relations and duplicate names.
- debian/rules: `override_dh_gencontrol: debian/default-apps.sh $(DEB_VERSION_OVERRIDE) >> debian/bande-desktop.substvars; dh_gencontrol -- -v$(DEB_VERSION_OVERRIDE)`. Writing the literal version avoids recursive substvar expansion.
- control: Depends unchanged (hand-written desktop packages); `Recommends: ${bande:Recommends}, bande-screenshot (= ${binary:Version}), ..., fonts...`; `Suggests: ${bande:Suggests}`.
- build-all.sh: each listed package appears unversioned in Recommends/Suggests, and no desktop-repo package (ci/release/packages) is listed.
- install-test/run.sh: add the `[apps]` names to `from_the_archive` in the "recommended and in Debian testing or built here" loop, next to `bande-apt-source`.
- apt publish.yml verify: after installing, check every Recommends of bande-desktop has a candidate from Debian or the archive (warning at first).
- rebuild-desktop: unchanged in principle; `pinned` is the `(=` pins in control.
- images fetch-desktop.sh: also read bande-desktop's `Recommends:` from Packages.xz; also fetch packages owned by `apps` that bande-desktop recommends, without the version-equality check; fail if one is missing unless `ALLOW_MISSING_APPS=1`.

## Milestones (each 1-2 h, each ending with both repos green)

### M0. Preconditions (no code)
Land or park everything that touches the moving folders, the metapackage or the shared files edited in M1-M2:
- fix/super-keys (every app), the store-remove work in wt/desktop-gaps (software, metainfo in terminal/monitor/files/settings, control), origin/wip/terminal-scopes, fix/zram-device-timeout (bande-desktop.install), fix/taskbar-launch (compositor session.rs; before M3).

Settings keyboard, Wi-Fi UI and compositor DRM work don't conflict. Then: freeze crates/apps/{terminal,monitor,software,installer} and crates/libs/catalog from M4 until M5 merges (about 4-6 h); re-check the live `state/owners`; `apt install git-filter-repo`. Late patches can be carried over: `git format-patch --relative=crates/apps/terminal main..branch`, then in apps `git am --directory=apps/terminal`.

### M1 (desktop). Default-apps and the generated control file
- Add default-apps with `[debian]` filled in (and `[apps]` empty until M5); add default-apps.sh and the rules/control substvars.
- Rewrite the build-all.sh checks and the install-test skip.
- shell/desktop tests/icons.rs takes names from default-apps instead of counting crates/apps.
- Move the mimeapps assertion out of the installer test into a desktop test: every `org.bande.*` in bande-mimeapps.list names an app in default-apps (bande-xdg mimeapps parser).
- Update rebuild-desktop.
- Done when `dpkg-deb -f bande-desktop Depends Recommends Suggests` is unchanged, and build-all.sh and the gate are green.

### M2 (desktop). Cut the code and test couplings
- bar tests/low_memory.rs loads icons from `data/icons/Bande/scalable/apps/` (same bytes; goldens unchanged).
- Compositor guest test, ci/vm/guest-tests.sh and session-bench type into bande-sample (crates/testing, records typed text to a file) instead of bande-term.
- ci/review/run.sh builds only desktop packages; scenarios needing a moved app check /usr/bin and say so.
- data/icons/src/apps.py HICOLOR keeps only Files and Settings; theme icons for Terminal, ActivityMonitor, Installer and Software stay.
- Missing-app behaviour: Files hides "Open in Terminal" without `bande-terminal` on PATH; `Terminal=true` launches return a clear LaunchError; the low-memory dialog hides "Open Activity Monitor" when it's missing.

### M3 (desktop). Move Force Quit into the shell (D2)
- `bande-shell --force-quit` on bande-status::memory (`force_quit`, `heaviest_apps`), with the look of monitor's force_quit.rs.
- Compositor `Summoned::new(SHELL)` for Force Quit; the bar asks the compositor (or runs bande-shell).
- Drop `/usr/bin/bande-monitor` from session-clients and the MONITOR constant if unused. Monitor keeps its in-app Force Quit.
- If D2 is declined: skip this, and when bande-monitor is missing the compositor quits the focused app after confirmation.

### M4a / M4b (apps). Import with history and build standalone
M4a: Terminal and Monitor. M4b: Software (with catalog) and Installer.
1. In a scratch clone of desktop: `git filter-repo --path crates/apps/terminal/ --path crates/unbound-term/ --path crates/apps/monitor/ --path crates/apps/software/ --path crates/apps/installer/ --path crates/libs/catalog/ --path-rename crates/apps/terminal/:apps/terminal/ --path-rename crates/unbound-term/:apps/terminal/ --path-rename crates/apps/monitor/:apps/monitor/ --path-rename crates/apps/software/:apps/software/ --path-rename crates/apps/installer/:apps/installer/ --path-rename crates/libs/catalog/:apps/software/catalog-history/` (first check unbound-term and apps/terminal never coexisted).
2. In apps: `git fetch <scratch> main && git merge --allow-unrelated-histories`.
3. Standalone Cargo.toml per app, following mail: explicit version 0.1.0, edition, license, `publish = false`; third-party deps at the desktop workspace's exact versions; desktop crates as `{ git = "https://github.com/bande-linux/desktop", rev = "<desktop main after M3>" }`; lints and profiles from mail; keep metadata.deb and assets.
4. Fold catalog into `apps/software/src/catalog/` (D5); delete catalog-history.
5. Each Cargo.lock starts as a copy of desktop's, pruned by `cargo metadata` (D10).
6. build-depends from desktop's ci/release/build-depends as needed, plus fonts; smoke.sh (`--version`, .desktop, icon).
7. Fixtures: copy the theme SVGs monitor's view test needs into `tests/fixture/icons/`; drop the installer's mimeapps test (moved in M1).
8. Run each app's gate; switch any ambient-session-bus test to bande_test_bus::SessionBus.
9. Check goldens against the apps CI's fonts; port ci/debian-fonts.sh into build-app.sh if they differ.
10. CI dispatch `build.yml -f apps="term monitor software installer"` is a release-time step; locally the gate is enough. Desktop is unchanged, so both repos stay green.

### M5 (desktop). Remove the moved crates
- Delete crates/apps/{terminal,monitor,software,installer} and crates/libs/catalog; remove their workspace entries plus alacritty_terminal and serde_yaml_ng.
- `cargo check` once without `--locked` to prune Cargo.lock (removals only).
- Update ci/release/packages and check-debs.sh, compositor recommends (drop bande-term), default-apps (the four are added to `[apps]`, their `(=)` Recommends removed from control), README, docs, review run.sh.
- Gate and build-all.sh.

### M6 (apps and tooling). Pins, gate and rules
- `ci/bump-desktop.sh [REV]`: default REV is desktop main (`git ls-remote`), later the newest `v*` tag (D8); checks REV is on main; sed across apps/*/Cargo.toml and template; `cargo metadata` per app to re-lock only the desktop source; runs `ci/gate.sh`; prints "Pin the desktop at <short>".
- `ci/gate.sh [app...]`: fmt, clippy `--locked --all-targets -D warnings`, tests `--locked`, shared `CARGO_TARGET_DIR`, under gentle and the kbuild lock like desktop-gate.
- `ci/with-desktop.sh <desktop checkout> <cargo args>`: `--config 'patch."https://github.com/bande-linux/desktop".<crate>.path="…"'` per crate, to test a toolkit branch before it merges; restore Cargo.lock afterwards.
- apps CLAUDE.md: desktop's core directives; the gate is ci/gate.sh for the apps touched; desktop crates only at the pinned rev; toolkit changes land in desktop first; one shared pin (D7).
- Bump Mail and Calendar from 3ff5f3e7 to the shared pin and fix what Super→Ctrl and the renames broke.
- ~/bande-linux/bin/rebuild-apps: apps-repo names build there (`cargo build --release`, `cargo deb --no-build`, `dpkg -i`); desktop crates/apps names go to `rebuild-desktop --apps`.

### M7 (apt and images), then the first releases
- images: the fetch-desktop.sh change above. apt: README table row, the Recommends check in publish.yml verify, a test for an apps package recommended by bande-desktop.
- Release order: tag terminal-v0.1.0, monitor-v0.1.0, software-v0.1.0, installer-v0.1.0 (and mail, calendar) and wait for them to publish; bump `[workspace.package]` and tag desktop v0.1.0; build an image and check the apps are in it.

### What a toolkit change costs afterwards
1. Desktop branch, gate, merge.
2. In apps, `ci/bump-desktop.sh` (a minute of editing, one shared toolkit build plus clippy and tests per app).
3. An app ships the change when it's tagged.

Bump in batches when an app needs a new API, not after every desktop merge. Before merging a desktop change to a public toolkit or lib API, optionally run `ci/with-desktop.sh ~/…/desktop cargo check` on all apps.

## Risks
- Conflicts with in-flight work: M0 sequencing, the freeze, format-patch/am.
- An app the desktop relies on becomes removable: Force Quit (M3), "Open in Terminal" and `Terminal=true` (M2 fallback; later xdg-terminal-exec), the low-memory dialog's Monitor button. The `.deb` → Installer default falls back to apt's own handling if the Installer is missing.
- apt name ownership: if desktop publishes bande-term and the others before M5, the apps repo can't publish them without an owner transfer and higher versions.
- Images silently lose the apps without the M7 fetch change.
- Runtime version skew between apps and desktop services; pinning to released desktop tags (D8) limits it.
- CI cost: each app compiles the toolkit; acceptable since builds run per tag. Locally use the shared target.
- Golden drift from different fonts or GPU setup in the apps CI; check in M4.
- Latent bug: check-debs.sh requires `Section = misc`, but bande-software and bande-installer say `admin`, so a desktop tag could fail on it until the move.

## Decisions (owner, 2026-11-01)
- **D1. Desktop-repo apps:** not in the default-apps list; they are part of the desktop and stay in the control file.
- **D2. Force Quit:** move it into the shell like macOS, but only if it is at least as reliable as today: it must open and kill under memory pressure, with Activity Monitor absent, with the bar or a client hung, and from the keyboard shortcut. M3 includes tests for each of those; if any of them can't match today's behaviour, Force Quit (and so Activity Monitor) stays in the desktop repo.
- **D3. Mail and Calendar as default apps:** yes, in `[apps]` after their first published release, with theme icons drawn.
- **D4. Terminal's package:** rename to `bande-terminal` (package, binary, the xdg TERMINAL constant, bench, review and guest scripts), folder `apps/terminal`. Do it in the move, before anything is published.
- **D5.** Fold bande-catalog into Software as a module.
- **D6.** git filter-repo, one combined import including crates/unbound-term.
- **D7.** One desktop pin shared by all apps, enforced by bump-desktop.sh.
- **D8.** Pin to desktop release tags after the first desktop release; main commits until then.
- **D9.** Apps start at 0.1.0; the first desktop tag waits until M5 has landed.
- **D10.** Each app's Cargo.lock starts from desktop's and is pruned.

## Critical files
- packaging/bande-desktop/debian/control, rules, and the new default-apps and default-apps.sh
- ci/release/build-all.sh, check-debs.sh, packages; ci/deb/install-test/run.sh
- crates/platform/compositor/src/state/mod.rs and session.rs; crates/shell/bar/src/menu_bar_client.rs (Force Quit)
- ~/bande-linux/repos/apps/ci/build-app.sh and apps/apps/mail/Cargo.toml (template for standalone apps)
- ~/bande-linux/repos/images/fetch-desktop.sh (prep/os-installable); ~/bande-linux/bin/rebuild-desktop
