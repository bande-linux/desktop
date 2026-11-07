# Rendering: the hard stages

Plan only; nothing here is built. It continues [rendering-path.md](rendering-path.md), which owns stages 0-4. Delete items as they land.

Stack: Smithay 0.7.0, `ash` 0.38, Honeykrisp in Mesa 26.1.6, our kernel `patches/7.3`, DCP firmware 13.5 on system firmware 26.2. Asahi's distributions run KWin or GNOME on this same driver, so what those do is what Asahi does; its own work is cited where it exists.

Marks: **[ran]** measured for this plan on the M2 (Honeykrisp, Mesa 26.1.6, 2560x1440 unless stated, other agents' jobs alongside, load about 4). **[code]** read from our code or the driver. **[Apple]** Apple's documents. **[memory]** from memory, unverified. **[computed]** arithmetic from the others.

## Ambition

macOS is the bar for behaviour. On the same Mac we can honestly beat it in three places, and match it in the rest.

1. **Latency.** Apple's own model is a frame begun at one vsync and presented two vsyncs later, with the last pixel a vsync after that: 25 ms at 120 Hz, 33 ms when it falls back to triple buffering **[Apple]**. A commit here is designed to reach the next vblank, so we can run under one vsync where macOS plans for two. Today we don't: see the first measurement.
2. **A pipeline tuned to one machine.** We own the kernel driver, the compositor and the toolkit, so a frame can be planned from the first wake-up to the flip with no guessing between layers. The optimisation run can do what a general-purpose system can't.
3. **Consistency under load.** Today nothing protects the compositor from a build or a game. After C, K and, if firmware cooperates, G, it should hold through them.

Parity is the floor: colour correct everywhere, video through the display engine, HDR behind a per-display toggle, a pointer that never stutters. Where hardware rules something out, macOS is equally limited and this plan says so: Apple tells users not to use HDMI for Adaptive Sync **[Apple]**, so ProMotion-style refresh is not available to macOS either on this mini's HDMI port.

### Target numbers

"Today" is what we know. Nothing marked unmeasured may be quoted as a result. macOS figures for this monitor do not exist yet. Item B measures them with the same phone test, and until then the targets stand on Apple's model only.

| Metric | Today | Target | macOS reference | Measured by |
|---|---|---|---|---|
| Mouse motion to photon, pointer, mid-screen, 120 Hz, 1 kHz mouse | unmeasured; about 16 mean **[computed]**: 0.5 USB + 4.2 wait + 1 render + 2.4 DCP + 4.2 scanout + about 3 monitor; 8.3 more on each missed swap | p50 ≤ 15 ms, p99 ≤ 21 ms | unmeasured; same phone test | model + phone (B4) |
| Key to photon, Terminal glyph, mid-screen | unmeasured | p50 ≤ 20 ms, p99 ≤ 28 ms | model 25 ms + input + panel **[Apple]**; old 60 Hz measurements of 29-45 ms for terminals, 35 ms for Sublime ([Harris](https://dev.to/lkhrs/measuring-terminal-latency-26m7), [Hume](https://thume.ca/2020/05/20/making-a-latency-tester/)), not comparable hardware | phone, model |
| Key to flip, software only | unmeasured | p50 ≤ 10 ms, p99 ≤ 16.7 ms | none | trace |
| Client commit to flip | ≤ 1 refresh by design; **22 % of swaps miss** on the real display **[ran]** | p99 ≤ 8.33 ms | 2 refreshes **[Apple]** | trace |
| Compositor CPU per frame, typical | 1.3-2.0 ms p50 headless, includes the GPU wait (audit, 2928x1818) | p99 ≤ 1 ms, thread never blocked on GPU | none | trace |
| Render budget used, latch to commit, p99 | unmeasured on display | ≤ 5.6 ms at 120, ≤ 4.2 ms at 144 | none | trace |
| Compositor GPU, one window updating | 0.25 ms per full-frame quad **[ran]** | p99 ≤ 2 ms | none | GPU timestamps |
| Drag at 2560x1440, Glass, large damage | 8.1 ms headless median + up to 2 ms copy | p99 ≤ 4 ms | none | bench, trace |
| Refresh slots used while dragging, 120 Hz | **about 70 per second; 51 % of intervals were one refresh** **[ran]**, some of the rest may be nothing to draw | ≥ 99 % of intervals one refresh while content changes, at 60, 120 and 144 | none | `bande-flip-report`, B2 |
| Dropped frames during drag, scroll, animation | see above | at 120 Hz ≤ 0.1 % (≤ 1 ms/s) in all three styles. At 144 Hz ≤ 0.1 % Refined Solid and Wallpaper-Lit, Glass after O and D. At 60 Hz ≤ 0.1 % | Apple: ≤ 10 ms/s good in Organizer, its older talk says < 5 ms/s **[Apple]** | trace |
| The same, with a GPU hog and all CPUs busy | unmeasured | ≤ 1 % (≤ 8 ms/s) | unmeasured; run the same | bench + trace |
| Flip interval error, frames not dropped | callback p50 −0.03 ms, p99 +1.07 ms around a fit **[ran]**; reported time is a further estimate **[code]** | p99 ≤ 0.3 ms on the filtered clock | none | trace |
| Window behind pointer during a drag | same frame by design, unproved | 0 frames | same frame **[memory]** | trace + camera |
| Idle power, desktop at rest, SMC DC-in | 6.1 W baseline (plan path) | ≤ today, then lower with compressed scanout if it proves possible | Apple rates this mini at 7 W idle at the wall ([Apple](https://support.apple.com/en-us/103253)): a different meter, so not comparable | B5 |
| Idle wake-ups and redraws, compositor, at any rate | 0 by rule | 0 at 60, 120, 144 | none | `bande-idle` |
| 1080p30 HEVC in a window, above idle | software decode now, unmeasured | ≤ +0.5 W with AVD and plane | MacBook Air M2 18 h movie on 52.6 Wh: 2.9 W for the whole laptop with panel **[computed]** | B5 |
| Extra memory per 2560x1440 window | 14.7 MB **[code]** | 0 | 0 **[memory]** | `bande-footprint` |
| Pixels: sRGB on this monitor | oversaturated unless the monitor is in sRGB mode | matching a reference photo in the monitor's sRGB mode | correct **[memory]** | headless shader check + owner's eye |

### What will not be claimed

Latency above assumes a 1 kHz mouse. A 125 Hz mouse adds 4 ms on average. Panel response is the monitor's. A camera number compares only against the same camera on macOS.

## What I measured for this plan

Throwaway program, `build-run --here`, GPU only, no display access. Each number is p50 unless stated.

| What | Result |
|---|---|
| Full-frame blended quad, 120 Hz cadence, submit to `sync_file` wake | 0.62 ms (p99 0.76). GPU work 0.25, `vkQueueSubmit` call 0.07, submit to GPU start 0.29 |
| Export `sync_file` from a semaphore, poll it | works, 0.006 ms to export |
| Frame after 1 s or 3 s idle | 0.72 ms: **no meaningful GPU cold-start for small frames**. First frame of a burst 0.65-0.76 against 0.62 |
| 4 ms timer overshoot | p50 0.05 ms, p99 0.09, max 0.30, at load 4. **The 1 ms safety margin is about ten times what the timer needs** |
| Sample one client's buffer on a second Vulkan device, waiting on its `sync_file` with foreign-queue barriers | **correct pixels** for LINEAR, GPU_TILED and GPU_TILED_COMPRESSED |
| Sampling cost, GPU | LINEAR 0.42 ms, GPU_TILED 0.42, compressed 0.27: compressed saves 0.15 ms per full window |
| Dmabuf size | tiled 1.02x, compressed 1.04x of bare pixels |
| Colour shader, full frame, GPU | passthrough 0.28 ms, sRGB decode + matrix + encode 0.47 (+0.19), into RGBA16F passthrough 0.26, decode + matrix into RGBA16F 0.36 |
| Vulkan global priority | LOW and MEDIUM accepted; HIGH and REALTIME: `ERROR_NOT_PERMITTED`. The kernel rejects them with `TODO: Allow with CAP_SYS_NICE` (`asahi/file.rs`) |
| GPU contention, foreground 120 Hz frame beside another queue | Beside 2.5 ms jobs: 1.5 ms (p99 2.7). Beside 47 ms jobs: **46 ms**. Beside 149 ms jobs: **149 ms**. Medium or low priority for the other queue: **no difference**. So one long job holds the compositor's frame for its whole length: no preemption here. The hog is one render pass of many full-screen draws, harsher than a typical game's passes, but a real shader-heavy pass does the same on a smaller scale |

Also **[code]**:
- The compositor runs SCHED_OTHER at nice 0, all threads (`ps`). rtkit is installed, no capability set.
- `apple_drv.c` advertises `DRIVER_SYNCOBJ` and `DRIVER_SYNCOBJ_TIMELINE`, and Smithay's `DrmCompositor` passes an exportable render fence through `IN_FENCE_FD` when it can. We hand it `SyncPoint::signaled()`.
- `FrameClock::record_render` measures to the return of `submit`, so a commit's lead time before DCP is not budgeted.
- Flip time is an estimate. `dcp_crtc_send_page_flip_event` subtracts 0-1 ms from the callback's arrival. Sequence is always 0.
- `dc_swap_complete_resp` carries 0x6c5 opaque bytes, `swap_info`, nobody has decoded. `dcp_swap` carries `ts1`, `ts2`, `ts3`, `ts64_unk`, `surf_flags` and `active_regions`, all unused except fake timestamps on MacBook panels.
- `dcp_surface` has `is_tearing_allowed`, unused.
- The DCP firmware here is 13.5, system firmware 26.2.
- Smithay 0.7 has `fifo` and `commit_timing`; we enable neither. Mesa's Wayland swapchains use them when offered.

### First measurement: half the refresh slots are lost today

The owner ran B1 while moving two windows: 667 swaps in 9.5 s, `dcp:iomfb_swap_submit` to `swap_complete` **[ran]**.

**`swap_complete` is locked to the display.** The callbacks fit a grid of 8.3342 ms (119.988 Hz, as the mode timing predicts), residual p50 −0.03 ms, p99 +1.07 ms. The driver calls this the arrival of a callback with "unknown slack" after the flip **[code]**. Slack is unknown but a constant, so this grid is good enough to schedule by. It is not yet a photon time; the phone test calibrates that.

**There is a sharp cut-off.** Time from `swap_submit` to the next grid point, and how many swaps missed it:

| Before the next grid point | Swaps | Missed by a frame |
|---|---|---|
| 1.0-2.0 ms | 107 | 107 |
| 2.0-2.2 ms | 32 | 24 |
| 2.2-2.4 ms | 58 | 5 |
| 2.4 ms or more | about 460 | 1 |

(The 10 swaps sent under 1 ms before are also all late, and a few others at gaps longer than a frame.) The driver comment's 2.3 ms is right. The trace point fires when `swap_start` has returned, so the lead counted from the compositor's commit is longer by that round trip, which B2 measures.

**Where intervals went:** of 666 intervals between completes:
- 343 were one refresh.
- 132 were two, from a **missed cut-off**.
- 163 were two with no miss: the swap was sent over 9 ms after the previous complete, so the compositor skipped a slot itself, or had nothing to draw.
- 28 were longer: mostly idle moments.

**Hits are late too:** the median hit left 4.0 ms before the grid point, 1.75 ms more than needed on average.

**My read of the cause [code].** `dcp_crtc_send_page_flip_event` reports flip time as the callback minus up to 1 ms, and `FrameClock` anchors on that. It aims to finish `SAFETY_MARGIN` (1 ms) before its "vblank", so about 2 ms before the callback grid, then adds the commit: **our aim point is almost exactly on the 2.1-2.4 ms cut-off.** `record_render` stops at the return of `submit`, so nothing in the budget accounts for this lead. Separately, `predicted_render` is the max of 16, capped at 7.33 ms: after one big frame, `frame_for` finds the first deadline already past on wake-up and picks the next vblank, which would explain the 163 self-inflicted gaps. Only B2 can separate them from "nothing to draw".

This is a low-risk, high-value fix ahead of everything else. It's item T. **Drags today run at about half the display's rate, and it's mostly our own scheduling, not the GPU.** Expect the owner to see this one.

## Refresh rates

No rate is a constant anywhere. Clock, deadlines, animation and bench work from each output's mode and its measured flips, at 60, 120, 144 and anything else, on mixed displays and across mode changes.

**What this monitor offers [ran]** (read from the DRM connector without master; DCP's mode list reaches userspace unchanged): at 2560x1440, 60 Hz, 119.989 Hz (486.29 MHz) and **144.001 Hz** (581.65 MHz). 144 fits under HDMI 2.0's 600 MHz at 8-bit RGB **[computed]** and DCP offers it. Not yet tried on the current kernel from the plan's side; an older note records a compositor modeset to 144 Hz **[notes/display.md]**. The session runs at 119.989 now.

**How it is chosen today [code]:** the compositor gives an unconfigured display its preferred mode at its highest rate. Settings' Refresh rate lists the rates of the current resolution, 60-240 (`state.rs`, `REFRESH_RANGE`). That's already right; rate is a user's choice, no change. The EDID's preferred mode is 2560x1440@60; the "highest rate" rule then picks 144 unless the owner chose otherwise, and the owner has 120 set.

**Which constants remain [code]:** `state::REFRESH` is the virtual rate of the headless backend and the fallback for an empty mode line, used nowhere else. It stays for that. The bench takes `--refresh`.

**Budget per frame** at any rate, from the cut-off of 2.4 ms **[ran at 120 Hz, re-measured at each rate by the owner's command; it may scale]**, and about 300 µs for wake delay and pad:

| Rate | Frame | Left to render, latch to commit | Refined Solid drag at 1440p, after stage 1 | Glass today (8.1 ms headless at about 1.4x the pixels, unscaled) |
|---|---|---|---|---|
| 60 Hz | 16.67 ms | 14.0 ms | fits | fits |
| 120 Hz | 8.33 ms | 5.6 ms | fits | misses when damage is large |
| 144 Hz | 6.94 ms | **4.2 ms** | fits if p99 ≤ 4 ms: stage 1 gives 1.4 ms median, 3.0 p95 at a larger size | **cannot, needs O** |

So 144 Hz requires, on a 4.2 ms budget at p99:
- **Refined Solid:** stage 1's copy (done on its branch) and T. CPU ≤ 1 ms, GPU ≤ 3 ms. Should hold today's content.
- **Wallpaper-Lit:** the same, and 4.5 ms headless needs about 25 % off: 5's cheaper sampling and M's copy limited to damage.
- **Glass:** must drop to half or less. O's reuse for static backdrops (most frames), half-size start for dragged ones. A big drag under Glass still may not fit: D gives one extra frame of latency at full rate rather than stutter. Honest: at 144 Hz Glass will be the style that rests on D until O is done.
- **Copies:** stage 1 and M bring the copy to damaged area only. At 144 Hz, 0.45 ms of a full-frame copy is 10 % of the budget.
- **Stage 6 and C:** a wake-up late by 1 ms costs a quarter of the budget, so at 144 Hz C pays off the most.

Budgets are the output of the clock, not a constant to reach: it plans frames by what is measured, so a style that can't make 144 falls to every other refresh only if D is also out of room.

**Efficiency at rest.** The compositor's cost at rest is zero at any rate: no timer, no repaint, no callback without something asking. Undamaged frames stay undrawn. The display engine still scans out at the mode's rate (1.77 GB/s at 120, 2.12 at 144 **[computed]**, LINEAR), so a higher rate may cost something at rest in DRAM and link power, which only B5 can show. That needs a mode change at a time the owner chooses, and it's the honest answer to "does 144 cost anything": not from us, maybe from the wire.

**Mixed displays [code]:** each output already has its own scheduler and clock, and animations are closed-form on target time, so displays of different rates don't interact. Today's weak spot: one thread, so a frame on one output delays the other's start, until 6. A window spanning two gets callbacks from its primary. Mode change: `FrameClock` is replaced on a new refresh; T also resets its phase and keeps learned render costs. Client timing follows through `wp_presentation`'s refresh.

## Order

```
T frame scheduler (now, F1-F4)
B bench and trace, alongside ── everything after is measured with it
├─ C CPU priority
├─ G GPU priority (boot)
├─ I input resampling
├─ K cursor on a plane
├─ S animation fit
└─ stage 1 (a1e373) ── 6 no GPU wait ─┬─ D dynamic triple buffering
                                      ├─ 5 direct textures ── M shared image, shm
                                      ├─ O Glass reuse
                                      └─ W fewer wake-ups
stage 2 ── 7 colour ── 8 video on a plane (stage 3, AVD) ── 9 HDR (kernel)
V variable refresh: after DP alt mode lands.
```

| Item | What | Gain | Risk | Boot? | Size |
|---|---|---|---|---|---|
| T | Frame scheduler: F1-F4 | **Most lost frames in drags; 8.3 ms latency on each; client pacing** | low | no | S-M |
| B | Bench suite, frame trace | Everything else | none | no | M |
| C | Real-time compositor thread, CPU floor | Holds under builds | medium: lock-up | no | S |
| G | GPU priority, preemption | Holds under games | medium: firmware | yes | S patch, unknown firmware; may not work |
| 6 | No GPU wait | Thread free; enables D | medium: freeze | no | M |
| D | Dynamic triple buffering | Glass never hitches on spikes | low | no | S |
| 5 | Direct textures, tiled, compressed | Memory, 0.15 + 0.56 ms per full window | medium: tearing | no | M-L |
| O | Glass reuse | Glass under budget | low | no | M |
| W | Wake-ups per update | CPU, power | low | no | S-M |
| I | Pointer and scroll resampling | Smooth slow motion | low | no | S-M |
| K | Cursor on a plane | Pointer never stutters, even when the GPU is held | medium: firmware | no | M |
| S | Animation fit, curves | Feel | none | no | S |
| 7 | Colour management | P3 correct | low-medium | stage 2's | M-L |
| 8 | Video on a plane | Power, no GPU for video | high: firmware | 8b yes | M + kernel |
| 9 | HDR | HDR | high | yes | L |
| V | Variable refresh | ProMotion-style, DP only | high | yes | L |
| M | Shared image, shm | 45 MB; rarely matters | low | no | S-M |

## Safety used by every item

**Kill switches.** Each risky path has one environment variable, `BANDE_RENDER_NO_<ITEM>=1`, read at start, and `BANDE_RENDER_SAFE=1` turns them all off. `BANDE_RENDERER=cpu` already exists. No Settings switch: it would be a control the owner cannot judge.

**First-use latch.** Before the first use of a risky display path (plane colour tags, video or cursor on a plane) on a given kernel and firmware, the compositor writes `<path> <kernel> <firmware>` to `$XDG_STATE_HOME/bande/render-trial`, then removes it after 300 good flips. A file still there at the next start means that path just killed the session or the machine: that path stays off for that kernel and firmware, the journal says why, and the owner sees it in fault reports. This prevents a black-screen loop with no keyboard.

**Runtime fallback.** `FrameRecovery` already escalates at 1 s (flip timeout), 3 s (restart display), 15 s (give up). Add a step: when a stall starts within 2 s of a newly enabled path, turn that path off for the session before restarting.

**A DCP crash** kills the display until reboot, and only `flip_done timed out` shows. Recovery is over SSH or the power button; the latch prevents a repeat. Only main reboots.

**A bad test boot.** A test kernel is chosen in the boot menu and the running kernel stays the default, so holding the power button and booting normally gets back to a good system. hvc0 stays the console. A boot request goes to main with the commit and the checks. Never unload `appledrm`.

## Where each proof can run

| Place | What it can prove | What it cannot |
|---|---|---|
| Headless on the M2 (`build-run --here`, time limit) | Honeykrisp behaviour, fences, imports, modifiers, shader costs, GPU priority (with the kernel patch booted), real clients, frame state machines | DCP, planes, flips, colour on glass |
| Build VM (no Apple GPU; lavapipe, `vkms` for atomic logic) | Protocol code, trace format, state machines, `TEST_ONLY` fallback logic, kernel patches build and boot in QEMU | Anything about timing or Honeykrisp |
| Owner's session, real compositor, no boot | The DCP trace, frame trace, cursor and video trials, watts, eyes | Kernel changes; needs a time they choose |
| Owner test boot | Kernel patches on DCP and firmware | Runs only through main |

## B. Bench suite and frame trace (first)

**What a user feels:** nothing. It decides what everyone else does. The owner's optimisation run gets a number to push against, and a regression gets caught.

**macOS:** Instruments' Hitches and Display templates; `CADisplayLink` gives begin, commit and presentation times **[Apple]**. We match this with a trace.

**Others:** KWin logs frame statistics to a file ([MR 5975](https://invent.kde.org/plasma/kwin/-/merge_requests/5975)). Mutter has its profiler (from memory). A plain per-frame record file is the proven shape.

### B1. First step: read DCP timing on the running system

Done once, results above. `bande-flip-report` parses it now. The same one line stays the re-measure for T, as root, while moving something, needing no restart:

`perf record -a -e dcp:iomfb_swap_submit,dcp:iomfb_swap_complete,dcp:iomfb_swap_complete_intent_gated -- sleep 10`

It fits the grid with no rate assumed, then prints the table above: misses by distance to grid, slack on hits, interval multiples. No `intent_gated` events appeared on HDMI. Needs root, so each run needs the owner's yes until they say otherwise.

### B2. Frame trace in the compositor

One fixed ring of about 4096 records of about 96 bytes, written by the frame scheduler with no allocation and a single branch when off. Per frame:
- target vblank, wake, latch, record end, submit, GPU start and end (timestamps through `VK_KHR_calibrated_timestamps`, which Honeykrisp has), commit, flip event, flip time
- damaged and blurred pixels, elements per plane, missed flag
- per client surface: commit time to latch, and input event time for the frame it answered, when known

Started and stopped on `org.bande.compositor` over the session bus, writing NDJSON, so a running session is traced without restarting it. `bande-frame-report` turns it into percentiles, hitch ms/s and a Perfetto file. Adding kernel tracepoints later merges on `CLOCK_MONOTONIC`.

Cost target: under 1 µs per frame on, one branch off. Proof: same bench with and without.

### B3. Workloads

Extend `bande-session-bench`, keeping its private bus and headless compositor. `--refresh` for each of 60, 120, 144 and a mixed pair, at 2560x1440 scale 1 and 2928x1818 at 1.5, plus a mode change mid-run.
- Scenarios it has: pointer, drag, typing, scroll, launcher, overview, menu, banner. Add a window close and minimise animation, a full-screen video client, and a Firefox-like client that commits a LINEAR dmabuf at 60 Hz.
- For each: three styles.
- Load variants: quiet; `gpu-hog` (a client submitting 16 ms jobs); `cpu-hog` (one thread per CPU at nice 0).
- Input is the existing virtual input with a recorded human trace.

`bande-bench run --scenarios … --out results.json` runs each, N rounds. It records the commit, load average, Mesa and kernel versions. It refuses to report as valid when load stays above a limit, as the bench notes did.

Results stay **outside the repo** (`~/bande-linux/bench-results/`), per the no-measurement-histories rule.

`bande-bench compare a.json b.json` bootstraps a confidence interval on p50 and p99 and prints only differences it can separate from noise. `--gate` exits non-zero past a set threshold. The optimisation run's loop is then one command: change, `build-run --here … bande-bench run`, compare, keep or revert. It needs a quiet GPU, so it takes `kbuild.lock` and says so if other work disturbed it.

**CI:** the GitHub runners have no Apple GPU and CI runs on tags only. CI runs the trace format under lavapipe to check that it parses, nothing about timings. The mini is the gate, as for everything else.

### B4. Real-display numbers

No purchases, and nothing depends on special hardware. Two levels that add up:

- **Trace-based model, automatic.** Input event time to flip comes from the trace; the flip's place on the grid and `bande-flip-report` give the rest of what we control. Photon time adds half a scanout at mid-screen (half a refresh, from the mode) and the monitor's constant delay, taken from the phone below. It's repeatable and can run on any commit, once the owner agrees to trace.
- **Phone at 240 fps, with a visible key press, on Bande and on macOS.** Frame the key and the screen; count frames from the key bottoming out to the glyph, 4.2 ms each. Ten presses in Terminal at 120 Hz; the same with a mouse button and a menu. Ten minutes. Good to ±2-4 ms per press and about ±1 ms on the mean of ten, and it includes the keyboard's own delay, so it compares only against the same run on macOS: a browser page or Terminal on the same monitor and port. A macOS boot once is fine by the owner, later. The phone also calibrates the model's constant.

A photodiode rig would be sharper but is not planned: the owner isn't buying one now.

### B5. Power

SMC DC-in updates at 1 Hz and scatters by 1 W. Paired rounds, mode against idle, 60 s each, 10 rounds, gives about 0.1 W standard error **[computed]**. A plug meter on the wall gives the same comparison with macOS and is optional. Display brightness is the monitor's, unaffected.

### Risks and first proof

| Risk | Failure | Mitigation |
|---|---|---|
| Measuring disturbs | changed timings | off by default, cost measured; ring no-alloc |
| Bench noise from other agents | false results | load gate, `kbuild.lock`, confidence intervals |
| Real trace needs a new compositor on the owner's session | none until they update | ships as a normal release; no run needed before |

**Proof:** report on one existing scenario, headless. **Size:** M (about 800 lines, no new dependencies). **Order:** first, alongside stage 1.

## T. Frame scheduler

Authorised by the owner to be built, in stages, each with the kill switch `BANDE_FRAME_TIMING=classic`, which keeps today's rule until the new one has proved itself on the real display.

**What a user feels:** dragging stops running at half rate. Typing and pointer respond up to a frame sooner. At 144 Hz the desktop actually reaches 144.

### What others do, read from source

Pinned: KWin `1bc2e471`, Mutter `b766717a`, sway `1652c54b`, wlroots `1dedee12`, cosmic-comp `46b26e63`, niri `9955d921`, Hyprland `ab614189`, gamescope `309eee74`, AOSP and Mesa `main`, on the day of writing.

| System | Render prediction | Commit margin | Learns from a late flip? | Clients |
|---|---|---|---|---|
| [KWin](https://invent.kde.org/plasma/kwin/-/blob/1bc2e471/src/core/renderjournal.cpp) | EMA (τ 500 ms) + 2 × overshoot variance, peaks instantly, decays τ 6 s; sample counted from scheduled wake, so wake lateness is in | [1 ms + vblank + adaptive 0-3 ms](https://invent.kde.org/plasma/kwin/-/blob/1bc2e471/src/backends/drm/drm_commit_thread.cpp#L167), on ioctl return only | no; [its own MR 8517](https://invent.kde.org/plasma/kwin/-/merge_requests/8517) calls the journal too conservative after spikes | callback right after painting; triple buffer with 10-frame exit |
| [Mutter](https://gitlab.gnome.org/GNOME/mutter/-/blob/b766717a/clutter/clutter/clutter-frame-clock.c#L551) | Asymmetric max and min filters: jump up past a margin, down slowly; retries with the min estimate before skipping a slot | [estimate + 800 µs + dynamic margin up to refresh/4](https://gitlab.gnome.org/GNOME/mutter/-/blob/b766717a/src/backends/native/meta-kms-crtc.c#L925) | no, missed counts are debug-only | callback timer at the frame deadline when idle; triple buffering while estimate ≥ a refresh |
| [sway](https://github.com/swaywm/sway/blob/1652c54b/sway/desktop/output.c#L324) | none: user's `max_render_time`, whole ms | same | no | per-view delay, manual |
| [cosmic-comp](https://github.com/pop-os/cosmic-comp/blob/46b26e63/src/backend/kms/surface/timings.rs#L366) | mean of last 5 + 3 ms | in that 3 ms | no | at queue and vblank, once per sequence |
| [niri](https://github.com/YaLTeR/niri/blob/9955d921/src/frame_clock.rs#L56) | none: draws at once after the vblank | none | no | once per sequence; timer at estimated vblank, idle clean |
| [Hyprland](https://github.com/hyprwm/Hyprland/blob/ab614189/src/output/MonitorFrameScheduler.cpp) | fence-based triple buffering, off by default | [2 ms fixed lead](https://github.com/hyprwm/aquamarine/blob/42c39606/src/backend/drm/DRM.cpp#L3217), off by default | logged only | at frame event |
| [gamescope](https://github.com/ValveSoftware/gamescope/blob/309eee74/src/vblankmanager.cpp#L150) | instant attack, 98 % decay; 2.4 ms floor | 1.65 ms redzone | no | fifo: one commit per vblank |
| [Android](https://android.googlesource.com/platform/frameworks/native/+/refs/heads/main/services/surfaceflinger/Scheduler/) | app and SF work durations per device | `vsync − work − ready` | classifies jank, no learning | Choreographer: input, animation, draw; timeline with deadline and expected present; regression on 20 HW vsyncs, outlier if 20-80 % of a period off |
| Apple **[Apple]** | not published | not published | not published | `targetTimestamp`, deadline backward from present; events either at once or at begin time |

**Where we go beyond them.** No compositor above learns from a swap that slipped after the ioctl returned; all pick a generic margin. A driver with a hard cut-off defeats them: that's the half rate the Asahi driver comment saw under KWin. We can observe the slip directly: which vblank the flip landed on, against the one we aimed for. That's the new part, and the part that makes this display reach full rate.

### Design: `pacing.rs`, pure logic, plus `clock.rs` keeps the state machine

1. **`VsyncEstimate`:** phase and period per output, from flip times only, Android's idea at small cost: a second-order filter on the error against the predicted grid (phase gain 1/16, period gain 1/256, within 1 % of the mode's nominal). Errors over 1/8 of a period are outliers (the real trace: p99 +1.08 ms, which at 120 Hz sits just at this limit), snap after 8 in a row, as after a monitor glitch. It owns `sequence`, so clients get a real count; the kernel's is always 0. Clients get grid time, not the noisy event time. Starts from the mode; nothing assumes a rate.
2. **`RenderPrediction`:** per output, in time from wake to commit returning.
   - Budget = p95 of the last 64, or the max of the last 4 if higher (instant attack, a spike forgotten in 4 frames not 16), plus p99 of wake delay over 64, measured against what we asked for, plus a pad.
   - Pad: +100 µs each time a commit came too late for its slot, up to a quarter of a refresh, −10 µs per 64 good frames, so the margin is earned, not fixed. On a loaded machine wake delay rises by itself, which answers the old doc's reason for choosing the max.
   - Heavier content such as the overview opening costs one miss, as today.
   - A style or workload change settles in 4 frames. **Classes by damage size** go in only if the bench shows a benefit (F4).
3. **`CommitLead`:** how far ahead of its slot a commit must return, learned per output by what flips do:
   - Starts at 35 % of a refresh on the first real flip (headless, without flips, stays 0).
   - While learning, each hit lowers it by 50 µs until the first miss, which costs one frame once per session and output.
   - A miss with time left ≥ lead raises it to `left + 100 µs`, at most +300 µs per event, so a rogue miss (one was seen at 7.6 ms left) can't blow it up. A miss with less time left than lead was our lateness, not DCP's: it goes to pad.
   - After 1024 hits, −20 µs, to track changes: a probing miss about every 40 s of constant drawing, under 0.03 % of frames.
4. **Slot choice:** `start = slot − budget − lead`; a wake past that may still go if `now + typical + lead ≤ slot`, as Mutter's min-estimate retry. Otherwise the next slot. This removes the self-inflicted half rate.
5. **Late input:** a timer wake dispatches all ready sources in one loop pass, including libinput, before `poll` runs **[code]**. So the latch sees the latest input as long as it starts as late as it safely can. Nothing else is needed; a separate thread is later work (K).
6. **Idle:** no timers unless `wanted`, or a client asked a callback with no damage (one wake at the slot, as niri). Same rule at any rate. Phase holds across idle, since the link runs free.
7. **Mode change, mixed outputs:** `FrameClock` per output, replaced with the new mode (keeps the render prediction, drops phase and lead). Outputs share one thread until stage 6: a frame on one delays the other; pad handles what's left.
8. **Repeated misses:** lead and pad raise themselves; if a window of 128 frames still shows over 5 % missed, render one slot later: a fixed pipeline depth would be D. Logged once. `BANDE_FRAME_TIMING=classic` restores the old rule in full.

### Stages

| # | What | Done when |
|---|---|---|
| F1 | `bande-flip-report`, `pacing.rs`, slot choice, sim regression test at 60, 120, 144 and cut-offs | merged; simulator baseline shows old failing, new passing |
| F2 | Real session re-measure by the owner's command; honest `wp_presentation`: `hw_completion`, grid time, sequence | misses < 1 %, one-refresh intervals > 95 % |
| F3 | Client callbacks timed per surface | key to flip p50 ≤ 10 ms, no client hitches |
| F4 | `fifo-v1`, `commit-timing-v1` (P); classes if bench asks | Mesa's clients pace |

**F3, callbacks.** Today at the flip, so a client's work competes with `lead + render` already gone (at 144 Hz only about 3.5 ms left). KWin sends right after painting (earliest, full budget, stale input). Mutter at the frame deadline (latest, for idle clients). We send after our commit for clients rendering at full rate, which gives the budget, and at `next start − client's own commit-after-callback p95 − 500 µs` for input-driven ones, starting from "after commit" and only delaying once its history is stable, backing off for a second after any late commit. Hidden surfaces get none, as the spec asks.

**Occlusion, per Mesa:** its FIFO path relies on `discarded` and barrier clearing for occluded clients, or `presentWait` degrades; F4 must drain them at a virtual refresh.

### How it's measured

Same command as the baseline, while moving two windows for ten seconds:

```
sudo perf record -o /tmp/flip.data -a -e dcp:iomfb_swap_submit,dcp:iomfb_swap_complete -- sleep 10
sudo perf script -i /tmp/flip.data | bande-flip-report
```

**Baseline `dcp-timing.txt`, through the tool, at 119.988 Hz:** 667 swaps in 9.5 s, 22.0 % over a refresh, 21.7 % missed the next flip; between flips: 51.5 % one refresh, 21.2 % missed, 24.8 % late or nothing to draw; 4.0 ms left at p50 on hits.

**Headless, in the gate:** the simulator in `clock.rs` tests drives the real logic against a display with a cut-off. Virtual time, at 60, 120, 144, a mixed mode change, render spikes and a cut-off of 2.3 ms. The old rule must miss, the new must not. It's a regression test for a real bug, allowed. `bande-session-bench --refresh` covers the true compositor headless, without DCP.

**Risks.**

| Risk | Failure | Mitigation |
|---|---|---|
| Learning misses at start | one lost frame per session | accepted; unnoticeable |
| Lead too small after a change in load | a few misses | raises at once; `lead` is per output |
| A hung flip | as today | `FLIP_TIMEOUT`, recovery unchanged |
| New logic wrong in the field | judder, half rate | `BANDE_FRAME_TIMING=classic`, a session restart away; no kernel change |
| Sim doesn't match DCP | good sim, bad display | owner's command is the gate, not the sim |
| Stage 1 changes draw time | moves numbers | learned, not constants; a1e373 told; re-measure once both landed |

**Target:** at 60, 120 and 144 Hz: under 1 % misses, then ≤ 0.1 %, one-refresh intervals > 95 % while content changes; left-over time on hits ≤ 1 ms above the cut-off; p99 flip error ≤ 0.3 ms. **Size:** S-M, under 400 lines plus tests. **No kernel change** in F1-F4.

### E1. DCP experiments (kernel, one test boot, batched)

Known: no `intent_gated` callbacks on HDMI **[ran]**; `swap_info` unread; `ts1-3` count CPU ticks one frame apart under macOS ([Asahi 6.19 report](https://asahilinux.org/2026/02/progress-report-6-19/)).

- Dump `swap_info` for 200 swaps via a debug tracepoint, on a test kernel. Fields moving by about 8.33 ms in timer ticks are timestamps. Compare with the callback's time.
- Set `ts1-3` to the real target (next vblank in ticks) on HDMI: does the flip wait for it? If yes, we have **presentation at a chosen time** without VRR, valuable for video.
- Why the cut-off is 2.1-2.4 ms, and if it can move. It might be DCP's own processing (fixed), or a programmed window, in `set_frame_sync_props` (kernel returns empty) or `dcp_swap` flags. A smaller cut-off is straight latency. Compare with what macOS sends, via m1n1 traces if the owner gives it time.

Failure: DCP may reject odd values. Mitigation: only on a test boot; values are ones macOS sends. **Size:** S for the dump.

## C. Real-time compositor thread and CPU floor

**What a user feels:** during a kernel build, the desktop still moves evenly. A first keypress after idle isn't slower.

**macOS:** QoS classes with user-interactive at the top **[Apple docs, from memory]**, WindowServer at raised priority **[memory]**.

**Others:** KWin takes SCHED_RR via `CAP_SYS_NICE` (from memory). Mutter's KMS thread goes real-time through rtkit (from memory).

**Design:**
- The compositor main thread gets SCHED_RR priority 1 with `SCHED_RESET_ON_FORK`, via rtkit over the bus (installed), falling back to nothing. It needs `RLIMIT_RTTIME`, which rtkit requires anyway. Exceeding it sends SIGXCPU: logged and dropped to SCHED_OTHER, not killed.
- Children and other threads stay normal.
- `uclamp.min` on the compositor (and on the focused app's UI thread through the toolkit, allowed without privilege within the limit) so a wake doesn't start at the lowest frequency; `schedutil` otherwise ramps after the frame is over. Verify on `apple-cpufreq`.

| Risk | Failure | Mitigation |
|---|---|---|
| Compositor spins at RT | one CPU stolen; with 8 CPUs, not a lock-up | RTTIME limit; RR priority 1; existing watchdog |
| Blocking I/O at RT stalls others | priority inversion | trace shows; audit the main thread's file I/O first (recall already has its own thread) |
| rtkit refuses | stays normal | runs as today; logged once |
| Higher power from uclamp | watts up | B5 before and after; scope to active interaction |

**Proof:** headless, `cpu-hog` variant, private session on the mini; rtkit works for any session. **Target:** drops under `cpu-hog` ≤ 1 %. **Size:** S.

## G. GPU priority for the compositor

**What a user feels:** a game or a heavy WebGL page doesn't take the desktop down with it. The pointer and windows keep moving.

**macOS:** WindowServer's GPU work outranks apps, and the GPU preempts **[memory]**.

**Others:** KWin asks for high-priority contexts where the driver allows (from memory). Linux drivers gate this on `CAP_SYS_NICE`. Asahi's driver has the plumbing and the `TODO`; nobody has reported using it.

**The problem is bigger than the plan assumed [ran]:** today a frame beside a 47 ms job takes 46 ms, and low priority on the other side changes nothing. A heavy shader-only page takes the desktop to about 20 Hz. It's the difference between a machine that feels strong and one that doesn't.

**Design.**
- Kernel: finish the `TODO: Allow with CAP_SYS_NICE` in `asahi/file.rs` for HIGH (the rest is plumbed: `new_queue` takes a priority and passes it to firmware).
- Compositor: queue with `VK_QUEUE_GLOBAL_PRIORITY_HIGH`, falling back to MEDIUM on `ERROR_NOT_PERMITTED`. Needs `CAP_SYS_NICE` on the binary, by `setcap` in the package, which C could use too.
- **If firmware doesn't preempt on priority:** the firmware structures have preemption buffers and `no_preemption: false` **[code]**, so it can stop a render between tiles, but what triggers it isn't known. Look at how macOS submits with m1n1 tracing, and what `drm_asahi` fields set it. Last resort: the kernel splits nothing, but Mesa can be asked to break long passes on Honeykrisp in a Debian-compatible way upstream.
- **Regardless:** K keeps the pointer moving with no GPU at all, and direct scanout and planes keep video moving. Those two are what a hog cannot touch.

| Risk | Failure | Mitigation |
|---|---|---|
| Firmware priority untested: comment says "Reserved for future extension" | **GPU firmware crash, everything frozen until reboot** | test boot only, this plan's contention program first, then session; kill switch; first-use latch |
| Priority without preemption | no gain, jobs still queue | the contention program settles it in one boot; then K and research above |
| Capability makes the binary `AT_SECURE` | `LD_*` ignored, debugging harder | acceptable; check nothing relies on it |

**Experiments, one test boot:** the patch, then contention program with foreground HIGH and REALTIME, hog at 47 and 149 ms. Lines in the scratchpad `plan-rendering-hard/proof`, to be moved into B3's `gpu-hog`.

**Target:** compositor frame p99 ≤ 4 ms beside a 47 ms hog; if not reachable, pointer unaffected through K, and an honest note. **Size:** S patch + one boot; research open-ended, time-boxed to two days. Batch with E1.

## 6. Stop the compositor thread waiting on the GPU

**What a user feels:** little on a light frame: it's 0.6 ms **[ran]**. It matters when frames are heavy (Glass, 8 ms): input isn't held, client commits aren't held, and a slow frame doesn't lose the next one. It's the base for D and for honest timings.

**macOS:** render server's CPU part stays in one vsync, the GPU part may overlap the next frame **[Apple]**. Beside a GPU hog (see G) a blocked thread would also stop input for the whole job; this removes that.

**Others:** all three use fences. wlroots and KWin hand `IN_FENCE_FD` to KMS or wait on a sync file; KWin's commit thread blocks instead of the main thread (from memory).

**Design.** Replace `submit_and_wait` in `VulkanFrame::finish` with:
1. N frames in flight (2, then 3 with D): one command buffer, fence, semaphore per slot; staging, garbage, blur-chain retirement keyed by slot, freed on that slot's fence.
2. Submit, export `SYNC_FD` from the semaphore (proved, 0.006 ms), and return a `SyncPoint` around it, implementing Smithay's `Fence` trait (`is_signaled`, `wait`, `export`).
3. `drm.rs` splits `submit` in two: `render_frame`, then a calloop `Generic` source on the fd; on readable, `queue_frame`. `FrameScheduler` gets `Phase::AwaitingGpu(vblank, since)`. Render time is recorded at the fence.

**Why not `IN_FENCE_FD`,** which the kernel supports and Smithay would use on its own? A fence that never signals would hang inside `drm_atomic_helper_wait_for_fences` in the commit worker, with no way to abandon the frame, and next commits get `EBUSY`. The event-loop wait can abandon the frame and recover, costs a thread wake of about 50 µs, and shows us when GPU work finished. Pass Smithay a signalled point. Compare later with the bench; the gain is at most the wake.

**Must stay synchronous:** readback for tests and screenshots, lavapipe's CPU copy to the display buffer, captures reading CPU memory. `copy_captures` follows the frame in submit order.

| Risk | Failure | Mitigation |
|---|---|---|
| Fence never signals | frozen screen with input alive | `AwaitingGpu` limit of 1 s, set `lost`, existing `renew_failed_renderer`; bench injects a never-signalling fence |
| Image reused while GPU uses it | tearing, corrupt frame | slot lifetimes; validation layers in a bench run |
| Client buffer released early | tearing in a window | release after the flip that replaced it (`latched_releases`) |
| Texture freed while in use | GPU fault, device lost | garbage by slot; validation |
| A second frame starts while first runs | frames out of order | scheduler allows one until D |
| Honeykrisp semaphore export bug under load | freezes | soak (`bande-soak`), 1 h; fallback flag `BANDE_RENDER_NO_ASYNC` |

**Proof headless:** soak on the M2 with `--validate`; virtual-time test of the state machine (hung fence). **No boot;** a real session update, with the switch. **Target:** thread blocked on GPU 0 per frame; no drop change; CPU per frame ≤ 1 ms p99. **Size:** M. **First proof:** the fence type on `bande-session-bench`.

### D. Dynamic triple buffering

**Feel:** a slow frame costs one frame's latency instead of a visible stumble, as Apple and GNOME both do.

**Design:** after a frame is late (fence after deadline), allow two frames in flight until 30 frames are on time; budget may then exceed one refresh. Smithay's `queue_frame` already holds a `queued_frame` behind a pending one **[code]**. Double buffer normally: no latency cost when keeping up, exactly the rule in the Ubuntu debrief **[from search]**.

**Risk:** latency stuck up: exit rule above, trace shows. **Target:** zero dropped frames during Glass drag at full damage, at most +8.33 ms latency only while recovering. **Size:** S after 6.

## 5. Sample window buffers directly, then tiled and compressed

**What a user feels:** less memory per window (14.7 MB of 1440p gone), clients render a little faster, a little more headroom. Without it, Firefox and video are always a copy behind.

**macOS:** IOSurface is shared across processes and GPU without a copy (from memory; [Apple doc index](https://developer.apple.com/documentation/iosurface)). Apple's GPU-native surfaces are lossless-compressed where it can (from memory).

**Others:** KWin and Mutter import dmabufs directly as textures, and offer tiled and compressed modifiers through per-surface feedback. wlroots likewise. None copies.

**Design.**

1. **5a: LINEAR direct.** `import.rs` makes one `VkImage` per dmabuf with `DRM_FORMAT_MODIFIER_EXT` and keeps it for the buffer's life. Reuses stage 3's holding. Acquire barrier from `QUEUE_FAMILY_FOREIGN` each commit, layout `GENERAL` to `SHADER_READ_ONLY`, as proved. Damage needs no copy, though the compositor's own damage stays as it is.
2. **Hold rules.** A buffer is held until the flip replacing it, and until the GPU finished any frame using it. `hold_replaced` today skips surfaces with no frame callback **[code]**; direct import must hold always. Closing animations and stretch-on-resize hold a `Buffer` instead of a copy. Clients now must have 3+ buffers; Mesa WSI has them, and check the toolkit's swapchain.
3. **Sync.** Syncobj acquire and dmabuf-poll blockers already delay the latch **[code]**. Confirm Honeykrisp's WSI uses `linux-drm-syncobj-v1` with a logged client, first thing. Release point signalled at the flip through `Buffer` drop.
4. **5b: modifiers.** `linear_formats` becomes the modifiers Honeykrisp offers, sampled. Validate: import failure at `create_params` gives the client an error before anything draws.
5. **Per-surface feedback.** Default tranche: compressed first. Scanout tranche: LINEAR for layer surfaces and full-screen windows (GPU_TILED cannot scan out, and Mesa has no INTERCHANGE), which keep plane 2 and direct scanout. Windows move between by `surface_feedback` on full-screen. Old buffers keep working.
6. **Keep the copy** as the fallback for anything failing import, and for the CPU renderer.

**Security:** a client's dmabuf reaches GPU sampling directly, so `lseek(fd, END)` must cover the layout before import, exactly (`vkGetImageMemoryRequirements`). A bad layout would otherwise fault the compositor's own GPU context. `fuzz/` gets a case.

| Risk | Failure | Mitigation |
|---|---|---|
| Client writes while sampled | **tearing** | hold rules; tear test: client alternates two full colours, compositor captures, asserts no mix, runs in the bench with explicit and implicit sync |
| Undersized or misaligned buffer | GPU fault, **device lost**, session black then recovered | size check; existing renewal; fuzz |
| Compressed metadata mismatch | **garbled window** | proved correct across devices **[ran]**; Mesa version pinned by Debian; `BANDE_RENDER_NO_TILED`, feedback goes back to LINEAR |
| Client stalls for a free buffer | latency up | check buffer count in the trace; toolkit gets one extra if needed |
| Client's fence never signals | that window stops only | blocker is per surface (exists) |
| Memory counted twice in the kernel | oversize RSS | `bande-footprint` |
| Firefox rejects new feedback | black tab | test with gap-firefox's widget; LINEAR stays offered |
| Feedback churn on full-screen | one flash | a client's own swapchain recreate; test; delay until settled |

**Proof headless:** all of it except plane behaviour, on the M2, real clients. No boot. **Target:** per-window extra 0; sampling full window 0.27 ms, client render −20 %. **Order:** 6 first, for lifetimes; stage 3 first, for the shared holds. **Size:** 5a M, 5b S-M. **First proof:** tear test on 5a.

## O. Glass: reuse the blur

**What a user feels:** Glass stops being the slow style. It's the look that makes the desktop feel expensive; it must not be the one that stutters.

**macOS:** materials blur the backdrop in the render server at reduced resolution (from memory). Nothing published.

**Others:** KWin's dual Kawase re-blurs damaged areas with margins; Mutter's blur is similar, with caches (from memory).

**Design.** Today each frame touching `Blur::bounds` filters the whole region, 2×passes **[code]**.
1. **Reuse:** keep each chain's top level, keyed by region and backdrop version. Skip filtering when damage *below* this element in the stack misses `bounds`. Content *on* the panel doesn't invalidate it, which is the common case.
2. **Partial filter:** when it does hit, refilter only damage + reach, per level.
3. **Start at half:** copy into `levels[0]` at half size with a linear blit. Quality checked by eye against today, Glass's noise hides it.
4. **Levels in RGBA16F** (same speed **[ran]**), which removes banding; dither where it joins the 8-bit target.

| Risk | Failure | Mitigation |
|---|---|---|
| Stale backdrop | wrong colours behind glass | damage-tracker based; debug flag re-filters and compares in bench |
| Dragging a window under glass | no gain | correct, full cost; D catches it |
| Quality drop at half size | visible softening | owner's eye; keep passes |

**Target:** drag under glass p99 ≤ 4 ms at 1440p; static panel over changing content under 0.5 ms. **Proof:** headless with `bande-session-bench` in Glass. **Size:** M. Measure after stage 1.

## W. Wake-ups per client update

Ten per update today (plan path). **Feel:** only power and CPU. **Design:** count them with B2, attribute each. Expected: frame timer re-arm on each commit, separate flip, release and callback sends. Coalesce sends at the flip into one flush, one timer per output moved only when the deadline moves. Toolkit: one wake per callback, nothing for releases it doesn't need. **Risk:** a delayed release blocks a client: test with 2-buffer clients. **Target:** ≤ 4 across compositor and client. **Size:** S-M. **Note:** there's no point guessing the list before the trace.

## P. `fifo-v1` and `commit-timing-v1`

**Feel:** Vulkan apps and games pace properly and don't hang when hidden. Video lands on the right frame.

**macOS:** `CAMetalDisplayLink` targets, `present(afterMinimumDuration:)` (from memory).

**Today:** absent, though Smithay 0.7 has both modules **[code]**. Mesa uses them on Wayland when offered, since 24.3 ([Phoronix](https://phoronix.com/news/Mesa-24.3-Vulkan-FIFO)). Honeykrisp lists `VK_EXT_present_timing`, which needs them.

**Design:** delegate both, hook the barriers into `wait_for_fences`' blockers, release at flip. **Risk:** an occluded FIFO client must still advance: the protocol's rule on occlusion, tested with a hidden client. **Size:** S. **Proof:** `vkcube`-like headless.

## I. Pointer and scroll resampling, prediction

**What a user feels:** slow pointer and scroll motion is even.

**macOS:** Coalesced and predicted touches exist on iOS **[memory]**; desktop pointer behaviour unpublished. Users do call macOS's pointer floaty ([Apple forum](https://discussions-kr-prz.apple.com/thread/256169613), anecdote only).

**Problem [computed]:** a 125 Hz mouse at 120 Hz gives 0, 1 or 2 events per frame on a slow beat, so displacement per frame jitters by a whole sample. 1 kHz mice are fine. Trackpads via Bluetooth are in between.

**Design:**
- **Position sampled for the latch:** when samples are coarser than a frame, render at `presentation − 1 sample period` on the line through the last two. Clients and hit-testing keep real events, so nothing logical moves.
- **Prediction:** extrapolate only when acceleration is low, capped at one sample, and off by default until bench and the owner's hand agree. It's the one thing that can feel wrong: overshoot on stops.
- **Scroll:** toolkit samples finger scroll at `frame_time.presentation` from a short filter of deltas, not one per event. Momentum is closed-form already.
- **Ballistics:** libinput's curve differs from macOS's. A `custom` accel table is a small, separate job, fitted by feel against macOS.

**Risk:** added lag up to one sample. **Measure:** offline on a recorded trace: frame-to-frame displacement RMS against latency. **Size:** S-M.

## K. Cursor on a plane

**What a user feels:** the pointer never stutters, even when everything else does. It's the most-felt single thing on a desktop, and macOS gets it from a hardware cursor **[memory]**; one report ties macOS's own lag to falling back to a software cursor ([ifun](https://www.ifun.de/macbook-neo-tool-beseitigt-mauszeiger-verzoegerungen-unter-macos-283182/), a suspicion, not proof).

**Today:** the cursor is drawn into the frame, so GPU work and a copy for every motion **[code]**.

**DCP:** no cursor plane: `apple_drv` assigns primary at zpos 0 and overlays above **[code]**. Blended surfaces under 32x32 crash firmware, and the kernel rejects planes off screen. Overlays work for a 341x1440 surface today; small moving surfaces are untried.

**Design.**
- 64x64 dmabuf cursor on plane 1 or 2, with transparent padding (so the minimum is met). Cropping at screen edges would drop below minimum near right and bottom: within 64 px of those edges, or when a plane is needed for video, fall back to composited cursor, seamless because same pixels.
- With a plane and no other damage, motion needs no GPU: commit directly, deadline = `lead + margin` only. Latency falls by the GPU part.
- Plane priority: video > cursor > layer surface.
- **A dedicated input thread** would make it immune to a stuck main thread: reads libinput, commits the cursor plane. Smithay's state is single-threaded, so this is a design of its own. Later, only if the bench shows main-thread stalls after C and 6.

| Risk | Failure | Mitigation |
|---|---|---|
| Small surface moving at 120 Hz trips firmware | **firmware crash, black screen** | first-use latch, kill switch, edges handled by fallback, kernel check stays |
| Swap per motion costs DCP power | watts up | B5 |
| Visible jump at the fallback edge | a flicker | same-frame switch; test by eye |
| Cursor colour differs from composed | shift at the edge | sRGB tag from 7; until then native both ways |

**Importance is up:** per G, a long GPU job holds every composed frame. This is the one thing that keeps the pointer alive beside one.

**Proof:** needs the real display. No boot, but a trial on the owner's session behind the latch, at a time they choose; SSH recovery, reboot if DCP crashes. **Target:** pointer motion to flip p99 ≤ 5 ms; no pointer drops under `gpu-hog`. **Size:** M. **First proof:** on a test kernel with a debug client, as part of a planned boot.

## S. Animation curves, 120 Hz handling, perceived smoothness

- **Curves.** Closed-form springs on target time, already interruptible (`bande-spring`). Apple's published default is `spring(response: 0.55, dampingFraction: 0.825)` in SwiftUI **[memory]**. Ours follow Bande's motion design, but timing should be at least as good. Owner step: film macOS and Bande on the same monitor at 240 fps for ten actions, fit durations and overshoot from frames; an agent can do the fitting. Not a copy, a check.
- **Sample time.** `Tick::lead` samples one refresh past the target vblank **[code]**, while drags don't. Check if that's right for gestures; a wrong sign would be half a frame of swim. Trace it.
- **Refresh choice** is the user's. Worth a sentence in Settings' help text only if it's asked: at 120, film and video at 24, 30 and 60 fit exactly **[computed]**, while 144 fits 24, 36, 48 and 72, so 30 and 60 fps content repeats unevenly at 144 unless a player paces it. Motion feel favours 144, video favours 120, and neither is forced.
- **Frame-rate hints.** Apple tells apps to use lower rates for small movements **[Apple]**. We don't throttle: on a fixed 120 Hz link scanout runs regardless, and repainting a spinner is the only cost, so a hint is a toolkit-level micro-saving. `spinner` at 60 Hz, small. Bench first.
- **Idle.** Our idle already sends nothing: DCP reads the same frame.

## V. ProMotion-equivalent: variable refresh

**Hardware truth:** the owner's HDMI path can't: DCP says `SupportsVariableRefreshRate` false, and Apple tells Mac users not to use HDMI for it ([Apple](https://support.apple.com/102144)). So macOS isn't ahead here. Over USB-C DisplayPort it could, once `held/dp-altmode` lands and with a DP cable.

**macOS:** Variable in Displays settings, for external displays; may flicker on unvalidated ones **[Apple]**. On ProMotion panels Core Animation arbitrates the rate; iPad 24-120, iPhone 10-120 **[Apple]**.

**Asahi:** DCP `IOMFBPARAM_ADAPTIVE_SYNC` nonzero before a modeset enables it; 0x300000 is 48 Hz as fixed point; DCP then uses it without telling KMS; userspace can't toggle it without a modeset; `force_vrr` is planned ([7.0 report](https://asahilinux.org/2026/04/progress-report-7-0/)). 120 Hz on MacBooks works by faking `ts1-3` ([6.19 report](https://asahilinux.org/2026/02/progress-report-6-19/)).

**Design for later:**
- Kernel: `vrr_capable` when DCP reports a range, enable at modeset only, as a per-display setting needing a modeset, like macOS.
- Compositor: a clock mode with no fixed phase. After idle, flip at once, instead of waiting up to 8.33 ms. That's the real gain: first response immediate.
- Real `ts1-3` from the target (E1) instead of fixed values, also for MacBook panels if we ever support them.

**Risk:** flicker at low rate; black screen on a bad modeset. Mitigation: setting reverts on a timer, like a resolution change; a low-frequency-compensation floor. **Blocked by** DP alt mode; **size** L. Not now; it's listed so the frame clock leaves room for it.

## 7. Colour management, P3

**What a user feels:** photos and video in Firefox show P3 colours on a P3 monitor, and sRGB content matches them, not stretched.

**macOS:** every surface tagged, untagged treated as sRGB **[memory]**; `CAMetalLayer.colorspace` **[Apple]**. The display coprocessor converts per surface **[memory + `dcp_surface` fields]**.

**Others:** GNOME 48 ([Phoronix](https://www.phoronix.com/news/GNOME-wp_color_management_v1)). KWin versioned since 6.1, official protocol in 6.4 and v2 in 6.6. wlroots protocol in 0.19, render plumbing in 0.20 ([timeline, secondary](https://www.youngju.dev/blog/2026-07-17-wayland-color-management-hdr-shipping-timeline-xserver.en)). KWin turns overlays off with a colour profile unless the KMS pipeline can do it.

**Branch point: stage 2's boot** (a1e373 reports it): does DCP convert `BG_SRGB` on an external display?

**Design.**

1. **Protocol.** Smithay has none. `wayland-protocols` has `wp_color_manager_v1`. New module `compositor/src/colour/`: parametric descriptions with named TFs (sRGB, gamma 2.2, BT.1886, PQ, linear) and primaries (sRGB, P3, BT.2020), intents perceptual and relative, `get_output` from EDID primaries, `get_surface_feedback`. ICC only if a client needs it. Mesa then offers `VK_EXT_swapchain_colorspace` colour spaces by itself.
2. **Composition space.** Blend in encoded space with the display's own primaries: the toolkit's shadows and blends were designed with encoded blending, and macOS blends encoded for SDR **[memory]**. So each non-native surface runs decode, matrix, encode in the shader, then blends: +0.19 ms per full-screen surface **[ran]**.
3. **Two modes, if DCP converts:**
   - **A, no wide content on screen:** compose in sRGB and tag the primary `BG_SRGB`. Free, and overlays are tagged too.
   - **B, wide content visible:** compose in native primaries, tag `NATIVE`, shaders convert sRGB.
   A switch between frames must not shift colour; that's the check. If DCP doesn't convert, always B, with overlays allowed only when content is native or converted by the client.
4. **Precision.** Link is 8-bit at 120 Hz here. Compose 8-bit, blue-noise dither after the matrix, cost about nothing. If the bench shows banding, compose 10-bit or RGBA16F (same speed **[ran]**), then dither in the copy to the display buffer.
5. **Per-plane tags.** `get_colorspace` hard-codes `NATIVE`. Need a plane property: driver-private to start, `COLOR_PIPELINE` for upstream later, and it's uAPI so "ask first" applies. Rule stays: colour-wrong planes may not be used, and KWin's rule applies until that exists.
6. **Night Light.** DCP's CTM is global; unchanged. Verify it sits after per-surface conversion, or night tint on wide content looks off.

| Risk | Failure | Mitigation |
|---|---|---|
| `BG_SRGB` black or wrong on HDMI | **black screen** | stage 2's boot settles it; kernel patch behind a module parameter |
| Mode A/B switch visible | colour shift when a video opens | owner's eye test with toggling pattern; always-B fallback |
| EDID primaries wrong | everything off | `DefaultColorSpaceIsSRGB`, known-bad list, setting to treat as sRGB |
| Banding from matrix in 8-bit | steps in gradients | dither |
| Client tags wrongly | one window off | their bug |
| Firefox path differs | P3 not reached | test with gap-firefox; Firefox needs its pref on |

**Proof headless:** known pixel through each conversion ±1/255, protocol with a `wayland-info`-class client, on the M2. **Owner:** photo against reference on the monitor. **Target:** per-surface cost ≤ +0.2 ms; no change to typical frames when everything is native. **Size:** M-L. **First proof:** protocol plus P3 surface in a headless capture.

## 8. Windowed video on a display plane

**What a user feels:** watching a video in a window costs almost nothing, with no fans and no GPU wake. Frames are exactly on time.

**macOS:** hardware surfaces for video (from memory); Apple's APIs let the system promote Metal layers to direct display (rendering-path.md's WWDC note).

**Others:** KWin overlays are off by default for driver freezes, 7.3 against 8.4 W on a laptop ([Phoronix](https://www.phoronix.com/news/Plasma-6.5-Overlay-Planes-KWin)). Mutter direct scanout only (from memory). wlroots leaves it to the compositor; sway and others use it on full screen.

**Depends on:** stage 3, AVD (`fix-media`), 7's tags. Without hardware decode, software decode dominates, and this can't show a gain.

**Design, two levels.**

**8a. Overlay above the frame, no kernel change.** Video surface on plane 1 when nothing covers it and its rectangle misses the window's rounded corners; full-screen goes to primary. Player controls: another plane if they are a surface, else composite while they show. `VideoElement` in `scene.rs` returns `underlying_storage`, using Smithay's atomic test to accept or decline. Declining falls back to the shader, in the same frame.

**8b. Underlay below the frame.** The general answer: rounded corners and overlaid controls keep working, the frame is above with a hole, and video always on the plane.
- Kernel: plane 1 becomes the DRM primary (AR24 premultiplied), plane 0 an overlay at lower zpos. It stays X-format for the console when nothing's below (fbdev needs XR24; offer both on primary, and alpha forced off when nothing's underneath).
- Smithay 0.7 handles underlays, including the hole, when the primary has alpha and the element is opaque **[code]**.
- It's a uAPI-visible plane change, so "ask first", and needs a test boot.

**Video timing:** with `commit-timing` and E1's target times, the player's frame lands on its vblank. 24, 30, 60 are exact on 120.

| Risk | Failure | Mitigation |
|---|---|---|
| Combination trips firmware: size, scale, pitch, or surfaces under 32x32 | **firmware crash, black until reboot** | kernel checks stay and grow (scale 1/2-4x, min size, pitch 64); first-use latch per format; TEST_ONLY always |
| Bandwidth: 4K NV12 + 1440p@120, `allocate_bandwidth` answers yes always **[code]** | **underflow, flicker or black** | start with 1080p; experiment rising to 4K, on a test session; refuse above what passed |
| Colour differs from the shader path | visible **shift** when controls appear | toggling test; match shader's BT.709 limited to DCP's; set `ycbcr_matrix` from `wp_color_representation` |
| Night Light not applied to planes | video stays blue | verify; decline plane if not |
| Buffer freed during scanout | **IOVA fault, DCP crash** | kernel holds until swap complete (exists); the test still runs for this |
| 8b breaks console or boot | **black at boot** | test kernel, normal entry stays; hvc0; console path checked first |
| Plane toggling flashes | frame of black | same atomic commit; test |
| Decoder pool short | stutter | stage 3's pool check |

**Experiments:** (1) one test session (not boot) for 8a: 1080p NV12 at 1:1, then scaled, at various positions, read `flip_done` and dmesg. (2) Boot with 8b. **Needs the real display; DCP has no headless path:** the only `appledrm` here drives the owner's monitor, so a trial takes over the session at a time they choose. Recovery: SSH, reboot, latch.

**Target:** 1080p30 in a window ≤ +0.5 W against idle by B5; zero GPU frames while video alone changes. Report honest watts; on a mini at 6 W idle the gain may be small: rendering-path.md measured copies at about nothing for 1080p30.

**Size:** 8a M, 8b S in lines but high in risk. **First proof:** 8a's atomic test accepting 1080p NV12 on plane 1, on a test session.

## 9. HDR

**What a user feels:** HDR films and photos are bright where they should be, with desktop whites staying put. It's a per-display switch, as on a Mac (from memory for external displays).

**macOS:** EDR: `wantsExtendedDynamicRangeContent`, `edrMetadata`; tone mapping costs GPU and memory **[Apple]**. External HDR is a toggle in Displays, with a mode change (from memory).

**Others:** KWin, Mutter and wlroots all ship it. KWin's bugs show the traps: SDR tone-mapping unwanted, brightness inverted in Chromium, overexposure ([510759](https://bugs.kde.org/show_bug.cgi?id=510759), [515387](https://bugs.kde.org/show_bug.cgi?id=515387)). wlroots gets renderer support in 0.20.

**Reality check [computed]:** DCP lists mode 127, 2560x1440@120 12-bit YCbCr 4:4:4 BT.2020 PQ. That's 486 MHz at 36 bits; on HDMI TMDS it needs 729 MHz, more than HDMI 2.0's 600. Unless the DP-to-HDMI converter (`AppleDCPMCDP29XX`) does FRL or DSC, it will not link. HDR here may mean 60 Hz over HDMI, or 120 Hz over USB-C DP. Test before designing around 120 Hz.

**Kernel (largest part).**
- Connector `Colorspace` and `HDR_OUTPUT_METADATA`, on change select a PQ color mode through `set_digital_out_mode`. Today `dcp` only picks SDR modes **[code]**.
- `dcp_surface.xfer_func = DCP_XFER_FUNC_HDR`, `colorspace = BG_BT2020`, FP16 and 10-bit formats exist.

**Unknowns, test boot each, batch them:**
1. Does a PQ color mode link? At which rates? Monitor reports and picture.
2. Is metadata sent by DCP or needed from us (`dcpav` props)?
3. FP16 linear extended surface: does DCP tone-map like EDR? Does it take SDR surfaces with `BG_SRGB` at the same time?
4. What does SDR white look like: is there a level setting (`IOMFB` brightness calls, memory)?

**Compositor.** Compose in RGBA16F linear BT.2020, SDR at a reference white of 203 nits (setting, slider), encode to PQ 10-bit in the last pass, tone-map only what exceeds display peak (409 nits per DCP), never SDR. Pass-through for PQ video on a plane through 8. `get_output` advertises. Settings toggle per display, only when PQ modes exist, with the "keep these settings?" timer (black 0.5 s is the modeset).

| Risk | Failure | Mitigation |
|---|---|---|
| Mode won't link | **black screen** | revert timer, existing `refused` modes |
| Firmware crash on an untried surface/colour combo | **black until reboot** | test boots first; latch |
| Tone mapping wrong | blown highlights, like KWin's bugs | SDR never tone-mapped; white-level slider; test images |
| Text on YCbCr | soft edges at 4:4:4 unlikely, but check | owner's eye |
| Power and bandwidth up | hotter | only while on |

**Target:** SDR content indistinguishable from SDR mode by eye at the set white level; PQ test patterns right to 400 nits by eye; toggle under 2 s. **Headless:** shaders only, by pixel. **Size:** L, half of it kernel. **Last.** Reverse-engineering help: Asahi reports early HDR experiments ([6.19](https://asahilinux.org/2026/02/progress-report-6-19/)); follow theirs before duplicating, and trace macOS through m1n1 if nothing else settles it (needs the owner's hardware time).

## M. Shared compositor image, straight to scanout, shm

- **Shared image.** One compositor image per output instead of per swap buffer: saves up to 45 MB at 1440p. Requires damage age 1 always, and per-buffer tracking for what to copy. Worth doing after stage 1: copies shrink to real damage too.
- **Rendering straight into the display buffer.** a1e373's colour fallback may do this. Otherwise: 0.37 ms saved per full frame, slower into LINEAR, blur needs a readable target, and it blocks 16F composition. **My view: don't, unless bench proves a typical-frame gain.** Better prize: compressed scanout below.
- **Compressed scanout.** `INTERCHANGE` is the only compressed layout DCP scans, kernel supports it, Mesa's Vulkan doesn't offer it. Why it matters: LINEAR scanout reads 1.77 GB/s at 120 Hz forever **[computed]**, even idle, a plausible idle-power saving of real size, and macOS surely avoids it **[memory]**. Unknown: can AGX write that format at all, or only the media blocks? Experiment: test whether Mesa's `ail` can address it, and render one frame on a throwaway build; scan out by test session. High risk of "no". Worth one day because idle power is everything on a laptop, less on the mini. Bytes-to-DRAM counters are still not found; measure by B5 only.
- **shm without a copy.** `udmabuf`, as KWin does. Toolkit uses shm only for first frames, and `VK_EXT_external_memory_host` isn't on Honeykrisp **[ran]**. Low value. Do it when a real shm client shows in a trace.

## Kernel and DCP work needed

| Change | Item | Boot | uAPI? |
|---|---|---|---|
| Debug dump of `swap_info` | E1 | yes | no |
| `ts1-3` real targets, experiment | E1, V, 8 | yes | no |
| Allow HIGH queues with `CAP_SYS_NICE` | G | yes | relaxes |
| Stricter plane checks | 8, K | yes | no |
| Plane colour tags, per-plane property | 7, 8 | yes | **yes, ask** |
| Plane 1 as primary | 8b | yes | **yes, ask** |
| `Colorspace`, HDR metadata, PQ color modes | 9 | yes | standard properties |
| `vrr_capable`, modeset-time enable | V | yes | standard |

Batch them: E1 and G share a boot; 7 and 8a tags share another. All through `patches/7.3/held/<topic>` first, so a test kernel takes them on request. Patches are written to mainline style; upstream Asahi's Rust rewrite of DCP may change the ground, so keep them small.

## What the owner needs to do

1. **Done: the DCP trace** (B1). Again after each scheduler stage, the same gesture for ten seconds; agents can run it if they're allowed that root command without asking each time.
2. **Later: the phone test,** ten minutes at 240 fps with a visible key press.
3. **One macOS boot, later:** the same phone test and filming animations on this monitor. It's the only real macOS comparison.
4. **Test boots,** at their pace, batched: E1 + G first; G decides if a GPU hog can ever be beaten.
5. **Yes or no on uAPI:** plane colour property, plane order. Asked when reached.
6. **Eye tests:** colour switching, Glass after half-size blur, prediction by hand, HDR.

## Sources

- Apple: [Understanding hitches](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app), [Explore UI animation hitches](https://developer.apple.com/videos/play/tech-talks/10855/), [ProMotion displays](https://developer.apple.com/documentation/quartzcore/optimizing-iphone-and-ipad-apps-to-support-promotion-displays), [CAMetalDisplayLink](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink), [Adaptive Sync](https://support.apple.com/102144), [Mac mini power](https://support.apple.com/en-us/103253)
- Asahi: [6.19](https://asahilinux.org/2026/02/progress-report-6-19/), [7.0](https://asahilinux.org/2026/04/progress-report-7-0/), [7.1](https://asahilinux.org/2026/06/progress-report-7-1/)
- Others: [Hugl on KWin's scheduling](https://planet.kde.org/xavers-blog-2024-06-24-fixing-kwins-performance-on-old-hardware), [KWin frame statistics](https://invent.kde.org/plasma/kwin/-/merge_requests/5975), [triple buffering debrief](https://discourse.ubuntu.com/t/triple-buffering-a-debrief/56314), [Dynamic triple buffering in GNOME 48](https://www.phoronix.com/news/GNOME-48-Triple-Buffering), [fifo-v1](https://wayland.app/protocols/fifo-v1), [commit-timing-v1](https://wayland.app/protocols/commit-timing-v1)
- Latency reference numbers: [Hume](https://thume.ca/2020/05/20/making-a-latency-tester/), [Harris on terminals](https://dev.to/lkhrs/measuring-terminal-latency-26m7)
