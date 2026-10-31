Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

swap Super->Ctrl like normal linux distros

keep in dock doesnt work for everything, for example launching -code- from terminal should still be able to save to dock. Also dock should be renamed to taskbar

its not possible to scroll in the settings app, for example bluetooth and wifi views. Also bluetooth devices have no names, so i have no idea which device is my mouse etc.

its not possible to install .deb files from Files, this needs an install tool like most distros provide

GTK apps do not fit with our window bar. Is that possible to solve so that all apps looks the same and adhere to unbound window bar theme, for example also claude desktop and vscode also do not use the unbound window bar

it seems like some apps are not getting the correct taskbar icon? this could also be that they simply dont have one. So dont dwell too deeply in that.

its not possible to install more keyboard layouts or languages in settings *important*

grapping a window edge or corner in order to reseize it is a bit difficult, the resizer is too unprecise to the actual corner or edge

the themes, especially the dark theme, looks a bit "dated". Review how modern dark themes look in 2026. This is too dark and hard on the eyes with strong contrast. I think MacOS dark theme is nicer on the eyes, a bit more slated. DO not copy that however, rely on the investigation.

the software update doesnt seem to work.

changing time zone doesnt apply

scrolling direction should have individual settings for trackpad and mouse. Also trackpad and mouse probably should have their own setting views with more option. The trackpad has 0 options for example.

in settings the selected item in the sidebar has blue markers, that looks pretty bad. Its enough to highlight the selection i think

Files has no "settings" (and other inhouse apps who needs it), for example i want to show file extensions

we have no swap (granted this is not an official installation of debian) and it may not be up to unbound-desktop, but quickly review how that should work and if we should do anything or if that is soley up to the debian installer

review the CI, its performance and result has been abysmal. Right now it only starts on tags but still has not run successfully. I want tagged releases to build reliably, both arm64 and amd64. These builds are then picked up by the apt repo and published as installable packages correctly for debian users (make sure architecture works correctly)

we have no app store, should we build one?

snapping windows to the sides is a bit slow and also to the left requires you do drag to the screen edge but dragging to the taskbar should be enough since thats the real edge of the window. So the screen estate is the whole screen minus the taskbar, not the whole screen like now.
