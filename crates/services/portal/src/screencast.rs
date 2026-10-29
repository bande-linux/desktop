use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

pub const MONITOR: u32 = 1;
pub const WINDOW: u32 = 2;
pub const VIRTUAL: u32 = 4;
pub const CURSOR_HIDDEN: u32 = 1;
pub const CURSOR_EMBEDDED: u32 = 2;
pub const CURSOR_METADATA: u32 = 4;
pub const RESTORE_VENDOR: &str = "unbound";
pub const RESTORE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub kind: u32,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRequest {
    pub app_id: String,
    pub types: u32,
    pub multiple: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub node_id: u32,
    pub source: Source,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}

pub trait ScreenCastBackend: Send + Sync {
    fn source_types(&self) -> u32;
    fn cursor_modes(&self) -> u32;
    fn pick(&self, request: &SourceRequest, cancellation: &Cancellation) -> Option<Vec<Source>>;
    fn resolve(&self, ids: &[String]) -> Option<Vec<Source>>;
    fn start(&self, sources: &[Source], cursor_mode: u32) -> Result<Vec<Stream>, String>;
    fn stop(&self, streams: &[Stream]);
}

#[derive(Debug, Clone, Default)]
struct Session {
    app_id: String,
    sources: Option<Vec<Source>>,
    cursor_mode: u32,
    persist_mode: u32,
    streams: Vec<Stream>,
}

type Sessions = Arc<Mutex<HashMap<OwnedObjectPath, Session>>>;
type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;
type RestoreData = (String, u32, OwnedValue);

fn lock(sessions: &Sessions) -> MutexGuard<'_, HashMap<OwnedObjectPath, Session>> {
    sessions.lock().unwrap_or_else(PoisonError::into_inner)
}

fn option<T: TryFrom<OwnedValue>>(options: &Options, key: &str) -> Option<T> {
    options
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
}

fn restored_ids(options: &Options) -> Option<Vec<String>> {
    let (vendor, version, data): RestoreData = option(options, "restore_data")?;
    if vendor != RESTORE_VENDOR || version != RESTORE_VERSION {
        return None;
    }
    let data = match Value::from(data) {
        Value::Value(inner) => *inner,
        other => other,
    };
    Vec::<String>::try_from(data).ok()
}

fn insert(results: &mut Results, key: &str, value: Value<'_>) {
    if let Ok(value) = OwnedValue::try_from(value) {
        results.insert(key.to_owned(), value);
    }
}

pub struct ScreenCast {
    pub backend: Arc<dyn ScreenCastBackend>,
    sessions: Sessions,
}

impl ScreenCast {
    pub fn new(backend: Arc<dyn ScreenCastBackend>) -> Self {
        Self {
            backend,
            sessions: Sessions::default(),
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let path = OwnedObjectPath::from(session_handle.to_owned());
        let exported = server
            .at(
                &path,
                SessionObject {
                    sessions: Arc::clone(&self.sessions),
                    backend: Arc::clone(&self.backend),
                    path: path.clone(),
                },
            )
            .await;
        if !matches!(exported, Ok(true)) {
            return (OTHER, Results::new());
        }
        lock(&self.sessions).insert(
            path,
            Session {
                app_id,
                cursor_mode: CURSOR_HIDDEN,
                ..Session::default()
            },
        );
        (SUCCESS, Results::new())
    }

    async fn select_sources(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let path = OwnedObjectPath::from(session_handle.to_owned());
        let types = option(&options, "types").unwrap_or(MONITOR);
        let cursor_mode = option(&options, "cursor_mode").unwrap_or(CURSOR_HIDDEN);
        let available = self.backend.source_types();
        let known = matches!(
            lock(&self.sessions).get(&path),
            Some(session) if session.app_id == app_id && session.sources.is_none()
        );
        if !known
            || types == 0
            || types & !available != 0
            || cursor_mode.count_ones() != 1
            || cursor_mode & !self.backend.cursor_modes() != 0
        {
            return (OTHER, Results::new());
        }
        let persist_mode = option::<u32>(&options, "persist_mode").unwrap_or(0).min(2);
        let restored = restored_ids(&options).and_then(|ids| self.backend.resolve(&ids));
        let sources = match restored {
            Some(sources) => Some(Some(sources)),
            None => {
                let backend = Arc::clone(&self.backend);
                let request = SourceRequest {
                    app_id,
                    types,
                    multiple: option(&options, "multiple").unwrap_or(false),
                };
                match request::run(server, &handle, move |cancellation| {
                    backend.pick(&request, cancellation)
                })
                .await
                {
                    Ok(picked) => picked,
                    Err(_) => return (OTHER, Results::new()),
                }
            }
        };
        let Some(Some(sources)) = sources else {
            return (CANCELLED, Results::new());
        };
        match lock(&self.sessions).get_mut(&path) {
            Some(session) => {
                session.sources = Some(sources);
                session.cursor_mode = cursor_mode;
                session.persist_mode = persist_mode;
                (SUCCESS, Results::new())
            }
            None => (CANCELLED, Results::new()),
        }
    }

    async fn start(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        _options: Options,
    ) -> (u32, Results) {
        let path = OwnedObjectPath::from(session_handle.to_owned());
        let Some(session) = lock(&self.sessions)
            .get(&path)
            .filter(|session| session.app_id == app_id && session.streams.is_empty())
            .cloned()
        else {
            return (OTHER, Results::new());
        };
        let Some(sources) = session.sources else {
            return (OTHER, Results::new());
        };
        let backend = Arc::clone(&self.backend);
        let cursor_mode = session.cursor_mode;
        let started = {
            let sources = sources.clone();
            blocking::unblock(move || backend.start(&sources, cursor_mode)).await
        };
        let Ok(streams) = started else {
            return (OTHER, Results::new());
        };
        let mut results = Results::new();
        let described: Vec<(u32, HashMap<&str, Value<'_>>)> = streams
            .iter()
            .map(|stream| {
                let mut properties = HashMap::from([
                    ("source_type", Value::from(stream.source.kind)),
                    ("id", Value::from(stream.source.id.clone())),
                ]);
                if let Some(position) = stream.position {
                    properties.insert("position", Value::from(position));
                }
                if let Some(size) = stream.size {
                    properties.insert("size", Value::from(size));
                }
                (stream.node_id, properties)
            })
            .collect();
        insert(&mut results, "streams", Value::from(described));
        insert(
            &mut results,
            "persist_mode",
            Value::from(session.persist_mode),
        );
        if session.persist_mode > 0 {
            let ids: Vec<String> = sources.iter().map(|source| source.id.clone()).collect();
            insert(
                &mut results,
                "restore_data",
                Value::from((RESTORE_VENDOR, RESTORE_VERSION, Value::from(ids))),
            );
        }
        match lock(&self.sessions).get_mut(&path) {
            Some(session) => session.streams = streams,
            None => {
                self.backend.stop(&streams);
                return (CANCELLED, Results::new());
            }
        }
        (SUCCESS, results)
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        self.backend.source_types()
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        self.backend.cursor_modes()
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        5
    }
}

struct SessionObject {
    sessions: Sessions,
    backend: Arc<dyn ScreenCastBackend>,
    path: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    async fn close(&self, #[zbus(object_server)] server: &ObjectServer) {
        let session = lock(&self.sessions).remove(&self.path);
        if let Some(session) = session {
            self.backend.stop(&session.streams);
        }
        let _ = server.remove::<SessionObject, _>(&self.path).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}
