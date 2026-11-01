# Debian archive readiness

An audit of how far the desktop repository is from an upload to the official Debian archive,
taken on 2026-10-31 at `main` 2d497b1. It covers dependencies, source packaging, licensing,
lintian and build hygiene, and ends with an ordered roadmap.

**In short.** The code is in a better state than most first uploads: it has no network access
in builds or tests, no prebuilt blobs, no bundled fonts, shaders compiled from source and
reproducible paths. Every dependency licence is DFSG-free. The distance to the archive is in
four places:

1. **Licensing paperwork**: there is no licence file anywhere in the tree, and no crate has a
   `license` field.
2. **The Rust stack**: 24 crates that Bande links are not in Debian, among them wgpu, Smithay
   and Vello. Another 52 are in Debian, but only in a different semver series.
3. **Packaging shape**: today there are 18 `cargo deb` binaries and two native debhelper
   packages. Debian wants one non-native source package, built offline with `dh-rust` against
   `librust-*-dev`.
4. **Policy friction**: the greeter takes over `greetd.service` and the display manager, a
   logind drop-in changes the power key system-wide, and the name collides with the Bande DNS
   resolver that is already in the archive.

Trixie is not a target. It is stable and takes no new packages; uploads go to unstable and
migrate to forky. Trixie could get Bande only through trixie-backports, and in trixie 102 of
the 408 runtime crates are missing and 99 are in another series, so in practice that means
forky.

## 1. Dependencies

### Method

- **Closure**: the runtime closure is the set of crates reachable from the 18 packaged crates
  through normal and build dependencies on `aarch64-unknown-linux-gnu`. It was computed
  feature-aware from `Cargo.lock` and each crate's manifest in `~/.cargo/registry`, without
  running cargo.
- **Debian side**: each crate was looked up as `librust-<crate>-dev`, including the
  semver-suffixed packages and their versioned `Provides`. The unstable data is
  `dists/unstable/main/binary-arm64/Packages`, dated 2026-10-31 02:09 UTC. The trixie data is
  the local APT lists.
- **Classes**:
  - *exact*: the locked version is packaged.
  - *newer*: a newer version in the same semver series is packaged.
  - *older*: only an older version in the same series is packaged; this builds once the pin is
    relaxed, unless Bande uses newer API.
  - *other series*: only an incompatible series is packaged.
  - *missing*: not packaged at all.

| Scope | Crates | exact | newer | older | other series | missing |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| Runtime, unstable | 408 | 271 | 4 | 57 | 52 | 24 |
| Runtime, trixie | 408 | 49 | 0 | 158 | 99 | 102 |
| Test only (dev-dependencies), unstable | 9 | 7 | 0 | 1 | 0 | 1 |

A further 75 crates in `Cargo.lock` are not used on Linux: `windows-*`, `objc2-*`, `wasm-*`,
`js-sys` and the like. Debian strips these with patches; they need no packaging.

### Direct dependencies (root `[workspace.dependencies]`)

| Crate | Locked | Unstable | Trixie |
| :--- | :--- | :--- | :--- |
| wgpu | 30.0.1 | **missing** (naga 30.0.1 and wgpu-types 30.0.1 are packaged) | none |
| smithay | 0.7.0 | **missing** (only smithay-client-toolkit) | none |
| vello_cpu, vello_gpu | 0.3.0 | **missing** (`rust-glifo` 0.4.0 has been in NEW for a week) | none |
| accesskit_unix | 0.24.1 | **missing** | none |
| atspi-common | 0.13.0 | **missing** | none |
| accesskit | 0.25.2 | 0.24.1 (other series) | none |
| parley, fontique | 0.12.0 | 0.11.1 (other series) | none |
| kdl | 6.7.1 | 4.6.0 (other series) | 4.6.0 |
| resvg | 0.49.0 | 0.48.1 (other series) | 0.45.1 |
| roxmltree | 0.21.2 | 0.20.0 (other series) | 0.20.0 |
| ruzstd | 0.9.1 | 0.8.3 (other series) | 0.7.3 |
| lzma-rust2 | 0.24.0 | 0.19.0 (other series) | none |
| pollster | 1.0.1 | 0.4.0 (other series) | 0.4.0 |
| xkbcommon | 0.8.0 | 0.9.0 (other series; Smithay 0.7 wants 0.8) | 0.8.0 |
| blocking | 1.6.2 (`=` pin) | 1.7.0 (newer, but the pin forbids it; see §5) | 1.6.1 |
| icu_calendar, icu_datetime, icu_locale_core | 2.4.0 | 2.2.x (older) | none |
| jiff | 0.2.38 | 0.2.37 (older) | 0.1.28 |
| libc | 0.2.191 | 0.2.186 (older) | 0.2.169 |
| rustix | 1.1.6 | 1.1.5 (older) | 0.38.37 |
| ash | 0.38.0 | 0.38.0 | none |
| image-compare (tests) | 0.5.0 | **missing** | none |

Exactly packaged: alacritty_terminal, ab_glyph, bzip2, flate2, image, libloading, md-5, naga,
pipewire, png, raw-window-handle, serde, serde_json, skrifa, tar, tempfile, tracing,
tracing-journald, tracing-subscriber, fluent-bundle, unic-langid, every `wayland-*` crate, zbus
and zip.

### All missing runtime crates (24)

These group into five stacks, each of which is one coordinated effort in debcargo-conf:

- **wgpu**: wgpu, wgpu-core, wgpu-hal, wgpu-core-deps-windows-linux-android, gpu-allocator.
- **Vello**: vello_common, vello_cpu, vello_gpu, vello_gpu_shaders, glifo (in NEW), wesl,
  wesl-macros, wgsl-parse, wgsl-types.
- **Parley**: parley_engine, parley_emoji; parley, parley_data and fontique also need the
  0.11 to 0.12 update.
- **AccessKit and AT-SPI**: accesskit_unix, accesskit_atspi_common, atspi, atspi-common,
  atspi-proxies. Three more need work: accesskit (Debian 0.24, Bande 0.25),
  accesskit_consumer (0.37 against 0.39) and zbus-lockstep (Debian 0.7, but atspi-common 0.13
  wants 0.5).
- **Smithay and ICU**: smithay; icu_locale_fallback and icu_locale_fallback_data.

### Runtime crates packaged only in another series (52)

Most of these are transitive. They resolve once the stacks above are packaged against what
Debian has, or through a `relax-dep-*.patch` in the parent crate.

| Pulled in by | Locked → Debian unstable |
| :--- | :--- |
| Smithay 0.7 | drm 0.14 → 0.15, input 0.9 → 0.10, xkbcommon 0.8 → 0.9, rand/rand_chacha/rand_core 0.9 → 0.8 or 0.10 |
| udev, drm-sys | io-lifetimes 1 → 2, rustix 0.38 → 1.1, linux-raw-sys 0.4 and 0.9 → 0.12 |
| wgsl-parse build (lalrpop) | lalrpop/lalrpop-util 0.22 → 0.20, ascii-canvas 4 → 3, term 1 → 0.7, petgraph 0.7 → 0.6, fixedbitset 0.5 → 0.4, bit-set 0.10 → 0.8, bit-vec 0.9 → 0.8, itertools 0.13 → 0.14, annotate-snippets 0.11 → 0.12 |
| accesskit_atspi_common | phf, phf_generator, phf_macros, phf_shared 0.14 → 0.11 |
| Parley, wgpu, naga | hashbrown 0.17 → 0.16, fearless_simd 0.7 → 1.0 |
| resvg | usvg 0.49 → 0.48, svgtypes 0.16 → 0.15 |
| kdl, zbus, toml | winnow 1.0 → 0.7 |
| others | getrandom 0.3 → 0.4, miniz_oxide 0.8 → 0.9, weezl 0.1 → 0.2, quick-xml 0.41 → 0.38 (wayland-scanner), system-deps 7 → 9 (pipewire-sys), shlex 2 → 1 (bindgen, cc), convert_case 0.10 → 0.11, litrs 1 → 0.5, approx 0.4 → 0.5 (cgmath), pollster 1 → 0.4, roxmltree, ruzstd, lzma-rust2, kdl, accesskit, parley, fontique |

### Runtime crates packaged only at an older patch or minor version (57)

These build once the exact pins are relaxed, or after a routine Debian update. Examples: the
whole ICU 2.4 set (Debian has 2.2), libc, rustix 1.1, jiff, calloop, cc, toml and toml_edit,
zerocopy 0.8.63 (Debian has 0.8.26), zvariant, naga-types, smallvec, memmap2, ordered-float and
xattr.

### What the Debian Rust team expects

From the team's book (rust-team.pages.debian.net/book):

- **Libraries**: each crates.io crate is its own source package `rust-<crate>`, generated by
  `debcargo` from `debcargo-conf` on salsa, and ships `librust-<crate>-dev`. Libraries no
  longer need an ITP, but the team files one for large stacks.
- **Applications**: binary crates need an ITP ("team policy"). A workspace that is not on
  crates.io, such as Bande, is one source package with one binary package per program, built
  with **dh-rust**; dh-cargo has no workspace support. dh-rust 0.1.4, dh-cargo 34 and debcargo
  2.8.4 are in unstable.
- **No network**: builds use only packaged crate sources through a directory registry.
  `Cargo.lock` is not used.
- **Exact pins**: these are relaxed by patching `Cargo.toml`, as `relax-deps.patch` or one
  `relax-dep-<crate>.patch` each. Changing a library's dependencies is announced on
  debian-rust@lists.debian.org first.
- **Semver-suffixed packages** (`rust-foo-0.3`) are made "only when absolutely necessary".
  Upgrading, downgrading or disabling an optional dependency comes first.
- **Vendoring**: the team has no rule permitting it. Debian Policy §4.13 discourages
  convenience copies. Vendoring 24 crates, among them wgpu and Smithay, into the Bande
  upload would be contested in NEW and is not recommended.
- **Toolchain**: upstream should declare and keep a conservative MSRV and use no nightly
  features.
- **Contact**: debian-rust@lists.debian.org, `#debian-rust` on OFTC, or
  `#debian-rust:matrix.debian.social`. Contributors join via a merge request on salsa.

No Smithay-based compositor (niri, cosmic-comp) is in Debian yet, so Bande would pioneer the
Smithay stack. Alacritty, which links the same terminal crate, is the nearest precedent for a
Rust GUI application.

## 2. Source packaging

### Today

| Piece | How it is built | Debian expects |
| :--- | :--- | :--- |
| 18 arm64 packages | `cargo deb --profile dist --no-build`, from each crate's `[package.metadata.deb]`; maintainer scripts copied by hand into `crates/*/debian/`; versions `0.0.0+git<date>.<count>.<sha>` | one source package, `debian/control` listing every binary package, `dh $@ --buildsystem rust` |
| `bande-desktop` meta-package | debhelper, `3.0 (native)`, version injected through `DEB_VERSION_OVERRIDE` | a binary package of the same source; the version comes from `debian/changelog` |
| `bande-archive-keyring`, `bande-apt-source(-testing)` | debhelper, `3.0 (native)` | not uploadable: third-party APT configuration. Keep it out of the Debian source |

### Gaps

- **No upstream release.** There are no tags and no tarballs. Debian needs a versioned
  upstream release, such as a `v0.1.0` tag on GitHub, and a `debian/watch` (format 4 or 5)
  that finds it.
- **Source format.** The source must be `3.0 (quilt)` with an orig tarball, not
  `3.0 (native)`. The `debian/` directory lives on salsa (or in a packaging branch), not in the
  upstream tarball. `packaging/` and `crates/*/debian/` inside the tarball do no harm, but
  become dead weight once `debian/` exists.
- **Build system.** dh-rust builds the workspace with Debian's RUSTFLAGS and the release
  profile. The `dist` profile's `strip = true` and fat LTO stop mattering: `dh_strip` makes
  `-dbgsym` packages. `${cargo:Static-Built-Using}` has to be recorded on every arch-dependent
  package.
- **Maintainer scripts.** The hand-copied `deb-systemd-helper --user` blocks in
  `crates/*/debian/postinst` and `postrm` are what `dh_installsystemduser` generates. They
  disappear in a debhelper build.
- **`debian/copyright`.** It must be DEP-5 and cover the whole source, not just `Files: *`.
  Today two copies exist, for the meta and APT packages, and `cargo deb` writes one per package
  that says "Copyright information missing". See §3.
- **Leftovers.** `bande-shell` conflicts with and replaces `bande-bar`,
  `bande-desktop-surface`, `bande-launcher`, `bande-notifications` and
  `bande-controlcenter`. These were never in Debian, so the fields go.
- **Undetected dlopen dependencies.** The toolkit loads `libvulkan.so.1` (wgpu) and
  fontconfig (fontique's `fontconfig-dlopen`) at runtime. Every toolkit app (Files, Terminal,
  Settings, Monitor, shell, lock, greeter, screenshot, file chooser) therefore needs explicit
  `libvulkan1, libfontconfig1` Depends. Today only `bande-compositor` declares
  `libvulkan1`. `bande-lock` already declares `libpam0g` for its dlopen of libpam.
- **Size.** The full set installs 238 MiB. Each of the ten toolkit binaries carries its own
  static copy of the toolkit, Vello and wgpu, at about 19 MiB each. This blocks nothing, but
  ftpmasters and the release team will notice it. A later multi-call binary would cut it.

### Proposed `debian/control` shape

- **Source**: `bande-desktop` (or the renamed name, see §6). `Section: x11`,
  `Build-Depends: debhelper-compat (= 13), dh-sequence-rust, rustc (>= <MSRV>), cargo`, the
  `librust-*-dev` list, pkgconf, `libinput-dev`, `libseat-dev`, `libgbm-dev`,
  `libxkbcommon-dev`, `libudev-dev`, `libpipewire-0.3-dev`, libclang-dev, and for tests
  dbus-daemon, mesa-vulkan-drivers, fonts-dejavu-core, fonts-inter and fonts-noto-cjk.
  `Rules-Requires-Root: no`, `Standards-Version: 4.7.2`.
- **Binary packages**: the 18 programs as today (`Architecture: linux-any` or an explicit
  list, see §5), plus the `Architecture: all` meta-package.

## 3. Licensing

| Item | State | Needed |
| :--- | :--- | :--- |
| Project licence | done: `LICENSE-MIT` and `LICENSE-APACHE` at the root, Apache-2.0 or MIT | Add `LICENSE-MIT` and `LICENSE-APACHE` at the root. Without them ftpmasters reject the upload: the licence must be stated by upstream, not by packaging |
| Crate `license` fields | done: `license = "MIT OR Apache-2.0"` in `[workspace.package]`, inherited by all 60 crates | `license = "MIT OR Apache-2.0"` in `[workspace.package]` and `license.workspace = true` in every crate |
| Copyright holder | everything is committed by Mikael Hug (agents commit under that name) | a `Copyright: 2026 Mikael Hug` line in each licence file is enough. Every contribution is the owner's, so there are no third-party holders to list |
| Icon themes (`data/icons`, 2.7 MB of SVG, 8 colour variants) | generated by the committed Python in `data/icons/src/` (`generate.py`, `apps.py`, …) | DFSG-fine: the generator is the preferred form. Optionally regenerate at build time; otherwise state the generator in `debian/copyright` |
| Wallpapers (`data/wallpapers/*.svg`) | hand-written SVG, 16 to 20 lines each | fine; SVG is the source |
| Sound theme (`data/sounds/Bande/stereo/*.wav`, 8 files, described as "original wooden sounds") | WAV only; no generator or recording notes in the tree | record their provenance: a statement of authorship, plus the synthesis script if they were synthesised. Without it a WAV of unclear origin is a typical NEW rejection |
| Emoji data (`crates/shell/launcher/data/emoji.tsv`, compiled in) | derived from Unicode `emoji-test.txt` and CLDR annotations by the committed `emoji.py` | its own `Files:` stanza under **Unicode-3.0**. Ideally regenerate at build time from Debian's `unicode-data` and `unicode-cldr-core` |
| Other embedded SVG (`process-stop-symbolic.svg`, app icons in `crates/apps/*/data`) | original | covered by `Files: *` |
| Fonts | none bundled; Inter, Manrope, JetBrains Mono, DejaVu and Noto come from Debian packages | nothing to do |
| Test goldens (116 PNG files in `crates/*/tests`) | produced by the tests themselves | covered by `Files: *`. They make the tarball larger; Files-Excluded is not needed |
| CI and VM files (`ci/vm/kernel.config`, `ci/hil`, the keyring `.pgp`) | not installed | covered by `Files: *`. The archive key needs no stanza, being data |
| Dependency licences (408 runtime crates) | MIT and/or Apache-2.0 (most), Unicode-3.0 (ICU4X, 35 crates), BSD-2/3-Clause, Zlib, ISC, Unlicense OR MIT, 0BSD, `bzip2-1.0.6` (libbz2-rs-sys), `Apache-2.0 OR GPL-2.0-only` (self_cell), `Apache-2.0 WITH LLVM-exception` | all DFSG-free. With system crates their copyright is carried by their own packages; Bande's `debian/copyright` lists only what is in its own tarball |

## 4. Lintian and installed files

### Lintian on the current binaries

The 18 installed `0.0.0+git20261030.879.9b9e2c5` packages were repacked into `.deb` files from
`dpkg` (no build) and checked with lintian 2.122.0 and
`--display-info --pedantic --display-experimental`. The repacking produced 198
`non-standard-dir-perm` warnings of its own, from the umask, which are left out below.

| Tag | Count | Cause | Fix |
| :--- | ---: | :--- | :--- |
| E: `no-changelog` | 18 | `cargo deb` writes no `changelog.Debian.gz` | goes away with a debhelper source package |
| W: `copyright-without-copyright-notice` | 18 | `cargo deb` copyright says "Copyright information missing" because there is no `license` field | §3 |
| W: `no-manual-page` | 17 | no man page for any program in `/usr/bin`; no program answers `--help` | man pages (scdoc or Markdown → `man`) for the 17 programs |
| W: `virtual-package-depends-without-real-package-depends` | 7 | `dbus-session-bus \| default-dbus-session-bus` puts the virtual first | `default-dbus-session-bus \| dbus-session-bus` |
| I: `dbus-session-service-wrong-name` | 3 | `org.bande.Notify.service`, `org.bande.Power.service` and `org.bande.Tray.service` activate `org.freedesktop.Notifications`, `org.freedesktop.ScreenSaver` and `org.kde.StatusNotifierWatcher` | name the files after the bus name they own, or override with the reason (several daemons may provide the name) |
| I: `hardening-no-fortify-functions` | 18 | Rust binaries do not use glibc's fortified functions | none needed |
| X: `spelling-error-in-binary` | 22 | strings such as "mor", "alha", "aquire", "occured" and "wasnt", mostly from statically linked crates | override where the string is not ours |

CI runs lintian (`--fail-on error,warning`) only on the meta and APT packages, never on the 18
`cargo deb` outputs. That is why `no-changelog` has never shown up.

### Likely findings once a source package exists

Expect `debian-watch-file-is-missing` and `upstream-metadata-file-is-missing` until those files
exist, and `no-manual-page` until the man pages do. `source-is-missing` should not appear: the
SVGs are text, and the WAVs need only the provenance note.

### Installed files

| Area | State | Verdict |
| :--- | :--- | :--- |
| Desktop files (4 apps, `org.bande.*.desktop`, plus `bande.desktop` in `wayland-sessions`) | keys, categories and actions are valid by inspection; lintian's desktop-entry checks pass. desktop-file-validate is not installed here | add `desktop-file-validate` to the package test |
| AppStream metainfo | none | add `org.bande.{Files,Terminal,Settings,ActivityMonitor}.metainfo.xml` in `/usr/share/metainfo`, or the apps are invisible in GNOME Software and Discover. The reverse-DNS IDs should use a domain the project controls (§6) |
| systemd user units (`/usr/lib/systemd/user`) | `systemd-analyze --user verify` reports nothing for Bande's units; enabled through `deb-systemd-helper --user` blocks copied by hand | correct location; let `dh_installsystemduser` generate the blocks |
| D-Bus activation files (`/usr/share/dbus-1/services`) | correct location | see the lintian info above |
| Portal (`/usr/share/xdg-desktop-portal/portals/bande.portal`, `bande-portals.conf`, backend in `/usr/libexec`) | correct locations | fine |
| polkit (`/usr/share/polkit-1/rules.d/50-bande-settings.rules`) | grants local, active `sudo` members password-free time zone, NTP, time, locale and hostname changes, which is what gnome-control-center does in Debian. Software Update uses offline updates (`trigger-offline-update`, `allow_active=yes` upstream), so there is no PackageKit grant | location fine |
| PAM (`/etc/pam.d/bande-lock`, `@include common-auth` and `common-account`) | a conffile, correct | fine |
| logind (`/usr/lib/systemd/logind.conf.d/bande-power.conf`, `HandlePowerKey=lock`) | **changes the power key system-wide** for every session and every desktop on the machine as soon as `bande-power` is installed | drop the drop-in and take a `handle-power-key` inhibitor lock in `bande-power`, as GNOME does |
| Memory policy (`/usr/lib/systemd/zram-generator.conf.d/50-bande.conf`, `/usr/lib/systemd/user/app.slice.d/50-bande-oomd.conf`, and `OOMScoreAdjust=100` drop-ins for the user units `dbus`, `pipewire`, `pipewire-pulse`, `wireplumber` and `xdg-desktop-portal`) | vendor drop-ins, the mechanism systemd and zram-generator document for this. No other package's file is changed, and admins override them in `/etc`. Inert without `systemd-zram-generator` and `systemd-oomd` (Recommends) | fine; name each drop-in in `README.Debian`. Needs a kernel with `CONFIG_PSI=y` and `CONFIG_ZRAM` (Debian's has both; the Bande Mac kernel has them from `linux` `bande-hw/integration` on) |
| greetd (`/usr/lib/systemd/system/greetd.service.d/bande.conf` replaces greetd's `ExecStart` with `--config /usr/share/bande-greeter/greetd-config.toml`; postinst points `display-manager.service` at greetd and writes `/etc/X11/default-display-manager`) | **overrides another package's service and the admin's `/etc/greetd/config.toml`**, and picks the display manager behind debconf's back | rework: let the admin opt in, either through the `shared/default-x-display-manager` debconf question as gdm3, lightdm and sddm do, or by documenting a one-line `/etc/greetd/config.toml` change; coordinate with the greetd maintainer. Policy §10.7.4 forbids changing other packages' configuration |
| `/var/lib/bande-greeter/pictures` (mode 3733, group `_greetd`) | created in postinst, removed on purge | fine; it relies on greetd creating `_greetd` first, which `Depends: greetd` ensures |
| udev rules | none | nothing to do |
| `/usr/share/applications/bande-mimeapps.list` | the XDG per-desktop defaults | fine |
| Greeter user list | read straight from `/etc/passwd` | misses LDAP/SSSD users; reviewers may ask for NSS (`getpwent`) or AccountsService |

## 5. Build hygiene

| Topic | State | Needed |
| :--- | :--- | :--- |
| Network during build or tests | none: no HTTP client crates, no sockets beyond Unix; URLs in tests are plain strings | nothing |
| Prebuilt artefacts | none: SPIR-V is compiled from `compose.wgsl` by `build.rs` through naga; no `.so`, `.a` or fonts | nothing |
| Reproducibility | CI passes `--remap-path-prefix`; `env!("CARGO_MANIFEST_DIR")` only appears in the test crates `bande-test-bus` and `bande-golden`; no timestamps; `CARGO_PKG_VERSION` is the only embedded build value | dh-rust sets the remapping itself. Verify with `reprotest` once a source package exists |
| Toolchain | `rust-toolchain.toml` pins **1.99.1**; unstable has rustc **1.97.1**, trixie **1.85.1**. The code uses let-chains (92 sites, stable since 1.88), and the highest dependency MSRV is kdl 6.7.1's **1.95** | trixie's rustc cannot build Bande. Declare `rust-version` in `[workspace.package]` and add a CI job that builds and tests with Debian unstable's `rustc` and `cargo` (a `debian:sid` container), so the code never needs a toolchain newer than Debian ships |
| Exact pins (`=x.y.z` for every external dependency) | a team rule, for reproducible upstream builds | Debian replaces all of them. Switching upstream to caret requirements and keeping `Cargo.lock` for reproducibility removes most `relax-deps.patch` work. This is an owner decision against the current team rule |
| `blocking = "=1.6.2"` | pinned because 1.7.0's idle pool threads wake twice a second forever (smol-rs/blocking#81 and #83, open; see `docs/architecture/performance.md`). Unstable has 1.7.0 | under Debian the pin is relaxed and the wake-ups come back. Options: get the upstream fix released and packaged; carry the fix as a patch in `rust-blocking` through the Rust team; or stop needing `blocking`, which zbus, the portal, polkit, screenshot and bluetooth pull in |
| Tests needing a session bus | `bande-test-bus` starts its own `dbus-daemon` with a private config; settings, monitor, compositor and clipboard tests use it | Build-Depends on dbus-daemon; works under sbuild |
| Tests needing a display or GPU | headless compositor and offscreen Vulkan on lavapipe | Build-Depends on mesa-vulkan-drivers. Goldens compare at SSIM ≥ 0.995, so a Mesa update on the buildds can break the build (FTBFS). They need a switch (an environment variable or feature) that turns them into smoke tests under `dh_auto_test` |
| Tests needing a system (PipeWire, polkitd, greetd, the VM) | 33 tests are `#[ignore]` with the reason given | fine for buildds. Candidates for autopkgtest with `Restrictions: isolation-machine` |
| `/tmp` and `$HOME` | tests use `tempfile` and `CARGO_TARGET_TMPDIR`; the literal `/tmp/...` paths are data in assertions or `#[ignore]`d guest tests | fine. The one non-guest test that needs `XDG_RUNTIME_DIR` (`screen_reader.rs`) sets its own. Still confirm under sbuild with `HOME=/nonexistent` and no `XDG_RUNTIME_DIR` |
| Fonts in tests | need fonts-dejavu-core, fonts-inter and fonts-noto-cjk | Build-Depends |
| Architectures | built and tested on arm64 only. `build.rs` writes SPIR-V little-endian, which is correct for SPIR-V; the big-endian and 32-bit (armhf, i386) paths are untested | start with `Architecture: amd64 arm64` (plus riscv64, ppc64el and loong64 once green) rather than `any`, to avoid FTBFS on s390x and armel blocking migration |

## 6. Naming

Debian already has the **Bande DNS resolver** (source `bande`; binaries `bande`,
`bande-anchor`, `bande-host`, `libbande8`, `libbande-dev`, `python3-bande`; programs
`bande-checkconf` and `bande-control`; config in `/etc/bande`). There is no file conflict
today: the desktop uses `/usr/share/bande/` and the resolver does not. But binary packages
such as `bande-config`, `bande-power` and `bande-sound` sit in the resolver's namespace,
and `bande-config` next to `bande-checkconf` is the kind of confusion ftpmasters reject in
NEW.

The `org.bande.*` D-Bus and application IDs also imply ownership of `bande.org`. AppStream
and Flathub expect a domain the project controls, for example `io.github.bande_linux.*`.

Deciding this before the ITP is cheap; after the upload it is a transition. One option keeps
the brand and makes Debian names unambiguous: source `bande-desktop`, binaries
`bande-desktop-<part>`. The other is a rename.

## Roadmap

The effort figures are working days for one developer. Elapsed time for the Debian stack work
is governed by NEW-queue review, typically days to weeks per new source package.

### Now: upstream changes in this repository (about 8 to 12 days)

1. **Licence files and fields** (0.5 d): `LICENSE-MIT` and `LICENSE-APACHE` at the root;
   `license = "MIT OR Apache-2.0"` in `[workspace.package]` and `license.workspace = true` in
   every crate. This also fixes the cargo-deb copyright files today.
2. **Asset provenance** (0.5 d): an authorship statement, and the generator if there is one,
   for the eight WAVs; a note that `emoji.tsv` is Unicode-3.0 data generated by `emoji.py`.
3. **Debian-toolchain CI job** (1 d): build and test with unstable's rustc 1.97 in a
   `debian:sid` container; declare `rust-version`. Stop raising the pin above what unstable
   ships.
4. **Policy fixes in the session** (3 to 4 d): replace the logind drop-in with a power-key
   inhibitor in `bande-power`; make the greeter opt-in rather than taking over
   `greetd.service` and `display-manager.service`. (The password-free PackageKit update rule
   is gone: Software Update installs offline.)
5. **Missing runtime Depends** (0.5 d): `libvulkan1, libfontconfig1` on every toolkit app;
   `default-dbus-session-bus | dbus-session-bus` order; drop the Conflicts and Replaces on
   never-shipped packages.
6. **Man pages** (1.5 d) for the 17 programs in `/usr/bin`, and **AppStream metainfo**
   (0.5 d) for the four apps.
7. **Golden switch** (1 d): an environment variable under which golden comparisons become
   smoke tests, so Mesa updates on buildds cannot make the build fail.
8. **First tagged release** `v0.1.0` (0.5 d), so there is a tarball to package.
9. **Owner decisions** (no code): caret requirements instead of `=` pins; the Debian naming
   (§6); the reverse-DNS ID domain.

### Before the ITP (about 4 to 6 weeks of work, 2 to 4 months elapsed)

10. **Join the Debian Rust team** on salsa, introduce the plan on debian-rust@, and find a
    sponsor. The owner is not a Debian Developer; the Rust team or debian-mentors can sponsor.
11. **Package the five missing stacks** in debcargo-conf (about 25 new source packages: wgpu,
    Smithay, Vello with WESL, Parley 0.12 with parley_engine and parley_emoji, AccessKit with
    AT-SPI, and the ICU locale fallback). Each one goes through NEW. Check WNPP and NEW first;
    someone is already packaging the Vello stack, since `rust-glifo` is in NEW.
12. **Update what is behind** (about 15 packages: accesskit, accesskit_consumer, fontique,
    parley, parley_data, kdl 4 → 6, resvg, usvg, svgtypes, roxmltree, ruzstd, lzma-rust2,
    pollster, zbus-lockstep, ICU 2.2 → 2.4). Where a major update would break other Debian
    users, use semver-suffixed packages or patch Bande down instead.
13. **Align Smithay's pins** (drm 0.14, input 0.9, xkbcommon 0.8, rand 0.9) with Debian's
    series: patches in `rust-smithay`, or wait for a Smithay release that matches.
14. **Fix the `blocking` regression** in a way that survives Debian: an upstream release, a
    Debian patch, or dropping the dependency.
15. **Write `debian/`** (3 to 5 d), on salsa:
    - a single `3.0 (quilt)` source package with dh-rust and 19 binary packages;
    - `debian/copyright` in DEP-5, with stanzas for `Files: *`, the sounds, `emoji.tsv` and the
      icon generator;
    - `debian/watch`, `debian/upstream/metadata`, `dh_installsystemduser`;
    - `Static-Built-Using`;
    - autopkgtests (`desktop-file-validate`, `appstreamcli validate`, an isolation-machine
      session smoke test).
16. **File the ITP** against wnpp, with the description, licence and URL.

### Before the upload

17. All Build-Depends in unstable; a clean `sbuild` against unstable with no network,
    `HOME=/nonexistent` and no `XDG_RUNTIME_DIR`; `dh_auto_test` green on amd64 and arm64.
18. `lintian --pedantic` clean on source and binaries, with any overrides explained;
    `piuparts` install, upgrade and purge clean; `reprotest` reproducible; autopkgtest green.
19. Fix the architecture list (start with amd64 and arm64).
20. Leave `packaging/bande-apt-source` out of the Debian source package; the GitHub APT
    repository keeps using it.
21. Upload to unstable through the sponsor and answer the NEW review. After migration to forky,
    consider trixie-backports only if the 200-odd crates trixie lacks or has at the wrong
    series get backported too, which is unlikely.
