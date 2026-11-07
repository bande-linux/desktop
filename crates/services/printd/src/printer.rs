use bande_printers::ipp::{Group, INTEGER, KEYWORD, MIME_TYPE, NAME, PRINTER, Request, URI, Value};
use bande_printers::{Capabilities, Error, JobOptions, Result, user};

use crate::link::{Address, exchange};
use crate::raster::{Backside, Format, Plan, media_size};
use crate::store::Pins;

const PRINT_JOB: u16 = 0x0002;
const CANCEL_JOB: u16 = 0x0008;
const GET_JOB: u16 = 0x0009;
const GET_PRINTER: u16 = 0x000B;
const JOB_CANCELED: i32 = 7;
const JOB_ABORTED: i32 = 8;
const JOB_COMPLETED: i32 = 9;
const PDF: &str = "application/pdf";
const PWG: &str = "image/pwg-raster";
const URF: &str = "image/urf";
const LARGEST_DPI: i32 = 600;
const DEFAULT_DPI: u32 = 300;
const FALLBACK_MEDIA: &str = "iso_a4_210x297mm";
const ATTRIBUTES: [&str; 13] = [
    "printer-make-and-model",
    "document-format-supported",
    "media-supported",
    "media-default",
    "sides-supported",
    "print-color-mode-supported",
    "copies-supported",
    "page-ranges-supported",
    "pwg-raster-document-resolution-supported",
    "pwg-raster-document-sheet-back",
    "pwg-raster-document-type-supported",
    "urf-supported",
    "printer-uri-supported",
];

#[derive(Debug, Clone)]
pub struct Abilities {
    pub formats: Vec<String>,
    pub media: Vec<String>,
    pub default_media: String,
    pub sides: Vec<String>,
    pub color: bool,
    pub maximum_copies: i32,
    pub page_ranges: bool,
    pub pwg_resolutions: Vec<i32>,
    pub pwg_back: String,
    pub urf: Vec<String>,
}

impl Abilities {
    pub fn public(&self) -> Capabilities {
        Capabilities {
            media: self.media.clone(),
            default_media: self.default_media.clone(),
            two_sided: self.sides.iter().any(|side| side.starts_with("two-sided")),
            color: self.color,
            maximum_copies: self.maximum_copies,
        }
    }

    fn accepts_pdf_as_asked(&self, options: &JobOptions) -> bool {
        self.formats.iter().any(|format| format == PDF)
            && (options.pages.is_empty() || self.page_ranges)
            && options.copies <= self.maximum_copies
    }

    fn raster_format(&self) -> Option<Format> {
        let has = |name: &str| self.formats.iter().any(|format| format == name);
        match (has(PWG), has(URF)) {
            (true, _) => Some(Format::Pwg),
            (_, true) => Some(Format::Urf),
            _ => None,
        }
    }

    fn dpi(&self, format: Format) -> u32 {
        let offered = match format {
            Format::Pwg => self.pwg_resolutions.clone(),
            Format::Urf => self
                .urf
                .iter()
                .find_map(|entry| entry.strip_prefix("RS"))
                .map(|list| list.split('-').filter_map(|dpi| dpi.parse().ok()).collect())
                .unwrap_or_default(),
        };
        offered
            .into_iter()
            .filter(|dpi| *dpi <= LARGEST_DPI)
            .max()
            .and_then(|dpi| u32::try_from(dpi).ok())
            .unwrap_or(DEFAULT_DPI)
    }

    fn backside(&self, format: Format, tumble: bool) -> Backside {
        let kind = match format {
            Format::Pwg => self.pwg_back.as_str(),
            Format::Urf => self
                .urf
                .iter()
                .find_map(|entry| match entry.as_str() {
                    "DM2" => Some("flipped"),
                    "DM3" => Some("rotated"),
                    "DM4" => Some("manual-tumble"),
                    _ => None,
                })
                .unwrap_or("normal"),
        };
        let (across, along) = match (kind, tumble) {
            ("flipped", true) => (true, false),
            ("flipped", false) => (false, true),
            ("manual-tumble", true) | ("rotated", false) => (true, true),
            _ => (false, false),
        };
        Backside {
            mirror_across: across,
            mirror_along: along,
        }
    }

    pub fn raster_plan(&self, options: &JobOptions) -> Option<Plan> {
        let format = self.raster_format()?;
        let keyword = match options.media.is_empty() {
            true => self.default_media.clone(),
            false => options.media.clone(),
        };
        let tumble = options.sides == "two-sided-short-edge";
        Some(Plan {
            format,
            dpi: self.dpi(format),
            media_points: media_size(&keyword)
                .or_else(|| media_size(FALLBACK_MEDIA))
                .unwrap_or((595.0, 842.0)),
            media_keyword: keyword,
            color: options.color && self.color,
            two_sided: options.sides.starts_with("two-sided"),
            tumble,
            backside: self.backside(format, tumble),
            landscape: options.landscape,
            copies: u32::try_from(options.copies.max(1)).unwrap_or(1),
            pages: options.pages.clone(),
        })
    }
}

fn printer_group(groups: &[Group]) -> Option<&Group> {
    groups.iter().find(|group| group.tag == PRINTER)
}

pub fn request_for(operation: u16, uri: &str) -> Request {
    let mut request = Request::new(operation, &user());
    request.string(URI, "printer-uri", uri);
    request
}

pub fn abilities(address: &Address, uri: &str, pins: &Pins) -> Result<(Abilities, String)> {
    let mut request = request_for(GET_PRINTER, uri);
    request.strings(KEYWORD, "requested-attributes", &ATTRIBUTES);
    let groups = exchange(address, pins, request)?;
    let group = printer_group(&groups).ok_or(Error::Malformed)?;
    let media = group.texts("media-supported");
    let color = group
        .texts("print-color-mode-supported")
        .iter()
        .any(|mode| mode == "color" || mode == "auto");
    let default_media = group
        .text("media-default")
        .or_else(|| media.first().cloned())
        .unwrap_or_else(|| FALLBACK_MEDIA.to_owned());
    let maximum_copies = match group.get("copies-supported").and_then(<[Value]>::first) {
        Some(Value::Range(_, last)) => *last,
        Some(Value::Integer(value)) => *value,
        _ => 1,
    };
    let pwg_resolutions = group
        .get("pwg-raster-document-resolution-supported")
        .unwrap_or_default()
        .iter()
        .filter_map(|value| match value {
            Value::Resolution(horizontal, _) => Some(*horizontal),
            _ => None,
        })
        .collect();
    Ok((
        Abilities {
            formats: group.texts("document-format-supported"),
            media,
            default_media,
            sides: group.texts("sides-supported"),
            color,
            maximum_copies: maximum_copies.max(1),
            page_ranges: group.boolean("page-ranges-supported").unwrap_or(false),
            pwg_resolutions,
            pwg_back: group
                .text("pwg-raster-document-sheet-back")
                .unwrap_or_else(|| "normal".to_owned()),
            urf: group.texts("urf-supported"),
        },
        group.text("printer-make-and-model").unwrap_or_default(),
    ))
}

pub enum Payload {
    Pdf,
    Raster(Plan),
}

pub fn payload_for(abilities: &Abilities, options: &JobOptions) -> Option<Payload> {
    if abilities.accepts_pdf_as_asked(options) {
        return Some(Payload::Pdf);
    }
    abilities.raster_plan(options).map(Payload::Raster)
}

pub fn print_request(uri: &str, title: &str, options: &JobOptions, payload: &Payload) -> Request {
    let mut request = request_for(PRINT_JOB, uri);
    let format = match payload {
        Payload::Pdf => PDF,
        Payload::Raster(plan) => match plan.format {
            Format::Pwg => PWG,
            Format::Urf => URF,
        },
    };
    request
        .string(NAME, "job-name", title)
        .string(MIME_TYPE, "document-format", format);
    request.begin_group(bande_printers::ipp::JOB);
    match payload {
        Payload::Pdf => options.write_to(&mut request),
        Payload::Raster(plan) => {
            let once = JobOptions {
                copies: 1,
                pages: Vec::new(),
                landscape: false,
                media: plan.media_keyword.clone(),
                color: plan.color,
                ..options.clone()
            };
            once.write_to(&mut request);
        }
    }
    request
}

pub fn accepted_job(groups: &[Group]) -> Option<i32> {
    groups
        .iter()
        .find(|group| group.tag == bande_printers::ipp::JOB)
        .and_then(|group| group.integer("job-id"))
}

pub enum RemoteState {
    Going,
    Done,
    Stopped(String),
}

pub fn job_state(address: &Address, uri: &str, pins: &Pins, job: i32) -> Result<RemoteState> {
    let mut request = request_for(GET_JOB, uri);
    request.integer(INTEGER, "job-id", job).strings(
        KEYWORD,
        "requested-attributes",
        &["job-state", "job-state-reasons", "job-state-message"],
    );
    let groups = exchange(address, pins, request)?;
    let group = groups
        .iter()
        .find(|group| group.tag == bande_printers::ipp::JOB)
        .ok_or(Error::Malformed)?;
    Ok(match group.integer("job-state") {
        Some(JOB_COMPLETED) | Some(JOB_CANCELED) => RemoteState::Done,
        Some(JOB_ABORTED) => RemoteState::Stopped(
            group
                .text("job-state-message")
                .or_else(|| group.texts("job-state-reasons").into_iter().next())
                .unwrap_or_default(),
        ),
        _ => RemoteState::Going,
    })
}

pub fn cancel_job(address: &Address, uri: &str, pins: &Pins, job: i32) -> Result<()> {
    let mut request = request_for(CANCEL_JOB, uri);
    request.integer(INTEGER, "job-id", job);
    exchange(address, pins, request).map(drop)
}

pub fn printer_state_reasons(address: &Address, uri: &str, pins: &Pins) -> Result<Vec<String>> {
    let mut request = request_for(GET_PRINTER, uri);
    request.strings(KEYWORD, "requested-attributes", &["printer-state-reasons"]);
    let groups = exchange(address, pins, request)?;
    Ok(printer_group(&groups)
        .map(|group| {
            group
                .texts("printer-state-reasons")
                .into_iter()
                .filter(|reason| reason != "none")
                .collect()
        })
        .unwrap_or_default())
}
