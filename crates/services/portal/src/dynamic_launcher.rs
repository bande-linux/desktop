use std::collections::HashMap;
use std::sync::Arc;

use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::file_chooser::option;
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

pub const APPLICATION: u32 = 1;
pub const WEBAPP: u32 = 2;
const TRUSTED_INSTALLERS: [&str; 4] = [
    "org.gnome.Software",
    "org.gnome.SoftwareDevel",
    "io.elementary.appcenter",
    "org.kde.discover",
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Launcher {
    pub app_id: String,
    pub name: String,
    pub icon: Vec<u8>,
    pub target: Option<String>,
    pub webapp: bool,
    pub editable_name: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmed {
    Named(String),
    Cancelled,
    Unavailable,
}

pub trait LauncherUi: Send + Sync {
    fn confirm(&self, launcher: &Launcher, cancellation: &Cancellation) -> Confirmed;
}

pub struct DynamicLauncher {
    pub ui: Arc<dyn LauncherUi>,
}

fn icon_bytes(icon: &OwnedValue) -> Option<Vec<u8>> {
    let (kind, data) = <(String, OwnedValue)>::try_from(icon.try_clone().ok()?).ok()?;
    let data = match &*data {
        Value::Value(inner) => inner.try_to_owned().ok()?,
        _ => data,
    };
    (kind == "bytes").then(|| Vec::<u8>::try_from(data).ok())?
}

#[zbus::interface(name = "org.freedesktop.impl.portal.DynamicLauncher")]
impl DynamicLauncher {
    #[allow(clippy::too_many_arguments)]
    async fn prepare_install(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        name: String,
        icon_v: OwnedValue,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let Some(icon) = icon_bytes(&icon_v) else {
            tracing::warn!("{app_id} offered a launcher icon that is not an image");
            return (OTHER, Results::new());
        };
        let launcher = Launcher {
            app_id,
            name,
            icon,
            target: option(&options, "target"),
            webapp: option::<u32>(&options, "launcher_type") == Some(WEBAPP),
            editable_name: option(&options, "editable_name").unwrap_or(true),
        };
        let ui = Arc::clone(&self.ui);
        let confirmed = request::run(server, &handle, move |cancellation| {
            ui.confirm(&launcher, cancellation)
        })
        .await;
        match confirmed {
            Ok(Some(Confirmed::Named(name))) if !name.trim().is_empty() => {
                let results = [
                    ("name", OwnedValue::try_from(Value::from(name)).ok()),
                    (
                        "icon",
                        OwnedValue::try_from(Value::Value(Box::new(Value::from(icon_v)))).ok(),
                    ),
                ]
                .into_iter()
                .filter_map(|(key, value)| Some((key.to_owned(), value?)))
                .collect();
                (SUCCESS, results)
            }
            Ok(Some(Confirmed::Cancelled) | None) => (CANCELLED, Results::new()),
            _ => (OTHER, Results::new()),
        }
    }

    fn request_install_token(&self, app_id: String, _options: Options) -> u32 {
        match TRUSTED_INSTALLERS.contains(&app_id.as_str()) {
            true => SUCCESS,
            false => OTHER,
        }
    }

    #[zbus(property)]
    fn supported_launcher_types(&self) -> u32 {
        APPLICATION | WEBAPP
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}
