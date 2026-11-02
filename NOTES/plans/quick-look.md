# Quick Look: what is left

Ranked, most important first. Everything else from the macOS comparison is done (see
`docs/architecture/behaviour.md`, Quick Look).

1. **Preview still keeps its own decoders.** `bande-linux/apps` Preview has copies of what is now
   `bande-content` (`decode.rs`, `facts.rs`, `pdf/{client,protocol,session,worker,outline}.rs`).
   Once the apps pin includes `bande-content`, depend on it (rename the key: the app's own crate is
   also called `bande-preview`), delete the copies and keep `run_pdf_worker` from the library.
2. **Files' panel is inside the Files window.** It cannot be larger than the window and its full
   screen fills the window, not the display. A separate floating panel needs a non-activating
   toplevel role (keyboard stays with the browser window) from the compositor and `bande-app`.
3. **No media playback.** Audio and video show a large icon, kind, size and date; no player, no
   duration, no video frame (needs a media stack).
4. **No Markup** for pictures and PDFs.
5. **Share is Copy only**: there is no share service to list.
6. **Hostile fonts and archives are read in the host process.** PDF is confined; fonts are parsed
   in Files or the shell, and counting a compressed tar decompresses it on a background thread
   (bounded by entries, not bytes). Move both into the confined child.
7. **Toolbar buttons are not separate accessibility nodes** (the panel is a dialog that announces
   the item).
8. **The panel cannot be moved**, only resized; locked PDFs say so instead of asking for the
   password; plain text does not wrap (long lines scroll sideways).
