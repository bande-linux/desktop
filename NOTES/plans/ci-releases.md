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

## Status (2026-11-01)

Everything that can be checked without CI is done. The first tag (below, "Cutting the first
release") is the first CI run of the whole path. What was built differs from the plan below in
places:

- **One suite, no channels.** The archive is one suite, `rolling`, for Debian testing; there
  is no `trixie` stable suite and no `trixie-testing` channel. All jobs build in
  `debian:testing`, not `debian:trixie`.
- **apt pulls, desktop does not push.** apt's `publish.yml` runs every 30 minutes and takes the
  green `release.yml` push of the highest `vX.Y.Z` tag. desktop has no `publish` or `release`
  job and makes no GitHub release; apt downloads the run's artifacts.
- **No approval step.** The `apt-publish` environment has a branch policy only, so a green tag
  publishes on its own.
- **`bande-linux/desktop` is public** (decision 1 is settled): 4 vCPU, 16 GB, free minutes.
- **The site is live** since 2026-10-31 21:44 with the repository packages (1.3, both
  architectures) and the kernel (`apple`, arm64). No desktop yet.

| Milestone | State |
|---|---|
| D1-D3, D5 | done on `main` (release.yml, `ci/release/*.sh`, extended.yml, ci.yml gone, polkit exempt and in Recommends) |
| D1 peak RSS | not measured: needs a dist cargo build and the disk is tight; the tag run's `debs` job prints disk and memory |
| D4 | done as "apt pulls" above. The `dch` changelog stanza is not done. Decided overnight 2026-11-01: keep `DEB_VERSION_OVERRIDE` with `dh_gencontrol -v`; lintian is clean at 0.1.0 and nothing reads the changelog version (plan recommendation was dch; owner may revisit) |
| A1, A2 | done on apt `main`; `tests/run-all.sh` green on the mini (240 checks) |
| A3 | done, apt `5fc022d`: `verify` waits until the site serves the desktop version the build just put in, installs `bande-desktop` on amd64 and arm64, checks every installed `bande-*` package is that architecture or `all` at that version, starts the compositor headless. Checked against a fixture archive; it runs in CI with the first desktop publish |
| I1 | done, images `ecdcaaf` on `prep/os-installable` (images has no `main`): `build.yml` fetches the desktop from the archive with `fetch-desktop.sh` (signature, index and package checksums, the desktop's and apt's packages from `state/owners`, optional `desktop-version` input) instead of the expired run 40739365836. Checked against fixture archives and the live site |
| R1 | not done (optional) |
| M-final | waits for the owner's tag |

Dry review of tag -> build -> publish, overnight, without CI: `version-test.sh`;
`check-debs.sh` and `smoke.sh` against the mini's 19 arm64 debs (fixed: `check-debs.sh` read
the control archive through a pipe that `grep -q` broke, exit 141); `build-all.sh` with
lintian at 0.1.0; every arch package is reachable from `bande-desktop` or `bande-os`;
shellcheck of every workflow `run:` block; the action versions, cargo-deb 3.8.0 and actionlint
1.7.12 exist; fetch-run's per-architecture completeness and version checks; `all` packages in
both indices. actionlint itself is not packaged for Debian, so only the `lint` job runs it.

Still open:
- **Image suite (owner).** The image is Debian 13 (`trixie`), the archive's packages are built
  for Debian testing (`bande-os` says "Debian testing installation"). dpkg-shlibdeps minimum
  versions usually still fit trixie, but a library only testing has makes the image build fail
  at `apt-get install`, and installed images follow `rolling` for updates. Move the image to
  testing, or build a trixie set as well?
- **D5 leftover.** `origin/wip/userland-efficiency` still carries the old `ci.yml` with
  `push: branches: '**'`, so a push to it starts self-hosted runs. Nine other unmerged branches
  carry the tag-only `ci.yml`, which is harmless since tags are cut on `main`. Not deleted:
  they are other agents' work.

## Cutting the first release

1. On the mini, `main` is what you want to ship and its gate is green, and `Cargo.toml` says
   `[workspace.package] version = "0.1.0"` (it does).
2. Optional, recommended: warm the caches with a test build of `main` (free, never published):
   `gh workflow run release.yml -R bande-linux/desktop --ref main`. It also shows the job
   times and whether the fat-LTO links fit in memory before a tag depends on it.
3. Tag and push: `git tag v0.1.0 origin/main && git push origin v0.1.0`.
4. Watch `release.yml` for the tag: `gh run list -R bande-linux/desktop -w release.yml -e push`,
   then `gh run watch -R bande-linux/desktop <id>`. All seven jobs must be green (lint, test,
   debs arm64 and amd64, deb-all, install arm64 and amd64): apt takes only a fully green run.
   - A flake: `gh run rerun <id> --failed`. Same run id, apt still takes it.
   - `signal 9` / exit 137 in `debs`: the fat-LTO link ran out of memory; set
     `lto = "thin"` for the `dist` profile, then release 0.1.1.
   - A code fix: bump to 0.1.1, merge, tag `v0.1.1`. Never move a published tag.
5. Within 30 minutes apt's `publish` takes it (`gh run list -R bande-linux/apt -w publish.yml`),
   or start it at once: `gh workflow run publish.yml -R bande-linux/apt -f repo=desktop -f run_id=<id>`.
   Watch find, build (its log says `bande-desktop 0.1.0`), deploy, release and both `verify`
   jobs (`ubuntu-24.04`, `ubuntu-24.04-arm`): they install `bande-desktop` from the live site
   and check the architecture and version. A `verify` that times out on "Pages serves
   bande-desktop none yet" is slow propagation: rerun it.
6. Check by hand: `curl -s https://bande-linux.github.io/apt/dists/rolling/main/binary-amd64/Packages | grep -A3 '^Package: bande-compositor$'`
   shows `Architecture: amd64` and `Version: 0.1.0`; the same with `arm64`.
7. On the mini, if it should follow the archive: install `bande-archive-keyring` and
   `bande-apt-source` as the site describes, `apt update`, `apt policy bande-desktop`.
   Local builds after the tag are `0.1.0+git...`, which sorts above `0.1.0`, so
   `rebuild-desktop` builds still win over the archive until the next tag.

## What the owner sees

- "Only starts on tags but still has not run successfully": it has never started. No tags or
  releases exist in `bande-linux/desktop` (`gh api repos/bande-linux/desktop/tags` is empty).
  Since `81a5e22e` (2026-10-31 07:18) only stale branches still run CI: a push to a branch with
  the old `ci.yml` still triggers it, for example run 41186754177 on `wip/userland-efficiency` at
  08:58.
- Before that change: from 2026-10-27 to 2026-10-31 there were 606 runs: 77 success, 149
  failure, 380 cancelled. After the fat-LTO `dist` profile landed (2026-10-30 08:12) there were
  185 runs: 2 success, 24 failure, 159 cancelled. Both successes were lint-only branch runs.
  `ci/vm` has not passed since then.
- `https://bande-linux.github.io/apt/dists/trixie/InRelease` returns 404. The apt `publish`
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
     compiling `bande-settings` with `-C lto=fat -C codegen-units=1`, `CARGO_BUILD_JOBS: 2`.
   - Run 41127744241 `ci/vm`: `collect2: fatal error: ld terminated with signal 9 [Killed]`.
   - Run 41106135202 `ci/lavapipe-test`: the same `ld` kill on `bande-desktop-surface`.
   - `main` now builds the dist binaries with one job (`CARGO_BUILD_JOBS: '1'`), but the other
     jobs still share the VM.
2. **Queueing.** Since Oct 30 only `bande-runner-1`, `bande-runner-2` and
   `bande-runner-kvm` took jobs (the laptop was off), and every branch push started a full
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
   gave the polkit agent a toolkit dialog (`bande-toolkit.workspace = true`). A tag pushed
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
   artifact named `bande-debs` from the pinned run 40739365836 (Oct 28). The artifact is now
   called `bande-debs-arm64`, and the run's artifacts expire after 90 days.
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
- Software Update follows whichever `bande-apt-source(-testing)` package is installed. The
  mini can install `bande-apt-source-testing` and stop relying on hand-built `0.0.0+git`
  debs.

**`desktop/.github/workflows/release.yml`**, triggered by `push: tags: ['v[0-9]+.[0-9]+.[0-9]+']`
and `workflow_dispatch`:

| Job | Runner | Does |
|---|---|---|
| `lint` | `ubuntu-24.04-arm`, `container: debian:trixie` | fmt, clippy `-D warnings`, the dependency rules |
| `test` | `ubuntu-24.04-arm`, trixie container | `cargo test --workspace --locked` on lavapipe (`mesa-vulkan-drivers`), Debian 13 fonts from apt (the goldens are arm64/Debian 13), each test on its hermetic `crates/testing/test-bus` session bus (needs `dbus-daemon`) |
| `debs` | matrix `{arm64: ubuntu-24.04-arm, amd64: ubuntu-24.04}` | see below |
| `deb-all` | any | builds `bande-desktop` (all), keyring and source packages; lintian |
| `install` | matrix, native arch | `docker run debian:trixie` installs `bande-desktop` from the debs alone (`ci/deb/install-test`), plus the portal check |
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
- Uploads `bande-debs-<arch>`.

**Caching:** `Swatinem/rust-cache` keyed per arch and job, saved only from `main` (`save-if`),
since tags can't share caches. Watch the 10 GB repository limit. No sccache on hosted runners.

**Versioning:**
- `ci/deb/version.sh` stays. A tag must equal the `[workspace.package] version`, so a release
  means bump, commit, tag.
- The release job writes a `debian/changelog` stanza for `bande-desktop` from the tag with
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
  - `apt-get install bande-desktop`;
  - assert `dpkg-query -W -f '${Architecture}' bande-compositor` equals
    `dpkg --print-architecture`, and that the version equals the published version;
  - run `bande-compositor --headless` on lavapipe.

## Implementation plan

Each milestone takes about 1 to 2 hours of agent work. Verification is local on the mini (no
Docker locally) unless it says CI. Order: D1, A1, D2, D3, A2, A3, D4, D5, I1, R1, then the
first tag (M-final).

### desktop

**D1. Make `main` release-clean.**
- Fix the lint rule failure: exempt `bande-polkit` as an agent with UI, or move it under
  `crates/shell/`. This is an owner decision; default to exempting it.
- Add `bande-polkit` to `bande-desktop` Recommends. It is packaged but nothing pulls it in.
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
  `bande-debs-{arm64,amd64}` artifacts are produced with matching `Architecture` fields, and
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
  `[self-hosted, bande]` and `[self-hosted, bande-kvm]`, not gating).
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
- Require `bande-debs-arm64`, `bande-debs-amd64` and `bande-debs-all`. The publisher
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
- `build.yml` installs `bande-desktop` from the apt stable channel (or downloads the desktop
  release assets of a given tag) instead of the `bande-debs` artifact of run 40739365836.
- Verify: on the owner's go-ahead, a dispatch on the `bande-kvm` runner (it boots a guest),
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
  architectures and `verify` is green; then `apt install bande-desktop` on the mini from
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

1. Make `bande-linux/desktop` public (4 vCPU, 16 GB, free minutes), or keep it private and
   use 2-vCPU hosted runners with paid minutes beyond 2000 per month?
   Settled: the repository is public.
2. Release gate: lint, lavapipe tests, packages and install tests, with the KVM guest, perf
   and polkit moved to a non-gating dispatch workflow. Agree?
   Done that way (D5; plan recommendation; owner may revisit).
3. Testing channel: manual `main` dispatch only, or also nightly when `main` changed?
   Superseded: one `rolling` suite fed by tags only; a `main` dispatch is a test build that
   is never published.
4. `dist` profile: keep fat LTO if it fits in memory, or switch to thin LTO for reliable builds
   on 8 GB runners?
   Decided overnight 2026-11-01: keep fat LTO, since the public runners have 16 GB; switch to
   thin if the first tag's `debs` job is killed (plan recommendation; owner may revisit).
5. `bande-polkit` and the lint rule: exempt it as an agent with UI, or move the crate under
   `crates/shell/`?
   Done: exempted in `ci/release/lint.sh` (plan default).
6. Keep the reviewer approval for every stable publish?
   Open: there is no approval today; a green tag publishes on its own. Add a required reviewer
   to apt's `apt-publish` environment to get one back.

Sources:
- GitHub-hosted runner specs (private repos: 2 vCPU, 8 GB, 14 GB; arm64 available):
  https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- Runner pricing: https://docs.github.com/en/billing/reference/actions-runner-pricing
- Cache scope (tags can't share caches; 7-day eviction; 10 GB per repository):
  https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching
- Debian repository format: https://wiki.debian.org/DebianRepository/Format
