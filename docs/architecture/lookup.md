# Dictionary and Look Up

`bande-lookup` (`crates/apps/lookup`, app id `org.bande.Lookup`, shown as Dictionary) is both the
Dictionary app and Look Up, as Dictionary.app is on a Mac:

- **Dictionary window**: a search field; local sources answer as you type, online sources once
  typing pauses.
- **Look Up**: started by D-Bus (`org.freedesktop.Application.Open` with the text, optionally
  `--language=<tag>` first) from a text field's or label's menu, Files, Terminal, the Browser or
  `Ctrl+Super+D`. It shows a popover at the pointer (`Kind::Popover`, `bande_app_v1.set_popover`)
  and exits when it closes (Esc or focus lost) unless the Dictionary window was opened. If no
  request arrives within 30 seconds of a D-Bus start it exits. Nothing runs at idle.
- **Dictionary Settings**: the sources as rows, each checkable and ordered (Move Up, Move Down),
  with its languages and, for an online source, that each word looked up is sent there. Look Up
  and Dictionary show one section per enabled source with a hit, in this order; the popover shows
  the first section and the first of each other kind, and More shows them all.

Lookups run on worker threads. Online sources are asked only when they are enabled.

## Sources

A source is a file in `<XDG data dir>/bande/dict/`: a `.bdict` pack (offline data) or a `.source`
file (an online source: the same metadata, no data). The first with an `id` wins. Packaged sources
are in `/usr/share`, the user's own in `~/.local/share`; they are treated the same, and the user's
can be removed in Settings.

No language and no source is named in the code. A source is on by default when it serves one of
the user's languages (their language list and their region's language), unless it says
`default=on|off`. The default order puts sources whose text is in the user's languages first, then
definitions before translations, thesauruses and encyclopedias, then the source's `priority`. The
user's choices are in `$XDG_CONFIG_HOME/bande/dictionary.conf` (`order=`, `on=`, `off=`, ids
separated by spaces).

Matching within a source: the forms table, then the source's `rules.<language>`, then for a
compounding language the longest known ending ("husvagns + ägare", marked as derived).

## Adding dictionaries

Dictionary Settings ▸ Add Dictionary… (the file chooser portal), or a file dropped on the window,
imports a StarDict dictionary (`.ifo` with `.idx` or `.idx.gz` and `.dict` or `.dict.dz`, an
optional `.syn`) or a dictd one (`.index` with `.dict` or `.dict.dz`), or a tarball of either
(unpacked with `tar`). It is converted on a worker thread into a pack in
`~/.local/share/bande/dict/`, serving any language (`*`); markup is reduced to text. A DICT server
(`dict://host[:port]/database`, RFC 2229, unencrypted) is checked and added as a `.source` with
`protocol=dict`. An unsupported or broken file gets a message naming it. Apple `.dictionary`
bundles are not imported: their `Body.data` format is undocumented and changes between macOS
versions, and the dictionaries Apple ships are licensed for macOS only.

## Pack format

All integers are little-endian `u32` unless noted; offsets are from the start of the file.

| Part | Content |
| --- | --- |
| Magic | `BDICT1\n\0` |
| Metadata | length, then `key=value` lines in UTF-8 |
| Index | number of forms, then one offset per form, sorted by the bytes of the key |
| Form record | key `<language>:<lowercase form>`, a zero byte, `u16` entry count, entry offsets best first |
| Entry | length, then UTF-8 lines: word, part of speech, then `<kind>\t<text>` |

Metadata keys: `id`, `name`, `attribution` (shown under each section), `languages` (tags, or `*`),
`definitions.<language>` (the language the text is written in), `kind.<language>` (`definition`,
`translation`, `thesaurus`, `encyclopedia`; `*` for every language), `priority` (higher first),
`default`, `rules.<language>` (`pos:ending:replacement` items tried on the first word when a form is
not in the table, kept only when the base has that part of speech), `compound.<language>`
(shortest ending to try). A `.source` file adds `protocol` and `url` (`{language}` and `{word}` are
filled in); the protocols are `wikimedia-summary` and `dict`. Line kinds: `D` definition,
`T` translation, `S` synonyms, `E` example, `O` form of another word of the pack. Parts of speech
`noun`, `verb`, `adj` and `adv` are translated; any other is shown as it is.

## Packaged sources

`packaging/dict/build.py <version> <sources> <out> [package...]` makes the packs and `.deb`s, with
their licence files, from the source data pinned by SHA-256 in `packaging/dict/sources`. The build fetches each source
from our own copy first (through the GitHub API, then the release download URL), the `<sha256>.xz` assets of the `dict-sources` release of
`bande-linux/desktop` (named by content, so they are only ever added),
and from the upstream URL only when ours lacks it; a file that does not match its pin stops the
build, so an upstream outage or a file replaced in place cannot change or break a release. To
take new upstream data: `build.py --pin <sources>` downloads it, rewrites the pins and leaves
`<sha256>.xz` copies to add to `dict-sources` (`gh release upload dict-sources <files>`); commit the pins to `main`, then push the tag `dict-YYYYMMDD` (the packages' version). `dict.yml` builds
them from that tag and uploads `bande-debs-all`, which the APT archive's `look.yml` picks up and
publishes to `main`, as it does the desktop's releases.

| Package | Sources | Licence |
| --- | --- | --- |
| `bande-dict-en` | English Dictionary and English Thesaurus, from Open English WordNet 2025 | CC BY 4.0 |
| `bande-dict-sv` | Folkets lexikon (Swedish↔English); Svenska synonymer (Synlex, from Debian's `mythes-sv`) | CC BY-SA 2.5; Synlex's own free licence |
| `bande-dict-sv-wiktionary` | Svenska ord: Swedish words from Wiktionary through kaikki.org | CC BY-SA 4.0 |
| `bande-dict-en-wiktionary` | English Wiktionary, optional and off by default (41 MB download, 168 MB) | CC BY-SA 4.0 |

`bande-lookup` ships `wikipedia.source` (Wikipedia, every language, on by default; the row says that
each word looked up is sent to Wikipedia).
