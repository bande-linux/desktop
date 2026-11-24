use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use bande_render::{HdrContent, set_hdr_content};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::color_management::v1::server::{
    wp_color_management_output_v1::{self, WpColorManagementOutputV1},
    wp_color_management_surface_feedback_v1::{self, WpColorManagementSurfaceFeedbackV1},
    wp_color_management_surface_v1::{self, WpColorManagementSurfaceV1},
    wp_color_manager_v1::{
        self, Feature, Primaries, RenderIntent, TransferFunction, WpColorManagerV1,
    },
    wp_image_description_creator_icc_v1::{self, WpImageDescriptionCreatorIccV1},
    wp_image_description_creator_params_v1::{self, WpImageDescriptionCreatorParamsV1},
    wp_image_description_info_v1::{self, WpImageDescriptionInfoV1},
    wp_image_description_v1::{self, Cause, WpImageDescriptionV1},
};
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum, Weak,
};
use smithay::wayland::compositor::{Cacheable, with_states};

use crate::state::{Compositor, State};

const VERSION: u32 = 1;
const BRIGHTEST: u32 = 10_000;
const REFERENCE_WHITE: u32 = 203;
const SRGB_WHITE: u32 = 80;
const CHROMATICITY: f64 = 1_000_000.0;
const LUMINANCE: f64 = 10_000.0;
const SRGB: [(f64, f64); 4] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)];
const BT2020: [(f64, f64); 4] = [
    (0.708, 0.292),
    (0.170, 0.797),
    (0.131, 0.046),
    (0.3127, 0.3290),
];

#[derive(Default)]
pub(crate) struct ColourManagement {
    global: Option<GlobalId>,
    hdr_surfaces: Vec<Weak<WlSurface>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Description {
    Srgb,
    Pq { max_nits: u32 },
}

impl Description {
    fn content(self) -> Option<HdrContent> {
        match self {
            Self::Srgb => None,
            Self::Pq { max_nits } => Some(HdrContent {
                max_nits: max_nits as f32,
            }),
        }
    }
}

pub(crate) struct DescriptionData {
    description: Option<Description>,
    ours: bool,
}

#[derive(Debug, Default, Clone)]
struct Parameters {
    transfer: Option<TransferFunction>,
    primaries: Option<Primaries>,
    mastering_max: Option<u32>,
    max_cll: Option<u32>,
    max_fall: Option<u32>,
    mastering_primaries: bool,
}

impl Parameters {
    fn description(&self) -> Option<Description> {
        match (self.transfer?, self.primaries?) {
            (TransferFunction::Gamma22, Primaries::Srgb) => Some(Description::Srgb),
            (TransferFunction::St2084Pq, Primaries::Bt2020) => Some(Description::Pq {
                max_nits: self
                    .max_cll
                    .filter(|nits| *nits > 0)
                    .or(self.mastering_max)
                    .unwrap_or(BRIGHTEST)
                    .min(BRIGHTEST),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct SurfaceDescription(Option<Description>);

impl Cacheable for SurfaceDescription {
    fn commit(&mut self, _: &DisplayHandle) -> Self {
        *self
    }

    fn merge_into(self, into: &mut Self, _: &DisplayHandle) {
        *into = self;
    }
}

#[derive(Default)]
struct Managed(AtomicBool);

pub(crate) fn commit(state: &mut State, surface: &WlSurface) {
    let management = &mut state.colour_management;
    if management.global.is_none() && management.hdr_surfaces.is_empty() {
        return;
    }
    let content = with_states(surface, |states| {
        let described = states
            .cached_state
            .get::<SurfaceDescription>()
            .current()
            .0
            .and_then(Description::content);
        set_hdr_content(states, described);
        described
    });
    management
        .hdr_surfaces
        .retain(|known| known.upgrade().is_ok_and(|known| known != *surface));
    if content.is_some() {
        management.hdr_surfaces.push(surface.downgrade());
    }
}

impl State {
    pub(crate) fn offer_colour_management(&mut self) {
        let wanted = self
            .screens
            .iter()
            .any(|screen| screen.hdr && screen.config.hdr);
        match (wanted, self.colour_management.global.take()) {
            (true, None) => {
                self.colour_management.global = Some(
                    self.display_handle
                        .create_global::<State, WpColorManagerV1, ()>(VERSION, ()),
                );
            }
            (true, kept) => self.colour_management.global = kept,
            (false, Some(global)) => self.display_handle.remove_global::<State>(global),
            (false, None) => {}
        }
    }

    fn preferred(&self, output: Option<&Output>) -> Description {
        let shows_hdr = self.screens.iter().any(|screen| {
            screen.hdr && screen.config.hdr && output.is_none_or(|output| screen.output == *output)
        });
        match shows_hdr {
            true => Description::Pq {
                max_nits: BRIGHTEST,
            },
            false => Description::Srgb,
        }
    }
}

impl Compositor {
    pub fn shows_hdr_content(&self, _output: &str) -> bool {
        self.state
            .colour_management
            .hdr_surfaces
            .iter()
            .filter_map(|surface| surface.upgrade().ok())
            .any(|surface| {
                with_renderer_surface_state(&surface, |state| state.buffer().is_some())
                    .unwrap_or(false)
            })
    }

    pub fn redraw_hdr_displays(&mut self) {
        for screen in &mut self.state.screens {
            screen.redraw |= screen.hdr && screen.config.hdr;
        }
    }
}

fn init_description(
    id: New<WpImageDescriptionV1>,
    description: Option<Description>,
    ours: bool,
    data_init: &mut DataInit<'_, State>,
) {
    static IDENTITY: AtomicU32 = AtomicU32::new(1);
    let resource = data_init.init(id, DescriptionData { description, ours });
    match description {
        Some(_) => resource.ready(IDENTITY.fetch_add(1, Ordering::Relaxed)),
        None => resource.failed(
            Cause::Unsupported,
            "only sRGB and PQ in BT.2020 are supported".to_owned(),
        ),
    }
}

impl GlobalDispatch<WpColorManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WpColorManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        manager.supported_intent(RenderIntent::Perceptual);
        manager.supported_feature(Feature::Parametric);
        manager.supported_feature(Feature::SetMasteringDisplayPrimaries);
        manager.supported_tf_named(TransferFunction::Gamma22);
        manager.supported_tf_named(TransferFunction::St2084Pq);
        manager.supported_primaries_named(Primaries::Srgb);
        manager.supported_primaries_named(Primaries::Bt2020);
        manager.done();
    }
}

impl Dispatch<WpColorManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        manager: &WpColorManagerV1,
        request: wp_color_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_color_manager_v1::Request::GetOutput { id, output } => {
                data_init.init(id, output);
            }
            wp_color_manager_v1::Request::GetSurface { id, surface } => {
                let taken = with_states(&surface, |states| {
                    states
                        .data_map
                        .insert_if_missing_threadsafe(Managed::default);
                    states
                        .data_map
                        .get::<Managed>()
                        .is_some_and(|managed| managed.0.swap(true, Ordering::Relaxed))
                });
                data_init.init(id, surface.downgrade());
                if taken {
                    manager.post_error(
                        wp_color_manager_v1::Error::SurfaceExists,
                        "the surface already has a colour management object",
                    );
                }
            }
            wp_color_manager_v1::Request::GetSurfaceFeedback { id, surface } => {
                data_init.init(id, surface.downgrade());
            }
            wp_color_manager_v1::Request::CreateParametricCreator { obj } => {
                data_init.init(obj, Mutex::new(Parameters::default()));
            }
            wp_color_manager_v1::Request::CreateIccCreator { obj } => {
                data_init.init(obj, ());
                unsupported(manager, "ICC descriptions are not supported");
            }
            wp_color_manager_v1::Request::CreateWindowsScrgb { image_description } => {
                init_description(image_description, None, false, data_init);
                unsupported(manager, "scRGB descriptions are not supported");
            }
            _ => {}
        }
    }
}

fn unsupported(manager: &WpColorManagerV1, why: &str) {
    manager.post_error(wp_color_manager_v1::Error::UnsupportedFeature, why);
}

impl Dispatch<WpColorManagementOutputV1, WlOutput> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WpColorManagementOutputV1,
        request: wp_color_management_output_v1::Request,
        output: &WlOutput,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wp_color_management_output_v1::Request::GetImageDescription { image_description } =
            request
        {
            let description = match Output::from_resource(output) {
                Some(output) => state.preferred(Some(&output)),
                None => Description::Srgb,
            };
            init_description(image_description, Some(description), true, data_init);
        }
    }
}

impl Dispatch<WpColorManagementSurfaceV1, Weak<WlSurface>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &WpColorManagementSurfaceV1,
        request: wp_color_management_surface_v1::Request,
        surface: &Weak<WlSurface>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use wp_color_management_surface_v1::{Error, Request};
        let Ok(surface) = surface.upgrade() else {
            if !matches!(request, Request::Destroy) {
                resource.post_error(Error::Inert, "the surface is gone");
            }
            return;
        };
        let describe = |description| {
            with_states(&surface, |states| {
                states.cached_state.get::<SurfaceDescription>().pending().0 = description;
            });
        };
        match request {
            Request::SetImageDescription {
                image_description,
                render_intent,
            } => {
                if render_intent != WEnum::Value(RenderIntent::Perceptual) {
                    resource.post_error(Error::RenderIntent, "the intent is not supported");
                    return;
                }
                match image_description
                    .data::<DescriptionData>()
                    .and_then(|data| data.description)
                {
                    Some(description) => describe(Some(description)),
                    None => {
                        resource.post_error(Error::ImageDescription, "the description failed");
                    }
                }
            }
            Request::UnsetImageDescription => describe(None),
            Request::Destroy => {
                describe(None);
                with_states(&surface, |states| {
                    if let Some(managed) = states.data_map.get::<Managed>() {
                        managed.0.store(false, Ordering::Relaxed);
                    }
                });
            }
            _ => {}
        }
    }
}

impl Dispatch<WpColorManagementSurfaceFeedbackV1, Weak<WlSurface>> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &WpColorManagementSurfaceFeedbackV1,
        request: wp_color_management_surface_feedback_v1::Request,
        surface: &Weak<WlSurface>,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wp_color_management_surface_feedback_v1::{Error, Request};
        let image_description = match request {
            Request::GetPreferred { image_description }
            | Request::GetPreferredParametric { image_description } => image_description,
            _ => return,
        };
        let alive = surface.upgrade().is_ok();
        let preferred = state.preferred(None);
        init_description(image_description, Some(preferred), true, data_init);
        if !alive {
            resource.post_error(Error::Inert, "the surface is gone");
        }
    }
}

impl Dispatch<WpImageDescriptionCreatorIccV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WpImageDescriptionCreatorIccV1,
        request: wp_image_description_creator_icc_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wp_image_description_creator_icc_v1::Request::Create { image_description } = request
        {
            init_description(image_description, None, false, data_init);
        }
    }
}

fn set_once<T>(resource: &WpImageDescriptionCreatorParamsV1, place: &mut Option<T>, value: T) {
    if place.replace(value).is_some() {
        resource.post_error(
            wp_image_description_creator_params_v1::Error::AlreadySet,
            "the parameter was set before",
        );
    }
}

impl Dispatch<WpImageDescriptionCreatorParamsV1, Mutex<Parameters>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &WpImageDescriptionCreatorParamsV1,
        request: wp_image_description_creator_params_v1::Request,
        parameters: &Mutex<Parameters>,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wp_image_description_creator_params_v1::{Error, Request};
        let mut parameters = parameters.lock().unwrap_or_else(PoisonError::into_inner);
        match request {
            Request::Create { image_description } => {
                let complete = parameters.transfer.is_some() && parameters.primaries.is_some();
                init_description(
                    image_description,
                    parameters.description(),
                    false,
                    data_init,
                );
                if !complete {
                    resource.post_error(
                        Error::IncompleteSet,
                        "the transfer function or the primaries are missing",
                    );
                }
            }
            Request::SetTfNamed { tf } => match tf {
                WEnum::Value(tf @ (TransferFunction::Gamma22 | TransferFunction::St2084Pq)) => {
                    set_once(resource, &mut parameters.transfer, tf)
                }
                _ => resource.post_error(Error::InvalidTf, "not an offered transfer function"),
            },
            Request::SetPrimariesNamed { primaries } => match primaries {
                WEnum::Value(named @ (Primaries::Srgb | Primaries::Bt2020)) => {
                    set_once(resource, &mut parameters.primaries, named);
                }
                _ => resource.post_error(Error::InvalidPrimariesNamed, "not offered primaries"),
            },
            Request::SetTfPower { .. }
            | Request::SetPrimaries { .. }
            | Request::SetLuminances { .. } => {
                resource.post_error(Error::UnsupportedFeature, "the parameter is not supported");
            }
            Request::SetMasteringDisplayPrimaries { .. } => {
                if std::mem::replace(&mut parameters.mastering_primaries, true) {
                    resource.post_error(Error::AlreadySet, "the parameter was set before");
                }
            }
            Request::SetMasteringLuminance { min_lum, max_lum } => {
                if u64::from(max_lum) * 10_000 <= u64::from(min_lum) {
                    resource.post_error(
                        Error::InvalidLuminance,
                        "the maximum is not above the minimum",
                    );
                    return;
                }
                set_once(resource, &mut parameters.mastering_max, max_lum);
            }
            Request::SetMaxCll { max_cll } => {
                set_once(resource, &mut parameters.max_cll, max_cll);
            }
            Request::SetMaxFall { max_fall } => {
                set_once(resource, &mut parameters.max_fall, max_fall);
            }
            _ => {}
        }
    }
}

impl Dispatch<WpImageDescriptionV1, DescriptionData> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &WpImageDescriptionV1,
        request: wp_image_description_v1::Request,
        data: &DescriptionData,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wp_image_description_v1::{Error, Request};
        let Request::GetInformation { information } = request else {
            return;
        };
        let information = data_init.init(information, ());
        match data.description {
            Some(description) if data.ours => describe(&information, description),
            Some(_) => resource.post_error(Error::NoInformation, "a client made this description"),
            None => resource.post_error(Error::NotReady, "the description failed"),
        }
    }
}

fn describe(information: &WpImageDescriptionInfoV1, description: Description) {
    let (corners, primaries, transfer, (max, reference)) = match description {
        Description::Srgb => (
            SRGB,
            Primaries::Srgb,
            TransferFunction::Gamma22,
            (SRGB_WHITE, SRGB_WHITE),
        ),
        Description::Pq { max_nits } => (
            BT2020,
            Primaries::Bt2020,
            TransferFunction::St2084Pq,
            (max_nits, REFERENCE_WHITE),
        ),
    };
    let [red, green, blue, white] =
        corners.map(|(x, y)| [x, y].map(|coordinate| (coordinate * CHROMATICITY).round() as i32));
    let minimum = (0.2 * LUMINANCE).round() as u32;
    let minimum = match description {
        Description::Srgb => minimum,
        Description::Pq { .. } => 0,
    };
    information.primaries(
        red[0], red[1], green[0], green[1], blue[0], blue[1], white[0], white[1],
    );
    information.primaries_named(primaries);
    information.tf_named(transfer);
    information.luminances(minimum, max, reference);
    information.target_primaries(
        red[0], red[1], green[0], green[1], blue[0], blue[1], white[0], white[1],
    );
    information.target_luminance(minimum, max);
    information.done();
}

impl Dispatch<WpImageDescriptionInfoV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WpImageDescriptionInfoV1,
        _: wp_image_description_info_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
