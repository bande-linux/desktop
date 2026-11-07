# Fault reports

When something goes wrong, the computer keeps one small report of it, and sends it only if sending is switched on. Normal use writes nothing. The local record of each failure comes from `bande-crash-reports`; this plan adds what it lacks: structured capture, hangs, kernel and GPU events, a spool and a sender, and a place in Settings to read, delete and send.

## How macOS does it

Each claim is marked with where it comes from:
- **[apple]**: Apple's documentation, read for this plan
- **[3rd]**: a third-party or community source. Some were seen only as search results and not opened, and links on forum claims show where to look rather than a checked quote
- **[memory]**: from memory, not checked. Do not treat these as fact.

### What triggers a report

| Trigger | Report | Source |
|---|---|---|
| A process dies from a signal or exception | Crash report, `.ips` with `bug_type` 309 | [apple] [JSON format](https://developer.apple.com/documentation/xcode/interpreting-the-json-format-of-a-crash-report) |
| The OS watchdog kills an unresponsive app | A crash report with termination code `0x8badf00d` and the wall-clock allowance used | [apple] [Watchdog terminations](https://developer.apple.com/documentation/xcode/addressing-watchdog-terminations) |
| A main run loop is unresponsive | Hang reporting; most tools start at 250 ms. It measures the main run loop as a proxy for what the user feels | [apple] [Understanding hangs](https://developer.apple.com/documentation/xcode/understanding-hangs-in-your-app) |
| Force Quit of an unresponsive app | `spindump` report: "used by various system components to create reports when an unresponsive application is force quit"; run by hand, it samples user and kernel stacks of every process, 10 s at 10 ms by default | [3rd] [spindump(8), old copy](https://www.unix.com/man_page/osx/8/spindump) |
| Memory pressure (iOS family, not macOS) | Jetsam report: memory of all processes, no backtraces | [apple] [Jetsam reports](https://developer.apple.com/documentation/xcode/identifying-high-memory-use-with-jetsam-event-reports) |
| CPU, disk-write or launch budget exceeded | MetricKit diagnostics, non-fatal | [apple] [MetricKit](https://developer.apple.com/documentation/metrickit) |
| WindowServer stops checking in | Kernel panic, "userspace watchdog timeout: no successful checkins from com.apple.WindowServer in 120 seconds", with a `.spin` report beside it | [3rd] [Apple forum thread](https://developer.apple.com/forums/thread/679016); forum posts only |
| The GPU restarts | `.gpuRestart` files in DiagnosticReports; no Apple documentation found | [3rd] [Apple forum thread](https://developer.apple.com/forums/thread/679016), [Eclectic Light](https://eclecticlight.co/2020/06/05/windowserver-gpu-crash-different-from-a-kernel-panic/) |
| Kernel panic | `panic-full-*.ips` or `.panic` after the reboot, with a stackshot | [3rd] [8kSec](https://8ksec.io/analyzing-kernel-panic-ios/), [Eclectic Light](https://eclecticlight.co/2025/04/15/save-and-read-the-panic-log/) |
| A person asks | `sysdiagnose`, also by the key chord Control-Option-Command-Shift-Period | [apple] [sysdiagnose(1)](https://keith.github.io/xcode-man-pages/sysdiagnose.1.html), a man page mirror |

### What a crash report captures, and why each field earns its place

All [apple]: [Examining the fields](https://developer.apple.com/documentation/xcode/examining-the-fields-in-a-crash-report), [JSON format](https://developer.apple.com/documentation/xcode/interpreting-the-json-format-of-a-crash-report).

- **Two JSON objects.** A short metadata line (`name`, `bug_type`, `incident_id`, `build_version`, `timestamp`), then the report. Reports can be indexed without parsing the whole thing.
- **Identity:** `incident` (never shared by two reports) and `crashReporterKey`, an anonymised per-device id reset on erase. Together they tell "one person, many times" from "many people".
- **Build:** process, version, `osVersion`, `modelCode`, `cpuType`. These separate a release's bug from a machine's bug.
- **Time and context:** `procLaunch`, `captureTime`, `uptime`, parent process, coalition. Crash at launch and crash after a week are different bugs.
- **Exception:** type, signal, subtype, codes, `vmregioninfo`. Says what kind of failure it was and whether the address was null or a stale pointer.
- **Termination:** namespace, code, indicator, and who ended the process. Separates "it crashed" from "the system killed it".
- **`asi` (Application Specific Information):** the framework's last message before dying; often the most direct description.
- **`threads`:** every thread, with name and frames as image index plus offset, plus a symbol when known. Registers for the faulting thread only.
- **`usedImages`:** name, path, UUID, base and size of each binary. Lets a report be symbolicated later, on another computer.
- **`vmSummary`:** a summary of memory in use.
- **Privacy:** `procPath` has user-identifiable path components replaced by placeholders. A crash report has no raw memory.

### Logging without storing normal behaviour

- All messages go to memory first. **Debug and info stay in memory only**; debug is captured only if enabled by configuration. Info is written to disk "when faults and, optionally, errors occur". Default, error and fault are written, compressed, up to a quota. [apple] [OSLogType](https://developer.apple.com/documentation/os/oslogtype), [Generating log messages](https://developer.apple.com/documentation/os/generating-log-messages-from-your-code). This is the idea to copy.
- Level sets cost: debug is very cheap because it stays in memory; faults cost more because more is captured and written. [apple] same page.
- Dynamic strings are redacted by default, numbers are not, and a value can be replaced by a hash. [apple] same page.
- On disk, `Persist` is held by size to about 520 MB of 10 MB files, so it covers hours or days. `Special` holds faults and errors and lasts weeks. [3rd] [Eclectic Light](https://eclecticlight.co/2025/09/29/inside-the-unified-log-3-log-storage-and-attrition/). Macs now keep about 5 to 7 days. [3rd] [Eclectic Light](https://eclecticlight.co/2026/03/12/how-long-does-the-log-keep-entries/).
- `os_signpost` marks intervals, recorded by Instruments, with their own persistence switch. [apple] [Recording performance data](https://developer.apple.com/documentation/os/recording-performance-data), [log(1)](https://keith.github.io/xcode-man-pages/log.1.html).
- `tailspin` keeps a kernel trace in a ring and saves it on request. [apple] [tailspin(1)](https://keith.github.io/xcode-man-pages/tailspin.1.html); man page title and synopsis only.
- `sysdiagnose` gathers a spindump, some seconds of `top`, kernel zones, extensions, per-process memory, recent logs, a System Profiler report, recent crash reports, disk use, the I/O registry and network status. It runs as root and writes to `/var/tmp`. [apple] [sysdiagnose(1)](https://keith.github.io/xcode-man-pages/sysdiagnose.1.html). Feedback Assistant attaches one to a bug report. [memory]

### Storage

- `~/Library/Logs/DiagnosticReports` for a user's processes, `/Library/Logs/DiagnosticReports` for system ones, readable by admins. [3rd] [Mojave man page](https://unix.com/man-page/mojave/8/diagnostics_agent/), community posts.
- **The Mac keeps analytics information for one month, then deletes it.** [apple] [Share analytics](https://support.apple.com/guide/mac-help/mh27990)
- A `Retired` folder for reports already submitted. [memory]
- `ReportCrash` is launched on demand by launchd, once as root and once per user; nothing runs between crashes. [3rd] [Eclectic Light](https://eclecticlight.co/2021/12/10/how-macos-reports-crashes/). `osanalyticshelper` and `SubmitDiagInfo` do the submission. [memory]. Only a man page title for the first was found.
- **Panics survive the reboot** through NVRAM: the kernel writes there, and the next boot formats it into a report. [3rd] [community answer](https://discussions.apple.com/thread/255754513); not Apple documentation. The key name `aapl,panic-info` and the role of iBoot on Apple silicon are [memory].

### Sending and consent

All [apple] [Share analytics](https://support.apple.com/guide/mac-help/mh27990).
- Opt-in: "sent only with your consent" (asked at setup: [memory]), changeable in Privacy & Security ▸ Analytics & Improvements.
- "Submitted anonymously"; personal data is left out or removed before sending, some with differential privacy.
- Sent automatically after an unexpected quit, a force quit or a restart from a system error.
- Console shows reports "even if you did not select to send reports automatically". Administrators see all, others only their own. This is what makes consent honest: you can read what would go.
- A separate switch shares with app developers. MetricKit delivers diagnostics at once and metrics at most daily, only from people who opted in. [apple] [MetricKit](https://developer.apple.com/documentation/metrickit)

TN2123 could not be read at Apple (the archive URL redirects). A [mirror](https://discmaster.textfiles.com/file/19409/Dev.CD%20Mar%2005.iso/%20What's%20New/Technical%20Notes%20and%20Q&As/ADC%20Reference%20Library/technotes/tn2004/tn2123.html?html=true) shows it describes CrashReporter in Mac OS X 10.3.5, so it was not used.

## What Bande does

| macOS | Linux equivalent | Decision |
|---|---|---|
| `ReportCrash` on demand | `systemd-coredump`, then `bande-crash-reports` | Use. Nothing runs between crashes |
| `threads`, `usedImages` | coredump's stack text (all threads and build ids, to be checked on a real dump) from `libdw`, symbols from `.gnu_debugdata` | Parse into JSON, one thread list and one image list |
| Registers, exception, address | `NT_PRSTATUS`, `NT_SIGINFO` in the core | Read from the core for the faulting thread |
| Minidump-size core slice | An ELF core with notes and stack windows only | **Local and dev config only.** Stack memory can hold typed text, so it is never sent by default. macOS sends none either |
| `asi` | Rust panic hook, `bande-log` | Panic message and location in the manifest |
| Info in memory, written on fault | `tracing` ring per process in `bande-log`, mapped on tmpfs so it outlives the process | Copy. Ring reads debug and up. Journal stays at info, which is small; ring cost is measured |
| `spindump` on force quit or hang | `/proc/<pid>/task/*/{stack,wchan,syscall,stat}` three times 1 s apart, then `eu-stack -p` for user stacks, bounded | Copy, without 10 s of sampling. Kernel stacks come first because a thread stuck in D state can block a ptrace attach |
| App hang | Compositor ping timeout | Report if Not Responding lasts 10 s and the person interacted with it, or on Force Quit |
| WindowServer watchdog | `WatchdogSec=20s`, plus a stall thread armed by the event loop only while it is working | Thread asks the reporter at 10 s, so stacks are taken before the abort at 20 s. Idle stays at zero extra wakeups |
| Kill code `0x8badf00d` | `SIGQUIT` from Force Quit, `SIGABRT` from the watchdog | `termination` field: who ended it and why |
| `.gpuRestart` | Compositor's device-lost path; `devcoredump`; kernel log | Fault from the compositor; attach `devcd` data when present |
| Display failures | Compositor's failed-frame and restart path | Fault at the 3 s renderer reset, not per failed frame |
| Panic through NVRAM | `pstore` (ramoops or efi-pstore), `systemd-pstore` | Use as `bande-crash-reports` does. To be tested in KVM; whether the mini's boot chain keeps it is an owner check |
| Panic stackshot | `kdump` | Not used: reserves memory and needs a second kernel on hardware where it is unproven. Stays an option |
| Unexplained restart | Journal `--boot=-1`, no `shutdown.target` | Use as `bande-crash-reports` does |
| Kernel oops, hung tasks, lockups | `/dev/kmsg` watcher, `hung_task` | Small root service blocked on kmsg; wakes on kernel messages only |
| Jetsam report | Kernel OOM line plus top processes by name and size, PSI | Copy the table, names only |
| Failed sleep or wake | `systemd-sleep` and kernel log | Fault from kmsg and the sleep unit result |
| Failed boot | systemd boot assessment (`fix-update-safety`) | Fault at next boot, when the entry counter shows failure |
| Failed unit | `OnFailure=` drop-in for `bande-.service`, greetd | Event-driven, no bus or journal follower |
| Stuck network mount | kmsg, D-state tasks in `cifs`, `nfs`, `fuse`; server identity hashed | Report only a stall with stuck stacks. A server going offline is counted |
| Metrics | None | **Counters only** for expected events: server offline, wrong password, Wi-Fi drop |
| `sysdiagnose` | `bande-diagnose` | Larger bundle on request, same scrubbing, read before sending |
| `crashReporterKey` | Random install id in the spool | Copy. Regenerated by a reinstall |
| Reports readable before sending | Settings ▸ Privacy & Security ▸ Diagnostics | Copy |
| One month's retention | Spool quotas | Copy: 30 days, at most 200 reports and 256 MiB |
| Path placeholders, redaction | Scrubber before anything is stored | Stricter than macOS, see below |

### What is not copied

- **Hundreds of MB of persisted logs.** Our journal already exists; faults take only warnings and above around the fault.
- **A kernel panic when the compositor hangs.** We end the session, not the kernel.
- **Continuous tracing (`tailspin`, persisted signposts).** Cost in normal use, against the rule of zero work when idle.
- **Spindump's 10 s at 10 ms of every process.** Three snapshots of the stuck process are enough, and sampling a stressed machine makes it worse.
- **Daily metrics (MetricKit), differential privacy.** No analytics, only fault reports and counts of expected events.
- **Stackshot in panics, `kdump`.** See the table.
- **Modal crash dialogs.** The existing notification stays.
- **Sharing with third-party developers.**

## Design

### The experience

- Nothing appears in normal use. Nothing is written in normal use.
- Settings ▸ Privacy & Security ▸ Diagnostics lists local reports. Each can be read in full, copied, deleted or sent now, with sending on or off. A switch turns automatic sending on, off by default.
- With no server configured the list still works and copies stay local.

### Parts

- **`bande-reporter`** (`crates/services/reporter`), no toolkit or desktop dependencies, also usable in Recovery. Root. One binary:
  - `send`: started by a `.path` unit while `outbox/` is non-empty; exits when it is empty, so idle means no process.
  - `serve`: socket-activated on `/run/bande/report.sock`. Takes faults from session processes, captures stacks, lists and deletes for Settings. Exits when idle.
  - `watch`: blocks on `/dev/kmsg`.
- **`bande-log`** gains the ring, the panic record and small calls to report a fault or count an expected event.
- **`bande-diagnose`**: the larger bundle.

### Spool (`/var/lib/bande/reports`, root only, mode 0700)

- `reports/<id>/`: `manifest.json` and attachments. Written in `tmp/`, each file fsynced, renamed, parent fsynced, before anything else happens.
- `outbox/<id>`: an empty marker, added only if sending is on. A report waits here across power cuts.
- Settings never reads the spool. It asks `serve`, which checks the peer's uid: administrators see all reports, others their own, as macOS does.
- A repeat of a signature within a day increments a count in the existing report rather than storing a new one.
- Quotas: 30 days, 200 reports, 256 MiB, 16 MiB per report. Oldest sent go first, then oldest unsent.

### Sending

`POST /v1/reports`, multipart with gzip on the whole body, the id as idempotency key. No polling: an attempt runs when the service starts and on rtnetlink route or address events, and otherwise waits for a deadline.

| Answer | What happens |
|---|---|
| 201, 200 | Done, marker removed, local copy kept and marked sent |
| 400 | Never retried; marked rejected |
| 401 | Waits for the config file to change or the next boot |
| 413 | Largest attachments dropped and named in the manifest, then sent again |
| 429, 503 | Waits for `Retry-After` |
| 507, 5xx, no network | Backoff 1 min, doubling to 6 h, or at once when the network changes |

Config is `/etc/bande/reports.conf`, root only: endpoint, submit key, `send = on`. Never in a package or a public repo.

### Signature

SHA-256 of kind and the top five meaningful frames of the faulting thread (for a hang, the main thread), or of kind and panic location if there is one. It leaves out panic and signal machinery, Rust's hash suffixes, addresses, paths and line numbers. Symbol names are used when known and module plus offset otherwise, with build ids in their own field so a rebuild keeps the signature whenever names are available.

### What each bundle carries

Kind, signature, id, time, OS and package versions, build ids, install id, arch. Plus: hardware model, kernel, Mesa, firmware, displays and scales, memory and PSI, uptime, session age, style and key settings, last action names, ring contents, journal lines at warning and above around the fault, failed units, and counters.

### What is never captured

File contents, typed text, window titles, clipboard, stack or heap memory (unless the owner's dev config asks). Home path, username, hostname, addresses, SSIDs and serial numbers are scrubbed from every text before storing, and so are file names outside Bande's own, and quoted text in log lines other than short identifiers. Processes that hold passwords stay undumpable: panic text only. `docs/architecture/fault-reports.md` will state this in full.

## Stages

1. `fault-reports-reporter`: spool, sender, config, quotas, signature.
2. `fault-reports-capture`: ring, manifest from crash records, hang capture, kmsg, failed units, stall thread, session facts, scrubbing, `bande-diagnose`. Builds on `bande-crash-reports` once that is merged.
3. `fault-reports-settings`: the Diagnostics pane; About ▸ Crash Reports moves here.
4. Faults caused on purpose in a VM, power cut, offline period, an hour idle and busy, cost measured.
