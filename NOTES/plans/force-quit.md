# Force Quit like macOS, owned by the compositor

Owner, 2026-11-01, on the apps-split M3 fallback (Force Quit and so Activity Monitor staying in
desktop): "do it like that as well even if it is more work, i prefer correctness." So Force Quit
leaves Activity Monitor, and Activity Monitor moves to `bande-linux/apps` (apps-split M3, M4, M5).

**Status: M1 done 2026-11-01 (the compositor window, pidfd kill, `show_force_quit`; tests for the four cases). M2 done 2026-11-01 (system menu Force Quit… with its shortcut and the Shift variant through new Shift menu alternates, Activity Monitor without Force Quit, the low-memory dialog without its Activity Monitor button when it is missing). M3-M4 follow.**

## How macOS does it
- `Cmd+Opt+Esc`, Apple menu ▸ Force Quit…, and holding Shift in the Apple menu turns the item into
  "Force Quit *frontmost app*", which quits that app at once. The Dock menu offers Force Quit with
  Option held, and directly when the app is not responding.
- The Force Quit Applications window belongs to `loginwindow`, a core process that is always
  running, so it opens even when the Finder, the Dock or the frontmost app hang.
- It lists only apps the user sees (no background processes), marks a hung app "(Not Responding)"
  in red from the window server's knowledge that the app stopped taking events, asks once ("Do
  you want to force *app* to quit? You will lose any unsaved changes.") and then kills it
  immediately.
- Activity Monitor is an ordinary app with its own per-process Quit and Force Quit.

## Findings
- **Today** Force Quit is `bande-monitor --force-quit`: the compositor starts it on its own socket
  (`Summoned`, `session.rs`) for `Ctrl+Alt+Escape`, the bar's system menu starts it through
  `spawn_detached`, and it samples `/proc` for its list. The taskbar's app menu already sends
  `bande_shell_v1.force_quit_app`, which the compositor serves by sending SIGKILL to the pid from
  `SO_PEERCRED` of each connection with a window of that app.
- **The compositor already knows everything Force Quit needs.** It holds every toplevel with its
  app id and connection (`State::windows`), pings apps through `xdg_wm_base` and marks one that
  has not answered for 3 s (`watchdog.rs`, `UNRESPONSIVE_AFTER`; Force Quit opening pings every
  app), loads app names and icons in the background for the switcher (`switcher::App`), and draws
  its own UI with `bande-widgets` and the CPU renderer (window menus, alerts, the switcher). The
  toolkit `Alert` and `Table` live in `bande-widgets`, which the compositor already links.
- **Kill identity is racy today.** `force_quit_app` looks up the pid at kill time and calls
  `kill(2)`: if the connection outlived its process (the socket was inherited), the pid may name
  another process. Linux 6.5+ gives the peer as a pidfd (`SO_PEERPIDFD`) at connection time.
- **Apps run in their own cgroup scopes.** Bande's launcher puts each app in
  `app-bande-<id>-<pid>.scope`, Flatpak in `app-flatpak-<id>-<pid>.scope`, GNOME in
  `app-gnome-<id>-<pid>.scope`; `bande_xdg::unit_app_ids` parses all of them. The user owns
  `cgroup.kill` in their scopes (checked on the mini), so one write kills an app with all its
  helpers (renderer processes, `bwrap`, the Flatpak D-Bus proxy) without D-Bus or systemd.

## Design
### D1. Where it lives: in the compositor, drawn by it
The Force Quit window is a compositor overlay, as the app switcher and window menus are, built
from the same `bande-widgets` pieces the toolkit dialogs use: a titled utility window (title
"Force Quit Applications", fixed size, the `Role::utility` look), a hint, the app `Table`, a
primary Force Quit button, and the toolkit `Alert` sheet to confirm.

Reliability is the first criterion, and the compositor is Bande's `loginwindow`: the one process
that is always running and that every hang already leaves alive.
- **Nothing to start at the moment of need.** Under memory pressure without swap, starting a
  client (exec, fonts, GPU setup, its first `/proc` sample) can take seconds to minutes while
  file pages thrash; the compositor is resident and its overlays render on the CPU it already
  uses for labels.
- **Nothing else has to answer.** No bande-shell, bar, Activity Monitor, session bus or
  `systemd --user` is on the path to open, list or kill.
- **One source of truth.** The list, the Not Responding state and the process to kill all come
  from the connections the compositor holds; a client would need a new protocol to learn them
  and would ask the compositor to kill anyway.

Rejected: a small compositor-started client (`bande-force-quit` on the toolkit's `Role::utility`
and `Alert`). Same look with less compositor code, but it has to start when the machine is least
able to, and keeping it resident costs memory all the time (footprint work is cutting exactly
that). The cost of D1 is about 800 lines in the compositor, held to its no-panic rules and
covered by headless tests.

### D2. The list
Every app with a mapped toplevel, grouped by app id (by connection for a window without one),
leaving out shell and session clients and the compositor's own process. Names and icons come from
the switcher's catalog; until it has loaded, the app id's last segment and a lettered placeholder
stand in, so opening never waits for the disk. Sorted by name; the frontmost app is selected, as
on macOS. Several rows can be selected. The list follows apps opening, closing and answering
while the window is open.

### D3. Not Responding
An app is Not Responding while one of its connections has left an `xdg_wm_base` ping unanswered for
3 s (the watchdog). Opening Force Quit pings every app, so a hung app is marked within 3 s; the
row reads "*Name* (Not Responding)" in the danger colour and returns to normal when it answers.

### D4. Killing: SIGKILL at once, through a pidfd, the whole app scope
- **SIGKILL straight away, as macOS's Force Quit.** The polite path already exists (Quit, Ctrl+Q,
  `bande_app_v1.quit_requested`, close). An app that reaches Force Quit is usually hung, and the
  toolkits most apps use (GTK, Qt, Electron, Firefox) handle SIGTERM in their event loop, which a
  hung app never runs: a SIGTERM grace period would only delay relief by its own length while
  the memory stays taken. The confirmation alert (D1) is where the user accepts losing unsaved
  work, as on macOS.
- **No pid-reuse race.** At connection time the compositor takes the peer as a pidfd
  (`SO_PEERPIDFD`, falling back to `pidfd_open` of `SO_PEERCRED` on kernels before 6.5) and keeps
  it with the client. Signals go through `pidfd_send_signal`, which also reaches a process in
  another pid namespace (Flatpak's sandbox).
- **The whole app.** If the process's cgroup is an app scope or service named after this app (the
  XDG convention `bande_xdg::unit_app_ids` parses), the compositor also writes `1` to its
  `cgroup.kill`, killing every helper process of the app in one step. It never does so for a
  cgroup that holds the compositor, or one named after another app: an app started from Terminal
  lives in Terminal's scope, so only its own process dies. Process groups are not used: what a
  launcher put in one is not the app.
- **Memory back now.** After SIGKILL, `process_mrelease(2)` on the pidfd frees the address space
  at once instead of when the process finishes dying (as `lmkd` and `systemd-oomd` do).
- Never: shell and session clients, the compositor itself.
- The taskbar's Force Quit (`bande_shell_v1.force_quit_app`) and the bar's Shift variant go
  through the same code.

### D5. Entry points
- `Ctrl+Alt+Escape` (the `bande-keyboard` table): opens Force Quit or brings it forward; works
  while an app inhibits shortcuts, not while the session is locked.
- Bar system menu "Force Quit…" (labelled `Ctrl+Alt+Esc`): the new request
  `bande_shell_v1.show_force_quit`. Holding Shift turns it into "Force Quit “*App*”", which force
  quits the frontmost app at once. Menus gain Shift alternates beside the Alt (Option) ones.
- Taskbar app menu: unchanged (Force Quit as Quit's Alt alternate, or directly when the app is
  not responding).
- In the window: Up/Down and type-to-select, Enter or the button asks, Enter again or the
  destructive default confirms, Escape cancels the alert or closes the window, a click outside
  closes it.

### D6. Accessibility and language
The window is exposed through its own AccessKit adapter as a Dialog named "Force Quit
Applications", with the widgets tree inside (list, rows, button, the alert as AlertDialog), and
screen reader actions reach the widgets. Strings are in the compositor's Fluent file.

### D7. Activity Monitor
It loses `--force-quit`, its Force Quit window and that model, and keeps its own per-process Quit
and Force Quit (as macOS's Activity Monitor does). The low-memory dialog hides its Activity Monitor
button when `bande-monitor` is not installed. Then Activity Monitor moves to the apps repo by the
apps-split M4/M5 recipe.

## Reliability cases (apps-split D2) and their tests
1. **Under memory pressure:** opening starts no process and reads nothing it must wait for; killing
   needs no D-Bus or systemd and releases memory at once. Headless test: the chord opens the list
   before the catalog has loaded, with no session client started. A real low-memory run is for
   the owner on the mini (see Owner checks).
2. **Activity Monitor absent:** Force Quit uses no other program. The headless tests run without
   one; the open, list and kill tests prove it.
3. **The bar or a client hung:** headless test with a shell client that never reads its socket and
   an app that stops answering pings: the chord opens Force Quit, the app is marked Not
   Responding after 3 s, and Force Quit kills it (a real child process that owns the connection).
4. **From the keyboard:** headless test that, with shortcuts inhibited by the hung focused app,
   `Ctrl+Alt+Escape`, Down, Enter and Enter kill the selected app, and Escape closes.
Unit tests: the scope rule (own scope, another app's scope, Flatpak, a service, the compositor's
own cgroup), the `cgroup.kill` write into a scratch tree, pidfd SIGKILL of a spawned child.

## Milestones
- **M1 (desktop) The compositor window.** `force_quit` module (view, list, kill, accessibility),
  pidfd at connection, `show_force_quit` request and its toolkit call, chord opens it, `Summoned`
  Force Quit, `MONITOR` and `/usr/bin/bande-monitor` in session-clients removed; headless and
  unit tests.
- **M2 (desktop) Entry points and Activity Monitor.** Bar system menu with the shortcut label and
  the Shift variant (menus: Shift alternates), low-memory Activity Monitor button hidden without
  it, Activity Monitor without Force Quit, docs (behaviour.md, decision 0004).
- **M3 (apps) Import Activity Monitor** with history (`git filter-repo` of `crates/apps/monitor`
  to `apps/monitor`), standalone Cargo.toml pinned to desktop `main` after M2, icon fixtures for
  its view tests, `--version`, smoke.sh, build-depends; apps gate.
- **M4 (desktop) Remove it from desktop.** Delete the crate, prune the lock, `[apps]` in
  default-apps gains `bande-monitor` (the pinned Recommends goes), check-debs, packages,
  build-all, review run.sh, README and docs; rebuild-apps needs no change (it builds apps-repo
  names there).
- Then the final gates: `desktop-gate` on main, apps `ci/gate.sh`.

## Owner checks on the mini
- `Ctrl+Alt+Escape` with a frozen app (`kill -STOP` a test app's pid, then click it): the row turns
  red within 3 s; Force Quit kills it and its helper processes (`systemd-cgls --user`).
- Under real memory pressure (for example `stress-ng --vm 1 --vm-bytes 95%` in a scope you can
  kill): the window opens and kills.
- Screen reader: Orca reads "Force Quit Applications, dialog" and the rows.
- Shift in the system menu shows "Force Quit “*App*”".
