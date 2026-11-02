# Repository layout: Decision Record

Status: **accepted** (2026-10-29). Scope: where crates live in the workspace, how they may
depend on each other, and the conventions every crate follows.

## Context

The workspace had 31 crates side by side in `crates/`, with nothing saying which were libraries,
which were processes and which role a process played, while several agents work in parallel and
each needs to know which directory it owns.

## Decision

Crates are grouped by role:

```
crates/
  platform/  compositor, render, protocols
  toolkit/   toolkit, design, graphics, platform, widgets
  shell/     shell, bar, desktop, notifications, controlcenter, launcher,
             screenshot, lock, greeter
  apps/      files, settings, file-chooser, secrets-prompt
  services/  config, notify, power, sound, clipboard, tray, automount, portal, polkit,
             secrets, secrets-pam
  libs/      app, app-menu, app-settings, settings-panes, xdg, fs, content, quick-look, thumbnails,
             search, status, taskbar-model, packages, audio, network, bluetooth, volumes,
             keyboard, datetime, l10n, log, bus, inbox, watch, wallpaper, edid, spring
  testing/   test-bus, test-wayland, bench, sample, review
data/        shared assets (wallpapers, icons, sounds)
```

The other apps live in [bande-linux/apps](https://github.com/bande-linux/apps) and depend on
these crates at a pinned revision.

- **`platform/`**: the compositor, its Vulkan renderer and the private Wayland protocols.
- **`toolkit/`**: `toolkit` is a facade that re-exports the others. `design` holds the Datum
  tokens with no dependencies; `graphics` holds the scene, the Vello renderers and Parley text,
  and has no Wayland dependency.
  `platform` holds the Wayland window, seat, clipboard, text input, layer and xdg shell
  surfaces, and the accessibility adapter. `widgets` holds the UI tree, the widgets and
  animation.
- **`shell/`**: one crate per shell client or shell part. `shell` is `bande-shell`, the one
  process that hosts the parts `desktop`, `bar`, `notifications`, `controlcenter` and `launcher`
  (library crates, see [overview](../architecture/overview.md)); `screenshot`, `lock` and
  `greeter` stay processes of their own.
- **`apps/`**: one crate per app built here, each on `bande-app` (`libs/app`), and the dialog
  processes of services: `file-chooser` is the portal's dialog and `secrets-prompt` the
  keychain's (`bande-secrets-prompt`, installed by `bande-secrets`).
- **`services/`**: one crate per session service; `portal` is `xdg-desktop-portal-bande`.
- **`libs/`**: shared logic without UI. `xdg` holds desktop entries, icons, MIME types,
  `mimeapps.list`, user directories and `Apps`, the one answer to "which apps open this path";
  `fs` holds folder listings and their sort order, size and date formatting, the trash, file
  operations with their running queue and undo, and watching; `content` decodes what a file
  holds without any toolkit type (image pyramids and animations, PDF pages from a confined
  `hayro` child process, text with cheap syntax colours, Markdown, RTF and HTML as blocks,
  fonts, folder and archive totals), shared by Files, the launcher, Quick Look and the apps
  repository's Preview; `quick-look` is the one Quick Look panel that Files and the desktop
  show (it draws with the toolkit, an exception to "without UI").
- **`testing/`**: `test-bus` (private D-Bus buses and fakes of system services),
  `test-wayland`, the benchmarks, the toolkit sample and `review` (records a scenario on a live
  session).

Rules:

1. **Names.** Directory names are short. Package, binary and Debian package names stay
   `bande-*` (and `xdg-desktop-portal-bande`), so nothing installed changes name.
2. **One crate per process.** Each app, shell client and service is its own crate; each part
   hosted by `bande-shell` is its own library crate.
3. **Dependency direction.** `libs` ← `toolkit` ← `shell` and `apps`. Services never depend on
   the widget crates (except the polkit agent, which shows a dialog); a service that needs a
   window starts a dialog process from `apps/`; `ci/release/lint.sh` checks this.
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

- The `shell/`, `apps/` and `services/` split is by process role, not by "system" versus "user"
  software. Settings and Activity Monitor are ordinary apps with no special access.
- Shared assets live in `data/` at the root; everything else a package installs stays with its
  crate.
