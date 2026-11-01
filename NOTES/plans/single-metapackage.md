# One metapackage: bande-desktop installs the whole Bande session

Owner decision, 2026-11-01: replace the two-package install (`bande-desktop` = the desktop,
`bande-os` = the desktop plus greeter, greetd, NetworkManager, PipeWire, zram and the APT
source) with a single `bande-desktop` that installs a complete working Bande session, following
Debian convention.

## Findings

### Nothing to migrate
The archive's `state/owners` (https://bande-linux.github.io/apt/state/owners, read 2026-11-01)
lists only `apt` packages (`bande-archive-keyring`, `bande-apt-source`) and `kernel` packages
(`bande-os-apple`, `linux-*`). Neither `bande-desktop` nor `bande-os` has ever been published,
so `bande-os` is deleted outright: no transitional package, no `Breaks`/`Replaces`.
(`bande-os-apple` is the kernel metapackage of bande-linux/kernel and keeps its name; so do the
`bande-os-*` image files of bande-linux/images.)

### How Debian's desktops split their relations
From `apt-cache show` on Debian testing, 2026-11-01:

| Package | Display manager | Network | Audio | Apps |
|---|---|---|---|---|
| `gnome-core` (metapackages) | Depends `gdm3` | Recommends `network-manager` | Depends `pipewire-audio` | core apps in Depends, browser in Recommends |
| `gnome` | via gnome-core | Depends `network-manager` | via gnome-core | Depends (office, mail), extras in Suggests |
| `task-gnome-desktop` (tasks) | - | - | - | Depends `tasksel, task-desktop`; Recommends `gnome-core, gnome` |
| `kde-plasma-desktop` | Recommends `sddm` | - | - | Depends `kde-baseapps`, `udisks2`, `upower` |
| `kde-standard` | via kde-plasma-desktop | Recommends `plasma-nm` | Depends `plasma-pa \| kmix` | apps in Recommends |
| `task-kde-desktop` | Recommends `sddm` | - | - | apps in Recommends |
| `xfce4` (section xfce) | - | - | Depends `xfce4-pulseaudio-plugin` | Recommends extras |
| `task-xfce-desktop` | Recommends `lightdm` | Recommends `network-manager-applet` | - | apps in Recommends |
| `lxqt` | Depends `sddm-theme-debian-elarun \| sddm-theme \| x-display-manager` | Recommends `nm-tray \| ... ` | Depends `pavucontrol-qt \| pavucontrol` | Recommends |
| `cinnamon-core` | Depends `slick-greeter \| lightdm \| x-display-manager` | - | Recommends `alsa-utils` | - |

Pattern:
- **Naming.** `task-*` packages are tasksel's (Section `tasks`, Depends `tasksel (= ...)`,
  `task-desktop`); they exist only for the Debian installer's software selection and pull in a
  desktop metapackage through Recommends. A desktop's own installable unit is a plain
  metapackage named after the desktop (`gnome-core`, `kde-plasma-desktop`, `xfce4`, `lxqt`,
  `cinnamon-core`), Section `metapackages`. Bande is not part of tasksel, so `bande-desktop`
  (Section `metapackages`) is the right name and the only one needed.
- **Depends** carries what the session cannot run without: the desktop's own components, the
  session plumbing, and, where the desktop needs one, a way to log in, written either as its own
  display manager (`gnome-core: gdm3`) or as "ours, or any display manager"
  (`cinnamon-core: slick-greeter | lightdm | x-display-manager`, `lxqt: ... | x-display-manager`).
  The audio stack is in Depends where the desktop's sound UI needs it (`gnome-core:
  pipewire-audio`, `kde-standard: plasma-pa | kmix`).
- **Recommends** carries what every normal install has but an unusual one may leave out:
  the preferred display manager (`kde-plasma-desktop`, `task-kde-desktop: sddm`,
  `task-xfce-desktop: lightdm`), `network-manager` (`gnome-core`), browsers and everyday apps.
  Debian Policy 7.2: Recommends declares "a strong, but not absolute, dependency ... packages
  that would be found together with this one in all but unusual installations"; apt installs
  Recommends by default.
- **Suggests** carries optional extras (`gnome-core: gnome-initial-setup`, `lxqt: calibre`).

### How display managers coexist
Read from the maintainer scripts of `gdm3` 51.1-1, `lightdm` 1.33.1-4 and `sddm` 0.21.0
(downloaded with `apt-get download`, unpacked with `dpkg-deb -e`) and of the installed
`greetd` 0.10.3-7 (`/var/lib/dpkg/info/greetd.*`):
- Each provides the virtual package `x-display-manager` (Policy's list of virtual package
  names).
- Each registers the shared debconf question `shared/default-x-display-manager`
  (`Type: select`, `Choices: ${choices}`) and its own `<pkg>/daemon_name`. The `config` script
  sets the choices to the question's owners (every installed display manager); when the owner set
  has changed it marks the question unseen and asks it at priority `high`. If the answer changed,
  it touches `/etc/X11/default-display-manager.debconf-update`.
- `postinst`: if that flag exists, writes the chosen DM's `daemon_name` to
  `/etc/X11/default-display-manager`; then points `/etc/systemd/system/display-manager.service`
  at `<basename of that file>.service`, unless the link is masked to `/dev/null`.
- `prerm` (remove): unregisters the question; if the DM being removed is the default, deletes
  `/etc/X11/default-display-manager`, asks again (priority `critical`) among the remaining owners
  and relinks `display-manager.service`.
- With the noninteractive frontend, debconf's `Select` element keeps the current value if it is
  among the choices, else takes the first choice
  (`/usr/share/perl5/Debconf/Element/Noninteractive/Select.pm`). So: on a system with no DM the
  newly installed one becomes the default; next to an existing one, the existing default stays.
- The unit's `Alias=display-manager.service` is created by `deb-systemd-helper enable` only when no
  `display-manager.service` link exists (`make_systemd_links`: `! -l $service_link`), so it never
  overrides another DM.
- **greetd participates fully** since 0.10.3-6 ("Support display-manager selection for systemd
  and sysinit systems", Closes: #1023982, #1137075): `Provides: x-display-manager`, the same
  `config`/`postinst`/`prerm` blocks as gdm3, `greetd/daemon_name = /usr/sbin/greetd`.
- **A greetd bug to report to Debian** (found by the local install test): purging greetd
  after another display manager took over again deletes that one's
  `/etc/systemd/system/display-manager.service`, because `deb-systemd-helper purge
  greetd.service` removes every link recorded for greetd's `Alias=display-manager.service`,
  although greetd's prerm had pointed it at lightdm. `/etc/X11/default-display-manager` still
  names lightdm, but the system boots without a display manager until
  `dpkg-reconfigure lightdm`. `apt remove` is fine. The install test therefore removes,
  not purges.
- **bande-greeter did its own, parallel selection**: its postinst linked
  `display-manager.service` to greetd on fresh installs and wrote
  `/etc/X11/default-display-manager` behind debconf's back, and its postrm deleted that file on
  purge. That is redundant with greetd >= 0.10.3-6 (greetd's postinst runs first, as a
  dependency) and bypasses the shared question, so `dpkg-reconfigure` would not know about it.

### Where repository-source packages sit
Debian itself has none for third-party archives; `apt` Depends `debian-archive-keyring` (the
keyring, not the source). For an archive the user already added by hand or with `extrepo`, a
hard dependency on a package that writes a second source file would be wrong, so the source
package belongs in **Recommends**: a default install keeps following the archive, and
`--no-install-recommends` or removing it lets people manage sources themselves. `bande-os`
already recommended `bande-apt-source`; it moves to `bande-desktop`. The website installs
`bande-apt-source` itself instead of writing `bande.list`, so the recommendation is already
satisfied and there is never a duplicate source.

## Design

### bande-desktop (packaging/bande-desktop, Architecture all, Section metapackages)
- **Depends**: the desktop's own components at the same version (unchanged), session plumbing
  (`xdg-desktop-portal`, `dbus-user-session`, `default-logind | logind`, `libpam-systemd`), the
  fallback fonts, and newly:
  - `bande-greeter (= ${binary:Version}) | x-display-manager`: there is always a way to log in.
    On a system without a DM apt takes the first alternative, so the Bande login window under
    greetd; next to GDM, SDDM or LightDM the dependency is already met (cinnamon-core, lxqt).
  - `pipewire-audio`: Bande's sound service, volume controls and screen recording speak PipeWire
    (gnome-core does the same). It brings `wireplumber`, `pipewire-pulse` and `pipewire-alsa`.
- **Recommends**: everything it recommended before (Bande apps, keyring, Qt portal theme,
  everyday apps, fonts, `cups`, `xdg-user-dirs`, `upower`, `udisks2`, `at-spi2-core`,
  `systemd-oomd`, `systemd-zram-generator`), and newly:
  - `bande-greeter (= ${binary:Version})`: the Bande login window comes by default even next to
    another DM (kde-plasma-desktop: `sddm`). greetd then asks which display manager to start, the
    Debian way.
  - `network-manager` (gnome-core).
  - `bande-apt-source` (see above).
  - The default apps (apps-split.md, default-apps list) stay in Recommends, unversioned once they
    move to bande-linux/apps.
- **Suggests**: `gnome-clocks` (unchanged).
- `bande-os` is deleted. Nothing depends on it; `bande-os-apple` (kernel) is unrelated.

What an install does:
- `sudo apt install bande-desktop` on a fresh Debian testing: the complete Bande system; greetd
  with the Bande login window becomes the display manager.
- On a system with GDM/SDDM/LightDM: greetd is installed and debconf asks "Default display
  manager" (gdm3 / greetd). Choosing greetd switches to the Bande login window at the next boot;
  keeping GDM leaves GDM, which lists the Bande session. Noninteractive installs keep the existing
  default. Later: `sudo dpkg-reconfigure greetd` (or `gdm3`).
- `--no-install-recommends`: the desktop, a login (bande-greeter, or the DM already there) and
  audio, without the apps, NetworkManager, zram or the APT source. The images use this and add
  their own system packages.

### DM integration (bande-greeter)
- Depends `greetd (>= 0.10.3-6)`, the first greetd that handles
  `shared/default-x-display-manager`.
- postinst: keeps only the greeter-pictures folder and a `daemon-reload` for its
  `greetd.service.d/bande.conf` drop-in; no more `display-manager.service` or
  `/etc/X11/default-display-manager` writes.
- postrm: keeps `daemon-reload` on remove and deleting `/var/lib/bande-greeter` on purge; no
  longer touches `/etc/X11/default-display-manager` (greetd's prerm owns that).
- bande-greeter does not provide `x-display-manager` or own debconf questions: greetd is the
  display manager, bande-greeter is its greeter (like `lightdm-gtk-greeter` provides
  `lightdm-greeter`).

## Test plan
- `ci/release/build-all.sh` (dpkg-buildpackage + lintian `--fail-on error,warning`) asserts the
  relations above.
- `ci/release/check-debs.sh`: bande-greeter depends on `greetd (>= 0.10.3-6)` and its scripts no
  longer write the display-manager files.
- `ci/deb/install-test/run.sh` (CI: debian:testing container; locally: mmdebstrap unshare chroot):
  1. fresh: `apt-get install --no-install-recommends bande-desktop` from the local packages
     brings bande-greeter, greetd and pipewire-audio; greetd is the default DM
     (`/etc/X11/default-display-manager`, `display-manager.service`, debconf answer);
  2. every Recommends that is not Bande's has a candidate in Debian testing;
  3. purging bande-greeter leaves greetd the display manager with its own greeter;
  4. next to another display manager: purge bande-desktop and greetd, install `lightdm`;
     `bande-desktop` without recommends does not bring greetd (lightdm meets
     `x-display-manager`); installing bande-greeter (what the recommends do) keeps lightdm the
     default and offers greetd; answering greetd to `dpkg-reconfigure greetd` (teletype frontend
     on a pty from `script`) makes it the default; removing greetd hands it back to lightdm.

## Milestones
| # | Status |
|---|---|
| M1 findings and design (this file) | done |
| M2 desktop: control, bande-greeter scripts and Depends, delete packaging/bande-os, build-all, check-debs, install test, release.yml, docs, README, plans; rebuild-desktop | done |
| M3 apt: README, fetch-run reserved names, site; website: one install command | done |
| M4 checks: build-all.sh (dpkg-buildpackage + lintian) green; bande-greeter's new Depends and scripts checked in a repacked deb; install-test/run.sh green in a rootless mmdebstrap chroot of Debian testing (arm64), fresh and next to lightdm. No Rust changed, so no desktop-gate. CI runs it at the next tag | done |
