# Bande's Browser

Bande's web browser: Debian's `firefox-esr` with telemetry, ads and sponsored content removed, and
Bande's look. Package `bande-browser`, app id `org.bande.Browser`.

This repository holds no browser source. It holds what we change in Debian's:

| Path | What it is |
|---|---|
| `patches/<esr>/` | our patches and `series`, applied after Debian's; `librewolf/` are from [LibreWolf](https://codeberg.org/librewolf/source) |
| `branding/` | name, icon, desktop file and metainfo, added with `--with-branding` |
| `prefs/bande.js` | locked prefs, the second line of defence behind the removals |
| `search/` | search engines, with no partner codes |
| `theme/` | chrome CSS using Bande's design tokens |
| `scripts/` | the build, run the same way anywhere |

The old engine is in this repository's history, where `apps/browser` still pins it.

## Build

```
BUILD_NO_MINI=1 BUILD_FETCH=/var/tmp/bande-browser/out BUILD_TIME_LIMIT=21600 build-run . \
    env BROWSER_JOBS=3 scripts/local-build.sh "$(scripts/debian-version.sh)" /var/tmp/bande-browser
```

Memory: with thin Rust LTO and 3 jobs the peak is 6.5 GB of real memory (48 min on 8 CPUs). 5 jobs was
killed at the 10.2 GB limit of a 13 GB build machine. `BROWSER_RUST_LTO=-Clto=off` is for quick test
builds; `scripts/mem-peak.sh` measures a build.

`local-build.sh` needs no root: a Debian forky chroot from `mmdebstrap --mode=unshare`. It fetches
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
- Re-run the network capture and go through what the new ESR added.

## Licence

Patches, branding text and anything from Firefox or LibreWolf: MPL 2.0. Our own scripts:
MIT OR Apache-2.0. Mozilla and Firefox are trademarks of the Mozilla Foundation; this browser is
based on their source and is not made or endorsed by Mozilla.
