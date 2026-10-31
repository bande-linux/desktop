mod client;
pub mod media;
mod modules;
mod now_playing;
mod person;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use unbound_audio::{Level, VolumeKey};
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{
    Axis, Context, LevelIndicator, Menu, MenuItem, PopUpButton, Size, Slider, Stack, Ui, Widget,
};
use unbound_toolkit::{
    Color, DisplayEdge, Elevation, Image, Item, Palette, Point, Radius, Rect, Scene, Scheme, Shape,
    Spacing, TextAlign, TextRole, TextStyle, TextSystem,
};

use modules::{Caption, Module, Tile, TileSpec};

pub use client::{ControlCenter, role};
use media::{Command, Player};
pub use now_playing::clock;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-controlcenter.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const HUD_SIZE: (f32, f32) = (232.0, 64.0);
pub const PANEL_WIDTH: f32 = 344.0;
pub const EDGE: f32 = 8.0;
pub const HUD_DURATION: Duration = Duration::from_millis(1500);
pub const SEGMENTS: usize = 16;
const PADDING: f32 = Spacing::M as f32;
const GAP: f32 = Spacing::S as f32;
const SEGMENT_GAP: f32 = 2.0;
const SEGMENT_HEIGHT: f32 = 6.0;
const SEGMENT_RADIUS: f32 = 1.0;
const LEVEL_STEP: f64 = 1.0 / SEGMENTS as f64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub edge: DisplayEdge,
    pub item: Rect,
}

pub fn place_beside(anchor: Option<Anchor>, surface: (f32, f32), size: (f32, f32)) -> Rect {
    let (width, height) = surface;
    let (panel_width, panel_height) = size;
    let along_x =
        |right: f32| (right - panel_width).clamp(EDGE, (width - EDGE - panel_width).max(EDGE));
    let along_y =
        |bottom: f32| (bottom - panel_height).clamp(EDGE, (height - EDGE - panel_height).max(EDGE));
    let (x, y) = match anchor {
        None => (width - EDGE - panel_width, EDGE),
        Some(Anchor { edge, item }) => match edge {
            DisplayEdge::Left => (item.x + item.width + EDGE, along_y(item.y + item.height)),
            DisplayEdge::Right => (item.x - EDGE - panel_width, along_y(item.y + item.height)),
            DisplayEdge::Bottom => (along_x(item.x + item.width), item.y - EDGE - panel_height),
            _ => (along_x(item.x + item.width), item.y + item.height + EDGE),
        },
    };
    Rect::new(x.round(), y.round(), panel_width, panel_height)
}

pub fn slide_from(anchor: Option<Anchor>, distance: f32) -> Point {
    match anchor.map(|anchor| anchor.edge) {
        Some(DisplayEdge::Left) => Point::new(-distance, 0.0),
        Some(DisplayEdge::Right) => Point::new(distance, 0.0),
        Some(DisplayEdge::Bottom) => Point::new(0.0, distance),
        _ => Point::new(0.0, -distance),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub wifi: Option<bool>,
    pub network: Option<String>,
    pub sign_in: bool,
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
    pub person: Option<String>,
    pub icon_dirs: Vec<PathBuf>,
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
    Lock,
    Power,
    Settings,
}

pub type Changes = Rc<RefCell<Vec<Change>>>;

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
        Box::new(Caption::new(label)),
        Box::new(Slider::new(
            label,
            f64::from(value),
            (0.0, 1.0),
            LEVEL_STEP,
            move |value| changes.borrow_mut().push(change(value as f32)),
        )),
    ]
}

fn module(rows: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
    Box::new(Module::new(Box::new(Stack::new(
        Axis::Vertical,
        Spacing::S as f32,
        0.0,
        rows,
    ))))
}

fn on_off(on: bool) -> String {
    L10N.tr(if on { "on" } else { "off" })
}

fn tiles(status: &Status, changes: &Changes) -> Vec<Box<dyn Widget>> {
    let mut specs = Vec::new();
    if let Some(on) = status.wifi {
        specs.push(TileSpec {
            title: L10N.tr("wifi"),
            detail: match (&status.network, on) {
                (_, true) if status.sign_in => L10N.tr("sign-in-required"),
                (Some(network), true) => network.clone(),
                (None, true) => L10N.tr("not-connected"),
                (_, false) => on_off(false),
            },
            symbol: if on {
                "network-wireless-signal-excellent-symbolic"
            } else {
                "network-wireless-disabled-symbolic"
            },
            on,
            toggle: Change::Wifi,
            details: Some(Change::ChooseNetwork),
        });
    }
    if let Some(on) = status.bluetooth {
        specs.push(TileSpec {
            title: L10N.tr("bluetooth"),
            detail: on_off(on),
            symbol: if on {
                "bluetooth-active-symbolic"
            } else {
                "bluetooth-disabled-symbolic"
            },
            on,
            toggle: Change::Bluetooth,
            details: None,
        });
    }
    specs.push(TileSpec {
        title: L10N.tr("do-not-disturb"),
        detail: on_off(status.do_not_disturb),
        symbol: if status.do_not_disturb {
            "notifications-disabled-symbolic"
        } else {
            "notifications-symbolic"
        },
        on: status.do_not_disturb,
        toggle: Change::DoNotDisturb,
        details: None,
    });
    if let Some(muted) = status.input_muted {
        specs.push(TileSpec {
            title: L10N.tr("microphone"),
            detail: if muted {
                L10N.tr("muted")
            } else {
                on_off(true)
            },
            symbol: if muted {
                "microphone-disabled-symbolic"
            } else {
                "audio-input-microphone-symbolic"
            },
            on: !muted,
            toggle: |on| Change::InputMuted(!on),
            details: None,
        });
    }
    let width = ((PANEL_WIDTH - 2.0 * PADDING - GAP) / 2.0).floor();
    let mut tiles: Vec<Box<dyn Widget>> = specs
        .into_iter()
        .map(|spec| Box::new(Tile::new(spec, width, &status.icon_dirs, changes)) as Box<dyn Widget>)
        .collect();
    let mut rows = Vec::new();
    while !tiles.is_empty() {
        let pair: Vec<Box<dyn Widget>> = tiles.drain(..tiles.len().min(2)).collect();
        rows.push(Box::new(Stack::new(Axis::Horizontal, GAP, 0.0, pair)) as Box<dyn Widget>);
    }
    rows
}

pub fn panel(status: &Status, palette: Palette, text: TextSystem, changes: &Changes) -> (Ui, f32) {
    let mut controls: Vec<Box<dyn Widget>> = Vec::new();
    if let Some(name) = &status.person {
        controls.push(Box::new(person::PersonRow::new(
            name,
            &status.icon_dirs,
            changes,
        )));
    }
    controls.extend(tiles(status, changes));
    let mut brightness = Vec::new();
    if let Some(level) = status.brightness {
        brightness.extend(slider(
            &L10N.tr("display"),
            level,
            changes,
            Change::Brightness,
        ));
    }
    if let Some(level) = status.keyboard_brightness {
        brightness.extend(slider(
            &L10N.tr("keyboard-brightness"),
            level,
            changes,
            Change::KeyboardBrightness,
        ));
    }
    if !brightness.is_empty() {
        controls.push(module(brightness));
    }
    let mut sound = Vec::new();
    if let Some(volume) = status.volume {
        sound.extend(slider(&L10N.tr("sound"), volume, changes, Change::Volume));
        if status.outputs.len() > 1 {
            sound.push(pop_up(
                &L10N.tr("output"),
                &status.outputs,
                status.output.as_deref(),
                changes,
                Change::Output,
            ));
        }
    }
    if status.input_muted.is_some() {
        sound.push(Box::new(Caption::new(&L10N.tr("input-level"))));
        sound.push(Box::new(LevelIndicator::new(
            &L10N.tr("input-level"),
            Rc::clone(&status.input_level),
        )));
        if status.inputs.len() > 1 {
            sound.push(pop_up(
                &L10N.tr("input"),
                &status.inputs,
                status.input.as_deref(),
                changes,
                Change::Input,
            ));
        }
    }
    if !sound.is_empty() {
        controls.push(module(sound));
    }
    if let Some(player) = &status.player {
        let mut playing: Vec<Box<dyn Widget>> = vec![Box::new(now_playing::NowPlaying::new(
            player.clone(),
            status.artwork.clone(),
            changes,
        ))];
        if status.players.len() > 1 {
            playing.push(pop_up(
                &L10N.tr("player"),
                &status.players,
                Some(&player.bus_name),
                changes,
                Change::Player,
            ));
        }
        controls.push(module(playing));
    }
    let mut root = Stack::new(Axis::Vertical, GAP, PADDING, controls);
    let mut cx = Context::new(text, palette);
    cx.keyboard_navigation = true;
    let height = root
        .measure(&mut cx, Size::new(PANEL_WIDTH, f32::INFINITY))
        .height;
    (Ui::new(Box::new(root), cx), height.ceil())
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

pub fn stepped(level: f32, up: bool, fine: bool) -> f32 {
    let key = if up { VolumeKey::Up } else { VolumeKey::Down };
    Level {
        volume: level,
        muted: false,
    }
    .press(key, fine)
    .volume
}

pub fn card(palette: &Palette, rect: Rect, items: &mut Vec<Item>) {
    let shape = Shape::rounded(rect, Radius::XL);
    items.extend(Item::shadows(
        shape,
        Elevation::Overlay,
        palette.scheme,
        true,
    ));
    items.push(Item::Fill {
        shape,
        paint: palette.chassis.into(),
    });
    if palette.scheme == Scheme::Dark {
        items.push(Item::Stroke {
            shape: Shape::rounded(rect.inset(0.5), Radius::XL - 0.5),
            width: 1.0,
            paint: palette.hairline.into(),
        });
    }
}

pub fn panel_items(ui: &mut Ui, palette: &Palette, rect: Rect) -> Vec<Item> {
    let mut items = Vec::new();
    card(palette, rect, &mut items);
    let content = ui.scene(rect.width as u32, rect.height as u32);
    items.push(Item::Translate {
        offset: Point::new(rect.x, rect.y),
        items: content.items,
    });
    items
}

pub fn hud(hud: Hud, palette: &Palette, text: &mut TextSystem) -> Vec<Item> {
    let (width, height) = HUD_SIZE;
    let inset = Spacing::L as f32;
    let title = text.paragraph(
        &hud.title(),
        TextStyle::role(TextRole::BodyStrong, palette.ink),
        None,
        TextAlign::Start,
    );
    let row = width - 2.0 * inset;
    let segment = (row - SEGMENT_GAP * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
    let top = height - inset - SEGMENT_HEIGHT;
    let mut items = Vec::new();
    card(palette, Rect::new(0.0, 0.0, width, height), &mut items);
    items.push(Item::Text {
        origin: Point::new(inset, Spacing::M as f32),
        paragraph: title,
    });
    for (index, fill) in hud.segment_fills().into_iter().enumerate() {
        let rect = Rect::new(
            inset + index as f32 * (segment + SEGMENT_GAP),
            top,
            segment,
            SEGMENT_HEIGHT,
        );
        let shape = Shape::rounded(rect, SEGMENT_RADIUS);
        items.push(Item::Fill {
            shape,
            paint: palette.well.into(),
        });
        if fill > 0.0 {
            items.push(Item::Clip {
                shape: Shape::rect(Rect::new(rect.x, rect.y, rect.width * fill, rect.height)),
                items: vec![Item::Fill {
                    shape,
                    paint: palette.accent.into(),
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
