# Localisation

Code is written in English, the development language (`[workspace.metadata.l10n]` in
`Cargo.toml`), and the English is the key, as `String(localized:)` is on macOS:

- `tr!("Open File…")`, `tr!("{app} quit unexpectedly", app = name)`;
- `tr!(context = "key", "Delete")` when one English text has two meanings (gettext's msgctxt);
- `trn!("{count} file", "{count} files", count = n)`: the first argument picks the plural form;
- `text!("Copy")` makes a `Text` kept for later and translated with `.tr()` (gettext's `N_`).

The macros check at compile time that every `{placeholder}` has an argument. Keys are always
literals, never built at run time.

**Catalogs.** Every language, English included, is a gettext `.mo` file per program,
`/usr/share/bande/l10n/<program>/<lang>.mo`, holding every string of every crate linked into
that executable. `bande-l10n-tool pack` writes them: `en.mo` from the code's keys (marked
`X-Bande-Development: yes`), shipped in the program's own package, and one tree per other language
for the language packs ([packaging.md](packaging.md)). Translators edit `crates/**/locales/<lang>.po`;
a regional file (`en-GB.po`) lists only what differs from its language.

**Lookup** (`bande-l10n`). A program reads its folder once, on its first string, and takes the
first language in `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG` that has a catalog there (`pt-BR`,
then `pt`), else the development catalog. A string is looked up in that catalog, its language
without the region, the development catalog, and finally shown as the English in the code. A
catalog is read into memory and searched in place; fallbacks are opened only on a miss. Since the
key is the English, a translation is only ever used for the English it was made for. A language
change applies at the next start of each program.

**Checks.** `bande-l10n-tool check ci/l10n-complete` (in `ci/release/lint.sh`) fails on a broken
`.po`, an entry the code no longer has, a placeholder the English lacks, a wrong number of plural
forms, two crates of one program translating the same English differently, a language in
`ci/l10n-complete` with a gap, and a program package without its English catalog.
`bande-l10n-tool update` adds new English to each `.po` and drops what the code no longer has.
