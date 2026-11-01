Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped.

The notifications bar open up under the task bar. Given that notifications open in the top right, should the notifications bar open there as well? or should we move the notifications to some kind of window on the left? on integrate them into the taskbar or notifications bar? How can we enhance this
Status: finished ([plans/notifications-placement.md](plans/notifications-placement.md)); M5 (several displays) left as optional

power management seems to not work. Screen saver, sleep/wake (wake with keyboard), display sleep, computer sleep
Status: started ([plans/power-management.md](plans/power-management.md)); display off at the lock stage, after a minute on a locked screen, and wake on input done (M1, M2, M4), sleep waits on the s2idle hardware test with a person at the mini (M3, M5-6)

crash management, bande-desktop is quite fragile and when it runs out of ram or crashes for some other reason it is quite common that it brings down the whole computer or just freezes. This never happens in MacOS so there must be better ways to handle such events. Investigate it, this is super important.
Status: started ([plans/crash-management.md](plans/crash-management.md)); kernel PSI/zram/lockup detectors, image oomd/zram/hardware watchdog, session bus at the OOM floor and the compositor watchdog done (M1-4); the owner installs the kernel and tests (M5), reopen after a crash (M6) and coredumps (M7) left
