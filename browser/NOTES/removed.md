# What is removed, and what stays

**Removed from the package:**
- Components replaced by stubs at build time by `scripts/gut.py`: the message and promo system (asrouter), the VPN (ipprotection, both parts), the AI chatbot and AI window (genai, aiwindow), report-broken-site.
- Turned off at build time: the crash reporter, the updater, Normandy studies and pingsender.
- Gone entirely: the built-in new-tab add-on, which carried sponsored tiles, stories and top sites, replaced by our own page; `about:telemetry` and `about:glean`; Safe Browsing's Google providers; Google geolocation.
- Gone from Settings: data collection and the default-browser prompt.

**Stays, and why:**
- **Telemetry and Glean calls** in about 740 files, and the libraries behind them. Deleting every call would conflict with each ESR update. They cannot send: upload is off at build time, and the uploaders and ping sender are gone.
- **Nimbus:** many features read their settings through it. Remote fetching is gone, so it serves only built-in defaults.
- **The stubbed modules' text strings:** they show nowhere.
- **Telemetry in about:support:** a few lines remain there.
