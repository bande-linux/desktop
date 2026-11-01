# Keyboard layouts and languages

Gap (owner, *important*): "its not possible to install more keyboard layouts or languages in settings".

## Summary

Languages cannot be added because the `locales` package is not installed on the mini, so only `C.UTF-8` exists and
Settings shows "No languages are installed". Debian 13's localed already generates a missing locale when it is asked
for one, so once `locales` is installed Settings only needs to offer every supported locale. Keyboard layouts can be
added today (Settings > Keyboard > Add Input Source…), but the picker is hard to use: 8 results, no multi-word search,
Mac variants out of reach. The login screen always uses US.

## What the owner sees

- **Language & Time > Language:** the row says "No languages are installed" and the Region row is missing. Nothing on
  the page can add one.
- **Keyboard > Input Sources:** one row, "English (US)", which comes from the built-in fallback (`config.kdl` has no
  `input-sources`, `/etc/default/keyboard` does not exist). "Add Input Source…" opens a search field. With nothing
  typed it lists "A user-defined custom Layout", "Abkhazian (Russia)", "Akan" and so on, 8 rows and no more. Typing
  "german" shows 8 of 25 matches, so "German (Macintosh)" never appears. "english" shows 8 of 46 matches.
  "swedish mac" finds nothing. On an Apple keyboard the Macintosh variants are the ones people want. This is the
  likely reason the owner found no way to add layouts. The flow itself works: it is tested, and the installed build
  (`0.0.0+git20261031.4b292c7`, installed 06:43) has it.
- **Login screen:** always US, whatever the user picked, because the greeter reads only `/etc/default/keyboard`.
- **UI language:** even with a language set, Bande's own apps stay in English. Only `en-US` `.ftl` files exist.
  Which languages to translate is a separate owner decision, already open.

## Findings

Live system (read-only checks):

- `locale -a` gives `C`, `C.utf8`, `POSIX`. `dpkg -l locales locales-all` shows neither installed.
  `/etc/locale.gen`, `/usr/sbin/locale-gen`, `/usr/share/i18n/SUPPORTED` and `/usr/share/i18n/locales/` are all
  missing. `localectl`: `LANG=C.UTF-8`, X11 layout unset.
- `/usr/lib/systemd/systemd-localed` (systemd 257.13, Debian 13) contains `/usr/sbin/locale-gen`, `/etc/locale.gen`,
  `/usr/share/i18n/SUPPORTED`, `locale_gen_enable_locale` and "Locale %s not installed, refusing.". In upstream v257
  `localed.c`, `SetLocale` checks polkit and then, if locale-gen is available, enables the locale in
  `/etc/locale.gen`, runs `locale-gen` and only then replies. Without locale-gen it refuses any locale that is not
  already installed. Source: github.com/systemd/systemd v257 `src/locale/localed.c`, `localed-util.c`.
- The same binary and `libsystemd-shared-257.so` refer to `/etc/vconsole.conf` and
  `/etc/X11/xorg.conf.d/00-keyboard.conf`, but not to `/etc/default/keyboard`. v257 `SetX11Keyboard` writes
  `XKBLAYOUT`, `XKBMODEL`, `XKBVARIANT` and `XKBOPTIONS` to `/etc/vconsole.conf` and reads them back from there first.
  These are only data files; no X11 code is involved.
- `/etc/default/locale` is a symlink to `/etc/locale.conf` (from `images/rootfs/etc/locale.conf`), so PAM's `pam_env`
  for greetd and localed read and write the same file.
- Fonts: only dejavu, inter, manrope, jetbrains-mono, noto-color-emoji, noto-mono and urw are installed.
  `fonts-noto-core` and `fonts-noto-cjk` are Recommends of `bande-desktop`
  (`packaging/bande-desktop/debian/control:44-45`), but the image installs the desktop with
  `--no-install-recommends` (`images/customize.sh:37`), so Arabic, Thai, Indic and CJK text has no font.
- `org.freedesktop.locale1.set-locale` is already allowed for local active `sudo` members
  (`crates/apps/settings/data/50-bande-settings.rules:6`). `set-keyboard` is not, and is `auth_admin_keep`.

Code:

- `crates/apps/settings/src/locale.rs:132-138` builds the language list from `locale -a`. `:74-92`
  `installed_locales` keeps only `*_*.utf8` entries and names them from `/usr/share/i18n/locales/<code>`
  (`locale_name`, `:53-68`), which is also missing. `language.rs:72-74` then shows `no-locales`, and
  `region_rows` (`:95-97`) returns nothing. Both are `PopUpButton` menus (`language.rs:24-59`), fine for 3 locales
  but not for ~300.
- `locale.rs:107-121` writes `LANG`, `LANGUAGE=<ll_CC>:<ll>` and the five `LC_*` region categories through
  `SetLocale(…, interactive=true)` (`:199`). Settings already does this right; only the list is wrong.
- Root cause (languages): the image's package list (`images/build.sh:44-48`, minbase) and the Settings package
  (`crates/apps/settings/Cargo.toml:43`, `depends = "$auto, fonts-inter, bande-config"`) never pull in `locales`.
- `crates/libs/keyboard/src/layouts.rs:23-63` parses `/usr/share/X11/xkb/rules/evdev.xml` (590 layouts and
  variants). It keeps the name, description and short label, but drops `countryList` and `languageList`, and keeps the
  `custom` layout. `search` (`:99-113`) matches the whole query as one substring, so "swedish mac" matches nothing.
- `crates/apps/settings/src/pages.rs:37` `SOURCE_RESULTS = 8` and `:836-846` `take(SOURCE_RESULTS)` cut the list.
  The input source picker is its own mechanism (`adding_source`, `Anchor::SourceSearch`, `Intent::AddingSource`,
  `Intent::SourceQuery` and a special Enter handler, `view.rs:836-844` and `:1082-1107`). It duplicates the generic
  `Picker` used for time zones and login items (`view.rs:100-112`, `:651-663`, `:1134-1144`, `clock.rs:53-113`).
- Live reload already works: Settings writes `InputSources` through `org.bande.Config`
  (`crates/services/config/src/service.rs:789-815`). The compositor follows PropertiesChanged and re-reads the
  config (`crates/platform/compositor/src/input_settings.rs:48-70`). `keymap.rs:116-154` sets the new XKB
  layout,variant list, keeps the active source, and the HUD, Alt+Super+Space cycling and the bar menu
  (`crates/shell/bar/src/menus.rs:196-208`) follow. The headless tests in
  `crates/platform/compositor/tests/headless/keyboard.rs` cover this. Nothing to fix here.
- Root cause (login layout): `crates/services/config/src/lib.rs:47` `SYSTEM_KEYBOARD = "/etc/default/keyboard"`,
  parsed by `system_input_sources` (`:263-298`, falls back to `us`). On Debian 13, localed writes
  `/etc/vconsole.conf`, not that file. The greeter session applies only `InputSettings::system()`
  (`crates/platform/compositor/src/main.rs:518`, `input_settings.rs:33-35`), and Settings has no control that calls
  `SetX11Keyboard`.
- Input methods: the compositor serves text-input-v3, input-method-v2 and virtual-keyboard
  (`crates/platform/compositor/src/ime.rs`), and the compositor package only Suggests `fcitx5` (`Cargo.toml:46`).
  Settings does not offer input methods (`docs/experience/maturity-gaps.md:118`, open).
- UI language: `crates/libs/l10n/src/lib.rs` already picks the bundle from `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and
  `LANG`. Branch `origin/wip/l10n-ready` (one WIP commit, 75 files) adds ICU number formatting, the en-XA
  pseudo-locale and a workspace message test (`docs/experience/l10n.md` on that branch). No translations exist yet.

## Options

**Languages: where locales come from**

| Option | Cost | Trade-off |
| --- | --- | --- |
| A. `locales` package plus localed's built-in locale-gen | Depends line, image list, list from `SUPPORTED` | Generates one locale on demand (a few seconds), under the polkit rule we already ship. No privileged code of ours. |
| B. `locales-all` | 236 MB installed, 11 MB download | Every locale already exists, nothing to wait for. Still needs `locales` for names and `SUPPORTED`. |
| C. Our own root helper (pkexec or a system D-Bus service) running `localedef` | New privileged binary, policy file, tests | Duplicates what localed does already. |

**Languages: system-wide or per user**

| Option | Trade-off |
| --- | --- |
| System-wide through localed (today) | Already built. The greeter, console and all users follow. Needs an administrator, which the `sudo` rule covers. |
| Per user (`language`/`region` in `config.kdl`, exported by the compositor with the session environment and set on spawned clients) | Like macOS, and no admin needed. But every session client is started with the compositor's own environment, so `LANG` has to go into `session_environment` (`environment.rs:39-48`) and into every spawn. The greeter would still use the system locale. |

**Fonts**

| Option | Trade-off |
| --- | --- |
| Depend on `fonts-noto-core` (43 MB) and `fonts-noto-cjk` (91 MB) | Text in any script shows right away. Uses disk. |
| Depend on `fonts-noto-core` only, install CJK on demand through PackageKit | Smaller. Needs PackageKit, which is not installed on the mini either (see `software-update.md`), plus a `package-install` polkit grant. |

**Keyboard picker**

| Option | Trade-off |
| --- | --- |
| Raise the cap only | One line, but still no Mac variants with "swedish mac", and an empty search still opens on "A user-defined custom Layout". |
| Reuse the generic `Picker`: word-wise search including country and language codes, all matches, suggestions for the current region | About 2 hours. Removes the duplicate picker code. |
| A browse tree (language > layouts), like GNOME | More UI to build, and the search already reaches everything once it matches word by word. |

## Recommendation

- **Languages:** option A. Make `bande-settings` depend on `locales`, and list every UTF-8 locale in
  `/usr/share/i18n/SUPPORTED` with a search picker like the time zone one. localed generates the chosen locale.
  Keep the language system-wide, as it is now.
- **Fonts:** make `bande-desktop` depend on `fonts-noto-core`, so the no-recommends image gets it.
- **Keyboard:** move input sources onto the generic `Picker`, with word-wise search, country and language matching,
  all results and region-based suggestions. Hide `custom`. Add "Use at Login Screen", which calls localed
  `SetX11Keyboard`, and have the config read `/etc/vconsole.conf` first.
- **Input methods:** keep them out of this plan. They come later as one milestone.

## Implementation plan

**Status (2026-11-01):** milestone 4 (layout picker) is on `main`. Milestone 1 (packages) belongs to the images and machine work; milestones 2, 3 and 5 are next; 6 is later.


Each step is one short branch from `main`, gated by fmt, workspace clippy `-D warnings` and workspace tests, and
checked on the mini before merge.

1. **Ship `locales` and the Noto fonts** (about 1 h)
   - `crates/apps/settings/Cargo.toml`: `depends = "$auto, fonts-inter, bande-config, locales"`, and mention
     languages in `extended-description`.
   - `packaging/bande-desktop/debian/control`: move `fonts-noto-core` (and `fonts-noto-cjk` if the owner agrees)
     from Recommends to Depends.
   - `images/build.sh:44-48`: add `locales` so minbase has it before the desktop debs.
   - On the mini: `sudo apt install locales fonts-noto-core`. Run `busctl call org.freedesktop.locale1
     /org/freedesktop/locale1 org.freedesktop.locale1 SetLocale asb 2 LANG=C.UTF-8 LC_TIME=sv_SE.UTF-8 false`,
     then check that `locale -a` lists `sv_SE.utf8` and `/etc/locale.gen` has it enabled. Restore with
     `… SetLocale asb 1 LANG=C.UTF-8 false`.
   - Proof: `dpkg-deb -f` on the built debs shows the new Depends. No Rust test, since there is no logic to test.
2. **Language and Region from every supported locale** (about 2 h)
   - `locale.rs`: replace `installed_locales` and `listing()` with `supported_locales(supported: &str,
     sources: &Path)`. It reads the UTF-8 lines of `/usr/share/i18n/SUPPORTED` (`sv_SE.UTF-8 UTF-8`,
     `sr_RS@latin UTF-8`; skip ISO-8859 lines), names each with the existing `locale_name`, sorts and dedupes.
     `LocaleState.installed` becomes `available`.
   - `view.rs`: add `Picker::Language` and `Picker::Region` (setting ids `Language` and `Region`).
     `pick_results` matches locale names word by word. `picked` sends `Intent::Locale`.
   - `language.rs`: drop the `PopUpButton` menus. Each row shows the current name and "Change…" (only when
     `allowed`) and opens the inline search, as `clock.rs` `zone_rows` does. Region gets a first result row,
     "Same as Language", which sends `Region(None)`. Keep the formats preview.
   - `locales/en-US/bande-settings.ftl`: `no-locales` becomes "Language support is not installed". Add
     `search-languages`, `choose-language`, `choose-region` and `same-as-language` (already exists, reuse it).
   - Tests: in `tests/general.rs`, replace `languages_come_from_installed_locales…` with a `supported_locales` test
     on a real `SUPPORTED` excerpt: UTF-8 only, `@latin` kept, names from a sample source file, sorted and
     deduped. In `tests/panes.rs`: Change… > type "swed" > activate "Swedish (Sweden)" gives
     `Command::Locale(Language("sv_SE.UTF-8"))`; Region > "Same as Language" gives `Region(None)`; a refused
     privilege shows no Change… button. Refresh `language-time-light.png`.
3. **Waiting for localed, and saying what changes** (about 1-2 h)
   - `locale.rs` worker: before `set_locale`, send an update that marks the chosen code as pending. The row's detail
     reads "Setting up Swedish…" until the PropertiesChanged arrives, or until the error comes back, which then
     shows under the row (existing `failed`).
   - `language.rs`: when the language has no Bande translation, the caption under the language row says
     "Bande's own apps stay in English for now; other apps use this language." That needs
     `bande_l10n::translated()`, which returns the tags the workspace ships (`["en-US"]` today). Keep the
     "next time you log in" caption.
   - Tests: a fake `org.freedesktop.locale1` on `bande-test-bus` (as `tests/privilege.rs` fakes polkit) that
     records `SetLocale`. Check: the assignments keep unrelated `LC_*` entries; pending shows and clears when the
     fake emits `Locale`; a fake error reverts the row and shows the sentence.
4. **A layout picker that finds everything** (about 2 h)
   - `crates/libs/keyboard/src/layouts.rs`: `Layout` gains `countries: Vec<String>` (ISO 3166) and
     `languages: Vec<String>` (ISO 639-2), inherited by variants that have none. Skip the `custom` layout.
     `search` matches when every word of the query is in the name, or equals the code, a country or a language.
     Add `suggested(country: &str)`: layouts for the region's territory, base layout first.
   - `crates/apps/settings`: delete `adding_source`, `Anchor::SourceSearch`, `Intent::AddingSource`/`SourceQuery`,
     `SOURCE_RESULTS` and the Enter special case. Add `Picker::InputSource`; `picked` pushes the code through
     `edit_sources`. An empty query shows `suggested(territory of LC_TIME or LANG)`, then a hint to type. A
     non-empty query shows every match (the page scrolls). Each result keeps the code as detail.
   - `settings-panes` registry: keywords "layout", "language", "keyboard language" on `input-sources`.
   - Tests: `crates/libs/keyboard/tests/layouts.rs` on the real `evdev.xml` excerpt: "swedish mac" finds
     `se(mac)`, "german" includes `de(mac)`, "se" and "swe" find Swedish, `custom` is absent, and `suggested("SE")`
     starts with `se`. In `tests/panes.rs`: the existing add, Escape and full-list tests move to the new picker; a
     view with region `sv_SE` and an empty query lists "Swedish" first; picking "German (Macintosh)" gives
     `codes(["se", "de(mac)"])`. Refresh `keyboard-light.png`.
5. **The login screen uses your layouts** (about 2 h)
   - `crates/services/config/src/lib.rs`: the system keyboard is read from `/etc/vconsole.conf` first and
     `/etc/default/keyboard` second (same `XKBLAYOUT=`/`XKBVARIANT=` lines, so `system_input_sources` is reused).
     `SYSTEM_KEYBOARD` becomes `SYSTEM_KEYBOARDS: [&str; 2]`.
   - `crates/apps/settings`: a `keyboard.rs` worker with a `locale1` proxy for `SetX11Keyboard(layouts, "",
     variants, "", convert=false, interactive=true)` and the `X11Layout`/`X11Variant` properties. The Input Sources
     list ends with "Use at Login Screen" when the system layouts differ from the user's, or "The login screen uses
     these layouts" when they match. Add `Privilege::Keyboard` (`org.freedesktop.locale1.set-keyboard`) and add
     that action to `data/50-bande-settings.rules`. `software-update.md` edits the same file; merge, don't
     overwrite.
   - Tests: a config unit test where `vconsole.conf` wins over `default/keyboard` and both fall back to `us`. A
     Settings test against the fake locale1 from step 3: the button sends `("se,us", "", "mac,", "")`, and the row
     changes when `X11Layout` changes. Extend `tests/privilege.rs` with the new action.
   - On the mini: press the button, then `cat /etc/vconsole.conf`, and log out to the greeter to type in the
     layout. Warn the owner before the logout.
6. **Later, own branch: input methods** (2-3 milestones, not sized here)
   - "Add Input Method…" for Chinese (Pinyin), Japanese (Mozc) and Korean (Hangul): install `fcitx5` plus the engine
     and `fonts-noto-cjk`, write `~/.config/fcitx5/profile`, and start fcitx5 through its XDG autostart entry.
   - Decide who switches between xkb layouts and the input method: the compositor's Alt+Super+Space, or fcitx5's
     own trigger.

## Risks

- `SetLocale` returns only after `locale-gen` finishes. Debian's `locale-gen` rebuilds every enabled locale each
  time, so with many locales this takes seconds. The call runs on the worker thread with no zbus timeout, but the UI
  must show it is busy (step 3).
- Without locale-gen (the `locales` package removed), localed checks `LANGUAGE=sv_SE:sv` as a locale and refuses
  it. Step 1's Depends prevents this. If it still happens, the error shows under the row.
- `SetX11Keyboard` validates with libxkbcommon if localed can load it. An invalid code fails the call with an
  error, so the picker must only offer registry codes, which it does.
- The greeter session reads the system layouts once at start. A change applies at the next greeter start, not to a
  greeter that is already showing.
- Apple ISO keyboards swap the `<`/`§` keys (`hid_apple iso_layout`). That is a separate gap, not a layout problem.
- `fonts-noto-cjk` adds 91 MB if it becomes a Depends.

## Decisions for the owner

1. **Per-user or system-wide language.** Recommended: system-wide for now (localed, administrators only). This is
   what exists, and the greeter and console follow it.
2. **CJK fonts always or on demand.** Recommended: always (`fonts-noto-cjk` as a Depends, 91 MB). On demand needs
   PackageKit and a package-install grant.
3. **Input methods (fcitx5) in Settings now or later.** Recommended: later, after layouts and languages land.
4. Which languages Bande's own UI ships first is already an open decision. This plan does not need it, but the
   step-3 caption keeps saying "English" until then.
