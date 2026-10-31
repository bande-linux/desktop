# Third-party title bars: GTK, libadwaita, Electron

## Summary
Not every app can get the Bande title bar, because on Wayland each toolkit decides whether it draws its own. We can widen the set that does get our bar. Today only `xdg-decoration` clients get it (Qt, SDL, foot, Electron windows with a frame). Adding KDE's server-decoration protocol brings in GTK 3 and GTK 4 windows that have no header bar (Firefox with its title bar on, LibreOffice, GIMP, Inkscape). Apps that will always draw their own header (libadwaita, GTK header bars, VS Code's and Claude Desktop's custom bars) can only be styled to look like ours: the control pill on the left, Bande glyphs, the frame colour and the Manrope title, plus our window menu on right-click. VS Code gets our real bar if `window.titleBarStyle` is set to `native`. Claude Desktop's main window cannot get it without a change in the app.

## What the owner sees
- Calculator, Text Editor, Loupe, Papers and Showtime (libadwaita) draw a GNOME header. The title is centred and bold, and a round close button sits on the right (`button-layout` is `appmenu:close`). Our bar has a left pill with close, minimise and zoom, and a flush-left Manrope title.
- Right-clicking those headers does nothing. Right-clicking our bar opens the window menu.
- VS Code draws its own title bar with the command centre, and Electron draws the controls on the right.
- Claude Desktop draws its own top bar, and Electron draws the controls.
- Firefox puts its tabs in a GTK client-side title bar with GNOME-style buttons.

## Findings
Compositor:
- `crates/platform/compositor/src/decoration.rs:8-37`: `xdg-decoration` is the only decoration protocol. New decorations default to `ServerSide`, and a client's request is honoured as is.
- `crates/platform/compositor/src/window.rs:55-61`: the 40 pt bar (`bande_graphics::FRAME_HEADER_HEIGHT`, `crates/toolkit/graphics/src/title_bar.rs:6`) exists only when the toplevel's `decoration_mode` is `ServerSide`. Geometry, sizing and bounds all key off that one field (`window.rs:174-208`, `282-285`).
- `crates/platform/compositor/src/state/globals.rs:57,118`: only `XdgDecorationState` is created. There is no `org_kde_kwin_server_decoration_manager`, although Smithay 0.7 ships it (`smithay::wayland::shell::kde::decoration::KdeDecorationState`, built with `wayland_frontend`). Smithay's xdg configure only sends a decoration event when an xdg decoration object exists (`smithay-0.7.0/src/wayland/shell/xdg/mod.rs:1493-1547`). So `decoration_mode` can also carry a mode chosen through the KDE protocol.
- `crates/platform/compositor/src/state/handlers.rs:155-240`: `XdgShellHandler` does not implement `show_window_menu`, so Smithay's default no-op runs. The window menu (`window_menu.rs:301`) opens only from a right-click on our own bar (`frame.rs:238-241`).
- `frame.rs:345`: the server-side bar uses `Palette::for_scheme` (default accent, standard contrast).
- Look to match (`title_bar.rs:5-17,126-235`): a 66 × 24 pill at x 12, filled with `well` when active and outlined with `hairline` when not. It holds glyphs at a 21 pt pitch in `ink2` (inactive `ink3`, `ink` on hover): a cross, a minus, and two corner arrows for zoom (pointing in when zoomed). A `danger` disc of radius 8 under close on hover, and an `ink` 12 % disc when pressed. The title uses `TextRole::WindowTitle`, Manrope 15/20 bold (`toolkit/design/src/type_scale.rs:69`), and is flush left. Our own apps' header is 48 pt (`HEADER_HEIGHT`).

Toolkits (checked on this machine and in upstream source):
- GTK 3.24.49 (`libgdk-3.so.0`) and GTK 4.18.6 (`libgtk-4.so.1`) contain `org_kde_kwin_server_decoration_manager` and no `zxdg_decoration_manager_v1` (`strings`). GTK 4.18 `gdk_wayland_display_prefers_ssd` is TRUE only when that manager's default mode is SERVER. `gtk_window_should_use_csd` on Wayland returns `!prefers_ssd` and does not read `GTK_CSD` there. `gtk_window_set_titlebar` always enables client-side decoration (gtk-4-18 `gtk/gtkwindow.c`, `gdk/wayland/gdkdisplay-wayland.c`). `gdk_wayland_toplevel_set_decorated` creates the KDE decoration and requests SERVER, or CLIENT for client-side windows. A window that never sets decorated sends nothing (`gdktoplevel-wayland.c`).
  - Root cause for GTK: we do not offer the one protocol GTK speaks. Without it GTK always draws client-side, even for windows that have no header bar.
- libadwaita 1.7.6: `AdwWindow` and `AdwApplicationWindow` always draw their own header (`AdwHeaderBar` inside the content), with a centred title. No setting makes them use server-side decorations. GNOME treats that as design (Mutter has no `xdg-decoration` at all).
- GTK 4.18 reads its settings from the Settings portal even when not sandboxed, if the portal interface exists (`gdk_display_should_use_portal`, gtk-4-18 `gdk/gdk.c`). This includes `org.gnome.desktop.wm.preferences button-layout`, which becomes `gtk-decoration-layout` (`gdk/wayland/gdksettings-wayland.c`). GTK 3 does the same because `bande-session` sets `GTK_USE_PORTAL=1` (`crates/platform/compositor/data/bande-session:14`).
- `crates/services/portal/src/settings.rs:282-345` serves no `org.gnome.desktop.wm.preferences`. `bande-portals.conf` falls back to the GTK backend (`Settings=bande;gtk;`), so the portal now answers `button-layout` with `"appmenu:close"` (`busctl ... ReadOne`). That is why the controls are on the right.
- `crates/services/portal/src/gtk.rs:14-45,82-96` writes `~/.config/gtk-4.0/bande.css` with libadwaita colour variables only, with no rules for the header, window controls or title font. GTK 4.16+ supports `var()` and `color-mix`, and libadwaita's own rules are written that way (`windowcontrols > button > image`, `headerbar { min-height: 47px }` in libadwaita's `base.css`). A user stylesheet has USER priority (800) and overrides them.
- GTK 3 Adwaita (`gtk-contained.css` in `libgtk-3.so.0`) uses literal colours: zero uses of `@theme_bg_color`. `@define-color` overrides therefore do not restyle GTK 3 header bars or title buttons. Explicit rules are needed. This matters for `NOTES/plans/dark-theme.md` milestone 4, which plans `@define-color` overrides for GTK 3.
- The Bande icon theme's `window-maximize-symbolic` is a rounded square, not our zoom glyph (`data/icons/src/symbols.py:681-684`, `/usr/share/icons/Bande/symbolic/actions/`). GTK takes these icons for its window controls.
- Qt 6.8 (`qt6-wayland`) negotiates `xdg-decoration` and already gets our bar. `QT_WAYLAND_DECORATION` only picks a client-side plugin (adwaita, bradient) when the compositor refuses server-side. No Qt apps are installed.

Electron:
- VS Code 1.144 runs Electron 43.8.3 (Chrome 150). Its settings schema defaults to `window.titleBarStyle: "custom"` (frameless, VS Code's own bar) and `window.controlsStyle: "native"` (Electron's Window Controls Overlay). `~/.config/Code/User/settings.json` has no `window.*` keys.
- Claude Desktop 4.4212.0 runs Electron 44.9.3 (Chrome 152). In `resources/app.asar`, the main window is created with `titleBarStyle:"hidden",titleBarOverlay:!0`, which is frameless. Artifact windows use `frame:!0`, which gives a native frame and so already gets our bar.
- Electron gives a `frame: true` window its own client frame only when `WaylandWindowDecorations` is on and the platform cannot do server-side decorations. Otherwise the window uses the native frame (Electron commit 7caa88c). On our compositor, which offers `xdg-decoration`, a framed window should therefore ask for server-side. This still has to be confirmed live (milestone 6).
- `ELECTRON_OZONE_PLATFORM_HINT=auto` (`bande-session:12`), and both apps run natively on Wayland. No flag forces a frame onto a frameless Electron window. If the compositor added a bar to such a window, it would sit on top of the app's own bar.

How others do it:
- KDE: KWin offers both protocols with SERVER as the default, so GTK windows without a header bar get Breeze decorations. For header-bar apps, kde-gtk-config syncs the button order into GTK (merged), and work was done to render KWin's buttons as GTK assets ([D28623](https://phabricator.kde.org/D28623), [button order](https://commits.kde.org/kde-gtk-config/2b12ceb1b89175fb96e5cd12f62549b66f10aa85)). We follow that pattern.
- GNOME: client-side decoration everywhere, with consistency through Adwaita. There is no server-side path.
- macOS: frameless Electron windows (`hiddenInset`) still get the system's traffic lights. That is why Claude Desktop looks native there. Linux has no protocol for controls drawn by the compositor over client content.

## Options
1. **KDE server-decoration with SERVER as the default.** It is small and uses Smithay's existing module. GTK 3 and GTK 4 windows without a header bar get the real Bande bar, and header-bar apps announce CLIENT and stay as they are. Risk: an app that sets a header bar late, after mapping. GTK sends the change explicitly, and we honour it.
2. **Style client-side headers to resemble ours.** This means the portal's `button-layout`, CSS for the libadwaita and GTK 4 header and controls, a GTK 3 rule set, and matching symbolic icons. It reaches every GTK app, but it is an approximation: libadwaita titles stay centred and headers are 47 pt, not 48. It also has to keep up with libadwaita's CSS node names.
3. **Our window menu for client-side headers** (`show_window_menu`). It is small, and GTK, libadwaita, Firefox and Chromium already request it on a right-click in the header.
4. **Force server-side on everyone, or crop client headers.** Rejected. libadwaita and Electron put toolbars and tabs in their headers, so this would give double bars or lost buttons.
5. **Patch GTK or libadwaita downstream, or use gtk-nocsd.** Rejected. The maintenance cost is high, gtk-nocsd is X11-era, and libadwaita cannot be patched that way.
6. **A full generated GTK 3 theme (Adwaita SCSS recompiled with Datum colours).** It is complete, but needs `sassc` in packaging and one theme per accent and scheme. It is deferred. Overriding only the title bar and accent is enough for now.
7. **Electron.** VS Code: the `native` title bar style gives our real bar but moves the menu into a row below it and drops the command centre. Claude Desktop: only Anthropic can change it (a `frame: true` main window on Linux Wayland).

## Recommendation
Do 1, 3 and 2 in that order. For GTK 3, ship two small themes written by the portal, `Bande` and `Bande-dark` in `$XDG_DATA_HOME/themes`. Each imports Adwaita or Adwaita-dark from GTK's resources and adds the title bar and accent rules. The portal serves them as `gtk-theme`, so running GTK 3 apps switch live when the scheme changes. A rule set in `~/.config/gtk-3.0/gtk.css` would stay in the old scheme until the app restarts. Electron needs no code: set VS Code to `native` (owner decision), and accept Claude Desktop's own bar or ask Anthropic.

## Implementation plan
Branch per milestone, gate as in CLAUDE.md, verify on the mini.

1. **KDE server-decoration** (about 2 h)
   - `compositor/src/state/globals.rs`: `KdeDecorationState::new::<State>(handle, DefaultMode::Server)`.
   - `compositor/src/decoration.rs`: implement `KdeDecorationHandler` and `delegate_kde_decoration!`. `new_decoration` applies Server, and `request_mode` maps Server to `ServerSide` and Client or None to `ClientSide`. Both go through the existing `State::decorate`, looking up the toplevel with `window_index(surface)`. A decoration for a surface with no toplevel is ignored.
   - `testing/test-wayland/src/decoration.rs`: `show_kde_decorated_window(server, image, Option<KdeMode>)` (`wayland-protocols-misc` client, already a dependency) and `kde_default_mode()`.
   - Tests in `compositor/tests/headless/decoration.rs`:
     - The manager announces Server.
     - A window that requests Server gets our bar: content 40 pt down, chassis pixels in the bar, configure height minus 40.
     - A window that requests Client keeps its full size with no bar.
     - Server then Client drops the bar and gives the height back.
   - Docs: `docs/architecture/compositor.md:43-47`. Fix the stale "28 pt, centred title" text too. Add a protocol table row, and update `docs/design/features/windows.md:15-16`.

2. **Window menu for client-drawn headers** (about 1 h)
   - `compositor/src/state/handlers.rs`: `show_window_menu` checks that the serial belongs to the current pointer press (as `grab.rs:102` does). It then opens the menu at the surface location plus the given point.
   - `window_menu.rs`: `open_window_menu` takes the point.
   - `test-wayland`: `request_window_menu(x, y)` on the toplevel.
   - Tests in `headless/window_menu.rs`:
     - A press followed by a request with its serial opens the menu at that point.
     - A request with a stale serial opens nothing.

3. **GTK 4 and libadwaita headers in our style** (about 2 h)
   - `portal/src/settings.rs`: serve `org.gnome.desktop.wm.preferences` with `button-layout` `"close,minimize,maximize:"`.
   - `portal/src/gtk.rs`: add `--bande-well`, `-hairline`, `-ink`, `-ink2`, `-ink3` and `-danger` per scheme. Add rules for:
     - `headerbar` (chassis, no shade line).
     - `headerbar .title, windowtitle .title` (Manrope, 15 px, 700).
     - `windowcontrols` (the pill: 24 px high, radius 12, buttons on a 21 px pitch, `:backdrop` outline instead of fill).
     - Glyph colours, close hover (danger disc, white glyph) and pressed (ink 12 %).
   - Tests in `portal/tests/settings.rs`:
     - `button-layout` is served and signalled.
     - The pill variables in the light and dark blocks equal `Palette::generate(...).well`, `hairline`, `ink2`, `ink3` and `danger`. This catches drift from `TitleBar`.
     - A user's `gtk.css` is still left alone.

4. **Window-control icons that match the bar** (about 1 h)
   - `data/icons/src/symbols.py`: redraw `window-close`, `-minimize`, `-maximize` (outward corners) and `-restore` (inward corners) as `title_bar.rs` glyphs on a 16 grid with a 1.5 stroke as filled outlines. Regenerate `data/icons/Bande*/symbolic/actions`.
   - Proof: refreshed `crates/shell/desktop/tests/golden/icons-{light,dark}.png`. The existing single-colour-fill test must still pass.

5. **GTK 3 title bars and accent** (about 2 h)
   - Coordinate with `dark-theme.md` milestone 4 and replace its GTK 3 `@define-color` file with this.
   - `portal/src/gtk.rs`: `install_gtk3_themes(data_home, accent)` writes `themes/Bande{,-dark}/index.theme` and `gtk-3.0/gtk.css`. Each `gtk.css` imports `resource:///org/gtk/libgtk/theme/Adwaita/gtk-contained{,-dark}.css` and adds:
     - `headerbar` and `.titlebar` in chassis, with no gradient and no inset shadow.
     - The title font.
     - `button.titlebutton.close`, `.minimize` and `.maximize` as one pill.
     - The accent on `:selected`, `.suggested-action`, `switch:checked`, `check:checked`, `radio:checked`, `progressbar progress` and `scale highlight`.
   - Also write `gtk-4.0/gtk.css` importing `Default/Default-{light,dark}.css`, so plain GTK 4 apps that get this theme name keep GTK's default theme.
   - `settings.rs`: `gtk_theme` serves `Bande` and `Bande-dark`. HighContrast stays under Increase Contrast.
   - Tests:
     - Switching to dark serves and signals `Bande-dark`.
     - Increase Contrast still serves HighContrast.
     - Each theme file imports its scheme's Adwaita and carries that scheme's chassis and accent hexes.
   - Update `docs/architecture/session.md` (GTK paragraph) and `docs/parity.md:47`.

6. **Live check, Electron and review scenario** (about 1.5 h)
   - On the mini, launch each installed app with `WAYLAND_DEBUG=1` and record the decoration requests: Firefox (title bar on and off), Calculator, Text Editor, Papers, VS Code (custom and native), and a Claude Desktop artifact window.
   - Confirm that a framed Electron window requests server-side, and check whether Electron's controls overlay follows `button-layout`.
   - Add `ci/review/scenarios/third-party.scenario`: Calculator, Text Editor and Firefox in light and dark, next to Files. This takes over the screen, so warn the owner first.
   - Write the per-app result table into `docs/architecture/compositor.md`.

## Risks
- A GTK 3 app that creates its KDE decoration after the first configure could flicker between modes for one frame. The tests in milestone 1 cover the order GTK uses (create, then request, before the first commit).
- libadwaita CSS node names change between releases (1.7 now). The rules in milestone 3 target `headerbar`, `windowtitle` and `windowcontrols` only, and must be rechecked when Debian moves libadwaita.
- Renaming `gtk-theme` to `Bande` affects anything that reads it: Chromium's GTK integration and GTK 2 (absent). The theme imports Adwaita, so the worst case is plain Adwaita.
- The GTK 3 accent rules cover the common widgets only, so rare widgets keep Adwaita blue.
- GTK 4.18 has no `prefers-color-scheme`, so libadwaita header colours still switch only on app restart, as today.
- Milestone 6 relies on Electron's behaviour as read from its source, not yet seen live.

## Decisions for the owner
1. VS Code: switch to `"window.titleBarStyle": "native"` on this machine for our real bar (menus move to a row below it, and the command centre goes), or keep VS Code's own bar styled as it is?
2. Claude Desktop's main window cannot get our bar from the desktop side. Accept that, or report it to Anthropic as a request for a native frame on Linux Wayland?
3. libadwaita apps will look close to ours but keep a centred title and a 47 pt header. Is that acceptable, or should replacing those GNOME apps with Bande apps be planned instead?
