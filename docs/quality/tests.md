# Tests

The desktop keeps a few tests, about 1,500 lines in all (directive 2 in `CLAUDE.md`). A test is
either an end-to-end scenario of what a user does and sees, run against real parts (the
headless compositor, a private D-Bus bus from `bande-test-bus`, real apps and processes), or a
regression test for a real bug or security boundary that fails without its fix. Tricky pure
logic (parsers, date maths) may have one test. There are no screenshot or golden-image tests:
behaviour is asserted through the accessibility tree, the Wayland protocol, D-Bus and files.
At most a couple of tests per app or feature; a new test replaces or merges with an old one
before the total grows.

`cargo test --workspace` runs them on the hermetic bus
(`dbus-run-session --config-file=crates/testing/test-bus/data/session.conf -- cargo test --workspace`);
the compositor and app tests render with lavapipe (`VK_DRIVER_FILES`).

## What is kept

| Test | Purpose |
| :--- | :--- |
| `platform/compositor/tests/compositor.rs` `while_locked_no_popup_grab_or_input_method_takes_keys_from_the_lock_surface` | Lock screen bypasses: a grabbing popup and an input method keyboard grab get no keys or clicks while the lock surface holds the keyboard. |
| `…compositor.rs` `only_the_screen_reader_may_grab_the_keyboard_and_it_lets_go_when_it_leaves` | The keyboard monitor serves only the screen reader's bus name, and a vanished reader releases the keyboard. |
| `…compositor.rs` `a_hung_app_is_force_quit_from_the_keyboard` | Force Quit marks a hung separate process and kills it from the keyboard without keys reaching the app. |
| `services/secrets/tests/keychain.rs` | The keychain boundary: an app keeps a secret over an encrypted session and cannot name another owner; another app's item opens only with the password, with one prompt from Settings, and only until `Conceal`; a cancelled or impossible prompt opens nothing; only root may pass a login password; a wrong old password changes nothing. |
| `apps/secrets-prompt/tests/prompt.rs` | Regression: the keychain prompt reads a request written after it started, and a window that fails to open is no answer. |
| `shell/lock/tests/lock.rs` | Lock screen: a password is submitted once and a rejected one is cleared for another try; a resting lock wakes on the first input, keeps a typed letter and rests again with the field cleared. |
| `services/portal/tests/screenshot.rs` | A stored screenshot permission is trusted only from the xdg-desktop-portal frontend; any other caller makes the user be asked. |
| `services/notify/tests/center.rs` | A notification shows as a banner and moves to the center; the center is served only under its own bus name, not through `org.freedesktop.Notifications`. |
| `services/automount/tests/automount.rs` | A drive plugged in while the session is locked is mounted only after unlocking. |
| `libs/fs/tests/trash.rs` | A volume Trash whose `files` folder is a planted link is not used. |
| `libs/xdg/tests/xdg.rs` | `.desktop` Exec parsing (quoting, escapes, field codes, reserved characters); a `.desktop` file opened in Files runs only when installed or marked executable by its owner. |
| `apps/files/tests/files.rs` | Files: a file goes to the Trash and is put back; a duplicate that cannot be written asks and leaves no copy. |
| `libs/volumes/tests/eject.rs` | Regression (a VM died on eject): ejecting one volume of a removable USB ISO unmounts every volume of the drive and calls `Drive.Eject`, never `PowerOff`. |
| `apps/settings/tests/updates.rs` | Settings: updates install at once through PackageKit (trusted only, simulated first) unless one needs a restart, which is prepared for the next boot. |
| `apps/settings/tests/settings.rs` | Settings: a setting found by search and changed from the keyboard is written to `config.kdl` through `org.bande.Config`. |
| `services/config/tests/config.rs` | KDL `config.kdl` parsing reports problems at their line and column while valid settings apply; edits keep comments. |
| `shell/shell/tests/session_end.rs` | The real confirmation dialog against a fake logind and systemd: a confirmed Shut Down really reaches `PowerOff` (the dialog once exited before the call left) and the dialog stays; a hung app does not stop Restart; an inhibitor stops the call and is shown; Log Out stops the compositor service. |
| `libs/app/tests/app.rs` | App framework: a second launch reaches the running instance, the app keeps running without windows, launching again reopens a window, Ctrl+Q quits. |
| `toolkit/toolkit/tests/accessibility.rs` | Toolkit: a screen reader finds a button over AT-SPI in a window on the headless compositor and presses it. |

## Support

- `crates/testing/test-bus`: private `dbus-daemon` buses (`SessionBus`) that never activate the
  machine's services, and fake services on them: PackageKit, logind, systemd and UDisks2 (`drives`), and
  the AT-SPI registry with a screen reader's lookups (`atspi`).
- `crates/testing/test-wayland`: a small Wayland client (`TestClient`, `TestClient::window`) for
  the compositor tests and the benchmarks, `SeparateApp` for a client in its own process,
  `headless` and `chord` to make and drive a headless compositor, and `Frames` to render its
  frames until a condition holds.
- Not tests: `crates/testing/bench` (benchmarks, `bande-idle`, `bande-footprint`),
  `crates/testing/sample` (the app the benchmarks start) and `crates/testing/review` (records a
  scenario on a live session for a person to look at).
