# web-html

The WHATWG HTML tokenizer and tree builder. It streams into a `TreeSink`, which `web-dom`
implements over its arena. All methods take `&mut self`; nodes are `Copy` handles.

The sink contract:
- `insert(parent, child, before)` inserts `child` (detaching it first if it has a parent)
  before `before`, or appends when `before` is `None`.
- `insert_text(parent, before, text)` appends to the text node just before the insertion point if
  there is one, else inserts a new text node there.
- `template_contents(template)` returns the template's contents fragment; the parser inserts into
  it, never into the template itself.
- `create_element` receives the final names: SVG and MathML case fixed, foreign attributes in
  the XLink, XML or XMLNS namespace, other attributes in `Namespace::None`.
- Names are atoms; the parser interns names that aren't static through `intern`.
