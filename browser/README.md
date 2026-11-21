# Bande's Browser

Bande's web browser: Debian's `firefox-esr` with telemetry, ads and sponsored content removed, and
Bande's look. Package `bande-browser`, app id `org.bande.Browser`.

This directory holds no browser source. It holds what we change in Debian's:

| Path | What it is |
|---|---|
| `patches/<esr>/` | our patches and `series`, applied after Debian's; `librewolf/` are from [LibreWolf](https://codeberg.org/librewolf/source) |
| `branding/` | name, icon, desktop file and metainfo, added with `--with-branding` |
| `firefox-esr-bande/` | the settings package's source; its `settings.js` holds the prefs both forms share, the second line of defence behind the removals |
| `prefs/bande.js` | prefs only the rebuild needs, added after `settings.js` |
| `search/` | search engines, with no partner codes |
| `theme/` | chrome CSS using Bande's design tokens |
| `scripts/` | the build, run the same way anywhere; `.github/workflows/browser-follow-debian.yml` runs it every 6 hours, on dispatch and on changes to `browser/` |

## Two forms

- **The rebuild**, `bande-browser` in Bande's archive: Debian's `firefox-esr` source with the patches,
  branding, prefs, search and theme here. Only this form removes code and has Bande's look.
- **The settings package**, `bande-firefox-esr-settings` from `firefox-esr-bande/`: Bande's prefs
  (`/etc/firefox-esr/bande-settings.js`) and enterprise policies
  (`/usr/share/firefox-esr/distribution/policies.json`) for Debian's own `firefox-esr`. It covers what
  a pref or policy controls; Remote Settings is off by policy, so translation is unavailable and
  security data arrives with `firefox-esr` updates. It can't carry the theme, the new tab and
  first-launch search pages, the menu button and title bar changes, the shipped Tracking Protection
  lists (Debian's browser fetches them) or search engines without partner codes. An administrator's
  `/etc/firefox/policies/policies.json` replaces its policies. Build it with
  `cd firefox-esr-bande && dpkg-buildpackage -us -uc -b`.

## Build

```
BROWSER_JOBS=3 scripts/local-build.sh "$(scripts/debian-version.sh)" /var/tmp/bande-browser
```

With thin Rust LTO, 3 jobs peak at about 6.5 GB of memory; `BROWSER_RUST_LTO=-Clto=off` is for quick
test builds, and `scripts/mem-peak.sh` measures a build.

`local-build.sh` needs no root: a Debian chroot (`SUITE`, default forky) from `mmdebstrap --mode=unshare`. It fetches
unstable's source through apt, runs `prepare.sh` and Debian's own build. `prepare.sh` exits 3 if a
patch no longer applies, leaving quilt at it.

## Changing a patch

In the prepared tree, `export QUILT_PATCHES=debian/patches QUILT_PC=.pc`, then `quilt` as usual:
`quilt pop <patch>`, edit, `quilt refresh`, and copy it back from `debian/patches/bande/`.

## Upgrading to a new ESR

- Re-check the `customtitlebar` uses (`grep -rl customtitlebar= browser toolkit --include=*.xhtml`, not tests).
  Only `browser.xhtml` may keep one: the compositor draws its control well over every client-side
  `org.bande.Browser` window, so any other window that draws its own title bar would get the well
  on top of it. `patches/<esr>/bande/no-custom-titlebars.patch` removes the known two.
- Windows that aren't the main browser window must carry their own `windowclass`, or they get the well too.
  `scripts/drive.py` drives the browser through Marionette under `scripts/session.sh` and asks for
  screenshots, to look at each window.
- Re-run the network capture (`scripts/capture.sh`, with `--live` for a browsing run) and go through
  what the new ESR added; `scripts/scan-hosts.py` lists every Google host its code and prefs name.

## Licence

Patches, branding, the new tab and rights pages, and anything from Firefox or LibreWolf: MPL 2.0.
Our own scripts and prefs: MIT OR Apache-2.0. The metainfo's metadata: CC0-1.0. `prepare.sh`
adds Bande's files to the package's `debian/copyright`. Mozilla and Firefox are trademarks of the Mozilla Foundation; this browser is
based on their source and is not made or endorsed by Mozilla.
