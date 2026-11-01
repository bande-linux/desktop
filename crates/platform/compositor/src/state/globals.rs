use smithay::backend::allocator::{Format as DmabufFormat, Fourcc, Modifier};
use smithay::input::SeatState;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::drm_syncobj::DrmSyncobjState;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::input_method::InputMethodManagerState;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::session_lock::SessionLockManagerState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as KdeDefaultMode;
use smithay::wayland::shell::kde::decoration::KdeDecorationState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::dialog::XdgDialogState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::xdg_activation::XdgActivationState;

use super::{ClientState, ScanoutDevice, State};
use crate::session::SessionRole;
use crate::shell;

const MONOTONIC: u32 = 1;

const DMABUF_FOURCCS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

pub(crate) struct Globals {
    pub(crate) syncobj_state: Option<DrmSyncobjState>,
    pub(crate) seat_state: SeatState<State>,
    pub(crate) primary_selection_state: PrimarySelectionState,
    pub(crate) data_control_state: DataControlState,
    pub(crate) dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    pub(crate) compositor_state: CompositorState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) shm_state: ShmState,
    _output_manager_state: OutputManagerState,
    _presentation_state: PresentationState,
    pub(crate) layer_shell_state: WlrLayerShellState,
    pub(crate) activation_state: XdgActivationState,
    _viewporter_state: ViewporterState,
    _fractional_scale_state: FractionalScaleManagerState,
    _decoration_state: XdgDecorationState,
    pub(crate) kde_decoration_state: KdeDecorationState,
    _dialog_state: XdgDialogState,
    _cursor_shape_state: CursorShapeManagerState,
    _pointer_gestures_state: PointerGesturesState,
    _text_input_state: TextInputManagerState,
    _input_method_state: InputMethodManagerState,
    _virtual_keyboard_state: VirtualKeyboardManagerState,
    pub(crate) foreign_toplevel_state: ForeignToplevelListState,
    pub(crate) data_device_state: DataDeviceState,
    _idle_inhibit_state: IdleInhibitManagerState,
    pub(crate) shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    _pointer_constraints_state: PointerConstraintsState,
    _relative_pointer_state: RelativePointerManagerState,
    pub(crate) session_lock_state: SessionLockManagerState,
}

impl Globals {
    pub(crate) fn new(handle: &DisplayHandle, scanout: Option<&ScanoutDevice>) -> Self {
        let primary_selection_state = PrimarySelectionState::new::<State>(handle);
        let data_control_state =
            DataControlState::new::<State, _>(handle, Some(&primary_selection_state), |client| {
                client
                    .get_data::<ClientState>()
                    .is_some_and(|state| state.role == Some(SessionRole::ClipboardDaemon))
            });
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = match scanout {
            Some(scanout) => {
                dmabuf_state.create_global_with_default_feedback::<State>(handle, &scanout.feedback)
            }
            None => dmabuf_state.create_global::<State>(
                handle,
                DMABUF_FOURCCS.map(|code| DmabufFormat {
                    code,
                    modifier: Modifier::Linear,
                }),
            ),
        };
        Self {
            syncobj_state: None,
            seat_state: SeatState::new(),
            primary_selection_state,
            data_control_state,
            dmabuf_state,
            _dmabuf_global: dmabuf_global,
            compositor_state: CompositorState::new::<State>(handle),
            xdg_shell_state: XdgShellState::new_with_capabilities::<State>(
                handle,
                [
                    xdg_toplevel::WmCapabilities::Maximize,
                    xdg_toplevel::WmCapabilities::Fullscreen,
                    xdg_toplevel::WmCapabilities::Minimize,
                ],
            ),
            shm_state: ShmState::new::<State>(handle, []),
            _output_manager_state: OutputManagerState::new(),
            _presentation_state: PresentationState::new::<State>(handle, MONOTONIC),
            layer_shell_state: WlrLayerShellState::new::<State>(handle),
            activation_state: XdgActivationState::new::<State>(handle),
            _viewporter_state: ViewporterState::new::<State>(handle),
            _fractional_scale_state: FractionalScaleManagerState::new::<State>(handle),
            _decoration_state: XdgDecorationState::new::<State>(handle),
            kde_decoration_state: KdeDecorationState::new::<State>(handle, KdeDefaultMode::Server),
            _dialog_state: XdgDialogState::new::<State>(handle),
            _cursor_shape_state: CursorShapeManagerState::new::<State>(handle),
            _pointer_gestures_state: PointerGesturesState::new::<State>(handle),
            _text_input_state: TextInputManagerState::new::<State>(handle),
            _input_method_state: InputMethodManagerState::new::<State, _>(handle, |_| true),
            _virtual_keyboard_state: VirtualKeyboardManagerState::new::<State, _>(handle, |_| true),
            foreign_toplevel_state: ForeignToplevelListState::new_with_filter::<State>(
                handle,
                shell::is_shell,
            ),
            data_device_state: DataDeviceState::new::<State>(handle),
            _idle_inhibit_state: IdleInhibitManagerState::new::<State>(handle),
            shortcuts_inhibit_state: KeyboardShortcutsInhibitState::new::<State>(handle),
            _pointer_constraints_state: PointerConstraintsState::new::<State>(handle),
            _relative_pointer_state: RelativePointerManagerState::new::<State>(handle),
            session_lock_state: SessionLockManagerState::new::<State, _>(handle, shell::is_shell),
        }
    }
}
