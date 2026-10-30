mod choice;
mod picture;

pub use choice::{Choice, DEFAULT_WALLPAPER, Fit, Variant, bundled, choose, follow, read};
pub use picture::{Picture, RenderError, backdrop, render};
