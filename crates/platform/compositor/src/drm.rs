use std::error::Error;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Allocator, Buffer, Fourcc, Modifier};
use smithay::backend::allocator::dumb::DumbAllocator;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::{
    DrmCompositor, FrameError, FrameFlags, PrimaryPlaneElement,
};
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmError, DrmEventMetadata, DrmEventTime, DrmNode, NodeType,
    VrrSupport,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::{ImportDma, ImportMem, Renderer as _};
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::output::OutputModeSource;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode as Trigger, PostAction};
use smithay::reexports::drm::control::atomic::AtomicModeReq;
use smithay::reexports::drm::control::{
    AtomicCommitFlags, Device as ControlDevice, Mode, ModeTypeFlags, connector, crtc, framebuffer, property,
};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::udev::MonitorBuilder;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
use smithay::utils::{Clock, DeviceFd, Monotonic, Size};
use smithay::wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder};
use bande_render::VulkanRenderer;

use crate::boot_screen::boot_screen_shown;
use crate::clock::{FrameOutput, Submission, Vblank};
use crate::element::drawing_everything;
use crate::render::{Snapshot, background};
use crate::reveal::ShownFrame;
use crate::scanout::{ScanoutAllocator, ScanoutExporter};
use crate::screen::{OutputConfig, OutputMode};
use crate::state::{Compositor, REFRESH, ScanoutDevice};
use bande_colour::Profile;
use bande_render::OutputColour;

pub type DrmBackendError = Box<dyn Error>;

type OutputCompositor = DrmCompositor<ScanoutAllocator, ScanoutExporter, (), DrmDeviceFd>;

const DRM_IOCTL_MODE_CLOSEFB: libc::c_ulong = 0xC008_64D0;
const DRM_IOCTL_MODE_MAP_DUMB: libc::c_ulong = 0xC010_64B3;
const CURSOR_SIZE: (u32, u32) = (64, 64);
const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

struct Head {
    connector: connector::Handle,
    config: OutputConfig,
    modes: Vec<Mode>,
    compositor: OutputCompositor,
    buffers: Buffers,
    variable_refresh: bool,
    scanned_out: bool,
    dark: bool,
    colour: Colour,
    tone: Option<f64>,
    lost_at: Option<Instant>,
    relink_wanted: bool,
    flip_pending: bool,
    shows_content: bool,
    refused: bool,
    kept_mode: Option<Mode>,
    display_profile: Option<Profile>,
    output_colour: Option<(Option<String>, Option<OutputColour>)>,
    converting: bool,
}

impl Head {
    fn darken(&mut self) -> Result<(), DrmBackendError> {
        if !self.dark {
            self.compositor.clear()?;
            self.dark = true;
            self.flip_pending = false;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Buffers {
    Tiled,
    Linear,
    Dumb,
}

impl Buffers {
    fn next(self) -> Option<Self> {
        match self {
            Self::Tiled => Some(Self::Linear),
            Self::Linear => Some(Self::Dumb),
            Self::Dumb => None,
        }
    }
}

impl std::fmt::Display for Buffers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tiled => "tiled GBM buffers",
            Self::Linear => "linear GBM buffers",
            Self::Dumb => "dumb buffers",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Colour {
    Unknown,
    Ramp(usize),
    Matrix(property::Handle),
    Fixed,
}

pub struct DrmBackend {
    session: LibSeatSession,
    path: PathBuf,
    device: DrmDevice,
    gbm: Option<GbmDevice<DrmDeviceFd>>,
    renderer: VulkanRenderer,
    node: DrmNode,
    feedback: DmabufFeedback,
    feedback_device: u64,
    heads: Vec<Head>,
    libinput: Option<Libinput>,
    kelvin: Option<f64>,
    first_mode: FirstMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstMode {
    OnScreen,
    Preferred,
}

pub struct DrmNotifiers {
    pub session: LibSeatSessionNotifier,
    pub device: DrmDeviceNotifier,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct HeadChanges {
    pub connected: Vec<OutputConfig>,
    pub disconnected: Vec<String>,
    pub refitted: Vec<OutputConfig>,
    pub returned: Vec<String>,
    pub look_again_in: Option<Duration>,
}

const LINK_STATUS: &[u8] = b"link-status";
const LINK_GOOD: u64 = 0;
const LINK_BAD: u64 = 1;
const DISPLAY_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rescanned {
    Kept,
    Lost(Duration),
    Gone,
}

fn rescanned(present: bool, another_display: bool, lost_for: Duration) -> Rescanned {
    match (present, another_display) {
        (true, false) => Rescanned::Kept,
        (true, true) => Rescanned::Gone,
        (false, _) if lost_for >= DISPLAY_GRACE => Rescanned::Gone,
        (false, _) => Rescanned::Lost(DISPLAY_GRACE - lost_for),
    }
}

fn link_status(device: &DrmDevice, handle: connector::Handle) -> Option<(property::Handle, u64)> {
    let properties = device.get_properties(handle).ok()?;
    properties.iter().find_map(|(property, value)| {
        let info = device.get_property(*property).ok()?;
        (info.name().to_bytes() == LINK_STATUS).then_some((*property, *value))
    })
}

fn relink(device: &DrmDevice, head: &mut Head) {
    head.relink_wanted = false;
    if let Some((property, _)) = link_status(device, head.connector) {
        let mut request = AtomicModeReq::new();
        request.add_property(
            head.connector,
            property,
            property::Value::UnsignedRange(LINK_GOOD),
        );
        if let Err(error) = device.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, request) {
            tracing::warn!("cannot retrain the link of {}: {error}", head.config.name);
        }
    }
    if let Err(error) = head.compositor.reset_state() {
        tracing::warn!("cannot reset {}: {error}", head.config.name);
    }
    head.tone = None;
}

fn refresh_of(mode: &Mode) -> Duration {
    let pixels_per_frame = u64::from(mode.hsync().2) * u64::from(mode.vsync().2);
    let kilohertz = u64::from(mode.clock());
    if pixels_per_frame == 0 || kilohertz == 0 {
        return REFRESH;
    }
    Duration::from_nanos(pixels_per_frame * 1_000_000 / kilohertz)
}

fn output_mode(mode: &Mode) -> OutputMode {
    let (width, height) = mode.size();
    OutputMode {
        size: Size::from((i32::from(width), i32::from(height))),
        refresh: refresh_of(mode),
    }
}

fn connector_name(info: &connector::Info) -> String {
    format!("{}-{}", info.interface().as_str(), info.interface_id())
}

fn sorted_modes(info: &connector::Info) -> Vec<Mode> {
    let mut modes = info.modes().to_vec();
    let preferred = modes
        .iter()
        .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or(modes.first())
        .map(Mode::size);
    modes.sort_by_key(|mode| {
        let (width, height) = mode.size();
        (
            Some(mode.size()) != preferred,
            std::cmp::Reverse(u32::from(width) * u32::from(height)),
            refresh_of(mode),
        )
    });
    modes.dedup_by_key(|mode| output_mode(mode));
    modes
}

fn modes_of(info: &connector::Info, first: Option<Mode>) -> Vec<Mode> {
    let mut modes = sorted_modes(info);
    if let Some(first) = first.filter(|first| info.modes().contains(first)) {
        modes.retain(|mode| output_mode(mode) != output_mode(&first));
        modes.insert(0, first);
    }
    modes
}

fn edid_of(device: &DrmDevice, handle: connector::Handle) -> Option<bande_edid::Edid> {
    let properties = device.get_properties(handle).ok()?;
    properties.iter().find_map(|(property, value)| {
        let info = device.get_property(*property).ok()?;
        if info.name().to_bytes() != b"EDID" {
            return None;
        }
        let property::Value::Blob(blob) = info.value_type().convert_value(*value) else {
            return None;
        };
        bande_edid::parse(&device.get_property_blob(blob).ok()?)
    })
}

fn shows_another_display(known: &OutputConfig, edid: Option<bande_edid::Edid>) -> bool {
    edid.is_some_and(|edid| {
        edid.make != known.make || edid.model != known.model || edid.serial != known.serial
    })
}

fn head_config(device: &DrmDevice, info: &connector::Info, modes: &[Mode]) -> OutputConfig {
    let edid = edid_of(device, info.handle());
    let name = connector_name(info);
    let (make, model, serial) = match edid {
        Some(edid) => (edid.make, edid.model, edid.serial),
        None => (String::new(), name.clone(), String::new()),
    };
    OutputConfig {
        name,
        make,
        model,
        serial,
        physical_size_mm: info
            .size()
            .map(|(width, height)| {
                (
                    i32::try_from(width).unwrap_or(0),
                    i32::try_from(height).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0))
            .into(),
        modes: modes.iter().map(output_mode).collect(),
    }
}

fn modes_in_trial_order(modes: &[Mode], wanted: Mode) -> Vec<Mode> {
    let same_size = |mode: &&Mode| mode.size() == wanted.size();
    modes
        .iter()
        .filter(|mode| **mode == wanted)
        .chain(
            modes
                .iter()
                .filter(same_size)
                .filter(|mode| **mode != wanted),
        )
        .chain(modes.iter().filter(|mode| mode.size() != wanted.size()))
        .copied()
        .collect()
}

fn first_working_mode(head: &mut Head, modes: &[Mode], wanted: Mode) -> Option<Mode> {
    modes_in_trial_order(modes, wanted)
        .into_iter()
        .find(|mode| head.compositor.use_mode(*mode).is_ok())
}

fn adopt_modes(head: &mut Head, modes: Vec<Mode>) {
    head.config.modes = modes.iter().map(output_mode).collect();
    head.modes = modes;
    head.refused = false;
}

fn wait_for_replug(head: &mut Head) {
    tracing::warn!(
        "{} accepts none of its modes; waiting for it to change",
        head.config.name
    );
    head.refused = true;
}

fn settle_modes(head: &mut Head, modes: Vec<Mode>, changes: &mut HeadChanges) {
    let wanted = head.compositor.pending_mode();
    if first_working_mode(head, &modes, wanted).is_none() {
        wait_for_replug(head);
        return;
    }
    adopt_modes(head, modes);
    changes.refitted.push(head.config.clone());
}

fn refuse_mode(head: &mut Head, refused: Mode, compositor: &mut Compositor) {
    tracing::warn!(
        "{} refused {}x{}; trying its other modes",
        head.config.name,
        refused.size().0,
        refused.size().1
    );
    let modes: Vec<Mode> = head
        .modes
        .iter()
        .filter(|mode| **mode != refused)
        .copied()
        .collect();
    if first_working_mode(head, &modes, refused).is_none() {
        wait_for_replug(head);
        return;
    }
    adopt_modes(head, modes);
    compositor.refit_output(head.config.clone());
}

fn after_refusal(head: &Head) -> Submission {
    if head.refused {
        Submission::Paused
    } else {
        Submission::Unchanged
    }
}

fn cards() -> Result<Vec<PathBuf>, DrmBackendError> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/dev/dri")?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("card"))
        })
        .collect();
    cards.sort();
    Ok(cards)
}

fn shows_connected_display(connectors: &Path) -> bool {
    let read = |connector: &Path, attribute: &str| {
        std::fs::read_to_string(connector.join(attribute)).unwrap_or_default()
    };
    std::fs::read_dir(connectors)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .any(|connector| {
            read(&connector, "status").trim() == "connected"
                && !read(&connector, "modes").trim().is_empty()
        })
}

fn event_time(metadata: Option<DrmEventMetadata>) -> Duration {
    match metadata.map(|metadata| metadata.time) {
        Some(DrmEventTime::Monotonic(time)) => time,
        _ => Duration::from(Clock::<Monotonic>::new().now()),
    }
}

fn connected(device: &DrmDevice) -> Result<Vec<connector::Info>, DrmBackendError> {
    let resources = device.resource_handles()?;
    Ok(resources
        .connectors()
        .iter()
        .filter_map(|&handle| device.get_connector(handle, true).ok())
        .filter(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
        .collect())
}

impl DrmBackend {
    pub fn wait_for_display(after_next_change: bool) -> Result<(), DrmBackendError> {
        let connectors = Path::new("/sys/class/drm");
        let monitor = MonitorBuilder::new()?.match_subsystem("drm")?.listen()?;
        if !after_next_change && shows_connected_display(connectors) {
            return Ok(());
        }
        tracing::info!("waiting for a display to be connected");
        let mut event_loop: EventLoop<bool> = EventLoop::try_new()?;
        event_loop
            .handle()
            .insert_source(
                Generic::new(monitor, Interest::READ, Trigger::Level),
                |_, monitor, connected| {
                    monitor.as_ref().iter().for_each(drop);
                    *connected = shows_connected_display(connectors);
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|inserted| inserted.error)?;
        let mut connected = false;
        while !connected {
            event_loop.dispatch(None, &mut connected)?;
        }
        Ok(())
    }

    pub fn open(
        path: Option<&Path>,
        first_mode: FirstMode,
    ) -> Result<Option<(Self, DrmNotifiers)>, DrmBackendError> {
        let (mut session, session_notifier) = LibSeatSession::new()?;
        let candidates = match path {
            Some(path) => vec![path.to_path_buf()],
            None => cards()?,
        };
        let mut failures = Vec::new();
        for candidate in candidates {
            match Self::open_card(&mut session, &candidate) {
                Ok((device, notifier)) => {
                    let mut backend = Self::with_device(session, candidate, device, first_mode)?;
                    backend.rescan()?;
                    if backend.heads.is_empty() {
                        tracing::warn!(
                            "{} cannot drive any connected display yet",
                            backend.path.display()
                        );
                        return Ok(None);
                    }
                    let notifiers = DrmNotifiers {
                        session: session_notifier,
                        device: notifier,
                    };
                    return Ok(Some((backend, notifiers)));
                }
                Err(error) => failures.push(format!("{}: {error}", candidate.display())),
            }
        }
        tracing::warn!(
            "no DRM device can drive a display yet ({})",
            failures.join("; ")
        );
        Ok(None)
    }

    fn open_card(
        session: &mut LibSeatSession,
        path: &Path,
    ) -> Result<(DrmDevice, DrmDeviceNotifier), DrmBackendError> {
        let fd = session.open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (device, notifier) = DrmDevice::new(fd, false)?;
        if connected(&device)?.is_empty() {
            return Err("no connected connector with a mode".into());
        }
        Ok((device, notifier))
    }

    fn with_device(
        session: LibSeatSession,
        path: PathBuf,
        device: DrmDevice,
        first_mode: FirstMode,
    ) -> Result<Self, DrmBackendError> {
        let fd = device.device_fd().clone();
        let node = DrmNode::from_file(&fd)?;
        let gbm = GbmDevice::new(fd).ok();
        let mut renderer = VulkanRenderer::new(Some(node.dev_id()))?;
        prove_with_gbm(&mut renderer, gbm.as_ref());
        let info = renderer.device_info();
        tracing::info!("composing on {info} with {}", info.extensions.join(", "));
        let feedback_device = match node.node_with_type(NodeType::Render) {
            Some(Ok(render)) => render.dev_id(),
            _ => node.dev_id(),
        };
        let feedback =
            DmabufFeedbackBuilder::new(feedback_device, renderer.dmabuf_formats()).build()?;
        Ok(Self {
            session,
            path,
            device,
            gbm,
            renderer,
            node,
            feedback,
            feedback_device,
            heads: Vec::new(),
            libinput: None,
            kelvin: None,
            first_mode,
        })
    }

    fn free_crtc(
        &self,
        info: &connector::Info,
        on_screen: Option<crtc::Handle>,
    ) -> Result<Option<crtc::Handle>, DrmBackendError> {
        let resources = self.device.resource_handles()?;
        let taken: Vec<crtc::Handle> = self
            .heads
            .iter()
            .map(|head| head.compositor.crtc())
            .collect();
        let free: Vec<crtc::Handle> = info
            .encoders()
            .iter()
            .filter_map(|&encoder| self.device.get_encoder(encoder).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .filter(|crtc| !taken.contains(crtc))
            .collect();
        Ok(on_screen
            .filter(|crtc| free.contains(crtc))
            .or(free.first().copied()))
    }

    fn output_compositor(
        &mut self,
        crtc: crtc::Handle,
        mode: Mode,
        connector: connector::Handle,
        buffers: Buffers,
    ) -> Result<OutputCompositor, DrmBackendError> {
        let surface = self.device.create_surface(crtc, mode, &[connector])?;
        let allocator = match (&self.gbm, buffers) {
            (Some(gbm), Buffers::Tiled | Buffers::Linear) => {
                ScanoutAllocator::Rendered(GbmAllocator::new(
                    gbm.clone(),
                    GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
                ))
            }
            _ => ScanoutAllocator::Copied(DumbAllocator::new(self.device.device_fd().clone())),
        };
        let mut render_formats = self.renderer.render_formats();
        if buffers == Buffers::Linear {
            render_formats = render_formats
                .iter()
                .filter(|format| format.modifier == Modifier::Linear)
                .copied()
                .collect();
        }
        let (width, height) = mode.size();
        Ok(DrmCompositor::new(
            OutputModeSource::Static {
                size: Size::from((i32::from(width), i32::from(height))),
                scale: 1.0.into(),
                transform: smithay::utils::Transform::Normal,
            },
            surface,
            None,
            allocator,
            ScanoutExporter::new(self.gbm.clone(), self.node),
            COLOR_FORMATS,
            render_formats,
            Size::from(CURSOR_SIZE),
            self.gbm.clone(),
        )?)
    }

    fn first_buffers(&self) -> Buffers {
        let tiled = self
            .renderer
            .render_formats()
            .iter()
            .any(|format| format.modifier != Modifier::Linear);
        match self.gbm.is_some() && tiled {
            true => Buffers::Tiled,
            false => Buffers::Dumb,
        }
    }

    fn buffered_compositor(
        &mut self,
        (crtc, mode, connector): (crtc::Handle, Mode, connector::Handle),
        mut buffers: Buffers,
        name: &str,
    ) -> Result<(OutputCompositor, Buffers), DrmBackendError> {
        loop {
            match (
                self.output_compositor(crtc, mode, connector, buffers),
                buffers.next(),
            ) {
                (Ok(compositor), _) => return Ok((compositor, buffers)),
                (Err(error), Some(next)) => {
                    tracing::warn!("{name} uses {next}: {error}");
                    buffers = next;
                }
                (Err(error), None) => return Err(error),
            }
        }
    }

    fn feedback_for(&self, scanout: &FormatSet) -> Result<DmabufFeedback, DrmBackendError> {
        let renderer_formats = self.renderer.dmabuf_formats();
        let scanout: FormatSet = scanout
            .iter()
            .filter(|format| renderer_formats.contains(format))
            .copied()
            .collect();
        Ok(
            DmabufFeedbackBuilder::new(self.feedback_device, renderer_formats)
                .add_preference_tranche(self.feedback_device, Some(TrancheFlags::Scanout), scanout)
                .build()?,
        )
    }

    fn create_head(&mut self, info: &connector::Info) -> Result<Option<Head>, DrmBackendError> {
        let on_screen = current_crtc(&self.device, info.handle())
            .inspect_err(|error| {
                tracing::warn!("cannot read what {} shows: {error}", connector_name(info))
            })
            .ok()
            .flatten();
        let kept_mode = on_screen
            .filter(|_| self.first_mode == FirstMode::OnScreen)
            .and_then(|crtc| crtc.mode());
        let modes = modes_of(info, kept_mode);
        let (Some(mode), Some(crtc)) = (
            modes.first().copied(),
            self.free_crtc(info, on_screen.map(|crtc| crtc.handle()))?,
        ) else {
            return Ok(None);
        };
        let (compositor, buffers) = self.buffered_compositor(
            (crtc, mode, info.handle()),
            self.first_buffers(),
            &connector_name(info),
        )?;
        let scanout_formats = compositor.surface().plane_info().formats.clone();
        let variable_refresh = matches!(
            compositor.vrr_supported(info.handle()),
            Ok(VrrSupport::Supported)
        );
        if self.heads.is_empty() {
            self.feedback = self.feedback_for(&scanout_formats)?;
        }
        Ok(Some(Head {
            connector: info.handle(),
            config: head_config(&self.device, info, &modes),
            modes,
            compositor,
            buffers,
            variable_refresh,
            scanned_out: false,
            dark: false,
            colour: Colour::Unknown,
            tone: None,
            lost_at: None,
            relink_wanted: false,
            flip_pending: false,
            shows_content: false,
            refused: false,
            kept_mode,
            display_profile: edid_of(&self.device, info.handle())
                .as_ref()
                .and_then(crate::colour::edid_profile),
            output_colour: None,
            converting: false,
        }))
    }

    pub fn rescan(&mut self) -> Result<HeadChanges, DrmBackendError> {
        let mut changes = HeadChanges::default();
        if !self.session.is_active() {
            return Ok(changes);
        }
        let present = connected(&self.device)?;
        let mut index = 0;
        while index < self.heads.len() {
            let head = &self.heads[index];
            let info = present.iter().find(|info| info.handle() == head.connector);
            let lost_for = head.lost_at.map_or(Duration::ZERO, |at| at.elapsed());
            let decision = rescanned(
                info.is_some(),
                info.is_some_and(|info| {
                    shows_another_display(&head.config, edid_of(&self.device, info.handle()))
                }),
                lost_for,
            );
            match decision {
                Rescanned::Kept => {
                    let link = link_status(&self.device, head.connector).map(|(_, value)| value);
                    let modes = info
                        .map(|info| modes_of(info, head.kept_mode))
                        .unwrap_or_default();
                    let head = &mut self.heads[index];
                    head.relink_wanted |= link == Some(LINK_BAD);
                    if head.lost_at.take().is_some() || head.relink_wanted {
                        tracing::info!("{} came back", head.config.name);
                        changes.returned.push(head.config.name.clone());
                    }
                    if modes != head.modes || head.refused {
                        settle_modes(head, modes, &mut changes);
                    }
                    index += 1;
                    continue;
                }
                Rescanned::Lost(left) => {
                    self.heads[index].lost_at.get_or_insert_with(Instant::now);
                    changes.look_again_in = Some(left);
                    index += 1;
                    continue;
                }
                Rescanned::Gone => {}
            }
            let head = self.heads.remove(index);
            if let Err(error) = head.compositor.surface().clear() {
                tracing::warn!("cannot switch {} off: {error}", head.config.name);
            }
            changes.disconnected.push(head.config.name);
        }
        for info in present {
            if self
                .heads
                .iter()
                .any(|head| head.connector == info.handle())
            {
                continue;
            }
            match self.create_head(&info) {
                Ok(Some(head)) => {
                    changes.connected.push(head.config.clone());
                    self.heads.push(head);
                }
                Ok(None) => tracing::warn!("{} has no free CRTC", connector_name(&info)),
                Err(error) => tracing::warn!("cannot drive {}: {error}", connector_name(&info)),
            }
        }
        Ok(changes)
    }

    pub fn outputs(&self) -> Vec<OutputConfig> {
        self.heads.iter().map(|head| head.config.clone()).collect()
    }

    pub fn darken_all_but(&mut self, shown: &[String]) -> Result<(), DrmBackendError> {
        if !self.session.is_active() {
            return Ok(());
        }
        for head in &mut self.heads {
            if head.lost_at.is_none() && !shown.contains(&head.config.name) {
                head.darken()?;
            }
        }
        Ok(())
    }

    fn head(&mut self, name: &str) -> Option<&mut Head> {
        self.heads.iter_mut().find(|head| head.config.name == name)
    }

    pub fn use_mode(&mut self, name: &str, wanted: OutputMode) -> Result<bool, DrmBackendError> {
        let Some(head) = self.head(name) else {
            return Ok(false);
        };
        let Some(mode) = head
            .modes
            .iter()
            .find(|mode| output_mode(mode) == wanted)
            .copied()
        else {
            return Ok(false);
        };
        if head.compositor.pending_mode() == mode {
            return Ok(false);
        }
        head.compositor.use_mode(mode)?;
        Ok(true)
    }

    pub fn refuse_output_mode(
        &mut self,
        name: &str,
        refused: OutputMode,
        compositor: &mut Compositor,
    ) {
        let Some(head) = self.head(name) else {
            return;
        };
        if let Some(mode) = head
            .modes
            .iter()
            .find(|mode| output_mode(mode) == refused)
            .copied()
        {
            refuse_mode(head, mode, compositor);
        }
    }

    pub fn relink_wanted(&self, name: &str) -> bool {
        self.heads
            .iter()
            .any(|head| head.config.name == name && head.relink_wanted)
    }

    pub fn device_fd(&self) -> DrmDeviceFd {
        self.device.device_fd().clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn scanout_device(&self) -> ScanoutDevice {
        ScanoutDevice {
            node: self.node,
            feedback: self.feedback.clone(),
            planes: self.renderer.dmabuf_planes(),
        }
    }

    pub fn leave_on_screen(mut self) {
        match self.device.resource_handles() {
            Ok(resources) => {
                for &framebuffer in resources.framebuffers() {
                    if let Err(error) = close_framebuffer(&self.device, framebuffer) {
                        tracing::warn!("the frame on screen ends with this process: {error}");
                        break;
                    }
                }
            }
            Err(error) => tracing::warn!("the frame on screen ends with this process: {error}"),
        }
        self.device.pause();
        for head in self.heads.drain(..) {
            std::mem::forget(head.compositor);
        }
    }

    pub fn session_active(&self) -> bool {
        self.session.is_active()
    }

    pub fn flipped(
        &mut self,
        crtc: crtc::Handle,
        metadata: Option<DrmEventMetadata>,
    ) -> Result<Option<(String, Vblank, bool)>, DrmBackendError> {
        let Some(head) = self
            .heads
            .iter_mut()
            .find(|head| head.compositor.crtc() == crtc)
        else {
            return Ok(None);
        };
        head.flip_pending = false;
        head.compositor.frame_submitted()?;
        if let Some(kelvin) = self.kelvin {
            tint(&self.device, head, kelvin);
        }
        Ok(Some((
            head.config.name.clone(),
            Vblank {
                sequence: metadata.map_or(0, |metadata| u64::from(metadata.sequence)),
                time: event_time(metadata),
            },
            head.shows_content,
        )))
    }

    pub fn input_backend(&mut self) -> Result<LibinputInputBackend, DrmBackendError> {
        let mut libinput =
            Libinput::new_with_udev(LibinputSessionInterface::from(self.session.clone()));
        libinput
            .udev_assign_seat(&self.session.seat())
            .map_err(|()| format!("libinput cannot use seat {}", self.session.seat()))?;
        self.libinput = Some(libinput.clone());
        Ok(LibinputInputBackend::new(libinput))
    }

    pub fn seat(&self) -> String {
        self.session.seat()
    }

    pub fn session_changed(&mut self, event: SessionEvent) -> Result<(), DrmBackendError> {
        match event {
            SessionEvent::PauseSession => {
                self.suspend_input();
                self.device.pause();
                Ok(())
            }
            SessionEvent::ActivateSession => {
                self.device.activate(false)?;
                self.wake()
            }
        }
    }

    pub fn suspend_input(&mut self) {
        if let Some(libinput) = self.libinput.as_mut() {
            libinput.suspend();
        }
    }

    pub fn wake(&mut self) -> Result<(), DrmBackendError> {
        if !self.session.is_active() {
            return Ok(());
        }
        for head in &mut self.heads {
            if let Err(error) = head.compositor.reset_state() {
                tracing::warn!("cannot reset {}: {error}", head.config.name);
            }
            head.tone = None;
            head.dark = false;
            head.flip_pending = false;
        }
        if let Some(libinput) = self.libinput.as_mut() {
            libinput
                .resume()
                .map_err(|()| "libinput cannot resume its devices")?;
        }
        Ok(())
    }
}

fn close_framebuffer(device: &DrmDevice, framebuffer: framebuffer::Handle) -> io::Result<()> {
    #[repr(C)]
    struct CloseFramebuffer {
        id: u32,
        pad: u32,
    }
    let request = CloseFramebuffer {
        id: framebuffer.into(),
        pad: 0,
    };
    let fd = device.device_fd().as_fd().as_raw_fd();
    // SAFETY: DRM_IOCTL_MODE_CLOSEFB reads a struct drm_mode_closefb, which CloseFramebuffer
    // matches field for field and which lives across the call.
    match unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_CLOSEFB, &request) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

fn splash_for(device: &DrmDevice, head: &Head) -> Option<Snapshot> {
    let (width, height) = head.compositor.pending_mode().size();
    let size = (u32::from(width), u32::from(height));
    let millimetres = device
        .get_connector(head.connector, false)
        .ok()
        .and_then(|info| info.size())
        .unwrap_or_default();
    match bande_splash::splash_frame(size, bande_splash::plymouth_scale(size, millimetres)) {
        Ok(frame) => Some(Snapshot {
            size: Size::from((frame.size.0 as i32, frame.size.1 as i32)),
            rgba: frame.premultiplied_rgba,
        }),
        Err(error) => {
            tracing::warn!("cannot draw the start screen: {error}");
            None
        }
    }
}

fn current_crtc(
    device: &DrmDevice,
    connector: connector::Handle,
) -> io::Result<Option<crtc::Info>> {
    let Some(encoder) = device.get_connector(connector, false)?.current_encoder() else {
        return Ok(None);
    };
    let Some(crtc) = device.get_encoder(encoder)?.crtc() else {
        return Ok(None);
    };
    device.get_crtc(crtc).map(Some)
}

enum OnScreen {
    Pixels(Snapshot),
    Buffer(Dmabuf),
}

fn shown_frame(device: &DrmDevice, connector: connector::Handle) -> io::Result<Option<OnScreen>> {
    let Some(framebuffer) = current_crtc(device, connector)?.and_then(|crtc| crtc.framebuffer())
    else {
        return Ok(None);
    };
    let info = device
        .get_planar_framebuffer(framebuffer)
        .map_err(io::Error::other)?;
    let linear = info
        .modifier()
        .is_none_or(|modifier| modifier == Modifier::Linear);
    let readable = matches!(info.pixel_format(), Fourcc::Xrgb8888 | Fourcc::Argb8888);
    let Some(buffer) = info.buffers()[0] else {
        return Ok(None);
    };
    let frame = match linear && readable {
        true => read_buffer(
            device,
            buffer.into(),
            info.size(),
            info.pitches()[0],
            info.offsets()[0],
        )
        .map(|frame| frame.map(OnScreen::Pixels)),
        false => exported_buffer(device, &info).map(|frame| frame.map(OnScreen::Buffer)),
    };
    let mut handles: Vec<_> = info.buffers().into_iter().flatten().collect();
    handles.dedup();
    for handle in handles {
        if let Err(error) = device.close_buffer(handle) {
            tracing::warn!("cannot let go of the frame on screen: {error}");
        }
    }
    frame
}

fn exported_buffer(
    device: &DrmDevice,
    info: &framebuffer::PlanarInfo,
) -> io::Result<Option<Dmabuf>> {
    let Some(modifier) = info.modifier() else {
        return Ok(None);
    };
    let (width, height) = info.size();
    let size = (
        i32::try_from(width).map_err(io::Error::other)?,
        i32::try_from(height).map_err(io::Error::other)?,
    );
    let mut builder = Dmabuf::builder(size, info.pixel_format(), modifier, DmabufFlags::empty());
    let flags = u32::try_from(libc::O_CLOEXEC).map_err(io::Error::other)?;
    for (index, handle) in info.buffers().into_iter().enumerate() {
        let Some(handle) = handle else {
            break;
        };
        let plane = u32::try_from(index).map_err(io::Error::other)?;
        let fd = device.buffer_to_prime_fd(handle, flags)?;
        builder.add_plane(fd, plane, info.offsets()[index], info.pitches()[index]);
    }
    Ok(builder.build())
}

fn read_buffer(
    device: &DrmDevice,
    handle: u32,
    (width, height): (u32, u32),
    pitch: u32,
    offset: u32,
) -> io::Result<Option<Snapshot>> {
    #[repr(C)]
    struct MapDumb {
        handle: u32,
        pad: u32,
        offset: u64,
    }
    let mut request = MapDumb {
        handle,
        pad: 0,
        offset: 0,
    };
    let fd = device.device_fd().as_fd().as_raw_fd();
    // SAFETY: DRM_IOCTL_MODE_MAP_DUMB reads and writes a struct drm_mode_map_dumb, which MapDumb
    // matches field for field and which lives across the call.
    if unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mut request) } != 0 {
        return Ok(None);
    }
    let row = width as usize * 4;
    let length = offset as usize + pitch as usize * height.saturating_sub(1) as usize + row;
    if (pitch as usize) < row || width == 0 || height == 0 {
        return Ok(None);
    }
    let Ok(map_offset) = libc::off_t::try_from(request.offset) else {
        return Ok(None);
    };
    // SAFETY: a fresh read-only shared mapping of the buffer the kernel just offered at
    // map_offset; nothing else in this process refers to it, and it is unmapped below.
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            map_offset,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the mapping above is `length` bytes long and stays mapped until munmap below.
    let bytes = unsafe { std::slice::from_raw_parts(mapped.cast::<u8>(), length) };
    let mut rgba = Vec::with_capacity(row * height as usize);
    for line in 0..height as usize {
        let start = offset as usize + line * pitch as usize;
        for [blue, green, red, _] in bytes[start..start + row].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[*red, *green, *blue, 0xFF]);
        }
    }
    // SAFETY: `mapped` and `length` are the mapping made above, and `bytes` is not used again.
    if unsafe { libc::munmap(mapped, length) } != 0 {
        tracing::warn!(
            "cannot unmap the frame on screen: {}",
            io::Error::last_os_error()
        );
    }
    Ok(Some(Snapshot {
        size: Size::from((width as i32, height as i32)),
        rgba,
    }))
}

fn colour_of(device: &DrmDevice, crtc: crtc::Handle) -> Colour {
    let size = device
        .get_crtc(crtc)
        .map_or(0, |info| info.gamma_length() as usize);
    if size > 0 {
        return Colour::Ramp(size);
    }
    device
        .get_properties(crtc)
        .ok()
        .and_then(|properties| {
            properties
                .as_props_and_values()
                .0
                .iter()
                .copied()
                .find(|handle| {
                    device
                        .get_property(*handle)
                        .is_ok_and(|info| info.name().to_bytes() == b"CTM")
                })
        })
        .map_or(Colour::Fixed, Colour::Matrix)
}

fn set_matrix(
    device: &DrmDevice,
    crtc: crtc::Handle,
    property: property::Handle,
    kelvin: f64,
) -> std::io::Result<()> {
    let blob = device.create_property_blob(&crate::night::matrix(kelvin))?;
    let applied = device.set_property(crtc, property, blob.into());
    device.destroy_property_blob(blob.into())?;
    applied
}

fn tint(device: &DrmDevice, head: &mut Head, kelvin: f64) {
    if head.tone == Some(kelvin) {
        return;
    }
    let crtc = head.compositor.crtc();
    if head.colour == Colour::Unknown {
        head.colour = colour_of(device, crtc);
        tracing::info!("{} tints through {:?}", head.config.name, head.colour);
    }
    let applied = match head.colour {
        Colour::Ramp(size) => {
            let [red, green, blue] = crate::night::ramp(size, kelvin);
            device.set_gamma(crtc, &red, &green, &blue)
        }
        Colour::Matrix(property) => set_matrix(device, crtc, property, kelvin),
        Colour::Unknown | Colour::Fixed => return,
    };
    match applied {
        Ok(()) => head.tone = Some(kelvin),
        Err(error) => {
            tracing::warn!("cannot tint {}: {error}", head.config.name);
            head.colour = Colour::Fixed;
        }
    }
}

impl FrameOutput for DrmBackend {
    type Error = DrmBackendError;

    fn frames_on_screen(&mut self) -> Vec<ShownFrame> {
        let splash_shown = boot_screen_shown();
        let frames: Vec<(String, OnScreen)> = self
            .heads
            .iter()
            .filter_map(|head| {
                let name = &head.config.name;
                match shown_frame(&self.device, head.connector) {
                    Ok(Some(frame)) => return Some((name.clone(), frame)),
                    Ok(None) => {}
                    Err(error) => tracing::warn!("cannot read what {name} shows: {error}"),
                }
                splash_shown
                    .then(|| splash_for(&self.device, head))
                    .flatten()
                    .map(|frame| (name.clone(), OnScreen::Pixels(frame)))
            })
            .collect();
        let context = self.renderer.context_id();
        frames
            .into_iter()
            .filter_map(|(output, frame)| {
                let imported = match frame {
                    OnScreen::Pixels(frame) => self.renderer.import_memory(
                        &frame.rgba,
                        Fourcc::Abgr8888,
                        (frame.size.w, frame.size.h).into(),
                        false,
                    ),
                    OnScreen::Buffer(buffer) => self.renderer.import_dmabuf(&buffer, None),
                };
                imported
                    .inspect_err(|error| {
                        tracing::warn!("cannot fade in over what {output} shows: {error}")
                    })
                    .ok()
                    .map(|texture| ShownFrame {
                        output,
                        texture,
                        context: context.clone(),
                    })
            })
            .collect()
    }

    fn set_color_temperature(&mut self, kelvin: f64) {
        self.kelvin = Some(kelvin);
        if !self.session.is_active() {
            return;
        }
        for head in &mut self.heads {
            tint(&self.device, head, kelvin);
        }
    }

    fn switch_virtual_terminal(&mut self, number: i32) {
        if let Err(error) = self.session.change_vt(number) {
            tracing::warn!("cannot switch to virtual terminal {number}: {error}");
        }
    }

    fn submit(
        &mut self,
        compositor: &mut Compositor,
        output: &str,
    ) -> Result<Submission, DrmBackendError> {
        self.present(compositor, output)
    }

    fn renew_failed_renderer(&mut self) -> Result<bool, DrmBackendError> {
        if !self.renderer.needs_renewal() {
            return Ok(false);
        }
        self.renew_renderer()?;
        Ok(true)
    }

    fn refresh(&self, output: &str) -> Option<Duration> {
        self.heads
            .iter()
            .find(|head| head.config.name == output)
            .map(|head| refresh_of(&head.compositor.pending_mode()))
    }

    fn flip_missed(&mut self, output: &str) {
        let Some(head) = self
            .heads
            .iter_mut()
            .find(|head| head.config.name == output)
        else {
            return;
        };
        head.flip_pending = false;
        if let Err(error) = head.compositor.frame_submitted() {
            tracing::warn!("cannot end the missed flip on {output}: {error}");
        }
    }

    fn restart_display(&mut self) -> Result<(), DrmBackendError> {
        self.renew_renderer()?;
        for head in &mut self.heads {
            if let Err(error) = head.compositor.reset_state() {
                tracing::warn!("cannot reset {}: {error}", head.config.name);
            }
            head.tone = None;
            head.flip_pending = false;
        }
        Ok(())
    }
}

fn prove_with_gbm(renderer: &mut VulkanRenderer, gbm: Option<&GbmDevice<DrmDeviceFd>>) {
    let mut allocator = gbm.map(|gbm| GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING));
    renderer.prove_unexported(|format| {
        let buffer = allocator
            .as_mut()?
            .create_buffer(64, 64, format.code, &[format.modifier])
            .ok()?;
        (buffer.format() == format)
            .then(|| buffer.export().ok())
            .flatten()
    });
}

impl DrmBackend {
    fn fall_back(&mut self, index: usize, next: Buffers) -> Result<(), DrmBackendError> {
        let head = self.heads.remove(index);
        let (crtc, mode) = (head.compositor.crtc(), head.compositor.pending_mode());
        let restore = head.buffers;
        drop(head.compositor);
        let target = (crtc, mode, head.connector);
        let (compositor, buffers) = match self.buffered_compositor(target, next, &head.config.name)
        {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!("{} cannot fall back: {error}", head.config.name);
                (
                    self.output_compositor(crtc, mode, head.connector, restore)?,
                    restore,
                )
            }
        };
        self.heads.insert(
            index,
            Head {
                compositor,
                buffers,
                flip_pending: false,
                tone: None,
                ..head
            },
        );
        Ok(())
    }

    fn renew_renderer(&mut self) -> Result<(), DrmBackendError> {
        tracing::error!(
            "starting the renderer again (lost: {}, holding {})",
            self.renderer.is_lost(),
            self.renderer.memory_use()
        );
        let mut renewed = self.renderer.renew()?;
        prove_with_gbm(&mut renewed, self.gbm.as_ref());
        if renewed.dmabuf_planes() != self.renderer.dmabuf_planes() {
            tracing::warn!(
                "the restarted renderer offers other dma-buf formats than clients were told"
            );
        }
        self.renderer.restore_feedback(&self.feedback);
        self.renderer = renewed;
        for head in &mut self.heads {
            head.compositor.reset_buffers();
        }
        Ok(())
    }

    fn present(
        &mut self,
        compositor: &mut Compositor,
        output: &str,
    ) -> Result<Submission, DrmBackendError> {
        if !self.session.is_active() {
            return Ok(Submission::Paused);
        }
        if compositor.holds_frames() {
            compositor.import_awaited(&mut self.renderer);
            return Ok(Submission::Unchanged);
        }
        let Some(settings) = compositor.output_settings(output) else {
            return Ok(Submission::Unchanged);
        };
        let Some(index) = self
            .heads
            .iter()
            .position(|head| head.config.name == output)
        else {
            return Ok(Submission::Paused);
        };
        let head = &mut self.heads[index];
        if head.lost_at.is_some() || head.refused {
            return Ok(Submission::Paused);
        }
        if !compositor.output_powered(output) {
            head.darken()?;
            return Ok(Submission::Paused);
        }
        if head.dark
            && link_status(&self.device, head.connector).map(|(_, value)| value) == Some(LINK_BAD)
        {
            head.relink_wanted = true;
        }
        head.dark = false;
        if head.relink_wanted {
            if head.flip_pending {
                return Ok(Submission::AwaitingFlip);
            }
            relink(&self.device, head);
        }
        if output_mode(&head.compositor.pending_mode()) != settings.mode
            && let Some(mode) = head
                .modes
                .iter()
                .find(|mode| output_mode(mode) == settings.mode)
                .copied()
            && head.compositor.use_mode(mode).is_err()
        {
            refuse_mode(head, mode, compositor);
            return Ok(after_refusal(head));
        }
        head.compositor
            .set_output_mode_source(OutputModeSource::Static {
                size: settings.mode.size,
                scale: compositor
                    .render_scale(output)
                    .unwrap_or(settings.scale)
                    .into(),
                transform: settings.transform,
            });
        let variable = head.variable_refresh && compositor.wants_variable_refresh(output);
        if variable != head.compositor.vrr_enabled() {
            match head.compositor.use_vrr(variable) {
                Ok(()) => tracing::info!("{output} variable refresh: {variable}"),
                Err(error) => {
                    tracing::warn!("{output} cannot change its refresh to follow frames: {error}");
                    head.variable_refresh = false;
                }
            }
        }
        compositor.refresh_follows_frames(output, head.compositor.vrr_enabled());
        let chosen = compositor.colour_profile(output).map(str::to_owned);
        if head
            .output_colour
            .as_ref()
            .is_none_or(|(current, _)| *current != chosen)
        {
            let conversion =
                crate::colour::conversion(chosen.as_deref(), head.display_profile.as_ref());
            head.output_colour = Some((chosen, conversion));
        }
        let conversion = head
            .output_colour
            .as_ref()
            .and_then(|(_, conversion)| conversion.clone());
        if conversion.is_some() != head.converting {
            head.converting = conversion.is_some();
            head.compositor.reset_buffers();
        }
        self.renderer.set_output_colour(conversion.clone());
        let flags = match conversion {
            Some(_) => FrameFlags::empty(),
            None => FrameFlags::DEFAULT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY,
        };
        let elements = compositor.render_elements_on(&mut self.renderer, output);
        let (rendered, drew_everything) = drawing_everything(|| {
            head.compositor
                .render_frame(&mut self.renderer, &elements, background(), flags)
        });
        let frame = match rendered {
            Ok(frame) => frame,
            Err(error)
                if !self.renderer.is_lost()
                    && !self.renderer.out_of_memory()
                    && let Some(next) = head.buffers.next() =>
            {
                tracing::warn!("{output} uses {next} from now on: {error}");
                self.fall_back(index, next)?;
                return Ok(Submission::Unchanged);
            }
            Err(error) => return Err(error.into()),
        };
        self.renderer.finish_uploads()?;
        head.scanned_out = matches!(frame.primary_element, PrimaryPlaneElement::Element(_));
        let empty = frame.is_empty;
        compositor.copy_captures(&mut self.renderer, output);
        if empty {
            return Ok(Submission::Unchanged);
        }
        head.shows_content = compositor.awaited_shown(&self.renderer, drew_everything);
        match head.compositor.queue_frame(()) {
            Ok(()) => {
                head.flip_pending = true;
                Ok(Submission::AwaitingFlip)
            }
            Err(FrameError::DrmError(DrmError::TestFailed(_))) => {
                let refused = head.compositor.pending_mode();
                refuse_mode(head, refused, compositor);
                Ok(after_refusal(head))
            }
            Err(error) => Err(error.into()),
        }
    }
}
