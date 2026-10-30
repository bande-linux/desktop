use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_fs::Trash;
use unbound_screenshot::L10N;
use unbound_screenshot::markup::{Command, Commanded, Markup, TOOLBAR_HEIGHT, Toolbar};
use unbound_toolkit::ui::{Context, Event, FreeLayout, Key, Size, Ui, Widget, WidgetId};
use unbound_toolkit::{
    App, Elevation, Image, Item, Palette, Point, Rect, Role, Scene, Scheme, ScreenImage, Shape,
    TextSystem, Transfer, Window, WindowError, WindowEvent,
};
use unbound_xdg::file_uri;

const MOST_WIDTH: f32 = 1100.0;
const MOST_HEIGHT: f32 = 720.0;
const LEAST_WIDTH: f32 = 620.0;
const MARGIN: f32 = 16.0;
const CLIPBOARD_GRACE: Duration = Duration::from_millis(600);
const APP_ID: &str = "org.unbound.Screenshot";

struct MarkupApp {
    layout: WidgetId,
    toolbar: WidgetId,
    path: PathBuf,
    image: Image,
    markup: Markup,
    ui: Ui,
    commanded: Commanded,
    text: TextSystem,
    dragging: bool,
    closing_at: Option<Instant>,
}

impl MarkupApp {
    fn canvas(&self, window: &Window) -> Rect {
        let (width, height) = window.size();
        let area = Rect::new(
            MARGIN,
            TOOLBAR_HEIGHT + MARGIN,
            width as f32 - 2.0 * MARGIN,
            height as f32 - TOOLBAR_HEIGHT - 2.0 * MARGIN,
        );
        let scale = (area.width / self.markup.width)
            .min(area.height / self.markup.height)
            .min(1.0);
        let (shown_width, shown_height) = (self.markup.width * scale, self.markup.height * scale);
        Rect::new(
            area.x + (area.width - shown_width) / 2.0,
            area.y + (area.height - shown_height) / 2.0,
            shown_width,
            shown_height,
        )
    }

    fn to_image(&self, window: &Window, point: Point) -> Point {
        let canvas = self.canvas(window);
        let factor = self.markup.width / canvas.width.max(1.0);
        Point::new((point.x - canvas.x) * factor, (point.y - canvas.y) * factor)
    }

    fn rendered(&mut self) -> Option<ScreenImage> {
        self.markup.commit_text();
        let snapshot = self.markup.export(&self.image, &mut self.text)?;
        Some(ScreenImage {
            width: snapshot.width,
            height: snapshot.height,
            rgba: snapshot.rgba,
        })
    }

    fn done(&mut self, window: &mut Window) {
        self.markup.commit_text();
        if !self.markup.edits().is_empty() {
            let saved = self
                .rendered()
                .ok_or_else(|| std::io::Error::other("the picture cannot be drawn"))
                .and_then(|image| unbound_screenshot::png(&image))
                .and_then(|bytes| std::fs::write(&self.path, bytes));
            if let Err(error) = saved {
                tracing::warn!("cannot save {}: {error}", self.path.display());
            }
        }
        window.close();
    }

    fn copy(&mut self, window: &mut Window) {
        let Some(image) = self.rendered() else {
            return;
        };
        match unbound_screenshot::png(&image) {
            Ok(bytes) => {
                window.copy_transfer(Transfer::uris(vec![file_uri(&self.path)]).with_png(bytes));
                self.closing_at = Some(Instant::now() + CLIPBOARD_GRACE);
                if let Err(error) = window.hide() {
                    tracing::warn!("cannot hide markup: {error}");
                }
            }
            Err(error) => tracing::warn!("cannot copy the picture: {error}"),
        }
    }

    fn delete(&mut self, window: &mut Window) {
        let trashed = Trash::for_user()
            .ok_or_else(|| std::io::Error::other("no trash"))
            .and_then(|trash| trash.put(&self.path));
        if let Err(error) = trashed {
            tracing::warn!("cannot move {} to the Trash: {error}", self.path.display());
        }
        window.close();
    }

    fn run_commands(&mut self, window: &mut Window) {
        let commands: Vec<Command> = self.commanded.borrow_mut().drain(..).collect();
        for command in commands {
            match command {
                Command::Tool(tool) => self.markup.tool = tool,
                Command::Undo => {
                    self.markup.undo();
                }
                Command::Delete => self.delete(window),
                Command::Copy => self.copy(window),
                Command::Done => self.done(window),
            }
        }
        window.request_redraw();
    }

    fn canvas_event(&mut self, window: &mut Window, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer)
                if pointer.is_primary() && pointer.position.y > TOOLBAR_HEIGHT =>
            {
                let at = self.to_image(window, pointer.position);
                self.markup.press(at);
                self.dragging = true;
                true
            }
            Event::PointerMove(pointer) if self.dragging => {
                let at = self.to_image(window, pointer.position);
                self.markup.drag_to(at);
                true
            }
            Event::PointerUp(_) if self.dragging => {
                self.dragging = false;
                self.markup.release();
                true
            }
            Event::Commit(text) if self.markup.is_typing() => {
                self.markup.type_text(text);
                true
            }
            Event::Key {
                key: Key::Character('z' | 'Z'),
                modifiers,
            } if modifiers.logo => {
                self.markup.undo();
                true
            }
            Event::Key {
                key: Key::Character('c' | 'C'),
                modifiers,
            } if modifiers.logo && !self.markup.is_typing() => {
                self.copy(window);
                true
            }
            Event::Key { key, modifiers } if self.markup.is_typing() && !modifiers.logo => {
                self.markup.key(*key)
            }
            Event::Key {
                key: Key::Enter | Key::Escape,
                ..
            } => {
                self.done(window);
                true
            }
            Event::Key {
                key: Key::Backspace | Key::Delete,
                modifiers,
            } if modifiers.logo => {
                self.delete(window);
                true
            }
            _ => false,
        }
    }
}

impl App for MarkupApp {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        if let Some(layout) = self.ui.widget_as::<FreeLayout>(self.layout) {
            layout.set_frame(
                self.toolbar,
                Rect::new(0.0, 0.0, width as f32, TOOLBAR_HEIGHT),
            );
        }
        self.ui.layout(Size::new(width as f32, height as f32));
        let mut scene = self.ui.scene(width, height);
        let canvas = self.canvas(window);
        let factor = canvas.width / self.markup.width.max(1.0);
        let crop = self.markup.crop();
        let mut marks = vec![Item::Image {
            rect: Rect::new(0.0, 0.0, self.markup.width, self.markup.height),
            image: self.image.clone(),
        }];
        marks.extend(self.markup.items(&mut self.text, true));
        let full = Rect::new(0.0, 0.0, self.markup.width, self.markup.height);
        if crop != full {
            let veil = Palette::for_scheme(Scheme::Dark).scrim;
            for outside in [
                Rect::new(0.0, 0.0, full.width, crop.y),
                Rect::new(
                    0.0,
                    crop.y + crop.height,
                    full.width,
                    full.height - crop.y - crop.height,
                ),
                Rect::new(0.0, crop.y, crop.x, crop.height),
                Rect::new(
                    crop.x + crop.width,
                    crop.y,
                    full.width - crop.x - crop.width,
                    crop.height,
                ),
            ] {
                marks.push(Item::Fill {
                    shape: Shape::rect(outside),
                    paint: veil.into(),
                });
            }
        }
        scene.items.extend(Item::shadows(
            Shape::rect(canvas),
            Elevation::Overlay,
            self.ui.context().palette.scheme,
            true,
        ));
        scene.items.push(Item::Translate {
            offset: Point::new(canvas.x, canvas.y),
            items: vec![Item::Clip {
                shape: Shape::rect(Rect::new(0.0, 0.0, canvas.width, canvas.height)),
                items: vec![Item::Scale {
                    origin: Point::new(0.0, 0.0),
                    factor,
                    items: marks,
                }],
            }],
        });
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                let on_toolbar = match &input {
                    Event::PointerDown(pointer) | Event::PointerMove(pointer) => {
                        pointer.position.y <= TOOLBAR_HEIGHT && !self.dragging
                    }
                    Event::Key { key: Key::Tab, .. } => true,
                    _ => false,
                };
                let handled = match on_toolbar {
                    true => self.ui.handle(&input),
                    false => {
                        self.canvas_event(window, &input)
                            || (!self.markup.is_typing() && self.ui.handle(&input))
                    }
                };
                if handled {
                    window.request_redraw();
                }
                self.run_commands(window);
            }
            WindowEvent::Accessibility(request) => {
                self.ui.perform(&request);
                self.run_commands(window);
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.closing_at
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.closing_at.is_some_and(|at| at <= now) {
            window.close();
        }
    }

    fn should_close(&mut self, window: &mut Window) -> bool {
        self.done(window);
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

pub fn run(path: &Path) -> Result<(), WindowError> {
    let decoded = image::open(path)
        .map_err(|error| WindowError::Io(std::io::Error::other(error)))?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    let image = Image::from_rgba(width, height, decoded.into_raw())
        .ok_or_else(|| WindowError::Io(std::io::Error::other("the picture is too large")))?;
    let scale = (MOST_WIDTH / width as f32)
        .min((MOST_HEIGHT - TOOLBAR_HEIGHT) / height as f32)
        .min(1.0);
    let size = (
        (width as f32 * scale + 2.0 * MARGIN)
            .max(LEAST_WIDTH)
            .round() as u32,
        (height as f32 * scale + TOOLBAR_HEIGHT + 2.0 * MARGIN).round() as u32,
    );
    let title = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| L10N.tr("markup"));
    let mut window = Window::open(Role::toplevel(&title, APP_ID), size)?;
    let appearance = window.appearance();
    let markup = Markup::new(width, height);
    let (toolbar, commanded) = Toolbar::new(markup.tool);
    let mut layout = FreeLayout::new();
    let layout_id = layout.id();
    let toolbar = layout.insert(toolbar, Rect::new(0.0, 0.0, size.0 as f32, TOOLBAR_HEIGHT));
    let mut ui = Ui::new(
        Box::new(layout),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    let mut app = MarkupApp {
        layout: layout_id,
        toolbar,
        path: path.to_path_buf(),
        image,
        markup,
        ui,
        commanded,
        text: TextSystem::new(),
        dragging: false,
        closing_at: None,
    };
    unbound_toolkit::run(&mut window, &mut app)
}
