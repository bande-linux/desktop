# Language packs: English in the code, every language as a catalog

The owner's model (2026-11-08). Research and the other language items are in [languages.md](languages.md).

## What the user gets

- Settings ▸ Language ▸ Svenska fetches the Swedish packs for the desktop and each installed app, then offers Log Out. After that every program is Swedish.
- A corrected word arrives as a small package with the normal updates. No program is rebuilt; each shows it the next time it starts.
- A string nobody has translated shows the English from the code. Never an id, never blank, never a translation made for other English.

## The model

- **A development language, as on macOS.** Code is written in English, and the English is the key: `tr!("Open File…")`, as `String(localized: "Open File…")` is. One English text with two meanings gets a context: `tr!(context = "door", "Open")`, gettext's msgctxt. Counts give the English singular and plural: `trn!("{count} file", "{count} files", count = n)`, gettext's ngettext; the first argument picks the form. Placeholders are named, `{name}`, and the macro rejects a placeholder without an argument at compile time.
- **Text kept for later** (menus built from tables, a status shown later) is a `Text` made with `text!("…")`, gettext's `N_()` marker, and translated with `.tr()`. Keys are never built at run time: every key is a literal the tool can find.
- **Every language, English included, is a catalog loaded the same way.** A catalog is a gettext `.mo` file, `/usr/share/bande/l10n/<program>/<lang>.mo`, where `<program>` is the executable's name. It holds the strings of every crate linked into that program, toolkit included, as a macOS bundle's `.lproj` does.
  - `en.mo` is generated from the code's keys and ships inside the program's own package, so it cannot go missing. Its header marks it as the development language.
  - Every other language, `en-GB` included, ships in a language pack.
  - The English in the code is the last resort for an entry no catalog has.
- **One language per program.** The program takes the first language in the user's list (`LANGUAGE`, then `LC_ALL`, `LC_MESSAGES`, `LANG`) that has a catalog in its folder, trying `pt-BR` and then `pt`. It finds the languages by listing that folder, so no code names one. If none matches, it uses the development catalog.
- **Lookup within that language:** the chosen catalog, then its language without the region (`en-GB` falls back to `en`), then the development catalog, then the code. The fallbacks are opened only on the first miss, so a complete Swedish catalog never opens English.
- **Version matching is the English itself.** A translation is found by its exact English and context, so it is only ever used for the English it was made from. This is the per-message English fingerprint the owner chose, without a separate hash: changed English in a newer or older program finds no entry and shows the new English. Programs and packs need no version pins.
- **No live reload.** A language change needs a log-out, as on macOS. Nothing polls or watches files.

**Why `.mo`.** The file is read into memory once and searched in place, with no parsing, so a catalog costs its file size and microseconds. It is gettext's format: `msgunfmt` reads it, and contexts and plural forms are built in. The plural rule is each catalog's `Plural-Forms` header, written by the translator; English uses gettext's default.

## Source files

- Translations are gettext `.po` files next to the code, `crates/**/locales/<lang>.po`, one per crate and language. Poedit, Pontoon and Weblate read them, plurals included.
- A regional file lists only what differs from its language: `en-GB.po` holds only spelling (colour, centre, licence, cancelled).
- There is no English source file: the code is the English. `[workspace.metadata.l10n] development-language = "en"` in each repo's `Cargo.toml` declares the development language.

## The tool

`bande-l10n-tool` (in `crates/libs/l10n`, reading the code with `syn`), used by both repos:
- `check` fails on a `.po` syntax error, a duplicate, an entry the code no longer has, a placeholder the English lacks, a plural count that differs from `Plural-Forms`, two crates in one program translating the same English differently, and a missing entry in a language listed in `ci/l10n-complete`. It prints each language's percentage.
- `update` adds new English to each language's `.po` and drops entries the code no longer has.
- `pack` writes every program's catalogs: `en.mo` for each program's package, and one tree per other language for the packs. In the apps repo it reads the toolkit at the pinned desktop revision, so an app's pack always matches the toolkit it was built with.

## Packages

| Package | Contains | Relations |
|---|---|---|
| each program's package | `<program>/en.mo` | |
| `bande-desktop-l10n-<lang>` | `<program>/<lang>.mo` for every desktop program | `Architecture: all`, `Section: localization`, `Enhances: bande-desktop` |
| `bande-<app>-l10n-<lang>` | that app's catalog | `Depends: bande-<app>`, `Enhances: bande-<app>` |
| `bande-browser-l10n-<lang>-<region>` (web repo) | Firefox's pack and `brand.ftl` | unchanged, pinned to the exact Browser version |

Package names use lower case, `bande-desktop-l10n-en-gb`, as `firefox-esr-l10n-sv-se` does.

## Installing on demand

- One rule in `bande-packages`: for each chosen language and each installed package `P`, install `P-l10n-<lang>-<region>` if it exists, else `P-l10n-<lang>`. Debian's own packs (LibreOffice, Firefox ESR) come with it.
- Settings ▸ Language lists first the languages with a desktop pack in the APT index or installed. Choosing one writes `locale.conf`, runs the rule with progress, and offers Log Out. Offline, the language is set and the page says the text is fetched when online.
- Software runs the rule when it installs an app, and the update check after updating. The setup assistant, installer and images run it for the language chosen at setup. A per-app language installs that app's pack.

## Translation-only releases

Desktop done (`docs/architecture/ci.md`): `l10n-YYYYMMDD` tags build only the packs, at `<last release>+l10nYYYYMMDD`. Left: the APT repository taking those runs (as it takes `dict.yml`), and the apps repo's equivalent.

## Measured

Release builds on the mini, Swedish session, median of 21 runs: the cost of loading a program's translations and showing its first string. Before is Fluent with every domain the program links parsed (Swedish and English); after is listing the program's folder, reading its `sv.mo` and one lookup.

| Program | Before | Heap before | After | Heap after (= catalog size) |
|---|---|---|---|---|
| bande-settings | 974 µs | 1,943 KB | 25 µs | 97 KB |
| bande-shell | 722 µs | 1,483 KB | 20 µs | 61 KB |
| bande-file-chooser | 458 µs | 897 KB | 16 µs | 37 KB |
| bande-files | 398 µs | 790 KB | 16 µs | 33 KB |
| bande-greeter, bande-setup, bande-unlock | 281-285 µs | 550 KB | 14 µs | 21 KB |
| bande-lock | 257 µs | 498 KB | 13 µs | 18 KB |
| bande-compositor | 96 µs | 193 KB | 12 µs | 8 KB |
| small services (power, spell, config, portal) | 11-15 µs | 23-28 KB | 8-11 µs | 1-2 KB |

The language packs are 65 KB (`sv`) and 3 KB (`en-GB`) as `.deb`.

## Stages

1. **Desktop** (done, `l10n-catalogs`): the runtime, macros and tool; every desktop crate moved from Fluent ids to English keys, with `sv.po` and `en-GB.po`; `en.mo` in each package; the `bande-desktop-l10n-sv` and `-en-gb` packs; `ci/check-l10n.py` replaced. Settings' language list reads the installed catalogs.
2. **Installing on demand:** the `bande-packages` rule, Settings, Software, the update check, setup and images.
3. **Apps repo:** the pin moves to stage 1, every app converted, `en.mo` in each app's package, per-app packs, the shared tool in the gate.
4. **Translation-only releases** (desktop done; the APT repository and apps left).
5. **Browser:** Settings and the rule pick up `bande-browser-l10n-*`.

**Stages 1 and 3 reach the owner's mini together, with their packs.** The desktop release and the app releases built on it go in one `apt` run that also installs `bande-desktop-l10n-sv` and every installed app's `bande-<app>-l10n-sv`; otherwise a Swedish session shows English. Until stage 2 does this by itself, main installs the packs by hand with each such release.

## Risks

- The conversion touches every string in both repos. Branches open at the same time convert their new strings before merging; a wrong `tr!` fails the compile and a wrong `.po` fails the check.
- An English typo fix shows English in every language until the packs are rebuilt. Releases rebuild them, so this lasts only between an app release and its packs.
- A missing pack means English, never broken text. The installer and images must install packs.
- Two packages are named `bande-browser` (the paused own engine in apps, and the web repo's Firefox build). Settle before stage 5.
