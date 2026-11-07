# Fault reports

`bande-reporter` keeps a small report of each fault and sends it only if sending is on. Design and how macOS compares: [the plan](../../NOTES/plans/fault-reports.md).

## Spool

`/var/lib/bande/reports`, root only (`0700`):

- `reports/<id>/`: `manifest.json`, attachments and `state.json` (occurrences, sent or rejected). A report is written in `tmp/`, every file and folder fsynced, then renamed into place, so a power cut leaves a whole report or none.
- `outbox/<id>`: an empty marker for a report waiting to be sent.
- `machine`: a random install id. It says nothing about the computer, and a reinstall replaces it.

Kept: 30 days, at most 200 reports and 256 MiB, 16 MiB a report. Sent and rejected reports go first, then the oldest. With under 512 MiB free on the disk a report keeps its manifest and drops its attachments, and names them in `attachments_dropped`. A repeat of a signature within an hour, or within a day while the first is unsent, only adds to `occurrences`.

## Sending

Nothing leaves the computer unless a server is configured and sending is on, or someone sends one report by hand. Shipped images configure no server.

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

## Signature

`kind`, the panic location without its line, and the top five frames that are not panic or signal machinery, hashed. Rust's symbol hashes are removed. Build ids are kept in their own field, so a rebuild keeps the signature whenever names are available; a frame with no name falls back to module and offset.

## Commands

`bande-reporter [--spool DIR] [--config FILE] COMMAND`: `send`, `list`, `store MANIFEST [ATTACHMENT...]`, `queue ID`, `delete ID`. It depends on no desktop or toolkit library, so it also runs from Recovery with `--spool` pointing at the mounted system.
