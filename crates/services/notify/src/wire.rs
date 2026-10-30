use crate::{Notification, Presentation, Stack, Urgency};

pub type WireNotification = (
    u32,
    String,
    String,
    String,
    String,
    Vec<(String, String)>,
    u8,
    String,
    bool,
    bool,
);
pub type WireStack = (String, Vec<WireNotification>);
pub type WirePresentation = (Vec<WireStack>, Vec<WireStack>, bool);

pub(crate) fn urgency_code(urgency: Urgency) -> u8 {
    match urgency {
        Urgency::Low => 0,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    }
}

pub(crate) fn urgency_of(code: Option<u8>) -> Urgency {
    match code {
        Some(0) => Urgency::Low,
        Some(2) => Urgency::Critical,
        _ => Urgency::Normal,
    }
}

fn notification_to_wire(notification: &Notification) -> WireNotification {
    (
        notification.id,
        notification.app_name.clone(),
        notification.app_icon.clone(),
        notification.summary.clone(),
        notification.body.clone(),
        notification.actions.clone(),
        urgency_code(notification.urgency),
        notification.desktop_entry.clone().unwrap_or_default(),
        notification.resident,
        notification.transient,
    )
}

fn notification_from_wire(wire: WireNotification) -> Notification {
    let (id, app_name, app_icon, summary, body, actions, urgency, entry, resident, transient) =
        wire;
    Notification {
        id,
        app_name,
        app_icon,
        summary,
        body,
        actions,
        urgency: urgency_of(Some(urgency)),
        desktop_entry: Some(entry).filter(|entry| !entry.is_empty()),
        resident,
        transient,
    }
}

fn stacks_to_wire(stacks: &[Stack]) -> Vec<WireStack> {
    stacks
        .iter()
        .map(|stack| {
            (
                stack.group.clone(),
                stack
                    .notifications
                    .iter()
                    .map(notification_to_wire)
                    .collect(),
            )
        })
        .collect()
}

fn stacks_from_wire(stacks: Vec<WireStack>) -> Vec<Stack> {
    stacks
        .into_iter()
        .map(|(group, notifications)| Stack {
            group,
            notifications: notifications
                .into_iter()
                .map(notification_from_wire)
                .collect(),
        })
        .collect()
}

impl Presentation {
    pub fn to_wire(&self) -> WirePresentation {
        (
            stacks_to_wire(&self.banners),
            stacks_to_wire(&self.center),
            self.do_not_disturb,
        )
    }

    pub fn from_wire((banners, center, do_not_disturb): WirePresentation) -> Self {
        Self {
            banners: stacks_from_wire(banners),
            center: stacks_from_wire(center),
            do_not_disturb,
        }
    }
}
