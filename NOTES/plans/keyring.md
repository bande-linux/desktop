# Bande keyring (replaces gnome-keyring)

Owner decision: Bande ships its own Secret Service so gnome-keyring, libpam-gnome-keyring and the
GTK 3/4, gcr and pinentry they pull in can go.

## Prior art checked

- **gnome-keyring**: daemon owns `org.freedesktop.secrets`; `pam_gnome_keyring` stashes the
  password in `auth`, unlocks in `session` (`auto_start`), re-encrypts in `password`
  (`Password-Final`); it ships a pam-auth-update profile; v0 files in `~/.local/share/keyrings`;
  portal impl keeps one item per app (`app_id`) and writes its secret to the fd.
- **KeePassXC**: blocks a `SearchItems` on a locked database behind its unlock dialog; optional
  per-access confirmation.
- **oo7** (linux-credentials, MIT): client crate with the v1 file format (AES-128-CBC +
  HMAC-SHA256, PBKDF2-SHA256), the legacy v0 reader that imports a gnome-keyring
  `login.keyring`, and the dh-ietf1024-sha256-aes128-cbc-pkcs7 primitives. `oo7-daemon` is a
  tokio server with GNOME/Plasma prompters and gettext; `oo7-pam` sets a global tracing
  subscriber and spins a tokio runtime inside the PAM host process. Its v1 layout
  (`~/.local/share/keyrings/v1/<name>.keyring`) is also libsecret's file backend.

## Decisions

- **Reuse `oo7` as a plain dependency** (`=0.7.0-beta`, `async-std` + `native_crypto` +
  `unstable`): it gives the vetted on-disk format, the v0 import and the session crypto
  (RustCrypto `aes`, `cbc`, `hkdf`, `pbkdf2`, `sha2`, `num-bigint-dig`). Not `oo7-daemon`: its
  prompter speaks GNOME's/KDE's prompt protocols, it brings tokio, clap and system gettext, and
  its object model is not built to embed. Not `oo7-pam`: a PAM module must not install a global
  tracing subscriber or a tokio runtime in greetd/login/passwd; ours is ~200 lines of FFI with
  no threads.
- **Store**: `~/.local/share/keyrings/v1/<name>.keyring` (same layout as oo7-daemon). First run
  imports `~/.local/share/keyrings/login.keyring` (v0) with the login password and leaves the
  v0 file untouched, so going back to gnome-keyring still works. Collections: `login` (alias
  `default`) and `session` (memory only, alias `session`); `CreateCollection` adds more.
  Item paths are per-run counters; clients re-search.
- **Daemon** `bande-secrets` (crate `crates/services/secrets`, package `bande-secrets`):
  zbus object server on the async-io runtime zbus already uses; D-Bus activated
  (`org.bande.Secrets.service`, `Name=org.freedesktop.secrets`, `SystemdService=`) and
  socket-activated by the PAM socket; started with the session so it also hosts the
  NetworkManager agent. Idle = blocked in epoll, no timers.
- **Unlock at login** (pam_gnome_keyring's shape): `pam_bande_secrets.so` in the
  pam-auth-update profile `bande-secrets`: `auth` stashes the password in PAM data (zeroed by
  the cleanup), `session` sends it to `$XDG_RUNTIME_DIR/bande-secrets/pam`
  (`/run/user/<uid>/…`, systemd user socket, mode 0600) and waits for the ack, `password`
  (`Password-Final`) sends old and new so Users & Groups (which runs `passwd`) and `passwd`
  re-encrypt the keyring. The module checks `SO_PEERCRED` uid == the user before writing; the
  daemon accepts only uid 0 or its own uid. The lock screen runs `auth` only, so it stashes and
  drops the password and never unlocks anything: the keyring is not locked by the lock screen,
  as on macOS (the login keychain stays unlocked; only the Passwords app re-locks).
  If the login password does not open the keyring (admin reset the password), the next unlock
  prompt offers macOS's "Update Keychain Password" (old password, re-encrypt with the login
  password) or "Create New Keychain" (old file kept as `login-<time>.keyring`).
- **Prompts** (macOS Keychain dialogs) run in `bande-secrets-prompt`, a toolkit layer-shell
  process spawned per prompt (the daemon never loads Wayland/GPU code). Kinds: unlock a
  collection, allow an app to use an item (password, Deny / Allow / Always Allow), new
  collection password, update the login keychain password. Password goes back over a pipe.
- **Per-app access** (macOS item ACL): items record the creating app (`bande:creator`, the
  flatpak id or the executable path of the D-Bus peer). For another app the item reports
  `Locked` and lands in `SearchItems`' locked list; `Unlock` gives a prompt; "Always Allow"
  adds the app to `bande:allowed`. Items without a creator (imported) stay open, as before.
- **Search on a locked collection** waits for one shared unlock prompt (KeePassXC) instead of
  returning nothing, since v1 files cannot match attributes without the key.
- **Portal** `org.freedesktop.impl.portal.Secret` in xdg-desktop-portal-bande: one item per app
  id (`xdg:schema=org.freedesktop.impl.portal.Secret`, `app_id`), 64 random bytes, written to
  the fd; same attributes as gnome-keyring so existing Flatpak keys survive the import.
- **NetworkManager agent** in the daemon (`org.freedesktop.NetworkManager.SecretAgent`,
  identifier `org.bande.Secrets`): agent-owned secrets stored with libnm's attributes
  (`xdg:schema=org.freedesktop.NetworkManager.Connection`, `connection-uuid`, `setting-name`,
  `setting-key`) so nm-applet/gnome-shell entries import; interactive requests show the Wi-Fi
  password prompt. Joins from Bande keep system-owned PSKs (macOS keeps Wi-Fi passwords in the
  System keychain so they work at the login window).
- **Settings ▸ Passwords**: list with search, app column, reveal/copy/edit/delete. Opening it
  unlocks all items for Settings through one prompt (macOS Passwords asks each time); the grant
  ends when Settings quits or the session locks (daemon drops grants on logind `Lock`).
- **Packaging**: `bande-secrets` `Provides: org.freedesktop.secrets`, `Conflicts`/`Replaces:
  gnome-keyring` (one owner of the bus name; Debian's MTA idiom), ships the PAM module at
  `/usr/lib/<triplet>/security/` and `/usr/share/pam-configs/bande-secrets` (postinst
  `pam-auth-update --package`, prerm `--remove` like libpam-gnome-keyring). `bande-desktop`
  depends on it instead of recommending gnome-keyring; Mail and Calendar recommend
  `bande-secrets | org.freedesktop.secrets`; the portal config routes Secret to `bande`.

## Threat model

- Protects: secrets at rest (stolen disk/backup) by the login password (PBKDF2, AES-CBC +
  HMAC); secrets of other users (socket and file modes, peer-uid checks); casual cross-app
  reads by the same user (per-app confirmations, visible in Settings); shoulder surfing in
  Settings (re-authenticate each time, re-lock on session lock).
- Does not protect against malicious code already running as the user outside a sandbox: it can
  impersonate an app (executable identity is advisory), read the file and brute-force it, or
  type into prompts. Flatpak apps only reach their own portal key.
- Leaks avoided: the daemon and prompt set `PR_SET_DUMPABLE=0` (no core dumps, no same-uid
  ptrace/`/proc/pid/mem`), `RLIMIT_CORE=0`; secrets live in `Zeroizing`; logs never carry
  secrets, attribute values or lengths; the PAM module never logs; the PAM socket path is
  derived from the uid, not the environment, and the peer uid is checked before writing.
- Empty keyrings accept any password (nothing to verify against); acceptable since there is
  nothing to reveal.

## Phases (each compiles and merges)

1. Daemon: store, import, Service/Collection/Item/Session/Prompt API, plain + DH sessions, PAM
   socket, PAM module + profile, units, packaging. Verify: secret-tool, DH, import, pamtester.
2. Prompt process: unlock, new collection, per-app access, update-login-password.
3. Portal Secret backend.
4. NetworkManager secret agent and Wi-Fi password prompt.
5. Settings ▸ Passwords.
6. Switch packaging: bande-desktop, portal config, apps (Mail, Calendar).
