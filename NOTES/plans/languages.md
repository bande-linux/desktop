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

**Files, in the package.** Each package installs `/usr/share/bande/l10n/<lang>/<domain>.ftl`. `Localizer` keeps en-US compiled in and reads only the one requested file at start. A missing file or message falls back to English, so a toolkit pinned at another revision by an app cannot break. No idle cost.

Why not `.deb` language packs: all languages together stay under about 8 MB across the whole suite, and packs bring version coupling (Debian's `firefox-esr-l10n-sv-se` depends on an exact Firefox version range). Debian's own gettext practice is translations inside the package. Why not compiled in: every binary would carry every language, and a fix would rebuild everything.

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

**Autocorrect: not now.** Compounds make "one sure suggestion" risky in Swedish, and a wrong silent change is worse than an underline. Look again once spelling has been used for a while.

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
5. **Autocorrect:** leave out for now, recommended.
6. **Per-user language:** do it in stage 2, recommended. The cost is small and it matches macOS.
7. **RTL:** wait until someone wants it, recommended.
