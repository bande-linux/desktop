# Plan: taskbar logo "U" -> "B"

## Cause
The taskbar's system-menu mark is hard-coded SVG, not an asset or icon-theme icon:
`crates/shell/bar/src/symbols.rs:56-61` (`Symbol::Logo`): a 14x14 rounded square (rx 4.2) in the
bar text colour with a "U" stroke path masked out (`<mask id="u">`, path `M5.2 4.6V8.6A2.8 2.8 0 0 0
10.8 8.6V4.6`, stroke 1.9). Size is at :116 (15x15); used at `menu_bar.rs:408` (`Slot::System`).
It is rendered as a vector in `color`, so light/dark and every scale factor already work.

## Other places checked
- Greeter, lock screen, About (settings-panes), launcher, Plymouth/splash, icon theme
  (`data/icons`, no `distributor-logo`): no logo mark anywhere. Nothing to change.
- No Bande "B" asset exists in desktop, apps, images, installer or the website.
- Website: `bande-linux.github.io/img/taskbar.png` (and any screenshot showing the bar top) shows
  the "U"; re-take after the fix.
- `installer/logo.icns` (from the initial commit, used by `build.sh` for the macOS-side installer
  volume icon) is inherited; could not render it here. Check what it shows; if it is not Bande,
  replace it with an icns built from the same SVG (16-1024 px, @1x/@2x).

## Fix
In `symbols.rs`, replace only the masked path with a "B" knocked out of the same rounded square,
same stroke weight 1.9 and round caps/joins, so it matches the existing style. Draft:
`M5.6 4.2V11.8 M5.6 4.2H8.6A1.9 1.9 0 0 1 8.6 8H5.6 M5.6 8H9A1.9 1.9 0 0 1 9 11.8H5.6`
(stroke-linejoin round; the lower bowl slightly wider, as in a typeset B). Rename mask id to "b".
Tune visually at 1x and 2x on the mini so the counters don't close up at 15 px.

## Owner decision
No designed Bande logo exists. Recommendation: draw the simple "B" in code now (above, ~5 lines),
matching the old "U" mark; it is a vector so needs no size/variant assets. If a designed mark comes
later, keep one master SVG in `data/` and generate the bar path, website favicon/screenshots and
installer icns from it.

## Risks
Very low: one path string. Legibility at 15 px 1x is the only concern.

## Verification
Owner looks at the top of the taskbar in light and dark mode on the mini. No test.

## Size
~5 lines in desktop; plus website screenshot refresh and possibly installer icns.
