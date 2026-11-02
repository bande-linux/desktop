# Network: Ethernet set up outside NetworkManager (ifupdown, networkd)

**Cause.** `bande_network::NetworkManager::ethernet` (crates/libs/network/src/connections.rs:228)
sets `connected = state == ACTIVATED` and reads addresses only when connected. Debian's installer
writes `allow-hotplug enp0s1 / iface enp0s1 inet dhcp` to /etc/network/interfaces unless the
desktop task put NetworkManager in the target, and NM's `[ifupdown] managed=false` makes such a
device state 10 (unmanaged). So the pane shows only "Set up by the system, outside
NetworkManager" (settings/src/network.rs:56) with no address and hides the IPv4 controls, and the
sidebar summary says "Ethernet not connected" (header.rs:157-161) while the VM is online.
systemd-networkd links are different: NM marks them "connected (externally)" (state 100, seen on
the mini for docker0). Bande shows those as connected, but offers Configure IPv4 on NM's generated
external connection, and an edit there would fight networkd. The bar and Control Center show only
Wi-Fi (bar/menu_bar.rs:383, controlcenter/lib.rs:186), so a wired-only VM shows nothing there.
That matches macOS, which has no Ethernet menu-bar item or Control Center module.

**Handover cannot go through D-Bus.** Setting `Device.Managed=true` overrides udev and
NetworkManager.conf unmanaged flags, but not `NM_UNMANAGED_USER_SETTINGS`, which ifupdown sets
(nm-device.c `_get_managed_by_flags`: "USER_SETTINGS can only explicitly unmanage a device").
`[ifupdown] managed=true` is also wrong: networking.service still runs `ifup` and its DHCP client
alongside NM's, and the ifupdown connections it imports are read-only, so Settings could not edit
them either. The safe Debian way, which the Debian wiki also recommends, is to remove the stanza:
`ifdown`, comment it out, and let NM manage the device. The image never meets this problem because
it is minbase without ifupdown. It does not convert anything, so there is nothing to copy from it.

**macOS / GNOME.** On macOS, Network ▸ Ethernet always shows the status dot, IP, subnet, router,
DNS and hardware details. When a profile manages the settings, the fields are read-only with a
note saying who manages them. GNOME Settings leaves unmanaged NM devices out completely
(cc-network-panel.c `device_added_cb`: they appear only once `notify::managed` says managed). That
gives Debian users with ifupdown an empty Wired section, a common complaint. Bande does better by
following macOS: show the real state read-only, then offer the handover.

## Fix
1. **Read-only state** (libs/network, ~120 lines). NM still publishes `Ip4Config` (kernel
   addresses and default route, from l3cfg, nm-device.c:5225) and `Wired.Speed` for unmanaged
   devices, and their PropertiesChanged signals already reach the watch in services.rs:371. That
   makes it event-driven with no netlink code. Add `Ethernet.owner: NetworkManager | Ifupdown {
   simple } | Networkd | System` (from `ifquery --list`/`ifquery <if>` run unprivileged when the
   pane opens, or networkd's `org.freedesktop.network1` Link `AdministrativeState == configured`),
   `speed`, and treat ActiveConnection `StateFlags & EXTERNAL (0x80)` as read-only. For a device
   NM does not manage: `connected = carrier && has an address`. DNS comes from systemd-resolved
   `Link.DNS` when resolved runs, otherwise from /etc/resolv.conf (watched with libs/watch).
2. **Settings pane** (~70 lines + ftl). Show "Connected" with IP, router, DNS and speed (left out
   when the speed is unknown, as on virtio), plus a footnote: "Configured by the system in
   /etc/network/interfaces" or "…by systemd-networkd". Hide Configure IPv4 for those devices and
   for external ones. Fix the summary in header.rs:157 to use the new `connected`.
3. **"Manage with Bande…"** (~300 lines). Offered only for ifupdown stanzas with
   inet dhcp/static (+inet6 auto) and the options address/netmask/gateway/dns-*/hwaddress. Bridges,
   bonds, VLANs, `pre-up` scripts and `wpa-*` stay read-only with no button. A new
   `/usr/libexec/bande-network-manage <iface>` runs through pkexec with action
   `org.bande.settings.network-manage` (auth_admin_keep, `exec.path` annotation), and ships with
   the .policy file in bande-settings. Steps: (a) write an equivalent keyfile, 0600, bound to the
   interface (dhcp → auto, static → manual address/gateway/dns), (b) back the file up to
   `<file>.bande-backup`, (c) `ifdown`, (d) comment the stanza out with a `# bande:` marker,
   (e) restart NetworkManager (other devices are kept by NM's assume), (f) wait for the device's
   StateChanged to reach 100. If that fails within 30 s, restore the backup, delete the keyfile,
   restart NM and run `ifup`. The confirmation tells the user the connection drops for a few seconds
   and the address may change.
   networkd handover is the same idea (rename a `.network` whose `[Match]` names only this
   interface) and is optional; see decisions.

## Risks
Mid-handover SSH drops, and the DHCP lease may change because client-ids differ. The rollback must
be tested with a broken keyfile. This is the first pkexec helper in the repo, so the input must
be strictly validated: the interface must exist in /sys/class/net and the stanza is parsed, never
passed to a shell. Assumption to check on the VM: `nmcli dev show enp0s1` lists IP4.ADDRESS while
the device is unmanaged. If it does not, step 1 reads rtnetlink through rustix `net` (~+150 lines).

## Verify
Owner, on the Debian VM: before the change, Settings shows Connected with IP, router and DNS,
read-only, with the footnote, and the sidebar says "Ethernet connected". Then Manage with Bande…,
enter the password: a short drop, after which `nmcli dev` shows `connected`, IPv4 Manual/Automatic
works, and the interfaces stanza is commented out. A reboot keeps it under NM. No e2e test: the
change is NM, ifupdown and polkit plumbing that a private bus cannot exercise.

## Size
1+2 ≈ 200 lines, half a day. 3 ≈ 300 lines plus .policy and packaging, about a day.

## Owner decisions
- **Install time?** Recommend **no**. A bande-desktop postinst that runs `ifdown` during `apt
  install`, often over SSH, can cut off the session that is installing it. NetworkManager.conf is
  network-manager's conffile. Silently rewriting the admin's interfaces file is not macOS-like
  either. The image needs nothing because it has no ifupdown. Settings offers the button instead.
- **networkd handover:** recommend read-only only for now. networkd users chose it on purpose.
  Add the button later if asked.
- **Ethernet in the bar or Control Center:** recommend **none**, as on macOS.
