# Languages: translations, spelling, input methods

Audit gap 17. Swedish, spelling and Look Up are built; translations move to the model in [language-packs.md](language-packs.md). Screen readers and X11 are out of scope.

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

- Strings: desktop about 2,400 messages, apps about 2,000, all translated to Swedish and compiled into each program with Fluent ids, until [language-packs.md](language-packs.md) replaces that.
- Language per user (`~/.config/locale.conf`) and per app (`LANGUAGE` at launch), read at session start.
- Text: HarfRust shaping, bidi, ICU line breaking and fontconfig fallback are there, and fields handle RTL. Parley gets no language tag, so Japanese can get Chinese glyph forms. Layouts do not mirror.
- `bande-desktop` already pulls `fonts-noto-cjk` (91 MB), `fonts-noto-core` (43 MB) and emoji (11 MB).
- Input: `text-input-v3`, `input-method-v2`, preedit and candidate placement work, consent is merged. Input methods are not offered as sources, and there is no shortcut for switching.
- `TextView` and `TextField` share `editor`; `SecureField` is separate and never enables text input, so password fields stay out of both input methods and spelling by construction.

## Translations

**Model:** English in the code is the key, and every language, English included, is a gettext catalog per program; see [language-packs.md](language-packs.md). Translators edit `locales/<lang>.po` next to the code; `ci/l10n-complete` lists the languages that must be complete (`sv`).

**Source of translations.** LLM draft from a glossary (Inställningar, Filer, Skrivbord), with the owner reading in daily use and correcting by message. A professional pass would be roughly €2,000-3,000 (estimate, no quote). Platform later, only with outside translators: Pontoon and Weblate both read `.po` with plurals.

**Switching.** Re-login, as macOS roughly does: programs read the language once. Settings says so and offers Log Out. Per-app language is `LANGUAGE` set at launch.

**Browser.** `web/NOTES/open.md`: the route is `bande-browser-l10n-sv` built from Debian's Firefox sources with Mozilla's translations and a Swedish `brand.ftl`, following each Debian release. Spelling needs nothing: `spellchecker.dictionary_path` already points at `/usr/share/hunspell`.

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

`mjukvaruupdatering`, `tangentbordgenväg` and `lösenordhanterare` got no suggestion from hunspell. "All right" is 38 words, not a corpus; the spelling work started with larger English, Swedish and German lists, because spellbook is younger. If it loses there, the design stays and nuspell replaces it. Other options: `zspell` is less complete.

**A small shared service**, as macOS does with its spell server. `bande-spell`, D-Bus activated, exits when idle, holds one copy of each dictionary and the user's learned words, and tells clients when they change. Why not a library in each app: 9 MB per dictionary per process, and suggestions of up to 235 ms need another thread anyway. Learn has to reach every open app either way. Words reach the service over the session bus, which only the user's own programs can watch. Service absent: no underlines, nothing else changes.

**In the toolkit (`editor`).** Words are checked when a word ends and when a paragraph first becomes visible, one paragraph at a time. The word being typed is not underlined until the caret leaves it. Ranges live per paragraph and are redone on edit, so nothing runs while idle, and only visible text in long documents.

Red underline by the theme. The context menu gets suggestions, Ignore and Learn; suggestions are fetched when the menu opens. Settings ▸ Keyboard gets a language choice (automatic or chosen), a list of learned words, and an off switch.

**Which fields.** On by default in every `TextView` and `TextField`. A field says it is off: search, URL, path, email, numbers, code editors, Terminal. `SecureField` has no path to it.

**Dictionaries.** Debian's `hunspell-*` (91 packages), installed with the language. English stays in the default install. Mixed-language text uses the set of chosen languages: a word is correct if any of them accepts it.

**Autocorrect:** see the text features table below.

## Input methods

**fcitx5, on demand only.** Debian closure without Recommends, which avoids Qt and GTK modules: base 26 MB (18 packages, with cairo, pango, glib, librsvg); plus Korean 33 MB, Japanese (mozc) 48 MB, Chinese (pinyin) 81 MB.

**User path (owner, 2026-11-08).** Settings ▸ Keyboard ▸ Add ▸ Chinese/Japanese/Korean → install with the same path Software uses → a one-time consent switch, shown in place when the first one is added, with the warning "can read what you type" → a user service starts only when one is configured. An input method is never active in password fields (`SecureField`) or on the lock screen.

**One list of sources.** `keyboard input-sources` holds layouts and methods together, and the compositor selects through fcitx5's D-Bus controller, so the Taskbar and the source card stay as they are. Add Control-Space, which CJK typists need and which is already missing for layouts.

**Candidate window.** A fcitx5 theme file in Bande's colours first; own panel only if that looks wrong.

**Testing without the owner's hardware.** On a build VM, the real headless compositor, real fcitx5 and the virtual keyboard: `nihao` gives 你好, `nihongo` gives 日本語, `dkssud` gives 안녕, in `TextField` and `TextView`, plus none in `SecureField`, none while locked, none without consent. One e2e test. A native typist's opinion would be welcome once, not a gate.

## Other scripts

- Pass the language to Parley and fontconfig so Japanese, Chinese and Korean get their own glyph forms. Small, and it matters as soon as input methods land.
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

1. **Language packs:** [language-packs.md](language-packs.md).
2. **Input methods.** Settings path with the one-time consent, one source list, Control-Space, language tags in text, the e2e test. Size M. Risk medium; real typists are not on hand.
3. **Browser Swedish.** Size S to M, long builds. Risk: Firefox release coupling. Independent; can move up.
4. **More languages.** Each small: draft, check, ship. Platform when outside translators appear.
5. **RTL.** Size L, risk high. Only on demand.

## Owner decisions

1. **Which languages after Swedish:** recommend none until Swedish has been lived with for a few weeks; then German, French and Spanish.
2. **Wiktionary data for Swedish (68 MB, on demand):** yes, recommended, because Folkets alone misses inflected compounds; ask Svenska Akademien about SAOL separately.
3. **RTL:** wait until someone wants it, recommended.

## Research: macOS in depth

Sourced from Apple's guide unless marked (memory).

| Behaviour | macOS | Bande today | Recommendation, size |
|---|---|---|---|
| Language order | Ordered list; next language is used if the first is unsupported; [adding an input source adds its language](https://support.apple.com/guide/mac-help/change-language-region-settings-on-mac-intl163/mac) | One language, then English | Keep one plus English now; list later with Fluent's chain and a real second language. S |
| Per-app language | Applications list, per app | None | `LANGUAGE` at launch. S, after Swedish |
| Region and formats | Region, calendar, temperature, measurement, first day, date and number format, list sort order, each separate | Region and format categories | Add temperature, first day and sort overrides only when asked. S each |
| Input sources | Input menu, Control-Space, Globe, Caps Lock | Menu only | Control-Space with input methods. S |
| Auto-switching | "Automatically switch to a document's input source" (memory) | One session-wide source, by design | Do not copy now; it surprises more than it helps. Offer the setting if asked. S |
| Spelling languages | [Spelling is checked in whichever language you type; or Set Up a set](https://support.apple.com/guide/mac-help/typing-suggestions-correct-mistakes-mac-mchlp2299/mac) | Planned | Copy: a word is right if any chosen dictionary accepts it; default set is the system language, English and the input sources' languages. |

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

Evidence still needed: compounds are where a valid new word can be rejected. On 38 words, spellbook accepted every valid compound and put the right word first for all 10 misspellings. That is too small. Gate: over a real Swedish text corpus, at most 1 wrong change per 1,000 correct words. On by default in English; in Swedish on only if the gate passes, otherwise suggestions only. Result, wrong changes per 1,000 words over Wikipedia featured articles (about 160,000 words each, CC BY-SA 4.0), after the rules below: Swedish 0.27, held out 0.24; English 0.20, held out 0.35. Both are on. Before the rules Swedish was 5.04, mostly names. Rules: a capitalised word only at a sentence start, at least `autocorrect-min-length` letters, the typed word not and the fix in `<language>.common` (30,000 most frequent), and never a word already typed or elsewhere in the document. A language turns on with `autocorrect = true` in its data file once `crates/services/spell/examples/autocorrect_gate.rs` passes.

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
