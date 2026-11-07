use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bande_printers::{JobOptions, PrintClient};
use zbus::blocking::Connection;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{self, ObjectPath, OwnedValue, Value};

use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

const MILLIMETRES_PER_POINT: f64 = 25.4 / 72.0;
const MARGIN_POINTS: f64 = 54.0;
const LATER: &str = "--later";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentSource {
    Later,
    File(PathBuf),
    App,
}

impl DocumentSource {
    pub const FLAG_LATER: &str = LATER;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintRequest {
    pub app_id: String,
    pub parent_window: String,
    pub title: String,
    pub document: DocumentSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintAnswer {
    pub printer: String,
    pub options: JobOptions,
}

impl PrintAnswer {
    pub fn pairs(&self) -> Vec<(String, String)> {
        let ranges = self
            .options
            .pages
            .iter()
            .map(|(first, last)| format!("{first}-{last}"))
            .collect::<Vec<_>>()
            .join(",");
        [
            ("printer", self.printer.clone()),
            ("media", self.options.media.clone()),
            ("landscape", self.options.landscape.to_string()),
            ("sides", self.options.sides.clone()),
            ("color", self.options.color.to_string()),
            ("copies", self.options.copies.to_string()),
            ("pages", ranges),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
    }

    pub fn from_pairs(pairs: &[(String, String)]) -> Option<Self> {
        let get = |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        let pages = get("pages")?
            .split(',')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let (first, last) = part.split_once('-')?;
                Some((first.parse().ok()?, last.parse().ok()?))
            })
            .collect::<Option<Vec<(i32, i32)>>>()?;
        Some(Self {
            printer: get("printer")?.to_owned(),
            options: JobOptions {
                media: get("media")?.to_owned(),
                landscape: get("landscape")? == "true",
                sides: get("sides")?.to_owned(),
                color: get("color")? == "true",
                copies: get("copies")?.parse().ok()?,
                pages,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    Done(Option<PrintAnswer>),
    Cancelled,
    Unavailable,
}

pub trait PrintUi: Send + Sync {
    fn ask(&self, request: &PrintRequest, cancellation: &Cancellation) -> Asked;
}

pub struct Print {
    pub ui: Arc<dyn PrintUi>,
    tokens: Arc<Mutex<HashMap<u32, (String, PrintAnswer)>>>,
    next_token: AtomicU32,
}

impl Print {
    pub fn new(ui: Arc<dyn PrintUi>) -> Self {
        Self {
            ui,
            tokens: Arc::default(),
            next_token: AtomicU32::new(1),
        }
    }
}

fn runtime_copy(document: OwnedFd) -> io::Result<PathBuf> {
    let folder = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("no runtime folder"))?;
    static COPIES: AtomicU32 = AtomicU32::new(0);
    let path = folder.join(format!(
        "bande-print-portal-{}-{}.pdf",
        std::process::id(),
        COPIES.fetch_add(1, Ordering::Relaxed)
    ));
    let mut target = File::create_new(&path)?;
    io::copy(&mut File::from(document), &mut target)?;
    Ok(path)
}

fn submit(title: &str, answer: &PrintAnswer, document: OwnedFd) -> zbus::Result<u32> {
    let session = Connection::session()?;
    PrintClient::new(&session)?.submit(title, &answer.printer, &answer.options, document)
}

fn owned(value: Value<'_>) -> Option<OwnedValue> {
    OwnedValue::try_from(value).ok()
}

fn insert_text(into: &mut Results, key: &str, value: &str) {
    if let Some(value) = owned(Value::from(value)) {
        into.insert(key.to_owned(), value);
    }
}

fn prepared_results(answer: &PrintAnswer, size_points: Option<(f64, f64)>, token: u32) -> Results {
    let mut settings = Results::new();
    let options = &answer.options;
    insert_text(&mut settings, "printer", &answer.printer);
    insert_text(&mut settings, "n-copies", "1");
    insert_text(
        &mut settings,
        "orientation",
        if options.landscape {
            "landscape"
        } else {
            "portrait"
        },
    );
    insert_text(
        &mut settings,
        "duplex",
        match options.sides.as_str() {
            "two-sided-long-edge" => "vertical",
            "two-sided-short-edge" => "horizontal",
            _ => "simplex",
        },
    );
    insert_text(
        &mut settings,
        "use-color",
        if options.color { "yes" } else { "no" },
    );
    if !options.pages.is_empty() {
        insert_text(&mut settings, "print-pages", "ranges");
        let ranges = options
            .pages
            .iter()
            .map(|(first, last)| format!("{}-{}", first - 1, last - 1))
            .collect::<Vec<_>>()
            .join(",");
        insert_text(&mut settings, "page-ranges", &ranges);
    }
    let mut setup = Results::new();
    insert_text(&mut setup, "PPDName", &options.media);
    insert_text(&mut setup, "Name", &options.media);
    insert_text(&mut setup, "DisplayName", &options.media);
    insert_text(
        &mut setup,
        "Orientation",
        if options.landscape {
            "landscape"
        } else {
            "portrait"
        },
    );
    if let Some((width, height)) = size_points {
        for (key, value) in [
            ("Width", width * MILLIMETRES_PER_POINT),
            ("Height", height * MILLIMETRES_PER_POINT),
            ("MarginTop", MARGIN_POINTS * MILLIMETRES_PER_POINT),
            ("MarginBottom", MARGIN_POINTS * MILLIMETRES_PER_POINT),
            ("MarginLeft", MARGIN_POINTS * MILLIMETRES_PER_POINT),
            ("MarginRight", MARGIN_POINTS * MILLIMETRES_PER_POINT),
        ] {
            if let Some(value) = owned(Value::from(value)) {
                setup.insert(key.to_owned(), value);
            }
        }
    }
    let mut results = Results::new();
    if let Some(value) = owned(Value::from(settings)) {
        results.insert("settings".to_owned(), value);
    }
    if let Some(value) = owned(Value::from(setup)) {
        results.insert("page_setup".to_owned(), value);
    }
    if let Some(value) = owned(Value::from(token)) {
        results.insert("token".to_owned(), value);
    }
    results
}

fn media_points(keyword: &str) -> Option<(f64, f64)> {
    let size = keyword.rsplit('_').next()?;
    let (per_unit, numbers) = match (size.strip_suffix("mm"), size.strip_suffix("in")) {
        (Some(numbers), _) => (72.0 / 25.4, numbers),
        (_, Some(numbers)) => (72.0, numbers),
        _ => return None,
    };
    let (width, height) = numbers.split_once('x')?;
    Some((
        width.parse::<f64>().ok()? * per_unit,
        height.parse::<f64>().ok()? * per_unit,
    ))
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Print")]
impl Print {
    #[zbus(name = "Print", out_args("response", "results"))]
    #[allow(clippy::too_many_arguments)]
    async fn print_document(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        fd: zvariant::OwnedFd,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let document: OwnedFd = fd.into();
        let token = options
            .get("token")
            .and_then(|value| u32::try_from(value).ok())
            .filter(|token| *token != 0);
        if let Some(token) = token {
            let prepared = self
                .tokens
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&token)
                .filter(|(owner, _)| *owner == app_id);
            let Some((_, mut answer)) = prepared else {
                return (OTHER, Results::new());
            };
            answer.options.pages.clear();
            answer.options.copies = 1;
            let sent = blocking::unblock(move || submit(&title, &answer, document)).await;
            return match sent {
                Ok(_) => (SUCCESS, Results::new()),
                Err(error) => {
                    tracing::warn!("cannot hand a document to the print service: {error}");
                    (OTHER, Results::new())
                }
            };
        }
        let ui = Arc::clone(&self.ui);
        let outcome = request::run(server, &handle, move |cancellation| {
            let path = runtime_copy(document).map_err(|error| error.to_string())?;
            let asked = ui.ask(
                &PrintRequest {
                    app_id,
                    parent_window,
                    title,
                    document: DocumentSource::File(path.clone()),
                },
                cancellation,
            );
            let _ = std::fs::remove_file(path);
            Ok::<Asked, String>(asked)
        })
        .await;
        match outcome {
            Ok(Some(Ok(Asked::Done(_)))) => (SUCCESS, Results::new()),
            Ok(Some(Ok(Asked::Cancelled)) | None) => (CANCELLED, Results::new()),
            Ok(Some(Ok(Asked::Unavailable))) | Err(_) => (OTHER, Results::new()),
            Ok(Some(Err(error))) => {
                tracing::warn!("cannot keep the document to print: {error}");
                (OTHER, Results::new())
            }
        }
    }

    #[zbus(out_args("response", "results"))]
    #[allow(clippy::too_many_arguments)]
    async fn prepare_print(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        _settings: Options,
        _page_setup: Options,
        _options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let ui = Arc::clone(&self.ui);
        let owner = app_id.clone();
        let outcome = request::run(server, &handle, move |cancellation| {
            ui.ask(
                &PrintRequest {
                    app_id,
                    parent_window,
                    title,
                    document: DocumentSource::Later,
                },
                cancellation,
            )
        })
        .await;
        match outcome {
            Ok(Some(Asked::Done(Some(answer)))) => {
                let token = self.next_token.fetch_add(1, Ordering::Relaxed);
                let results = prepared_results(&answer, media_points(&answer.options.media), token);
                self.tokens
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(token, (owner, answer));
                (SUCCESS, results)
            }
            Ok(Some(Asked::Cancelled) | None) => (CANCELLED, Results::new()),
            _ => (OTHER, Results::new()),
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }
}
