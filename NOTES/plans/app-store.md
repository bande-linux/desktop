# Plan: an app store ("Software")

Gap (owner): "we have no app store, should we build one?"

## Summary

**Yes, build our own: a small "Software" app on the Unbound toolkit.** It reads AppStream data
(Debian DEP-11 and Flathub) and installs debs through PackageKit and Flatpaks through the
`flatpak` CLI. It shares one package client with Software Update and the .deb installer.
Do not make GNOME Software or Discover the default. Fix the foundations first: PackageKit,
appstream and flatpak are not even installed today. The MVP is milestones 1 to 9 (about 9 to 15
agent hours). Screenshots, featured apps and Flathub setup follow in 10 to 13.

## Findings

**What users get today: only the apt CLI, and less than it seems.**
- On the mini, `packagekit`, `appstream`, `apt-config-icons`, `flatpak` and `gnome-software` are
  all *not installed*. All are in trixie: packagekit 1.3.1, appstream 1.0.5, flatpak 1.16.6,
  gnome-software 48.3.
- `unbound-settings` only *Recommends* `packagekit`, so an install without recommends has a
  Software Update pane with no service behind it. This is probably the "software update doesn't
  work" gap.
- With `appstream` missing, APT never downloads DEP-11 data, so no catalogue exists on disk.
  `/var/lib/apt/lists` holds only `Packages` and `Translation` files, and there is no
  `/usr/share/swcatalog` or `/var/lib/swcatalog`.
- Only `libappstream5` is installed. `bubblewrap` and `xdg-desktop-portal` are installed.
- Our own apps ship no AppStream metainfo, so every store would hide them
  (`docs/debian-readiness.md`, metainfo row).

**What is decided already.**
- `docs/roadmap.md` puts "an app store" under **Later**. The B3 row has "Software Update via
  PackageKit".
- `docs/parity.md` uses a hybrid plan. Finder, Settings, Terminal, Activity Monitor and
  Spotlight are ours. Firefox and the GNOME apps are preinstalled third-party apps. A store is
  not mentioned. `docs/design/catalogue.md` does not mention one either.
- There are no decision records about packaging front ends.
- No sibling plans (`software-update.md`, `deb-installer.md`) existed when this was written.
  This plan proposes the shared client below. The plan that lands first creates it, and the
  others extend it.

**Code that exists.**
- `crates/apps/settings/src/software.rs` already drives PackageKit over zbus: transaction
  runner, signals, progress, restart detection, and RefreshCache/GetUpdates/GetDetails/
  UpdatePackages.
- `crates/apps/settings/tests/updates.rs` has a fake PackageKit on the test bus.
- The polkit agent and the `50-unbound-settings.rules` grant for
  `org.freedesktop.packagekit.system-update` exist (maturity gaps #2).
- Privacy & Security already reads and writes the portal PermissionStore
  (`apps/settings/src/permissions.rs`, `testing/test-bus/src/permission_store.rs`).
- Settings opens a pane given as an argument (`PaneId::from_keyword`), so the store can deep-link
  to Software Update.
- Reusable pieces:
  - `unbound-search::matcher` (ranked fuzzy matching)
  - `unbound-xdg` (desktop entries, launching, `data_dirs`)
  - Files' `grid.rs` icon grid (crate-private today)
  - `roxmltree`, `flate2`, `kdl`, `image` (png/jpeg) and `zbus`, all in the workspace
- What the workspace lacks: glib, an HTTP client and a YAML parser.

**A pure-Wayland trap.** Debian's DEP-11 catalogue contains many X11-only programs (xterm,
Motif and Xaw apps, old SDL1, some Electron). On Unbound they will not open, because there is
no XWayland. Flathub apps whose `sockets` list `x11` without `wayland` or `fallback-x11` fail the
same way. GNOME Software and Discover know nothing about this and would offer them. Our store
can mark or hide them:
- Flatpak: the metadata `[Context] sockets`.
- Debs: a `Depends` heuristic from the APT `Packages` index. Treat a package as X11-only if it
  depends on libx11/libxt/libxaw/libmotif and on no Wayland-capable toolkit (gtk3/4, qt5/6 gui,
  sdl2/3, libwayland-client).

**Flatpak in the session.** `crates/platform/compositor/data/unbound-session` imports
`XDG_DATA_DIRS` from the greetd environment into systemd. Flatpak's exports
(`/var/lib/flatpak/exports/share`, `~/.local/share/flatpak/exports/share`) reach that
environment only through `/etc/profile.d/flatpak.sh` or the systemd environment generator. The
import may overwrite the generator's value. If so, installed Flatpaks will not appear in the
launcher or Dock. This must be checked on the mini.

**Alternatives, measured on the mini** (`apt-get install -s --no-install-recommends`):
- `gnome-software` + `gnome-software-plugin-flatpak`: only **9 new packages** (GTK4 and
  libadwaita are already there for Loupe and Papers), about 8 MB.
  - Works on Wayland.
  - Takes our accent and dark mode through the portal.
  - Looks Adwaita, not Datum.
  - Has its own update engine (`download-updates` and `allow-updates` gsettings keys, both true
    by default) that would duplicate Software Update.
  - Registers itself for `.deb` and `appstream:` links.
- `plasma-discover` + its flatpak backend: **168 new packages** (Qt6/KDE Frameworks/Kirigami).
  No.
- Bazaar (a GTK4 store for Flathub only): sid/forky only, not trixie. Souk is abandoned.

**Upstream facts.**
- Flathub has about 3,757 aarch64 builds against 4,307 x86_64 (about 87 %) [pkgdex]. arm64 is
  1.3 % of Flathub downloads [linuxiac 2025].
- Flathub subsets `verified`, `floss` and `verified_floss` can be set with
  `flatpak remote-add/remote-modify --subset=` [Flathub docs].
- Debian does not enable Flathub or install flatpak by default [GNOME Software #2259].
- PackageKit 2.0 is planned for H2 2026 with a **complete API break**, D-Bus clients included
  [linuxiac Aug 2026, Klumpp]. trixie stays on 1.3.x, but a future Debian will move.
- ODRS reviews (`odrs.gnome.org/1.0/reviews/api/...`) have no official docs. They send a user
  hash, and fetching ratings for one app needs the whole ratings list [flathub/website#50].
- Rust bindings for libflatpak exist (`libflatpak` 0.7, gtk-rs, gir) and pull in glib/gio.

## Options

| Option | User gets | Cost | Trade-offs |
|---|---|---|---|
| A. Nothing; apt CLI and docs | nothing new | 0 | the first-hour gap stays; Flatpak only via the terminal |
| B. GNOME Software as the default (Recommends + gschema override `allow-updates=false`, `download-updates=false`) | a mature store with debs and Flathub, ratings and screenshots | about 2 milestones (packaging, overrides, mini check) | Adwaita look; offers X11-only apps; a second update UI unless disabled; claims `.deb` and `appstream:` links in place of our installer; will be replaced later, which churns users |
| C. Discover | same as B | same as B | 168 packages, Qt look; no |
| D. Our own store, debs only | browse, search and install Debian apps in Datum | about 9 milestones | no Flathub, so on arm64 users miss many popular apps (Spotify-style vendor apps, newer versions) |
| **E. Our own store, debs + Flatpak** | D plus Flathub, permissions shown before install, one update list | about 13 milestones | the most work and upkeep (metadata quirks, two backends); needs an HTTP client and a YAML parser |

Backend choices inside E:

| Concern | Choice | Why |
|---|---|---|
| Debs | PackageKit over zbus (we already have it) | polkit and transaction queueing come free; the 2.0 break is contained in one crate |
| Flatpak | `flatpak` CLI (`--noninteractive -y`, `--columns=`) | no glib in the workspace; the CLI is stable and uses the system helper and polkit for system installs; libflatpak (glib) is the fallback if progress parsing proves too weak |
| Metadata | parse AppStream ourselves: XML (Flathub, `swcatalog/xml`) with `roxmltree`, DEP-11 YAML with one new YAML crate | libappstream through FFI would bring glib; `appstreamcli` output is meant for people, not programs |
| Screenshots, featured, ratings | HTTP with an on-disk cache in `$XDG_CACHE_HOME/unbound/software`; featured apps from a curated KDL file we ship; no ODRS in the MVP | network access is a new capability; ratings are low value and send a hash to GNOME |

## Recommendation

1. **Build E in phases**, and pull it forward from "Later". Most of the work is shared:
   - The package client, PackageKit and metainfo work is needed anyway by Software Update and
     the .deb installer.
   - Only the catalogue parser and the app UI belong to the store alone.
2. **No GNOME Software by default.** Once milestone 1 lands, users who want it can
   `apt install gnome-software`; mention it in the docs. If the owner wants a store this week,
   B is the stopgap. It needs the `allow-updates=false` override and must not take the `.deb`
   MIME type.
3. **Flathub:**
   - Ship its remote in a small `unbound-flathub` package that `unbound-desktop` Recommends.
   - Leave it **off until the user turns it on** in Software. One switch, explained in place:
     "Apps from Flathub are built by their developers, sandboxed, and updated separately from
     Debian."
   - When an app is in both sources, prefer Debian and offer Flathub in a source pop-up on the
     app page.
   - Owner decision: on by default, and which subset.
4. **One update list.** Software Update in Settings shows deb and Flatpak updates together.
   Software's Installed view marks apps with "Update" and runs the same client. There is no
   separate Updates tab.
5. **Sandbox UI:**
   - Show Flatpak permissions read-only on the app page before installing, with plain phrases:
     "Files: your home folder", "Network", "Devices", "Needs X11: won't open on Unbound".
   - Editing overrides (Flatseal-style, `~/.local/share/flatpak/overrides/<id>` keyfiles) comes
     later, in Privacy & Security, next to the portal permissions it already has.
6. Hide X11-only apps by default. A "Show apps that need X11" choice in the View menu shows them
   with a warning.

## Implementation plan

Each milestone is one branch, sized for about 1 to 2 agent hours, and must pass the full gate.
"Mini" means checked live on the M2 mini, as the verification rule requires.

1. **Foundations in packaging and session.** Skip any part the software-update plan already did.
   - In `unbound-desktop`'s Recommends (`packaging/unbound-desktop`), add `packagekit`,
     `appstream` and `apt-config-icons-hidpi` (it pulls in `apt-config-icons`).
   - Install them on the mini. Record where DEP-11 YAML and icons land after `apt update`
     (lists dir, `swcatalog/icons/...`).
   - Add AppStream metainfo for Files, Terminal, Settings and Activity Monitor
     (`crates/apps/*/data/org.unbound.*.metainfo.xml`, in the deb assets).
   - Test: `appstreamcli validate --no-net` over every metainfo in a packaging test script that
     CI already runs, if one exists. Otherwise run it on the mini and record the result in the
     branch.
2. **Shared client `crates/libs/packages` (`unbound-packages`).**
   - Move the transaction runner, `Package` and the signal handling out of
     `apps/settings/src/software.rs`.
   - Add `resolve(names)`, `installed()` (GetPackages with the installed filter),
     `install(ids)`, `remove(ids)`, `install_files(paths)`, and progress through a callback.
     Settings uses the crate.
   - Move the fake PackageKit from `apps/settings/tests/updates.rs` into
     `crates/testing/test-bus/src/packagekit.rs`.
   - Tests: resolve picks the arch-native id over others; install reports progress and
     restart; ErrorCode becomes a failure. All on the hermetic bus. The existing Settings
     update tests keep passing.
3. **Catalogue model and AppStream XML: `crates/libs/catalog` (`unbound-catalog`).**
   - `Component`: id, kind, name, summary, description (paragraphs and lists), developer,
     license, categories, keywords, icons, screenshots, launchable, releases.
   - `Bundle` is `Deb(package)` or `Flatpak(ref)`.
   - Parse gzip XML (`roxmltree`, `flate2`) from `/usr/share/swcatalog/xml` and
     `/var/lib/flatpak/appstream/<remote>/<arch>/active/appstream.xml.gz`, picking localised
     fields for `LANG`.
   - Tests: on a trimmed real Flathub fixture, the locale falls back from `sv_SE` to `sv` to
     the default; description markup becomes blocks; non-applications are dropped.
4. **DEP-11 and icons.**
   - Add one YAML dependency (owner decision; `saphyr` or `serde_yaml_ng`).
   - Parse `*_dep11_Components-<arch>.yml.gz` from the paths found in milestone 1.
   - Resolve `cached` icons through the icon tarball directories and `stock` icons through
     the icon theme.
   - Merge the same component id across sources into one app with several `Bundle`s.
   - Tests: a trimmed real trixie Components fixture parses to the expected apps; an app in
     both sources merges; a missing icon falls back to `stock`.
5. **Search, categories, Wayland filter.**
   - Search ranks name, then keywords, then summary with `unbound-search::matcher`.
   - Map freedesktop main categories to about 10 store categories.
   - `needs_x11()`: Flatpak from `sockets` in the remote metadata; debs by reading `Depends`
     from the APT `Packages` index with the heuristic in Findings.
   - Tests: ranking order on fixtures; `xterm`- and `gedit`-shaped control stanzas classify
     correctly; `sockets=x11;` versus `sockets=wayland;fallback-x11;`.
6. **Flatpak backend in `unbound-packages` (`flatpak.rs`).**
   - remotes, installed apps (`flatpak list --app --columns=application,origin,version,size`),
     `remote-info --show-metadata` for permissions, install/remove/update with
     `--noninteractive -y`, progress lines into the same callback, and updates
     (`remote-ls --updates`).
   - A `Permissions` summary built from the metadata keyfile.
   - Tests: parsing of real captured CLI output and metadata (committed fixtures, taken from the
     mini). The end-to-end check is an install and removal of a small Flathub app on the mini.
7. **App skeleton `crates/apps/software` (`unbound-software`, `org.unbound.Software`).**
   - Window with the Datum frame header and a search field.
   - Sidebar: Discover, the categories, Installed.
   - A tile grid (icon, name, summary). Lift Files' `grid.rs` layout into
     `crates/toolkit/widgets` if the store needs it, rather than copying it.
   - App menu through `unbound-app-menu`.
   - Desktop file, icon (D3 style), metainfo, deb metadata; `unbound-desktop` Recommends it.
   - Tests: a golden test of a category page from fixtures; a search test that typing filters
     the grid in rank order.
8. **App page and install.**
   - Header: icon, name, developer, source pop-up (Debian or Flathub), and an
     Install/Open/Remove button with a segmented progress bar.
   - Body: description, an info table (version, download size, licence, source), the Flatpak
     permission list, and the X11 notice.
   - Install resolves and installs through `unbound-packages`. Open launches through
     `unbound-xdg`.
   - Tests: a golden test of the app page; a test that Install on a Debian app calls the fake
     PackageKit with the resolved id and the button ends as Open. Mini: install a small
     Wayland-capable Debian app from Software and open it.
9. **Installed view and removal (end of MVP).**
   - Installed apps: catalogue entries matched to installed debs and Flatpaks; desktop entries
     without a catalogue entry as plain rows.
   - Remove with an inline confirmation.
   - An "Update" badge with an "Update All" button that opens Settings at the Software Update
     pane.
   - Tests: matching on fixtures (deb by package name, Flatpak by ref); removal through the
     fake PackageKit.
   - Mini: install and remove a Debian app and a Flatpak (with Flathub added by hand); the app
     appears in the launcher and leaves it again.
   - Fix `unbound-session`'s `XDG_DATA_DIRS` if Flatpaks do not appear.
10. **Flatpak in Software Update.**
    - The Settings Software Update pane lists Flatpak updates with the debs and installs both.
    - The timer check covers both.
    - Coordinate with `NOTES/plans/software-update.md`.
    - Tests: with debs and Flatpaks pending, the list holds both and Install All runs both
      backends; a Flatpak failure does not hide the deb result.
11. **Flathub source.**
    - A `packaging/unbound-flathub` deb ships the `.flatpakrepo` and the
      `remote-add --if-not-exists` step (or `/etc/flatpak/remotes.d`, if 1.16 supports it,
      checked on the mini).
    - Software shows the explained switch, applied with `remote-modify --enable/--disable`
      under polkit.
    - The polkit rule lets sudo members install Flatpaks system-wide without a prompt, as
      for PackageKit.
    - Tests: the switch's state comes from parsed `flatpak remotes` output (fixture). Mini:
      turn it on, see Flathub apps, turn it off, see them disappear.
12. **Screenshots and Featured.**
    - Add an HTTP client (owner decision; for example `ureq` with rustls).
    - Fetch screenshots and remote icons into a size-capped cache.
    - Add a screenshot strip on the app page.
    - The Discover page shows a curated `data/featured.kdl` (our picks, per source) and
      "New and updated" from release dates.
    - Tests: cache eviction by size and age; featured ids missing from the catalogue are
      skipped.
    - Mini: pages load offline from the cache.
13. **Integration and finish.**
    - `x-scheme-handler/appstream` (`appstream:id` opens the app page) and `.flatpakref` files.
    - The launcher offers "Get <name> in Software" when no installed app matches. This is a
      small adapter in `shell/launcher`.
    - Strings in sv/de/fr/es through `unbound-l10n`, and AccessKit names.
    - A guest test on the dev VM: search, install, open, remove.
    - `docs/parity.md` and `docs/roadmap.md` rows updated.

Later, not planned here: editing Flatpak permissions in Privacy & Security, ODRS ratings,
firmware (fwupd), OS release upgrades, and a "Needs X11" request list.

## Risks

- **PackageKit 2.0 breaks the D-Bus API.** Keep every PackageKit call inside
  `unbound-packages` so only one crate changes. The fake PackageKit in test-bus pins down the
  1.3 behaviour.
- **DEP-11 paths and icon layout differ from what we expect** on trixie's appstream 1.0.
  Milestone 1 records the real paths before any parser is written.
- **Parsing `flatpak` CLI output is brittle** (progress lines and translations). Run it with
  `LC_ALL=C` and `--columns`. If progress is unusable, show an indeterminate bar or move to
  libflatpak, accepting glib.
- **The X11 heuristic is wrong for some debs** (an app that links Xlib but also speaks Wayland).
  Show a warning rather than refusing to install, and keep an allowlist file for known
  exceptions.
- **Upkeep.** A store is never "done". The curated list and categories need care, and bad
  metadata shows up as our bug.
- **Network and privacy.** Screenshots and Flathub contact third parties. Fetch only after the
  user opens a page, and never send identifiers.
- **Two front ends on one system.** A user who also installs GNOME Software gets a second
  updater. Document the gschema override.

## Decisions for the owner

1. Build our own store (E), or ship GNOME Software (B) as a stopgap first?
2. Flathub: off until the user turns it on (recommended), or on by default? Unfiltered, or
   the `verified` subset?
3. Flatpak installs: system-wide with a polkit rule for sudo members (recommended), or
   per-user (`--user`, no polkit)?
4. New dependencies: a YAML parser for DEP-11, and an HTTP client for screenshots. Or skip
   screenshots.
5. X11-only apps: hidden by default (recommended) or shown with a warning?
6. Bring "an app store" forward from roadmap "Later" into wave B, next to B3 Software Update?
