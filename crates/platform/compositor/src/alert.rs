use std::cell::RefCell;
use std::io;
use std::path::Path;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::utils::{Logical, Point};
use unbound_render::VulkanRenderer;

use crate::label::{Label, LabelStyle, Labels};
use crate::state::{Headless, OutputElement, State};

const ALERT_STYLE: LabelStyle = LabelStyle {
    text_size: 15.0,
    padding: (18.0, 8.0),
};
const ALERT_MARGIN: f64 = 24.0;
const ALERT_GAP: f64 = 8.0;

#[derive(Default)]
pub(crate) struct Alerts {
    messages: Vec<String>,
    rendered: RefCell<Vec<Label>>,
    labels: RefCell<Labels>,
}

pub(crate) fn start_failure(executable: &Path, error: &io::Error) -> String {
    format!("Cannot start {}: {error}", executable.display())
}

impl State {
    pub(crate) fn raise_alert(&mut self, message: String) {
        if self.alerts.messages.contains(&message) {
            return;
        }
        tracing::error!("{message}");
        self.alerts.messages.push(message);
        self.needs_redraw = true;
    }

    pub(crate) fn alert_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        if self.alerts.messages.is_empty() {
            return Vec::new();
        }
        let output = self.screen_rect(screen);
        let width = f64::from(output.size.w) - 2.0 * ALERT_MARGIN;
        let messages = &self.alerts.messages;
        let mut rendered = self.alerts.rendered.borrow_mut();
        let current = rendered.len() == messages.len()
            && rendered
                .iter()
                .zip(messages)
                .all(|(label, message)| label.text == *message && label.scale == scale);
        if !current {
            let mut labels = self.alerts.labels.borrow_mut();
            *rendered = messages
                .iter()
                .filter_map(|message| labels.render(message, ALERT_STYLE, width, scale))
                .collect();
        }
        let mut top = f64::from(output.location.y) + ALERT_MARGIN;
        let mut elements = Vec::new();
        for label in rendered.iter() {
            let location = Point::<f64, Logical>::from((
                f64::from(output.location.x) + (f64::from(output.size.w) - label.size.w) / 2.0,
                top,
            ))
            .to_physical(scale)
            .to_i32_round::<i32>()
            .to_f64();
            top += label.size.h + ALERT_GAP;
            if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location,
                &label.buffer,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                elements.push(OutputElement::Memory(element));
            }
        }
        elements
    }
}

impl Headless {
    pub fn report_start_failure(&mut self, executable: &Path, error: &io::Error) {
        self.state.raise_alert(start_failure(executable, error));
    }
}
