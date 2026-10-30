use std::rc::Rc;

use unbound_audio::Direction;
use unbound_config::MAX_INPUT_SOURCES;
use unbound_keyboard::{Area, shortcuts_in};
use unbound_toolkit::Spacing;
use unbound_toolkit::ui::{
    Axis, Button, Label, PopUpButton, SearchField, SecureField, SegmentedControl, Slider, Stack,
    TextField, Toggle, Widget, WidgetId, shortcut_symbols,
};

use crate::L10N;
use crate::follow::Command;
use crate::pictures::{Pair, PictureCommand, expand};
use crate::state::{Answer, Display, Question};
use crate::view::{Anchor, Intent, Intents, SettingsView, intent};
use crate::widgets::{
    Caption, Captioned, Card, Check, Column, Row, Scroll, SidebarRow, Signal, Text, Thumbnail,
    Thumbnails, Tone,
};
use unbound_settings_panes::SettingHit;
use unbound_settings_panes::{AUTOMATIC, Kind, List, PaneId, Section, Setting, SettingId, Value};

const PAGE_PADDING: f32 = Spacing::XL as f32;
const PAGE_SPACING: f32 = Spacing::S as f32;
const VOLUME_STEP: f64 = 1.0 / 16.0;
const FIELD_SIZE: f32 = 13.0;
const SOURCE_RESULTS: usize = 8;

pub(crate) type Widgets = Vec<Box<dyn Widget>>;

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
    output: Option<unbound_config::Output>,
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
                let selected = id == self.pane || self.pane.parent() == Some(id);
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
        let connector = scope.map(|display| display.connector.clone());
        intent(&self.intents, move |value| match &connector {
            Some(connector) => Intent::Display(connector.clone(), id, value),
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
            Some(display) => {
                display_value(display, self.outputs.get(&display.connector).copied(), id)
            }
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
                boxed(SegmentedControl::new(
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
                boxed(SegmentedControl::new(
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
            List::InputSources => self.input_source_rows(),
            List::Shortcuts => Vec::new(),
            List::About => self.about_rows(),
            List::Storage => self.storage_rows(),
            List::LargeFiles => self.large_file_rows(),
            List::Clock => self.clock_rows(),
            List::TimeZones => self.zone_rows(),
            List::LoginItems => self.login_rows(),
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
        for (index, code) in sources.iter().enumerate() {
            let name = self.layouts().name(code);
            let short = self.layouts().short(code);
            let buttons = self.source_buttons(index, count);
            let row = Row::new(Text::new(&name, Tone::Primary), Some(buttons))
                .detail(Text::new(&short, Tone::Muted).caption());
            rows.push(self.item_row(code.clone(), row));
        }
        match self.adding_source.clone() {
            Some(query) => rows.extend(self.source_search_rows(&query, &sources)),
            None if count < MAX_INPUT_SOURCES => {
                let mut add = intent(&self.intents, |()| Intent::AddingSource(true));
                let button = Button::new(&L10N.tr("add-input-source"), move || add(()));
                self.anchors
                    .push((Anchor::Setting(SettingId::InputSources, None), button.id()));
                rows.push(boxed(Row::new(
                    Text::new(&L10N.tr("switch-sources-hint"), Tone::Muted).caption(),
                    Some(boxed(button)),
                )));
            }
            None => rows.push(muted_row(
                &L10N.tr_args("input-sources-full", [("count", MAX_INPUT_SOURCES.into())]),
            )),
        }
        rows
    }

    pub(crate) fn source_matches(
        &mut self,
        query: &str,
        added: &[String],
    ) -> Vec<(String, String)> {
        self.layouts()
            .search(query)
            .into_iter()
            .filter(|layout| !added.contains(&layout.code))
            .take(SOURCE_RESULTS)
            .map(|layout| (layout.code.clone(), layout.name.clone()))
            .collect()
    }

    fn source_search_rows(&mut self, query: &str, added: &[String]) -> Widgets {
        let field = SearchField::new(
            &L10N.tr("search-layouts"),
            query,
            submit(&self.intents, Intent::SourceQuery),
        );
        self.anchors.push((Anchor::SourceSearch, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::AddingSource(false));
        let mut rows = vec![boxed(Row::new(
            Text::new(&L10N.tr("add-input-source"), Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(()))),
                ],
            ))),
        ))];
        let matches = self.source_matches(query, added);
        if matches.is_empty() {
            rows.push(muted_row(&L10N.tr("no-layouts")));
        }
        for (code, name) in matches {
            let mut add = intent(&self.intents, Intent::AddSource);
            let chosen = code.clone();
            let row = Row::new(Text::new(&name, Tone::Primary), None)
                .detail(Text::new(&code, Tone::Muted).caption())
                .on_activate(move || add(chosen.clone()));
            rows.push(self.item_row(format!("add:{code}"), row));
        }
        rows
    }

    fn shortcut_cards(&mut self) -> Widgets {
        let mut page = Vec::new();
        for area in Area::ALL {
            page.push(boxed(Caption::new(
                Text::new(&area.title(), Tone::Muted).caption(),
            )));
            let rows = shortcuts_in(area)
                .map(|shortcut| {
                    boxed(Row::new(
                        Text::new(&shortcut.action.label(), Tone::Primary),
                        Some(boxed(Label::new(&shortcut_symbols(shortcut.keys)))),
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
            SettingId::TrackpadTapToClick
            | SettingId::TrackpadNaturalScrolling
            | SettingId::TrackpadSpeed
            | SettingId::TrackpadSecondaryClick
            | SettingId::TrackpadDisableWhileTyping => self.pointing.trackpad,
            SettingId::MouseNaturalScrolling | SettingId::MouseSpeed => self.pointing.mouse,
            SettingId::AutomaticTime => self.time.as_ref().is_some_and(|time| time.can_automatic),
            SettingId::DateAndTime | SettingId::TimeZone => self.time.is_some(),
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

    fn display_sections(
        &mut self,
        section: &Section,
        matched: &dyn Fn(SettingId) -> Option<SettingHit>,
    ) -> Widgets {
        let mut page = Vec::new();
        for display in self.displays.clone() {
            let title = if display.description.is_empty() {
                display.connector.clone()
            } else {
                display.description.clone()
            };
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
        let mut page: Widgets = pane
            .id
            .parent()
            .and_then(|parent| self.back_button(parent))
            .into_iter()
            .collect();
        page.push(boxed(Label::heading(&pane.name)));
        if pane.id == PaneId::General {
            page.push(boxed(Card::new(self.general_rows())));
        }
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
            if section
                .settings
                .iter()
                .any(|setting| setting.kind == Kind::List(List::Shortcuts))
            {
                if let Some(title) = &section.title {
                    page.push(boxed(Caption::new(
                        Text::new(title, Tone::Muted).caption().semibold(),
                    )));
                }
                page.extend(self.shortcut_cards());
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
