Hand written notes by a user when gaps are found. These need to be properly investigated seperately.

Status values: open, planning, planned, started, finished, dropped. A finished gap is deleted (git keeps it); what is left of a gap is listed under it. Owner checks and decisions that span gaps are in [open.md](open.md).

its not possible to install .deb files from Files, this needs an install tool like most distros provide
Status: started. Built (Package Installer in bande-linux/apps, opened from Files). Left: the owner installs a `.deb` from Files on the mini (it asks for the password on screen).

GTK apps do not fit with our window bar. Is that possible to solve so that all apps looks the same and adhere to bande window bar theme, for example also claude desktop and vscode also do not use the bande window bar
Status: started. GTK windows without a header bar get our bar; GTK 3/4 and libadwaita headers are styled like ours. Left: the live review on screen (owner), a framed Claude Desktop artifact window and Electron's controls overlay unchecked, and three owner decisions: VS Code `"window.titleBarStyle": "native"` on this machine; ask Anthropic for a native frame for Claude Desktop on Wayland; whether libadwaita's centred title is acceptable or those apps get Bande replacements.

its not possible to install more keyboard layouts or languages in settings *important*
Status: started. Layouts and languages work in Settings. Left: input methods (fcitx5) in Settings, deferred by decision; the owner picks a language and checks `locale -a`, and checks "Use at Login Screen" at the greeter (needs a logout).

the software update doesnt seem to work.
Status: started. Live updates with offline install only for reboot-class packages are built. Left: the owner runs one update on the mini with no reboot-class package (installs with no restart, only "Quit and reopen …"; `sudo needrestart -b` lists no stale services; `journalctl -u packagekit` shows needrestart's hook ran without prompting), and one offline update end to end (Restart, `journalctl -b -u packagekit-offline-update`).

we have no swap (granted this is not an official installation of debian) and it may not be up to bande-desktop, but quickly review how that should work and if we should do anything or if that is soley up to the debian installer
Status: started. Kernel (PSI, zram) and image packages are merged; no disk swap by decision. Left: the owner installs the new kernel on the mini, `sudo apt install systemd-oomd`, upgrades `bande-desktop` and reboots.

review the CI, its performance and result has been abysmal. Right now it only starts on tags but still has not run successfully. I want tagged releases to build reliably, both arm64 and amd64. These builds are then picked up by the apt repo and published as installable packages correctly for debian users (make sure architecture works correctly)
Status: started. Everything up to the first tag is done and dry-run on the mini. Left: the owner cuts the first releases (order in [open.md](open.md)); the first tag run is the first real test.

we have no app store, should we build one?
Status: started. Software (bande-linux/apps) browses Discover, categories, Installed and Updates with icons, screenshots and app facts, and installs and removes Debian apps; updates hand off to Software Update. Left: Flatpak and Flathub (planned in the apps repo's `NOTES/plans/software-depth.md`), automatic updates in Software Update, reviews.
