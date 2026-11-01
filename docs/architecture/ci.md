# CI, the gate and releases

## The local gate

Day-to-day correctness is checked on the build machine, not in CI. Each change only has to
compile: `desktop-check <worktree> -p <crate>` runs `cargo fmt` and `cargo check --all-targets`.
Finished branches are queued with `desktop-enqueue <branch>`; the merge train compile-checks the
batch and pushes `main`. The full gate, `desktop-gate <worktree>`, runs once on `main` after a
batch has merged: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D
warnings`, and the workspace tests on the hermetic bus with lavapipe:

```bash
dbus-run-session --config-file=crates/testing/test-bus/data/session.conf -- cargo test --workspace --locked
```

These scripts live in `~/bande-linux/bin` on the build machine, outside this repository; the
commands above are what they run. The tests and why they exist are in
[`../quality/tests.md`](../quality/tests.md). A test bus lists no service directories, so it
never D-Bus-activates the services installed on the machine.

## GitHub CI

The only workflow, `release.yml`, runs on version tags (`vX.Y.Z`) and on manual dispatch, never
on pushes or pull requests. It runs on GitHub-hosted runners (`ubuntu-24.04-arm` and
`ubuntu-24.04`), every job in a `debian:testing` container that installs what it needs with
`ci/apt-install.sh` (build libraries in `ci/release/build-depends`) and the toolchain pinned in
`rust-toolchain.toml`. Rendering runs on lavapipe, Mesa's CPU Vulkan, so no job needs a GPU or
a display; the compositor tests run it headless.

### Jobs

`release.yml` runs when a version is tagged and when dispatched by hand for a test build
(`gh workflow run release.yml --ref <branch>`, never published). A newer run on the same ref
cancels the older one, and `deb-all` fails a tag whose commit is not on `main`. The packaging
steps are scripts in `ci/release/`, so the same build and checks run natively on a developer
machine:

```bash
ci/release/build-debs.sh && ci/release/check-debs.sh && ci/release/smoke.sh
ci/release/build-all.sh
```

- `lint` (arm64): `actionlint` with shellcheck over the workflows, then `ci/release/lint.sh`:
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  the dependency rules (no session service except the polkit agent depends on the widget
  crates; graphics and widgets stay off Wayland) and `ci/check-apps.sh`.
- `test` (arm64): the workspace tests on the hermetic bus, as in the local gate.
- `debs (arm64)`, `debs (amd64)`: each on its native runner, `ci/release/build-debs.sh` runs
  `cargo build --profile dist` for the crates in `ci/release/packages` and `cargo deb` for each
  at the version from `ci/deb/version.sh`. cargo-deb takes the architecture from the native
  target and the shared-library dependencies from Debian testing's `dpkg-shlibdeps`.
  `ci/release/check-debs.sh` checks that every package is there for `dpkg --print-architecture`
  at that version with no build paths inside, the D-Bus activation files, the user units, the
  portal backend's registration and the login session; `ci/release/smoke.sh` starts the
  packaged settings service on a private bus to read a property and the packaged compositor
  headless. The job uploads `bande-debs-<arch>`.
- `deb-all`: `ci/release/build-all.sh` builds the architecture-independent meta-package
  `bande-desktop`, checks that it pins its packages at the build's version, that it depends on
  `bande-greeter | x-display-manager` and `pipewire-audio` and recommends `bande-greeter`,
  `network-manager`, `bande-apt-source` and `systemd-zram-generator`, and lints it with
  `lintian`. It uploads
  `bande-debs-all`. The APT source and keyring packages are built by the APT repository
  itself.
- `install (arm64)`, `install (amd64)`: in a clean `debian:testing` on the native runner,
  `ci/deb/install-test/run.sh` installs `bande-desktop` from that architecture's packages and
  the `all` packages alone without recommends and checks that it brings the login window,
  greetd and PipeWire, then installs every package and checks that they have the same version,
  that every package it recommends has an installation candidate (`bande-apt-source` comes from
  the APT repository), that no X server, XWayland or X session came along, that greetd is the
  display manager with the Bande login window, the greeter's wallpaper permissions, and that the
  installed compositor serves Wayland clients. Last, next to LightDM: installing the greeter keeps
  LightDM the default and offers greetd, `dpkg-reconfigure greetd` switches to it, removing
  greetd hands the display back to LightDM.
  `ci/deb/install-test/portal-frontend.sh` then reads the colour scheme through Debian's
  `xdg-desktop-portal` and checks it follows `config.kdl`.

The APT repository publishes on its own: every 30 minutes it looks for new green `release.yml`
runs for tags and takes their `bande-debs-arm64`, `bande-debs-amd64` and
`bande-debs-all` artifacts into the `main` component of its `rolling` suite. It refuses a run
whose architectures do not carry the same packages at the tag's version, and after each deploy
it installs `bande-desktop` from the live site on amd64 and on arm64 and checks that every
package is that architecture's build (or `all`) at the published version
([`bande-linux/apt`](https://github.com/bande-linux/apt)).

Versions come from `ci/deb/version.sh`. A tag `vX.Y.Z` builds version `X.Y.Z` and must equal
the `[workspace.package] version` in `Cargo.toml`. Test builds and local builds get `X.Y.Z+gitDATE.COUNT.sha`, counted from the last
tag, which sorts above that tag and below the next.

Hosted runners start from a clean machine, so `Swatinem/rust-cache` restores the cargo
registry, the installed `cargo-deb` and the target directory, keyed per job and architecture
and saved only from runs on `main`, whose caches every branch and tag can read. The `test` and `debs` jobs first free disk on the host by emptying
the .NET, Android and GHC installations they mount, and `debs` builds two crates at a time so
the fat-LTO links stay within the runner's memory.

## Releasing

A release is: bump `[workspace.package] version` in `Cargo.toml`, merge to `main`, tag.

1. `git tag vX.Y.Z origin/main && git push origin vX.Y.Z`.
2. Watch it: `gh run list -R bande-linux/desktop -w release.yml -e push`, then
   `gh run watch -R bande-linux/desktop <id>`. All seven jobs must be green; apt takes only a
   fully green run. A flake: `gh run rerun <id> --failed` (same run id). Exit 137 in `debs` means
   the fat-LTO link ran out of memory: set `lto = "thin"` in the `dist` profile and release the
   next patch.
3. Within 30 minutes apt's `publish.yml` takes it (or
   `gh workflow run publish.yml -R bande-linux/apt -f repo=desktop -f run_id=<id>`), deploys the
   site and installs `bande-desktop` from it on both architectures.
4. Check: `curl -s https://bande-linux.github.io/apt/dists/rolling/main/binary-amd64/Packages`
   lists `bande-compositor` at the new version for `amd64`, and the same for `arm64`.

A published version is never moved or deleted. A bad release is fixed forward with the next
patch version; a tag whose run failed before apt published it may be deleted and pushed again.
Apps in bande-linux/apps are released the same way with `<app>-vX.Y.Z` tags in that repository.
