# web-html

The WHATWG HTML tokenizer and tree builder (the current Living Standard, including processing
instructions and the customizable `select` parsing). It streams into a `TreeSink`, which `web-dom`
implements over its arena. An XML parser for XHTML feeds the same sink.

## API
- `Parser::new(sink, Options)` parses a document; `Parser::new_fragment(sink, context, Options)`
  parses a fragment for `innerHTML` (the result is the children of the sink's root `html`).
- `feed(&[u8])` takes network bytes: it sniffs the encoding (BOM, then the transport encoding,
  then a prescan of the first 1024 bytes, then windows-1252) and decodes with `encoding_rs`.
  `feed_str(&str)` takes text that is already decoded. `finish()` ends the input and returns the
  sink. Input may arrive in chunks of any size; tokens and nodes are produced as it arrives.
- `reparse_encoding()` is `Some` when a `<meta>` charset past the prescan contradicts a tentative
  encoding: the caller reparses the bytes with that encoding as `Options::transport_encoding`.
- `sniff_encoding(head, transport)` exposes the sniffing alone; `tokenize(text, state,
  last_start_tag, atoms, each)` runs the tokenizer alone, for the conformance runners.
- `Options::scripting` is the scripting flag (it decides how `noscript` parses).
- `parse_xml(sink, bytes, transport_encoding)` parses an XML document (XHTML: `.xht`, `.xhtml`,
  `application/xhtml+xml`) into the same sink. It takes the whole document at once; the encoding
  is the BOM's, then the transport encoding, then the XML declaration's, then UTF-8, and bytes
  invalid in that encoding are an error.

## XML
XML 1.0 well-formedness with Namespaces: CDATA sections, comments, processing instructions, the
predefined entities and character references, and the HTML named character references when the
DOCTYPE's public identifier is one the HTML standard lists for the XML parser. Namespace URIs map
to `Namespace`; any URI it doesn't name is `Namespace::Other`. A `template`'s children go to its
contents. DTDs are never read: an internal subset is skipped, external entities are never
fetched, and with a DTD present an undeclared entity expands to nothing (otherwise it is an
error). Expansion is at most two characters per reference, and elements nest at most 512 deep.
On the first well-formedness error, parsing stops and the document's children are replaced by a
`parsererror` element whose text names the error, line and column, as in Firefox.

## Limits
The stack of open elements is capped at 512, as in Blink: a start tag that would nest deeper first
closes the deepest element, so the tree is at most 512 deep and every scope check is bounded. Open
HTML elements are counted per name, so scope checks for names that aren't open cost nothing.

## The sink contract
All methods take `&mut self`; nodes are `Copy` handles.
- `insert(parent, child, before)` inserts `child` (detaching it first if it has a parent)
  before `before`, or appends when `before` is `None`.
- `insert_text(parent, before, text)` appends to the text node just before the insertion point if
  there is one, else inserts a new text node there.
- `template_contents(template)` returns the template's contents fragment; the parser inserts into
  it, never into the template itself.
- `create_element` receives the final names: SVG and MathML case fixed, foreign attributes in
  the XLink, XML or XMLNS namespace, other attributes in `Namespace::None`.
- Names are atoms; the parser interns names that aren't static through `intern`.

## Not yet
Script execution (`document.write`, parser pausing at `</script>`), form-owner association,
declarative shadow roots, template patching (`<template for>`) and the parser sanitizer. The
`selectedcontent` mirroring of the chosen `option` is DOM behaviour for `web-dom`. For XML:
entities declared in an internal subset, and streaming input.

## Fuzzing
`fuzz/` is a cargo-fuzz project with the targets `tokenizer`, `tree_builder` and `xml`. It builds on the
stable toolchain with `cargo fuzz build -s none -O -a`; corpora live in
`~/.cache/bande-web/fuzz/web-html/<target>`.
