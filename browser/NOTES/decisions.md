# Owner's decisions

- **Base:** Debian's `firefox-esr`, followed from unstable; a scheduled check every 6 hours publishes when Debian has a newer one.
- **Accounts and Sync:** kept, off until sign-in. Push connects only once signed in.
- **Web push for sites:** off by default.
- **Extensions:** none preinstalled. Installing and updating from addons.mozilla.org must work.
- **Safe Browsing:** removed. No connection to Google remains, except Widevine (below).
- **Security data:** arrives only with browser updates. No background downloads.
- **Tracking Protection:** stays on. Mozilla's lists are packed into each build at build time.
- **Widevine:** stays in, off by default: no download and no connection until the user turns on DRM content in Settings or at a page's prompt. It is the single allowed exception to no connection to Google. Mozilla's update server offers it for Linux amd64 only, so it isn't available this way on arm64.
- **GTK:** the toolkit, used as a dependency and not extended.
- **Translation:** on-device, off by default, turned on in Settings. Until then no connection and no download; after that Remote Settings is allowed only for its models.
- **Search:** no default engine, asked on first launch, no partner codes.
- **Window frame:** tabs share a row with the control well the compositor draws. The compositor draws the well over the last 78 px of a client-side `org.bande.Browser` window and takes the clicks there; `theme/bande.css` copies the desktop's `title_bar.rs` numbers (`WINDOW_HEADER_HEIGHT` 48, `window_controls_width` 78, `MENU_GAP` 6, `HEADER_LEADING` 12). The well stays on the right in every locale, so the gap uses physical sides. Fullscreen has no well, so the restore button is the mouse way out.
- **App menu:** the ⋮ button left of the well, as in Bande's own apps.
- **Bookmarks Toolbar:** the favorites bar, shown in every window.
- **Updates:** from apt only; the browser's own updater is off.
