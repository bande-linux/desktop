use std::sync::Arc;

use encoding_rs::Encoding;
use web_atoms::Atom;
use web_css::stylesheet::{CssRule, Stylesheet, parse_stylesheet};
use web_dom::{Document, NodeId};
use web_html::{Options, Parser};
use web_layout::{BoxFragment, BoxTree, Fragment, FragmentTree};
use web_paint::DisplayList;
use web_style::{Styles, Stylist};

pub use web_style::Viewport;
pub use web_text::Fonts;

pub struct Page {
    pub document: Document,
    pub styles: Styles,
    pub boxes: BoxTree,
    pub fragments: FragmentTree,
    pub display_list: Arc<DisplayList>,
    stylist: Stylist,
    viewport: Viewport,
    sheets: Vec<(String, String)>,
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
    parse_with_charset(bytes, None)
}

pub fn parse_with_charset(bytes: &[u8], charset: Option<&str>) -> Document {
    let options = Options {
        transport_encoding: charset.and_then(|label| Encoding::for_label(label.as_bytes())),
        ..Options::default()
    };
    let mut parser = Parser::new(Document::new(), options);
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
    let xml = path
        .extension()
        .is_some_and(|extension| extension == "xht" || extension == "xhtml");
    let document = match xml {
        true => web_html::parse_xml(Document::new(), &bytes, None),
        false => parse(&bytes),
    };
    let directory = path.parent().unwrap_or(std::path::Path::new("."));
    let viewport = Viewport {
        width: f32::from(width),
        height: f32::from(height),
    };
    let rgba = FONTS.with_borrow_mut(|fonts| {
        let mut page = Page::with_viewport(document, viewport, fonts);
        page.load_stylesheets_from(directory, fonts);
        page.display_list.rasterize(0.0, width, height)
    });
    Ok(Pixels {
        width: u32::from(width),
        height: u32::from(height),
        rgba,
    })
}

const STYLESHEET_LIMIT: usize = 8 << 20;

impl Page {
    pub fn new(document: Document, width: f32, fonts: &mut Fonts) -> Self {
        let viewport = Viewport {
            width,
            height: width * 0.75,
        };
        Self::with_viewport(document, viewport, fonts)
    }

    pub fn with_viewport(mut document: Document, viewport: Viewport, fonts: &mut Fonts) -> Self {
        let stylist = build_stylist(&mut document, &[], viewport);
        let styles = Styles::compute(&document, &stylist);
        let boxes = BoxTree::build(&document, &styles, fonts);
        let fragments = boxes.layout(viewport.width, fonts);
        let display_list = Arc::new(DisplayList::build(&fragments, fonts, viewport.width));
        Page {
            document,
            styles,
            boxes,
            fragments,
            display_list,
            stylist,
            viewport,
            sheets: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: f32, fonts: &mut Fonts) {
        self.viewport.width = width;
        self.restyle(fonts);
    }

    pub fn stylesheet_links(&mut self) -> Vec<String> {
        let mut links = Vec::new();
        for (source, _) in stylesheet_sources(&self.document) {
            if let Source::Link(href) = source
                && !self.sheets.iter().any(|(loaded, _)| *loaded == href)
            {
                links.push(href);
            }
        }
        for (href, css) in &self.sheets {
            let sheet = parse_stylesheet(css, self.document.atoms_mut());
            for url in imports(&sheet) {
                if !self.sheets.iter().any(|(loaded, _)| *loaded == url) && !links.contains(&url) {
                    links.push(resolve_relative(href, &url));
                }
            }
        }
        links.retain(|link| !self.sheets.iter().any(|(loaded, _)| loaded == link));
        links
    }

    pub fn load_stylesheets_from(&mut self, directory: &std::path::Path, fonts: &mut Fonts) {
        let mut attempted: Vec<String> = Vec::new();
        while let Some(href) = self
            .stylesheet_links()
            .into_iter()
            .find(|href| !attempted.contains(href))
        {
            let local = href.split(['?', '#']).next().unwrap_or_default();
            let css = match local.strip_prefix('/') {
                Some(rooted) => directory
                    .ancestors()
                    .find_map(|root| std::fs::read(root.join(rooted)).ok()),
                None => std::fs::read(directory.join(local)).ok(),
            }
            .unwrap_or_default();
            self.add_stylesheet(&href, &css, fonts);
            attempted.push(href);
        }
    }

    pub fn add_stylesheet(&mut self, href: &str, bytes: &[u8], fonts: &mut Fonts) {
        let bytes = bytes
            .get(..bytes.len().min(STYLESHEET_LIMIT))
            .unwrap_or_default();
        let (css, _) = web_css::decode_stylesheet(bytes, None, None);
        self.sheets.push((href.to_owned(), css.into_owned()));
        self.restyle(fonts);
    }

    fn restyle(&mut self, fonts: &mut Fonts) {
        self.stylist = build_stylist(&mut self.document, &self.sheets, self.viewport);
        self.styles = Styles::compute(&self.document, &self.stylist);
        self.boxes = BoxTree::build(&self.document, &self.styles, fonts);
        self.relayout(fonts);
    }

    fn relayout(&mut self, fonts: &mut Fonts) {
        self.fragments = self.boxes.layout(self.viewport.width, fonts);
        self.display_list = Arc::new(DisplayList::build(
            &self.fragments,
            fonts,
            self.viewport.width,
        ));
    }

    pub fn hover_at(&mut self, x: f32, y: f32, fonts: &mut Fonts) -> bool {
        let target = self
            .display_list
            .hit_test(x, y)
            .and_then(|node| self.document.parent(node));
        self.set_hover(target, fonts)
    }

    pub fn set_hover(&mut self, target: Option<NodeId>, fonts: &mut Fonts) -> bool {
        let changed = self.document.set_hovered(target);
        if changed.is_empty() {
            return false;
        }
        let restyle = self
            .styles
            .restyle_hover(&self.document, &self.stylist, &changed);
        if restyle.changed.is_empty() {
            return false;
        }
        if restyle.layout {
            self.boxes = BoxTree::build(&self.document, &self.styles, fonts);
        } else {
            self.boxes.refresh_styles(&self.styles);
        }
        self.relayout(fonts);
        true
    }

    pub fn restyled(&self) -> usize {
        self.styles.restyled()
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        self.link_of(self.display_list.hit_test(x, y)?)
    }

    pub fn link_of(&self, node: NodeId) -> Option<&str> {
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

    pub fn fragment_y(&self, fragment: &str) -> Option<f32> {
        let target = self.find(|node| {
            self.document.attribute(node, Atom::ID) == Some(fragment)
                || (self.document.html_element_name(node) == Some(Atom::A)
                    && self.document.attribute(node, Atom::NAME) == Some(fragment))
        })?;
        first_y(&self.fragments.root, 0.0, &|node| {
            self.document
                .ancestors(node)
                .any(|ancestor| ancestor == target)
        })
    }

    fn find_element(&self, name: Atom) -> Option<NodeId> {
        self.find(|node| self.document.html_element_name(node) == Some(name))
    }

    fn find(&self, wanted: impl Fn(NodeId) -> bool) -> Option<NodeId> {
        let mut stack = vec![NodeId::DOCUMENT];
        while let Some(node) = stack.pop() {
            if self.document.kind(node) == Some(web_dom::NodeKind::Element) && wanted(node) {
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

fn first_y(fragment: &BoxFragment, y: f32, inside: &impl Fn(NodeId) -> bool) -> Option<f32> {
    let top = y + fragment.rect.y;
    if fragment.node.is_some_and(inside) {
        return Some(top);
    }
    fragment.children.iter().find_map(|child| match child {
        Fragment::Box(child) => first_y(child, top, inside),
        Fragment::Text(text) => text
            .node
            .filter(|&node| inside(node))
            .map(|_| top + text.rect.y),
    })
}

enum Source {
    Inline(String),
    Link(String),
}

fn stylesheet_sources(document: &Document) -> Vec<(Source, NodeId)> {
    let mut sources = Vec::new();
    let mut stack = vec![NodeId::DOCUMENT];
    while let Some(node) = stack.pop() {
        match document.html_element_name(node) {
            Some(Atom::STYLE) => {
                let css: String = document
                    .children(node)
                    .filter_map(|child| document.text(child))
                    .collect();
                sources.push((Source::Inline(css), node));
            }
            Some(Atom::LINK) => {
                let rel = document.attribute(node, Atom::REL).unwrap_or_default();
                let stylesheet = rel
                    .split_ascii_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("stylesheet"));
                let alternate = rel
                    .split_ascii_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("alternate"));
                if stylesheet
                    && !alternate
                    && let Some(href) = document.attribute(node, Atom::HREF)
                {
                    sources.push((Source::Link(href.trim().to_owned()), node));
                }
            }
            _ => {}
        }
        let start = stack.len();
        stack.extend(document.children(node));
        if let Some(added) = stack.get_mut(start..) {
            added.reverse();
        }
    }
    sources
}

fn imports(sheet: &Stylesheet) -> Vec<String> {
    sheet
        .rules
        .iter()
        .filter_map(|rule| match rule {
            CssRule::Import(import) => Some(import.url.to_string()),
            _ => None,
        })
        .collect()
}

fn resolve_relative(base: &str, url: &str) -> String {
    if url.contains("://") || url.starts_with('/') {
        return url.to_owned();
    }
    match base.rfind('/') {
        Some(slash) => format!("{}{url}", base.get(..=slash).unwrap_or_default()),
        None => url.to_owned(),
    }
}

fn build_stylist(
    document: &mut Document,
    sheets: &[(String, String)],
    viewport: Viewport,
) -> Stylist {
    let sources = stylesheet_sources(document);
    let mut parsed = Vec::new();
    let mut seen = Vec::new();
    for (source, _) in sources {
        let (href, css) = match source {
            Source::Inline(css) => (None, css),
            Source::Link(href) => match sheets.iter().find(|(loaded, _)| *loaded == href) {
                Some((_, css)) => (Some(href), css.clone()),
                None => continue,
            },
        };
        add_with_imports(
            document,
            sheets,
            href.as_deref(),
            &css,
            &mut parsed,
            &mut seen,
            0,
        );
    }
    Stylist::new(document.atoms_mut(), parsed, viewport)
}

const IMPORT_DEPTH: u8 = 8;

fn add_with_imports(
    document: &mut Document,
    sheets: &[(String, String)],
    href: Option<&str>,
    css: &str,
    parsed: &mut Vec<Stylesheet>,
    seen: &mut Vec<String>,
    depth: u8,
) {
    let sheet = parse_stylesheet(css, document.atoms_mut());
    if depth < IMPORT_DEPTH {
        for url in imports(&sheet) {
            let url = href.map_or(url.clone(), |href| resolve_relative(href, &url));
            if seen.contains(&url) {
                continue;
            }
            seen.push(url.clone());
            if let Some((_, imported)) = sheets.iter().find(|(loaded, _)| *loaded == url) {
                add_with_imports(
                    document,
                    sheets,
                    Some(&url),
                    imported,
                    parsed,
                    seen,
                    depth + 1,
                );
            }
        }
    }
    parsed.push(sheet);
}
