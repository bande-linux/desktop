# Audited `unsafe`

## `web-av1`

The crate denies `unsafe_code` and allows it per block; every block carries a `SAFETY` comment
(clippy `undocumented_unsafe_blocks` is denied). It calls rav1d's C ABI, the only API rav1d
offers. The invariants:

- `Decoder` owns the context from `dav1d_open` and closes it exactly once in `Drop`.
- Input bytes are copied into a buffer from `dav1d_data_create`, sized from the input, and
  released with `dav1d_data_unref` whether or not rav1d consumed them.
- `Picture` owns a frame from a successful `dav1d_get_picture` and unreferences it once in
  `Drop`; pictures that were never filled are not unreferenced.
- Plane slices are built from rav1d's own `stride` and the frame's visible size, and live only
  while the `Picture` holds its reference.
- `frame_size_limit` and the caller's `max_pixels` bound the frame before conversion; the RGBA
  output is allocated by safe code.

## `web-ipc`

Same rules as `web-av1`: `unsafe_code` is denied and allowed per block, each with a `SAFETY`
comment. Sockets, fd passing, memfds, seals, Landlock and seccomp go through safe APIs; only these
need `unsafe`:

- `Mapping::new` maps a received memfd read-only and shared, only after checking that it carries
  the write, shrink, grow and seal seals. The bytes can then neither change nor go away (no
  `SIGBUS`) while mapped, which is what makes handing out `&[u8]`, and `Send` and `Sync`, sound.
  Its length comes from `fstat` and is capped by the caller's limit before mapping.
- `Mapping::drop` unmaps exactly the range `new` mapped; slices borrow the `Mapping`.
- `sandbox::unshare_namespaces` calls `unshare` with user, network and IPC flags only. rustix
  marks `unshare` unsafe because of `CLONE_FILES` and `CLONE_FS`, which these flags exclude.
