# Tests

Bande keeps few tests (`CLAUDE.md`, Tests). A test is either an end-to-end scenario of
what a user does and sees, run against real parts (the headless compositor, a private D-Bus bus
from `bande-test-bus`, real apps and processes), or a regression test for a real bug or security
boundary that fails without its fix. Tricky pure logic (parsers, protocol encodings, timing maths)
may have one test. There are no screenshot or golden-image tests: behaviour is asserted through
the accessibility tree, the Wayland protocol, D-Bus and files. At most a couple of tests per app
or feature; a new test replaces or merges with an old one before the total grows.

The guide is about 1,500 lines of tests for the desktop and 80–100 per app. Today there are
4,355: 3,818 lines in `crates/**/tests/*.rs` (`wc -l`) and 537 lines in the 17 `#[cfg(test)]`
modules under `crates/**/src` (each counted from its `#[cfg(test)]` line to its closing brace).
The keyring boundary (406), the compositor's input and lock boundaries (312) and Mail's
end-to-end test with dovecot are most of the excess; trim them before adding anything.

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
| `…compositor.rs` `idle_surfaces_that_ask_for_a_frame_without_new_content_still_get_one` | Regression guard for `wl_surface.frame`: an idle window, and a desynchronised subsurface, that ask for a frame without new content still get their callback, or apps waiting on it stall. |
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
| `apps/mail/tests/mail.rs` `a_message_goes_out_once_without_injected_headers_after_the_imap_session_went_stale` | Uses a real dovecot and an SMTP listener. When the server drops the IMAP session, a message must still go out only once and be filed in Sent; it went out twice before the fix. It also checks that CR/LF in a name or subject cannot inject a `Bcc:` or another header, and that Bcc recipients never appear in the message. Skips itself when `/usr/sbin/dovecot` is not installed. |
| `apps/mail/tests/mail.rs` `an_account_found_in_its_domains_records_signs_in_after_a_wrong_password` | End to end with a real dovecot, an SMTP listener and a nameserver from `bande-discovery`'s `testing` feature: an address's servers are found in its domain's RFC 6186 SRV records, a wrong password is reported as such with the settings kept, and the right one then signs in. Skips itself when `/usr/sbin/dovecot` is not installed. |
| `apps/mail/tests/mail.rs` `a_mailto_link_cannot_add_hidden_recipients_or_attachments` | Security boundary: a web page's `mailto:` link must not add Bcc recipients or attach local files. |
| `apps/mail/tests/mail.rs` `search_is_told_of_new_mail_its_text_and_its_removal_and_catches_up_after_losing_its_index` | End to end with a search index on the test bus: new mail and its sender reach search, the text follows once the body arrives, a removed message is forgotten, and an index that lost everything is given it all again. |
| `apps/mail/src/smtp.rs` `addresses_cannot_smuggle_commands` | Security boundary: an address with `>` or CR/LF must not inject SMTP commands. |
| `apps/calendar/tests/untrusted.rs` `a_hostile_fifo_or_huge_calendar_file_opens_without_panicking_hanging_or_being_read_whole` | Regression: huge `INTERVAL` values, `BYDAY` that is not ASCII, oversized `BY*` lists and overflowing `DURATION`s used to panic or blow up during expansion; opening a FIFO used to hang Calendar, and a huge `.ics` file was read whole. |
| `apps/calendar/tests/accounts.rs` `a_digest_caldav_server_found_from_the_domains_records_signs_in_after_a_wrong_password` | End to end against a CalDAV server that asks for Digest and a nameserver from `bande-discovery`'s `testing` feature: the server is found from the address's RFC 6764 SRV and TXT `path` records without a password, a wrong password is refused as such, and the right one follows principal, home set and calendars. Digest used to fail as a wrong password. |
| `apps/calendar/tests/accounts.rs` `a_password_is_sent_only_over_https_or_to_this_computer` | Security boundary: when a server on plain HTTP asks for a password, Calendar must refuse to send it, except to loopback. |
| `apps/calendar/tests/time_zone.rs` `a_series_moved_to_another_zone_keeps_its_times_exceptions_and_end` | Tricky date maths: moving a repeating event to another time zone must keep its wall times, its exceptions (`EXDATE`) and its `UNTIL` end in the new zone, or occurrences shift by hours or reappear. |
| `apps/clock/tests/alarm_time.rs` `repeating_alarms_keep_their_wall_time_across_weekends_and_daylight_saving_changes` | Tricky date maths: a repeating alarm must ring at its wall time across a weekend and in another zone, at the first minute that exists when the clock springs forward over it, and once, at the first occurrence, when the clock falls back. |
| `apps/clock/tests/alarm_time.rs` `an_agent_started_late_rings_a_missed_alarm_once_but_not_a_stale_one` | The agent started by the wake-up timer or at login after the alarm's minute must still ring a missed one-off alarm, once, and turn it off, but not ring one more than an hour old. |
| `apps/monitor/tests/signal.rs` `quit_and_force_quit_signal_the_sampled_process_and_never_a_reused_pid` | Against real `/proc`: Quit and Force Quit send `SIGTERM` and `SIGKILL` to a real sampled process, and a reused PID (another start time) or an exited process is never signalled. |
| `apps/software/tests/catalog.rs` `a_dep11_file_with_a_syntax_error_is_read_up_to_the_error_and_no_further` | Regression: a YAML syntax error used to make the catalogue loop forever. |
| `apps/software/tests/catalog.rs` `appstream_xml_that_declares_a_doctype_is_read` | Regression: Debian's AppStream XML declares a DOCTYPE, so the parser refused the file and lost all its apps. |
| `apps/installer/tests/deb.rs` `a_built_deb_opens_with_its_fields_and_a_truncated_download_is_refused` | End to end with `dpkg-deb`: a real `.deb` opens with its fields and its app's name and icon (read from the archive with size caps), and a truncated download is refused instead of shown. |
| `apps/installer/tests/deb.rs` `a_refused_password_says_so_instead_of_returning_silently_to_install` | Regression against a PackageKit on the test bus: when authentication is refused, Package Installer used to go back to Install without a word; it must say that nothing was changed. |
| `apps/calculator/tests/calculator.rs` `typing_and_clicking_calculate_exactly_with_precedence_in_every_mode` | End to end through the keypad widget, its keyboard bindings and its accessibility tree: decimal arithmetic without binary-float surprises (0.1 + 0.2 = 0.3), precedence and parentheses, exact trigonometry in degrees and radians, RPN, and programmer word sizes, bases and bits. |
| `apps/calculator/tests/calculator.rs` `errors_and_word_sizes_wrap_like_a_real_calculator` | Tricky integer logic: division by zero is an error the next key clears, entry stops at the word size, word flips and narrowing wrap, and a pasted negative number is two's complement in hex and signed in decimal. |
| `apps/archive/tests/archives.rs` `compressed_items_expand_back_without_setuid_and_encrypted_archives_ask_for_the_password` | End to end through Archives' work: each format it creates expands back with links and modes but never setuid, and encrypted zip and 7-Zip archives made by `7z` ask for the password, again on a wrong one. The `7z` part skips itself when `7z` is not installed. |
| `apps/archive/tests/archives.rs` `hostile_archives_are_refused_and_leave_nothing_behind` | Security boundary: `..`, absolute paths, writing through a link and links that leave the folder are refused, as are a zip that declares 3.75 GB and a bzip2 bomb, with the refusal as the message and nothing written. |
| `apps/velox/tests/engine.rs` `the_list_follows_events_and_reconciles_when_the_engine_stops_and_returns` | End to end against a fake Engine API on a unix socket (`DOCKER_HOST`): with no engine the not-ready view offers copyable steps; when the socket appears the list and live events arrive without a restart; when the engine goes away and comes back, Velox reconnects and lists again. |
| `apps/velox/src/engine/informer.rs` `only_unasked_failures_are_crashes` | Tricky event logic: a container that exits with a failure is a crash, but not a clean exit, a `SIGTERM` exit, or a `SIGKILL` exit right after Velox or the user asked for a kill; the next unasked `SIGKILL` is one again. |
| `apps/velox/src/engine/places.rs` `ssh_endpoints_keep_user_host_and_port` | Tricky parser: an `ssh://` engine address keeps its user, host and port, and falls back to port 22. |
| `apps/velox/src/engine/http.rs` `chunked_bodies_decode_across_split_chunks_extensions_and_trailers` | Tricky parser: the engine's chunked streams split JSON across chunks and may carry extensions and trailers. |
| `apps/player/tests/engine.rs` `a_song_opens_with_its_length_and_plays_to_the_end` | End to end with the real `bande-media-engine` child process and GStreamer: a song opens with its length and plays to the end. |
| `apps/player/tests/engine.rs` `a_file_that_is_not_media_fails_without_killing_the_engine` | Untrusted input: a file that is not media fails as unsupported, and the same engine still opens the next song. |
| `apps/viewer/tests/open.rs` `a_folder_steps_through_pictures_and_hostile_files_never_panic_hang_or_wake` | End to end through Viewer's window view: a picture opens, Next steps through its folder and wraps around, an animated GIF plays and schedules no wake-up once paused (nor does a still picture), and a truncated file, a GIF that declares 65535 × 65535 pixels and a FIFO each show an error instead of panicking, allocating the declared size or hanging. |
| `apps/viewer/tests/open.rs` `a_pdf_renders_in_its_own_process_finds_text_and_lists_its_outline` | End to end with the real `bande-viewer --pdf-worker` child process: a two-page PDF opens with its outline, a search finds the text on page 2 and scrolls there, and a damaged PDF shows an error instead of taking Viewer down. |
| `apps/text-editor/src/encoding.rs` `encodings_and_line_endings_survive_a_round_trip` | Tricky format logic: a Latin-1 file with CRLF endings must be detected and saved back byte for byte, a character the encoding cannot hold must be refused rather than written as `&#…;`, UTF-16 without a BOM must be recognised, and a BOM or a cut-off UTF-8 sequence must not lose the rest of the text. |
| `apps/text-editor/src/formats/mod.rs` `rich_formats_read_styles_and_write_them_back` | Tricky format logic: Markdown, RTF (with a list, a code page and `\u` escapes) and HTML (with scripts, nested lists and a table) must read as the right text, styles, alignment and lists, and write back to the same thing; a stray `*` or `_` must stay text. |
| `apps/text-editor/tests/versions.rs` `saving_over_a_file_keeps_old_versions_thinned_with_age` | Tricky date logic: saving keeps earlier versions, the same content as the newest is not kept twice, and versions older than a day are thinned to one. |
| `apps/text-editor/tests/untrusted.rs` `hostile_documents_open_within_bounds_without_panicking_or_hanging` | Security boundary: a FIFO must not hang the opener; RTF nested 200,000 groups deep, HTML with 300,000 unclosed tags and Markdown made of unmatched delimiters must open quickly without unbounded memory; a `.docx` whose part inflates past 64 MB, an `.odt` nested 100,000 elements deep and a truncated zip must be refused. |
| `apps/disks/tests/safety.rs` `the_system_disk_stays_untouchable_while_a_usb_stick_is_erased_and_ejected` | Security boundary: the system disk, its EFI system partition and the partition holding `/` offer no Erase, Restore or Eject and their commands do nothing; a spare partition there asks with its device, size and a system-disk warning; erasing a USB stick names it with its device and size and queues exactly one erase, nothing else while it runs. |
| `libs/vm/src/password.rs` `matches_glibc_sha512_crypt` | Tricky format logic: the password hash given to cloud-init must equal glibc's SHA-512 `crypt`, or the guest refuses the password. |
| `libs/vm/src/autostart.rs` `autostart_units_quote_what_systemd_would_expand` | Untrusted input: a machine name or path with `%`, spaces or a newline must not be expanded or split by systemd in the autostart unit. |
| `libs/vm/tests/qmp.rs` `events_around_replies_and_jobs_are_kept_in_order` | Tricky protocol logic against a fake QEMU on a socket pair: QMP events that arrive between replies and around a snapshot job are kept, in order. |

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
