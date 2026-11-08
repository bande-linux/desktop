# Debian packaging

Bande ships as Debian packages for Debian testing on arm64 and amd64. Each crate that installs
something is its own binary package, built with `cargo deb` from the crate's
`[package.metadata.deb]`; the architecture-independent packages are built with debhelper from
`packaging/`. Every package of one build carries the same version, derived from git by
`ci/deb/version.sh`. How CI builds, checks and publishes them is in [ci.md](ci.md).

Packages are built with the `dist` profile (`cargo deb --profile dist`): fat LTO, one codegen
unit, `panic = "abort"`, and `ci/release/symbols.sh` splits the symbols: each binary keeps only a
small compressed table of function names (`.gnu_debugdata`, as Fedora does), so a crash report has a
stack trace with names on every computer and with nothing to install or fetch. The line tables go
to `bande-debug-symbols` (paths by build id under `/usr/lib/debug`), which CI keeps as an artifact of the
run and does not publish: it is about as big as all the packages together. Install it to have
`coredumpctl debug` show files and lines. It strips copies in `target/stripped`, which `cargo deb`
packages, so cargo's own outputs keep their symbols and a later cargo step can't put them back;
`check-debs.sh` fails any packaged program with `.debug_*` sections or no `.gnu_debugdata`. Every build links with packed relative relocations
(`.cargo/config.toml`, [decision 0006](../decisions/0006-separate-gui-binaries.md)). Development and test builds keep the default
profiles. Sizes are in [performance.md](performance.md).

## Packages

- **Per crate** (`cargo deb`, one build per architecture; the list is `ci/release/packages`): `bande-compositor`, the session services
  (`bande-config`, `bande-notify`, `bande-power`, `bande-printd`, `bande-recovery`, `bande-reporter`, `bande-crash-reports`, `bande-sound`, `bande-spell`, `bande-clipboard`, `bande-tray`, `bande-indexer`,
  `bande-automount`, `bande-backup`, `bande-polkit`, `bande-secrets` with `libpam-bande-secrets`,
  `xdg-desktop-portal-bande`), the shell clients (`bande-shell`,
  `bande-screenshot`, `bande-lock`, `bande-greeter`) and the apps (`bande-files`,
  `bande-settings`, `bande-file-chooser`, `bande-lookup`). The Look Up dictionary packs
  (`bande-dict-en`, `bande-dict-sv`, `bande-dict-sv-wiktionary`) are made from their source data
  by `packaging/dict/build.py`, not from this tree's code; see [lookup.md](lookup.md). Package names stay `bande-*` whatever directory the crate lives in
  ([decision 0003](../decisions/0003-repository-layout.md)). Crates in `ci/release/bundled` build with them and ship inside another package
  (`bande-secrets-prompt` in `bande-secrets`). Packages in `ci/release/arch-variants` build
  with `--variant <arch>`, whose assets name the multiarch path (`libpam-bande-secrets`' PAM module). `bande-shell` installs the one
  shell binary that hosts the desktop, the bar, the notifications, Control Center and the
  launcher, together with the Bande wallpapers; the crates of those parts are
  libraries and build no package of their own.
- **`bande-icon-theme`** (built from `packaging/bande-desktop` with the metapackage,
  architecture `all`): the Bande icon theme and its accent variants from `data/icons`. Every
  package that draws icons depends on it, so a symbol never goes missing because the shell is
  not installed.
- **`bande-desktop`** (`packaging/bande-desktop`, architecture `all`, section
  `metapackages`): the one package that installs a complete Bande system, the way `gnome-core`
  or `kde-plasma-desktop` do.
  It depends on the compositor, the core session services, the shell, the lock
  screen, the keychain (`bande-secrets`) and the portal backend at the same version, on `dbus-user-session`, logind and
  `libpam-systemd`, on PipeWire, and on `bande-greeter | x-display-manager`, so there is always
  a way to log in: the Bande login window under greetd, or the display manager the system
  already has. It recommends `bande-greeter` anyway (greetd then asks which display manager to
  start), `bande-apt-source`, the screenshot tool, Files, Settings, the tray, automount, file
  chooser, polkit agent and the keychain's PAM module, all built here at the same version, and the few Debian packages in
  [Runtime stack](#runtime-stack). It also recommends, unversioned, every Bande app, listed in
  `packaging/bande-desktop/default-apps`: an app built in
  [bande-linux/apps](https://github.com/bande-linux/apps) is added there when it gets its first
  APT release. It recommends no apps from Debian; a browser comes from Software. Network printers
  need nothing more; support for USB and older printers comes from Settings ▸ Printers.
  `debian/default-apps.sh` turns that file into the `${bande:Recommends}` substitution variable
  at build time, so any default app can be removed or replaced. It installs
  `/usr/share/applications/bande-mimeapps.list`, read only in a Bande session, which opens
  folders in Files, `.deb` files in the Package Installer, plain text (with source code and
  logs, its subtypes) in TextEdit when it is installed and archives (zip, tar, gzip, bzip2, xz,
  zstd, 7-Zip) in Archive Utility, which also makes Files' Compress; Files expands an archive
  itself only when no app opens it; other types open in an installed app
  whose desktop entry declares them, and a person's own choice in
  `~/.config/mimeapps.list` still wins. A file no installed app opens gets the Open With sheet in
  Files (every app that opens files, with "Always open with"); on the desktop, it is revealed in
  Files. The `install` jobs install without the third-party recommends and check that each of
  them has an installation candidate in Debian testing. Installing it adds "Bande" to the sessions a
  display manager such as GDM, SDDM or greetd offers. With `--no-install-recommends` it brings
  the desktop, a login and sound and nothing else; the images install it that way.
- **Language packs** (`bande-l10n-tool debs`, architecture `all`, section `localization`):
  `bande-desktop-l10n-<lang>` (`sv`, `en-gb`) holds one catalog per program of this tree,
  `/usr/share/bande/l10n/<program>/<lang>.mo`, and enhances `bande-desktop`. Each program's
  own package carries its English catalog, `en.mo`, through the cargo-deb asset
  `target/release/l10n/development/<program>/*`, which `bande-l10n-tool check` requires. See
  [localisation.md](localisation.md).
  Packs follow by themselves: after each apt or PackageKit run (the `DPkg::Post-Invoke` stamp
  `/run/bande-desktop/packages-installed`, watched by `bande-language-packs.path`),
  `bande-language-packs.service` runs `/usr/libexec/bande-desktop/language-packs`. For each
  language in use (`/etc/locale.conf`, each person's `~/.config/locale.conf` and per-app
  languages) and each installed package `P`, it installs `P-l10n-<lang>-<region>`, else
  `P-l10n-<lang>`, when the archive has it and it is missing; Debian's own packs (Firefox ESR,
  LibreOffice) come the same way. It decides from dpkg's final state, exits at once and silently
  when nothing is missing, and only to install waits for apt or PackageKit to release the lock; it
  logs only what it installs, or why it couldn't (`LogLevelMax=notice` also drops systemd's start
  and finish lines). A run with nothing missing costs about 0.1 s and 0.1 s of CPU, measured on
  an arm64 builder; with no language set beyond `C` it skips the archive query. An upgrade, a new app or a new language each brings its packs.
  A run that failed, offline, runs again when NetworkManager reports a connection
  (`/usr/lib/NetworkManager/dispatcher.d/90-bande-language-packs`). Settings ▸ Language puts the
  languages with a pack (installed or in the APT index) first, says while Bande's text is being
  fetched, and says when it will come once the computer is online. Settings starts
  the service when a language changes (a polkit rule lets an active local user start this one
  unit) and follows the job's `JobRemoved` signal on its own thread, so region and keyboard
  changes never wait behind apt's lock; the setup assistant starts it at its end. The images install the packs of every language
  (`language-packs --every-language`), so a first start in any language works offline.
- **`bande-archive-keyring`, `bande-apt-source`** are built and published by the APT
  repository itself (`bande-linux/apt`), not here. Installation is described in the
  top-level `README.md`.

## Runtime stack

The packages pull in the runtime stack the architecture relies on:

Each Debian package is declared by the Bande package that uses it, and only when a Bande
component uses it at runtime; nothing a Bande component replaces comes along, directly or through
another package's recommends. `bande-desktop` itself keeps:

- `dbus-user-session`: the session bus every Bande service is activated on, under the user manager;
- `default-logind | logind`, `libpam-systemd`: the compositor's seat and session;
- `fonts-noto-core`, `fonts-noto-cjk`: the fallback faces for scripts Inter does not cover;
- `pipewire`, `wireplumber`: the sound service, screen capture and recording;
- `pipewire-pulse`: apps that play through PulseAudio (the session drop-ins start it);
- `libspa-0.2-bluetooth`: Bluetooth headphones, which the shell's Bluetooth menu connects;
- `apt-config-icons-hidpi`: the AppStream icons Software shows, at 2×;
- `systemd-oomd`, `systemd-zram-generator`: run the memory policy it ships.

It does not pull in `gnome-keyring` (the Bande keychain replaces it and conflicts with it, so the
bus name has one owner), `pipewire-alsa`, `needrestart` (Bande has
`/usr/libexec/bande-desktop/restart-services`, below), `cups` or `avahi-daemon` (`bande-printd` prints to
network printers without them; Settings ▸ Printers installs `cups` and `ipp-usb` on demand for USB
and older printers), `xdg-utils` (Bande opens files and links with `bande_xdg::open`, so neither its X11 tools
nor a fallback terminal come along), Qt's portal platform theme, `at-spi2-core` (a screen reader
depends on it) or the fonts, PackageKit, AppStream, UPower, UDisks, NetworkManager and
`xdg-user-dirs`, which the Bande packages that use them declare:

- `bande-shell` depends on NetworkManager (network menu), UPower (battery) and BlueZ
  (Bluetooth), since without them the bar would read as a computer without that hardware, and
  recommends Debian's connectivity check (captive portals), PipeWire and WirePlumber (volume),
  `xdg-user-dirs` (the Desktop and the folders Files shows); Bande's own icon theme falls back
  only to `hicolor`, so no GNOME icon theme comes along; `bande-compositor` ships Bande's own
  pointers, so no cursor theme package comes along (see compositor.md); `bande-settings` adds
  AccountsService (Users), PackageKit (Software Update), power-profiles-daemon (Power) and depends on NetworkManager (Network, VPN
  import), `xkb-data` and `locales`, so its helper can generate any language picked in Settings;
  the OpenVPN plugin is installed through PackageKit when an `.ovpn` file is imported, as Samba
  and `systemd-resolved` are from Sharing (resolved announces shares but also takes over DNS, so
  it is never pulled in silently);
- every package that draws text depends on the fonts the toolkit asks for by name (Inter, Manrope
  for titles, JetBrains Mono where it shows code), so none silently falls back to another face;
- `bande-player` (apps repo) is the one package linking GStreamer: its engine process plays for Player and Quick Look; `gstreamer1.0-plugins-bad` stays out because it pulls GTK 3;
- `bande-screenshot` depends on `gst-launch-1.0`, `pipewiresrc` and the VP8 encoder, so screen
  recording always works, and recommends the H.264 encoder for MP4;
- `bande-backup` depends on `restic` and `fuse3`: Debian's restic is the backup engine ([session.md](session.md#backups)).
- `bande-automount` depends on UDisks, `bande-polkit` on `polkitd`, and `bande-greeter` on
  `greetd (>= 0.10.3-6)`;
- `xdg-desktop-portal-bande` depends on `xdg-desktop-portal`, provides
  `xdg-desktop-portal-backend` and serves every interface itself (no `xdg-desktop-portal-gtk`);
- `mesa-vulkan-drivers` and `libvulkan1` come with `bande-compositor`.

After each dpkg run, `restart-services` (`DPkg::Post-Invoke` in
`/etc/apt/apt.conf.d/50bande-restart-services`) try-restarts every `system.slice` service whose
processes map a replaced `/usr` or `/opt` executable or library, as needrestart did, except the
login managers, the buses, logind, getty, `user@`, polkit, PackageKit, networking, Bluetooth and
container managers; it does nothing in a chroot or without systemd. It then touches
`/run/bande-desktop/packages-installed`, which makes each logged-in user's
`bande-update-finish.path` deal with the session's own processes ([Software Update](session.md)).

Every program the desktop runs and every system file it reads by path is listed in
`ci/runtime-deps` with the Debian package that ships it ([ci.md](ci.md#runtime-dependencies)).

No package depends on an X server, XWayland or an X session; the `install` jobs check that none
comes along.
