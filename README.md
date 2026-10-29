# Unbound Desktop Environment (`unbound-desktop`)

> Next-Generation, High-Performance Linux Desktop Environment built in **Rust** for **Debian** & Wayland.

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)
[![Debian](https://img.shields.io/badge/Debian-13%20(Trixie)-red.svg)](https://www.debian.org)
[![Wayland](https://img.shields.io/badge/Wayland-Pure%20Native-orange.svg)](https://wayland.freedesktop.org)
[![Rust](https://img.shields.io/badge/Rust-2024%20Edition-black.svg)](https://www.rust-lang.org)

---

## ⚡ Overview

**Unbound** is an ultra-modern, production-grade Linux Desktop Environment engineered from scratch in pure **Rust**, targeting **Debian** as its primary operating system.

It marries the organizational depth and workflow polish of **macOS** (Miller columns, exhaustive settings taxonomy, instant file previews, unkillable force-quit watchdog) with the desktop layout ergonomics of modern Linux (modular, repositionable panels—defaulting to an ergonomic left vertical taskbar, with seamless support for bottom, right, or top layouts).

### Core Highlights
* **Layered Graphics Engine**: Built on **Smithay** for kernel DRM/KMS atomic modesetting, libinput, and session management, paired with a custom **`wgpu` / Vulkan** render pipeline and Apple-grade spring physics layout engine.
* **Pure Wayland**: No X11 code, X libraries, X server or XWayland bridge anywhere in the desktop; every app runs as a native Wayland client.
* **Full Session Infrastructure**: Native XDG Desktop Portals (`xdg-desktop-portal-unbound` with PipeWire screen casting), clipboard management (`wl_data_device`, `zwp_primary_selection`, persistent `ext-data-control-v1`), screen lock (`ext-session-lock-v1` with crash resilience), idle management, polkit agent, and CJK input methods.
* **Robust Display Management**: Privileged PAM session handling via **`greetd`**, paired with an unprivileged, modern GPU-accelerated client (**`unbound-greeter`**).
* **Autonomous Software Factory**: Driven by a zero-human-in-the-loop autonomous SDLC engine (`unbound-factory`) backed by self-hosted arm64 CI runners (headless Wayland via Lavapipe + ephemeral Debian 13 arm64 VM boot tests).

---

## 🏛️ Architecture & Component Repositories

The Unbound ecosystem is maintained under the **[`unbound-linux`](https://github.com/unbound-linux)** organization:

| Repository | Description | Status |
| :--- | :--- | :---: |
| **[`unbound-linux/desktop`](https://github.com/unbound-linux/desktop)** | Umbrella workspace root, meta-package, systemd units, session targets | Active |
| **[`unbound-linux/compositor`](https://github.com/unbound-linux/compositor)** | Smithay + wgpu Wayland compositor with explicit sync and spring physics | Planning |
| **[`unbound-linux/shell`](https://github.com/unbound-linux/shell)** | Modular, repositionable panel framework, top menu bar, and watchdog | Planning |
| **[`unbound-linux/greeter`](https://github.com/unbound-linux/greeter)** | Modern, unprivileged login greeter for greetd | Planning |
| **[`unbound-linux/portal`](https://github.com/unbound-linux/portal)** | xdg-desktop-portal backend (PipeWire screencast, screenshot, file picker) | Planning |
| **[`unbound-linux/toolkit`](https://github.com/unbound-linux/toolkit)** | Shared Rust UI widget library (wgpu, semantic colors, text navigation, AccessKit) | Planning |
| **[`unbound-linux/apps`](https://github.com/unbound-linux/apps)** | Productivity applications (launcher, Miller columns file manager, settings, terminal) | Planning |
| **[`unbound-linux/factory`](https://github.com/unbound-linux/factory)** | Autonomous SDLC orchestrator CLI daemon and worker loop | Planning |
| **[`unbound-linux/runners`](https://github.com/unbound-linux/runners)** | Self-hosted arm64 CI runners (Docker on Apple Silicon) | Active |

For complete technical specifications, see **[Master Architecture & Execution Plan](docs/ARCHITECTURE.md)**.

---

## 🚀 Installation on Debian 13

Unbound installs from its APT repository at <https://unbound-linux.github.io/apt> on Debian 13
(trixie), arm64. You need a display manager that lists Wayland sessions, such as GDM, SDDM, or a
greetd front-end.

1. Choose a channel:
   - **Release** carries tagged versions only.
   - **Testing** carries a build of every change merged to `main`.

   Download the archive keyring and the channel's package from the latest release, then install
   both. For the release channel:

   ```bash
   cd /tmp
   wget https://github.com/unbound-linux/apt/releases/latest/download/unbound-archive-keyring.deb \
        https://github.com/unbound-linux/apt/releases/latest/download/unbound-apt-source.deb
   sudo apt install ./unbound-archive-keyring.deb ./unbound-apt-source.deb
   ```

   For testing, use `unbound-apt-source-testing.deb` in both places.
2. Install the desktop:

   ```bash
   sudo apt update
   sudo apt install unbound-desktop
   ```

Log out and choose **Unbound** at the login screen. Updates arrive with `sudo apt upgrade` from
your channel only.

- **Switch channels:** run `sudo apt install unbound-apt-source-testing` or
  `sudo apt install unbound-apt-source`.
- **Signing key:** the repository is signed with the key
  `6287 DE66 4599 29AA 1C01  E146 495A 6C19 8264 7E05`.

---

## 🛠️ Development & Documentation

* **[Master Architecture Plan](docs/ARCHITECTURE.md)**: Exhaustive technical design document covering the compositor, desktop session infrastructure, 30-protocol Wayland matrix, D-Bus interfaces, and 8-phase roadmap.
* **Workspace**: this repository is the Cargo workspace root. Its crates:
  * [`crates/unbound-compositor`](crates/unbound-compositor): a headless Smithay compositor (`wl_compositor`, `wl_shm`, `linux-dmabuf-v1`, `xdg_wm_base`, `wl_output`, `wp_presentation`, clipboard, primary selection and `ext-data-control-v1`) that renders with `unbound-render`, packaged as `unbound-compositor` for arm64. DRM/KMS output is not implemented yet.
  * [`crates/unbound-render`](crates/unbound-render): the compositor's Vulkan renderer on `ash`, with persistent textures, damage-limited uploads and repaints, and occlusion of what opaque windows hide.
  * [`crates/unbound-toolkit`](crates/unbound-toolkit): semantic colour and spacing tokens, and offscreen fills and text on `wgpu`.
  * [`crates/unbound-config`](crates/unbound-config): parsing, validation and atomic application of `config.kdl`, watched on disk and published as `org.unbound.Config` on D-Bus by the `unbound-config` session daemon, which also writes settings back to the file and reports an invalid file as a notification.
  * [`crates/unbound-spring`](crates/unbound-spring): the damped spring model for animations.
  * [`crates/unbound-notify`](crates/unbound-notify): an `org.freedesktop.Notifications` daemon (specification 1.2), started by D-Bus activation; no UI yet.
  * [`crates/unbound-power`](crates/unbound-power): a session service that locks before sleep through logind, posts battery alerts from UPower and serves `org.freedesktop.ScreenSaver` idle inhibition; also the idle state machine.
  * [`crates/unbound-polkit`](crates/unbound-polkit): the polkit authentication agent protocol, tested against polkitd; its dialog awaits the toolkit.
  * [`crates/xdg-desktop-portal-unbound`](crates/xdg-desktop-portal-unbound): the xdg-desktop-portal backend; `Settings` is live, `FileChooser`, `Screenshot` and `ScreenCast` wait for their UI and capture implementations.
  * [`crates/unbound-tray`](crates/unbound-tray): the `org.kde.StatusNotifierWatcher` registry for tray icons.
  * [`crates/unbound-automount`](crates/unbound-automount): mounts removable drives through UDisks2 and warns about drives pulled without ejecting.
  * [`crates/unbound-clipboard`](crates/unbound-clipboard): a clipboard daemon that keeps the clipboard and primary selection after the copying client quits; the compositor starts it and offers it `ext-data-control-v1` on that connection only.
  * [`crates/unbound-bench`](crates/unbound-bench): frame time, start time and memory measurements that `ci/perf` runs on every push.
  * [`crates/unbound-test-bus`](crates/unbound-test-bus): private session buses for D-Bus tests.
  * [`crates/unbound-test-wayland`](crates/unbound-test-wayland): Wayland test clients for the compositor and clipboard tests.

---

## 📄 License

Dual-licensed under either of:
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
