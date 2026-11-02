use std::fs;
use std::io;
use std::path::PathBuf;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{self, XdgToplevel};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::xdg_toplevel_tag::XdgToplevelTagHandler;

use crate::shell::toplevel_names;
use crate::state::{Compositor, State};
use crate::window::Window;

#[derive(Debug, Clone, PartialEq)]
struct Remembered {
    displays: String,
    app_id: String,
    tag: String,
    frame: Rectangle<i32, Logical>,
}

#[derive(Default)]
pub(crate) struct Recall {
    path: Option<PathBuf>,
    frames: Vec<Remembered>,
}

fn parse(text: &str) -> Vec<Remembered> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let displays = fields.next()?.to_owned();
            let app_id = fields.next()?.to_owned();
            let mut rest: Vec<&str> = fields.collect();
            let tag = match rest.len() {
                5 => rest.remove(0).to_owned(),
                _ => String::new(),
            };
            let numbers: Vec<i32> = rest
                .into_iter()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            let [x, y, w, h] = numbers.as_slice() else {
                return None;
            };
            (*w > 0 && *h > 0 && !displays.is_empty() && !app_id.is_empty()).then(|| Remembered {
                displays,
                app_id,
                tag,
                frame: Rectangle::new((*x, *y).into(), (*w, *h).into()),
            })
        })
        .collect()
}

fn format(frames: &[Remembered]) -> String {
    frames
        .iter()
        .map(|remembered| {
            let frame = remembered.frame;
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                remembered.displays,
                remembered.app_id,
                remembered.tag,
                frame.loc.x,
                frame.loc.y,
                frame.size.w,
                frame.size.h
            )
        })
        .collect()
}

pub(crate) fn clamp_into(
    frame: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let w = frame.size.w.min(area.size.w);
    let h = frame.size.h.min(area.size.h);
    let x = frame.loc.x.clamp(area.loc.x, area.loc.x + area.size.w - w);
    let y = frame.loc.y.clamp(area.loc.y, area.loc.y + area.size.h - h);
    Rectangle::new((x, y).into(), (w, h).into())
}

impl Recall {
    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let staged = path.with_extension("new");
        fs::write(&staged, format(&self.frames))?;
        fs::rename(staged, path)
    }
}

impl State {
    fn display_set(&self) -> String {
        let mut identities: Vec<String> = self
            .screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .map(|screen| {
                let identity = screen.config.identity();
                if identity.is_empty() {
                    screen.name()
                } else {
                    identity
                }
            })
            .collect();
        identities.sort();
        identities.join(" + ")
    }

    fn recallable(&self, index: usize) -> Option<(String, String)> {
        let main_window =
            |window: &Window| window.toplevel.parent().is_none() && !window.fixed_size();
        let key = |window: &Window| (toplevel_names(window.surface()).1, window.tag.clone());
        let window = &self.windows[index];
        let own = key(window);
        let alone = !self.windows.iter().enumerate().any(|(other, sibling)| {
            other != index && sibling.size.is_some() && main_window(sibling) && key(sibling) == own
        });
        (!own.0.is_empty() && main_window(window) && alone).then_some(own)
    }

    pub(crate) fn recall_frame(&mut self, index: usize) {
        let Some((app_id, tag)) = self.recallable(index) else {
            return;
        };
        let displays = self.display_set();
        let Some(frame) = self
            .recall
            .frames
            .iter()
            .find(|remembered| {
                remembered.displays == displays
                    && remembered.app_id == app_id
                    && remembered.tag == tag
            })
            .map(|remembered| remembered.frame)
        else {
            return;
        };
        let center = frame.loc + Point::from((frame.size.w / 2, frame.size.h / 2));
        let screen = self
            .screen_at(center.to_f64())
            .unwrap_or_else(|| self.pointer_screen());
        let frame = clamp_into(frame, self.usable_area(screen));
        let window = &mut self.windows[index];
        window.workspace = self.screens[screen].workspaces.active_id();
        window.recalled = Some(frame.loc);
        window.request_size(Some(frame.size));
    }

    pub(crate) fn recall_late(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &self.windows[index];
        if window.size.is_some()
            || window.recalled.is_some()
            || !window.toplevel.is_initial_configure_sent()
        {
            return;
        }
        self.recall_frame(index);
        if self.windows[index].recalled.is_some() {
            self.windows[index].configure();
        }
    }

    pub(crate) fn remember_frame(&mut self, index: usize) {
        if self.recall.path.is_none() {
            return;
        }
        let Some((app_id, tag)) = self.recallable(index) else {
            return;
        };
        let window = &self.windows[index];
        let placed = window.tile.is_some()
            || window.has_state(xdg_toplevel::State::Maximized)
            || window.has_state(xdg_toplevel::State::Fullscreen);
        let frame = if placed {
            window.restore
        } else {
            window.geometry()
        };
        let Some(frame) = frame.filter(|frame| frame.size.w > 0 && frame.size.h > 0) else {
            return;
        };
        let displays = self.display_set();
        self.recall.frames.retain(|remembered| {
            remembered.displays != displays || remembered.app_id != app_id || remembered.tag != tag
        });
        self.recall.frames.push(Remembered {
            displays,
            app_id,
            tag,
            frame,
        });
        if let Err(error) = self.recall.save() {
            tracing::warn!("cannot remember where the window was: {error}");
        }
    }
}

impl XdgToplevelTagHandler for State {
    fn set_tag(&mut self, toplevel: XdgToplevel, tag: String) {
        let Some(window) = self
            .windows
            .iter_mut()
            .find(|window| window.toplevel.xdg_toplevel() == &toplevel)
        else {
            return;
        };
        window.tag = if tag.chars().any(char::is_control) {
            String::new()
        } else {
            tag
        };
        let surface = window.surface().clone();
        self.recall_late(&surface);
    }
}

smithay::delegate_xdg_toplevel_tag!(State);

impl Compositor {
    pub fn remember_windows_in(&mut self, path: PathBuf) {
        let frames = fs::read_to_string(&path)
            .map(|text| parse(&text))
            .unwrap_or_default();
        self.state.recall = Recall {
            path: Some(path),
            frames,
        };
    }
}
