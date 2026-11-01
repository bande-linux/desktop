# Settings: structure, page pattern and options

Spec of 2026-10-30, branch `night/settings-spec`. This is research and a spec only. Nothing was
built or run.

It answers the owner's note: "Settings is not super well structured (no categories or red line
to the options and lacking a lot of options compared to macOS/Windows/other DE)". The spec has
four parts:

1. A sidebar of four named groups, each answering one question (§3).
2. One page pattern that every pane and subpage follows (§4).
3. Where every option of today goes (§5).
4. A gap table against macOS 15 and 26, Windows 11, GNOME 48 and Plasma 6, marked now, later or
   never (§6). Ordered build slices follow it (§7).

It applies `DESIGN.md`: settings only where people genuinely differ, one obvious way, and
progressive disclosure. It also applies Datum: the sidebar (§8), the controls (§9) and the type
roles (§7). macOS sets the quality bar but is not the template: the grouping and the page pattern
are Bande's own.

## 1. What exists today

Settings is one window. On the frame sit a search field and a flat sidebar; the selected pane
fills the sheet. The panes are declared once in `crates/libs/settings-panes` as pane → section →
setting, each setting with a `Kind`. The launcher searches the same list. `crates/apps/settings`
draws the panes (`pages.rs`, `view.rs`, `widgets.rs`) and talks to the services.

| Pane today | Rows | Backend |
| :--- | :--- | :--- |
| Wi-Fi | Wi-Fi switch; Networks (join, password, status) | NetworkManager through `libs/network` |
| Bluetooth | Bluetooth switch; pairing request card; My devices (Connect, Disconnect); Nearby devices (Pair) | BlueZ through `libs/bluetooth`; the pairing agent lives in Settings |
| Network | Ethernet status; Configure IPv4 (Automatic, Manual) with IP address, Router and DNS fields; VPNs (Connect, Disconnect); Import VPN… | NetworkManager through `libs/network` |
| Notifications | Do Not Disturb; Application notifications, which expand inline to Allow, Banner style and Show in Notification Center | `org.bande.NotificationCenter` (`services/notify`) |
| Sound | Alert sound, Alert volume, interface sound effects, volume feedback; Output volume, Mute; Play sound through; Input volume, Mute microphone, Input level; Record sound from; Apps playing sound | the config for alerts and effects, played by `services/sound`; PipeWire through `libs/audio` for the rest |
| General | a hub of rows leading to the six subpages below | — |
| ↳ About | Name; Model, Chip, Memory, Storage, Bande, System, Serial | hostnamed for the name; `/proc` and the device tree |
| ↳ Software Update | status with Check Now, Download Updates or Restart Now; the package list. Updates download in the background and install at the next restart (PackageKit offline updates); the result shows once after the restart | `bande-packages` (PackageKit, offline) and logind; `bande-update-check.timer` |
| ↳ Storage | usage bar by category; Trash with Empty Trash…; Largest files with Show in Files | statvfs and Settings' own scan (`storage.rs`) |
| ↳ Date & Time | Set time automatically; manual date and time; Time zone with a Change… picker; 24-hour time | timedated; the config's `clock-format` |
| ↳ Language & Region | Language; Region with a formats preview | localed and every UTF-8 locale in `/usr/share/i18n/SUPPORTED` |
| ↳ Login Items | Open at login: a list with Add… and Remove | XDG autostart files (`login.rs`) |
| Appearance | Appearance (Light, Dark, Auto); Accent colour; Tint window frames with the wallpaper | the config |
| Accessibility | Screen reader (only when Orca is installed), Zoom, Increase contrast, Reduce transparency, Reduce motion, Pointer size; Flash the screen; Sticky, Slow and Bounce keys | the config, applied by the compositor and served by the portal |
| Wallpaper | picture grid with Add Picture…; Picture size | the config and `libs/wallpaper` |
| Bar | Position, Size, Automatically hide, Menu bar | the config |
| Displays | Keep or Revert card; Arrangement; per display: Use as, Resolution, Size of text and controls, Refresh rate, Rotation, Variable refresh rate | the config's displays and arrangements, applied by the compositor; displays listed through wlr-output-management |
| Lock Screen | Dim the display after; Turn off the display and lock after; Sleep after | the config's `idle`, applied by `services/power` |
| Battery | level and 24-hour history; Power mode; Use Power Saver on battery; When the lid is closed (on the adapter) | UPower, power-profiles-daemon and the config |
| Keyboard | Input sources (add, reorder, remove); Repeat rate and Delay until repeat; Caps Lock key; Keyboard shortcuts (read only) | the config and the compositor; the shortcut table comes from `libs/keyboard` |
| Trackpad & Mouse | Trackpad: Tap to click, Secondary click, Natural scrolling, Tracking speed, Ignore while typing. Mouse: Natural scrolling, Tracking speed | the config and the compositor (libinput); devices found through udev |

Two config keys had no control: `accessibility { text-size }`, which the portal serves but the
toolkit does not follow, and `power { show-battery-percentage }`, set from the battery menu by
decision (roadmap).

What is wrong with the structure:

- **No groups and no red line.** The sidebar follows the registry's order: Wi-Fi, Bluetooth,
  Network, Notifications, Sound, General, Appearance, Accessibility, Wallpaper, Bar, Displays,
  Lock Screen, Battery, Keyboard, Trackpad & Mouse. General sits between Sound and Appearance,
  and Battery between Lock Screen and Keyboard.
- **Text only.** The sidebar has no symbols and no group labels, and a page has no header beyond
  its title.
- **Options in odd places.**
  - "Sleep after" is under Lock Screen.
  - The 24-hour clock is under Date & Time, though it is a format.
  - Wallpaper and "Tint window frames with the wallpaper" sit on two different panes.
  - Login Items hide inside General.
- **General is a drawer** of six unrelated subpages. Nothing tells you that an update waits
  inside it.
- **One-off patterns.**
  - Notification apps expand inline; it is the only expander in Settings.
  - Wi-Fi and Bluetooth put their power switch in the first row.
  - Choices of five and six options are segmented controls (the idle timers, the display
    scale).
  - The input source hint shows macOS glyphs (`⌥⌘Space`), which Datum forbids.
- **Failures** show as one line at the bottom of the page, with raw D-Bus text: "Not changed:
  org.freedesktop.DBus.Error.InteractiveAuthorizationRequired".
- **No administrator path.** Without a polkit agent or rules, changing the time zone, the
  language or the computer name fails for everyone (maturity gaps #2).

What already works well and stays:

- changes apply at once;
- search covers every setting, with weighted matches, highlighted rows and Return to jump to the
  best match;
- keyboard navigation works everywhere;
- the display arrangement reverts on a timer;
- the inline pickers for time zones, input sources and login items;
- the inline confirmation for Empty Trash.

## 2. What the references do

**macOS 15 and 26, System Settings.**

- One scrolling sidebar of about 30 panes, in clusters separated by space, with no labels. The
  account banner sits on top.
- A "Software Update Available" row appears at the top when an update waits.
- General is a hub of about 14 subpages.
- Top-level pages open with a header block: icon, title, a one-line description, and sometimes
  the master switch.
- Pages are grouped inset forms. Disclosure rows push subpages with a back button; rare details
  open in sheets ("Details…", "Options…").
- Search sits above the sidebar and highlights the matching control.
- Tahoe turned Control Center into a Menu Bar pane.
- The critique since Ventura:
  - the order reads as arbitrary and changes each release (Sequoia reshuffled it);
  - General is a dumping ground;
  - the window cannot grow wider;
  - people lean on search, which often fails (Tsai; Ars Technica via Wikipedia).

**Windows 11, Settings.**

- Home plus 11 labelled categories with icons: System, Bluetooth & devices, Network & internet,
  Personalization, Apps, Accounts, Time & language, Gaming, Accessibility, Privacy & security,
  Windows Update.
- A category page is a list of cards (icon, title, one-line description, chevron) that lead to
  subpages. The title is a breadcrumb: "System > Display".
- Expanders reveal sub-options, one level deep. "Related settings" close the page.
- Microsoft's own rules:
  - a switch for a binary setting;
  - radio buttons for up to five exclusive options, a combo box for more;
  - one-word section headers;
  - say in the description why a setting is unavailable;
  - apply at once, without a confirmation button.

**GNOME 48, Settings.**

- About 20 panels in one flat list with symbolic icons, and no group labels.
- GNOME 46 shortened the list:
  - Region & Language, Date & Time, Users, Remote Desktop, Secure Shell and About moved into one
    System panel;
  - Default Apps and Removable Media moved into Apps.
- Pages are boxed lists with these row types: switch, combo, action (a go-next arrow to a
  subpage), expander, entry, spin and property. One control per row; descriptions are row
  subtitles.
- Privacy & Security holds Screen Lock, Location, Camera, Microphone, File History & Trash and
  Diagnostics.
- Search filters the panel list, subpages included.

**Plasma 6, System Settings.**

- Quick Settings as the home page, then nine labelled categories: Input & Output, Connected
  Devices, Networking, Appearance & Style, Apps & Windows, Workspace, Security & Privacy,
  Language & Time, System.
- Modules apply through Apply, Reset and Defaults buttons.
- "Highlight Changed Settings" marks values that differ from the default.
- The 6.0 reorganisation avoided "a mega category with like 12 things in it".

**What Bande takes from this:**

1. Labelled groups read as order (Windows, Plasma). Unlabelled clusters read as arbitrary
   (macOS).
2. At most five panes per group, and a sidebar short enough to see whole.
3. A header with a symbol and a one-line summary orients the reader (macOS, Windows).
4. Hubs hide things. Use subpages for depth inside one topic, never as a "General" drawer.
5. Apply at once (macOS, Windows, GNOME); no Apply buttons.
6. A disclosure row opens a subpage with a way back (all four). No expanders.
7. Search reaches every setting and speaks every platform's vocabulary.
8. What needs the user rises to the top: macOS' update row, Windows' Home.

## 3. The sidebar

```
 Search
 3 updates available ›          only while something needs you
 CONNECTIONS
   Wi-Fi
   Bluetooth
   Network
 DESKTOP
   Appearance
   Bar
   Notifications
   Accessibility
 DEVICES
   Displays
   Sound
   Keyboard
   Mouse
   Trackpad
 SYSTEM
   Power
   Privacy & Security
   Apps
   Language & Time
   About
```

**The red line.** The list runs from the outside in, and each group answers one question:

- **Connections**: how does this computer reach the world?
- **Desktop**: how does my desktop look and behave, and how does it adapt to me?
- **Devices**: how do the screens, sound, keyboard and pointer work?
- **System**: how does the machine run, who may use what, what opens what, where and when am I,
  and what is this computer?

Inside a group, the panes people open most come first. About comes last, as everywhere.

- Each label is one plain word, in Datum's tracked capitals (the Label role in `ink3`).
- Each group has three to five panes: 17 rows in all, against today's 15 rows plus 6 hidden
  subpages.
- Groups follow the user's question, not the implementation:
  - Bluetooth sits with Wi-Fi as a radio you join, as on macOS, Windows and GNOME.
  - Accessibility sits with Appearance, as on macOS, because both shape how the desktop looks and
    responds.
- **Sizes.**
  - Rows are 28 pt with a 2 pt gap, and a 16 pt line symbol in `ink2` that turns accent when
    selected. The selection is a `well` fill with radius 6 (Datum §8).
  - The four labels and 16 rows need about 600 pt, so the default window grows from 900×600 to
    900×720.
  - The sidebar scrolls when the window is shorter, keeping the selected row in view.
- **Symbols are monochrome.** There is one signal colour (Datum §3), so the coloured squares of
  macOS and Windows have no place here.
- **The attention row** appears at the very top only while something waits for the user:
  updates ready to install, or a restart to finish them. It opens the page that resolves it, and
  disappears with the need.

| Group | Pane | Symbol | Holds | Subpages | Shown when |
| :--- | :--- | :--- | :--- | :--- | :--- |
| Connections | Wi-Fi | `network-wireless-symbolic` | the switch (in the header), networks | — | a Wi-Fi device exists |
| | Bluetooth | `bluetooth-symbolic` | the switch (in the header), pairing, my devices, nearby devices | — | an adapter exists |
| | Network | `network-wired-symbolic` | Ethernet, VPN | — | NetworkManager runs |
| Desktop | Appearance | new: `appearance` (a half-filled disc) | mode, accent, wallpaper, text size | — | always |
| | Bar | `view-sidebar-start-symbolic` | position, size, auto-hide, menu bar | — | always |
| | Notifications | `preferences-system-notifications-symbolic` | Do Not Disturb, apps | one per app | the notification service runs |
| | Accessibility | new: `accessibility` | vision, hearing, motor | — | always |
| Devices | Displays | `video-display-symbolic` | arrangement, brightness, Night Light, each display | Night Light | a display is known |
| | Sound | `audio-volume-high-symbolic` | output, input, sound effects, apps | — | PipeWire runs |
| | Keyboard | `input-keyboard-symbolic` | input sources, key repeat, modifier keys | Keyboard Shortcuts | always |
| | Mouse | `input-mouse-symbolic` | primary button, scrolling, tracking | — | a mouse exists |
| | Trackpad | new: `input-touchpad-symbolic` | click, scroll, pointer | — | a trackpad exists |
| System | Power | the live `battery-level-*-symbolic`; `system-shutdown-symbolic` without a battery | battery, power mode, when inactive, lid | — | always |
| | Privacy & Security | `changes-prevent-symbolic` | app permissions, screen lock, clipboard | Camera, Screenshots, Screen Sharing | always |
| | Apps | `view-app-grid-symbolic` | default apps, open at login | — | always |
| | Language & Time | new: `language-time` (a globe) | language, formats, time, time zone | — | localed or timedated answers |
| | About | `computer-symbolic` | name, this computer, summaries of Storage and Software Update | Storage, Software Update | always |

The new symbols (`appearance`, `accessibility`, `trackpad`, `language-time`) belong to the 16 pt
set of `data/icons/Bande/symbolic` (roadmap D3). Until they exist, slice 1 uses
`weather-clear-night`, `avatar-default`, `input-mouse` and `preferences-system`.

**Changes from today.**

- **General is gone.**
  - About becomes a top-level pane and holds Storage and Software Update as subpages.
  - Date & Time and Language & Region merge into **Language & Time**.
  - Login Items move to **Apps**.
- **Wallpaper** merges into **Appearance**. GNOME's Appearance did the same, and the tint
  switch already depends on the wallpaper.
- **Lock Screen** becomes **Power ▸ When inactive**:
  - the three timers cascade (dim before lock, lock no later than sleep), so they stay together;
  - "Sleep after" never was a lock setting;
  - Power exists on every machine, while a Lock Screen pane would hold nothing else today.
- **Battery** is renamed **Power** and is always shown. Its battery section appears only with a
  battery.
- **New panes:** **Privacy & Security** (slice 3) and **Apps** (slice 1, with default apps in
  slice 3).

**Rules.**

- A pane belongs to one group.
- A setting has one home. Another page may link to it with a disclosure row, but never repeats
  the control.
- A pane is hidden only when its hardware is absent (§4, States).
- Keywords keep the old names: "general", "lock screen", "wallpaper", "battery", "date",
  "region", "login items". Search and the launcher find the new home from the old word.

## 4. The page pattern

Every pane and subpage is built from the same parts, in this order.

1. **Back** (subpages only). A quiet "‹ Pane" button at the top left. Alt+Left and the mouse's
   back button do the same. Focus returns to the row that opened the subpage. This
   exists today.
2. **Header.**
   - A 40 pt tile (`well`, radius 10) holds the pane's symbol at 24 pt in `ink`.
   - Next to it: the title (Title role, Manrope 22/28) and one summary line below it (Callout
     12/16, `ink2`).
   - The summary is live where there is a state to show, and a static sentence otherwise (table
     below).
   - A pane that is one big on/off (Wi-Fi, Bluetooth, the Night Light subpage) carries its
     **master switch** at the trailing edge of the header.
   - About uses the Display role (Manrope 28/34) for the computer's name, the "Settings hero"
     of Datum §7.
3. **Transient cards.** Some requests need an answer now: a pairing request, Keep this display
   setting?, an inline confirmation. They sit under the header with an accent tint, and take
   focus. This exists today.
4. **Sections.**
   - A section is a tracked-capitals label, its rows on the sheet with no card outline (Datum
     §11), and an optional footer caption.
   - The order within a page:
     - what most people came for comes first;
     - a list of objects comes before their options when the list is the point (networks,
       devices, sound outputs);
     - subpages and destructive actions come last.
   - There is no "Advanced" section. Depth goes into a subpage named for what it holds.

**Header summaries.**

| Pane | Live summary (static sentence when there is no state) |
| :--- | :--- |
| Wi-Fi | Connected to "Home" · Off |
| Bluetooth | On · 2 devices connected · Off |
| Network | Ethernet connected · VPN "Work" connected · No cable |
| Appearance | Light, dark or automatic, accent colour, wallpaper and text size |
| Bar | Where the bar sits and how it behaves |
| Notifications | Do Not Disturb is on · Banners and Notification Center |
| Accessibility | Make the screen, sound, keyboard and pointer easier to use |
| Displays | Built-in display · 2 displays, extended |
| Sound | MacBook Pro Speakers · 60 % |
| Keyboard | English (US) · 2 input sources |
| Mouse | Buttons, scrolling and tracking speed |
| Trackpad | Tapping, clicking, scrolling, tracking speed and gestures |
| Power | 82 % · 4 h 10 min left · On power adapter |
| Privacy & Security | Which apps may use the camera, take screenshots or share the screen |
| Apps | Which apps open your files and links, and which open at login |
| Language & Time | English (US) · Stockholm · 14:32 |
| About | MacBook Air (M2, 2022) |

**Rows.** Rows are 44 pt high, or 56 pt with a description. Each row has one control. A click
anywhere on the row operates the control: it flips a switch, opens a pop-up, focuses a field.

| Row | Looks like | Use for | Registry `Kind` | Widget |
| :--- | :--- | :--- | :--- | :--- |
| Switch | label (+ description) … switch | on and off | `Switch` | `Toggle` |
| Choice | label … segmented control for 2–4 short options, pop-up for 5 or more or for long labels | one of a set | `Choices` (segmented), `Menu` (pop-up); the registry picks by count and length | `SegmentedControl`, `PopUpButton` |
| Picture choice | a grid of thumbnails or swatches under the label | wallpaper, accent | `Pictures`, `Swatches` | `Thumbnails`, `Swatches` |
| Slider | label … slider with end captions (Slow, Fast) or end symbols (volume) | magnitudes, stepped or continuous | `Stops`, `Range`, `Level` | `Slider` in `Captioned` |
| Disclosure | label … current value in `ink2` › | opens a subpage | new: `Page(PaneId)` | `Row` with `on_activate` (today's General rows) |
| Action | label (+ description) … button | do something now | — | `Button`; primary only for the page's main action, danger only for the destructive verb |
| Field | label … text field | names and addresses | `Text` | `TextField`; Return or leaving the field commits, Escape reverts (as today) |
| Info | label … value | read-only facts | — | `info_row` |
| Item | an object's name with a status line … a trailing button or check | lists of networks, devices, outputs, sources, apps | `List(..)` | `Row` with `detail` |

- A button title ends in "…" when it asks for more: Add Picture…, Change…, Empty Trash….
- **Destructive actions confirm inline**, as Empty Trash does today. The row turns into the
  question, with Cancel and the red verb; Escape cancels. Forget a network and Forget a device
  follow the same pattern. There are no modal "Are you sure?" dialogs.
- **Editable lists:** add with a row at the end of the list ("Add Input Source…"); remove with a
  button at the row's end; reorder with Move Up and Move Down, and with Alt+Up and Alt+Down.
- **Pickers stay inline**, as for time zones, input sources and login apps today. Change… or
  Add… turns the row into a search field with up to eight results below it. Return takes the
  first result, Escape cancels.
- **Object pages.** An object with several settings of its own (an app's notifications, an app's
  permissions) opens a subpage of its own. It does not expand in place.
- **Shortcuts** are drawn with the `Shortcut` widget, as words ("Alt+Super+Space"), never ⌘ or ⌥.
  This fixes `switch-sources-hint`.

**Where explanations go.** There are three places, one sentence each at most:

- **The header summary** says what the page is for, or its live state.
- **A row description** goes under the label, and only when the label cannot say what happens.
  - Sticky keys: "Press Shift, Control, Alt or Super one at a time for a shortcut".
  - Slow keys: "Accept a key only after it is held down briefly".
  - Bounce keys: "Ignore quick repeated presses of the same key".
  - A description never restates the label.
- **A section footer** gives context or a consequence for the whole section: "Changes take
  effect the next time you log in", or the arrangement hint.
- **Never:**
  - a tooltip as the only explanation;
  - a "Learn more" link to the web;
  - a modal info dialog;
  - an "Advanced" heading.

**Errors.**

- An error shows at the row that failed: a danger-coloured Callout line under it.
- The control springs back to the real value.
- The text says in plain words what happened and what to do. Known D-Bus errors map to
  sentences:
  - `InteractiveAuthorizationRequired`, `NotAuthorized`, `AccessDenied`: "Only an administrator
    can change this."
  - `ServiceUnknown`, `NoReply`, `TimedOut`: "<Service> isn't responding. Try again in a
    moment."
  - Anything else: "Couldn't change <setting>." The raw text goes to the journal only.
- The page-bottom "Not changed: …" line goes away.

**States.**

- **Off.**
  - When a pane's master switch is off, the body gives way to a calm empty state: the symbol at
    48 pt in `ink3`, a Heading ("Wi-Fi is off") and one Callout line ("Turn on Wi-Fi to see
    networks nearby.").
  - Turning the switch on cross-fades the content in. Under Reduce Motion it appears at once.
- **Unavailable.**
  - Hardware absent (no Wi-Fi device, no Bluetooth adapter, no pointing device): the pane is
    hidden from the sidebar and from search, as today.
  - A deep link to a hidden pane opens it in the unavailable state, instead of doing nothing.
  - Service absent or failing: the pane is shown in the unavailable state, with the symbol,
    "Bluetooth isn't available" and "The Bluetooth service isn't responding." There is no button
    that cannot work.
- **Missing feature.** A row or section that needs hardware the machine lacks is simply
  omitted. This is not an error. Examples: the trackpad, a battery, a lid, power profiles, VRR, a
  second display.
- **Loading.**
  - Rows appear at once with muted text ("Looking for networks…", "Calculating…").
  - Spinners appear only for work the user started: Check Now, Download Updates.
- **Empty lists** show one muted row in the list saying what would be there: "Apps appear here
  after they send a notification".
- **Needs an administrator** (slice 2). A system-wide row checks its polkit action in advance:
  - When the user is authorised, or may authenticate, the row works.
  - Otherwise the control is disabled, with the description "Only an administrator can change
    this."

**Search.**

- **What stays from today:**
  - typing anywhere starts a search;
  - the sidebar narrows to the matching panes, best first;
  - the page shows the first hit, with matched rows tinted and matched letters bold;
  - Return focuses the best setting;
  - Up and Down step through the matching panes from the search field;
  - Escape clears.
- **What changes:**
  - While searching, the sidebar drops its group labels and shows one ranked list.
  - Subpages show their path: "About › Storage", "Displays › Night Light".
  - A disclosure row whose subpage matches is tinted on its parent page.
  - Ctrl+F focuses the search field.
  - With no result, the page says: No settings match "xyz".
- **Vocabulary.** Every setting's keywords carry the other platforms' words, so people find
  things with the words they bring:

  | Setting | Also found by |
  | :--- | :--- |
  | Night Light | night shift, blue light, warm, redshift, colour temperature |
  | Power | energy, battery, sleep, suspend, low power mode, power saver |
  | When inactive | screen saver, screen timeout, blank, auto lock |
  | Open at login | startup apps, autostart, login items |
  | Default apps | default browser, mail, open with, file associations |
  | Privacy & Security | permissions, screen recording, camera access |
  | Language & Time | locale, region, formats, timezone, clock, NTP |
  | About | hostname, computer name, system information, version, general |

**Keyboard.**

- Tab moves through rows and Shift+Tab back.
- In the sidebar, Up and Down move between panes and Return opens one.
- Alt+Left goes back.
- Escape closes, in this order: a confirmation, then a picker, then the search.
- The View menu lists the panes by group, with separators between groups.

**Words.**

- Pane and subpage names are Title Case nouns of one or two words: "Privacy & Security".
- Section labels are one or two words.
- Row labels are in sentence case and say what is set, in positive form: "Tap to click",
  "Automatically hide the bar". There is no "Enable", no "settings" and no "Configure".
- Established names stay: Do Not Disturb, Night Light, Natural scrolling.
- Values follow the locale.
- Choices run from small to large, or short to long. "Never" comes last.

## 5. Every option today and its new home

| Today | New home | Change |
| :--- | :--- | :--- |
| Wi-Fi ▸ Wi-Fi switch | Wi-Fi ▸ header master switch | moves into the header; off state replaces the body |
| Wi-Fi ▸ Networks (join, password, status) | Wi-Fi ▸ Networks | unchanged; Forget added (slice 4) |
| Bluetooth ▸ Bluetooth switch | Bluetooth ▸ header master switch | moves into the header |
| Bluetooth ▸ pairing request | Bluetooth ▸ transient card | unchanged |
| Bluetooth ▸ My devices; Nearby devices | Bluetooth ▸ My devices; Nearby devices | battery level and Forget added (slice 4) |
| Network ▸ Ethernet (status, Configure IPv4, IP address, Router, DNS) | Network ▸ Ethernet | unchanged |
| Network ▸ VPN (Connect, Disconnect, Import VPN…) | Network ▸ VPN | unchanged |
| Notifications ▸ Do Not Disturb | Notifications ▸ first section | gains the description "Banners wait; urgent alerts still show" |
| Notifications ▸ Application notifications (inline expander) | Notifications ▸ Apps ▸ one subpage per app (Allow, Banner style, Show in Notification Center) | expander becomes a subpage (slice 3) |
| Sound ▸ Play sound through; Output volume; Mute | Sound ▸ Output (first section) | output moves first; device list, then volume and mute |
| Sound ▸ Record sound from; Input volume; Mute microphone; Input level | Sound ▸ Input (second) | same order rule |
| Sound ▸ Alert sound; Alert volume; interface sound effects; volume feedback | Sound ▸ Sound effects (third) | moves below output and input |
| Sound ▸ Apps playing sound | Sound ▸ Apps (last) | unchanged |
| Appearance ▸ Appearance; Accent colour | Appearance ▸ first sections | unchanged |
| Appearance ▸ Tint window frames with the wallpaper | Appearance ▸ Wallpaper | moves next to the wallpaper it tints |
| Wallpaper ▸ pictures, Add Picture…; Picture size | Appearance ▸ Wallpaper | pane removed; `shell/desktop`'s "Change Wallpaper…" opens `appearance` |
| Accessibility ▸ Vision (Screen reader, Zoom, Increase contrast, Reduce transparency, Reduce motion, Pointer size) | Accessibility ▸ Vision | adds a "Text size ›" link row to Appearance (slice 7) |
| Accessibility ▸ Hearing (Flash the screen) | Accessibility ▸ Hearing | unchanged |
| Accessibility ▸ Motor (Sticky, Slow, Bounce keys) | Accessibility ▸ Motor | adds row descriptions |
| Bar ▸ Position, Size, Automatically hide, Menu bar | Bar | unchanged |
| Displays ▸ Keep card; Arrangement; per display Use as, Resolution, Size of text and controls, Refresh rate, Rotation, VRR | Displays | 5 or more choices become pop-ups (resolutions, refresh rates, scale); Brightness (slice 4) and Night Light › (slice 5) added above the per-display sections |
| Lock Screen ▸ Dim, Turn off the display and lock, Sleep after | Power ▸ When inactive | pane removed; segmented controls become pop-ups; Privacy & Security ▸ Screen lock links here |
| Battery ▸ level and history | Power ▸ Battery | shown only with a battery |
| Battery ▸ Power mode; Use Power Saver on battery | Power ▸ Power mode | unchanged |
| Battery ▸ When the lid is closed | Power ▸ Lid | shown only with a lid |
| Keyboard ▸ Input sources | Keyboard ▸ Input sources | hint shows the keys with the `Shortcut` widget |
| Keyboard ▸ Repeat rate, Delay until repeat; Caps Lock key | Keyboard ▸ Key repeat; Modifier keys | unchanged |
| Keyboard ▸ Keyboard shortcuts (inline, long) | Keyboard ▸ Keyboard Shortcuts › subpage | the long list leaves the main page |
| Trackpad & Mouse ▸ Mouse rows | Mouse | its own pane, shown only with a mouse; primary button and acceleration added (slice 6) |
| Trackpad & Mouse ▸ Trackpad rows | Trackpad | its own pane, shown only with a trackpad; acceleration, dragging and scroll method added (slice 6) |
| General (hub) | — | removed |
| General ▸ About (Name, Model, Chip, Memory, Storage, Bande, System, Serial) | About | top-level; the name is the hero; Storage and Software Update become disclosure rows with their summaries |
| General ▸ Storage (usage, Trash, Largest files) | About ▸ Storage | unchanged |
| General ▸ Software Update | About ▸ Software Update, and the sidebar's attention row | unchanged on the page |
| General ▸ Date & Time ▸ Set time automatically, date and time, Time zone | Language & Time ▸ Time | merged pane |
| General ▸ Date & Time ▸ 24-hour time | Language & Time ▸ Formats | a format, next to the region preview |
| General ▸ Language & Region ▸ Language; Region and preview | Language & Time ▸ Language; Formats | merged pane |
| General ▸ Login Items | Apps ▸ Open at login | moves |
| Page-bottom "Not changed: …" | the failing row | plain sentences (slice 2) |

## 6. Gaps against the references

Who has each option: **M** macOS 15/26, **W** Windows 11, **G** GNOME 48, **K** Plasma 6. A
letter counts when the option is in that desktop's settings app or one click from it.

- **Now** means a backend exists or is small, so a slice of §7 builds it.
- **Later** means it needs real platform work first.
- **Never** means it fails "settings only where people genuinely differ", or it is out of scope.

Several of the owner's examples already exist and need only their new home: key repeat rate and
delay, per-display scale and arrangement, sound devices and the alert sound, power modes, and
notifications per app.

**Connections**

| Option | Who | Bande | Call | Backend and note |
| :--- | :--- | :--- | :--- | :--- |
| Forget a Wi-Fi network | M W G K | missing | now (slice 4) | NetworkManager `Settings.Connection.Delete` in `libs/network` |
| Auto-join per network | M W G K | missing | later | connection `autoconnect`; S–M |
| Hidden networks, WPA-Enterprise (eduroam) | M W G K | missing | later | M in `libs/network` (maturity gaps §11) |
| Wi-Fi details (IP, router, DNS) | M W G K | Ethernet only | later | reuse the Ethernet rows |
| Proxy | M W G K | missing | later | GLib, Firefox and the portal read GSettings; M |
| Airplane mode | W G K | missing | later | rfkill; belongs to Control Center first |
| Forget a Bluetooth device, device battery | M W G K | missing | now (slice 4) | BlueZ `Adapter1.RemoveDevice`, `Battery1.Percentage` in `libs/bluetooth` |
| Mobile hotspot | W G K | — | never | rare on a laptop desktop |

**Desktop**

| Option | Who | Bande | Call | Backend and note |
| :--- | :--- | :--- | :--- | :--- |
| Light, dark, automatic; accent | M W G K | have | — | — |
| Wallpaper with fit | M W G K | own pane | now (slice 1) | merged into Appearance |
| Text size (larger text) | M W G K | config and portal only | now (slice 7) | toolkit follows `text-size` (maturity gaps #25); the control lands with it |
| Fonts, icon and cursor themes | W K | — | never | Datum fixes them; text size covers the need |
| Wallpaper slideshow | M W K | light and dark pairs | later | small, but a nice-to-have |
| Hot corners | M G K | missing | later | opt-in only (roadmap); S in the compositor |
| Reopen windows at login | M K | missing | later | maturity gaps #22 makes it default behaviour; no switch |
| Do Not Disturb schedule | M W | missing | later | `services/notify` timer; S |
| Per-app notification sound and badge | M W | missing | later | needs the bar badge model |
| Notifications on the lock screen, previews | M W G | missing | later | `shell/lock`; M |
| Colour filters | M W K | missing | later | compositor shader; M |
| On-screen keyboard, mouse keys | M W G K | missing | later | L |
| Locate the pointer (shake or Ctrl) | M G K | missing | later | S in the compositor |
| Screen reader, zoom, contrast, transparency, motion, pointer size, flash, sticky, slow, bounce | M W G K | have | — | descriptions added |
| Captions | M W | — | never | no caption service |

**Devices**

| Option | Who | Bande | Call | Backend and note |
| :--- | :--- | :--- | :--- | :--- |
| Built-in brightness | M W K | Control Center only | now (slice 4) | logind `Session.SetBrightness`, as Control Center does; the backlight helper moves to `libs/status` |
| Night Light with schedule and warmth | M W G K | missing | now (slice 5) | compositor gamma ramp (maturity gaps #23); the sunset schedule reuses `services/portal/src/daylight.rs` |
| Automatic brightness | M W G | missing | later | ambient light sensor; `services/power`; M |
| Resolution, refresh, rotation, scale, VRR, arrangement, mirroring | M W G K | have | — | pop-ups past four choices |
| HDR and colour profiles | M W G K | missing | later | L (maturity gaps §10) |
| Output and input device, volume, mute, input level | M W G K | have | — | reordered |
| Alert sound and volume, interface sounds | M W G K | have | — | — |
| Per-app volume | W G K | have | — | — |
| Balance | M W G K | missing | later | channel volumes in `libs/audio`; S |
| Headset profile (high quality or microphone) | G K | missing | later | maturity gaps §11 |
| Input sources, key repeat, Caps Lock | M W G K | have | — | — |
| Shortcut list | M W G K | read only | now (slice 1) | becomes a subpage |
| Custom shortcuts and rebinding | M G K | missing | later | compositor and config; M (maturity gaps §4) |
| F1–F12 as standard keys (Apple `fnmode`) | M | missing | later | `hid_apple` parameter needs root; S–M |
| Keyboard light off when idle | M | missing | later | `services/power`; S |
| Input methods (fcitx5) | M W G K | missing | later | maturity gaps §5 |
| Tap, secondary click, natural scrolling, speed, ignore while typing | M W G K | have | — | — |
| Primary button (left-handed) | M W G K | have | slice 6 | libinput `left_handed` in the compositor's `pointing.rs`; S |
| Pointer acceleration off (flat) | M W G K | have | slice 6 | libinput accel profile; S |
| Scroll speed | M W K | missing | later | the compositor must scale it; S–M |
| Three-finger drag | M | missing | later | roadmap |
| Gesture switches (workspaces, overview, show desktop, launcher) | M W K | have | slice 6 | one fixed set, each can be turned off and then reaches the app |

**System**

| Option | Who | Bande | Call | Backend and note |
| :--- | :--- | :--- | :--- | :--- |
| Battery level, power modes, Power Saver on battery, lid | M W G K | have | — | moved to Power |
| Dim, display off and lock, sleep timers | M W G K | under Lock Screen | now (slice 1) | moved to Power ▸ When inactive |
| Separate timers on battery and on the adapter | M W G K | one set | later | `services/power`, config |
| Charge limit | M G K | missing | later | Asahi `charge_control_end_threshold` needs root |
| Power button action | W G K | — | never | the power key locks while sleep is off and suspends once it works (logind drop-in); a five-second hold powers off; no choice needed |
| Battery percentage in the bar | M W G K | battery menu | never in Settings | roadmap decision |
| Camera permission per app | M W G K | missing | now (slice 3) | permission store table `devices`, id `camera` |
| Screenshot permission per app | M W | missing | now (slice 3) | table `screenshot`, id `screenshot` |
| Remembered screen-sharing sources | — | missing | now (slice 3) | table `screencast` (restore tokens); our ScreenCast backend already restores sources |
| Clipboard history: clear | W | always on | now (slice 3) | `org.bande.Clipboard.Clear` exists |
| Clipboard history: off | W | always on | later | config key and `services/clipboard`; S |
| Microphone permission | M W G | missing | later | no portal enforces it; needs a WirePlumber policy |
| Location services | M W G | missing | later | no geoclue agent; Night Light uses the time zone |
| Background apps | M W G | missing | later | needs our Background portal backend |
| File history, automatic trash purge | W G K | missing | later | S each |
| Lock delay after the display turns off | M W G | locks at once | later | `shell/lock`, power |
| Users: name, picture, password, other accounts | M W G K | missing | later | AccountsService and the polkit agent; M |
| Default apps (browser, mail, music, video, pictures, PDF, text) | M W G K | missing | now (slice 3) | `libs/xdg` `apps_for` and `set_default_app`, as Files uses them |
| Open at login | M W G K | under General | now (slice 1) | moved to Apps |
| Installed apps and uninstall | W | missing | later | PackageKit `RemovePackages` |
| Language, formats with preview, 24-hour time | M W G K | have | — | merged into Language & Time |
| Automatic time, time zone, manual time | M W G K | have | — | merged |
| Automatic time zone | M W G | missing | later | geoclue |
| Install a language | M W G K | have | — | any supported locale; localed runs locale-gen, "Setting up…" while it works |
| Preferred language order | M W G K | one language | later | — |
| Separate overrides for number, date and measurement formats | M W K | the region decides | never | one region choice is enough |
| Administrators change time, language and name without a prompt | — | fails without an agent | now (slice 2) | polkit rules (maturity gaps #2) |
| Computer name, About, Storage, Software Update | M W G K | have | — | moved under About |
| Automatic security updates, offline updates at restart | M W G K | partly: offline updates at restart are in; no automatic security updates | later | `unattended-upgrades` (maturity gaps §12) |
| Printers | M W G K | missing | later | roadmap Later |
| Sharing: files, remote desktop | M W G K | missing | later | RemoteDesktop portal |
| Secure Shell switch, firewall | M G K | — | never | administrator tools, not desktop settings |
| Online accounts, backup, reset, startup disk, Screen Time, Focus modes, Game Mode | M W G K | — | never | out of scope (roadmap "Never" and "Later") |

**A note on privacy lists.** xdg-desktop-portal keys permissions by app id.

- Flatpak apps always have one.
- Apps outside a sandbox get an id only through the host app registry, or through a systemd scope
  named `app-<launcher>-<id>-<random>.scope` (maturity gaps #17). Until #17 lands, those apps'
  entries carry an empty id.
- Privacy & Security shows them as one "Other apps" row, with Reset.

## 7. Build slices

Each slice takes one agent one to two hours, and leaves Settings better and working.

- Slices 1–4 are Settings work and run in this order.
- Slices 5–7 need a platform change first. Their Settings rows are small, and they can run in
  parallel with 2–4 in agents that own the compositor or the toolkit.

Every slice follows the night brief:

- a test that fails without the change;
- light and dark goldens, or review screenshots;
- `cargo fmt --check`, clippy and tests on the hermetic bus for every crate touched and every
  crate that depends on it.

### Slice 1: structure and page pattern

Owns `libs/settings-panes`, `apps/settings` and one constant in `shell/desktop`.

1. **Registry.**
   - Add `Group` (Connections, Desktop, Devices, System) with its label.
   - `Pane` gains `group`, `symbol`, `summary` and `master: Option<SettingId>`.
   - Add `Kind::Page(PaneId)` for disclosure rows. A disclosure row shows its subpage's summary,
     and search sees it.
2. **Pane moves.** Move the panes as in §3 and §5:
   - add `Power`, `Apps`, `LanguageTime` and `KeyboardShortcuts`;
   - make `About` top level, with `Storage` and `SoftwareUpdate` as children;
   - remove `General`, `Wallpaper`, `LockScreen`, `Battery`, `DateTime`, `LanguageRegion` and
     `LoginItems`;
   - keep the old names as keywords.
3. **Sidebar.**
   - Group labels and 16 pt symbols, loaded through `libs/xdg` `find_icon` and tinted as Files'
     `icons.rs` does.
   - A scrolling column that keeps the selection in view.
   - While searching: a flat ranked list, with "Parent › Subpage" for subpages.
4. **Header.**
   - The tile, the title and the live summary.
   - The Wi-Fi and Bluetooth switches move into the header, with the off state.
5. **Other changes.**
   - The choice rule: 5 or more options become a pop-up. This covers the idle timers, the scale,
     resolutions and refresh rates.
   - Sound reorders to Output, Input, Sound effects, Apps.
   - Keyboard Shortcuts become a subpage.
   - The attention row for updates.
   - Ctrl+F focuses search.
   - The View menu groups the panes.
   - The default window becomes 900×720.
   - `shell/desktop` opens `appearance` instead of `wallpaper`.
6. **Proof.**
   - Registry tests: every top-level pane has a group; every child's parent is a top-level pane;
     each old name finds its new home ("lock screen" → Power, "wallpaper" → Appearance, "login
     items" → Apps, "date" → Language & Time).
   - View tests:
     - the off state replaces the body and the switch turns it back;
     - a disclosure row opens its subpage;
     - back returns focus to that row;
     - the attention row appears only with pending updates.
   - Goldens of the sidebar with Appearance, Wi-Fi off, Power and About, light and dark.
7. **Seam if it runs long:** 1a covers the registry, the moves, the sidebar and the deep links;
   1b covers the header, the states, the attention row and the choice rule.

### Slice 2: administrator rights and plain errors

Owns `apps/settings` (including `data/` and `debian/`) and `ci/vm`.

1. **Rules file.** Ship `data/50-bande-settings.rules`, installed to
   `/usr/share/polkit-1/rules.d/`. It returns `YES` to local, active members of `sudo` for:
   - `org.freedesktop.timedate1.set-timezone`, `set-ntp` and `set-time`;
   - `org.freedesktop.locale1.set-locale`;
   - `org.freedesktop.hostname1.set-static-hostname` and `set-machine-info`.

   Every one of these is `auth_admin_keep` on Debian 13. Software Update needs no rule: it
   downloads with `ONLY_DOWNLOAD` (no authorization) and triggers an offline update, which is
   `allow_active=yes`.
   NetworkManager system connections are already allowed for `sudo` and `netdev`.
2. **Rows that check in advance.**
   - Before drawing the system-wide rows (time zone, automatic time, manual time, language,
     region, computer name), check each action with polkit's
     `CheckAuthorization`, without interaction.
   - A row the user may not change is disabled, with the description "Only an administrator
     can change this."
3. **Plain errors.**
   - Map D-Bus errors to the sentences of §4, shown at the failing row.
   - Remove the page-bottom message.
4. **Proof.**
   - A guest test: `pkcheck` authorises a `sudo` member for each action without an agent, and
     refuses a non-member. It runs in `ci/vm`, or, if that image lacks polkitd, in a Debian 13
     container with polkitd.
   - A view test: a refused change shows the sentence under its row and reverts the control.

### Slice 3: Apps, Privacy & Security, notification app pages

Owns `apps/settings` and `libs/settings-panes`. It reuses `libs/xdg`.

1. **Apps ▸ Default apps.** One pop-up row per role: Web browser, Mail, Music, Video, Pictures,
   PDF, Text.
   - Each role has a main type and a family of related types:
     - Web browser: `x-scheme-handler/https`, with `http` and `text/html`.
     - Mail: `x-scheme-handler/mailto`.
     - Music, Video and Pictures: `audio/mpeg`, `video/mp4` and `image/jpeg`, each with the
       rest of its family.
     - PDF: `application/pdf`.
     - Text: `text/plain`.
   - The pop-up lists the apps that handle the main type, with their icons, from
     `MimeApps::apps_for`.
   - Choosing an app writes each type of the role that the app handles, with
     `set_default_app`, to the user's `mimeapps.list`.
   - A role with no capable app shows the muted row "No app installed".
2. **Privacy & Security.**
   - **App permissions:** Camera ›, Screenshots ›, Screen Sharing ›. Each opens a subpage that
     lists the apps in the permission store (`org.freedesktop.impl.portal.PermissionStore`):
     - Camera: table `devices`, id `camera`.
     - Screenshots: table `screenshot`, id `screenshot`.
     - Screen sharing: table `screencast`.
   - On those subpages:
     - Camera and Screenshots get one switch per app (`SetPermission` "yes" or "no").
     - Screen Sharing gets Forget per app (`Delete`, inline confirmation).
     - The `Changed` signal keeps each list live.
     - Entries without an app id show as "Other apps".
   - **Screen lock:** a disclosure row "Lock when the display turns off: after 10 min ›" that
     opens Power.
   - **Clipboard:** "Clear clipboard history…", which confirms inline and then calls
     `org.bande.Clipboard.Clear`.
3. **Notifications.** Each app's row opens a subpage with Allow, Banner style and Show in
   Notification Center. The inline expander and `expanded_app` are deleted.
4. **Proof.**
   - Default apps: a test on a temporary `XDG_CONFIG_HOME`, where choosing a browser writes the
     http, https and HTML types, and the pop-up shows the current default.
   - Privacy: tests against a permission store fake on the test bus (add one to `testing/test-bus`
     if none exists). A switch writes "no"; Forget deletes the token; a `Changed` signal updates
     the list.
   - Goldens of Apps and Privacy & Security.

### Slice 4: connections and brightness

Owns `libs/network`, `libs/bluetooth`, `libs/status`, `apps/settings` and, for the moved helper,
`shell/controlcenter`.

1. **Wi-Fi.** "Forget This Network…" on known networks, through the row's context menu and the
   Delete key. It confirms inline, then deletes the NetworkManager connection.
2. **Bluetooth.**
   - "Forget This Device…" on paired devices, with the same pattern, calling
     `Adapter1.RemoveDevice`.
   - The battery level from `Battery1.Percentage` shows in the device's status line ("Connected ·
     80 %").
3. **Displays ▸ Brightness.** A slider for the built-in panel.
   - Move Control Center's backlight read and set code to `libs/status`, which is now its second
     consumer.
   - The slider sets through logind `Session.SetBrightness`.
   - It follows external changes from the brightness keys.
4. **Proof.**
   - Tests on the NetworkManager and BlueZ fakes of `testing/test-bus`.
   - A `libs/status` test with a fake sysfs backlight.
   - Goldens of Bluetooth and Displays.

### Slice 5: Night Light (platform first)

Owns `platform/compositor`, `services/config`, `apps/settings` and `libs/settings-panes`.
`daylight.rs` moves from `services/portal` to `libs/datetime` because it gains a second consumer.

1. **Config.** A top-level `night-light` node with:
   - `on`;
   - `schedule`, either `"sunset"` or `"custom"`, with `from` and `to`;
   - `warmth`.
2. **Compositor.**
   - Apply a warm gamma ramp (CRTC `GAMMA_LUT`; the colour matrix when the driver offers only
     that) on every output.
   - Fade over a minute at the scheduled times.
   - Apply at once while the warmth slider moves, as a preview.
   - Check what Asahi's display driver exposes on the M-series VM and hardware first.
3. **Settings.** Displays ▸ Night Light › opens a subpage:
   - the master switch in the header;
   - Schedule (Sunset to sunrise, Custom), with the times shown when custom;
   - Warmth (a slider from Less warm to More warm);
   - the summary "On until 07:12" or "Off until 18:40".
4. **Proof.**
   - Unit tests of the ramp and the schedule.
   - A `ci/vm` vkms readback of the gamma property.
   - A golden of the subpage.

### Slice 6: primary button and pointer acceleration

Owns `services/config`, `platform/compositor` and `apps/settings`.

1. **Config.** `mouse { primary-button "left" | "right"; acceleration #true }` and
   `trackpad { acceleration #true }`.
2. **Compositor.** `pointing.rs` sets libinput's left-handed option and the flat or adaptive
   accel profile.
3. **Settings.** Mouse gains Primary button (Left, Right) and "Pointer acceleration", with the
   description "Faster movements move the pointer farther". Trackpad groups its rows into Click
   (Tap to click; Dragging, shown while tapping is on; Secondary click), Scroll (Natural
   scrolling; Scroll with two fingers or along the edge) and Pointer (Tracking speed, Pointer
   acceleration, Ignore the trackpad while typing), then Gestures: Swipe between workspaces,
   Overview, Show desktop and Launcher, each a switch. `trackpad { workspace-swipe;
   overview-swipe; desktop-spread; launcher-pinch }` hold them; a gesture that is off goes to
   the app under the pointer.
4. **Proof.**
   - A config round-trip test.
   - A headless compositor test where the device reports the configured options.

### Slice 7: text size (toolkit first)

Owns `toolkit/graphics`, `toolkit/widgets` and `apps/settings`.

1. **Toolkit.** The toolkit scales its type roles, and the control heights derived from them, by
   the portal's `text-size` (100–200), live (maturity gaps #25).
2. **Settings.**
   - Appearance ▸ Text size: a stepped slider from Default to Largest with 100, 125, 150, 175
     and 200 %.
   - Accessibility ▸ Vision links to it.
3. **Proof.** Control goldens at 100 and 150 %, and a test that a field at 200 % still fits its
   text.

## 8. Decisions taken in this spec, for the owner to confirm

1. Four labelled groups and 17 panes. There is no General hub: About is top level, and Storage
   and Software Update live under it.
2. Date & Time and Language & Region merge into **Language & Time**. Login Items join default
   apps in **Apps**.
3. Wallpaper merges into **Appearance**. Lock Screen's timers move to **Power ▸ When inactive**,
   and Battery is renamed Power.
4. Text size lives in Appearance, as roadmap D6 says, and Accessibility links to it. Increase
   Contrast stays in Accessibility, where all four references keep it. This departs from the
   Appearance list in Datum §10.
5. No Apply or Defaults buttons, no expanders, no "Advanced" sections. Depth goes into named
   subpages.
6. Monochrome sidebar symbols, with one accent for the selection.
7. An attention row at the top of the sidebar for pending updates. It is the only place that
   badges.
8. Night Light is one setting for all displays, as on every reference. It is not set per
   display.
9. No power button choice: the key suspends (the logind drop-in of the sleep and wake work), and
   nobody needs another answer.

Left for later, by the owner: a "You" card at the top of the sidebar for Users (name, picture,
password), once AccountsService work is planned. Also later: whether a right-click on a row
should offer "Restore Default", as Plasma's highlighting of changed settings suggests.

## 9. Built on 2026-10-31 (branch `night/settings-structure`)

**Slice 1.** The registry carries `Group`, a symbol per pane, a static summary, the master
switch and `Kind::Page`. The sidebar shows the four groups with symbols and the attention row;
while searching it is one ranked list with "About › Storage" paths. Every page opens with the
header tile, title and live summary; Wi-Fi and Bluetooth carry their switch there and show the
off state; a pane without its device or service shows the unavailable state instead of jumping
away; a search without hits says so. General, Wallpaper, Lock Screen, Battery, Date & Time,
Language & Region and Login Items are gone; Power, Apps, Language & Time and Keyboard Shortcuts
are new. Five or more choices are pop-ups. Sound runs Output, Input, Sound effects, Apps. The
input source hint shows the real keys. Super+F focused search (Ctrl+F since 2026-11-01), the View menu is grouped, the
window opens at 900×720 and the desktop's "Change Wallpaper…" opens Appearance.

**Slice 2.** `data/50-bande-settings.rules` lets local, active members of `sudo` change the time
zone, automatic and manual time, the language, the computer name and install updates.
Settings asks polkit once at start (without interaction) and shows the rows it may not change as
values with "Only an administrator can change this." Failures show at their row in plain
words; the raw D-Bus text goes to the journal.

**Slice 3, in part.** Apps opens with Default apps: one pop-up per role (Web browser, Mail,
Music, Video, Pictures, PDF, Text), and choosing an app writes every type of the role that the
app opens. Privacy & Security joins System with the Screen lock link ("Automatic lock: After
10 min ›", which opens Power) and Clear History… for the clipboard, confirmed inline. Each
notifying app opens its own page; the inline expander is gone.

**Decisions.**

- A polkit challenge counts as refused: Bande ships no polkit agent yet, so nobody could
  answer it.
- Search ties go to the setting whose pane is also about the word ("lock" finds the lock timer
  in Power before the Caps Lock key).
- The Power pane's sections are Battery, Energy, When inactive and On power adapter; the lid
  row shows only with a battery.
- Settings checks for updates once when it opens, so the attention row can appear.

**Slice 4, in part (branch `night/settings-depth-2`).** A saved Wi-Fi network and a paired
Bluetooth device carry "Forget…", confirmed inline at their row (Escape cancels);
`libs/network` deletes the saved connection and `libs/bluetooth` removes the device from the
adapter. A paired device that reports its battery shows it beside its status ("Connected ·
Battery 80%"). Turning Wi-Fi or Bluetooth off, or a device or service going away, cross-fades
the page into the off state with the fade preset, which stays under Reduce motion.

**Slices 3 to 5 and 7 (branch `day/settings-depth-3`).**

- **Night Light.** Displays ▸ Night Light › opens a subpage with the master switch in the
  header, Schedule (Sunset to sunrise, Custom) with From and To shown only when custom, and a
  Warmth slider from Less warm to More warm. The header and the Displays row say "On until
  07:00" or "Off until 22:00". `NightLight` on `org.bande.Config` is writable as a whole and
  refuses a bad time, schedule or warmth. A typed time is normalised ("6:5" becomes "06:05") or
  refused at its row. Changing the warmth by day previews it at full strength for three seconds
  and then fades back (compositor). The daily window moved to `libs/datetime` as
  `within_daily`, shared by the compositor and Settings.
- **Brightness.** Displays opens with a Brightness slider for the built-in panel, shown only
  when a backlight exists. `libs/status::backlight` now holds the display and keyboard light
  code that Control Center had; both set through logind `SetBrightness`, and Settings follows
  key presses by polling `actual_brightness` for the kernel's change notification (no timer).
- **App permissions.** Privacy & Security opens with Camera, Screenshots and Screen Sharing,
  and Microphone and Location only when the permission store holds entries for them. Each
  row shows how many apps asked; each page lists them with a switch ("yes" or "no" in the
  store; Location writes `NONE` or `EXACT`), or, for Screen Sharing, Forget… with an inline
  confirmation that deletes the app's restore tokens. Entries without an app id read "Other
  apps". The store's `Changed` signal keeps the pages live; a missing table or store reads as
  the empty state ("No app has asked to use the camera yet.").
- **Text size.** The toolkit follows the portal's `text-size` (100–200 %) live: the text system
  scales every paragraph and the editor, and buttons, pop-ups, segmented controls, fields,
  steppers, disclosures and alert buttons grow by the extra line height. App windows follow
  it; the taskbar, panels, lock screen and menus keep their size. Appearance has a Text size
  slider (Default to Largest, five stops); Accessibility ▸ Vision links to it with its value
  and opens Appearance with the slider focused. The Settings sidebar widens with the text.

**Open.**

- Menus (pop-up menus and context menus) do not scale with the text size yet.
- Files, Terminal and Activity Monitor take only the palette from the appearance today; they
  follow the text size once they hand the appearance to their `Ui`.
- Row labels in Settings do not wrap, so a very long label at 200 % can meet its control.
- The guest `pkcheck` test of the rules (no polkitd in the VM image tonight).

## Sources

- Microsoft, Understand and explore Windows Settings (categories, keyboard use):
  <https://support.microsoft.com/en-us/accessibility/windows/understand-and-explore-windows-settings>
- Microsoft, Guidelines for app settings (cards, expanders, control choice, immediate apply):
  <https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings>
- AskWoody, Windows 11 Settings map, 23H2:
  <https://www.askwoody.com/wp-content/uploads/2024/04/NL-2024-05-06-tittel-settings-diagram.html>
- GNOME 46 release notes (the System panel, Apps consolidation): <https://release.gnome.org/46/>
- GNOME Settings panel list (`default_panels`, `default_subpages`):
  <https://gitlab.gnome.org/GNOME/gnome-control-center/-/raw/main/shell/cc-panel-loader.c>
- GNOME HIG, boxed lists and row types:
  <https://developer.gnome.org/hig/patterns/containers/boxed-lists.html>
- GNOME Help, Privacy: <https://help.gnome.org/gnome-help/privacy.html>
- KDE System Settings category files (names and weights), for example
  <https://invent.kde.org/plasma/systemsettings/-/raw/master/categories/settings-hardware-input.desktop>
- KDE, the re-organised System Settings for Plasma 6:
  <https://pointieststick.com/2023/10/06/this-week-in-kde-re-organized-system-settings/>
- Michael Tsai, System Settings in Sequoia (reshuffle, critique of order and search):
  <https://mjtsai.com/blog/2024/07/19/system-settings-in-sequoia>
- Wikipedia, System Settings (Ventura's sidebar redesign and its reception):
  <https://en.wikipedia.org/wiki/System_Settings>
- TechRadar and AppleInsider on Tahoe's Menu Bar pane:
  <https://www.techradar.com/computing/mac-os/what-is-the-macos-menu-bar-heres-how-new-macbook-neo-users-can-get-the-most-out-of-apples-app-control-bar>,
  <https://appleinsider.com/inside/macos-tahoe/tips/inside-control-center-on-macos-tahoe>
- XDG Desktop Portal, PermissionStore:
  <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.PermissionStore.html>.
  The table names were read from Debian 13's `xdg-desktop-portal` 1.20.3 binary on the VM.
- Debian 13 polkit actions and rules, read on the VM: `/usr/share/polkit-1/actions/` and
  `/usr/share/polkit-1/rules.d/`.
- Maturity gaps audit: `docs/experience/maturity-gaps.md` on `origin/night/maturity-audit`,
  items #1, #2, #17, #22, #23 and #25.
