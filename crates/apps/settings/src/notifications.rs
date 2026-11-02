use std::sync::mpsc::{self, Sender};

use bande_notify::client::NotificationCenter;
use zbus::blocking::Connection;

use bande_settings_panes::PaneId;
use bande_toolkit::TextRole;
use bande_toolkit::ui::{Label, SegmentedControl, Toggle, Widget, WidgetId};

use crate::L10N;
use crate::failure::unexpected;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, Intents, SettingsView, intent};
use crate::widgets::{Card, HEADER_SYMBOL, Header, Row, Text, Tone};

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

fn read(rules: &NotificationCenter<'_>) -> zbus::Result<Vec<AppNotifications>> {
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
        let Ok(rules) = NotificationCenter::new(&session) else {
            return;
        };
        for command in receiver {
            if let NotificationCommand::Set(app) = command
                && let Err(error) =
                    rules.set_app_rule(&app.group, app.allow, &app.banners, app.in_center)
            {
                updates.send(unexpected(error));
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
            let mut open = intent(&self.intents, Intent::OpenApp);
            let group = app.group.clone();
            let row = Row::new(
                Text::new(&app.name, Tone::Primary),
                Some(boxed(Label::new("›"))),
            )
            .detail(Text::new(&summary(&app), Tone::Muted).caption())
            .on_activate(move || open(group.clone()));
            rows.push(self.item_row(format!("app:{}", app.group), row));
        }
        rows
    }

    pub(crate) fn app_page(&mut self) -> Option<Widgets> {
        let Some(crate::view::Detail::App(group)) = self.detail.clone() else {
            return None;
        };
        let app = self
            .notification_apps
            .as_ref()?
            .iter()
            .find(|app| app.group == group)?
            .clone();
        let mut page: Widgets = self
            .back_button(PaneId::Notifications)
            .into_iter()
            .collect();
        let ink = self.ui().context().palette.ink;
        let symbol = self.symbol(PaneId::Notifications.symbol(), HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&app.name, Tone::Primary).role(TextRole::Title),
            &summary(&app),
            None,
        )));
        page.push(boxed(Card::new(self.app_rule_rows(&app))));
        Some(page)
    }
}
