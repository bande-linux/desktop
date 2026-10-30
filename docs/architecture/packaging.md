# Debian packaging

Unbound ships as Debian packages for Debian 13 (trixie) on arm64. Each crate that installs
something is its own binary package, built with `cargo deb` from the crate's
`[package.metadata.deb]`; the architecture-independent packages are built with debhelper from
`packaging/`. Every package of one build carries the same version, derived from git by
`ci/deb/version.sh`. How CI builds, checks and publishes them is in [ci.md](ci.md).

## Packages

- **Per crate** (`cargo deb`, arm64): `unbound-compositor`, the session services
  (`unbound-config`, `unbound-notify`, `unbound-power`, `unbound-clipboard`, `unbound-tray`,
  `unbound-automount`, `xdg-desktop-portal-unbound`), the shell clients (`unbound-dock`,
  `unbound-menubar`, `unbound-launcher`, `unbound-controlcenter`, `unbound-notifications`, `unbound-screenshot`,
  `unbound-lock`, `unbound-greeter`) and the apps (`unbound-term`, `unbound-files`,
  `unbound-settings`). Package names stay `unbound-*` whatever directory the crate lives in
  ([decision 0003](../decisions/0003-repository-layout.md)).
- **`unbound-desktop`** (`packaging/unbound-desktop`, architecture `all`): the meta-package.
  It depends on the compositor, the core session services, the Dock, the launcher, the lock
  screen and the portal backend at the same version, and on `xdg-desktop-portal`,
  `dbus-user-session`, logind and `libpam-systemd`; it recommends the terminal, screenshot
  tool, Files, Settings, tray and automount services, `fonts-inter`, `xdg-user-dirs`, `upower`
  and `udisks2`, and suggests `firefox-esr`. Installing it adds "Unbound" to the sessions a
  display manager such as GDM, SDDM or greetd offers.
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
