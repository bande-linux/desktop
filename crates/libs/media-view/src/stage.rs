use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, Role};
use bande_l10n::{text, tr};
use bande_media::{FailureKind, Playback};
use bande_toolkit::ui::{
    Context, Event, Key, Modifiers, Parts, Size, SurfaceScaling, Widget, WidgetId,
};
use bande_toolkit::{
    Color, Item, Palette, Paragraph, Path, Point, Radius, Rect, Shape, TextAlign, TextRole,
    TextStyle, TextSystem,
};

use crate::Session;

const BAR_HEIGHT: f32 = 44.0;
const BAR_WIDTH: f32 = 560.0;
const BAR_INSET: f32 = 16.0;
const BUTTON: f32 = 36.0;
const GAP: f32 = 10.0;
const TRACK: f32 = 4.0;
const TRACK_HIT: f32 = 24.0;
const TIME_WIDTH: f32 = 52.0;
const VOLUME_WIDTH: f32 = 72.0;
const VOLUME_ICON: f32 = 28.0;
const NARROW: f32 = 380.0;
const ART: f32 = 180.0;
const LINE_GAP: f32 = 6.0;
const SUBTITLE_MARGIN: f32 = 24.0;
const SUBTITLE_PADDING: f32 = 6.0;
const CONTROLS_SHOWN: Duration = Duration::from_secs(3);
const SCRUB_INTERVAL: Duration = Duration::from_millis(120);
const SEEK_STEP: Duration = Duration::from_secs(5);
const VOLUME_STEP: f64 = 0.1;
const BAR_OPACITY: f32 = 0.62;
const DIM_OPACITY: f32 = 0.32;
const INK: Color = Color::WHITE;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Press {
    Play,
    Scrub(f32),
    Volume,
    Mute,
}

#[derive(Default)]
struct Layout {
    frame: Rect,
    bar: Rect,
    play: Rect,
    elapsed: Rect,
    track: Rect,
    remaining: Rect,
    mute: Rect,
    volume: Rect,
    wide: bool,
}

pub struct Stage {
    session: Session,
    layout: Layout,
    press: Option<Press>,
    controls_visible: bool,
    hide_at: Option<Instant>,
    last_scrub: Option<Instant>,
    elapsed_text: Option<Paragraph>,
    remaining_text: Option<Paragraph>,
    subtitle_text: Option<Paragraph>,
    title_text: Option<Paragraph>,
    artist_text: Option<Paragraph>,
    failure_text: Vec<Paragraph>,
    hint_text: Option<Paragraph>,
}

fn clock(time: Duration) -> String {
    let seconds = time.as_secs();
    match seconds / 3600 {
        0 => format!("{}:{:02}", seconds / 60, seconds % 60),
        hours => format!("{hours}:{:02}:{:02}", seconds / 60 % 60, seconds % 60),
    }
}

fn triangle(center: Point) -> Path {
    Path::new()
        .move_to(Point::new(center.x - 5.0, center.y - 8.0))
        .line_to(Point::new(center.x + 8.0, center.y))
        .line_to(Point::new(center.x - 5.0, center.y + 8.0))
        .close()
}

fn speaker(center: Point, muted: bool) -> Vec<Item> {
    let (x, y) = (center.x, center.y);
    let body = Path::new()
        .move_to(Point::new(x - 8.0, y - 3.0))
        .line_to(Point::new(x - 4.0, y - 3.0))
        .line_to(Point::new(x + 1.0, y - 7.0))
        .line_to(Point::new(x + 1.0, y + 7.0))
        .line_to(Point::new(x - 4.0, y + 3.0))
        .line_to(Point::new(x - 8.0, y + 3.0))
        .close();
    let mut items = vec![Item::FillPath {
        path: body,
        paint: INK.into(),
    }];
    let line = |from: Point, to: Point| Item::Line {
        from,
        to,
        width: 1.5,
        paint: INK.into(),
    };
    match muted {
        true => {
            items.push(line(
                Point::new(x + 4.0, y - 3.0),
                Point::new(x + 10.0, y + 3.0),
            ));
            items.push(line(
                Point::new(x + 10.0, y - 3.0),
                Point::new(x + 4.0, y + 3.0),
            ));
        }
        false => {
            items.push(line(
                Point::new(x + 4.0, y - 3.0),
                Point::new(x + 4.0, y + 3.0),
            ));
            items.push(line(
                Point::new(x + 7.5, y - 5.0),
                Point::new(x + 7.5, y + 5.0),
            ));
        }
    }
    items
}

fn fraction_in(rect: Rect, x: f32) -> f32 {
    ((x - rect.x) / rect.width.max(1.0)).clamp(0.0, 1.0)
}

fn grown(rect: Rect, height: f32) -> Rect {
    Rect::new(
        rect.x,
        rect.y + (rect.height - height) / 2.0,
        rect.width,
        height,
    )
}

impl Stage {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            layout: Layout::default(),
            press: None,
            controls_visible: true,
            hide_at: None,
            last_scrub: None,
            elapsed_text: None,
            remaining_text: None,
            subtitle_text: None,
            title_text: None,
            artist_text: None,
            failure_text: Vec::new(),
            hint_text: None,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    fn fades(&self) -> bool {
        self.session.has_video() && self.session.is_playing() && self.press.is_none()
    }

    fn duration(&self) -> Option<Duration> {
        self.session
            .info()
            .duration_ms
            .map(Duration::from_millis)
            .filter(|duration| !duration.is_zero())
    }

    fn fraction(&self, now: Instant) -> f32 {
        if let Some(Press::Scrub(fraction)) = self.press {
            return fraction;
        }
        match self.duration() {
            Some(duration) => {
                (self.session.position(now).as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
            }
            None => 0.0,
        }
    }

    pub fn settle(&mut self, now: Instant) {
        match self.fades() {
            true if self.controls_visible => {
                self.hide_at.get_or_insert(now + CONTROLS_SHOWN);
            }
            _ => self.hide_at = None,
        }
        if !self.fades() {
            self.controls_visible = true;
        }
        self.session.set_refreshing(self.controls_visible);
    }

    pub fn deadline(&self) -> Option<Instant> {
        [self.hide_at, self.session.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = self.session.tick(now);
        if self.hide_at.is_some_and(|at| at <= now) {
            self.hide_at = None;
            self.controls_visible = false;
            changed = true;
        }
        self.settle(now);
        changed
    }

    pub fn reveal_controls(&mut self, now: Instant) -> bool {
        let shown = !std::mem::replace(&mut self.controls_visible, true);
        self.hide_at = None;
        self.settle(now);
        shown
    }

    pub fn layout(&mut self, text: &mut TextSystem, palette: &Palette, frame: Rect, now: Instant) {
        let bar_width = BAR_WIDTH.min(frame.width - 2.0 * BAR_INSET).max(0.0);
        let bar = Rect::new(
            frame.x + (frame.width - bar_width) / 2.0,
            frame.y + frame.height - BAR_HEIGHT - BAR_INSET,
            bar_width,
            BAR_HEIGHT,
        );
        let middle = bar.y + (BAR_HEIGHT - BUTTON) / 2.0;
        let wide = bar.width >= NARROW;
        let play = Rect::new(bar.x + GAP / 2.0, middle, BUTTON, BUTTON);
        let right = bar.x + bar.width - GAP;
        let (mute, volume) = match wide {
            true => (
                Rect::new(
                    right - VOLUME_WIDTH - VOLUME_ICON,
                    middle,
                    VOLUME_ICON,
                    BUTTON,
                ),
                Rect::new(right - VOLUME_WIDTH, middle, VOLUME_WIDTH, BUTTON),
            ),
            false => (Rect::default(), Rect::default()),
        };
        let track_end = match wide {
            true => mute.x - GAP,
            false => right,
        };
        let elapsed = Rect::new(play.x + BUTTON, middle, TIME_WIDTH, BUTTON);
        let remaining = Rect::new(track_end - TIME_WIDTH, middle, TIME_WIDTH, BUTTON);
        let track_x = elapsed.x + TIME_WIDTH + GAP / 2.0;
        let track = Rect::new(
            track_x,
            middle,
            (remaining.x - GAP / 2.0 - track_x).max(0.0),
            BUTTON,
        );
        self.layout = Layout {
            frame,
            bar,
            play,
            elapsed,
            track,
            remaining,
            mute,
            volume,
            wide,
        };
        let time_style = TextStyle::role(TextRole::Label, INK);
        let position = match (self.press, self.duration()) {
            (Some(Press::Scrub(fraction)), Some(duration)) => duration.mul_f32(fraction),
            _ => self.session.position(now),
        };
        self.elapsed_text = Some(text.paragraph(
            &clock(position),
            time_style,
            Some(TIME_WIDTH),
            TextAlign::Center,
        ));
        self.remaining_text = self.duration().map(|duration| {
            let left = duration.saturating_sub(position);
            text.paragraph(
                &format!("-{}", clock(left)),
                time_style,
                Some(TIME_WIDTH),
                TextAlign::Center,
            )
        });
        self.hint_text = (!self.session.is_loaded()).then(|| {
            text.paragraph(
                &tr!("Open an audio or video file to play it."),
                TextStyle::role(TextRole::Body, palette.ink2),
                Some(frame.width - 2.0 * BAR_INSET),
                TextAlign::Center,
            )
        });
        let info = self.session.info();
        self.subtitle_text = self.session.subtitle().map(|subtitle| {
            text.paragraph(
                subtitle,
                TextStyle::role(TextRole::Heading, INK),
                Some(frame.width - 2.0 * SUBTITLE_MARGIN),
                TextAlign::Center,
            )
        });
        self.title_text = info.title.as_deref().map(|title| {
            text.paragraph(
                title,
                TextStyle::role(TextRole::Title, palette.ink),
                Some(frame.width - 2.0 * BAR_INSET),
                TextAlign::Center,
            )
        });
        self.artist_text = info.artist.as_deref().map(|artist| {
            text.paragraph(
                artist,
                TextStyle::role(TextRole::Body, palette.ink2),
                Some(frame.width - 2.0 * BAR_INSET),
                TextAlign::Center,
            )
        });
        let failure = self.session.failure().map(|kind| match kind {
            FailureKind::Unsupported => (
                text!("This file can't be played"),
                text!("Bande has no decoder for its format."),
            ),
            FailureKind::Unreadable => (
                text!("This file can't be opened"),
                text!("It may have been moved, or you may not have permission to read it."),
            ),
            FailureKind::Failed => (
                text!("Playback stopped"),
                text!("The player ran into a problem with this file."),
            ),
        });
        self.failure_text = failure.map_or_else(Vec::new, |(title, detail)| {
            vec![
                text.paragraph(
                    &title.tr(),
                    TextStyle::role(TextRole::Heading, palette.ink),
                    Some(frame.width - 2.0 * BAR_INSET),
                    TextAlign::Center,
                ),
                text.paragraph(
                    &detail.tr(),
                    TextStyle::role(TextRole::Body, palette.ink2),
                    Some(frame.width - 2.0 * BAR_INSET),
                    TextAlign::Center,
                ),
            ]
        });
    }

    fn paint_stack(&self, paragraphs: &[&Paragraph], top: f32, items: &mut Vec<Item>) {
        let mut y = top;
        for paragraph in paragraphs {
            items.push(Item::Text {
                origin: Point::new(self.layout.frame.x + BAR_INSET, y),
                paragraph: (*paragraph).clone(),
            });
            y += paragraph.height() + LINE_GAP;
        }
    }

    fn paint_audio(&self, palette: &Palette, items: &mut Vec<Item>) {
        let frame = self.layout.frame;
        let above_bar = frame.height - BAR_HEIGHT - 2.0 * BAR_INSET;
        let lines: Vec<&Paragraph> = [&self.title_text, &self.artist_text]
            .into_iter()
            .flatten()
            .collect();
        let lines_height: f32 = lines.iter().map(|line| line.height() + LINE_GAP).sum();
        let art = ART.min(above_bar - lines_height - BAR_INSET).max(0.0);
        let top = frame.y + (above_bar - art - lines_height - BAR_INSET).max(0.0) / 2.0;
        let cover = Rect::new(frame.x + (frame.width - art) / 2.0, top, art, art);
        match self.session.art() {
            Some(image) if art > 0.0 => {
                let (width, height) = (image.width() as f32, image.height() as f32);
                let scale = (art / width).max(art / height);
                items.push(Item::Clip {
                    shape: Shape::rounded(cover, Radius::M),
                    items: vec![Item::Image {
                        rect: Rect::new(
                            cover.x + (art - width * scale) / 2.0,
                            cover.y + (art - height * scale) / 2.0,
                            width * scale,
                            height * scale,
                        ),
                        image: image.clone(),
                    }],
                });
            }
            _ if art > 0.0 => items.push(Item::Fill {
                shape: Shape::rounded(cover, Radius::M),
                paint: palette.well.into(),
            }),
            _ => {}
        }
        self.paint_stack(&lines, top + art + BAR_INSET, items);
    }

    fn paint_bar(&self, now: Instant, items: &mut Vec<Item>) {
        let layout = &self.layout;
        items.push(Item::Fill {
            shape: Shape::rounded(layout.bar, Radius::L),
            paint: Color::BLACK.with_opacity(BAR_OPACITY).into(),
        });
        let center = Point::new(
            layout.play.x + layout.play.width / 2.0,
            layout.play.y + layout.play.height / 2.0,
        );
        match self.session.is_playing() {
            true => {
                for x in [center.x - 5.0, center.x + 1.5] {
                    items.push(Item::Fill {
                        shape: Shape::rounded(Rect::new(x, center.y - 7.5, 3.5, 15.0), 1.0),
                        paint: INK.into(),
                    });
                }
            }
            false => items.push(Item::FillPath {
                path: triangle(center),
                paint: INK.into(),
            }),
        }
        let text = |rect: Rect, paragraph: &Paragraph| Item::Text {
            origin: Point::new(rect.x, rect.y + (rect.height - paragraph.height()) / 2.0),
            paragraph: paragraph.clone(),
        };
        if let Some(elapsed) = &self.elapsed_text {
            items.push(text(layout.elapsed, elapsed));
        }
        if let Some(remaining) = &self.remaining_text {
            items.push(text(layout.remaining, remaining));
        }
        let meter = |rect: Rect, fraction: f32, items: &mut Vec<Item>| {
            let line = grown(rect, TRACK);
            items.push(Item::Fill {
                shape: Shape::rounded(line, TRACK / 2.0),
                paint: INK.with_opacity(DIM_OPACITY).into(),
            });
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(line.x, line.y, line.width * fraction, TRACK),
                    TRACK / 2.0,
                ),
                paint: INK.into(),
            });
        };
        if self.duration().is_some() {
            meter(layout.track, self.fraction(now), items);
        }
        if layout.wide {
            let center = Point::new(
                layout.mute.x + layout.mute.width / 2.0,
                layout.mute.y + layout.mute.height / 2.0,
            );
            items.extend(speaker(center, self.session.is_muted()));
            let level = match self.session.is_muted() {
                true => 0.0,
                false => self.session.volume() as f32,
            };
            meter(layout.volume, level, items);
        }
    }

    pub fn paint(&self, palette: &Palette, now: Instant, items: &mut Vec<Item>) {
        let frame = self.layout.frame;
        if !self.session.is_loaded() && self.failure_text.is_empty() {
            if let Some(hint) = &self.hint_text {
                let top = frame.y + (frame.height - hint.height()) / 2.0;
                self.paint_stack(&[hint], top, items);
            }
            return;
        }
        if !self.failure_text.is_empty() {
            let height: f32 = self
                .failure_text
                .iter()
                .map(|line| line.height() + LINE_GAP)
                .sum();
            let lines: Vec<&Paragraph> = self.failure_text.iter().collect();
            self.paint_stack(&lines, frame.y + (frame.height - height) / 2.0, items);
            return;
        }
        match self.session.has_video() {
            true => {
                items.push(Item::Fill {
                    shape: Shape::rect(frame),
                    paint: Color::BLACK.into(),
                });
                if let Some((id, size)) = self.session.surface() {
                    items.push(Item::Surface {
                        id,
                        frame,
                        size,
                        scaling: SurfaceScaling::Fit,
                    });
                }
            }
            false => {
                if self.session.playback() != Playback::Opening {
                    self.paint_audio(palette, items);
                }
            }
        }
        if let Some(subtitle) = &self.subtitle_text {
            let lifted = match self.controls_visible {
                true => BAR_HEIGHT + 2.0 * BAR_INSET,
                false => SUBTITLE_MARGIN,
            };
            let origin = Point::new(
                frame.x + SUBTITLE_MARGIN,
                frame.y + frame.height - lifted - subtitle.height(),
            );
            let width = subtitle.width() + 2.0 * SUBTITLE_PADDING;
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        frame.x + (frame.width - width) / 2.0,
                        origin.y - SUBTITLE_PADDING,
                        width,
                        subtitle.height() + 2.0 * SUBTITLE_PADDING,
                    ),
                    Radius::S,
                ),
                paint: Color::BLACK.with_opacity(BAR_OPACITY).into(),
            });
            items.push(Item::Text {
                origin,
                paragraph: subtitle.clone(),
            });
        }
        if self.controls_visible {
            self.paint_bar(now, items);
        }
    }

    fn scrub_to(&mut self, fraction: f32, exact: bool, now: Instant) {
        self.press = Some(Press::Scrub(fraction));
        let Some(duration) = self.duration() else {
            return;
        };
        let elapsed = self
            .last_scrub
            .is_none_or(|last| now.duration_since(last) >= SCRUB_INTERVAL);
        if exact || elapsed {
            self.last_scrub = Some(now);
            self.session.seek(duration.mul_f32(fraction), exact);
        }
    }

    pub fn pointer_down(&mut self, point: Point, now: Instant) -> bool {
        let shown = self.reveal_controls(now);
        let layout = &self.layout;
        let track_hit = grown(layout.track, TRACK_HIT);
        let (play, mute, volume, bar) = (layout.play, layout.mute, layout.volume, layout.bar);
        if !bar.contains(point) {
            return shown;
        }
        if play.contains(point) {
            self.press = Some(Press::Play);
        } else if track_hit.contains(point) && self.session.info().seekable {
            self.last_scrub = None;
            self.scrub_to(fraction_in(track_hit, point.x), false, now);
        } else if layout.wide && volume.contains(point) {
            self.press = Some(Press::Volume);
            self.session
                .set_volume(f64::from(fraction_in(volume, point.x)));
        } else if layout.wide && mute.contains(point) {
            self.press = Some(Press::Mute);
        }
        self.settle(now);
        true
    }

    pub fn pointer_moved(&mut self, point: Point, now: Instant) -> bool {
        let track = self.layout.track;
        let volume = self.layout.volume;
        let changed = match self.press {
            Some(Press::Scrub(_)) => {
                self.scrub_to(fraction_in(track, point.x), false, now);
                true
            }
            Some(Press::Volume) => {
                self.session
                    .set_volume(f64::from(fraction_in(volume, point.x)));
                true
            }
            _ => false,
        };
        self.reveal_controls(now) || changed
    }

    pub fn pointer_up(&mut self, point: Point, now: Instant) -> bool {
        let Some(press) = self.press.take() else {
            return false;
        };
        match press {
            Press::Play if self.layout.play.contains(point) => self.session.toggle(),
            Press::Mute if self.layout.mute.contains(point) => self.session.toggle_mute(),
            Press::Scrub(_) => {
                if let Some(duration) = self.duration() {
                    let fraction = fraction_in(self.layout.track, point.x);
                    self.session.seek(duration.mul_f32(fraction), true);
                }
            }
            _ => {}
        }
        self.settle(now);
        true
    }

    pub fn pointer_left(&mut self, now: Instant) -> bool {
        if self.fades() {
            self.hide_at = Some(now);
        }
        false
    }

    pub fn is_pressed(&self) -> bool {
        self.press.is_some()
    }

    pub fn in_bar(&self, point: Point) -> bool {
        self.controls_visible && self.layout.bar.contains(point)
    }

    pub fn key(&mut self, key: &Key, modifiers: &Modifiers, now: Instant) -> bool {
        if modifiers.control || modifiers.alt || modifiers.logo {
            return false;
        }
        let position = self.session.position(now);
        match key {
            Key::Space => self.session.toggle(),
            Key::Character('m' | 'M') => self.session.toggle_mute(),
            Key::Left if self.session.is_playing() => {
                self.session.seek(position.saturating_sub(SEEK_STEP), true);
            }
            Key::Right if self.session.is_playing() => {
                self.session.seek(position + SEEK_STEP, true);
            }
            Key::Left => self.session.step(false),
            Key::Right => self.session.step(true),
            Key::Up => self.session.set_volume(self.session.volume() + VOLUME_STEP),
            Key::Down => self.session.set_volume(self.session.volume() - VOLUME_STEP),
            _ => return false,
        }
        self.reveal_controls(now);
        true
    }
}

pub struct StageWidget {
    id: WidgetId,
    stage: Stage,
    frame: Rect,
}

impl Default for StageWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl StageWidget {
    pub fn new() -> Self {
        Self {
            id: WidgetId::next(),
            stage: Stage::new(Session::new()),
            frame: Rect::default(),
        }
    }

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn stage_mut(&mut self) -> &mut Stage {
        &mut self.stage
    }

    fn part_bounds(rect: Rect) -> accesskit::Rect {
        accesskit::Rect {
            x0: f64::from(rect.x),
            y0: f64::from(rect.y),
            x1: f64::from(rect.x + rect.width),
            y1: f64::from(rect.y + rect.height),
        }
    }
}

impl Widget for StageWidget {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let palette = cx.palette;
        let now = cx.now();
        self.stage.layout(&mut cx.text, &palette, frame, now);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.stage.paint(&cx.palette, cx.now(), items);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Video);
        node.set_label(tr!("Playback controls"));
        node
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let now = cx.now();
        match event {
            Event::PointerDown(pointer) => self.stage.pointer_down(pointer.position, now),
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                self.stage.pointer_moved(pointer.position, now)
            }
            Event::PointerUp(pointer) => self.stage.pointer_up(pointer.position, now),
            Event::PointerLeave => self.stage.pointer_left(now),
            Event::Key { key, modifiers } => self.stage.key(key, modifiers, now),
            _ => false,
        }
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let layout = &self.stage.layout;
        let session = self.stage.session();
        let mut play = Node::new(Role::Button);
        play.set_label(
            (match session.is_playing() {
                true => text!("Pause"),
                false => text!("Play"),
            })
            .tr(),
        );
        play.add_action(Action::Click);
        play.set_bounds(Self::part_bounds(layout.play));
        let mut timeline = Node::new(Role::Slider);
        timeline.set_label(tr!("Timeline"));
        timeline.set_min_numeric_value(0.0);
        timeline.set_max_numeric_value(
            session
                .info()
                .duration_ms
                .map_or(0.0, |ms| ms as f64 / 1000.0),
        );
        timeline.set_numeric_value(session.position(Instant::now()).as_secs_f64());
        timeline.add_action(Action::SetValue);
        timeline.set_bounds(Self::part_bounds(layout.track));
        let mut volume = Node::new(Role::Slider);
        volume.set_label(tr!("Volume"));
        volume.set_min_numeric_value(0.0);
        volume.set_max_numeric_value(1.0);
        volume.set_numeric_value(session.volume());
        volume.add_action(Action::SetValue);
        volume.set_bounds(Self::part_bounds(layout.volume));
        let nodes = vec![
            (self.id.part(0), play),
            (self.id.part(1), timeline),
            (self.id.part(2), volume),
        ];
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: None,
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        let value = match &request.data {
            Some(ActionData::NumericValue(value)) => Some(*value),
            _ => None,
        };
        let session = self.stage.session_mut();
        match (part, request.action, value) {
            (0, Action::Click, _) => session.toggle(),
            (1, Action::SetValue, Some(seconds)) => {
                session.seek(Duration::from_secs_f64(seconds.max(0.0)), true);
            }
            (2, Action::SetValue, Some(volume)) => session.set_volume(volume),
            _ => return false,
        }
        true
    }
}
