# CI and tagged releases (arm64 + amd64) into the APT repository

## Summary

The tag-only CI has never run: no tag exists, and if one were pushed now it would fail at
`ci/lint` on `main`. Before it went tag-only, CI failed mainly because two runners and a KVM
runner shared one 12 GB Docker VM. Linkers and fat-LTO compiles ran out of memory, and timing
tests failed under the load. Jobs waited hours in the queue. Nothing builds amd64, and the APT
repository is arm64-only and has never published (the site returns 404).

Plan: a new `release.yml` on GitHub-hosted runners, with native `ubuntu-24.04-arm` and
`ubuntu-24.04`, building inside `debian:trixie`. The heavy jobs (KVM guest, perf, polkit) move
out of the release gate. The APT tooling learns amd64 and checks each architecture after every
publish.

## What the owner sees

- "Only starts on tags but still has not run successfully": it has never started. No tags or
  releases exist in `unbound-linux/desktop` (`gh api repos/unbound-linux/desktop/tags` is empty).
  Since `81a5e22e` (2026-10-31 07:18) only stale branches still run CI: a push to a branch with
  the old `ci.yml` still triggers it, for example run 41186754177 on `wip/userland-efficiency` at
  08:58.
- Before that change: from 2026-10-27 to 2026-10-31 there were 606 runs: 77 success, 149
  failure, 380 cancelled. After the fat-LTO `dist` profile landed (2026-10-30 08:12) there were
  185 runs: 2 success, 24 failure, 159 cancelled. Both successes were lint-only branch runs.
  `ci/vm` has not passed since then.
- `https://unbound-linux.github.io/apt/dists/trixie/InRelease` returns 404. The apt `publish`
  workflow has never run; the apt repo shows only two `check` runs, from 2026-10-29.

## Findings

### Run history by failing job (all 149 failed runs)

| Job | Failed runs | Main causes (sample of up to 30 logs per job) |
|---|---|---|
| ci/vm | 117 | `ld terminated with signal 9 [Killed]` (11/30), exit 137 (4/30), guest-test timeouts, QEMU killed by `timeout` (status 124) |
| ci/lavapipe-test | 79 | `ld ... signal 9` / exit 137; timing assertions under load; one golden/wgpu validation failure (Oct 30, fixed since) |
| ci/deb | 63 | `sccache: Compile terminated by signal 9` in 22/30: every sampled failure since `dist` (fat LTO, 1 CGU) landed |
| ci/lint | 14 | today: dependency rule broken on `main`; earlier: rustfmt, E0603, missing `libasound2-dev` on a branch |
| ci/perf | 11 | frame-time budgets missed while other jobs share the Mac (backlog.md) |
| ci/polkit | 1 | — |

### Root causes, with evidence

1. **Memory: a shared 12 GB Docker VM.** `runners/compose.yaml` puts up to four runners in one
   8-CPU VM (STATUS.md: "more ran the 12 GB Docker VMs out of memory"). lint, the test build, the
   dist build and the KVM guest all run in it at the same time.
   - Run 41132776728 `ci/deb`, 03:28:30: `sccache: Compile terminated by signal 9` while
     compiling `unbound-settings` with `-C lto=fat -C codegen-units=1`, `CARGO_BUILD_JOBS: 2`.
   - Run 41127744241 `ci/vm`: `collect2: fatal error: ld terminated with signal 9 [Killed]`.
   - Run 41106135202 `ci/lavapipe-test`: the same `ld` kill on `unbound-desktop-surface`.
   - `main` now builds the dist binaries with one job (`CARGO_BUILD_JOBS: '1'`), but the other
     jobs still share the VM.
2. **Queueing.** Since Oct 30 only `unbound-runner-1`, `unbound-runner-2` and
   `unbound-runner-kvm` took jobs (the laptop was off), and every branch push started a full
   run. Run 41132776728 (dispatch on `desktop/ci-green`) was created 21:40 and ended 05:40,
   8 h in all. Its jobs started this late:

   | Job | Started after | Ran for |
   |---|---|---|
   | deb-all | 2 h 15 | |
   | vm | 3 h 07 | 68 min (test build `53m 18s`) |
   | deb | 5 h 41 | killed after 8 min |
   | lint | 6 h 24 | |
   | lavapipe-test | 6 h 50 | 70 min |

   380 of 606 runs were cancelled as superseded.
3. **Load-sensitive tests in the gate.** These fail under load and pass when the runner is
   idle:
   - `ci/vm` guest tests, run 41132776728: `timed out waiting for the crosshair of the area
     overlay` and `greetd sees the refused attempt cancelled: Timeout`.
   - `ci/lavapipe-test`, run 41013599518: `every_keystroke_updates_the_results_within_a_frame`
     with `median 17.555334ms`.
   - `ci/perf`: p99 frame budgets.

   `fetch-run.sh` in the apt repo requires every job to be green, so any one of these flakes
   blocks a publish.
4. **`main` is red at lint.** In runs 41175256578 and 41186754177, the job
   `Keep services off the widget crates...` prints
   `crates/services/polkit/Cargo.toml must not depend on the crates above`. Cause: `a38b0dfc`
   gave the polkit agent a toolkit dialog (`unbound-toolkit.workspace = true`). A tag pushed
   today fails on this first.
5. **No amd64 at all.**
   - There are no x86 runners; every runner is arm64 in a Mac.
   - `ci.yml` asserts `Architecture = arm64`.
   - `apt/conf/distributions` has `Architectures: arm64` for both suites, so reprepro rejects
     amd64 debs. On an amd64 client, apt skips the source because the Release file does not
     list amd64.
   - `rebuild.sh` fails with `Release does not list main/binary-amd64/Packages.xz` once amd64
     is added, because it requires every configured architecture in the live Release.
6. **The publish chain is broken even when builds are green.**
   - `apt/.github/workflows/publish.yml` gained `push: tags: ['v*']` in `1a8bf20`. On an apt
     tag it would run with an empty `run_id` and fail.
   - The testing channel accepts only `event == push && ref == main` (`fetch-run.sh`), which
     can no longer happen, so the testing channel is dead.
   - The first publish needs `initial: true`, which desktop's `ci/publish` never passes.
   - The stable channel waits for a reviewer in the `apt-stable-approval` environment.
   - Re-running a desktop build produces new bytes for the same version. `rebuild.sh` then
     refuses it with "already in ... with different contents", so a retry needs the original
     artifacts.
7. **Images consume a dead artifact.** `images/.github/workflows/build.yml` downloads an
   artifact named `unbound-debs` from the pinned run 40739365836 (Oct 28). The artifact is now
   called `unbound-debs-arm64`, and the run's artifacts expire after 90 days.
8. **Stale docs.** `docs/architecture/ci.md` still says jobs run on Ubuntu 24.04 and that
   `ci/perf` runs on `main`. The runner image is `debian:trixie` (runners/README.md).

### What worked

- A warm, uncontended full run took about 40 min: run 40928339797 (Oct 29, before fat LTO),
  with `ci/deb` at 5 min and `ci/lavapipe-test` at 6 min.
- The only green dist `ci/deb`, run 41117545815, ran on a nearly empty target dir (2 GiB):
  `Finished dist profile in 27m 54s`.
- The `ci/lavapipe-test` suite itself passed in 41132776728.
- Size: 19 dist binaries of 3 to 22 MB each, dist target dir 2.0 GB, debs 68 MB per
  architecture per version (local `target/debian`). Two suites times two architectures comes to
  about 275 MB of the site's 800 MB budget.

## Options

**A. Keep self-hosted for arm64, add hosted amd64.**
- For: warm caches, fast arm64 builds.
- Against: releases depend on the Macs being on and idle, the 12 GB VM keeps the OOM risk,
  and the arm64 and amd64 builds run in different environments.

**B. All release jobs on GitHub-hosted runners, building natively in `debian:trixie`
(recommended).**
- For: clean, identical environments for both architectures, independent of the Macs, no QEMU.
- Against: `desktop` is private, so runners are 2 vCPU, 8 GB RAM and 14 GB SSD. Minutes are
  billed against the Free plan's 2000 per month (Linux x64 $0.006/min, arm64 $0.005/min beyond
  that).
- A release estimate of about 200 min is under 10 % of the monthly allowance.
- Caches: a tag run can restore only caches from `main`, never from another tag, and caches go
  after 7 days unused. Tag builds are therefore usually cold (estimate 45 to 90 min per job,
  to be measured). A `main` dispatch for testing warms them.

**C. Option B with `desktop` made public.**
- Same as B, but 4 vCPU, 16 GB, unlimited free minutes and twice the speed. It also removes the
  fat-LTO memory risk.
- Owner decision: the packages are public already, and the apt repo is public.

**D. Cross-compile amd64 on the arm64 Macs.**
- Rejected: it needs multiarch `-dev` packages for pipewire, libinput and the others, and
  `$auto` dependencies are less trustworthy. It also keeps the Macs in the release path.

For each option, also decide what gates a release:
- **Today:** every job gates the release, including the KVM guest, perf and polkit.
- **Recommended:** lint, the lavapipe workspace tests (arm64), package checks and install
  tests on both architectures. Move `ci/vm`, `ci/perf` and `ci/polkit` to a dispatch-only
  `extended.yml` on the self-hosted runners that does not gate. They overlap with the owner's
  verification on the mini.

## Recommendation

Option B (or C if the owner makes `desktop` public), with this shape.

**Channels after the change:**
- **stable** (`trixie`) is fed by `vX.Y.Z` tags only.
- **testing** (`trixie-testing`) is fed by `gh workflow run release.yml --ref main`, with
  version `X.Y.Z+gitDATE.N.sha`, the same formula as local builds.
- An optional nightly schedule on `main` (only when `main` moved) is an owner decision.
- Software Update follows whichever `unbound-apt-source(-testing)` package is installed. The
  mini can install `unbound-apt-source-testing` and stop relying on hand-built `0.0.0+git`
  debs.

**`desktop/.github/workflows/release.yml`**, triggered by `push: tags: ['v[0-9]+.[0-9]+.[0-9]+']`
and `workflow_dispatch`:

| Job | Runner | Does |
|---|---|---|
| `lint` | `ubuntu-24.04-arm`, `container: debian:trixie` | fmt, clippy `-D warnings`, the dependency rules |
| `test` | `ubuntu-24.04-arm`, trixie container | `cargo test --workspace --locked` on lavapipe (`mesa-vulkan-drivers`), Debian 13 fonts from apt (the goldens are arm64/Debian 13), each test on its hermetic `crates/testing/test-bus` session bus (needs `dbus-daemon`) |
| `debs` | matrix `{arm64: ubuntu-24.04-arm, amd64: ubuntu-24.04}` | see below |
| `deb-all` | any | builds `unbound-desktop` (all), keyring and source packages; lintian |
| `install` | matrix, native arch | `docker run debian:trixie` installs `unbound-desktop` from the debs alone (`ci/deb/install-test`), plus the portal check |
| `release` | tags only | creates GitHub release `vX.Y.Z` with all debs and `SHA256SUMS`: durable, the same bytes on retry |
| `publish` | | dispatches `apt/publish.yml` with `run_id` and the tag (or empty for testing from `main`) |

`debs` runs on the host, then in a container:
- On the host: free disk (remove `/usr/local/lib/android`, `/usr/share/dotnet`, `/opt/ghc`,
  `/opt/hostedtoolcache`), add an 8 GB swap file, restore the cache.
- In `docker run debian:trixie` with the workspace and `~/.cargo` mounted, run
  `ci/release/build-debs.sh`:
  - `cargo build --profile dist -j1` for the packaged crates;
  - `cargo deb --no-build --deb-version $V`. cargo-deb derives `Architecture` from the native
    target and `$auto` dependencies from trixie's dpkg-shlibdeps;
  - `ci/release/check-debs.sh`: architecture equals `dpkg --print-architecture`, the version,
    the D-Bus, unit and portal checks from today's `ci/deb`, and the headless compositor smoke
    test on lavapipe.
- Uploads `unbound-debs-<arch>`.

**Caching:** `Swatinem/rust-cache` keyed per arch and job, saved only from `main` (`save-if`),
since tags can't share caches. Watch the 10 GB repository limit. No sccache on hosted runners.

**Versioning:**
- `ci/deb/version.sh` stays. A tag must equal the `[workspace.package] version`, so a release
  means bump, commit, tag.
- The release job writes a `debian/changelog` stanza for `unbound-desktop` from the tag with
  `dch`, instead of `DEB_VERSION_OVERRIDE` alone.

**APT repository:**
- `Architectures: amd64 arm64` for both suites. reprepro writes `binary-amd64/` and
  `binary-arm64/` indices and puts `Architecture: all` packages into both.
- `rebuild.sh` treats an architecture missing from the live Release as empty, for the first
  amd64 publish.
- New check: every arch-specific package exists for every architecture at one version.
  Otherwise the `all` meta package, which pins `(= version)`, is uninstallable on the
  architecture that is missing it.
- Signing stays as is: `APT_SIGNING_KEY` in the `apt-publish` environment, reprepro `SignWith`.
- New `verify` job after deploy, on `ubuntu-24.04` and `ubuntu-24.04-arm` in
  `container: debian:trixie` (apt is public, so these runners are free and 4 vCPU):
  - follow the site's install steps for the channel just published, retrying while Pages
    propagates;
  - `apt-get install unbound-desktop`;
  - assert `dpkg-query -W -f '${Architecture}' unbound-compositor` equals
    `dpkg --print-architecture`, and that the version equals the published version;
  - run `unbound-compositor --headless` on lavapipe.

## Implementation plan

Each milestone takes about 1 to 2 hours of agent work. Verification is local on the mini (no
Docker locally) unless it says CI. Order: D1, A1, D2, D3, A2, A3, D4, D5, I1, R1, then the
first tag (M-final).

### desktop

**D1. Make `main` release-clean.**
- Fix the lint rule failure: exempt `unbound-polkit` as an agent with UI, or move it under
  `crates/shell/`. This is an owner decision; default to exempting it.
- Add `unbound-polkit` to `unbound-desktop` Recommends. It is packaged but nothing pulls it in.
- Measure peak RSS of `cargo build --profile dist -j1` per binary on the mini with
  `/usr/bin/time -v` and record it in `docs/architecture/ci.md`. If any binary exceeds about
  6 GB, propose `lto = "thin"` for `dist`.
- Verify: the full local gate on the mini, plus the lint step run by hand.

**D2. Extract the packaging steps into scripts.**
- `ci/release/build-debs.sh`, `check-debs.sh` and `smoke.sh`, taken from the inline steps in
  `ci.yml`, with no hard-coded arm64: use `dpkg --print-architecture`.
- Verify: run them natively on the mini and check that the 19 debs pass and carry
  `Architecture: arm64`. Read the script output back in full.

**D3. Add `release.yml` (lint, test, debs matrix, deb-all, install matrix).**
- No publish yet. `actionlint` locally.
- Verify (CI): `gh workflow run release.yml --ref ci/release`. Both
  `unbound-debs-{arm64,amd64}` artifacts are produced with matching `Architecture` fields, and
  both install jobs are green. Record each job's wall time and the host's `df` and peak memory.

**D4. The `release` and `publish` jobs.**
- On tags: `gh release create` with the assets and `SHA256SUMS`, and the `debian/changelog`
  stanza from the tag.
- `publish` dispatches `apt/publish.yml` with `run_id` and the tag (empty from `main`).
- Extend `version-test.sh` for the changelog stanza.
- Verify: a dry run on a branch dispatch (release step skipped, publish to testing only after
  A1 and A2 are merged).

**D5. Retire `ci.yml`.**
- Move `vm`, `polkit` and `perf` into `extended.yml` (`workflow_dispatch` only,
  `[self-hosted, unbound]` and `[self-hosted, unbound-kvm]`, not gating).
- Delete `ci.yml` and rewrite `docs/architecture/ci.md` (hosted runners, channels, release
  steps).
- Delete or merge `main` into stale branches that still carry the old `ci.yml`, so pushes stop
  starting runs.
- Verify: `actionlint`; `gh workflow list` shows release and extended only.

### apt

**A1. Multi-architecture archive.**
- Remove `push: tags` from `publish.yml` and `Architectures: amd64 arm64` in
  `conf/distributions`.
- `rebuild.sh` tolerates an architecture missing from the live Release.
- New per-architecture completeness check.
- `site/index.html` says "arm64 and amd64".
- Tests: rebuild with both architectures, `all` packages in both indices, refusal of a
  one-arch set.
- Verify: `tests/run-all.sh` natively on the mini (install `ci/packages` with apt as root).

**A2. Fetch from `release.yml`.**
- `fetch-run.sh` accepts `.github/workflows/release.yml`:
  - stable needs a run on the tag itself;
  - testing needs a `workflow_dispatch` on `main`.
- Drop the main-to-release promote path.
- Require `unbound-debs-arm64`, `unbound-debs-amd64` and `unbound-debs-all`. The publisher
  job name becomes `publish`.
- Verify: `tests/fetch-run-test.sh` updated and green on the mini.

**A3. Check each architecture after deploy.**
- A `verify` job on `ubuntu-24.04` and `ubuntu-24.04-arm` (in `container: debian:trixie`)
  installs from the live site and asserts the architecture and version.
- Document the first publish: `gh workflow run publish.yml -f run_id=… -f initial=true`.
- Verify (CI): the first testing publish from a D4 `main` dispatch. The site serves
  `dists/trixie-testing/main/binary-{amd64,arm64}/Packages`, and `verify` is green on both
  architectures.

### images

**I1. Take desktop packages from the archive.**
- `build.yml` installs `unbound-desktop` from the apt stable channel (or downloads the desktop
  release assets of a given tag) instead of the `unbound-debs` artifact of run 40739365836.
- Verify: on the owner's go-ahead, a dispatch on the `unbound-kvm` runner (it boots a guest),
  or a local `build.sh` dry run on the mini.

### runners

**R1. Fewer runners per Mac** (optional; the release no longer depends on them).
- One build runner plus the KVM runner per Mac, and `mem_limit` per container, so `extended.yml`
  stops hitting the OOM killer.
- Update STATUS.md and the runners README (M4 mini plus laptop, 12 GB VM).
- Verify: on the owner's go-ahead, an `extended.yml` dispatch with no `signal 9` in the logs.

### M-final. First release `v0.1.0`

- The owner approves the tag (Cargo.toml already says 0.1.0) and the stable approval.
- Verify: the `release.yml` tag run is green; `apt/publish` deploys `trixie` with both
  architectures and `verify` is green; then `apt install unbound-desktop` on the mini from
  stable, with Software Update following it.

## Risks

- **Private runners (2 vCPU, 8 GB, 14 GB).** The fat-LTO links may run out of memory, and the
  test target dir may fill the disk. Mitigations: `-j1`, a swap file, freeing the host disk,
  thin LTO as a fallback, or making the repo public (C). D1 and D3 measure all of this before
  anything depends on it.
- **Cold tag builds** take an hour or more. That is acceptable for releases; a `main` dispatch
  shortly before tagging warms the caches.
- **amd64 is not tested beyond install and smoke.** The goldens are arm64/Debian 13; an amd64
  `cargo test` job can come later, starting as non-gating, because llvmpipe on x86 may differ
  by a few pixels.
- **Site size** grows with each architecture. At about 275 MB now it is far from 800 MB;
  `check-site.sh` already warns at 600 MB.
- **Stable releases stop at a manual approval** (`apt-stable-approval`). That is intended, but
  a forgotten approval looks like "did not publish".
- **Branches with the old `ci.yml`** keep starting self-hosted runs until D5 cleans them up.
- **Actions minutes** are billed on the Free plan once 2000 per month are used. Check usage
  after the first releases.

## Decisions for the owner

1. Make `unbound-linux/desktop` public (4 vCPU, 16 GB, free minutes), or keep it private and
   use 2-vCPU hosted runners with paid minutes beyond 2000 per month?
2. Release gate: lint, lavapipe tests, packages and install tests, with the KVM guest, perf
   and polkit moved to a non-gating dispatch workflow. Agree?
3. Testing channel: manual `main` dispatch only, or also nightly when `main` changed?
4. `dist` profile: keep fat LTO if it fits in memory, or switch to thin LTO for reliable builds
   on 8 GB runners?
5. `unbound-polkit` and the lint rule: exempt it as an agent with UI, or move the crate under
   `crates/shell/`?
6. Keep the reviewer approval for every stable publish?

Sources:
- GitHub-hosted runner specs (private repos: 2 vCPU, 8 GB, 14 GB; arm64 available):
  https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- Runner pricing: https://docs.github.com/en/billing/reference/actions-runner-pricing
- Cache scope (tags can't share caches; 7-day eviction; 10 GB per repository):
  https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching
- Debian repository format: https://wiki.debian.org/DebianRepository/Format
