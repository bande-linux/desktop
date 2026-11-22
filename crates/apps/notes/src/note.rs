use std::hash::{BuildHasher, RandomState};

use bande_mail_core::mime::{encoded_words, quoted_printable, unique_token};
use bande_rich_text::{heading, heading_level, html, link, paragraphs};
use bande_toolkit::ui::{ListMarker, ParagraphAttributes, RichText, TextAttributes};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mailparse::{MailHeaderMap, ParsedMail};

pub const NOTE_TYPE: &str = "com.apple.mail-note";
pub const MAX_NOTE_BYTES: usize = 16 << 20;
const MAX_DEPTH: usize = 16;
const MAX_HEADING: usize = 3;
const MAX_LEVEL: u8 = 8;
const SNIPPET_CHARS: usize = 120;
const LINK_SCHEMES: [&str; 3] = ["http:", "https:", "mailto:"];

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub uuid: String,
    pub created: i64,
    pub modified: i64,
    pub rich: RichText,
    pub extras: bool,
}

pub fn new_uuid() -> String {
    let state = RandomState::new();
    let now = Timestamp::now().as_nanosecond();
    let high = state.hash_one((now, std::process::id()));
    let low = state.hash_one((high, now));
    let hex = format!("{high:016X}{low:016X}");
    format!(
        "{}-{}-4{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn valid_uuid(uuid: &str) -> bool {
    !uuid.is_empty()
        && uuid.len() <= 64
        && uuid
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
}

fn allowed_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    LINK_SCHEMES.iter().any(|scheme| lower.starts_with(scheme))
}

fn sanitised(rich: RichText) -> RichText {
    let runs = rich
        .runs
        .into_iter()
        .filter_map(|(range, attributes)| {
            let level = heading_level(&attributes).filter(|level| *level <= MAX_HEADING);
            let base = match &attributes.link {
                Some(url) if allowed_link(url) => link(url),
                _ => TextAttributes::default(),
            };
            let kept = TextAttributes {
                bold: attributes.bold,
                italic: attributes.italic,
                underline: attributes.underline && attributes.link.is_none(),
                strikethrough: attributes.strikethrough,
                size: level.and_then(|level| heading(level).size),
                ..base
            };
            (kept != TextAttributes::default()).then_some((range, kept))
        })
        .collect();
    let paragraphs = rich
        .paragraphs
        .into_iter()
        .map(|paragraph| ParagraphAttributes {
            list: paragraph.list,
            level: match paragraph.list {
                Some(_) => paragraph.level.min(MAX_LEVEL),
                None => 0,
            },
            ..ParagraphAttributes::default()
        })
        .collect();
    RichText {
        text: rich.text.replace('\u{a0}', " "),
        runs,
        paragraphs,
    }
}

pub fn from_html(body: &str) -> RichText {
    sanitised(html::read(body))
}

pub fn from_plain(text: &str) -> RichText {
    let text = text.replace("\r\n", "\n");
    let text = text.trim_end_matches('\n');
    RichText {
        text: text.to_owned(),
        runs: Vec::new(),
        paragraphs: vec![ParagraphAttributes::default(); text.split('\n').count()],
    }
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            character if character.is_control() => {}
            character => out.push(character),
        }
    }
}

fn list_tag(marker: ListMarker) -> &'static str {
    match marker {
        ListMarker::Bullet => "ul",
        ListMarker::Number => "ol",
    }
}

pub fn to_html(rich: &RichText) -> String {
    let mut out = String::new();
    let mut lists: Vec<ListMarker> = Vec::new();
    for paragraph in paragraphs(rich) {
        let attributes = paragraph.attributes;
        let depth = attributes
            .list
            .map_or(0, |_| usize::from(attributes.level.min(MAX_LEVEL)) + 1);
        while lists.len() > depth
            || lists.len() == depth && depth > 0 && lists.last().copied() != attributes.list
        {
            if let Some(marker) = lists.pop() {
                out.push_str(&format!("</{}>", list_tag(marker)));
            }
        }
        while lists.len() < depth {
            let marker = attributes.list.unwrap_or(ListMarker::Bullet);
            out.push_str(&format!("<{}>", list_tag(marker)));
            lists.push(marker);
        }
        let level = heading_level(&paragraph.first())
            .filter(|level| *level <= MAX_HEADING && depth == 0 && !paragraph.is_empty());
        let tag = match (depth, level) {
            (0, Some(level)) => format!("h{level}"),
            (0, None) => "div".to_owned(),
            _ => "li".to_owned(),
        };
        out.push_str(&format!("<{tag}>"));
        if paragraph.is_empty() {
            out.push_str("<br>");
        }
        for (text, span) in &paragraph.spans {
            if text.is_empty() {
                continue;
            }
            let mut close = Vec::new();
            if let Some(url) = span.link.as_ref().filter(|url| allowed_link(url)) {
                let mut href = String::new();
                escape(url, &mut href);
                out.push_str(&format!("<a href=\"{href}\">"));
                close.push("</a>");
            }
            for (on, open, end) in [
                (span.bold && level.is_none(), "<b>", "</b>"),
                (span.italic, "<i>", "</i>"),
                (span.underline && span.link.is_none(), "<u>", "</u>"),
                (span.strikethrough, "<strike>", "</strike>"),
            ] {
                if on {
                    out.push_str(open);
                    close.push(end);
                }
            }
            escape(text, &mut out);
            for end in close.iter().rev() {
                out.push_str(end);
            }
        }
        out.push_str(&format!("</{tag}>"));
    }
    while let Some(marker) = lists.pop() {
        out.push_str(&format!("</{}>", list_tag(marker)));
    }
    out
}

pub fn title(rich: &RichText) -> String {
    rich.text
        .split('\n')
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

pub fn snippet(rich: &RichText) -> String {
    let mut lines = rich
        .text
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty());
    lines.next();
    lines
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(SNIPPET_CHARS)
        .collect()
}

fn header_date(seconds: i64) -> String {
    Timestamp::from_second(seconds)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned(TimeZone::system())
        .strftime("%a, %d %b %Y %H:%M:%S %z")
        .to_string()
}

fn header_text(text: &str) -> String {
    let text = text
        .chars()
        .map(|character| match character.is_control() {
            true => ' ',
            false => character,
        })
        .collect::<String>();
    encoded_words(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub fn write(note: &Note, name: &str, email: &str) -> Vec<u8> {
    let email = email.replace(
        |character: char| character.is_control() || "<>".contains(character),
        "",
    );
    let from = match name.trim() {
        "" => format!("<{email}>"),
        name => format!("{} <{email}>", header_text(&name.replace('"', ""))),
    };
    let domain = email
        .rsplit_once('@')
        .map_or("localhost", |(_, domain)| domain);
    let body = format!(
        "<html><head></head><body>{}</body></html>",
        to_html(&note.rich)
    );
    format!(
        "From: {from}\r\n\
         Date: {}\r\n\
         X-Uniform-Type-Identifier: {NOTE_TYPE}\r\n\
         X-Universally-Unique-Identifier: {}\r\n\
         X-Mail-Created-Date: {}\r\n\
         Subject: {}\r\n\
         Message-Id: <{}@{domain}>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Transfer-Encoding: quoted-printable\r\n\r\n\
         {}\r\n",
        header_date(note.modified),
        note.uuid,
        header_date(note.created),
        header_text(&title(&note.rich)),
        unique_token(),
        quoted_printable(&body),
    )
    .into_bytes()
}

#[derive(Default)]
struct Parts {
    html: Option<String>,
    plain: Option<String>,
    extras: bool,
}

fn walk(part: &ParsedMail<'_>, depth: usize, parts: &mut Parts) {
    if depth > MAX_DEPTH {
        parts.extras = true;
        return;
    }
    let mime = part.ctype.mimetype.to_ascii_lowercase();
    if mime.starts_with("multipart/") {
        for child in &part.subparts {
            walk(child, depth + 1, parts);
        }
        return;
    }
    let attachment =
        part.get_content_disposition().disposition == mailparse::DispositionType::Attachment;
    let body = || part.get_body().ok();
    match mime.as_str() {
        "text/html" if !attachment && parts.html.is_none() => parts.html = body(),
        "text/plain" if !attachment && parts.plain.is_none() => parts.plain = body(),
        _ => parts.extras = true,
    }
}

pub fn read(raw: &[u8]) -> Option<Note> {
    if raw.len() > MAX_NOTE_BYTES {
        return None;
    }
    let parsed = mailparse::parse_mail(raw).ok()?;
    let headers = &parsed.headers;
    let date = |name: &str| {
        headers
            .get_first_value(name)
            .and_then(|value| mailparse::dateparse(&value).ok())
    };
    let modified = date("Date").unwrap_or_else(|| Timestamp::now().as_second());
    let created = date("X-Mail-Created-Date").unwrap_or(modified);
    let uuid = headers
        .get_first_value("X-Universally-Unique-Identifier")
        .map(|uuid| uuid.trim().to_owned())
        .filter(|uuid| valid_uuid(uuid))
        .unwrap_or_else(new_uuid);
    let mut parts = Parts::default();
    walk(&parsed, 0, &mut parts);
    let (rich, extras) = match (parts.html, parts.plain) {
        (Some(body), _) => {
            let lower = body.to_ascii_lowercase();
            let embedded = ["<img", "<object", "<embed", "<iframe", "<video", "<audio"]
                .iter()
                .any(|tag| lower.contains(tag));
            (from_html(&body), parts.extras || embedded)
        }
        (None, Some(text)) => (from_plain(&text), parts.extras),
        (None, None) => (RichText::default(), true),
    };
    Some(Note {
        uuid,
        created,
        modified,
        rich,
        extras,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRITTEN_ELSEWHERE: &str = "From: Ann Lee <ann@example.org>\r\n\
X-Uniform-Type-Identifier: com.apple.mail-note\r\n\
Content-Type: text/html;\r\n\tcharset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
X-Mail-Created-Date: Sat, 15 Mar 2014 10:12:00 +0100\r\n\
Subject: Shopping\r\n\
Date: Sun, 16 Mar 2014 08:00:00 +0100\r\n\
X-Universally-Unique-Identifier: 5A9B4C8E-1F2D-4E3A-9B7C-0D1E2F3A4B5C\r\n\
Message-Id: <1@example.org>\r\n\
Mime-Version: 1.0\r\n\r\n\
<html><head></head><body style=3D\"word-wrap: break-word; -webkit-nbsp-mode: s=\r\n\
pace;\">Shopping<div><br></div><div><b>Caf=C3=A9</b> &amp; <i>cr=C3=A8me</i>&nbsp;=\r\n\
=3D 2</div><ul><li>milk</li><li><a href=3D\"https://example.org\">site</a></li>=\r\n\
<li><a href=3D\"javascript:alert(1)\">bad</a></li></ul><h1>Big</h1><div><script>al=\r\n\
ert(1)</script><font color=3D\"red\" face=3D\"Courier\">red</font></div></body></html>\r\n";

    #[test]
    fn notes_from_other_clients_read_sanitised_and_write_back_the_same() {
        let note = read(WRITTEN_ELSEWHERE.as_bytes()).unwrap_or_else(|| unreachable!());
        assert_eq!(note.uuid, "5A9B4C8E-1F2D-4E3A-9B7C-0D1E2F3A4B5C");
        assert_eq!(note.created, 1_394_874_720);
        assert_eq!(note.modified, 1_394_953_200);
        assert!(!note.extras);
        assert_eq!(
            note.rich.text,
            "Shopping\n\nCafé & crème = 2\nmilk\nsite\nbad\nBig\nred"
        );
        assert_eq!(title(&note.rich), "Shopping");
        assert_eq!(note.rich.paragraphs[3].list, Some(ListMarker::Bullet));
        let links = note
            .rich
            .runs
            .iter()
            .filter_map(|(_, attributes)| attributes.link.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(links, ["https://example.org"]);
        assert!(
            note.rich
                .runs
                .iter()
                .all(|(_, attributes)| attributes.family.is_none())
        );

        let written = write(&note, "Ann Lee", "ann@example.org");
        let text = String::from_utf8_lossy(&written);
        assert!(text.contains("X-Uniform-Type-Identifier: com.apple.mail-note\r\n"));
        assert!(
            text.contains(
                "X-Universally-Unique-Identifier: 5A9B4C8E-1F2D-4E3A-9B7C-0D1E2F3A4B5C\r\n"
            )
        );
        assert!(text.contains("Subject: Shopping\r\n"));
        assert!(text.lines().all(|line| line.len() <= 76));
        assert!(!text.contains("script") && !text.contains("javascript"));
        let again = read(&written).unwrap_or_else(|| unreachable!());
        assert_eq!(
            (again.uuid.as_str(), again.created),
            (note.uuid.as_str(), note.created)
        );
        assert_eq!(again.rich, note.rich);

        let unicode = Note {
            rich: from_plain("naïve = ok\n.dot first &amp; <b>"),
            ..note
        };
        let written = write(&unicode, "Åsa", "asa@example.org");
        let text = String::from_utf8_lossy(&written);
        assert!(text.contains("Subject: =?UTF-8?B?"));
        assert_eq!(read(&written).map(|note| note.rich), Some(unicode.rich));

        let with_picture = "Content-Type: multipart/related; boundary=b\r\n\r\n--b\r\n\
Content-Type: text/html\r\n\r\n<div>See<img src=\"cid:1\"></div>\r\n--b\r\n\
Content-Type: image/png\r\nContent-Id: <1>\r\n\r\nPNG\r\n--b--\r\n";
        let note = read(with_picture.as_bytes()).unwrap_or_else(|| unreachable!());
        assert!(note.extras);
        assert_eq!(note.rich.text, "See");
        assert!(valid_uuid(&note.uuid));
    }
}
