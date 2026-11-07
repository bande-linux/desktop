Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped. A finished gap is deleted (git keeps it); what is left of a gap is listed under it. Owner checks and decisions that span gaps are in [open.md](open.md).

power management seems to not work. Screen saver, sleep/wake (wake with keyboard), display sleep, computer sleep
Status: started, branch gap-power. Built, as macOS's Lock Screen settings ([session](../docs/architecture/session.md#screen-lock-and-idle-bande-lock-bande-power)): screen saver, display off and lock with a grace period, every keep-awake path (ScreenSaver D-Bus, the portal, logind `idle` inhibitors, Wayland `idle-inhibit`), wake on input, the power button (lock, hold five seconds to power off) and recovery of a display that dropped its link in standby. The 2026-11-02 night without a lock came from idle events never being sent while nothing drew and the compositor never hearing logind's `Lock` (both fixed). Left: a resting look for the screen saver (no password field until the first input); sleep, which stays off until s2idle survives ten clean cycles on the mini with someone at the machine, then the power button and idle go back to `suspend`, audio output streams keep the system (not the display) awake, and the portal's suspend flag (dropped by the GTK backend today) becomes a logind `sleep` block inhibitor through a Bande Inhibit backend; logind `IdleHint` is never set.

crash management, bande-desktop is quite fragile and when it runs out of ram or crashes for some other reason it is quite common that it brings down the whole computer or just freezes. This never happens in MacOS so there must be better ways to handle such events. Investigate it, this is super important.
the mac mini m2 still freezes sometimes when wake from sleep or unlock, investigate why this is and also how we can avoid freezes. The desktop has to be reliable and fail in different stages. For example macos NEVER completely fail and force a reboot. But perhaps a force quit of the desktop is needed as worst case (and that still works with mouse and keyboard!) bande-desktop should be equally good
Status: started, branch gap-crash (the two notes above are merged into this gap). Kernel PSI/zram/lockup detectors, oomd and zram in the image, the session bus at the OOM floor, the compositor watchdog and opt-in reopen after a crash are built, and every failure mode now has a recovery ([session](../docs/architecture/session.md#crash-handling-and-supervision)): memory protection for the session, Relaunch for a hung shell, relaunch of a hung lock screen, a hang alarm for the login window, VT switching, renderer recovery after GPU loss, `TasksMax` for apps. Left: the owner installs `systemd-oomd` and the new kernel and tests under real memory pressure (checks in [open.md](open.md)); ramoops (panic logs kept across a reboot); why the mini still freezes on wake or unlock.

Comebine DDC/CI brightness adjustment with normal laptop screen adjustment. Multiple displays connect gets an individual brightness bar.
Status: started, branch gap-brightness.

saving region format to English (Sweden) or Sweden (Sweden) doesnt work, i get "couldn't make this change"
Status: started, branch gap-region.

screen lock under "Privacy & Security" seems to conflict with the options in Power. Either way these are related and should only be in one location not scattered out like this.
Status: started, branch gap-lock-settings.

not possible to adjust window size that has been "snapped" to either half the screen or a quarter. The snapping should just adjust the window into that shape and nothing else. Adjusting it from there on should be possible just like in macos.
Status: started, branch gap-snap-resize.

sometimes moving windows around is hard, like the terminal with two tabs. Dont change how the windows work but the problem is that tha "arrow" to drag out and make the window larger present itself not only on the "edge" and outward but also a few pixels inward. So the little space between the actual tabs and the window edge cant be grabbed, since it will go to the window size changer. Make the size changed only show itself on the edge of the window and a few pixel outward (like today) but not inward. Also windows have rounded corners which is not taken into account making it much harder to "grab" corners to expand both x and y axis at the same time.
Status: started, branch gap-resize-edges.

bring back taskbar application "focus" but change it this way. For applications that are open/closed but still in focus. Since we removed the "focus icon" it is not possible to see which app is in focus now if i X a window. For example, i X Text Editor, it is still in focus but it is impossible to see. So Ctrl + Q will close it. Keep the small open indicator for open apps but change the focused app to be highlighted with a nice modern "border". Make sure it is nice looking and doesnt cause any other issues. This "focus frame" will swap to which ever app is in focus. Other "open" apps will have the normal dot indicator, and the focused app will have the highlighted frame.
Status: started, branch gap-taskbar-focus.

In Files i want to be able to drag change coloumn order in the detailed list view. The dragging should be nice and smooth. (can we reuse such draging framework with other bande-native apps in the toolkit? are there more things we should (NOT COULD, SHOULD) share in the toolkit?)
Status: started, branch gap-files-columns.

In Files, there is no need to have the viewing options in the right click menu, since they are present with nice buttons. This type of right click bloat and general settings/menu bloat should be avoided. Applications should also not be allowed to alter the right click menu like in Windows/MacOS. (for example google drive will add an option to the right click menu everywhere in both those OSes without any consent. We should not allow that in Bande Desktop)
Status: started, branch gap-context-menus.

Package Installer is stuck on "Install", no matter what i do or how many times i press nothing happens.
Status: started, branch gap-package-installer.

The shutdown button doesnt work. (right click Bande icon -> shutdown). Also the restart button.
Status: started, branch gap-shutdown.

when showing hidden files i think the hidden files should be slightly "greyed out" or toned down so it is obvious which shown file/folder is normally hidden and which is not
Status: started, branch gap-files-hidden.

##CHECK THIS## cant write ~ in terminal?
Status: started, branch gap-terminal-tilde.

super+space opening search popup
Status: started, branch gap-search-shortcut.

bande-browser icon is like a safari clone but we own our own design theme and should stick to that now clone apple.
Status: started, branch gap-browser-icon.

there is no point in having: Current releases on https://bande-linux.github.io if since they are not updated. I stripped down moreof the website because it contained alot of unneeded information. Remove this also.
Status: started.

Passwords (keyring) cant be unlocked from Settings->Passwords. When failing to unlock the text "Couldnt make this change" stick to the settings window in all tabs no matter what view i go to.
Status: started, branch gap-passwords.

hover over the buttons (close, minimize, maxmimize) in any bande-app only works for close, the maximize gets a weird submeny popup like right click? Review this and make sure that all bande apps are solved and using the same module in the toolkit for this. At the same time also go over the topbar of windows for the toolkit as well and improve it. For example on macos hovering buttons will get a backfill, generally they look nicer, you can grab the window by any "free space" (where there are no buttons or text)
Status: started, branch gap-titlebar.

the login screen is extremly crude. Go over it and compare it to MAcos lock screen. Spend some time in getting this nicer for the user. Also the transition from the lock screen to when entering the desktop is just a black flash, that could be improved. Same with the book sequence, perhaps thats the bootloader but right now its just rows of "command line text" instead of some nice logo etc.
Status: started, branch gap-login-look.
