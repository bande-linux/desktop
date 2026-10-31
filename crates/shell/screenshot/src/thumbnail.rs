use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unbound_toolkit::ui::{Animated, Event, Menu, MenuItem, Spring};
use unbound_toolkit::{
    Color, Elevation, Image, Item, Palette, Point, Radius, Rect, Scene, Scheme, Shape,
};

use crate::L10N;

const MOST_WIDTH: f32 = 192.0;
const MOST_HEIGHT: f32 = 128.0;
const MARGIN: f32 = 18.0;
const BORDER: f32 = 3.0;
const RADIUS: f32 = Radius::S;
const SHOWN_FOR: Duration = Duration::from_secs(5);
const LINGER: Duration = Duration::from_secs(2);
const DRAG_THRESHOLD: f32 = 4.0;
const SWIPE_AWAY: f32 = 40.0;
const PLAY_RADIUS: f32 = 16.0;
const PLAY_DISC: f32 = 0.7;

#[derive(Debug, Clone, PartialEq)]
pub enum ThumbnailAction {
    None,
    Open(PathBuf),
    Drag(PathBuf),
    Menu(Point),
    Gone,
}

pub const SHOW_IN_FILES: &str = "thumbnail-show-in-files";
pub const OPEN: &str = "thumbnail-open";
pub const COPY: &str = "thumbnail-copy";
pub const TRASH: &str = "thumbnail-trash";
pub const CLOSE: &str = "thumbnail-close";

pub struct Thumbnail {
    image: Image,
    path: PathBuf,
    video: bool,
    palette: Palette,
    reduce_motion: bool,
    shown: Animated,
    until: Instant,
    hovered: bool,
    pressed: Option<Point>,
    swipe: Animated,
    swiping: bool,
    menu_open: bool,
    leaving: bool,
}

impl Thumbnail {
    pub fn new(
        image: Image,
        path: PathBuf,
        video: bool,
        palette: Palette,
        reduce_motion: bool,
        now: Instant,
    ) -> Self {
        let mut shown = Animated::new(0.0);
        let spring = match reduce_motion {
            true => Spring::FADE,
            false => Spring::STANDARD,
        };
        shown.animate_at(now, false, 1.0, spring);
        Self {
            image,
            path,
            video,
            palette,
            reduce_motion,
            shown,
            until: now + SHOWN_FOR,
            hovered: false,
            pressed: None,
            swipe: Animated::points(0.0),
            swiping: false,
            menu_open: false,
            leaving: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_video(&self) -> bool {
        self.video
    }

    fn held(&self) -> bool {
        self.hovered || self.pressed.is_some() || self.menu_open
    }

    pub fn menu(&self) -> Menu {
        let open = match self.video {
            true => "thumbnail-play",
            false => "thumbnail-markup",
        };
        let mut items = vec![
            MenuItem::new(&L10N.tr(open), OPEN),
            MenuItem::new(&L10N.tr("thumbnail-show-in-files"), SHOW_IN_FILES),
        ];
        if !self.video {
            items.push(MenuItem::new(&L10N.tr("copy"), COPY));
        }
        items.extend([
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("thumbnail-trash"), TRASH),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("close"), CLOSE),
        ]);
        Menu::new(items)
    }

    pub fn menu_closed(&mut self, now: Instant) {
        if std::mem::take(&mut self.menu_open) {
            self.until = self.until.max(now + LINGER);
        }
    }

    fn size(&self) -> (f32, f32) {
        let (width, height) = (self.image.width() as f32, self.image.height() as f32);
        let scale = (MOST_WIDTH / width).min(MOST_HEIGHT / height);
        (width * scale, height * scale)
    }

    pub fn rest(&self, screen: (u32, u32)) -> Rect {
        let (width, height) = self.size();
        Rect::new(
            screen.0 as f32 - MARGIN - width - BORDER,
            screen.1 as f32 - MARGIN - height - BORDER,
            width + 2.0 * BORDER,
            height + 2.0 * BORDER,
        )
    }

    fn progress(&self, now: Instant) -> (f32, bool) {
        self.shown.value_at(now)
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.progress(now).1 || self.swipe.value_at(now).1
    }

    pub fn is_leaving(&self) -> bool {
        self.leaving
    }

    pub fn deadline(&self) -> Option<Instant> {
        (!self.leaving && !self.held()).then_some(self.until)
    }

    pub fn tick(&mut self, now: Instant) -> ThumbnailAction {
        if !self.leaving && !self.held() && now >= self.until {
            self.leave(now);
        }
        match self.leaving && !self.is_animating(now) {
            true => ThumbnailAction::Gone,
            false => ThumbnailAction::None,
        }
    }

    fn leave(&mut self, now: Instant) {
        self.leaving = true;
        let spring = match self.reduce_motion {
            true => Spring::FADE,
            false => Spring::STANDARD,
        };
        self.shown.animate_at(now, false, 0.0, spring);
    }

    fn contains(&self, screen: (u32, u32), point: Point) -> bool {
        let rest = self.rest(screen);
        point.x >= rest.x
            && point.y >= rest.y
            && point.x < rest.x + rest.width
            && point.y < rest.y + rest.height
    }

    pub fn handle(&mut self, screen: (u32, u32), event: &Event, now: Instant) -> ThumbnailAction {
        match event {
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                self.hovered = self.contains(screen, pointer.position);
                if let Some(from) = self.pressed {
                    let (across, down) = (pointer.position.x - from.x, pointer.position.y - from.y);
                    if self.swiping || (across > DRAG_THRESHOLD && across > 2.0 * down.abs()) {
                        self.swiping = true;
                        self.swipe.jump_to(across.max(0.0));
                        return ThumbnailAction::None;
                    }
                    if across.hypot(down) > DRAG_THRESHOLD {
                        self.pressed = None;
                        return ThumbnailAction::Drag(self.path.clone());
                    }
                }
                ThumbnailAction::None
            }
            Event::PointerLeave => {
                self.hovered = false;
                self.until = self.until.max(now + LINGER);
                ThumbnailAction::None
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.pressed = self
                    .contains(screen, pointer.position)
                    .then_some(pointer.position);
                ThumbnailAction::None
            }
            Event::PointerDown(pointer) if self.contains(screen, pointer.position) => {
                self.pressed = None;
                self.menu_open = true;
                ThumbnailAction::Menu(pointer.position)
            }
            Event::PointerUp(_) if std::mem::take(&mut self.swiping) => {
                self.pressed = None;
                match self.swipe.target() >= SWIPE_AWAY {
                    true => self.dismiss(now),
                    false => {
                        self.swipe
                            .animate_at(now, self.reduce_motion, 0.0, Spring::STANDARD);
                    }
                }
                ThumbnailAction::None
            }
            Event::PointerUp(_) if self.pressed.take().is_some() => {
                ThumbnailAction::Open(self.path.clone())
            }
            _ => ThumbnailAction::None,
        }
    }

    pub fn dismiss(&mut self, now: Instant) {
        if !self.leaving {
            self.leave(now);
        }
    }

    pub fn icon(&self) -> Scene {
        let (width, height) = self.size();
        let frame = Rect::new(0.0, 0.0, width + 2.0 * BORDER, height + 2.0 * BORDER);
        Scene {
            width: frame.width.ceil() as u32,
            height: frame.height.ceil() as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Opacity {
                alpha: 0.85,
                items: self.card(frame),
            }],
        }
    }

    fn card(&self, frame: Rect) -> Vec<Item> {
        let picture = frame.inset(BORDER);
        let shape = Shape::rounded(frame, RADIUS);
        let mut items =
            Item::shadows(shape, Elevation::Overlay, self.palette.scheme, true).to_vec();
        items.extend([
            Item::Fill {
                shape,
                paint: self.palette.overlay.into(),
            },
            Item::Clip {
                shape: Shape::rounded(picture, RADIUS - BORDER / 2.0),
                items: vec![Item::Image {
                    rect: picture,
                    image: self.image.clone(),
                }],
            },
        ]);
        if self.video {
            let night = Palette::for_scheme(Scheme::Dark);
            let center = Point::new(
                picture.x + picture.width / 2.0,
                picture.y + picture.height / 2.0,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        center.x - PLAY_RADIUS,
                        center.y - PLAY_RADIUS,
                        PLAY_RADIUS * 2.0,
                        PLAY_RADIUS * 2.0,
                    ),
                    PLAY_RADIUS,
                ),
                paint: night.chassis.with_opacity(PLAY_DISC).into(),
            });
            let path = unbound_graphics::Path::new()
                .move_to(Point::new(center.x - 5.0, center.y - 8.0))
                .line_to(Point::new(center.x + 9.0, center.y))
                .line_to(Point::new(center.x - 5.0, center.y + 8.0))
                .close();
            items.push(Item::FillPath {
                path,
                paint: night.ink.into(),
            });
        }
        items
    }

    pub fn scene(&self, screen: (u32, u32), now: Instant) -> Scene {
        let rest = self.rest(screen);
        let (progress, _) = self.progress(now);
        let (swiped, _) = self.swipe.value_at(now);
        let items = match self.reduce_motion {
            true => vec![Item::Opacity {
                alpha: progress.clamp(0.0, 1.0),
                items: vec![Item::Translate {
                    offset: Point::new(swiped, 0.0),
                    items: self.card(rest),
                }],
            }],
            false => vec![Item::Translate {
                offset: Point::new(
                    swiped + (1.0 - progress) * (rest.width + MARGIN + 24.0),
                    0.0,
                ),
                items: self.card(rest),
            }],
        };
        Scene {
            width: screen.0,
            height: screen.1,
            background: Color::TRANSPARENT,
            items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_toolkit::ui::{Pointer, PointerButton};

    fn thumbnail(now: Instant) -> Thumbnail {
        let image = Image::from_rgba(400, 200, vec![0xff; 400 * 200 * 4]).expect("image");
        Thumbnail::new(
            image,
            PathBuf::from("/tmp/shot.png"),
            false,
            Palette::for_scheme(Scheme::Light),
            false,
            now,
        )
    }

    const SCREEN: (u32, u32) = (1280, 800);

    #[test]
    fn it_rests_in_the_bottom_right_corner_keeping_the_picture_shape() {
        let rest = thumbnail(Instant::now()).rest(SCREEN);

        assert_eq!(rest.width, MOST_WIDTH + 2.0 * BORDER);
        assert_eq!(rest.height, 96.0 + 2.0 * BORDER);
        assert_eq!(rest.x + rest.width, 1280.0 - MARGIN + BORDER);
    }

    #[test]
    fn a_click_opens_and_a_drag_carries_the_file() {
        let now = Instant::now();
        let mut clicked = thumbnail(now);
        let inside = Point::new(1200.0, 740.0);
        clicked.handle(SCREEN, &Event::PointerDown(Pointer::at(inside)), now);
        let opened = clicked.handle(SCREEN, &Event::PointerUp(Pointer::at(inside)), now);
        let mut dragged = thumbnail(now);
        dragged.handle(SCREEN, &Event::PointerDown(Pointer::at(inside)), now);
        let moved = Point::new(1180.0, 730.0);
        let carried = dragged.handle(SCREEN, &Event::PointerMove(Pointer::at(moved)), now);

        assert_eq!(
            opened,
            ThumbnailAction::Open(PathBuf::from("/tmp/shot.png"))
        );
        assert_eq!(
            carried,
            ThumbnailAction::Drag(PathBuf::from("/tmp/shot.png"))
        );
    }

    #[test]
    fn a_secondary_click_asks_for_its_menu_and_keeps_it_until_the_menu_closes() {
        let now = Instant::now();
        let mut shown = thumbnail(now);
        let inside = Point::new(1200.0, 740.0);
        let outside = Point::new(400.0, 300.0);
        let secondary =
            |at| Event::PointerDown(Pointer::at(at).with_button(PointerButton::Secondary));

        let missed = shown.handle(SCREEN, &secondary(outside), now);
        let asked = shown.handle(SCREEN, &secondary(inside), now);
        shown.handle(SCREEN, &Event::PointerLeave, now);
        let later = now + SHOWN_FOR + Duration::from_secs(5);

        assert_eq!(missed, ThumbnailAction::None);
        assert_eq!(asked, ThumbnailAction::Menu(inside));
        assert_eq!(shown.tick(later), ThumbnailAction::None);
        shown.menu_closed(later);
        assert_eq!(shown.tick(later), ThumbnailAction::None);
        shown.tick(later + LINGER);
        assert_eq!(
            shown.tick(later + LINGER + Duration::from_secs(2)),
            ThumbnailAction::Gone
        );
    }

    #[test]
    fn a_swipe_to_the_right_sends_it_away_and_a_short_one_brings_it_back() {
        let now = Instant::now();
        let inside = Point::new(1200.0, 740.0);
        let swipe = |by: f32| {
            let mut shown = thumbnail(now);
            shown.handle(SCREEN, &Event::PointerDown(Pointer::at(inside)), now);
            let to = Point::new(inside.x + by, inside.y + 3.0);
            let moved = shown.handle(SCREEN, &Event::PointerMove(Pointer::at(to)), now);
            let released = shown.handle(SCREEN, &Event::PointerUp(Pointer::at(to)), now);
            assert_eq!(
                (moved, released),
                (ThumbnailAction::None, ThumbnailAction::None)
            );
            shown
        };
        let mut away = swipe(60.0);
        let mut back = swipe(12.0);
        let settled = now + Duration::from_secs(2);

        assert_eq!(away.tick(settled), ThumbnailAction::Gone);
        assert_eq!(back.tick(settled), ThumbnailAction::None);
        assert!(!back.is_leaving());
        assert_eq!(back.swipe.value_at(settled), (0.0, false));
    }

    #[test]
    fn it_leaves_after_a_while_unless_the_pointer_rests_on_it() {
        let now = Instant::now();
        let mut idle = thumbnail(now);
        let mut hovered = thumbnail(now);
        hovered.handle(
            SCREEN,
            &Event::PointerMove(Pointer::at(Point::new(1200.0, 740.0))),
            now,
        );
        let later = now + SHOWN_FOR + Duration::from_secs(5);

        idle.tick(now + SHOWN_FOR);

        assert_eq!(idle.tick(later), ThumbnailAction::Gone);
        assert_eq!(hovered.tick(later), ThumbnailAction::None);
    }
}
