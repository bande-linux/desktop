Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped. A finished gap is deleted (git keeps it); what is left of a gap is listed under it. Owner checks and decisions that span gaps are in [open.md](open.md).

power management seems to not work. Screen saver, sleep/wake (wake with keyboard), display sleep, computer sleep
Status: started. Built ([session](../docs/architecture/session.md#screen-lock-and-idle-bande-lock-bande-power)): screen saver with a resting lock screen, display off, lock with a grace period, every keep-awake path, streams and `org.bande.Power.KeepAwake` holding back idle sleep, `IdleHint`, wake on input, the power button. Left: the owner runs `sudo ~/bande-linux/bin/s2idle-test.sh keyboard` at the mini until ten clean cycles in a row, at least two in `desktop` mode; then `AllowSuspend=yes` in `crates/services/power/data/sleep.conf` turns sleep on, and the mini's masked targets and `suspend-after "never"` are undone.

crash management, bande-desktop is quite fragile and when it runs out of ram or crashes for some other reason it is quite common that it brings down the whole computer or just freezes. This never happens in MacOS so there must be better ways to handle such events. Investigate it, this is super important.
the mac mini m2 still freezes sometimes when wake from sleep or unlock, investigate why this is and also how we can avoid freezes. The desktop has to be reliable and fail in different stages. For example macos NEVER completely fail and force a reboot. But perhaps a force quit of the desktop is needed as worst case (and that still works with mouse and keyboard!) bande-desktop should be equally good
Status: started, branch gap-crash (the two notes above are merged into this gap). Kernel PSI/zram/lockup detectors, oomd and zram in the image, the session bus at the OOM floor, the compositor watchdog and opt-in reopen after a crash are built, and every failure mode now has a recovery ([session](../docs/architecture/session.md#crash-handling-and-supervision)): memory protection for the session, Relaunch for a hung shell, relaunch of a hung lock screen, a hang alarm for the login window, VT switching, renderer recovery after GPU loss, `TasksMax` for apps. Left: the owner installs `systemd-oomd` and the new kernel and tests under real memory pressure (checks in [open.md](open.md)); ramoops (panic logs kept across a reboot); why the mini still freezes on wake or unlock.

Comebine DDC/CI brightness adjustment with normal laptop screen adjustment. Multiple displays connect gets an individual brightness bar.
Status: started, branch gap-brightness.

saving region format to English (Sweden) or Sweden (Sweden) doesnt work, i get "couldn't make this change"
Status: started, branch gap-region.

sometimes moving windows around is hard, like the terminal with two tabs. Dont change how the windows work but the problem is that tha "arrow" to drag out and make the window larger present itself not only on the "edge" and outward but also a few pixels inward. So the little space between the actual tabs and the window edge cant be grabbed, since it will go to the window size changer. Make the size changed only show itself on the edge of the window and a few pixel outward (like today) but not inward. Also windows have rounded corners which is not taken into account making it much harder to "grab" corners to expand both x and y axis at the same time.
Status: started, branch gap-resize-edges.

Package Installer is stuck on "Install", no matter what i do or how many times i press nothing happens.
Status: started. Fixed on main: the polkit agent uses polkit's socket-activated helper (Debian's helper is no longer setuid, so every password request failed with no dialog), and the installer reports refusals. Left: the owner runs a real install after the next desktop release and `installer-v0.1.1`; Software still treats `NotAuthorized` as a silent stop.

The shutdown button doesnt work. (right click Bande icon -> shutdown). Also the restart button.
Status: started, branch gap-shutdown.

##CHECK THIS## cant write ~ in terminal?
Status: started, branch gap-terminal-tilde.

hover over the buttons (close, minimize, maxmimize) in any bande-app only works for close, the maximize gets a weird submeny popup like right click? Review this and make sure that all bande apps are solved and using the same module in the toolkit for this. At the same time also go over the topbar of windows for the toolkit as well and improve it. For example on macos hovering buttons will get a backfill, generally they look nicer, you can grab the window by any "free space" (where there are no buttons or text)
Status: started, branch gap-titlebar.

the login screen is extremly crude. Go over it and compare it to MAcos lock screen. Spend some time in getting this nicer for the user. Also the transition from the lock screen to when entering the desktop is just a black flash, that could be improved. Same with the book sequence, perhaps thats the bootloader but right now its just rows of "command line text" instead of some nice logo etc.
Status: started, branch gap-login-look.
