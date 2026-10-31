use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::utils::{Logical, Point};
use unbound_graphics::{Scheme, TextRole};
use unbound_render::VulkanRenderer;

use crate::L10N;
use crate::label::{Label, LabelStyle, Look};
use crate::state::{Compositor, OutputElement, State};

const ALERT_STYLE: LabelStyle = LabelStyle {
    role: TextRole::BodyStrong,
    padding: (18.0, 8.0),
    look: Look::Card,
};
const ALERT_MARGIN: f64 = 24.0;
const ALERT_GAP: f64 = 8.0;

struct Alert {
    executable: PathBuf,
    message: String,
}

#[derive(Clone, Copy, PartialEq)]
struct Layout {
    screen: usize,
    width: i32,
    scale: f64,
    scheme: Scheme,
    generation: u64,
}

struct Rendered {
    layout: Layout,
    labels: Vec<Label>,
}

#[derive(Default)]
pub(crate) struct Alerts {
    alerts: Vec<Alert>,
    generation: u64,
    rendered: RefCell<Vec<Rendered>>,
}

impl State {
    fn raise_alert(&mut self, executable: &Path, message: String) {
        let alerts = &mut self.alerts;
        match alerts
            .alerts
            .iter_mut()
            .find(|alert| alert.executable == executable)
        {
            Some(alert) if alert.message == message => return,
            Some(alert) => alert.message.clone_from(&message),
            None => alerts.alerts.push(Alert {
                executable: executable.to_path_buf(),
                message: message.clone(),
            }),
        }
        tracing::error!("{message}");
        alerts.generation += 1;
        self.needs_redraw = true;
    }

    pub(crate) fn clear_alert(&mut self, executable: &Path) {
        let alerts = &mut self.alerts;
        let before = alerts.alerts.len();
        alerts.alerts.retain(|alert| alert.executable != executable);
        if alerts.alerts.len() != before {
            alerts.generation += 1;
            self.needs_redraw = true;
        }
    }

    pub(crate) fn alert_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        if self.alerts.alerts.is_empty() {
            return Vec::new();
        }
        let output = self.screen_rect(screen);
        let layout = Layout {
            screen,
            width: output.size.w,
            scale,
            scheme: self.decorations.scheme,
            generation: self.alerts.generation,
        };
        let mut rendered = self.alerts.rendered.borrow_mut();
        rendered.retain(|entry| entry.layout.generation == layout.generation);
        let index = match rendered
            .iter()
            .position(|entry| entry.layout.screen == screen)
        {
            Some(index) if rendered[index].layout == layout => index,
            found => {
                let width = f64::from(output.size.w) - 2.0 * ALERT_MARGIN;
                let mut labels = self.labels.borrow_mut();
                let entry = Rendered {
                    layout,
                    labels: self
                        .alerts
                        .alerts
                        .iter()
                        .filter_map(|alert| {
                            labels.render(&alert.message, ALERT_STYLE, width, layout.scheme, scale)
                        })
                        .collect(),
                };
                match found {
                    Some(index) => {
                        rendered[index] = entry;
                        index
                    }
                    None => {
                        rendered.push(entry);
                        rendered.len() - 1
                    }
                }
            }
        };
        let mut top = f64::from(output.loc.y) + ALERT_MARGIN;
        let mut elements = Vec::new();
        for label in &rendered[index].labels {
            let location = Point::<f64, Logical>::from((
                f64::from(output.loc.x) + (f64::from(output.size.w) - label.size.w) / 2.0,
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

impl Compositor {
    pub fn report_start_failure(&mut self, executable: &Path, error: &io::Error) {
        self.state
            .raise_alert(executable, start_failure(executable, error));
    }
}

fn start_failure(executable: &Path, error: &io::Error) -> String {
    let name = executable
        .file_name()
        .unwrap_or(executable.as_os_str())
        .to_string_lossy()
        .into_owned();
    let reason = match error.kind() {
        io::ErrorKind::NotFound => L10N.tr("start-failure-missing"),
        io::ErrorKind::PermissionDenied => L10N.tr("start-failure-denied"),
        _ => L10N.tr_args("start-failure-other", [("error", error.to_string().into())]),
    };
    L10N.tr_args(
        "start-failure",
        [("name", name.into()), ("reason", reason.into())],
    )
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::start_failure;

    #[test]
    fn a_start_failure_names_the_program_and_says_why_in_words() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(
            start_failure(Path::new("/usr/bin/unbound-shell"), &missing),
            "unbound-shell could not start because it is not installed."
        );
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(
            start_failure(Path::new("/usr/libexec/unbound-greeter"), &denied),
            "unbound-greeter could not start because it may not be run."
        );
    }
}
