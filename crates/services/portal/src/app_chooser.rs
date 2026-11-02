use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::file_chooser::option;
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;
type Updates = Arc<Mutex<HashMap<OwnedObjectPath, Sender<Vec<String>>>>>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Offer {
    pub app_id: String,
    pub choices: Vec<String>,
    pub last_choice: Option<String>,
    pub content_type: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    App {
        id: String,
        activation_token: Option<String>,
    },
    Cancelled,
    Unavailable,
}

pub trait AppChooserUi: Send + Sync {
    fn choose(
        &self,
        offer: &Offer,
        updates: Receiver<Vec<String>>,
        cancellation: &Cancellation,
    ) -> Picked;
}

pub struct AppChooser {
    pub ui: Arc<dyn AppChooserUi>,
    updates: Updates,
}

impl AppChooser {
    pub fn new(ui: Arc<dyn AppChooserUi>) -> Self {
        Self {
            ui,
            updates: Updates::default(),
        }
    }
}

fn lock(updates: &Updates) -> MutexGuard<'_, HashMap<OwnedObjectPath, Sender<Vec<String>>>> {
    updates.lock().unwrap_or_else(PoisonError::into_inner)
}

#[zbus::interface(name = "org.freedesktop.impl.portal.AppChooser")]
impl AppChooser {
    async fn choose_application(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        choices: Vec<String>,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let offer = Offer {
            app_id,
            choices,
            last_choice: option(&options, "last_choice"),
            content_type: option(&options, "content_type"),
            location: option(&options, "filename").or_else(|| option(&options, "uri")),
        };
        let (sender, updates) = mpsc::channel();
        let key = OwnedObjectPath::from(handle.to_owned());
        lock(&self.updates).insert(key.clone(), sender);
        let ui = Arc::clone(&self.ui);
        let picked = request::run(server, &handle, move |cancellation| {
            ui.choose(&offer, updates, cancellation)
        })
        .await;
        lock(&self.updates).remove(&key);
        match picked {
            Ok(Some(Picked::App {
                id,
                activation_token,
            })) => {
                let results = [("choice", Some(id)), ("activation_token", activation_token)]
                    .into_iter()
                    .filter_map(|(key, value)| {
                        let value = OwnedValue::try_from(Value::from(value?)).ok()?;
                        Some((key.to_owned(), value))
                    })
                    .collect();
                (SUCCESS, results)
            }
            Ok(Some(Picked::Cancelled) | None) => (CANCELLED, Results::new()),
            Ok(Some(Picked::Unavailable)) | Err(_) => (OTHER, Results::new()),
        }
    }

    fn update_choices(&self, handle: ObjectPath<'_>, choices: Vec<String>) {
        let key = OwnedObjectPath::from(handle.to_owned());
        if let Some(sender) = lock(&self.updates).get(&key) {
            let _ = sender.send(choices);
        }
    }
}
