# Files reach other devices

Delete each stage when it ships; delete the file when the list is empty.

## What the user experiences

Go ▸ Connect to Server (Ctrl+K) takes `smb://nas`, `sftp://host` or `https://host/dav`. A server with
several shares offers a list. Passwords are asked once, kept in the Bande keychain if the user says so
and never asked again; a server marked Reconnect at Login comes back by itself after login. Every
connected server, phone and camera is an ordinary folder under Locations in the sidebar, the desktop
and the file chooser, with an eject button, and every other app can open its files. Servers on the network
appear under Network in the sidebar with one click to connect. A phone on USB appears when it is
plugged in and unlocked. When a connection drops Files keeps working: the folder shows "server not
responding" within seconds, with Try Again and Disconnect, and the mount heals itself when the network
returns. Settings ▸ Sharing has File Sharing and Remote Login switches. A Share menu sends files to Mail
or a connected place.

## Design

**Everything is a real mount**, so Files' copy, trash, thumbnails and every other app work unchanged.
All mounts sit in `$XDG_RUNTIME_DIR/bande/volumes/<name>`, a new root in `bande_volumes::media_roots`;
Files, the desktop and the file chooser already follow mountinfo for those roots. The session service
`bande-automount` owns them (`org.bande.Mounts`): connect, disconnect, keychain, reconnecting and the
device watch, so mounts outlive Files and every client sees the same state.

| Protocol | Tool (Debian) | Why |
| --- | --- | --- |
| SMB | kernel `cifs`, `cifs-utils`; `smbclient -L` to list shares | The reference client: SMB3 encryption, Kerberos, leases, kernel reconnect. Mounting needs root: a small `pkexec` helper, polkit `allow_active`, which pins the mount point and forces `nosuid,nodev,soft,uid=` |
| SFTP | `sshfs` | Uses the user's `~/.ssh` (config, keys, agent, known_hosts). `SSH_ASKPASS` to the service brings host keys and passwords to Files |
| WebDAV | `davfs2` through the same helper | Debian's only WebDAV mount. `delay_upload 0` so a finished copy is on the server |
| MTP | `go-mtpfs` | Debian has dropped `jmtpfs` and `simple-mtpfs`; `aft-mtp-mount` drags in Qt. No libfuse or GUI dependency |
| PTP | `gphotofs` | libgphoto2's own |

Ruled out: GVfs (GNOME, and not a real path for other apps); `rclone` (not in Debian testing); `smbnetfs` (gone);
own FUSE daemons in Rust (re-implementing protocols Debian maintains).

**Discovery** is `systemd-resolved`, already in the image: `io.systemd.Resolve.BrowseServices` over varlink
streams `_smb._tcp`, `_sftp-ssh._tcp`, `_webdav._tcp`, `_webdavs._tcp`, event-driven, no daemon of ours. The
package enables mDNS with `resolved.conf.d` and NetworkManager `connection.mdns=2`, which also makes
`nas.local` resolve everywhere. Windows PCs announce through WS-Discovery instead, not mDNS: a small native
probe later. Avahi stays out.

**Sharing out** is Samba, installed on demand through PackageKit as printing is: per-user `net usershare`,
a `.dnssd` file so resolved announces it, the Samba password set from the pane. Remote Login is `ssh.service`
through systemd's own polkit-guarded calls.

**Dropped connections.** Measured with a cut link: `cifs` with `soft,echo_interval=5` fails calls with an error
in 10–22 s and reconnects by itself; `sshfs` with `reconnect,ServerAliveInterval=5` in 1–15 s; `davfs2` in up to
30 s. So a call can stall for half a minute and Files must never make one on its UI thread: folders load on a
worker with a short wait for local speed, then show a loading state; stat calls on the UI thread are moved to
workers or skipped for these mounts. Disconnect runs in the service and never waits on Files.

**Nearby transfer.** AirDrop itself is out of reach: AWDL needs Wi-Fi monitor mode, which `brcmfmac` lacks here,
and Apple's protocol is closed. The realistic form is the open LocalSend protocol (multicast, then HTTPS on the
LAN) that phones and other computers already run. Its own stage, last.

## Stages

1. Connect to Server: helper, service, dialog, keychain, sidebar, eject, reconnect at login.
2. Files never blocks on a mount; "not responding" state with Try Again and Disconnect.
3. Phones and cameras.
4. Discovery in the sidebar.
5. Settings ▸ Sharing.
6. Share menu.
7. Nearby transfer (LocalSend protocol).
