use std::fmt;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bande_app::choose_save_file;
use bande_l10n::{tr, trn};
use bande_pdf::Picture;
use bande_printers::{PrintClient, ScanCapabilities, ScanOptions, ScanSource};
use bande_toolkit::Image;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Alert, Axis, Button, PopUpButton, Stack, Widget};
use bande_xdg::state_home;
use image::{ImageFormat, ImageReader};
use ring::digest::{SHA256, digest};
use zbus::blocking::Connection;

use crate::follow::{Command, Update};
use crate::pages::{Widgets, boxed, muted_row};
use crate::printers::{PrinterCommand, PrintersState};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Caption, Card, Preview, Row, Text, Tone};
use bande_settings_panes::PaneId;

const PLATEN: &str = "Platen";
const FEEDER: &str = "Feeder";
const DEFAULT_RESOLUTION: u32 = 300;
const PAGE_LIMIT: u64 = 128 << 20;
const PREVIEW_PIXELS: u32 = 720;
const POINTS_PER_INCH: f32 = 72.0;
const STATE_FOLDER: &str = "bande";
const KEPT_FOLDER: &str = "scans";
const NAME_FILE: &str = "name";
const SCANNER_FILE: &str = "scanner";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Png,
    Jpeg,
}

impl Format {
    fn title(self) -> String {
        match self {
            Self::Pdf => tr!("PDF"),
            Self::Png => tr!("PNG picture"),
            Self::Jpeg => tr!("JPEG picture"),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Page {
    jpeg: Arc<Vec<u8>>,
    width: u32,
    height: u32,
    dpi: u32,
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Page({}×{} at {} dpi)",
            self.width, self.height, self.dpi
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scanned {
    pages: Vec<Page>,
    preview: Option<Image>,
    warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScanUpdate {
    Capabilities(String, Result<ScanCapabilities, String>),
    Scanned(String, Result<Scanned, String>),
    Saved(String, Result<Option<String>, String>),
    Kept(String, String, Vec<Page>, Option<Image>),
}

/// Whether the window closes or the app quits once the scanned pages are taken care of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaving {
    Window,
    App,
}

const SAVE: &str = "save";
const DISCARD: &str = "discard";
const CANCEL: &str = "cancel";
const STOP: &str = "stop";
static SCANS: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanIntent {
    Source(usize),
    Color(usize),
    Resolution(usize),
    Format(usize),
    Retry,
    Start,
    Cancel,
    Save,
    Discard,
    ConfirmDiscard,
    KeepPages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Asking,
    Scanning,
    Cancelling,
    Saving,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanState {
    scanner: String,
    name: String,
    capabilities: Option<ScanCapabilities>,
    source: usize,
    color: bool,
    resolution: u32,
    format: Format,
    pages: Vec<Page>,
    preview: Option<Image>,
    busy: Option<Busy>,
    message: Option<String>,
    asking_discard: bool,
    token: u64,
}

impl ScanState {
    fn new(scanner: &str, name: &str) -> Self {
        Self {
            scanner: scanner.to_owned(),
            name: name.to_owned(),
            capabilities: None,
            source: 0,
            color: true,
            resolution: DEFAULT_RESOLUTION,
            format: Format::Pdf,
            pages: Vec::new(),
            preview: None,
            busy: Some(Busy::Asking),
            message: None,
            asking_discard: false,
            token: 0,
        }
    }

    fn formats(&self) -> Vec<Format> {
        match self.pages.len() {
            1 => vec![Format::Pdf, Format::Png, Format::Jpeg],
            _ => vec![Format::Pdf],
        }
    }

    fn source(&self) -> Option<&ScanSource> {
        self.capabilities.as_ref()?.sources.get(self.source)
    }

    fn options(&self) -> Option<ScanOptions> {
        let source = self.source()?;
        Some(ScanOptions {
            source: source.name.clone(),
            color: self.color && source.color,
            resolution: self.resolution,
        })
    }

    fn scanning(&self) -> bool {
        matches!(self.busy, Some(Busy::Scanning | Busy::Cancelling))
    }

    fn discard(&mut self) {
        self.pages.clear();
        self.preview = None;
        self.format = Format::Pdf;
        self.asking_discard = false;
    }
}

fn source_title(source: &str) -> String {
    match source {
        PLATEN => tr!("Flatbed"),
        FEEDER => tr!("Document feeder"),
        other => other.to_owned(),
    }
}

fn nearest(resolutions: &[u32], wanted: u32) -> u32 {
    resolutions
        .iter()
        .copied()
        .min_by_key(|dpi| dpi.abs_diff(wanted))
        .unwrap_or(wanted)
}

fn reason(error: zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(_, Some(message), _) => message,
        zbus::Error::FDO(error) => match *error {
            zbus::fdo::Error::Failed(message) => message,
            other => {
                tracing::warn!("the print service refused to scan: {other}");
                tr!("Couldn’t reach the scanner.")
            }
        },
        other => {
            tracing::warn!("the print service refused to scan: {other}");
            tr!("Couldn’t reach the scanner.")
        }
    }
}

pub fn capabilities(session: &Connection, scanner: &str) -> Update {
    let answer = PrintClient::new(session)
        .and_then(|client| client.scan_capabilities(scanner))
        .map_err(reason);
    Update::Scan(ScanUpdate::Capabilities(scanner.to_owned(), answer))
}

fn kept_folder() -> Option<PathBuf> {
    state_home().map(|state| state.join(STATE_FOLDER).join(KEPT_FOLDER))
}

fn folder_of(scanner: &str) -> Option<PathBuf> {
    let name: String = digest(&SHA256, scanner.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    kept_folder().map(|folder| folder.join(name))
}

fn create_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
}

fn keep_pages(scanner: &str, name: &str, pages: &[Page]) -> std::io::Result<()> {
    let folder = folder_of(scanner).ok_or_else(|| std::io::Error::other("no state folder"))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)?;
    for (file, text) in [(SCANNER_FILE, scanner), (NAME_FILE, name)] {
        let path = folder.join(file);
        if !path.exists() {
            create_private(&path, text.as_bytes())?;
        }
    }
    let first = std::fs::read_dir(&folder)?
        .flatten()
        .filter_map(|entry| page_number(&entry.path()))
        .max()
        .map_or(0, |highest| highest + 1);
    for (number, page) in (first..).zip(pages) {
        create_private(
            &folder.join(format!("{number:04}-{}.jpg", page.dpi)),
            &page.jpeg,
        )?;
    }
    Ok(())
}

fn page_number(path: &Path) -> Option<usize> {
    if path.extension()? != "jpg" {
        return None;
    }
    path.file_stem()?.to_str()?.split_once('-')?.0.parse().ok()
}

fn forget_pages(scanner: &str) {
    let Some(folder) = folder_of(scanner) else {
        return;
    };
    match std::fs::remove_dir_all(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("cannot remove the kept scan {}: {error}", folder.display()),
    }
}

fn kept_scan(folder: &Path) -> std::io::Result<Option<(String, String, Vec<Page>)>> {
    let scanner = std::fs::read_to_string(folder.join(SCANNER_FILE))?;
    let name = std::fs::read_to_string(folder.join(NAME_FILE)).unwrap_or_else(|error| {
        tracing::warn!(
            "cannot read the name of the kept scan {}: {error}",
            folder.display()
        );
        scanner.clone()
    });
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "jpg"))
        .collect();
    files.sort_by_key(|file| page_number(file));
    let mut pages = Vec::new();
    for file in files {
        let dpi = file
            .file_stem()
            .and_then(|stem| stem.to_str()?.split_once('-')?.1.parse().ok())
            .unwrap_or(DEFAULT_RESOLUTION);
        match read_page(File::open(&file)?.into(), dpi) {
            Ok(page) => pages.push(page),
            Err(error) => {
                tracing::warn!("skipping the kept page {}: {error}", file.display());
            }
        }
    }
    Ok((!pages.is_empty()).then_some((scanner, name, pages)))
}

/// The pages scanned earlier and not saved or discarded yet, one update per scanner.
pub fn kept() -> Vec<Update> {
    let Some(folder) = kept_folder() else {
        return Vec::new();
    };
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(
                "cannot read the kept scans in {}: {error}",
                folder.display()
            );
            return Vec::new();
        }
    };
    entries
        .flatten()
        .filter_map(|entry| match kept_scan(&entry.path()) {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(
                    "cannot read the kept scan {}: {error}",
                    entry.path().display()
                );
                None
            }
        })
        .map(|(scanner, name, pages)| {
            let preview = pages
                .last()
                .and_then(|page| Image::decode(&page.jpeg, PREVIEW_PIXELS));
            Update::Scan(ScanUpdate::Kept(scanner, name, pages, preview))
        })
        .collect()
}

fn read_page(page: std::os::fd::OwnedFd, dpi: u32) -> Result<Page, String> {
    let unreadable = |error: &dyn fmt::Display| {
        tracing::warn!("cannot read a scanned page: {error}");
        tr!("The scanned page couldn’t be read.")
    };
    let mut jpeg = Vec::new();
    File::from(page)
        .take(PAGE_LIMIT)
        .read_to_end(&mut jpeg)
        .map_err(|error| unreadable(&error))?;
    let (width, height) = ImageReader::with_format(Cursor::new(&jpeg), ImageFormat::Jpeg)
        .into_dimensions()
        .map_err(|error| unreadable(&error))?;
    Ok(Page {
        jpeg: Arc::new(jpeg),
        width,
        height,
        dpi,
    })
}

pub fn scan(
    session: &Connection,
    scanner: &str,
    name: &str,
    token: u64,
    options: &ScanOptions,
) -> Update {
    let scanned = PrintClient::new(session)
        .and_then(|client| client.scan(scanner, token, options))
        .map_err(reason)
        .and_then(|pages| {
            let pages = pages
                .into_iter()
                .map(|(page, dpi)| read_page(page, dpi))
                .collect::<Result<Vec<_>, _>>()?;
            let warning = keep_pages(scanner, name, &pages).err().map(|error| {
                tracing::warn!("cannot keep the scanned pages until they are saved: {error}");
                tr!("These pages couldn’t be kept on disk, so they are lost if Settings closes before they are saved.")
            });
            let preview = pages
                .last()
                .and_then(|page| Image::decode(&page.jpeg, PREVIEW_PIXELS));
            Ok(Scanned {
                pages,
                preview,
                warning,
            })
        });
    Update::Scan(ScanUpdate::Scanned(scanner.to_owned(), scanned))
}

fn encoded(pages: &[Page], format: Format) -> Result<Vec<u8>, String> {
    let failed = |error: &dyn fmt::Display| {
        tracing::warn!("cannot save a scan: {error}");
        tr!("Couldn’t save the scan.")
    };
    match (format, pages) {
        (Format::Jpeg, [page]) => Ok(page.jpeg.to_vec()),
        (Format::Png, [page]) => {
            let picture = image::load_from_memory_with_format(&page.jpeg, ImageFormat::Jpeg)
                .map_err(|error| failed(&error))?;
            let mut png = Cursor::new(Vec::new());
            picture
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|error| failed(&error))?;
            Ok(png.into_inner())
        }
        _ => {
            let pictures: Vec<Picture> = pages
                .iter()
                .map(|page| {
                    let points = |pixels: u32| pixels as f32 * POINTS_PER_INCH / page.dpi as f32;
                    Picture {
                        jpeg: Arc::clone(&page.jpeg),
                        size: (points(page.width), points(page.height)),
                    }
                })
                .collect();
            bande_pdf::pictures(&tr!(context = "document", "Scan"), &pictures)
                .map_err(|error| failed(&error))
        }
    }
}

pub fn cancel(session: &Connection, scanner: &str, token: u64) {
    let cancelled = PrintClient::new(session).and_then(|client| client.cancel_scan(scanner, token));
    if let Err(error) = cancelled {
        tracing::warn!("cannot stop a scan: {error}");
    }
}

pub fn save(session: &Connection, scanner: &str, pages: &[Page], format: Format) -> Update {
    let name = format!(
        "{}.{}",
        tr!(context = "document", "Scan"),
        format.extension()
    );
    let saved = match choose_save_file(session, &tr!("Save Scan"), &name) {
        Ok(Some(path)) => write(&path, pages, format).map(|()| {
            Some(
                path.file_name()
                    .map_or(name, |file| file.to_string_lossy().into_owned()),
            )
        }),
        Ok(None) => Ok(None),
        Err(error) => {
            tracing::warn!("cannot choose where to save a scan: {error}");
            Err(tr!("Couldn’t save the scan."))
        }
    };
    if let Ok(Some(_)) = saved {
        forget_pages(scanner);
    }
    Update::Scan(ScanUpdate::Saved(scanner.to_owned(), saved))
}

fn write(path: &Path, pages: &[Page], format: Format) -> Result<(), String> {
    let bytes = encoded(pages, format)?;
    std::fs::write(path, bytes).map_err(|error| {
        tracing::warn!("cannot save a scan to {}: {error}", path.display());
        tr!("Couldn’t save the scan.")
    })
}

impl SettingsView {
    /// How many scanned pages have not been saved yet.
    pub fn unsaved_pages(&self) -> usize {
        self.scans.values().map(|scan| scan.pages.len()).sum()
    }

    /// Asks what to do with the scanned pages not saved yet before leaving, and answers whether
    /// there is nothing to ask about.
    /// Whether a scan is running or scanned pages wait to be saved.
    pub fn has_scan_work(&self) -> bool {
        self.unsaved_pages() > 0 || self.scans.values().any(ScanState::scanning)
    }

    pub fn may_leave(&mut self, leaving: Leaving) -> bool {
        let scanning = self.scans.values().any(ScanState::scanning);
        let unsaved = self.unsaved_pages();
        if !scanning && unsaved == 0 {
            return true;
        }
        if self.leaving.is_some() || self.leaving_after_scan.is_some() {
            return false;
        }
        if self.ui.has_sheet() {
            self.leaving_after_sheet = Some(leaving);
            return false;
        }
        self.leaving = Some(leaving);
        let mut answer = intent(&self.intents, Intent::UnsavedScans);
        if scanning {
            let alert = Alert::new(
                &tr!("A scan is in progress"),
                &tr!("The pages scanned so far are kept."),
                move |choice| answer(choice.to_owned()),
            )
            .destructive(&tr!("Cancel Scan and Close"), STOP)
            .cancel(&tr!("Keep Scanning"), CANCEL);
            self.ui.present_sheet(alert);
            self.invalidate();
            return false;
        }
        let alert = Alert::new(
            &trn!(
                "Save the scanned page?",
                "Save the {count} scanned pages?",
                count = unsaved
            ),
            &tr!("They will be lost if you don’t save them."),
            move |choice| answer(choice.to_owned()),
        )
        .primary(&tr!("Save…"), SAVE)
        .other(&tr!("Discard"), DISCARD)
        .cancel(&tr!("Cancel"), CANCEL);
        self.ui.present_sheet(alert);
        self.invalidate();
        false
    }

    pub(crate) fn unsaved_scans(&mut self, choice: &str) {
        self.ui.close_sheet();
        let Some(leaving) = self.leaving else {
            return;
        };
        match choice {
            SAVE => {
                let first = self
                    .scans
                    .iter()
                    .find(|(_, scan)| !scan.pages.is_empty())
                    .map(|(id, _)| id.clone());
                if let Some(id) = first {
                    self.select_pane(PaneId::Printers);
                    self.detail = Some(Detail::Printer(id.clone()));
                    self.scan_intent(ScanIntent::Save);
                    if self.scans.get(&id).and_then(|scan| scan.busy) != Some(Busy::Saving) {
                        self.leaving = None;
                    }
                }
            }
            STOP => {
                let mut stopping = Vec::new();
                for (id, scan) in &mut self.scans {
                    if scan.busy == Some(Busy::Scanning) {
                        scan.busy = Some(Busy::Cancelling);
                        stopping.push((id.clone(), scan.token));
                    }
                }
                for (id, token) in stopping {
                    self.commands
                        .push(Command::Printers(PrinterCommand::CancelScan(id, token)));
                }
                self.leaving = None;
                self.leaving_after_scan = Some(leaving);
            }
            DISCARD => {
                for (id, scan) in &mut self.scans {
                    scan.discard();
                    forget_pages(id);
                }
                self.leaving = None;
                self.commands.push(Command::Leave(leaving));
            }
            _ => self.leaving = None,
        }
    }

    pub(crate) fn leave_once_sheet_closed(&mut self) {
        if self.ui.has_sheet() {
            return;
        }
        if let Some(leaving) = self.leaving_after_sheet.take()
            && self.may_leave(leaving)
        {
            self.commands.push(Command::Leave(leaving));
        }
    }

    fn scan_ended_while_leaving(&mut self) {
        if self.scans.values().any(ScanState::scanning) {
            return;
        }
        if let Some(leaving) = self.leaving_after_scan.take()
            && self.may_leave(leaving)
        {
            self.commands.push(Command::Leave(leaving));
        }
    }

    fn saved_while_leaving(&mut self, saved: bool) {
        let Some(leaving) = self.leaving.take() else {
            return;
        };
        if saved && !self.may_leave(leaving) {
            return;
        }
        if saved {
            self.commands.push(Command::Leave(leaving));
        }
    }

    fn opened_scan(&mut self) -> Option<&mut ScanState> {
        let Some(Detail::Printer(id)) = &self.detail else {
            return None;
        };
        self.scans.get_mut(id)
    }

    pub(crate) fn scanner_name(&self, id: &str) -> Option<String> {
        let state = self.printers.as_ref();
        state
            .and_then(|state| state.scanners.iter().find(|scanner| scanner.id == id))
            .map(|scanner| scanner.name.clone())
            .or_else(|| self.scans.get(id).map(|scan| scan.name.clone()))
    }

    pub(crate) fn kept_away(&self, state: &PrintersState) -> Vec<(String, String)> {
        let mut away: Vec<(String, String)> = self
            .scans
            .iter()
            .filter(|(id, scan)| {
                !scan.pages.is_empty()
                    && !state.printers.iter().any(|printer| printer.id == **id)
                    && !state.scanners.iter().any(|scanner| scanner.id == **id)
            })
            .map(|(id, scan)| (id.clone(), scan.name.clone()))
            .collect();
        away.sort_by_key(|(_, name)| name.to_lowercase());
        away
    }

    pub(crate) fn unsaved_pages_of(&self, id: &str) -> usize {
        self.scans.get(id).map_or(0, |scan| scan.pages.len())
    }

    pub(crate) fn open_scanner(&mut self, id: &str) {
        let Some(name) = self
            .printers
            .as_ref()
            .and_then(|state| state.scanners.iter().find(|scanner| scanner.id == id))
            .map(|scanner| scanner.name.clone())
        else {
            return;
        };
        let scan = self
            .scans
            .entry(id.to_owned())
            .or_insert_with(|| ScanState::new(id, &name));
        if scan.capabilities.is_some() || scan.busy.is_some() || scan.message.is_some() {
            return;
        }
        scan.busy = Some(Busy::Asking);
        self.commands
            .push(Command::Printers(PrinterCommand::ScanCapabilities(
                id.to_owned(),
            )));
    }

    pub(crate) fn scan_update(&mut self, update: ScanUpdate) {
        let id = match &update {
            ScanUpdate::Capabilities(id, _)
            | ScanUpdate::Scanned(id, _)
            | ScanUpdate::Saved(id, _) => id.clone(),
            ScanUpdate::Kept(id, name, pages, preview) => {
                let scan = self.scans.entry(id.clone()).or_insert_with(|| ScanState {
                    busy: None,
                    ..ScanState::new(id, name)
                });
                if scan.pages.is_empty() {
                    scan.pages.clone_from(pages);
                    scan.preview.clone_from(preview);
                }
                return;
            }
        };
        let opened = matches!(&self.detail, Some(Detail::Printer(shown)) if *shown == id);
        let Some(scan) = self.scans.get_mut(&id) else {
            return;
        };
        let mut focus = None;
        let mut finished = None;
        let mut ended = false;
        match update {
            ScanUpdate::Capabilities(_, answer) => {
                scan.busy = None;
                match answer {
                    Ok(capabilities) => {
                        scan.source = scan
                            .source
                            .min(capabilities.sources.len().saturating_sub(1));
                        scan.capabilities = Some(capabilities);
                        if let Some(source) = scan.source() {
                            scan.resolution = nearest(&source.resolutions, scan.resolution);
                        }
                        scan.message = None;
                    }
                    Err(message) => scan.message = Some(message),
                }
            }
            ScanUpdate::Scanned(_, scanned) => {
                scan.busy = None;
                ended = true;
                match scanned {
                    Ok(Scanned {
                        pages,
                        preview,
                        warning,
                    }) => {
                        scan.pages.extend(pages);
                        scan.preview = preview.or(scan.preview.take());
                        scan.message = warning;
                        if !scan.formats().contains(&scan.format) {
                            scan.format = Format::Pdf;
                        }
                        focus = Some("scan-save");
                    }
                    Err(message) => scan.message = Some(message),
                }
            }
            ScanUpdate::Kept(..) => {}
            ScanUpdate::Saved(_, saved) => {
                scan.busy = None;
                finished = Some(matches!(saved, Ok(Some(_))));
                match saved {
                    Ok(Some(name)) => {
                        scan.discard();
                        scan.message = Some(tr!("Saved “{name}”", name = name));
                        focus = Some("scan");
                    }
                    Ok(None) => {}
                    Err(message) => scan.message = Some(message),
                }
            }
        }
        if opened && let Some(anchor) = focus {
            self.focus_next = Some(Anchor::Item(anchor.to_owned()));
        }
        if let Some(saved) = finished {
            self.saved_while_leaving(saved);
        }
        if ended {
            self.scan_ended_while_leaving();
        }
    }

    pub(crate) fn scan_intent(&mut self, change: ScanIntent) {
        let Some(scan) = self.opened_scan() else {
            return;
        };
        let id = scan.scanner.clone();
        let mut command = None;
        let mut focus = None;
        match change {
            ScanIntent::Source(index) => {
                scan.source = index;
                if let Some(source) = scan.source() {
                    scan.resolution = nearest(&source.resolutions, scan.resolution);
                }
            }
            ScanIntent::Color(index) => scan.color = index == 0,
            ScanIntent::Resolution(index) => {
                if let Some(dpi) = scan
                    .source()
                    .and_then(|source| source.resolutions.get(index))
                {
                    scan.resolution = *dpi;
                }
            }
            ScanIntent::Format(index) => {
                if let Some(format) = scan.formats().get(index) {
                    scan.format = *format;
                }
            }
            ScanIntent::Retry => {
                scan.busy = Some(Busy::Asking);
                scan.message = None;
                command = Some(PrinterCommand::ScanCapabilities(id));
            }
            ScanIntent::Start => {
                if let Some(options) = scan.options()
                    && scan.busy.is_none()
                {
                    scan.busy = Some(Busy::Scanning);
                    scan.message = None;
                    scan.asking_discard = false;
                    scan.token = SCANS.fetch_add(1, Ordering::Relaxed);
                    command = Some(PrinterCommand::Scan(
                        id,
                        scan.name.clone(),
                        scan.token,
                        options,
                    ));
                    focus = Some("scan");
                }
            }
            ScanIntent::Cancel => {
                if scan.busy == Some(Busy::Scanning) {
                    scan.busy = Some(Busy::Cancelling);
                    command = Some(PrinterCommand::CancelScan(id, scan.token));
                }
            }
            ScanIntent::Save => {
                if !scan.pages.is_empty() && scan.busy.is_none() {
                    scan.busy = Some(Busy::Saving);
                    scan.message = None;
                    scan.asking_discard = false;
                    command = Some(PrinterCommand::SaveScan(
                        id,
                        scan.pages.clone(),
                        scan.format,
                    ));
                }
            }
            ScanIntent::Discard => {
                scan.asking_discard = true;
                focus = Some("scan-keep");
            }
            ScanIntent::KeepPages => {
                scan.asking_discard = false;
                focus = Some("scan-discard");
            }
            ScanIntent::ConfirmDiscard => {
                scan.discard();
                forget_pages(&id);
                scan.message = None;
                focus = Some("scan");
            }
        }
        if let Some(command) = command {
            self.commands.push(Command::Printers(command));
        }
        if let Some(anchor) = focus {
            self.focus_next = Some(Anchor::Item(anchor.to_owned()));
        }
    }

    fn scan_menu(
        &mut self,
        label: &str,
        anchor: &str,
        titles: &[String],
        selected: usize,
        make: fn(usize) -> ScanIntent,
    ) -> Box<dyn Widget> {
        let choose = intent(&self.intents, move |index| Intent::Scan(make(index)));
        let menu = PopUpButton::new(label, titles, selected, choose);
        self.anchors
            .push((Anchor::Item(anchor.to_owned()), menu.id()));
        boxed(Row::new(Text::new(label, Tone::Primary), Some(boxed(menu))))
    }

    fn scan_button(
        &mut self,
        label: &str,
        anchor: &str,
        enabled: bool,
        action: ScanIntent,
    ) -> Box<dyn Widget> {
        let mut act = intent(&self.intents, move |()| Intent::Scan(action));
        let mut button = Button::new(label, move || act(()));
        button.set_enabled(enabled);
        self.anchors
            .push((Anchor::Item(anchor.to_owned()), button.id()));
        boxed(button)
    }

    pub(crate) fn scan_rows(&mut self, scanner: &str) -> Widgets {
        let Some(scan) = self.scans.get(scanner).cloned() else {
            return Vec::new();
        };
        let mut page: Widgets = vec![boxed(Caption::new(Text::label(&tr!("Scan"))))];
        let idle = scan.busy.is_none();
        let mut rows = Vec::new();
        match scan.capabilities.clone() {
            None => {
                let row = match (scan.busy, &scan.message) {
                    (Some(Busy::Asking), _) => {
                        muted_row(&tr!("Asking the scanner what it can do…"))
                    }
                    (_, None) => muted_row(&tr!(
                        "The scanner isn’t on this network now. Its scanned pages can still be saved."
                    )),
                    (_, Some(message)) => {
                        let retry =
                            self.scan_button(&tr!("Try Again"), "scan", true, ScanIntent::Retry);
                        boxed(Row::new(Text::new(message, Tone::Muted), Some(retry)))
                    }
                };
                rows.push(row);
            }
            Some(capabilities) => {
                if capabilities.sources.len() > 1 {
                    let titles: Vec<String> = capabilities
                        .sources
                        .iter()
                        .map(|source| source_title(&source.name))
                        .collect();
                    rows.push(self.scan_menu(
                        &tr!("Source"),
                        "scan-source",
                        &titles,
                        scan.source,
                        ScanIntent::Source,
                    ));
                }
                let source = scan.source().cloned().unwrap_or_default();
                if source.color {
                    rows.push(self.scan_menu(
                        &tr!("Color"),
                        "scan-color",
                        &[tr!("Color"), tr!("Grayscale")],
                        usize::from(!scan.color),
                        ScanIntent::Color,
                    ));
                }
                if source.resolutions.len() > 1 {
                    let titles: Vec<String> = source
                        .resolutions
                        .iter()
                        .map(|dpi| tr!("{dpi} dpi", dpi = dpi.to_string()))
                        .collect();
                    let selected = source
                        .resolutions
                        .iter()
                        .position(|dpi| *dpi == scan.resolution)
                        .unwrap_or(0);
                    rows.push(self.scan_menu(
                        &tr!("Resolution"),
                        "scan-resolution",
                        &titles,
                        selected,
                        ScanIntent::Resolution,
                    ));
                }
                let status = match (scan.busy, &scan.message) {
                    (Some(Busy::Scanning), _) => tr!("Scanning…"),
                    (Some(Busy::Cancelling), _) => tr!("Stopping…"),
                    (Some(Busy::Saving), _) => tr!("Saving…"),
                    (_, Some(message)) => message.clone(),
                    _ if scan.pages.is_empty() => {
                        tr!("Place the page on the scanner or the pages in its feeder")
                    }
                    _ => trn!(
                        "1 page scanned",
                        "{count} pages scanned",
                        count = scan.pages.len()
                    ),
                };
                let label = match scan.pages.is_empty() {
                    true => tr!("Scan"),
                    false => tr!("Scan More"),
                };
                let button = match scan.busy {
                    Some(Busy::Scanning | Busy::Cancelling) => self.scan_button(
                        &tr!("Cancel"),
                        "scan",
                        scan.busy == Some(Busy::Scanning),
                        ScanIntent::Cancel,
                    ),
                    _ => self.scan_button(&label, "scan", idle, ScanIntent::Start),
                };
                rows.push(boxed(
                    Row::new(
                        Text::new(&tr!("Scan with these settings"), Tone::Primary),
                        Some(button),
                    )
                    .detail(Text::new(&status, Tone::Muted).caption()),
                ));
            }
        }
        if !scan.pages.is_empty() {
            if let Some(preview) = scan.preview.clone() {
                let label = tr!(
                    "Scanned page {number}",
                    number = scan.pages.len().to_string()
                );
                rows.push(boxed(Preview::new(&label, preview)));
            }
            let formats = scan.formats();
            if formats.len() > 1 {
                let titles: Vec<String> = formats.iter().map(|format| format.title()).collect();
                let selected = formats
                    .iter()
                    .position(|format| *format == scan.format)
                    .unwrap_or(0);
                rows.push(self.scan_menu(
                    &tr!("Save as"),
                    "scan-format",
                    &titles,
                    selected,
                    ScanIntent::Format,
                ));
            }
            let save = self.scan_button(&tr!("Save…"), "scan-save", idle, ScanIntent::Save);
            rows.push(boxed(Row::new(
                Text::new(&tr!("Save the scanned pages"), Tone::Primary),
                Some(save),
            )));
            if scan.asking_discard {
                let keep = self.scan_button(&tr!("Keep"), "scan-keep", true, ScanIntent::KeepPages);
                let discard = self.scan_button(
                    &tr!("Discard"),
                    "scan-discard",
                    idle,
                    ScanIntent::ConfirmDiscard,
                );
                rows.push(boxed(
                    Row::new(
                        Text::new(
                            &trn!(
                                "Discard the scanned page?",
                                "Discard the {count} scanned pages?",
                                count = scan.pages.len()
                            ),
                            Tone::Primary,
                        ),
                        Some(boxed(Stack::new(
                            Axis::Horizontal,
                            Spacing::S as f32,
                            0.0,
                            vec![keep, discard],
                        ))),
                    )
                    .detail(Text::new(&tr!("They have not been saved."), Tone::Muted).caption()),
                ));
            } else {
                let discard =
                    self.scan_button(&tr!("Discard…"), "scan-discard", idle, ScanIntent::Discard);
                rows.push(boxed(Row::new(
                    Text::new(&tr!("Start over"), Tone::Primary),
                    Some(discard),
                )));
            }
        }
        page.push(boxed(Card::new(rows)));
        page
    }
}
