# Gated Debian testing updates

Switch in Software Update, "Only install Debian updates tested with Bande". Bande CI tests
bande-desktop against Debian testing as of time T. If the test passes, T becomes a **gate**.
With the switch on, Debian packages never move past the newest gate. Bande packages stay live.

## Findings (measured 2026-11-02 on the mini, unprivileged apt with scratch dirs)
- apt 3.3 resolves snapshots natively. `APT::Snapshot "T"` in apt.conf.d maps every source whose
  Origin is Debian (any mirror) or whose Label is Debian-Security to
  `snapshot.debian.org/archive/<archive>/T/`. Sources it can't map stay live: the Bande archive,
  Docker, VS Code. So one drop-in file gates Debian without touching the user's sources.list.
  This is in libapt-pkg, which PackageKit's apt backend links (libapt-pkg.so.7.0).
- A snapshot's Release expires 7 days after T. `Acquire::Min-ValidTime::Debian` (and
  `::Debian-Security`) extends that for Debian only. Measured: a 30-day setting accepts a
  14-day-old snapshot and refuses a 60-day-old one, which bounds how stale a gate can get.
  Don't use the global `Check-Valid-Until=false`.
- Fetch speed was 13 MB/s, 32 MB of indices in 2-3 s. snapshot.debian.org publishes no usage
  policy. Its Netfilter limits were replaced by HTTP-level throttling, and it has a history of
  throttling and outages. Every .deb also comes from snapshot, not the CDN.
- `testing-security` is empty: 0 arm64 packages today. Testing gets security fixes by migration
  from unstable, so a fast track has to come from CI, not from a live security source.
- Rejected: version pins on the live mirror (superseded debs leave the pool within about a day),
  Valid-Until/Date tricks (a live mirror can't serve an old Release), and a Bande mirror on Pages
  (1 GB site cap, 100 GB/month). An R2/S3 mirror is the path if we outgrow snapshot: about
  150-200 GB, roughly $3/month on R2 with free egress, plus a weekly sync job. The client wouldn't
  change: override `Acquire::Snapshots::URI::Origin::Debian`.

## Design
- **The gate is a package.** `bande-gate` (all, version `1.<T>`) is in apt `main`, built by the
  apt repo and owned by `apt` in state/owners. Its postinst writes
  `/etc/apt/apt.conf.d/50bande-gate` (`APT::Snapshot`, `Min-ValidTime::Debian{,-Security}` set to
  21 days past T) and its postrm deletes that file. The archive signature authenticates it, so
  there's no new signing, helper or polkit rule. Switch on/off = install/remove through
  PackageKit, which handles the password.
- **Turning it on when the computer is already newer:** postinst holds the snapshot at
  max(gate, Date of the cached testing InRelease). The computer stays at what it already has and
  gets nothing from Debian until a gate passes that date. Nothing is downgraded and nothing is
  left without a source.
  Off: the file is removed, and the next check moves to live testing (warned as one big catch-up).
- **Client order:** if `bande-gate` is among the updates, Settings (and automatic updates)
  installs it first, refreshes, and then resolves the rest. That's about 40 lines in
  `settings/src/software.rs` `update`/`update_automatically`. Without this the gate lags one daily
  check. The live/boot/offline classes and idle install are unchanged. Updates arrive in weekly
  batches, so the offline class (libc6/systemd) and its restart come roughly weekly.
- **CI** is a new `apt/.github/workflows/gate.yml` that follows follow-debian. It runs daily on a
  schedule but exits in seconds unless (7 days since the last gate) or (the security tracker
  shows a CVE fixed in testing after the gate that also has a stable DSA: the fast track). It
  also has a dispatch. Matrix amd64+arm64 in `debian:testing` with `APT::Snapshot=T` (T = run
  start; immutable afterwards), two parts:
  1. A fresh install of bande-desktop.
  2. An upgrade from the previous gate to T, which is the transition users actually make.

  Then the existing smoke (headless compositor, plus shell components on a private bus for 10 s).
  On green it uploads `gate-T`. `find-runs.sh`/`publish.yml` pick that up as a fourth source,
  like kernel runs, and build and publish `bande-gate`.
- **Bande packages (main, apple) stay live.** They aren't snapshot-capable, so this happens on
  its own. Fixes must not wait a week, and the mini verifies them before release. Risk: a
  release built in live `debian:testing` gets shlib deps newer than the gate, so apt holds it
  back. Fix: desktop `release.yml` and apps `build.yml` build with the current gate's
  `APT::Snapshot` (about 10 lines each).
- **Settings UX** (`settings-panes/registry.rs:1488`): a second switch under Automatic updates,
  "Only install Debian updates tested with Bande". Description: "Debian testing changes daily.
  This computer installs Debian updates only up to the date Bande last tested, about once a week.
  Bande's own updates still arrive right away." Status line "Tested up to 26 October 2026".
  Warning text:
  - Gate older than 10 days (CI failing): "Bande hasn't tested newer Debian updates since 12
    October. Debian updates, including security fixes, are held at that date."
  - Held: "This computer already has Debian updates from 2 November; they continue once Bande has
    tested past that date."

  Hidden unless a Debian source with Suite testing is configured.

## Risks
- **snapshot outage or throttling:** the check fails as "Couldn't check" and nothing breaks.
  Load grows with users × weekly upgrades (hundreds of MB each); move to the R2 mirror when needed.
- **Stale gate:** at gate + 21 days apt refuses the snapshot and the check fails visibly. The
  10-day warning comes first. The signed package bounds a freeze attack.
- **Partial upgrades when switching:** handled by the hold-at-current rule. A snapshot is a
  consistent testing state, so it's never a mix.
- **Coverage:** containers don't exercise the Mac GPU, kernel or firmware, so "tested" ≠ tested
  on the mini.
- **Images:** `images/build.sh:16` defaults `SUITE=trixie`, but the archive targets testing, so
  the switch is hidden on image installs until that's resolved.

## Verify (on the mini)
Install a locally built `bande-gate`. Then check: `apt-cache policy` shows snapshot origins;
Settings' check and Update Now go through PackageKit to snapshot; switching on at a newer date
holds; switching off returns to live. Use one CI dispatch of gate.yml; dispatch runs are never
published. There are no new e2e tests: the CI gate is the test.

## Size
- M1, CI (apt: gate.yml, the find-runs gate source, the bande-gate build, tests/): ~350 lines.
- M2, client (postinst/postrm ~60, Settings switch + status + gate-first ~200 + ftl): ~1 day.
- M3, security fast-track probe (~80) and release builds against the gate (~20).

## Owner decisions
1. Allow a new **scheduled** workflow in apt (daily, a no-op on most days)? It is outside today's
   two exceptions. Recommend yes.
2. **snapshot.debian.org vs our own mirror:** recommend snapshot, after one email to the
   snapshot/DSA admins about expected load. Keep the R2 mirror as the scale path.
3. **Default:** off until a month of green gates, then on for new installs (macOS-like). Recommend.
4. **Stale gate:** hold and warn (recommend) rather than silently falling back to live.
5. **Fast-track rule:** "CVE fixed in testing that also has a stable DSA". Recommend; keep manual
   dispatch for anything else.
6. Build Bande releases against the gate (recommend) so live Bande packages always install on
   gated systems.
