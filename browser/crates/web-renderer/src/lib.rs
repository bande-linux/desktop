use std::collections::HashMap;
use std::io;
use std::os::fd::OwnedFd;
use std::process::ExitCode;
use std::sync::Arc;

use rustix::event::{PollFd, PollFlags, poll};
use web_content::{Fonts, Page, Viewport};
use web_ipc::{
    Channel, Destination, FromNetwork, FromRenderer, LinkTable, Mapping, Message, Patch, Sent,
    Size, ToNetwork, ToRenderer, Url, sandbox,
};
use web_paint::{DisplayList, Image, Item, Rect};

pub const MAX_BODY: usize = 64 << 20;
const MAX_STYLESHEETS: usize = 64;
const MAX_IMAGES: usize = 256;
const MAX_FONTS: usize = 32;

pub fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("web-renderer: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    sandbox::unshare_namespaces();
    let control = web_ipc::inherited_control()?;
    let network = web_ipc::inherited_network()?;
    let mut fonts = Fonts::new();
    web_content::render_html("<p>a</p>", 100.0, &mut fonts);
    sandbox::enter(&sandbox::font_directories())?;
    let mut renderer = Renderer {
        control: Channel::new(control)?,
        network: Some(Channel::new(network)?),
        next_fetch: 0,
        fonts_sent: Vec::new(),
        images_sent: Vec::new(),
        sent: None,
        shown: None,
        styling: None,
        images: Vec::new(),
        images_arrived: Vec::new(),
        images_attempted: 0,
        web_fonts: Vec::new(),
        web_fonts_arrived: Vec::new(),
        web_fonts_attempted: 0,
        viewport: Viewport {
            width: 1.0,
            height: 1.0,
        },
        resized: false,
        hover: None,
    };
    renderer.run(&mut fonts)
}

struct Shown {
    load: u64,
    url: Url,
    page: Page,
}

struct Styling {
    shown: Shown,
    fetches: Vec<(u64, String)>,
    arrived: Vec<(String, Vec<u8>)>,
    attempted: Vec<String>,
}

struct Renderer {
    control: Channel,
    network: Option<Channel>,
    next_fetch: u64,
    fonts_sent: Vec<(u64, u32)>,
    images_sent: Vec<Arc<Image>>,
    sent: Option<(Arc<DisplayList>, LinkTable)>,
    shown: Option<Shown>,
    styling: Option<Styling>,
    images: Vec<(u64, String)>,
    images_arrived: Vec<(String, Vec<u8>)>,
    images_attempted: usize,
    web_fonts: Vec<(u64, String)>,
    web_fonts_arrived: Vec<(String, Vec<u8>)>,
    web_fonts_attempted: usize,
    viewport: Viewport,
    resized: bool,
    hover: Option<(u64, f32, f32)>,
}

fn borrowed(arrived: &[(String, Vec<u8>)]) -> Vec<(&str, &[u8])> {
    arrived
        .iter()
        .map(|(href, bytes)| (href.as_str(), bytes.as_slice()))
        .collect()
}

fn resolved(base: &Url, href: &str) -> Option<Url> {
    base.join(href.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
}

fn body(mut message: Message) -> Vec<u8> {
    message
        .fd()
        .and_then(|fd| Mapping::new(&fd, MAX_BODY))
        .map(|mapping| mapping.bytes().to_vec())
        .unwrap_or_default()
}

fn readable(channel: &Channel) -> PollFlags {
    match channel.wants_write() {
        true => PollFlags::IN | PollFlags::OUT,
        false => PollFlags::IN,
    }
}

impl Renderer {
    fn run(&mut self, fonts: &mut Fonts) -> io::Result<()> {
        loop {
            let mut sources = vec![PollFd::new(&self.control, readable(&self.control))];
            sources.extend(
                self.network
                    .as_ref()
                    .map(|network| PollFd::new(network, readable(network))),
            );
            match poll(&mut sources, None) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
            drop(sources);
            self.control.flush()?;
            while let Some(mut message) = self.control.receive()? {
                let command = message.decode::<ToRenderer>()?;
                let fd = match command {
                    ToRenderer::Commit { .. } | ToRenderer::Network => Some(message.fd()?),
                    _ => None,
                };
                self.command(command, fd, fonts)?;
            }
            if self.receive_network().is_err() {
                self.lose_network();
            }
            self.style(fonts)?;
            self.add_images(fonts)?;
            self.add_fonts(fonts)?;
            self.relayout(fonts)?;
        }
    }

    fn receive_network(&mut self) -> io::Result<()> {
        let Some(network) = self.network.as_mut() else {
            return Ok(());
        };
        network.flush()?;
        while let Some(message) = self
            .network
            .as_mut()
            .map(Channel::receive)
            .transpose()?
            .flatten()
        {
            self.fetched(message)?;
        }
        Ok(())
    }

    fn lose_network(&mut self) {
        self.network = None;
        let images = std::mem::take(&mut self.images);
        self.images_arrived
            .extend(images.into_iter().map(|(_, source)| (source, Vec::new())));
        if let Some(styling) = &mut self.styling {
            let fetches = std::mem::take(&mut styling.fetches);
            styling
                .arrived
                .extend(fetches.into_iter().map(|(_, href)| (href, Vec::new())));
        }
    }

    fn fetch(&mut self, url: Url, destination: Destination) -> Option<u64> {
        self.next_fetch += 1;
        let id = self.next_fetch;
        let fetch = ToNetwork::Fetch {
            id,
            url,
            destination,
        };
        let sent = self.network.as_mut()?.send(&fetch, Vec::new());
        match sent {
            Ok(()) => Some(id),
            Err(_) => {
                self.lose_network();
                None
            }
        }
    }

    fn cancel(&mut self, fonts: &mut Fonts) -> io::Result<()> {
        let mut styling = self.styling.take();
        if let Some(styling) = &mut styling {
            styling.shown.page.release(fonts);
        }
        let sheets = styling
            .into_iter()
            .flat_map(|styling| styling.fetches.into_iter().map(|(id, _)| id));
        let images = std::mem::take(&mut self.images)
            .into_iter()
            .map(|(id, _)| id);
        let web_fonts = std::mem::take(&mut self.web_fonts)
            .into_iter()
            .map(|(id, _)| id);
        let pending: Vec<u64> = sheets.chain(images).chain(web_fonts).collect();
        self.images_arrived.clear();
        self.web_fonts_arrived.clear();
        if let Some(network) = self.network.as_mut() {
            for id in pending {
                network.send(&ToNetwork::Cancel { id }, Vec::new())?;
            }
        }
        Ok(())
    }

    fn command(
        &mut self,
        command: ToRenderer,
        fd: Option<OwnedFd>,
        fonts: &mut Fonts,
    ) -> io::Result<()> {
        match command {
            ToRenderer::Commit {
                load,
                url,
                charset,
                viewport,
            } => {
                self.cancel(fonts)?;
                let mapping = fd
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))
                    .and_then(|fd| Mapping::new(&fd, MAX_BODY))?;
                let document = web_content::parse_with_charset(mapping.bytes(), charset.as_deref());
                drop(mapping);
                self.viewport = Viewport {
                    width: viewport.width.max(1.0),
                    height: viewport.height.max(1.0),
                };
                self.resized = false;
                self.images_attempted = 0;
                self.web_fonts_attempted = 0;
                self.images_sent.clear();
                self.sent = None;
                if let Some(mut shown) = self.shown.take() {
                    shown.page.release(fonts);
                }
                self.styling = Some(Styling {
                    shown: Shown {
                        load,
                        url,
                        page: Page::with_viewport(document, self.viewport, fonts),
                    },
                    fetches: Vec::new(),
                    arrived: Vec::new(),
                    attempted: Vec::new(),
                });
            }
            ToRenderer::Resize { viewport } => {
                let Size { width, height } = viewport;
                self.resized |= self.viewport.width != width.max(1.0);
                self.viewport = Viewport {
                    width: width.max(1.0),
                    height: height.max(1.0),
                };
            }
            ToRenderer::Hover { load, x, y } => self.hover = Some((load, x, y)),
            ToRenderer::Fragment { load, name } => {
                if let Some(shown) = self.shown.as_ref().filter(|shown| shown.load == load) {
                    let y = shown.page.fragment_y(&name).unwrap_or(0.0);
                    self.control
                        .send(&FromRenderer::Scroll { load, y }, Vec::new())?;
                }
            }
            ToRenderer::Stop => self.cancel(fonts)?,
            ToRenderer::Network => {
                let fd = fd.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
                self.network = Some(Channel::new(fd)?);
            }
        }
        Ok(())
    }

    fn fetched(&mut self, message: Message) -> io::Result<()> {
        let (id, ok) = match message.decode::<FromNetwork>()? {
            FromNetwork::Fetched { id, status, .. } => (id, (200..300).contains(&status)),
            FromNetwork::Failed { id, .. } => (id, false),
            FromNetwork::Paused { .. } => {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
        };
        let bytes = match ok {
            true => body(message),
            false => Vec::new(),
        };
        if let Some(index) = self.images.iter().position(|(pending, _)| *pending == id) {
            let (_, source) = self.images.swap_remove(index);
            self.images_arrived.push((source, bytes));
        } else if let Some(index) = self
            .web_fonts
            .iter()
            .position(|(pending, _)| *pending == id)
        {
            let (_, source) = self.web_fonts.swap_remove(index);
            self.web_fonts_arrived.push((source, bytes));
        } else if let Some(styling) = &mut self.styling
            && let Some(index) = styling
                .fetches
                .iter()
                .position(|(pending, _)| *pending == id)
        {
            let (_, href) = styling.fetches.swap_remove(index);
            styling.arrived.push((href, bytes));
        }
        Ok(())
    }

    fn style(&mut self, fonts: &mut Fonts) -> io::Result<()> {
        let Some(mut styling) = self.styling.take() else {
            return Ok(());
        };
        if !styling.fetches.is_empty() {
            self.styling = Some(styling);
            return Ok(());
        }
        if !styling.arrived.is_empty() {
            let arrived = std::mem::take(&mut styling.arrived);
            styling
                .shown
                .page
                .add_stylesheets(&borrowed(&arrived), fonts);
        }
        for href in styling.shown.page.stylesheet_links() {
            if styling.attempted.contains(&href) || styling.attempted.len() >= MAX_STYLESHEETS {
                continue;
            }
            styling.attempted.push(href.clone());
            match resolved(&styling.shown.url, &href)
                .and_then(|url| self.fetch(url, Destination::Style))
            {
                Some(id) => styling.fetches.push((id, href)),
                None => styling.arrived.push((href, Vec::new())),
            }
        }
        if !styling.fetches.is_empty() || !styling.arrived.is_empty() {
            self.styling = Some(styling);
            return Ok(());
        }
        let mut shown = styling.shown;
        if std::mem::take(&mut self.resized) {
            shown.page.resize(self.viewport.width, fonts);
        }
        self.show(shown)
    }

    fn show(&mut self, shown: Shown) -> io::Result<()> {
        let title = shown.page.title().filter(|title| !title.is_empty());
        let fragment = shown
            .url
            .fragment()
            .and_then(|name| shown.page.fragment_y(name));
        let load = shown.load;
        let list = self.painted(&shown)?;
        self.shown = Some(shown);
        self.control.send(
            &FromRenderer::Frame {
                load,
                title,
                fragment,
            },
            vec![list],
        )?;
        self.request_images()?;
        self.request_fonts()
    }

    fn request_fonts(&mut self) -> io::Result<()> {
        let Some(shown) = &self.shown else {
            return Ok(());
        };
        let base = shown.url.clone();
        for source in shown.page.font_links() {
            if self.web_fonts_attempted >= MAX_FONTS
                || self.web_fonts.iter().any(|(_, pending)| *pending == source)
                || self
                    .web_fonts_arrived
                    .iter()
                    .any(|(arrived, _)| *arrived == source)
            {
                continue;
            }
            self.web_fonts_attempted += 1;
            match resolved(&base, &source).and_then(|url| self.fetch(url, Destination::Font)) {
                Some(id) => self.web_fonts.push((id, source)),
                None => self.web_fonts_arrived.push((source, Vec::new())),
            }
        }
        Ok(())
    }

    fn add_fonts(&mut self, fonts: &mut Fonts) -> io::Result<()> {
        if self.web_fonts_arrived.is_empty() {
            return Ok(());
        }
        let arrived = std::mem::take(&mut self.web_fonts_arrived);
        let Some(mut shown) = self.shown.take() else {
            return Ok(());
        };
        shown.page.add_fonts(&borrowed(&arrived), fonts);
        drop(arrived);
        let relaid = self.relaid(&shown);
        self.shown = Some(shown);
        relaid?;
        self.request_fonts()
    }

    fn request_images(&mut self) -> io::Result<()> {
        let Some(shown) = &self.shown else {
            return Ok(());
        };
        let base = shown.url.clone();
        let sources = shown.page.image_links();
        for source in sources {
            if self.images_attempted >= MAX_IMAGES
                || self.images.iter().any(|(_, pending)| *pending == source)
            {
                continue;
            }
            self.images_attempted += 1;
            match resolved(&base, &source).and_then(|url| self.fetch(url, Destination::Image)) {
                Some(id) => self.images.push((id, source)),
                None => self.images_arrived.push((source, Vec::new())),
            }
        }
        Ok(())
    }

    fn add_images(&mut self, fonts: &mut Fonts) -> io::Result<()> {
        if self.images_arrived.is_empty() {
            return Ok(());
        }
        let arrived = std::mem::take(&mut self.images_arrived);
        let Some(mut shown) = self.shown.take() else {
            return Ok(());
        };
        shown.page.add_images(&borrowed(&arrived), fonts);
        drop(arrived);
        let relaid = self.relaid(&shown);
        self.shown = Some(shown);
        relaid
    }

    fn relayout(&mut self, fonts: &mut Fonts) -> io::Result<()> {
        let resized = std::mem::take(&mut self.resized);
        let hover = self.hover.take();
        let Some(mut shown) = self.shown.take() else {
            self.resized = resized;
            return Ok(());
        };
        let mut changed = false;
        if resized {
            shown.page.resize(self.viewport.width, fonts);
            changed = true;
        }
        if let Some((load, x, y)) = hover
            && load == shown.load
        {
            changed |= shown.page.hover_at(x, y, fonts);
        }
        let relaid = match changed {
            true => self.relaid(&shown),
            false => Ok(()),
        };
        self.shown = Some(shown);
        relaid
    }

    fn relaid(&mut self, shown: &Shown) -> io::Result<()> {
        let list = self.painted(shown)?;
        self.control
            .send(&FromRenderer::Relaid { load: shown.load }, vec![list])
    }

    fn painted(&mut self, shown: &Shown) -> io::Result<OwnedFd> {
        let list: &DisplayList = &shown.page.display_list;
        let mut fonts = Vec::with_capacity(list.fonts.len());
        for font in &list.fonts {
            let key = (font.data.id(), font.index);
            let id = match self.fonts_sent.iter().position(|sent| *sent == key) {
                Some(id) => id as u32,
                None => {
                    let id = self.fonts_sent.len() as u32;
                    let data = web_ipc::sealed(font.data.data())?;
                    self.control.send(
                        &FromRenderer::Font {
                            id,
                            index: font.index,
                        },
                        vec![data],
                    )?;
                    self.fonts_sent.push(key);
                    id
                }
            };
            fonts.push(id);
        }
        let mut images = Vec::with_capacity(list.images.len());
        for image in &list.images {
            let id = match self
                .images_sent
                .iter()
                .position(|sent| Arc::ptr_eq(sent, image))
            {
                Some(id) => id as u32,
                None => {
                    let id = self.images_sent.len() as u32;
                    let pixels = web_ipc::sealed(&image.premultiplied)?;
                    self.control.send(
                        &FromRenderer::Image {
                            load: shown.load,
                            id,
                            width: image.width,
                            height: image.height,
                            natural_width: image.natural.0,
                            natural_height: image.natural.1,
                        },
                        vec![pixels],
                    )?;
                    self.images_sent.push(Arc::clone(image));
                    id
                }
            };
            images.push(id);
        }
        let (links, targets) = links(&shown.page, &shown.url);
        let sent = Sent {
            list,
            links: &links,
            targets: &targets,
        };
        let base = self.sent.as_ref().map(|(list, (links, targets))| Sent {
            list,
            links,
            targets,
        });
        let patch = Patch::encode(base.as_ref(), &sent, &fonts, &images);
        self.sent = Some((Arc::clone(&shown.page.display_list), (links, targets)));
        web_ipc::sealed(&patch)
    }
}

fn links(page: &Page, base: &Url) -> (Vec<(Rect, u32)>, Vec<Url>) {
    let mut targets: Vec<Url> = Vec::new();
    let mut resolved = HashMap::new();
    let mut areas = Vec::new();
    for item in &page.display_list.items {
        let Item::Glyphs {
            rect,
            node: Some(node),
            ..
        } = item
        else {
            continue;
        };
        let target = *resolved.entry(*node).or_insert_with(|| {
            let url = page
                .link_of(*node)
                .and_then(|href| base.join(href.trim()).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https"))?;
            targets.push(url);
            Some(targets.len() as u32 - 1)
        });
        if let Some(target) = target {
            areas.push((*rect, target));
        }
    }
    (areas, targets)
}
