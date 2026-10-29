use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::file_chooser::uri;
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub app_id: String,
    pub parent_window: String,
    pub interactive: bool,
    pub ask_permission: bool,
}

pub trait ScreenshotUi: Send + Sync {
    fn capture(&self, capture: &Capture, cancellation: &Cancellation) -> Option<PathBuf>;
    fn pick_color(&self, app_id: &str, cancellation: &Cancellation) -> Option<(f64, f64, f64)>;
}

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

fn flag(options: &Options, key: &str) -> bool {
    options
        .get(key)
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(false)
}

fn answer(outcome: zbus::fdo::Result<Option<Option<Value<'static>>>>, key: &str) -> (u32, Results) {
    match outcome {
        Ok(Some(Some(value))) => match OwnedValue::try_from(value) {
            Ok(value) => (SUCCESS, Results::from([(key.to_owned(), value)])),
            Err(_) => (OTHER, Results::new()),
        },
        Ok(_) => (CANCELLED, Results::new()),
        Err(_) => (OTHER, Results::new()),
    }
}

pub struct Screenshot {
    pub ui: Arc<dyn ScreenshotUi>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let capture = Capture {
            app_id,
            parent_window,
            interactive: flag(&options, "interactive"),
            ask_permission: !flag(&options, "permission_store_checked"),
        };
        let ui = Arc::clone(&self.ui);
        let outcome = request::run(server, &handle, move |cancellation| {
            ui.capture(&capture, cancellation)
                .map(|path| Value::from(uri(&path)))
        })
        .await;
        answer(outcome, "uri")
    }

    async fn pick_color(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        _options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let ui = Arc::clone(&self.ui);
        let outcome = request::run(server, &handle, move |cancellation| {
            ui.pick_color(&app_id, cancellation).map(Value::from)
        })
        .await;
        answer(outcome, "color")
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}
