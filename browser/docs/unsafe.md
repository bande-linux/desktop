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
