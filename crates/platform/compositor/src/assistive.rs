use std::process::{Child, Command};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process};
use smithay::backend::renderer::element::Id;
use smithay::utils::Rectangle;
use unbound_config::{Accessibility, BUS_NAME, OBJECT_PATH, SCREEN_READER};
use unbound_graphics::Palette;
use unbound_spring::{Animation, Spring, State as Phase};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

use crate::cursor::CursorTheme;
use crate::environment::export_cursor_environment;
use crate::motion::SETTLED_ALPHA;
use crate::scene::{FillElement, rounded};
use crate::state::{Compositor, OutputElement, State};

const SOUND_BUS_NAME: &str = "org.unbound.Sound";
const SOUND_PATH: &str = "/org/unbound/Sound";
const FLASH_OPACITY: f64 = 0.45;

type Remembered = (&'static str, OwnedValue);

pub(crate) fn screen_reader_installed() -> bool {
    unbound_xdg::find_executable(SCREEN_READER).is_some()
}

#[derive(Default)]
pub(crate) struct Assistive {
    pub(crate) settings: Accessibility,
    screen_reader: Option<Child>,
    remembering: Option<Sender<Remembered>>,
    flash: Option<Flash>,
}

struct Flash {
    id: Id,
    opacity: Animation,
    last: Option<Duration>,
}

fn remembering(first: Remembered) -> Option<Sender<Remembered>> {
    let (sender, settings) = channel::<Remembered>();
    thread::spawn(move || {
        let Ok(connection) = Connection::session() else {
            return;
        };
        for (property, value) in settings {
            let set = connection.call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &(BUS_NAME, property, Value::from(value)),
            );
            if let Err(error) = set {
                tracing::debug!("cannot remember {property}: {error}");
            }
        }
    });
    sender.send(first).ok().map(|()| sender)
}

fn stop(mut child: Child) {
    if let Ok(pid) = i32::try_from(child.id())
        && let Some(pid) = Pid::from_raw(pid)
    {
        let _ = kill_process(pid, Signal::TERM);
    }
    thread::spawn(move || child.wait());
}

pub fn follow_alerts(
    connection: &Connection,
    mut alerted: impl FnMut() -> bool + Send + 'static,
) -> zbus::Result<()> {
    let sound = Proxy::new(connection, SOUND_BUS_NAME, SOUND_PATH, SOUND_BUS_NAME)?;
    let alerts = sound.receive_signal("AlertPlayed")?;
    thread::spawn(move || {
        for _ in alerts {
            if !alerted() {
                return;
            }
        }
    });
    Ok(())
}

impl State {
    pub(crate) fn remember_setting(&mut self, property: &'static str, value: Value<'_>) {
        let Ok(value) = OwnedValue::try_from(value) else {
            return;
        };
        let assistive = &mut self.assistive;
        let unsent = match &assistive.remembering {
            Some(sender) => sender.send((property, value)).err().map(|error| error.0),
            None => Some((property, value)),
        };
        if let Some(remembered) = unsent {
            assistive.remembering = remembering(remembered);
        }
    }

    pub(crate) fn toggle_screen_reader(&mut self) {
        let on = !self.assistive.settings.screen_reader;
        self.assistive.settings.screen_reader = on;
        self.remember_setting("ScreenReader", Value::from(on));
    }

    pub(crate) fn apply_accessibility(&mut self, settings: Accessibility) {
        let before = std::mem::replace(&mut self.assistive.settings, settings);
        self.zoom.configure(settings.zoom, self.reduce_motion);
        if settings.cursor_size != self.cursor_theme.size {
            let theme = CursorTheme {
                size: settings.cursor_size,
                ..self.cursor_theme.clone()
            };
            let exported = theme.clone();
            thread::spawn(move || {
                let result = Connection::session()
                    .and_then(|session| export_cursor_environment(&session, &exported));
                if let Err(error) = result {
                    tracing::warn!("cannot export the pointer size: {error}");
                }
            });
            self.use_cursor_theme(theme);
        }
        self.configure_access_keys(&settings);
        if before.reduce_transparency != settings.reduce_transparency {
            self.needs_redraw = true;
        }
    }

    pub(crate) fn backdrop(&self) -> Option<[f32; 4]> {
        if !self.assistive.settings.reduce_transparency {
            return None;
        }
        let chassis = Palette::for_scheme(self.decorations.scheme).chassis;
        let [r, g, b] = [chassis.r, chassis.g, chassis.b].map(|channel| f32::from(channel) / 255.0);
        Some([r, g, b, 1.0])
    }

    pub(crate) fn flash_screen(&mut self) {
        if !self.assistive.settings.flash_screen {
            return;
        }
        let id = self
            .assistive
            .flash
            .take()
            .map_or_else(Id::new, |flash| flash.id);
        self.assistive.flash = Some(Flash {
            id,
            opacity: Animation::new(
                Spring::GENTLE,
                Phase {
                    position: FLASH_OPACITY,
                    velocity: 0.0,
                },
                0.0,
            ),
            last: None,
        });
        self.needs_redraw = true;
    }

    pub(crate) fn animate_flash(&mut self, time: Duration) {
        let Some(flash) = self.assistive.flash.as_mut() else {
            return;
        };
        let step = flash
            .last
            .map_or(Duration::ZERO, |last| time.saturating_sub(last));
        flash.last = Some(time);
        flash.opacity.advance(step.as_secs_f64());
        if flash.opacity.is_at_rest(SETTLED_ALPHA) {
            self.assistive.flash = None;
        }
    }

    pub(crate) fn is_flashing(&self) -> bool {
        self.assistive.flash.is_some()
    }

    pub(crate) fn flash_elements(&self, screen: usize, scale: f64) -> Vec<OutputElement> {
        let Some(flash) = &self.assistive.flash else {
            return Vec::new();
        };
        let opacity = flash.opacity.state().position.clamp(0.0, 1.0) as f32;
        let rect = self.screen_rect(screen);
        let area = Rectangle::new(rect.location.to_f64(), rect.size.to_f64()).to_physical(scale);
        vec![OutputElement::Fill(FillElement::new(
            flash.id.clone(),
            rounded(area, 0.0),
            [opacity, opacity, opacity, opacity],
        ))]
    }
}

impl Compositor {
    pub(crate) fn keep_screen_reader(&mut self) {
        let assistive = &mut self.state.assistive;
        if let Some(child) = assistive.screen_reader.as_mut()
            && !matches!(child.try_wait(), Ok(None))
        {
            assistive.screen_reader = None;
            if assistive.settings.screen_reader {
                self.state.toggle_screen_reader();
            }
            return;
        }
        let wanted = assistive.settings.screen_reader;
        match (wanted, assistive.screen_reader.take()) {
            (true, None) => {
                let mut command = Command::new(SCREEN_READER);
                command.arg("--replace").env("DISPLAY", "");
                if let Some(name) = &self.socket_name {
                    command.env("WAYLAND_DISPLAY", name);
                }
                match command.spawn() {
                    Ok(child) => self.state.assistive.screen_reader = Some(child),
                    Err(error) => {
                        tracing::warn!("cannot start the screen reader: {error}");
                        self.state.toggle_screen_reader();
                    }
                }
            }
            (true, running) => self.state.assistive.screen_reader = running,
            (false, Some(child)) => stop(child),
            (false, None) => {}
        }
    }

    pub fn apply_accessibility(&mut self, settings: Accessibility) {
        self.state.apply_accessibility(settings);
    }

    pub fn flash_screen(&mut self) {
        self.state.flash_screen();
    }
}
