# Time zone change does not apply

## Summary

Settings writes the new zone correctly; timedated changes `/etc/localtime`. The running
desktop does not see it because every clock asks jiff for the system zone, and jiff caches
that zone for 5 minutes with no public way to reset it. Fix: read the zone ourselves, without
a cache, and redraw clocks when timedated announces a new zone.

## What the owner sees

In Settings > Language & Time, Change… > pick a city. The Settings row shows the new city at
once. The bar clock, the notification centre and other clocks keep the old time for up to
5-6 minutes. Then they jump. Anyone who checks within a few minutes concludes it did not apply.

## Findings

The write works:
- `/etc/localtime -> ../usr/share/zoneinfo/Europe/Stockholm`, mtime 2026-10-31 10:46:20.
  The user journal shows Settings (pid 265229) open from 10:42:17 to 10:49:17, so the change
  was made from Settings and written.
- `timedatectl status` shows `Europe/Stockholm (CET, +0100)`.
- `/usr/share/polkit-1/rules.d/50-unbound-settings.rules` returns YES for
  `org.freedesktop.timedate1.set-timezone` for local, active members of `sudo`. The owner is
  in `sudo`. `pkcheck --action-id org.freedesktop.timedate1.set-timezone` returns 0. No
  password prompt is needed, and the polkit agent is not involved.
- Settings calls `SetTimezone(zone, interactive=true)` (`crates/apps/settings/src/time.rs:130`).
  It updates its own row at once (`view.rs:1136-1140`) and then follows timedated's
  PropertiesChanged with an uncached proxy (`time.rs:139-158`). Settings itself is fine. The
  existing test `crates/apps/settings/tests/clock.rs:122` covers this path.
- I could not read the system journal for timedated or polkit without sudo. The evidence
  above makes that unnecessary.

Pick-up fails:
- No session process has `TZ` set: compositor, shell, notify and config were checked through
  `/proc/<pid>/environ`, and so was `systemctl --user show-environment`. That is correct:
  with `TZ` set, nothing would ever follow `/etc/localtime`.
- The clocks call `jiff::Zoned::now()`: the bar (`crates/shell/bar/src/menu_bar_client.rs:681,684,797`),
  the notification centre (`crates/shell/notifications/src/lib.rs:150,294`, `client.rs:268,296`),
  the lock screen (`crates/shell/lock/src/main.rs:121,197`), the greeter
  (`crates/shell/greeter/src/main.rs:86,304`), Night Light
  (`crates/platform/compositor/src/night.rs:238`), the portal's automatic appearance
  (`crates/services/portal/src/settings.rs:440,475`) and screenshot names. Files and
  `system_place()` call `TimeZone::system()` (`crates/libs/fs/src/listing.rs:268,328,339`,
  `ops/archive.rs:155,182`, `crates/libs/datetime/src/daylight.rs:161`).
- jiff 0.2.38 caches the system zone in a process-global with a 5-minute TTL
  (`jiff-0.2.38/src/tz/system/mod.rs:69` `TTL = 5 * 60`, `get` at :123-143). The reset its
  doc comment mentions is not public. Only `get_force` bypasses the cache, and it is
  `pub(crate)`.
- The bar redraws only on the minute timer (`crates/libs/status/src/clock.rs:6-13`). The timer
  uses `TFD_TIMER_CANCEL_ON_SET` (`crates/libs/datetime/src/minute.rs:37`), which fires on
  clock *jumps* only. A zone change does not touch CLOCK_REALTIME, so nothing wakes early.
  Worst case: 5 min (jiff TTL) + 1 min (tick).
- No desktop process outside Settings listens to `org.freedesktop.timedate1`
  PropertiesChanged. timedated emits it for `Timezone` on every successful SetTimezone.
- The portal's automatic dark/light schedule arms a timerfd for the next sunrise or sunset,
  using the old zone and the place derived from it (`settings.rs:226-237`, `wake_at` at :388).
  After a zone change it can be wrong until that boundary, which can be hours away. Night
  Light re-checks every 15 min (`night.rs:18`), so it corrects itself once the zone is read
  uncached.

Root cause: the zone is written but not picked up. jiff's 5-minute system-zone cache, plus no
listener for timedated's Timezone change.

Side finding, same pane: `refused()` (`crates/apps/settings/src/privilege.rs:367-374`) uses
`?` inside the loop. Polkit answers `Action org.freedesktop.packagekit.system-update is not
registered` (user journal, every Settings start), so the whole check fails and Settings
never learns which time, locale or hostname actions are refused. Here that is harmless,
since the owner is allowed. A non-admin user would get no up-front "Only an administrator"
row, only the error after trying.

## Options

1. **Own uncached zone read, plus a timedated listener (recommended).** `unbound_datetime::now()`
   reads the `/etc/localtime` symlink on each call: one `readlink` plus jiff's per-name db
   lookup, which is cached. Long-lived processes subscribe to timedate1 PropertiesChanged and
   redraw at once. The change shows within a second and works for `timedatectl` too. Cost:
   one small module and one shared follower.
2. **Uncached read only, no listener.** The clock shows the new zone within a minute.
   Simpler, but the portal schedule stays wrong until its next boundary, and "within a
   minute" still looks broken right after a change.
3. **inotify on `/etc` for `localtime`, instead of D-Bus.** Also catches a root `ln -sf`
   that bypasses timedated. It needs a new inotify wrapper and is harder to test on the
   hermetic bus. Not worth it: every supported path (Settings, `timedatectl`) goes through
   timedated.
4. **Restart the shell on a zone change.** Visible flicker and lost state. Rejected.
5. **Set `TZ` in-process.** `setenv` in a multithreaded process is unsound, and it would pin
   the zone. Rejected.

## Recommendation

Option 1. Put the zone reading and the shared timedate1 proxy in `libs/datetime`, so Settings,
the shell and the portal use one proxy. The shell and the portal follow zone changes. Every
other clock just reads the zone uncached.

## Implementation plan

Each commit is one short branch from `main`, merged when the local gate is green, then
verified on the mini.

1. **Read the local zone without jiff's cache** (~1 h)
   - New `crates/libs/datetime/src/zone.rs`:
     - `pub fn local_zone() -> TimeZone`: if `TZ` is set, return `TimeZone::system()` (an
       explicit override wins). Otherwise readlink `/etc/localtime`, take the part after the
       last `zoneinfo/` and look it up with `jiff::tz::db().get`. If any step fails, fall back
       to `TimeZone::system()`.
     - `pub fn now() -> Zoned`.
     - Keep an inner `zone_at(&Path)` for tests.
   - `daylight.rs:161`: `system_place()` uses `local_zone()`.
   - Add `tempfile` to the datetime dev-dependencies.
   - Tests (unit tests in `zone.rs`):
     - Re-pointing a symlink in a tempdir from `…/zoneinfo/Asia/Tokyo` to
       `…/zoneinfo/America/New_York` changes the zone on the very next read. This is the bug;
       it fails with `TimeZone::system()`.
     - `../usr/share/zoneinfo/America/Argentina/Buenos_Aires` resolves to the full
       three-part name.
     - A link with no `zoneinfo/` in it falls back and does not panic.

2. **One timedate1 proxy and a fake timedated on the test bus** (~1.5 h)
   - Move the `Timedate` proxy from `crates/apps/settings/src/time.rs:18-35` to
     `crates/libs/datetime/src/timedate.rs`. This adds `zbus` to datetime.
   - Add `pub fn zone_changes(system: &Connection) -> zbus::Result<impl Iterator<Item = String>>`.
     It yields the new zone name on each PropertiesChanged that carries `Timezone`.
   - Settings `follow_time` uses the shared proxy.
   - Move the fake from `crates/apps/settings/tests/clock.rs:20-88` into
     `crates/testing/test-bus/src/timedate.rs`, with an `install(&SessionBus, zone)` helper in
     the style of `bluez.rs` and `networkmanager.rs`. Keep its refusal of `Asia/Tokyo`, which
     the Settings test relies on.
   - Tests:
     - The existing `the_time_zone_is_chosen_by_typing_a_city…` test passes, using the moved
       fake.
     - New `crates/libs/datetime/tests/timedate.rs`: `SetTimezone("Europe/Paris")` from a
       client yields `"Europe/Paris"` from `zone_changes` within the wait. A refused zone
       yields nothing.

3. **Clocks redraw on a zone change and read the zone uncached** (~1.5 h)
   - `crates/libs/status/src/clock.rs`: `follow_zone(system, outbox)` sends `Update::Minute`
     for each item from `zone_changes`.
   - `menu_bar_client.rs:766`: spawn it when the system bus is present, next to
     `follow_minutes`.
   - Replace `jiff::Zoned::now()` with `unbound_datetime::now()` in the bar, the notification
     centre, the lock screen, the greeter, compositor `night.rs:238` and screenshot.
   - Replace `TimeZone::system()` with `local_zone()` in `libs/fs` `listing.rs` and
     `archive.rs`.
   - Grep afterwards: no `Zoned::now()` or `TimeZone::system()` is left outside
     `libs/datetime`.
   - Test: new `crates/libs/status/tests/clock.rs`, modelled on `devices.rs:80`. Run the fake
     timedated on the hermetic bus with `follow_zone` on an `Inbox`. After `SetTimezone`, an
     `Update::Minute` arrives well before the next whole minute. Assert it within 2 s, and
     start the test right after a minute boundary so the tick cannot cause a false pass.

4. **The portal re-plans automatic appearance on a zone change** (~1-1.5 h)
   - `crates/services/portal/src/settings.rs`: `serve` takes an optional system connection.
     A thread follows `zone_changes` and sends on the existing `wake` channel.
   - `read` uses `unbound_datetime::now()`. Pass the system bus in from the portal's `main`.
   - Test, in `crates/services/portal/tests/settings.rs`: with appearance `auto` and the fake
     timedated, a zone change triggers a fresh `SettingChanged` or `ReadAll` with the zone's
     schedule, without waiting for the armed boundary. Pick two zones where it is day in one
     and night in the other at test time, computed from `daylight::sky`, so the color scheme
     must flip.

5. **Privilege check survives an unregistered action** (~45 min)
   - `crates/apps/settings/src/privilege.rs:367`: an error for one action no longer aborts
     the loop. Log it and treat that privilege as refused.
   - Test: extend `crates/apps/settings/tests/privilege.rs`. The fake authority returns
     `Failed("… is not registered")` for the packagekit action. `refused()` still returns Ok,
     with `Hostname` and `Updates` refused and `Time` allowed.

Verification on the mini after commit 3, and again after 4:
- Install the debs.
- Open Settings > Language & Time and change the zone to Europe/London. The bar clock moves
  back an hour within a second, and the notification centre's dates follow.
- `timedatectl set-timezone Europe/Stockholm` from a terminal makes the clock follow back.
- Lock the screen and check the lock clock.

## Risks

- `now()` is called while rendering notification ages: one `readlink` plus a db lookup per
  call. That is microseconds, but check the notification centre's frame time with many
  notifications after commit 3. If it shows up, read the zone once per frame.
- `jiff::tz::db()` also re-checks zoneinfo on its own TTL. That is harmless here, since only
  the link changes.
- The greeter runs as the greeter user and gets no live follow. It reads the zone uncached
  on each minute tick, which is enough at the login screen.
- If an admin ever sets `TZ` in `environment.d`, zone changes are ignored by design, as in
  glibc. Never set `TZ` in the session environment.

## Decisions for the owner

None required. Commit 5 is outside the literal complaint; drop it if you want this kept
strictly to the zone change.
