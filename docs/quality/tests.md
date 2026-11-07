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
| `…compositor.rs` `a_hung_app_is_force_quit_from_the_keyboard` | Force Quit, opened by the shell, marks a hung separate process and kills it from the keyboard without keys reaching the app. |
| `…compositor.rs` `super_tap_and_super_space_open_search_and_ctrl_alt_t_opens_terminal` | The shortcut table drives the keys: Alt+Super+Space reaches the app and opens nothing, Super+Space and a Super tap each send `show_launcher` once and Super+Space does not reach the app, `Ctrl+Alt+T` sends `open_terminal` and does not reach the app. |
| `…compositor.rs` `a_window_on_a_workspace_out_of_view_is_suspended` | A window on a workspace switched out of view is suspended, and resumes when its workspace comes back. |
| `…compositor.rs` `windows_with_turned_or_flipped_buffers_do_not_stop_the_screen_from_updating` | A window with any rotated or flipped buffer transform renders instead of failing the whole frame. |
| `libs/app/tests/app.rs` | App framework: a second launch reaches the running instance, the app keeps running without windows, launching again reopens a window, F11 enters and leaves full screen, Ctrl+Q quits. |
| `services/portal/tests/screenshot.rs` | A stored screenshot permission is trusted only from the xdg-desktop-portal frontend; any other caller makes the user be asked. |
| `services/notify/tests/center.rs` | A notification shows as a banner and moves to the center; the center is served only under its own bus name, not through `org.freedesktop.Notifications`. |
| `libs/xdg/tests/xdg.rs` `exec_lines_follow_…` | `.desktop` Exec parsing: quoting, escapes, field codes and reserved characters. |
| `libs/xdg/tests/xdg.rs` `a_launcher_file_runs_only_…` | A `.desktop` file opened in Files runs only when installed or marked executable by its owner. |
| `libs/status/src/memory.rs` `pressure_trigger_waits_for_a_stall` | The memory pressure trigger stays quiet without a stall (it once woke the status service in a busy loop). |
| `toolkit/toolkit/tests/title_bar.rs` | A pointer resting on a toolkit window's zoom control is offered Move & Resize, which leaves with the pointer and does not get in the way of a click that zooms; a double-click on free title bar space or an app's move area zooms. |
| `toolkit/widgets/tests/capture.rs` | A press cut short by a sheet is released (the stuck press). |
| `libs/fs/tests/trash.rs` | A volume Trash whose `files` folder is a planted link is not used. |
| `services/automount/tests/automount.rs` | A drive plugged in while the session is locked is mounted only after unlocking. |
| `libs/volumes/tests/eject.rs` | Regression (a VM died on eject): ejecting one volume of a removable USB ISO unmounts every volume of the drive and calls `Drive.Eject`, never `PowerOff`. |
| `libs/datetime/tests/daylight.rs` | Sunrise and sunset fall on the right day east and west of Greenwich and past the Arctic Circle. |
| `services/config/tests/config.rs` | KDL `config.kdl` parsing reports problems at their line and column while valid settings apply; edits keep comments; bars written before the `layout` key keep their layout. |
| `libs/keyboard/tests/layouts.rs` | The XKB rules XML is read despite its DOCTYPE. |
| `apps/settings/tests/settings.rs` | Settings: a failure stays on the pane that raised it; a setting found by search and changed from the keyboard is written to `config.kdl` through `org.bande.Config`; a pane chosen after a press lost to a pop-up is shown at once (the stuck press). |
| `services/secrets/tests/keychain.rs` | The keychain boundary: another app's item opens only with the password; Settings unlocks with one prompt and reads only until `Conceal`; a cancelled or impossible prompt opens nothing; only root may pass a login password; a wrong old password changes nothing. |
| `apps/secrets-prompt/tests/prompt.rs` | Regression: the keychain prompt reads a request written after it started, and a window that fails to open is no answer. |
| `apps/settings/tests/updates.rs` | Settings: updates install at once through PackageKit (trusted only, simulated first) unless one needs a restart, which is prepared for the next boot. |
| `libs/packages/src/flatpak.rs` | Flatpak's progress lines across several refs, and an app's sandbox permissions from its metadata. |
| `apps/settings/src/software/stale.rs` | Which processes run replaced packaged code, and the unit that holds each, from `/proc` maps and cgroup lines. |
| `apps/files/tests/files.rs` | Files: a file goes to the Trash and is put back; a new folder is named in place and undone; a duplicate that cannot be written asks and leaves no copy. |
| `shell/launcher/tests/launcher.rs` | Launcher: typing ranks apps, arrows and Return open the choice. |
| `shell/lock/tests/lock.rs` | Lock screen: a password is submitted once and a rejected one is cleared for another try; a resting lock wakes on the first input, keeps a typed letter and rests again with the field cleared. |
| `services/power/tests/idle.rs` | A running stream holds back sleep but not the display off, and sleep follows its end. |
| `toolkit/toolkit/tests/accessibility.rs` | Toolkit: a screen reader finds a button over AT-SPI in a window on the headless compositor and presses it. |

## Support

- `crates/testing/test-bus`: private `dbus-daemon` buses (`SessionBus`) that never activate the
  machine's services, and fake services on them: PackageKit, logind and UDisks2 (`drives`), and
  the AT-SPI registry with a screen reader's lookups (`atspi`).
- `crates/testing/test-wayland`: a small Wayland client (`TestClient`, `TestClient::window`) for
  the compositor tests and the benchmarks, `SeparateApp` for a client in its own process,
  `headless` and `chord` to make and drive a headless compositor, and `Frames` to render its
  frames until a condition holds.
- Not tests: `crates/testing/bench` (benchmarks, `bande-idle`, `bande-footprint`),
  `crates/testing/sample` (the app the benchmarks start) and `crates/testing/review` (records a
  scenario on a live session for a person to look at).
