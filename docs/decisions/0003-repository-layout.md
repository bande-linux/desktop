# Repository layout: Decision Record

Status: **accepted** (2026-10-29). Scope: where crates live in the workspace, how they may
depend on each other, and the conventions every crate follows.

## Context

- The workspace has 31 crates side by side in `crates/`, from the compositor to test harnesses,
  with nothing in the layout saying which are libraries, which are processes, and which role a
  process plays.
- The project will grow much larger: more apps, more shell clients, more session services, and
  shared logic that today sits inside the first crate that needed it.
- Several agents work in parallel on separate branches. Each needs to know which directory it
  owns and which dependencies it may add without asking.

## Decision

Crates are grouped by role:

```
crates/
  platform/  compositor, render, protocols
  toolkit/   toolkit, graphics, platform, widgets
  shell/     dock, menubar, launcher, controlcenter, screenshot, lock, greeter,
             desktop, notifications
  apps/      files, settings, terminal, monitor
  services/  config, notify, power, clipboard, tray, automount, portal, polkit
  libs/      spring, xdg, fs, preview, audio, network, bluetooth, l10n, log
  testing/   test-bus, test-wayland, bench, sample
data/        shared assets (wallpapers, icons, sounds)
```

- **`platform/`**: the compositor, its Vulkan renderer and the private Wayland protocols.
- **`toolkit/`**: `toolkit` is a facade that re-exports the other three. `graphics` holds the
  scene, the Vello renderers, Parley text and the design tokens, and has no Wayland dependency.
  `platform` holds the Wayland window, seat, clipboard, text input, layer and xdg shell
  surfaces, and the accessibility adapter. `widgets` holds the UI tree, the widgets and
  animation.
- **`shell/`**: one crate per shell client or shell part. `shell` is `bande-shell`, the one
  process that hosts the parts `desktop`, `bar`, `notifications`, `controlcenter` and `launcher`
  (library crates, see [overview](../architecture/overview.md)); `screenshot`, `lock` and
  `greeter` stay processes of their own.
- **`apps/`**: one crate per app.
- **`services/`**: one crate per session service; `portal` is `xdg-desktop-portal-bande`.
- **`libs/`**: shared logic without UI. `xdg` holds desktop entries, icons, MIME types,
  `mimeapps.list`, user directories and `Apps`, the one answer to "which apps open this path";
  `fs` holds folder listings and their sort order, size and date formatting, the trash, file
  operations with their running queue and undo, and watching; `preview` reads the Quick Look
  content of a file and hands decoded pixels to a constructor the caller passes, so it needs no
  toolkit type.
- **`testing/`**: `test-bus` (with the shared D-Bus fakes of system services), `test-wayland`,
  the benchmarks and the toolkit sample.

Rules:

1. **Names.** Directory names are short. Package, binary and Debian package names stay
   `bande-*` (and `xdg-desktop-portal-bande`), so nothing installed changes name.
2. **One crate per process.** Each app, shell client and service is its own crate; each part
   hosted by `bande-shell` is its own library crate.
3. **Dependency direction.** `libs` ← `toolkit` ← `shell` and `apps`. Services never depend on
   the widget crates; CI checks this.
4. **Crate structure.** In each app and shell crate, `main.rs` is thin, model modules hold no
   toolkit types, and view modules build on the toolkit. Logic that a second crate needs moves
   to `libs`.
5. **Installable files.** Everything a crate installs (desktop files, icons, units, D-Bus
   activation files, PAM configuration) lives in that crate's `data/`.
6. **App IDs.** Reverse-DNS IDs, one per app, used for the desktop file, the icon, the Wayland
   `app_id` and the D-Bus name: `org.bande.Files` for Files.
7. **No panics.** `[workspace.lints]` denies `unwrap`, `expect` and `panic` outside tests.
8. **Logging.** Through `tracing`, to the journal, set up by `libs/log`.
9. **Localization.** Every user-visible string goes through Fluent (`libs/l10n`), with
   `locales/en-US/*.ftl` in each crate.
10. **Appearance.** Dark mode and the accent colour are followed by the toolkit itself, from the
    portal's `org.freedesktop.appearance` settings, not by each app.

## Consequences

- The moves are done once, each crate with `git mv` so that history follows; afterwards a new
  crate goes straight into its directory.
- The `shell/`, `apps/` and `services/` split is by process role, not by "system" versus "user"
  software. Settings and Activity Monitor are ordinary apps with no special access.
- Shared assets live in `data/` at the root; everything else a package installs stays with its
  crate.
