# Owner's decisions

- **Base:** Debian's `firefox-esr`, followed from unstable; a scheduled check every 6 hours publishes when Debian has a newer one (approved exception to CI-only-on-tags).
- **Accounts and Sync:** kept, off until sign-in. Push connects only once signed in.
- **Web push for sites:** off by default.
- **Extensions:** none preinstalled. Installing and updating from addons.mozilla.org must work.
- **Safe Browsing:** removed. No connection to Google remains; Widevine and DRM video go with it.
- **Security data:** arrives only with browser updates. No background downloads.
- **Tracking Protection:** stays on. Mozilla's lists are packed into each build at build time.
- **Widevine:** out by default until the owner answers.
- **GTK:** this build stays on GTK as the interim; the Bande platform layer that replaces it is on `gap-firefox-bande-widget`.
- **Translation:** on-device, off by default, turned on in Settings. Until then no connection and no download; after that Remote Settings is allowed only for its models.
- **Search:** no default engine, asked on first launch, no partner codes.
- **Window frame:** tabs share a row with the control well the compositor draws.
