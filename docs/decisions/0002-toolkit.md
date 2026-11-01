# UI Toolkit: Decision Record

Status: **accepted** (2026-10-29). Scope: which widget layer Bande's shell and apps are built
on. Rendering (Vello, Parley) is settled by [`0001-rendering.md`](0001-rendering.md) Decision 2.

## Context

The apps need macOS-quality depth: a full pointer model, a retained widget tree with focus,
interruptible spring animation, menus as `xdg_popup`s, scroll views with momentum, list, table
and grid views on one selection model, sheets, alerts and the AppKit control set, drag and drop,
AccessKit on AT-SPI and input methods. The options were to extend our own `bande-toolkit`
(A), adopt Linebender's Masonry (B), or adopt another Rust toolkit such as libcosmic (iced),
Slint or GTK 4 through gtk-rs (C).

## Recommendation

**Option A: extend `bande-toolkit`**, taking Masonry's architecture as a reference
rather than as a dependency.

1. **Masonry does not have what is expensive.** Of the feature list, Masonry saves the
   widget core: tree, passes, hover, capture, focus, IME routing, accessibility per
   widget, a test harness. That is perhaps 4–5 of the 25 weeks. The bulk (menus as
   popups, collection views with a selection model, momentum and elastic scrolling,
   springs, drag and drop, toolbars, sheets, AppKit-sized controls) is missing upstream
   and would be built either way, inside a framework whose own maintainers flag its
   scroll widget for a rewrite [8] and whose menus are a tracking issue [7].
2. **It would undo the stack we just chose.** Masonry pins Parley 0.8, AccessKit 0.24
   and paints through `imaging`, whose CPU backend uses `vello_cpu` 0.0.7 [3][11].
   Adopting it means downgrading Parley, Vello and AccessKit, or maintaining a fork that
   tracks ours; and the text quality work in our `Scene` (0.004 px flattening, unhinted
   outlines, Inter Display switch, whole-pixel baselines) would sit behind a bridge or be
   lost. Two Parley/fontique copies in one process would also load the font collection
   twice, which works against the 30 MB RSS budget (inference).
3. **Feel is the product.** Interruptible springs, momentum that matches macOS decay,
   rubber-band overscroll and menu tracking are where macOS smoothness lives. Owning the
   event loop, frame clock and every widget keeps those decisions in one codebase with
   our tests and measurements, rather than in upstream review of an experimental API that
   has made one release in a year [2].

Option C does not fit: libcosmic (iced) and Slint bring their own renderers and text
stacks, which would reverse Decision 2, and Slint has no built-in layer-shell [14].
libcosmic's accessibility is marked experimental [13]. GTK4 via gtk-rs is the
most complete, but it is a C runtime with its own renderer and CSS theming, which
means an own look fights the toolkit, and our Vello/Parley/AccessKit work is discarded.

## Consequences

- Every widget, the event loop, the frame clock and the animation model are ours
  ([toolkit.md](../architecture/toolkit.md) describes what exists); Masonry's passes were the
  reference design for the widget core.
- Our pointer vocabulary mirrors `ui-events`, so adopting Masonry's core later stays possible if
  it moves to our Vello, Parley and AccessKit versions.
- New widgets are built when an app needs them, not ahead of it.

## Sources

1. Masonry 0.4.0 crate documentation: <https://docs.rs/masonry/latest/masonry/>
2. Masonry versions on crates.io: <https://crates.io/crates/masonry/versions>
3. Xilem workspace `Cargo.toml` (pinned versions): <https://github.com/linebender/xilem/blob/main/Cargo.toml>
4. `RenderRoot` and `RenderRootSignal`: <https://github.com/linebender/xilem/blob/main/masonry_core/src/app/render_root.rs>
5. Masonry events (`Update`, `Ime`, `WindowEvent`): <https://github.com/linebender/xilem/blob/main/masonry_core/src/core/events.rs>
6. Masonry widgets on `main`: <https://github.com/linebender/xilem/tree/main/masonry/src/widgets>
7. Tracking issue for layers and menus: <https://github.com/linebender/xilem/issues/1343>
8. Tracking issue for Portal/Scroll widget: <https://github.com/linebender/xilem/issues/1344>
9. `VisualLayerPlan`: <https://github.com/linebender/xilem/blob/main/masonry_core/src/app/visual_layers.rs>
10. `ui-events` pointer types: <https://github.com/endoli/ui-events/blob/main/ui-events/src/pointer/mod.rs>
11. `imaging_vello_cpu` dependencies: <https://crates.io/crates/imaging_vello_cpu>
12. Xilem README: <https://github.com/linebender/xilem/blob/main/README.md>
13. libcosmic: <https://github.com/pop-os/libcosmic>
14. Slint discussion on `zwlr_layer_shell_v1`: <https://github.com/slint-ui/slint/discussions/1765>
15. `wl_pointer` (axis source, stop, value120, relative direction): <https://wayland.app/protocols/wayland#wl_pointer>
16. Cursor shape protocol: <https://wayland.app/protocols/cursor-shape-v1>
17. `xdg_popup` and layer-shell `get_popup`: <https://wayland.app/protocols/xdg-shell#xdg_popup>, <https://wayland.app/protocols/wlr-layer-shell-unstable-v1>; `gtk4-layer-shell`: <https://github.com/wmww/gtk4-layer-shell>
