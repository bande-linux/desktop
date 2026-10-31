# Debian packaging

Unbound ships as Debian packages for Debian testing on arm64 and amd64. Each crate that installs
something is its own binary package, built with `cargo deb` from the crate's
`[package.metadata.deb]`; the architecture-independent packages are built with debhelper from
`packaging/`. Every package of one build carries the same version, derived from git by
`ci/deb/version.sh`. How CI builds, checks and publishes them is in [ci.md](ci.md).

Packages are built with the `dist` profile (`cargo deb --profile dist`): fat LTO, one codegen
unit, `panic = "abort"` and stripped symbols. Development and test builds keep the default
profiles. Sizes are tracked in [performance.md](performance.md).

## Packages

- **Per crate** (`cargo deb`, one build per architecture; the list is `ci/release/packages`): `unbound-compositor`, the session services
  (`unbound-config`, `unbound-notify`, `unbound-power`, `unbound-clipboard`, `unbound-tray`,
  `unbound-automount`, `xdg-desktop-portal-unbound`), the shell clients (`unbound-shell`,
  `unbound-screenshot`, `unbound-lock`, `unbound-greeter`) and the apps (`unbound-term`, `unbound-files`,
  `unbound-settings`). Package names stay `unbound-*` whatever directory the crate lives in
  ([decision 0003](../decisions/0003-repository-layout.md)). `unbound-shell` installs the one
  shell binary that hosts the desktop, the bar, the notifications, Control Center and the
  launcher, together with the Unbound wallpapers and icon theme; the crates of those parts are
  libraries and build no package of their own.
- **`unbound-desktop`** (`packaging/unbound-desktop`, architecture `all`): the meta-package.
  It depends on the compositor, the core session services, the shell, the lock
  screen and the portal backend at the same version, and on `xdg-desktop-portal`,
  `dbus-user-session`, logind and `libpam-systemd`; it recommends the terminal, screenshot
  tool, Files, Settings, Activity Monitor, tray and automount services, `gnome-keyring` with
  `libpam-gnome-keyring`, Qt's portal platform theme (so Qt apps follow dark mode), the everyday apps a fresh install needs (`firefox-esr`,
  `gnome-text-editor`, `loupe`, `papers`, `showtime`, `gnome-calculator`), `fonts-inter`,
  `xdg-user-dirs`, `upower` and `udisks2`, and suggests `gnome-clocks`. It installs
  `/usr/share/applications/unbound-mimeapps.list`, read only in an Unbound session, which
  opens folders in Files, web pages in Firefox, text in Text Editor, pictures in Loupe, PDFs
  in Papers and videos in Showtime; a person's own choice in `~/.config/mimeapps.list` still
  wins. The `install` jobs install without the third-party recommends and check that each of
  them has an installation candidate in Debian testing. Installing it adds "Unbound" to the sessions a
  display manager such as GDM, SDDM or greetd offers.
- The old `unbound-bar`, `unbound-desktop-surface`, `unbound-launcher`, `unbound-notifications`
  and `unbound-controlcenter` packages have no successors of their own: `unbound-shell`
  conflicts with and replaces them, so an upgrade removes them.
- **`unbound-os`** (`packaging/unbound-os`, architecture `all`): the meta-package of the
  operating system. It depends on `unbound-desktop` and `unbound-greeter` at the same version,
  `unbound-apt-source`, `greetd`, `network-manager`, `pipewire`, `wireplumber` and
  `systemd-zram-generator`. `unbound-desktop` never depends on it, so the desktop installs on
  any Debian testing system on its own.
- **`unbound-archive-keyring`, `unbound-apt-source`** (`packaging/unbound-apt-source`): the
  repository's signing key, the APT source for the `rolling` suite with the `main` and `apple`
  components, and a pin that lets the repository provide only `unbound-*`,
  `xdg-desktop-portal-unbound`, the Apple kernels (`linux-*-apple-*`) and `unbound-os-apple`.
  Installation is described in the top-level `README.md`.

## Runtime stack

The packages pull in the runtime stack the architecture relies on:

- `unbound-desktop` recommends `fonts-noto-core` and `fonts-noto-cjk`, the fallback faces
  for scripts Inter does not cover, and `cups` for printing;
- `unbound-shell` recommends PipeWire, WirePlumber, NetworkManager with Debian's
  connectivity check, BlueZ and the Adwaita icon theme, and `unbound-settings` adds
  PackageKit and power-profiles-daemon;
- `unbound-polkit` depends on `polkitd`, and `unbound-greeter` on `greetd`;
- `mesa-vulkan-drivers` and `libvulkan1` come with `unbound-compositor`.

No package depends on an X server, XWayland or an X session; the `install` jobs check that none
comes along.
