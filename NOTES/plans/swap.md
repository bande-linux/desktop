# Swap and zram

## Summary

Swap is not only the installer's job. The desktop already ships its part: a zram-generator
drop-in and `Recommends: systemd-zram-generator, systemd-oomd` (commit 117500a1, today). It does
nothing here for three reasons. The bande kernel has no `CONFIG_ZRAM` and no `CONFIG_PSI`. The
images install the desktop with `--no-install-recommends`. The installed desktop package predates
the commit. Fix the kernel and the image. Ship no disk swap by default.

## Findings

- **This machine**: 15 GB RAM, `/proc/swaps` empty. One ext4 root (nvme0n1p6) beside the macOS
  APFS partitions, no swap partition. `systemd-zram-generator` and `systemd-oomd` are not
  installed. The installed `bande-desktop` (…1083.4b292c7) does not contain 117500a1, so it
  has neither `/usr/lib/systemd/zram-generator.conf.d/50-bande.conf` nor the `app.slice.d`
  oomd drop-in.
- **Kernel** (`/proc/config.gz`, built as `make defconfig bande-headless.config [bande-dev.config]`
  per `linux/.github/workflows/build.yml`): `# CONFIG_ZRAM is not set`, `# CONFIG_ZSWAP is not set`,
  `# CONFIG_PSI is not set`, `CONFIG_CRYPTO_ZSTD=m`, `CONFIG_HIBERNATION=y` (from the arm64
  defconfig). None of the fragments mention ZRAM or PSI. Without PSI, three layers of decision
  0004 fail at once. systemd-oomd needs `/proc/pressure/*` and refuses to start. The shell's
  low-memory window opens a trigger on `/proc/pressure/memory`. Activity Monitor's pressure graph
  reads PSI too. Without ZRAM, zram-generator 1.2.1 still writes `dev-zram0.swap`
  into swap.target, so boot waits 90 s for a `/dev/zram0` that never appears (seen on the mini
  2026-11-01); bande-desktop's `dev-zram0.device.d` drop-in cuts that wait to 10 s. Debian's kernel
  has `PSI=y`, `ZRAM=m`, `ZSMALLOC`, the zstd backend, and `ZSWAP=y` (off by default).
- **Images** (`images/customize.sh:36-37`): kernel, scripts and desktop .debs are all installed
  with `apt-get install --no-install-recommends`, so no image ever gets zram-generator or oomd.
  `images/rootfs` has no swap or fstab entry.
- **Installer** (`installer/`, the macOS-side Asahi-style installer): makes the APFS stub, ESP and
  root partitions. It creates no swap and no swapfile.
- **Debian installer**: guided partitioning creates a swap partition by default. The trixie
  manual suggests about RAM-sized swap, at least 512 MB [1]. d-i sets up no zram. Debian ships
  `systemd-zram-generator` (default `min(ram/2, 4096)`, lz4) and `systemd-oomd` as opt-in
  packages. A d-i install of `bande-desktop` would get a swap partition plus our Recommends.
- **Fedora Workstation**: zram by default since F33 (`min(ram, 8192)`, zstd), no disk swap, oomd
  on. **Ubuntu**: a swapfile (`/swap.img`) at install, no zram, oomd swap kill dropped (see 0004).
- **Fedora Asahi Remix** (the nearest reference, same hardware): zram plus an 8 GB swapfile on 8
  and 16 GB machines. They then moved swapfile systems to **zswap + swapfile** (12 GB for new
  installs) and turned zram off, because zram in front of disk swap caused premature OOM kills
  (Firefox). Machines with 24 GB or more, and no swapfile, stay on zram only [2]. So zram alone is
  fine, but zram stacked over disk swap is a known bad pairing. Decision 0004 says "an existing
  swap partition stays as overflow". That needs a caveat.
- **Hibernation**: needs disk swap (zram cannot hold the image). Asahi does not list hibernation
  for Apple Silicon. Its feature pages cover sleep and suspend only. Resume would have to go
  through iBoot/m1n1 and re-start every coprocessor's firmware (DCP, SMC, ANS…), which nobody
  supports. Treat it as unsupported. It is no reason to add disk swap. The desktop has no
  hibernate UI. logind reports `CanHibernate=na` without swap.
- **NVMe wear**: zram writes nothing to the SSD. A swapfile on Apple's NVMe gets real but modest
  write traffic. macOS itself swaps to that disk all the time, so it is acceptable if wanted. It
  is not needed at 16 GB with zram + oomd.
- **Debian policy**: shipping `/usr/lib/systemd/zram-generator.conf.d/50-bande.conf` is the
  generator's documented vendor drop-in mechanism. It changes no other package's conffile, admins
  override it in `/etc/systemd/zram-generator.conf[.d]`, and with zram-generator absent it is
  inert. Recommends, not Depends, is right: whoever already has a swap layout keeps it. No policy
  problem. A desktop environment carrying memory policy is what GNOME/Fedora do through their
  presets.

## Recommendation (who owns what)

| Owner | Job |
| :--- | :--- |
| **Kernel** (`linux`, `bande-headless.config`) | `PSI=y`, `ZRAM=m` with the zstd backend, `ZSMALLOC`, `ZSWAP=y` (default off), so it matches Debian's kernel. This is the blocker. |
| **Images** (`images/customize.sh`) | Install `systemd-zram-generator` and `systemd-oomd` explicitly (keep `--no-install-recommends` for the rest). No swap partition, no swapfile. |
| **Installer** (`installer/`) | Nothing for now. No swap partition. A swapfile, if ever, comes from a first-boot script, not from partitioning. |
| **Desktop** (`bande-desktop`) | Already done (drop-in + Recommends + oomd slice). Only a doc caveat in 0004. |
| **Debian installer** | Its business for d-i installs: a swap partition plus our zram (priority 100) in front. Acceptable. |

## Implementation steps

1. **linux**: append to `arch/arm64/configs/bande-headless.config`:
   `CONFIG_PSI=y`, `# CONFIG_PSI_DEFAULT_DISABLED is not set`, `CONFIG_ZSMALLOC=y`,
   `CONFIG_ZRAM=m`, `CONFIG_ZRAM_BACKEND_ZSTD=y`, `CONFIG_ZRAM_BACKEND_LZ4=y`,
   `CONFIG_CRYPTO_ZSTD=y` (the default compressor does not matter, because the drop-in sets zstd),
   `CONFIG_ZSWAP=y`, `# CONFIG_ZSWAP_DEFAULT_ON is not set`. Build on the VM, boot the mini, check
   `/proc/pressure/memory` exists and `modprobe zram` works. Check with `make savedefconfig` or the
   resulting `.config` that every symbol survived (names differ between kernel versions).
2. **images**: in `customize.sh`, add `systemd-zram-generator systemd-oomd` to the desktop
   `apt-get install` line (or a separate one). Verify in a built image that
   `systemctl status dev-zram0.swap` and `systemd-oomd` are active after boot.
3. **This mini** (main agent, with root): install the new kernel, upgrade `bande-desktop`, and
   `apt install systemd-zram-generator systemd-oomd`. Reboot. Check `zramctl` (zram0 about 8 GB,
   zstd), `swapon --show`, `oomctl` (app.slice monitored at 40 %).
4. **desktop**: in `docs/decisions/0004-memory-pressure.md` §1, add one sentence: with a disk
   swap partition below zram, Fedora Asahi saw premature OOM kills and moved to zswap. We do not
   add disk swap, and an admin who does should prefer zswap
   (`zswap.enabled=1`, no zram-generator). Add `CONFIG_PSI`/`CONFIG_ZRAM` to whatever kernel
   requirement list `docs/debian-readiness.md` keeps (it says nothing about this today).

## Status (2026-11-01)

1. **linux**: done. Commits "arm64: bande: enable PSI, zram and zswap" and the lockup
   detectors are on `bande-hw/integration`. Every line survives `defconfig` + fragment, and
   `Image modules` built with `kbuild`.
2. **images**: done. `systemd-zram-generator` and `systemd-oomd` (with PackageKit, locales,
   Noto, BlueZ and the PipeWire Bluetooth plugin) are on the `mmdebstrap --include` list in
   `build.sh` on `main`, the branch the image is built from. No image was built: that needs CI.
3. **This mini**: waits on the owner. Install a kernel built from `bande-hw/integration`, then
   `sudo apt install systemd-oomd` (`systemd-zram-generator` is already installed), upgrade
   `bande-desktop`, reboot, and run the checks in step 3.
4. **desktop**: done. The 0004 caveat and the kernel requirement in `debian-readiness.md`.

## Decisions for the owner

1. **Disk swap at all?** Recommend no. zram + oomd covers 16 GB, there is no hibernation on Apple
   Silicon, and the SSD sees no writes. The alternative is Fedora Asahi's swapfile + zswap (more
   headroom for very large builds, NVMe writes).
   Decided overnight 2026-11-01: no disk swap (plan recommendation; owner may revisit).
2. **Turn off `CONFIG_HIBERNATION`** in the headless fragment so nothing ever offers it? Low
   value, since logind already says `na` without swap. Recommend leaving it.
   Decided overnight 2026-11-01: leave it (plan recommendation; owner may revisit).

Sources: [1] https://www.debian.org/releases/trixie/arm64/apcs03.en.html
[2] https://discussion.fedoraproject.org/t/psa-transitioning-from-zram-swap-to-zswap/138256
