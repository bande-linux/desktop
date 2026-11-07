# Look Up

`bande-lookup` (`crates/apps/lookup`) shows what a word means in a popover at the pointer. It is
started by D-Bus (`org.bande.Lookup`, `org.freedesktop.Application.Open`) with the text as the
argument and optionally `--language=<tag>` first, and exits when the popover closes (Esc or focus
lost). If no request arrives within 30 seconds of starting it exits. Nothing runs at idle.

The popover is `Kind::Popover` in `bande-app`, `bande_app_v1.set_popover` in the compositor: it maps
at the pointer, takes the keyboard, is on top, is not in the Taskbar and is not resumed. Lookups run
on a worker thread and go over the network never.

## Languages

No language and no pack is named in the code. Every installed pack is asked, in every language it
covers; adding German means installing a pack. Entries are never merged: each pack and language
with a hit is its own named section. The popover shows the best section and the best of each other
kind (a thesaurus, say), and More shows every section. Sections are ranked by exact headword over
inflected form over compound part, then definitions in the UI language, then kind (definition,
translation, thesaurus), then the user's order or else the pack's priority, then the word's
language (the hint, then the user's languages). A section identical to one above is hidden.

## Pack format

Packs are found at runtime in `<XDG data dir>/bande/dict/*.bdict`; the first pack with an `id` wins.
All integers are little-endian `u32` unless noted; offsets are from the start of the file.

| Part | Content |
| --- | --- |
| Magic | `BDICT1\n\0` |
| Metadata | length, then `key=value` lines in UTF-8 |
| Index | number of forms, then one offset per form, sorted by the bytes of the key |
| Form record | key `<language>:<lowercase form>`, a zero byte, `u16` entry count, entry offsets best first |
| Entry | length, then UTF-8 lines: word, part of speech, then `<kind>\t<text>` |

Metadata keys: `id`, `name`, `attribution` (shown in the panel), `languages` (space separated tags),
`definitions.<language>` (the language its text is written in), `kind.<language>` (`definition`,
`translation`, `thesaurus`), `priority` (higher first),
`rules.<language>` (`pos:ending:replacement` items tried on the first word when a form is not in
the table, kept only when the base has that part of speech), `compound.<language>` (shortest ending
to try; present means the language compounds and the longest known ending is shown, marked as
derived). Line kinds: `D` definition, `T` translation, `S` synonyms, `E` example, `O` form of
another word of the pack. Parts of speech `noun`, `verb`, `adj` and `adv` are translated by the
panel; any other is shown as it is.

A form's entries are ordered by the builder, so `ran` reaches only the verb `run`, and WordNet's
entries follow its sense counts.

## Packs

`packaging/dict/build.py <version> <sources> <out> [package...]` downloads the source data and makes
the packs and `.deb`s with their licence files:

| Package | Source | Licence |
| --- | --- | --- |
| `bande-dict-en` | WordNet | Princeton's, with notice |
| `bande-dict-sv` | Folkets lexikon, both ways | CC BY-SA 2.5 |
| `bande-dict-sv-wiktionary` | Swedish words from Wiktionary through kaikki.org | CC BY-SA |
