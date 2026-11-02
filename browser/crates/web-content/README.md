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
- `Fonts::new()` scans the system fonts once; keep one per thread and reuse it across pages.
- `Page` holds the document, styles, box tree, fragment tree and `display_list`.
  - `resize(width, fonts)` lays out again without restyling or reshaping.
  - `link_at(x, y)` is the `href` of the link under a point in document coordinates.
  - `fragment_y(name)` is the top of the element a `#name` link points to (an `id`, or an
    `a name`), for scrolling.
  - `title()` and `memory()` (heap bytes per stage) report on the page.
- `DisplayList` is in document coordinates (CSS px, y down). `items` paint in order:
  - `Item::Rect { rect, color }` fills.
  - `Item::Glyphs { rect, node, font, size, color, glyphs }` draws `glyphs` (absolute
    positions, baseline origin) with `fonts[font]`, a `FontData` that vello and the toolkit draw
    directly. `hit_test(x, y)` returns the text node under a point.
- With the `raster` feature:
  - `render_file(path, width, height) -> io::Result<Pixels>` renders a file headless into RGBA8
    pixels. Calls are independent and may run on several threads; each thread keeps its own
    `Fonts`.
  - `DisplayList::rasterize(scroll_y, width, height)` rasterizes through vello's CPU renderer.

`cargo run -p web-content --features raster --example render-png -- page.html out.png [width]`
renders a whole page to PNG for visual checks and prints bytes per element for each stage. Keep its
images out of the repository.
