use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustix::io::{FdFlags, fcntl_setfd};

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
    BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, Damage,
    SurfaceAttributes, with_states,
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
use smithay::wayland::shm::{ShmHandler, ShmState, with_buffer_contents};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_ext_data_control,
    delegate_primary_selection, delegate_seat, delegate_shm, delegate_xdg_shell,
};
use unbound_render::Rect;

pub(crate) struct PendingBuffer {
    pub(crate) buffer: WlBuffer,
    pub(crate) damage: Vec<Rect>,
}

pub struct Window {
    pub(crate) id: u64,
    toplevel: ToplevelSurface,
    location: Point<i32, Physical>,
    size: Option<Size<i32, Physical>>,
    pub(crate) pending: Option<PendingBuffer>,
}

impl Window {
    pub fn location(&self) -> Point<i32, Physical> {
        self.location
    }

    pub fn size(&self) -> Option<Size<i32, Physical>> {
        self.size
    }

    fn bounds(&self) -> Option<Rect> {
        let size = self.size?;
        Some(Rect::new(self.location.x, self.location.y, size.w, size.h))
    }

    fn release_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.buffer.release();
        }
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
    pub(crate) windows: Vec<Window>,
    next_window_id: u64,
    pub(crate) damage: Vec<Rect>,
    needs_redraw: bool,
}

pub struct Headless {
    display: Display<State>,
    pub(crate) state: State,
    clipboard_daemon: Option<ClipboardDaemon>,
}

pub const CLIPBOARD_DAEMON: &str = "/usr/bin/unbound-clipboard";

pub const CLIPBOARD_DAEMON_RESTART_THROTTLE: Duration = Duration::from_secs(10);

struct ClipboardDaemon {
    executable: PathBuf,
    child: Option<Child>,
    last_restart: Option<Instant>,
}

impl Headless {
    pub fn new(output_size: Size<i32, Physical>) -> io::Result<Self> {
        let display = Display::new().map_err(io::Error::other)?;
        let handle = display.handle();
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&handle, "seat0");
        seat.add_keyboard(XkbConfig::default(), 600, 25)
            .map_err(io::Error::other)?;
        let primary_selection_state = PrimarySelectionState::new::<State>(&handle);
        let data_control_state =
            DataControlState::new::<State, _>(&handle, Some(&primary_selection_state), |client| {
                client
                    .get_data::<ClientState>()
                    .is_some_and(|state| state.clipboard_daemon)
            });
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
            next_window_id: 0,
            damage: Vec::new(),
            needs_redraw: false,
        };
        Ok(Self {
            display,
            state,
            clipboard_daemon: None,
        })
    }

    pub fn insert_client(&mut self, stream: UnixStream) -> io::Result<Client> {
        self.display
            .handle()
            .insert_client(stream, Arc::new(ClientState::default()))
    }

    pub fn insert_clipboard_daemon(&mut self, stream: UnixStream) -> io::Result<Client> {
        let state = ClientState {
            clipboard_daemon: true,
            ..ClientState::default()
        };
        self.display.handle().insert_client(stream, Arc::new(state))
    }

    pub fn start_clipboard_daemon(&mut self, executable: &Path) -> io::Result<()> {
        let child = self.spawn_clipboard_daemon(executable)?;
        self.clipboard_daemon = Some(ClipboardDaemon {
            executable: executable.to_path_buf(),
            child: Some(child),
            last_restart: None,
        });
        Ok(())
    }

    pub fn clipboard_daemon_pid(&self) -> Option<u32> {
        let child = self.clipboard_daemon.as_ref()?.child.as_ref()?;
        Some(child.id())
    }

    pub fn keep_clipboard_daemon_running(&mut self) -> io::Result<Option<Duration>> {
        let Some(daemon) = self.clipboard_daemon.as_mut() else {
            return Ok(None);
        };
        if let Some(child) = daemon.child.as_mut() {
            if child.try_wait()?.is_none() {
                return Ok(None);
            }
            daemon.child = None;
        }
        if let Some(since) = daemon.last_restart.map(|last| last.elapsed())
            && since < CLIPBOARD_DAEMON_RESTART_THROTTLE
        {
            return Ok(Some(CLIPBOARD_DAEMON_RESTART_THROTTLE - since));
        }
        daemon.last_restart = Some(Instant::now());
        let executable = daemon.executable.clone();
        let child = self.spawn_clipboard_daemon(&executable)?;
        if let Some(daemon) = self.clipboard_daemon.as_mut() {
            daemon.child = Some(child);
        }
        Ok(None)
    }

    fn spawn_clipboard_daemon(&mut self, executable: &Path) -> io::Result<Child> {
        let (compositor_end, daemon_end) = UnixStream::pair()?;
        self.insert_clipboard_daemon(compositor_end)?;
        let daemon_end = OwnedFd::from(daemon_end);
        let inherited = daemon_end.as_raw_fd();
        let mut command = Command::new(executable);
        command
            .env("WAYLAND_SOCKET", inherited.to_string())
            .env_remove("WAYLAND_DISPLAY");
        unsafe {
            command.pre_exec(move || {
                fcntl_setfd(BorrowedFd::borrow_raw(inherited), FdFlags::empty())
                    .map_err(io::Error::from)
            });
        }
        command.spawn()
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
    clipboard_daemon: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

fn shm_size(buffer: &WlBuffer) -> Option<Size<i32, Physical>> {
    let data = with_buffer_contents(buffer, |_, _, data| data).ok()?;
    matches!(data.format, Format::Argb8888 | Format::Xrgb8888)
        .then(|| Size::from((data.width, data.height)))
}

fn buffer_rect(damage: &Damage) -> Rect {
    let (x, y, width, height) = match damage {
        Damage::Surface(rect) => (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
        Damage::Buffer(rect) => (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
    };
    Rect::new(x, y, width, height)
}

impl State {
    fn focused(&self) -> Option<WlSurface> {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
    }

    fn focus(&mut self, surface: Option<WlSurface>) {
        if let Some(surface) = &surface
            && let Some(index) = self
                .windows
                .iter()
                .position(|window| window.toplevel.wl_surface() == surface)
        {
            let window = self.windows.remove(index);
            if index < self.windows.len() {
                self.damage.extend(window.bounds());
            }
            self.windows.push(window);
            self.needs_redraw = true;
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
    }

    fn frontmost(&self, owner: Option<&Client>) -> Option<WlSurface> {
        self.windows
            .iter()
            .rev()
            .filter(|window| window.size.is_some())
            .map(|window| window.toplevel.wl_surface())
            .find(|surface| {
                surface
                    .client()
                    .is_some_and(|client| owner.is_none_or(|owner| &client == owner))
            })
            .cloned()
    }

    fn refocus_after_losing(&mut self, lost: &WlSurface) {
        if self.focused().as_ref() != Some(lost) {
            return;
        }
        let next = match lost.client() {
            Some(owner) => self.frontmost(Some(&owner)),
            None => self.frontmost(None),
        };
        self.focus(next);
    }

    fn takes_focus_when_mapped(&self, surface: &WlSurface) -> bool {
        let Some(focused) = self.focused() else {
            return true;
        };
        let owner = surface.client();
        if focused.client() == owner {
            return true;
        }
        !self.windows.iter().any(|window| {
            window.size.is_some()
                && window.toplevel.wl_surface() != surface
                && window.toplevel.wl_surface().client() == owner
        })
    }

    fn place_behind_frontmost(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel.wl_surface() == surface)
        else {
            return;
        };
        let window = self.windows.remove(index);
        let behind = self.windows.len().saturating_sub(1);
        self.windows.insert(behind, window);
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
        let (assignment, damage) = with_states(surface, |states| {
            let mut attributes = states.cached_state.get::<SurfaceAttributes>();
            let current = attributes.current();
            (current.buffer.take(), std::mem::take(&mut current.damage))
        });
        let before = window.bounds();
        let mut updated = Vec::new();
        match assignment {
            Some(BufferAssignment::NewBuffer(buffer)) => match shm_size(&buffer) {
                Some(size) => {
                    if window.size.is_none() {
                        window.location = centered(self.output_size, size);
                    }
                    if window.size == Some(size) {
                        updated = damage.iter().map(buffer_rect).collect();
                    } else {
                        updated = vec![Rect::new(0, 0, size.w, size.h)];
                    }
                    window.size = Some(size);
                    let mut pending = updated.clone();
                    if let Some(previous) = window.pending.take() {
                        previous.buffer.release();
                        pending.extend(previous.damage);
                    }
                    window.pending = Some(PendingBuffer {
                        buffer,
                        damage: pending,
                    });
                }
                None => buffer.release(),
            },
            Some(BufferAssignment::Removed) => {
                window.size = None;
                window.release_pending();
            }
            None => {}
        }
        let after = window.bounds();
        if before != after {
            self.damage.extend(before);
            self.damage.extend(after);
        } else if let Some(bounds) = after {
            self.damage.extend(
                updated
                    .iter()
                    .filter_map(|rect| rect.translated(bounds.x, bounds.y).intersection(&bounds)),
            );
        }
        let surface = window.toplevel.wl_surface().clone();
        match (before.is_some(), after.is_some()) {
            (false, true) if self.takes_focus_when_mapped(&surface) => self.focus(Some(surface)),
            (false, true) => self.place_behind_frontmost(&surface),
            (true, false) => self.refocus_after_losing(&surface),
            _ => {}
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
        self.next_window_id += 1;
        self.windows.push(Window {
            id: self.next_window_id,
            toplevel,
            location: Point::default(),
            size: None,
            pending: None,
        });
    }

    fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
        if let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel == toplevel)
        {
            let mut window = self.windows.remove(index);
            window.release_pending();
            self.damage.extend(window.bounds());
        }
        self.needs_redraw = true;
        self.refocus_after_losing(toplevel.wl_surface());
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
