use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role, TreeId};
use bande_datetime::DateFormats;
use bande_l10n::tr;
use bande_toolkit::ui::{Button, Context, Label, Size, TEXT_FIELD_HEIGHT, Ui, Widget, WidgetId};
use bande_toolkit::{Item, Paint, Radius, Rect, Shape};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::panel::{Line, Panel, Width};
use super::{Signal, Signals, sender};
use crate::caldav::Availability;
use crate::ical::freebusy::Period;
use crate::schedule::{Hours, free_slots};

const WIDTH: f32 = 520.0;
const LABEL_WIDTH: f32 = 150.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;
const STRIP: f32 = 18.0;
const SUGGESTIONS: usize = 3;
const DAYS: i64 = 14;

pub struct Strip {
    id: WidgetId,
    frame: Rect,
    name: String,
    open: Period,
    event: Period,
    busy: Option<Vec<Period>>,
    description: String,
}

impl Strip {
    fn new(name: &str, open: Period, event: Period) -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            name: name.to_owned(),
            open,
            event,
            busy: None,
            description: tr!("Not known"),
        }
    }

    fn x_of(&self, at: Timestamp) -> f32 {
        let (open, close) = self.open;
        let whole = open.duration_until(close).as_secs_f32().max(1.0);
        let part = open.duration_until(at.clamp(open, close)).as_secs_f32();
        self.frame.x + self.frame.width * part / whole
    }

    fn span(&self, (from, to): Period) -> Option<Rect> {
        let (left, right) = (self.x_of(from), self.x_of(to));
        (right > left).then(|| Rect::new(left, self.frame.y, right - left, self.frame.height))
    }
}

impl Widget for Strip {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, STRIP)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let palette = &cx.palette;
        let well = match self.busy {
            Some(_) => palette.well,
            None => palette.hairline,
        };
        items.push(Item::Fill {
            shape: Shape::rounded(self.frame, Radius::XS),
            paint: Paint::Solid(well),
        });
        for rect in self
            .busy
            .iter()
            .flatten()
            .filter_map(|period| self.span(*period))
        {
            items.push(Item::Fill {
                shape: Shape::rounded(rect, Radius::XS),
                paint: Paint::Solid(palette.danger),
            });
        }
        if let Some(rect) = self.span(self.event) {
            items.push(Item::Stroke {
                shape: Shape::rounded(rect, Radius::XS),
                width: 2.0,
                paint: Paint::Solid(palette.ink),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_label(self.name.clone());
        node.set_value(self.description.clone());
        node
    }
}

pub struct Sheet {
    panel: WidgetId,
    cancel: WidgetId,
    status: WidgetId,
    strips: Vec<(String, WidgetId)>,
    choices: Vec<WidgetId>,
    choices_line: usize,
    starts: Vec<Timestamp>,
    length: SignedDuration,
    open: Period,
    from: Timestamp,
    dismissed: Rc<Cell<bool>>,
}

pub struct Asked<'a> {
    pub attendees: &'a [(String, String)],
    pub event: Period,
    pub day: Date,
    pub hours: &'a Hours<'a>,
    pub now: Timestamp,
}

impl Asked<'_> {
    pub fn window(&self) -> Option<Period> {
        let from = self.day.to_zoned(self.hours.zone.clone()).ok()?.timestamp();
        let to = from
            .checked_add(SignedDuration::from_hours(24 * DAYS))
            .ok()?;
        Some((from, to))
    }
}

fn describe(formats: &DateFormats, zone: &TimeZone, busy: &[Period], open: Period) -> String {
    let clock = |at: Timestamp| formats.clock_time(&at.to_zoned(zone.clone()));
    let shown: Vec<String> = busy
        .iter()
        .filter(|(from, to)| *from < open.1 && *to > open.0)
        .map(|(from, to)| format!("{}–{}", clock(*from.max(&open.0)), clock(*to.min(&open.1))))
        .collect();
    match shown.is_empty() {
        true => tr!("Free"),
        false => tr!("Busy {times}", times = shown.join(", ")),
    }
}

impl Sheet {
    pub fn build(signals: &Signals, asked: &Asked, formats: &DateFormats) -> Option<(Panel, Self)> {
        let open = asked.hours.of(asked.day)?;
        let length = asked.event.0.duration_until(asked.event.1);
        let mut panel = Panel::new(Some(&tr!("Availability")), WIDTH, LABEL_WIDTH).sheet();
        let day = asked
            .day
            .to_zoned(asked.hours.zone.clone())
            .map(|at| formats.long_date(&at))
            .unwrap_or_default();
        let day = panel.add(Label::new(&day));
        panel.line(Line::new(None, vec![(day, Width::Fill)], 18.0));
        let strips: Vec<(String, usize)> = asked
            .attendees
            .iter()
            .map(|(address, name)| {
                let strip = panel.add(Strip::new(name, open, asked.event));
                panel.line(Line::new(Some(name), vec![(strip, Width::Fill)], STRIP));
                (address.clone(), strip)
            })
            .collect();
        let status = panel.add(Label::new(&tr!("Asking the server…")).secondary());
        panel.line(Line::new(None, vec![(status, Width::Fill)], 18.0).separated());
        let choices: Vec<usize> = (0..SUGGESTIONS)
            .map(|index| {
                let send = sender(signals);
                panel.add(Button::new("", move || send(Signal::Suggestion(index))))
            })
            .collect();
        let choices_line = panel.line(Line::new(
            Some(&tr!("Free at:")),
            choices
                .iter()
                .map(|choice| (*choice, Width::Fill))
                .collect(),
            ROW,
        ));
        panel.show_line(choices_line, false);
        let send = sender(signals);
        let cancel =
            panel.add(Button::new(&tr!("Close"), move || send(Signal::AvailabilityClose)).cancel());
        panel.line(
            Line::new(None, vec![(cancel, Width::Fixed(96.0))], ROW)
                .separated()
                .trailing(),
        );
        let id = |index: usize| panel.id_of(index).unwrap_or_else(WidgetId::next);
        let sheet = Self {
            panel: bande_toolkit::ui::Widget::id(&panel),
            cancel: id(cancel),
            status: id(status),
            strips: strips
                .into_iter()
                .map(|(address, strip)| (address, id(strip)))
                .collect(),
            choices: choices.into_iter().map(id).collect(),
            choices_line,
            starts: Vec::new(),
            length,
            open,
            from: asked.now.max(open.0),
            dismissed: panel.dismisser(),
        };
        Some((panel, sheet))
    }

    pub fn answered(
        &mut self,
        ui: &mut Ui,
        answers: Result<Vec<(String, Availability)>, String>,
        formats: &DateFormats,
        hours: &Hours,
    ) {
        let answers = match answers {
            Ok(answers) => answers,
            Err(error) => {
                let text = tr!("Can’t get availability: {error}", error = error);
                ui.update_widget::<Label>(self.status, |label, _| label.set_text(&text));
                return;
            }
        };
        let mut all = Vec::new();
        let mut unknown = 0;
        for (address, strip) in &self.strips {
            let busy = answers
                .iter()
                .find(|(answered, _)| answered.eq_ignore_ascii_case(address))
                .and_then(|(_, availability)| match availability {
                    Availability::Busy(busy) => Some(busy.clone()),
                    Availability::Unknown => None,
                });
            let description = match &busy {
                Some(busy) => describe(formats, hours.zone, busy, self.open),
                None => tr!("Not known"),
            };
            match &busy {
                Some(busy) => all.extend(busy.iter().copied()),
                None => unknown += 1,
            }
            ui.update_widget::<Strip>(*strip, |strip, _| {
                strip.busy = busy;
                strip.description = description;
            });
        }
        self.starts = free_slots(&all, self.length, self.from, hours, DAYS, SUGGESTIONS);
        let status = match (unknown, self.starts.is_empty()) {
            (0, true) => tr!("No common free time in the next two weeks."),
            (0, false) => tr!("Everyone is free at these times."),
            (_, _) => tr!("The server doesn’t share everyone’s availability."),
        };
        ui.update_widget::<Label>(self.status, |label, _| label.set_text(&status));
        for (index, choice) in self.choices.iter().enumerate() {
            let title = self.starts.get(index).map_or_else(String::new, |start| {
                formats.date_time(&start.to_zoned(hours.zone.clone()), false)
            });
            let shown = index < self.starts.len();
            ui.update_widget::<Button>(*choice, |button, _| {
                button.set_title(&title);
                button.set_enabled(shown);
            });
        }
        let (line, shown) = (self.choices_line, !self.starts.is_empty());
        ui.update_widget::<Panel>(self.panel, |panel, _| panel.show_line(line, shown));
    }

    pub fn chosen(&self, index: usize) -> Option<Period> {
        let start = *self.starts.get(index)?;
        Some((start, start.checked_add(self.length).ok()?))
    }

    pub fn close(&self, ui: &mut Ui) {
        self.dismissed.set(true);
        ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: self.cancel.into(),
            data: None,
        });
    }
}
