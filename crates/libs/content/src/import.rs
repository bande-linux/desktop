use std::borrow::Cow;
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::pdf::worker::document_text;
use crate::{Block, html, rtf};

const MAX_PART_BYTES: u64 = 64 << 20;
const MAX_PARTS: usize = 4096;
const WORD_BREAKS: &[&[u8]] = &[
    b"p",
    b"h",
    b"h1",
    b"h2",
    b"h3",
    b"h4",
    b"h5",
    b"h6",
    b"br",
    b"tab",
    b"s",
    b"tr",
    b"td",
    b"th",
    b"tc",
    b"li",
    b"div",
    b"si",
    b"c",
    b"title",
    b"list-item",
    b"table-cell",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Importer {
    Pdf,
    Rtf,
    Html,
    Word,
    Slides,
    Sheets,
    OpenDocument,
    Epub,
}

impl Importer {
    pub const ALL: [Self; 8] = [
        Self::Pdf,
        Self::Rtf,
        Self::Html,
        Self::Word,
        Self::Slides,
        Self::Sheets,
        Self::OpenDocument,
        Self::Epub,
    ];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Rtf => "rtf",
            Self::Html => "html",
            Self::Word => "word",
            Self::Slides => "slides",
            Self::Sheets => "sheets",
            Self::OpenDocument => "opendocument",
            Self::Epub => "epub",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|importer| importer.keyword() == keyword)
    }

    pub fn for_extension(extension: &str) -> Option<Self> {
        match extension {
            "pdf" => Some(Self::Pdf),
            "rtf" => Some(Self::Rtf),
            "html" | "htm" | "xhtml" => Some(Self::Html),
            "docx" | "docm" | "dotx" => Some(Self::Word),
            "pptx" | "pptm" | "potx" => Some(Self::Slides),
            "xlsx" | "xlsm" | "xltx" => Some(Self::Sheets),
            "odt" | "ods" | "odp" | "ott" | "ots" | "otp" | "odg" => Some(Self::OpenDocument),
            "epub" => Some(Self::Epub),
            _ => None,
        }
    }

    pub fn import(self, bytes: Vec<u8>, limit: usize) -> String {
        match self {
            Self::Pdf => document_text(bytes, limit),
            Self::Rtf => blocks_text(&rtf(&String::from_utf8_lossy(&bytes))),
            Self::Html => blocks_text(&html(&String::from_utf8_lossy(&bytes))),
            Self::Word => parts_text(bytes, limit, |name| {
                name == "word/document.xml"
                    || name.starts_with("word/header")
                    || name.starts_with("word/footer")
                    || name == "word/footnotes.xml"
                    || name == "docProps/core.xml"
            }),
            Self::Slides => parts_text(bytes, limit, |name| {
                name.starts_with("ppt/slides/slide")
                    || name.starts_with("ppt/notesSlides/")
                    || name == "docProps/core.xml"
            }),
            Self::Sheets => parts_text(bytes, limit, |name| {
                name == "xl/sharedStrings.xml"
                    || name.starts_with("xl/worksheets/sheet")
                    || name == "docProps/core.xml"
            }),
            Self::OpenDocument => parts_text(bytes, limit, |name| {
                name == "content.xml" || name == "meta.xml"
            }),
            Self::Epub => parts_text(bytes, limit, |name| {
                name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".opf")
            }),
        }
    }
}

fn blocks_text(blocks: &[Block]) -> String {
    let mut text = String::new();
    for block in blocks {
        match block {
            Block::Heading(_, line)
            | Block::Paragraph(line)
            | Block::Bullet(line)
            | Block::Quote(line)
            | Block::Code(line)
            | Block::Plain(line) => {
                text.push_str(line);
                text.push('\n');
            }
            Block::Rule => {}
        }
    }
    text
}

fn parts_text(bytes: Vec<u8>, limit: usize, wanted: impl Fn(&str) -> bool) -> String {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return String::new();
    };
    let mut names: Vec<String> = archive
        .file_names()
        .flatten()
        .filter(|name| wanted(name))
        .take(MAX_PARTS)
        .map(Cow::into_owned)
        .collect();
    names.sort();
    let mut text = String::new();
    for name in names {
        if text.len() >= limit {
            break;
        }
        let Ok(part) = archive.by_name(&name) else {
            continue;
        };
        let mut xml = Vec::new();
        if part.take(MAX_PART_BYTES).read_to_end(&mut xml).is_ok() {
            xml_text(&xml, limit, &mut text);
            text.push('\n');
        }
    }
    text
}

fn xml_text(xml: &[u8], limit: usize, text: &mut String) {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = false;
    let mut skipping = 0usize;
    let mut buffer = Vec::new();
    while text.len() < limit {
        buffer.clear();
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Text(words)) if skipping == 0 => {
                if let Ok(words) = words.decode() {
                    text.push_str(&words);
                }
            }
            Ok(Event::CData(words)) if skipping == 0 => {
                text.push_str(&String::from_utf8_lossy(&words));
            }
            Ok(Event::GeneralRef(reference)) if skipping == 0 => {
                match reference.resolve_char_ref() {
                    Ok(Some(character)) => text.push(character),
                    _ => match &*reference {
                        b"amp" => text.push('&'),
                        b"lt" => text.push('<'),
                        b"gt" => text.push('>'),
                        b"quot" => text.push('"'),
                        b"apos" => text.push('\''),
                        _ => text.push(' '),
                    },
                }
            }
            Ok(Event::Start(tag)) => {
                let name = tag.local_name();
                if matches!(name.as_ref(), b"script" | b"style") {
                    skipping += 1;
                }
            }
            Ok(Event::End(tag)) => {
                let name = tag.local_name();
                if matches!(name.as_ref(), b"script" | b"style") {
                    skipping = skipping.saturating_sub(1);
                } else if WORD_BREAKS.contains(&name.as_ref()) {
                    text.push(' ');
                }
            }
            Ok(Event::Empty(tag)) => {
                if WORD_BREAKS.contains(&tag.local_name().as_ref()) {
                    text.push(' ');
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_across_runs_stay_whole_and_paragraphs_stay_apart() {
        let mut text = String::new();
        xml_text(
            b"<w:p><w:r><w:t>Quar</w:t></w:r><w:r><w:t>terly</w:t></w:r></w:p><w:p><w:t>a &amp; b</w:t></w:p>",
            1000,
            &mut text,
        );
        assert_eq!(text, "Quarterly a & b ");
    }
}
