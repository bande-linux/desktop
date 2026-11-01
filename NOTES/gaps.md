Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped.

swap Super->Ctrl like normal linux distros
Status: started ([plans/super-ctrl.md](plans/super-ctrl.md)); Ctrl for app shortcuts done (M1-M9), M10 Linux habits on Super open

keep in dock doesnt work for everything, for example launching -code- from terminal should still be able to save to dock. Also dock should be renamed to taskbar
Status: started ([plans/keep-in-taskbar.md](plans/keep-in-taskbar.md)); keeping apps without an entry and the Taskbar name done, internal code rename (M6) left

its not possible to scroll in the settings app, for example bluetooth and wifi views. Also bluetooth devices have no names, so i have no idea which device is my mouse etc.
Status: finished ([plans/settings-scroll-and-bluetooth-names.md](plans/settings-scroll-and-bluetooth-names.md))

its not possible to install .deb files from Files, this needs an install tool like most distros provide
Status: planned ([plans/deb-installer.md](plans/deb-installer.md))

GTK apps do not fit with our window bar. Is that possible to solve so that all apps looks the same and adhere to bande window bar theme, for example also claude desktop and vscode also do not use the bande window bar
Status: planned ([plans/third-party-title-bars.md](plans/third-party-title-bars.md))

it seems like some apps are not getting the correct taskbar icon? this could also be that they simply dont have one. So dont dwell too deeply in that.
Status: started ([plans/taskbar-icons.md](plans/taskbar-icons.md)); M1 and M3 done, M2 waits on an owner decision

its not possible to install more keyboard layouts or languages in settings *important*
Status: started ([plans/keyboard-layouts-and-languages.md](plans/keyboard-layouts-and-languages.md)); M4 layout picker done

grapping a window edge or corner in order to reseize it is a bit difficult, the resizer is too unprecise to the actual corner or edge
Status: finished ([plans/window-resize-zones.md](plans/window-resize-zones.md)); not yet checked live on the mini

the themes, especially the dark theme, looks a bit "dated". Review how modern dark themes look in 2026. This is too dark and hard on the eyes with strong contrast. I think MacOS dark theme is nicer on the eyes, a bit more slated. DO not copy that however, rely on the investigation.
Status: finished ([plans/dark-theme.md](plans/dark-theme.md)); moving light to the slate family (M6) waits on the owner

the software update doesnt seem to work.
Status: started ([plans/software-update.md](plans/software-update.md)); M3 plain states done

changing time zone doesnt apply
Status: finished ([plans/time-zone.md](plans/time-zone.md)); M5 polkit fix waits on an owner decision

scrolling direction should have individual settings for trackpad and mouse. Also trackpad and mouse probably should have their own setting views with more option. The trackpad has 0 options for example.
Status: started ([plans/mouse-and-trackpad.md](plans/mouse-and-trackpad.md)); M1-M5 done, gestures (M6-M7) left

in settings the selected item in the sidebar has blue markers, that looks pretty bad. Its enough to highlight the selection i think
Status: planned ([plans/sidebar-selection.md](plans/sidebar-selection.md))

Files has no "settings" (and other inhouse apps who needs it), for example i want to show file extensions
Status: started ([plans/app-settings.md](plans/app-settings.md))

we have no swap (granted this is not an official installation of debian) and it may not be up to bande-desktop, but quickly review how that should work and if we should do anything or if that is soley up to the debian installer
Status: started ([plans/swap.md](plans/swap.md)); kernel config and image packages merged, the 0004 caveat written; the owner installs the kernel, enables zram and oomd and reboots

review the CI, its performance and result has been abysmal. Right now it only starts on tags but still has not run successfully. I want tagged releases to build reliably, both arm64 and amd64. These builds are then picked up by the apt repo and published as installable packages correctly for debian users (make sure architecture works correctly)
Status: finished up to the first tag, which the owner cuts ([plans/ci-releases.md](plans/ci-releases.md), "Cutting the first release")

we have no app store, should we build one?
Status: planned ([plans/app-store.md](plans/app-store.md))

snapping windows to the sides is a bit slow and also to the left requires you do drag to the screen edge but dragging to the taskbar should be enough since thats the real edge of the window. So the screen estate is the whole screen minus the taskbar, not the whole screen like now.
Status: started ([plans/window-snapping.md](plans/window-snapping.md))
