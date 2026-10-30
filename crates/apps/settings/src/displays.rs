use std::os::fd::{AsFd, BorrowedFd};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::Transform;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use crate::state::{Display, Mode};

const MANAGER_VERSION: u32 = 4;
const MANAGER: &str = "zwlr_output_manager_v1";

struct Head {
    head: ZwlrOutputHeadV1,
    display: Display,
    modes: Vec<(ZwlrOutputModeV1, Mode)>,
}

#[derive(Default)]
struct Heads {
    heads: Vec<Head>,
    changed: bool,
}

impl Heads {
    fn head(&mut self, head: &ZwlrOutputHeadV1) -> Option<&mut Head> {
        self.heads.iter_mut().find(|known| known.head == *head)
    }

    fn mode(&mut self, mode: &ZwlrOutputModeV1) -> Option<&mut Mode> {
        self.heads
            .iter_mut()
            .flat_map(|head| head.modes.iter_mut())
            .find(|(known, _)| known == mode)
            .map(|(_, mode)| mode)
    }
}

fn degrees(transform: WEnum<Transform>) -> u32 {
    match transform {
        WEnum::Value(Transform::_90 | Transform::Flipped90) => 90,
        WEnum::Value(Transform::_180 | Transform::Flipped180) => 180,
        WEnum::Value(Transform::_270 | Transform::Flipped270) => 270,
        _ => 0,
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Heads {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for Heads {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => state.heads.push(Head {
                head,
                display: Display::default(),
                modes: Vec::new(),
            }),
            zwlr_output_manager_v1::Event::Done { .. } => state.changed = true,
            _ => {}
        }
    }

    event_created_child!(Heads, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for Heads {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_output_head_v1::Event::Finished = event {
            state.heads.retain(|known| known.head != *head);
            head.release();
            return;
        }
        let Some(known) = state.head(head) else {
            return;
        };
        match event {
            zwlr_output_head_v1::Event::Name { name } => known.display.connector = name,
            zwlr_output_head_v1::Event::Description { description } => {
                known.display.identity = description;
            }
            zwlr_output_head_v1::Event::Mode { mode } => known.modes.push((mode, Mode::default())),
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                known.display.current = known
                    .modes
                    .iter()
                    .find(|(known, _)| *known == mode)
                    .map(|(_, mode)| *mode);
            }
            zwlr_output_head_v1::Event::Make { make } => known.display.make = make,
            zwlr_output_head_v1::Event::Model { model } => known.display.model = model,
            zwlr_output_head_v1::Event::Position { x, y } => known.display.position = (x, y),

            zwlr_output_head_v1::Event::Scale { scale } => known.display.scale = scale,
            zwlr_output_head_v1::Event::Transform { transform } => {
                known.display.rotation = degrees(transform);
            }
            _ => {}
        }
    }

    event_created_child!(Heads, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for Heads {
    fn event(
        state: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                if let Some(known) = state.mode(mode) {
                    known.width = width;
                    known.height = height;
                }
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                if let Some(known) = state.mode(mode) {
                    known.millihertz = refresh;
                }
            }
            zwlr_output_mode_v1::Event::Finished => {
                for head in &mut state.heads {
                    head.modes.retain(|(known, _)| known != mode);
                }
                if mode.version() >= zwlr_output_mode_v1::REQ_RELEASE_SINCE {
                    mode.release();
                }
            }
            _ => {}
        }
    }
}

pub struct Displays {
    connection: Connection,
    queue: EventQueue<Heads>,
    heads: Heads,
}

impl Displays {
    pub fn connect() -> Option<Self> {
        let connection = Connection::connect_to_env()
            .inspect_err(|error| tracing::warn!("cannot list the displays: {error}"))
            .ok()?;
        Self::on(connection)
    }

    pub fn on(connection: Connection) -> Option<Self> {
        let (globals, mut queue) = registry_queue_init::<Heads>(&connection)
            .inspect_err(|error| tracing::warn!("cannot list the displays: {error}"))
            .ok()?;
        let handle = queue.handle();
        let _manager: ZwlrOutputManagerV1 = globals
            .bind(&handle, 1..=MANAGER_VERSION, ())
            .inspect_err(|error| tracing::warn!("cannot bind {MANAGER}: {error}"))
            .ok()?;
        let mut heads = Heads::default();
        queue.roundtrip(&mut heads).ok()?;
        queue.roundtrip(&mut heads).ok()?;
        Some(Self {
            connection,
            queue,
            heads,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }

    pub fn list(&self) -> Vec<Display> {
        let mut displays: Vec<Display> = self
            .heads
            .heads
            .iter()
            .filter(|head| !head.display.connector.is_empty())
            .map(|head| Display {
                modes: head.modes.iter().map(|(_, mode)| *mode).collect(),
                ..head.display.clone()
            })
            .collect();
        displays.sort_by_key(|display| display.position);
        displays
    }

    pub fn dispatch(&mut self) -> Option<Vec<Display>> {
        if let Some(guard) = self.queue.prepare_read()
            && let Err(error) = guard.read()
        {
            tracing::debug!("display events not read: {error}");
        }
        if let Err(error) = self.queue.dispatch_pending(&mut self.heads) {
            tracing::warn!("display events failed: {error}");
        }
        let _ = self.connection.flush();
        std::mem::take(&mut self.heads.changed).then(|| self.list())
    }
}
