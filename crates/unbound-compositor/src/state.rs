use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display};
use smithay::utils::{Physical, Point, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
    with_states,
};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::shm::{BufferData, ShmHandler, ShmState, with_buffer_contents};
use smithay::{delegate_compositor, delegate_shm, delegate_xdg_shell};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub size: Size<i32, Physical>,
    pub bgra_premultiplied: Vec<u8>,
}

pub struct Window {
    toplevel: ToplevelSurface,
    location: Point<i32, Physical>,
    frame: Option<Frame>,
}

impl Window {
    pub fn location(&self) -> Point<i32, Physical> {
        self.location
    }

    pub fn frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
    }
}

pub struct State {
    output_size: Size<i32, Physical>,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<State>,
    windows: Vec<Window>,
    needs_redraw: bool,
}

pub struct Headless {
    display: Display<State>,
    state: State,
}

impl Headless {
    pub fn new(output_size: Size<i32, Physical>) -> io::Result<Self> {
        let display = Display::new().map_err(io::Error::other)?;
        let handle = display.handle();
        let state = State {
            output_size,
            compositor_state: CompositorState::new::<State>(&handle),
            xdg_shell_state: XdgShellState::new::<State>(&handle),
            shm_state: ShmState::new::<State>(&handle, []),
            seat_state: SeatState::new(),
            windows: Vec::new(),
            needs_redraw: false,
        };
        Ok(Self { display, state })
    }

    pub fn insert_client(&mut self, stream: UnixStream) -> io::Result<Client> {
        self.display
            .handle()
            .insert_client(stream, Arc::new(ClientState::default()))
    }

    pub fn dispatch(&mut self) -> io::Result<()> {
        self.display.dispatch_clients(&mut self.state)?;
        self.display.flush_clients()
    }

    pub fn poll_fd(&mut self) -> io::Result<OwnedFd> {
        self.display.backend().poll_fd().try_clone_to_owned()
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.state.needs_redraw)
    }

    pub fn frame_done(&mut self, time: Duration) -> io::Result<()> {
        let millis = u32::try_from(time.as_millis()).unwrap_or(u32::MAX);
        for window in &self.state.windows {
            with_states(window.toplevel.wl_surface(), |states| {
                states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                    .for_each(|callback| callback.done(millis));
            });
        }
        self.display.flush_clients()
    }

    pub fn output_size(&self) -> Size<i32, Physical> {
        self.state.output_size
    }

    pub fn windows(&self) -> &[Window] {
        &self.state.windows
    }
}

#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

fn copy_shm_frame(buffer: &WlBuffer) -> Option<Frame> {
    with_buffer_contents(buffer, |ptr, len, data| {
        let pool = unsafe { std::slice::from_raw_parts(ptr, len) };
        frame_from_shm(pool, data)
    })
    .ok()
    .flatten()
}

fn frame_from_shm(pool: &[u8], data: BufferData) -> Option<Frame> {
    let opaque = match data.format {
        Format::Argb8888 => false,
        Format::Xrgb8888 => true,
        _ => return None,
    };
    let width = usize::try_from(data.width).ok()?;
    let height = usize::try_from(data.height).ok()?;
    let stride = usize::try_from(data.stride).ok()?;
    let offset = usize::try_from(data.offset).ok()?;
    let row_bytes = width.checked_mul(4)?;
    let mut pixels = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let start = offset.checked_add(row.checked_mul(stride)?)?;
        pixels.extend_from_slice(pool.get(start..start.checked_add(row_bytes)?)?);
    }
    if opaque {
        pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|pixel| pixel[3] = u8::MAX);
    }
    Some(Frame {
        size: (data.width, data.height).into(),
        bgra_premultiplied: pixels,
    })
}

fn centered(output: Size<i32, Physical>, window: Size<i32, Physical>) -> Point<i32, Physical> {
    ((output.w - window.w) / 2, (output.h - window.h) / 2).into()
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        match client.get_data::<ClientState>() {
            Some(state) => &state.compositor_state,
            None => unreachable!("every client is inserted with ClientState"),
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        let Some(window) = self
            .windows
            .iter_mut()
            .find(|window| window.toplevel.wl_surface() == surface)
        else {
            return;
        };
        self.needs_redraw = true;
        let assignment = with_states(surface, |states| {
            states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer
                .take()
        });
        match assignment {
            Some(BufferAssignment::NewBuffer(buffer)) => {
                let frame = copy_shm_frame(&buffer);
                buffer.release();
                if let Some(frame) = frame {
                    if window.frame.is_none() {
                        window.location = centered(self.output_size, frame.size);
                    }
                    window.frame = Some(frame);
                }
            }
            Some(BufferAssignment::Removed) => window.frame = None,
            None => {}
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });
        toplevel.send_configure();
        self.windows.push(Window {
            toplevel,
            location: Point::default(),
            frame: None,
        });
    }

    fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
        self.windows.retain(|window| window.toplevel != toplevel);
        self.needs_redraw = true;
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
