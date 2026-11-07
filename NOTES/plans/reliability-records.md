# Failures leave a record, stability is measured

**What a person gets.** When anything crashes, hangs or restarts the computer, one dated report
appears in Settings with what failed, a stack trace with function names, and the last log lines;
Copy puts it on the clipboard. Nothing is ever sent anywhere. Separately, every night a build
machine runs the real desktop for hours and writes down whether anything grew, hung or crashed.

**Records.**
- `systemd-coredump` is a dependency of `bande-desktop`, with size limits. The core limit is opened
  for system and user managers: the soft limit is 0 by default, so today even an installed
  coredump handler stores nothing. Programs that hold passwords (lock screen, login window,
  polkit agent, as the keychain and clipboard already do) are not dumpable.
- `dist` builds keep line tables; `cargo deb --dbgsym` splits them into Debian-style `-dbgsym`
  packages with build-id paths. Stack traces read them through elfutils with no network.
- `bande-crash-reports`: a root oneshot, started by each `systemd-coredump@.service`, at boot and at
  each login. It reads the journal from a saved cursor (crashes, failed session services, panics of
  undumpable programs, kernel panics from `systemd-pstore`, a previous boot with no clean shutdown)
  and writes text reports to `/var/lib/bande/crash-reports/<uid>/`, owned by that user, or `system/`
  for the `sudo` group. A hard cap prunes old ones. Root opens only with `openat` + `O_NOFOLLOW`.
- A system service, not a user one: a compositor crash takes the user manager down with it, and the
  kernel and login window have no user. No daemon: nothing runs between failures.
- Force Quit of a not-responding app and the keyboard's last resort first send `SIGQUIT` and wait
  briefly, so a hang leaves a stack, then kill as before.
- Settings ▸ General ▸ Crash Reports lists them and shows the text, with Copy and Delete.
- `configure` catches every wgpu error class, so a failed configure reaches the CPU fallback.
- ramoops stays an owner test (`NOTES/open.md`): reports read `systemd-pstore`'s folder either way.

**Soak.** `bande-soak` (`crates/testing/bench`, not a test) runs the compositor in-process like
`bande-session-bench`, with the real shell, lock screen and apps on lavapipe: window churn, display
hotplug and modes, lock and unlock, memory pressure, killing apps and the shell. It samples RSS,
file descriptors, threads and renderer memory per process, and frame times; a fit over the second
half flags growth, and a missed ping or a frame beyond the limit flags a hang. `ci/soak/run.sh`
runs it and the fuzzers on a build machine, nightly from a timer on the mini; the result goes to
`~/bande-linux/soak/`.

**Fuzz.** `fuzz/` (outside the workspace, `cargo fuzz`): the Wayland wire against a real compositor,
`.desktop`, config and image/rich-text parsers in the desktop; MIME and document parsers in apps.

**Stages.** 1 coredump, limits, undumpable, configure. 2 symbols. 3 reports service. 4 Settings
view, force-quit stack. 5 soak. 6 fuzz. Test budget is cut in its own branch.
