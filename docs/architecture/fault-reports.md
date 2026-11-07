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

- **`bande-crash-reports@.service`** (no network): after each `systemd-coredump@`, at boot and at login. Crashes, panics of the programs that hold passwords, failed services, and what the last boot left: pstore, a restart with no clean shutdown, partial hang snapshots.
- **`bande-reporter.socket`** starts `bande-reporter serve` when something asks, and it exits after 30 s idle. Session programs report faults through `bande_log::fault`; it checks the sender owns the process it names. It also answers Settings.
- **`bande-reporter-watch.service`** blocks on `/dev/kmsg` and wakes only on kernel messages: oops, kernel warnings, hung tasks, lockups, OOM, GPU and display errors, failed suspend, disk errors. A server not responding or a Wi-Fi drop is counted, not reported.
- **A hang:** the compositor's ping (10 s unanswered: `app-hang`, `shell-stall`), Force Quit (`force-quit`, before `SIGQUIT`), and for the compositor a stall thread (`compositor-stall` at 10 s, before the watchdog's abort at 20 s). The thread sleeps until the event loop wakes, so an idle desktop costs nothing. Capture takes the kernel stacks of every thread and fsyncs that first snapshot, then two more 1 s apart, then user stacks with `eu-stack`. A hang inside `cifs`, `nfs` or `fuse` becomes `mount-stall`, with the protocol and a hash of the server.

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

## Signature

`kind`, the panic location without its line, and the top five frames that are not panic or signal machinery, hashed. Rust's symbol hashes are removed. Build ids are kept in their own field, so a rebuild keeps the signature whenever names are available; a frame with no name falls back to module and offset.

## Commands

`bande-reporter [--spool DIR] [--config FILE] COMMAND`: `send`, `serve`, `watch`, `collect-failures`, `list`, `store MANIFEST [ATTACHMENT...]`, `queue ID`, `delete ID`. It depends on no desktop or toolkit library, so it also runs from Recovery with `--spool` pointing at the mounted system.
