use std::rc::Rc;
use std::time::Instant;

use bande_audio::Direction;
use bande_bluetooth::Kind as DeviceKind;
use bande_config::{ACCENTS, MAX_INPUT_SOURCES};
use bande_keyboard::{Action, SHORTCUTS, shortcuts_in};
use bande_toolkit::ui::{
    Axis, Button, Label, LevelIndicator, PopUpButton, ScrollView, SearchField, SecureField,
    SegmentedControl, Shortcut, Slider, Stack, TextField, Toggle, Widget, WidgetId,
};
use bande_toolkit::{Scheme, Spacing, TextRole};

use crate::L10N;
use crate::arrangement::{Area, Arrangement, Screen};
use crate::follow::Command;
use crate::forget::Forget;
use crate::locale::LocaleCommand;
use crate::pictures::{Pair, PictureCommand, expand};
use crate::privilege::Privilege;
use crate::state::{Answer, Display, Question};
use crate::view::{
    Anchor, Intent, Intents, MIRROR_PREFIX, Picker, SettingsView, USE_EXTENDED, USE_MAIN, intent,
};
use crate::widgets::{
    Caption, Captioned, Card, Check, Column, Fade, Row, SIDEBAR_SYMBOL, Signal, Swatch, Swatches,
    Text, Thumbnail, Thumbnails, Tone,
};
use bande_config::Placement;
use bande_settings_panes::SettingHit;
use bande_settings_panes::{
    AUTOMATIC, Choice, Kind, List, MAX_SEGMENTS, PaneId, Section, Setting, SettingId, Value,
};

const PAGE_PADDING: f32 = Spacing::XL as f32;
const PAGE_SPACING: f32 = Spacing::S as f32;
const ROW_INSET: f32 = Spacing::M as f32;
const VOLUME_STEP: f64 = 1.0 / 16.0;
const FIELD_SIZE: f32 = 13.0;

pub(crate) type Widgets = Vec<Box<dyn Widget>>;

const KEEP_ANCHOR: &str = "keep-display-settings";

pub(crate) fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

pub(crate) fn submit(intents: &Intents, make: fn(String) -> Intent) -> impl FnMut(&str) + 'static {
    let intents = Rc::clone(intents);
    move |text: &str| intents.borrow_mut().push(make(text.to_owned()))
}

pub(crate) fn muted_row(text: &str) -> Box<dyn Widget> {
    boxed(Row::new(Text::new(text, Tone::Muted), None))
}

fn picture_name(name: &str) -> String {
    let mut characters = name.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

fn display_value(
    display: &Display,
    output: Option<bande_config::DisplaySettings>,
    id: SettingId,
) -> Option<Value> {
    let output = output.unwrap_or_default();
    Some(match id {
        SettingId::DisplayScale => match output.scale {
            Some(scale) => Value::Number((scale * 100.0).round() as u32),
            None => Value::Keyword(AUTOMATIC.to_owned()),
        },
        SettingId::RefreshRate => Value::Number(
            output
                .refresh_rate
                .map(|rate| rate.round() as u32)
                .or_else(|| display.current.map(|mode| mode.hertz()))?,
        ),
        SettingId::VariableRefreshRate => {
            Value::Flag(output.variable_refresh_rate.unwrap_or(false))
        }
        SettingId::DisplayResolution => {
            let (width, height) = output.resolution.or_else(|| {
                let mode = display.current?;
                Some((
                    u32::try_from(mode.width).ok()?,
                    u32::try_from(mode.height).ok()?,
                ))
            })?;
            Value::Keyword(format!("{width}x{height}"))
        }
        SettingId::DisplayRotation => Value::Number(
            output
                .rotation
                .map_or(display.rotation, |rotation| rotation.degrees()),
        ),
        _ => return None,
    })
}

fn choice_control(
    label: &str,
    labels: &[String],
    selected: Option<usize>,
    on_select: impl FnMut(usize) + 'static,
) -> Box<dyn Widget> {
    if labels.len() > MAX_SEGMENTS {
        boxed(PopUpButton::new(
            label,
            labels,
            selected.unwrap_or(0),
            on_select,
        ))
    } else {
        boxed(SegmentedControl::new(label, labels, selected, on_select))
    }
}

impl SettingsView {
    fn offered(&self, id: SettingId) -> Option<Vec<String>> {
        match id {
            SettingId::PowerMode => self
                .battery
                .as_ref()
                .map(|battery| battery.profiles.clone()),
            _ => None,
        }
    }

    fn setting_intent(
        &self,
        id: SettingId,
        scope: Option<&Display>,
    ) -> impl FnMut(Value) + 'static {
        let identity = scope.map(|display| display.identity.clone());
        intent(&self.intents, move |value| match &identity {
            Some(identity) => Intent::Display(identity.clone(), id, value),
            None => Intent::Set(id, value),
        })
    }

    fn control(
        &mut self,
        setting: &Setting,
        scope: Option<&Display>,
    ) -> Option<(Box<dyn Widget>, WidgetId)> {
        let id = setting.id;
        let value = match scope {
            Some(display) => display_value(
                display,
                self.display_settings.get(&display.identity).copied(),
                id,
            ),
            None => self.values.get(&id).cloned(),
        };
        let mut set = self.setting_intent(id, scope);
        let control: Box<dyn Widget> = match &setting.kind {
            Kind::Choices(choices) => {
                let offered = self.offered(id);
                let choices: Vec<&Choice> = choices
                    .iter()
                    .filter(|choice| match (&offered, &choice.value) {
                        (Some(offered), Value::Keyword(keyword)) => offered.contains(keyword),
                        _ => true,
                    })
                    .collect();
                let selected = value
                    .as_ref()
                    .and_then(|value| choices.iter().position(|choice| choice.value == *value));
                let labels: Vec<String> =
                    choices.iter().map(|choice| choice.label.clone()).collect();
                let values: Vec<Value> =
                    choices.iter().map(|choice| choice.value.clone()).collect();
                choice_control(&setting.label, &labels, selected, move |index| {
                    if let Some(value) = values.get(index) {
                        set(value.clone());
                    }
                })
            }
            Kind::Swatches(choices) => {
                let selected = value
                    .as_ref()
                    .and_then(|value| setting.kind.position(value));
                let values: Vec<Value> =
                    choices.iter().map(|choice| choice.value.clone()).collect();
                let swatches = choices
                    .iter()
                    .filter_map(|choice| {
                        let Value::Keyword(keyword) = &choice.value else {
                            return None;
                        };
                        let accent = ACCENTS
                            .iter()
                            .find(|(candidate, _)| candidate == keyword)
                            .map(|(_, accent)| *accent)?;
                        Some(Swatch {
                            label: choice.label.clone(),
                            light: accent.color(Scheme::Light),
                            dark: accent.fill(Scheme::Dark),
                        })
                    })
                    .collect();
                boxed(Swatches::new(
                    &setting.label,
                    swatches,
                    selected,
                    move |index| {
                        if let Some(value) = values.get(index) {
                            set(value.clone());
                        }
                    },
                ))
            }
            Kind::Menu(choices) => {
                let labels: Vec<String> =
                    choices.iter().map(|choice| choice.label.clone()).collect();
                let values: Vec<Value> =
                    choices.iter().map(|choice| choice.value.clone()).collect();
                let selected = value
                    .as_ref()
                    .and_then(|value| setting.kind.position(value))
                    .unwrap_or(0);
                boxed(PopUpButton::new(
                    &setting.label,
                    &labels,
                    selected,
                    move |index| {
                        if let Some(value) = values.get(index) {
                            set(value.clone());
                        }
                    },
                ))
            }
            Kind::Stops { stops, low, high } => {
                let selected = value
                    .as_ref()
                    .and_then(|value| setting.kind.position(value));
                let values: Vec<Value> = stops.iter().map(|stop| stop.value.clone()).collect();
                let slider = Slider::new(
                    &setting.label,
                    selected.unwrap_or(0) as f64,
                    (0.0, values.len().saturating_sub(1) as f64),
                    1.0,
                    move |index| {
                        if let Some(value) = values.get(index.round() as usize) {
                            set(value.clone());
                        }
                    },
                );
                let focus = slider.id();
                return Some((boxed(Captioned::new(boxed(slider), low, high)), focus));
            }
            Kind::Switch => {
                let Some(Value::Flag(on)) = value else {
                    return None;
                };
                boxed(Toggle::new(&setting.label, on, move |on| {
                    set(Value::Flag(on))
                }))
            }
            Kind::Range {
                range,
                step,
                low,
                high,
            } => {
                let Some(Value::Number(number)) = value else {
                    return None;
                };
                let slider = Slider::new(
                    &setting.label,
                    f64::from(number),
                    (f64::from(range.0), f64::from(range.1)),
                    f64::from(*step),
                    move |number| set(Value::Number(number.round() as u32)),
                );
                let focus = slider.id();
                return Some((boxed(Captioned::new(boxed(slider), low, high)), focus));
            }
            Kind::Level => {
                let Some(Value::Fraction(level)) = value else {
                    return None;
                };
                boxed(Slider::new(
                    &setting.label,
                    f64::from(level),
                    (0.0, 1.0),
                    VOLUME_STEP,
                    move |level| set(Value::Fraction(level as f32)),
                ))
            }
            Kind::RefreshRates => {
                let rates = scope?.refresh_rates();
                if rates.len() < 2 {
                    return None;
                }
                let labels: Vec<String> = rates
                    .iter()
                    .map(|rate| L10N.tr_args("hertz", [("value", (*rate).into())]))
                    .collect();
                let selected = match value {
                    Some(Value::Number(rate)) => {
                        rates.iter().position(|candidate| *candidate == rate)
                    }
                    _ => None,
                };
                choice_control(&setting.label, &labels, selected, move |index| {
                    if let Some(rate) = rates.get(index) {
                        set(Value::Number(*rate));
                    }
                })
            }
            Kind::Text => {
                let saved = match value {
                    Some(Value::Text(text)) => Some(text),
                    _ => None,
                };
                let text = self.drafts.get(&id).cloned().or(saved)?;
                let mut draft = intent(&self.intents, move |text: String| Intent::Draft(id, text));
                boxed(TextField::new(&setting.label, &text, move |text: &str| {
                    draft(text.to_owned())
                }))
            }
            Kind::Resolutions => {
                let sizes = scope?.resolutions();
                if sizes.len() < 2 {
                    return None;
                }
                let labels: Vec<String> = sizes
                    .iter()
                    .map(|(width, height)| {
                        L10N.tr_args(
                            "resolution",
                            [("width", (*width).into()), ("height", (*height).into())],
                        )
                    })
                    .collect();
                let selected = match value {
                    Some(Value::Keyword(size)) => sizes
                        .iter()
                        .position(|(width, height)| size == format!("{width}x{height}")),
                    _ => None,
                };
                choice_control(&setting.label, &labels, selected, move |index| {
                    if let Some((width, height)) = sizes.get(index) {
                        set(Value::Keyword(format!("{width}x{height}")));
                    }
                })
            }
            Kind::DisplayUses => {
                let display = scope?;
                let others: Vec<&Display> = self
                    .displays
                    .iter()
                    .filter(|other| other.identity != display.identity)
                    .collect();
                if others.is_empty() {
                    return None;
                }
                let mut choices = vec![
                    (L10N.tr("use-main"), USE_MAIN.to_owned()),
                    (L10N.tr("use-extended"), USE_EXTENDED.to_owned()),
                ];
                for other in &others {
                    let label = if others.len() == 1 {
                        L10N.tr("use-mirror")
                    } else {
                        L10N.tr_args("use-mirror-of", [("display", other.name().into())])
                    };
                    choices.push((label, format!("{MIRROR_PREFIX}{}", other.identity)));
                }
                let placement = self
                    .placements()
                    .into_iter()
                    .find(|placement| placement.display == display.identity);
                let current = match placement {
                    Some(Placement {
                        mirror: Some(mirror),
                        ..
                    }) => format!("{MIRROR_PREFIX}{mirror}"),
                    Some(Placement { main: true, .. }) => USE_MAIN.to_owned(),
                    _ => USE_EXTENDED.to_owned(),
                };
                let selected = choices.iter().position(|(_, value)| *value == current);
                let labels: Vec<String> = choices.iter().map(|(label, _)| label.clone()).collect();
                choice_control(&setting.label, &labels, selected, move |index| {
                    if let Some((_, value)) = choices.get(index) {
                        set(Value::Keyword(value.clone()));
                    }
                })
            }
            Kind::List(List::InputLevel) => {
                self.audio.as_ref()?.default_input()?;
                boxed(LevelIndicator::new(
                    &setting.label,
                    Rc::clone(&self.input_level),
                ))
            }
            Kind::List(_) | Kind::Pictures | Kind::Page(_) => return None,
        };
        let focus = control.id();
        Some((control, focus))
    }

    fn setting_row(
        &mut self,
        setting: &Setting,
        scope: Option<&Display>,
        matched: Option<&SettingHit>,
    ) -> Option<Box<dyn Widget>> {
        if setting.id == SettingId::VariableRefreshRate && !scope?.variable_refresh_capable() {
            return None;
        }
        let (control, focus) = match self.allowed(setting.id) {
            true => self.control(setting, scope)?,
            false => {
                let shown = match self.values.get(&setting.id)? {
                    Value::Flag(true) => L10N.tr("on"),
                    Value::Flag(false) => L10N.tr("off"),
                    Value::Text(text) => text.clone(),
                    _ => return None,
                };
                let label = Label::new(&shown);
                let focus = label.id();
                (boxed(label), focus)
            }
        };
        let connector = scope.map(|display| display.connector.clone());
        self.anchors
            .push((Anchor::Setting(setting.id, connector), focus));
        let label = Text::new(&setting.label, Tone::Primary)
            .highlighted(matched.map(|hit| hit.label.clone()).unwrap_or_default());
        let mut row = Row::new(label, Some(control)).highlighted(matched.is_some());
        if let Some(description) = &setting.description {
            row = row.detail(Text::new(description, Tone::Muted).caption());
        }
        Some(boxed(row))
    }

    fn page_row(
        &mut self,
        setting: &Setting,
        child: PaneId,
        matched: Option<&SettingHit>,
    ) -> Box<dyn Widget> {
        let (summary, revealed) = match setting.id {
            SettingId::TextSizeLink => (self.text_size_label(), Some(SettingId::TextSize)),
            SettingId::ScreenLock => (self.pane_summary(child), Some(SettingId::LockAfter)),
            _ => (self.pane_summary(child), None),
        };
        let value = summary.map_or_else(|| "›".to_owned(), |summary| format!("{summary}  ›"));
        let mut open = intent(&self.intents, move |child| match revealed {
            Some(setting) => Intent::Open(child, setting),
            None => Intent::SelectPane(child),
        });
        let label = Text::new(&setting.label, Tone::Primary)
            .highlighted(matched.map(|hit| hit.label.clone()).unwrap_or_default());
        let row = Row::new(label, Some(boxed(Label::new(&value))))
            .highlighted(matched.is_some() || self.hit(child).is_some())
            .on_activate(move || open(child));
        self.item_row(child.keyword().to_owned(), row)
    }

    fn setting_rows(&mut self, setting: &Setting, matched: Option<&SettingHit>) -> Widgets {
        let mut rows = match setting.kind {
            Kind::List(List::Shortcuts) => self.shortcut_cards(),
            Kind::List(list) if list != List::InputLevel => self.list_rows(list),
            Kind::Pictures => self.picture_rows(setting),
            Kind::Page(child) if self.available(child) => {
                vec![self.page_row(setting, child, matched)]
            }
            Kind::Page(_) => Vec::new(),
            Kind::Text
                if matches!(setting.id, SettingId::NightFrom | SettingId::NightTo)
                    && !self.night_custom() =>
            {
                Vec::new()
            }
            _ if setting.id == SettingId::TrackpadDragging
                && self.values.get(&SettingId::TrackpadTapToClick) != Some(&Value::Flag(true)) =>
            {
                Vec::new()
            }
            _ => self
                .setting_row(setting, None, matched)
                .into_iter()
                .collect(),
        };
        if rows.is_empty() {
            return rows;
        }
        let note = match self.errors.get(&setting.id) {
            Some(error) => Some(Text::new(error, Tone::Danger)),
            None if !self.allowed(setting.id) => {
                Some(Text::new(&L10N.tr("administrator-only"), Tone::Muted))
            }
            None => None,
        };
        rows.extend(
            note.map(|note| boxed(Caption::new(note.role(TextRole::Callout)).inset(ROW_INSET))),
        );
        rows
    }

    pub(crate) fn item_row(&mut self, anchor: String, row: Row) -> Box<dyn Widget> {
        self.anchors.push((Anchor::Item(anchor), row.id()));
        boxed(row)
    }

    fn network_rows(&mut self) -> Widgets {
        let Some(wifi) = self.wifi.clone() else {
            return Vec::new();
        };
        if wifi.networks.is_empty() {
            return vec![muted_row(&L10N.tr("searching-networks"))];
        }
        let mut rows = Vec::new();
        for network in wifi.networks {
            let name = network.name();
            if self.forgetting.as_ref() == Some(&Forget::Network(network.ssid.clone())) {
                let row = self.forget_row(&name, &L10N.tr("forget-network-warning"));
                rows.push(row);
                continue;
            }
            let status = if network.current {
                Some(L10N.tr("connected"))
            } else if self.join.joining.as_ref() == Some(&network.ssid) {
                Some(L10N.tr("joining"))
            } else if network.known {
                Some(L10N.tr("saved"))
            } else {
                None
            };
            let signal = boxed(Signal::new(
                network.strength,
                network.security.needs_password(),
            ));
            let control = if network.known {
                boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![
                        self.forget_button(Forget::Network(network.ssid.clone())),
                        signal,
                    ],
                ))
            } else {
                signal
            };
            let mut row = Row::new(Text::new(&name, Tone::Primary), Some(control));
            if let Some(status) = status {
                row = row.detail(Text::new(&status, Tone::Muted).caption());
            }
            if !network.current {
                let mut join = intent(&self.intents, Intent::Join);
                let ssid = network.ssid.clone();
                row = row.on_activate(move || join(ssid.clone()));
            }
            rows.push(self.item_row(name.clone(), row));
            if self.join.asking.as_ref() == Some(&network.ssid) {
                let field = SecureField::new(
                    &L10N.tr("password"),
                    FIELD_SIZE,
                    submit(&self.intents, Intent::Password),
                );
                self.anchors.push((Anchor::Password, field.id()));
                let detail = self
                    .join
                    .error
                    .clone()
                    .unwrap_or_else(|| L10N.tr("password-hint"));
                rows.push(boxed(
                    Row::new(
                        Text::new(
                            &L10N.tr_args("password-for", [("network", name.into())]),
                            Tone::Primary,
                        ),
                        Some(boxed(field)),
                    )
                    .detail(Text::new(&detail, Tone::Muted).caption()),
                ));
            }
        }
        rows
    }

    fn device_rows(&mut self, paired: bool) -> Widgets {
        let Some(bluetooth) = self.bluetooth.clone() else {
            return Vec::new();
        };
        let devices = if paired {
            bluetooth.paired
        } else {
            bluetooth.nearby
        };
        if devices.is_empty() {
            return vec![muted_row(&if paired {
                L10N.tr("no-devices")
            } else {
                L10N.tr("searching-devices")
            })];
        }
        devices
            .into_iter()
            .map(|device| {
                let name = match device.named {
                    true => device.name.clone(),
                    false => kind_noun(device.kind),
                };
                if self.forgetting.as_ref() == Some(&Forget::Device(device.address.clone())) {
                    return self.forget_row(&name, &L10N.tr("forget-device-warning"));
                }
                let busy = self.busy.contains(&device.address);
                let (status, action, make): (String, String, fn(String) -> Intent) =
                    match (paired, device.connected) {
                        (true, true) => (
                            L10N.tr("connected"),
                            L10N.tr("disconnect"),
                            Intent::Disconnect,
                        ),
                        (true, false) => (
                            L10N.tr("not-connected"),
                            L10N.tr("connect"),
                            Intent::Connect,
                        ),
                        (false, _) => (String::new(), L10N.tr("pair"), Intent::Pair),
                    };
                let status = if busy {
                    if paired {
                        L10N.tr("connecting")
                    } else {
                        L10N.tr("pairing")
                    }
                } else {
                    status
                };
                let status = match device.battery {
                    Some(level) if paired && !busy => L10N.tr_args(
                        "status-with-battery",
                        [("status", status.into()), ("level", level.into())],
                    ),
                    _ => status,
                };
                let status = match device.named {
                    true => status,
                    false => L10N.tr_args(
                        "unnamed-device-status",
                        [
                            ("address", device.address.clone().into()),
                            ("status", status.into()),
                        ],
                    ),
                };
                let control = (!busy).then(|| {
                    let mut act = intent(&self.intents, make);
                    let address = device.address.clone();
                    let button = boxed(Button::new(&action, move || act(address.clone())));
                    if paired {
                        boxed(Stack::new(
                            Axis::Horizontal,
                            Spacing::S as f32,
                            0.0,
                            vec![
                                button,
                                self.forget_button(Forget::Device(device.address.clone())),
                            ],
                        ))
                    } else {
                        button
                    }
                });
                let ink = self.palette().ink2;
                let symbol = self.symbol(device.kind.symbol(), SIDEBAR_SYMBOL, ink);
                let mut row = Row::new(Text::new(&name, Tone::Primary), control).symbol(symbol);
                if !status.is_empty() {
                    row = row.detail(Text::new(&status, Tone::Muted).caption());
                }
                self.item_row(device.address, row)
            })
            .collect()
    }

    fn audio_rows(&mut self, direction: Direction) -> Widgets {
        let Some(audio) = self.audio.clone() else {
            return Vec::new();
        };
        let (devices, default) = match direction {
            Direction::Output => (audio.outputs, audio.default_output),
            Direction::Input => (audio.inputs, audio.default_input),
        };
        if devices.is_empty() {
            return vec![muted_row(&L10N.tr("no-sound-devices"))];
        }
        devices
            .into_iter()
            .map(|device| {
                let chosen = default.as_ref() == Some(&device.name);
                let mut row = Row::new(
                    Text::new(&device.description, Tone::Primary),
                    chosen.then(|| boxed(Check::new())),
                );
                if !chosen {
                    let mut choose =
                        intent(&self.intents, move |name| Intent::Default(direction, name));
                    let name = device.name.clone();
                    row = row.on_activate(move || choose(name.clone()));
                }
                self.item_row(device.name, row)
            })
            .collect()
    }

    fn app_rows(&mut self) -> Widgets {
        let Some(audio) = self.audio.clone() else {
            return Vec::new();
        };
        if audio.apps.is_empty() {
            return vec![muted_row(&L10N.tr("no-apps-playing"))];
        }
        audio
            .apps
            .into_iter()
            .map(|app| {
                let name = app.name.clone();
                let mut change = intent(&self.intents, move |volume: f32| {
                    Intent::AppVolume(name.clone(), volume)
                });
                let slider = Slider::new(
                    &L10N.tr_args("app-volume", [("app", app.name.clone().into())]),
                    f64::from(if app.level.muted {
                        0.0
                    } else {
                        app.level.volume
                    }),
                    (0.0, 1.0),
                    VOLUME_STEP,
                    move |volume| change(volume as f32),
                );
                let row = Row::new(Text::new(&app.name, Tone::Primary), Some(boxed(slider)));
                self.item_row(format!("app:{}", app.name), row)
            })
            .collect()
    }

    fn list_rows(&mut self, list: List) -> Widgets {
        match list {
            List::Networks => self.network_rows(),
            List::PairedDevices => self.device_rows(true),
            List::NearbyDevices => self.device_rows(false),
            List::OutputDevices => self.audio_rows(Direction::Output),
            List::InputDevices => self.audio_rows(Direction::Input),
            List::AppVolumes => self.app_rows(),
            List::InputLevel => Vec::new(),
            List::InputSources => self.input_source_rows(),
            List::Shortcuts => Vec::new(),
            List::About => self.about_rows(),
            List::Storage => self.storage_rows(),
            List::LargeFiles => self.large_file_rows(),
            List::Clock => self.clock_rows(),
            List::TimeZones => self.zone_rows(),
            List::LoginItems => self.login_rows(),
            List::BatteryHistory => self.battery_rows(),
            List::NotificationApps => self.notification_app_rows(),
            List::SoftwareUpdates => self.software_rows(),
            List::Ethernet => self.ethernet_rows(),
            List::Vpns => self.vpn_rows(),
            List::Languages => self.language_rows(),
            List::Regions => self.region_rows(),
            List::DefaultApps => self.default_app_rows(),
            List::Clipboard => self.clipboard_rows(),
            List::SearchExcluded => self.excluded_rows(),
            List::Permissions(permission) => self.permission_rows(permission),
        }
    }

    fn source_buttons(&self, index: usize, count: usize) -> Box<dyn Widget> {
        let mut buttons: Widgets = Vec::new();
        for (shown, title, tooltip, make) in [
            (
                index > 0,
                L10N.tr("move-up"),
                L10N.tr("move-up-tooltip"),
                Intent::MoveSource(index, true),
            ),
            (
                index + 1 < count,
                L10N.tr("move-down"),
                L10N.tr("move-down-tooltip"),
                Intent::MoveSource(index, false),
            ),
            (
                count > 1,
                L10N.tr("remove"),
                L10N.tr("remove-tooltip"),
                Intent::RemoveSource(index),
            ),
        ] {
            if shown {
                let mut act = intent(&self.intents, move |()| make.clone());
                buttons.push(boxed(
                    Button::new(&title, move || act(())).tooltip(&tooltip),
                ));
            }
        }
        boxed(Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            buttons,
        ))
    }

    fn input_source_rows(&mut self) -> Widgets {
        let sources = self.input_sources();
        let count = sources.len();
        let mut rows = Vec::new();
        if let Some(next) = SHORTCUTS
            .iter()
            .find(|shortcut| shortcut.action == Action::NextInputSource)
            .filter(|_| count > 1)
        {
            rows.push(boxed(Row::new(
                Text::new(&L10N.tr("switch-sources"), Tone::Primary),
                Some(boxed(Shortcut::new(next.keys))),
            )));
        }
        for (index, code) in sources.iter().enumerate() {
            let name = self.layouts().name(code);
            let short = self.layouts().short(code);
            let buttons = self.source_buttons(index, count);
            let row = Row::new(Text::new(&name, Tone::Primary), Some(buttons))
                .detail(Text::new(&short, Tone::Muted).caption());
            rows.push(self.item_row(code.clone(), row));
        }
        if let Some(login) = self
            .locale
            .as_ref()
            .map(|locale| locale.login_sources.clone())
        {
            rows.push(self.login_layouts_row(&sources, &login));
        }
        let picking = self
            .picking
            .clone()
            .filter(|(picker, _)| *picker == Picker::InputSource);
        match picking {
            Some((_, query)) => rows.extend(self.source_search_rows(&query)),
            None if count < MAX_INPUT_SOURCES => {
                let mut add = intent(&self.intents, |()| Intent::Pick(Some(Picker::InputSource)));
                let button = Button::new(&L10N.tr("add-input-source"), move || add(()));
                self.anchors
                    .push((Anchor::Setting(SettingId::InputSources, None), button.id()));
                rows.push(boxed(Row::new(
                    Text::new(&L10N.tr("more-input-sources"), Tone::Muted).caption(),
                    Some(boxed(button)),
                )));
            }
            None => rows.push(muted_row(
                &L10N.tr_args("input-sources-full", [("count", MAX_INPUT_SOURCES.into())]),
            )),
        }
        rows
    }

    fn login_layouts_row(&mut self, sources: &[String], login: &[String]) -> Box<dyn Widget> {
        if sources == login {
            return muted_row(&L10N.tr("login-layouts-same"));
        }
        let button = (!self.refused.contains(&Privilege::Keyboard)).then(|| {
            let codes = sources.to_vec();
            let mut send = intent(&self.intents, Intent::Locale);
            boxed(Button::new(&L10N.tr("use-at-login"), move || {
                send(LocaleCommand::LoginKeyboard(codes.clone()))
            }))
        });
        boxed(Row::new(
            Text::new(&L10N.tr("login-layouts-differ"), Tone::Muted).caption(),
            button,
        ))
    }

    pub(crate) fn source_matches(&mut self, query: &str) -> Vec<String> {
        let added = self.input_sources();
        let territory = self
            .locale
            .as_ref()
            .and_then(|locale| locale.territory().map(str::to_owned));
        let layouts = self.layouts();
        let found = if query.trim().is_empty() {
            territory.map_or_else(Vec::new, |territory| layouts.suggested(&territory))
        } else {
            layouts.search(query)
        };
        found
            .into_iter()
            .filter(|layout| !added.contains(&layout.code))
            .map(|layout| layout.code.clone())
            .collect()
    }

    fn source_search_rows(&mut self, query: &str) -> Widgets {
        let field = SearchField::new(
            &L10N.tr("search-layouts"),
            query,
            submit(&self.intents, Intent::PickQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::Pick(None));
        let mut rows = vec![boxed(Row::new(
            Text::new(&L10N.tr("add-input-source"), Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                ],
            ))),
        ))];
        let typed = !query.trim().is_empty();
        let matches = self.pick_results();
        if typed && matches.is_empty() {
            rows.push(muted_row(&L10N.tr("no-layouts")));
        }
        for code in matches {
            let name = self.layouts().name(&code);
            let mut add = intent(&self.intents, Intent::Picked);
            let chosen = code.clone();
            let row = Row::new(Text::new(&name, Tone::Primary), None)
                .detail(Text::new(&code, Tone::Muted).caption())
                .on_activate(move || add(chosen.clone()));
            rows.push(self.item_row(format!("add:{code}"), row));
        }
        if !typed {
            rows.push(muted_row(&L10N.tr("layout-hint")));
        }
        rows
    }

    fn shortcut_cards(&mut self) -> Widgets {
        let mut page = Vec::new();
        for area in bande_keyboard::Area::ALL {
            page.push(boxed(Caption::new(Text::label(&area.title()))));
            let mut bindings: Vec<(Action, Vec<&str>)> = Vec::new();
            for shortcut in shortcuts_in(area).filter(|shortcut| {
                shortcut.action != Action::ToggleScreenReader || self.offers_screen_reader()
            }) {
                match bindings
                    .iter_mut()
                    .find(|(action, _)| *action == shortcut.action)
                {
                    Some((_, keys)) => keys.push(shortcut.keys),
                    None => bindings.push((shortcut.action, vec![shortcut.keys])),
                }
            }
            let rows = bindings
                .into_iter()
                .map(|(action, keys)| {
                    let keys = keys
                        .into_iter()
                        .map(|keys| boxed(Shortcut::new(keys)))
                        .collect();
                    boxed(Row::new(
                        Text::new(&action.label(), Tone::Primary),
                        Some(boxed(Stack::new(
                            Axis::Horizontal,
                            Spacing::M as f32,
                            0.0,
                            keys,
                        ))),
                    ))
                })
                .collect();
            page.push(boxed(Card::new(rows)));
        }
        page
    }

    fn picture_rows(&mut self, setting: &Setting) -> Widgets {
        let (light, dark) = match self.values.get(&SettingId::Wallpaper) {
            Some(Value::Pictures { light, dark }) => (light.clone(), dark.clone()),
            _ => (String::new(), String::new()),
        };
        let mut choices: Vec<(String, Pair)> = self
            .pictures
            .iter()
            .map(|(name, pair)| (picture_name(name), pair.clone()))
            .collect();
        let mut selected = if light.is_empty() {
            self.pictures
                .iter()
                .position(|(name, _)| name == bande_wallpaper::DEFAULT_WALLPAPER)
        } else {
            self.pictures
                .iter()
                .position(|(_, pair)| pair.light.to_string_lossy() == light)
        };
        let home = std::env::home_dir();
        let dark = if dark.is_empty() { &light } else { &dark };
        if selected.is_none()
            && let (Some(light), Some(dark)) = (
                expand(&light, home.as_deref()),
                expand(dark, home.as_deref()),
            )
        {
            let label = light
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            choices.insert(0, (label, Pair { light, dark }));
            selected = Some(0);
        }
        for (_, pair) in &choices {
            let bundled = self.pictures.iter().any(|(_, known)| known == pair);
            if !bundled && !self.thumbnails.contains_key(pair) {
                self.thumbnails.insert(pair.clone(), None);
                self.commands
                    .push(Command::Picture(PictureCommand::Thumbnail(pair.clone())));
            }
        }
        let tiles = choices
            .iter()
            .map(|(label, pair)| Thumbnail {
                label: label.clone(),
                image: self.thumbnails.get(pair).cloned().flatten(),
            })
            .collect();
        let pairs: Vec<Pair> = choices.into_iter().map(|(_, pair)| pair).collect();
        let mut set = self.setting_intent(SettingId::Wallpaper, None);
        let grid = Thumbnails::new(&setting.label, tiles, selected, move |index| {
            if let Some(pair) = pairs.get(index) {
                set(Value::Pictures {
                    light: pair.light.to_string_lossy().into_owned(),
                    dark: pair.dark.to_string_lossy().into_owned(),
                });
            }
        });
        self.anchors
            .push((Anchor::Setting(setting.id, None), grid.id()));
        let mut choose = intent(&self.intents, |()| Intent::ChoosePicture);
        let add = Button::new(&L10N.tr("add-picture"), move || choose(()));
        vec![
            boxed(grid),
            boxed(Row::new(
                Text::new(&L10N.tr("own-picture"), Tone::Primary),
                Some(boxed(add)),
            )),
        ]
    }

    fn section_shown(&self, section: &Section) -> bool {
        section.settings.iter().all(|setting| match setting.id {
            SettingId::Networks => self.wifi.as_ref().is_some_and(|wifi| wifi.enabled),
            SettingId::PairedDevices | SettingId::NearbyDevices => self
                .bluetooth
                .as_ref()
                .is_some_and(|bluetooth| bluetooth.powered),
            SettingId::AutomaticTime => self.time.as_ref().is_some_and(|time| time.can_automatic),
            SettingId::DateAndTime | SettingId::TimeZone => self.time.is_some(),
            SettingId::PowerMode | SettingId::LowPowerOnBattery => self
                .battery
                .as_ref()
                .is_some_and(|battery| !battery.profiles.is_empty()),
            SettingId::BatteryLevel | SettingId::LidOnPower => self.battery.is_some(),
            _ => true,
        })
    }

    fn pairing_card(&mut self) -> Option<Widgets> {
        let question = self.pairing.clone()?;
        let buttons = |intents: &Intents, decline: &str, accept: &str| -> Box<dyn Widget> {
            let button = |title: &str, answer: Answer| {
                let mut send = intent(intents, Intent::Answer);
                boxed(Button::new(title, move || send(answer.clone())))
            };
            boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    button(decline, Answer::Decline),
                    button(accept, Answer::Accept),
                ],
            ))
        };
        let (text, control) = match question {
            Question::Confirm { device, code } => (
                L10N.tr_args(
                    "confirm-code",
                    [("device", device.into()), ("code", code.into())],
                ),
                Some(buttons(&self.intents, &L10N.tr("cancel"), &L10N.tr("pair"))),
            ),
            Question::Authorize { device } => (
                L10N.tr_args("authorize-device", [("device", device.into())]),
                Some(buttons(&self.intents, &L10N.tr("deny"), &L10N.tr("allow"))),
            ),
            Question::Show { device, code } => (
                L10N.tr_args(
                    "type-code",
                    [("device", device.into()), ("code", code.into())],
                ),
                None,
            ),
            Question::Pin { device } => {
                let field = SecureField::new(
                    &L10N.tr("pin"),
                    FIELD_SIZE,
                    submit(&self.intents, |pin| Intent::Answer(Answer::Pin(pin))),
                );
                self.anchors.push((Anchor::Password, field.id()));
                (
                    L10N.tr_args("enter-pin", [("device", device.into())]),
                    Some(boxed(field)),
                )
            }
        };
        if let Some(control) = &control {
            let focus = control
                .children()
                .last()
                .map_or(control.id(), |button| button.id());
            if !self
                .anchors
                .iter()
                .any(|(anchor, _)| *anchor == Anchor::Password)
            {
                self.anchors.push((Anchor::Password, focus));
            }
        }
        Some(vec![
            boxed(Caption::new(Text::label(&L10N.tr("pairing-request")))),
            boxed(Card::new(vec![boxed(
                Row::new(Text::new(&text, Tone::Primary), control).highlighted(true),
            )])),
        ])
    }

    fn keep_card(&mut self) -> Option<Widgets> {
        self.keep.as_ref()?;
        let button = |intents: &Intents, title: &str, keep: bool| {
            let mut send = intent(intents, Intent::KeepDisplays);
            boxed(Button::new(title, move || send(keep)))
        };
        let buttons = boxed(Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![
                button(&self.intents, &L10N.tr("revert"), false),
                button(&self.intents, &L10N.tr("keep"), true),
            ],
        ));
        let focus = buttons
            .children()
            .last()
            .map_or(buttons.id(), |button| button.id());
        self.anchors
            .push((Anchor::Item(KEEP_ANCHOR.to_owned()), focus));
        let countdown = L10N.tr_args("reverting-in", [("seconds", self.seconds_left().into())]);
        Some(vec![boxed(Card::new(vec![
            boxed(
                Row::new(
                    Text::new(&L10N.tr("keep-display-settings"), Tone::Primary).semibold(),
                    Some(buttons),
                )
                .highlighted(true),
            ),
            boxed(Caption::new(Text::new(&countdown, Tone::Muted))),
        ]))])
    }

    fn arrangement_card(&mut self) -> Option<Widgets> {
        if self.displays.len() < 2 {
            return None;
        }
        let placements = self.placements();
        let shown: Vec<(Display, bool)> = self
            .displays
            .iter()
            .zip(&placements)
            .filter(|(_, placement)| placement.mirror.is_none())
            .map(|(display, placement)| (display.clone(), placement.main))
            .collect();
        let screens = shown
            .iter()
            .map(|(display, main)| {
                let (width, height) = display.logical_size();
                Screen {
                    label: display.name(),
                    area: Area {
                        x: display.position.0,
                        y: display.position.1,
                        width,
                        height,
                    },
                    main: *main,
                }
            })
            .collect();
        let identities: Vec<String> = shown
            .iter()
            .map(|(display, _)| display.identity.clone())
            .collect();
        let mut send = intent(&self.intents, |(identity, position)| {
            Intent::Arrange(identity, position)
        });
        let canvas = Arrangement::new(&L10N.tr("arrangement"), screens, move |index, position| {
            if let Some(identity) = identities.get(index) {
                send((identity.clone(), position));
            }
        });
        Some(vec![
            boxed(Caption::new(Text::label(&L10N.tr("arrangement")))),
            boxed(Card::new(vec![boxed(canvas)])),
            boxed(Caption::new(Text::new(
                &L10N.tr("arrangement-hint"),
                Tone::Muted,
            ))),
        ])
    }

    fn display_sections(
        &mut self,
        section: &Section,
        matched: &dyn Fn(SettingId) -> Option<SettingHit>,
    ) -> Widgets {
        let mut page = Vec::new();
        page.extend(self.keep_card().into_iter().flatten());
        page.extend(self.arrangement_card().into_iter().flatten());
        for display in self.displays.clone() {
            let title = display.name();
            page.push(boxed(Caption::new(Text::label(&title))));
            let rows: Widgets = section
                .settings
                .iter()
                .filter_map(|setting| {
                    self.setting_row(setting, Some(&display), matched(setting.id).as_ref())
                })
                .collect();
            page.push(boxed(Card::new(rows)));
        }
        page
    }

    pub(crate) fn content(&mut self) -> Box<dyn Widget> {
        let Some(pane) = self.panes.iter().find(|pane| pane.id == self.pane).cloned() else {
            return boxed(Column::new(0.0, 0.0, Vec::new()));
        };
        let hit = self.hit(pane.id).cloned();
        let matched = move |id: SettingId| hit.as_ref().and_then(|hit| hit.setting(id)).cloned();
        let mut page: Widgets = pane
            .id
            .parent()
            .and_then(|parent| self.back_button(parent))
            .into_iter()
            .collect();
        if !self.query.trim().is_empty() && self.hits.is_empty() {
            let notice = self.no_match();
            return self.page(vec![notice]);
        }
        if pane.id == PaneId::Notifications
            && let Some(app) = self.app_page()
        {
            return self.page(app);
        }
        page.push(self.header(&pane));
        if pane.id == PaneId::Bluetooth
            && let Some(card) = self.pairing_card()
        {
            page.extend(card);
        }
        let notice = self.notice(&pane);
        self.crossfade_on(pane.id, notice.is_some());
        let mut body: Widgets = notice.into_iter().collect();
        let sections: &[Section] = if body.is_empty() { &pane.sections } else { &[] };
        for section in sections {
            if section
                .settings
                .iter()
                .any(|setting| setting.id == SettingId::DisplayUse)
            {
                body.extend(self.display_sections(section, &matched));
                continue;
            }
            if !self.section_shown(section) {
                continue;
            }
            let mut rows: Widgets = Vec::new();
            let mut listed = None;
            for setting in &section.settings {
                if Some(setting.id) == pane.master {
                    continue;
                }
                if matches!(setting.kind, Kind::List(list) if list != List::InputLevel) {
                    listed = listed.or(matched(setting.id));
                }
                rows.extend(self.setting_rows(setting, matched(setting.id).as_ref()));
            }
            if rows.is_empty() {
                continue;
            }
            if let Some(title) = &section.title {
                let highlights = listed.map(|hit| hit.label).unwrap_or_default();
                body.push(boxed(Caption::new(
                    Text::label(title).highlighted(highlights),
                )));
            }
            body.push(boxed(Card::new(rows)));
            if let Some(note) = &section.note {
                body.push(boxed(Caption::new(Text::new(note, Tone::Muted))));
            }
            if section
                .settings
                .iter()
                .any(|setting| setting.id == SettingId::Networks)
                && self.join.asking.is_none()
                && let Some(error) = &self.join.error
            {
                body.push(boxed(Caption::new(Text::new(error, Tone::Muted))));
            }
        }
        if let Some(message) = &self.message {
            body.push(boxed(Caption::new(Text::new(message, Tone::Muted))));
        }
        page.push(boxed(Fade::new(
            boxed(Column::new(PAGE_SPACING, 0.0, body)),
            Rc::clone(&self.crossfade),
        )));
        self.page(page)
    }

    fn crossfade_on(&mut self, pane: PaneId, off: bool) {
        match self.shown.replace((pane, off)) {
            Some(shown) if shown == (pane, !off) => {
                self.crossfade.borrow_mut().start(Instant::now());
            }
            Some((shown, _)) if shown != pane => self.crossfade.borrow_mut().stop(),
            _ => {}
        }
    }

    fn page(&mut self, page: Widgets) -> Box<dyn Widget> {
        let view = ScrollView::shared(
            boxed(Column::new(PAGE_SPACING, PAGE_PADDING, page)),
            Rc::clone(&self.scroll),
        );
        self.page_view = Some(view.id());
        boxed(view)
    }
}

fn kind_noun(kind: DeviceKind) -> String {
    L10N.tr(match kind {
        DeviceKind::Mouse => "device-mouse",
        DeviceKind::Keyboard => "device-keyboard",
        DeviceKind::Tablet => "device-tablet",
        DeviceKind::GameController => "device-game-controller",
        DeviceKind::Headphones => "device-headphones",
        DeviceKind::Headset => "device-headset",
        DeviceKind::Speaker => "device-speaker",
        DeviceKind::Phone => "device-phone",
        DeviceKind::Computer => "device-computer",
        DeviceKind::Camera => "device-camera",
        DeviceKind::Printer => "device-printer",
        DeviceKind::Display => "device-display",
        DeviceKind::Other => "unknown-device",
    })
}
