use std::io;
use std::sync::mpsc::{self, Sender};

use unbound_toolkit::ui::{Label, PopUpButton, Widget};
use unbound_xdg::{Apps, config_dirs, data_dirs};

use crate::L10N;
use crate::failure::unexpected;
use crate::follow::{Update, Updates, spawn};
use crate::login::App;
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

pub struct Role {
    pub name: &'static str,
    mimes: &'static [&'static str],
}

pub const ROLES: [Role; 7] = [
    Role {
        name: "web-browser",
        mimes: &[
            "x-scheme-handler/https",
            "x-scheme-handler/http",
            "text/html",
        ],
    },
    Role {
        name: "mail",
        mimes: &["x-scheme-handler/mailto"],
    },
    Role {
        name: "music",
        mimes: &[
            "audio/mpeg",
            "audio/flac",
            "audio/ogg",
            "audio/x-vorbis+ogg",
            "audio/x-opus+ogg",
            "audio/mp4",
            "audio/x-wav",
        ],
    },
    Role {
        name: "video",
        mimes: &[
            "video/mp4",
            "video/x-matroska",
            "video/webm",
            "video/quicktime",
            "video/ogg",
        ],
    },
    Role {
        name: "pictures",
        mimes: &[
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/heif",
            "image/avif",
            "image/tiff",
        ],
    },
    Role {
        name: "pdf",
        mimes: &["application/pdf"],
    },
    Role {
        name: "text",
        mimes: &["text/plain"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultApp {
    pub role: &'static str,
    pub apps: Vec<App>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultAppCommand {
    Refresh,
    Choose(&'static str, String),
}

pub fn default_apps(apps: &Apps) -> Vec<DefaultApp> {
    ROLES
        .iter()
        .map(|role| {
            let main = role.mimes.first().copied().unwrap_or_default();
            DefaultApp {
                role: role.name,
                apps: apps
                    .apps_for(main)
                    .into_iter()
                    .map(|entry| App {
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                    })
                    .collect(),
                current: apps.default_for(main).map(|entry| entry.id.clone()),
            }
        })
        .collect()
}

pub fn choose_default(apps: &mut Apps, role: &str, id: &str) -> io::Result<()> {
    let Some(role) = ROLES.iter().find(|candidate| candidate.name == role) else {
        return Ok(());
    };
    for mime in role.mimes {
        let handler = apps
            .apps_for(mime)
            .into_iter()
            .find(|entry| entry.id == id)
            .cloned();
        if let Some(handler) = handler {
            apps.set_default(mime, &handler)?;
        }
    }
    Ok(())
}

pub fn follow_default_apps(updates: Updates) -> Sender<DefaultAppCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("default-apps", move || {
        let mut apps: Option<Apps> = None;
        for command in receiver {
            if command == DefaultAppCommand::Refresh {
                apps = None;
            }
            let apps = apps.get_or_insert_with(|| Apps::load(data_dirs(), &config_dirs()));
            if let DefaultAppCommand::Choose(role, id) = command
                && let Err(error) = choose_default(apps, role, &id)
            {
                updates.send(unexpected(error));
            }
            if !updates.send(Update::DefaultApps(default_apps(apps))) {
                break;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn default_app_rows(&mut self) -> Widgets {
        let Some(defaults) = self.default_apps.clone() else {
            return vec![muted_row(&L10N.tr("looking-for-apps"))];
        };
        defaults
            .into_iter()
            .map(|role| {
                let label = L10N.tr(&format!("role-{}", role.role));
                if role.apps.is_empty() {
                    return boxed(Row::new(
                        Text::new(&label, Tone::Primary),
                        Some(boxed(Label::new(&L10N.tr("no-app-installed")))),
                    ));
                }
                let ids: Vec<String> = role.apps.iter().map(|app| app.id.clone()).collect();
                let names: Vec<String> = role.apps.iter().map(|app| app.name.clone()).collect();
                let selected = role
                    .current
                    .as_ref()
                    .and_then(|current| ids.iter().position(|id| id == current))
                    .unwrap_or(0);
                let name = role.role;
                let mut choose = intent(&self.intents, move |id| Intent::DefaultApp(name, id));
                let menu = PopUpButton::new(&label, &names, selected, move |index| {
                    if let Some(id) = ids.get(index) {
                        choose(id.clone());
                    }
                });
                self.anchors
                    .push((Anchor::Item(format!("default:{name}")), menu.id()));
                boxed(Row::new(
                    Text::new(&label, Tone::Primary),
                    Some(boxed(menu)),
                ))
            })
            .collect()
    }
}
