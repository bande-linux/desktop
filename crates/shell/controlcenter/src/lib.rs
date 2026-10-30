pub mod media;
mod now_playing;

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use unbound_audio::{Level, VolumeKey};
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{
    Axis, BUTTON_HEIGHT, Button, Context, Label, LevelIndicator, Menu, MenuItem, PROGRESS_HEIGHT,
    PopUpButton, SLIDER_HEIGHT, SWITCH_SIZE, Slider, Stack, Toggle, Ui, Widget,
};
use unbound_toolkit::{
    Color, Image, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use media::{Command, Player};
pub use now_playing::clock;

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
const BLUR_BANDS: usize = 5;
const SEGMENT_GAP: f32 = 2.0;
const SEGMENT_HEIGHT: f32 = 6.0;
const TITLE_SIZE: f32 = 13.0;
const LEVEL_STEP: f64 = 1.0 / SEGMENTS as f64;

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub wifi: Option<bool>,
    pub network: Option<String>,
    pub bluetooth: Option<bool>,
    pub do_not_disturb: bool,
    pub volume: Option<f32>,
    pub outputs: Vec<Choice>,
    pub output: Option<String>,
    pub brightness: Option<f32>,
    pub keyboard_brightness: Option<f32>,
    pub inputs: Vec<Choice>,
    pub input: Option<String>,
    pub input_muted: Option<bool>,
    pub input_level: Rc<Cell<f32>>,
    pub player: Option<Player>,
    pub artwork: Option<Image>,
    pub players: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub key: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Wifi(bool),
    Bluetooth(bool),
    DoNotDisturb(bool),
    Volume(f32),
    Brightness(f32),
    KeyboardBrightness(f32),
    ChooseNetwork,
    Output(String),
    InputMuted(bool),
    Input(String),
    Media(Command),
    RaisePlayer,
    Player(String),
}

pub type Changes = Rc<RefCell<Vec<Change>>>;

fn toggle(label: &str, on: bool, changes: &Changes, change: fn(bool) -> Change) -> Box<dyn Widget> {
    let changes = Rc::clone(changes);
    let switch = Toggle::new(label, on, move |on| {
        changes.borrow_mut().push(change(on));
    });
    Box::new(Stack::new(
        Axis::Horizontal,
        SPACING,
        0.0,
        vec![Box::new(switch), Box::new(Label::new(label))],
    ))
}

fn chooser(title: &str, changes: &Changes, change: Change) -> Box<dyn Widget> {
    let changes = Rc::clone(changes);
    Box::new(Button::new(title, move || {
        changes.borrow_mut().push(change.clone());
    }))
}

fn pop_up(
    label: &str,
    choices: &[Choice],
    selected: Option<&str>,
    changes: &Changes,
    change: fn(String) -> Change,
) -> Box<dyn Widget> {
    let changes = Rc::clone(changes);
    let keys: Vec<String> = choices.iter().map(|choice| choice.key.clone()).collect();
    let titles: Vec<&str> = choices.iter().map(|choice| choice.title.as_str()).collect();
    let selected = selected
        .and_then(|selected| keys.iter().position(|key| key == selected))
        .unwrap_or(0);
    Box::new(PopUpButton::new(label, &titles, selected, move |index| {
        if let Some(key) = keys.get(index) {
            changes.borrow_mut().push(change(key.clone()));
        }
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
    if let Some(brightness) = status.keyboard_brightness {
        controls.extend(slider(
            &L10N.tr("keyboard-brightness"),
            brightness,
            changes,
            Change::KeyboardBrightness,
        ));
        heights.extend([LABEL_HEIGHT, SLIDER_HEIGHT]);
    }
    if let Some(volume) = status.volume {
        controls.extend(slider(&L10N.tr("sound"), volume, changes, Change::Volume));
        heights.extend([LABEL_HEIGHT, SLIDER_HEIGHT]);
        if status.outputs.len() > 1 {
            controls.push(pop_up(
                &L10N.tr("output"),
                &status.outputs,
                status.output.as_deref(),
                changes,
                Change::Output,
            ));
            heights.push(BUTTON_HEIGHT);
        }
    }
    if let Some(muted) = status.input_muted {
        controls.push(toggle(&L10N.tr("microphone"), !muted, changes, |on| {
            Change::InputMuted(!on)
        }));
        controls.push(Box::new(LevelIndicator::new(
            &L10N.tr("input-level"),
            Rc::clone(&status.input_level),
        )));
        heights.extend([SWITCH_SIZE.height, PROGRESS_HEIGHT]);
        if status.inputs.len() > 1 {
            controls.push(pop_up(
                &L10N.tr("input"),
                &status.inputs,
                status.input.as_deref(),
                changes,
                Change::Input,
            ));
            heights.push(BUTTON_HEIGHT);
        }
    }
    if let Some(player) = &status.player {
        heights.push(now_playing::height(player));
        controls.push(Box::new(now_playing::NowPlaying::new(
            player.clone(),
            status.artwork.clone(),
            changes,
        )));
        if status.players.len() > 1 {
            controls.push(pop_up(
                &L10N.tr("player"),
                &status.players,
                Some(&player.bus_name),
                changes,
                Change::Player,
            ));
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hud {
    Volume(Level),
    Display(f32),
    Keyboard(f32),
}

impl Hud {
    fn title(self) -> String {
        match self {
            Self::Volume(Level { muted: true, .. }) => L10N.tr("sound-muted"),
            Self::Volume(_) => L10N.tr("sound"),
            Self::Display(_) => L10N.tr("display"),
            Self::Keyboard(_) => L10N.tr("keyboard-brightness"),
        }
    }

    fn level(self) -> f32 {
        match self {
            Self::Volume(Level { muted: true, .. }) => 0.0,
            Self::Volume(level) => level.volume,
            Self::Display(level) | Self::Keyboard(level) => level,
        }
    }

    pub fn segment_fills(self) -> [f32; SEGMENTS] {
        let filled = self.level().clamp(0.0, 1.0) * SEGMENTS as f32;
        std::array::from_fn(|index| (filled - index as f32).clamp(0.0, 1.0))
    }
}

pub const SYSFS_LEDS: &str = "/sys/class/leds";
const KEYBOARD_LIGHT: &str = "kbd_backlight";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLight {
    pub name: String,
    directory: PathBuf,
}

fn read_number(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

impl KeyboardLight {
    pub fn find(leds: &Path) -> Option<Self> {
        let mut names: Vec<String> = fs::read_dir(leds)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.contains(KEYBOARD_LIGHT))
            .collect();
        names.sort();
        let name = names.into_iter().next()?;
        Some(Self {
            directory: leds.join(&name),
            name,
        })
    }

    pub fn maximum(&self) -> Option<u32> {
        read_number(&self.directory.join("max_brightness")).filter(|maximum| *maximum > 0)
    }

    pub fn level(&self) -> Option<f32> {
        let maximum = self.maximum()?;
        Some(read_number(&self.directory.join("brightness"))? as f32 / maximum as f32)
    }
}

pub fn stepped(level: f32, up: bool, fine: bool) -> f32 {
    let key = if up { VolumeKey::Up } else { VolumeKey::Down };
    Level {
        volume: level,
        muted: false,
    }
    .press(key, fine)
    .volume
}

pub fn blur_rects(rect: Rect) -> Vec<Rect> {
    Shape::rounded(rect, RADIUS).bands(BLUR_BANDS)
}

pub fn material(palette: Palette, translucent: bool) -> Palette {
    Palette {
        surface_elevated: palette
            .surface_elevated
            .with_alpha(if translucent { 214 } else { 248 }),
        ..palette
    }
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
        paint: palette.surface_elevated.into(),
    });
    items.push(Item::Stroke {
        shape,
        width: 0.5,
        paint: palette.separator_color.into(),
    });
}

pub fn hud(hud: Hud, palette: &Palette, text: &mut TextSystem) -> Vec<Item> {
    let (width, height) = HUD_SIZE;
    let title = text.paragraph(
        &hud.title(),
        TextStyle::new(TITLE_SIZE, palette.label_primary).weight(TextStyle::SEMIBOLD),
        None,
        TextAlign::Start,
    );
    let row = width - 2.0 * PADDING;
    let segment = (row - SEGMENT_GAP * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
    let top = height - PADDING - SEGMENT_HEIGHT;
    let mut items = Vec::new();
    card(palette, Rect::new(0.0, 0.0, width, height), &mut items);
    items.push(Item::Text {
        origin: Point::new(PADDING, PADDING - 4.0),
        paragraph: title,
    });
    for (index, fill) in hud.segment_fills().into_iter().enumerate() {
        let rect = Rect::new(
            PADDING + index as f32 * (segment + SEGMENT_GAP),
            top,
            segment,
            SEGMENT_HEIGHT,
        );
        let shape = Shape::rounded(rect, SEGMENT_HEIGHT / 2.0);
        items.push(Item::Fill {
            shape,
            paint: palette.separator_color.into(),
        });
        if fill > 0.0 {
            items.push(Item::Clip {
                shape: Shape::rect(Rect::new(rect.x, rect.y, rect.width * fill, rect.height)),
                items: vec![Item::Fill {
                    shape,
                    paint: palette.label_primary.into(),
                }],
            });
        }
    }
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
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("now_playing.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
