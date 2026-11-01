# Gap plans (2026-10-31)

One plan per item in `NOTES/gaps.md` and `NOTES/gaps2.md`. Each plan has findings, a recommendation, milestones of about 1-2 hours and the decisions it needs.

## Order

Plans share work. Land the shared pieces first:

1. **Packages on the machine.** `packagekit`, `locales`, `fonts-noto-core`, `systemd-zram-generator`, `systemd-oomd`, `bluez`, `libspa-0.2-bluetooth` are missing because the image and the first install use `--no-install-recommends`. This alone fixes what the owner sees in software update and languages. Images repo plus the mini. Done 2026-11-01: all seven in `images` `prep/os-installable`; on the mini `packagekit`, `locales` and `fonts-noto-core` are installed (`systemd-zram-generator` already was), `systemd-oomd` waits for the PSI kernel, `bluez`/`libspa-0.2-bluetooth` are left to the Bluetooth work.
2. **Kernel:** `CONFIG_PSI=y`, `CONFIG_ZRAM=m` ([swap](swap.md)). Done 2026-11-01 in `linux` `bande-hw/integration`; not installed on the mini yet.
3. **App entry reload in the bar** ([taskbar-icons](taskbar-icons.md) M1). Step 0 of [keep-in-taskbar](keep-in-taskbar.md).
4. **`crates/libs/packages`**, the shared PackageKit client, created by whichever of [software-update](software-update.md), [deb-installer](deb-installer.md) and [app-store](app-store.md) lands first.
5. **Dark theme tokens** ([dark-theme](dark-theme.md)) before [sidebar-selection](sidebar-selection.md) takes its dark values. [third-party-title-bars](third-party-title-bars.md) M5 replaces dark-theme M4's GTK 3 part.
6. **[super-ctrl](super-ctrl.md)** before [app-settings](app-settings.md) fixes its Settings… shortcut.
7. **[app-settings](app-settings.md)** M5-M9 after `wip/files-depth-3`; [mouse-and-trackpad](mouse-and-trackpad.md) builds on the merged platform-quality logging.

## Plans

| Plan | Root cause in one line | Milestones |
| --- | --- | --- |
| [super-ctrl](super-ctrl.md) | App shortcuts use Super; Ctrl does nothing in Bande text fields | 9 (+1 optional) |
| [keep-in-taskbar](keep-in-taskbar.md) | Bar reads app entries once at shell start; apps installed later can't be kept | 7 |
| [taskbar-icons](taskbar-icons.md) | Same load-once bug; own dialogs have no entry | 3 |
| [settings-scroll-and-bluetooth-names](settings-scroll-and-bluetooth-names.md) | Settings' own scroll widget ignores wheel and touchpad; rows show BlueZ Alias (the address) | 4 |
| [deb-installer](deb-installer.md) | Nothing handles `.deb`; PackageKit not installed | 7 |
| [third-party-title-bars](third-party-title-bars.md) | GTK only speaks KDE server-decoration, which the compositor lacks | 6 |
| [keyboard-layouts-and-languages](keyboard-layouts-and-languages.md) | `locales` not installed; layout picker shows 8 results | 5 |
| [window-resize-zones](window-resize-zones.md) | 6 pt band fully outside the edge, move strip inside, corner misses the curve | 3 |
| [dark-theme](dark-theme.md) | Near-black warm neutrals, 15:1 text, darker menus, pastel selection | 7 |
| [software-update](software-update.md) | PackageKit not installed; apt repo never published | 7 |
| [time-zone](time-zone.md) | jiff caches the system zone for 5 minutes; bar redraws once a minute | 5 |
| [mouse-and-trackpad](mouse-and-trackpad.md) | No touchpad on the mini, so the Trackpad section is hidden | 7 |
| [sidebar-selection](sidebar-selection.md) | Accent symbol plus a focus ring on every click | 4 |
| [app-settings](app-settings.md) | Files truncates names at the row end (hides extensions); no app settings pattern | 9 |
| [swap](swap.md) | Kernel lacks ZRAM and PSI; recommends never installed | per repo |
| [ci-releases](ci-releases.md) | Never ran on a tag; polkit lint failure; OOM on a shared 12 GB runner VM; arm64 only | 10 |
| [app-store](app-store.md) | Only the apt CLI; recommends building our own Software app | 13 (MVP 9) |
| [window-snapping](window-snapping.md) | Left/right trigger at the screen edge behind the taskbar; 150 ms dwell reset by 2 px drift | 3 |
| [notifications-placement](notifications-placement.md) | Banners hard-wired top right while the centre opens beside a left taskbar | 4 (+1 optional) |
| [power-management](power-management.md) | Nothing turns the display off; sleep is off at the system level after an s2idle abort | 7 |
| [crash-management](crash-management.md) | Kernel OOM killed the session bus 6 times while the build lived (adj 200 vs 0); no PSI, no watchdogs | 7 |

## Decisions for the owner

Each plan's last section lists its decisions with a recommendation. The ones that block the most work:

- **CI:** make `desktop` public for bigger free runners; drop KVM, perf and polkit jobs from the release gate; thin or fat LTO.
- **Shortcuts:** shortcut labels as words ("Ctrl+C") or macOS symbols.
- **Theme:** slate hue, mid-tone selection fill, whether light moves too.
- **Packages:** offline-only updates; Flathub on or off; own store or GNOME Software as a stopgap.
- **Notifications:** everything opens from the clock corner, or banners stay top right.
- **Swap:** no disk swap (zram plus oomd) or swapfile plus zswap.
- **Power:** re-enable sleep after ten clean s2idle cycles; what the power button does while sleep is unavailable.
- **Crashes:** systemd-coredump in the image; whether losing the session bus should end the session.
