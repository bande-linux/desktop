# Bande

Bande is a Linux desktop environment written in Rust for Debian testing on arm64 and amd64, with
Apple Silicon as its primary hardware. It has its own compositor, shell, session services and
apps, holds itself to the polish of macOS, and is pure Wayland: no X11 or XWayland anywhere.
It is early and not yet a daily driver; [`docs/parity.md`](docs/parity.md) shows where it stands.

## This repository

One Cargo workspace with the desktop itself, grouped by role in `crates/`
([decision 0003](docs/decisions/0003-repository-layout.md)):

| Directory | What |
| :--- | :--- |
| `platform/` | `bande-compositor` (Smithay with its own Vulkan renderer, `bande-render`) and the private Wayland protocols |
| `toolkit/` | `bande-toolkit`: Datum design tokens, Vello drawing, Parley text, widgets, Wayland windows, AccessKit |
| `shell/` | `bande-shell` (desktop, Taskbar, notifications, Control Center, launcher in one process), screenshots, lock screen, greeter |
| `apps/` | Files, Settings and the portal's file chooser |
| `services/` | config (`config.kdl` over D-Bus), notifications, power, sound, clipboard, tray, automount, portal backend, polkit agent |
| `libs/` | shared logic, including `bande-app`, the framework every app is built on |
| `testing/` | the test bus, a Wayland test client, benchmarks |

Terminal, Activity Monitor, Software, the Package Installer, Mail and Calendar live in
[bande-linux/apps](https://github.com/bande-linux/apps), built on `bande-app` at a pinned revision
of this repository and released on their own. `packaging/` holds the `bande-desktop`
metapackage, `ci/` the release and packaging scripts, `data/` wallpapers, icons and sounds.

## Building and testing

On Debian testing, install the build and test dependencies (with `sudo`, Debian's `rustup`
included) and the toolchain pinned in `rust-toolchain.toml`:

```bash
xargs ci/apt-install.sh libvulkan1 mesa-vulkan-drivers dbus-daemon at-spi2-core libfontconfig1 \
    libwayland-client0 procps fonts-inter < ci/release/build-depends
ci/release/toolchain.sh
```

The gate is fmt, clippy and the tests on a private D-Bus bus; tests need no GPU or display
(lavapipe renders, the compositor runs headless):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
dbus-run-session --config-file=crates/testing/test-bus/data/session.conf -- cargo test --workspace --locked
```

`cargo run -p bande-compositor -- --headless` starts the compositor without a display.

On the build machine, `desktop-check <worktree> -p <crate>` (fmt and `cargo check`) is all a
change needs; branches are queued with `desktop-enqueue <branch>` and `desktop-gate <worktree>`
runs the full gate once on `main` after a batch merges. These scripts live in `~/bande-linux/bin`,
outside this repository. The desktop keeps only a few end-to-end and regression tests and no
screenshot tests; [`docs/quality/tests.md`](docs/quality/tests.md) lists them and the rules.

## Releases

GitHub CI runs only on version tags (and on manual dispatch for a test build). A release is: bump
`[workspace.package] version` in `Cargo.toml`, merge to `main`, tag `vX.Y.Z`. The tag's
`release.yml` lints, tests, builds the packages for arm64 and amd64 and install-tests them in a
clean Debian testing; the APT repository ([bande-linux/apt](https://github.com/bande-linux/apt))
publishes a fully green tag run on its own. Details: [`docs/architecture/ci.md`](docs/architecture/ci.md).

## Installing

Once a version is published, Bande installs on Debian testing (not Debian 13) from
<https://bande-linux.github.io/apt>:

```bash
cd /tmp
wget https://github.com/bande-linux/apt/releases/latest/download/bande-archive-keyring.deb \
     https://github.com/bande-linux/apt/releases/latest/download/bande-apt-source.deb
sudo apt install ./bande-archive-keyring.deb ./bande-apt-source.deb
sudo apt update
sudo apt install bande-desktop
```

`bande-desktop` brings the desktop, its login window under greetd, NetworkManager, PipeWire, the
default apps and compressed swap in memory; with `--no-install-recommends` only the desktop, a
login and sound. Pick the **Bande** session at login. Updates arrive with `sudo apt upgrade` or
Settings ▸ Software Update. The archive is signed with the key
`6287 DE66 4599 29AA 1C01  E146 495A 6C19 8264 7E05`.

## Documentation

[`docs/README.md`](docs/README.md) is the index; start with
[`docs/architecture/overview.md`](docs/architecture/overview.md). Open work and owner decisions
are in [`NOTES/`](NOTES/open.md).

## License

Dual-licensed under the Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE)), or the
MIT license ([`LICENSE-MIT`](LICENSE-MIT)), at your option.
