use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use unbound_audio::Level;
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{
    Axis, BUTTON_HEIGHT, Button, Context, Label, Menu, MenuItem, SLIDER_HEIGHT, SWITCH_SIZE,
    Slider, Stack, Toggle, Ui, Widget,
};
use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-controlcenter.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const HUD_SIZE: (f32, f32) = (232.0, 64.0);
pub const PANEL_WIDTH: f32 = 320.0;
pub const EDGE: f32 = 8.0;
pub const HUD_DURATION: Duration = Duration::from_millis(1500);
pub const SEGMENTS: usize = 16;
const PADDING: f32 = 16.0;
const SPACING: f32 = 12.0;
const LABEL_HEIGHT: f32 = 16.0;
const RADIUS: f32 = 14.0;
const SEGMENT_GAP: f32 = 2.0;
const SEGMENT_HEIGHT: f32 = 6.0;
const TITLE_SIZE: f32 = 13.0;
const LEVEL_STEP: f64 = 1.0 / SEGMENTS as f64;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub wifi: Option<bool>,
    pub network: Option<String>,
    pub bluetooth: Option<bool>,
    pub do_not_disturb: bool,
    pub volume: Option<f32>,
    pub output: Option<String>,
    pub brightness: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Wifi(bool),
    Bluetooth(bool),
    DoNotDisturb(bool),
    Volume(f32),
    Brightness(f32),
    ChooseNetwork,
    ChooseOutput,
}

pub type Changes = Rc<RefCell<Vec<Change>>>;

fn toggle(label: &str, on: bool, changes: &Changes, change: fn(bool) -> Change) -> Box<dyn Widget> {
    let changes = Rc::clone(changes);
    Box::new(Toggle::new(label, on, move |on| {
        changes.borrow_mut().push(change(on));
    }))
}

fn chooser(title: &str, changes: &Changes, change: Change) -> Box<dyn Widget> {
    let changes = Rc::clone(changes);
    Box::new(Button::new(title, move || {
        changes.borrow_mut().push(change.clone());
    }))
}

fn slider(
    label: &str,
    value: f32,
    changes: &Changes,
    change: fn(f32) -> Change,
) -> Vec<Box<dyn Widget>> {
    let changes = Rc::clone(changes);
    vec![
        Box::new(Label::new(label)),
        Box::new(Slider::new(
            label,
            f64::from(value),
            (0.0, 1.0),
            LEVEL_STEP,
            move |value| changes.borrow_mut().push(change(value as f32)),
        )),
    ]
}

pub fn panel(status: &Status, palette: Palette, text: TextSystem, changes: &Changes) -> (Ui, f32) {
    let mut controls: Vec<Box<dyn Widget>> = Vec::new();
    let mut heights = Vec::new();
    if let Some(on) = status.wifi {
        controls.push(toggle(&L10N.tr("wifi"), on, changes, Change::Wifi));
        heights.push(SWITCH_SIZE.height);
        if on {
            let title = status
                .network
                .clone()
                .unwrap_or_else(|| L10N.tr("choose-network"));
            controls.push(chooser(&title, changes, Change::ChooseNetwork));
            heights.push(BUTTON_HEIGHT);
        }
    }
    if let Some(on) = status.bluetooth {
        controls.push(toggle(
            &L10N.tr("bluetooth"),
            on,
            changes,
            Change::Bluetooth,
        ));
        heights.push(SWITCH_SIZE.height);
    }
    controls.push(toggle(
        &L10N.tr("do-not-disturb"),
        status.do_not_disturb,
        changes,
        Change::DoNotDisturb,
    ));
    heights.push(SWITCH_SIZE.height);
    if let Some(brightness) = status.brightness {
        controls.extend(slider(
            &L10N.tr("display"),
            brightness,
            changes,
            Change::Brightness,
        ));
        heights.extend([LABEL_HEIGHT, SLIDER_HEIGHT]);
    }
    if let Some(volume) = status.volume {
        controls.extend(slider(&L10N.tr("sound"), volume, changes, Change::Volume));
        heights.extend([LABEL_HEIGHT, SLIDER_HEIGHT]);
        if let Some(output) = &status.output {
            controls.push(chooser(output, changes, Change::ChooseOutput));
            heights.push(BUTTON_HEIGHT);
        }
    }
    let height = 2.0 * PADDING
        + heights.iter().sum::<f32>()
        + SPACING * heights.len().saturating_sub(1) as f32;
    let ui = Ui::new(
        Box::new(Stack::new(Axis::Vertical, SPACING, PADDING, controls)),
        Context::new(text, palette),
    );
    (ui, height.ceil())
}

pub fn chooser_menu(choices: &[(String, String, bool)]) -> Menu {
    Menu::new(
        choices
            .iter()
            .map(|(title, action, chosen)| MenuItem::new(title, action).checked(*chosen))
            .collect(),
    )
}

fn filled_segments(level: f32) -> usize {
    (level.clamp(0.0, 1.0) * SEGMENTS as f32).round() as usize
}

pub fn volume_segments(level: Level) -> usize {
    if level.muted {
        return 0;
    }
    filled_segments(level.volume)
}

pub fn card(palette: &Palette, rect: Rect, items: &mut Vec<Item>) {
    let shape = Shape::rounded(rect, RADIUS);
    items.push(Item::Shadow {
        shape,
        offset: Point::new(0.0, 6.0),
        blur_radius: 24.0,
        color: Color::hex(0).with_alpha(50),
    });
    items.push(Item::Fill {
        shape,
        paint: palette.surface_elevated.with_alpha(248).into(),
    });
    items.push(Item::Stroke {
        shape,
        width: 0.5,
        paint: palette.separator_color.into(),
    });
}

pub fn volume_hud(level: Level, palette: &Palette, text: &mut TextSystem) -> Vec<Item> {
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
    let row = width - 2.0 * PADDING;
    let segment = (row - SEGMENT_GAP * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
    let top = height - PADDING - SEGMENT_HEIGHT;
    let filled = volume_segments(level);
    let mut items = Vec::new();
    card(palette, Rect::new(0.0, 0.0, width, height), &mut items);
    items.push(Item::Text {
        origin: Point::new(PADDING, PADDING - 4.0),
        paragraph: title,
    });
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
    items
}

pub fn scene(width: u32, height: u32, items: Vec<Item>) -> Scene {
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
