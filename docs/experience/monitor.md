# Activity Monitor: depth and polish

Status: research and build plan of 2026-10-30, branch `night/files-monitor-spec`. Built on
2026-10-31 in `night/monitor-depth`: M2 to M6, M8, M9, the request plumbing of M7 and the redraw
part of M1 (see §9). Since 2026-11-01 its shortcuts use Ctrl, not Super
([decision 0005](../decisions/0005-ctrl-for-app-shortcuts.md)); the records in §9 to §11 keep the
keys of their day. It reads with the principles in [`../design/DESIGN.md`](../design/DESIGN.md)
and the design language [`../design/datum.md`](../design/datum.md). There is no feature proposal
for Activity Monitor; roadmap item C6 (inspector, GPU tab, column chooser) is done.

The owner's verdict: "Activity Monitor is very crude." The task is depth and polish inside the
app: what it shows, how it shows it, and what it lets you do. No new app, no new window type.

**Effort.** S is one slice: one agent, 1–2 hours. M is 2–4 slices. L needs several days or an
owner decision first.

**Now, later, never.** Now means the ordered slices in §6. Later means worth having after them.
Never means left out on purpose, with the reason.

## 1. What Activity Monitor does today

Checked in `crates/apps/monitor/src` and its goldens.

| Area | What works | Where |
| :--- | :--- | :--- |
| Tabs | CPU, Memory, Disk, Network, and GPU once DRM `fdinfo` reports use; Ctrl+1 to Ctrl+5 | `model.rs`, `monitor_view.rs` |
| Rows | Processes grouped by app (desktop entry or systemd unit), expandable to their processes; kernel threads in one group; other users' processes included | `apps.rs`, `model.rs` |
| Columns | Per tab, chosen by right-clicking the header, remembered in `~/.local/state/bande/monitor`; sort by header, the selection stays on its row while numbers reorder | `model.rs`, `menu.rs` |
| Measures | % CPU, CPU time, threads, PID, user, proportional memory (only while shown), resident, disk read and write per second and in total, GPU %, GPU time, GPU memory; network per interface | `sampler.rs`, `system.rs`, `proc.rs` |
| Footer | Up to three rows of label and value per column (CPU: System, User, Idle, Processes, Threads; Memory: Physical, Used, Cached Files, Swap, Compressed (zram), Pressure as a %) and a 240 × 88 pt bar graph of the last 60 samples | `monitor_view.rs` |
| Actions | Quit (SIGTERM to the group's roots) and Force Quit (SIGKILL to all, Alt) with an alert; several rows at once; other users' processes refused; Quit is Ctrl+E; Inspect (Ctrl+I, double click); Show in Files (Shift+Ctrl+R, starts `bande-files <path>`); Copy Name, Copy PID; Ctrl+F searches names and PIDs | `menu.rs`, `signal.rs`, `view.rs` |
| Inspector | A window per process: Info (parent, children, state, user, command, executable, running for, CPU, threads, resident, virtual), Threads, Open Files, Memory Map | `inspector.rs`, `inspect.rs` |
| Force Quit | Ctrl+Alt+Esc window listing your apps, Not Responding in red, Force Quit | `force_quit.rs` |
| Sampling | Every 2 s (first after 0.5 s), PSS and GPU only while a column needs them; every window is asked to redraw on every tick | `main.rs` |

**What reads as crude:** the footer is a small grid of numbers and a 240 pt bar chart without a
scale, drawn in red and blue; memory pressure is a bare percentage; the inspector is a raw table;
per-core use is sampled (`Snapshot::cores`) but never shown; nothing tells you what drains the
battery or keeps the laptop awake; and in the main window, Quit sends SIGTERM to an app that
could have asked to save.

## 2. What the references do

- **macOS Activity Monitor.** Panes for CPU, GPU, Memory, Energy, Disk, Network and Cache [A1].
  View filters: All Processes, All Processes Hierarchically, My Processes, System Processes, Other
  User Processes, Active and Inactive Processes, GPU Processes, Windowed Processes, Selected
  Processes, Applications in last 12 hours; View ▸ Columns; updates every 5 s by default, changeable
  under View ▸ Update Frequency [A2][A3]. Window ▸ CPU Usage and CPU History; the Dock icon can show
  live CPU [A4]. Memory: a Memory Pressure graph (green, yellow, red) and Physical Memory, Memory
  Used, App Memory, Wired, Compressed, Cached Files, Swap Used [A5][A6]. Energy: Energy Impact, 12 hr
  Power, App Nap, Graphics Card, Preventing Sleep, User; on laptops the footer shows remaining
  charge, time remaining or until full, time on battery or AC, and the battery over the last 12
  hours [A7].
- **Windows Task Manager.** Pages for Processes, Performance, App history, Startup apps, Users,
  Details and Services. Efficiency mode limits one process's resources and shows a leaf icon [M1].
  Startup apps rate their impact None, Not Measured, Low, Medium or High from CPU and disk use at
  sign-in [M2].
- **GNOME System Monitor.** Processes, Resources and File Systems; End Process first, Force Stop
  if it does not end; change priority; memory maps; open files and "which program uses this file";
  an update interval in the preferences [N1].
- **KDE Plasma System Monitor.** Overview, Applications, History and Processes pages, all editable
  into custom dashboards [P1] (third-party description).

## 3. Datum for an instrument

How the graphs and readouts should look, so every slice draws the same way:

- **Readouts** sit on the sheet under a `hairline` (Datum §4 allows the status-bar top). Labels use
  the Label role (10.5 pt, 600, caps, tracked, `ink3`), values Body strong with tabular figures in
  `ink`. Columns of readouts align on the 8 pt grid and wrap to fewer columns before anything
  clips.
- **Graphs** sit in a `well` with radius 6. Three `hairline` grid lines at 25, 50 and 75 %; the
  scale (100 %, or the peak rate for disk and network) top left in Label `ink3`; "2 min" bottom
  left. Newest sample at the right edge; the empty past is the well, not a gap.
- **Series:** the primary series is an `accent` line of 1.5 pt over an area of `accent_soft`; the
  secondary series (system CPU, written bytes, sent bytes) is an `ink2` line without fill. Red,
  amber and green appear only for a state: memory pressure, a battery that is charging, Not
  Responding. Today system CPU and written bytes are drawn in `danger`, which breaks Datum's one
  signal.
- **Motion:** a tab switch cross-fades the footer (`fade`); expanding the footer moves the divider on
  `standard`; Reduce Motion fades.

## 4. Gap table

### Look and graphs

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| The graph is a 240 pt bar chart without scale, in red and blue | It looks like a debug view; you cannot read a value off it; red suggests an error | now | S |
| Readouts are a fixed 210 pt grid that hides the graph in narrow windows and clips long labels | Resize the window and the graph disappears | now | S |
| Per-core use is sampled but never shown | "Is one core pegged?" is the first CPU question; macOS has CPU History, Windows the Performance page | now | S |
| Memory pressure is a bare "3.2 %" | Nobody knows if 3.2 % is good; macOS answers with green, yellow, red | now | S |
| The inspector's Info section is a raw two-column table | It reads as a dump, not a designed panel | now | S |
| Only light goldens | Dark mode regressions slip through | now | S |
| A live CPU graph on the taskbar icon | macOS shows it in the Dock; needs a shell protocol for live icons | later | M |
| The graph slides smoothly instead of stepping every 2 s | A finer touch; costs frames every 2 s | later | S |

### Measures

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| No Energy tab on a laptop | "What drains my battery?" has no answer; macOS's most used pane on a MacBook | now | M |
| Nothing shows what keeps the computer awake | A video tab or a backup holds off sleep and the lid stays warm; macOS has Preventing Sleep | now | S |
| No wake-ups per process | An idle-looking app that wakes 500 times a second costs battery; macOS shows Idle Wake Ups | now | S |
| Memory has no breakdown (App, Kernel, Compressed, Cached) | "Used 7.1 GB" alone does not say whether apps or caches hold it | now | S |
| Network lists `wlp1s0f0` and `enp0s1` | People know "Wi-Fi" and "Ethernet" | now | S |
| Disk shows bytes only, no operations per second | Many small writes (a database, a sync client) look idle in bytes | now | S |
| Per-process network | Linux has no per-process byte counters without root or eBPF | never | — |
| 12-hour average energy per app | Needs a daemon that samples while Activity Monitor is closed | never | — |
| App Nap and Graphics Card columns | No such concepts here | never | — |

### Control

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| Quit sends SIGTERM to an app | An app with unsaved work dies without asking; the bar's and the taskbar's Quit ask the app politely through the compositor | now | S |
| The main window never marks an app Not Responding | Only the Force Quit window (started by the compositor as a shell client) sees `app_responding` | now | S |
| No view filter (All, Mine, Apps) | With 300 system processes, finding your own work means scrolling or searching | now | S |
| Pause and Resume a process (SIGSTOP, SIGCONT) | Freezes a runaway job without losing it; GNOME has Stop and Continue | later | S |
| Lower a process's priority ("Run in Background", like Efficiency mode) | Needs apps in their own systemd scopes first (maturity audit §2) | later | M |
| Sample a process (stack traces) | Needs ptrace rights and debug info; rarely used | later | M |
| Process tree by parent (Hierarchically) | App grouping plus Parent and Children in the inspector cover most uses | later | S |
| An update-frequency setting | 2 s while shown is right; the answer is to be cheap, not configurable | never | — |
| Startup apps, App history, Users, Services pages | Login items belong to Settings; the rest needs daemons or has no user on a one-person laptop | never | — |
| Custom dashboard pages (KDE) | One designed view per tab is the Bande answer | never | — |

### Inspector

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| Open Files lists files but not network ports | "What is listening on 8080?" is a top reason to open it; macOS has Open Files and Ports | now | S |
| No systemd unit or app shown for a process | You cannot tell which service started a worker | now | S |
| Environment variables | Useful to developers, noise to everyone else | later | S |

### Cost, keyboard, access, empty states

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| Every tick redraws every window, and sampling continues at full cost while hidden | A monitor that costs battery is the wrong tool; the night's idle work aims at zero busy work | now | S |
| No number for the Monitor's own CPU and memory | Regressions go unseen | now | S |
| Search with no match shows an empty table | No message, no way out | now | S |
| Footer readouts and graphs are not in the accessibility tree | A screen reader hears the table, never the totals | now | S |

## 5. Decisions

1. **No CPU History window.** Clicking the graph, dragging the footer's top edge, or View ▸ Show
   CPU Cores grows the footer into one strip per core. Progressive disclosure inside the window
   instead of another window type.
2. **Energy tab only when a battery exists,** like the GPU tab only when the GPU reports use.
3. **Energy Impact is a published formula,** not a secret score: % CPU + % GPU + wake-ups per
   second ÷ 10, averaged over the last 30 s. One sentence under the column's tooltip says so.
4. **Linux names in Memory:** App Memory (`AnonPages` + `Shmem`), Kernel (`SUnreclaim` +
   `KernelStack` + `PageTables`), Compressed (zram), Cached Files, Swap Used. "Wired" is a macOS
   term and would mislead.
5. **Memory pressure states from PSI:** green while `some avg10` < 10 %, amber from 10 %, red when
   `full avg10` ≥ 5 %. The thresholds live in one place and are tested.
6. **View filter in the View menu:** All Processes (default), My Processes, Apps. Remembered.
7. **Red never draws a series** (see §3).
8. **Quit is polite for apps.** An app row asks the app through the compositor, as the bar's Quit does; a
   bare process row still gets SIGTERM; Force Quit stays SIGKILL.
9. **No update-frequency setting.** 2 s while a window is shown; while every window is hidden, only
   the system totals are sampled, so the graphs keep their history at a fraction of the cost.

## 6. Build slices

Each slice is one agent for 1–2 hours and ends green on the night brief's gate. Tests are
behaviour tests that fail without the change; the sampler reads a fixture `/proc` and `/sys` as
today (`tests/fixture`). "Goldens" are headless renders in `crates/apps/monitor/tests/golden/`,
light and dark. "Review" appends steps and `shot` lines to one growing scenario,
`ci/review/scenarios/monitor-depth.scenario`, recorded with `ci/review/run.sh` and copied to
`/home/mikael/.cache/bande-agents/report/<branch>/`. Crate: `apps/monitor` unless named.

### M1. The Monitor's own cost

Measure with `bande-footprint` (or a small harness) the Monitor's CPU time per sample and its RSS
on this VM with the session's processes, then remove the waste: redraw a window only when a sample
arrived for it or it animates; skip formatting rows for tabs that are not shown. If the toolkit
reports a hidden window, sample only `/proc/stat`, `/proc/meminfo`, `/proc/diskstats` and
`/proc/net/dev` until one is shown; if it does not, record the need (decision 9).

- Tests: a tick without a new sample requests no redraw; with every window hidden, a sample reads no
  `/proc/<pid>` file from the fixture.
- Acceptance: before and after numbers (CPU ms per sample, % of one core, RSS) in the report and in
  `docs/architecture/performance.md`.

### M2. The footer as an instrument

Redraw the footer per §3: readouts as Label and tabular values, wrapping from three columns to two
to one; the graph takes the rest of the width (at least 200 pt) with grid lines, scale and "2 min";
series colours from §3 (CPU user `accent`, system `ink2`; Disk read `accent`, written `ink2`;
Network received `accent`, sent `ink2`). The graph publishes an accessible description ("CPU 34 %,
user 28 %, system 6 %").

- Tests: at 600 pt width the graph is still painted and no readout is clipped; the disk scale label
  follows the peak of the shown history; no painted series uses `palette.danger`.
- Goldens: `monitor-cpu` (updated), `monitor-disk`, `monitor-network`, light and dark; this slice
  adds dark variants of the four existing goldens.
- Review: the CPU tab while a build runs; shot.

### M3. Memory pressure and what holds memory

The Memory footer's graph becomes the pressure graph: each sample's bar in `success`, `warning` or
`danger` per decision 5, with "Memory Pressure" as its title. Readouts: Physical Memory, Memory
Used, App Memory, Kernel, Compressed, Cached Files, Swap Used (decision 4). The memory used share
moves into the readouts as a stacked bar (App, Kernel, Compressed, Cached) under "Memory Used".

- Tests: fixture `meminfo` and `pressure/memory` give the expected readouts; PSI at 4 %, 12 % and
  `full` 6 % give green, amber and red.
- Goldens: `monitor-memory` light and dark (one per state is not needed; use amber).

### M4. Per-core strips

Clicking the CPU graph, dragging the footer's top edge, or View ▸ Show CPU Cores grows the footer
(`standard` spring) into one 28 pt strip per core. When `/sys/devices/system/cpu/cpu*/cpu_capacity`
differs between cores (Apple silicon), the strips group under PERFORMANCE and EFFICIENCY labels.
The expanded state is remembered. Escape or the same command collapses it.

- Tests: four fixture cores with two capacities give two labelled groups; a core's strip shows its
  own share from `/proc/stat`; the state survives a restart.
- Goldens: `monitor-cores` light and dark. Review: expand during a build; shot.

### M5. The inspector, designed

Info becomes grouped facts on the sheet: PROCESS (name, PID, state, user, app or unit), PARENT AND
CHILDREN (clickable, opening their inspectors), COMMAND (command line, executable with "Show in
Files"), RESOURCES (CPU, CPU time, threads, resident, virtual, running for), labels in the Label
role and values tabular, every value selectable and copyable with Ctrl+C. Threads, Open Files and
Memory Map keep their tables.

- Tests: Ctrl+C on the PID row copies the PID; activating a child opens its inspector; the app or
  unit row names the systemd unit from the fixture `cgroup`.
- Goldens: `inspector` (updated) light and dark.

### M6. View filter

View ▸ All Processes, My Processes, Apps (groups with a desktop entry), as radio items without
shortcuts (Ctrl+digits switch tabs); remembered in the state file beside the columns. Search applies within
the filter. The window title stays "Activity Monitor"; the filter shows as a quiet label beside
the search field when it is not All.

- Tests: My Processes hides root's daemons from the fixture; Apps keeps only grouped apps; the
  choice survives a restart.

### M7. Polite Quit and Not Responding in the main window

Quit on an app row emits a request to quit that app id; `main.rs` sends it through the
compositor's `quit_app` (the same path as the bar's Quit); process rows still use SIGTERM. The main window
marks apps Not Responding like the Force Quit window. Both need the compositor to offer
`quit_app` and `app_responding` to Activity Monitor (§8); without that, ship the request plumbing
behind the existing fallback and report the need.

- Tests: Quit on an app group yields a quit-app request with its id and sends no signal; Quit on a
  child process signals only that PID; an `AppResponding` event marks the group in the main view.

### M8. Wake-ups and Energy Impact

A Wake-ups/s column (voluntary context switches per second, summed over
`/proc/<pid>/task/*/status`, read only while the column is shown, like proportional memory) for the
CPU tab's column choice, and an Energy tab, shown when UPower reports a battery, with Name, Energy
Impact (decision 3), Wake-ups/s, % CPU, % GPU, User, sorted by Energy Impact.

- Tests: a fixture process with 0 % CPU and 600 switches per second ranks above an idle one; the
  tab is absent without a battery; wake-ups are not read while no column needs them.

### M9. Preventing Sleep

Read logind's `ListInhibitors` on a worker (system bus), keep `block` inhibitors whose `what`
contains `sleep` or `idle`, and map their PIDs to rows: a Preventing Sleep column (Yes, with the
reason as the tooltip) in the Energy tab, and the reason in the inspector. The list refreshes with
each sample.

- Tests: parsing of `a(ssssuu)` tuples; a fake logind on a private bus with one sleep blocker marks
  the right group; a `delay` inhibitor does not.

### M10. The battery in the Energy footer

Readouts: Remaining Charge, Time Remaining or Time Until Full, Time on Battery (from the last
change of state in the history). Graph: "Battery (last 12 hours)" from UPower's
`GetHistory("charge", 43200, 72)`, the charge as the `accent` series and charging spans as a thin
`success` band at the bottom. The UPower history proxy moves from `apps/settings` into
`libs/status` as its second consumer.

- Tests: a history with a plug-in event gives the right Time on Battery; charging spans are found;
  Settings' battery tests still pass on the moved code.
- Goldens: `monitor-energy` light and dark. Crates: `apps/monitor`, `libs/status`,
  `apps/settings`.

### M11. Open Files and Ports, and the unit

Open Files gains a Ports part: TCP and UDP sockets from `/proc/net/{tcp,tcp6,udp,udp6}` matched to
the process's socket inodes, shown as "TCP 127.0.0.1:8080 (listening)" or "TCP 10.0.0.2:51514 →
93.184.216.34:443". The Info section names the systemd unit (M5 uses it).

- Tests: fixture `net/tcp` and `fd` links give a listening and a connected row; IPv6 addresses are
  printed in their short form.

### M12. Disk and network details

Disk: reads and writes per second from `/proc/diskstats` (fields 4 and 8) in the readouts. Network:
interfaces named by kind from `/sys/class/net/<if>` (`wireless` → Wi-Fi, `type` 1 with a device →
Ethernet, `tun` or `wireguard` → VPN, else the kernel name), with the kernel name in `ink2` after
it; packets in and out per second as choosable columns; down interfaces hidden.

- Tests: fixture `sys/class/net` gives Wi-Fi, Ethernet and VPN names; IOPS from two fixture
  `diskstats` samples.

### M13. Empty states and access

"No processes match “text”" centred in `ink3` with a quiet "Clear Search" button; the footer
readouts publish label and value; the Energy and GPU tabs announce themselves when they appear.
The keyboard path that works today (Ctrl+F, Return or Down into the list, Return to expand a group
or inspect a process, Escape to close the inspector) gets one end-to-end test so it stays working.

- Tests: a search for a missing name shows the message and Clear Search restores the rows; the
  accessibility tree contains "Processes, 312" and the graph description; the keyboard path above
  opens an inspector and closes it without a pointer.

## 7. Polish checklist (every slice)

- Every action is in the menu bar with its shortcut, in the row's context menu where it applies,
  and reachable without a pointer.
- Tabular figures for every number; units from `format_bytes` and `format_rate`; percentages one
  decimal.
- Nothing new reads `/proc/<pid>` files unless a shown column needs them.
- Light and dark goldens for every visible change; 1× and 1.5×.
- Strings through `bande-l10n`; the message-id test covers new files.

## 8. Needs from other crates

- `platform/compositor`: send xdg `suspended` also for windows on another workspace and for
  windows fully covered by others (today only minimised and hidden windows) (M1).
- `libs/status`: UPower history, moved from `apps/settings` (M10).
- `shell/bar` (later): a way for an app to draw a live bar icon.
- `libs/xdg` (later): apps in their own systemd scopes, for priority control.

## 9. Built on 2026-10-31

- **M2.** Readouts in the Label role with tabular values, column width measured from the widest
  readout, three to two to one columns before anything clips; the graph is a well with grid lines,
  the scale and "2 min", `accent` line over `accent_soft` and an `ink2` secondary line; the footer
  publishes its readouts and the graph's description to the accessibility tree. The graph keeps at
  least 160 pt (not 200) so the Memory readouts still fit in two columns at 600 pt.
- **M3.** Memory Pressure graph coloured by PSI state (thresholds in `Pressure::level`); App
  Memory, Kernel, Compressed, Cached Files and Swap Used with a stacked bar under Memory Used and
  colour keys on the readouts it explains.
- **M4.** View ▸ Show CPU Cores, a click on the CPU graph, or dragging the footer's top edge up
  (24 pt snaps) opens one strip per core on the `standard` spring, grouped by `cpu_capacity` into
  Performance and Efficiency; the same command, a click, Escape or dragging down closes it; the
  choice is remembered.
- **M5.** The inspector's Info groups into Process (with the systemd unit), Parent and Children,
  Command and Resources as disclosure sections; Super+C copies the selected values; activating the
  executable shows it in Files. The values are table cells, so the Label role and tabular figures
  wait for a cell style in the toolkit's table.
- **M6.** View ▸ All Processes, My Processes, Apps; remembered. The filter is named in the search
  field's placeholder ("Search My Processes") instead of a label beside it: a label there pushed
  the tabs into the window title at 860 pt. The View menu shows them as plain items because
  `bande-app-menu` has no radio items yet.
- **M7.** Quit on an app row asks the compositor to quit the app (`Window::quit_app`) and falls back
  to SIGTERM of its roots when the window has no `bande_shell_v1`, which is the case for the main
  window today; process rows still get SIGTERM. The compositor side (§8) is open.
- **M8.** Wake-ups/s from `voluntary_ctxt_switches` of every task, read only while a column needs
  it; Energy tab when `/sys/class/power_supply` has a battery; Energy Impact as decision 3.
- **M9.** logind `ListInhibitors` on a worker thread, asked once per sample and only while the
  Preventing Sleep column or an inspector shows; the reason appears in the inspector.
- **M1.** A tick between samples now redraws only the window whose own timer is due; before, every
  wake redrew every window. Sampling less while hidden still needs the toolkit's hidden signal (§8).

## 10. Built on 2026-10-31, second pass

- **M7, compositor side.** A new global, `bande_app_control_v1`, offered to every client, carries
  only `quit_app(app_id)` and the `app_responding` event; it lists no windows and types no text, so
  ordinary apps get no more than they could do to another app of the same user. Shell clients are
  never asked to quit through it. `Window::quit_app` uses it when the window has no
  `bande_shell_v1`, and `Window::watch_apps` binds it on demand, so apps that never ask receive
  no events. The Monitor's main window watches apps: Quit on an app row is the polite path the bar's Quit takes
  and hung apps are marked Not Responding in the list.
- **M1, hidden sampling.** The compositor sets xdg_toplevel `suspended` (xdg_wm_base 6) on
  minimised and hidden windows; the toolkit reports it in `ToplevelState::suspended`. While every
  Monitor window is suspended the sampler reads only `/proc/stat`, `meminfo`, `diskstats` and
  `net/dev`, keeps the last process list and skips redraws; on showing it samples at once and
  re-baselines processes so no CPU spike appears.
- **M6, radio items.** `bande-app-menu` has `Entry::radio(selected)` (dbusmenu
  `toggle-type` radio); the menu bar already draws the checkmark. View ▸ All Processes, My
  Processes and Apps are radio items and the Monitor republishes its menus when the filter changes.
- **M5, cell style.** Table cells take `.label()` (Label role) and `.tabular()`; the inspector's
  labels and section titles use the Label role and its values tabular figures.
- **M12.** Disk readouts start with Reads/s and Writes/s from `/proc/diskstats` fields 4 and 8.
  Interfaces are named by kind, "Wi-Fi (wlan0)", "Ethernet (eth0)", "VPN (wg0)", from
  `/sys/class/net`; down interfaces are hidden. Packets per second as columns are not built.
- **M11.** Open Files names sockets by their ports from `/proc/<pid>/net/{tcp,tcp6,udp,udp6}`:
  "TCP 127.0.0.1:8080 (listening)", "TCP 10.0.0.2:51450 → 93.184.216.34:443", IPv6 in short
  form; the net files are read only when the process has a socket.
- **M13.** A search that matches nothing says "No processes match “text”" in `ink3` with a quiet
  Clear Search button; tests drive Super+F, typing, Down and Super+I to an inspect request, and
  Escape closes the inspector. Return on a top-level row expands its group rather than inspecting,
  even when the group holds one process; that is left as it is.
- **Not built:** the Energy and GPU tabs announcing themselves when they appear.

## 11. Built on 2026-10-31, `day/shell-polish`

- **M10.** The Energy footer adds Remaining Charge, Time Remaining or Time Until Full (from
  UPower's display device, "–" while UPower is still estimating) and Time on Battery (from the
  first reading after the last plugged-in one in the history; "–" on power or when the last 12
  hours hold no plug-in). The graph is "Battery (last 12 hours)": `GetHistory("charge", 43200, 72)`
  placed by time, the charge as the `accent` series over `accent_soft`, plugged-in spans as a 3 pt
  `success` band along the bottom, and "12 hr" in the corner; the accessibility description names
  the charge. A worker reads the display device each sample and the history at most once a minute,
  only while the Energy tab is shown. `GetHistory` and the battery lookup live in
  `bande_status::history`, which Settings' Battery pane now uses too.

## Sources

- A1. Apple, Activity Monitor User Guide: https://support.apple.com/guide/activity-monitor/welcome/mac
- A2. Apple, View information about processes in Activity Monitor:
  https://support.apple.com/guide/activity-monitor/view-information-about-processes-actmntr1001/mac
- A3. Apple, Change how often information is updated:
  https://support.apple.com/guide/activity-monitor/change-how-often-information-is-updated-actmntr2224/mac
- A4. Apple, View CPU activity: https://support.apple.com/guide/activity-monitor/view-cpu-activity-actmntr43452/mac
- A5. Apple, View memory usage: https://support.apple.com/guide/activity-monitor/view-memory-usage-actmntr1004/mac
- A6. Apple, Check if your Mac needs more RAM:
  https://support.apple.com/guide/activity-monitor/check-if-your-mac-needs-more-ram-actmntr34865/mac
- A7. Apple, View energy consumption:
  https://support.apple.com/guide/activity-monitor/view-energy-consumption-actmntr43697/mac
- M1. Microsoft, Learn about performance features in Microsoft Edge (Task Manager efficiency mode):
  https://support.microsoft.com/en-gb/topic/learn-about-performance-features-in-microsoft-edge-7b36f363-2119-448a-8de6-375cfd88ab25
- M2. Microsoft, Configure startup applications in Windows:
  https://support.microsoft.com/en-us/windows/experience/startup-boot/configure-startup-applications-in-windows
- N1. GNOME, System Monitor help: https://help.gnome.org/users/gnome-system-monitor/stable/ and
  Force Stop: https://help.gnome.org/users/gnome-system-monitor/stable/process-kill.html.en
- P1. It's FOSS, Getting more out of KDE Plasma System Monitor (third party):
  https://itsfoss.com/kde-system-monitor-tweaks/
- Linux interfaces used: `proc(5)` (`/proc/<pid>/status`, `/proc/net/tcp`, `/proc/diskstats`),
  the kernel's PSI documentation (`Documentation/accounting/psi.rst`),
  `org.freedesktop.login1.Manager.ListInhibitors` and `org.freedesktop.UPower.Device.GetHistory`.
