# Software Update

## Summary

Software Update fails because PackageKit is not installed on the mini. The image and the first
desktop install used `--no-install-recommends`, and `packagekit` is only a Recommends. Settings
then shows a raw D-Bus error. Even with PackageKit, no Bande package can update: they are local
builds, no Bande apt source is configured, and the repository has never published. The plan:
install PackageKit in the image, handle its absence in Settings, move the PackageKit code into a
shared crate, and switch to offline updates (install at restart). Offline updates need no
password, so the PackageKit polkit rule can go.

## What the owner sees

Settings ▸ Software Update shows "Could not check for updates". Under it is the raw error,
`org.freedesktop.DBus.Error.ServiceUnknown: The name org.freedesktop.PackageKit was not provided
by any .service files`, and a Try Again button that fails the same way. The daily check
(`bande-update-check.timer`) fails without a message, so no notification ever appears. I worked
this out from the code path and the live `busctl` error. I did not take a screenshot.

## Findings

### 1. Machine setup (the main cause): PackageKit is missing

- `dpkg-query -W packagekit` → `not-installed`. `busctl --system introspect
  org.freedesktop.PackageKit /org/freedesktop/PackageKit` → "was not provided by any .service
  files". `/var/lib/PackageKit` does not exist. The candidate `1.3.1-1+deb13u1` is in trixie.
- `packagekit` is only a Recommends of bande-settings (`crates/apps/settings/Cargo.toml:55`).
- The first desktop install was `apt-get install -y --allow-downgrades --no-install-recommends
  ./bande-*.deb` (`/var/log/apt/history.log`, 2026-10-30 14:53). Later reinstalls dropped the
  flag, but on an upgrade apt only adds Recommends that are new in that version, so packagekit
  never came in. `power-profiles-daemon`, another Recommends of Settings, is missing for the
  same reason.
- The image does the same thing: `images/customize.sh:37` installs the desktop with
  `--no-install-recommends`, and `images/build.sh:44-48` does not list packagekit. Every image
  ships without PackageKit.
- The machine is otherwise ready for offline updates: `systemd-system-update-generator` and
  `system-update.target` are present. Debian's packagekit ships `packagekit-offline-update.service`
  in `system-update.target.wants` (packages.debian.org file list). Plymouth is not installed.
- polkitd and the Bande agent are running (`busctl --system list`: `bande-polkit` on `:1.330`).

### 2. Code bugs in Settings

| Where | Bug | Effect |
|---|---|---|
| `crates/apps/settings/src/software.rs:210` | `check` turns any D-Bus error into `Failed(error.to_string())`. It does not use `failure::sentence` and has no "PackageKit is missing" state | the raw D-Bus text, and a Try Again that can never work |
| `crates/apps/settings/src/privilege.rs:375-386` | one `?` inside the loop over all four actions: an action that is not registered aborts the whole check | journal, three times today: `cannot ask polkit: … Action org.freedesktop.packagekit.system-update is not registered`. Time, language and computer name are then never marked refused for non-administrators |
| `software.rs:117-168` | the signal loop ends only on `Finished`, with no timeout and no watch on the daemon | if packagekitd crashes or is restarted mid-transaction, the spinner turns forever |
| `crates/apps/settings/src/updates.rs:143-146` | there is one `Failed` state, with the title "Could not check for updates" | a failed install is reported as a failed check |
| `software.rs:216` | a live `UpdatePackages(ONLY_TRUSTED)` | needs `org.freedesktop.packagekit.system-update` (`auth_admin_keep`), so it depends on our password-free rule (`data/50-bande-settings.rules:9`), which `docs/debian-readiness.md:262,322` says to justify or drop. It also replaces the running compositor and shell with nothing restarted afterwards (maturity-gaps §12) |
| `software.rs:122` | ignores the `info` of each `Package` | updates the backend marks `BLOCKED` (info 9) would be offered and then fail in `UpdatePackages` |
| `main.rs:230`, `view.rs:1223` | every Settings launch runs `RefreshCache` | an apt index refresh each time Settings opens, which also holds the apt lock (see Risks) |

The constants are right (checked against `pk-enum.h` 1.3.1): `FILTER_NONE = 1<<1`,
`ONLY_TRUSTED = 1<<1`, restart 4 and 6, exit 1. The signal signatures match
`org.freedesktop.PackageKit.Transaction`. With PackageKit installed, the check would work. Today
it would say "Your computer is up to date", because `apt list --upgradable` is empty.

### 3. The repository has never published

- Every Bande package is a local build, version `0.0.0+git20261031.1083.4b292c7`.
  `apt-cache policy bande-settings` lists only `/var/lib/dpkg/status`. The six
  `linux-image-7.3.0-bande-*` kernels are local builds too.
- `/etc/apt/sources.list.d/` holds only `claude-desktop.list` and `vscode.sources`.
  `bande-archive-keyring` and `bande-apt-source` are not installed.
- `https://bande-linux.github.io/apt/` and `dists/trixie{,-testing}/InRelease` return 404.
  `gh run list -R bande-linux/apt -w publish.yml` shows no runs. The last apt `check` run
  ("Publish verified desktop builds…", 2026-10-29) failed. Desktop has no `v*` tags.
- `apt/README.md` says the testing channel is fed by "every green push to main". Since
  2026-10-31, desktop CI runs only on tags, so nothing feeds testing any more.
- The versions do line up: `ci/deb/version.sh` makes `0.0.0+git…` builds sort below `0.1.0`, and
  `0.1.0+git…` builds sort above it. Once the repository publishes and the mini follows it,
  upgrades will be clean.

**Root causes, in order:** (1) machine setup: PackageKit is not installed, because of
`--no-install-recommends` here and in the image; (2) code: the missing service is reported as a
raw error, plus the smaller bugs above; (3) repository: nothing publishes, so Software Update can
only ever offer Debian, Claude Desktop and VS Code updates. The third belongs to the CI/apt gap
item. This plan only states what it depends on.

## Options

1. **Installing only.** Install packagekit on the mini and in the image. The rest stays as it is.
   Cheap, but it keeps the raw errors, the stuck spinner, the live install of the running
   desktop and the polkit rule Debian will question.
2. **Fix in place inside `apps/settings`.** It fixes the bugs, but the .deb installer and the app
   store plans would each write their own PackageKit transaction code (CLAUDE.md rule 3).
3. **A shared `bande-packages` client crate, with offline updates (recommended).** One tested
   transaction runner and fake for all three users. Downloading with `ONLY_DOWNLOAD` needs no
   authorization (`pk_transaction_obtain_authorization`, `src/pk-transaction.c` in
   1.3.1-1+deb13u1). `Offline.Trigger` needs `trigger-offline-update`, which is `allow_active=yes`
   upstream. So any active local user can update without a password, and
   `org.freedesktop.packagekit.system-update` leaves our rule. This is how GNOME Software works
   on Debian. The cost: every update needs a restart.
4. **Option 3, keeping live install as well** for administrators, with a password prompt. More
   states and more tests, and a running desktop gets replaced underneath itself. Not worth it now.

## Recommendation

Option 3. Fix the machine and the image first, since that alone makes the page work. Then build
the shared crate, the plain states and offline updates. The Bande packages themselves start
updating once the CI/apt item publishes a channel and the mini installs `bande-apt-source` or
`bande-apt-source-testing`.

### Shared PackageKit client (for Settings, the .deb installer and the app store)

This is the same crate and the same split as `NOTES/plans/app-store.md` (its milestone 2). The
plan that lands first creates the crate and moves the code out of `software.rs`. The others add
their calls to it. The app store plan later adds its Flatpak backend (`flatpak.rs`) here too.

The crate `crates/libs/packages` (`bande-packages`) uses blocking zbus, like `libs/network`.
Every PackageKit D-Bus call lives in this crate, because PackageKit 2.0 (planned for the second
half of 2026) breaks the whole API, D-Bus clients included (see app-store.md, Findings). trixie
stays on 1.3.x. When Debian moves, only this crate and its fake change. It has these parts:

- **`Transaction`.**
  - `CreateTransaction`, then `SetHints` (`interactive=true|false`, `background=true` for the
    timer, `locale=…`).
  - It collects `Package`, `Details`, `RequireRestart`, `ErrorCode` and `Percentage` until
    `Finished`.
  - It also ends with `Error::Vanished` when `NameOwnerChanged` shows packagekitd gone.
  - It offers a `cancel()` handle (`Transaction.Cancel`).
- **Types.**
  - `Package { id, name, version, arch, origin, info, summary }`, with `Info` as an enum
    (`Security`, `Blocked`, …).
  - `Failure { kind, details }`, where `kind` comes from the PackageKit error code: `NoNetwork`,
    `Locked`, `NoSpace`, `NotAuthorized`, `Cancelled`, `Missing`, `Other`.
- **Availability.** `available(system)`: is `org.freedesktop.PackageKit` among the activatable
  or active names.
- **Updates (this plan).**
  - `refresh(force)`, `updates()` (with `Blocked` filtered out), `details(ids)` and
    `download(ids)` (`UpdatePackages` with `ONLY_TRUSTED|ONLY_DOWNLOAD`).
  - `offline::{prepared, trigger, cancel, results, clear_results}` on
    `org.freedesktop.PackageKit.Offline`.
- **The .deb installer** (the .deb installer plan, written in parallel, in `NOTES/plans/`).
  - `install_files(paths)` takes `SIMULATE` first, to list the extra packages it would install,
    then the real install.
  - Local .deb files are untrusted, so it must not pass `ONLY_TRUSTED`. It then needs
    `package-install-untrusted` (`auth_admin`) through `bande-polkit`.
- **The app store** (`NOTES/plans/app-store.md`, milestone 2):
  - `resolve(names)`, `installed()` (GetPackages with the installed filter), `install(ids)` and
    `remove(ids)`;
  - progress through a callback, the same one `download` uses.
- **Testing.** The fake moves out of `apps/settings/tests/updates.rs` into
  `crates/testing/test-bus/src/packagekit.rs` as `FakePackageKit`, next to `FakeBluez`. It
  scripts packages, errors, a daemon that disappears, and the offline state. It records the
  flags and calls it receives.

Each part lands only when its first user lands (YAGNI). This plan adds the updates part. The
installer and store plans add theirs to the same crate and fake.

## Implementation plan

**Status (2026-11-01):** milestone 3 (plain states) is on `main`. The shared PackageKit crate and milestones 2 and 4-7 are separate work.


Each step is a short branch from `main`, verified on the mini and passing the full local gate.

1. **PackageKit in the image and on the mini.**
   - `images/build.sh:44-48`: add `packagekit` to `packages`.
   - `images/customize.sh`: an `BANDE-TEST-PACKAGEKIT` line that introspects
     `/org/freedesktop/PackageKit`.
   - `images/vm-test.sh`: `check "BANDE-TEST-PACKAGEKIT: .*org.freedesktop.PackageKit.Offline"`.
   - On the mini: `sudo apt install packagekit`. Prove it with `busctl introspect` and with
     Settings ▸ Software Update showing "Your computer is up to date".
   - Note the other dropped Recommends (`power-profiles-daemon`) for the owner.
2. **The `bande-packages` crate and `FakePackageKit`.** This is the same step as app-store.md
   milestone 2. Whichever plan runs first does it, and the other plan adds only its own calls.
   - New: `crates/libs/packages/{Cargo.toml,src/lib.rs}` and
     `crates/testing/test-bus/src/packagekit.rs`.
   - `software.rs` keeps its behaviour, but calls the crate.
   - `tests/updates.rs` uses the shared fake.
   - Tests in the crate:
     - updates listed with sizes, and `Blocked` dropped;
     - an `ErrorCode` for no network becomes `Failure::NoNetwork`;
     - packagekitd leaving mid-transaction returns `Vanished` within `WAIT`. This hangs today.
3. **Plain states in Settings.**
   - `software.rs`, `updates.rs`, `general.rs`, `header.rs`, `locales/*/bande-settings.ftl`:
     - `SoftwareState::Unavailable` gets the row "Updates aren't available on this computer",
       with the detail "Install the packagekit package", and no Try Again.
     - `Failed { during: Check | Download, kind }` gets a matching title and a plain sentence,
       with no raw D-Bus text.
   - `privilege.rs`: check each action on its own; an unregistered action no longer hides the
     others.
   - Tests:
     - `tests/updates.rs`: no PackageKit on the bus shows the Unavailable row; a NoNetwork error
       shows its sentence.
     - `tests/privilege.rs`: a fake Authority that errors on the PackageKit action still reports
       Hostname as refused.
4. **Offline updates: download, then restart to install.**
   - `software.rs`, `updates.rs` and the strings:
     - Install Now becomes Download & Restart: `download(ids)` with progress, `Offline.Trigger
       ("reboot")`, then the state `Prepared(n)` with "Restart to install n updates" and
       Restart Now.
     - At startup, Settings reads `UpdatePrepared` and `UpdateTriggered`, so a prepared update
       shows as prepared.
   - Polkit:
     - Remove `org.freedesktop.packagekit.system-update` from `data/50-bande-settings.rules`.
     - Remove `Privilege::Updates`.
     - Update the `extended-description` in `Cargo.toml:49-54` and
       `docs/debian-readiness.md:262,322`.
   - Tests:
     - The fake records `UpdatePackages` flags equal to `ONLY_TRUSTED|ONLY_DOWNLOAD` and one
       `Trigger("reboot")`.
     - The view reaches Restart Now and sends no reboot unless clicked.
     - A prepared update at startup shows as prepared.
5. **Results after the restart.**
   - In `software.rs` and `main.rs` (`--check-updates`): `Offline.GetResults`. Success brings
     the notification "n updates were installed"; failure brings a notification and the Failed
     row with its sentence. Then `ClearResults`.
   - Tests with the fake: success gives the state `Installed`, and results are cleared exactly
     once; failure gives `Failed` with the mapped kind.
6. **Cheaper checks.**
   - `software.rs` and `main.rs`:
     - Opening Settings skips `RefreshCache` when `GetTimeSinceAction(RefreshCache)` is under
       an hour. Check Now and the daily timer always refresh.
     - The timer run sends `background=true` and `interactive=false`.
   - Test: the fake counts `RefreshCache`. Two opens give one refresh; Check Now gives a second.
7. **End-to-end on the mini, and docs.**
   - The real flow: a check against the trixie, Claude Desktop and VS Code repositories, then
     download, trigger and reboot. Ask main before the reboot (visible disruption).
     `journalctl -b -u packagekit-offline-update` must show success, and Settings must show the
     result.
   - Update `docs/experience/settings.md` (the Software Update row, §12) and
     `docs/experience/maturity-gaps.md:200`.

Seven milestones. Step 1 alone fixes what the owner sees today.

## Risks

- **The offline update screen.** Without plymouth, the restart shows console text or a black
  screen for minutes. It needs a check on the mini's direct boot chain. A later step could show a
  plain progress screen.
- **The apt lock.** packagekitd holds the apt lock during refresh and download. Agents running
  `sudo apt-get install ./target/debian/*.deb` on the mini can collide with the daily check or
  with a Settings open. Step 6 reduces this; dev scripts should wait for the lock.
- **Kernel updates offline.** Kernel postinst hooks regenerate `boot.bin`/initrd while running
  under `system-update.target`. This must be verified once with a kernel update.
- **Local builds against the repository.** A local build and a repository build of the same
  commit have the same version but different checksums. apt treats them as the same version.
  Fine, but confusing when checking which build is installed.
- **PackageKit 2.0 breaks the D-Bus API.** Every call stays inside `bande-packages`. The
  offline update interface may change as well, so step 4 needs to be checked again when Debian
  moves to 2.0.
- **Bande packages do not update** until the CI/apt item publishes a channel and the mini is
  pointed at it.

## Decisions for the owner

1. **Offline only, or live install too?** Offline (install at restart, no password, like GNOME
   and macOS) is recommended. The alternative keeps a live install for administrators, behind a
   password prompt and our polkit rule.
2. **Which Bande packages the mini follows.** It can follow the repository's testing channel
   once it publishes, or stay on local builds, where Software Update covers only Debian and the
   third-party repositories. This depends on the CI/apt item. The testing channel's "every green
   push to main" feed no longer runs now that CI is tag-only.
