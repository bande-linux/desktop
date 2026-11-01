# Apps polish: Terminal, Screenshot, file chooser, Launcher

Gap tables for four of our own apps, each against its best-in-class counterparts.
"Now" means more depth in what the app already is: no new apps and no speculative options.
**Built** marks what this pass delivered.

## Terminal

Compared with macOS Terminal, iTerm2, GNOME Console, Ptyxis and Windows Terminal.

Already there: profiles, text size, cursor shape, bell (flash or badge), notifications when long commands finish, find,
URL detection with Ctrl+click, links from OSC 8 escapes, dropping files to paste their quoted paths, splits, tabs,
window restore, and Show in Files for the folder the shell is in.

| Now | Later | Never |
| --- | --- | --- |
| **Built:** paths in the output are links. Ctrl+hover underlines a path that exists (relative to the shell's folder, `~/…`, or absolute, with any `:line:column` suffix dropped). Ctrl+click opens it. Its menu offers Open, Copy Path and Show in Files, and Show in Files selects that item. | Jump to the previous or next prompt (Shift+Ctrl+Up/Down) from OSC 133 marks. The marks are already parsed for notifications; the jump also needs grid positions that survive scrollback trimming and reflow. | A font-family picker for any installed font: the profile stays on the Datum monospace. |
| | Ctrl+click on `file:line` opens the default editor at that line (iTerm2's semantic history). Needs a desktop-wide "open at line" convention. | Background transparency and blur. |
| | A per-profile palette editor. Today the profiles are curated, and that is enough. | Quake-style drop-down window. |
| | "Not responding" detection for a hung foreground job. | |

## Screenshot and Markup

Compared with macOS Screenshot.app and Markup, GNOME Screenshot and KDE Spectacle.

Already there: the capture palette (Super+Shift+5) with screen, window and area capture plus screen and area recording;
an options menu (save to Desktop, Pictures or the clipboard; a timer of none, 5 or 10 seconds; show pointer); window
shots with a shadow; a floating thumbnail that opens Markup when clicked and can be dragged into apps; Markup with crop,
arrow, rectangle, text and highlight.

| Now | Later | Never |
| --- | --- | --- |
| **Built:** a recording that cannot start says so. If the screen cannot be opened, if the recorder fails to start, or if the recording cannot be saved, an alert now says what happened. Before, nothing happened at all (this was an open item). | Escape cancels a running timer. The countdown has no keyboard focus today, and taking focus would close the menus the timer is meant to capture. | Uploading to an online service. |
| **Built:** a screenshot that cannot be saved says so, and names the folder. | Escape cancels a running timer. The countdown has no keyboard focus today. | Arbitrary scripting hooks after a capture. |
| **Built:** a right-click menu on the floating thumbnail with Open in Markup (Open for recordings), Show in Files (selects the file), Copy, Move to Trash and Close. The thumbnail stays on screen while its menu is open. | "Other location…" in Save To, through the file chooser. | |
| **Built:** clicking the thumbnail of a recording opens it in the default video player. Before, it tried to open the video in Markup, which cannot decode video, so nothing appeared. | Recording with microphone audio. | |
| **Built:** swipe the thumbnail to the right to send it away. A short swipe springs back. | | |

## File chooser (Open and Save dialogs)

Compared with the macOS open and save panels and the GTK file chooser.

Already there: a sidebar with the same places as Files, a path bar, sortable columns, type-to-select, filters, New
Folder (Shift+Ctrl+N), Alt+Up and Alt+Down, a confirmation before replacing a file, the last folder remembered for
each app.

| Now | Later | Never |
| --- | --- | --- |
| **Built:** Go to Folder. Ctrl+L opens a sheet with the shown folder already typed in. Typing `/` or `~` in the file list opens it with that character, as in macOS and GTK. A folder opens; a file opens its folder and selects it, so Return then chooses it. A path that does not exist gets an alert that says so. | A preview pane for the selected file (Quick Look-style), sharing Files' previews. | Bookmark editing inside the dialog: the sidebar mirrors Files. |
| **Built:** go straight to a folder from the keyboard: Alt+Home (Home), Shift+Ctrl+D (Desktop) and Shift+Ctrl+O (Documents), the same keys as Files. | Back and forward buttons in the path bar. | |
| **Built:** Alt+Left and Alt+Right go back and forward through the folders visited. Going back selects the folder you came from. | | |
| | A Recent place listing recently used files. | |
| | An icon view next to the list. | |

## Launcher

Compared with Spotlight, GNOME Shell search and KDE KRunner.

Already there: a fuzzy search across apps, System Settings panes, folders and documents, ranked by how often and how
recently each was used; a calculator with unit conversion (length, mass, time, data, temperature, volume, speed);
Ctrl+Return to show a file in Files, Ctrl+C to copy a path or result; previews; emoji and clipboard modes; an empty
state with recent apps and documents.

| Now | Later | Never |
| --- | --- | --- |
| **Built:** unit conversion covers area (m², km², hectares, acres, square feet and so on) and kitchen volumes (cups, tablespoons, teaspoons, fluid ounces, pints, quarts), which people convert often in Spotlight. | "Search in Files" as the last row, to carry the query into a Files search window. | Web search and web suggestions. |
| | Currency conversion. Needs exchange rates from the network and a privacy decision. | Plugins. |
| | Definitions from an offline dictionary. | |
