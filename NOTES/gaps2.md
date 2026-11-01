Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped.

The notifications bar open up under the task bar. Given that notifications open in the top right, should the notifications bar open there as well? or should we move the notifications to some kind of window on the left? on integrate them into the taskbar or notifications bar? How can we enhance this
Status: finished ([plans/notifications-placement.md](plans/notifications-placement.md)); M5 (several displays) left as optional

power management seems to not work. Screen saver, sleep/wake (wake with keyboard), display sleep, computer sleep
Status: started ([plans/power-management.md](plans/power-management.md)); display off at the lock stage, after a minute on a locked screen, and wake on input done (M1, M2, M4), the power button locks with a five-second power-off (M7), a display whose link broke in standby is set again; sleep waits on the s2idle hardware test with a person at the mini (M3, M5-6)

crash management, bande-desktop is quite fragile and when it runs out of ram or crashes for some other reason it is quite common that it brings down the whole computer or just freezes. This never happens in MacOS so there must be better ways to handle such events. Investigate it, this is super important.
Status: started ([plans/crash-management.md](plans/crash-management.md)); kernel PSI/zram/lockup detectors, image oomd/zram/hardware watchdog, session bus at the OOM floor and the compositor watchdog done (M1-4); reopen after a crash done as an opt-in (M6); the owner installs the kernel and tests (M5), ramoops (M7) left


remove the ability to drag out the taskbar into the wide state. That looks bad and is another feature to support that wont be used. I want sensible options for the bande desktop but not bloated options (KDE for example has way too many options and if feels bad. Gnome has way too few options and it feels limiting. We want a sweetspot like MacOS where the user can customize what is reasonable that an average user would want but not more. Too many options will likely bring bugs as well.)
Status: finished ([plans/taskbar-width-and-options.md](plans/taskbar-width-and-options.md)); the wide taskbar and Show names are removed (old configs still load); the options inventory lists six further removals and merges that wait on the owner

UI polish: the ui needs to be polished and improved in some ways. Like the restart window. Is way to large and looks hacky. Go over the OS and make sure each view/item/panel is professional, clean and modern looking.
Status: started ([plans/ui-polish.md](plans/ui-polish.md)); M1-M4 finished (the restart/shut down/log out confirmation, pairing, Low Memory and About as compact fixed dialogs; polkit fitted; Package Installer and Terminal confirmations), M5 consistency partly done, M6 waits for the other agents' surfaces

