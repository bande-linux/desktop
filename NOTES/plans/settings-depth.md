# Settings depth

What a user needs from System Settings to rely on it instead of a terminal, pane by pane,
measured against macOS System Settings. Each pane says what Settings has, then what is missing.
The ranked list at the end is the work order; delete a line when it ships and the file when
the list is empty.

## Pane by pane

**Wi-Fi.** Has: power, Known Networks (saved, in range or not, with status) and Other Networks
with signal and lock, join with password (wrong password asked again), Other… for hidden
networks, Sign In for a captive portal, and a Details page per known network: Auto-Join, Low
Data Mode (metered), Private Wi-Fi Address (off / fixed / rotating), IP address, router, DNS,
hardware address, IPv4 automatic or manual, DNS servers while automatic, Forget. Missing:
Personal Hotspot (NetworkManager mode `ap`, method `shared`), Ask to Join, enterprise (802.1X)
and WEP networks, copying a saved password.

**Bluetooth.** Has: power, my devices with connect/disconnect/forget and battery level, nearby
devices, pairing with codes, PINs and authorization. Missing: rename a device, the computer's
Bluetooth name.

**Network.** Has: Ethernet status with IPv4 automatic/manual, router and DNS; VPN connect,
disconnect, import of WireGuard and OpenVPN files, DNS servers while the address is automatic.
Missing: VPN details and removal, adding a VPN by hand,
proxies (NetworkManager's per-connection proxy only reaches apps through the portal's
ProxyResolver, so this needs a session-wide proxy first), IPv6, service order, firewall.

**Notifications.** Has: Do Not Disturb, per-app allow, banner style, show in center, play sound.
Missing: per-app badges, "show previews" (always / when unlocked / never), notifications
while the display is mirrored or sleeping.

**Focus.** Has: Do Not Disturb on/off and a daily schedule (from–to, owned by `bande-notify`).
Missing: days of the week, "for an hour / until tomorrow", allowed apps.

**Screen Time.** Out of scope.

**Sound.** Has: output and input device choice, volumes and mute, input level meter, per-app
volume, alert sound and volume, sound effects, volume feedback. Missing: balance (needs
per-channel volume in `bande-audio`), friendly names for the Mac mini's built-in speaker and
the headphone jack (check what WirePlumber reports now that both work), "play sound on
startup", show a device as unplugged for the jack.

**General.** About: computer name, chip, memory, version, serial; storage page with categories,
large files and Empty Trash; Software Update with offline updates. Date & Time: automatic
time, manual date and time, time zone, 24-hour clock. Language & Region: languages, region,
login keyboard layouts. Login Items: apps that open at login. Missing: Sharing (remote login
over SSH, file sharing over Samba, the local hostname shown with `.local`), Startup Disk (not on
Apple silicon under Linux: U-Boot owns it), a "Transfer or Reset" equivalent (out of scope), a
storage breakdown bar in About. Apps ▸ Reopen switches reopening at login and after a crash.

**Appearance.** Has: light/dark/automatic, accent colours, text size, tinted frame. Missing:
highlight colour (follows accent, fine), sidebar icon size, show scroll bars (automatic /
when scrolling / always) and "click in the scroll bar to" (needs toolkit support).

**Accessibility.** Has: screen reader, zoom, increase contrast, reduce transparency, reduce
motion, cursor size, flash screen, sticky/slow/bounce keys. Missing: Orca is not installed
in the image (the switch has nothing to start; owner decision), zoom style and keyboard
shortcut to zoom, pointer colour, shake to locate, hover text, captions.

**Control Center.** Missing: which modules show in the bar and in Control Center (Wi-Fi,
Bluetooth, Sound, Battery percentage exists under Power).

**Desktop & Dock.** The Taskbar pane is being reworked by another agent. Missing here: hot
corners (needs the compositor), Mission Control options (group windows by app, switch to a
space with open windows), "click wallpaper to reveal desktop", default web browser lives in
Apps already.

**Displays.** Has: brightness, resolution, scaling, refresh rate, rotation, variable refresh,
arrangement with a revert countdown, mirroring, Night Light (Night Shift) with schedule and
warmth. Missing: colour profile, HDR (blocked on the kernel), "optimise for" on mirroring.

**Wallpaper.** Has: bundled light/dark pairs, choose a picture, fill or fit. Missing: a
solid colour, a folder that rotates, "show on all spaces" is moot.

**Screen Saver / Lock Screen.** Has: dim after, lock after, sleep after, lock when the
display turns off. Missing: a screen saver (out of scope; blanking only), a message on the
lock screen, "show user name and photo", "require password immediately after sleep" as its
own choice.

**Battery / Energy.** Has: level and history, power mode, low power on battery, lid on power,
sleep timers, battery percentage in the bar. Missing: wake for network access (Wake-on-LAN
through NetworkManager `802-3-ethernet.wake-on-lan`), "start up automatically after a power
failure" (needs the SMC driver), scheduled sleep and wake.

**Users & Groups.** Has: the accounts on this computer (administrator or standard) and Change
Password for your own, which runs `passwd` on a pseudo-terminal as GNOME's fallback does (the
current password, the new one twice, and passwd's own reason when it refuses one). Missing:
picture and full name, add and remove users, the administrator flag, automatic login.
AccountsService (`org.freedesktop.Accounts`: SetRealName, SetIconFile, CreateUser,
SetAccountType) is the right service for those but is not in the image.

**Passwords.** Out of scope.

**Keyboard.** Has: input sources with search, login layouts, repeat rate and delay, Caps Lock
behaviour, shortcuts page. Missing: editable shortcuts, keyboard brightness (no backlight on the
mini), input methods (fcitx5, deferred by decision), dictation (out of scope).

**Mouse / Trackpad.** Has: speed, acceleration, natural scrolling, primary button; trackpad tap,
dragging, secondary click, scroll method, gestures. Missing: double-click speed, scroll speed.

**Printers & Scanners.** Has (`bande-printers`: IPP on CUPS's socket with PeerCred, Avahi for
nearby printers, `com.redhat.PrinterSpooler` signals for changes): printers with state and
queued documents, the default printer, nearby IPP printers to add (driverless, `lpadmin -m
everywhere`), add by address, and a page per printer with kind, location, address, its queue
with Cancel, Pause/Resume, Make Default and Remove. Missing: scanners (SANE / eSCL), printer
options (paper size, duplex, colour defaults), supply levels, sharing a printer. CUPS is not in
the image yet, so the pane says printing isn't set up.

## Ranked gaps

Ranked by how often a user needs them and how badly they are stuck without them.

1. Printers: put `cups` (and `cups-client` for lpadmin) in the image and the user in `lpadmin`
   (owner decision); then printer options, supply levels and scanners.
2. Users & Groups: picture, full name, add and remove users. Blocked: owner decision to add
   `accountsservice` to the image.
4. Sound: balance and the built-in speaker and headphone names.
7. Hot corners (compositor).
8. Sharing: Remote Login (ssh.service through systemd's D-Bus API) and the `.local` name.
9. Notifications: previews and badges per app.
10. Lock screen message, show scroll bars, double-click speed, Wake for network access.
11. Personal hotspot.
