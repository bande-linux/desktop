# UI polish: every surface professional, clean and modern

## Summary

The owner's example, the Restart / Shut Down / Log Out confirmation, shows the two causes behind most of what looks hacky:

1. **Dialogs are full app windows.** Every shell prompt opened as a normal toplevel. It had the 48 pt header, the window-control well, a title repeating the button, and an inset sheet. Inside were two plain labels and left-aligned buttons.
2. **They borrowed each other's size.** All bar dialogs (About, About App, pairing, Low Memory, the session confirmation) share the app id `org.bande.MenuBar`. The compositor remembers one frame per app id (`~/.local/state/bande/windows`) and gives it to the next window with that id. On the mini the remembered frame is 372×264, so the confirmation opened at the About window's size whatever it needed.

Recommendation: a toolkit **dialog window**. It has no title bar, is sized to its content, is not resizable, and is never remembered. Its content is the toolkit `Alert`: a semibold title, a secondary message, and a footer tray with the buttons right-aligned and the default on the far right. Then move every prompt and small utility window onto it, and fix the remaining inconsistencies surface by surface, in order of how often the owner sees them.

## Audit

Method: I read the goldens (light and dark where they exist) against `docs/design/datum.md` and the tokens in `crates/toolkit/design`. Coverage was about 50 of 65 shell, toolkit and compositor goldens and about 15 of 110 app goldens; the 1.5× and inactive variants were mostly skipped. Some "blank icon" findings come from the hermetic test environment, which has no icon theme, so the live desktop shows real icons there. Those findings are marked *(test fixture)* and are not counted as defects.

### Shell prompts (seen on every restart, pairing, low memory)
- `shell/bar/tests/golden/shut-down-light.png`, `shut-down-blocked-dark.png`: two plain labels with equal weight, buttons bottom-left, a full window frame around them, and a size borrowed from another dialog (see Summary). **Fixed in M1.**
- `shell/bar/tests/golden/pairing-confirm-light.png`, `pairing-pin-light.png`: the same plain-label layout with left-aligned buttons, in a titled toplevel (`crates/shell/bar/src/pairing.rs:325`). It should be an alert: the title "Pair with …?", the code in large monospace, and buttons in the footer.
- About This Computer (`dialogs.rs` `run_about`): label rows that join label and value with four spaces (`"{label}    {value}"`) in a 360×210 titled window. It has no golden. It should be a fixed-size window with an icon, the name, and a two-column grid of right-aligned labels and values.
- `shell/bar/tests/golden/low-memory-light.png`: good structure. The table is a fixed height, so three rows leave 40 % empty, and the window is a resizable 460×410 toplevel that shares the remembered frame. Use a dialog window and fit the table to its rows (with a cap).
- `shell/bar/tests/golden/low-memory-confirm-dark.png`: the nested sheet leaves the parent footer's red Force Quit visible next to the sheet's own. Dim the parent footer under a sheet.

### Authentication
- `services/polkit/tests/golden/asking.png`, `wrong-password.png`, `choose-user.png`: the right alert style, but there is a dead band of about 50 pt between the password field and the footer. The title wraps to two lines while the message ends on a one-word orphan line. Fit the height to the content (`crates/services/polkit/src/dialog.rs`, the body height around `:185`).

### Toolkit
- `toolkit/platform/tests/golden/alert-light-1x.png`: the sheet runs past the parent's bottom edge, the table rows show through under it, and its radius does not match the window's. "Don't Quit" uses link styling where datum asks for a quiet button.
- `toolkit/platform/tests/golden/table-light-1x.png`: "% CPU" is truncated in the header while the user column has room to spare. The column minimums need fixing.

### Shell surfaces
- `shell/notifications/tests/golden/center-light.png`, `banners-light.png`: the cards touch the right screen edge ("Clear All" sits on the edge), and a stray stack-peek bar shows under the second card. Owned by the notification-placement agent's area: do this after they land.
- `shell/controlcenter/tests/golden/panel-light.png`: the sliders' left inset differs from the labels' inset, and the media popup is undersized.
- `shell/lock/tests/golden/login-power-shut-down.png`: the confirmation is a bare caption line between the password field and the buttons. It should match the alert copy and hierarchy (a title plus the countdown).
- `shell/lock/tests/golden/lock-screen.png`: there is a large empty band between the clock and the avatar. Fine as macOS-like, so no change is planned.
- `shell/launcher/tests/golden/launcher-results-light.png`: the section gaps are uneven ("ALL APPS" sits tight under a two-line row). The grey squircles are *(test fixture)*.
- `shell/bar/tests/golden/bar-light.png`, `taskbar-bottom-light.png`, `compositor/tests/golden/switcher-light.png`, `shell/desktop/tests/golden/desktop-light.png`: letter tiles and blank tiles *(test fixture)*. The switcher's selection square is larger than the tile and not concentric (datum §4 radius rule).
- Fine: `status-menus-light`, `cascade-light`, `window-menu`, `screenshot/toolbar-light`, `widgets/controls-*`, `lock-screen-status`, `launcher-none`, `launcher-emoji`.

### Apps
- `apps/software/tests/golden/utilities-light.png`: the app names are centred over left-aligned descriptions, the "Utilities" heading repeats the category pop-up, and 40 % of the window is empty. Owned by the app-store agent: do this after they land.
- `apps/installer/tests/golden/ready-light.png`: the buttons sit bottom-left right after the content (`crates/apps/installer/src/view.rs:321`). The body is a flat stack of same-size lines in which "Version" and "Installs version" repeat each other, and the warning has no callout. Right-align the footer as an alert tray, put the metadata in a secondary style, and drop the duplicate.
- `apps/terminal/tests/golden/term-confirm.png`: the close confirmation is a bare strip with no hierarchy, and Close is not the emphasised default. Use the toolkit `Alert` sheet.
- `apps/terminal/tests/golden/term-window-dark.png`: a stray accent square shows in the first tab and the titles truncate early.
- `apps/file-chooser/tests/golden/save.png`: the sidebar has no symbols and the "Favourites" header is not the tracked small-caps header that Files uses. The path crumbs are in the accent colour. Reuse the Files sidebar style.
- `apps/monitor/tests/golden/inspector.png`: the window is too short, so the last row ("THREADS") is clipped, and the keys are tracked small caps where Files Get Info uses right-aligned sentence-case labels.
- `apps/files/tests/golden/inspector-file-dark.png`: the permission rows are clipped at the bottom with no scroll affordance. Section header styles differ from the sidebar's.
- `apps/settings/tests/golden/about-dark.png`: the Name field takes half the row while the other values are short and right-aligned.
- Fine: `term-settings`, `force-quit` (a loose inset only), `appearance-light` (minor slider label size).

## Milestones (ordered by visibility)

- **M1. Restart / Shut Down / Log Out as a dialog window.**
  - Toolkit `Role::dialog`: no title bar or control well, sized to its content (it refits when the stage or text size changes), fixed size (xdg min = max), and exposed to accessibility as a Dialog.
  - Compositor: never remember or recall a fixed-size window, so dialogs stop lending frames to each other.
  - The confirmation is an `Alert` whose title is the question and whose message is the countdown, with Cancel and a default Restart/Shut Down/Log Out that answers Enter. The blocked stage shows "X is keeping…" with a destructive "… Anyway"; the quitting stage shows "Quitting apps…" with Cancel.
  - Goldens: `shut-down-light`, `shut-down-blocked-dark`.
- **M2. The other shell prompts on the dialog window.** Bluetooth pairing as an Alert, About This Computer as a fixed info window with a label/value grid, About App fixed, Low Memory as a dialog window with a table fitted to its rows. Goldens: `pairing-*`, `low-memory-*`.
- **M3. Polkit fit and the greeter power prompt.** Fit the polkit height to its content; give the greeter's power confirmation the title plus countdown hierarchy. Goldens: `polkit/*`, `lock/login-power-shut-down`.
- **M4. App confirmations onto the toolkit Alert.** Terminal close confirmation, Package Installer footer and hierarchy, the alert sheet clamped inside its parent (`toolkit alert-light-1x`).
- **M5. Consistency pass across apps.** Section headers and label/value grids (Monitor inspector and Files Get Info), the file-chooser sidebar reusing Files', table column minimums, the Terminal tab indicator.
- **M6. Surfaces other agents own now**, done after they merge: notification centre margins and stack peek, the Software list layout, the Control Center insets, the switcher selection ring.

## Owner decisions

- **Dialogs have no title bar** (as macOS alerts). They cannot be moved by dragging; they open centred. Decided overnight 2026-11-01: no title bar, not movable (plan recommendation; owner may revisit).
- **Session confirmation has no scrim** and does not take the whole screen, so an app that blocks shutdown can still be reached to save. Decided overnight 2026-11-01: plain floating dialog kept in front (plan recommendation; owner may revisit).

## Status

M1 finished (merged). M2-M6 open.
