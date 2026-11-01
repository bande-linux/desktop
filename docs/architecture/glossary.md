# Glossary: the taskbar and around it

One name for each part, used in docs, notes, strings and talk. macOS names are given where
Bande follows them. "In code" is what the code calls the part today.

| Name | What it is | macOS | In code |
| :--- | :--- | :--- | :--- |
| Taskbar | The whole bar on the left edge of every display, holding everything below; with a separate menu bar, only the Taskbar icons on the configured edge | menu bar + Dock | crate `bande-bar`, `TaskbarView`, `Taskbar`, layer namespaces `bar` and `taskbar` |
| Bande menu | The logo at the top: a click opens Search, a right click the Bande menu (About This Computer, System Settings…, Force Quit…, Sleep, Restart…, Shut Down…, Lock Screen, Log Out) | Apple menu | `Slot::BandeMenu`, `Slot::Search`, `Symbol::Logo`, `system_menu` |
| App menu | The active app's icon and name with `›` under the Bande menu, and the cascade it opens (Super+F2): the "Type to search" field, the app row, then File, Edit, View… | application menu and the app's menus | `Slot::AppMenu`, `paint_app_menu`, `menus::cascade` |
| Menu Search | The centred card on Shift+Super+/ listing every menu item of the active app | Help menu search | `menu_search`, namespace `menu-search` |
| Search | The centred card on Super+Space (no Ctrl+Space binding) (or Super+A, or a tap of Super): apps, settings, files, calculator; also Clipboard History (Super+V) and Emoji & Symbols (Super+.) | Spotlight | crate `bande-launcher`, `Action::ShowLauncher` |
| Taskbar icons | The app icons of the Taskbar: kept apps, a hairline, running apps, then the Trash; there is no separate Dock | Dock | `Taskbar`, `TaskbarLayout`, namespace `taskbar` |
| Kept apps / running apps | The two groups of the Taskbar icons: apps kept in the Taskbar, and running apps that are not | Keep in Dock / recent apps | `kept`, `running` |
| Active bar | The accent bar at the edge beside the active app's icon while it has a window | none (the Dock marks no active app) | `ACTIVE_BAR` |
| Running dot | The small grey square beside an app that runs but is not active, and beside the active app while it has no window | indicator for open apps | `RUNNING_DOT`, `DOT` |
| Badge, progress, attention ring | A count, a progress bar, or a pulsing orange ring on an app's icon | badge, bounce | `LauncherEntry`, `attention` |
| Icon menu | The menu on an app icon's right click: windows, Open, actions, Keep in Taskbar, Hide, Quit | Dock menu | taskbar menus |
| More apps | The "+N" cell when the icons do not fit | none | `more`, `more-apps` |
| Trash | The Trash icon at the end of the Taskbar icons | Trash | `trash` |
| Desktop dots | The workspace dots, the active one a pill; a click switches, on the active one opens the Overview | Spaces | `Slot::Workspace`, `workspace` |
| Status items | The tray icons of other programs, two per row, with `•••` for the rest | menu bar extras | `Slot::StatusItem`, `Slot::MoreStatusItems`, StatusNotifierItem |
| Input menu | The short name of the active input source (SV, EN), with more than one source | Input menu | `Slot::InputSource` |
| Control Center button | One button with the Wi-Fi, Sound, Bluetooth and Battery glyphs; a click opens Control Center | Control Center | `Slot::ControlCenter`, `Slot::Wifi`… |
| Control Center | The panel the button opens | Control Center | crate `bande-controlcenter` |
| Clock | Time over the short day, with a notification dot or a Do Not Disturb moon; a click opens Notification Center | clock | `Slot::Clock`, `ClockMark` |
| Notification Center | The sheet of notifications beside the taskbar | Notification Center | crate `bande-notifications` |
| Menu bar | The separate layout: a bar at the top with the Bande menu, the app's menus, status items and clock | menu bar | `MenuBar`, namespace `menubar` |
| App Switcher | The row of apps on Super+Tab | Command-Tab | `switcher` |
| Overview | All windows of the active desktop, and the desktops | Mission Control | `overview` |
