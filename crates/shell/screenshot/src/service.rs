use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use unbound_inbox::Outbox;
use zbus::blocking::Connection;
use zbus::message::Header;
use zbus::zvariant::{OwnedValue, Value};

use crate::L10N;
use crate::cast::Cast;
use crate::engine::{Engine, Shot, Source};
use crate::indicator::{Activity, Indicator};

pub const BUS_NAME: &str = "org.unbound.Screenshot";
pub const PATH: &str = "/org/unbound/Screenshot";
pub const INTERFACE: &str = "org.unbound.Screenshot";
pub const PORTAL: &str = "org.freedesktop.impl.portal.desktop.unbound";
pub const MONITOR: u32 = 1;
pub const WINDOW: u32 = 2;
pub const CURSOR_EMBEDDED: u32 = 2;
const SCREEN_ID: &str = "monitor:screen";
const WINDOW_PREFIX: &str = "window:";
const THUMBNAIL: u32 = 320;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub kind: u32,
    pub title: String,
    pub app_id: String,
    pub thumbnail: Option<Thumbnail>,
}

pub enum UiRequest {
    Pick {
        app_id: String,
        multiple: bool,
        choices: Vec<Choice>,
        reply: Sender<Option<Vec<String>>>,
    },
    Shot {
        app_id: String,
        interactive: bool,
        ask: bool,
        reply: Sender<Option<PathBuf>>,
    },
    PickColor {
        reply: Sender<Option<(f64, f64, f64)>>,
    },
    Cancel,
}

pub fn source_of(id: &str) -> Option<Source> {
    match id {
        SCREEN_ID => Some(Source::Screen),
        _ => id
            .strip_prefix(WINDOW_PREFIX)
            .map(|identifier| Source::Window(identifier.to_owned())),
    }
}

fn kind_of(id: &str) -> u32 {
    match id {
        SCREEN_ID => MONITOR,
        _ => WINDOW,
    }
}

pub fn thumbnail(shot: &Shot) -> Thumbnail {
    let scale = (THUMBNAIL as f32 / shot.width.max(shot.height).max(1) as f32).min(1.0);
    let width = ((shot.width as f32 * scale).round() as u32).max(1);
    let height = ((shot.height as f32 * scale).round() as u32).max(1);
    let rgba = shot.rgba();
    let source = image::RgbaImage::from_raw(shot.width, shot.height, rgba)
        .unwrap_or_else(|| image::RgbaImage::new(1, 1));
    let scaled = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    Thumbnail {
        width: scaled.width(),
        height: scaled.height(),
        rgba: scaled.into_raw(),
    }
}

struct Running {
    node_id: u32,
    stream: u64,
    _cast: Cast,
}

pub struct Casting {
    engine: Engine,
    running: Mutex<Vec<Running>>,
    indicator: Mutex<Option<Indicator>>,
    next_stream: AtomicU64,
    connection: Mutex<Option<Connection>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Casting {
    pub fn new(engine: Engine) -> Arc<Self> {
        Arc::new(Self {
            engine,
            running: Mutex::new(Vec::new()),
            indicator: Mutex::new(None),
            next_stream: AtomicU64::new(1),
            connection: Mutex::new(None),
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    fn choices(&self, types: u32) -> Vec<Choice> {
        let mut choices = Vec::new();
        if types & MONITOR != 0 {
            choices.push(Choice {
                id: SCREEN_ID.to_owned(),
                kind: MONITOR,
                title: L10N.tr("entire-screen"),
                app_id: String::new(),
                thumbnail: self
                    .engine
                    .shoot(Source::Screen, false)
                    .as_ref()
                    .map(thumbnail),
            });
        }
        if types & WINDOW != 0 {
            for window in self.engine.windows() {
                let thumbnail = self
                    .engine
                    .shoot(Source::Window(window.identifier.clone()), false)
                    .as_ref()
                    .map(thumbnail);
                choices.push(Choice {
                    id: format!("{WINDOW_PREFIX}{}", window.identifier),
                    kind: WINDOW,
                    title: window.title,
                    app_id: window.app_id,
                    thumbnail,
                });
            }
        }
        choices
    }

    fn resolve(&self, ids: &[String]) -> Vec<String> {
        let windows = self.engine.windows();
        ids.iter()
            .filter(|id| match source_of(id) {
                Some(Source::Screen) => true,
                Some(Source::Window(identifier)) => {
                    windows.iter().any(|window| window.identifier == identifier)
                }
                None => false,
            })
            .cloned()
            .collect()
    }

    pub fn start(
        self: &Arc<Self>,
        id: &str,
        cursor: bool,
        activity: Activity,
    ) -> Option<(u32, u32, u32)> {
        let source = source_of(id)?;
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        let (cast, started) = Cast::new(&L10N.tr("stream-name")).ok()?;
        self.engine.stream(stream, source, cursor, cast.sink());
        let Some(started) = Cast::started(&started) else {
            self.engine.stop(stream);
            return None;
        };
        lock(&self.running).push(Running {
            node_id: started.node_id,
            stream,
            _cast: cast,
        });
        let mut indicator = lock(&self.indicator);
        if indicator.is_none() {
            let casting = Arc::downgrade(self);
            let title = match activity {
                Activity::Sharing => L10N.tr("sharing-screen"),
                Activity::Recording => L10N.tr("recording-screen"),
            };
            match Indicator::show(activity, &title, move || {
                if let Some(casting) = casting.upgrade() {
                    casting.stop_all();
                }
            }) {
                Ok(shown) => *indicator = Some(shown),
                Err(error) => tracing::warn!("cannot show the sharing indicator: {error}"),
            }
        }
        Some((started.node_id, started.width, started.height))
    }

    pub fn stop(&self, nodes: &[u32]) {
        let stopped: Vec<Running> = {
            let mut running = lock(&self.running);
            let (stopped, kept) = std::mem::take(&mut *running)
                .into_iter()
                .partition(|running| nodes.contains(&running.node_id));
            *running = kept;
            stopped
        };
        for running in &stopped {
            self.engine.stop(running.stream);
        }
        if lock(&self.running).is_empty() {
            *lock(&self.indicator) = None;
        }
    }

    pub fn stop_all(&self) {
        let nodes: Vec<u32> = lock(&self.running)
            .iter()
            .map(|running| running.node_id)
            .collect();
        self.ended(&nodes);
    }

    pub fn stream_ended(&self, stream: u64) {
        let nodes: Vec<u32> = lock(&self.running)
            .iter()
            .filter(|running| running.stream == stream)
            .map(|running| running.node_id)
            .collect();
        self.ended(&nodes);
    }

    fn ended(&self, nodes: &[u32]) {
        if nodes.is_empty() {
            return;
        }
        self.stop(nodes);
        if let Some(connection) = lock(&self.connection).as_ref()
            && let Err(error) = connection.emit_signal(
                None::<()>,
                PATH,
                INTERFACE,
                "StreamsEnded",
                &(nodes.to_vec(),),
            )
        {
            tracing::warn!("cannot tell the portal that sharing stopped: {error}");
        }
    }
}

struct Service {
    casting: Arc<Casting>,
    ui: Outbox<UiRequest>,
}

type Properties = HashMap<String, OwnedValue>;

fn property(value: Value<'_>) -> Option<OwnedValue> {
    OwnedValue::try_from(value).ok()
}

async fn from_portal(header: &Header<'_>, connection: &zbus::Connection) -> zbus::fdo::Result<()> {
    let sender = header.sender().map(ToString::to_string);
    let owner = zbus::fdo::DBusProxy::new(connection)
        .await?
        .get_name_owner(
            zbus::names::BusName::try_from(PORTAL)
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?,
        )
        .await
        .ok()
        .map(|owner| owner.to_string());
    match sender.is_some() && sender == owner {
        true => Ok(()),
        false => Err(zbus::fdo::Error::AccessDenied(
            "only the desktop portal may capture the screen".to_owned(),
        )),
    }
}

impl Service {
    async fn ask<T: Send + 'static>(
        &self,
        request: impl FnOnce(Sender<Option<T>>) -> UiRequest,
    ) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        if !self.ui.send(request(reply)) {
            return None;
        }
        blocking::unblock(move || answer.recv().ok().flatten()).await
    }
}

#[zbus::interface(name = "org.unbound.Screenshot")]
impl Service {
    async fn select_sources(
        &self,
        app_id: String,
        types: u32,
        multiple: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<Vec<String>> {
        from_portal(&header, connection).await?;
        let casting = Arc::clone(&self.casting);
        let choices = blocking::unblock(move || casting.choices(types)).await;
        Ok(self
            .ask(|reply| UiRequest::Pick {
                app_id,
                multiple,
                choices,
                reply,
            })
            .await
            .unwrap_or_default())
    }

    async fn resolve_sources(
        &self,
        ids: Vec<String>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<Vec<String>> {
        from_portal(&header, connection).await?;
        let casting = Arc::clone(&self.casting);
        Ok(blocking::unblock(move || casting.resolve(&ids)).await)
    }

    async fn start_streams(
        &self,
        ids: Vec<String>,
        cursor_mode: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<Vec<(u32, Properties)>> {
        from_portal(&header, connection).await?;
        let casting = Arc::clone(&self.casting);
        let started = blocking::unblock(move || {
            let mut started = Vec::new();
            for id in &ids {
                match casting.start(id, cursor_mode == CURSOR_EMBEDDED, Activity::Sharing) {
                    Some(stream) => started.push((id.clone(), stream)),
                    None => {
                        let nodes: Vec<u32> =
                            started.iter().map(|(_, (node, _, _))| *node).collect();
                        casting.stop(&nodes);
                        return None;
                    }
                }
            }
            Some(started)
        })
        .await
        .ok_or_else(|| zbus::fdo::Error::Failed("the screen cannot be shared".to_owned()))?;
        Ok(started
            .into_iter()
            .map(|(id, (node_id, width, height))| {
                let size = (
                    i32::try_from(width).unwrap_or(i32::MAX),
                    i32::try_from(height).unwrap_or(i32::MAX),
                );
                let properties = [
                    ("source_type", property(Value::from(kind_of(&id)))),
                    ("id", property(Value::from(id))),
                    ("size", property(Value::from(size))),
                ]
                .into_iter()
                .filter_map(|(key, value)| Some((key.to_owned(), value?)))
                .collect();
                (node_id, properties)
            })
            .collect())
    }

    async fn stop_streams(
        &self,
        nodes: Vec<u32>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        from_portal(&header, connection).await?;
        let casting = Arc::clone(&self.casting);
        blocking::unblock(move || casting.stop(&nodes)).await;
        Ok(())
    }

    async fn screenshot(
        &self,
        app_id: String,
        interactive: bool,
        ask: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        from_portal(&header, connection).await?;
        Ok(self
            .ask(|reply| UiRequest::Shot {
                app_id,
                interactive,
                ask,
                reply,
            })
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default())
    }

    async fn pick_color(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<(bool, (f64, f64, f64))> {
        from_portal(&header, connection).await?;
        Ok(
            match self.ask(|reply| UiRequest::PickColor { reply }).await {
                Some(color) => (true, color),
                None => (false, (0.0, 0.0, 0.0)),
            },
        )
    }

    async fn cancel(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        from_portal(&header, connection).await?;
        self.ui.send(UiRequest::Cancel);
        Ok(())
    }
}

pub fn serve(casting: &Arc<Casting>, ui: Outbox<UiRequest>) -> zbus::Result<Connection> {
    let connection = Connection::session()?;
    connection.object_server().at(
        PATH,
        Service {
            casting: Arc::clone(casting),
            ui,
        },
    )?;
    connection.request_name(BUS_NAME)?;
    *lock(&casting.connection) = Some(connection.clone());
    Ok(connection)
}
