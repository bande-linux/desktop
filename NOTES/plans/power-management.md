# Power management

## Summary

Of the four things the owner names, only the lock works. The display never turns off.
Settings says "Turn off the display and lock after", but nothing in the compositor can power
an output down. Dimming does nothing on the mini, because an HDMI monitor has no backlight.
Sleep is turned off on purpose at the system level. On 2026-10-30, s2idle aborted in the Wi-Fi
driver and left the display dead, so `CanSuspend` is `no`. `bande-power`'s suspend stage is
then a silent no-op, and the power button does nothing. Wake by keyboard needs kernel patches
that are written but have not been tried on the hardware. There is no visual screen saver, and
none is needed.

Fix in this order. First, the desktop turns the displays off at the lock stage, and any input
turns them back on (DPMS, testable now). Then the hardware sleep series is tested with a person
at the mini. Once ten cycles pass, sleep comes back on.

## What the owner sees

- After 5 minutes idle: nothing (dim has no backlight to dim). After 10 minutes, the lock
  screen appears and stays lit until someone touches the machine.
- Sleep from the menu bar is missing, or does nothing. The power button does nothing.
- "Screen saver": nothing ever blanks the screen.

## Findings

- **Idle chain** (`crates/services/power/src/idle.rs`, `controller.rs`). The compositor's
  `ext_idle_notifier_v1` (`crates/platform/compositor/src/idle.rs`) drives a state machine:
  Dim at `dim_after` (default 300 s), Lock at `lock_after` (600 s), Suspend at `suspend_after`
  (default "with the display off" = `lock_after`, `docs/architecture/behaviour.md` "Sleep when
  idle"). Inhibitors (`org.freedesktop.ScreenSaver`, Wayland idle-inhibit) hold it. This part
  works and has tests (`crates/services/power/tests/wayland.rs`).
- **Dim**: `logind_executor` sets the `backlight` class device through logind
  `SetBrightness`. `/sys/class/backlight` is empty on the mini, so Dim and Undim do nothing.
  macOS does not dim external displays either.
- **Display off: missing.** The Settings row is `lock-after = Turn off the display and lock
  after` (`crates/libs/settings-panes/locales/en-US/bande-settings-panes.ftl:99`), and
  `behaviour.md` says "Bande's lock stage is the display turning off". But `drm.rs` never calls
  `DrmCompositor::clear()` (smithay's DPMS off). The compositor offers no
  `zwlr_output_power_manager_v1`, and `bande-power` has no display command. The lock screen
  stays lit forever.
- **Sleep: off at the system level.** `CanSuspend` = `no`. `suspend.target` and `sleep.target`
  are masked on the mini, and `images` `prep/hw-pm-nosuspend` adds `AllowSuspend=no`. The
  reason is in `~/bande-linux/notes/pm.md`. On 2026-10-30 an idle suspend at 802 s aborted:
  brcmfmac `pci_pm_suspend` -EIO (D3 ack timeout), and DCP's abort path left the display dead
  (drm resume -22). `/sys/power/mem_sleep` is `[s2idle]` only. Apple Silicon has no deep sleep.
  `controller.rs:165` skips Suspend unless `CanSuspend` is `yes`, so the stage is a silent
  no-op.
- **Wake with keyboard**: needs the kernel series `prep/hw-pm-v5`. It includes the AIC
  `IRQCHIP_SKIP_SET_WAKE`, msi-lib set_wake and the pcie-apple MSI wake forward, so a USB
  keyboard's PME reaches the AIC. The series also has brcmfmac WoWLAN-gated PCI wakeup and the
  i2c-pasemi noirq reinit. It is reviewed but untested on hardware. `macsmc-rtc` has no wake
  alarm, so every test needs a person. `~/bande-linux/bin/s2idle-test.sh` runs one cycle.
- **Power button**: `HandlePowerKey=suspend` (`crates/services/power/data/logind.conf`). With
  sleep not allowed, logind refuses the action, so the button does nothing visible.
- **Screen saver**: `bande-power` serves `org.freedesktop.ScreenSaver` for inhibition only.
  On a desktop Mac the display turns off at 10 minutes, before the default screen saver
  timer, so in practice no screen saver ever shows. Display off is the modern equivalent, and
  no screen saver is needed.
- **Display power risk on DCP** (`repos/HARDWARE-GAPS.md` §B): "DCP doesn't re-read modes
  after a power-down until a real hotplug, so an HDMI replug or display sleep can leave 0
  modes." A DPMS off/on may come back black on the mini. The compositor keeps the mode it
  had, so re-enabling with the same mode may work anyway. Only a test at the machine can tell.

## Recommendation

1. **Display off at the lock stage** (desktop). The compositor implements
   `wlr-output-power-management-unstable-v1`. It is the standard protocol (sway, Hyprland,
   niri, labwc), and `wlopm` and `swayidle` already speak it. Off calls
   `DrmCompositor::clear()` and holds the frame scheduler, so clients get no frame callbacks
   and the GPU idles. On re-arms the scheduler, and the next frame re-enables the CRTC. **Any
   input turns every display back on** in the compositor itself, so a crashed or hung
   `bande-power` can never leave the screen black. `bande-power` sends Off right after Lock
   and On when the user comes back.
2. **Sleep stays off until the hardware passes** (kernel + images). Boot a kernel with
   `prep/hw-pm-v5` and the Wi-Fi D3 fix. Run `s2idle-test.sh keyboard|power|mouse` ten times
   with a person at the mini. Then drop `AllowSuspend=no` from the image and unmask the sleep
   targets. The desktop needs no change: its suspend stage starts working when `CanSuspend`
   says `yes`.
3. **Power button while sleep is unavailable** (desktop). Hold logind's `handle-power-key`
   inhibitor in `bande-power`. On a short press, sleep if `CanSuspend` is `yes`, otherwise lock
   and turn the displays off. A long press stays logind's power-off. That way the button
   always does something visible.
4. **No dimming of external displays.** DDC/CI brightness (ddcutil) is slow, flaky over HDMI
   and not what macOS does. Dim stays a built-in-panel feature.

## Milestones

1. **Compositor output power** (1-2 h). New `output_power.rs`: the
   `zwlr_output_power_manager_v1` global, per-output power state, a `mode` event to every
   control of that output, and `failed` for an unknown or removed output. `DrmBackend::submit`
   clears the head once and returns `Paused` while its output is off. Turning it on resumes
   the schedulers. Input powers every display on. Tests run on the headless compositor.
2. **`bande-power` turns displays off at the lock stage** (1 h). `Command::DisplayOff` follows
   `Lock` at the lock stage, and `Command::DisplayOn` comes on resume. The Wayland watcher binds
   the manager and every `wl_output` and runs both commands. An end-to-end test with the
   headless compositor checks idle → Lock + DisplayOff → output off → key → output on.
3. **Owner test at the mini** (15 min, person). Set "Turn off the display and lock after" to
   1 minute, wait, then press a key and move the mouse. If the monitor comes back black, the
   fix is kernel §B (`dcp_poweron` re-requests `TimingElements` when `nr_modes == 0`).
4. **Blank a manually locked screen** (1 h). When the session is locked (Super+L or the menu)
   and idle for 60 s, turn the displays off. GNOME blanks its lock screen after 30 s, and macOS
   sleeps the display shortly after a manual lock.
5. **s2idle on hardware** (1-2 h, person, kernel). Boot `prep/hw-pm-v5` + Wi-Fi D3 fix, then
   ten cycles each of keyboard, power button and mouse wake. Check that the display, Wi-Fi,
   Ethernet, USB and audio come back.
6. **Sleep back on** (30 min, images + mini). Remove `AllowSuspend=no` from
   `prep/hw-pm-nosuspend` before it merges, and unmask `sleep.target`/`suspend.target` on the
   mini. Check the menu bar's Sleep item, idle suspend after display off, and the power button.
7. **Power button fallback** (1-2 h). Recommendation 3, with a test on the hermetic bus using
   a fake logind.

## Status

- Milestones 1, 2 and 4 done (2026-11-01). The compositor serves
  `zwlr_output_power_manager_v1` (`output_power.rs`) and turns every display back on at the
  first input. `bande-power` turns the displays off at the lock stage, and after 60 s idle on a
  screen that is already locked. Neither is installed on the mini yet.
- Milestones 3, 5 and 6 need someone at the machine. Milestone 7 is open (see decision 3).

## Decisions for the owner

1. **Display off and lock together** (current Settings wording) or **display off before
   lock** (macOS: display sleep and "require password after N" are separate)? Recommend
   together. It matches Settings and needs no new row.
   Decided overnight 2026-11-01: together (plan recommendation; owner may revisit).
2. **Re-enable sleep after ten clean s2idle cycles**, or keep it off until upstream Asahi
   calls s2idle stable on the M2 mini? Recommend after ten cycles. Still open (needs a person
   at the machine).
3. **Power button without sleep**: lock and display off (recommended), or show the Shut Down
   dialog like macOS's power-key dialog.
   Decided overnight 2026-11-01: lock and display off (plan recommendation; owner may revisit).
   Not built yet: check first whether logind's `handle-power-key` inhibitor also blocks the
   image's `HandlePowerKeyLongPress=poweroff`, the only way out of a hung session.
