use std::fmt;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use bande_app::choose_save_file;
use bande_l10n::{tr, trn};
use bande_pdf::Picture;
use bande_printers::{PrintClient, ScanCapabilities, ScanOptions};
use bande_toolkit::Image;
use bande_toolkit::ui::{Button, PopUpButton, Widget};
use image::{ImageFormat, ImageReader};
use zbus::blocking::Connection;

use crate::follow::Update;
use crate::pages::{Widgets, boxed, muted_row};
use crate::printers::PrinterCommand;
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Caption, Card, Preview, Row, Text, Tone};

const PLATEN: &str = "Platen";
const FEEDER: &str = "Feeder";
const DEFAULT_RESOLUTION: u32 = 300;
const PAGE_LIMIT: u64 = 128 << 20;
const PREVIEW_PIXELS: u32 = 720;
const POINTS_PER_INCH: f32 = 72.0;

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
pub enum ScanUpdate {
    Capabilities(String, Result<ScanCapabilities, String>),
    Scanned(String, Result<(Vec<Page>, Option<Image>), String>),
    Saved(Result<Option<String>, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanIntent {
    Source(usize),
    Color(usize),
    Resolution(usize),
    Format(usize),
    Retry,
    Start,
    Save,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Asking,
    Scanning,
    Saving,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanState {
    scanner: String,
    capabilities: Option<ScanCapabilities>,
    source: usize,
    color: bool,
    resolution: u32,
    format: Format,
    pages: Vec<Page>,
    preview: Option<Image>,
    busy: Option<Busy>,
    message: Option<String>,
}

impl ScanState {
    fn new(scanner: &str) -> Self {
        Self {
            scanner: scanner.to_owned(),
            capabilities: None,
            source: 0,
            color: true,
            resolution: DEFAULT_RESOLUTION,
            format: Format::Pdf,
            pages: Vec::new(),
            preview: None,
            busy: Some(Busy::Asking),
            message: None,
        }
    }

    fn formats(&self) -> Vec<Format> {
        match self.pages.len() {
            1 => vec![Format::Pdf, Format::Png, Format::Jpeg],
            _ => vec![Format::Pdf],
        }
    }

    fn options(&self) -> Option<ScanOptions> {
        let capabilities = self.capabilities.as_ref()?;
        Some(ScanOptions {
            source: capabilities.sources.get(self.source)?.clone(),
            color: self.color && capabilities.color,
            resolution: self.resolution,
        })
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

pub fn scan(session: &Connection, scanner: &str, options: &ScanOptions) -> Update {
    let scanned = PrintClient::new(session)
        .and_then(|client| client.scan(scanner, options))
        .map_err(reason)
        .and_then(|pages| {
            let pages = pages
                .into_iter()
                .map(|page| read_page(page, options.resolution))
                .collect::<Result<Vec<_>, _>>()?;
            let preview = pages
                .last()
                .and_then(|page| Image::decode(&page.jpeg, PREVIEW_PIXELS));
            Ok((pages, preview))
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

pub fn save(session: &Connection, pages: &[Page], format: Format) -> Update {
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
    Update::Scan(ScanUpdate::Saved(saved))
}

fn write(path: &Path, pages: &[Page], format: Format) -> Result<(), String> {
    let bytes = encoded(pages, format)?;
    std::fs::write(path, bytes).map_err(|error| {
        tracing::warn!("cannot save a scan to {}: {error}", path.display());
        tr!("Couldn’t save the scan.")
    })
}

impl SettingsView {
    pub(crate) fn open_scanner(&mut self, id: &str) {
        let known = self
            .printers
            .as_ref()
            .is_some_and(|state| state.scanners.iter().any(|scanner| scanner.id == id));
        if !known || self.scan.as_ref().is_some_and(|scan| scan.scanner == id) {
            return;
        }
        self.scan = Some(ScanState::new(id));
        self.commands.push(crate::follow::Command::Printers(
            PrinterCommand::ScanCapabilities(id.to_owned()),
        ));
    }

    pub(crate) fn scan_update(&mut self, update: ScanUpdate) {
        let Some(scan) = self.scan.as_mut() else {
            return;
        };
        match update {
            ScanUpdate::Capabilities(id, answer) if id == scan.scanner => {
                scan.busy = None;
                match answer {
                    Ok(capabilities) => {
                        scan.resolution = nearest(&capabilities.resolutions, scan.resolution);
                        scan.source = scan
                            .source
                            .min(capabilities.sources.len().saturating_sub(1));
                        scan.capabilities = Some(capabilities);
                        scan.message = None;
                    }
                    Err(message) => scan.message = Some(message),
                }
            }
            ScanUpdate::Scanned(id, scanned) if id == scan.scanner => {
                scan.busy = None;
                match scanned {
                    Ok((pages, preview)) => {
                        scan.pages.extend(pages);
                        scan.preview = preview.or(scan.preview.take());
                        scan.message = None;
                        if !scan.formats().contains(&scan.format) {
                            scan.format = Format::Pdf;
                        }
                        self.focus_next = Some(Anchor::Item("scan-save".to_owned()));
                    }
                    Err(message) => scan.message = Some(message),
                }
            }
            ScanUpdate::Saved(saved) => {
                scan.busy = None;
                match saved {
                    Ok(Some(name)) => {
                        scan.pages.clear();
                        scan.preview = None;
                        scan.format = Format::Pdf;
                        scan.message = Some(tr!("Saved “{name}”", name = name));
                        self.focus_next = Some(Anchor::Item("scan".to_owned()));
                    }
                    Ok(None) => {}
                    Err(message) => scan.message = Some(message),
                }
            }
            _ => {}
        }
    }

    pub(crate) fn scan_intent(&mut self, change: ScanIntent) {
        let Some(scan) = self.scan.as_mut() else {
            return;
        };
        let capabilities = scan.capabilities.clone().unwrap_or_default();
        match change {
            ScanIntent::Source(index) => scan.source = index,
            ScanIntent::Color(index) => scan.color = index == 0,
            ScanIntent::Resolution(index) => {
                if let Some(dpi) = capabilities.resolutions.get(index) {
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
                let id = scan.scanner.clone();
                self.commands.push(crate::follow::Command::Printers(
                    PrinterCommand::ScanCapabilities(id),
                ));
            }
            ScanIntent::Start => {
                if let Some(options) = scan.options()
                    && scan.busy.is_none()
                {
                    scan.busy = Some(Busy::Scanning);
                    scan.message = None;
                    let id = scan.scanner.clone();
                    self.commands
                        .push(crate::follow::Command::Printers(PrinterCommand::Scan(
                            id, options,
                        )));
                }
            }
            ScanIntent::Save => {
                if !scan.pages.is_empty() && scan.busy.is_none() {
                    scan.busy = Some(Busy::Saving);
                    scan.message = None;
                    self.commands
                        .push(crate::follow::Command::Printers(PrinterCommand::SaveScan(
                            scan.pages.clone(),
                            scan.format,
                        )));
                }
            }
            ScanIntent::Discard => {
                scan.pages.clear();
                scan.preview = None;
                scan.format = Format::Pdf;
                scan.message = None;
                self.focus_next = Some(Anchor::Item("scan".to_owned()));
            }
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
        let Some(scan) = self.scan.clone().filter(|scan| scan.scanner == scanner) else {
            return Vec::new();
        };
        let mut page: Widgets = vec![boxed(Caption::new(Text::label(&tr!("Scan"))))];
        let Some(capabilities) = scan.capabilities.clone() else {
            let row = match (scan.busy, &scan.message) {
                (Some(Busy::Asking), _) | (_, None) => {
                    muted_row(&tr!("Asking the scanner what it can do…"))
                }
                (_, Some(message)) => {
                    let retry =
                        self.scan_button(&tr!("Try Again"), "scan", true, ScanIntent::Retry);
                    boxed(Row::new(Text::new(message, Tone::Muted), Some(retry)))
                }
            };
            page.push(boxed(Card::new(vec![row])));
            return page;
        };
        let idle = scan.busy.is_none();
        let mut rows = Vec::new();
        if capabilities.sources.len() > 1 {
            let titles: Vec<String> = capabilities
                .sources
                .iter()
                .map(|source| source_title(source))
                .collect();
            rows.push(self.scan_menu(
                &tr!("Source"),
                "scan-source",
                &titles,
                scan.source,
                ScanIntent::Source,
            ));
        }
        if capabilities.color {
            rows.push(self.scan_menu(
                &tr!("Color"),
                "scan-color",
                &[tr!("Color"), tr!("Grayscale")],
                usize::from(!scan.color),
                ScanIntent::Color,
            ));
        }
        if capabilities.resolutions.len() > 1 {
            let titles: Vec<String> = capabilities
                .resolutions
                .iter()
                .map(|dpi| tr!("{dpi} dpi", dpi = dpi.to_string()))
                .collect();
            let selected = capabilities
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
        let button = self.scan_button(&label, "scan", idle, ScanIntent::Start);
        rows.push(boxed(
            Row::new(
                Text::new(&tr!("Scan with these settings"), Tone::Primary),
                Some(button),
            )
            .detail(Text::new(&status, Tone::Muted).caption()),
        ));
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
            let discard =
                self.scan_button(&tr!("Discard"), "scan-discard", idle, ScanIntent::Discard);
            rows.push(boxed(Row::new(
                Text::new(&tr!("Start over"), Tone::Primary),
                Some(discard),
            )));
        }
        page.push(boxed(Card::new(rows)));
        page
    }
}
