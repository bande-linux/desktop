# Security baseline

Owner of disk encryption and the permission model. `fix-first-run` owns the image and the
assistant, `fix-update-safety` the btrfs layout inside the LUKS volume, `fix-backup` builds on both.

## Threat model, ranked by what protects a user

| # | Who | What they get today | Answer | Real boundary? |
|---|---|---|---|---|
| 1 | Thief with the Mac or the disk | every file, saved Wi-Fi keys, keychain | LUKS on root, on by default in first run | yes, while powered off |
| 2 | Someone at a locked Mac, or a program acting while it is locked | virtual keyboard types into the lock field; input method can feed it | nothing but the lock client's own wl_keyboard reaches the lock surface; `bande-lock` gets its own role | yes |
| 3 | A sandboxed app (Flatpak) | reads every key, types anywhere, reconfigures displays, overlays the screen | `wp_security_context_v1`; privileged protocols refused unless consented | yes |
| 4 | Others on the network | SSH with password login is on in the image; LLMNR answers | SSH off until asked for (Remote Login, key or password chosen there); LLMNR off; firewall below | yes |
| 5 | A hostile unsandboxed program of the user's | everything the user can reach | raises the bar only: the same gates, consent kept where the user cannot write, `yama.ptrace_scope=1` | **no** |
| 6 | Whoever tampers with updates | | apt signatures, audited below | yes |
| 7 | Evil maid: changes the unencrypted boot files while the Mac is away | the passphrase on next boot | none. The chain after Apple's firmware is unsigned and the SEP is out of reach. Stated in the pane | not covered |

Clipboard history over the session bus stays open to unsandboxed callers: its files are readable by
the same user anyway, and a sandbox never reaches that bus. Row 5 stays honest. A program running as you can edit your `~/.profile` and be inside your next
session; Linux has no SIP. Nothing in Settings may claim otherwise. Sandboxes and root-owned consent
are what we have; macOS-level containment of unsandboxed apps is not reachable.

## Permission model for privileged Wayland protocols

Who a client is, decided once when it connects:

- **Session client:** started by the compositor on a socket pair with a role (`Shell`,
  `ClipboardDaemon`, and now `LockScreen`).
- **Sandboxed:** connected through a `wp_security_context_v1` listener; named by the app id. Flatpak
  uses this when it is offered.
- **Program:** `/proc/<pid>/exe` of the peer, believed only if it is under `/usr`, root-owned, not
  group or world writable, the same inode as that path in the compositor's mount namespace, in the
  compositor's mount namespace, and not an interpreter. Otherwise **unknown**.

| Capability | Protocols | Who |
|---|---|---|
| Read keys, type | `zwp_input_method_v2`, `zwp_virtual_keyboard_v1` | consent per program or app id; never while locked |
| Screen capture | `ext-image-copy-capture`, `bande_capture_v1` | shell role; everyone else through the ScreenCast portal, which asks |
| Clipboard history | `ext-data-control` | clipboard daemon role |
| Global shortcuts | none offered yet: no portal backend, no protocol | nobody |
| Overlays, displays, idle | `wlr-layer-shell`, `wlr-output-management`, output power, idle notifier | everyone but sandboxed clients |
| Lock | `ext-session-lock` | lock role only |

Overlays, displays and idle stay open to unsandboxed programs: a native program can draw a full-screen
window anyway, and Settings, the polkit agent and alerts need them, so narrowing further protects little
and breaks running anything from a build tree.

**Consent as on macOS:** the global stays visible, so an asking program learns the answer. Refused,
`get_input_method` gets the protocol's `unavailable` and `create_virtual_keyboard` the protocol's
`unauthorized`. The first ask from a known identity names it in a notification that opens Privacy &
Security; there the person turns it on, with their password. It works from the program's next start;
turning it off disconnects it at once. An unknown identity cannot be granted.

**Consent lives in `/var/lib/bande/consent/<uid>`**, root-owned, written by a small helper through
polkit with `auth_admin`, no retained authorisation. A consent file in the home would be a gate any
program could open silently. Same reason macOS guards TCC.db.

**Secure input:** a granted input method sees keys only while a text field is active and never in a
password field (the toolkit sends the `password` purpose, the compositor releases the grab), as
macOS Secure Input does. Conforming input methods grab again on activate.

## Disk encryption

**Experience.** First run asks for a password and says it also unlocks the disk; on by default,
skippable, can be turned on later in Settings without a reinstall. It shows a recovery key once to
write down. At power-on, after the Bande mark: the same mark with a password field. The right
password goes straight to the desktop, without a second password at the login window. A wrong one
shakes the field. Forgot it: "Use recovery key".

**Layout.** GPT: ESP, recovery, `BANDE-ROOT` as LUKS2 (aes-xts-plain64, argon2id per machine) with
btrfs inside. Kernel and initrd on the ESP, unencrypted, so rollbacks and boot counting are unaffected:
snapshot entries change `rootflags` only.

**Mac image** (macOS cannot run cryptsetup). `root.img` is LUKS2 with the well-known passphrase `bande-provisional` (cheap KDF) and a
header token `bande-provisional`. Grow first, then at first run the helper adds the user's slot,
removes the provisional one and reencrypts online in the background, because every image shares one volume
key until then. A resume service continues it across reboots. USB install: fresh volume key at
install, no reencrypt.

Honest states, also in the pane: **Off** (provisional key still there), **Encrypting N%** (what is
not yet rewritten is readable with the public key), **On**. No secret is written to disk before the
passphrase is set and reencrypt has started. Nothing secret goes on the ESP; the installer's plain
first-boot files are retired by the assistant.

**Unlock prompt:** `bande-unlock`, a small native program in the initramfs, no Plymouth (see
`session.md` for why). It is a `keyscript` for Debian's `cryptsetup-initramfs` on initramfs-tools,
which the image already uses. It draws the mark and the field on the console's framebuffer, reads
evdev directly, and falls back to the console's `askpass` if there is no framebuffer. It holds no
fonts or xkb data: `bande-unlock-assets` writes a small non-secret file on the ESP with the
pre-rendered strings for the system language and a keycode table for each configured layout, so a
passphrase with `å` types right. The layout is shown and can be switched at the prompt. Keeping this
on the ESP means changing language or layout never rebuilds an initrd. Missing file: US layout and
English.

**One password.** On success `bande-unlock` puts the passphrase in the kernel keyring (`user`
keyring, timed), and the login path takes it from there to sign the one matching user in and unlock
their keychain. Each user with disk access has their own labelled slot; changing the login password
changes the slot in the same PAM step. The passphrase is never written anywhere.

**Recovery key:** 128 bits random in eight groups, in its own labelled slot, shown once. No escrow.
Lost password and lost key is lost data; first run says that plainly.

**Keyboards:** Bluetooth does not work in the initramfs. Mac laptops use the built-in keyboard; a Mac
mini needs a USB keyboard at boot. First run tells the person. Running BlueZ in the initramfs is not
lean.

**Helper:** `/usr/libexec/bande-disk-encryption`: `status`, `set-passphrase` (stdin; prints the
recovery key), `change-passphrase`, `resume`. Safe to run again after a power cut at any step.

The mini stays plain ext4 and is not reinstalled.

## Other baseline

- **Sandboxing.** Flatpak is the sandbox; the portal already asks. Native apps run unsandboxed and the
  pane says so. AppArmor profiles for a few exposed native parsers are the realistic extra, later and
  only if a profile survives real use. `systemd` hardening for the services we own costs little.
- **Signed updates, audit:** apt verifies `InRelease` against the pinned `Signed-By` keyring, insecure
  repositories are refused, and our archive sets `Valid-Until`. Kernel and firmware arrive as apt
  packages. The gap: the Mac installer takes the image over HTTPS with no signature of its own
  (`fix-first-run`).
- **Firewall: no default.** With SSH off and LLMNR off only mDNS listens, so a default-deny ruleset
  protects nothing that is on by default, and it silently breaks what a person starts on purpose (file
  sharing, a dev server). macOS also ships its firewall off. Revisit if anything listens by default.

## Privacy & Security pane

Real state only, read from the system: disk (Off, Encrypting N%, On, and whether a recovery key
exists), Input Monitoring and Typing with who is allowed and who asked, screen recording and camera
grants from the portal, Remote Login, and one honest paragraph on
what native apps can reach.

## Stages

1. Done: input method and virtual keyboard gate, lock role, security context, secure input.
2. Done: consent store, helper, notification, pane.
3. Done: real state in the pane, ptrace scope, LLMNR off; SSH is not in the image.
4. Built, not proven end to end: disk helper, keyscript, `bande-unlock`. Left: boot an encrypted image in
   QEMU with `fix-first-run`, power cut during reencrypt, Mac hardware (framebuffer, keyboard, modules).
5. Built, not proven end to end: one password through the greeter and PAM.
