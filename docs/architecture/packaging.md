# Debian packaging

Bande ships as Debian packages for Debian 14 (forky) on arm64 and amd64. Every package of the
desktop, the apps and the language packs included, is a binary package of one Debian source
package, `bande-desktop`, whose `debian/` is at the root of this repository: `3.0 (quilt)`,
debhelper compat 14, built with Debian's `rustc` and `cargo` through dh-cargo's cargo wrapper
(`/usr/share/cargo/bin/cargo`, which passes Debian's build flags, the target and
`DEB_BUILD_OPTIONS`). The dictionary packs are a second source package, `bande-dict`
([lookup.md](lookup.md)). How CI builds, checks and publishes them is in [ci.md](ci.md).

## The source package

- **Upstream tarball**: `bande-desktop_X.Y.Z.orig.tar.xz` is `git archive` of the commit
  (`ci/deb/orig.sh`); `.gitattributes` leaves `debian/` and `browser/` out.
- **Vendored crates**: `bande-desktop_X.Y.Z.orig-vendor.tar.xz`, a component tarball unpacked
  in `vendor/`, holds every crate of `Cargo.lock` (`ci/deb/vendor.sh`: `cargo vendor --locked
  --versioned-dirs`). A crate that no Linux build on amd64 or arm64 uses keeps only its
  `Cargo.toml` and an empty `src/lib.rs`, which cargo needs to resolve, so no code or prebuilt
  library for other platforms is in the source. Both tarballs are made byte for byte the same for
  the same commit (sorted, dated by the commit, single-threaded xz).
- **Two ways to build, one `debian/`**: with the build profile `pkg.bande.vendor`, the way Bande
  builds, `debian/rules` builds from `vendor/` with `Cargo.lock` (`--locked`), offline. Without
  it, the way a build in Debian's archive would, it builds from Debian's packaged crates
  (`cargo prepare-debian debian/cargo_registry --link-from-system`, without `Cargo.lock`), and
  fills `Built-Using`/`Static-Built-Using` with dh-cargo; the `librust-*-dev` Build-Depends are
  marked `<!pkg.bande.vendor>` and written from the workspace's direct dependencies by
  `ci/deb/build-depends.py` (`--check` in lint). `debian/rules` fails when the profile and the
  presence of `vendor/` disagree. Most crates Bande uses are in Debian; `ci/deb/debian-gap.py`
  reports the rest ([ci.md](ci.md#jobs)).
- **`debian/control`** holds every binary package; siblings are pinned at `(=
  ${binary:Version})`, the architecture-independent ones at `${source:Version}` (`bande-desktop`
  asks for the others at `>= ${source:Version}`, which a binNMU of them still meets).
  `debian/<package>.install` lists what each ships: the programs from `debian/build/bin`
  (the cargo output), files from the crates' `data/` and `data/` at the root under their
  installed names, and what `debian/rules` generates in `debian/build`: the pointers
  (`bande-cursor-theme`), the catalogs (`bande-l10n-tool pack`) and each metainfo with its
  releases (`debian/metainfo.py`: one per upstream version in `debian/changelog`, dated by its
  newest entry and described by its entries' items). After `dh_install`,
  `debian/manpages.py` writes a manual page for each program in `/usr/bin`: the crate's
  `data/<program>.1` when it has one (a package's second program must), or else the Cargo
  description of the crate that builds it and the package's description.
- **`debian/copyright`** is one DEP-5 file for the whole source, written by `ci/copyright.py`
  (`--check` in lint fails when it no longer matches `Cargo.lock`): `ci/copyright` for this tree,
  `ci/copyright.d/*` for the third-party material in it, a paragraph per vendored crate with the
  authors and licence from its `Cargo.toml`, then each licence's text from `ci/licenses`.
- **Units and maintainer scripts**: `dh_installsystemd` and `dh_installsystemduser` enable and
  restart the units: the user units of the session services are enabled for every user and
  never started from a package; `bande-backup`'s timer and service are neither enabled nor
  started (Settings turns backups on; `postinst` restarts a running backup service), the
  login window's console fallback, its boot screen path unit and the fault reporter's
  socket-activated services are not started, and `bande-crash-reports@.service`, a template that `dh_installsystemd` skips, is
  enabled by its own `postinst`. `bande-reporter` and `bande-greeter` create their state
  folders with `systemd-tmpfiles` (`debian/<package>.tmpfiles`); each person's login-window
  folder below `/var/lib/bande-greeter/users` comes from the static template
  `bande-greeter-share@.service`, which a `user@.service` drop-in wants, so only its owner writes
  it and only the login window reads it. `bande-greeter` ships its PAM service as the conffile
  `/etc/pam.d/bande-greeter` (`debian/bande-greeter.pam`, after Debian's `gdm-password`), which
  its greetd configuration names, so greetd's own `/etc/pam.d/greetd` stays untouched.
  `/etc/bande` is created by `bande-reporter`'s tmpfiles entry, or by Settings' locale helper
  when it first writes the login window's `/etc/bande/login.kdl`; purging `bande-reporter` or
  `bande-settings` removes the file it wrote there, and the folder once it is empty. The other scripts purge state, restart nothing else and run
  `pam-auth-update` for `libpam-bande-secrets` (`Multi-Arch: same`, its module in
  `/usr/lib/<multiarch>/security`); `pinentry-bande` is a `pinentry` alternative
  ([session](session.md#keyring-bande-secrets)), Terminal is an `x-terminal-emulator` alternative and the
  boot screen a `default.plymouth` alternative (`dh_installalternatives`). The boot screen's
  initramfs hook (`dh_installinitramfs`) rebuilds the initramfs through initramfs-tools'
  trigger. GRUB has no trigger to regenerate `grub.cfg`, so the boot screen's `postinst` and
  its `postrm` on purge run `update-grub` when it is on the `PATH` and `/boot/grub/grub.cfg`
  exists, reporting a failure without failing the package; no maintainer script runs
  `update-initramfs` or `plymouth-set-default-theme`, and none touches another package's
  configuration.
- **Version**: `debian/changelog` names the release, `X.Y.Z-1`, with `X.Y.Z` the
  `[workspace.package] version`; a packaging-only fix is `X.Y.Z-2` (tag `vX.Y.Z-2`). A build
  for a Debian release adds an entry with that release's suffix, `X.Y.Z-1~deb14` for forky
  (`ci/deb/build.sh`; [ci.md](ci.md#versions)); sbuild's `--append-to-version` would make a
  binNMU-style build without the architecture-independent packages, so an sbuild build takes
  a source package with that entry.
- **Tests**: `debian/tests` holds the autopkgtests: `desktop` (bande-desktop alone brings a
  login window and sound, no X server, the login window as the display manager with greetd's
  own service and configuration untouched and Bande's PAM service, its pictures and
  failed-session folders, and purging it handing the display to greetd), `every-package` (the
  one version, every program and file in
  `ci/runtime-deps` without recommends, the installed compositor serving Wayland clients),
  `boot-screen` (the boot screen is a `default.plymouth` alternative that only applies when no
  other theme is chosen, the initramfs carries it, its link and Inter, it installs without GRUB,
  with GRUB the generated `linux` lines carry `splash` once, also when `/etc/default/grub` has
  it, Settings' helper turns it off and back to the unmodified conffile, and purging it leaves
  neither the alternative, the theme nor `splash` behind), `system-setup` (every package with its recommends brings none of the short list of packages
  that take over system-wide setup, such as `systemd-resolved`, `iwd`, a firewall or another
  display manager, and leaves `/etc/resolv.conf`, `/etc/network/interfaces` and
  `/etc/nsswitch.conf` of a system booted with systemd as they were),
  `display-manager` (next to LightDM, the login window is offered and not imposed,
  `dpkg-reconfigure` switches both ways, and removing it gives the display back to LightDM),
  `coexistence` (`bande-desktop` installs next to `gnome-core` without removing anything, and
  GNOME's keyring keeps its PAM profile, socket and activation, and Bande's keyring, started
  while logind (python-dbusmock's template) lists only a GNOME session of the user, exits
  without taking `org.freedesktop.secrets`, which it takes once a Bande session is listed), `services` (the
  settings service on a private bus, the compositor headless), `portal` (the colour scheme
  through `xdg-desktop-portal`) and `metadata` (desktop entries, metainfo and units validate).

Packages are built with the `dist` profile (`PROFILE` picks another): fat LTO, one codegen
unit, `panic = "abort"`, line tables only. `dh_strip` moves the line tables to each package's
`-dbgsym` package (paths by build id under `/usr/lib/debug`), which CI keeps as an artifact of
the run and does not publish, and `debian/gnu-debugdata.sh` keeps in each program a small
compressed table of its function names (`.gnu_debugdata`), so a crash report has a stack
trace with names on every computer, with nothing to install or fetch. Install the `-dbgsym`
packages to have `coredumpctl debug` show files and lines. `check-debs.sh` fails any packaged
program that keeps `.debug_info`, `.debug_line` or `.debug_str`, or lacks `.gnu_debugdata`.
Every build links with packed relative relocations (`.cargo/config.toml`, [decision
0006](../decisions/0006-separate-gui-binaries.md)) and maps the build paths to `/build`. Sizes
are in [performance.md](performance.md).

## Packages

- **The programs** (architecture `amd64`, `arm64`): `bande-compositor`, the session services
  (`bande-config`, `bande-notify`, `bande-power`, `bande-printd`, `bande-reporter`, `bande-crash-reports`, `bande-sound`, `bande-spell`, `bande-clipboard`, `bande-tray`, `bande-indexer`,
  `bande-automount`, `bande-backup`, `bande-polkit`, `bande-secrets` with `libpam-bande-secrets`
  and `pinentry-bande`,
  `xdg-desktop-portal-bande`), the shell clients (`bande-shell`,
  `bande-screenshot`, `bande-lock`, `bande-greeter`) and the apps (`bande-files`,
  `bande-settings`, `bande-file-chooser`, `bande-lookup`, `bande-terminal`, `bande-software`,
  `bande-installer`, `bande-monitor`, `bande-mail`, `bande-notes`, `bande-calendar`, `bande-velox`,
  `bande-calculator`, `bande-player`, `bande-viewer`, `bande-clock`, `bande-text-editor`,
  `bande-disks`, `bande-archive`). Package names stay `bande-*` whatever directory the crate
  lives in ([decision 0003](../decisions/0003-repository-layout.md)); a program may ship in
  another crate's package (`bande-secrets-prompt` in `bande-secrets`). A package that renders
  PDF text through hayro carries a lintian override for `embedded-library freetype`: the
  pure-Rust skrifa carries FreeType's list of hint-reliant fonts, not FreeType. `bande-shell`
  installs the one shell binary that hosts the desktop, the bar, the notifications, Quick
  Settings and the launcher, together with the Bande wallpapers; the crates of those parts are
  libraries and build no package of their own.
- **`bande-icon-theme`** (architecture `all`): the Bande icon theme and its accent variants from `data/icons`. Every
  package that draws icons depends on it, so a symbol never goes missing because the shell is
  not installed.
- **`bande-plymouth-theme`** (architecture `all`): the boot screen, a Plymouth `two-step` theme
  that `bande-plymouth-theme` (a program of `crates/libs/splash`, never installed) draws at build
  time from the Datum tokens and the mark in `data/branding/mark.svg`, the same mark the bar
  shows. The boot screen is part of the desktop, so `bande-desktop` recommends it (a system
  installed without recommends starts with no Plymouth). Plymouth only draws with `splash` on the
  kernel command line, which Debian's GRUB leaves out, so the package ships the conffile
  `/etc/default/grub.d/bande-splash.cfg` (`crates/libs/splash/data`), which `grub-mkconfig`
  sources after `/etc/default/grub` and which adds `splash` unless it is already there. It stays
  until purge like any conffile; a removed package's `splash` only shows Plymouth's own theme.
  Settings ▸ About ▸ Show a boot screen shows whether that line is active and comments it out or
  back in through `bande-boot-screen-helper`, an administrator's change to the conffile that
  dpkg keeps on upgrade, installing the package first when it is missing; on a boot manager
  other than GRUB it says what to do instead. Plymouth draws images at their own size and doubles
  them on a HiDPI display, with no larger variant to choose, so the assets are drawn on whole
  pixels at scale 1.
- **`bande-desktop`** (architecture `all`, section `metapackages`): the one package that installs a complete Bande desktop.
  It depends on the compositor, the core session services, the shell, the lock
  screen, the keyring (`bande-secrets`) and the portal backend at the same version, on `dbus-user-session`, logind and
  `libpam-systemd`, on PipeWire and WirePlumber, and on `bande-greeter | x-display-manager`, so there is always
  a way to log in: the Bande login window, or the display manager the system already has. It
  recommends `bande-greeter` anyway (debconf then asks which display manager to start; see
  [session.md](session.md#display-manager-and-greeter)), Bande's Browser (`bande-browser`, built
  from `browser/` on its own rhythm, so unversioned), the boot screen (`bande-plymouth-theme`), fault reporting (`bande-reporter`, `bande-crash-reports`), printing, spell checking, the
  clipboard daemon, the screenshot tool, Files, Settings, the tray, file indexing
  (`bande-indexer`), automount, backups (`bande-backup`), the file chooser, the polkit agent, the keyring's PAM module and the default apps (every app but Velox),
  all built here at the same version, and the few Debian packages in
  [Runtime stack](#runtime-stack); being recommends, any of them can be removed or replaced. It
  recommends no apps from Debian. Network printers
  need nothing more; support for USB and older printers comes from Settings ▸ Printers. It installs
  `/usr/share/applications/x-bande-mimeapps.list` (`data/x-bande-mimeapps.list`), read only in a Bande session, which opens
  folders in Files, `.deb` files in the Package Installer, plain text (with source code and
  logs, its subtypes) in Text Editor when it is installed, web pages and links in the Browser,
  `mailto:` links in Mail, calendar files and `webcal:` links in Calendar, and archives (zip, tar,
  gzip, bzip2, xz, zstd, 7-Zip) in Archives, which also makes Files' Compress; Files expands an archive
  itself only when no app opens it; other types open in an installed app
  whose desktop entry declares them, and a person's own choice in
  `~/.config/mimeapps.list` still wins. A file no installed app opens gets the Open With sheet in
  Files (every app that opens files, with "Always open with"); on the desktop, it is revealed in
  Files. The autopkgtests install it without the third-party recommends and check that each of
  them has an installation candidate in Debian forky. Installing it adds "Bande" to the sessions a
  display manager such as GDM, SDDM or greetd offers. With `--no-install-recommends` it brings
  the desktop, a login and sound and nothing else.
- **Language packs** (architecture `all`, section `localization`): `bande-desktop-l10n-<lang>`
  (`sv`, `en-gb`, a stanza in `debian/control` and a `.install` each) holds one catalog per
  program the source package ships, `/usr/share/bande/l10n/<program>/<lang>.mo`, and enhances
  `bande-desktop`. Each program's own package carries its English catalog, `en.mo`, through the
  line `debian/build/l10n/development/<program>/* usr/share/bande/l10n/<program>/` of its
  `.install`, which `bande-l10n-tool check` requires. A translation-only release builds the
  packs alone with the build profile `pkg.bande.l10n-only`, which leaves out the other
  architecture-independent packages ([ci.md](ci.md#jobs)). See [localisation.md](localisation.md).
  Settings ▸ Language puts the languages with a pack (installed or in the APT index) first. When
  a language is chosen there, for the system or for one app, Settings installs its missing pack
  (`bande-desktop-l10n-<lang>-<region>` and `bande-desktop-l10n-<lang>`) through PackageKit on
  its own thread, says while Bande's text is being fetched, and says when it could not be.
- **`bande-archive-keyring`, `bande-apt-source`** are built and published by the APT
  repository itself (`bande-linux/apt`), not here. Installation is described in the
  top-level `README.md`.

## Runtime stack

Each Debian package is declared by the Bande package that uses it, and only when a Bande
component uses it at runtime; nothing a Bande component replaces comes along, directly or through
another package's recommends. A package depends only on what it cannot work without; what a
feature of it needs is a recommendation, so the desktop stays installable while Debian lacks a
package for a while, and the feature says what is missing. `bande-desktop` itself keeps:

- `dbus-user-session`: the session bus every Bande service is activated on, under the user manager;
- `default-logind | logind`, `libpam-systemd`: the compositor's seat and session;
- `libx11-data`: the Compose tables xkbcommon reads for dead keys, text files and no library
  ([overview.md](overview.md));
- `fonts-noto-core`, `fonts-noto-cjk` (recommended): the fallback faces for scripts Inter does not cover;
- `pipewire`, `wireplumber`: the sound service, screen capture and recording;
- `pipewire-pulse` (recommended): apps that play through PulseAudio (the session drop-ins start it).
  The login window's user (`_greetd`) gets PipeWire and WirePlumber from the ordinary user units,
  so the screen reader can speak before anyone signs in; WirePlumber opens the sound card only for
  the session active on the seat, so it moves to the person's session at sign-in;
- `libspa-0.2-bluetooth` (recommended): Bluetooth headphones, which the shell's Bluetooth menu connects;
- `apt-config-icons-hidpi` (recommended): the AppStream icons Software shows, at 2×;
- `systemd-oomd` (recommended): closes an app when memory stays under pressure.

It does not pull in `gnome-keyring` (it installs beside Bande's keyring, which runs only in a Bande
session, so another desktop's keyring owns `org.freedesktop.secrets` there), `pipewire-alsa` or `cups` (`bande-printd` prints to network printers
without it; Settings ▸ Printers installs `cups` and `ipp-usb` on demand for USB and older printers), `xdg-utils` (Bande opens files and links with `bande_xdg::open`, so neither its X11 tools
nor a fallback terminal come along), Qt's portal platform theme, `at-spi2-core` (a screen reader
depends on it) or the fonts, PackageKit, AppStream, UPower, UDisks, NetworkManager and
`xdg-user-dirs`, which the Bande packages that use them declare:

- `bande-shell` depends on NetworkManager (network menu) and UPower (battery), since without
  them the bar would read as a computer without a network or battery, and recommends BlueZ
  (Bluetooth), the colour emoji font, PipeWire and WirePlumber (volume),
  `xdg-user-dirs` (the Desktop and the folders Files shows); Bande's own icon theme falls back
  only to `hicolor`, so no GNOME icon theme comes along; `bande-compositor` ships Bande's own
  pointers, so no cursor theme package comes along (see compositor.md); `bande-settings` adds
  AccountsService (Users), PackageKit (Software Update), power-profiles-daemon (Power) and
  NetworkManager (Network, VPN import) to its recommends and depends on `xkb-data` and `locales`,
  so its helper can generate any language picked in Settings;
  the OpenVPN plugin is installed through PackageKit when an `.ovpn` file is imported, as Samba
  and `avahi-daemon` are from Sharing and Debian's connectivity check
  (`network-manager-config-connectivity-debian`) is from Privacy & Security;
- every package that draws text depends on the fonts the toolkit asks for by name (Inter, Manrope
  for titles, JetBrains Mono where it shows code), so none silently falls back to another face;
- `bande-player` is the one package linking GStreamer: its engine process plays for Player and
  Glance; it depends on the base plugins and recommends the good, PulseAudio, libav and dav1d
  plugins (the codecs); `gstreamer1.0-plugins-bad` stays out because it pulls GTK 3;
- `bande-screenshot` recommends `gst-launch-1.0`, `pipewiresrc`, the VP8 encoder and the H.264
  encoder for MP4: screen recording is offered only when they are there;
- `bande-reporter` recommends `eu-stack` (elfutils) for the stacks of a hung program and
  `bande-crash-reports` `libdw` for systemd-coredump's stack traces; a report without them
  has no stack;
- `bande-spell` recommends the English dictionary, `bande-files` and `bande-lookup` DejaVu Sans
  Mono, and `bande-clock` the notification and sound services, which ring its alarms;
- `bande-backup` depends on `restic` and `fuse3`: Debian's restic is the backup engine ([session.md](session.md#backups)).
- `bande-automount` suggests `avahi-daemon`, which Files browses for nearby computers over its
  D-Bus interface (Samba announces shares through it too), and `libnss-mdns`, so the `.local`
  names it finds resolve when mounting; they stay out of an install because Avahi announces the
  computer on the network and `libnss-mdns` changes `/etc/nsswitch.conf`, so the Network folder
  offers to install them;
- `bande-printd` suggests `avahi-daemon` too, which it browses for driverless printers; Settings ▸
  Printers offers to install it and printers added by address print without it;
- `bande-automount` recommends `bande-secrets`, whose prompt asks for the passphrase of an
  encrypted volume (without it a notification points to Disks);
- `bande-automount` depends on UDisks, `bande-polkit` on `polkitd`, and `bande-greeter` on
  `greetd (>= 0.10.3-6)`, which it runs from its own `bande-greeter.service`;
- `xdg-desktop-portal-bande` depends on `xdg-desktop-portal`, provides
  `xdg-desktop-portal-backend` and serves every interface itself (no `xdg-desktop-portal-gtk`);
- `mesa-vulkan-drivers` and `libvulkan1` come with `bande-compositor`.

Every program Bande runs and every system file it reads by path is listed in
`ci/runtime-deps` with the Debian package that ships it ([ci.md](ci.md#runtime-dependencies)).

No package depends on an X server, XWayland or an X session; the `desktop` autopkgtest checks that none
comes along.

Bande ships no firewall rules and changes no system-wide kernel or resolver policy: none of its
components listens on the network until the person turns a service on (`avahi-daemon`, once the
person installs it, answers for the computer's name on the local network as Debian configures it), and a default-deny ruleset
would only break what a person turns on (file sharing, a development server). Flatpak is the sandbox and the portal asks for access; other programs run with the user's
full access, and Settings ▸ Privacy & Security says so.
