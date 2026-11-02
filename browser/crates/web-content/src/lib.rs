use std::sync::Arc;

use encoding_rs::Encoding;
use web_atoms::Atom;
use web_css::stylesheet::{CssRule, Stylesheet, parse_stylesheet};
use web_dom::{Document, NodeId};
use web_html::{Options, Parser};
use web_image::ImageCache;
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
    loaded: Vec<String>,
    imports: Vec<(String, u32)>,
    images: ImageCache,
}

const IMAGE_BUDGET: usize = 256 << 20;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Memory {
    pub elements: usize,
    pub dom: usize,
    pub styles: usize,
    pub boxes: usize,
    pub fragments: usize,
    pub display_list: usize,
    pub stylist: usize,
    pub images: usize,
}

pub fn parse_file(path: &std::path::Path) -> std::io::Result<Document> {
    let bytes = std::fs::read(path)?;
    let xml = path
        .extension()
        .is_some_and(|extension| extension == "xht" || extension == "xhtml");
    Ok(match xml {
        true => web_html::parse_xml(Document::new(), &bytes, None),
        false => parse(&bytes),
    })
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
static FONT_FILES: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();

#[cfg(feature = "raster")]
pub fn set_font_files(paths: &[std::path::PathBuf]) -> std::io::Result<()> {
    let fonts = paths
        .iter()
        .map(std::fs::read)
        .collect::<std::io::Result<Vec<_>>>()?;
    let _ = FONT_FILES.set(fonts);
    Ok(())
}

#[cfg(feature = "raster")]
pub fn render_file(path: &std::path::Path, width: u16, height: u16) -> std::io::Result<Pixels> {
    thread_local! {
        static FONTS: std::cell::RefCell<Fonts> = std::cell::RefCell::new({
            let mut fonts = Fonts::new();
            for font in FONT_FILES.get().into_iter().flatten() {
                fonts.register(font.clone());
            }
            fonts
        });
    }
    let document = parse_file(path)?;
    let directory = path.parent().unwrap_or(std::path::Path::new("."));
    let viewport = Viewport {
        width: f32::from(width),
        height: f32::from(height),
    };
    let rgba = FONTS.with_borrow_mut(|fonts| {
        let mut page = Page::with_viewport(document, viewport, fonts);
        page.load_stylesheets_from(directory, fonts);
        page.load_images_from(directory, fonts);
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
        let mut stylist = Stylist::new(document.atoms_mut(), viewport);
        let mut imports = Vec::new();
        for (index, (source, _)) in stylesheet_sources(&document).into_iter().enumerate() {
            if let Source::Inline(css) = source {
                let sheet = parse_stylesheet(&css, document.atoms_mut());
                let position = sheet_position(index);
                for (offset, url) in self::imports(&sheet).into_iter().enumerate() {
                    imports.push((url, import_position(position, offset)));
                }
                stylist.add_sheet(sheet, position);
            }
        }
        let styles = Styles::compute(&document, &stylist);
        let boxes = BoxTree::build(&document, &styles, fonts);
        let images = ImageCache::new(IMAGE_BUDGET);
        let fragments = boxes.layout((viewport.width, viewport.height), fonts, &|source| {
            images.size(source)
        });
        let display_list = Arc::new(DisplayList::build(
            &fragments,
            fonts,
            viewport.width,
            &|source| images.get(source).cloned(),
        ));
        Page {
            document,
            styles,
            boxes,
            fragments,
            display_list,
            stylist,
            viewport,
            loaded: Vec::new(),
            imports,
            images,
        }
    }

    pub fn resize(&mut self, width: f32, fonts: &mut Fonts) {
        self.viewport.width = width;
        self.restyle(fonts);
    }

    pub fn stylesheet_links(&self) -> Vec<String> {
        let mut links: Vec<String> = stylesheet_sources(&self.document)
            .into_iter()
            .filter_map(|(source, _)| match source {
                Source::Link(href) => Some(href),
                Source::Inline(_) => None,
            })
            .chain(self.imports.iter().map(|(url, _)| url.clone()))
            .filter(|href| !self.loaded.contains(href))
            .collect();
        links.dedup();
        links
    }

    fn position_of(&self, href: &str) -> Option<u32> {
        stylesheet_sources(&self.document)
            .into_iter()
            .enumerate()
            .find_map(|(index, (source, _))| match source {
                Source::Link(link) if link == href => Some(sheet_position(index)),
                _ => None,
            })
            .or_else(|| {
                self.imports
                    .iter()
                    .find(|(url, _)| url == href)
                    .map(|(_, position)| *position)
            })
    }

    pub fn load_stylesheets_from(&mut self, directory: &std::path::Path, fonts: &mut Fonts) {
        loop {
            let links: Vec<String> = self.stylesheet_links();
            if links.is_empty() {
                return;
            }
            let sheets: Vec<(String, Vec<u8>)> = links
                .into_iter()
                .map(|href| {
                    let css = read_local(directory, &href).unwrap_or_default();
                    (href, css)
                })
                .collect();
            let borrowed: Vec<(&str, &[u8])> = sheets
                .iter()
                .map(|(href, css)| (href.as_str(), css.as_slice()))
                .collect();
            self.add_stylesheets(&borrowed, fonts);
        }
    }

    pub fn image_links(&self) -> Vec<String> {
        let mut links: Vec<String> = Vec::new();
        let mut stack = vec![NodeId::DOCUMENT];
        while let Some(node) = stack.pop() {
            let source = match self.document.html_element_name(node) {
                Some(Atom::IMG) => self.document.attribute(node, Atom::SRC).map(str::to_owned),
                _ => None,
            };
            let background = self
                .styles
                .get(node)
                .filter(|_| self.document.kind(node) == Some(web_dom::NodeKind::Element))
                .and_then(|style| style.reset.background_image.as_deref().map(str::to_owned));
            for link in source.into_iter().chain(background) {
                if !self.images.contains(&link) && !links.contains(&link) {
                    links.push(link);
                }
            }
            stack.extend(self.document.children(node));
        }
        links
    }

    pub fn add_images(&mut self, images: &[(&str, &[u8])], fonts: &mut Fonts) {
        for &(source, bytes) in images {
            self.images.insert(source, bytes);
        }
        self.relayout(fonts);
    }

    pub fn load_images_from(&mut self, directory: &std::path::Path, fonts: &mut Fonts) {
        let images: Vec<(String, Vec<u8>)> = self
            .image_links()
            .into_iter()
            .map(|source| {
                let bytes = read_local(directory, &source).unwrap_or_default();
                (source, bytes)
            })
            .collect();
        if images.is_empty() {
            return;
        }
        let borrowed: Vec<(&str, &[u8])> = images
            .iter()
            .map(|(source, bytes)| (source.as_str(), bytes.as_slice()))
            .collect();
        self.add_images(&borrowed, fonts);
    }

    pub fn add_stylesheet(&mut self, href: &str, bytes: &[u8], fonts: &mut Fonts) {
        self.add_stylesheets(&[(href, bytes)], fonts);
    }

    pub fn add_stylesheets(&mut self, sheets: &[(&str, &[u8])], fonts: &mut Fonts) {
        for &(href, bytes) in sheets {
            if self.loaded.iter().any(|loaded| loaded == href) {
                continue;
            }
            let position = self.position_of(href);
            self.loaded.push(href.to_owned());
            let Some(position) = position else {
                continue;
            };
            let bytes = bytes
                .get(..bytes.len().min(STYLESHEET_LIMIT))
                .unwrap_or_default();
            let (css, _) = web_css::decode_stylesheet(bytes, None, None);
            let mut sheet = parse_stylesheet(&css, self.document.atoms_mut());
            drop(css);
            rebase_urls(&mut sheet.rules, href);
            for (offset, url) in imports(&sheet).into_iter().enumerate() {
                let url = resolve_relative(href, &url);
                if !self.imports.iter().any(|(known, _)| *known == url) {
                    self.imports.push((url, import_position(position, offset)));
                }
            }
            self.stylist.add_sheet(sheet, position);
        }
        self.restyle(fonts);
    }

    fn restyle(&mut self, fonts: &mut Fonts) {
        self.display_list = Arc::default();
        self.fragments = FragmentTree::default();
        self.boxes = BoxTree::default();
        self.stylist.set_viewport(self.viewport);
        self.styles = Styles::compute(&self.document, &self.stylist);
        self.boxes = BoxTree::build(&self.document, &self.styles, fonts);
        self.relayout(fonts);
    }

    fn relayout(&mut self, fonts: &mut Fonts) {
        let images = &self.images;
        self.fragments = self.boxes.layout(
            (self.viewport.width, self.viewport.height),
            fonts,
            &|source| images.size(source),
        );
        let mut displayed: Vec<(Arc<str>, f32, f32)> = Vec::new();
        displayed_images(&self.fragments.root, &mut displayed);
        for (source, width, height) in displayed {
            self.images
                .fit(&source, width * DEVICE_SCALE, height * DEVICE_SCALE);
        }
        let images = &self.images;
        self.display_list = Arc::new(DisplayList::build(
            &self.fragments,
            fonts,
            self.viewport.width,
            &|source| images.get(source).cloned(),
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
            stylist: self.stylist.heap_bytes(),
            images: self.images.bytes(),
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
    if url.contains(':') {
        return url.to_owned();
    }
    let scheme_end = base.find("://");
    if let Some(rest) = url.strip_prefix("//") {
        return match scheme_end {
            Some(end) => format!("{}//{rest}", base.get(..=end).unwrap_or_default()),
            None => url.to_owned(),
        };
    }
    if url.starts_with('/') {
        return match scheme_end {
            Some(end) => {
                let host_end = base
                    .get(end + 3..)
                    .and_then(|rest| rest.find('/'))
                    .map_or(base.len(), |slash| end + 3 + slash);
                format!("{}{url}", base.get(..host_end).unwrap_or_default())
            }
            None => url.to_owned(),
        };
    }
    let path_end = base.find(['?', '#']).unwrap_or(base.len());
    let base = base.get(..path_end).unwrap_or(base);
    match base.rfind('/') {
        Some(slash) => format!("{}{url}", base.get(..=slash).unwrap_or_default()),
        None => url.to_owned(),
    }
}

const SHEET_SPACING: u32 = 64;

fn sheet_position(index: usize) -> u32 {
    u32::try_from(index)
        .unwrap_or(u32::MAX / SHEET_SPACING)
        .saturating_mul(SHEET_SPACING)
        .saturating_add(SHEET_SPACING / 2)
}

fn import_position(parent: u32, offset: usize) -> u32 {
    let offset = u32::try_from(offset)
        .unwrap_or(0)
        .min(SHEET_SPACING / 2 - 1);
    parent.saturating_sub(SHEET_SPACING / 2) + offset
}

const LOCAL_LIMIT: u64 = 64 << 20;

fn read_local(directory: &std::path::Path, link: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let local = link.split(['?', '#']).next().unwrap_or_default();
    let open = |path: std::path::PathBuf| {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(LOCAL_LIMIT)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(bytes)
    };
    match local.strip_prefix('/') {
        Some(rooted) => directory
            .ancestors()
            .find_map(|root| open(root.join(rooted))),
        None => open(directory.join(local)),
    }
}

fn rebase_urls(rules: &mut [CssRule], base: &str) {
    use web_css::properties::{DeclaredValue, PropertyValue};
    use web_css::values::specified::Image;
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                for declaration in &mut style.declarations.declarations {
                    if let DeclaredValue::Value(PropertyValue::ImageList(list)) =
                        &mut declaration.value
                    {
                        for image in list.0.iter_mut() {
                            if let Image::Url(url) = image {
                                *url = resolve_relative(base, url).into();
                            }
                        }
                    }
                }
                rebase_urls(&mut style.rules, base);
            }
            CssRule::Media(_, rules)
            | CssRule::Supports(_, rules)
            | CssRule::Container(_, _, rules)
            | CssRule::Layer(_, rules) => rebase_urls(rules, base),
            _ => {}
        }
    }
}

const DEVICE_SCALE: f32 = 2.0;

fn displayed_images(fragment: &BoxFragment, found: &mut Vec<(Arc<str>, f32, f32)>) {
    if let Some(source) = fragment.image() {
        let (width, height) = (fragment.rect.width, fragment.rect.height);
        match found.iter_mut().find(|(known, _, _)| known == source) {
            Some(entry) => {
                entry.1 = entry.1.max(width);
                entry.2 = entry.2.max(height);
            }
            None => found.push((source.clone(), width, height)),
        }
    }
    for child in &fragment.children {
        if let Fragment::Box(child) = child {
            displayed_images(child, found);
        }
    }
}
