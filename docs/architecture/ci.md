# CI, the gate and releases

## One entry point

Every check a release runs is a stage of `ci/release/verify.sh`, and both `release.yml` and the
local gate call it, so CI holds no check the gate skips:

| Stage | What it runs |
|---|---|
| `lint` | `actionlint` (pinned release binary, with shellcheck) over the workflows, `ci/deb/version-test.sh`, then `ci/release/lint.sh`: `cargo fmt --all --check`, workspace clippy `-D warnings`, the dependency rules (no session service except the polkit agent depends on the widget crates; graphics and widgets stay off Wayland) and `ci/check-apps.sh` |
| `test` | the workspace tests on the hermetic bus with lavapipe |
| `all` | `ci/release/build-all.sh`: the `bande-desktop` metapackage, its relations and `lintian` |
| `debs` | `ci/release/build-debs.sh`, `ci/release/check-debs.sh`, `ci/release/smoke.sh` |
| `gate` | `lint test all debs` |

`verify.sh` sets the environment the stages share (no incremental, no debug info, build paths
remapped to `/build`, `/cargo` and `/rustup`) and checks that the Debian packages each stage
needs are installed, failing with the list of what is missing rather than skipping a step;
`verify.sh --setup STAGE` installs them, the toolchain and the pinned `cargo-deb` instead (what
CI does). `VERIFY_PROFILE` picks the cargo profile of the debs (default `dist`).

Only these run in CI alone:

- `install` (`ci/deb/install-test/`): it installs the packages as root into a clean Debian
  testing and switches display managers, which needs a throwaway system; the build machine
  runs no containers.
- The check that a released commit is on `main`: the train only ever pushes `main`.
- The amd64 debs: the build machine is arm64; both architectures build from the same scripts.

## The local gate

Each change only has to compile: `desktop-check <worktree> -p <crate>` runs `cargo fmt` and
`cargo check --all-targets`. Finished branches are queued with `desktop-enqueue <branch>`; the
merge train (`desktop-train`) merges the queued branches onto `main`, runs `desktop-gate` on the
batch and pushes `main` when it is green. `desktop-gate <worktree>` is `ci/release/verify.sh
gate` under the build lock with the debs built with the `dev` profile, a few minutes; a batch
that changes `[workspace.package] version` builds them with `dist`, as the release does. These
scripts live in `~/bande-linux/bin` on the build machine, outside this repository. The tests and
why they exist are in [`../quality/tests.md`](../quality/tests.md). A test bus lists no service
directories, so it never D-Bus-activates the services installed on the machine.

## GitHub CI

The only workflow, `release.yml`, runs on version tags (`vX.Y.Z`) and on manual dispatch, never
on pushes or pull requests. It runs on GitHub-hosted runners (`ubuntu-24.04-arm` and
`ubuntu-24.04`), every job in a `debian:testing` container that installs what it needs with
`ci/apt-install.sh` (build libraries in `ci/release/build-depends`) and the toolchain pinned in
`rust-toolchain.toml`. Rendering runs on lavapipe, Mesa's CPU Vulkan, so no job needs a GPU or
a display; the compositor tests run it headless.

### Jobs

`release.yml` runs when a version is tagged and when dispatched by hand: with a `tag` input it
is the dry run of that release (`ci/release/dry-run.sh`), built exactly as the tag will be, at
the tag's version; without one, a test build of the ref at a `+git` version. Dispatch runs are
never published. A newer run on the same ref cancels the older one, and `deb-all` fails a
release whose commit is not on `main`. Each job installs with `verify.sh --setup` and runs one
stage:

- `lint` (arm64): `verify.sh lint`.
- `test` (arm64): `verify.sh test`.
- `debs (arm64)`, `debs (amd64)`: each on its native runner, `verify.sh debs`: `cargo build
  --profile dist` for the crates in `ci/release/packages` and `ci/release/bundled` (built with
  them, shipped inside another package: `bande-secrets-prompt` in `bande-secrets`) and `cargo
  deb` for each at the version from `ci/deb/version.sh`. cargo-deb takes the architecture from
  the native target and the shared-library dependencies from Debian testing's `dpkg-shlibdeps`.
  `check-debs.sh` checks that every package is there for `dpkg --print-architecture` at that
  version with no build paths inside, the D-Bus activation files, the user units, the portal
  backend's registration and the login session; `smoke.sh` starts the packaged settings service
  on a private bus to read a property and the packaged compositor headless. The job uploads
  `bande-debs-<arch>`.
- `deb-all`: `verify.sh all` builds the architecture-independent meta-package `bande-desktop`,
  checks that it pins its packages at the build's version, that it depends on `bande-greeter |
  x-display-manager`, `pipewire`, `wireplumber` and `pipewire-pulse` and recommends
  `bande-greeter`, `bande-apt-source` and `systemd-zram-generator`, and lints it with `lintian`.
  It uploads `bande-debs-all`. The APT source and keyring packages are built by the APT
  repository itself.
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

Versions come from `ci/deb/version.sh`. A release (`RELEASE_TAG`: the pushed tag, or the dry
run's `tag` input) `vX.Y.Z` builds version `X.Y.Z` and must equal the `[workspace.package]
version` in `Cargo.toml`. Test builds and local builds get `X.Y.Z+gitDATE.COUNT.sha`, counted
from the last tag, which sorts above that tag and below the next.

Hosted runners start from a clean machine, so `Swatinem/rust-cache` restores the cargo
registry, the installed `cargo-deb` and the target directory, keyed per job and architecture
and saved only from runs on `main`, whose caches every branch and tag can read. The `test` and `debs` jobs first free disk on the host by emptying
the .NET, Android and GHC installations they mount, and `debs` builds two crates at a time so
the fat-LTO links stay within the runner's memory.

## Releasing

1. Bump `[workspace.package] version` in `Cargo.toml` in a commit "Release X.Y.Z" and queue it
   with `desktop-enqueue`; the train runs the gate with the `dist` debs for it and pushes `main`.
2. Dry run: `ci/release/dry-run.sh` dispatches `release.yml` on `main` with `tag=vX.Y.Z` for
   the release commit (it must be `main`'s head) and waits for every job; it prints the run's URL.
3. Only when the dry run is green: `git tag vX.Y.Z <commit> && git push origin vX.Y.Z`.
4. Watch the tag run: `gh run list -R bande-linux/desktop -w release.yml -e push`, then
   `gh run watch -R bande-linux/desktop <id>`. All seven jobs must be green; apt takes only a
   fully green tag run. A flake: `gh run rerun <id> --failed` (same run id). Exit 137 in `debs`
   means the fat-LTO link ran out of memory: set `lto = "thin"` in the `dist` profile and
   release the next patch.
5. Within 30 minutes apt's `look.yml` finds it (or at once from its Run workflow button) and
   starts `publish.yml`, which deploys the site and installs `bande-desktop` from it on both
   architectures.
6. Check: `curl -s https://bande-linux.github.io/apt/dists/rolling/main/binary-amd64/Packages`
   lists `bande-compositor` at the new version for `amd64`, and the same for `arm64`.

The tag run rebuilds rather than promoting the dry run's packages: apt verifies that a run is
the push of the tag and takes that run's own artifacts, and a promotion path would be the one
part of the pipeline the dry run cannot exercise. The two runs build the same commit, version,
lockfile and scripts in the same image.

A published version is never moved or deleted. A bad release is fixed forward with the next
patch version; a tag whose run failed before apt published it may be deleted and pushed again.
Apps in bande-linux/apps are released the same way with `<app>-vX.Y.Z` tags in that repository.
