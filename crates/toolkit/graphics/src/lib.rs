mod cpu;
mod damage;
mod draw;
#[cfg(feature = "gpu")]
mod gpu;
mod input;
mod outline;
mod render;
mod scene;
mod surface;
mod text;
mod title_bar;

pub use bande_design::{
    Accent, Appearance, BarForm, Color, Contrast, ControlSize, Elevation, Look, Material, Palette,
    Radius, Scheme, Shadow, Spacing, Style, TextRole, TextSize, Typeface, grown_line,
    limit_backdrop, window_edge_opacity,
};
pub use cpu::CpuRenderer;
pub use damage::{Region, damage};
#[cfg(feature = "gpu")]
pub use gpu::{GpuDevice, GpuRenderer};
pub use input::{
    Captured, Cursor, DOUBLE_CLICK_INTERVAL, Event, Key, Modifiers, Pointer, PointerButton, Scroll,
    ScrollPhase, ScrollSource, past_drag_slop,
};
pub use parley::FontData;
pub use render::{DrawError, Snapshot, pixels};
pub use scene::{
    Blend, Image, Item, Paint, Path, Picture, Point, Rect, Scene, Shape, Stop, Transform,
};
pub use surface::{PlacedSurface, SurfaceId, SurfaceScaling, placed_surfaces, surface_area};
pub use text::{
    Cut, GlyphRun, Paragraph, PositionedGlyph, TextAlign, TextStyle, TextSystem, shortened,
};
pub use title_bar::{
    ControlHover, FRAME_HEADER_HEIGHT, HEADER_LEADING, PLACEMENTS_AFTER_HOLD,
    PLACEMENTS_AFTER_REST, RESIZE_MARGIN, RESIZE_WALL_BAND, ResizeEdge, TitleBar,
    WINDOW_HEADER_HEIGHT, WalledSides, WindowControl, WindowControls, menu_button_area,
    resize_zone, title_style, walled_resize_zone, window_controls_end,
};
