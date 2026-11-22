use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::Path;

use bande_graphics::{
    Color, ControlSize, CpuRenderer, Image, Item, Palette, Point, Rect, Scene, Scheme, Shape,
};
use bande_spring::{Spring, State};
use bande_widgets::{
    CAPS_LOCK_SIZE, FOCUS_RING_OUTSET, PROGRESS_CELL_GAP, PROGRESS_CELLS, PROGRESS_HEIGHT,
    caps_lock_for, indeterminate_sweep, paint_field_for, progress_cells_for,
};
use resvg::tiny_skia::{IntSize, Pixmap};

pub const MARK: &str = include_str!("../../../../data/branding/mark.svg");
const LOCK: &str =
    include_str!("../../../../data/icons/Bande/symbolic/status/system-lock-screen-symbolic.svg");
const KEYBOARD: &str =
    include_str!("../../../../data/icons/Bande/symbolic/status/input-keyboard-symbolic.svg");

pub const PLYMOUTH_THEME: &str = "bande";
const THEMES: &str = "/usr/share/plymouth/themes";
const FONT: &str = "Inter 10";
const MARK_SIZE: u32 = 96;
const MARK_ALIGNMENT: f64 = 0.5;
const INDICATOR_ALIGNMENT: f64 = 0.64;
const CELL_WIDTH: f32 = 6.0;
const PLYMOUTH_FRAME_RATE: f64 = 30.0;
const THROBBER_SECONDS: f64 = 2.0;
const END_FRAMES: usize = 8;
const ENTRY_WIDTH: f32 = 240.0;
const BULLET: f32 = 6.0;
const BULLET_PITCH: f32 = 12.0;
const BULLET_RADIUS: f32 = 1.0;
const ICON: u32 = 16;
const LOCK_GAP: u32 = 12;

fn palette() -> Palette {
    Palette::for_scheme(Scheme::Dark)
}

pub fn splash_background() -> Color {
    palette().deep
}

fn hex(color: Color) -> String {
    format!("0x{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn mark_image(size: u32) -> Option<Image> {
    let ink = palette().ink;
    let coloured = MARK.replace(
        "currentColor",
        &format!("#{:02x}{:02x}{:02x}", ink.r, ink.g, ink.b),
    );
    Image::from_svg(coloured.as_bytes(), size, size)
}

fn draw(width: u32, height: u32, background: Color, items: Vec<Item>) -> io::Result<Pixmap> {
    let scene = Scene {
        width,
        height,
        background,
        items,
    };
    let snapshot = CpuRenderer::new()
        .render(&scene)
        .map_err(|error| io::Error::other(error.to_string()))?;
    IntSize::from_wh(snapshot.width, snapshot.height)
        .and_then(|size| Pixmap::from_vec(snapshot.rgba, size))
        .ok_or_else(|| io::Error::other("the image has no size"))
}

fn write_png(directory: &Path, name: &str, pixmap: &Pixmap) -> io::Result<()> {
    pixmap
        .save_png(directory.join(name))
        .map_err(|error| io::Error::other(format!("cannot write {name}: {error}")))
}

fn indicator_width() -> f32 {
    PROGRESS_CELLS as f32 * CELL_WIDTH + (PROGRESS_CELLS - 1) as f32 * PROGRESS_CELL_GAP
}

fn indicator(lit: impl Fn(f32) -> f32, alpha: f32) -> io::Result<Pixmap> {
    let width = indicator_width();
    let mut cells = Vec::new();
    progress_cells_for(
        &palette(),
        Rect::new(0.0, 0.0, width, PROGRESS_HEIGHT),
        lit,
        &mut cells,
    );
    draw(
        width as u32,
        PROGRESS_HEIGHT as u32,
        Color::TRANSPARENT,
        vec![Item::Opacity {
            alpha,
            items: cells,
        }],
    )
}

fn fade_out(frame: usize) -> f32 {
    if frame + 1 >= END_FRAMES {
        return 0.0;
    }
    let still = State {
        position: 1.0,
        velocity: 0.0,
    };
    let seconds = frame as f64 / PLYMOUTH_FRAME_RATE;
    Spring::FADE
        .state_at(still, 0.0, seconds)
        .position
        .clamp(0.0, 1.0) as f32
}

fn icon(source: &str, width: u32, x: u32) -> io::Result<Pixmap> {
    let image = Image::symbolic(source, ICON, palette().ink2)
        .ok_or_else(|| io::Error::other("cannot draw a symbol"))?;
    draw(
        width,
        ICON,
        Color::TRANSPARENT,
        vec![Item::Image {
            rect: Rect::new(x as f32, 0.0, ICON as f32, ICON as f32),
            image,
        }],
    )
}

fn entry() -> io::Result<Pixmap> {
    let height = ControlSize::Regular.height();
    let mut items = Vec::new();
    paint_field_for(
        &palette(),
        Rect::new(FOCUS_RING_OUTSET, FOCUS_RING_OUTSET, ENTRY_WIDTH, height),
        true,
        &mut items,
    );
    draw(
        (ENTRY_WIDTH + 2.0 * FOCUS_RING_OUTSET) as u32,
        (height + 2.0 * FOCUS_RING_OUTSET) as u32,
        Color::TRANSPARENT,
        items,
    )
}

fn bullet() -> io::Result<Pixmap> {
    let inset = (BULLET_PITCH - BULLET) / 2.0;
    draw(
        BULLET_PITCH as u32,
        BULLET_PITCH as u32,
        Color::TRANSPARENT,
        vec![Item::Fill {
            shape: Shape::rounded(Rect::new(inset, inset, BULLET, BULLET), BULLET_RADIUS),
            paint: palette().ink.into(),
        }],
    )
}

fn caps_lock() -> io::Result<Pixmap> {
    let offset = (ICON as f32 - CAPS_LOCK_SIZE) / 2.0;
    let mut items = Vec::new();
    caps_lock_for(
        &palette(),
        Point::new(LOCK_GAP as f32 + offset, offset),
        CAPS_LOCK_SIZE,
        &mut items,
    );
    draw(LOCK_GAP + ICON, ICON, Color::TRANSPARENT, items)
}

fn theme_file(directory: &str) -> String {
    let palette = palette();
    let background = hex(palette.deep);
    let width = indicator_width();
    format!(
        "[Plymouth Theme]
Name=Bande
Description=The start, shut down and update screen of the Bande desktop
ModuleName=two-step

[two-step]
ImageDir={directory}
Font={FONT}
TitleFont={FONT}
WatermarkHorizontalAlignment=.5
WatermarkVerticalAlignment={MARK_ALIGNMENT}
HorizontalAlignment=.5
VerticalAlignment={INDICATOR_ALIGNMENT}
DialogHorizontalAlignment=.5
DialogVerticalAlignment={INDICATOR_ALIGNMENT}
TitleHorizontalAlignment=.5
TitleVerticalAlignment={INDICATOR_ALIGNMENT}
ProgressBarHorizontalAlignment=.5
ProgressBarVerticalAlignment={INDICATOR_ALIGNMENT}
ProgressBarWidth={width}
ProgressBarHeight={PROGRESS_HEIGHT}
ProgressBarBackgroundColor={well}
ProgressBarForegroundColor={accent}
Transition=none
TransitionDuration=0.0
BackgroundStartColor={background}
BackgroundEndColor={background}
ConsoleLogBackgroundColor={background}
MessageBelowAnimation=true

[boot-up]
UseEndAnimation=true

[shutdown]
UseEndAnimation=false

[reboot]
UseEndAnimation=false

[updates]
UseProgressBar=true
UseAnimation=false

[system-upgrade]
UseProgressBar=true
UseAnimation=false

[firmware-upgrade]
UseProgressBar=true
UseAnimation=false
",
        well = hex(palette.well),
        accent = hex(palette.accent),
    )
}

pub fn write_plymouth_theme(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let installed = format!("{THEMES}/{PLYMOUTH_THEME}");
    fs::write(
        directory.join(format!("{PLYMOUTH_THEME}.plymouth")),
        theme_file(&installed),
    )?;
    let mark = mark_image(MARK_SIZE).ok_or_else(|| io::Error::other("cannot draw the mark"))?;
    let watermark = draw(
        MARK_SIZE,
        MARK_SIZE,
        Color::TRANSPARENT,
        vec![Item::Image {
            rect: Rect::new(0.0, 0.0, MARK_SIZE as f32, MARK_SIZE as f32),
            image: mark,
        }],
    )?;
    write_png(directory, "watermark.png", &watermark)?;
    let frames = (THROBBER_SECONDS * PLYMOUTH_FRAME_RATE) as usize;
    for frame in 0..frames {
        let pixmap = indicator(indeterminate_sweep(frame as f32 / frames as f32), 1.0)?;
        write_png(
            directory,
            &format!("throbber-{:04}.png", frame + 1),
            &pixmap,
        )?;
    }
    for frame in 0..END_FRAMES {
        let pixmap = indicator(indeterminate_sweep(0.0), fade_out(frame))?;
        write_png(
            directory,
            &format!("animation-{:04}.png", frame + 1),
            &pixmap,
        )?;
    }
    write_png(directory, "entry.png", &entry()?)?;
    write_png(directory, "bullet.png", &bullet()?)?;
    write_png(directory, "lock.png", &icon(LOCK, ICON + LOCK_GAP, 0)?)?;
    write_png(directory, "keyboard.png", &icon(KEYBOARD, ICON, 0)?)?;
    write_png(directory, "capslock.png", &caps_lock()?)?;
    let keymaps = directory.join("keymap-render.png");
    if keymaps.symlink_metadata().is_ok() {
        fs::remove_file(&keymaps)?;
    }
    symlink("../spinner/keymap-render.png", keymaps)
}

pub fn plymouth_scale(size: (u32, u32), millimetres: (u32, u32)) -> u32 {
    let (width, height) = (f64::from(size.0), f64::from(size.1));
    let (width_mm, height_mm) = (f64::from(millimetres.0), f64::from(millimetres.1));
    let encodes_aspect = matches!(
        millimetres,
        (160, 90) | (160, 100) | (16, 9) | (16, 10) | (0, _) | (_, 0)
    );
    if encodes_aspect {
        return 1;
    }
    let inches = width_mm.hypot(height_mm) / 25.4;
    let dpi = width.hypot(height) / inches;
    let target = if inches >= 20.0 { 110.0 } else { 135.0 };
    if dpi / target > 1.625 { 2 } else { 1 }
}

pub struct Frame {
    pub size: (u32, u32),
    pub premultiplied_rgba: Vec<u8>,
}

pub fn splash_frame(size: (u32, u32), scale: u32) -> io::Result<Frame> {
    let scale = scale.max(1);
    let (width, height) = (size.0 / scale, size.1 / scale);
    let left = (f64::from(width) * 0.5 - f64::from(MARK_SIZE) * 0.5) as f32;
    let top = (f64::from(height) * MARK_ALIGNMENT - f64::from(MARK_SIZE) * MARK_ALIGNMENT) as f32;
    let mark =
        mark_image(MARK_SIZE * scale).ok_or_else(|| io::Error::other("cannot draw the mark"))?;
    let scene = Scene {
        width,
        height,
        background: splash_background(),
        items: vec![Item::Image {
            rect: Rect::new(left, top, MARK_SIZE as f32, MARK_SIZE as f32),
            image: mark,
        }],
    };
    let pixels = (
        bande_graphics::pixels(width, f64::from(scale)),
        bande_graphics::pixels(height, f64::from(scale)),
    );
    let mut premultiplied_rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
    CpuRenderer::new()
        .render_into(&scene, f64::from(scale), &mut premultiplied_rgba)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(Frame {
        size: pixels,
        premultiplied_rgba,
    })
}
