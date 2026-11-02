# web-content

The renderer pipeline: parse (`web-html`) into the arena DOM (`web-dom`), compute styles
(`web-style`), build the box tree and shape text (`web-layout`, `web-text`), lay out into a
fragment tree, and paint a display list (`web-paint`). It runs in process for now and becomes the
per-site renderer's library in W4.

The entry points below are stable; the app and `tools/conformance` drive them.

- `render_html(html: &str, width: f32, fonts: &mut Fonts) -> Page` lays a document out at a
  viewport width in CSS px. `parse(bytes)` alone sniffs the encoding and returns the `Document`;
  `parse_with_charset(bytes, Some(label))` passes the HTTP `charset` as the transport encoding;
  `Page::new(document, width, fonts)` renders a parsed one.
- Stylesheets: `<style>` applies at once. `stylesheet_links()` lists the `<link rel=stylesheet>`
  and `@import` hrefs still missing (imports resolved against their sheet); the app fetches each
  and calls `add_stylesheet(href, bytes, fonts)`, which restyles and lays out again.
  `load_stylesheets_from(directory, fonts)` does the same from disk. `with_viewport` sets the
  height that `vh` and media queries see (`new` assumes 4:3).
- Images: `image_links()` lists `<img>` sources and CSS `background-image` URLs still missing.
  URLs from a stylesheet are already rebased onto the sheet's href (as passed to
  `add_stylesheets`), so the app resolves every link against the document. `add_images(&[(link,
  bytes)])` decodes them (PNG, JPEG, WebP, GIF) into premultiplied RGBA shared through `Arc` and
  lays out again. The page keeps each image's encoded bytes; pixels exist only for displayed
  images, resampled to their display size at 2x (backgrounds stay at natural size), and are
  decoded again when that size changes. The display list's `images` hold the same `Arc`s, each
  with a stable `id`. `release_image_pixels()` swaps them for pixel-less copies with the same ids,
  for a renderer that has already sent the pixels.
- Web fonts: `font_links()` lists `@font-face` `src` URLs still missing, only for faces whose
  family the page uses and whose `unicode-range` covers its text, and the next source once one
  fails. Sheet URLs are rebased like images. `add_fonts(&[(link, bytes)], fonts)` registers WOFF2,
  WOFF, TTF or OTF for this page only and lays out again; text shows in fallback fonts until then.
  A page holds at most 64 MB of fonts (`memory().fonts`), and a font decodes to at most 32 MB.
  `load_fonts_from(directory, fonts)` does the same from disk. `release(fonts)` drops the page's
  fonts from the shared `Fonts`; call it before dropping the page.
- Hover: `hover_at(x, y, fonts)` or `set_hover(node, fonts)` restyle only what `:hover` rules
  can reach and return whether to repaint; `restyled()` counts the elements restyled.
- `Fonts::new()` scans the system fonts once; keep one per thread and reuse it across pages.
- `Page` holds the document, styles, box tree and `display_list`, an `Arc` the app shares
  instead of copying; the page hit-tests the same list. The fragment tree lives only during
  layout; the page keeps each node's top for `fragment_y`.
  - `resize(width, fonts)` lays out again without restyling or reshaping.
  - `link_at(x, y)` is the `href` of the link under a point in document coordinates.
  - `fragment_y(name)` is the top of the element a `#name` link points to (an `id`, or an
    `a name`), for scrolling.
  - `title()` and `memory()` (heap bytes per stage) report on the page.
- `DisplayList` is in document coordinates (CSS px, y down). `items` paint in order:
  - `Item::Rect { rect, color }` fills.
  - `Item::Glyphs { rect, baseline, node, font, size, color, glyphs }` draws `glyphs` (x
    absolute, y relative to `baseline`) with `fonts[font]`, a `FontData` that vello and the
    toolkit draw directly. `hit_test(x, y)` returns the text node under a point.
  - `Item::Image { rect, clip, image }` draws `images[image]` (premultiplied RGBA) scaled to
    `rect` and clipped to `clip`; `Item::Tiles { tile, clip, image }` repeats it from `tile`
    across `clip`.
  - `PushClip { rect }`, `PushOpacity { alpha }` and `Pop` nest layers.
- With the `raster` feature:
  - `render_file(path, width, height) -> io::Result<Pixels>` renders a file headless into RGBA8
    pixels. Calls are independent and may run on several threads; each thread keeps its own
    `Fonts`.
  - `DisplayList::rasterize(scroll_y, width, height)` rasterizes through vello's CPU renderer.

`cargo run -p web-content --features raster --example render-png -- page.html out.png [width]`
renders a whole page to PNG for visual checks and prints bytes per element for each stage. Keep its
images out of the repository.
