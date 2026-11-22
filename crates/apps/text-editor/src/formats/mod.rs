pub mod docx;
pub mod markdown;
pub mod odt;
pub mod rtf;

use std::io::{Read, Seek};

use bande_toolkit::ui::RichText;

pub use bande_rich_text::{
    Builder, HEADINGS, MAX_NESTING, MONOSPACE, Paragraph, code, family, heading, heading_level,
    html, is_code, link, paragraphs,
};

const MAX_PART_BYTES: u64 = 64 << 20;
const MAX_XML_NODES: u32 = 4_000_000;

pub struct Opened {
    pub rich: RichText,
    pub lossy: bool,
}

pub fn options<'input>() -> roxmltree::ParsingOptions<'input> {
    roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..roxmltree::ParsingOptions::default()
    }
}

pub fn part(archive: &mut zip::ZipArchive<impl Read + Seek>, name: &str) -> Result<String, String> {
    let file = archive.by_name(name).map_err(|error| error.to_string())?;
    if file.size() > MAX_PART_BYTES {
        return Err(format!("{name} is too large"));
    }
    let mut text = String::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    match text.len() as u64 > MAX_PART_BYTES {
        true => Err(format!("{name} is too large")),
        false => Ok(text),
    }
}

pub fn xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            character if (character as u32) < 0x20 && character != '\t' => {}
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use bande_toolkit::ui::{Alignment, ListMarker};

    #[test]
    fn rich_formats_read_styles_and_write_them_back() {
        let source = "# Title\n\nSome **bold**, *italic*, `code`, ~~gone~~ and a [link](https://example.org).\n\n- one\n  - nested\n\n1. first\n2. second\n\n```\nfn main() {}\n\nlet x = 1;\n```\n\n> quoted\n";
        let rich = markdown::read(source);
        assert_eq!(
            rich.text,
            "Title\n\nSome bold, italic, code, gone and a link.\n\none\nnested\n\nfirst\nsecond\n\nfn main() {}\n\nlet x = 1;\n\nquoted"
        );
        assert_eq!(markdown::write(&rich), source);
        assert_eq!(
            markdown::read("2 * 3 and snake_case **open").runs,
            Vec::new()
        );

        let rtf = rtf::read(
            br"{\rtf1\ansi{\fonttbl{\f1 Courier;}}{\colortbl;\red255\green0\blue0;}
\pard\qc{\b Bold \i both}\par
\pard{\listtext \'95\tab}\ls1 caf\'e9 \u8364? {\f1 code}{\cf1 red}\par
{\*\generator skipped}\pard plain\par}",
        );
        assert_eq!(rtf.text, "Bold both\ncafé € codered\nplain");
        assert_eq!(rtf.paragraphs[0].alignment, Alignment::Center);
        assert_eq!(rtf.paragraphs[1].list, Some(ListMarker::Bullet));
        assert_eq!(rtf::read(rtf::write(&rtf).as_bytes()), rtf);

        let page = "<head><title>T</title><script>alert(1)</script></head><h1>Heading</h1>\
            <p align=center>Some <b>bold</b> and <a href=\"https://example.org\">a link</a> &amp; more</p>\
            <ul><li>one<ol><li>deep</li></ol></li></ul><table><tr><td>a</td><td>b</td></tr></table>";
        let html = html::read(page);
        assert_eq!(
            html.text,
            "Heading\nSome bold and a link & more\none\ndeep\na\tb"
        );
        assert_eq!(html.paragraphs[3].list, Some(ListMarker::Number));
        assert_eq!(html::read(&html::write(&html, "T")), html);
        for office in [docx::write(&html), odt::write(&html)] {
            let bytes = office.unwrap_or_default();
            let read = docx::read(&bytes)
                .or_else(|_| odt::read(&bytes))
                .map(|opened| opened.rich);
            assert_eq!(read.ok(), Some(html.clone()));
        }
    }
}
