use web_atoms::Atom;
use web_dom::{Document, NodeId};
use web_html::{Options, Parser};
use web_layout::{BoxTree, FragmentTree};
use web_paint::DisplayList;
use web_style::Styles;

pub use web_text::Fonts;

pub struct Page {
    pub document: Document,
    pub styles: Styles,
    pub boxes: BoxTree,
    pub fragments: FragmentTree,
    pub display_list: DisplayList,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Memory {
    pub elements: usize,
    pub dom: usize,
    pub styles: usize,
    pub boxes: usize,
    pub fragments: usize,
    pub display_list: usize,
}

pub fn parse(bytes: &[u8]) -> Document {
    let mut parser = Parser::new(Document::new(), Options::default());
    parser.feed(bytes);
    parser.finish()
}

pub fn render_html(html: &str, width: f32, fonts: &mut Fonts) -> Page {
    let mut parser = Parser::new(Document::new(), Options::default());
    parser.feed_str(html);
    Page::new(parser.finish(), width, fonts)
}

#[cfg(feature = "raster")]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(feature = "raster")]
pub fn render_file(path: &std::path::Path, width: u16, height: u16) -> std::io::Result<Pixels> {
    thread_local! {
        static FONTS: std::cell::RefCell<Fonts> = std::cell::RefCell::new(Fonts::new());
    }
    let bytes = std::fs::read(path)?;
    let document = parse(&bytes);
    let rgba = FONTS.with_borrow_mut(|fonts| {
        Page::new(document, f32::from(width), fonts)
            .display_list
            .rasterize(0.0, width, height)
    });
    Ok(Pixels {
        width: u32::from(width),
        height: u32::from(height),
        rgba,
    })
}

impl Page {
    pub fn new(document: Document, width: f32, fonts: &mut Fonts) -> Self {
        let styles = Styles::compute(&document);
        let boxes = BoxTree::build(&document, &styles, fonts);
        let fragments = boxes.layout(width, fonts);
        let display_list = DisplayList::build(&fragments, fonts, width);
        Page {
            document,
            styles,
            boxes,
            fragments,
            display_list,
        }
    }

    pub fn resize(&mut self, width: f32, fonts: &mut Fonts) {
        self.fragments = self.boxes.layout(width, fonts);
        self.display_list = DisplayList::build(&self.fragments, fonts, width);
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        let node = self.display_list.hit_test(x, y)?;
        self.document
            .ancestors(node)
            .find(|&ancestor| self.document.html_element_name(ancestor) == Some(Atom::A))
            .and_then(|link| self.document.attribute(link, Atom::HREF))
    }

    pub fn title(&self) -> Option<String> {
        let title = self.find_element(Atom::TITLE)?;
        let text: String = self
            .document
            .children(title)
            .filter_map(|child| self.document.text(child))
            .collect();
        Some(text.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
    }

    fn find_element(&self, name: Atom) -> Option<NodeId> {
        let mut stack = vec![NodeId::DOCUMENT];
        while let Some(node) = stack.pop() {
            if self.document.html_element_name(node) == Some(name) {
                return Some(node);
            }
            let start = stack.len();
            stack.extend(self.document.children(node));
            stack[start..].reverse();
        }
        None
    }

    pub fn memory(&self) -> Memory {
        Memory {
            elements: self.document.elements(),
            dom: self.document.heap_bytes(),
            styles: self.styles.heap_bytes(),
            boxes: self.boxes.heap_bytes(),
            fragments: self.fragments.heap_bytes(),
            display_list: self.display_list.heap_bytes(),
        }
    }
}
