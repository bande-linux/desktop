# Resume: work comes back after a crash, log out or restart

Audit gap 2, branch `fix-restore`.

## What the user experiences

- **Log out, restart, shut down**: the confirmation has "Reopen windows when logging back in", remembered as the next default. At next login every app returns with each of its windows: display, Space, frame, tiled, zoomed, full screen or minimised state, stacking, tabs, the folder or document, and the app's own state.
- **After a crash**: the next login asks "Reopen your apps?", listing them, before anything starts. An app that brought the session down does not come straight back unasked.
- **One app crashes**: its next launch comes back as it was. If it dies while reopening, its next launch asks once before trying again.
- **Quit** is a decision: windows are closed and the next launch is fresh. **Closing a window** removes it for good.
- **Unsaved documents survive** everything: TextEdit keeps edited and untitled documents on disk as they change, and keeps older versions to go back to.
- **Third-party apps**: reopened from their desktop entry, windows back on their display, Space and frame. Apps that speak `xdg-session-management-v1` get this per window by name; others are matched by app id, tag and title, in order.

## Design

Two halves, as AppKit and the WindowServer split it.

1. **Compositor owns window management state.** It implements the standard `xdg_session_manager_v1`. Each toplevel in a session is a record: session id, name, app id, tag, display identity, Space position, frame to return to, tile, zoom, full screen, minimised, stacking rank. Records live in `$XDG_STATE_HOME/bande/window-sessions`, written from a worker thread after a change, so a compositor crash loses at most a second. Closing a named window (`remove_toplevel`) or `remove` drops the record; losing the client does not. Windows without a session get an implicit one by app id, used only for apps the shell says it is reopening. Records win over window memory (`recall`), which remains for new windows. Spaces are recreated as needed; a missing display falls back to the display under the pointer and the frame is clamped into it.
2. **`bande-app` owns the app's state.** It binds the session, names each window, and keeps `$XDG_STATE_HOME/bande/resume/<app id>`: session id, plus each window's kind and the app's saved blob. Written on a worker thread after any change, throttled; no wakeups while idle.

```rust
fn save_window(&self, cx, window) -> Option<Saved>;      // default: main windows, empty state
fn restore_window(&mut self, cx, saved: Saved);          // default: self.reopen(cx)
```

`start` builds the program and opens only what its arguments ask for. With no arguments, `bande-app` restores the saved windows, and calls `reopen` if there were none. So an app with no window state of its own gets frames, Spaces and window count with no code; an app with more serialises a small struct. The Settings… window is handled by `bande-app` itself.

Keeping or dropping the state: user Quit deletes the file and removes the session. `bande_app_v1.session_ending`, sent before `quit_requested` at log out, and a lost display both leave it. A marker file set while reopening catches an app dying in restoration.

**Session flow.** Apps of a clean end are in `bande/reopen`, of an unclean end in `bande/crashed` (both already exist). `bande-shell` at start: clean list means reopen; crashed list means the prompt, answered before anything launches; an unticked or declined reopen means every `bande/resume` file is deleted.

The config key `reopen-apps-after-crash` goes: asking replaces it. `reopen-apps` is the checkbox's memory.

## Why this over the alternatives

- **The standard protocol over a private one**: same work, and Qt and others that speak it benefit. Its model (compositor keeps state, client keeps the id, remove on close) is what is needed.
- **App sends frames to the compositor** (a private "place me here"): loses state on compositor crash, needs every change echoed to clients.
- **Compositor-only restoration**: cannot bring tabs or folders back.
- **Checkpointing processes** (CRIU): fragile with GPU and sockets; nobody ships it for desktops.
- **`start` keeps opening the default window, restored windows replace it**: flashes a window, and each app needs its own branch. `reopen` already is the default window.

## Stages

1. Compositor: protocol, store, restoring placement, implicit sessions.
2. `bande-app` API and toolkit; Files and Settings.
3. Shell: prompt, checkbox, reopen flow, config.
4. Apps repo: bump, every app, Terminal and TextEdit and Browser moved to the API.
5. TextEdit versions; Terminal scrollback.
