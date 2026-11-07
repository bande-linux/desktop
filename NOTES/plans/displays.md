# Trustworthy displays

## What was wrong
- **No link breaks during use were found.** Every `the link to HDMI-A-1 broke` on the mini came when a display was turned back on after `dcp_poweroff`. About 0.6 s after the first mode-set the monitor pulses HPD low for about 0.6 s, the DCP reports an unplug and the kernel marks the link bad, so the mode is set twice: about 3.5 s of black at wake. The pulse is the monitor's; we cannot avoid it.
- **Hot-plug never ran.** `rescan` read connectors without probing, so the kernel answered with the status and modes it had cached. `was disconnected` and `was connected` appear nowhere in the journal since 2026-10-20.
- **Replug failures** (2026-11-04): the DCP published a connected display with no modes (`TimingElements without valid modes`), so every frame's mode-set failed with EINVAL until the owner replugged. Frames stalling for 15 s also ends the session.
- The last display is never removed. Relink commits outside the frame path and races a flip.

## What the user experiences
- A display that blinks (wake pulse, monitor standby, KVM) comes back within the grace period with nothing moved.
- Unplugging one of several: after the grace period its windows go to the main display; plugging it back returns them to their places, sizes and workspaces.
- Unplugging the only display: nothing moves and apps keep running. The same display returns as it was. Another display takes over with the windows fitted to it, and they go back when the first returns.
- A mode that fails falls back to the next one that passes the driver's test. If none passes, the display waits for the next hot-plug event: no retries, no log flood, and the session stays up.
- Scale and arrangement stay per monitor and per set of monitors in `config.kdl`; a fallback never rewrites them.

## Design
- **Probe on every change event**, like wlroots. Now that a pulse is visible, a head that disappears is held as lost for `DISPLAY_GRACE` (3 s, a one-shot timer) before it counts as gone. Another EDID on the connector is gone at once.
- **The compositor always has a screen.** The last one is parked (no head, no frames, not powered, hidden from the display bus) instead of removed, because clients and the shell keep a `wl_output` and `screens[0]` stays valid. Alternative rejected: allowing zero screens, which means every path and every client handling no output, for no user gain.
- **Modes are tested before use** with `DrmCompositor::use_mode`, which runs the driver's atomic test. Windows only refit once a mode has passed, so a kernel's made-up 1024x768 cannot shrink them.
- **Relink happens in `present`**, after any pending flip, so it cannot race one.
- **Kernel:** a display that publishes no modes gets its DP link cycled a few times instead of waiting for a replug.

## Left
- Kernel: branch `fix-displays` in the kernel repo adds the no-modes recovery. Not installed, not booted.
- On the mini: unplug and replug the only display, and swap it for another (`NOTES/open.md`). Real DRM hot-plug has only run on a builder's virtio-gpu, through the connector's sysfs `status` and a `change` uevent. Delete this plan once that passes.
