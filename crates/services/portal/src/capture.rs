use std::collections::HashMap;
use std::path::PathBuf;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use crate::request::Cancellation;
use crate::screencast::{
    CURSOR_EMBEDDED, CURSOR_HIDDEN, MONITOR, ScreenCastBackend, Source, SourceRequest, Stream,
    WINDOW,
};
use crate::screenshot::{Capture, ScreenshotUi};

const MONITOR_PREFIX: &str = "monitor:";

#[zbus::proxy(
    interface = "org.bande.Screenshot",
    default_service = "org.bande.Screenshot",
    default_path = "/org/bande/Screenshot",
    gen_async = false,
    blocking_name = "ScreenshotService"
)]
pub trait Screenshot {
    fn select_sources(&self, app_id: &str, types: u32, multiple: bool)
    -> zbus::Result<Vec<String>>;

    fn resolve_sources(&self, ids: &[String]) -> zbus::Result<Vec<String>>;

    fn start_streams(
        &self,
        ids: &[String],
        cursor_mode: u32,
    ) -> zbus::Result<Vec<(u32, HashMap<String, OwnedValue>)>>;

    fn stop_streams(&self, nodes: &[u32]) -> zbus::Result<()>;

    fn screenshot(&self, app_id: &str, interactive: bool, ask: bool) -> zbus::Result<String>;

    fn pick_color(&self) -> zbus::Result<(bool, (f64, f64, f64))>;

    fn cancel(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn streams_ended(&self, nodes: Vec<u32>) -> zbus::Result<()>;
}

pub struct ScreenshotTool {
    connection: Connection,
}

fn source(id: String) -> Source {
    let kind = match id.starts_with(MONITOR_PREFIX) {
        true => MONITOR,
        false => WINDOW,
    };
    Source { kind, id }
}

fn pair(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<(i32, i32)> {
    properties
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| <(i32, i32)>::try_from(value).ok())
}

impl ScreenshotTool {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    fn proxy(&self) -> Option<ScreenshotService<'_>> {
        ScreenshotService::new(&self.connection)
            .inspect_err(|error| tracing::warn!("cannot reach the screenshot tool: {error}"))
            .ok()
    }

    fn cancel_on(&self, cancellation: &Cancellation) {
        let connection = self.connection.clone();
        cancellation.on_cancel(move || {
            if let Ok(proxy) = ScreenshotService::new(&connection) {
                let _ = proxy.cancel();
            }
        });
    }
}

impl ScreenCastBackend for ScreenshotTool {
    fn source_types(&self) -> u32 {
        MONITOR | WINDOW
    }

    fn cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    }

    fn pick(&self, request: &SourceRequest, cancellation: &Cancellation) -> Option<Vec<Source>> {
        self.cancel_on(cancellation);
        let ids = self
            .proxy()?
            .select_sources(&request.app_id, request.types, request.multiple)
            .inspect_err(|error| tracing::warn!("cannot ask what to share: {error}"))
            .ok()?;
        (!ids.is_empty()).then(|| ids.into_iter().map(source).collect())
    }

    fn resolve(&self, ids: &[String]) -> Option<Vec<Source>> {
        let resolved = self.proxy()?.resolve_sources(ids).ok()?;
        (!resolved.is_empty() && resolved.len() == ids.len())
            .then(|| resolved.into_iter().map(source).collect())
    }

    fn start(&self, sources: &[Source], cursor_mode: u32) -> Result<Vec<Stream>, String> {
        let ids: Vec<String> = sources.iter().map(|source| source.id.clone()).collect();
        let proxy = self
            .proxy()
            .ok_or_else(|| "the screenshot tool is not running".to_owned())?;
        let started = proxy
            .start_streams(&ids, cursor_mode)
            .map_err(|error| error.to_string())?;
        Ok(started
            .into_iter()
            .zip(sources)
            .map(|((node_id, properties), source)| Stream {
                node_id,
                source: source.clone(),
                position: pair(&properties, "position"),
                size: pair(&properties, "size"),
            })
            .collect())
    }

    fn stop(&self, streams: &[Stream]) {
        let nodes: Vec<u32> = streams.iter().map(|stream| stream.node_id).collect();
        if let Some(proxy) = self.proxy()
            && let Err(error) = proxy.stop_streams(&nodes)
        {
            tracing::warn!("cannot stop sharing: {error}");
        }
    }
}

impl ScreenshotUi for ScreenshotTool {
    fn capture(&self, capture: &Capture, cancellation: &Cancellation) -> Option<PathBuf> {
        self.cancel_on(cancellation);
        let path = self
            .proxy()?
            .screenshot(&capture.app_id, capture.interactive, capture.ask_permission)
            .inspect_err(|error| tracing::warn!("cannot take a screenshot: {error}"))
            .ok()?;
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn pick_color(&self, _app_id: &str, cancellation: &Cancellation) -> Option<(f64, f64, f64)> {
        self.cancel_on(cancellation);
        match self.proxy()?.pick_color().ok()? {
            (true, color) => Some(color),
            (false, _) => None,
        }
    }
}
