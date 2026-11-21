# Tests

The desktop keeps few tests (`CLAUDE.md`, Tests). A test is either an end-to-end scenario of
what a user does and sees, run against real parts (the headless compositor, a private D-Bus bus
from `bande-test-bus`, real apps and processes), or a regression test for a real bug or security
boundary that fails without its fix. Tricky pure logic (parsers, protocol encodings, timing maths)
may have one test. There are no screenshot or golden-image tests: behaviour is asserted through
the accessibility tree, the Wayland protocol, D-Bus and files. At most a couple of tests per app
or feature; a new test replaces or merges with an old one before the total grows.

The guide is about 1,500 lines of tests. Today there are 2,329: 1,999 lines in
`crates/**/tests/*.rs` (`wc -l`) and 330 lines in the nine `#[cfg(test)]` modules under
`crates/**/src` (each counted from its `#[cfg(test)]` line to its closing brace). The keyring
boundary (406) and the compositor's input and lock boundaries (312) are most of the excess; trim
them before adding anything.

`cargo test --workspace` runs them on the hermetic bus
(`dbus-run-session --config-file=crates/testing/test-bus/data/session.conf -- cargo test --workspace`);
the compositor and app tests render with lavapipe (`VK_DRIVER_FILES`).

## What is kept

Paths are under `crates/`.

| Test | Purpose |
| :--- | :--- |
| `platform/compositor/tests/compositor.rs` `while_locked_no_popup_grab_input_method_or_virtual_keyboard_takes_keys_from_the_lock_surface` | Lock screen bypasses: a grabbing popup, an input method keyboard grab and a consented virtual keyboard get no keys or clicks while the lock surface holds the keyboard. |
| `…compositor.rs` `a_consented_input_method_sees_keys_only_in_a_text_field_that_is_no_password` | An input method with consent reads keys only while a text field is active, never in a password field. |
| `…compositor.rs` `without_consent_nothing_types_and_turning_it_off_hands_the_keys_back_at_once` | Regression: without consent no virtual keyboard is made, and revoking an input method's consent sends the next key to the app. |
| `…compositor.rs` `only_the_lock_screen_may_lock_and_a_sandbox_gets_no_privileged_protocol` | Only the lock screen sees the session lock; a sandboxed client sees none of the privileged globals. |
| `…compositor.rs` `only_the_screen_reader_may_grab_the_keyboard_and_it_lets_go_when_it_leaves` | The keyboard monitor serves only the screen reader's bus name, and a vanished reader releases the keyboard. |
| `…compositor.rs` `a_hung_app_is_force_quit_from_the_keyboard` | Force Quit marks a hung separate process and kills it from the keyboard without keys reaching the app. |
| `…compositor.rs` `the_browser_row_gives_the_well_to_the_compositor_and_the_rest_to_the_browser` | On the Browser's own tab row, a click on the control well minimizes through the compositor and any other click reaches the Browser. |
| `platform/compositor/src/clock.rs` (3 tests) | Regressions in the frame clock: the first frame is drawn although timers wake late, unusable flip times fall back to classic timing, and a render longer than a refresh aims at a slot it can still start for instead of spinning. |
| `services/secrets/tests/keyring.rs` | The keyring boundary: an app keeps a secret over an encrypted session and cannot name another owner; another app's item opens only with the password, with one prompt from Settings, and only until `Conceal`; a cancelled or impossible prompt opens nothing; only root may pass a login password (`an_ordinary_process_sends_a_login_message` is the helper the test reruns as `nobody`); a wrong old password changes nothing. |
| `apps/secrets-prompt/tests/prompt.rs` | Regression: the keyring prompt reads a request written after it started, and a window that fails to open is no answer. |
| `shell/lock/tests/lock.rs` | Regression: the key that wakes a dark display is not typed into the password, and Select All then Backspace clears the field. |
| `services/portal/tests/screenshot.rs` | A stored screenshot permission is trusted only from the xdg-desktop-portal frontend; any other caller makes the user be asked. |
| `services/notify/tests/center.rs` | A notification shows as a banner and moves to the center; the center is served only under its own bus name, not through `org.freedesktop.Notifications`. |
| `services/automount/tests/automount.rs` | A drive plugged in while the session is locked is mounted only after unlocking. |
| `libs/volumes/tests/eject.rs` | Regression (a VM died on eject): ejecting one volume of a removable USB ISO unmounts every volume of the drive and calls `Drive.Eject`, never `PowerOff`. |
| `libs/volumes/src/server.rs` | A typed server address that could inject options or paths into the mount tools is refused. |
| `libs/fs/tests/trash.rs` | A volume Trash whose `files` folder is a planted link is not used. |
| `libs/fs/tests/operations.rs` | Regressions in file operations: Replace keeps the old item until the new one is complete; names that are not UTF-8 survive listing, copy and Keep Both. |
| `apps/files/tests/files.rs` | Files: a file goes to the Trash and is put back; Quit during a copy asks, then goes through when the copy ends. |
| `libs/xdg/tests/xdg.rs` | `.desktop` Exec parsing (quoting, escapes, field codes, reserved characters); a `.desktop` file opened in Files runs only when installed or marked executable by its owner. |
| `apps/settings/tests/settings.rs` | Settings: a setting found by search and changed from the keyboard is written to `config.kdl` through `org.bande.Config`. |
| `apps/settings/tests/updates.rs` | Settings: updates install at once through PackageKit (trusted only, simulated first) unless one needs a restart, which is prepared for the next boot. |
| `apps/settings/src/bin/bande-locale-helper.rs` | The root locale helper writes only real locale and keyboard values, never an injected line or variable. |
| `services/config/tests/config.rs` | KDL `config.kdl` parsing reports problems at their line and column while valid settings apply; edits keep comments. |
| `shell/shell/tests/session_end.rs` | The real confirmation dialog against a fake logind and systemd: a confirmed Shut Down really reaches `PowerOff` (the dialog once exited before the call left) and the dialog stays; a hung app does not stop Restart; an inhibitor stops the call and is shown; Log Out stops the compositor service. |
| `libs/app/tests/app.rs` | App framework: a second launch reaches the running instance, the app keeps running without windows, launching again reopens a window, the switcher activates it without a window, Ctrl+Q quits. |
| `toolkit/toolkit/tests/accessibility.rs` | Toolkit: a screen reader finds a button over AT-SPI in a window on the headless compositor and presses it. |
| `libs/l10n-source/src/plural.rs` | Parser: gettext plural rules pick the forms their languages need. |
| `libs/content/src/import.rs` | Parser: words split across Word XML runs stay whole and paragraphs stay apart in the indexed text. |
| `services/printd/src/mdns.rs` | Parser of untrusted network input: a printer announcement with compressed DNS names resolves to its address. |
| `services/printd/src/raster.rs` (2 tests) | Encoding: raster lines pack runs and literals, and equal lines repeat, as the printer formats define. |
| `services/reporter/src/scrub.rs` | Privacy boundary: nothing personal survives in a fault report's text while diagnostics do. |
| `services/reporter/src/signature.rs` | The documented fault signature test vectors, which the report server relies on to group reports. |

## Support

- `crates/testing/test-bus`: private `dbus-daemon` buses (`SessionBus`) that never activate the
  machine's services, and fake services on them: PackageKit, logind, systemd and UDisks2 (`drives`), and
  the AT-SPI registry with a screen reader's lookups (`atspi`).
- `crates/testing/test-wayland`: a small Wayland client (`TestClient`, `TestClient::window`) for
  the compositor tests and the benchmarks, `SeparateApp` for a client in its own process,
  `grant_consent` for keyboard protocol consent, `headless` and `chord` to make and drive a
  headless compositor, and `Frames` to render its frames until a condition holds.
- Not tests: `crates/testing/bench` (benchmarks, `bande-idle`, `bande-footprint`),
  `crates/testing/sample` (the app the benchmarks start), `crates/testing/review` (records a
  scenario on a live session for a person to look at) and the `fuzz/` targets.
