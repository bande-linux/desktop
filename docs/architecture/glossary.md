# Glossary: the taskbar and around it

One name for each part, used in docs, notes, strings and talk. macOS names are given where
Bande follows them. "In code" is what the code calls the part today.

| Name | What it is | macOS | In code |
| :--- | :--- | :--- | :--- |
| Taskbar | The whole bar on the chosen edge (left, bottom, right or top) of every display, holding everything below | menu bar + Dock | crate `bande-bar`, `BarClient`, `TaskbarView`, `StatusClient`, `Taskbar`, layer namespace `bar` |
| Bande menu | The logo at the start of the bar (top, or left on a horizontal bar): a click opens Search, a right click the Bande menu (About This Computer, System Settings…, Force Quit…, Sleep, Restart…, Shut Down…, Lock Screen, Log Out) | Apple menu | `Slot::BandeMenu`, `Slot::Search`, `Symbol::Logo`, `system_menu` |
| Menu button | The ⋮ right of a window's close, minimise and zoom controls, in Bande apps' title bars and in the compositor's bar of third-party windows that export menus; it opens the App menu below it | the app's menus in the menu bar | `WindowControl::Menu`, `show_app_menu` |
| App menu | The cascade the menu button opens: the "Type to search" field, the app row, then File, Edit, View… | application menu and the app's menus | `menus::cascade` |
| Search | The centred card on a tap of Super: apps, settings, files, calculator; also Clipboard History (Super+V) and Emoji & Symbols (Super+.) | Spotlight | crate `bande-launcher`, `Action::ShowLauncher` |
| Taskbar icons | The app icons of the Taskbar: kept apps, a hairline, running apps, then the Trash; there is no separate Dock | Dock | `Taskbar`, `TaskbarLayout` |
| Kept apps / running apps | The two groups of the Taskbar icons: apps kept in the Taskbar, and running apps that are not | Keep in Dock / recent apps | `kept`, `running` |
| Focus frame | The accent frame around the icon of the active app, with or without windows | none (the Dock marks no active app) | `paint_frame`, `framed` |
| Running dot | The small grey square beside an app that runs but is not active | indicator for open apps | `RUNNING_DOT` |
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
| Position | The Taskbar's edge, set in Settings ▸ Taskbar; there is no separate menu bar | Dock & Menu Bar settings | `BarEdge`, `bar { edge }` |
| App Switcher | The row of apps on Alt+Tab | Command-Tab | `switcher` |
| Overview | All windows of the active desktop, and the desktops | Mission Control | `overview` |
