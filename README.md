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
* **Seamless Gaming & Legacy Compatibility**: Dedicated external XWayland bridge via **`xwayland-satellite`** (full day-one compatibility with Steam, Proton, and Wine with zero X11 pollution inside the core compositor).
* **NVIDIA Explicit Sync**: Native `linux-drm-syncobj-v1` integration for tearing-free, glitch-free rendering on modern NVIDIA 560+ hardware.
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

## 🚀 Installation on Minimal Debian Server (Target)

Unbound is designed to install onto a bare-minimum Debian 13 (Trixie) server installation and boot directly into a hardware-accelerated session upon reboot:

```bash
# Add Unbound repository
curl -fsSL https://repo.unbound-linux.org/pubkey.gpg | sudo gpg --dearmor -o /etc/apt/keyrings/unbound-linux.gpg
echo "deb [signed-by=/etc/apt/keyrings/unbound-linux.gpg] https://repo.unbound-linux.org/debian trixie main" | sudo tee /etc/apt/sources.list.d/unbound-linux.list

# Install full desktop suite
sudo apt-get update && sudo apt-get install -y unbound-desktop

# Reboot directly into Unbound
sudo reboot
```

---

## 🛠️ Development & Documentation

* **[Master Architecture Plan](docs/ARCHITECTURE.md)**: Exhaustive technical design document covering the compositor, desktop session infrastructure, 30-protocol Wayland matrix, D-Bus interfaces, and 8-phase roadmap.
* **Workspace**: this repository is the Cargo workspace root. Its crates:
  * [`crates/unbound-compositor`](crates/unbound-compositor): a headless Smithay compositor (`wl_compositor`, `wl_shm`, `xdg_wm_base`, clipboard, primary selection and `ext-data-control-v1`) that renders through `wgpu` on Vulkan, packaged as `unbound-compositor` for arm64. DRM/KMS output is not implemented yet.
  * [`crates/unbound-toolkit`](crates/unbound-toolkit): semantic colour and spacing tokens, and offscreen fills and text on `wgpu`.
  * [`crates/unbound-config`](crates/unbound-config): parsing, validation and atomic application of `config.kdl`, watched on disk and published as `org.unbound.Config` on D-Bus.
  * [`crates/unbound-spring`](crates/unbound-spring): the damped spring model for animations.
  * [`crates/unbound-notify`](crates/unbound-notify): an `org.freedesktop.Notifications` daemon (specification 1.2), started by D-Bus activation; no UI yet.
  * [`crates/unbound-clipboard`](crates/unbound-clipboard): a clipboard daemon that keeps the clipboard and primary selection after the copying client quits, packaged as a systemd user service.
  * [`crates/unbound-test-bus`](crates/unbound-test-bus): private session buses for D-Bus tests.
  * [`crates/unbound-test-wayland`](crates/unbound-test-wayland): Wayland test clients for the compositor and clipboard tests.

---

## 📄 License

Dual-licensed under either of:
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
