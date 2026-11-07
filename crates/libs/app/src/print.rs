use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bande_pdf::{Block, Orientation, Page, PageSetup, Paper, flow, write};
use bande_toolkit::ui::{RichText, TextView};
use bande_toolkit::{Color, Image, Item, Rect, TextAlign, TextStyle, TextSystem, Typeface};

const DIALOG_PROGRAM: &str = "/usr/libexec/bande-file-chooser";

pub type Layout = Box<dyn FnMut(&PageSetup, &mut TextSystem) -> Vec<Page> + Send>;

pub enum Document {
    Pages(Layout),
    File(PathBuf),
}

const BODY_SIZE: f32 = 11.0;
const IMAGE_PIXELS: u32 = 4096;

pub fn text_document(text: String, monospace: bool) -> Document {
    Document::Pages(Box::new(move |setup, system| {
        let width = setup.content().width;
        let mut style = TextStyle::new(BODY_SIZE, Color::BLACK);
        if monospace {
            style.typeface = Typeface::Monospace;
        }
        let blocks: Vec<Block> = text
            .lines()
            .map(|line| Block {
                paragraph: system.paragraph(line, style, Some(width), TextAlign::Start),
                marker: None,
                indent: 0.0,
                space_after: 0.0,
            })
            .collect();
        flow(&blocks, setup)
    }))
}

pub fn rich_document(rich: RichText, monospace: bool) -> Document {
    Document::Pages(Box::new(move |setup, system| {
        let width = setup.content().width;
        let blocks: Vec<Block> =
            TextView::printed_blocks(&rich, system, BODY_SIZE, width, monospace)
                .into_iter()
                .map(|block| Block {
                    paragraph: block.paragraph,
                    marker: block.marker,
                    indent: block.indent,
                    space_after: 0.0,
                })
                .collect();
        flow(&blocks, setup)
    }))
}

pub fn image_document(path: PathBuf) -> Option<Document> {
    let image = Image::load(&path, IMAGE_PIXELS)?;
    Some(Document::Pages(Box::new(move |setup, _| {
        let area = setup.content();
        let (width, height) = (image.width() as f32, image.height() as f32);
        let scale = (area.width / width).min(area.height / height);
        let (shown_width, shown_height) = (width * scale, height * scale);
        let mut page = Page::new(setup);
        page.draw(Item::Image {
            rect: Rect::new(
                area.x + (area.width - shown_width) / 2.0,
                area.y + (area.height - shown_height) / 2.0,
                shown_width,
                shown_height,
            ),
            image: image.clone(),
        });
        vec![page]
    })))
}

pub fn can_print(mime: &str) -> bool {
    mime == "application/pdf" || mime.starts_with("image/") || mime.starts_with("text/")
}

pub fn file_document(path: &Path, mime: &str) -> Option<Document> {
    match mime {
        "application/pdf" => Some(Document::File(path.to_path_buf())),
        kind if kind.starts_with("image/") => image_document(path.to_path_buf()),
        kind if kind.starts_with("text/") => std::fs::read_to_string(path)
            .ok()
            .map(|text| text_document(text, kind != "text/plain")),
        _ => None,
    }
}

fn dialog(title: &str, id: &str) -> Command {
    let mut command = Command::new(DIALOG_PROGRAM);
    command
        .arg("--print")
        .args(["--app-id", id])
        .args(["--title", title]);
    let parent = crate::parent_window();
    if !parent.is_empty() {
        command.args(["--parent", &parent]);
    }
    command
}

fn page_setup(request: &str) -> Option<PageSetup> {
    let (keyword, landscape) = request.trim().split_once(' ')?;
    Some(PageSetup {
        paper: Paper::with_keyword(keyword).unwrap_or(Paper::A4),
        orientation: match landscape {
            "1" => Orientation::Landscape,
            _ => Orientation::Portrait,
        },
        ..PageSetup::default()
    })
}

fn serve_layouts(title: &str, id: &str, mut layout: Layout) -> io::Result<()> {
    let mut child = dialog(title, id)
        .arg("--from-app")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut answers), Some(requests)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(io::Error::other("the print dialog has no pipes"));
    };
    let mut text = TextSystem::new();
    for request in BufReader::new(requests).lines() {
        let Some(setup) = page_setup(&request?) else {
            continue;
        };
        let pages = layout(&setup, &mut text);
        let bytes = write(title, &pages).map_err(io::Error::other)?;
        writeln!(answers, "{}", bytes.len())?;
        answers.write_all(&bytes)?;
        answers.flush()?;
    }
    child.wait().map(drop)
}

fn show_file(title: &str, id: &str, path: &Path) -> io::Result<()> {
    dialog(title, id)
        .arg("--document")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map(drop)
}

pub fn print(title: &str, id: &str, document: Document) {
    let (title, id) = (title.to_owned(), id.to_owned());
    std::thread::spawn(move || {
        let shown = match document {
            Document::Pages(layout) => serve_layouts(&title, &id, layout),
            Document::File(path) => show_file(&title, &id, &path),
        };
        if let Err(error) = shown {
            tracing::warn!("cannot show the print dialog: {error}");
        }
    });
}
