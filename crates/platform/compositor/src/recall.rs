use std::fs;
use std::io;
use std::path::PathBuf;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Point;

use crate::shell::toplevel_names;
use crate::state::{Compositor, Rect, State};

#[derive(Debug, Clone, PartialEq)]
struct Remembered {
    displays: String,
    app_id: String,
    frame: Rect,
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
            let numbers: Vec<i32> = fields.map(str::parse).collect::<Result<_, _>>().ok()?;
            let [x, y, w, h] = numbers.as_slice() else {
                return None;
            };
            (*w > 0 && *h > 0 && !displays.is_empty() && !app_id.is_empty()).then(|| Remembered {
                displays,
                app_id,
                frame: Rect {
                    location: (*x, *y).into(),
                    size: (*w, *h).into(),
                },
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
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                remembered.displays,
                remembered.app_id,
                frame.location.x,
                frame.location.y,
                frame.size.w,
                frame.size.h
            )
        })
        .collect()
}

pub(crate) fn clamp_into(frame: Rect, area: Rect) -> Rect {
    let w = frame.size.w.min(area.size.w);
    let h = frame.size.h.min(area.size.h);
    let x = frame
        .location
        .x
        .clamp(area.location.x, area.location.x + area.size.w - w);
    let y = frame
        .location
        .y
        .clamp(area.location.y, area.location.y + area.size.h - h);
    Rect {
        location: (x, y).into(),
        size: (w, h).into(),
    }
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

    fn recallable(&self, index: usize) -> Option<String> {
        let window = &self.windows[index];
        let app_id = toplevel_names(window.surface()).1;
        let alone = !self.windows.iter().enumerate().any(|(other, sibling)| {
            other != index
                && sibling.size.is_some()
                && toplevel_names(sibling.surface()).1 == app_id
        });
        (!app_id.is_empty() && window.toplevel.parent().is_none() && alone).then_some(app_id)
    }

    pub(crate) fn recall_frame(&mut self, index: usize) {
        let Some(app_id) = self.recallable(index) else {
            return;
        };
        let displays = self.display_set();
        let Some(frame) = self
            .recall
            .frames
            .iter()
            .find(|remembered| remembered.displays == displays && remembered.app_id == app_id)
            .map(|remembered| remembered.frame)
        else {
            return;
        };
        let center = frame.location + Point::from((frame.size.w / 2, frame.size.h / 2));
        let screen = self
            .screen_at(center.to_f64())
            .unwrap_or_else(|| self.pointer_screen());
        let frame = clamp_into(frame, self.usable_area(screen));
        let window = &mut self.windows[index];
        window.workspace = self.screens[screen].workspaces.active_id();
        window.recalled = Some(frame.location);
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
        let Some(app_id) = self.recallable(index) else {
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
        let Some(frame) = frame else {
            return;
        };
        let displays = self.display_set();
        self.recall
            .frames
            .retain(|remembered| remembered.displays != displays || remembered.app_id != app_id);
        self.recall.frames.push(Remembered {
            displays,
            app_id,
            frame,
        });
        if let Err(error) = self.recall.save() {
            tracing::warn!("cannot remember where the window was: {error}");
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect {
            location: (x, y).into(),
            size: (w, h).into(),
        }
    }

    #[test]
    fn remembered_frames_survive_a_round_trip_and_bad_lines_are_skipped() {
        let frames = vec![Remembered {
            displays: "Apple Studio Display 123 + eDP-1".to_owned(),
            app_id: "org.unbound.Files".to_owned(),
            frame: rect(-40, 30, 900, 600),
        }];

        let text = format(&frames) + "broken\tline\n\tx\t1\t2\t3\t4\n";

        assert_eq!(parse(&text), frames);
    }

    #[test]
    fn a_frame_is_moved_and_shrunk_into_the_visible_area() {
        let area = rect(0, 30, 1280, 770);

        assert_eq!(
            clamp_into(rect(1000, 0, 600, 400), area),
            rect(680, 30, 600, 400)
        );
        assert_eq!(
            clamp_into(rect(-50, 100, 2000, 900), area),
            rect(0, 30, 1280, 770)
        );
        assert_eq!(
            clamp_into(rect(100, 100, 300, 200), area),
            rect(100, 100, 300, 200)
        );
    }
}
