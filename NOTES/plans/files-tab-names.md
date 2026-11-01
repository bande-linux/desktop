# Plan: Files tab names skewed (files-tab-names)

## Cause
`crates/apps/files/src/tabs.rs` lays tabs out correctly (equal widths, `tab_rects`), but draws
each title with `centred()`, which builds a *wrapping* paragraph:
`text().paragraph(text, style, Some(area.width), TextAlign::Center)` (tabs.rs `centred`).
The label area is `tab.width - 2*LABEL_INSET` (52 px reserved for the close button on both sides).
- When a title is wider than that area, parley wraps it onto 2+ lines. `centred()` then centres
  the multi-line block vertically and clips it to the 28 px label rect, so you see the middle
  line(s) or the bottom/top halves of two lines: text shifted up/down and cut.
- A single word that cannot break overflows the max width starting at the left edge, so it is
  no longer centred and is cut off on the right (looks shifted right).
- With more tabs the label area shrinks (6 tabs in a 1000 px window: ~70 px), so the problem
  grows with tab count; at very narrow widths the area hits 0 and every glyph wraps.
- No "…" truncation exists anywhere in this path.
Not reproduced as an image (no headless PNG path for FilesView without the compositor review
harness); cause follows directly from the code. Terminal (apps repo, `apps/terminal/src/render.rs`)
has its own tab bar that already truncates with "…" and is not affected; nothing is shared.

## Fix (macOS Finder reference)
Finder: equal-width tabs filling the bar, title on one line, centred in the tab, tail-truncated
with "…"; close "x" appears on hover at the left and the title stays centred in the whole tab.
1. Toolkit: add one single-line helper on `TextSystem` (crates/toolkit/graphics/src/text.rs),
   e.g. `fitted(text, style, max_width) -> Paragraph`: lay out with `max_width = None`; if too
   wide, binary-search the longest char prefix + "…" that fits (same algorithm as
   `crates/shell/launcher/src/rows.rs::fitted`, which then delegates to it).
2. Files `tabs.rs`: replace `centred()` with: `fitted(title, style, label.width)`, then position
   at `x = tab.x + (tab.width - p.width())/2` clamped to `label` (centre in the whole tab, as
   Finder does), `y` centred on the single line, rounded to pixels. Keep the clip as a guard.
3. Use a symmetric inset of ~CLOSE_INSET+CLOSE_SIZE+4 (26 px) as today so the centred title
   never runs under the hover close button.
Optional follow-up (not in this change): move launcher and Terminal truncation to the helper.

## Risks
Low. Binary search costs a few extra layouts per tab per frame; cache per (title, width) if the
bench shows it (unlikely for <20 tabs). RTL titles: truncation is by chars at the end, fine for
now (matches launcher).

## Verification
`desktop-check`; owner opens Files, Cmd-T several times into folders with long names (and 8+
tabs), checks titles sit on one line, centred, ending in "…", and stay put on hover. No e2e test
(pure painting; existing `tab_titles` test covers the strings).

## Size
~40 lines (helper ~25, tabs.rs ~15 net), one commit. No owner decision needed.
