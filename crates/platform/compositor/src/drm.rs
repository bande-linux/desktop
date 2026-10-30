use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::dumb::DumbAllocator;
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
use smithay::reexports::input::Libinput;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, connector, crtc,
};
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
use smithay::utils::{Clock, DeviceFd, Monotonic, Physical, Raw, Size, Transform};
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

pub struct DrmOutput {
    session: LibSeatSession,
    path: PathBuf,
    device: DrmDevice,
    compositor: OutputCompositor,
    renderer: VulkanRenderer,
    connector: String,
    size: Size<i32, Physical>,
    physical_size_mm: Size<i32, Raw>,
    refresh: Duration,
    node: DrmNode,
    feedback: DmabufFeedback,
    scanned_out: bool,
    libinput: Option<Libinput>,
}

pub struct DrmNotifiers {
    pub session: LibSeatSessionNotifier,
    pub device: DrmDeviceNotifier,
}

struct Chosen {
    connector: connector::Info,
    crtc: crtc::Handle,
    mode: Mode,
}

fn choose_output(device: &DrmDevice) -> Result<Chosen, DrmBackendError> {
    let resources = device.resource_handles()?;
    for &handle in resources.connectors() {
        let info = device.get_connector(handle, false)?;
        if info.state() != connector::State::Connected {
            continue;
        }
        let Some(mode) = info
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied()
        else {
            continue;
        };
        let crtc = info
            .encoders()
            .iter()
            .filter_map(|&encoder| device.get_encoder(encoder).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .next();
        if let Some(crtc) = crtc {
            return Ok(Chosen {
                connector: info,
                crtc,
                mode,
            });
        }
    }
    Err("no connected connector with a mode and a CRTC".into())
}

fn refresh_of(mode: &Mode) -> Duration {
    let pixels_per_frame = u64::from(mode.hsync().2) * u64::from(mode.vsync().2);
    let kilohertz = u64::from(mode.clock());
    if pixels_per_frame == 0 || kilohertz == 0 {
        return REFRESH;
    }
    Duration::from_nanos(pixels_per_frame * 1_000_000 / kilohertz)
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

impl DrmOutput {
    pub fn open(path: Option<&Path>) -> Result<(Self, DrmNotifiers), DrmBackendError> {
        let (mut session, session_notifier) = LibSeatSession::new()?;
        let candidates = match path {
            Some(path) => vec![path.to_path_buf()],
            None => cards()?,
        };
        let mut failures = Vec::new();
        for candidate in candidates {
            match Self::open_card(&mut session, &candidate) {
                Ok((device, notifier, chosen)) => {
                    let output = Self::with_device(session, candidate, device, chosen)?;
                    let notifiers = DrmNotifiers {
                        session: session_notifier,
                        device: notifier,
                    };
                    return Ok((output, notifiers));
                }
                Err(error) => failures.push(format!("{}: {error}", candidate.display())),
            }
        }
        Err(format!("no usable DRM device ({})", failures.join("; ")).into())
    }

    fn open_card(
        session: &mut LibSeatSession,
        path: &Path,
    ) -> Result<(DrmDevice, DrmDeviceNotifier, Chosen), DrmBackendError> {
        let fd = session.open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (device, notifier) = DrmDevice::new(fd, true)?;
        let chosen = choose_output(&device)?;
        Ok((device, notifier, chosen))
    }

    fn with_device(
        session: LibSeatSession,
        path: PathBuf,
        mut device: DrmDevice,
        chosen: Chosen,
    ) -> Result<Self, DrmBackendError> {
        let fd = device.device_fd().clone();
        let node = DrmNode::from_file(&fd)?;
        let surface =
            device.create_surface(chosen.crtc, chosen.mode, &[chosen.connector.handle()])?;
        let scanout_formats = surface.plane_info().formats.clone();
        let gbm = GbmDevice::new(fd.clone()).ok();
        let renderer = VulkanRenderer::new()?;
        let renderer_formats = renderer.dmabuf_formats();
        let (width, height) = chosen.mode.size();
        let size = Size::from((i32::from(width), i32::from(height)));
        let compositor = DrmCompositor::new(
            OutputModeSource::Static {
                size,
                scale: 1.0.into(),
                transform: Transform::Normal,
            },
            surface,
            None,
            DumbAllocator::new(fd.clone()),
            ScanoutExporter::new(gbm.clone(), node),
            COLOR_FORMATS,
            renderer_formats.clone(),
            Size::from(CURSOR_SIZE),
            gbm,
        )?;
        let scanout: FormatSet = scanout_formats
            .iter()
            .filter(|format| renderer_formats.contains(format))
            .copied()
            .collect();
        let feedback = DmabufFeedbackBuilder::new(node.dev_id(), renderer_formats)
            .add_preference_tranche(node.dev_id(), Some(TrancheFlags::Scanout), scanout)
            .build()?;
        let connector = format!(
            "{}-{}",
            chosen.connector.interface().as_str(),
            chosen.connector.interface_id()
        );
        Ok(Self {
            session,
            path,
            device,
            compositor,
            renderer,
            connector,
            size,
            physical_size_mm: chosen
                .connector
                .size()
                .map(|(width, height)| {
                    (
                        i32::try_from(width).unwrap_or(0),
                        i32::try_from(height).unwrap_or(0),
                    )
                })
                .unwrap_or((0, 0))
                .into(),
            refresh: refresh_of(&chosen.mode),
            node,
            feedback,
            scanned_out: false,
            libinput: None,
        })
    }

    pub fn device_fd(&self) -> DrmDeviceFd {
        self.device.device_fd().clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn output_config(&self) -> OutputConfig {
        OutputConfig {
            name: self.connector.clone(),
            make: String::new(),
            model: self.renderer.device_info().name.clone(),
            serial: String::new(),
            physical_size_mm: self.physical_size_mm,
            modes: vec![OutputMode {
                size: self.size,
                refresh: self.refresh,
            }],
        }
    }

    pub fn scanout_device(&self) -> ScanoutDevice {
        ScanoutDevice {
            node: self.node,
            feedback: self.feedback.clone(),
        }
    }

    pub fn refresh(&self) -> Duration {
        self.refresh
    }

    pub fn scanned_out_directly(&self) -> bool {
        self.scanned_out
    }

    pub fn session_active(&self) -> bool {
        self.session.is_active()
    }

    pub fn flipped(
        &mut self,
        metadata: Option<DrmEventMetadata>,
    ) -> Result<Vblank, DrmBackendError> {
        self.compositor.frame_submitted()?;
        Ok(Vblank {
            sequence: metadata.map_or(0, |metadata| u64::from(metadata.sequence)),
            time: event_time(metadata),
        })
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
                self.compositor.reset_state()?;
            }
        }
        Ok(())
    }
}

impl FrameOutput for DrmOutput {
    type Error = DrmBackendError;

    fn submit(
        &mut self,
        compositor: &mut Headless,
        output: &str,
    ) -> Result<Submission, DrmBackendError> {
        let Some(settings) = compositor.output_settings(output) else {
            return Ok(Submission::AtVblank);
        };
        self.compositor
            .set_output_mode_source(OutputModeSource::Static {
                size: self.size,
                scale: settings.scale.into(),
                transform: settings.transform,
            });
        let elements = compositor.render_elements_on(&mut self.renderer, output);
        let frame = self.compositor.render_frame(
            &mut self.renderer,
            &elements,
            background(),
            FrameFlags::DEFAULT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY,
        )?;
        compositor.copy_captures(&mut self.renderer, output);
        self.scanned_out = matches!(frame.primary_element, PrimaryPlaneElement::Element(_));
        if frame.is_empty {
            return Ok(Submission::AtVblank);
        }
        self.compositor.queue_frame(())?;
        Ok(Submission::AwaitingFlip)
    }
}
