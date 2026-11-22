use std::cell::Cell;
use std::io;
use std::path::Path;
use std::rc::Rc;

use accesskit::{ActionRequest, Node};
use bande_app_settings::{Shared, Stored, flag, pairs, settings_folder};
use bande_l10n::{Text, text, tr};
use bande_toolkit::ui::{
    Button, Context, Event, Form, FormLine, ScrollView, Size, Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Item, Rect, TextSystem};

use crate::diagnostics::diagnostics;

pub const SETTINGS_SIZE: (u32, u32) = (560, 440);
const DESKTOP_ENTRY: &str = "org.bande.Velox.desktop";
const LOGIN_ENTRY: &str = "[Desktop Entry]
Type=Application
Name=Velox
Exec=bande-velox --background
TryExec=bande-velox
Icon=org.bande.Velox
OnlyShowIn=X-Bande;
";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VeloxSettings {
    pub status_item: bool,
}

impl VeloxSettings {
    pub fn current() -> Self {
        settings_folder()
            .as_deref()
            .map(Self::load)
            .unwrap_or_default()
    }
}

impl Stored for VeloxSettings {
    const FILE: &'static str = "velox.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            if key == "status-item" {
                settings.status_item = flag(value).unwrap_or_default();
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        format!("status-item = {}\n", self.status_item)
    }
}

pub fn launches_at_login() -> bool {
    bande_xdg::autostart_dir().is_some_and(|dir| bande_xdg::opens_at_login(&dir, DESKTOP_ENTRY))
}

fn set_launch_at_login(on: bool) -> io::Result<()> {
    let dir = bande_xdg::autostart_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home folder"))?;
    if !on {
        return bande_xdg::set_open_at_login(&dir, DESKTOP_ENTRY, Path::new(""), false);
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(DESKTOP_ENTRY), LOGIN_ENTRY)
}

fn login_toggle() -> Box<dyn Widget> {
    Box::new(Toggle::new(
        &tr!("Launch Velox at login"),
        launches_at_login(),
        |on| {
            if let Err(error) = set_launch_at_login(on) {
                tracing::warn!("cannot change launch at login: {error}");
            }
        },
    ))
}

struct CopyDiagnostics {
    button: Button,
    clicked: Rc<Cell<bool>>,
}

impl CopyDiagnostics {
    fn new() -> Self {
        let clicked = Rc::new(Cell::new(false));
        let click = Rc::clone(&clicked);
        Self {
            button: Button::new(&tr!("Copy Diagnostics"), move || click.set(true)).tooltip(&tr!(
                "Copies versions, the engine's state and the end of its log, for a bug report"
            )),
            clicked,
        }
    }

    fn copy_if_clicked(&self, cx: &mut Context) {
        if self.clicked.replace(false) {
            cx.copy(&diagnostics());
        }
    }
}

impl Widget for CopyDiagnostics {
    fn id(&self) -> WidgetId {
        self.button.id()
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.button.measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.button.place(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.button.frame()
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.button.paint(cx, items);
    }

    fn access(&self, cx: &Context) -> Node {
        self.button.access(cx)
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let handled = self.button.handle(cx, event);
        self.copy_if_clicked(cx);
        handled
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        let handled = self.button.perform(cx, request);
        self.copy_if_clicked(cx);
        handled
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        self.button.accepts_focus(cx)
    }

    fn tooltip(&self) -> Option<&str> {
        Widget::tooltip(&self.button)
    }
}

fn toggle(
    settings: &Shared<VeloxSettings>,
    label: Text,
    field: fn(&mut VeloxSettings) -> &mut bool,
) -> Box<dyn Widget> {
    let shared = settings.clone();
    let on = *field(&mut settings.borrow_mut());
    Box::new(Toggle::new(&label.tr(), on, move |on| {
        *field(&mut shared.borrow_mut()) = on;
    }))
}

pub fn settings_form(appearance: Appearance, settings: &Shared<VeloxSettings>) -> Ui {
    let form = Form::new(
        &tr!("Velox Settings"),
        vec![
            FormLine::Heading(tr!("Startup")),
            FormLine::Row(tr!("Launch Velox at login"), login_toggle()),
            FormLine::Heading(tr!("Quick Access")),
            FormLine::Row(
                tr!("Status item in the Taskbar"),
                toggle(settings, text!("Status item in the Taskbar"), |settings| {
                    &mut settings.status_item
                }),
            ),
            FormLine::Heading(tr!("About")),
            FormLine::Row(tr!("Diagnostics"), Box::new(CopyDiagnostics::new())),
        ],
    );
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form)),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
