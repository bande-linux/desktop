# Bande web engine

A web engine written from scratch in Rust for the Bande Browser app: HTML, CSS, layout, DOM, rendering through Bande's own wgpu/vello stack, a sandboxed multi-process model, and JavaScript. The goal is full compatibility with today's web while staying far leaner than existing engines.

The plan and milestones are in `NOTES/plans/browser.md` (the owner's brief is `NOTES/browser.md`). The Browser app lives in `bande-linux/apps`; this repository is the engine it embeds.

Licensed under either of Apache-2.0 or MIT, at your option.
