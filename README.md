# Bande

Bande is a Linux desktop environment written in Rust for Debian 14 (forky) on amd64 and arm64. It
follows forky while it is Debian testing and aims to be ready when forky is released as stable.
It has its own compositor, shell, session services and apps, and is pure Wayland: no X11 or
XWayland anywhere. It is early and not yet a daily driver.

## This repository

One Cargo workspace with the desktop and its apps, grouped by role in `crates/`
([decision 0003](docs/decisions/0003-repository-layout.md)):

| Directory | What |
| :--- | :--- |
| `platform/` | `bande-compositor` (Smithay with its own Vulkan renderer, `bande-render`) and the private Wayland protocols |
| `toolkit/` | `bande-toolkit`: Datum design tokens, Vello drawing, Parley text, widgets, Wayland windows, AccessKit |
| `shell/` | `bande-shell` (desktop, Taskbar, notifications, Quick Settings, launcher in one process), screenshots, lock screen, greeter |
| `apps/` | the apps (below), the template a new app starts from, the portal's file chooser and the keyring prompt |
| `services/` | config (`config.kdl` over D-Bus), notifications, power, sound, clipboard, tray, automount, backups, search index, printing, spell checking, fault reports and crash collection, portal backend, polkit agent, keyring (Secret Service and its PAM module) |
| `libs/` | shared logic, including `bande-app`, the framework every app is built on, and what apps share (`bande-keyring`, `bande-discovery`, `bande-mail-core`, `bande-rich-text`, `bande-terminal-view`, `bande-vm`) |
| `testing/` | the test bus, a Wayland test client, benchmarks, a sample app and the review recorder |

`debian/` builds every package, the apps' included, as the one Debian source package
`bande-desktop`; `packaging/dict` holds the dictionary packs' source package, `ci/` the release
and packaging scripts, `data/` wallpapers, icons, sounds and the session's systemd drop-ins.

## Apps

Each app is `crates/apps/<name>`, packaged as `bande-<name>`:

| App | Package |
| :--- | :--- |
| Files | `bande-files` |
| Settings | `bande-settings` |
| Dictionary | `bande-lookup` (Look Up) |
| Terminal | `bande-terminal` |
| Software | `bande-software` |
| Package Installer | `bande-installer` (opens `.deb` files) |
| System Monitor | `bande-monitor` |
| Mail | `bande-mail` (IMAP and SMTP) |
| Notes | `bande-notes` (on this computer or in an IMAP account's Notes folder) |
| Calendar | `bande-calendar` (CalDAV and iCalendar files) |
| Contacts | `bande-contacts` (CardDAV and vCard files) |
| Velox | `bande-velox` (containers of Docker Engine or Podman, and QEMU virtual machines) |
| Calculator | `bande-calculator` (Basic, Scientific and Programmer, paper tape, RPN) |
| Player | `bande-player` (audio and video; decoding runs in the `bande-media-engine` child process, the only part linking GStreamer) |
| Viewer | `bande-viewer` (images and PDF documents; PDFs render in a child process) |
| Clock | `bande-clock` (alarms, timers, a stopwatch and a world clock; rings with its window closed) |
| Text Editor | `bande-text-editor` (plain and rich text, Markdown, HTML, Office Open XML and OpenDocument) |
| Disks | `bande-disks` (disks, volumes and disk images through UDisks2) |
| Archives | `bande-archive` (zip, tar, gzip, bzip2, xz, zstd and 7-Zip; Files' Compress) |

`ci/new-app.sh mail-merge MailMerge "Mail Merge"` makes `crates/apps/mail-merge` (`bande-mail-merge`,
`org.bande.MailMerge`) from the template and prints what else to add. How an app is built is in
[`docs/architecture/apps.md`](docs/architecture/apps.md).

## Browser

Bande's Browser (`bande-browser`) is Debian's `firefox-esr` with Bande's patches, branding, prefs
and theme, in `browser/`, outside the Cargo workspace and with its own release rhythm: it builds
whenever Debian uploads a new `firefox-esr`. `browser/firefox-esr-bande/` is the
`bande-firefox-esr-settings` package for Debian's own `firefox-esr`. See
[`browser/README.md`](browser/README.md).

## Building and testing

On Debian forky, install the build and test dependencies, the Build-Depends of `debian/control`
and the tools of the checks (with `sudo`; Debian's own `rustc`, `cargo`, `rustfmt` and
`rust-clippy` included):

```bash
ci/release/verify.sh --setup gate
```

The gate is the release checks: fmt, clippy, the layering rules, the tests on a private D-Bus bus
and the packages with their checks; tests need no GPU or display (lavapipe renders, the
compositor runs headless):

```bash
PROFILE=dev ci/release/verify.sh gate
```

`cargo run -p bande-compositor -- --headless` starts the compositor without a display.

Each change only has to compile (`cargo fmt --all --check`, `cargo check --all-targets`); the
full gate, `ci/release/verify.sh gate` (the release checks CI runs), runs before a branch is
merged to `main`. Bande keeps only a few end-to-end and regression tests and no screenshot tests;
[`docs/quality/tests.md`](docs/quality/tests.md) lists them and the rules.

## Releases

`release.yml` runs only on version tags (and on manual dispatch for an unpublished build). Every
package, desktop and apps, is released together at one version. A release
is: bump `[workspace.package] version` in `Cargo.toml` and add the `X.Y.Z-1` entry to
`debian/changelog`, merge to `main` once the gate passes, and tag
`vX.Y.Z`. The tag's `release.yml` lints, tests, makes the source package, builds every package
from it offline for arm64 and amd64 and runs its autopkgtests in a clean Debian forky; the APT repository ([bande-linux/apt](https://github.com/bande-linux/apt))
publishes a fully green tag run on its own. Details: [`docs/architecture/ci.md`](docs/architecture/ci.md).

## Installing

Once a version is published, Bande installs on Debian 14 (forky) from
<https://bande-linux.github.io/apt>:

```bash
cd /tmp
suite=$(. /etc/os-release && echo "$VERSION_CODENAME")
wget https://github.com/bande-linux/apt/releases/latest/download/bande-archive-keyring.deb \
     "https://github.com/bande-linux/apt/releases/latest/download/bande-apt-source-$suite.deb"
sudo apt install ./bande-archive-keyring.deb "./bande-apt-source-$suite.deb"
sudo apt update
sudo apt install bande-desktop
```

`bande-desktop` brings the desktop, its login window under greetd, NetworkManager and PipeWire,
and recommends the Bande apps (all but Velox) and Bande's Browser; other apps install from Software. With `--no-install-recommends` it
brings only the desktop, a login and sound. Pick the **Bande** session at login. Updates arrive with `sudo apt upgrade` or
Settings ▸ Software Update. The archive is signed with the key
`04FC EAF9 B799 98D9 828C  256B 7963 AC44 9BD9 992E`.

## Documentation

[`docs/README.md`](docs/README.md) is the index; start with
[`docs/architecture/overview.md`](docs/architecture/overview.md).

## License

Dual-licensed under the Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE)), or the
MIT license ([`LICENSE-MIT`](LICENSE-MIT)), at your option.
