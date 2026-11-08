# Fault reports

How macOS captures and triages diagnostics, and what Bande copies. What is built is described in [the architecture doc](../../docs/architecture/fault-reports.md).

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
| `spindump` on force quit or hang | Kernel stacks first, then `eu-stack -p` every 500 ms for up to 10 s, merged into a call tree, plus the process the main thread waits on | Copy, only during a confirmed hang. Kernel stacks come first because a thread stuck in D state can block a ptrace attach |
| App hang, 250 ms / 2 s / watchdog | Compositor ping round trip after input | Counter over 250 ms, wait cursor at 2 s, report at 10 s, finished with the hang's length and whether it recovered |
| WindowServer watchdog | `WatchdogSec=20s`, plus a stall thread armed by the event loop only while it is working | Thread asks the reporter at 6 s, so stacks are taken before the watchdog abort, which can come 10 s into a hang. Idle stays at zero extra wakeups |
| Kill code `0x8badf00d` | `SIGQUIT` from Force Quit, `SIGABRT` from the watchdog | `termination` field: who ended it and why |
| `.gpuRestart` | Compositor's device-lost path; `devcoredump`; kernel log | Fault from the compositor; attach `devcd` data when present |
| Display failures | Compositor's failed-frame and restart path | Fault at the 3 s renderer reset, not per failed frame |
| Panic through NVRAM | `pstore` (ramoops or efi-pstore), `systemd-pstore` | Use as `bande-crash-reports` does. To be tested in KVM; whether the mini's boot chain keeps it is an owner check |
| Panic stackshot | `kdump` | Not used: reserves memory and needs a second kernel on hardware where it is unproven. Stays an option |
| Unexplained restart | Journal `--boot=-1`, no `shutdown.target` | Use as `bande-crash-reports` does |
| Kernel oops, hung tasks, lockups | `/dev/kmsg` watcher, `hung_task` | Small root service blocked on kmsg; wakes on kernel messages only |
| Jetsam report | Kernel OOM kills, systemd-oomd kills, PSI trigger | `oom` and `memory-pressure` reports: top processes and units with resident and swap, the limit hit |
| `cpu_resource`, `wakeups_resource`, `disk_writes_resource` | cgroup `cpu.stat`, `io.stat`, context switches every 10 minutes | Copy, coalesced, one report per program per day |
| Failed sleep or wake | `systemd-sleep` and kernel log | Fault from kmsg and the sleep unit result |
| Failed boot | systemd boot assessment (`fix-update-safety`) | Fault at next boot, when the entry counter shows failure |
| Failed unit | `OnFailure=` drop-in for `bande-.service`, greetd | Event-driven, no bus or journal follower |
| Stuck network mount | kmsg, D-state tasks in `cifs`, `nfs`, `fuse`; server identity hashed | Report only a stall with stuck stacks. A server going offline is counted |
| Metrics | Daily check-in while sending is on: hours used, sessions, boots, versions | Only for fault rates per version; counters for expected events |
| `sysdiagnose` | `bande-diagnose` | Larger bundle on request, same scrubbing, read before sending |
| `crashReporterKey` | Random install id in the spool | Copy. Regenerated by a reinstall |
| Reports readable before sending | Settings ▸ Privacy & Security ▸ Diagnostics | Copy |
| One month's retention | Spool quotas | Copy: 30 days, at most 200 reports and 256 MiB |
| Path placeholders, redaction | Scrubber before anything is stored | Stricter than macOS, see below |

### What is not copied

- **Hundreds of MB of persisted logs.** Our journal already exists; faults take only warnings and above around the fault.
- **A kernel panic when the compositor hangs.** We end the session, not the kernel.
- **Continuous tracing (`tailspin`, persisted signposts).** Cost in normal use, against the rule of zero work when idle.
- **Spindump's 10 ms sampling of every process.** Only the hung process and what it waits on, every 500 ms.
- **Analytics beyond the check-in, differential privacy.**
- **Stackshot in panics, `kdump`.** See the table.
- **Modal crash dialogs.** The existing notification stays.
- **Sharing with third-party developers.**
