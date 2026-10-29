use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use unbound_audio::Level;
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{Axis, Context, Slider, Stack, Toggle, Ui, Widget};
use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-controlcenter.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const HUD_SIZE: (u32, u32) = (232, 64);
pub const PANEL_SIZE: (u32, u32) = (320, 200);
pub const HUD_DURATION: Duration = Duration::from_millis(1500);
pub const SEGMENTS: usize = 16;
const PADDING: f32 = 16.0;
const SPACING: f32 = 12.0;
const RADIUS: f32 = 12.0;
const SEGMENT_GAP: f32 = 2.0;
const SEGMENT_HEIGHT: f32 = 6.0;
const TITLE_SIZE: f32 = 13.0;
const VOLUME_STEP: f64 = 1.0 / SEGMENTS as f64;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Status {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
    pub do_not_disturb: bool,
    pub volume: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Wifi(bool),
    Bluetooth(bool),
    DoNotDisturb(bool),
    Volume(f32),
}

pub type Changes = Rc<RefCell<Vec<Change>>>;

fn toggle(label: &str, on: bool, changes: &Changes, change: fn(bool) -> Change) -> Box<dyn Widget> {
    let changes = Rc::clone(changes);
    Box::new(Toggle::new(label, on, move |on| {
        changes.borrow_mut().push(change(on));
    }))
}

pub fn panel(status: Status, palette: Palette, text: TextSystem, changes: &Changes) -> Ui {
    let mut controls = Vec::new();
    if let Some(on) = status.wifi {
        controls.push(toggle(&L10N.tr("wifi"), on, changes, Change::Wifi));
    }
    if let Some(on) = status.bluetooth {
        controls.push(toggle(
            &L10N.tr("bluetooth"),
            on,
            changes,
            Change::Bluetooth,
        ));
    }
    controls.push(toggle(
        &L10N.tr("do-not-disturb"),
        status.do_not_disturb,
        changes,
        Change::DoNotDisturb,
    ));
    if let Some(volume) = status.volume {
        let changes = Rc::clone(changes);
        controls.push(Box::new(Slider::new(
            &L10N.tr("sound"),
            f64::from(volume),
            (0.0, 1.0),
            VOLUME_STEP,
            move |volume| changes.borrow_mut().push(Change::Volume(volume as f32)),
        )));
    }
    Ui::new(
        Box::new(Stack::new(Axis::Vertical, SPACING, PADDING, controls)),
        Context::new(text, palette),
    )
}

pub fn filled_segments(level: Level) -> usize {
    if level.muted {
        return 0;
    }
    (level.volume.clamp(0.0, 1.0) * SEGMENTS as f32).round() as usize
}

pub fn card(palette: &Palette, size: (u32, u32)) -> Item {
    Item::Fill {
        shape: Shape::rounded(Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32), RADIUS),
        paint: palette.surface_elevated.with_alpha(245).into(),
    }
}

pub fn volume_hud(level: Level, palette: &Palette, text: &mut TextSystem) -> Scene {
    let (width, height) = HUD_SIZE;
    let title = if level.muted {
        L10N.tr("sound-muted")
    } else {
        L10N.tr("sound")
    };
    let title = text.paragraph(
        &title,
        TextStyle::new(TITLE_SIZE, palette.label_primary).weight(TextStyle::SEMIBOLD),
        None,
        TextAlign::Start,
    );
    let row = width as f32 - 2.0 * PADDING;
    let segment = (row - SEGMENT_GAP * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
    let top = height as f32 - PADDING - SEGMENT_HEIGHT;
    let filled = filled_segments(level);
    let mut items = vec![
        card(palette, HUD_SIZE),
        Item::Text {
            origin: Point::new(PADDING, PADDING - 4.0),
            paragraph: title,
        },
    ];
    items.extend((0..SEGMENTS).map(|index| Item::Fill {
        shape: Shape::rounded(
            Rect::new(
                PADDING + index as f32 * (segment + SEGMENT_GAP),
                top,
                segment,
                SEGMENT_HEIGHT,
            ),
            SEGMENT_HEIGHT / 2.0,
        ),
        paint: if index < filled {
            palette.label_primary.into()
        } else {
            palette.separator_color.into()
        },
    }));
    Scene {
        width,
        height,
        background: Color::TRANSPARENT,
        items,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("lib.rs"), include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
