# Debian packaging

Bande ships as Debian packages for Debian testing on arm64 and amd64. Each crate that installs
something is its own binary package, built with `cargo deb` from the crate's
`[package.metadata.deb]`; the architecture-independent packages are built with debhelper from
`packaging/`. Every package of one build carries the same version, derived from git by
`ci/deb/version.sh`. How CI builds, checks and publishes them is in [ci.md](ci.md).

Packages are built with the `dist` profile (`cargo deb --profile dist`): fat LTO, one codegen
unit, `panic = "abort"` and stripped symbols. Development and test builds keep the default
profiles. Sizes are tracked in [performance.md](performance.md).

## Packages

- **Per crate** (`cargo deb`, one build per architecture; the list is `ci/release/packages`): `bande-compositor`, the session services
  (`bande-config`, `bande-notify`, `bande-power`, `bande-clipboard`, `bande-tray`,
  `bande-automount`, `xdg-desktop-portal-bande`), the shell clients (`bande-shell`,
  `bande-screenshot`, `bande-lock`, `bande-greeter`) and the apps (`bande-term`, `bande-files`,
  `bande-settings`). Package names stay `bande-*` whatever directory the crate lives in
  ([decision 0003](../decisions/0003-repository-layout.md)). `bande-shell` installs the one
  shell binary that hosts the desktop, the bar, the notifications, Control Center and the
  launcher, together with the Bande wallpapers and icon theme; the crates of those parts are
  libraries and build no package of their own.
- **`bande-desktop`** (`packaging/bande-desktop`, architecture `all`): the meta-package.
  It depends on the compositor, the core session services, the shell, the lock
  screen and the portal backend at the same version, and on `xdg-desktop-portal`,
  `dbus-user-session`, logind and `libpam-systemd`; it recommends the terminal, screenshot
  tool, Files, Settings, Activity Monitor, the Package Installer (which opens `.deb` files and
  depends on `packagekit`), tray and automount services, `gnome-keyring` with
  `libpam-gnome-keyring`, Qt's portal platform theme (so Qt apps follow dark mode), the everyday apps a fresh install needs (`firefox-esr`,
  `gnome-text-editor`, `loupe`, `papers`, `showtime`, `gnome-calculator`), `fonts-inter`,
  `xdg-user-dirs`, `upower` and `udisks2`, and suggests `gnome-clocks`. It installs
  `/usr/share/applications/bande-mimeapps.list`, read only in an Bande session, which
  opens folders in Files, web pages in Firefox, text in Text Editor, pictures in Loupe, PDFs
  in Papers and videos in Showtime; a person's own choice in `~/.config/mimeapps.list` still
  wins. The `install` jobs install without the third-party recommends and check that each of
  them has an installation candidate in Debian testing. Installing it adds "Bande" to the sessions a
  display manager such as GDM, SDDM or greetd offers.
- The old `bande-bar`, `bande-desktop-surface`, `bande-launcher`, `bande-notifications`
  and `bande-controlcenter` packages have no successors of their own: `bande-shell`
  conflicts with and replaces them, so an upgrade removes them.
- **`bande-os`** (`packaging/bande-os`, architecture `all`): the meta-package of the
  operating system. It depends on `bande-desktop` and `bande-greeter` at the same version,
  `greetd`, `network-manager`, `pipewire`, `wireplumber` and `systemd-zram-generator`, and
  recommends `bande-apt-source`. `bande-desktop` never depends on it, so the desktop installs on
  any Debian testing system on its own.
- **`bande-archive-keyring`, `bande-apt-source`** are built and published by the APT
  repository itself (`bande-linux/apt`), not here. Installation is described in the
  top-level `README.md`.

## Runtime stack

The packages pull in the runtime stack the architecture relies on:

- `bande-desktop` recommends `fonts-noto-core` and `fonts-noto-cjk`, the fallback faces
  for scripts Inter does not cover, and `cups` for printing;
- `bande-shell` recommends PipeWire, WirePlumber, NetworkManager with Debian's
  connectivity check, BlueZ and the Adwaita icon theme, and `bande-settings` adds
  PackageKit and power-profiles-daemon;
- `bande-polkit` depends on `polkitd`, and `bande-greeter` on `greetd`;
- `mesa-vulkan-drivers` and `libvulkan1` come with `bande-compositor`.

No package depends on an X server, XWayland or an X session; the `install` jobs check that none
comes along.
