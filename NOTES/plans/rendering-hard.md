# Rendering: the hard stages

Plan only; nothing here is built except where stated. It continues [rendering-path.md](rendering-path.md), which owns stages 0-4. Delete items as they land.

**Scope (owner, 2026-11-07):** most users run Debian's own kernel, so every item must work there with any GPU through atomic KMS. No feature may need a Bande kernel patch. Driver-specific extras sit behind runtime checks, never as dependencies. The frame scheduler T learns flip times, render times and commit lead per output on the machine it runs on, so it suits any driver without per-GPU constants.

Stack: Smithay 0.7.0, `ash` 0.38, Mesa and the kernel from Debian 14 (forky).

Marks: **[code]** read from our code or a driver. **[Apple]** Apple's documents. **[memory]** from memory, unverified. **[computed]** arithmetic from the others.

## Ambition

macOS is the bar for behaviour. We can honestly beat it in two places, and match it in the rest.

1. **Latency.** Apple's own model is a frame begun at one vsync and presented two vsyncs later, with the last pixel a vsync after that: 25 ms at 120 Hz, 33 ms when it falls back to triple buffering **[Apple]**. A commit here is designed to reach the next vblank, so we can run under one vsync where macOS plans for two.
2. **Consistency under load.** Today nothing protects the compositor from a build or a game. After C, K and, where the driver allows it, G, it should hold through them.

Parity is the floor: colour correct everywhere, video through the display engine, HDR behind a per-display toggle, a pointer that never stutters.

### Target numbers

Nothing is measured on the build machine yet; nothing unmeasured may be quoted as a result. macOS figures for the owner's monitor do not exist yet. Item B measures them with the same phone test, and until then the targets stand on Apple's model only.

| Metric | Target | macOS reference | Measured by |
|---|---|---|---|
| Mouse motion to photon, pointer, mid-screen, 120 Hz, 1 kHz mouse | p50 ≤ 15 ms, p99 ≤ 21 ms | unmeasured; same phone test | model + phone (B4) |
| Key to photon, Terminal glyph, mid-screen | p50 ≤ 20 ms, p99 ≤ 28 ms | model 25 ms + input + panel **[Apple]**; old 60 Hz measurements of 29-45 ms for terminals, 35 ms for Sublime ([Harris](https://dev.to/lkhrs/measuring-terminal-latency-26m7), [Hume](https://thume.ca/2020/05/20/making-a-latency-tester/)), not comparable hardware | phone, model |
| Key to flip, software only | p50 ≤ 10 ms, p99 ≤ 16.7 ms | none | trace |
| Client commit to flip | p99 ≤ 8.33 ms | 2 refreshes **[Apple]** | trace |
| Compositor CPU per frame, typical | p99 ≤ 1 ms, thread never blocked on GPU | none | trace |
| Render budget used, latch to commit, p99 | within the frame less the learned commit lead | none | trace |
| Compositor GPU, one window updating | p99 ≤ 2 ms | none | GPU timestamps |
| Drag at 2560x1440, Glass, large damage | p99 ≤ 4 ms | none | bench, trace |
| Refresh slots used while dragging | ≥ 99 % of intervals one refresh while content changes, at 60, 120 and 144 | none | B2 |
| Dropped frames during drag, scroll, animation | at 120 Hz ≤ 0.1 % (≤ 1 ms/s) in all three styles. At 144 Hz ≤ 0.1 % Refined Solid and Wallpaper-Lit, Glass after O and D. At 60 Hz ≤ 0.1 % | Apple: ≤ 10 ms/s good in Organizer, its older talk says < 5 ms/s **[Apple]** | trace |
| The same, with a GPU hog and all CPUs busy | ≤ 1 % (≤ 8 ms/s) | unmeasured; run the same | bench + trace |
| Flip interval error, frames not dropped | p99 ≤ 0.3 ms on the filtered clock | none | trace |
| Window behind pointer during a drag | 0 frames (same frame by design, unproved) | same frame **[memory]** | trace + camera |
| Idle power, desktop at rest | ≤ today, then lower with compressed scanout if it proves possible | unmeasured | B5 |
| Idle wake-ups and redraws, compositor, at any rate | 0 at 60, 120, 144 (0 by rule today) | none | `bande-idle` |
| 1080p30 HEVC in a window, above idle | ≤ +0.5 W with hardware decode and a plane | unmeasured | B5 |
| Extra memory per 2560x1440 window | 0 (14.7 MB today **[code]**) | 0 **[memory]** | `bande-footprint` |
| Pixels: sRGB on a wide-gamut monitor | matching a reference photo in the monitor's sRGB mode | correct **[memory]** | headless shader check + owner's eye |

### What will not be claimed

Latency above assumes a 1 kHz mouse. A 125 Hz mouse adds 4 ms on average. Panel response is the monitor's. A camera number compares only against the same camera on macOS.

## Refresh rates

No rate is a constant anywhere. Clock, deadlines, animation and bench work from each output's mode and its measured flips, at 60, 120, 144 and anything else, on mixed displays and across mode changes.

**How it is chosen today [code]:** the compositor gives an unconfigured display its preferred mode at its highest rate. Settings' Refresh rate lists the rates of the current resolution, 60-240 (`state.rs`, `REFRESH_RANGE`). That's already right; rate is a user's choice, no change.

**Which constants remain [code]:** `state::REFRESH` is the virtual rate of the headless backend and the fallback for an empty mode line, used nowhere else. It stays for that. The bench takes `--refresh`.

**Budget per frame** is the frame (16.67 ms at 60 Hz, 8.33 at 120, 6.94 at 144) less the learned commit lead (T) and about 300 µs for wake delay and pad. At 144 Hz the budget is tight:
- **Refined Solid:** stage 1's copy and T. CPU ≤ 1 ms, GPU ≤ 3 ms.
- **Wallpaper-Lit:** the same, with 5's cheaper sampling and M's copy limited to damage.
- **Glass:** must drop to half or less. O's reuse for static backdrops (most frames), half-size start for dragged ones. A big drag under Glass still may not fit: D gives one extra frame of latency at full rate rather than stutter.
- **Copies:** stage 1 and M bring the copy to damaged area only.
- **Stage 6 and C:** a wake-up late by 1 ms costs a large part of the budget, so at 144 Hz C pays off the most.

Budgets are the output of the clock, not a constant to reach: it plans frames by what is measured, so a style that can't make 144 falls to every other refresh only if D is also out of room.

**Efficiency at rest.** The compositor's cost at rest is zero at any rate: no timer, no repaint, no callback without something asking. Undamaged frames stay undrawn. The display engine still scans out at the mode's rate (1.77 GB/s at 2560x1440 and 120 Hz, 2.12 at 144 **[computed]**, LINEAR), so a higher rate may cost something at rest in DRAM and link power, which only B5 can show.

**Mixed displays [code]:** each output already has its own scheduler and clock, and animations are closed-form on target time, so displays of different rates don't interact. Today's weak spot: one thread, so a frame on one output delays the other's start, until 6. A window spanning two gets callbacks from its primary. Mode change: `FrameClock` is replaced on a new refresh; T also resets its phase and keeps learned render costs. Client timing follows through `wp_presentation`'s refresh.

## Order

```
T frame scheduler (F2, F4)
B bench and trace, alongside ── everything after is measured with it
├─ C CPU priority
├─ G GPU priority
├─ I input resampling
├─ K cursor on a plane
├─ S animation fit
└─ 6 no GPU wait ─┬─ D dynamic triple buffering
                  ├─ 5 direct textures ── M shared image, shm
                  ├─ O Glass reuse
                  └─ W fewer wake-ups
7 colour ── 8 video on a plane ── 9 HDR
V variable refresh
```

| Item | What | Gain | Risk | Size |
|---|---|---|---|---|
| T | Frame scheduler: F2, F4 | **Lost frames in drags; latency; client pacing** | low | S-M |
| B | Bench suite, frame trace | Everything else | none | M |
| C | Real-time compositor thread, CPU floor | Holds under builds | medium: lock-up | S |
| G | GPU priority | Holds under games, where the driver preempts | low | S |
| 6 | No GPU wait | Thread free; enables D | medium: freeze | M |
| D | Dynamic triple buffering | Glass never hitches on spikes | low | S |
| 5 | Direct textures, tiled, compressed | Memory, a copy per window | medium: tearing | M-L |
| O | Glass reuse | Glass under budget | low | M |
| W | Wake-ups per update | CPU, power | low | S-M |
| I | Pointer and scroll resampling | Smooth slow motion | low | S-M |
| K | Cursor on a plane | Pointer never stutters, even when the GPU is held | medium: driver | M |
| S | Animation fit, curves | Feel | none | S |
| 7 | Colour management | P3 correct | low-medium | M-L |
| 8 | Video on a plane | Power, no GPU for video | high: driver | M |
| 9 | HDR | HDR | high | L |
| V | Variable refresh | ProMotion-style | high | L |
| M | Shared image, shm | 45 MB; rarely matters | low | S-M |

## Safety used by every item

**Kill switches.** Each risky path has one environment variable, `BANDE_RENDER_NO_<ITEM>=1`, read at start, and `BANDE_RENDER_SAFE=1` turns them all off. `BANDE_RENDERER=cpu` already exists. No Settings switch: it would be a control the owner cannot judge.

**First-use latch.** Before the first use of a risky display path (plane colour tags, video or cursor on a plane) on a given kernel and driver, the compositor writes `<path> <kernel> <driver>` to `$XDG_STATE_HOME/bande/render-trial`, then removes it after 300 good flips. A file still there at the next start means that path just killed the session or the machine: that path stays off for that kernel and driver, the journal says why, and the owner sees it in fault reports. This prevents a black-screen loop with no keyboard.

**Runtime fallback.** `FrameRecovery` already escalates at 1 s (flip timeout), 3 s (restart display), 15 s (give up). Add a step: when a stall starts within 2 s of a newly enabled path, turn that path off for the session before restarting.

**A display-engine fault** can leave the display dead until reboot, with only `flip_done timed out` in the journal. Recovery is over SSH or the power button; the latch prevents a repeat.

## Where each proof can run

| Place | What it can prove | What it cannot |
|---|---|---|
| Headless on the build machine (`build-run --here`, time limit) | Its GPU's behaviour, fences, imports, modifiers, shader costs, GPU priority, real clients, frame state machines | Planes, flips, colour on glass |
| Build VM (lavapipe, `vkms` for atomic logic) | Protocol code, trace format, state machines, `TEST_ONLY` fallback logic | Anything about timing |
| Owner's session, real compositor | Frame trace, cursor and video trials, watts, eyes | Needs a time they choose |

## B. Bench suite and frame trace (first)

**What a user feels:** nothing. It decides what everyone else does. The owner's optimisation run gets a number to push against, and a regression gets caught.

**macOS:** Instruments' Hitches and Display templates; `CADisplayLink` gives begin, commit and presentation times **[Apple]**. We match this with a trace.

**Others:** KWin logs frame statistics to a file ([MR 5975](https://invent.kde.org/plasma/kwin/-/merge_requests/5975)). Mutter has its profiler (from memory). A plain per-frame record file is the proven shape.

### B2. Frame trace in the compositor

One fixed ring of about 4096 records of about 96 bytes, written by the frame scheduler with no allocation and a single branch when off. Per frame:
- target vblank, wake, latch, record end, submit, GPU start and end (timestamps through `VK_KHR_calibrated_timestamps` where the driver has it), commit, flip event, flip time
- damaged and blurred pixels, elements per plane, missed flag
- per client surface: commit time to latch, and input event time for the frame it answered, when known

Started and stopped on `org.bande.compositor` over the session bus, writing NDJSON, so a running session is traced without restarting it. `bande-frame-report` turns it into percentiles, hitch ms/s, the share of one-refresh intervals and missed slots, and a Perfetto file. Adding kernel tracepoints later merges on `CLOCK_MONOTONIC`.

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

**CI:** the GitHub runners have no GPU and CI runs on tags only. CI runs the trace format under lavapipe to check that it parses, nothing about timings. The build machine is the gate, as for everything else.

### B4. Real-display numbers

No purchases, and nothing depends on special hardware. Two levels that add up:

- **Trace-based model, automatic.** Input event time to flip comes from the trace, with the flip's place on the vblank grid. Photon time adds half a scanout at mid-screen (half a refresh, from the mode) and the monitor's constant delay, taken from the phone below. It's repeatable and can run on any commit, once the owner agrees to trace.
- **Phone at 240 fps, with a visible key press, on Bande and on macOS.** Frame the key and the screen; count frames from the key bottoming out to the glyph, 4.2 ms each. Ten presses in Terminal at 120 Hz; the same with a mouse button and a menu. Ten minutes. Good to ±2-4 ms per press and about ±1 ms on the mean of ten, and it includes the keyboard's own delay, so it compares only against the same run on macOS on the same monitor. The phone also calibrates the model's constant.

A photodiode rig would be sharper but is not planned: the owner isn't buying one now.

### B5. Power

A plug meter at the wall, paired rounds, mode against idle, 60 s each, 10 rounds. The same meter gives the comparison with macOS. Display brightness is the monitor's, unaffected.

### Risks and first proof

| Risk | Failure | Mitigation |
|---|---|---|
| Measuring disturbs | changed timings | off by default, cost measured; ring no-alloc |
| Bench noise from other agents | false results | load gate, `kbuild.lock`, confidence intervals |
| Real trace needs a new compositor on the owner's session | none until they update | ships as a normal release; no run needed before |

**Proof:** report on one existing scenario, headless. **Size:** M (about 800 lines, no new dependencies). **Order:** first.

## T. Frame scheduler

Authorised by the owner to be built, in stages, behind `BANDE_FRAME_TIMING=learned`; classic stays the default until the new rule has proved itself on a real display (it left a KVM/llvmpipe session black on 2026-11-07). Built: the scheduler, honest `wp_presentation` and per-surface callback timing (see `compositor.md`). Remaining below.

**What a user feels:** dragging stops losing refresh slots. Typing and pointer respond up to a frame sooner. At 144 Hz the desktop actually reaches 144.

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

**Where we go beyond them.** No compositor above learns from a swap that slipped after the ioctl returned; all pick a generic margin. A driver with a hard commit cut-off before vblank defeats them and halves the rate. We can observe the slip directly: which vblank the flip landed on, against the one we aimed for. That's the new part.

### Design: `pacing.rs`, pure logic, plus `clock.rs` keeps the state machine

1. **`VsyncEstimate`:** phase and period per output, from flip times only, Android's idea at small cost: a second-order filter on the error against the predicted grid (phase gain 1/16, period gain 1/256, within 1 % of the mode's nominal). Errors over 1/8 of a period are outliers, snap after 8 in a row, as after a monitor glitch. It owns `sequence`, so clients get a real count. Clients get grid time, not the noisy event time. Starts from the mode; nothing assumes a rate.
2. **`RenderPrediction`:** per output, in time from wake to commit returning.
   - Budget = p95 of the last 64, or the max of the last 4 if higher (instant attack, a spike forgotten in 4 frames not 16), plus p99 of wake delay over 64, measured against what we asked for, plus a pad.
   - Pad: +100 µs each time a commit came too late for its slot, up to a quarter of a refresh, −10 µs per 64 good frames, so the margin is earned, not fixed. On a loaded machine wake delay rises by itself, which answers the old doc's reason for choosing the max.
   - Heavier content such as the overview opening costs one miss, as today.
   - A style or workload change settles in 4 frames. **Classes by damage size** go in only if the bench shows a benefit (F4).
3. **`CommitLead`:** how far ahead of its slot a commit must return, learned per output by what flips do:
   - Starts at 35 % of a refresh on the first real flip (headless, without flips, stays 0).
   - While learning, each hit lowers it by 50 µs until the first miss, which costs one frame once per session and output.
   - A miss with time left ≥ lead raises it to `left + 100 µs`, at most +300 µs per event, so a rogue miss can't blow it up. A miss with less time left than lead was our lateness, not the driver's: it goes to pad.
   - After 1024 hits, −20 µs, to track changes: a probing miss about every 40 s of constant drawing, under 0.03 % of frames.
4. **Slot choice:** `start = slot − budget − lead`; a wake past that may still go if `now + typical + lead ≤ slot`, as Mutter's min-estimate retry. Otherwise the next slot. This removes self-inflicted skipped slots.
5. **Late input:** a timer wake dispatches all ready sources in one loop pass, including libinput, before `poll` runs **[code]**. So the latch sees the latest input as long as it starts as late as it safely can. Nothing else is needed; a separate thread is later work (K).
6. **Idle:** no timers unless `wanted`, or a client asked a callback with no damage (one wake at the slot, as niri). Same rule at any rate. Phase holds across idle, since the link runs free.
7. **Mode change, mixed outputs:** `FrameClock` per output, replaced with the new mode (keeps the render prediction, drops phase and lead). Outputs share one thread until stage 6: a frame on one delays the other; pad handles what's left.
8. **Repeated misses:** lead and pad raise themselves; if a window of 128 frames still shows over 5 % missed, render one slot later: a fixed pipeline depth would be D. Logged once. Leaving out `BANDE_FRAME_TIMING=learned` restores the old rule in full.

### Stages

| # | What | Done when |
|---|---|---|
| F2 | Real session measured through B2 by the owner's command, at 60, 120 and 144 | misses < 1 %, one-refresh intervals > 95 % |
| F4 | `fifo-v1`, `commit-timing-v1` (P); classes if bench asks | Mesa's clients pace |

**Occlusion, per Mesa:** its FIFO path relies on `discarded` and barrier clearing for occluded clients, or `presentWait` degrades; F4 must drain them at a virtual refresh.

### How it's measured

On the real display, B2's trace while moving two windows for ten seconds.

**Headless, in the gate:** the simulator in `clock.rs` tests drives the real logic against a display with a commit cut-off. Virtual time, at 60, 120, 144, a mixed mode change and render spikes. The old rule must miss, the new must not. It's a regression test for a real bug, allowed. `bande-session-bench --refresh` covers the true compositor headless.

**Risks.**

| Risk | Failure | Mitigation |
|---|---|---|
| Learning misses at start | one lost frame per session | accepted; unnoticeable |
| Lead too small after a change in load | a few misses | raises at once; `lead` is per output |
| A hung flip | as today | `FLIP_TIMEOUT`, recovery unchanged |
| New logic wrong in the field | judder, half rate | classic is the default; `learned` is opt-in |
| Sim doesn't match the display | good sim, bad display | the owner's trace is the gate, not the sim |
| Draw time changes | moves numbers | learned, not constants |

**Target:** at 60, 120 and 144 Hz: under 1 % misses, then ≤ 0.1 %, one-refresh intervals > 95 % while content changes; left-over time on hits ≤ 1 ms above the cut-off; p99 flip error ≤ 0.3 ms. **Size:** S-M, under 400 lines plus tests. **No kernel change.**

## C. Real-time compositor thread and CPU floor

**What a user feels:** during a kernel build, the desktop still moves evenly. A first keypress after idle isn't slower.

**macOS:** QoS classes with user-interactive at the top **[Apple docs, from memory]**, WindowServer at raised priority **[memory]**.

**Others:** KWin takes SCHED_RR via `CAP_SYS_NICE` (from memory). Mutter's KMS thread goes real-time through rtkit (from memory).

**Design:**
- The compositor main thread gets SCHED_RR priority 1 with `SCHED_RESET_ON_FORK`, via rtkit over the bus, falling back to nothing. It needs `RLIMIT_RTTIME`, which rtkit requires anyway. Exceeding it sends SIGXCPU: logged and dropped to SCHED_OTHER, not killed.
- Children and other threads stay normal.
- `uclamp.min` on the compositor (and on the focused app's UI thread through the toolkit, allowed without privilege within the limit) so a wake doesn't start at the lowest frequency; `schedutil` otherwise ramps after the frame is over.

| Risk | Failure | Mitigation |
|---|---|---|
| Compositor spins at RT | one CPU stolen; not a lock-up on a multi-core machine | RTTIME limit; RR priority 1; existing watchdog |
| Blocking I/O at RT stalls others | priority inversion | trace shows; audit the main thread's file I/O first (recall already has its own thread) |
| rtkit refuses | stays normal | runs as today; logged once |
| Higher power from uclamp | watts up | B5 before and after; scope to active interaction |

**Proof:** headless, `cpu-hog` variant, private session on the build machine; rtkit works for any session. **Target:** drops under `cpu-hog` ≤ 1 %. **Size:** S.

## G. GPU priority for the compositor

**What a user feels:** a game or a heavy WebGL page doesn't take the desktop down with it. The pointer and windows keep moving.

**macOS:** WindowServer's GPU work outranks apps, and the GPU preempts **[memory]**.

**Others:** KWin asks for high-priority contexts where the driver allows (from memory). Linux drivers gate HIGH and REALTIME on `CAP_SYS_NICE`. Whether a higher-priority queue preempts a running job depends on the driver and the GPU.

**Design.**
- Compositor: queue with `VK_QUEUE_GLOBAL_PRIORITY_HIGH`, falling back to MEDIUM on `ERROR_NOT_PERMITTED`. Needs `CAP_SYS_NICE` on the binary, by `setcap` in the package, which C could use too.
- **Where the GPU doesn't preempt:** K keeps the pointer moving with no GPU at all, and direct scanout and planes keep video moving. Those two are what a hog cannot touch.

| Risk | Failure | Mitigation |
|---|---|---|
| Priority without preemption | no gain, jobs still queue | a contention program settles it; then K |
| Capability makes the binary `AT_SECURE` | `LD_*` ignored, debugging harder | acceptable; check nothing relies on it |

**Experiment:** a contention program on the build machine, foreground at HIGH beside 47 ms and 149 ms hog jobs, then moved into B3's `gpu-hog`.

**Target:** compositor frame p99 ≤ 4 ms beside a 47 ms hog; if not reachable on a driver, pointer unaffected through K, and an honest note. **Size:** S.

## 6. Stop the compositor thread waiting on the GPU

**What a user feels:** little on a light frame. It matters when frames are heavy (Glass): input isn't held, client commits aren't held, and a slow frame doesn't lose the next one. It's the base for D and for honest timings.

**macOS:** render server's CPU part stays in one vsync, the GPU part may overlap the next frame **[Apple]**. Beside a GPU hog (see G) a blocked thread would also stop input for the whole job; this removes that.

**Others:** all three use fences. wlroots and KWin hand `IN_FENCE_FD` to KMS or wait on a sync file; KWin's commit thread blocks instead of the main thread (from memory).

**Design.** Replace `submit_and_wait` in `VulkanFrame::finish` with:
1. N frames in flight (2, then 3 with D): one command buffer, fence, semaphore per slot; staging, garbage, blur-chain retirement keyed by slot, freed on that slot's fence.
2. Submit, export `SYNC_FD` from the semaphore, and return a `SyncPoint` around it, implementing Smithay's `Fence` trait (`is_signaled`, `wait`, `export`).
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
| Driver semaphore export bug under load | freezes | soak (`bande-soak`), 1 h; fallback flag `BANDE_RENDER_NO_ASYNC` |

**Proof headless:** soak on the build machine with `--validate`; virtual-time test of the state machine (hung fence). A real session update, with the switch. **Target:** thread blocked on GPU 0 per frame; no drop change; CPU per frame ≤ 1 ms p99. **Size:** M. **First proof:** the fence type on `bande-session-bench`.

### D. Dynamic triple buffering

**Feel:** a slow frame costs one frame's latency instead of a visible stumble, as Apple and GNOME both do.

**Design:** after a frame is late (fence after deadline), allow two frames in flight until 30 frames are on time; budget may then exceed one refresh. Smithay's `queue_frame` already holds a `queued_frame` behind a pending one **[code]**. Double buffer normally: no latency cost when keeping up, exactly the rule in the Ubuntu debrief **[from search]**.

**Risk:** latency stuck up: exit rule above, trace shows. **Target:** zero dropped frames during Glass drag at full damage, at most one refresh more latency only while recovering. **Size:** S after 6.

## 5. Sample window buffers directly, then tiled and compressed

**What a user feels:** less memory per window (14.7 MB of 1440p gone), clients render a little faster, a little more headroom. Without it, Firefox and video are always a copy behind.

**macOS:** IOSurface is shared across processes and GPU without a copy (from memory; [Apple doc index](https://developer.apple.com/documentation/iosurface)). Apple's GPU-native surfaces are lossless-compressed where it can (from memory).

**Others:** KWin and Mutter import dmabufs directly as textures, and offer tiled and compressed modifiers through per-surface feedback. wlroots likewise. None copies.

**Design.**

1. **5a: LINEAR direct.** `import.rs` makes one `VkImage` per dmabuf with `DRM_FORMAT_MODIFIER_EXT` and keeps it for the buffer's life. Reuses stage 3's holding. Acquire barrier from `QUEUE_FAMILY_FOREIGN` each commit, layout `GENERAL` to `SHADER_READ_ONLY`. Damage needs no copy, though the compositor's own damage stays as it is.
2. **Hold rules.** A buffer is held until the flip replacing it, and until the GPU finished any frame using it. `hold_replaced` today skips surfaces with no frame callback **[code]**; direct import must hold always. Closing animations and stretch-on-resize hold a `Buffer` instead of a copy. Clients now must have 3+ buffers; Mesa WSI has them, and check the toolkit's swapchain.
3. **Sync.** Syncobj acquire and dmabuf-poll blockers already delay the latch **[code]**. Confirm Mesa's WSI uses `linux-drm-syncobj-v1` with a logged client, first thing. Release point signalled at the flip through `Buffer` drop.
4. **5b: modifiers.** `linear_formats` becomes the modifiers the GPU driver offers, sampled. Validate: import failure at `create_params` gives the client an error before anything draws.
5. **Per-surface feedback.** Default tranche: compressed first. Scanout tranche: the modifiers the planes accept (`IN_FORMATS`) for layer surfaces and full-screen windows, which keep their plane and direct scanout. Windows move between by `surface_feedback` on full-screen. Old buffers keep working.
6. **Keep the copy** as the fallback for anything failing import, and for the CPU renderer.

**Security:** a client's dmabuf reaches GPU sampling directly, so `lseek(fd, END)` must cover the layout before import, exactly (`vkGetImageMemoryRequirements`). A bad layout would otherwise fault the compositor's own GPU context. `fuzz/` gets a case.

| Risk | Failure | Mitigation |
|---|---|---|
| Client writes while sampled | **tearing** | hold rules; tear test: client alternates two full colours, compositor captures, asserts no mix, runs in the bench with explicit and implicit sync |
| Undersized or misaligned buffer | GPU fault, **device lost**, session black then recovered | size check; existing renewal; fuzz |
| Compressed metadata mismatch | **garbled window** | Mesa version pinned by Debian; `BANDE_RENDER_NO_TILED`, feedback goes back to LINEAR |
| Client stalls for a free buffer | latency up | check buffer count in the trace; toolkit gets one extra if needed |
| Client's fence never signals | that window stops only | blocker is per surface (exists) |
| Memory counted twice in the kernel | oversize RSS | `bande-footprint` |
| Firefox rejects new feedback | black tab | test with gap-firefox's widget; LINEAR stays offered |
| Feedback churn on full-screen | one flash | a client's own swapchain recreate; test; delay until settled |

**Proof headless:** all of it except plane behaviour, on the build machine, real clients. **Target:** per-window extra 0. **Order:** 6 first, for lifetimes; stage 3 first, for the shared holds. **Size:** 5a M, 5b S-M. **First proof:** tear test on 5a.

## O. Glass: reuse the blur

**What a user feels:** Glass stops being the slow style. It's the look that makes the desktop feel expensive; it must not be the one that stutters.

**macOS:** materials blur the backdrop in the render server at reduced resolution (from memory). Nothing published.

**Others:** KWin's dual Kawase re-blurs damaged areas with margins; Mutter's blur is similar, with caches (from memory).

**Design.** Today each frame touching `Blur::bounds` filters the whole region, 2×passes **[code]**.
1. **Reuse:** keep each chain's top level, keyed by region and backdrop version. Skip filtering when damage *below* this element in the stack misses `bounds`. Content *on* the panel doesn't invalidate it, which is the common case.
2. **Partial filter:** when it does hit, refilter only damage + reach, per level.
3. **Start at half:** copy into `levels[0]` at half size with a linear blit. Quality checked by eye against today, Glass's noise hides it.
4. **Levels in RGBA16F**, which removes banding; dither where it joins the 8-bit target. Check its cost in the bench.

| Risk | Failure | Mitigation |
|---|---|---|
| Stale backdrop | wrong colours behind glass | damage-tracker based; debug flag re-filters and compares in bench |
| Dragging a window under glass | no gain | correct, full cost; D catches it |
| Quality drop at half size | visible softening | owner's eye; keep passes |

**Target:** drag under glass p99 ≤ 4 ms at 1440p; static panel over changing content under 0.5 ms. **Proof:** headless with `bande-session-bench` in Glass. **Size:** M.

## W. Wake-ups per client update

Ten per update today (plan path). **Feel:** only power and CPU. **Design:** count them with B2, attribute each. Expected: frame timer re-arm on each commit, separate flip, release and callback sends. Coalesce sends at the flip into one flush, one timer per output moved only when the deadline moves. Toolkit: one wake per callback, nothing for releases it doesn't need. **Risk:** a delayed release blocks a client: test with 2-buffer clients. **Target:** ≤ 4 across compositor and client. **Size:** S-M. **Note:** there's no point guessing the list before the trace.

## P. `fifo-v1` and `commit-timing-v1`

**Feel:** Vulkan apps and games pace properly and don't hang when hidden. Video lands on the right frame.

**macOS:** `CAMetalDisplayLink` targets, `present(afterMinimumDuration:)` (from memory).

**Today:** absent, though Smithay 0.7 has both modules **[code]**. Mesa uses them on Wayland when offered, since 24.3 ([Phoronix](https://phoronix.com/news/Mesa-24.3-Vulkan-FIFO)); `VK_EXT_present_timing` needs them.

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

**Design.**
- The cursor on the CRTC's cursor plane, or on an overlay where there is none, accepted by an atomic `TEST_ONLY`. Where the plane is refused, or needed for video, fall back to the composited cursor, seamless because same pixels.
- With a plane and no other damage, motion needs no GPU: commit directly, deadline = `lead + margin` only. Latency falls by the GPU part.
- Plane priority: video > cursor > layer surface.
- **A dedicated input thread** would make it immune to a stuck main thread: reads libinput, commits the cursor plane. Smithay's state is single-threaded, so this is a design of its own. Later, only if the bench shows main-thread stalls after C and 6.

| Risk | Failure | Mitigation |
|---|---|---|
| A moving plane at 120 Hz trips the driver | **black screen** | `TEST_ONLY`, first-use latch, kill switch |
| Commit per motion costs display power | watts up | B5 |
| Visible jump at a fallback | a flicker | same-frame switch; test by eye |
| Cursor colour differs from composed | shift at the switch | colour tags from 7; until then native both ways |

**Importance:** where the GPU doesn't preempt (G), a long GPU job holds every composed frame. This is the one thing that keeps the pointer alive beside one.

**Proof:** needs a real display: a trial on the owner's session behind the latch, at a time they choose; SSH recovery. **Target:** pointer motion to flip p99 ≤ 5 ms; no pointer drops under `gpu-hog`. **Size:** M.

## S. Animation curves, 120 Hz handling, perceived smoothness

- **Curves.** Closed-form springs on target time, already interruptible (`bande-spring`). Apple's published default is `spring(response: 0.55, dampingFraction: 0.825)` in SwiftUI **[memory]**. Ours follow Bande's motion design, but timing should be at least as good. Owner step: film macOS and Bande on the same monitor at 240 fps for ten actions, fit durations and overshoot from frames; an agent can do the fitting. Not a copy, a check.
- **Sample time.** `Tick::lead` samples one refresh past the target vblank **[code]**, while drags don't. Check if that's right for gestures; a wrong sign would be half a frame of swim. Trace it.
- **Refresh choice** is the user's. Worth a sentence in Settings' help text only if it's asked: at 120, film and video at 24, 30 and 60 fit exactly **[computed]**, while 144 fits 24, 36, 48 and 72, so 30 and 60 fps content repeats unevenly at 144 unless a player paces it. Motion feel favours 144, video favours 120, and neither is forced.
- **Frame-rate hints.** Apple tells apps to use lower rates for small movements **[Apple]**. We don't throttle: on a fixed-rate link scanout runs regardless, and repainting a spinner is the only cost, so a hint is a toolkit-level micro-saving. `spinner` at 60 Hz, small. Bench first.
- **Idle.** Our idle already sends nothing: the display engine scans out the same frame.

## V. ProMotion-equivalent: variable refresh

**macOS:** Variable in Displays settings, for external displays; may flicker on unvalidated ones **[Apple]**. On ProMotion panels Core Animation arbitrates the rate; iPad 24-120, iPhone 10-120 **[Apple]**.

**Design for later:**
- KMS: the connector's `vrr_capable` and the CRTC's `VRR_ENABLED`, through Smithay 0.7's VRR API, as a per-display setting, like macOS.
- Compositor: a clock mode with no fixed phase. After idle, flip at once, instead of waiting up to a refresh. That's the real gain: first response immediate.

**Risk:** flicker at low rate; black screen on a bad modeset. Mitigation: setting reverts on a timer, like a resolution change; a low-frequency-compensation floor. **Size** L. Not now; it's listed so the frame clock leaves room for it.

## 7. Colour management, P3

**What a user feels:** photos and video in Firefox show P3 colours on a P3 monitor, and sRGB content matches them, not stretched.

**macOS:** every surface tagged, untagged treated as sRGB **[memory]**; `CAMetalLayer.colorspace` **[Apple]**. The display hardware converts per surface **[memory]**.

**Others:** GNOME 48 ([Phoronix](https://www.phoronix.com/news/GNOME-wp_color_management_v1)). KWin versioned since 6.1, official protocol in 6.4 and v2 in 6.6. wlroots protocol in 0.19, render plumbing in 0.20 ([timeline, secondary](https://www.youngju.dev/blog/2026-07-17-wayland-color-management-hdr-shipping-timeline-xserver.en)). KWin turns overlays off with a colour profile unless the KMS pipeline can do it.

**Starting point:** the sRGB-to-display shader is built (`rendering-path.md`, Colour) and works on every GPU.

**Design.**

1. **Protocol.** Smithay has none. `wayland-protocols` has `wp_color_manager_v1`. New module `compositor/src/colour/`: parametric descriptions with named TFs (sRGB, gamma 2.2, BT.1886, PQ, linear) and primaries (sRGB, P3, BT.2020), intents perceptual and relative, `get_output` from EDID primaries, `get_surface_feedback`. ICC only if a client needs it. Mesa then offers `VK_EXT_swapchain_colorspace` colour spaces by itself.
2. **Composition space.** Blend in encoded space with the display's own primaries: the toolkit's shadows and blends were designed with encoded blending, and macOS blends encoded for SDR **[memory]**. So each non-native surface runs decode, matrix, encode in the shader, then blends.
3. **Planes.** Colour-wrong planes may not be used. Where the driver has a per-plane colour pipeline (`COLOR_PIPELINE`), a plane may carry converted content; otherwise only content that is native or converted by the client, as KWin does.
4. **Precision.** Compose 8-bit, blue-noise dither after the matrix, cost about nothing. If the bench shows banding, compose 10-bit or RGBA16F, then dither in the copy to the display buffer.
5. **Night Light.** It uses the gamma ramp; verify it sits after per-surface conversion, or night tint on wide content looks off.

| Risk | Failure | Mitigation |
|---|---|---|
| EDID primaries wrong | everything off | `DefaultColorSpaceIsSRGB`, known-bad list, setting to treat as sRGB |
| Banding from matrix in 8-bit | steps in gradients | dither |
| Client tags wrongly | one window off | their bug |
| Firefox path differs | P3 not reached | test with gap-firefox; Firefox needs its pref on |

**Proof headless:** known pixel through each conversion ±1/255, protocol with a `wayland-info`-class client. **Owner:** photo against reference on the monitor. **Target:** per-surface cost ≤ +0.2 ms; no change to typical frames when everything is native. **Size:** M-L. **First proof:** protocol plus P3 surface in a headless capture.

## 8. Windowed video on a display plane

**What a user feels:** watching a video in a window costs almost nothing, with no fans and no GPU wake. Frames are exactly on time.

**macOS:** hardware surfaces for video (from memory); Apple's APIs let the system promote Metal layers to direct display (rendering-path.md's WWDC note).

**Others:** KWin overlays are off by default for driver freezes, 7.3 against 8.4 W on a laptop ([Phoronix](https://www.phoronix.com/news/Plasma-6.5-Overlay-Planes-KWin)). Mutter direct scanout only (from memory). wlroots leaves it to the compositor; sway and others use it on full screen.

**Depends on:** stage 3, hardware decode, 7's tags. Without hardware decode, software decode dominates, and this can't show a gain.

**Design, two levels.**

**8a. Overlay above the frame.** Video surface on an overlay when nothing covers it and its rectangle misses the window's rounded corners; full-screen goes to primary. Player controls: another plane if they are a surface, else composite while they show. `VideoElement` in `scene.rs` returns `underlying_storage`, using Smithay's atomic test to accept or decline. Declining falls back to the shader, in the same frame.

**8b. Underlay below the frame.** The general answer: rounded corners and overlaid controls keep working, the frame is above with a hole, and video always on the plane. Smithay 0.7 handles underlays, including the hole, when the primary has alpha and the element is opaque **[code]**. Needs a driver whose overlays can sit below the primary.

**Video timing:** with `commit-timing`, the player's frame lands on its vblank. 24, 30, 60 are exact on 120.

| Risk | Failure | Mitigation |
|---|---|---|
| Combination trips the driver: size, scale, format | **black until reboot** | `TEST_ONLY` always; first-use latch per format |
| Bandwidth: 4K NV12 beside a high-rate desktop | **underflow, flicker or black** | start with 1080p; rise to 4K on a test session; refuse above what passed |
| Colour differs from the shader path | visible **shift** when controls appear | toggling test; set the plane's YCbCr encoding from `wp_color_representation` |
| Night Light not applied to planes | video stays blue | verify; decline plane if not |
| Buffer freed during scanout | fault, black | hold until the flip that replaces it |
| Plane toggling flashes | frame of black | same atomic commit; test |
| Decoder pool short | stutter | stage 3's pool check |

**Experiments:** one test session for 8a: 1080p NV12 at 1:1, then scaled, at various positions, reading `flip_done` and dmesg. A trial takes over the owner's session at a time they choose. Recovery: SSH, reboot, latch.

**Target:** 1080p30 in a window ≤ +0.5 W against idle by B5; zero GPU frames while video alone changes. Report honest watts.

**Size:** 8a M, 8b S in lines but high in risk. **First proof:** 8a's atomic test accepting 1080p NV12 on an overlay.

## 9. HDR

**What a user feels:** HDR films and photos are bright where they should be, with desktop whites staying put. It's a per-display switch, as on a Mac (from memory for external displays).

**macOS:** EDR: `wantsExtendedDynamicRangeContent`, `edrMetadata`; tone mapping costs GPU and memory **[Apple]**. External HDR is a toggle in Displays, with a mode change (from memory).

**Others:** KWin, Mutter and wlroots all ship it. KWin's bugs show the traps: SDR tone-mapping unwanted, brightness inverted in Chromium, overexposure ([510759](https://bugs.kde.org/show_bug.cgi?id=510759), [515387](https://bugs.kde.org/show_bug.cgi?id=515387)). wlroots gets renderer support in 0.20.

**Driver:** the connector's `Colorspace` and `HDR_OUTPUT_METADATA`, where the driver has them. Link bandwidth for 10-bit or more at high rates may limit HDR to lower rates on some links; test before designing around 120 Hz.

**Compositor.** Compose in RGBA16F linear BT.2020, SDR at a reference white of 203 nits (setting, slider), encode to PQ 10-bit in the last pass, tone-map only what exceeds display peak (from EDID), never SDR. Pass-through for PQ video on a plane through 8. `get_output` advertises. Settings toggle per display, only when the display supports PQ, with the "keep these settings?" timer (the modeset is a short black).

| Risk | Failure | Mitigation |
|---|---|---|
| Mode won't link | **black screen** | revert timer, existing `refused` modes |
| Driver fault on an untried combination | **black until reboot** | first-use latch |
| Tone mapping wrong | blown highlights, like KWin's bugs | SDR never tone-mapped; white-level slider; test images |
| Text on YCbCr | soft edges | owner's eye |
| Power and bandwidth up | hotter | only while on |

**Target:** SDR content indistinguishable from SDR mode by eye at the set white level; PQ test patterns right by eye; toggle under 2 s. **Headless:** shaders only, by pixel. **Size:** L. **Last.**

## M. Shared compositor image, straight to scanout, shm

- **Shared image.** One compositor image per output instead of per swap buffer: saves up to 45 MB at 1440p. Requires damage age 1 always, and per-buffer tracking for what to copy. Copies shrink to real damage too.
- **Rendering straight into the display buffer.** It saves a copy per full frame, renders slower into LINEAR, blur needs a readable target, and it blocks 16F composition. **My view: don't, unless bench proves a typical-frame gain.** Better prize: compressed scanout below.
- **Compressed scanout.** A compressed modifier that both the GPU renders and the planes scan out (`IN_FORMATS`), for the compositor's own target. LINEAR scanout reads 1.77 GB/s at 120 Hz at 1440p forever **[computed]**, even idle, a plausible idle-power saving of real size, and macOS surely avoids it **[memory]**. Driver-dependent; measure by B5.
- **shm without a copy.** `udmabuf`, as KWin does. Toolkit uses shm only for first frames. Low value. Do it when a real shm client shows in a trace.

## What the owner needs to do

1. **The frame trace** (B2) on the real display after each scheduler stage, the same gesture for ten seconds.
2. **Later: the phone test,** ten minutes at 240 fps with a visible key press.
3. **The same phone test and filming animations on macOS, later,** on the same monitor. It's the only real macOS comparison.
4. **Eye tests:** colour switching, Glass after half-size blur, prediction by hand, HDR.

## Sources

- Apple: [Understanding hitches](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app), [Explore UI animation hitches](https://developer.apple.com/videos/play/tech-talks/10855/), [ProMotion displays](https://developer.apple.com/documentation/quartzcore/optimizing-iphone-and-ipad-apps-to-support-promotion-displays), [CAMetalDisplayLink](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink)
- Others: [Hugl on KWin's scheduling](https://planet.kde.org/xavers-blog-2024-06-24-fixing-kwins-performance-on-old-hardware), [KWin frame statistics](https://invent.kde.org/plasma/kwin/-/merge_requests/5975), [triple buffering debrief](https://discourse.ubuntu.com/t/triple-buffering-a-debrief/56314), [Dynamic triple buffering in GNOME 48](https://www.phoronix.com/news/GNOME-48-Triple-Buffering), [fifo-v1](https://wayland.app/protocols/fifo-v1), [commit-timing-v1](https://wayland.app/protocols/commit-timing-v1)
- Latency reference numbers: [Hume](https://thume.ca/2020/05/20/making-a-latency-tester/), [Harris on terminals](https://dev.to/lkhrs/measuring-terminal-latency-26m7)
