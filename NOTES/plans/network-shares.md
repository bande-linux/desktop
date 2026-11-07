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

- **NFS:** `softerr,softreval,timeo=30,retrans=2,retry=0,nosuid,nodev`; NFSv4 first (kernel negotiates 4.2 down), then `vers=3,nolock`: `mount.nfs` asks for `rpc.statd` with `local_lock=all`, and `nolock` keeps locks local without it, so no daemon has to run. `hard` is what makes Linux NFS famous for freezing, and `intr` has been ignored since 2.6.25. The price: `nfs(5)` warns a soft timeout can cause silent corruption when a write times out unseen. `softerr` returns ETIMEDOUT rather than EIO, and Files' copies fsync and check before renaming a hidden partial file to the real name, so a timed-out write is an error and not a false success. Programs that ignore `close()` errors can still lose data, as on macOS's soft mounts. `softreval` keeps cached attributes answering while the server is gone. NFSv3 locks stay local because NLM and `statd` are a hang of their own.
- **SMB:** `soft,echo_interval=5,actimeo=1,nosuid,nodev`. Measured: calls fail in 10–31 s and the kernel reconnects by itself. Same bound as macOS's 30 s.

## The guarantee

1. Nothing on a UI thread touches a connected volume, across the whole desktop. Paths are recognised by prefix (`bande_volumes::on_connected_volume`), which costs no system call.
2. Workers may stall, for at most the soft timeout. One stalled worker never holds up other work, and quitting never waits for it.
3. Forced unmount by `umount2(MNT_FORCE)`, then `MNT_DETACH|MNT_FORCE|UMOUNT_NOFOLLOW`: unlike `umount(8)` it does not stat the mount first, and it aborts stuck requests, which frees D-state callers. Disconnect, the dead timer, logout and shutdown all use it.

Health: while a mount exists the service runs one statfs in a disposable thread and a TCP connect, every 8 s and at once on wake or a network change, with 4 s to answer. This is a justified deadline and stops with the last mount. Two misses in a row raise the alert: 7–11 s measured over six cuts, 16 s at worst. The trade-off of being this quick: an 8 s blackout raises nothing, but a link with a 3 s round trip and loss shows the alert on and off (27 of 240 s measured); it clears by itself and the dead timer only runs while misses are unbroken.

WebDAV has no dead timer: `davfs2` keeps unsent files in its cache and sends them when the server returns, and a forced unmount would lose them. Its calls end in at most about 30 s by themselves. **Known limit:** `davfs2` finishes a copy into its cache before uploading, so Files can report success for a file the server later refuses (seen with a server answering 500). SMB, NFS and SFTP do not have this limit.

## Nothing listens

`nfs-common` is not installed until the first NFS connection, like Samba. Installing it starts `rpcbind` on port 111, so the service stops and disables `rpcbind` straight after. NFSv4 and NFSv3 with `nolock` both mount with nothing listening (measured in a clean VM).

## Audit, row by row

Every place the code audit found a path on a mount reaching a blocking call on a UI thread or an only worker. State: **guarded** (no call on these paths), **off** (runs on a thread of its own), **reduced** (safe, with less function on a server).

| Where | Trigger | State |
| --- | --- | --- |
| chooser `model.rs` list (open, hidden toggle) | Open a server folder | off: listing thread, result by inbox |
| chooser `model.rs` is_dir at start | Remembered folder | guarded |
| chooser `model.rs` accept, save `is_dir`, `exists` | Accept, Save | guarded: uses the listed entries |
| chooser `view.rs` select, activate, Alt+Down | Row actions | guarded: uses the listed entries |
| chooser `view.rs` go to typed path | Go to folder | guarded |
| chooser `view.rs` create_dir | New Folder | off |
| Files `paint.rs` mime for thumbnails | Icons, Gallery paint | guarded: name-based mime |
| search `recents.rs` is_file, from Files | Recents | guarded |
| search `recents.rs` is_file, from launcher | Recent documents | guarded |
| Files `search.rs` metadata | Kind or date filter | reduced: hits on servers pass the filter |
| Files `view.rs` open, `.desktop` check | Open a file | guarded: name-based mime; launchers on servers are not trusted |
| Files `quick_look.rs` | Space | guarded |
| Files `context.rs` single file, menus | Menu rebuild | reduced: Print off for server files |
| Files `context.rs` apps for selection | Right click | guarded |
| Files `view.rs` inspector mime | Get Info | guarded |
| Files `open_with.rs` | Open With | guarded |
| `same_volume`, from Files drag | Drag motion | guarded |
| `same_volume`, from desktop drag | Drag motion | guarded |
| Files `tabs.rs` | Resume | guarded |
| Files `settings.rs` | New window folder | guarded |
| Files `trouble.rs` mount_point | No-space dialog | guarded |
| Files rename | Inline rename | off |
| Files new folder, new document, alias | Commands | off |
| Files batch rename plan | Sheet validation | guarded: rename itself refuses to replace |
| Files batch rename apply | Apply | off |
| Files `inspector.rs` extension xattr, permissions | Get Info controls | off |
| chooser `print.rs`, `pdf/client.rs` drop wait | Close a PDF | off: reaped on a thread |
| `media/player.rs` drop wait | Close media | off: reaped on a thread |
| lock `backdrop.rs` redraw | Lock wallpaper | guarded: a wallpaper chosen on a server is copied here first |
| desktop `client.rs` redraw, `wallpaper/cache.rs` | Wallpaper redraw | guarded: same |
| desktop `places.rs` volume stat | Relist | guarded |
| launcher `client.rs` default app | Open recent | guarded |
| launcher `client.rs` preview worker | Preview recent | reduced: no preview for server files |
| Files `loader.rs` | Previews | off: one thread per server request |
| `quick-look/loader.rs` | Quick Look | off: one thread per server request |
| `thumbnails/service.rs` | Thumbnails | off: a lane of its own |
| `search/index.rs` watch | Search root | guarded |
| `search/index.rs` read_dir | Search root | off: that search's own worker |
| `xdg` open, from Terminal | Click a link | off |
| `xdg` open, from Screenshot | Open recording | off |
| `xdg/apps.rs` launcher trust and read | `.desktop` on a server | guarded |
| desktop `client.rs` symlink rows | Symlink into a mount | guarded: symlinks are followed by text |
| Files `sidebar.rs` user dir | User dir on a mount | guarded |
| `app/print.rs` file document | Print | reduced: off for server files |
| `xdg/launch.rs` `Path=` | `Path=` on a mount | reduced: starts without that folder |
| Terminal `session.rs` chdir | Start in a server folder | off: the shell does `cd` |
| Terminal `main.rs` is_dir | Working directory | guarded |
| Terminal `terminal.rs` link check | Hover a path | guarded |
| TextEdit canonicalize | Open | guarded |
| TextEdit exists | Restore | guarded |
| Preview `files.rs` list | Next, previous | reduced: off on servers |
| Preview `info.rs` metadata | Info | reduced: no size on servers |
| Disks `view.rs` image size | Restore | reduced: size check skipped |
| Settings `pictures.rs` thumbnail | Wallpaper list | guarded: same as redraw |

## Left

- The rows marked reduced: run them off the UI thread with full function.
- Wake from sleep with a dead mount: entering sleep is measured; waking needs a real machine.
- Shutdown stays bounded by `cifs`'s own teardown (about 16 s measured), which systemd's mount job waits for. Windows PCs under Network.
