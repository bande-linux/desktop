use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use smithay::input::keyboard::XkbConfig;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, Resource};
use smithay::utils::{Physical, Point, SERIAL_COUNTER, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
    with_states,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    set_data_device_focus,
};
use smithay::wayland::selection::ext_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::shm::{BufferData, ShmHandler, ShmState, with_buffer_contents};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_ext_data_control,
    delegate_primary_selection, delegate_seat, delegate_shm, delegate_xdg_shell,
};

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
    display_handle: DisplayHandle,
    output_size: Size<i32, Physical>,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<State>,
    seat: Seat<State>,
    data_device_state: DataDeviceState,
    primary_selection_state: PrimarySelectionState,
    data_control_state: DataControlState,
    windows: Vec<Window>,
    needs_redraw: bool,
}

pub struct Headless {
    display: Display<State>,
    state: State,
}

pub const CLIPBOARD_DAEMON: &str = "/usr/bin/unbound-clipboard";

fn executable(handle: &DisplayHandle, client: &Client) -> Option<PathBuf> {
    let credentials = client.get_credentials(handle).ok()?;
    std::fs::read_link(format!("/proc/{}/exe", credentials.pid)).ok()
}

impl Headless {
    pub fn new(output_size: Size<i32, Physical>) -> io::Result<Self> {
        Self::with_clipboard_daemons(output_size, &[Path::new(CLIPBOARD_DAEMON)])
    }

    pub fn with_clipboard_daemons(
        output_size: Size<i32, Physical>,
        clipboard_daemons: &[&Path],
    ) -> io::Result<Self> {
        let display = Display::new().map_err(io::Error::other)?;
        let handle = display.handle();
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&handle, "seat0");
        seat.add_keyboard(XkbConfig::default(), 600, 25)
            .map_err(io::Error::other)?;
        let primary_selection_state = PrimarySelectionState::new::<State>(&handle);
        let trusted: Vec<PathBuf> = clipboard_daemons
            .iter()
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
            .collect();
        let filter_handle = handle.clone();
        let data_control_state = DataControlState::new::<State, _>(
            &handle,
            Some(&primary_selection_state),
            move |client| {
                executable(&filter_handle, client).is_some_and(|path| trusted.contains(&path))
            },
        );
        let state = State {
            display_handle: handle.clone(),
            output_size,
            compositor_state: CompositorState::new::<State>(&handle),
            xdg_shell_state: XdgShellState::new::<State>(&handle),
            shm_state: ShmState::new::<State>(&handle, []),
            seat_state,
            seat,
            data_device_state: DataDeviceState::new::<State>(&handle),
            primary_selection_state,
            data_control_state,
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

impl State {
    fn focus(&mut self, surface: Option<WlSurface>) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
    }
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
        let was_mapped = window.frame.is_some();
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
        if !was_mapped && window.frame.is_some() {
            let surface = window.toplevel.wl_surface().clone();
            self.focus(Some(surface));
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
        let focused = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        if focused.as_ref() == Some(toplevel.wl_surface()) {
            let next = self
                .windows
                .iter()
                .rev()
                .find(|window| window.frame.is_some())
                .map(|window| window.toplevel.wl_surface().clone());
            self.focus(next);
        }
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

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client.clone());
        set_primary_focus(&self.display_handle, seat, client);
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}

impl ServerDndGrabHandler for State {}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

delegate_compositor!(State);
delegate_data_device!(State);
delegate_ext_data_control!(State);
delegate_primary_selection!(State);
delegate_seat!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
