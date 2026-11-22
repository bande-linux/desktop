use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use accesskit::{Node, NodeId, Role};
use bande_l10n::tr;

use crate::access::{Announcer, window_with};
use crate::label::{Label, LabelStyle, Look};
use crate::session::SHELL;
use crate::state::{Compositor, OutputElement, State};
use bande_graphics::{Scheme, TextRole};
use bande_render::VulkanRenderer;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::utils::{Logical, Point};

const ALERT_STYLE: LabelStyle = LabelStyle {
    role: TextRole::BodyStrong,
    padding: (18.0, 8.0),
    look: Look::Card,
};
const ALERT_MARGIN: f64 = 24.0;
const ALERT_GAP: f64 = 8.0;
const FIRST_ALERT: u64 = 2;

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
    announcer: Announcer,
}

impl Alerts {
    fn announce(&mut self) {
        self.announcer.withdraw();
        if self.alerts.is_empty() {
            return;
        }
        let (ids, nodes): (Vec<NodeId>, Vec<(NodeId, Node)>) = self
            .alerts
            .iter()
            .zip(FIRST_ALERT..)
            .map(|(alert, id)| {
                let mut node = Node::new(Role::Alert);
                node.set_label(alert.message.as_str());
                (NodeId(id), (NodeId(id), node))
            })
            .unzip();
        let focus = ids.first().copied();
        self.announcer.publish(true, || {
            window_with(&tr!("Desktop problems"), ids, nodes, focus)
        });
    }
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
        alerts.announce();
        self.needs_redraw = true;
    }

    pub(crate) fn report_shell_crash_loop(&mut self, executable: &Path) {
        self.raise_alert(
            executable,
            tr!("The taskbar and desktop keep stopping. Press Ctrl+Alt+Esc to log out."),
        );
    }

    pub(crate) fn alert_messages(&self) -> Vec<String> {
        self.alerts
            .alerts
            .iter()
            .map(|alert| alert.message.clone())
            .collect()
    }

    pub(crate) fn shell_failing(&self) -> bool {
        self.alerts
            .alerts
            .iter()
            .any(|alert| alert.executable == Path::new(SHELL))
    }

    pub(crate) fn clear_alert(&mut self, executable: &Path) {
        let alerts = &mut self.alerts;
        let before = alerts.alerts.len();
        alerts.alerts.retain(|alert| alert.executable != executable);
        if alerts.alerts.len() != before {
            alerts.generation += 1;
            alerts.announce();
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
        io::ErrorKind::NotFound => tr!("it is not installed"),
        io::ErrorKind::PermissionDenied => tr!("it may not be run"),
        _ => tr!("of an error: {error}", error = error.to_string()),
    };
    tr!(
        "{name} could not start because {reason}.",
        name = name,
        reason = reason
    )
}
