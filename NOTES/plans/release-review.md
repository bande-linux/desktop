# Release review: open findings

Open findings of the pre-release reviews (2026-11-01: the whole workspace, then the apps,
compositor, shell and services). Fixed findings are deleted; their commits are in git. Severity:
**critical** lock-screen bypass or session-wide compromise; **high** a client, file or volume can
hang or crash a session component, run code or delete data; **medium** wrong behaviour, unbounded
growth or a weaker boundary than documented; **low** edge cases and hygiene.

## Workspace

- critical: any client (Flatpak included) can bind `zwp_input_method_v2` and read every key, and `zwp_virtual_keyboard_v1` types into the focused surface, the lock surface included (`state/globals.rs` filters `|_| true`); needs `wp_security_context_v1` to refuse these globals, layer-shell overlay, output management and output power to sandboxed clients, and virtual-keyboard keys dropped while locked.
- high: Control Center still runs `scan`, `networks`, `sign_in_page` and the Wi-Fi, Bluetooth and Do Not Disturb setters on the shell's UI thread (bounded by the 25 s bus timeout); tray/dbusmenu calls in `status/tray.rs` and `low_memory.rs` `force_quit` too.
- high: notification, MPRIS and tray images are decoded on the shell's UI thread (capped at 16 MiB); decode on a worker.
- high: the last display is never removed and a reconnect on the same connector keeps the old config (`displays.rs`); the lock's password field can land on a dead output. Needs the mini.
- medium: any shell-role client can unlock the session with a second `ext_session_lock_v1` (`lock.rs`); give bande-lock its own role.
- medium: `org.bande.Clipboard` serves the history to any session-bus caller; restrict to the shell and Settings.
- medium: the clipboard daemon reads offers one MIME at a time with no total deadline or MIME cap, and `pipe()` EMFILE ends it (`clipboard/src/main.rs`); undecodable and `.partial` history files are never deleted.
- medium: `wlr-output-management` `SetMode` accepts another head's mode and a failed `use_mode` is silent (`management.rs`).
- medium: toolkit paste and drop reads have no size cap, timeout or expiry (`platform/src/clipboard.rs`).
- medium: notify's `rule_for` remembers every new `app_name` and rewrites `notification-apps` each time.
- medium: `fs/listing.rs` builds paths from the lossy UTF-8 name, so non-UTF-8 names cannot be opened, copied or trashed; a volume `.trashinfo` `Path=` may be absolute or hold `..` (low).
- medium: thumbnails decode without `Limits` (512 MiB each, three at once, abort in dist).
- medium: a busy eject only logs (`volumes/eject.rs`); the user may pull a mounted drive.
- medium: Settings' Variable refresh rate switch writes a key nothing reads.
- medium: the desktop keeps the bar edge and icon size from when it was created (`desktop/client.rs`).
- medium: a missing configured wallpaper never falls back to the default and a failed load is never retried (`wallpaper/choice.rs`).
- medium: a keyboard layouts XML or read error gives an empty list with no log (`keyboard/layouts.rs`).
- medium: `SetWallpaper*` copies any readable file into the greeter folder; share only files that decode as images, log at warn (`login_picture.rs`).
- low (compositor): a thread leaked per Shift-five-times prompt; `recall.rs` app ids unescaped and never pruned; blocking D-Bus calls at start-up; `output_power` open to all and not pruned; unbounded cursor hint; `show_emoji_picker` from any client; shortcut inhibitors activated without consent.
- low (toolkit): configure-wait loops ignore `closed`; a keymap that fails to compile is ignored; a read error ends a paste as if complete; clipboard `send` threads without a deadline; preedit cursor offsets.
- low: lock drops PAM info and error messages (an expired account looks like a wrong password); polkit runs `getent passwd` synchronously; notify's `read_apps` treats a read error as empty and writes non-atomically.
- low: network `replace_password` overwrites a saved PSK before it is verified and `wait_for_activation` has no deadline.
- low: installer re-reads the previewed `.deb` at install time (copy it to a private folder); Software's removal confirmation omits `Obsoleting` packages; catalogue description errors become empty.
- low: tray register-then-disconnect race; sound has no cap on `Play` streams and keeps undrained ones; power has no per-client `Inhibit` cap.
- low: mount points unescaped lossily; `IN_Q_OVERFLOW` ignored; full name read from `/etc/passwd` instead of NSS.
- low (fs): names over 245 bytes cannot be trashed; unsafe zip names skipped without a failure; rename can overwrite (use `RENAME_NOREPLACE`).
- low: Terminal replies to a program that never reads stdin grow without bound (apps repo).
- low: file-chooser `*` matching backtracks exponentially; xdg drops field codes inside an argument (`--file=%f`).
- low: the screenshot shutter plays before the shot and failures are silent; desktop rename and new-folder errors only log; Settings shows a change as saved when its worker could not connect; taskbar `launcher` map grows from spoofed `LauncherEntry` ids; `Context::config()` drops errors.
- low: dead code: `packages::details_local`, spacing `XXL`/`XXXL`, a `Command::Reveal(_) => false` arm in Settings, test-only pub fns, 13 binary crates in `[workspace.dependencies]` nothing uses.

## Apps

- low: Activity Monitor's search field gives the input method no area (the `Screen` trait lacks `text_input_area`; apps repo).
- low: Settings `deadline()` covers only the display countdown, and `bande-app`'s `View for Ui` forwards neither `deadline` nor `tick` (tooltip delays).
- low: Settings reads NetworkManager signals twice while a network pane is open.
- low: every Files window loads `Apps` and the thumbnailer again on the UI thread.
- low: Terminal loads a settings change once per app and again per window (apps repo).
- low: `bande-app` rebuilds the menu model after every handled event, pointer moves included.
- low: a portal app id with a tab or newline is written unescaped into `file-chooser-folders`.

## Compositor

- medium (hardware): `drm.rs` `relink` commits `link-status = GOOD` outside `DrmCompositor` and can fail with EBUSY while a flip is pending; check with an external display through standby.
- low: `force_quit_due` rebuilds the app list every loop iteration while Force Quit is open.
- low: a second button pressed inside Force Quit replaces `held`, so the first release goes to the window below.

## Shell

- low: Control Center shows the last Wi-Fi and Bluetooth state for its first frame and does not follow signals while open.
- low: the status hub broadcasts `Update::Menu` to every bar; with several displays a bar may show another bar's tray menu (needs a requester id on `Errand::ShowMenu`).
- low: Trash checks on overlapping threads can finish out of order.
- low: redrawable wallpapers are redrawn synchronously on the UI thread when the GPU drops them.
- low: the lock reads the user's configured wallpaper with `fs::read` (no size or kind check).

## Services

- low: any client may close or replace another app's notification by id.
- low: the `org.freedesktop.impl.portal.*` methods accept any session caller; only restore data and stored permissions are frontend-checked.
- low: 200 large notifications can push `PresentationChanged` past the bus message limit.
- low: `bande-power` keeps every `wl_output` it saw, calls logind synchronously in `Inhibit`, and `idle_for` and `set_inhibited` can interleave.
- low: clipboard paste threads block on `write_all` without a deadline.
- low: automount's `ActionInvoked` match accepts any sender.
- low: `ci/release/check-debs.sh` does not check `org.bande.NotificationCenter.service`.
