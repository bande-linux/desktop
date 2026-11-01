use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{ActionRequest, Role, TreeUpdate};
use bande_graphics::{
    Appearance, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
    TextSystem, grown_line,
};
use bande_widgets::{
    Alert, BUTTON_HEIGHT, Button, Cell, Column, Context, Event, FreeLayout, Key, Row, Size, Table,
    Ui, Widget, WidgetId,
};

use crate::L10N;

pub(crate) const CONTENT: (f32, f32) = (400.0, 400.0);
const PADDING: f32 = 16.0;
const HINT_GAP: f32 = 12.0;
const LIST_INSET: f32 = 4.0;
const FOOTER: f32 = 60.0;
const BUTTON_MIN_WIDTH: f32 = 96.0;
const CAPTION_GAP: f32 = 12.0;
const FORCE_QUIT: &str = "force-quit";
const CANCEL: &str = "cancel";

#[derive(Clone)]
pub(crate) struct Listed {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) icon: Option<Image>,
    pub(crate) responding: bool,
}

impl Listed {
    fn look(&self) -> (&str, &str, bool, bool) {
        (&self.key, &self.name, self.responding, self.icon.is_some())
    }

    pub(crate) fn label(&self) -> String {
        match self.responding {
            true => self.name.clone(),
            false => L10N.tr_args(
                "force-quit-not-responding",
                [("name", self.name.clone().into())],
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    ForceQuit(Vec<String>),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Selected(Vec<String>),
    Ask,
    Chosen(String),
}

type Signals = Rc<RefCell<Vec<Signal>>>;

pub(crate) struct ForceQuitView {
    ui: Ui,
    layout: WidgetId,
    table: WidgetId,
    button: (WidgetId, f32),
    signals: Signals,
    listed: Vec<Listed>,
    selected: Vec<String>,
    confirming: Vec<String>,
}

fn paragraph(text: &mut TextSystem, content: &str, style: TextStyle, width: f32) -> Paragraph {
    text.paragraph(content, style, Some(width), TextAlign::Start)
}

impl ForceQuitView {
    pub(crate) fn new(appearance: Appearance, text: TextSystem) -> Self {
        let signals = Signals::default();
        let selection = signals.clone();
        let activation = signals.clone();
        let table = Table::new(vec![
            Column::new(&L10N.tr("force-quit-title"), 200.0).flexible(),
        ])
        .without_header()
        .on_selection(move |keys| selection.borrow_mut().push(Signal::Selected(keys.to_vec())))
        .on_activate(move |_| activation.borrow_mut().push(Signal::Ask));
        let asking = signals.clone();
        let mut button = Button::new(&L10N.tr("force-quit-button"), move || {
            asking.borrow_mut().push(Signal::Ask);
        })
        .primary();
        button.set_enabled(false);
        let mut cx = Context::new(text, appearance.palette());
        let width = button
            .measure(&mut cx, Size::new(f32::MAX, BUTTON_HEIGHT))
            .width
            .max(BUTTON_MIN_WIDTH);
        let mut layout = FreeLayout::new();
        let table = layout.insert(table, Rect::default());
        let button = layout.insert(button, Rect::default());
        let layout_id = layout.id();
        let mut ui = Ui::new(Box::new(layout), cx);
        ui.set_appearance(appearance);
        Self {
            ui,
            layout: layout_id,
            table,
            button: (button, width),
            signals,
            listed: Vec::new(),
            selected: Vec::new(),
            confirming: Vec::new(),
        }
    }

    pub(crate) fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    pub(crate) fn listed(&self) -> &[Listed] {
        &self.listed
    }

    pub(crate) fn selected(&self) -> &[String] {
        &self.selected
    }

    pub(crate) fn set_apps(&mut self, listed: Vec<Listed>) -> bool {
        let same = listed.len() == self.listed.len()
            && listed
                .iter()
                .zip(&self.listed)
                .all(|(new, old)| new.look() == old.look());
        if same {
            return false;
        }
        let rows = listed
            .iter()
            .map(|app| {
                let cell = Cell::new(app.label()).icon(app.icon.clone());
                let cell = match app.responding {
                    true => cell,
                    false => cell.tinted(self.ui.context().palette.danger_text),
                };
                Row::new(app.key.clone(), vec![cell])
            })
            .collect();
        if let Some(table) = self.ui.widget_as::<Table>(self.table) {
            table.set_rows(rows);
        }
        self.listed = listed;
        let kept = self
            .selected
            .iter()
            .filter(|key| self.listed.iter().any(|app| app.key == **key))
            .cloned()
            .collect();
        self.select(kept);
        true
    }

    pub(crate) fn select(&mut self, keys: Vec<String>) {
        if let Some(table) = self.ui.widget_as::<Table>(self.table) {
            table.select(&keys);
        }
        self.signals
            .borrow_mut()
            .retain(|signal| !matches!(signal, Signal::Selected(_)));
        let enabled = !keys.is_empty();
        self.selected = keys;
        if let Some(button) = self.ui.widget_as::<Button>(self.button.0) {
            button.set_enabled(enabled);
        }
    }

    pub(crate) fn focus_list(&mut self) {
        self.ui.set_focus(self.table);
    }

    pub(crate) fn is_asking(&self) -> bool {
        self.ui.has_sheet()
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn place_controls(&mut self, list: Rect, width: f32, height: f32, footer: f32) {
        let (button, button_width) = self.button;
        let button_height = BUTTON_HEIGHT + self.ui.context().grown_line();
        let y = height - footer + ((footer - button_height) / 2.0).round();
        let frames = [
            (self.table, list.inset(LIST_INSET)),
            (
                button,
                Rect::new(
                    width - PADDING - button_width,
                    y,
                    button_width,
                    button_height,
                ),
            ),
        ];
        if let Some(layout) = self.ui.widget_as::<FreeLayout>(self.layout) {
            for (id, frame) in frames {
                layout.set_frame(id, frame);
            }
        }
    }

    pub(crate) fn paint(&mut self, width: f32, height: f32) -> Vec<Item> {
        let palette = self.ui.context().palette;
        let text_size = self.ui.context().text.size();
        let footer = FOOTER + grown_line(text_size);
        let text_width = width - 2.0 * PADDING;
        let caption_width = width - 3.0 * PADDING - self.button.1 - CAPTION_GAP;
        let empty = self.listed.is_empty();
        let text = &mut self.ui.context().text;
        let hint = paragraph(
            text,
            &L10N.tr("force-quit-hint"),
            TextStyle::role(TextRole::Callout, palette.ink2),
            text_width,
        );
        let caption = paragraph(
            text,
            &L10N.tr("force-quit-shortcut-hint"),
            TextStyle::role(TextRole::Callout, palette.ink3),
            caption_width.max(0.0),
        );
        let empty = empty.then(|| {
            text.paragraph(
                &L10N.tr("force-quit-empty"),
                TextStyle::role(TextRole::Body, palette.ink2),
                Some(text_width),
                TextAlign::Center,
            )
        });
        let list_top = (PADDING + hint.height() + HINT_GAP).round();
        let list = Rect::new(
            PADDING,
            list_top,
            text_width,
            (height - footer - list_top).max(0.0).round(),
        );
        self.place_controls(list, width, height, footer);
        let mut items = vec![
            Item::Text {
                origin: Point::new(PADDING, PADDING),
                paragraph: hint,
            },
            Item::Fill {
                shape: Shape::rounded(list, Radius::M),
                paint: palette.overlay.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(list.inset(0.5), Radius::M - 0.5),
                width: 1.0,
                paint: palette.hairline.into(),
            },
        ];
        if let Some(empty) = empty {
            items.push(Item::Text {
                origin: Point::new(
                    PADDING,
                    (list.y + (list.height - empty.height()) / 2.0).round(),
                ),
                paragraph: empty,
            });
        }
        items.push(Item::Text {
            origin: Point::new(
                PADDING,
                (height - footer + (footer - caption.height()) / 2.0).round(),
            ),
            paragraph: caption,
        });
        let (controls, dialog) = self.ui.paint_layers(width as u32, height as u32);
        items.extend(controls);
        items.extend(dialog);
        items
    }

    fn ask(&mut self) {
        let keys = self.selected.clone();
        let names: Vec<&str> = self
            .listed
            .iter()
            .filter(|app| keys.contains(&app.key))
            .map(|app| app.name.as_str())
            .collect();
        let title = match names.as_slice() {
            [] => return,
            [name] => L10N.tr_args("force-quit-confirm", [("name", (*name).to_owned().into())]),
            _ => L10N.tr_args("force-quit-confirm-many", [("count", names.len().into())]),
        };
        let icon = match keys.as_slice() {
            [key] => self
                .listed
                .iter()
                .find(|app| app.key == *key)
                .and_then(|app| app.icon.clone()),
            _ => None,
        };
        let signals = self.signals.clone();
        let alert = Alert::new(
            &title,
            &L10N.tr("force-quit-confirm-message"),
            move |choice| {
                signals.borrow_mut().push(Signal::Chosen(choice.to_owned()));
            },
        )
        .primary(&L10N.tr("force-quit-button"), FORCE_QUIT)
        .cancel(&L10N.tr("force-quit-cancel"), CANCEL);
        let alert = match icon {
            Some(icon) => alert.icon(icon),
            None => alert,
        };
        self.confirming = keys;
        self.ui.present_dialog(alert);
    }

    pub(crate) fn handle(&mut self, event: &Event) -> (bool, Vec<Effect>) {
        let handled = match event {
            Event::Key {
                key: Key::Escape, ..
            } if !self.ui.has_sheet() => {
                return (true, vec![Effect::Close]);
            }
            Event::Key {
                key: Key::Enter, ..
            } if !self.ui.has_sheet() => {
                self.signals.borrow_mut().push(Signal::Ask);
                true
            }
            _ => self.ui.handle(event),
        };
        self.drain(handled)
    }

    pub(crate) fn perform(&mut self, request: &ActionRequest) -> (bool, Vec<Effect>) {
        let performed = self.ui.perform(request);
        self.drain(performed)
    }

    fn drain(&mut self, mut handled: bool) -> (bool, Vec<Effect>) {
        let mut effects = Vec::new();
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() {
                break;
            }
            handled = true;
            for signal in signals {
                match signal {
                    Signal::Selected(keys) => self.select(keys),
                    Signal::Ask => self.ask(),
                    Signal::Chosen(choice) => {
                        let keys = std::mem::take(&mut self.confirming);
                        if choice == FORCE_QUIT && !keys.is_empty() {
                            effects.push(Effect::ForceQuit(keys));
                        }
                    }
                }
            }
        }
        (handled, effects)
    }

    pub(crate) fn accessibility(&self) -> TreeUpdate {
        let mut update = self.ui.accessibility();
        if let Some((_, root)) = update.nodes.first_mut() {
            root.set_role(Role::Dialog);
            root.set_label(L10N.tr("force-quit-title"));
            root.set_description(L10N.tr("force-quit-hint"));
        }
        update
    }
}
