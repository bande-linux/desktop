# Printing

## What the user experiences
File ▸ Print… (`Ctrl+P`) in every app with a document: Files, Preview, TextEdit, Mail, Calendar, Terminal, Browser. One dialog, attached to the window: live preview, printer, pages, copies, paper, orientation, two-sided, colour, Save as PDF. Printers on the network are simply in the list, nothing to add, nothing to install, no password. Closing the app does not lose a job; an offline printer's job waits and goes out when it returns; a failure is a notification.

## Printing without CUPS: why
The image has neither CUPS nor Avahi, on purpose. Alternatives weighed:

- **CUPS and Avahi in the image.** Most compatible, but the owner removed them for leanness, and a daemon set for a feature many sessions never use.
- **CUPS installed when the first printer is added.** Needs the network, an admin password and a wait before the first print; Debian's `cups` brings `cups-filters`, Poppler and more; discovery still has to happen before the install, so printers cannot just appear. Fine for what only CUPS can drive, wrong as the main path.
- **Direct IPP Everywhere (chosen).** AirPrint and IPP Everywhere printers (nearly everything sold since about 2013) take IPP over the network with a PDF, PWG raster or URF document and need no driver. A small per-user service speaks it. Nothing to install, no root, no daemon when nothing is printing. CUPS 3's own local server goes the same way, per user and driverless.
- **CUPS remains the fallback, on demand.** A legacy printer with only a vendor driver, by address or USB, still goes through the existing Settings ▸ Printers install of `cups`, and CUPS queues show up in the dialog beside direct printers. USB printers that speak IPP-over-USB get Debian's `ipp-usb` on demand with the same install.

Discovery is our own small mDNS browser (`_ipp._tcp`, `_ipps._tcp`), as `systemd-resolved` has mDNS off on these links and Avahi is gone. It runs only while the dialog or the Settings pane is open, so idle stays at zero wakeups; the last-seen printers are kept so the list is not empty at first.

## Design
- **`bande-pdf`** (`crates/toolkit/pdf`): turns toolkit `Item`s (the same scene language apps draw with) into PDF through `krilla` (Typst's PDF writer, light without its optional features): real vector text, subset fonts, selectable and searchable.
- **`bande-printd`** (`crates/services/printd`): session-bus-activated spooler and discovery, `org.bande.Print`; holds all network code (mDNS, IPP over TCP and TLS, PDF to URF or PWG raster), so apps stay free of it. Keeps the queue in `$XDG_STATE_HOME/bande/print-queue`, sends each job, follows its state, notifies on failure and exits when nothing is left. A user unit restarts it at login only if jobs wait.
- **`bande-printers`**: the client for that service, the IPP codec and the CUPS client.
- **The dialog**: `bande-file-chooser --print`, one program for every caller (the crate that already holds the portal's dialogs). It shows its preview by rasterising the actual PDF in the confined `--pdf-worker` child, so the preview is what prints.
- **`bande-app`**: `Cx::print`, the standard File ▸ Print… item and `Ctrl+P`. The app hands a `Fn(&PageSetup) -> Vec<Page>`; the dialog asks for a new layout over a socket when paper or orientation changes, so text reflows as on macOS.
- **Portal**: `org.freedesktop.impl.portal.Print` in `xdg-desktop-portal-bande`. `Print` without a token shows the dialog with the app's PDF and preview; `PreparePrint` shows it without one and the later `Print` goes straight out.
- **Parenting**: `zxdg_exporter_v2`/`importer` in the compositor; `bande-app` exports each window and passes the key window to the portal.

## Stages
1. Dialog parenting (xdg-foreign), this plan.
2. `bande-pdf`.
3. `bande-printd`.
4. The dialog; portal `Print`; `bande-app` Print; Settings pane on the service.
5. Each app's Print command, after a desktop bump.
6. Settings offers the install when a USB printer is plugged in.
