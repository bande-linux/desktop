use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, Role};
use unbound_toolkit::ui::{Context, Event, Key, Parts, Size, Widget, WidgetId};
use unbound_toolkit::{
    Image, Item, Paragraph, Path, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::media::{Command, Playback, Player};
use crate::{Change, Changes, L10N};

const ART: f32 = 48.0;
const ART_RADIUS: f32 = 8.0;
const TEXT_GAP: f32 = 12.0;
const BUTTON: f32 = 28.0;
const BUTTON_GAP: f32 = 2.0;
const SCRUB_TOP: f32 = ART + 12.0;
const TRACK: f32 = 4.0;
const SCRUB_HIT: f32 = 16.0;
const TIMES_TOP: f32 = SCRUB_TOP + TRACK + 5.0;
const TIMES_HEIGHT: f32 = 13.0;
const SEEK_STEP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Previous,
    PlayPause,
    Next,
}

const CONTROLS: [Control; 3] = [Control::Previous, Control::PlayPause, Control::Next];
const SCRUBBER_PART: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Press {
    Control(Control),
    Scrub(f32),
    Card,
}

pub struct NowPlaying {
    id: WidgetId,
    player: Player,
    artwork: Option<Image>,
    changes: Changes,
    press: Option<Press>,
    lines: Vec<(Paragraph, f32)>,
    times: Option<(Paragraph, Paragraph, f32)>,
    frame: Rect,
}

pub fn height(player: &Player) -> f32 {
    if scrubbable(player) {
        TIMES_TOP + TIMES_HEIGHT
    } else {
        ART
    }
}

fn scrubbable(player: &Player) -> bool {
    player.length.is_some() && player.position.is_some()
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

pub fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn triangle(left: f32, top: f32, width: f32, height: f32, forward: bool) -> Path {
    let (tip, base) = if forward {
        (left + width, left)
    } else {
        (left, left + width)
    };
    Path::new()
        .move_to(Point::new(base, top))
        .line_to(Point::new(tip, top + height / 2.0))
        .line_to(Point::new(base, top + height))
        .close()
}

fn bar(left: f32, top: f32, width: f32, height: f32) -> Path {
    Path::new()
        .move_to(Point::new(left, top))
        .line_to(Point::new(left + width, top))
        .line_to(Point::new(left + width, top + height))
        .line_to(Point::new(left, top + height))
        .close()
}

fn symbol(control: Control, playing: bool, center: Point) -> Vec<Path> {
    let (x, y) = (center.x, center.y);
    match control {
        Control::PlayPause if playing => vec![
            bar(x - 5.0, y - 6.5, 3.5, 13.0),
            bar(x + 1.5, y - 6.5, 3.5, 13.0),
        ],
        Control::PlayPause => vec![triangle(x - 4.5, y - 7.0, 11.0, 14.0, true)],
        Control::Next => vec![
            triangle(x - 8.0, y - 5.0, 8.0, 10.0, true),
            triangle(x, y - 5.0, 8.0, 10.0, true),
        ],
        Control::Previous => vec![
            triangle(x - 8.0, y - 5.0, 8.0, 10.0, false),
            triangle(x, y - 5.0, 8.0, 10.0, false),
        ],
    }
}

fn music_note(center: Point) -> Path {
    let (x, y) = (center.x, center.y);
    Path::new()
        .move_to(Point::new(x - 2.0, y + 6.0))
        .cubic_to(
            Point::new(x - 2.0, y + 2.5),
            Point::new(x - 10.0, y + 2.5),
            Point::new(x - 10.0, y + 6.0),
        )
        .cubic_to(
            Point::new(x - 10.0, y + 9.5),
            Point::new(x - 2.0, y + 9.5),
            Point::new(x - 2.0, y + 6.0),
        )
        .line_to(Point::new(x - 2.0, y - 9.0))
        .line_to(Point::new(x + 8.0, y - 11.0))
        .line_to(Point::new(x + 8.0, y - 7.0))
        .line_to(Point::new(x, y - 5.4))
        .line_to(Point::new(x, y + 6.0))
        .close()
}

fn fitted(text: &mut TextSystem, line: &str, style: TextStyle, width: f32) -> Paragraph {
    let whole = text.paragraph(line, style, None, TextAlign::Start);
    if whole.width() <= width {
        return whole;
    }
    let characters: Vec<char> = line.chars().collect();
    let shortened = |count: usize| {
        let kept: String = characters[..count].iter().collect();
        format!("{}…", kept.trim_end())
    };
    let (mut fits, mut too_long) = (0, characters.len());
    while too_long - fits > 1 {
        let middle = (fits + too_long) / 2;
        if text
            .paragraph(&shortened(middle), style, None, TextAlign::Start)
            .width()
            <= width
        {
            fits = middle;
        } else {
            too_long = middle;
        }
    }
    text.paragraph(&shortened(fits), style, None, TextAlign::Start)
}

impl NowPlaying {
    pub fn new(player: Player, artwork: Option<Image>, changes: &Changes) -> Self {
        Self {
            id: WidgetId::next(),
            player,
            artwork,
            changes: Changes::clone(changes),
            press: None,
            lines: Vec::new(),
            times: None,
            frame: Rect::default(),
        }
    }

    fn playing(&self) -> bool {
        self.player.playback == Playback::Playing
    }

    fn enabled(&self, control: Control) -> bool {
        match control {
            Control::Previous => self.player.can_go_previous,
            Control::Next => self.player.can_go_next,
            Control::PlayPause if self.playing() => self.player.can_pause,
            Control::PlayPause => self.player.can_play,
        }
    }

    fn label(&self, control: Control) -> String {
        L10N.tr(match control {
            Control::Previous => "previous-track",
            Control::Next => "next-track",
            Control::PlayPause if self.playing() => "pause",
            Control::PlayPause => "play",
        })
    }

    fn control_rect(&self, index: usize) -> Rect {
        let right = self.frame.x + self.frame.width;
        let left = right - (CONTROLS.len() - index) as f32 * (BUTTON + BUTTON_GAP) + BUTTON_GAP;
        Rect::new(left, self.frame.y + (ART - BUTTON) / 2.0, BUTTON, BUTTON)
    }

    fn track_rect(&self) -> Rect {
        Rect::new(
            self.frame.x,
            self.frame.y + SCRUB_TOP,
            self.frame.width,
            TRACK,
        )
    }

    fn scrub_hit(&self) -> Rect {
        let track = self.track_rect();
        Rect::new(
            track.x,
            track.y - (SCRUB_HIT - TRACK) / 2.0,
            track.width,
            SCRUB_HIT,
        )
    }

    fn fraction_at(&self, point: Point) -> f32 {
        let track = self.track_rect();
        ((point.x - track.x) / track.width.max(1.0)).clamp(0.0, 1.0)
    }

    fn position(&self, now: Instant) -> Option<(Duration, Duration)> {
        let length = self.player.length?;
        let position = match self.press {
            Some(Press::Scrub(fraction)) => length.mul_f32(fraction),
            _ => self.player.position_at(now)?,
        };
        Some((position, length))
    }

    fn send(&self, change: Change) {
        self.changes.borrow_mut().push(change);
    }

    fn run(&self, control: Control) {
        if !self.enabled(control) {
            return;
        }
        self.send(Change::Media(match control {
            Control::Previous => Command::Previous,
            Control::Next => Command::Next,
            Control::PlayPause => Command::PlayPause,
        }));
    }

    fn seek_to(&self, position: Duration) {
        let (Some(length), true) = (self.player.length, self.player.can_seek) else {
            return;
        };
        self.send(Change::Media(Command::SetPosition {
            track: self.player.track.clone(),
            position: position.min(length),
            from: self.player.position_at(Instant::now()).unwrap_or_default(),
        }));
    }

    fn seek_by(&self, forward: bool) {
        let now = self.player.position_at(Instant::now()).unwrap_or_default();
        self.seek_to(if forward {
            now + SEEK_STEP
        } else {
            now.saturating_sub(SEEK_STEP)
        });
    }

    fn paint_artwork(&self, cx: &Context, items: &mut Vec<Item>) {
        let rect = Rect::new(self.frame.x, self.frame.y, ART, ART);
        let shape = Shape::rounded(rect, ART_RADIUS);
        match &self.artwork {
            Some(image) => {
                let (width, height) = (image.width() as f32, image.height() as f32);
                let scale = (ART / width).max(ART / height);
                let shown = Rect::new(
                    rect.x + (ART - width * scale) / 2.0,
                    rect.y + (ART - height * scale) / 2.0,
                    width * scale,
                    height * scale,
                );
                items.push(Item::Clip {
                    shape,
                    items: vec![Item::Image {
                        rect: shown,
                        image: image.clone(),
                    }],
                });
            }
            None => {
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.hairline.into(),
                });
                items.push(Item::FillPath {
                    path: music_note(Point::new(rect.x + ART / 2.0, rect.y + ART / 2.0)),
                    paint: cx.palette.ink2.into(),
                });
            }
        }
    }
}

impl Widget for NowPlaying {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let (title, subtitle) = if self.player.title.is_empty() {
            (self.player.identity.as_str(), "")
        } else if self.player.artist.is_empty() {
            (self.player.title.as_str(), self.player.identity.as_str())
        } else {
            (self.player.title.as_str(), self.player.artist.as_str())
        };
        let styles = [
            (
                title,
                TextStyle::new(13.0, cx.palette.ink).weight(TextStyle::SEMIBOLD),
                7.0,
            ),
            (subtitle, TextStyle::new(12.0, cx.palette.ink2), 26.0),
        ];
        let text_width =
            available.width - ART - 2.0 * TEXT_GAP - CONTROLS.len() as f32 * (BUTTON + BUTTON_GAP)
                + BUTTON_GAP;
        self.lines = styles
            .into_iter()
            .filter(|(line, ..)| !line.is_empty())
            .map(|(line, style, top)| (fitted(&mut cx.text, line, style, text_width), top))
            .collect();
        self.times = self.position(cx.now()).map(|(position, length)| {
            let style = TextStyle::new(11.0, cx.palette.ink2).tabular();
            let fraction =
                (position.as_secs_f32() / length.as_secs_f32().max(f32::EPSILON)).min(1.0);
            (
                cx.text
                    .paragraph(&clock(position), style, None, TextAlign::Start),
                cx.text.paragraph(
                    &format!("-{}", clock(length.saturating_sub(position))),
                    style,
                    None,
                    TextAlign::Start,
                ),
                fraction,
            )
        });
        Size::new(available.width, height(&self.player))
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, height(&self.player));
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.paint_artwork(cx, items);
        let text_left = self.frame.x + ART + TEXT_GAP;
        let text_width = (self.control_rect(0).x - TEXT_GAP - text_left).max(0.0);
        let clipped = self
            .lines
            .iter()
            .map(|(paragraph, top)| Item::Text {
                origin: Point::new(text_left, self.frame.y + top),
                paragraph: paragraph.clone(),
            })
            .collect();
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(text_left, self.frame.y, text_width, ART)),
            items: clipped,
        });
        for (index, control) in CONTROLS.into_iter().enumerate() {
            let rect = self.control_rect(index);
            let enabled = self.enabled(control);
            if self.press == Some(Press::Control(control)) && enabled {
                items.push(Item::Fill {
                    shape: Shape::rounded(rect, BUTTON / 2.0),
                    paint: cx.palette.hairline.into(),
                });
            }
            let color = if enabled {
                cx.palette.ink
            } else {
                cx.palette.ink2.with_alpha(110)
            };
            let center = Point::new(rect.x + BUTTON / 2.0, rect.y + BUTTON / 2.0);
            for path in symbol(control, self.playing(), center) {
                items.push(Item::FillPath {
                    path,
                    paint: color.into(),
                });
            }
        }
        if cx.is_focused(self.id) {
            items.push(Item::Stroke {
                shape: Shape::rounded(self.frame.inset(-3.0), ART_RADIUS + 3.0),
                width: 3.0,
                paint: cx.palette.accent.with_alpha(128).into(),
            });
        }
        let Some((elapsed, remaining, fraction)) = &self.times else {
            return;
        };
        let track = self.track_rect();
        items.push(Item::Fill {
            shape: Shape::rounded(track, TRACK / 2.0),
            paint: cx.palette.hairline.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rounded(
                Rect::new(track.x, track.y, track.width * fraction, TRACK),
                TRACK / 2.0,
            ),
            paint: cx.palette.ink2.into(),
        });
        let top = self.frame.y + TIMES_TOP;
        items.push(Item::Text {
            origin: Point::new(self.frame.x + self.frame.width - remaining.width(), top),
            paragraph: remaining.clone(),
        });
        items.push(Item::Text {
            origin: Point::new(self.frame.x, top),
            paragraph: elapsed.clone(),
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(L10N.tr("now-playing"));
        let description = [
            &self.player.title,
            &self.player.artist,
            &self.player.identity,
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
        node.set_description(description);
        node.add_action(Action::Click);
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let bounds = |rect: Rect| accesskit::Rect {
            x0: f64::from(rect.x),
            y0: f64::from(rect.y),
            x1: f64::from(rect.x + rect.width),
            y1: f64::from(rect.y + rect.height),
        };
        let mut nodes: Vec<_> = CONTROLS
            .into_iter()
            .enumerate()
            .map(|(index, control)| {
                let mut node = Node::new(Role::Button);
                node.set_label(self.label(control));
                if self.enabled(control) {
                    node.add_action(Action::Click);
                } else {
                    node.set_disabled();
                }
                node.set_bounds(bounds(self.control_rect(index)));
                (self.id.part(index as u64), node)
            })
            .collect();
        if let Some((position, length)) = self.position(cx.now()) {
            let mut node = Node::new(Role::Slider);
            node.set_label(L10N.tr("playback-position"));
            node.set_value(clock(position));
            node.set_numeric_value(position.as_secs_f64());
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(length.as_secs_f64());
            node.set_numeric_value_step(SEEK_STEP.as_secs_f64());
            if self.player.can_seek {
                node.add_action(Action::Increment);
                node.add_action(Action::Decrement);
                node.add_action(Action::SetValue);
            }
            node.set_bounds(bounds(self.track_rect()));
            nodes.push((self.id.part(SCRUBBER_PART), node));
        }
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            focus: None,
            nodes,
        }
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.send(Change::RaisePlayer);
        true
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if let Some(control) = CONTROLS.get(part as usize) {
            if request.action != Action::Click || !self.enabled(*control) {
                return false;
            }
            self.run(*control);
            return true;
        }
        if part != SCRUBBER_PART || !self.player.can_seek {
            return false;
        }
        match (request.action, &request.data) {
            (Action::Increment, _) => self.seek_by(true),
            (Action::Decrement, _) => self.seek_by(false),
            (Action::SetValue, Some(ActionData::NumericValue(seconds))) => {
                self.seek_to(Duration::from_secs_f64(seconds.max(0.0)));
            }
            _ => return false,
        }
        true
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                let point = pointer.position;
                self.press = (0..CONTROLS.len())
                    .find(|index| contains(self.control_rect(*index), point))
                    .map(|index| Press::Control(CONTROLS[index]))
                    .or_else(|| {
                        (self.player.can_seek
                            && scrubbable(&self.player)
                            && contains(self.scrub_hit(), point))
                        .then(|| Press::Scrub(self.fraction_at(point)))
                    })
                    .or_else(|| {
                        contains(
                            Rect::new(self.frame.x, self.frame.y, self.frame.width, ART),
                            point,
                        )
                        .then_some(Press::Card)
                    });
            }
            Event::PointerMove(pointer) => match self.press {
                Some(Press::Scrub(_)) => {
                    self.press = Some(Press::Scrub(self.fraction_at(pointer.position)));
                }
                _ => return false,
            },
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let point = pointer.position;
                match self.press.take() {
                    Some(Press::Control(control)) => {
                        let index = CONTROLS.iter().position(|known| *known == control);
                        if index.is_some_and(|index| contains(self.control_rect(index), point)) {
                            self.run(control);
                        }
                    }
                    Some(Press::Scrub(_)) => {
                        if let Some(length) = self.player.length {
                            self.seek_to(length.mul_f32(self.fraction_at(point)));
                        }
                    }
                    Some(Press::Card) if contains(self.frame, point) => {
                        self.send(Change::RaisePlayer);
                    }
                    _ => return false,
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => self.run(Control::PlayPause),
            Event::Key { key: Key::Left, .. } => self.seek_by(false),
            Event::Key {
                key: Key::Right, ..
            } => self.seek_by(true),
            Event::Key {
                key: Key::Enter, ..
            } => self.send(Change::RaisePlayer),
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
