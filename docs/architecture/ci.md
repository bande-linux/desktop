# CI, the gate and releases

## One entry point

Every check a release runs is a stage of `ci/release/verify.sh`, and both `release.yml` and the
local gate call it, so CI holds no check the gate skips:

| Stage | What it runs |
|---|---|
| `lint` | `actionlint` (pinned release binary, with shellcheck) over the workflows, `ci/deb/version-test.sh`, then `ci/release/lint.sh`: `cargo fmt --all --check`, workspace clippy `-D warnings`, the dependency rules (no session service except the polkit agent depends on the widget crates; graphics and widgets stay off Wayland), `ci/check-apps.sh` (every app in `crates/apps`, the template included, is a `bande-app` app packaged like the others: package `bande-<dir>`, the app id naming its `.desktop` file, `Icon=` and metainfo, `Exec=`, `StartupNotify=true`, the toolkit's fonts in its `debian/control` Depends, no loop or menu export of its own), `ci/check-runtime-deps.py`, `ci/copyright.py --check` and `ci/deb/build-depends.py --check` (`debian/copyright` and the `librust-*-dev` Build-Depends follow `Cargo.lock`) |
| `test` | the workspace tests, every app's included, on the hermetic bus with lavapipe |
| `debs` | the source package (`ci/deb/source.sh`), every binary package built from it offline (`ci/deb/build.sh --build=binary`, without the tests, which `test` ran), `ci/release/check-debs.sh` and `lintian --fail-on error,warning` on the `.changes` |
| `gate` | `lint test debs` |

`verify.sh` sets the environment the stages share (no incremental, no debug info, build paths
remapped to `/build` and `/cargo` in Rust and C code) and checks that the Debian packages each stage needs are
installed, the Build-Depends of `debian/control` with the `pkg.bande.vendor` profile among them,
failing with the list of what is missing rather than skipping a step; `verify.sh --setup STAGE`
installs them instead (what CI does). `PROFILE` picks the cargo profile of the debs (default
`dist`).

Only these run in CI alone:

- `install`, the autopkgtests (`debian/tests`): they install the packages as root into a clean
  Debian 14 (forky) and switch display managers, which needs a throwaway system.
- The check that a released commit is on `main`.
- The arm64 debs; both architectures build from the same scripts.

## The local gate

Each change only has to compile: `cargo fmt --all --check` and `cargo check --all-targets`. A
finished branch passes `ci/release/verify.sh gate` before it is merged to `main` and `main` is
pushed; `PROFILE=dev` builds the debs with the `dev` profile, a few minutes, and a change of
`[workspace.package] version` is gated with `dist`, as the release builds. A fat-LTO link of the
shell needs much memory; `CARGO_PROFILE_DIST_LTO=thin`, which cargo reads through
`debian/rules`, builds the `dist` debs with thin LTO on a machine that lacks it. The tests and
why they exist are in [`../quality/tests.md`](../quality/tests.md). A test bus lists no service
directories, so it never D-Bus-activates the services installed on the machine.

## Runtime dependencies

`ci/runtime-deps` lists every program Bande runs (`Command::new`) and every system path it
reads (`/usr/...`, `/bin`, `/sbin`), each with the Debian package that ships it and a kind:
`required` (Debian priority required), `depends` (the package that runs or reads it, or for a
shared library one package using it, declares it in Depends), `optional` (the code checks
first and shows the user why a feature is off). `ci/check-runtime-deps.py` in `lint` fails on a
path or program missing from the list and on a `depends` entry its package does not declare, so
a new one is declared in the change that adds it; the `every-package` autopkgtest checks that
every `required` and `depends` entry is on a system installed with `--no-install-recommends`.

## GitHub CI

The workflows and what starts them:

- `release.yml`: version tags (`vX.Y.Z`, `vX.Y.Z-R`) and manual dispatch; below.
- `l10n.yml`: `l10n-YYYYMMDD` tags (`.N` for another the same day); builds only the language
  packs with the build profile `pkg.bande.l10n-only`, so a corrected translation ships without
  a release.
- `dict.yml`: `dict-YYYYMMDD` tags; builds the dictionary packs' source package `bande-dict`
  ([lookup.md](lookup.md)).
- `browser-follow-debian.yml`: every six hours, to follow Debian's `firefox-esr` uploads,
  pushes to `main` that change `browser/`, and dispatch; builds Bande's Browser
  ([`browser/README.md`](../../browser/README.md)).
- `reproducible.yml`: weekly and on dispatch; below.
- `debian-mode.yml`: weekly, on dispatch and on pull requests that change `Cargo.toml`,
  `Cargo.lock` or `debian/control`; below.

`release.yml` runs on GitHub-hosted runners (`ubuntu-24.04-arm` and `ubuntu-24.04`). Every job
runs once per supported Debian release, in a `debian:<suite>` container: the `suites` job reads
the list from the APT repository's `conf/suites` (through its reusable `suites.yml`). Each job
installs what it needs with `verify.sh --setup` (the Build-Depends of `debian/control`, the
suite's own `rustc` and `cargo` among them, and `rustfmt` and `rust-clippy` for lint), so each
suite builds with its Debian toolchain; cargo refuses a `rust-version` newer than that `rustc`.
Rendering runs on lavapipe, Mesa's CPU Vulkan, so no job needs a GPU or a display; the
compositor tests run it headless.

### Jobs

`release.yml` runs when a version is tagged and when dispatched by hand for an unpublished build
(`gh workflow run release.yml --ref <branch>`, never published). A newer run on the same ref
cancels the older one, and `source` fails a tag whose commit is not on `main`.

- `lint` (arm64, first suite): `verify.sh lint`.
- `test (<suite>)` (arm64): `verify.sh test`.
- `source` (first suite): checks that the tag, `Cargo.toml` and `debian/changelog` agree
  (`ci/deb/version.sh`), makes the source package (`ci/deb/source.sh`: the upstream tarball,
  the vendored crates and `debian/`), checks that `debian/copyright` and the librust
  Build-Depends follow `Cargo.lock`, and uploads it as `bande-source`.
- `debs (<suite>, arm64)`, `debs (<suite>, amd64)`: each on its native runner, installs the
  Build-Depends and builds the source package offline with `ci/deb/build.sh` (`dpkg-source -x`,
  a changelog entry for the suite's `~debNN`, `dpkg-buildpackage` with `pkg.bande.vendor`, the
  tests left to `test`): amd64 the architecture-dependent and -independent packages, arm64 the
  architecture-dependent ones. `check-debs.sh` checks that every package of `dh_listpackages`
  is there for `dpkg --print-architecture` at that version with its `-dbgsym`, each program
  stripped to its `.gnu_debugdata`, the D-Bus activation files, the user units, the portal
  backend's registration, the login session, the pointers and the English catalogs, that
  `bande-desktop` asks for its packages at the build's version, depends on `bande-greeter |
  x-display-manager`, `pipewire` and `wireplumber`, recommends `bande-greeter`,
  `bande-reporter`, `bande-crash-reports`, `pipewire-pulse` and `bande-browser` and otherwise only
  Bande packages built here,
  that `bande-greeter` provides `x-display-manager` from its own unit,
  and that the language packs hold only catalogs; then `lintian --fail-on error,warning` on the
  `.changes` and the `.dsc`. The job uploads `bande-debs-<suite>-<arch>`, amd64 also
  `bande-debs-<suite>-all`, and the `-dbgsym` packages as `bande-debug-symbols-<suite>-<arch>`.
  The APT source and keyring packages are built by the APT repository itself.
- `install (<suite>, <arch>)`: in a clean `debian:<suite>` on the native runner, runs the
  autopkgtests of the source package against that architecture's and the `all` packages
  (`autopkgtest --no-built-binaries <dsc> <debs> -- null`): `desktop`, `every-package`,
  `system-setup`, `display-manager`, `services`, `portal` and `metadata` (see
  [packaging.md](packaging.md#the-source-package)). A Debian package that the release lacks is a
  warning: `bande-desktop` only recommends `bande-backup`, and `every-package` installs it only
  when its dependencies can be.
- `release-source` (tags only, after `install`): attaches the source package to the tag's GitHub
  release; the vendored crates make it too big to keep in git.

`reproducible.yml` (weekly and on dispatch) builds the same source package twice on amd64, the
second time with another target folder, and writes the difference of the `.deb` checksums to the
run's summary. `debian-mode.yml` (weekly, on dispatch and on pull requests that change
`Cargo.toml`, `Cargo.lock` or `debian/control`; `forky` and `sid`, reporting only) builds
without `pkg.bande.vendor`, the way Debian's archive would: `ci/deb/debian-gap.py` lists each
`librust-*-dev` Build-Depends that `dpkg-checkbuilddeps` finds unmet with what Debian has
(installable, too old, a feature not packaged, or not packaged at that major version) in the
run's summary and as the `debian-gap-<suite>` JSON artifact; it then installs what Debian has
and tries `debian/rules build-arch` against Debian's crate registry.

The APT repository publishes on its own: every 30 minutes it looks for new green `release.yml`
runs for tags and takes each suite's `bande-debs-<suite>-arm64`, `bande-debs-<suite>-amd64` and
`bande-debs-<suite>-all` artifacts into the `main` component of that suite. It refuses a run
whose architectures do not carry the same packages at the tag's version, and after each deploy
it installs `bande-desktop` from the live site on amd64 and on arm64 and checks that every
package is that architecture's build (or `all`) at the published version
([`bande-linux/apt`](https://github.com/bande-linux/apt)).

### Versions

Versions come from `ci/deb/version.sh`. A tag `vX.Y.Z` builds the `debian/changelog` version
`X.Y.Z-1`, which must name `X.Y.Z`, the `[workspace.package] version` in `Cargo.toml`; a
packaging-only fix is a `debian/changelog` entry `X.Y.Z-R` and the tag `vX.Y.Z-R`. The source
package carries that version; each binary build adds a `debian/changelog` entry with the Debian
release suffix `~debNN` of the release it runs on (`ci/deb/debian-release.sh`:
`VERSION_CODENAME`, numbered by distro-info-data; forky is `~deb14`), the way Debian's backports
carry `~bpoNN`: `X.Y.Z-1~deb14` sorts below `X.Y.Z-1~deb15`, so upgrading Debian upgrades Bande
to the next release's build. Unpublished and local builds get
`X.Y.Z~gitDATE.COUNT.sha-1~debNN`, X.Y.Z the `Cargo.toml` version and COUNT the commits since
the last tag, which sorts below the release of X.Y.Z, or `X.Y.Z+git…` once X.Y.Z is released,
which sorts above it. Language packs put their stamp after the suffix, `X.Y.Z-R~debNN+l10nYYYYMMDD`,
so they too sort below the next Debian release's build; the dictionary packs are
`YYYYMMDD-1~debNN`.

Hosted runners start from a clean machine, so `Swatinem/rust-cache` restores the cargo
registry and the target directory (for `debs`, `target/deb`, which `CARGO_TARGET_DIR` points
the package build at), keyed per job and architecture and saved only from runs on `main`, whose
caches every branch and tag can read. The `test` and `debs` jobs first free disk on the host by
emptying the .NET, Android and GHC installations they mount, and `debs` builds with
`DEB_BUILD_OPTIONS=parallel=2`, so the fat-LTO links stay within memory.

## The update test

`ci/upgrade-test/run.sh <old-debs> <new-debs>` boots a throwaway Debian VM with QEMU and KVM, logs a
user in on a headless compositor at the old version and upgrades with apt while it runs. The new
build must take over the session services and ask for a logout, while the shell and lock screen,
killed after the update, come back as the old build and stay up. The shell must reconnect after
PipeWire restarts, and new Settings must run on the old compositor. It needs `qemu-system`,
`mtools` and `/dev/kvm`, so it runs by hand before a release; compare the new debs with the
previous release's.

## Releasing

1. Bump `[workspace.package] version` in `Cargo.toml` and add the entry `X.Y.Z-1` to
   `debian/changelog` (`dch -v X.Y.Z-1 -D unstable`) in a commit "Release X.Y.Z", run
   `ci/release/verify.sh gate` (with the `dist` debs), merge it to `main` and push `main`.
2. Tag the release commit: `git tag vX.Y.Z <commit> && git push origin vX.Y.Z`. The tag run
   builds and publishes, once.
3. Watch it: `gh run list -R bande-linux/desktop -w release.yml -e push`, then
   `gh run watch -R bande-linux/desktop <id>`. Every job must be green; apt takes only a
   fully green tag run. A flake: `gh run rerun <id> --failed` (same run id). Exit 137 in `debs`
   means the fat-LTO link ran out of memory: set `lto = "thin"` in the `dist` profile and
   release the next patch.
4. Within 30 minutes apt's `look.yml` finds it (or at once from its Run workflow button) and
   starts `publish.yml`, which deploys the site and installs `bande-desktop` from it on both
   architectures.
5. Check: `curl -s https://bande-linux.github.io/apt/dists/<suite>/main/binary-amd64/Packages`, for each suite,
   lists `bande-compositor` at the new version for `amd64`, and the same for `arm64`.

A published version is never moved or deleted. A bad release is fixed forward with the next
patch version; a tag whose run failed before apt published it may be deleted and pushed again.
Every package, the apps included, is released together at the one version.

## Soak and fuzz

`ci/soak/run.sh` (`--hours`, `--fuzz-seconds`, `--out`) is what proves stability; it runs by
hand, never as a gate. `bande-soak` (`crates/testing/bench`, not a test) runs the compositor in-process on
lavapipe with the real shell, lock screen, Files and Settings and cycles through window churn, the overview
and launcher, a second display with scale changes, lock and unlock (for the run, the machine's PAM file for `bande-lock`
permits any password, so run it only on a machine nobody else uses), memory pressure and
killed apps and shells. After each cycle it samples memory, file descriptors, threads, renderer memory and
frame times, and flags growth over the second half, a stall over 5 s or a client not answering for 30 s. Lavapipe's GPU
memory is process memory, so GPU growth shows as RSS; a leak only in a hardware GPU driver cannot show here. Results go
to `--out` (`result.txt`, `summary.txt`, `samples.tsv`, `crashes.txt` with the stack of any crash the soak did not
cause). The fuzzers in `fuzz/` (`cargo fuzz run --sanitizer none <target>`, outside the workspace) then run: the
Wayland wire against the real compositor, file previews, text formats, config and `.desktop` files, and the lines privileged
helpers read. Their corpus stays in `~/.cache/bande-fuzz` on the machine; a crash leaves its input in the results.
