# Separate GUI Binaries: Decision Record

Status: **decided** (2026-11-02). Scope: whether the GUI programs share one executable (or
other link-time tricks) to stop carrying a copy of the toolkit each.

## Context

Every GUI program links its own copy of the toolkit, Vello, wgpu, Parley and zbus: 13–28 MB per
`dist` binary, about 167 MB for the shell, lock screen, login window, screenshot tool, polkit
agent, file chooser, keychain prompt, Files and Settings. The `dist` profile already has the
settings that shrink code without slowing it (fat LTO, one codegen unit, `panic = "abort"`,
stripped, wgpu with Vulkan only); thin LTO or more codegen units make it bigger, and `opt-level`
`s`/`z` make it slower. The bar for any change: no program starts slower (cold or warm), uses
more memory alone, wakes when idle, or loses isolation or security.

## Decision

Keep one executable per program, and link every binary with packed relative relocations
(`-Wl,-z,pack-relative-relocs`, DT_RELR) from `[build] rustflags` in `.cargo/config.toml`;
`ci/release/verify.sh` adds its flags through `CARGO_BUILD_RUSTFLAGS`, which Cargo merges with
that list (`RUSTFLAGS` would replace it).

## Alternatives measured

`dist` builds of both sides, on the mini, against a headless compositor with private buses;
time from exec to the first buffer the compositor receives, 45–48 launches per case.

- **One multicall binary for the nine programs** (symlinks, dispatch on `argv[0]`): 35.8 MB
  instead of 167 MB, and 14–19 % less PSS when the shell, polkit agent and file chooser run
  together. Rejected:
  - Cold start is 2–16 ms slower for every program (the lock screen 56 → 69 ms, Settings 75 →
    82 ms, the polkit agent 12 → 19 ms): the larger file is read from more places and every
    program loads `libpipewire`.
  - Every process relocates the whole binary: about 1.05 MB of dirty RELRO instead of 0.3–0.9 MB,
    and 3–6 MB more RSS at start for each program alone.
  - The keychain identifies apps by `/proc/<pid>/exe`: nine programs would share one identity and
    read each other's items. The login window, run by greetd before anyone signs in, would map
    every app's code and `libpipewire`.
  - `/proc/self/exe` re-execs (content workers, the shell's dialogs), the taskbar's match of a
    process to its app, Settings' check for programs running replaced code and coredump names
    all see the shared path.

  Grouping only some programs keeps the same costs for the smaller member of each group.
- **Packed relative relocations** (`-Wl,-z,pack-relative-relocs`, DT_RELR): about 6.5 MB smaller
  across the desktop's binaries, cold start 1–10 ms faster and up to 1 MB less RSS at start
  (Settings 2.7 MB), but warm start up to 0.1 ms slower: glibc decodes RELR bitmaps bit by bit,
  and the scattered relocations of Rust's `.data.rel.ro` make that branchy loop slower than plain
  RELA. Adopted: the owner accepted the 0.1 ms warm cost for the faster cold start, the lower
  RSS and the smaller packages. glibc ≥ 2.36 loads such binaries; `dpkg-shlibdeps` adds the
  `libc6` version through the `GLIBC_ABI_DT_RELR` symbol version.
- **ICU segmentation data loaded at run time** (about 2.9 MB per binary): needs a blob data
  provider in Parley's segmenter and slower lookups; not pursued.
