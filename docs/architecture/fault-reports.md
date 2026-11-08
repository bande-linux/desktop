# Fault reports

`bande-reporter` keeps a small report of each fault and sends it only if sending is on. How macOS compares: [the plan](../../NOTES/plans/fault-reports.md).

## Spool

`/var/lib/bande/reports`, root only (`0700`):

- `reports/<id>/`: `manifest.json`, attachments and `state.json` (occurrences, sent or rejected). A report is written in `tmp/`, every file and folder fsynced, then renamed into place, so a power cut leaves a whole report or none.
- `outbox/<id>`: an empty marker for a report waiting to be sent.
- `machine`: a random install id. It says nothing about the computer, and a reinstall replaces it.

Kept: 30 days, at most 200 reports and 256 MiB, 16 MiB a report. Sent and rejected reports go first, then the oldest. With under 512 MiB free on the disk a report keeps its manifest and drops its attachments, and names them in `attachments_dropped`. A repeat of a signature within an hour, or within a day while the first is unsent, only adds to `occurrences`; if the first was already sent, the manifest goes again without attachments so the server keeps count.

## Sending

Nothing leaves the computer unless a server is configured and sending is on, or someone sends one report by hand. Shipped images configure no server. Settings ▸ Privacy & Security ▸ Diagnostics shows which applies.

`/etc/bande/reports.conf`, owned by root, mode `0600` (any other mode is refused, because it holds a key):

```
endpoint = https://host:port
key = <submit key>
send = on
ca = /path/to/ca.pem
```

`ca` is optional; without it the system's roots are used. The file never goes in a package or a public repo.

`bande-reporter-send.path` starts `bande-reporter-send.service` while `outbox/` has anything in it, and the service exits when `outbox/` is empty, so nothing runs between faults. While reports wait it sleeps in `poll`. It wakes on a deadline, on a network change (rtnetlink) or when the settings folder changes.

`POST <endpoint>/v1/reports` with `Authorization: Bearer <key>`: `multipart/form-data` with `Content-Encoding: gzip` on the whole body, a part named `manifest` (`application/json`) first, then parts named `attachment`, each with a `filename`. The report's id makes a resend harmless.

| Answer | What happens |
|---|---|
| 201, 200 | Marked sent; the local copy stays until the limits above remove it |
| 400 | Marked rejected, never retried |
| 401, 403 | Waits for the settings to change |
| 413 | The largest attachment is left out and named in `attachments_dropped`, then sent again; with none left, rejected |
| 429, 5xx | Waits for `Retry-After`, or else for the backoff |
| No connection | Backoff from 1 minute doubling to 6 hours, cut short by a network change |

## What makes a report

- **`bande-crash-reports@.service`** (no network): after each `systemd-coredump@`, at boot and at login. Crashes, panics of the programs that hold passwords, failed services, and what the last boot left: pstore, a restart with no clean shutdown, partial hang snapshots. A failing `bande-*` service starts its own instance through `OnFailure=bande-crash-reports@%N.service` (user services: `report-bande-failure@%N.service`, which asks `serve` for `bande-crash-reports@user-%N.service`), so services failing together are each collected.
- **`bande-reporter.socket`** starts `bande-reporter serve` when something asks, and it exits after 30 s idle. Session programs report faults through `bande_log::fault`; it checks the sender owns the process it names.
- **Only installed programs:** faults, counters, rings, actions and session facts come only from programs under `/usr` (not `/usr/local`), and only about them; `bande_log` stays silent in anything else, such as test binaries in `target/`, `serve` refuses them, and crash reports skip their coredumps and panics. It also answers Settings.
- **`bande-reporter-watch.service`** blocks on `/dev/kmsg` and wakes only on kernel messages: oops, kernel warnings, hung tasks, lockups, OOM, GPU and display errors, failed suspend, disk errors. A server not responding or a Wi-Fi drop is counted, not reported.
- **A hang:** the compositor pings a program after input and times the answer. Over 250 ms it counts `microhang:<app id>`; at 2 s the wait cursor shows and the program is Not Responding; at 10 s it asks for a report (`app-hang`, `shell-stall`). When the program answers or goes away, the compositor tells `serve`, which finishes the report with `hang_seconds` and `recovered` (it waits up to 2 minutes for that). Also Force Quit (`force-quit`, before `SIGQUIT`), and for the compositor a stall thread (`compositor-stall` at 6 s: the watchdog is pinged every 10 s and aborts 20 s after the last ping, so its abort can come 10 s into a hang). The thread sleeps until the event loop wakes, so an idle desktop costs nothing. Capture takes the kernel stacks of every thread and fsyncs that first snapshot, then samples user stacks with `eu-stack` every 500 ms: up to 10 s for a pinged hang, ending early when it answers, 3 s for Force Quit and the compositor. The samples merge into `hang-tree.txt`, and the most frequent main-thread stack becomes the report's threads. If the main thread waits in a read or a one-descriptor poll on a socket or pipe, the process at the other end (the socket's peer credentials, or the pipe's writer) is `blocked_on`, sampled the same way unless it is the compositor, which answers pings and is not stopped for this. A hang inside `cifs`, `nfs` or `fuse` becomes `mount-stall`, with the protocol and a hash of the server. A hang (not Force Quit) whose main thread sat in `epoll_wait` through every sample, with that epoll set ready neither at the start nor at the end, was idle, not hung: it is dropped and only counted as `idle-hang-dropped`, which rides along with the next report. Crashes, panics, OOM kills and failed services have no such guard.

## What a report carries

- **Always:** kind, signature, time, boot ids, program, threads with frames, images with build ids, package versions, kernel, firmware, model, displays and scales, memory, PSI, uptime, failed units, counters of expected events, and the journal at warning and above around the fault.
- **From the session:** style, displays and session start, written to `$XDG_RUNTIME_DIR/bande/session.json` when they change, and the last 32 action names. The ring of each program: `$XDG_RUNTIME_DIR/bande/ring/<program>`, 32 KiB of tmpfs, debug and above from Bande's code, never written to disk unless a fault copies it. Programs that hold passwords have no ring.
- **A crash:** signal, fault address, registers.
- **Dev config only:** `stack_memory = on` adds `stack-memory.core`, an ELF core of the notes and 64 KiB of each thread's stack. It holds raw memory, the notes hold the command line and mapped file names, and it is not scrubbed. `signature_tag = x` prefixes signatures, to mark test reports. Full core dumps stay with `systemd-coredump` and are never sent.

## What is never captured

File contents, typed text, window titles and the clipboard are not read. Every text is scrubbed before it is stored: home paths, account and real names, hostname, addresses, emails, URLs, SSIDs of saved networks, serial numbers and machine id, paths outside the system's own folders, and quoted text longer than a short identifier. Scrubbing is by pattern, so read a report before sending if it matters; a sentence in an unquoted log line from another program can pass. Text you type in Send Report is sent as typed.

## By hand, and after a freeze

Settings ▸ General ▸ About ▸ Send Report… or `bande-diagnose [--send] [--describe TEXT]`. It makes a `manual` report with this boot's and the previous boot's logs, pstore, core dump list, processes, disks, modules and anything a hang left behind. Settings shows it first, with Send Now, Copy, Delete, and Save to File when no server is configured. `bande-diagnose` works without the service as root, so it also runs from a text console or Recovery; with no server it saves a text file in the home folder.

During a total freeze nothing runs. What survives: the journal up to its last fsync (`SyncIntervalSec=15s`, at once for critical messages), hang snapshots already fsynced, and pstore if the kernel panicked. A hardware watchdog reset leaves no pstore record.

## Protocol

The contract with the log server. Every field added after protocol v1 is optional, so an older client or server keeps working.

### Manifest

- `desktop_version`: the Debian version of the package that owns the faulting program, else of `bande-desktop`. `os_version` stays the system's name and kernel.
- `threads`: `[{id, faulting, frames}]`, one thread `faulting: true` (the crashed one, or the main thread of a hang). Each frame is `{symbol?, module, build_id?, offset}`: `module` is the file name of the ELF object, `build_id` its GNU build id in hex, `offset` the program counter minus the module's load address, as elfutils reports it (one less than the return address for every frame but the innermost, so it points into the call). Address randomisation leaves it unchanged, and with the build id it symbolicates. Frames with no module are left out.
- `signature_text`: an extra first frame for the signature (a signal name, a failed unit and its result, kernel frames), scrubbed, as hashed.
- `hang_seconds`: on hangs, whole seconds from the first unanswered ping to the answer, or to the report if it never came.
- `recovered`: on hangs, `true` if the program answered again, `false` if it was closed, killed or still stuck when the report was made.
- `blocked_on`: on hangs, `{program, via, frames}` for the process the main thread waits on. `via` is `socket` or `pipe` (for a D-Bus call that is the bus daemon); `frames` as above, at most 16, of that process's most frequent main-thread stack, empty if it was not sampled.
- `termination`: on `crash`, `force-quit`, `oom` and `not-responding`, who ended the process: `watchdog`, `force-quit`, `oomd`, `kernel-oom`, `stop-timeout` or `logout-timeout`. Missing when the process ended by itself.

### `hang-tree.txt`

Sampled stacks of a hang, merged. A header line, then one block per thread:

```
hang-tree <program> pid <pid> samples <n> interval-ms <ms>
thread <tid> <name>
<count> <symbol or ?> <module>+0x<offset>
  <count> <symbol or ?> <module>+0x<offset>
```

The symbol may hold spaces; the last field is always the module and offset. Outermost frame first; each level is indented two more spaces and holds the number of samples that passed through it, children in falling count. Identical stacks fall into one path. A second header, `blocked-on <program> pid <pid> via <via> samples <n> interval-ms <ms>`, starts the blocks of the process it waited on.

### Check-in

`POST <endpoint>/v1/checkins`, the same key, JSON: `{machine, os_version, desktop_version, arch, active_hours, sessions, boots, day}`. `day` is the UTC date (`YYYY-MM-DD`) it covers, `active_hours` a number with one decimal, `sessions` and `boots` counts. At most one a day, only while sending is on. 200 or 201: done; 404: the server does not know check-ins yet, tried again the next day; anything else follows the table above.

### Symbols

`PUT <endpoint>/v1/symbols/<build-id>` with `Authorization: Bearer <LOGS_SYMBOLS_KEY>` (a key of its own, not the submit key) and the `.debug` file as `application/octet-stream`; its debug sections are zlib-compressed (`SHF_COMPRESSED`). `ci/release/upload-symbols.sh <deb> <endpoint>` sends every `.debug` of a `bande-debug-symbols` deb (2xx is success). The owner's machines run it, since GitHub cannot reach the server: `send-symbols` on the mini after `rebuild-desktop` installs a build and after a release tag's run is green, with `LOGS_SYMBOLS_KEY` from `~/bande-linux/logs-keys.env` or `/etc/bande/reports.conf`.

### Signature

SHA-256 in lowercase hex of a text built from the manifest:

1. `kind`, a space, and `program` (`-` without one).
2. With `panic_location`: a newline and the location up to its first `:`.
3. Frames: `signature_text` as a named frame, then the faulting thread's frames, innermost first. Leave out named frames whose name starts with one of `__GI_`, `__libc_`, `__pthread_`, `__restore_rt`, `__rust_`, `__kernel_rt_sigreturn`, `__futex`, `__ppoll`, `__poll`, `abort`, `core::option::`, `core::panicking::`, `core::result::`, `epoll_pwait`, `gsignal`, `pthread_kill`, `raise`, `rust_begin_unwind`, `rust_panic`, `std::panicking::`, `std::sys::`, `syscall`, and unnamed frames with an empty module. Take the first five.
4. For each: a newline and the name without a Rust hash suffix (`::h` and 16 hex digits), or, unnamed, `<module>+0x<offset in lowercase hex>`.

`signature_tag = x` in the config gives `x-<hash>`. Test vectors (also in `signature.rs`):

| kind program | panic_location | frames (name or module+offset) | signature |
|---|---|---|---|
| `crash bande-shell` | | `SIGSEGV`, `__pthread_kill_implementation`, `bande_shell::bar::draw::h0123456789abcdef` | `e47d81b3f9358fa3b02eea676c835b04b772214c78eebc5a44d05ed1a98b647c` |
| `panic bande-files` | `crates/apps/files/src/view.rs:120:9` | none | `5ee7d8c5e2075213f84bc4fe57f6b8e579bf27c70933874a76de6dfd8d8d1883` |
| `app-hang bande-files` | | `bande-files+0x1a2b3`, (empty module)`+0x7fff`, `epoll_pwait`, `libc.so.6+0x0`, `a::one`, `a::two`, `a::three`, `a::four` | `5d8260dc938a5c566e60899298459d1769bdaec6a724eee84cd80f39492563f1` |

A rebuild keeps a signature whenever names are available; an unnamed frame changes with the build.

## Commands

`bande-reporter [--spool DIR] [--config FILE] COMMAND`: `send`, `serve`, `watch`, `collect-failures`, `list`, `store MANIFEST [ATTACHMENT...]`, `queue ID`, `delete ID`. It depends on no desktop or toolkit library, so it also runs from Recovery with `--spool` pointing at the mounted system.
