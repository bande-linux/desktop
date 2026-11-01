# Plan: install .deb files from Files

Gap (owner): "its not possible to install .deb files from Files, this needs an install tool like
most distros provide" (`NOTES/gaps.md:9`).

## Summary

Build a small **Package Installer** app (`bande-installer`) that opens `.deb` files. It reads
the file with `dpkg-deb`, checks the architecture itself, previews the change with a simulated
PackageKit `InstallFiles`, then installs, upgrades, downgrades or removes through PackageKit
with the existing polkit agent asking for the password. It uses the shared PackageKit client
`crates/libs/packages` that the app-store plan defines. 7 milestones.

## What the owner sees

- Double-clicking a `.deb` in Files opens the Open With sheet with no recommended app. Text
  Editor or Firefox are the only choices, and neither installs anything.
- Firefox's "open" on a downloaded `.deb` fails the same way, because no desktop file claims
  `application/vnd.debian.binary-package`.
- The only way to install is `sudo apt install ./file.deb` in Terminal.

## Findings

**Nothing handles the MIME type.**
- `crates/apps/files/src/view.rs:1709-1752`: `open()` asks `Apps::default_for(mime)`. If no
  app is found, the file goes to `unknown` (`:1743`) and on to `ask_open_with` (`:1752`).
- `.deb` is not an archive to Files either. `crates/libs/fs/src/ops/archive.rs:40-53`
  (`SUFFIXES`) has no `.deb`, which is correct.
- `packaging/bande-desktop/bande-mimeapps.list` has no `.deb` entry. No file in
  `/usr/share/applications` lists that MIME type. The MIME database knows the type
  (`/usr/share/mime/globs2`: `*.deb`, `*.udeb`).
- `Apps::apps_for`/`default_for` (`crates/libs/xdg/src/apps.rs:78-86`) also see `NoDisplay`
  handlers. `openers()` (`:88-93`) hides them. So an installer with `NoDisplay=true` and a
  `MimeType=` line still becomes the default without cluttering the launcher.

**PackageKit is not on the machine.**
- `dpkg -l packagekit` says not installed (trixie candidate 1.3.1-1+deb13u1).
- `bande-settings` only Recommends it (`crates/apps/settings/Cargo.toml:55`).
- Every dev install on the mini used `apt-get install --no-install-recommends ./target/debian/*.deb`
  (`/var/log/apt/history.log`). So do the images (`images/customize.sh:37`).
- The app-store plan (`NOTES/plans/app-store.md`) found the same problem. The installer can't
  work without PackageKit, so it must **Depend** on `packagekit`. A Recommends is not enough,
  because the images install our own debs explicitly but skip Recommends.

**A PackageKit client exists, inside Settings.**
- `crates/apps/settings/src/software.rs:102-170` (`run`): CreateTransaction, a signal match
  on the transaction path, `SetHints(interactive=true)` (`:114`), and handlers for Package,
  Details, RequireRestart, ErrorCode, Percentage and Finished.
- It keeps only the ErrorCode *text* (`:141-145`), not the code. The installer needs the code to
  tell dependency, lock, auth and file errors apart.
- `crates/apps/settings/tests/updates.rs:38-130` has a fake PackageKit on the hermetic bus.
- The app-store plan, milestone 2, moves this into `crates/libs/packages` (`bande-packages`)
  with `resolve`, `installed`, `install`, `remove`, `install_files` and a progress callback.
  The fake moves to `crates/testing/test-bus/src/packagekit.rs`. This plan uses that crate and
  that boundary. It adds only what local files need (milestone 2).

**How PackageKit 1.3.1 (trixie) handles local debs** (upstream source, tag `v1.3.1`):
- API: `GetDetailsLocal(as files)`, `GetFilesLocal(as files)`, `InstallFiles(t flags, as paths)`.
  Upstream main, after 1.4.0, switches these to file handles (`ah`). 1.4.0 (2026-09-09) still
  takes paths.
- The apt backend adds the file to apt's cache as a volatile source (`AddVolatileFile`). Apt
  then resolves dependencies from the configured repositories, which is exactly what
  `apt install ./x.deb` does (`backends/apt/apt-job.cpp`, `markFileForInstall`,
  `resolveLocalFiles`).
- **Local files are always untrusted.** With `ONLY_TRUSTED` set, `AptJob::init` fails at once
  with `CANNOT_INSTALL_REPO_UNSIGNED` ("Local packages cannot be authenticated"). So the
  installer must call without that flag. polkit then checks
  `org.freedesktop.packagekit.package-install-untrusted`, which is `auth_admin` (no keep): a
  password every time (`src/pk-transaction.c`, `pk_transaction_role_to_actions`).
- **Simulation needs no authorisation.** `SIMULATE` skips polkit
  (`pk_transaction_obtain_authorization`). A simulated InstallFiles emits one `Package` signal
  per change: info 12 installing, 11 updating, 20 downgrading, 13 removing. On failure it emits
  `ErrorCode` 13 `DEP_RESOLUTION_FAILED` with apt's broken-package text. This is the preview.
- **Wrong architecture gives a useless error.** A foreign, unconfigured architecture never
  enters apt's cache. `resolveLocalFiles` then returns nothing and the job fails with
  `PACKAGE_NOT_FOUND` "Could not find package(s)". So the installer must check the
  architecture before calling PackageKit. On this Mac that is the common case: Chrome, Slack and
  many vendor debs are amd64 only.
- `InstallFiles` has no downgrade guard. `ALLOW_DOWNGRADE` is checked only for
  Install/UpdatePackages, so an older local file simply downgrades. The UI has to warn.
- Removal: `RemovePackages(t flags, as ids, b allow_deps, b autoremove)`, action
  `package-remove` (`auth_admin_keep`).
- debconf questions get the noninteractive defaults unless the client passes a
  `frontend-socket` hint. Conffiles keep the old version (`--force-confold`).
- Error codes used below: 9 `PACKAGE_ALREADY_INSTALLED`, 13 `DEP_RESOLUTION_FAILED`,
  20 `CANNOT_REMOVE_SYSTEM_PACKAGE`, 26 `CANNOT_GET_LOCK`, 29 `LOCAL_INSTALL_FAILED`,
  38 `INVALID_PACKAGE_FILE`, 45 `INCOMPATIBLE_ARCHITECTURE`, 48 `NOT_AUTHORIZED`,
  50 `CANNOT_INSTALL_REPO_UNSIGNED`. Exit 1 success, 3 cancelled.

**polkit.**
- The agent `bande-polkit-agent` runs (`systemctl --user is-active`) and shows polkit's
  message (`crates/services/polkit/src/agent.rs:312`).
- `crates/apps/settings/data/50-bande-settings.rules:9` grants only `system-update`. That is
  the open decision in `NOTES/resume-2026-10-31.md:61` and `docs/debian-readiness.md:262`. This
  plan does not touch it and needs no new rule.
- Debian patches PackageKit's policy (`debian/patches/policy.diff`: some `auth_admin_keep` become
  `auth_admin`, some `yes` become `auth_admin_keep`, and the rule covers `upgrade-system` and
  `trigger-offline-update` for `sudo`). After installing packagekit on the mini, confirm the
  effective defaults with `pkaction --verbose`.

**What others ship.**
- GNOME Software and KDE Discover: GetDetailsLocal, then InstallFiles through PackageKit. Ubuntu
  App Center had it removed, then restored it in 24.04 after complaints
  ([ubuntuhandbook](https://ubuntuhandbook.org/index.php/tag/deb/),
  [linuxiac](https://linuxiac.com/ubuntu-24-04-finally-fixed-an-annoying-app-center-issue/)).
- gdebi: python-apt run as root through pkexec, so it duplicates apt.
- `apt install ./x.deb` gives the same result as PackageKit's apt backend.
- macOS: a separate Installer.app opens `.pkg` files (intro, licence, install, summary). It is
  not the App Store.

## Options

| Option | For | Against |
| :--- | :--- | :--- |
| A. Own installer on PackageKit (shared `bande-packages`) | apt's own dependency resolution; no root code of ours; progress, cancel and errors come with it; polkit and the agent already exist; same client as Software Update and the store | depends on PackageKit; wrong-architecture and dependency errors need our own wording; the API moves to file handles in a later release |
| B. Own root helper (pkexec + `apt-get install ./x.deb`, parse the output) | no PackageKit | we would write and audit a privileged helper and our own polkit action; parse apt's human output; no structured progress; a second package path next to Software Update |
| C. Ship gdebi or GNOME Software for `.deb` | no code | GTK/Adwaita look; gdebi is barely maintained; GNOME Software brings its own update engine (store plan, option B) |
| D. A sheet inside Files | no new app | Files would depend on PackageKit; Firefox and other apps open `.deb` through the default handler and need a desktop file anyway |

## Recommendation

Option A, as a standalone app:
- **`crates/apps/installer`** builds package `bande-installer` with binary
  `bande-installer` and desktop file `org.bande.Installer.desktop`: `NoDisplay=true`,
  `MimeType=application/vnd.debian.binary-package;`, `Exec=bande-installer %f`. Each file
  gets its own window. PackageKit queues concurrent transactions.
- `Depends: packagekit, bande-polkit`.
- `bande-desktop` Recommends it, and `bande-mimeapps.list` makes it the default.

The flow:
1. **Read.** `dpkg-deb --show --showformat=…` reads Package, Version, Architecture, Maintainer,
   Description, Installed-Size and Homepage. If it fails: "This file isn't a valid Debian
   package." That also catches broken downloads.
2. **Architecture.** Accept `all`, the output of `dpkg --print-architecture`, and the output of
   `dpkg --print-foreign-architectures`. Anything else is refused before PackageKit is called:
   "“Google Chrome” is built for Intel and AMD computers (amd64). This computer needs a package
   for 64-bit ARM (arm64)." Then a hint to look for an arm64 download.
3. **Installed state.** Check with `dpkg-query -W` on `name:arch`. If the same version is
   installed, say "Version X is installed" and offer **Remove…** and **Open**. There is no
   reinstall.
4. **Preview.** Simulate `InstallFiles` (no password needed). Show:
   - the action: Install, or "Upgrade from 1.2 to 1.3", or **Downgrade** with a caution line,
     "This is older than the installed version 1.4";
   - "Also installs N packages" (names in a disclosure);
   - **any removals, as a warning that needs explicit confirmation**;
   - for `DEP_RESOLUTION_FAILED`: "Can't install: it needs packages that aren't available",
     then the missing requirements parsed from apt's "Depends: X but it is not installable"
     lines, with the raw text in a disclosure.
5. **Trust.** Every local file gets a caution box. Debs have no usable signature, so the warning
   is about where the file came from: "This package doesn't come from your software sources.
   Its install scripts get full control of this computer, and it may add its own software
   source. Install it only if you trust where you got it." Show the Maintainer and Homepage
   fields. The primary button keeps its normal style. The password prompt is the confirmation.
6. **Install.** `InstallFiles(0, [path])` with `interactive=true`. polkit asks for the
   password through our agent. Show a progress bar from `Percentage`, and a line from `Status`
   (Downloading, Installing, Running scripts). **Cancel** while `AllowCancel` is true.
7. **Done.** Show "Installed". Find the package's desktop files with `GetFilesLocal`, filtered
   to `usr/share/applications/*.desktop` that are not NoDisplay, and offer **Open** through
   `bande-xdg`. If a reboot is required, show "Restart to finish".
8. **Errors**, as plain sentences:
   - auth refused or dismissed (48): back to the preview, no alarm;
   - lock (26): "Another installation is running. Try again when it finishes.";
   - 29 or 38: "The package couldn't be installed", with apt's details in a disclosure.
9. **Remove.** `resolve([name])` with the installed filter gives the id. Simulate
   `RemovePackages(allow_deps=true, autoremove=false)` and list what else would go. Confirm,
   then apply. For 20 (essential): "This package is part of the system and can't be removed."
   General uninstall of apps belongs to the store's Installed view and to Settings
   (`docs/experience/settings.md:589`). This button covers only the package in the file.

**Shared client** (shared with the software-update and app-store plans; same crate and
boundary as `NOTES/plans/app-store.md` milestone 2):
- The store's API: `resolve`, `installed`, `install`, `remove`, `install_files`, progress
  callback.
- What this plan adds:
  - `details_local(path)`, `files_local(path)`;
  - a `Mode::{Simulate, Apply}` argument on `install_files` and `remove`, returning
    `Changes { packages: Vec<(Info, Package)> }`;
  - a typed `Failure { code: ErrorCode, details }`, with the ErrorCode enum covering the codes
    above plus `Other(u32)`;
  - `Progress { percent, status, allow_cancel }` and `cancel()`.
- Settings keeps its behaviour. The plan that lands first creates the crate and the others
  extend it.

## Implementation plan

**Status (2026-11-01):** milestones 1-6 are on `main`, and the wiring of 7 (default handler in
`bande-mimeapps.list`, Recommends in `bande-desktop`, the release package list and deb checks,
`docs/architecture/packaging.md`, a test that the shipped list opens `.deb` in the installer).
`crates/apps/installer` builds `bande-installer` on `bande-packages`. Differences from the plan:
the installed version comes from PackageKit's `Resolve(installed)` instead of `dpkg-query`, so the
fake can script it; there are no goldens yet; the Present state offers Remove and Done but not
Open. Left: the live checks on the mini in milestone 7 (an install asks for the owner's password
on screen), and the `#[ignore]` real-daemon tests of milestone 2. Checked read-only on the mini:
`GetDetailsLocal` on `~/Downloads/Discord.deb` (amd64) answers with `discord;1.0.168;amd64;local`,
so the architecture check has to come from `dpkg-deb`, as planned.

Each milestone is one branch from `main` and must pass the full gate (fmt, clippy `-D warnings`,
workspace tests on the hermetic bus). "Mini" means checked live on the M2 mini.

1. **Shared client `crates/libs/packages`.** This is store-plan milestone 2. Skip it if that
   plan or the software-update plan already landed it.
   - Move `run`, `Package` and the signal handling out of `crates/apps/settings/src/software.rs`.
   - Move the fake to `crates/testing/test-bus/src/packagekit.rs`.
   - Settings uses the crate.
   - Tests: the existing `crates/apps/settings/tests/updates.rs` passes unchanged; a fake
     `ErrorCode` becomes a `Failure` that keeps its code.
2. **Local files in the client** (`crates/libs/packages/src/{transaction,local}.rs`,
   `crates/testing/test-bus/src/packagekit.rs`).
   - `details_local`, `files_local`, `install_files(path, Mode)`, `remove(ids, Mode)`,
     `Changes`, `ErrorCode`, `Progress`, `cancel`.
   - Hermetic tests against the fake:
     - a simulated install that emits 12/11/20/13 packages classifies them as install,
       upgrade, downgrade and remove;
     - `install_files` never sets `ONLY_TRUSTED` (the fake records the flags; the real apt
       backend rejects that bit);
     - `Simulate` sets bit 2 and `Apply` does not;
     - code 13 becomes `ErrorCode::Dependencies`;
     - `Percentage` 101 is "unknown", not 101 %;
     - cancel calls `Transaction.Cancel`.
   - `#[ignore]` test "needs packagekitd", run on the mini (simulation needs no password):
     - a fixture deb built at test time with `dpkg-deb --build`, depending on a package that
       doesn't exist, gives code 13 and a details text with the missing name;
     - an amd64 fixture gives `PACKAGE_NOT_FOUND`, which shows the architecture pre-check is
       needed.
     - Record the real details format for the parser in milestone 3.
3. **Reading the file** (`crates/apps/installer/src/deb.rs`, the new crate's `Cargo.toml`).
   - `read(path) -> DebInfo` via `dpkg-deb --show`.
   - `fits(arch, native, foreign) -> Fit`.
   - `installed(name, arch)` via `dpkg-query`.
   - `missing_requirements(details)` parses apt's broken-package text.
   - Tests:
     - real fixture debs built in a tempdir with `dpkg-deb --build` parse to the expected fields;
     - a truncated copy is `Invalid`;
     - amd64 against native arm64 is refused and `all` fits;
     - amd64 fits when it is listed as foreign;
     - the recorded apt text from milestone 2 yields the missing names.
4. **Installer model and worker** (`crates/apps/installer/src/{model,worker,lib}.rs`,
   `locales/en-US/bande-installer.ftl`).
   - The state machine is Reading → Refused(reason) | Ready(preview) | Present(version) →
     Installing(progress) → Done | Failed, plus Removing.
   - The worker thread uses the settings `follow` pattern.
   - Hermetic tests against the fake:
     - a new package reaches Ready with "also installs 2";
     - an older file reaches Ready with the downgrade flag and the installed version;
     - a simulated removal in the preview needs confirmation;
     - code 48 during apply returns to Ready;
     - code 26 gives the lock sentence;
     - the same installed version reaches Present.
5. **Installer window** (`crates/apps/installer/src/{view,main}.rs`, `data/` desktop file and
   icon, `[package.metadata.deb]` with `Depends: $auto, packagekit, bande-polkit, fonts-inter`).
   - The layout: header (icon, name, version and size), summary, scrolling description, caution
     box, changes disclosure, buttons; a progress bar and Cancel while installing.
   - Tests:
     - accessibility-tree tests (as in `crates/apps/settings/tests/updates.rs`): Install starts
       an apply `InstallFiles` on the fake, the progress value follows `Percentage`, and Open
       appears when `files_local` lists a desktop file;
     - goldens for ready, refused-architecture, dependency failure and installing, light and
       dark (`bande-golden`).
6. **Remove and Open** (`model.rs`, `view.rs`, `worker.rs`).
   - The remove flow (resolve installed id, simulate, confirm, apply).
   - Open launches the found desktop entry through `bande_xdg::launch_with`.
   - Tests:
     - Remove lists a dependent package from the simulation before the apply call;
     - code 20 shows the system-package sentence;
     - Open picks the non-NoDisplay entry among two.
7. **Wire up, then check on the mini.**
   - `packaging/bande-desktop/bande-mimeapps.list` gets
     `application/vnd.debian.binary-package=org.bande.Installer.desktop;`.
   - `packaging/bande-desktop/debian/control` Recommends `bande-installer (= ${binary:Version})`.
   - `.github/workflows/ci.yml` gets the deb list (around `:102`, `:135`), an asset check like
     `:182-184`, and the Recommends check (`:312`).
   - `docs/architecture/packaging.md` gets the runtime stack line.
   - Test: extend the mimeapps test in `crates/libs/xdg/tests/mime.rs` so the shipped list
     resolves `.deb` to the installer.
   - Mini:
     - install packagekit and run `pkaction --verbose` for the three actions;
     - from Files: install an arm64 third-party deb (password asked, progress, Open works);
     - an amd64 deb is refused with the architecture sentence;
     - a fixture with a missing dependency shows the parsed requirement;
     - a newer fixture upgrades, then an older one downgrades with the caution;
     - Remove works;
     - Firefox's "open" on a downloaded deb lands in the installer.

## Risks

- **PackageKit API change.** Upstream main moves `InstallFiles`, `GetDetailsLocal` and
  `GetFilesLocal` to file handles. When Debian ships that, the client must pass fds, chosen by
  introspecting the argument signature. Debian 13 has 1.3.1, so not now.
- **apt's broken-package text is not an API.** The parser falls back to showing the raw text.
- **Maintainer scripts run as root with debconf defaults.** A package that needs answers
  (licence prompts) gets the defaults. Passing a `frontend-socket` is a later step.
- **Files the root daemon can't read**, such as FUSE mounts without `allow_other`, fail with
  29. Later, copy them to `/var/tmp` first.
- **Duplicate PackageKit work with two other plans.** Milestone 1 is shared. Agree on one
  owner for the crate before branching.
- **Later upgrades.** A local deb whose package also exists in a repository is later "updated"
  by Software Update to the repository version. That is expected apt behaviour.

## Decisions for the owner

1. **Standalone Package Installer** (recommended, like macOS's Installer.app), or the future
   Software store's "open a file" page. The store is about 9 to 15 agent hours away.
   Decided overnight 2026-11-01: standalone Package Installer (plan recommendation; owner may
   revisit).
2. **Password for local packages:**
   - keep polkit's `package-install-untrusted` = `auth_admin`, a password on every install
     (recommended: the file's scripts run as root);
   - or add it to the password-free `sudo` rule next to `system-update`.
   Decided overnight 2026-11-01: keep `auth_admin`, a password on every install (plan
   recommendation; owner may revisit).
