# Owner's decisions

- **Base:** Debian's `firefox-esr`, followed from unstable; a scheduled check every 6 hours publishes when Debian has a newer one (approved exception to CI-only-on-tags).
- **Accounts and Sync:** kept, off until sign-in. Push connects only once signed in.
- **Web push for sites:** off by default.
- **Extensions:** none preinstalled. Installing and updating from addons.mozilla.org must work.
- **Safe Browsing:** removed. No connection to Google remains; Widevine and DRM video go with it until the owner decides otherwise.
- **Security data:** arrives only with browser updates. No background downloads.
- **Translation:** on-device, off by default, turned on in Settings. Until then no connection and no download; after that Remote Settings is allowed only for its models.
- **Search:** no default engine, asked on first launch, no partner codes.
- **Window frame:** tabs share a row with the control well the compositor draws.
- **GTK 3:** still open with the owner; kept for now.
