use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dumb::DumbAllocator;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEventMetadata, DrmEventTime, DrmNode,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::ImportDma;
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::output::OutputModeSource;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, connector, crtc, property,
};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
use smithay::utils::{Clock, DeviceFd, Monotonic, Size};
use smithay::wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder};
use unbound_render::VulkanRenderer;

use crate::clock::{FrameOutput, Submission, Vblank};
use crate::render::background;
use crate::scanout::ScanoutExporter;
use crate::screen::{OutputConfig, OutputMode};
use crate::state::{Headless, REFRESH, ScanoutDevice};

pub type DrmBackendError = Box<dyn Error>;

type OutputCompositor = DrmCompositor<DumbAllocator, ScanoutExporter, (), DrmDeviceFd>;

const CURSOR_SIZE: (u32, u32) = (64, 64);
const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

struct Head {
    connector: connector::Handle,
    config: OutputConfig,
    modes: Vec<Mode>,
    compositor: OutputCompositor,
    scanned_out: bool,
}

pub struct DrmBackend {
    session: LibSeatSession,
    path: PathBuf,
    device: DrmDevice,
    gbm: Option<GbmDevice<DrmDeviceFd>>,
    renderer: VulkanRenderer,
    node: DrmNode,
    feedback: DmabufFeedback,
    heads: Vec<Head>,
    libinput: Option<Libinput>,
}

pub struct DrmNotifiers {
    pub session: LibSeatSessionNotifier,
    pub device: DrmDeviceNotifier,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct HeadChanges {
    pub connected: Vec<OutputConfig>,
    pub disconnected: Vec<String>,
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

fn edid_of(device: &DrmDevice, handle: connector::Handle) -> Option<crate::edid::Edid> {
    let properties = device.get_properties(handle).ok()?;
    properties.iter().find_map(|(property, value)| {
        let info = device.get_property(*property).ok()?;
        if info.name().to_bytes() != b"EDID" {
            return None;
        }
        let property::Value::Blob(blob) = info.value_type().convert_value(*value) else {
            return None;
        };
        crate::edid::parse(&device.get_property_blob(blob).ok()?)
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
        .filter_map(|&handle| device.get_connector(handle, false).ok())
        .filter(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
        .collect())
}

impl DrmBackend {
    pub fn open(path: Option<&Path>) -> Result<(Self, DrmNotifiers), DrmBackendError> {
        let (mut session, session_notifier) = LibSeatSession::new()?;
        let candidates = match path {
            Some(path) => vec![path.to_path_buf()],
            None => cards()?,
        };
        let mut failures = Vec::new();
        for candidate in candidates {
            match Self::open_card(&mut session, &candidate) {
                Ok((device, notifier)) => {
                    let mut backend = Self::with_device(session, candidate, device)?;
                    backend.rescan()?;
                    if backend.heads.is_empty() {
                        return Err("no connected connector with a mode and a CRTC".into());
                    }
                    let notifiers = DrmNotifiers {
                        session: session_notifier,
                        device: notifier,
                    };
                    return Ok((backend, notifiers));
                }
                Err(error) => failures.push(format!("{}: {error}", candidate.display())),
            }
        }
        Err(format!("no usable DRM device ({})", failures.join("; ")).into())
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
        let (device, notifier) = DrmDevice::new(fd, true)?;
        if connected(&device)?.is_empty() {
            return Err("no connected connector with a mode".into());
        }
        Ok((device, notifier))
    }

    fn with_device(
        session: LibSeatSession,
        path: PathBuf,
        device: DrmDevice,
    ) -> Result<Self, DrmBackendError> {
        let fd = device.device_fd().clone();
        let node = DrmNode::from_file(&fd)?;
        let gbm = GbmDevice::new(fd).ok();
        let renderer = VulkanRenderer::new()?;
        let feedback =
            DmabufFeedbackBuilder::new(node.dev_id(), renderer.dmabuf_formats()).build()?;
        Ok(Self {
            session,
            path,
            device,
            gbm,
            renderer,
            node,
            feedback,
            heads: Vec::new(),
            libinput: None,
        })
    }

    fn free_crtc(&self, info: &connector::Info) -> Result<Option<crtc::Handle>, DrmBackendError> {
        let resources = self.device.resource_handles()?;
        let taken: Vec<crtc::Handle> = self
            .heads
            .iter()
            .map(|head| head.compositor.crtc())
            .collect();
        Ok(info
            .encoders()
            .iter()
            .filter_map(|&encoder| self.device.get_encoder(encoder).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .find(|crtc| !taken.contains(crtc)))
    }

    fn create_head(&mut self, info: &connector::Info) -> Result<Option<Head>, DrmBackendError> {
        let modes = sorted_modes(info);
        let (Some(mode), Some(crtc)) = (modes.first().copied(), self.free_crtc(info)?) else {
            return Ok(None);
        };
        let fd = self.device.device_fd().clone();
        let surface = self.device.create_surface(crtc, mode, &[info.handle()])?;
        let scanout_formats = surface.plane_info().formats.clone();
        let renderer_formats = self.renderer.dmabuf_formats();
        let (width, height) = mode.size();
        let compositor = DrmCompositor::new(
            OutputModeSource::Static {
                size: Size::from((i32::from(width), i32::from(height))),
                scale: 1.0.into(),
                transform: smithay::utils::Transform::Normal,
            },
            surface,
            None,
            DumbAllocator::new(fd),
            ScanoutExporter::new(self.gbm.clone(), self.node),
            COLOR_FORMATS,
            renderer_formats.clone(),
            Size::from(CURSOR_SIZE),
            self.gbm.clone(),
        )?;
        if self.heads.is_empty() {
            let scanout: FormatSet = scanout_formats
                .iter()
                .filter(|format| renderer_formats.contains(format))
                .copied()
                .collect();
            self.feedback = DmabufFeedbackBuilder::new(self.node.dev_id(), renderer_formats)
                .add_preference_tranche(self.node.dev_id(), Some(TrancheFlags::Scanout), scanout)
                .build()?;
        }
        Ok(Some(Head {
            connector: info.handle(),
            config: head_config(&self.device, info, &modes),
            modes,
            compositor,
            scanned_out: false,
        }))
    }

    pub fn rescan(&mut self) -> Result<HeadChanges, DrmBackendError> {
        let present = connected(&self.device)?;
        let mut changes = HeadChanges::default();
        let mut index = 0;
        while index < self.heads.len() {
            let still = present
                .iter()
                .any(|info| info.handle() == self.heads[index].connector);
            if still {
                index += 1;
                continue;
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

    pub fn refresh(&self, name: &str) -> Option<Duration> {
        self.heads
            .iter()
            .find(|head| head.config.name == name)
            .map(|head| refresh_of(&head.compositor.pending_mode()))
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
        }
    }

    pub fn scanned_out_directly(&self, name: &str) -> bool {
        self.heads
            .iter()
            .any(|head| head.config.name == name && head.scanned_out)
    }

    pub fn session_active(&self) -> bool {
        self.session.is_active()
    }

    pub fn flipped(
        &mut self,
        crtc: crtc::Handle,
        metadata: Option<DrmEventMetadata>,
    ) -> Result<Option<(String, Vblank)>, DrmBackendError> {
        let Some(head) = self
            .heads
            .iter_mut()
            .find(|head| head.compositor.crtc() == crtc)
        else {
            return Ok(None);
        };
        head.compositor.frame_submitted()?;
        Ok(Some((
            head.config.name.clone(),
            Vblank {
                sequence: metadata.map_or(0, |metadata| u64::from(metadata.sequence)),
                time: event_time(metadata),
            },
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
                if let Some(libinput) = self.libinput.as_mut() {
                    libinput.suspend();
                }
                self.device.pause();
            }
            SessionEvent::ActivateSession => {
                if let Some(libinput) = self.libinput.as_mut() {
                    libinput
                        .resume()
                        .map_err(|()| "libinput cannot resume its devices")?;
                }
                self.device.activate(true)?;
                for head in &mut self.heads {
                    head.compositor.reset_state()?;
                }
            }
        }
        Ok(())
    }
}

impl FrameOutput for DrmBackend {
    type Error = DrmBackendError;

    fn submit(
        &mut self,
        compositor: &mut Headless,
        output: &str,
    ) -> Result<Submission, DrmBackendError> {
        let Some(settings) = compositor.output_settings(output) else {
            return Ok(Submission::AtVblank);
        };
        let Some(index) = self
            .heads
            .iter()
            .position(|head| head.config.name == output)
        else {
            return Ok(Submission::AtVblank);
        };
        let head = &mut self.heads[index];
        head.compositor
            .set_output_mode_source(OutputModeSource::Static {
                size: settings.mode.size,
                scale: settings.scale.into(),
                transform: settings.transform,
            });
        let elements = compositor.render_elements_on(&mut self.renderer, output);
        let frame = head.compositor.render_frame(
            &mut self.renderer,
            &elements,
            background(),
            FrameFlags::DEFAULT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY,
        )?;
        head.scanned_out = matches!(frame.primary_element, PrimaryPlaneElement::Element(_));
        let empty = frame.is_empty;
        compositor.copy_captures(&mut self.renderer, output);
        if empty {
            return Ok(Submission::AtVblank);
        }
        head.compositor.queue_frame(())?;
        Ok(Submission::AwaitingFlip)
    }
}
