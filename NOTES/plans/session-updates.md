# A session survives its own updates

**What the user experiences.** Installing updates while logged in, through Settings or `apt` in Terminal, changes nothing they can see going wrong. The taskbar, lock screen and clipboard keep working, even if one of them crashes after the update. Sound controls come back by themselves when PipeWire restarts. Session services move to the new version on their own. If the desktop itself was replaced, one notification says "Log out to finish updating the desktop", and Software Update shows Log Out / Later. Nothing logs out by itself.

**macOS.** System parts are swapped together and take effect at restart; a service that restarts (coreaudiod) gets its clients back without their noticing.

## Design
1. **The session holds its own build.** At login the compositor opens `bande-shell`, `bande-lock`, `bande-clipboard` and `bande-screenshot` and keeps the files open. It starts them from those descriptors (`/proc/self/fd/N`), so a relaunch after an update runs the build the session began with. dpkg replaces files by rename, so the old inode lives until the compositor exits and the kernel frees it. Helpers that re-run themselves (dialogs) use `/proc/<pid>/exe`, not the `current_exe()` path that an update turns into "... (deleted)".
2. **Contracts between builds that do mix.** Apps and services are new while the compositor is old. `bande_app_v1`, capture and the other protocols may only grow: `ci/release/check-protocols.py` compares against the last release in lint. `bande_shell_v1` is exempt because only held helpers can see it. `org.bande.*` D-Bus interfaces only grow; a missing property falls back to the default.
3. **Audio reconnects in `bande-audio`.** `Audio::start` never gives up: one thread connects, serves, and reconnects, waiting on inotify for the socket between tries. Shell, Settings, power and sound all share it.
4. **Every dpkg run finishes the session update.** The apt hook touches `/run/bande-desktop/packages-installed`. `bande-update-finish.path` in each logged-in user's manager runs `bande-settings --finish-update`, which restarts stale `session.slice` services and notifies when a logout is needed. Software Update and the daily reminder read the same state.
5. **The test.** `ci/upgrade-test` boots a throwaway VM, logs a user into a real systemd session on a headless compositor at the old version, upgrades packages with apt, then crashes the shell, locks the screen, restarts PipeWire, and checks each one recovers.

## Why this over the alternatives
- `/usr/lib/bande/<version>/` (the audit's suggestion) needs package scripts to keep old files against dpkg, plus cleanup, and two rebuilds at one version would still mix. Held descriptors are exact per build, leave nothing behind and need no packaging.
- Versioned kernel-style packages have the same cleanup problem and more apt churn.
- A frozen `bande_shell_v1` alone would tie every shell change to compatibility code for a pair that never has to mix.

## Limits
- A login during a dpkg run can catch a new compositor with an older helper. The window is seconds, needs a login while packages unpack, and clears at the next login.
- The first upgrade from a compositor without this (v0.1.7 and older) is not protected: that compositor still starts helpers by path.

## Stages
1. Audio reconnect, held helpers, contract check, finish-update (this branch).
2. VM upgrade test. Delete this plan when it is merged.
