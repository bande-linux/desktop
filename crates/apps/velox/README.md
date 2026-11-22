# Bande Velox

Containers and virtual machines for the Bande desktop, in one app with two areas that are each
ready or not on their own. Packaged as `bande-velox`.

## Design

- **Containers** talk to any Docker Engine API (Docker Engine, Debian's `docker.io`, rootless
  Docker, Podman's compatible socket) over its unix socket: one `/events` stream with a full
  re-list on every reconnect, no async runtime, and nothing in the engine's datapath. Compose
  actions run `docker compose` with `DOCKER_HOST` set.
- **Virtual machines** run QEMU directly (`libs/vm`, the crate `bande-vm`), each in a transient
  systemd user unit with QMP for control and QEMU's D-Bus display for the console, so machines
  outlive the app and there is no libvirt or daemon of Velox's own. Machines run on the host's
  architecture: amd64 on `q35` with OVMF (`ovmf`), arm64 on `virt` with AAVMF
  (`qemu-efi-aarch64`), KVM when `/dev/kvm` is usable and TCG emulation when asked. Cloud images
  of the same architecture are verified against the vendor's checksums, signed where the vendor
  signs them.
- **In the background**: "Launch Velox at login" writes an autostart entry that starts
  `bande-velox --background`, which opens no window and follows the engine, so a container that
  stops unexpectedly still raises a notice. Velox stays there after Quit while that or the
  status item is on; with both off `--background` exits at once.
- **Installing only when asked.** When no engine is installed, Install Podman installs Debian's
  `podman` through PackageKit and starts its user socket; otherwise, when an engine or the QEMU
  stack is missing, Velox shows copyable steps. Passing a USB device to a machine and bridging it onto the local network
  ask for an administrator's password through polkit and run only `velox-helper` as root.
