use std::os::fd::{AsFd, BorrowedFd};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};

use crate::state::{Display, Mode};

const OUTPUT_VERSION: u32 = 4;
const OUTPUT: &str = "wl_output";

#[derive(Default)]
struct Outputs {
    outputs: Vec<(u32, WlOutput, Display, bool)>,
    changed: bool,
}

impl Outputs {
    fn bind(&mut self, registry: &WlRegistry, name: u32, version: u32, queue: &QueueHandle<Self>) {
        let output = registry.bind(name, version.min(OUTPUT_VERSION), queue, name);
        self.outputs.push((name, output, Display::default(), false));
    }

    fn entry(&mut self, name: u32) -> Option<&mut (u32, WlOutput, Display, bool)> {
        self.outputs.iter_mut().find(|(global, ..)| *global == name)
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Outputs {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == OUTPUT => state.bind(registry, name, version, queue),
            wl_registry::Event::GlobalRemove { name } => {
                let before = state.outputs.len();
                state.outputs.retain(|(global, ..)| *global != name);
                state.changed |= state.outputs.len() != before;
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for Outputs {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, _, display, ready)) = state.entry(*name) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => display.connector = name,
            wl_output::Event::Description { description } => display.description = description,
            wl_output::Event::Mode {
                flags,
                width,
                height,
                refresh,
            } => {
                let mode = Mode {
                    width,
                    height,
                    millihertz: refresh,
                };
                if !display.modes.contains(&mode) {
                    display.modes.push(mode);
                }
                if matches!(flags, WEnum::Value(flags) if flags.contains(wl_output::Mode::Current))
                {
                    display.current = Some(mode);
                }
            }
            wl_output::Event::Done => {
                *ready = true;
                state.changed = true;
            }
            _ => {}
        }
    }
}

pub struct Displays {
    connection: Connection,
    queue: EventQueue<Outputs>,
    outputs: Outputs,
}

impl Displays {
    pub fn connect() -> Option<Self> {
        let connection = Connection::connect_to_env()
            .inspect_err(|error| tracing::warn!("cannot list the displays: {error}"))
            .ok()?;
        Self::on(connection)
    }

    pub fn on(connection: Connection) -> Option<Self> {
        let (globals, mut queue) = registry_queue_init::<Outputs>(&connection)
            .inspect_err(|error| tracing::warn!("cannot list the displays: {error}"))
            .ok()?;
        let mut outputs = Outputs::default();
        let handle = queue.handle();
        globals.contents().with_list(|list| {
            for global in list.iter().filter(|global| global.interface == OUTPUT) {
                outputs.bind(globals.registry(), global.name, global.version, &handle);
            }
        });
        queue.roundtrip(&mut outputs).ok()?;
        Some(Self {
            connection,
            queue,
            outputs,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }

    pub fn list(&self) -> Vec<Display> {
        let mut displays: Vec<Display> = self
            .outputs
            .outputs
            .iter()
            .filter(|(_, _, display, ready)| *ready && !display.connector.is_empty())
            .map(|(_, _, display, _)| display.clone())
            .collect();
        displays.sort_by(|a, b| a.connector.cmp(&b.connector));
        displays
    }

    pub fn dispatch(&mut self) -> Option<Vec<Display>> {
        if let Some(guard) = self.queue.prepare_read()
            && let Err(error) = guard.read()
        {
            tracing::debug!("display events not read: {error}");
        }
        if let Err(error) = self.queue.dispatch_pending(&mut self.outputs) {
            tracing::warn!("display events failed: {error}");
        }
        let _ = self.connection.flush();
        std::mem::take(&mut self.outputs.changed).then(|| self.list())
    }
}
