# Debian packaging

Unbound ships as Debian packages for Debian 13 (trixie) on arm64. Each crate that installs
something is its own binary package, built with `cargo deb` from the crate's
`[package.metadata.deb]`; the architecture-independent packages are built with debhelper from
`packaging/`. Every package of one build carries the same version, derived from git by
`ci/deb/version.sh`. How CI builds, checks and publishes them is in [ci.md](ci.md).

Packages are built with the `dist` profile (`cargo deb --profile dist`): fat LTO, one codegen
unit, `panic = "abort"` and stripped symbols. Development and test builds keep the default
profiles. Sizes are tracked in [performance.md](performance.md).

## Packages

- **Per crate** (`cargo deb`, arm64): `unbound-compositor`, the session services
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
  wins. `ci/deb-install` installs without the third-party recommends and checks that each of
  them has an installation candidate in Debian 13. Installing it adds "Unbound" to the sessions a
  display manager such as GDM, SDDM or greetd offers.
- The old `unbound-bar`, `unbound-desktop-surface`, `unbound-launcher`, `unbound-notifications`
  and `unbound-controlcenter` packages have no successors of their own: `unbound-shell`
  conflicts with and replaces them, so an upgrade removes them.
- **`unbound-archive-keyring`, `unbound-apt-source`, `unbound-apt-source-testing`**
  (`packaging/unbound-apt-source`): the repository's signing key and the APT source for the
  release and testing channels. Installation is described in the top-level `README.md`.

## Runtime stack still to be pulled in

As the remaining subsystems land, the meta-package grows toward the full runtime stack the
architecture relies on. Not yet in its dependencies:

- `greetd` (once `unbound-greeter` installs its configuration),
- PipeWire, `pipewire-pulse`, WirePlumber and `gstreamer1.0-pipewire` (audio and screen
  casting),
- `network-manager`, `bluez` and `polkitd` (Control Center, Settings and the polkit agent),
- `fonts-noto-core` and `fonts-noto-cjk` (fallback faces), and an icon theme,
- `mesa-vulkan-drivers` and `libvulkan1` are already pulled in by `unbound-compositor`.

No package depends on an X server, XWayland or an X session; `ci/deb-install` checks that none
comes along.
