# Settings

Part of [Bande Intents](../../intents.md). Main noun `setting`. Owner: `bande-settings-intents`.
Work package C2.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `settings.setting` | title~ text, value text, kind `switch` `choice` `number` `time` `list`, accepts list<text>, labels list<text> (the same in the person's words), minimum, maximum, step integer, pane text, writable bool | title, value, pane |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `settings.find` | `text`, `pane` | `read` |  | done | A |
| `settings.get` | `settings` (value, kind, what is accepted) | `read` |  | done, for the settings below | A |
| `settings.open_pane` | `pane` | `show` |  | done | A |
| `settings.set` | `setting`, `value`, `until` | `change`, `disrupt` or `critical` as marked | the stored value | done, without `until` (C8), for the settings below | A |

Served today are the settings that `org.bande.Config` stores as one value, and Do Not Disturb; a
setting marked `critical` is `Unavailable` until the prompt is built (C3). Not yet: the wallpaper
(needs D1a), sound levels and devices, brightness, the power mode, displays, Wi-Fi, Bluetooth and
airplane mode, languages, time and what is only read. They are `NotFound`.

Phrases in Search: "{setting} {value}".

`value` is text: the setting's word, `on` or `off`, a number, a time, or a list separated by commas.
`until` puts the stored value back [at that time](../../intents.md#later-and-by-itself). A setting
of one display or device is a thing of its own (`settings.setting:display-scale@…`), and the
wallpaper is one for light and one for dark and takes a `files.file`. "Theme" finds `appearance`:
the registry (`bande-settings-panes`) has labels and keywords and is UI-free. `settings.get` gives
proxies without user information. A setting the computer does not have is `NotFound`, by the same
filter as Settings' search. The owner applies values; what it needs of
`crates/apps/settings/src/client.rs` moves to a library. The registry gives every setting a word and
a mark, for muting the microphone by value, in an exhaustive match, so the compiler finds one
without:

| Mark | Settings |
| :--- | :--- |
| `set`, `change` | appearance, style, accent, tinted frame, window buttons, text size, wallpaper and fit; taskbar edge, size, gap, floating and auto-hide; brightness, night light, its schedule and warmth; output volume, mute, balance and device, muting the microphone, alert sound and its volume, sound effects, volume feedback; Do Not Disturb and its schedule; power mode; pointer, trackpad, key repeat, input sources; contrast, motion, transparency, zoom, cursor size, flash; sticky, slow and bounce keys, Caps Lock, Compose; clock format; the kinds Search shows; recent files; app volumes |
| `set`, `disrupt` | screen reader; unmuting the microphone, its volume and its device; Wi-Fi, Bluetooth and airplane mode power; display-off delays; app languages; a display's use, arrangement, resolution, scale, rotation, refresh and HDR, with the 15-second trial, whose Keep and Revert only the Settings window draws today |
| `set`, `critical` | time zone and automatic time, language and region, computer name, which known networks join by themselves, automatic updates and how often, proxies, login items (apps that are installed, without arguments), default apps, folders left out of search, sending reports, what the lid does, sleep delay, charge limit |
| `read` | battery, storage, power and sleep status, software update status, backup status, Ethernet status, security status |
| `hidden` | everything else, among it password delays, lock message, previews, shortcuts, text replacements, sign-in accessibility, saved passwords, app permissions, keyboard access, accounts and users, clipboard history |
