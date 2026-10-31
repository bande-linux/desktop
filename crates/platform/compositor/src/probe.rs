use crate::state::Compositor;
use smithay::backend::renderer::utils::with_renderer_surface_state;

impl Compositor {
    pub fn popup_count(&self) -> usize {
        (0..self.state.screens.len())
            .flat_map(|screen| self.state.stack(screen))
            .filter(|(surface, _)| {
                self.state.popups.find_popup(surface).is_some()
                    && with_renderer_surface_state(surface, |state| state.buffer().is_some())
                        .unwrap_or(false)
            })
            .count()
    }
}
