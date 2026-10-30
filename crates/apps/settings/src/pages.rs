use std::rc::Rc;

use unbound_audio::Direction;
use unbound_toolkit::Spacing;
use unbound_toolkit::ui::{
    Axis, Button, Label, SecureField, Slider, Stack, Toggle, Widget, WidgetId,
};

use crate::L10N;
use crate::arrangement::{Area, Arrangement, Screen};
use crate::follow::Command;
use crate::pictures::{Pair, PictureCommand, expand};
use crate::state::{Answer, Display, Question};
use crate::view::{
    Anchor, Intent, Intents, MIRROR_PREFIX, SettingsView, USE_EXTENDED, USE_MAIN, intent,
};
use crate::widgets::{
    Caption, Captioned, Card, Check, Column, Row, Scroll, Segmented, SidebarRow, Signal, Text,
    Thumbnail, Thumbnails, Tone,
};
use unbound_config::Placement;
use unbound_settings_panes::SettingHit;
use unbound_settings_panes::{AUTOMATIC, Kind, List, PaneId, Section, Setting, SettingId, Value};

const PAGE_PADDING: f32 = Spacing::XL as f32;
const PAGE_SPACING: f32 = Spacing::S as f32;
const VOLUME_STEP: f64 = 1.0 / 16.0;
const FIELD_SIZE: f32 = 13.0;

type Widgets = Vec<Box<dyn Widget>>;

const KEEP_ANCHOR: &str = "keep-display-settings";

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn submit(intents: &Intents, make: fn(String) -> Intent) -> impl FnMut(&str) + 'static {
    let intents = Rc::clone(intents);
    move |text: &str| intents.borrow_mut().push(make(text.to_owned()))
}

fn muted_row(text: &str) -> Box<dyn Widget> {
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
    output: Option<unbound_config::DisplaySettings>,
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

impl SettingsView {
    pub(crate) fn sidebar(&mut self) -> Box<dyn Widget> {
        let rows = self
            .visible_panes()
            .into_iter()
            .filter_map(|id| {
                let pane = self.panes.iter().find(|pane| pane.id == id)?;
                let selected = id == self.pane;
                let tone = if selected {
                    Tone::OnAccent
                } else {
                    Tone::Primary
                };
                let highlights = self.hit(id).map(|hit| hit.name.clone()).unwrap_or_default();
                let mut select = intent(&self.intents, Intent::SelectPane);
                let row = SidebarRow::new(
                    Text::new(&pane.name, tone).highlighted(highlights),
                    selected,
                    move || select(id),
                    intent(&self.intents, Intent::StepPane),
                );
                self.anchors.push((Anchor::Pane(id), row.id()));
                Some(boxed(row))
            })
            .collect();
        boxed(Column::new(2.0, 0.0, rows))
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
                let selected = value
                    .as_ref()
                    .and_then(|value| setting.kind.position(value));
                let labels: Vec<String> =
                    choices.iter().map(|choice| choice.label.clone()).collect();
                let values: Vec<Value> =
                    choices.iter().map(|choice| choice.value.clone()).collect();
                boxed(Segmented::new(
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
                boxed(Segmented::new(
                    &setting.label,
                    &labels,
                    selected,
                    move |index| {
                        if let Some(rate) = rates.get(index) {
                            set(Value::Number(*rate));
                        }
                    },
                ))
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
                boxed(Segmented::new(
                    &setting.label,
                    &labels,
                    selected,
                    move |index| {
                        if let Some((width, height)) = sizes.get(index) {
                            set(Value::Keyword(format!("{width}x{height}")));
                        }
                    },
                ))
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
                boxed(Segmented::new(
                    &setting.label,
                    &labels,
                    selected,
                    move |index| {
                        if let Some((_, value)) = choices.get(index) {
                            set(Value::Keyword(value.clone()));
                        }
                    },
                ))
            }
            Kind::List(_) | Kind::Pictures => return None,
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
        let (control, focus) = self.control(setting, scope)?;
        let connector = scope.map(|display| display.connector.clone());
        self.anchors
            .push((Anchor::Setting(setting.id, connector), focus));
        let label = Text::new(&setting.label, Tone::Primary)
            .highlighted(matched.map(|hit| hit.label.clone()).unwrap_or_default());
        Some(boxed(
            Row::new(label, Some(control)).highlighted(matched.is_some()),
        ))
    }

    fn item_row(&mut self, anchor: String, row: Row) -> Box<dyn Widget> {
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
            let status = if network.current {
                Some(L10N.tr("connected"))
            } else if self.join.joining.as_ref() == Some(&network.ssid) {
                Some(L10N.tr("joining"))
            } else if network.known {
                Some(L10N.tr("saved"))
            } else {
                None
            };
            let mut row = Row::new(
                Text::new(&name, Tone::Primary),
                Some(boxed(Signal::new(
                    network.strength,
                    network.security.needs_password(),
                ))),
            );
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
                let control = (!busy).then(|| {
                    let mut act = intent(&self.intents, make);
                    let address = device.address.clone();
                    boxed(Button::new(&action, move || act(address.clone())))
                });
                let mut row = Row::new(Text::new(&device.name, Tone::Primary), control);
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

    fn list_rows(&mut self, list: List) -> Widgets {
        match list {
            List::Networks => self.network_rows(),
            List::PairedDevices => self.device_rows(true),
            List::NearbyDevices => self.device_rows(false),
            List::OutputDevices => self.audio_rows(Direction::Output),
            List::InputDevices => self.audio_rows(Direction::Input),
        }
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
                .position(|(name, _)| name == unbound_wallpaper::DEFAULT_WALLPAPER)
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
            boxed(Caption::new(
                Text::new(&L10N.tr("pairing-request"), Tone::Muted)
                    .caption()
                    .semibold(),
            )),
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
            boxed(Caption::new(
                Text::new(&L10N.tr("arrangement"), Tone::Muted)
                    .caption()
                    .semibold(),
            )),
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
            page.push(boxed(Caption::new(
                Text::new(&title, Tone::Muted).caption().semibold(),
            )));
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
        let mut page: Widgets = vec![boxed(Label::heading(&pane.name))];
        if pane.id == PaneId::Bluetooth
            && let Some(card) = self.pairing_card()
        {
            page.extend(card);
        }
        for section in &pane.sections {
            if pane.id == PaneId::Displays {
                page.extend(self.display_sections(section, &matched));
                continue;
            }
            if !self.section_shown(section) {
                continue;
            }
            let list = section
                .settings
                .iter()
                .find_map(|setting| match setting.kind {
                    Kind::List(list) => Some((list, setting.id)),
                    _ => None,
                });
            let pictures = section
                .settings
                .iter()
                .find(|setting| setting.kind == Kind::Pictures);
            let rows: Widgets = match (list, pictures) {
                (Some((list, _)), _) => self.list_rows(list),
                (None, Some(setting)) => self.picture_rows(setting),
                (None, None) => section
                    .settings
                    .iter()
                    .filter_map(|setting| {
                        self.setting_row(setting, None, matched(setting.id).as_ref())
                    })
                    .collect(),
            };
            if rows.is_empty() {
                continue;
            }
            if let Some(title) = &section.title {
                let highlights = list
                    .and_then(|(_, id)| matched(id))
                    .map(|hit| hit.label)
                    .unwrap_or_default();
                page.push(boxed(Caption::new(
                    Text::new(title, Tone::Muted)
                        .caption()
                        .semibold()
                        .highlighted(highlights),
                )));
            }
            page.push(boxed(Card::new(rows)));
            if list.is_some_and(|(list, _)| list == List::Networks)
                && self.join.asking.is_none()
                && let Some(error) = &self.join.error
            {
                page.push(boxed(Caption::new(Text::new(error, Tone::Muted))));
            }
        }
        if let Some(message) = &self.message {
            page.push(boxed(Caption::new(Text::new(message, Tone::Muted))));
        }
        boxed(Scroll::new(
            boxed(Column::new(PAGE_SPACING, PAGE_PADDING, page)),
            Rc::clone(&self.scroll),
        ))
    }
}
