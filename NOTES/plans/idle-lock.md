# Idle, screen saver and lock

Owner report 2026-11-02: after a night away the session was unlocked and the display had never
turned off.

## Cause of that night (fixed in `idle-lock`)

1. The compositor queued `ext_idle_notification_v1.idled` but flushed clients only after a
   client request or a vblank. With the monitor switched off at 23:58 nothing drew, so
   `bande-power` never heard "idle": no lock, no display off. Proven on the live 22:53
   compositor: a probe's 1 s notification arrived only after the probe sent a request.
2. Even when heard, the lock could not start: the compositor matched logind's `Lock` signal on
   `/org/freedesktop/login1/session/auto`, but logind signals on the real session path. Idle lock
   and lock-before-sleep never reached `bande-lock`; only `Super+L` worked.

Not the cause: no idle inhibitor was held (`systemd-inhibit --list` shows only `sleep` and
`shutdown` delays; Claude Desktop's is `sleep`), builds do not inhibit, the busy shell does not
count as activity, and the config (`suspend-after "never"`, defaults dim 5 min, lock 10 min)
was fine.

## macOS and GNOME/KDE

- macOS Lock Screen: Start Screen Saver when inactive; Turn display off when inactive; Require
  password after screen saver begins or display is turned off (Immediately, 5 s, 1, 5, 15 min,
  1, 8 h); show message when locked. Energy: Prevent automatic sleeping when the display is
  off; Wake for network access. `Ctrl+Cmd+Q` locks. Assertions: a playing, visible video holds
  `PreventUserIdleDisplaySleep` (blocks dim, saver, display off and so the lock); audio holds
  only `PreventUserIdleSystemSleep`; `caffeinate -d/-i`.
- GNOME: idle delay → blank (and lock after a delay) → suspend; inhibitors through gnome-session
  (`ScreenSaver.Inhibit` proxied), mutter honours `idle-inhibit` only for unobscured surfaces.
- KDE: PowerDevil dims, turns off and locks; `ScreenSaver.Inhibit` blocks all three; KWin honours
  `idle-inhibit` only when shown and on the current desktop (KDE 471909: minimised mpv locks).

## Keep-awake matrix (verified headless with a fake logind, 10 s lock)

| Signal | Blocks lock | Lock after release |
|---|---|---|
| `ScreenSaver.Inhibit` held by a live client | yes | 10 s later |
| portal `Inhibit` flag 8 (GTK backend → ScreenSaver) | yes | 10 s later |
| portal `Inhibit` flag 4 (suspend) | no (dropped) | — |
| `systemd-inhibit --what=idle` | yes | 10 s later |
| `zwp_idle_inhibitor_v1` on a covered, unfocused window | yes | 10 s later |
| nothing | locks and turns the display off at 10 s | — |

## Next: the macOS model (branch `idle-macos`)

- Config `idle`: `screen-saver-after` (or "never"), `display-off-after` (or "never"),
  `require-password-after` ("0s" = immediately), `suspend-after`; `dim-after`/`lock-after` go.
  Defaults: saver 5 min, display off 10 min, password immediately, sleep with the display.
- Chain in `bande_power::idle`: backlight dim 10 s before the first of saver/display off →
  saver (`Session.Lock`; the lock screen in its resting look) → display off → password
  required once `require-password-after` has passed since the saver or display off. Input
  before then dismisses without a password; `Super+L`, `Ctrl+Cmd+Q`, sleep and logind locks
  always require it.
- Screen saver: the lock screen's clock and date over the dimmed, blurred desktop picture,
  without the field. Lean (no new process, nothing animates but the minute), covers the
  desktop through `ext-session-lock`, and matches macOS's saver-to-lock-screen hand-over.
- Settings ▸ Lock Screen pane with macOS's three rows and the lock message; Power keeps sleep.

## Later

- Sleep stays off until s2idle passes ten cycles on the mini; then `suspend-after`, audio
  output streams as a system (not display) keep-awake, and portal flag 4 in a Bande Inhibit
  backend as logind `sleep` block inhibitors.
- logind `IdleHint` is never set (`IdleAction` is `ignore`, so nothing depends on it).
- The lock screen ignores logind's `Unlock`.
