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
| `…compositor.rs` `a_hung_app_is_force_quit_from_the_keyboard_while_the_shell_is_hung` | Force Quit marks a hung separate process and kills it from Ctrl+Alt+Esc, without the shell and without keys reaching the app. |
| `libs/app/tests/app.rs` | App framework: a second launch reaches the running instance, the app keeps running without windows, launching again reopens a window, Ctrl+Q quits. |
| `services/portal/tests/screenshot.rs` | A stored screenshot permission is trusted only from the xdg-desktop-portal frontend; any other caller makes the user be asked. |
| `services/notify/tests/center.rs` | A notification shows as a banner and moves to the center; the center is served only under its own bus name, not through `org.freedesktop.Notifications`. |
| `libs/xdg/tests/xdg.rs` `exec_lines_follow_…` | `.desktop` Exec parsing: quoting, escapes, field codes and reserved characters. |
| `libs/xdg/tests/xdg.rs` `a_launcher_file_runs_only_…` | A `.desktop` file opened in Files runs only when installed or marked executable by its owner. |
| `libs/fs/tests/trash.rs` | A volume Trash whose `files` folder is a planted link is not used. |
| `services/automount/tests/automount.rs` | A drive plugged in while the session is locked is mounted only after unlocking. |
| `libs/datetime/tests/daylight.rs` | Sunrise and sunset fall on the right day east and west of Greenwich and past the Arctic Circle. |
| `services/config/tests/config.rs` | KDL `config.kdl` parsing reports problems at their line and column while valid settings apply, and edits keep comments. |
| `libs/keyboard/tests/layouts.rs` | The XKB rules XML is read despite its DOCTYPE. |
| `apps/settings/tests/settings.rs` | Settings: a setting found by search and changed from the keyboard is written to `config.kdl` through `org.bande.Config`. |
| `apps/files/tests/files.rs` | Files: a file goes to the Trash and is put back; a new folder is named in place and undone. |
| `shell/launcher/tests/launcher.rs` | Launcher: typing ranks apps, arrows and Return open the choice. |
| `shell/lock/tests/lock.rs` | Lock screen: a password is submitted once and a rejected one is cleared for another try. |
| `toolkit/toolkit/tests/accessibility.rs` | Toolkit: a screen reader finds a button over AT-SPI in a window on the headless compositor and presses it. |

## Support

- `crates/testing/test-bus`: private `dbus-daemon` buses (`SessionBus`) that never activate the
  machine's services.
- `crates/testing/test-wayland`: a small Wayland client (`TestClient`) for the compositor tests
  and the benchmarks, and `SeparateApp` for a client in its own process.
- Not tests: `crates/testing/bench` (benchmarks, `bande-idle`, `bande-footprint`),
  `crates/testing/sample` (the app the benchmarks start) and `crates/testing/review` (records a
  scenario on a live session for a person to look at).
