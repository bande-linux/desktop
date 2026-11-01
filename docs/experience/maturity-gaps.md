# Maturity gaps: Bande against a mature desktop

Audit of 2026-10-30. The question: what would a picky user who has lived with macOS 15,
Windows 11, GNOME 48 and Plasma 6 expect from Bande that it lacks or does poorly? Nothing was
built or run. The method:

- Read the docs: `architecture/behaviour.md`, `architecture/session.md`,
  `architecture/compositor.md`, `design/datum.md`, `parity.md`, `roadmap.md`.
- Read the code: `crates/shell`, `crates/apps`, `crates/platform/compositor`, `crates/services`,
  and the libraries they call.
- Looked at the review screenshots in the agents' report folder.
- Read the public documentation of the four reference desktops (sources at the end).

Each gap below was checked in the code, and the evidence (file or doc) is named in the row.

**Status, re-checked against `main` on 2026-10-31.** The last column of every table says
what has happened since: **Done** with the commit where it landed, **Partly** with what is
left, **Open**, or **Owner decision** for items that wait on the owner. Rows without a commit
landed on the `day/maturity-next` branch.

**Where Bande already meets the bar.** These need no more than polish:

- the app switcher and same-app window cycling;
- the overview and App Exposé;
- per-display workspaces with 1:1 swipes;
- snapping with Fill the Other Half;
- the display arrangement with a timed revert;
- clipboard persistence and history;
- primary selection;
- drag and drop with Escape to cancel;
- input source switching that also works while locked;
- zoom;
- notification grouping and Do Not Disturb;
- the screenshot palette, recording and Markup;
- screen sharing through the portal;
- battery alerts;
- event sounds;
- menu search.

**Already being worked on tonight, so not repeated here:**

- busy CPU loops;
- the bar's top icon vanishing on right click;
- sleep and wake bugs;
- the handover from the login window to the desktop;
- Files depth against the Finder;
- the structure of Settings and its missing options;
- Activity Monitor depth;
- polish of Terminal, Screenshot, the file chooser and the launcher.

Rows that touch these areas say "(→ that work)" and name only what lies outside it.

**Severity**:

- **blocker**: data loss, an unsafe default, or a basic task that cannot be done;
- **major**: noticed in the first day, and makes the desktop feel unfinished;
- **minor**: noticed by the picky over weeks.

**Effort**:

- **S**: one agent, 1–2 hours;
- **M**: about a day, which splits into 2–4 S tasks;
- **L**: several days, or a design decision is needed first.

## 1. First run, login and lock

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| No setup assistant and no welcome on first login (`session.md`: "planned") | Every reference desktop greets a new account: macOS Setup Assistant, the Windows setup and Get Started, the GNOME Tour, the KDE Welcome Center. Here the first login lands on the defaults, with no hint that `Super+Space` or `Shift+Super+5` exist | major | L | new surface; owner decision | Owner decision |
| The login window has no Sleep, Restart or Shut Down (`session.md`: "Also not done") | To turn the computer off from the login window you press the hardware key, which logind turns into an instant power-off (§12), or you sign in first. Every reference login screen offers power actions | major | S | `shell/greeter` | Done (`1a9e996`) |
| The login window has no accessibility controls: zoom, larger text, an on-screen keyboard | GNOME and Windows put an accessibility menu on the login screen, and macOS shows Accessibility Options at login. A person with low vision cannot sign in comfortably | major | M | `shell/greeter`, `platform/compositor` | Open |
| The user picker is Up and Down only, with no list of people and pictures. PAM's visible prompts (one-time codes) go into a secure field. There is no session chooser | Shared family computers; a code you cannot see as you type it | minor | M | `shell/greeter` | Open |
| The lock screen shows the time, the name and the password field, nothing more: no battery, network, Now Playing or notification count (`shell/lock/src`) | "Is my laptop about to die?" is answered on every reference lock screen: GNOME's top bar, Windows' corner icons, macOS' status icons | major | S | `shell/lock` | Done (`1a9e996`) |
| There is no Users pane: you cannot change your password, picture or full name, or add an account (no AccountsService code anywhere; → Settings options) | `passwd` in a terminal is not an answer on a desktop | major | M | `apps/settings` | Open |
| No fast user switching (roadmap: Later) | Families sharing one Mac-like laptop | minor | L | `shell/greeter`, `shell/lock` | Open |

## 2. Session lifecycle: ending, restoring, crashing

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| Log Out, Restart and Shut Down end the session without asking apps to quit. `end_session` (`libs/status/src/devices.rs`) calls logind's `Reboot`, `PowerOff` or `Terminate` at once, so every app gets SIGTERM | Unsaved work is lost without a save prompt: a Text Editor document, a Firefox form, a LibreOffice file. macOS asks every app to quit, and an app with unsaved changes cancels the logout. GNOME and Windows list the apps that hold it up | blocker | M | `shell/shell`, `platform/compositor`, `libs/status` | Done (`b8088c9`) |
| No polkit authentication agent runs, and no polkit rules let administrators skip the prompt. `services/polkit` is a library only ("until then no agent binary is packaged", `session.md`) | Settings calls timedated, localed, hostnamed and PackageKit with `interactive=true` (`apps/settings/src/time.rs`, `locale.rs`, `about.rs`, `software.rs`). Without an agent or rules, each of these calls that needs `auth_admin` fails, and so do NetworkManager system connections, mounting internal disks and system-wide Flatpak installs | blocker | S (rules) + M (agent) | packaging of `apps/settings`, `services/polkit` | Done: agent (`a38b0df`) and rules for administrators (`3cc1848`) |
| Nothing reopens at login after a restart | macOS reopens apps and windows ("Reopen windows when logging back in", on by default), and Plasma restores the session. Bande already remembers window frames (`platform/compositor/src/recall.rs`) but not which apps were open | major | M | `shell/shell`, `libs/xdg` | Done (`29edf99`) |
| A failed launch or a crash is silent. `spawn_detached` (`libs/xdg/src/launch.rs`) waits on the child in a thread and drops its exit status, and the bar's launch pulse just stops after 10 s | macOS says "X can't be opened" and "X quit unexpectedly", with a Reopen button. On a pure Wayland desktop, an X11-only app (Steam, an old Electron app) simply never appears | major | S | `libs/xdg`, `shell/bar`, `shell/launcher` | Done (`79d0b41`) |
| Apps are not placed in systemd scopes (`app-<launcher>-<id>-<random>.scope` in `app.slice`, as systemd.io/DESKTOP_ENVIRONMENTS asks); they run inside the compositor's unit | There is no per-app memory protection or OOM order, and the journal cannot say which app logged what. Activity Monitor and a crash notice cannot tell apps apart. GNOME and Plasma both use scopes | major | S | `libs/xdg` | Done (`79d0b41`) |
| A compositor crash ends the session and every app | macOS' WindowServer behaves the same, but Plasma 6's Qt apps survive a KWin crash | minor | L | `platform/compositor`, `toolkit/platform` | Open |
| No crash reporter (`session.md`: "planned") | Picky users want to file a report; a journal link in the crash notice would do at first | minor | M | `services`; later | Open |

## 3. Window management

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| The zoom control has no tiling menu on hover or hold. Tiling is reached only by a shortcut, a drag to the edge or the window menu (nothing in `toolkit/platform/src/chrome.rs` or `decoration.rs`) | When the pointer rests on the green button, macOS 15 shows Move & Resize. When it rests on the maximise button, Windows 11 shows Snap layouts. Tiling is only useful once people can find it | major | S | `toolkit/platform`, `platform/compositor` (server-side frames) | Done (`2efda70`, server-side frames `4197efa`) |
| Show Desktop has no keyboard path, only the four-finger spread (`platform/compositor/src/gesture.rs`) | DESIGN §2 asks for every action to be reachable from the keyboard. macOS has Fn-F11 and Command-Mission Control, Windows has Win+D, Plasma has Meta+D | major | S | `platform/compositor`, `libs/keyboard` | Done (`2efda70`) |
| No modifier-drag to move or resize a window from anywhere in it | GNOME has Super-drag, Plasma Meta-drag, macOS 15 Control-Command-drag. It is the rescue when a title bar ends up off screen | minor | S | `platform/compositor` | Done: Control-Super drag moves, Control-Super right drag resizes |
| The overview cannot close a window: no × on hover, no middle click (`overview.rs`) | GNOME, Plasma and Windows Task View all close windows from the overview | minor | S | `platform/compositor` | Done: middle click closes; no × on hover yet (rendering) |
| No hot corners. The research makes them opt-in, but no setting exists (`compositor.md`) | GNOME's Activities corner and macOS hot corners are muscle memory for many people | minor | S | `platform/compositor` (+ Settings options) | Open (needs a setting) |
| Tiling stops at halves and quarters | Windows has thirds and Snap groups, and Plasma custom tiles | minor | M | `platform/compositor` | Open |
| No `keyboard-shortcuts-inhibit` | In a virtual machine (GNOME Boxes, virt-manager) or a remote desktop, `Super+Tab` and `Super+Q` act on the host | minor | S | `platform/compositor` | Done: the focused window may take the shortcuts; Force Quit and Lock stay |
| No `pointer-constraints` or `relative-pointer` | Games, Blender and the browser's Pointer Lock API (web games, remote desktops) cannot capture the mouse | major for those users | M | `platform/compositor` | Done: lock, confine and relative motion while the window has focus |

## 4. Keyboard shortcuts and discoverability

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| Super acts as Command only in Bande's own apps. Firefox, GTK, Qt and Electron apps keep `Ctrl+C`, `V`, `Z`, `S`, `F` and `T`, because the compositor translates nothing (`architecture/overview.md`, keyboard model) | The keyboard model promises that `Super+C` copies in every app. In practice copy is `Super+C` in Files and `Ctrl+C` in Firefox, and in a third-party terminal `Ctrl+C` interrupts. On a Mac, Command works everywhere, and Toshy and Kinto exist because Linux breaks this. For Firefox alone, `ui.key.accelKey` can be set to Super instead, but editing keys did not follow it until Firefox 157 (Mozilla bug 2069698) | major | M | `platform/compositor`; owner decision | Done: the other way round. Ctrl is the app key in Bande's own apps too and Super is the system key, as in GNOME and KDE, so nothing is translated ([decision 0005](../decisions/0005-ctrl-for-app-shortcuts.md)) |
| No custom shortcuts (to launch an app or run a command) and no rebinding. Settings ▸ Keyboard lists the shortcuts read-only, and `services/config` has no shortcut section | All four reference desktops allow both | major | M | `services/config`, `platform/compositor` (+ Settings options) | Open |
| No shortcut opens Notification Center or Control Center | macOS has Fn-N and Fn-C, Windows Win+N and Win+A, GNOME Super+V and Super+S. Someone who works only with the keyboard cannot read their notifications | major | S | `platform/compositor`, `platform/protocols`, `shell/bar` | Done (`2efda70`) |
| The shortcut table (`libs/keyboard/src/shortcuts.rs`) leaves out shortcuts that are bound: `Shift+Super+5`, `Alt+Shift+Super+arrows` (move to the next display), ``Shift+Super+` `` and the Mission Control key | The list in Settings is the only cheat sheet, so a missing row hides the feature | minor | S | `libs/keyboard` | Done (`13ae4fe`) |
| The Print Screen key does nothing | It is the first thing anyone with a PC keyboard presses | minor | S | `platform/compositor` | Done: opens the capture options, like Shift+Super+5 |
| Tapping Super alone does nothing | Windows, GNOME and Plasma open their launcher on a Super tap; macOS does nothing. The owner decides | minor | S | `platform/compositor` | Owner decision |

## 5. Text input and fonts

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| Emoji & Symbols types its character only when an input method runs; otherwise it copies it ("Return to copy, then paste with Ctrl+V", `shell/launcher` strings) | macOS inserts the character into the field, and so does Windows' Win+. picker. Copying and then pasting feels broken | major | M | `platform/compositor` (commit through text-input itself), `shell/launcher` | Done: types into any focused text field through text-input; copies only without one |
| Input methods are not offered as input sources (`behaviour.md`), and fcitx5 is only Suggested | A Chinese, Japanese or Korean user must install and set up fcitx5 by hand before typing a word. GNOME and Plasma list input methods next to the layouts | major (CJK) | M | `platform/compositor`, `apps/settings`, packaging | Open |
| No press-and-hold accent picker | Typing é, ü or ø on a US layout this way is a macOS signature | minor | M | `platform/compositor` | Open |
| No fallback fonts for other scripts: `fonts-noto-core` and `fonts-noto-cjk` are in no package's Depends or Recommends (`packaging.md` lists them as not yet pulled in) | Arabic, Hebrew, Thai, Devanagari and CJK file names, notifications and launcher results show tofu boxes | major | S | `packaging/bande-desktop` | Done: `fonts-noto-core` and `fonts-noto-cjk` recommended |
| No spell checking in toolkit text views | Little impact: our apps take little prose, and the default editor checks spelling itself | minor | M | `toolkit/widgets` | Open |
| No Paste and Match Style | macOS has Option-Shift-Command-V | minor | S | `toolkit/widgets` | Open |
| Sizes and numbers ignore the region: `format_size` (`libs/fs/src/listing.rs`) always writes "110.37" | With the region set to Sweden, a Swede expects "110,37 GB" | minor | S | `libs/fs`, `libs/l10n` | Open |

## 6. Accessibility

The screen reader is skipped by the owner's decision: its switch appears only when Orca is
installed.

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| Larger text: the portal serves `text-size`, but the toolkit does not follow it and Settings has no control for it (`behaviour.md`) | Larger text is the most used accessibility setting on every platform | major | M | `toolkit/graphics`, `toolkit/widgets` | Open |
| No colour filters: greyscale, invert, colour blindness | macOS Display ▸ Colour Filters, Windows Color filters, Plasma's colour blindness correction | minor | M | `platform/compositor`, `platform/render` | Open |
| No on-screen keyboard and no mouse keys | macOS has the Accessibility Keyboard, and Windows and GNOME have an on-screen keyboard | minor | L | `platform/compositor` | Open |
| Backlog P6 follow-ups: a full keyboard navigation switch, shaking to find the pointer, a latched-modifier indicator, zoom following the caret | Small touches that people expect | minor | S each | `platform/compositor`, `toolkit` | Partly: zoom follows the focused window; the rest open |

## 7. Notifications

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| The `image-data` and `image-path` hints are ignored: `notify` (`services/notify/src/lib.rs`) reads only the sound, urgency, desktop entry, resident and transient hints | Chat apps send the sender's picture and browsers the site's icon; macOS shows the contact's picture. The card can already draw an image (`shell/notifications/src/card.rs`) | major | S | `services/notify`, `shell/notifications` | Done (`ee3d22e`) |
| Banners keep showing while the screen is shared or recorded | By default, macOS turns notifications off while mirroring or sharing the screen, and Windows turns Do Not Disturb on while duplicating the display. Here a private message can pop up during a video call | major | S | `services/notify`, `services/portal`, `shell/screenshot` | Done (`ee3d22e`) |
| A click on the clock shows no calendar (there is no calendar anywhere in `shell/`) | Windows, GNOME and Plasma open a month calendar from the clock, and macOS' Notification Center has a Calendar widget | major | S–M | `shell/notifications`, `libs/datetime` | Done (`ee3d22e`) |
| No Do Not Disturb schedule, and no automatic Do Not Disturb for full-screen apps and games | macOS has Focus schedules, and Windows has automatic rules | minor | S | `services/notify`, `services/config` | Partly: quiet while the display in use shows a full-screen app; no schedule |
| No notifications on the lock screen, neither a count nor a summary | GNOME and macOS list them, with a privacy switch | minor | M | `shell/lock`, `services/notify` | Open |
| No inline reply (roadmap: Later) | Plasma and macOS have it | minor | M | `services/notify`, `shell/notifications` | Open |

## 8. Search, clipboard, drag and drop

The clipboard is on par: persistence, opt-in history, primary selection and skipped secrets. All
that is left is Paste and Match Style (§5), and copying a capture to the clipboard (→ Screenshot
polish).

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| File search matches names in the home folder only, never contents (`libs/search`) | Spotlight, GNOME (TinySPARQL) and Plasma (Baloo) find a PDF by a phrase inside it | major | L | `libs/search` | Open |
| The launcher has no system commands (Lock, Sleep, Restart, Log Out, Empty Trash) and no "Search the web" fallback (→ launcher polish) | KRunner has both, and Spotlight offers web suggestions | minor | S | `shell/launcher` | Done: Lock Screen, Sleep, Log Out, Restart and Shut Down; web search is a Never |
| No spring-loading. A dragged file held over a taskbar icon does not bring that app's window forward, and a folder on the desktop does not open under a held drag (→ Files depth for its own folders) | Windows brings the window forward from the taskbar, and macOS springs folders and Dock items open. Without it, dropping into a covered window means rearranging windows first | major | M | `shell/bar`, `shell/desktop` | Open |
| Dropping a file on the app header (`features/bar.md`) is specified but not built | | minor | S | `shell/bar` | Open |

## 9. Files, default apps and app identity

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| A window's app id is matched to its desktop entry only exactly (`entry` in `shell/bar/src/taskbar_client.rs` and `menu_bar_client.rs`): no `StartupWMClass`, no case-insensitive match, no fallback to the last reverse-DNS part or the `Exec` name | GIMP 3 (`gimp` against `org.gimp.GIMP`), Slack, Signal, JetBrains IDEs and many Electron apps get a letter placeholder and show their raw id in the bar header (the review capture `night/bar-reliability/mark-right-click.png` shows "org.…"). They also get a second icon next to the kept one, and no About or desktop actions | major | S | `libs/xdg`, `shell/bar`, `platform/compositor` (switcher), `shell/launcher` | Done (`79d0b41`) |
| No app opens audio files: `bande-mimeapps.list` has no `audio/*` entry and no audio player is recommended (Decibels is not packaged in Debian 13) | Double-clicking an MP3 does nothing useful; every reference desktop plays it | major | S | `packaging/bande-desktop` | Owner decision |
| No mail client and no `mailto:` handler | Clicking an email link does nothing. That is fine if the desktop says why | minor | S | packaging, `libs/xdg` | Open |
| Nowhere to choose the default browser, mail, music or video app (→ Settings options) | macOS has Desktop & Dock ▸ Default web browser, and GNOME and Windows have Default Apps | major | S | `apps/settings` | Done (`3cc1848`) |

## 10. Displays

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| No Night Light: there is no gamma or colour matrix code and no `wlr-gamma-control`, so `gammastep` cannot fill in either | macOS has Night Shift, Windows Night light, GNOME and Plasma Night Light | major | M | `platform/compositor`, `apps/settings` | Done (`2efda70`) |
| No colour management: sRGB content is not converted for the MacBook's P3 panel, and there are no ICC profiles and no HDR | Photos and web colours look oversaturated next to macOS. GNOME 48 and Plasma 6 manage colour | major (creative work) | L | `platform/render`, `platform/compositor`, `libs/edid` | Open |
| No automatic brightness from the ambient light sensor | MacBooks have one, and macOS uses it by default | minor | M | `services/power` | Open |
| No custom display modes (`wlr-output-management`: "no custom modes") | Rarely needed | minor | M | `platform/compositor` | Open |

## 11. Input devices, sound, Bluetooth and network

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| No microphone, camera or screen-in-use indicator in the bar. Screen sharing has its own symbol, and `libs/audio` already sees capture streams in the PipeWire graph | People look for this privacy cue: macOS shows orange and green dots, and GNOME and Windows show indicators | major | S–M | `libs/audio`, `shell/bar` | Done (`7f10f23`) |
| No captive portal sign-in. `libs/network` never reads NetworkManager's `Connectivity`, and Debian only checks connectivity once `network-manager-config-connectivity-debian` is installed | Hotel and train Wi-Fi "connects", and then nothing loads. macOS, Windows and GNOME open the sign-in page | major | S–M | `libs/network`, `shell/bar`, packaging | Done (`29edf99`) |
| Wi-Fi has no Forget, no auto-join switch, no hidden network and no WPA-Enterprise (eduroam) | A university or office network cannot be joined, and a wrong network cannot be removed | major | M | `libs/network`, `shell/controlcenter`, `apps/settings` | Partly: Forget (`ab3e4da`); auto-join, hidden and Enterprise open |
| Bluetooth has no Forget (`RemoveDevice`) and no device battery (`org.bluez.Battery1`). The pairing agent runs only inside Settings (`apps/settings/src/services.rs`), so a device that starts pairing while Settings is closed is refused | macOS and GNOME show the battery of AirPods and mice, and removing a device is basic | major | S | `libs/bluetooth`, `shell/controlcenter` | Done (`29edf99`, `ab3e4da`) |
| Pointer acceleration profile (flat), primary button side, scroll speed and middle-click emulation are not applied: `pointing.rs` sets tap, typing detection, click method, natural scrolling and speed only (→ Settings options for the controls) | Gamers and left-handed people notice. macOS 14 added a pointer acceleration switch. libinput has no scroll speed, so the compositor would have to scale it | minor | S | `platform/compositor` | Open (needs Settings controls) |
| No choice of headset profile: high-quality playback or the headset microphone | Calls on Bluetooth headphones | minor | M | `libs/audio`, `shell/controlcenter` | Open |
| No three-finger drag (roadmap: later) and no tablet input | Three-finger drag is a macOS favourite; tablets matter to artists | minor | M | `platform/compositor` | Open |

## 12. Power, storage, updates and printing

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| The hardware power key turns the computer off at once. Bande holds only `handle-lid-switch` (`services/power/src/adapter.rs`), so logind's default `HandlePowerKey=poweroff` applies | On a Mac with Touch ID, a press of that key locks the screen. Here an accidental tap loses all open work | blocker | S | `services/power`, `platform/compositor` | Done: the key puts the computer to sleep (`dcb97a0`) |
| No low disk space warning | macOS says "Your disk is almost full", Windows has Storage Sense, GNOME's housekeeping warns. A full disk breaks logins and updates | major | S | `services/automount` | Done (`8a3ff11`) |
| No printing: cups is not recommended, there is no Printers pane, and the Print portal is GTK's | A home or office user cannot print a PDF from Papers until they find and install cups | major | S (packaging) + L (Printers pane, Later) | `packaging/bande-desktop`, `apps/settings` | Partly: `cups` recommended; no Printers pane |
| No charge limit to preserve the battery (Asahi exposes `charge_control_end_threshold`) | macOS has Optimised Battery Charging, GNOME 48 "Preserve Battery Health" | minor | S–M | `services/power`, `apps/settings` | Open |
| The keyboard backlight stays on when idle, and idle times are the same on battery and on power | macOS turns the keyboard light off after a while of inactivity, and macOS and GNOME treat battery and power separately | minor | S | `services/power` | Partly: the keyboard light goes dark with the display; one set of idle times |
| Updates only install live (`apps/settings/src/software.rs`). There is no offline update at restart, no switch for automatic security updates and no list of what changes, and session services are not restarted after an update (`session.md`: "planned") | GNOME Software installs at reboot, and macOS shows what an update contains | minor | M | `apps/settings` | Open |

## 13. App integration and portals

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| GTK apps miss the appearance. The portal serves no `gtk-theme`, `font-name`, `document-font-name`, `monospace-font-name` or `enable-animations`. Unsandboxed GTK 3 apps read GSettings rather than the portal (`gdk_should_use_portal` in GTK 3's `gdkscreen-wayland.c`), and nothing writes dconf | GIMP 3, Inkscape, LibreOffice and Evolution stay light in dark mode, keep the Adwaita icons and ignore larger text. GTK 4 apps draw in DejaVu Sans next to our Inter, which is visible in every screenshot that holds both | major | S | `services/portal`, `platform/compositor` (`data/bande-session`) | Done (`ee4896a`) |
| The portal covers Settings, FileChooser, Screenshot, ScreenCast and Secret, and everything else falls to GTK. Wallpaper, GlobalShortcuts, RemoteDesktop, InputCapture, Background and Clipboard are not implemented (`crates/services/portal/data/bande-portals.conf`) | "Set as Background" in Loupe or Firefox changes GNOME's setting, and nothing changes on screen. Push-to-talk in Discord or OBS gets no global shortcut | major | S (Wallpaper), M (each other) | `services/portal` | Partly: Wallpaper (`da88ae2`); the others open |
| Portal permissions cannot be reviewed or revoked: the screenshot "Allow", remembered screen-share sources, Flatpak background and notification rights (→ Settings structure) | macOS has Privacy & Security, and GNOME the Apps and Privacy panes | major | M | `apps/settings` | Open |
| Portal dialogs are not attached to their app's window, because `xdg-foreign` is missing: the file chooser, the screenshot permission, the share picker (→ file chooser polish for the chooser) | They can open behind the app or on another workspace, and they do not follow it | major | M | `platform/compositor`, `services/portal` | Open |
| Third-party apps use protocols the compositor lacks: `xdg-toplevel-icon` (an app sets its own icon), `xdg-toplevel-drag` (tearing off a browser tab), `content-type` and `tearing-control` (games and video), `single-pixel-buffer` and `security-context` (Flatpak isolation) | Small seams, one at a time | minor | S each | `platform/compositor` | Open |
| The Print, Open With and Access dialogs come from GTK and look like GNOME | Mixed looks within one task | minor | L | `services/portal` | Open |

## 14. Motion, consistency, error states

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| Some colours are still hard-coded instead of tokens (`shell/bar/src/taskbar.rs`, `shell/bar/src/menu_search.rs`) | Contrast and accent changes skip those pixels | minor | S | `shell/bar` | Open: only shadow colours remain literal |
| Settings reports failures as "Not changed: { $error }" with the raw D-Bus text. The compositor's start-failure alert is hard-coded English (`platform/compositor/src/alert.rs`) | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" is not a message for a person | minor | S | `apps/settings`, `platform/compositor` | Done: Settings words failures (`3cc1848`); the start alert is localised |
| The docs describe the first draft. `parity.md` lists finished features as "none". `compositor.md` and `behaviour.md` speak of traffic lights and translucency | They mislead agents and reviewers | minor | S | `docs` | Open |

## 15. Localisation, startup and memory

| Gap | Why a picky user notices | Severity | Effort | Owner | Status |
| :--- | :--- | :---: | :---: | :--- | :--- |
| English only: 24 `.ftl` files hold about 1,140 strings, all `en-US`. Settings ▸ Language changes the system locale, but Bande's own interface stays in English | A Swedish, German or French user gets Firefox and the GNOME apps in their language, and Bande's own shell in English | blocker (non-English) | L (S per language per group of crates) | every crate's `locales/`, `libs/l10n` | Owner decision |
| No pseudo-locale and no string length check in CI | Truncated text and hard-coded strings slip through | minor | S | `libs/l10n`, `ci` | Open |
| No right-to-left mirroring (roadmap: later) | | minor | L | `toolkit` | Open |
| No measured budget for login to a usable desktop, or for an app's first frame (Files, Settings): `performance.md` measures memory and frames, not cold starts | "Fast" is never checked | minor | S | `testing/bench` | Partly: time to desktop measured (`418ed6d`); app first frames open |
| Every graphical program carries the whole stack and its own copy of the ICU segmentation data (`performance.md`) | Disk and page cache; the one-process shell already solved this for the shell | minor | L | packaging | Open |

## Top 24: user impact ÷ effort

Each item is scoped so that one agent can finish it in 1–2 hours, with the proof the night
brief asks for.

**Status on 2026-10-31:** items 1–4, 6–10 and 12–23 are done. Item 5 is done except the
audio player, and item 11 waits on the owner. Item 24, Larger Text, is open. The former item 11,
translating Super to Ctrl for third-party apps, was dropped on 2026-11-01: Ctrl is now the app
key everywhere ([decision 0005](../decisions/0005-ctrl-for-app-shortcuts.md)). Of the
"next" items, modifier-drag, Wi-Fi Forget, closing from the overview and typing emoji are
done; hidden networks and Do Not Disturb schedules are open.

**Ranking.** Impact is scored 1–10, with blockers at 8–10. Effort counts S as 1, S–M as 1.5
and M as 2. Items with the same score are ordered by impact.

1. **The power key locks instead of turning the computer off** (`services/power`,
   `platform/compositor`; S; blocker).
   - Hold a logind `handle-power-key` block inhibitor for the session, as `adapter.rs` does for
     the lid.
   - Bind `XF86PowerOff` in `input.rs` to lock the session, as on a Mac with Touch ID. Holding
     the key still forces the power off in firmware.
   - Proof: a power test against the fake logind of `services/power/tests/logind.rs`
     (inhibitor taken, released at exit), and a headless compositor test showing that the key
     locks.
2. **Administrators can change the time zone, language and computer name without an agent**
   (packaging of `apps/settings`, `ci/polkit`; S; blocker).
   - Ship a polkit rules file as gnome-control-center does: `YES` for local, active members of
     `sudo` on the timedate1, locale1 and hostname1 actions that Settings calls.
   - Check the action ids with `pkaction` on Debian 13.
   - Proof: in the `ci/polkit` container, `pkcheck` authorises a member without an agent and
     refuses a non-member.
3. **One app identity matcher** (`libs/xdg`, `shell/bar`, `platform/compositor`,
   `shell/launcher`; S).
   - Parse `StartupWMClass` in `entry.rs`.
   - Add one lookup that tries, in order: exact, case-insensitive, `StartupWMClass`, the last
     reverse-DNS part, the `Exec` name.
   - Use it in the taskbar (apps and header), the switcher and overview names, and the launcher's
     running apps.
   - Proof: unit tests with real pairs (`gimp`/`org.gimp.GIMP`, `Slack`/`slack`,
     `signal`/`signal-desktop`), and a bar golden in which a mismatched id shows its own icon
     and name.
4. **GTK apps follow the appearance and the fonts** (`services/portal`, `platform/compositor`
   `data/bande-session`; S).
   - Add `gtk-theme` (Adwaita or Adwaita-dark), `font-name`, `document-font-name` (Inter),
     `monospace-font-name` (JetBrains Mono) and `enable-animations` (off under Reduce Motion)
     to the portal's `org.gnome.desktop.interface`.
   - Make unsandboxed GTK 3 apps read the same values: either `GTK_USE_PORTAL=1` in
     `bande-session` (which also sends their Open and Save dialogs to our chooser) or a write
     to dconf. Decide by testing GIMP and gnome-text-editor on Debian 13.
   - Proof: a portal test of the keys before and after a switch, and `ci/portal` screenshots.
5. **A fresh install reads every script, prints and plays music** (`packaging/bande-desktop`;
   S).
   - Recommend `fonts-noto-core`, `fonts-noto-cjk` and `cups`.
   - Recommend an audio player available in Debian 13 (Totem or Rhythmbox; check Showtime's
     MIME types first), and add the `audio/*` defaults to `bande-mimeapps.list`.
   - Proof: a `lintian`-clean build, and a review screenshot of an Arabic and a Japanese file
     name in Files.
6. **Apps are asked to quit before Log Out, Restart and Shut Down** (`platform/compositor`,
   `platform/protocols`, `shell/shell`, `libs/status`; M; blocker).
   - The confirmation first asks every app to quit, through the `Super+Q` path for each app id.
   - It waits up to 10 s for their windows to close.
   - If a window stays (a save sheet), it cancels and says "<App> is keeping your computer
     from restarting", with Cancel and Restart Anyway.
   - Proof: a headless compositor test with a client that refuses to close.
7. **Low disk space warning** (`services/automount`; S).
   - Check `statvfs` of `/` and of the home folder every 10 minutes and after each mount.
   - Below 5 % or 1 GB, post a notification with Manage Storage… that opens
     `bande-settings storage`. Repeat only after the space recovers.
   - Proof: a unit test of the threshold rule, and a notification test on the test bus.
8. **Notifications show pictures** (`services/notify`, `shell/notifications`; S).
   - Read `image-data` (iiibiiay), `image-path` and the old `icon_data`.
   - Prefer the image over `app_icon` in the card, as the spec orders.
   - Proof: a model test for each hint, and a notification golden with a sender picture.
9. **No banners while the screen is shared or recorded** (`services/notify`, `services/portal`,
   `shell/screenshot`; S).
   - The portal's ScreenCast sessions and the recorder tell `bande-notify` that sharing is
     on.
   - While it is on, the center behaves as under Do Not Disturb: critical notifications still
     show, and nothing is replayed afterwards.
   - Proof: a `Center` test with an injected clock, and a portal test that a session start
     sets the state.
10. **Sleep, Restart and Shut Down on the login window** (`shell/greeter`; S).
    - Three buttons that the keyboard can reach, calling logind as the greeter's session
      (polkit allows the active seat).
    - Restart and Shut Down ask first, as the system menu does.
    - Proof: a greeter test against a fake logind (as `services/power/tests/logind.rs` serves
      one), and light and dark goldens.
11. **Swedish for the shell** (`crates/shell/*/locales`, `libs/keyboard`, `toolkit/*/locales`;
    M).
    - Add `sv-SE` `.ftl` files for the bar, launcher, notifications, Control Center, lock
      screen, login window, desktop and toolkit strings.
    - Proof: an `bande-l10n` test that every `sv-SE` key exists in `en-US` and resolves, and
      a review screenshot of the bar and launcher.
    - Owner: the owner's language first; the same task repeats per language and group of
      crates.
12. **Tell the user when an app fails to open or quits unexpectedly** (`libs/xdg`,
    `shell/bar`, `shell/launcher`; S–M).
    - `spawn_detached` reports the exit through a channel.
    - An exit before the first window: "<App> could not be opened", with its last stderr line.
    - An exit by a signal: a "<App> quit unexpectedly" notification with Reopen.
    - Proof: tests with `/bin/false` and with `sh -c 'kill -SEGV $$'`.
13. **Microphone and camera in-use indicator** (`libs/audio`, `shell/bar`; S–M).
    - Detect `Stream/Input/Audio` and `Stream/Input/Video` nodes of other apps, leaving out
      our own level meter.
    - The bar shows an accent dot, and its menu lists the apps that use them.
    - Proof: a graph model test, and a bar golden.
14. **Captive portal sign-in** (`libs/network`, `shell/bar`, packaging; S–M).
    - Follow NetworkManager's `Connectivity`. On `PORTAL`, post "Sign in to <network>", which
      opens the browser at the check URI; the Wi-Fi menu shows the same line.
    - Recommend `network-manager-config-connectivity-debian`.
    - Proof: a network model test on the NetworkManager fake of `testing/test-bus`.
15. **A month calendar in the Notification Center** (`shell/notifications`, `libs/datetime`;
    S–M).
    - Add a month grid above the stacks, starting on the locale's first weekday (ICU4X), with
      today marked and arrows (and Page Up and Down) to change month.
    - Proof: a `libs/datetime` test of week starts for en-US and sv-SE, and light and dark
      goldens.
16. **Apps run in their own systemd scopes** (`libs/xdg`; S).
    - After spawning, move the pid into `app-bande-<id>-<random>.scope` in `app.slice`
      through `org.freedesktop.systemd1.Manager.StartTransientUnit`.
    - Skip this quietly without a user manager.
    - Proof: a unit test of the unit name (no `-` in the id part, as systemd requires), and a
      VM check with `systemctl --user status`.
17. **"Set as Background" works** (`services/portal`; S).
    - Implement `org.freedesktop.impl.portal.Wallpaper.SetWallpaperURI`: copy the picture as
      Settings ▸ Wallpaper's "Add Picture" does, write it to the config, and add it to
      `bande.portal` and `bande-portals.conf`.
    - Proof: a portal test that calls it and reads back `org.bande.Config`.
18. **Shortcuts for Show Desktop, the Notification Center and Control Center**
    (`platform/compositor`, `platform/protocols`, `shell/bar`, `libs/keyboard`; S–M).
    - Bind keys the owner approves (proposal: `Super+F11` and Super with the Mission Control
      key for Show Desktop, `Alt+Super+N`, `Alt+Super+C`).
    - Add an `bande_shell_v1` event that toggles a panel.
    - Complete the shortcut table with every bound shortcut (§4).
    - Proof: compositor tests, and the existing table-against-handler test.
19. **A tiling menu on the zoom control** (`toolkit/platform`, `platform/compositor`; S–M).
    - Resting 500 ms on the zoom control, or pressing and holding it, opens the Move & Resize
      submenu of the window menu at the control.
    - Do it in the toolkit's chrome and in the server-side frame (`decoration.rs`).
    - Proof: a toolkit test that the hold opens the menu, and a golden.
20. **Bluetooth: Forget, battery and pairing at any time** (`libs/bluetooth`,
    `shell/controlcenter`; S–M).
    - Add `RemoveDevice` and the `Battery1` percentage to the library.
    - Show both in Control Center's device list.
    - Register the pairing agent in the shell process, so a device can pair while Settings is
      closed.
    - Proof: tests on the BlueZ fake of `testing/test-bus`.
21. **Reopen apps at login** (`shell/shell`, `libs/xdg`; M).
    - When the session ends through the confirmation, save the ids of the running apps to
      `$XDG_STATE_HOME/bande/reopen`.
    - Launch them once at the next session start, leaving out the shell's own surfaces; window
      memory then puts the frames back.
    - Proof: a model test of the file, and a VM check.
22. **Night Light, first slice** (`platform/compositor`; M).
    - On DRM, apply a warm gamma ramp (CRTC `GAMMA_LUT`) from a `display { night-light }`
      setting, scheduled from sunset to sunrise with the portal's `daylight.rs` (which moves to
      `crates/libs` as it gains a second consumer).
    - The ramp fades in over a minute.
    - Proof: a unit test of the ramp and the schedule, and a `ci/vm` vkms readback.
23. **Battery and network on the lock screen** (`shell/lock`; S).
    - Show the battery level, charging state and Wi-Fi glyph in a corner, live from UPower and
      NetworkManager.
    - Proof: lock goldens in light and dark, with a fake UPower at 12 % (as in
      `libs/status/tests/devices.rs`).
24. **The toolkit follows Larger Text** (`toolkit/graphics`, `toolkit/widgets`; M).
    - Scale the type roles, and the control heights that derive from them, by the portal's
      `text-size` (100–200), live on `SettingChanged`.
    - Proof: control goldens at 100 % and 150 %, and a test that a 200 % field still fits its
      text.

Next after these: modifier-drag to move windows, Wi-Fi Forget and hidden networks, closing
windows from the overview, typing emoji directly without an input method, and Do Not Disturb
schedules.

## Sources

- Apple, Mac keyboard shortcuts (power key, Show Desktop, Fn-N/Fn-C, Character Viewer,
  accessibility focus keys): <https://support.apple.com/en-us/102650>
- Apple, Change Notifications settings on Mac (notifications off when mirroring or sharing):
  <https://support.apple.com/guide/mac-help/mchl205da693>
- Apple, Log out, sleep, wake, restart or shut down your Mac ("Reopen windows when logging
  back in"): <https://support.apple.com/guide/mac-help/mchlp2522/mac>
- Microsoft, Focus and Do Not Disturb (automatic rules when duplicating the display, gaming,
  full screen):
  <https://support.microsoft.com/windows/turn-off-notifications-in-windows-during-certain-times-81ed1b25-809b-741d-549c-7696474d15d3>
- Microsoft, Snap layouts on the maximise button:
  <https://learn.microsoft.com/windows/apps/desktop/modernize/apply-snap-layout-menu>
- GNOME Help, the accessibility menu: <https://help.gnome.org/gnome-help/a11y-icon.html>;
  automatic suspend and the power button:
  <https://help.gnome.org/users/gnome-help/stable/power-autosuspend.html>
- KDE, Qt apps survive the Wayland compositor crashing:
  <https://pointieststick.com/2023/03/10/this-week-in-kde-qt-apps-survive-the-wayland-compositor-crashing/>
- systemd, Desktop Environment Integration (app scopes and slices):
  <https://systemd.io/DESKTOP_ENVIRONMENTS/>
- XDG Desktop Portal API reference (frontend and backend interfaces):
  <https://flatpak.github.io/xdg-desktop-portal/docs/api-reference.html>
- GTK 3 settings on Wayland (portal only when sandboxed or forced):
  <https://gitlab.gnome.org/GNOME/gtk/-/blob/gtk-3-24/gdk/wayland/gdkscreen-wayland.c>; GTK 4:
  <https://gitlab.gnome.org/GNOME/gtk/-/blob/main/gdk/wayland/gdksettings-wayland.c>
- Mozilla bug 2069698 (`ui.key.accelKey` set to Super on Wayland):
  <https://bugzilla.mozilla.org/show_bug.cgi?id=2069698>
- Desktop Notifications Specification (`image-data`, `image-path`):
  <https://specifications.freedesktop.org/notification-spec/latest/>
