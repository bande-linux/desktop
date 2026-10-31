# Unbound

Unbound is a Linux desktop environment written in Rust for Debian testing on arm64 and amd64,
with Apple Silicon as its primary hardware. It is its own desktop, with its own compositor, shell, session
services and apps, and it holds itself to the polish of macOS: every surface should feel as
smooth, consistent and finished. It is pure Wayland, with no X11 or XWayland anywhere. The
compositor, renderer, toolkit, shell, apps and services all live in this one Cargo workspace.

## Status

Unbound is early and not yet a daily driver.

- **Far along**: the compositor (Smithay with its own Vulkan renderer on `ash`, DRM/KMS output,
  frame pacing, fractional scaling, clipboard, session lock, idle, input methods, screen
  capture) and the session plumbing (configuration in `config.kdl` over D-Bus, notifications,
  power and idle, clipboard persistence, tray registry, automount, the portal backend's
  settings, the polkit agent protocol), packaged as `.deb` files and tested on every push.
- **Early**: the UI toolkit (Vello drawing and Parley text are mature; widgets, menus, scroll
  views and lists are just starting) and therefore the shell clients and apps built on it:
  Dock, menu bar, launcher, Control Center, lock screen, greeter, Files, Settings and Terminal
  all exist in a basic form.

[`docs/parity.md`](docs/parity.md) tracks each feature against macOS, and
[`NOTES/backlog.md`](NOTES/backlog.md) lists the milestones.

## Workspace

Every crate is in `crates/`. [Decision 0003](docs/decisions/0003-repository-layout.md) groups
them into role directories; the grouping below is that target.

| Role | Crates |
| :--- | :--- |
| Platform | `unbound-compositor` (the Wayland compositor), `unbound-render` (its Vulkan renderer), `unbound-protocols` (private Wayland protocols for the shell) |
| Toolkit | `unbound-toolkit` (tokens, Vello drawing, Parley text, Wayland windows, widgets, AccessKit) |
| Shell | `unbound-shell` (one process hosting `unbound-desktop-surface`, `unbound-bar`, `unbound-notifications`, `unbound-controlcenter` and `unbound-launcher`), `unbound-screenshot`, `unbound-lock`, `unbound-greeter` |
| Apps | `unbound-files`, `unbound-settings`, `unbound-term` |
| Services | `unbound-config`, `unbound-notify`, `unbound-power`, `unbound-clipboard`, `unbound-tray`, `unbound-automount`, `xdg-desktop-portal-unbound`, `unbound-polkit` |
| Libraries | `unbound-spring` (damped springs), `unbound-xdg` (desktop entries, icons, launching), `unbound-audio` (PipeWire), `unbound-network` (NetworkManager), `unbound-bluetooth` (BlueZ) |
| Testing | `unbound-test-bus` (private D-Bus session buses), `unbound-test-wayland` (Wayland test clients), `unbound-bench` (performance measurements), `unbound-sample` (toolkit sample for `ci/perf`) |

Other directories: `packaging/` holds the `unbound-desktop` and `unbound-os` meta-packages and
the APT source packages, `ci/` the scripts CI runs (release packages, install test, guest VM,
hardware-in-the-loop), and `docs/` the documentation.

## Building and testing

The toolchain is pinned in `rust-toolchain.toml` and installed by `rustup`. On Debian testing,
install the system libraries the workspace links against, and the Vulkan driver,
fonts and D-Bus daemon the tests use:

```bash
sudo apt install pkg-config libxkbcommon-dev libgbm-dev libinput-dev libseat-dev libudev-dev \
    libpipewire-0.3-dev libclang-dev libvulkan1 mesa-vulkan-drivers dbus-daemon \
    fonts-dejavu-core fonts-inter fonts-noto-cjk libfontconfig1 libwayland-client0 procps
```

Then, from the repository root:

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Tests need no GPU and no display: rendering runs on lavapipe, the compositor tests run it
headless, and D-Bus tests start their own session buses. `cargo run -p unbound-compositor --
--headless` starts the compositor without a display.

On every push to `main`, CI runs these checks in Debian testing, builds the packages for arm64
and amd64, installs them into a clean Debian testing and publishes them to the APT repository;
the KVM guest, polkit and performance jobs run on demand. See
[`docs/architecture/ci.md`](docs/architecture/ci.md).

## Installing on Debian testing

Unbound installs from its APT repository at <https://unbound-linux.github.io/apt> on Debian
testing, arm64 or amd64. The repository has one rolling suite: every change merged to `main`
that passes CI is published. Its packages are built against Debian testing and do not install
on Debian 13 (trixie).

1. Download the archive keyring and the source package from the latest release and install
   both:

   ```bash
   cd /tmp
   wget https://github.com/unbound-linux/apt/releases/latest/download/unbound-archive-keyring.deb \
        https://github.com/unbound-linux/apt/releases/latest/download/unbound-apt-source.deb
   sudo apt install ./unbound-archive-keyring.deb ./unbound-apt-source.deb
   ```
2. Install the desktop alone, next to the desktop you have:

   ```bash
   sudo apt update
   sudo apt install unbound-desktop
   ```

   Log out and choose **Unbound** at the login screen. You need a display manager that lists
   Wayland sessions, such as GDM, SDDM or a greetd front-end.

   Or turn the system into unbound-os, with Unbound's login window as the display manager,
   NetworkManager, PipeWire and compressed swap in memory:

   ```bash
   sudo apt update
   sudo apt install unbound-os
   ```

Updates arrive with `sudo apt upgrade`. The APT source lists the `main` component, with the
desktop and `unbound-os`, and the `apple` component, with the kernel for Apple Silicon Macs;
its pin lets the repository provide only Unbound's own packages. The repository is signed with
the key `6287 DE66 4599 29AA 1C01  E146 495A 6C19 8264 7E05`.

## Documentation

[`docs/README.md`](docs/README.md) is the index and gives a reading order. The main entry points:

- [`docs/architecture/overview.md`](docs/architecture/overview.md): components, process model,
  IPC, keyboard model and configuration.
- [`docs/design/DESIGN.md`](docs/design/DESIGN.md): the design principles.
- [`docs/decisions/`](docs/decisions/): decision records for rendering, the toolkit and the
  repository layout.
- [`docs/parity.md`](docs/parity.md): feature parity with macOS.

## License

Dual-licensed under the Apache License, Version 2.0, or the MIT license, at your option (as
declared in `packaging/unbound-desktop/debian/copyright`).
