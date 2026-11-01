# Live updates: restart only when an update needs it

**Owner report.** Software Update always says "Restart to install". Linux rarely needs that.

**Cause.** This is a design choice, not a bug. `software.rs:168-185` sends every update through `download()`
(`UpdatePackages` with `ONLY_DOWNLOAD`, `libs/packages/src/lib.rs:354`) and then `offline::trigger("reboot")`.
`updates.rs:120-128` then offers only Restart Now. The client never reads what an update needs: it calls
`GetDetails` (sizes) and never `GetUpdateDetail`, and `transaction.rs:128-131` reduces `RequireRestart` to a single bool.
**Why it is offline-only** (`git show 37b326aa:NOTES/plans/software-update.md`, Options 3/4, decided overnight
2026-11-01, "owner may revisit"): (1) `trigger-offline-update` is `allow_active=yes`, so no password and no
Bande polkit grant; (2) the old live `UpdatePackages` replaced the running compositor and shell and restarted
nothing; (3) it meant fewer states. Reason 1 was weaker than the plan assumed. Debian's packagekit 1.4.0 ships
`/usr/share/polkit-1/rules.d/org.freedesktop.packagekit.rules`, which gives `system-update` to active local sudo/wheel
users with no password. So a live install needs no Bande rule for admins. Reason 2 is real, and this plan solves it.
Also, the offline path itself is still unverified on the mini (milestone 7 is open).
**Safe to keep:** the `bande-packages` crate, the fake, download/trigger/results, the results notice, the timer, and the absence of any Bande polkit grant.

**Measured (read-only, live session).** 16 of the owner's processes run with deleted executables after dev reinstalls,
including the compositor, bande-shell, and the sound, power, tray, polkit-agent and portal services, plus Files, Terminal,
Mail, Monitor and Settings. The session works, which shows that replacing a binary live is safe on Linux. Each one keeps
its old inode. The only cost is that it keeps running the old code. A cgroup tells you which ones can restart: `session.slice/bande-compositor.service` holds
the compositor and the shell clients it spawns, `session.slice/<svc>.service` holds the user services, and
`app.slice/app-*-<app-id>-<pid>.scope` holds the apps. One false positive: Claude Desktop maps a deleted
`icon-theme.cache`, so count only mappings with `x` permission plus `/proc/<pid>/exe`. The apt backend's `strings` show
`utilRestartRequired` = `linux-image-`, `nvidia-`, `libc6`, `dbus-broker`, plus `/run/reboot-required`. So PackageKit
says `system` or nothing; it never says `session` or `application`.

## Fix (recommended default: live install, ask only for what is needed)
1. **Classify at check time** (`libs/packages`). Add `update_details(ids)` (`GetUpdateDetail`, read the `restart` enum),
   and turn `Outcome.restart` into the set of `RequireRestart` kinds. Mark an update reboot-class when PackageKit says
   `system`/`security-system`, or when it is `libc6`, `systemd`, `libsystemd0`, `udev`, `dbus*`, or a `linux-image-*` whose
   release equals `uname -r`. The Bande kernel keeps one package name per release
   (`linux-image-7.3-rc7+bande-apple-arm64-16k`), so an upgrade replaces the running kernel's `/lib/modules` in place
   and modprobe can no longer load modules safely. Those packages go **offline** (download, Trigger, as today). Everything else installs
   **live**: new-release kernels, firmware, bootloader, initramfs-tools. Debian does the same, and their hooks run in a normal
   system, not under `system-update.target`. Packages that touch the boot (kernel, firmware, bootloader, `/run/reboot-required`)
   add "Restart to finish".
2. **Install live** (`lib.rs`): `update(ids)` = `UpdatePackages(ONLY_TRUSTED)`. First, `SIMULATE` the live set. If apt
   would pull in a reboot-class package, everything goes offline. If the user is not an admin (the
   `privilege.rs` check), use the offline path. It needs no password, so nobody gets a prompt.
3. **System services restart automatically.** Make `needrestart` a Recommends of bande-desktop and add
   `/etc/needrestart/conf.d/bande.conf` with `restart='a'`, `kernelhints=0` and `ucodehints=0`, and keep it from restarting
   `greetd`, `dbus`, `systemd-logind`, `user@`, `getty@` or `polkit`. It runs as root from apt's `DPkg::Post-Invoke`, and it uses
   the same `/proc/*/maps` check. Settings can't read root processes' maps.
4. **Session scan** (new `software/stale.rs`, unprivileged, own uid only). After a live install, find the processes with
   deleted executable mappings and group them by cgroup:
   - user services in `session.slice`, apart from the compositor: `systemctl --user try-restart` them automatically;
   - `bande-compositor.service`: **"Log out to finish updating the desktop"**, with Log Out and Later;
   - app scopes: **"Quit and reopen Files and Terminal to use the new versions"**. Names come from the desktop files. Settings leaves itself off this list.
5. **Settings UI** (`software.rs`, `updates.rs`, ftl). The primary button becomes **Update Now**. If some updates need a restart, the detail says
   "Some updates need a restart". New state `Installed { count, reboot, logout, apps }` shows one row for each need. The restart row
   offers **Restart Now / Later**. Settings never restarts by itself. The daily timer reminds about a pending restart. The reboot-only path keeps today's
   "Restart to install n updates". macOS works the same way: app and minor updates install live and apps
   are asked to quit; system updates restart, with Restart Now/Later. GNOME Software uses offline mode for packages by default,
   updates Flatpaks live, and can't tell them apart. Debian's own unattended-upgrades runs live with needrestart.

## Safety
- **dpkg interrupted.** packagekitd is a system service. A logout or a Settings crash doesn't stop it, and it holds a logind
  shutdown/sleep inhibitor during the transaction. While it installs, Settings disables Restart and Log Out. If an update is
  left half-done (an apt "dpkg was interrupted" error), the page shows "Finish installing updates". That calls
  `RepairSystem` (`dpkg --configure -a`).
- **Compositor replaced while running.** Measured above, it is harmless. The risk is version skew. The old compositor respawns
  shell clients from `/usr/bin`, so after a crash a new bar can end up talking to the old compositor. Mitigation: the private protocols
  keep their wl_registry version bumps, and the Log Out prompt stays until the user logs out.
- **greetd/PAM.** Never restart greetd live, because it is the session's parent and a restart ends the session. needrestart excludes
  it. A greetd update takes effect at the next boot, with no prompt. bande-lock is a fresh process at each lock, so it loads the new PAM
  modules. A security update to greetd or libpam counts as reboot-class.

## Size and verification
About 650 lines over three branches: (a) crate and needrestart config, (b) session scan, (c) Settings states and strings.
Plus the docs (`docs/architecture/session.md:447`, `docs/parity.md:30`). One e2e test in `tests/updates.rs` with
FakePackageKit: libc6 goes Download+Trigger, the rest goes `UpdatePackages(ONLY_TRUSTED)`, and a simulated pull-in of
a reboot-class package sends everything offline. The maps/cgroup parser gets unit tests on fixture text.
Owner check on the mini: a Debian update that has no restart-class package installs with no restart and only
"Quit and reopen …". `needrestart -b` lists no stale services afterwards. Also confirm that needrestart's hook runs
non-interactively under PackageKit.

## Owner decisions
1. **Default: live with targeted prompts (recommended)**, or keep offline-only. Recommend live.
2. **Who chooses the restart?** Recommend the user always does: Restart Now / Later, Log Out / Later, no automatic reboot.
