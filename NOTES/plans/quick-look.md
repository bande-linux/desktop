# Quick Look: what is left

Ranked, most important first. Everything else from the macOS comparison is done (see
`docs/architecture/behaviour.md`, Quick Look).

1. **No media playback.** Audio and video show a large icon, kind, size and date; no player, no
   duration, no video frame (needs a media stack).
2. **No Markup** for pictures and PDFs.
3. **Share is Copy only**: there is no share service to list.
4. **Files' panel grows in place** instead of zooming out of the item's icon: a client does not
   know where its icon is on the screen, so the compositor would have to animate the panel's
   window from a rectangle the app hands it.
