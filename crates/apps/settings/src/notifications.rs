use std::sync::mpsc::{self, Sender};

use zbus::blocking::Connection;

use unbound_toolkit::ui::{Label, SegmentedControl, Toggle, Widget, WidgetId};

use crate::L10N;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, Intents, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

type WireApp = (String, String, bool, String, bool);

#[zbus::proxy(
    interface = "org.unbound.NotificationCenter",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait AppRules {
    fn get_apps(&self) -> zbus::Result<Vec<WireApp>>;
    fn set_app_rule(
        &self,
        group: &str,
        allow: bool,
        banners: &str,
        in_center: bool,
    ) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNotifications {
    pub group: String,
    pub name: String,
    pub allow: bool,
    pub banners: String,
    pub in_center: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationCommand {
    Refresh,
    Set(AppNotifications),
}

fn read(rules: &AppRulesProxyBlocking<'_>) -> zbus::Result<Vec<AppNotifications>> {
    let mut apps: Vec<AppNotifications> = rules
        .get_apps()?
        .into_iter()
        .map(
            |(group, name, allow, banners, in_center)| AppNotifications {
                name: if name.is_empty() { group.clone() } else { name },
                group,
                allow,
                banners,
                in_center,
            },
        )
        .collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

pub fn follow_app_notifications(
    session: &Connection,
    updates: Updates,
) -> Sender<NotificationCommand> {
    let (sender, receiver) = mpsc::channel();
    let session = session.clone();
    spawn("notification-apps", move || {
        let Ok(rules) = AppRulesProxyBlocking::new(&session) else {
            return;
        };
        for command in receiver {
            if let NotificationCommand::Set(app) = command
                && let Err(error) =
                    rules.set_app_rule(&app.group, app.allow, &app.banners, app.in_center)
            {
                updates.send(Update::Failed(error.to_string()));
            }
            match read(&rules) {
                Ok(apps) => {
                    if !updates.send(Update::NotificationApps(apps)) {
                        break;
                    }
                }
                Err(error) => tracing::warn!("cannot read the notification apps: {error}"),
            }
        }
    });
    sender
}

const BANNER_STYLES: [&str; 3] = ["temporary", "persistent", "off"];

fn summary(app: &AppNotifications) -> String {
    if !app.allow {
        return L10N.tr("notifications-off");
    }
    let banners = L10N.tr(&format!("banners-{}", app.banners));
    match (app.banners.as_str(), app.in_center) {
        ("off", true) => L10N.tr("center-only"),
        ("off", false) => L10N.tr("notifications-off"),
        (_, true) => L10N.tr_args("banners-and-center", [("banners", banners.into())]),
        (_, false) => L10N.tr_args("banners-only", [("banners", banners.into())]),
    }
}

fn rule_setter(
    intents: &Intents,
    app: &AppNotifications,
    change: fn(&mut AppNotifications, usize),
) -> impl FnMut(usize) + use<> {
    let app = app.clone();
    let intents = std::rc::Rc::clone(intents);
    move |value| {
        let mut changed = app.clone();
        change(&mut changed, value);
        intents.borrow_mut().push(Intent::SetAppRule(changed));
    }
}

impl SettingsView {
    fn rule_row(
        &mut self,
        label: &str,
        control: Box<dyn Widget>,
        focus: WidgetId,
        anchor: String,
    ) -> Box<dyn Widget> {
        self.anchors.push((Anchor::Item(anchor), focus));
        boxed(Row::new(Text::new(label, Tone::Primary), Some(control)))
    }

    fn app_rule_rows(&mut self, app: &AppNotifications) -> Widgets {
        let mut allow = rule_setter(&self.intents, app, |app, on| app.allow = on == 1);
        let toggle = Toggle::new(&L10N.tr("allow-notifications"), app.allow, move |on| {
            allow(usize::from(on))
        });
        let focus = toggle.id();
        let mut rows = vec![self.rule_row(
            &L10N.tr("allow-notifications"),
            boxed(toggle),
            focus,
            format!("allow:{}", app.group),
        )];
        if !app.allow {
            return rows;
        }
        let labels: Vec<String> = BANNER_STYLES
            .iter()
            .map(|style| L10N.tr(&format!("banners-{style}")))
            .collect();
        let selected = BANNER_STYLES.iter().position(|style| *style == app.banners);
        let mut style = rule_setter(&self.intents, app, |app, index| {
            if let Some(style) = BANNER_STYLES.get(index) {
                (*style).clone_into(&mut app.banners);
            }
        });
        let segmented =
            SegmentedControl::new(&L10N.tr("banner-style"), &labels, selected, move |index| {
                style(index)
            });
        let focus = segmented.id();
        rows.push(self.rule_row(
            &L10N.tr("banner-style"),
            boxed(segmented),
            focus,
            format!("banners:{}", app.group),
        ));
        let mut center = rule_setter(&self.intents, app, |app, on| app.in_center = on == 1);
        let toggle = Toggle::new(&L10N.tr("show-in-center"), app.in_center, move |on| {
            center(usize::from(on))
        });
        let focus = toggle.id();
        rows.push(self.rule_row(
            &L10N.tr("show-in-center"),
            boxed(toggle),
            focus,
            format!("center:{}", app.group),
        ));
        rows
    }

    pub(crate) fn notification_app_rows(&mut self) -> Widgets {
        let Some(apps) = self.notification_apps.clone() else {
            return Vec::new();
        };
        if apps.is_empty() {
            return vec![muted_row(&L10N.tr("no-notification-apps"))];
        }
        let mut rows = Vec::new();
        for app in apps {
            let expanded = self.expanded_app.as_deref() == Some(app.group.as_str());
            let mut expand = intent(&self.intents, Intent::ExpandApp);
            let target = (!expanded).then(|| app.group.clone());
            let row = Row::new(
                Text::new(&app.name, Tone::Primary),
                Some(boxed(Label::new(if expanded { "⌄" } else { "›" }))),
            )
            .detail(Text::new(&summary(&app), Tone::Muted).caption())
            .on_activate(move || expand(target.clone()));
            rows.push(self.item_row(format!("app:{}", app.group), row));
            if expanded {
                rows.extend(self.app_rule_rows(&app));
            }
        }
        rows
    }
}
