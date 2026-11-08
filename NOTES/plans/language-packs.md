# Language packs: translations as files, shipped on their own

Plan only. Today every translation is compiled into each program by `bande_l10n::translations!` (see [languages.md](languages.md)), so a new language or a fixed word needs a rebuild and a release of every app. The goal is to keep English built in and ship the other languages as small Debian packages. They are read at runtime, so fixes and new languages can ship without rebuilding apps.

## What the user gets

- In Settings ▸ Language, the user chooses Svenska. Settings fetches the Swedish packs for the desktop and for each installed app and Browser, then offers Log Out. After that every program is in Swedish.
- A corrected word arrives with the normal updates as a small package. No app is rebuilt or restarted for it; programs show it the next time they start.
- A string that has no matching translation shows in English. The user never sees a message id, a stale translation or an error.

## How macOS does it

These points come from Apple's documentation unless marked (memory).

**Files in the bundle.** Each localisation is a folder, `<lang>.lproj`, in the app bundle's `Resources`. Strings are key/value `.strings` tables. Plurals are in `.stringsdict`, using CLDR categories (zero, one, two, few, many and other), and only `other` is required ([plurals](https://developer.apple.com/documentation/xcode/localizing-strings-that-contain-plurals)).

**Base Internationalization** keeps one set of `.storyboard` and `.xib` files in `Base.lproj`, written in the development language. Each language then gets only a `.strings` file for them ([guide](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPInternational/InternationalizingYourUserInterface/InternationalizingYourUserInterface.html)).

**String Catalogs** (`.xcstrings`) are JSON used only while authoring. "At build time, these files compile to .strings and .stringsdict files", and the source strings extracted from code are left out of the build. Each string has a state: New, Needs Review, Stale or Translated ([WWDC23 10155](https://developer.apple.com/videos/play/wwdc2023/10155/)). The runtime format has not changed in many years.

**Lookup.** Bundle chooses one localisation for the whole bundle. It takes the first preferred language that has an `.lproj` folder. If none matches, it uses the development language, `CFBundleDevelopmentRegion`. Within a localisation, a resource is searched in this order: global, then the region folder, then the language folder, then the development language ([bundles](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/AccessingaBundlesContents/AccessingaBundlesContents.html)).

A missing key never fails. `localizedString(forKey:value:table:)` returns the given default value, or the key itself. With `NSShowNonLocalizedStrings` it also logs the miss ([API](https://developer.apple.com/documentation/foundation/bundle/localizedstring(forkey:value:table:))).

Bundle caches what it has looked up, and a table named `*.nocache.strings` is not cached ([strings](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/LoadingResources/Strings/Strings.html)). Since 10.11, Bundle is thread-safe. Two points are from memory: only the tables an app asks for, in its chosen localisation, are ever read; and a framework's strings follow the app's chosen localisation unless the app sets `CFBundleAllowMixedLocalizations`. So an app that has only English shows English AppKit menus on a Swedish Mac.

**Language versus region.** The user keeps an ordered list of languages. If an app does not support the first one, the next one is used ([system language](https://support.apple.com/guide/mac-help/change-the-system-language-mh26684/mac)). Adding an input source adds its language to the list. Region is a separate setting for dates, numbers and currency ([Language & Region](https://support.apple.com/guide/mac-help/change-language-region-settings-on-mac-intl163/mac)). Language IDs are `language[-script][-region]`, and Apple asks developers to use the most specific one they really mean ([IDs](https://developer.apple.com/documentation/xcode/choosing-localization-regions-and-scripts)).

**Per-app language** is set in Language & Region ▸ Applications ([support](https://support.apple.com/HT202036)).

**Live change.** There is none. An open app "may need to quit and reopen", and the Mac may need a restart before every app shows the new language (same pages).

**Grammar.** Plurals come from `.stringsdict` files or catalog variants. Automatic grammar agreement, the `^[…](inflect: true)` markup, inflects marked text at runtime with the keyboard's morphology data. It launched with English and Spanish ([WWDC21 10109](https://developer.apple.com/videos/play/wwdc2021/10109/)). More languages were added later (memory).

**System languages.** The macOS user interface comes with every localisation installed, and nothing is downloaded when the language changes (memory). Tools such as Monolingual exist only to delete them. Only extras are downloaded on demand: translation models, which are "downloaded to your computer so you can translate when offline" (Language & Region page), and dictation and voices (memory).

**Code signing.** "Removing files from .lproj directories inside Contents/Resources will not invalidate the code signature, but adding or changing files will" ([TN2206](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)). So a signed app cannot gain a language or a fixed string later. On macOS both come only as a new version of the app.

**What this means for Bande.** macOS does keep translations as files that are read at runtime and only when needed. It does not ship them as separate packages. They are sealed inside each app and change only with the app. The owner's goal therefore needs two parts:

- From macOS: the runtime model. Files per language, read lazily; one localisation per program; the development language always present; a miss never fails.
- From Debian: separate `-l10n` packages.

The code-signing seal is what stops a macOS app from ever loading strings that belong to another version. Bande needs its own replacement for it: a per-message English fingerprint, described below.

## Other systems

- **gettext and Debian.** Translations are looked up at `dir/locale/LC_MESSAGES/domain.mo` ([gettext](https://www.gnu.org/software/gettext/manual/html_node/Locating-Catalogs.html)), with `LANGUAGE` as a priority list. A `.mo` file has sorted strings and a hash table ([MO](https://www.gnu.org/software/gettext/manual/html_node/MO-Files.html)).
  - The key is the English text itself, so a translation always belongs to exactly the English it was made from. A changed English string simply stops matching and falls back to English. The fingerprint below copies this property.
  - Most Debian packages ship every language's `.mo` inside the main package (on the mini, more than 100 packages own `/usr/share/locale/sv/LC_MESSAGES`). Large suites split them out instead:
    - `libreoffice-l10n-sv` Depends loosely on `libreoffice-common (>= 1:7.0.0~alpha~)`.
    - `firefox-esr-l10n-sv-se` pins `firefox-esr (>= 153.5.1esr-1), (<< 153.5.1esr-1.1~)`.
    - `task-swedish-desktop` Recommends both.
  - Debian has no rule that installs a translation package when both the app and the language are present. Fedora's langpacks do this with `Supplements:` (memory). Ubuntu's `language-pack-*` packages took translations out of the app packages so they could be updated separately (memory).
- **Firefox language packs** are add-ons that contain only the localised resources. Building one needs no compilation, but a pack is "only compatible with the mozilla-central source they're built with" ([Firefox docs](https://firefox-source-docs.mozilla.org/build/buildsystem/locales.html)). Their manifests pin the exact version (memory). That is why Debian pins the version too.
- **Android** compiles resources (`values-sv`) into the APK.
  - Since 7.0 it walks the user's list of locales. For each one it tries the exact locale, then the parent, then the parent's children (fr_CH, then fr, then fr_FR), and only then the default resources ([Android](https://developer.android.com/guide/topics/resources/multilingual-support)).
  - Per-app language (Android 13) recreates the activity ([app languages](https://developer.android.com/guide/topics/resources/app-languages)).
  - Play delivers language resources as split APKs for the device's languages and fetches more when the user adds a language. The splits are locked to the app's version (memory).

**Chosen for Bande:** the macOS runtime model, Debian `-l10n` packages, and gettext's "the English is the key" safety, done as a fingerprint. Exact version pins in the style of Firefox and Debian would force translation packs and apps to upgrade in lockstep. They would also break the toolkit linked into apps that pin an older desktop. The fingerprint needs no pins.

## Design

### Where the files live

```
/usr/share/bande/l10n/<lang>/<domain>.ftl       translation, Fluent, as in the repo
/usr/share/bande/l10n/<lang>/<domain>.english   one line per message: id and English fingerprint
```

- `<lang>` is a BCP 47 tag, written language-only wherever possible: `sv`, `de`, `pt-BR`, `zh-Hant`.
- `<domain>` is the crate's domain as it is today: `bande-settings`, `bande-widgets`, `bande-mail`. Domains are crate names, so they are unique across both repos.

**Why one shared tree instead of a folder per package.** Library domains such as `bande-widgets`, `bande-app` and `bande-fs` are linked into many packages. In the old `/usr/share` attempt this caused the ownership trouble. The fix is one rule: each domain is shipped by exactly one language pack, the pack built from the repo the crate lives in. The program finds a domain without knowing which package installed it.

**Source layout stays as it is.** `crates/**/locales/<lang>/<domain>.ftl`, next to the code. A language is still a directory, and no code names a language.

### English stays built in

`translations!` compiles only `en-US` from now on, together with a table of English fingerprints. Shipping English as a file too was considered and rejected, for these reasons:

- **Always there.** The login window, the lock screen and the recovery tools must show text even when `/usr/share` is damaged or a package is half installed. macOS gets the same guarantee because the development-language `.lproj` sits inside a sealed bundle.
- **It defines the version.** The English compiled into a binary is exactly the English its code was written for. The fingerprints come from it, so it has to be in the binary.
- **Cost, measured** with a throwaway on the mini, fluent-bundle 0.16, using Settings, the largest domain at 28.9 KB English and 31.5 KB Swedish, 601 messages, single runs at load 2:

| | Compiled in (today) | Read from file |
|---|---|---|
| Parse the Swedish Settings bundle | 154 µs | 166 µs warm; about 200 µs with the file dropped from the page cache |
| Heap for that bundle | 451 KB | 451 KB |
| No pack installed (English user) | – | under 3 µs per domain for the failed open (12 opens took 6-33 µs) |
| All 33 desktop Swedish domains | – | 0.67 ms, 1.7 MB heap |

Reading from a file costs about 10 µs per domain when the file is cached. Medians over 21 runs on a busy machine were noisier, 305 µs compiled and 375 µs warm, but the gap was about the same.

**Memory goes down.** Today a Swedish user has both the Swedish and the English bundle parsed at start, 451 KB and 448 KB for Settings. The new loader parses English only on the first miss, so a complete translation never parses English. That saves about as much heap as the translation itself uses. The English source stays in the binary's read-only data, and those pages are not read until they are needed.

**Disk goes down too.** Today every binary carries every language of every domain it links, and the toolkit's strings are copied into each app. With packs, each binary carries English only.

### Version matching: a fingerprint for each message

- **At build time.** The macro computes a 64-bit FNV-1a hash of each English message's normalised source: the value and its attributes, trailing whitespace removed, plus the source of every message and term it references, followed transitively. The table is sorted by id and embedded in the binary.
- **When the pack is built.** For each translated message, the tool writes into `<domain>.english` the fingerprint of the English message with that id in the same commit. Fluent's rule that a changed meaning gets a new id, together with the check, keeps the translation current with that English.
- **At load.** A translated message is used only if its recorded fingerprint equals the binary's. Every other message falls back to the built-in English. The same happens when a translated message fails to format: its English is used and the miss is logged once, as today.

What this gives:
- A program never shows a translation made for different English, whether its package is older or newer than the pack.
- A fixed typo reaches every installed version whose English for that string is unchanged.
- An app pinned to an older toolkit gets the toolkit strings that still match and English for the rest.
- No version numbers have to match, so the packs need no version pins.

The hashing code lives in one small crate, `bande-l10n-source`. Both the macro and the pack tool use it, so the two sides cannot hash differently. One test (below) proves it end to end.

### Loading

- **Lazy, per domain.** A domain is loaded the first time its `LazyLock` is used, as today.
  - The loader opens exactly `<root>/<lang>/<domain>.ftl` for the chosen language; it never scans directories.
  - It reads the fingerprints, parses the file once, and keeps the set of accepted ids.
  - The English bundle is parsed only on the first miss.
- **Only the chosen language is read.** The preferred list comes from `LANGUAGE`, then `LC_ALL`, `LC_MESSAGES` and `LANG`, and it is negotiated with `fluent-langneg`, which is already in the lock file.
  - For each requested tag the order is: exact tag, then without the region (`pt-BR`, then `pt`), then the next language in the list, then English.
  - Siblings are not tried (Android would try fr_FR for fr_CH), because translations are stored language-only anyway.
- **One language per program (macOS).** The program's own domain decides the language: the first preferred language for which its pack exists. Library domains follow that choice. An app with no Swedish pack therefore shows English everywhere, toolkit included, instead of a mix.
  - `bande_app::run` and the shell's and services' `main` name their domain once.
  - A library used without this falls back to negotiating for itself.
- **No live reload.** A running program keeps the bundles it has parsed. Domains loaded later read whatever is installed at that moment, and the fingerprints keep each message correct. A language change needs a log-out, as now: the macOS bar, and no polling or file watching.
- **Fluent built-ins.** Plurals keep working through the CLDR rules compiled into `intl_pluralrules`, which cover every language, so packs need no data for them. Grammatical gender and case are done with selectors and term attributes written by the translator (`-app-name.gender`). Fluent has no inflection engine like Apple's, and none is planned.

### Packages

| Package | Contains | Relations |
|---|---|---|
| `bande-desktop-l10n-<lang>` | every desktop-repo domain, the toolkit and libraries included | `Architecture: all`, `Section: localization`, `Enhances: bande-desktop`, no Depends |
| `bande-<app>-l10n-<lang>` (one per app, e.g. `bande-mail-l10n-sv`) | that app's own domain | `Depends: bande-<app>, bande-desktop-l10n-<lang>` (unversioned), `Enhances: bande-<app>` |
| `bande-browser-l10n-<lang>-<region>` (web repo, exists) | Firefox language pack and Bande's `brand.ftl` | unchanged: `Depends: bande-browser (= version)`, `Enhances` |

- **Naming** follows Debian: `<package>-l10n-<lang>`, lower case, with a region only where the translation has one. That is the same pattern as `firefox-esr-l10n-sv-se` and `libreoffice-l10n-sv`.
- **Per-app packs** match how apps are released: each app is tagged on its own.
- **Unversioned Depends** are safe because of the fingerprints. Removing an app removes its pack, and the desktop pack is pulled in for the toolkit strings.
- **The Browser keeps its exact pin**, because Firefox refuses a language pack from another version.
- **Size per language:**
  - desktop pack: about 107 KB of Fluent plus about 75 KB of fingerprints, under 60 KB as a compressed `.deb` (estimate);
  - app packs: 2-20 KB each, 83 KB for all 16 apps' Swedish;
  - Browser: under 1 MB, as before.

### Installing on demand

- **One rule in `bande-packages`.** For each chosen language and each installed package `P`, install `P-l10n-<lang>-<region>` if it exists, otherwise `P-l10n-<lang>`. This is the same lookup `settings/src/spelling.rs` already does for `hunspell-*`. It also brings Debian's own packs, such as LibreOffice and Firefox ESR, for free.
- **Settings ▸ Language.**
  - The list puts first the languages that have a desktop pack in the APT index or installed.
  - Choosing one writes `locale.conf` as today, runs the rule with progress, and then offers Log Out.
  - Offline, the language is still set and the page says that Swedish text is fetched when the computer is online.
  - `translated()` reads the pack index instead of the compiled table.
- **Later installs.** When Software installs an app, or the update check runs, the same rule runs for the chosen languages. It is event-driven by those actions and needs no timer.
- **Setup assistant, installer and images.** They install the desktop pack and the apps' packs for the language chosen during setup.
- **Per-app language** (`LANGUAGE` at launch, already in Settings): picking a language for one app installs that app's pack and the desktop pack for that language.

### Checks and tests

- **`ci/check-l10n.py` becomes `bande-l10n-tool check`.** It is a Rust binary in `crates/libs/l10n` built on `bande-l10n-source`. One real Fluent parser replaces the regex, and the apps repo runs the same tool instead of its own copy of the script.
  - It keeps every rule it has now: syntax, duplicates, variables, unused ids, and complete or percentage per language.
  - It adds two rules: every domain name is unique across both repos, and a message may reference only messages its English also references.
- **`bande-l10n-tool pack <lang> <out>`** writes the `.ftl` and `.english` files that the deb build installs.
- **`ci/l10n-complete`** stays the list of languages that must be complete before a release, `sv` for now. Packs are built for every language directory, complete or not. A language below complete shows its percentage in Settings.
- **The made-up-language test** (`qaa`, in `crates/libs/l10n`) becomes the one regression test for the version boundary. It packs the fixture with the tool into a temporary root, then changes one English message, and checks:
  - the unchanged message is translated;
  - the changed one falls back to English;
  - a message with no translation is English;
  - a pack that does not exist gives English, not an id.
- **Toolkit coverage in the apps gate.** The apps gate reports how many of the toolkit messages at the pinned desktop revision match the current desktop pack. It warns below 100% and never fails.

### Translation-only releases

- **Tags.** A new tag kind, `l10n-YYYYMMDD` (with `.N` if there is more than one a day), on `main` in the desktop repo and in the apps repo.
  - `release.yml` runs only the stage that builds the packs: the tool, `dpkg-deb`, `check-debs` and lintian, with no `cargo build` of programs. Then it publishes.
  - The version is `<last release>+l10nYYYYMMDD`. That sorts above the release it fixes and below the next one: `0.9.0+l10n20261110` is greater than `0.9.0` and less than `0.9.1`, checked with `dpkg --compare-versions`. `ci/deb/version.sh` must keep counting only from `v` tags.
- **Normal releases** build the packs too, at the release version, so a release always carries its current translations.
- **Apps.** An `<app>-v` tag also builds that app's packs. An `l10n-` tag in the apps repo rebuilds only the packs whose files changed since the last tag.
- **Browser.** Its packs keep coming from the browser build. Mozilla's strings change only with ESR releases. A repack of Bande's own `brand.ftl` without a rebuild is possible later and has not been checked.

## Moving Swedish

No migration code is needed.
- The `locales/sv/` files stay where they are.
- The macro stops embedding them.
- The deb build packs them.

On the owner's computer, install `bande-desktop-l10n-sv` and the app packs once, at the same time as the release that drops the compiled-in Swedish. Until then a Swedish session shows English.

## Stages

1. **Loader and tool** (desktop repo). The work:
   - `bande-l10n-source`;
   - the fingerprints in the macro, so `translations!` compiles English only;
   - the file loader with lazy English;
   - negotiation, and one language per program in `bande_app::run` and the shell's `main`;
   - `bande-l10n-tool check|pack`, which replaces `ci/check-l10n.py`;
   - the `qaa` test;
   - the deb build packs `bande-desktop-l10n-sv`.
   
   Update languages.md and `docs/architecture/packaging.md` in the same change. Size M. Risk low: an error shows English.
2. **Settings and installing.** The `bande-packages` rule; Settings' language list, install and offline text; Software and update hook; setup assistant and images. Size M. Risk medium: package installs during setup, and Settings' text when offline.
3. **Apps repo.** Move the pin to the stage 1 desktop, per-app packs in `build-app.sh`, the shared tool replacing `ci/check-l10n.py`, the toolkit coverage report. Size S-M.
4. **Translation-only releases.** The `l10n-` tags in `release.yml`, the version rule, the apps-repo equivalent. Size S. Risk: the apt publish path for a run that builds only `all` packages.
5. **Browser.** Settings and the rule pick up `bande-browser-l10n-*` with nothing else to do. Optionally, a `brand.ftl` repack. Size S.

Stages 1 and 3 must reach the owner's computer together, with the packs installed.

## Risks

- **The fingerprint is strict.** An English typo fix shows English in every language until the pack is rebuilt. Normal releases rebuild the packs, so this only happens between an app release and a pack that was not rebuilt with it. That is the intended trade-off: the owner asked that strings never come from another version.
- **A pack goes missing.** Text is in English, never broken. The installer and images have to install packs, or a fresh Swedish setup starts in English.
- **Two packages are both named `bande-browser`:** `apps/browser`, the own engine (paused), and the web repo's Firefox build. This has to be settled before `bande-browser-l10n-*` can mean one thing.
- **The Rust tool replaces a Python script in both gates.** Compiling it adds a little gate time; it is a small crate.

## Owner decisions

1. **Version matching: a per-message English fingerprint** (recommended). The alternative, an exact version pin as Debian does for Firefox, forces lockstep upgrades and fails for toolkit strings in apps that pin an older desktop.
2. **English stays built in** (recommended). Measured above: about 10 µs per domain more to read a file, less heap than today, and text even when `/usr/share` is broken.
3. **Packs: one per language for the desktop and one per app per language** (recommended, matches app releases). The alternative is one `bande-apps-l10n-<lang>` for every app.
4. **Install on demand only** (recommended, Debian's way). macOS installs every UI language with the system instead; that would be about 0.3 MB per language here and could be revisited.
5. **One language per program, picked by the program's own domain** (recommended, as macOS does).
6. **Translation-only release tags `l10n-YYYYMMDD`** on `main` in both repos (recommended).
