use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bande_content::{Decoded, Pixels, decode_confined};
use bande_xdg::runtime_dir;
use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;
use rustix::fs::{MemfdFlags, memfd_create};

use bande_l10n::tr;
use bande_printers::http::Response;
use bande_printers::{Error, ScanCapabilities, ScanOptions};
use roxmltree::{Document, Node};

use crate::link::{Address, HttpRequest, http, is_identity_change};
use crate::store::Pins;

pub const PLATEN: &str = "Platen";
pub const FEEDER: &str = "Feeder";
const JPEG: &str = "image/jpeg";
const COLOR: &str = "RGB24";
const GRAY: &str = "Grayscale8";
const DESCRIPTION_LIMIT: usize = 1 << 20;
const ANSWER_LIMIT: usize = 64 << 10;
pub const PAGE_LIMIT: usize = 128 << 20;
pub const MOST_PAGES: usize = 200;
const ANSWER_TIME: Duration = Duration::from_secs(30);
const PAGE_TIME: Duration = Duration::from_secs(180);
const BUSY_RETRIES: u32 = 15;
const BUSY_PAUSE: Duration = Duration::from_secs(2);
const STANDARD_RESOLUTIONS: [u32; 6] = [75, 100, 150, 200, 300, 600];
const HIGHEST_RESOLUTION: u32 = 1200;
const JPEG_QUALITY: u8 = 90;
const DEFAULT_VERSION: &str = "2.0";
const OK: u16 = 200;
const CREATED: u16 = 201;
const NOT_FOUND: u16 = 404;
const CONFLICT: u16 = 409;
const UNAVAILABLE: u16 = 503;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    Unreachable,
    IdentityChanged,
    Busy,
    FeederEmpty,
    NotSupported,
    Unreadable,
    Refused(u16),
}

impl ScanError {
    pub fn sentence(&self) -> String {
        match self {
            Self::Unreachable => {
                tr!("The scanner can’t be reached. Check that it is turned on and on this network.")
            }
            Self::IdentityChanged => tr!(
                "The scanner's identity has changed, so nothing was scanned. If it was replaced or reset, remove it in Settings ▸ Printers and try again."
            ),
            Self::Busy => tr!("The scanner is busy. Try again in a moment."),
            Self::FeederEmpty => tr!("Put the pages in the document feeder and try again."),
            Self::NotSupported => tr!("The scanner can’t scan this way."),
            Self::Unreadable => tr!("The scanner sent something that can’t be read."),
            Self::Refused(status) => tr!(
                "The scanner refused to scan (error {status}).",
                status = status.to_string()
            ),
        }
    }
}

impl From<Error> for ScanError {
    fn from(error: Error) -> Self {
        match error {
            error if is_identity_change(&error) => Self::IdentityChanged,
            Error::Io(error) => {
                tracing::info!("scanner: {error}");
                Self::Unreachable
            }
            Error::Malformed => Self::Unreadable,
            Error::NotAllowed => Self::Refused(403),
            Error::Refused(status, _) => Self::Refused(status),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub color: bool,
    pub gray: bool,
    pub jpeg: bool,
    pub resolutions: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanner {
    pub version: String,
    pub sources: Vec<Source>,
}

impl Scanner {
    pub fn public(&self) -> ScanCapabilities {
        let usable = || self.sources.iter().filter(|source| source.jpeg);
        let mut resolutions: Vec<u32> = usable()
            .flat_map(|source| source.resolutions.iter().copied())
            .collect();
        resolutions.sort_unstable();
        resolutions.dedup();
        let standard: Vec<u32> = resolutions
            .iter()
            .copied()
            .filter(|dpi| STANDARD_RESOLUTIONS.contains(dpi))
            .collect();
        ScanCapabilities {
            sources: usable().map(|source| source.name.to_owned()).collect(),
            color: usable().any(|source| source.color),
            resolutions: match standard.is_empty() {
                true => resolutions,
                false => standard,
            },
        }
    }
}

fn named<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.descendants()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

fn texts(node: Node<'_, '_>, names: &[&str]) -> Vec<String> {
    node.descendants()
        .filter(|child| child.is_element() && names.contains(&child.tag_name().name()))
        .filter_map(|child| child.text())
        .map(|text| text.trim().to_owned())
        .collect()
}

fn number(node: Node<'_, '_>, name: &str) -> Option<u32> {
    named(node, name)?.text()?.trim().parse().ok()
}

fn resolutions(caps: Node<'_, '_>) -> Vec<u32> {
    let mut found: Vec<u32> = caps
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "DiscreteResolution")
        .filter_map(|node| {
            let x = number(node, "XResolution")?;
            (number(node, "YResolution").is_none_or(|y| y == x)).then_some(x)
        })
        .collect();
    if let Some(range) = named(caps, "XResolutionRange")
        && let (Some(min), Some(max)) = (number(range, "Min"), number(range, "Max"))
    {
        let step = number(range, "Step").unwrap_or(1).max(1);
        found.extend(
            STANDARD_RESOLUTIONS
                .into_iter()
                .filter(|dpi| (min..=max).contains(dpi) && (dpi - min) % step == 0),
        );
    }
    found.retain(|dpi| (1..=HIGHEST_RESOLUTION).contains(dpi));
    found.sort_unstable();
    found.dedup();
    found
}

pub fn parse_capabilities(bytes: &[u8]) -> Option<Scanner> {
    if bytes.len() > DESCRIPTION_LIMIT {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let document = Document::parse(text).ok()?;
    let root = document.root_element();
    let version = root
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Version")
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|version| {
            !version.is_empty()
                && version.len() < 8
                && version.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .unwrap_or(DEFAULT_VERSION)
        .to_owned();
    let mut sources = Vec::new();
    for (name, tag) in [(PLATEN, "PlatenInputCaps"), (FEEDER, "AdfSimplexInputCaps")] {
        let Some(caps) = named(root, tag) else {
            continue;
        };
        let modes = texts(caps, &["ColorMode"]);
        let formats = texts(caps, &["DocumentFormat", "DocumentFormatExt"]);
        let resolutions = resolutions(caps);
        let (Some(width), Some(height)) = (number(caps, "MaxWidth"), number(caps, "MaxHeight"))
        else {
            continue;
        };
        if resolutions.is_empty() || width == 0 || height == 0 {
            continue;
        }
        sources.push(Source {
            name,
            width,
            height,
            color: modes.iter().any(|mode| mode == COLOR),
            gray: modes.iter().any(|mode| mode == GRAY),
            jpeg: formats.iter().any(|format| format == JPEG),
            resolutions,
        });
    }
    (!sources.is_empty()).then_some(Scanner { version, sources })
}

fn base(address: &Address) -> &str {
    address.resource.trim_end_matches('/')
}

pub fn capabilities(address: &Address, pins: &Pins) -> Result<Scanner, ScanError> {
    let path = format!("{}/ScannerCapabilities", base(address));
    let response = http(
        address,
        pins,
        &HttpRequest {
            method: "GET",
            path: &path,
            body: None,
            most: DESCRIPTION_LIMIT,
            answer: ANSWER_TIME,
        },
    )?;
    match response.status {
        OK => parse_capabilities(&response.body).ok_or(ScanError::Unreadable),
        status => Err(ScanError::Refused(status)),
    }
}

fn settings(scanner: &Scanner, source: &Source, options: &ScanOptions) -> String {
    let mode = match (options.color && source.color, source.gray) {
        (false, true) => GRAY,
        _ => COLOR,
    };
    let dpi = source
        .resolutions
        .iter()
        .copied()
        .min_by_key(|dpi| dpi.abs_diff(options.resolution))
        .unwrap_or(options.resolution);
    let mut parts = scanner
        .version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let version = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let extended = match version >= (2, 1) {
        true => format!("<scan:DocumentFormatExt>{JPEG}</scan:DocumentFormatExt>"),
        false => String::new(),
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<scan:ScanSettings xmlns:scan=\"http://schemas.hp.com/imaging/escl/2011/05/03\" \
xmlns:pwg=\"http://www.pwg.org/schemas/2010/12/sm\">\
<pwg:Version>{version}</pwg:Version>\
<pwg:ScanRegions><pwg:ScanRegion>\
<pwg:Height>{height}</pwg:Height><pwg:Width>{width}</pwg:Width>\
<pwg:XOffset>0</pwg:XOffset><pwg:YOffset>0</pwg:YOffset>\
<pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>\
</pwg:ScanRegion></pwg:ScanRegions>\
<pwg:InputSource>{input}</pwg:InputSource>\
<scan:ColorMode>{mode}</scan:ColorMode>\
<scan:XResolution>{dpi}</scan:XResolution><scan:YResolution>{dpi}</scan:YResolution>\
<pwg:DocumentFormat>{JPEG}</pwg:DocumentFormat>{extended}\
</scan:ScanSettings>",
        version = scanner.version,
        height = source.height,
        width = source.width,
        input = source.name,
    )
}

fn job_path(address: &Address, location: &str) -> Option<String> {
    if location.is_empty()
        || !location
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'#')
    {
        return None;
    }
    let path = match location.split_once("://") {
        Some((_, rest)) => format!("/{}", rest.split_once('/')?.1),
        None if location.starts_with('/') => location.to_owned(),
        None => format!("{}/{location}", base(address)),
    };
    Some(path.trim_end_matches('/').to_owned())
}

fn get(address: &Address, pins: &Pins, path: &str, most: usize) -> Result<Response, ScanError> {
    let request = HttpRequest {
        method: "GET",
        path,
        body: None,
        most,
        answer: PAGE_TIME,
    };
    Ok(http(address, pins, &request)?)
}

/// Scans with `options`, giving each page as the scanner sent it to `keep`, and returns how many
/// pages there were.
pub fn scan(
    address: &Address,
    pins: &Pins,
    scanner: &Scanner,
    options: &ScanOptions,
    keep: &mut dyn FnMut(Vec<u8>) -> Result<(), ScanError>,
) -> Result<usize, ScanError> {
    let source = scanner
        .sources
        .iter()
        .find(|source| source.name == options.source && source.jpeg)
        .ok_or(ScanError::NotSupported)?;
    let body = settings(scanner, source, options);
    let jobs = format!("{}/ScanJobs", base(address));
    let mut retries = 0;
    let created = loop {
        let created = http(
            address,
            pins,
            &HttpRequest {
                method: "POST",
                path: &jobs,
                body: Some(("text/xml", body.as_bytes())),
                most: ANSWER_LIMIT,
                answer: ANSWER_TIME,
            },
        )?;
        match created.status {
            UNAVAILABLE if retries < BUSY_RETRIES => {
                retries += 1;
                std::thread::sleep(BUSY_PAUSE);
            }
            _ => break created,
        }
    };
    let feeder = source.name == FEEDER;
    let job = match created.status {
        CREATED => created
            .location
            .as_deref()
            .and_then(|location| job_path(address, location))
            .ok_or(ScanError::Unreadable)?,
        CONFLICT if feeder => return Err(ScanError::FeederEmpty),
        CONFLICT | UNAVAILABLE => return Err(ScanError::Busy),
        status => return Err(ScanError::Refused(status)),
    };
    let next = format!("{job}/NextDocument");
    let mut pages = 0;
    let mut retries = 0;
    while pages < MOST_PAGES {
        let response = get(address, pins, &next, PAGE_LIMIT)?;
        match response.status {
            OK => {
                retries = 0;
                pages += 1;
                keep(response.body)?;
                if !feeder {
                    break;
                }
            }
            NOT_FOUND if pages == 0 && feeder => return Err(ScanError::FeederEmpty),
            NOT_FOUND if pages > 0 => break,
            UNAVAILABLE if retries < BUSY_RETRIES => {
                retries += 1;
                std::thread::sleep(BUSY_PAUSE);
            }
            UNAVAILABLE => return Err(ScanError::Busy),
            status => return Err(ScanError::Refused(status)),
        }
    }
    if !feeder {
        let request = HttpRequest {
            method: "DELETE",
            path: &job,
            body: None,
            most: ANSWER_LIMIT,
            answer: ANSWER_TIME,
        };
        if let Err(error) = http(address, pins, &request) {
            tracing::debug!("cannot end a scan job: {error}");
        }
    }
    Ok(pages)
}

fn decoded(page: &[u8]) -> Result<(u32, u32, Vec<u8>), ScanError> {
    static PAGES: AtomicU32 = AtomicU32::new(0);
    let folder = runtime_dir().ok_or(ScanError::Unreadable)?;
    let path = folder.join(format!(
        "bande-scan-{}-{}",
        std::process::id(),
        PAGES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| file.write_all(page));
    let decoded = written
        .map_err(|error| {
            tracing::warn!("cannot keep a scanned page: {error}");
            ScanError::Unreadable
        })
        .and_then(|()| {
            decode_confined(&path, u32::MAX).map_err(|failure| {
                tracing::info!("cannot read a scanned page: {failure:?}");
                ScanError::Unreadable
            })
        });
    let _ = std::fs::remove_file(&path);
    let Decoded {
        pixels: Pixels::Still(levels),
        ..
    } = decoded?
    else {
        return Err(ScanError::Unreadable);
    };
    let level = levels.into_iter().next().ok_or(ScanError::Unreadable)?;
    let rgba = Arc::try_unwrap(level.rgba).unwrap_or_else(|shared| (*shared).clone());
    Ok((level.width, level.height, rgba))
}

/// Reads a page in a confined decoder and writes it again as a JPEG of its own, so that nothing
/// the scanner sent reaches the person's files.
pub fn clean_page(page: &[u8], color: bool) -> Result<OwnedFd, ScanError> {
    let (width, height, rgba) = decoded(page)?;
    let pixels = rgba.as_chunks::<4>().0.iter();
    let (pixels, kind): (Vec<u8>, _) = match color {
        true => (
            pixels.flat_map(|&[r, g, b, _]| [r, g, b]).collect(),
            ExtendedColorType::Rgb8,
        ),
        false => (
            pixels
                .map(|&[r, g, b, _]| {
                    let [r, g, b] = [r, g, b].map(u32::from);
                    ((r * 299 + g * 587 + b * 114) / 1000) as u8
                })
                .collect(),
            ExtendedColorType::L8,
        ),
    };
    let failed = |error: &dyn std::fmt::Display| {
        tracing::warn!("cannot write a scanned page: {error}");
        ScanError::Unreadable
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(&pixels, width, height, kind)
        .map_err(|error| failed(&error))?;
    let memory = memfd_create("bande-scan", MemfdFlags::CLOEXEC).map_err(|error| failed(&error))?;
    let mut file = File::from(memory);
    file.write_all(&jpeg)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(drop))
        .map_err(|error| failed(&error))?;
    Ok(OwnedFd::from(file))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    use super::*;

    const CAPABILITIES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScannerCapabilities xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
  <pwg:Version>2.63</pwg:Version>
  <pwg:MakeAndModel>Office Jet</pwg:MakeAndModel>
  <scan:Platen><scan:PlatenInputCaps>
    <scan:MaxWidth>2550</scan:MaxWidth><scan:MaxHeight>3508</scan:MaxHeight>
    <scan:SettingProfiles><scan:SettingProfile>
      <scan:ColorModes><scan:ColorMode>RGB24</scan:ColorMode><scan:ColorMode>Grayscale8</scan:ColorMode></scan:ColorModes>
      <scan:DocumentFormats><pwg:DocumentFormat>application/pdf</pwg:DocumentFormat><pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat></scan:DocumentFormats>
      <scan:SupportedResolutions><scan:DiscreteResolutions>
        <scan:DiscreteResolution><scan:XResolution>100</scan:XResolution><scan:YResolution>100</scan:YResolution></scan:DiscreteResolution>
        <scan:DiscreteResolution><scan:XResolution>300</scan:XResolution><scan:YResolution>300</scan:YResolution></scan:DiscreteResolution>
        <scan:DiscreteResolution><scan:XResolution>4800</scan:XResolution><scan:YResolution>4800</scan:YResolution></scan:DiscreteResolution>
      </scan:DiscreteResolutions></scan:SupportedResolutions>
    </scan:SettingProfile></scan:SettingProfiles>
  </scan:PlatenInputCaps></scan:Platen>
  <scan:Adf><scan:AdfSimplexInputCaps>
    <scan:MaxWidth>2550</scan:MaxWidth><scan:MaxHeight>4200</scan:MaxHeight>
    <scan:SettingProfiles><scan:SettingProfile>
      <scan:ColorModes><scan:ColorMode>Grayscale8</scan:ColorMode></scan:ColorModes>
      <scan:DocumentFormats><scan:DocumentFormatExt>image/jpeg</scan:DocumentFormatExt></scan:DocumentFormats>
      <scan:SupportedResolutions><scan:XResolutionRange><scan:Min>150</scan:Min><scan:Max>600</scan:Max><scan:Step>150</scan:Step></scan:XResolutionRange></scan:SupportedResolutions>
    </scan:SettingProfile></scan:SettingProfiles>
  </scan:AdfSimplexInputCaps></scan:Adf>
</scan:ScannerCapabilities>"#;

    fn page() -> Vec<u8> {
        let mut jpeg = Vec::new();
        JpegEncoder::new(&mut jpeg)
            .encode(&[128; 64], 8, 8, ExtendedColorType::L8)
            .unwrap();
        jpeg
    }

    fn answer(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend(body);
        out
    }

    fn device(mut respond: impl FnMut(&str, &str) -> Vec<u8> + Send + 'static) -> Address {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let line = format!("{request}{}", String::from_utf8_lossy(&body));
                let reply = respond(request.split(' ').nth(1).unwrap(), &line);
                (&stream).write_all(&reply).unwrap();
            }
        });
        Address::parse(&format!("http://127.0.0.1:{port}/eSCL")).unwrap()
    }

    #[test]
    fn a_feeder_scan_follows_its_job_to_the_last_page() {
        let mut sent = 0;
        let address = device(move |path, request| match path {
            "/eSCL/ScannerCapabilities" => answer("200 OK", "", CAPABILITIES.as_bytes()),
            "/eSCL/ScanJobs" => {
                assert!(request.contains("<pwg:InputSource>Feeder</pwg:InputSource>"));
                assert!(request.contains("<scan:ColorMode>Grayscale8</scan:ColorMode>"));
                assert!(request.contains("<scan:XResolution>300</scan:XResolution>"));
                assert!(request.contains("<pwg:Height>4200</pwg:Height>"));
                answer(
                    "201 Created",
                    "Location: http://elsewhere.example/eSCL/ScanJobs/7/\r\n",
                    b"",
                )
            }
            "/eSCL/ScanJobs/7/NextDocument" if sent < 2 => {
                sent += 1;
                answer("200 OK", "Content-Type: image/jpeg\r\n", &page())
            }
            _ => answer("404 Not Found", "", b""),
        });
        let pins = Pins::default();
        let scanner = capabilities(&address, &pins).unwrap();
        assert_eq!(
            scanner.public(),
            ScanCapabilities {
                sources: vec![PLATEN.to_owned(), FEEDER.to_owned()],
                color: true,
                resolutions: vec![100, 150, 300, 600],
            }
        );
        let options = ScanOptions {
            source: FEEDER.to_owned(),
            color: true,
            resolution: 300,
        };
        let mut pages = Vec::new();
        let scanned = scan(&address, &pins, &scanner, &options, &mut |page| {
            pages.push(page);
            Ok(())
        });
        assert_eq!(scanned, Ok(2));
        assert_eq!(pages, vec![page(), page()]);
    }

    #[test]
    fn a_device_cannot_inject_requests_or_send_without_limit() {
        let address = device(|path, _| match path {
            "/eSCL/ScannerCapabilities" => answer("200 OK", "", CAPABILITIES.as_bytes()),
            "/eSCL/ScanJobs" => answer(
                "201 Created",
                "Location: /eSCL/ScanJobs/1 HTTP/1.1\r\n",
                b"",
            ),
            "/eSCL/ScanJobs/1" => answer("200 OK", "", &page()),
            _ => b"HTTP/1.1 200 OK\r\nContent-Length: 999999999999\r\n\r\n".to_vec(),
        });
        let pins = Pins::default();
        let scanner = capabilities(&address, &pins).unwrap();
        let options = ScanOptions {
            source: PLATEN.to_owned(),
            color: false,
            resolution: 300,
        };
        let scanned = scan(&address, &pins, &scanner, &options, &mut |_| Ok(()));
        assert_eq!(scanned, Err(ScanError::Unreadable));
        let next = get(&address, &pins, "/eSCL/ScanJobs/1/NextDocument", PAGE_LIMIT);
        assert_eq!(next.err(), Some(ScanError::Unreadable));
        assert!(parse_capabilities(b"<!DOCTYPE x [<!ENTITY a \"b\">]><x>&a;</x>").is_none());
    }
}
