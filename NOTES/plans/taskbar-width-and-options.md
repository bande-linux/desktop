# Taskbar width and the options budget

## Summary

The owner asked to remove the wide taskbar (dragging the inner edge out to 248 pt) and to keep Bande's options at a macOS-like level: what an average user reasonably changes, nothing more. The wide state was a whole second layout of the taskbar, a config key, a D-Bus property, a Settings switch, a drag gesture and a set of tests and goldens. It is removed end to end (milestone 1). An old `config.kdl` that still says `names` loads without a warning. The rest of this plan is an inventory of every user-facing option in Settings and `config.kdl`, each marked keep, remove or merge. Only the owner-decided removal is implemented; the other removals and merges are recommendations for the owner.

## What the owner sees

- Dragging the taskbar's inner edge (the outer 4 pt, with a column-resize pointer) past the middle snapped the bar open to 248 pt: a search field beside the mark, the app's menus listed as rows, the apps as named rows, numbered workspace pills, the network and battery written out, a 22 pt clock with the long date. It looked like a different product and it was another layout to keep working.
- Settings ▸ Taskbar had five rows: Position, Size, Show names, Automatically hide, Menu bar. Show names was the same wide state.

## Findings

1. **The wide state was a second layout, not a width.** `RailView` had `expanded` and `EXPANDED_WIDTH = 248.0` (`crates/shell/bar/src/rail.rs:54` on `origin/main` before this branch), its own layout (`layout_expanded`, `rail.rs:380`) and its own painters (`paint_search`, `paint_menu_row`, `paint_pill`, `paint_summary`, `paint_long_clock`, `paint_wide_header`, `rail.rs:741-998`). The apps strip had a parallel path too: `Rail.expanded`, row icons of 30 pt, name paragraphs per slot, a tinted active row and badges on the right (`crates/shell/bar/src/taskbar.rs:59-63`, `1215-1230`, `1556-1590`, `1900-1915`, `2004-2020`, `2048-2059`). `rail.rs` shrinks from 1307 to 930 lines with it gone.
2. **The drag lived in the combined client.** `CombinedClient::resize` (`crates/shell/bar/src/combined_client.rs:78`) grabbed pointer presses within `EDGE_GRAB = 4.0` of the inner edge, showed `Cursor::ColumnResize` on hover and on release saved `names` when the pointer was past the midpoint of 64 and 248 pt.
3. **The setting ran through every layer.** `bar { names #true }` (`crates/services/config/src/lib.rs:1118`), `Setting::BarNames` (`edit.rs`), the `BarNames` property on `org.bande.Config` (`service.rs`), `BusEvent::Names` and `write_names` (`crates/libs/taskbar-model/src/bus.rs`), `Taskbar::names/set_names` (`taskbar_client.rs`), `SettingId::BarNames` and the "Show names" switch (`crates/libs/settings-panes/src/registry.rs:790`, `bande-settings-panes.ftl:155`) and the Settings D-Bus client.
4. **Tests and goldens.** `the_expanded_bar_matches_its_golden` (`bar-expanded-light.png`), `the_expanded_bar_lists_the_menus_as_rows_that_open_their_menu`, the expanded half of the tray test in `tests/rail.rs`, and the whole `every_expanded_bar_target_answers_every_press` sweep in `tests/live.rs`. The Settings Taskbar golden (`crates/apps/settings/tests/golden/bar-light.png`) loses one row.
5. **Unknown config keys are reported, not fatal.** `Parser::bar` turned an unknown child into `unknown setting \`bar names\`` through `unknown_child` (`lib.rs:1070`), which `config.kdl` reports as a problem. So simply deleting the arm would flag every config written while the switch existed. The arm now matches `names` and does nothing.
6. **The Menu bar row only works on the left.** `MenuBarPlacement::combines` is `self == InBar && edge == Left` (`lib.rs:142`). With Position Right, Bottom or Top, "In the taskbar" silently gives the separate menu bar at the top, yet Settings still offers the choice. That is a half-built combination (see the inventory).
7. **Two options exist only in `config.kdl`.** `reopen-apps` (`lib.rs:920`, read by `crates/shell/shell/src/shell.rs:390`) has no Settings row, and nothing writes it. `show-battery-percentage` is written from the battery menu's "Show Percentage" item (`crates/shell/bar/src/menus.rs:93`), as on macOS, so it is fine.

## The yardstick

macOS System Settings is the reference: a person changes how things look (appearance, accent, wallpaper, text size), where the Dock sits and how big it is, whether it hides, what the trackpad and mouse do, displays, sound devices, power timers and privacy. Anything that is a second way of drawing the same thing, a legacy hardware mode, or a switch for behaviour that should just be right, is not an option. Accessibility settings are never bloat.

## Inventory

Keep = stays as is. Remove = recommended for removal (owner decides). Merge = fold into another control. **Done** = removed on this branch.

### Appearance
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Appearance: Light, Dark, Automatic | `appearance` | Keep | macOS has the same three |
| Accent colour (8 swatches) | `accent` | Keep | macOS has eight too |
| Text size | `accessibility text-size` | Keep | Average users change it |
| Wallpaper (light and dark picture) | `wallpaper light/dark` | Keep | |
| Wallpaper fit: Fill, Fit | `wallpaper fit` | Keep | Two choices, cheap |
| Tint window frames with the wallpaper | `tinted-frame` | Keep | macOS has "Allow wallpaper tinting in windows" |

### Taskbar
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Position: Left, Right, Bottom, Top | `bar edge` | Keep, **remove Top** (recommended) | The Dock offers left, bottom, right. A top taskbar sits under the separate menu bar, needs its own goldens and offsets (`crates/shell/desktop/src/client.rs:316`), and nobody coming from macOS or Windows expects it |
| Size (icon 32-56 pt) | `bar size` | Keep | macOS Dock size slider |
| Show names (the wide taskbar) | `bar names` | **Done: removed** | Owner decision 2026-11-01 |
| Automatically hide | `bar auto-hide` | Keep | macOS Dock option |
| Menu bar: In the taskbar, Separate | `bar menu-bar` | Keep, **show only when Position is Left** (recommended) | Finding 6: the choice does nothing on other edges |
| Kept apps | `bar apps` | Keep | Edited by Keep in Taskbar and dragging, not a Settings row |

### Notifications
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Do Not Disturb | session state | Keep | |
| Per-app notifications | notify service | Keep | macOS has per-app settings |

### Accessibility
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Screen reader, Zoom, Increase contrast, Reduce transparency, Reduce motion, Cursor size, Flash the screen, Sticky keys, Slow keys, Bounce keys | `accessibility …` | Keep | Accessibility is never bloat; all are in macOS |

### Keyboard
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Input sources | `keyboard input-sources` | Keep | |
| Key repeat rate, delay | `keyboard repeat-rate/repeat-delay` | Keep | macOS has both |
| Caps Lock: Caps Lock, Escape, Control, Compose, Off | `keyboard caps-lock` | Keep | macOS Modifier Keys; Compose is the Linux way to type accents |
| Keyboard shortcuts | shortcut list | Keep | |

### Displays
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Brightness | live | Keep | |
| Use as, Resolution, Scale, Refresh rate, Rotation | `display …`, `arrangement` | Keep | |
| Variable refresh rate (switch) | `display variable-refresh-rate` | **Merge** into Refresh rate (recommended) | macOS shows ProMotion as a refresh rate choice, not a separate switch |
| Night Light: on, schedule, from, to, warmth | `night-light …` | Keep | macOS Night Shift has the same |

### Sound
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Output and input device, volume, mute, input level | live | Keep | |
| Alert sound, alert volume | `sound alert/alert-volume` | Keep | macOS has both |
| Play user interface sound effects | `sound effects` | Keep | macOS has it |
| Play feedback when volume is changed | `sound volume-feedback` | Keep | macOS has it |
| Per-app volumes | live | Keep | Beyond macOS, but asked for on every desktop and cheap |

### Mouse
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Primary button, Natural scrolling, Speed, Pointer acceleration | `mouse …` | Keep | macOS has all four |

### Trackpad
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Tap to click, Secondary click (two fingers, corner), Natural scrolling, Speed | `trackpad …` | Keep | macOS has all |
| Dragging: Off, Tap, Tap with drag lock | `trackpad dragging` | Keep | macOS has it under Accessibility |
| Scroll method: Two fingers, Edge | `trackpad scroll-method` | **Remove** (recommended) | Edge scrolling is a legacy Synaptics mode; every current trackpad scrolls with two fingers; macOS has no such row |
| Trackpad pointer acceleration | `trackpad acceleration` | **Remove** (recommended) | macOS has no trackpad acceleration switch; keep libinput's adaptive profile |
| Disable while typing | `trackpad disable-while-typing` | **Remove** (recommended) | macOS just does it; keep it always on |
| Gestures: workspace swipe, overview swipe, desktop spread, launcher pinch | `trackpad …` | Keep | macOS has a switch per gesture |

### Privacy & Security, General
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Camera, Microphone, Screenshots, Screen sharing, Location per app | portal | Keep | |
| Clipboard history, Default apps, Login items | | Keep | |
| Computer name, Language, Region, 24-hour clock, Automatic time, Date and time, Time zone | | Keep | |

### Power
| Option | Where | Verdict | Why |
| --- | --- | --- | --- |
| Power mode, Use Power Saver on battery | `power low-power-on-battery` | Keep | macOS Low Power Mode |
| Dim the display after | `idle dim-after` | **Merge** (recommended): dim a fixed time before the display turns off | macOS has one "Turn display off after" timer and dims shortly before it |
| Turn off the display and lock after, Sleep after | `idle lock-after/suspend-after` | Keep | |
| When the lid is closed (on power) | `power lid-on-power` | Keep | |
| Show battery percentage | `power show-battery-percentage` (battery menu) | Keep | As on macOS |

### config.kdl only
| Option | Verdict | Why |
| --- | --- | --- |
| `reopen-apps` | **Decide** (recommended: a "Reopen windows when logging back in" checkbox in the Log Out confirmation, as macOS does) | Today a hidden key that nothing in the UI writes |
| `clock` | Keep | Written by the 24-hour switch |

## Recommendation

Remove the wide taskbar completely, keep old configs loading by ignoring the key, and stop there in code. Then let the owner decide the inventory's six recommendations in one pass: drop Top, show Menu bar only on the left, merge VRR into Refresh rate, drop Scroll method, Trackpad acceleration and Disable while typing, merge Dim into the display-off timer, and give `reopen-apps` a home or a fixed value. Each is a small branch.

## Implementation plan

Status (2026-11-01): milestone 1 done and merged; milestones 2-6 wait on the owner's decisions below.

1. **Remove the wide taskbar** (done). `rail.rs`, `taskbar.rs` and `combined_client.rs` lose the expanded layout, the painters, the name rows and the edge drag. `bande-taskbar-model` loses `BusEvent::Names` and `write_names`. `bande-config` loses `Bar::names`, `Setting::BarNames` and the `BarNames` property, and ignores `names` when parsing (test: a `bar` with `names #true` loads with no problems). Settings loses the switch, its strings and its client calls. The bar's unused `search` and `search-shortcut` strings go too. Tests: the expanded golden, the menu-rows test and the expanded live sweep are deleted; the live sweep keeps one set of cases. Docs: `docs/design/features/bar.md`, `docs/architecture/behaviour.md`, `docs/experience/settings.md`, `docs/roadmap.md`, `docs/design/datum.md`.
2. **Position without Top** (~1 h, on a yes). Drop `BarEdge::Top` from the Settings choices; map a stored `edge "top"` to `bottom` when parsing; remove the top branches in `taskbar.rs`, `taskbar_client.rs` and `desktop/src/client.rs`.
3. **Menu bar only on the left** (~1 h, on a yes). Settings hides the Menu bar row unless Position is Left (the registry needs a "shown when" rule, or Position gains a fifth entry "Left, with the menu bar" and the row goes).
4. **Trackpad trims** (~1.5 h, on a yes). Remove `scroll-method`, `trackpad acceleration` and `disable-while-typing` from Settings, config (ignored like `names`) and the compositor's libinput setup, which applies two-finger scrolling, the adaptive profile and disable-while-typing always.
5. **Refresh rate absorbs VRR** (~1 h, on a yes). The Refresh rate menu gains "Variable (up to N Hz)" on displays that support it; the switch goes.
6. **One display-off timer** (~1.5 h, on a yes). Dim at `lock-after` minus 30 s (or half of it, whichever is less); `dim-after` is ignored when parsing.

## Risks

- Someone who used the wide taskbar loses it silently. It existed from 2026-10-30, so few did.
- Each later removal must keep old configs loading; the `names` arm is the pattern.

## Decisions for the owner

Decided by the owner 2026-11-01: remove the wide taskbar (milestone 1, done).

Still open (recommendations, not built):
1. Drop Top from Position (milestone 2)?
2. Show the Menu bar row only when Position is Left (milestone 3)?
3. Drop Scroll method, Trackpad acceleration and Disable while typing (milestone 4)?
4. Fold Variable refresh rate into the Refresh rate menu (milestone 5)?
5. Replace Dim after with dimming shortly before the display turns off (milestone 6)?
6. `reopen-apps`: a checkbox in the Log Out confirmation, always on, or always off?
