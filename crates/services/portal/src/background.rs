use std::collections::HashMap;
use std::thread;

use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::OBJECT_PATH;
use crate::request::SUCCESS;

const APPS: &str = "org.bande.Apps";
const APPS_PATH: &str = "/org/bande/Apps";
const INTERFACE: &str = "org.freedesktop.impl.portal.Background";
const CHANGED: &str = "RunningApplicationsChanged";
pub const RUNNING: u32 = 1;
pub const ACTIVE: u32 = 2;
pub const ALLOW: u32 = 1;

type Running = HashMap<String, HashMap<String, OwnedValue>>;

pub fn app_states(running: &Running) -> HashMap<String, u32> {
    let mut states: HashMap<String, u32> = HashMap::new();
    for (app_id, details) in running {
        let text = |key: &str| {
            details
                .get(key)
                .and_then(|value| value.try_clone().ok())
                .and_then(|value| String::try_from(value).ok())
        };
        let seats = details
            .get("active-on-seats")
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| Vec::<String>::try_from(value).ok())
            .unwrap_or_default();
        let id = text("sandboxed-app-id").unwrap_or_else(|| app_id.clone());
        let state = if seats.is_empty() { RUNNING } else { ACTIVE };
        let known = states.entry(id).or_insert(state);
        *known = (*known).max(state);
    }
    states
}

pub struct Background {
    session: zbus::Connection,
}

impl Background {
    pub fn serve(session: &Connection) -> zbus::Result<()> {
        let rule = zbus::MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(APPS)?
            .interface(APPS)?
            .member(CHANGED)?
            .path(APPS_PATH)?
            .build();
        let changes = MessageIterator::for_match_rule(rule, session, None)?;
        let emitting = session.clone();
        thread::spawn(move || {
            for _ in changes.flatten() {
                let _ = emitting.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, CHANGED, &());
            }
        });
        session.object_server().at(
            OBJECT_PATH,
            Self {
                session: session.inner().clone(),
            },
        )?;
        Ok(())
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Background")]
impl Background {
    async fn get_app_state(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let running: Running = self
            .session
            .call_method(
                Some(APPS),
                APPS_PATH,
                Some(APPS),
                "GetRunningApplications",
                &(),
            )
            .await?
            .body()
            .deserialize()?;
        Ok(app_states(&running)
            .into_iter()
            .filter_map(|(app_id, state)| {
                Some((app_id, OwnedValue::try_from(Value::from(state)).ok()?))
            })
            .collect())
    }

    fn notify_background(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        _name: String,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let allowed = OwnedValue::try_from(Value::from(ALLOW))
            .into_iter()
            .map(|result| ("result".to_owned(), result))
            .collect();
        (SUCCESS, allowed)
    }

    #[zbus(signal)]
    async fn running_applications_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
