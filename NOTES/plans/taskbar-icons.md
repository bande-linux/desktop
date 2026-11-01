# Taskbar (dock) icons

## Summary

The icon lookup itself is sound. The main real failure is that the dock reads the installed apps once, when the shell starts, so an app installed later (Zed today) gets a plain letter tile until the shell restarts. Our own dialog windows (file chooser, About/Log Out/low-memory dialogs, screenshot markup) also show letter tiles because they have no desktop entry. Three small commits.

## What the owner sees

Some running apps show a grey tile with a letter instead of their icon. On this mini, right now:

- **Zed** (`dev.zed.Zed`): letter tile. Its entry `~/.local/share/applications/dev.zed.Zed.desktop` was written at 09:59. `bande-shell` has been running since 07:52, so the dock has never seen that entry.
- **File chooser** (`org.bande.FileChooser`), **shell dialogs** (`org.bande.MenuBar`: About, Log Out, low memory, Bluetooth pairing), **screenshot markup** (`org.bande.Screenshot`): letter tiles while they are open. None of them has a desktop entry.
- **VS Code** gets the right icon, but from a 1024x1024 PNG that is shrunk on every frame. It can look grainy.

Everything else that is installed and has run here finds its icon: Terminal, Files, Settings, Activity Monitor, Text Editor, Papers, Firefox ESR, Claude (`com.anthropic.Claude` and its `claude-desktop` side windows), VS Code (`com.microsoft.VSCode` and `code`). I checked the window list in `~/.local/state/bande/windows` and every `Icon=` against the theme folders.

## Findings

How a tile gets its icon: the window's app_id goes to `find_app` (`crates/libs/xdg/src/identity.rs:25`), which tries an exact id, then case-insensitive, `StartupWMClass`, the last reverse-DNS part, then the Exec program. Then `Icon=`, then `find_icon` (`crates/libs/xdg/src/icon.rs:183`), which searches Bande, Adwaita, inherited themes, hicolor, pixmaps, and absolute paths. Then `Image::load`, which handles PNG and SVG. Finally `Dock::icon`/`describe` (`crates/shell/bar/src/dock_client.rs:112`, `:152`) draw it. When nothing is found, the tile is `generic_icon` with the name's first letter (`crates/shell/bar/src/dock.rs:2206`).

1. **Entries are loaded once.** `Dock::new` calls `applications(...)` a single time (`dock_client.rs:1012`). Nothing ever reloads it, and `icons` is only cleared on a scale change (`dock_client.rs:198`). Any app installed while the session runs (apt, a tarball into `~/.local`, Flatpak) has no entry, so it gets a letter tile and the last-part name. The app switcher already refreshes its entries on a timer (`crates/platform/compositor/src/switcher.rs:294`). The launcher has the same load-once pattern (`crates/shell/launcher/src/client.rs:520`); I only note it here.
2. **Our own windows have no entry.** The app_ids `org.bande.FileChooser` (`crates/apps/file-chooser/src/main.rs:25`), `org.bande.MenuBar` (`crates/shell/bar/src/dialogs.rs:19`, also used by `pairing.rs:326` and `shell/src/shell.rs:148`) and `org.bande.Screenshot` (`crates/shell/screenshot/src/markup_app.rs:20`) have no `.desktop` file. Only Terminal, Files, Settings and Activity Monitor ship one. The compositor publishes every toplevel to the dock (`crates/platform/compositor/src/shell.rs:332`). The theme already has `org.bande.Screenshot.svg` and `org.bande.About.svg` in `data/icons/Bande/scalable/apps/`, but `describe` only ever looks up an icon through an entry.
3. **Large PNGs are not scaled at load.** `Image::load` (`crates/toolkit/graphics/src/scene.rs:173`) scales SVGs to the requested size but keeps PNGs at their native size. VS Code (`/usr/share/pixmaps/vscode.png`, 1024px) and Zed (512px) are kept that big and sampled down every frame (`crates/toolkit/graphics/src/draw.rs:305`).
4. **Not problems here, left alone:**
   - `xdg-toplevel-icon-v1` is not implemented. No app running here needs it, since every one has an entry.
   - No Flatpak or snap exports are installed. They would be found anyway once `XDG_DATA_DIRS` lists them.
   - XPM and `Icon=name.png` forms are not supported. Only `python3.13.desktop` uses XPM, and it is `NoDisplay`.
   - NoDisplay entries are skipped for matching (`entry.rs:361`). The only ones here are portal and keyring helpers.
   - Apps with no entry at all (scripts, dev builds run from a terminal) keep the letter tile. That is expected.

**Overlap:** the "Keep in Dock doesn't work for apps launched from a terminal" plan owns the app_id-to-entry matching (`find_app`/`running_app`/`identity`). This plan does not change matching. Note that finding 1 also hides a kept app: `describe` drops a kept tile whose entry is missing (`dock_client.rs:155`). So a kept Zed vanishes after it quits until the shell restarts. That plan should check this case before blaming matching.

## Recommendation

Make the dock re-read desktop entries when an applications folder changes. Give our own windows icons, or keep them out of the dock (owner decision). Scale big PNGs down once at load. Do not add `xdg-toplevel-icon-v1` until an app actually needs it.

## Implementation plan

1. **Dock picks up apps installed while it runs** (about 2h)
   - `crates/shell/bar/src/dock_client.rs`: start a `bande_watch::FolderWatch` (`Changes::Any`) on `<dir>/applications` for each existing `data_dirs()` entry, plus the data-home folder itself so that a new `~/.local/share/applications` is caught. Wake through the dock's existing `Inbox` source. Debounce about 300 ms, since dpkg writes several files. On wake: reload `entries`, clear `icons`, `load_trash_icons()`, `follow_windows(self.windows.clone())`, `sync()`.
   - Test, `crates/shell/bar/tests/live.rs`: open a toplevel with app_id `org.example.Late` and no entry. The dock tile's accessible label is `Late`. Write `org.example.Late.desktop` with `Name=Late Arrival` into the test `apps/applications` folder. The label becomes `Late Arrival` without restarting the shell.
2. **Our own windows get the right tile** (about 1h, shape set by the owner decision below) Done: the file chooser and the shell dialogs are hidden from the dock, and a window without an entry falls back to an icon named after its app_id (the screenshot markup window gets `org.bande.Screenshot.svg`).
   - Default (show with an icon): in `describe`, when there is no entry, try the app_id itself as the icon name. This is the usual fallback for apps whose hicolor icon is named after the app_id. Add `org.bande.FileChooser.svg` to `data/icons/Bande/scalable/apps/`. Give the shell dialogs an app_id that has an icon (`org.bande.About`), or add `org.bande.MenuBar.svg`.
   - Alternative (hide): skip `org.bande.FileChooser` and `dialogs::DIALOGS` in `follow_windows`, so they never become dock tiles.
   - Test: the hide variant gets a `live.rs` case where a `org.bande.MenuBar` toplevel adds no dock tile. The icon variant is a one-line fallback plus assets. Verify it on the mini by opening the file chooser, About and a screenshot markup window. No unit test, since it would be tautological.
3. **Raster icons are scaled once at load** (about 1h)
   - `crates/toolkit/graphics/src/scene.rs`: in `Image::load`, if a decoded PNG is larger than `size`, resize it to fit `size`x`size`, keeping the aspect ratio (`image::imageops::resize`, `FilterType::Lanczos3`). Smaller images stay as they are. All 18 callers pass the size they draw at. Check the control-center artwork caller (`crates/shell/controlcenter/src/client.rs:111`) still looks right, because its images are not square.
   - Test, `crates/toolkit/graphics/tests/render.rs`: write a 256x128 PNG (left half red, right half blue) to a temp dir. Load it at 64: the image is 64x32, the left edge is red and the right edge is blue. A 32x32 PNG loaded at 64 stays 32x32.

## Decisions for the owner

- Should the file chooser and the shell's own dialogs (About, Log Out, low memory, pairing) show up in the dock at all? On macOS, open/save panels belong to their app and system dialogs get no dock tile. Recommended: hide them. Choose "show" if you want to be able to click back to an open file chooser from the dock.

Decided overnight 2026-11-01: hide the file chooser and the shell's own dialogs from the dock (plan recommendation; owner may revisit).
