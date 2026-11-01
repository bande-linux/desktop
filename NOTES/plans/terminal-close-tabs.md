# Terminal: the window close button closes the whole window

Repo: apps (apps/terminal). No framework change needed.

## Cause
- The bande-app runner sends the titlebar close button to `Program::should_close`
  (desktop crates/libs/app/src/runner.rs:188). Terminal's `should_close`
  (apps/terminal/src/main.rs:237) calls `TerminalWindow::close_by_button`
  (src/app.rs:893), which closes the window only when one idle tab is left. Otherwise
  it calls `request_close_tab(active)` and returns false, so each click closes one tab.
- Close Window (menu and Shift+Ctrl+W; Terminal uses the shifted keys, menus.rs:10) goes
  first to `Program::close_tab` (runner.rs:237). Terminal's version (main.rs:228) closes the
  focused pane or tab through `request_close`, which matches Cmd+W in macOS Terminal. Leave it as it is.

## Why it was kept, and why it should change
- The rule comes from the Datum restyle (3229ed1, 2026-10-30): `should_close` checked
  `close_requested_by_button()` and closed only the active tab. The bande-app port
  (4757752, 2026-11-01) was meant to keep behaviour unchanged, so it moved that rule
  into `close_by_button`.
  The likely reason was caution: one click should not discard many shells, and the
  header tabs sit next to the window's x.
- macOS: the red close button closes the whole window. Terminal asks once ("Do you want
  to terminate running processes in this window?") only if something is running, and
  Cmd+W closes one tab. Each tab already has its own x (TabTarget::Close, app.rs:1276),
  so per-tab closing stays available. Files on bande-app already closes the whole window
  (default `should_close`), so Terminal is the odd one out. A click count that grows
  with the number of tabs is a bug, not a safety feature.

## Fix (apps/terminal/src/app.rs, locales)
1. Add `Pending::CloseWindow`. `ask()` labels its button "Close". In `close_tab`, the
   stale-banner match treats it as not stale.
2. `close_by_button`: busy = `busy_tabs()`. If busy == 0, call `forget_window()` and
   return true, closing all tabs at once. Otherwise call `ask(window, confirm-close-window(count),
   Pending::CloseWindow)`, `settle` and return false.
3. `confirm()` on CloseWindow closes every tab (loop `close_tab(window, 0)`). The last
   one calls `forget_window`, `finished()` becomes true, and `serve` (main.rs:164)
   closes the window. Restore then does not reopen it.
4. New string `confirm-close-window = Close this window? {count: one/other} program(s)
   still running` in en-US and the other locales, in the same style as confirm-quit.
5. Quit (`should_quit`/`ask_to_quit`) stays the same. If a CloseWindow banner is showing
   and the user then quits, `ask` replaces it.

## Risks
- A click now closes several idle shells with no prompt. This is the macOS behaviour,
  and the restore files for those tabs are dropped as for any closed window.
- A shell that is busy only in a split pane: `running_program()` per tab must cover
  all panes. Check this, and count by panes if it does not.

## Verify
Owner on the mini: open 3 tabs, one running `sleep 999`, and click x. It should ask
once, then the window should go after Close. With 3 idle tabs, one click should close
the window. Shift+Ctrl+W should still close one tab or pane, and the tab x should
still close one tab. Optional single e2e test: 2 idle tabs, close by button, window gone.
This is worth keeping only because it guards the routing in `close_by_button`.

## Size
About 30 lines plus one locale string per language. Compile-check with check-app.

## Owner decision
- Add a separate "Close Window" (macOS Shift+Cmd+W) to Terminal's File menu with no
  shortcut, since Shift+Ctrl+W is already Close Tab? Recommendation: yes, a menu item
  only, routed to the same path as the button. It is optional and can be done later.
