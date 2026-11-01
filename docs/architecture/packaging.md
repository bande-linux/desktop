# Debian packaging

Bande ships as Debian packages for Debian testing on arm64 and amd64. Each crate that installs
something is its own binary package, built with `cargo deb` from the crate's
`[package.metadata.deb]`; the architecture-independent packages are built with debhelper from
`packaging/`. Every package of one build carries the same version, derived from git by
`ci/deb/version.sh`. How CI builds, checks and publishes them is in [ci.md](ci.md).

Packages are built with the `dist` profile (`cargo deb --profile dist`): fat LTO, one codegen
unit, `panic = "abort"` and stripped symbols. Development and test builds keep the default
profiles. Sizes are in [performance.md](performance.md).

## Packages

- **Per crate** (`cargo deb`, one build per architecture; the list is `ci/release/packages`): `bande-compositor`, the session services
  (`bande-config`, `bande-notify`, `bande-power`, `bande-sound`, `bande-clipboard`, `bande-tray`,
  `bande-automount`, `bande-polkit`, `xdg-desktop-portal-bande`), the shell clients (`bande-shell`,
  `bande-screenshot`, `bande-lock`, `bande-greeter`) and the apps (`bande-files`,
  `bande-settings`, `bande-file-chooser`). Package names stay `bande-*` whatever directory the crate lives in
  ([decision 0003](../decisions/0003-repository-layout.md)). `bande-shell` installs the one
  shell binary that hosts the desktop, the bar, the notifications, Control Center and the
  launcher, together with the Bande wallpapers and icon theme; the crates of those parts are
  libraries and build no package of their own.
- **`bande-desktop`** (`packaging/bande-desktop`, architecture `all`, section
  `metapackages`): the one package that installs a complete Bande system, the way `gnome-core`
  or `kde-plasma-desktop` do.
  It depends on the compositor, the core session services, the shell, the lock
  screen and the portal backend at the same version, and on `xdg-desktop-portal`,
  `dbus-user-session`, logind and `libpam-systemd`, on `pipewire-audio`, and on
  `bande-greeter | x-display-manager`, so there is always a way to log in: the Bande login
  window under greetd, or the display manager the system already has. It recommends
  `bande-greeter` anyway (greetd then asks which display manager to start), `network-manager`,
  `bande-apt-source`, the screenshot
  tool, Files, Settings, tray and automount services, `gnome-keyring` with
  `libpam-gnome-keyring`, Qt's portal platform theme (so Qt apps follow dark mode), `fonts-inter`,
  `xdg-user-dirs`, `upower` and `udisks2`, all of them built here at the same version. It also
  recommends, unversioned, the default apps listed in `packaging/bande-desktop/default-apps`:
  Terminal, Activity Monitor, Software and the Package Installer from
  [bande-linux/apps](https://github.com/bande-linux/apps), which are released on their own, and
  the everyday Debian apps a fresh install needs (`firefox-esr`, `gnome-text-editor`, `loupe`,
  `papers`, `showtime`, `gnome-calculator`), and suggests `gnome-clocks`. `debian/default-apps.sh`
  turns that file into the `${bande:Recommends}` and `${bande:Suggests}` substitution variables
  at build time, so any default app can be removed or replaced. It installs
  `/usr/share/applications/bande-mimeapps.list`, read only in a Bande session, which
  opens folders in Files, web pages in Firefox, text in Text Editor, pictures in Loupe, PDFs
  in Papers and videos in Showtime; a person's own choice in `~/.config/mimeapps.list` still
  wins. The `install` jobs install without the third-party recommends and check that each of
  them has an installation candidate in Debian testing. Installing it adds "Bande" to the sessions a
  display manager such as GDM, SDDM or greetd offers. With `--no-install-recommends` it brings
  the desktop, a login and sound and nothing else; the images install it that way.
- The old `bande-bar`, `bande-desktop-surface`, `bande-launcher`, `bande-notifications`
  and `bande-controlcenter` packages have no successors of their own: `bande-shell`
  conflicts with and replaces them, so an upgrade removes them.
- **`bande-archive-keyring`, `bande-apt-source`** are built and published by the APT
  repository itself (`bande-linux/apt`), not here. Installation is described in the
  top-level `README.md`.

## Runtime stack

The packages pull in the runtime stack the architecture relies on:

- `bande-desktop` depends on `fonts-noto-core` and `fonts-noto-cjk`, the fallback faces
  for scripts Inter does not cover (so an image built without recommends still has them),
  and recommends `cups` for printing; `bande-settings` depends on `locales`, so localed can
  generate any language picked in Settings;
- `bande-shell` recommends PipeWire, WirePlumber, NetworkManager with Debian's
  connectivity check, BlueZ and the Adwaita icon theme, and `bande-settings` adds
  PackageKit and power-profiles-daemon;
- `bande-polkit` depends on `polkitd`, and `bande-greeter` on `greetd (>= 0.10.3-6)`;
- `mesa-vulkan-drivers` and `libvulkan1` come with `bande-compositor`.

No package depends on an X server, XWayland or an X session; the `install` jobs check that none
comes along.
