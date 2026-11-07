# Network shares that never hang

Delete each stage when it ships; delete the file when the list is empty. `devices.md` covers the rest of Connect to Server.

## How macOS behaves, and where Bande stands

| macOS | Source | Bande today | Target |
| --- | --- | --- | --- |
| A mount that stops answering is reported after 12 s, then every 30 s | `nfs.conf(5)` `initialdowndelay`, `nextdowndelay` | Files shows it only when a folder is opened | `bande-automount` watches each mount and says so after about 12 s |
| "Server connections interrupted" names the server, offers Disconnect, blocks nothing | Forum logs: `KernelEventAgent` on `VQ_NOTRESP`; no Apple document found | None | A notification with Disconnect, and a mark in the sidebar; gone when the server is back |
| Soft mounts fail calls after `retrans` timeouts; a soft mount unresponsive for 60 s is marked dead and forcibly unmounted | `mount_nfs(8)` `soft`, `deadtimeout` | Calls fail; the mount stays and every call stalls again | Same 60 s, then forced unmount, so every later call fails at once; reconnect at the same path |
| Soft and read-only NFS mounts lock locally | `mount_nfs(8)` `locallocks` | No NFS | Same for NFSv3 |
| NFS client rate-limits statfs | `nfs.conf(5)` `statfs_rate_limit` | statfs on the UI thread is skipped, so no free space on servers | Free space read off the UI thread, cached |
| `max_resp_timeout` 30 s, soft available, directory cache 30–60 s, change notifications | `nsmb.conf(5)`, `mount_smbfs(8)` | `soft`, `echo_interval=5`; no refresh | Same bound; visible folders refresh |
| Go ▸ Connect to Server, `nfs://` needs the export path, recent and favourite servers | Apple Mac Help `mchlp1140`, Indiana KB `aklh` | No NFS; recents; no favourites in the sidebar | `nfs://`, export list when the server gives one, remembered servers under Network |

Sources: <https://keith.github.io/xcode-man-pages/mount_nfs.8.html>, <https://keith.github.io/xcode-man-pages/nfs.conf.5.html>, <https://keith.github.io/xcode-man-pages/mount_smbfs.8.html>, <https://keith.github.io/xcode-man-pages/nsmb.conf.5.html>, <https://support.apple.com/ml-in/guide/mac-help/mchlp1140/10.13/mac/10.13>, <https://kb.iu.edu/d/aklh>, <https://discussions.apple.com/thread/5426061?page=1>, <https://community.jamf.com/t5/jamf-pro/losing-smb-shares-frequently/td-p/50316>, <https://man7.org/linux/man-pages/man5/nfs.5.html>.

## Mount options

- **NFS:** `softerr,softreval,timeo=30,retrans=2,retry=0,nosuid,nodev`; NFSv4 first (kernel negotiates 4.2 down), then `vers=3,local_lock=all`. `hard` is what makes Linux NFS famous for freezing, and `intr` has been ignored since 2.6.25. The price: `nfs(5)` warns a soft timeout can cause silent corruption when a write times out unseen. `softerr` returns ETIMEDOUT rather than EIO, and Files' copies fsync and check before renaming a hidden partial file to the real name, so a timed-out write is an error and not a false success. Programs that ignore `close()` errors can still lose data, as on macOS's soft mounts. `softreval` keeps cached attributes answering while the server is gone. NFSv3 locks stay local because NLM and `statd` are a hang of their own.
- **SMB:** `soft,echo_interval=5,actimeo=1,nosuid,nodev`. Measured: calls fail in 10–31 s and the kernel reconnects by itself. Same bound as macOS's 30 s.

## The guarantee

1. Nothing on a UI thread touches a connected volume, across the whole desktop. Paths are recognised by prefix (`bande_volumes::on_connected_volume`), which costs no system call.
2. Workers may stall, for at most the soft timeout. One stalled worker never holds up other work, and quitting never waits for it.
3. Forced unmount by `umount2(MNT_FORCE)`, then `MNT_DETACH|MNT_FORCE|UMOUNT_NOFOLLOW`: unlike `umount(8)` it does not stat the mount first, and it aborts stuck requests, which frees D-state callers. Disconnect, the dead timer, logout and shutdown all use it.

Health: while a mount exists the service runs one statfs in a disposable thread and a TCP connect, every 10 s and at once on wake or a network change. This is a justified deadline and stops with the last mount. Two misses mean unresponsive.

## Left

- Print a text or image file on a server (off there for now), Preview's next and previous there, and Disks' size check of a restore image on a server: run them off the UI thread.
- Wake from sleep with a dead mount: entering sleep is measured; waking needs a real machine.
- Shutdown stays bounded by `cifs`'s own teardown (about 16 s measured), which systemd's mount job waits for. Windows PCs under Network.
