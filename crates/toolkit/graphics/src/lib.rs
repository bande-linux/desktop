mod cpu;
mod damage;
mod draw;
mod gpu;
mod input;
mod outline;
mod render;
mod scene;
mod text;
mod title_bar;

pub use cpu::CpuRenderer;
pub use damage::{Region, damage};
pub use gpu::{GpuDevice, GpuRenderer};
pub use input::{
    Cursor, DOUBLE_CLICK_INTERVAL, Event, Key, Modifiers, Pointer, PointerButton, Scroll,
    ScrollPhase, ScrollSource,
};
pub use render::{RenderError, Snapshot, pixels};
pub use scene::{Image, Item, Paint, Path, Point, Rect, Scene, Shape, Stop};
pub use text::{GlyphRun, Paragraph, PositionedGlyph, TextAlign, TextStyle, TextSystem};
pub use title_bar::{
    FRAME_HEADER_HEIGHT, HEADER_HEIGHT, RESIZE_CORNER, RESIZE_MARGIN, ResizeEdge, TitleBar,
    WindowControl, resize_edge, title_style, window_controls_area,
};
pub use unbound_design::{
    Accent, Appearance, Color, Contrast, ControlSize, Elevation, Palette, Radius, Scheme, Shadow,
    Spacing, TextRole, TextSize, Typeface, grown_line,
};
