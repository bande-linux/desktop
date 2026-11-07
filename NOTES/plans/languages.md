# Languages: translations, spelling, input methods

Audit gap 17. Plan only; nothing here is built yet. Screen readers and X11 are out of scope.

## What the user gets

- Choose Svenska in Settings, log out and in: every word of the desktop, its apps and the Browser is Swedish, with correct plurals, dates and sorting. A string nobody has translated yet shows in English, never as an id.
- Misspelled words get a red underline in every text field. A right click offers suggestions, Ignore and Learn. Checked in the language of the system or the one chosen. Never in password fields.
- Settings ▸ Keyboard offers Chinese, Japanese and Korean next to the keyboard layouts. Picking one installs it, asks for consent once, and it works in the Taskbar's Input menu.

## macOS, the bar

- About 40 languages, in an ordered list; the first one an app supports is used, and a restart may be needed to see the change everywhere ([language](https://support.apple.com/guide/mac-help/change-the-system-language-mh26684/mac)).
- Per-app language in Language & Region ▸ Applications ([settings](https://support.apple.com/guide/mac-help/change-language-region-settings-on-mac-intl163/mac)).
- Input sources are added in Keyboard ▸ Text Input and switched from the Input menu, Control-Space or Globe ([input sources](https://support.apple.com/guide/mac-help/write-in-another-language-on-mac-mchlp1406/mac)).
- Spelling: underline, Ignore (this document), Learn (everywhere), a chosen set of languages, autocorrect on one sure suggestion, off per app ([spelling](https://support.apple.com/guide/mac-help/typing-suggestions-correct-mistakes-mac-mchlp2299/mac)).

## What exists today

- `bande-l10n` does Fluent with an English fallback, but every program compiles in `[("en-US", ENGLISH)]`. **No language can be added without changing every crate.**
- Strings: desktop 2,237 messages in 31 domains (Settings 585, settings-panes 473, Files 327); apps 1,968 (Velox 380, Mail 219, Calendar 197). About 16,000 English words, 172 KB. 84 plural selectors in desktop, and counts are passed as numbers, so CLDR plurals work.
- No check of `.ftl` files in CI. The `password-new` duplicate lived for days because of that.
- `NUMBER()` is not registered (`add_builtins` is never called), and `fluent-bundle` has no `DATETIME`. Dates go through `bande-datetime` (ICU), but some code still calls `strftime` with English month names (`files/src/paint.rs`).
- 21 `.desktop` files, none with `Name[xx]`. Metainfo and polkit texts are English too. `wip/l10n-ready` and its pseudo-locale no longer exist on the remote, so hard-coded strings are unmeasured.
- Language is system-wide (`/etc/locale.conf`), read at session start. macOS has it per user. The login window follows what greetd started with.
- Text: HarfRust shaping, bidi, ICU line breaking and fontconfig fallback are there, and fields handle RTL. Parley gets no language tag, so Japanese can get Chinese glyph forms. Layouts do not mirror.
- `bande-desktop` already pulls `fonts-noto-cjk` (91 MB), `fonts-noto-core` (43 MB) and emoji (11 MB).
- Input: `text-input-v3`, `input-method-v2`, preedit and candidate placement work, consent is merged. Input methods are not offered as sources, and there is no shortcut for switching.
- `TextView` and `TextField` share `editor`; `SecureField` is separate and never enables text input, so password fields stay out of both input methods and spelling by construction.

## Translations

**Compiled in, in each package.** `bande_l10n::translations!("domain")` includes every `locales/<lang>/<domain>.ftl` it finds at compile time, so a language is a directory and no code names one; `ci/l10n-complete` lists the languages the check holds complete. Only the requested language is parsed, at start. A library's strings travel with the binary that links it, so an app pinned to an older toolkit always has matching strings, and the login window needs no file access. All languages stay under about 8 MB across the suite. This replaces the earlier files in `/usr/share`, which ran into ownership trouble for strings in statically linked libraries.

**Source layout.** `locales/sv/` next to `locales/en-US/` in each crate, so a string and its translations move together.

**Keeping it complete.** One check, `ci/check-l10n.sh`, run in both repos' gates:
- fails on syntax errors, duplicate ids, a variable that differs from English, ids in the source with no English message, ids in `.ftl` nothing uses;
- fails on a missing message for Swedish, reports a percentage for the others;
- a pseudo-locale run (`en-XA`) over the review scenarios finds hard-coded strings.

**Staleness.** Fluent's own rule: when English changes meaning, the id changes. The old translation then shows up as unused and the new one as missing, so the check catches it. Whoever changes a string adds the Swedish in the same change, as a draft; the owner sees it in daily use.

**Source of translations.** LLM draft from a glossary (Inställningar, Filer, Skrivbord), with the owner reading in daily use and correcting by message. One pass is about 60,000 tokens of input and as much output, plus a review pass with context: an afternoon's agent work. Owner review is about 80 minutes to read it flat, and better done by use. A professional pass would be roughly €2,000-3,000 (my estimate at market rates, no quote).

Platform later, only with outside translators. Weblate lists Fluent plurals as unsupported ([docs](https://docs.weblate.org/en/latest/formats/fluent.html)), which rules it out for this project's 84 selectors; Pontoon, Mozilla's own Fluent tool, is the one to try first.

**Switching.** Re-login, as macOS roughly does: the shell and apps read the language once. Settings says so and offers Log Out. Live switching would add invalidation to every view for a rare action, so no. Per-app language is `LANGUAGE` set at launch by the launcher, small, later.

**Settings.** The language list puts languages with a translation first and marks the others "formats only". Coordinate with `fix-locale-write`.

**Browser.** `web/NOTES/open.md`: only en-US is built. Debian's pack cannot be reused as is: it is tied to `firefox-esr` versions, and our build has its own identity and branding (`brand.ftl` in en-US only). The route is to build `bande-browser-l10n-sv` from Debian's same Firefox sources with Mozilla's translations and add a Swedish `brand.ftl`, then the package follows each Debian release. Not yet verified by a build; `apt-cache showsrc` gave no l10n component here, so where Debian keeps them has to be found first. Spelling needs nothing: `spellchecker.dictionary_path` already points at `/usr/share/hunspell`.

## Spelling

Measured on this machine with Debian's `hunspell-sv` (4.6 MB unpacked `sv_SE`, 153,717 entries), on 38 Swedish words: compounds, inflections, 10 misspellings and one nonsense word.

| | spellbook 0.4.2 (Rust) | nuspell 5.1.9 (C++) | hunspell 1.7.6 (C++) |
|---|---|---|---|
| Accept/reject | all right | all right | all right |
| First suggestion right | 10 of 10 | 10 of 10 | 6 of 10; none for 3 compounds |
| Load + memory | 36 ms, 9 MB | 17 MB peak | 13 MB peak |
| Check | about 16 µs a word | not separated | not separated |
| Suggest | 5-235 ms | whole run 0.8 s | whole run 0.2 s |
| Integration | a crate, MPL-2.0, no C++ | C++ bridge | C API |

`mjukvaruupdatering`, `tangentbordgenväg` and `lösenordhanterare` got no suggestion from hunspell. "All right" is 38 words, not a corpus; Stage 3 starts with larger English, Swedish and German lists, because spellbook is younger. If it loses there, the design stays and nuspell replaces it. Other options: `zspell` is less complete.

**A small shared service**, as macOS does with its spell server. `bande-spell`, D-Bus activated, exits when idle, holds one copy of each dictionary and the user's learned words, and tells clients when they change. Why not a library in each app: 9 MB per dictionary per process, and suggestions of up to 235 ms need another thread anyway. Learn has to reach every open app either way. Words reach the service over the session bus, which only the user's own programs can watch. Service absent: no underlines, nothing else changes.

**In the toolkit (`editor`).** Words are checked when a word ends and when a paragraph first becomes visible, one paragraph at a time. The word being typed is not underlined until the caret leaves it. Ranges live per paragraph and are redone on edit, so nothing runs while idle, and only visible text in long documents.

Red underline by the theme. The context menu gets suggestions, Ignore and Learn; suggestions are fetched when the menu opens. Settings ▸ Keyboard gets a language choice (automatic or chosen), a list of learned words, and an off switch.

**Which fields.** On by default in every `TextView` and `TextField`. A field says it is off: search, URL, path, email, numbers, code editors, Terminal. `SecureField` has no path to it.

**Dictionaries.** Debian's `hunspell-*` (91 packages), installed with the language. English stays in the default install. Mixed-language text uses the set of chosen languages: a word is correct if any of them accepts it.

**Autocorrect:** see the text features table below.

## Input methods

**fcitx5, on demand only.** Debian closure without Recommends, which avoids Qt and GTK modules: base 26 MB (18 packages, with cairo, pango, glib, librsvg); plus Korean 33 MB, Japanese (mozc) 48 MB, Chinese (pinyin) 81 MB.

**User path.** Settings ▸ Keyboard ▸ Add ▸ Chinese/Japanese/Korean → install with the same path Software uses → the existing consent switch shown in place with its warning ("can read what you type") → a user service starts only when one is configured.

**One list of sources.** `keyboard input-sources` holds layouts and methods together, and the compositor selects through fcitx5's D-Bus controller, so the Taskbar and the source card stay as they are. Add Control-Space, which CJK typists need and which is already missing for layouts.

**Candidate window.** A fcitx5 theme file in Bande's colours first; own panel only if that looks wrong.

**Testing without the owner's hardware.** On a build VM, the real headless compositor, real fcitx5 and the virtual keyboard: `nihao` gives 你好, `nihongo` gives 日本語, `dkssud` gives 안녕, in `TextField` and `TextView`, plus none in `SecureField`, none while locked, none without consent. One e2e test. A native typist's opinion would be welcome once, not a gate.

## Other scripts

- Pass the language to Parley and fontconfig so Japanese, Chinese and Korean get their own glyph forms. Small, and it matters as soon as stage 4 lands.
- Keep the Noto fonts in the default install, as macOS shows every script everywhere; 145 MB is the price of mail and web never showing boxes.
- RTL UI is the one big item: rows, sidebars, icons with direction and animations must mirror, in the toolkit and in custom paint code. Also turn on Fluent's bidi isolation for RTL languages only. Only when an RTL language is actually wanted.

## Size

| | Default install | On demand |
|---|---|---|
| Translations | about 200 KB per language, all in the package | |
| Browser language | | under 1 MB each |
| Spelling | service under 1 MB, English dictionary as now | `hunspell-sv` 8.3 MB |
| Input methods | nothing | 33-81 MB |

## Stages

1. **Pipeline.** Files in `Localizer`, `NUMBER`, the check, pseudo-locale, sweep of hard-coded strings and English dates. Size S to M, depending on the sweep. Risk low. Can take a day of mechanical crate edits.
2. **Swedish.** Glossary, draft of all 4,200 messages, `.desktop`, metainfo, polkit, Settings' language list, the login window's language, per-user language. Size M. Risk: wording; the owner's reading handles that. Depends on `fix-locale-write`.
3. **Spelling.** Service, toolkit, Settings. Size M to L. Risk medium: the editor's layout and undo must stay untouched; new crate.
4. **Input methods.** Settings path, one source list, Control-Space, language tags in text, the e2e test. Size M. Risk medium; real typists are not on hand.
5. **Browser Swedish.** Size S to M, long builds. Risk: Firefox release coupling. Independent; can move up.
6. **More languages.** Each small: draft, check, ship. Platform when outside translators appear.
7. **RTL.** Size L, risk high. Only on demand.

## Owner decisions

1. **Translation source:** LLM draft, with you reading Swedish in daily use. Recommended. Alternatives: pay a translator, or a platform now.
2. **Which languages after Swedish:** recommend none until Swedish has been lived with for a few weeks; then German, French and Spanish.
3. **In the package or as language packs:** in the package, recommended; 8 MB at most and no version coupling.
4. **New crate `spellbook`:** recommended if the larger lists agree.
5. **Autocorrect:** build the undoable design below, off by default in Swedish until the measurement passes, recommended.
8. **Look Up:** build it, after spelling and in the order given, with the data below, recommended.
9. **Wiktionary data for Swedish (68 MB, on demand):** yes, recommended, because Folkets alone misses inflected compounds; ask Svenska Akademien about SAOL separately.
6. **Per-user language:** do it in stage 2, recommended. The cost is small and it matches macOS.
7. **RTL:** wait until someone wants it, recommended.

## Research: macOS in depth

Sourced from Apple's guide unless marked (memory).

| Behaviour | macOS | Bande today | Recommendation, size |
|---|---|---|---|
| Language order | Ordered list; next language is used if the first is unsupported; [adding an input source adds its language](https://support.apple.com/guide/mac-help/change-language-region-settings-on-mac-intl163/mac) | One language, then English | Keep one plus English now; list later with Fluent's chain and a real second language. S |
| Per-app language | Applications list, per app | None | `LANGUAGE` at launch. S, after Swedish |
| Region and formats | Region, calendar, temperature, measurement, first day, date and number format, list sort order, each separate | Region and format categories | Add temperature, first day and sort overrides only when asked. S each |
| Input sources | Input menu, Control-Space, Globe, Caps Lock | Menu only | Control-Space in stage 4. S |
| Auto-switching | "Automatically switch to a document's input source" (memory) | One session-wide source, by design | Do not copy now; it surprises more than it helps. Offer the setting if asked. S |
| Spelling languages | [Spelling is checked in whichever language you type; or Set Up a set](https://support.apple.com/guide/mac-help/typing-suggestions-correct-mistakes-mac-mchlp2299/mac) | Planned | Copy: a word is right if any chosen dictionary accepts it; default set is the system language, English and the input sources' languages. In stage 3 |

## Research: text features

Layer is the toolkit `editor` unless said, so every `TextField` and `TextView` gets it; the Browser and Terminal do not.

| Feature | macOS | Copy? How | Size |
|---|---|---|---|
| Autocorrect | [With one suggestion, corrected as you type on; solid underline; put the caret after it to see the original; Control-click gives "Change Back to …"; off per app](https://support.apple.com/guide/mac-help/typing-suggestions-correct-mistakes-mac-mchlp2299/mac) | Yes, this design, see below | M after spelling |
| Capitalise first word | Same pane; also proper nouns | First word of a sentence, one undo step; names via dictionary case | S |
| Double-space period | [Yes](https://support.apple.com/guide/mac-help/use-text-replacements-mh35735/mac) | Yes, S; on with the others | S |
| Smart quotes and dashes | Yes, with styles | Yes, per language (Swedish ”…”), not in code or Terminal | S |
| Text replacements | Keyboard ▸ Text Replacements | Yes: one file held by the spell service, editor applies at word end | S to M |
| Inline predictions | Grey text, Space accepts; not in all languages | No. Needs a model; Space-accepts collides with typing | - |
| Emoji and symbols | Picker | Exists (`Super+.`). Keywords are English only: add Swedish CLDR names | S |
| Dictation | [Start key, on-device varies by language](https://support.apple.com/guide/mac-help/dictate-your-messages-and-documents-mh40584/mac) | Later, on-demand on-device model through input-method-v2 (engine not evaluated) | L |
| Writing Tools | [Rewrite, proofread, summarise; Apple Intelligence](https://support.apple.com/guide/mac-help/use-writing-tools-mchldcd6c260/mac) | No: no on-device model that is good and small here. Revisit | - |
| Live Text | Select text in images; in Language & Region | Later in Preview and Quick Look: Debian's Tesseract, on demand (not measured) | M |
| Data detectors | Smart Links for URLs and addresses; dates and more (memory) | URLs and email in Mail and read-only text; rest later | S |
| Grammar | Dotted underline, some languages | No: no good offline Swedish checker known (memory) | - |

**Autocorrect revisited.** My "no" rested on a wrong silent change. macOS's own design answers most of that: the change is visible, one key or click from undone. Design:
- Only one suggestion, one edit away, and nothing when the word has capitals inside, digits or a dot; never in fields that opt out of spelling.
- Corrected at word end. The word gets a thin underline until the next edit elsewhere; Backspace right after restores the original and types nothing else; the menu has "Change Back to …". Changing back twice learns the word.

Evidence still needed: compounds are where a valid new word can be rejected. On 38 words, spellbook accepted every valid compound and put the right word first for all 10 misspellings. That is too small. Gate: over a real Swedish text corpus, at most 1 wrong change per 1,000 correct words. On by default in English; in Swedish on only if the gate passes, otherwise suggestions only. Result: over 160,518 words of Swedish Wikipedia featured articles (CC BY-SA 4.0), 5.04 wrong changes per 1,000, mostly names; the gate fails, so `autocorrect = false` in the Swedish data file and Swedish stays suggestions only. Measure with `crates/services/spell/examples/autocorrect_gate.rs`.

## Research: Look Up

**macOS.** [Control-click a word (some apps need a selection), three-finger tap or force click; dictionary, thesaurus, Wikipedia with a chosen language, Siri Knowledge; sources chosen and ordered in Dictionary settings; content is not sent, only hints](https://support.apple.com/guide/mac-help/look-up-words-mchl3983326c/mac). `Ctrl+Cmd+D` and the word picked from under the pointer in any app through the text system are from memory, as is Translate being a separate menu entry.

**Bande design.**
- **Service and panel:** `bande-lookup`, D-Bus activated, a native panel at the pointer with tabs per source, exits a short time after closing. No idle cost, nothing runs in the default state.
- **Word:** the selection, or the word under the pointer using Parley's word boundaries, then the toolkit passes text and language hint. Every dictionary is asked; the UI language's result comes first.
- **Inflections and compounds:** forms table first, then the longest known ending with the first part shown ("… + genväg"). Marked plainly as derived.
- **Where:** "Look Up “word”" in `editor` menus, read-only text, labels (a default `context_menu` with hit testing, so it costs nothing until right click), Files names, Terminal selection. `Ctrl+Super+D` in the toolkit. Three-finger tap only if the owner wants it (touchpad, gesture conflict unchecked).
- **Browser:** a patch in `web` beside the "Search for" entry, starting `bande-lookup` with the selection, no extension. To agree with the Browser agents. Word-under-pointer without selection needs checking.
- **Private:** no network by default. Wikipedia is off until turned on in Settings, and each lookup then says it goes online.
- **Translation:** Firefox's on-device Bergamot models [cover Swedish and English](https://github.com/mozilla/firefox-translations-models); model size and licence not confirmed, and they run inside Firefox today, so use outside it means a native runtime. Later stage; a Folkets translation already answers single words.

**Sources.**

| Source | Licence | Size | Finding |
|---|---|---|---|
| WordNet (Debian `wordnet-base`) | Princeton's, free with notice | 33 MB as indexed db | Best English; recommended default |
| Folkets lexikon | CC BY-SA 2.5 (in the XML) | 15 MB db, both ways | 39,554 sv and 55,889 en entries; Swedish definitions on many; recommended |
| Wiktionary, Swedish words ([kaikki](https://kaikki.org)) | CC BY-SA | 68 MB db, 18 MB compressed; English glosses | 313,781 entries including inflected forms; best coverage; on demand |
| English Wiktionary all | CC BY-SA | 3.4 GB raw, not reduced | Too big without trimming; not now |
| GCIDE (`dict-gcide`) | GPL | 16 MB | 1913-based, dated; no |
| FreeDict `swe-eng` | GPL | under 1 MB | Small, weaker than Folkets; no |
| `mythes-sv`, `mythes-en-us` | per package | 1 MB, 21 MB | Thesaurus; Swedish worth adding, English is in WordNet |
| `dictd` | | | A running server and old format; zero idle rules it out |
| SAOL, Svenska Akademien | [No reuse licence found; SAOL 15 data listed by Språkbanken](https://spraakbanken.gu.se/en/node/5113), terms unread | - | Don't ship; ask the Academy. A "Search svenska.se" link is fine |
| Lexin | [Handed to KTH as Folkets' base](https://folkets-lexikon.csc.kth.se/folkets/projekt/slutrapport.pdf); current terms not found | - | Covered through Folkets |

**Measured (throwaway SQLite, Python), 30 words on the mini.** Lookups 0.01-0.3 ms. English 13 of 14 found (`looked-up` missed; `mice`, `geese`, `children`, `took` reach their lemma); ranking needs work (`ran` shows the baseball noun first; order verbs by the form that matched). Swedish 16 of 16 gave a useful meaning through Wiktionary with Folkets; Folkets alone 10 of 16, missing `flickorna`, `husvagn`, `järnvägsstationen`, `fotbollsspelarna`. `husvagnsägare`, `lösenordshanterare` and `tangentbordsgenväg` came from the last part only, which is honest but partial. Swedish-language definitions exist only in Folkets; Wiktionary's are in English.

**Size.** Service, panel and menu entries M. Data packages S (`bande-dict-en` 33 MB, `bande-dict-sv` 15 MB, optional Wiktionary 68 MB, all on demand). Browser patch S. Risk low; the label hit testing is the unknown. New stage after spelling; can start earlier since it needs no spell service.
