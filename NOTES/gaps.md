Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped. A finished gap is deleted (git keeps it); what is left of a gap is listed under it. Owner checks and decisions that span gaps are in [open.md](open.md).

its not possible to install .deb files from Files, this needs an install tool like most distros provide
Status: started. Built (Package Installer in bande-linux/apps, opened from Files). Left: the owner installs a `.deb` from Files on the mini (it asks for the password on screen).

GTK apps do not fit with our window bar. Is that possible to solve so that all apps looks the same and adhere to bande window bar theme, for example also claude desktop and vscode also do not use the bande window bar
Status: started. GTK windows without a header bar get our bar; GTK 3/4 and libadwaita headers are styled like ours. Left: the live review on screen (owner), a framed Claude Desktop artifact window and Electron's controls overlay unchecked, and three owner decisions: VS Code `"window.titleBarStyle": "native"` on this machine; ask Anthropic for a native frame for Claude Desktop on Wayland; whether libadwaita's centred title is acceptable or those apps get Bande replacements.

its not possible to install more keyboard layouts or languages in settings *important*
Status: started. Layouts and languages work in Settings. Left: input methods (fcitx5) in Settings, deferred by decision; the owner picks a language and checks `locale -a`, and checks "Use at Login Screen" at the greeter (needs a logout).

the software update doesnt seem to work.
Status: started. Live updates with offline install only for reboot-class packages are built. Left: the owner runs one update on the mini with no reboot-class package (installs with no restart, only "Quit and reopen …"; no system service outside the excluded ones still maps a deleted `/usr` library afterwards, so `bande-desktop`'s `restart-services` hook ran), and one offline update end to end (Restart, `journalctl -b -u packagekit-offline-update`).

we have no swap (granted this is not an official installation of debian) and it may not be up to bande-desktop, but quickly review how that should work and if we should do anything or if that is soley up to the debian installer
Status: started. Kernel (PSI, zram) and image packages are merged; no disk swap by decision. Left: the owner installs the new kernel on the mini, `sudo apt install systemd-oomd`, upgrades `bande-desktop` and reboots.

review the CI, its performance and result has been abysmal. Right now it only starts on tags but still has not run successfully. I want tagged releases to build reliably, both arm64 and amd64. These builds are then picked up by the apt repo and published as installable packages correctly for debian users (make sure architecture works correctly)
Status: started. Everything up to the first tag is done and dry-run on the mini. Left: the owner cuts the first releases (order in [open.md](open.md)); the first tag run is the first real test.

we have no app store, should we build one?
Status: started. Software (bande-linux/apps) browses Discover, categories, Installed and Updates with icons, screenshots and app facts, and installs and removes Debian apps; updates hand off to Software Update. Flatpak and Flathub are dormant until the user installs `flatpak` (see the apps repo's `NOTES/plans/software-depth.md`). Left: automatic updates in Software Update, reviews.

power management seems to not work. Screen saver, sleep/wake (wake with keyboard), display sleep, computer sleep
Status: started. Built ([session](../docs/architecture/session.md#screen-lock-and-idle-bande-lock-bande-power)): screen saver with a resting lock screen, display off, lock with a grace period, every keep-awake path, streams and `org.bande.Power.KeepAwake` holding back idle sleep, `IdleHint`, wake on input, the power button. Left: the owner runs `sudo ~/bande-linux/bin/s2idle-test.sh keyboard` at the mini until ten clean cycles in a row, at least two in `desktop` mode; then `AllowSuspend=yes` in `crates/services/power/data/sleep.conf` turns sleep on, and the mini's masked targets and `suspend-after "never"` are undone.

crash management, bande-desktop is quite fragile and when it runs out of ram or crashes for some other reason it is quite common that it brings down the whole computer or just freezes. This never happens in MacOS so there must be better ways to handle such events. Investigate it, this is super important.
the mac mini m2 still freezes sometimes when wake from sleep or unlock, investigate why this is and also how we can avoid freezes. The desktop has to be reliable and fail in different stages. For example macos NEVER completely fail and force a reboot. But perhaps a force quit of the desktop is needed as worst case (and that still works with mouse and keyboard!) bande-desktop should be equally good
Status: started, branch gap-crash (the two notes above are merged into this gap). Kernel PSI/zram/lockup detectors, oomd and zram in the image, the session bus at the OOM floor, the compositor watchdog and opt-in reopen after a crash are built, and every failure mode now has a recovery ([session](../docs/architecture/session.md#crash-handling-and-supervision)): memory protection for the session, Relaunch for a hung shell, relaunch of a hung lock screen, a hang alarm for the login window, VT switching, renderer recovery after GPU loss, `TasksMax` for apps. Left: the owner installs `systemd-oomd` and the new kernel and tests under real memory pressure (checks in [open.md](open.md)); ramoops (panic logs kept across a reboot); why the mini still freezes on wake or unlock.

Package Installer is stuck on "Install", no matter what i do or how many times i press nothing happens.
Status: started. Fixed on main: the polkit agent uses polkit's socket-activated helper (Debian's helper is no longer setuid, so every password request failed with no dialog), and the installer reports refusals. Left: the owner runs a real install after the next desktop release and `installer-v0.1.1`; Software still treats `NotAuthorized` as a silent stop.

the login screen is extremly crude. Go over it and compare it to MAcos lock screen. Spend some time in getting this nicer for the user. Also the transition from the lock screen to when entering the desktop is just a black flash, that could be improved. Same with the book sequence, perhaps thats the bootloader but right now its just rows of "command line text" instead of some nice logo etc.
Status: started. Login and lock screens and the frame-keeping handover are on main. Left: the boot parts are on `gap-login-look` branches in kernel, images, bootloader and u-boot, and the test kernel; the login handover stays black until the `fbcon=vc:1-6` argument is set; nothing has run on the mini.

# GAPS from real world usage #
some apps should "quit" after use/close and not go to background like they do today, for example:
-> closing Settings should close the app definitevly like on MacOS.
-> finishing installing a .deb with "Installer" should quit it when pressing "done". There is also some issue with installer, after installation i cant hover the window but i can tab to it.

clicking a tar/zip in Files should use the Files built in unarchiever and not open Archieve Utility

when using a floating taskbar with a gap, there should probably by an equally large gap in the middle when using two apps side by side by snapping, ie: | gap | taskbar | gap | app1 (half screen) | gap | app2(other half) |. Its also possible that the user wants the same gap on the top, right and bottom as well for a full "floating" interface. I think those should be configurable in settings (kinda like the tiling manager Hyprland)

bande-browser title bar still has the wrong colors and the three dot button have black buttons even in dark theme (should be white). The browser also doesnt close to taskbar like it should, it quits completely on close. (same with vscode, zed and possibly more apps). Also the close button in the browser tabs are to the right, can they be moved to the left like rest of bande desktop?

right click menu in bande-browser shows EVERY option i think, its taller than the screen. Go over this properly.

The login/lock screen has a nice visual loading progress bar now still going from login screen to the desktop is mostly completely black screen. Can we make the loading bar progress show the whole time until the desktop is ready?

opening mail without accounts configured and then pressing Cancel cause a lag in the app and it only closes seconds later. That seems buggy and should be fixed. Using ctrl+a in the password entry also doesnt work in mail when setting up. Trying to setup gmail in mail fails with no option. I think this should never happen, i want to configure an imap email so why dont we give users the option to select account type? Basically i cant use the mail app so this needs alot of work. The calendar has the same issues.

ctrl+w wont close windows like "about this ..mail/player/browser"

Settings->About->Software Updates has a field "Check for updates", this field has too much text, never use sentences in the UI.

Activity Monitor has no GPU section

Settings -> Display "Size of text and controls" should just offer "zoom levels" and right now the default, outmost zoom is also wrong. Its showing 3840x2160 but im only on a 1440p monitor.

Settings -> Audio needs more work. 1. Its hard to tell the devices (input/output) apart. The balance should "stick" to the middle at 50% and also show where the middle is.

The password prompt when an app asks for sudo when installing something, for example the ArtCraft Launcher installing PhotoCraft had the super long request in the question instead of just a nice "ask for password".