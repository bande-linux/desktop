use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::os::fd::OwnedFd;
use std::process::Child;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use linebender_resource_handle::{Blob, FontData};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::{Itimerspec, TimerfdTimerFlags, Timespec, timerfd_settime};
use web_ipc::{
    Channel, Destination, FromNetwork, FromRenderer, MAX_LIST, Mapping, Message, NetworkControl,
    Patch, Reader, Size, ToRenderer, Url, Verdict,
};
use web_paint::{DisplayList, Image, Item, Rect};

use crate::{
    Clear, Command, Frame, Helpers, Links, LoadId, Outcome, PausedRequest, Placement, Session,
    SiteData, Update, ViewId,
};

const HOST: u32 = 0;
const MAX_FONTS: usize = 1024;
const MAX_FONT_BYTES: usize = 64 << 20;
const MAX_IMAGE_SIDE: u32 = 1 << 15;
const MAX_IMAGE_BYTES: usize = 512 << 20;
const MAX_TITLE: usize = 4096;
const MAX_SITE_DATA: usize = 16 << 20;
const RESTART_GAP: Duration = Duration::from_secs(5);

struct Network {
    channel: Channel,
    child: Child,
}

impl Drop for Network {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Renderer {
    channel: Channel,
    child: Child,
    site: String,
    load: LoadId,
    url: Url,
    fonts: Vec<FontData>,
    images: Vec<Arc<Image>>,
    image_bytes: usize,
    shown: Option<(Arc<DisplayList>, Arc<Links>)>,
    commands: u64,
    done: u64,
    deadline: Option<Instant>,
    reported: bool,
    hang: Duration,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct View {
    viewport: Size,
    session: Session,
    client: u32,
    navigation: Option<u64>,
    renderer: Option<Renderer>,
}

struct Navigation {
    view: ViewId,
    load: LoadId,
    url: Url,
}

enum Source {
    Commands,
    Timer,
    Network,
    Renderer(ViewId),
}

pub(crate) struct Broker {
    commands: Receiver<Command>,
    signal: Arc<OwnedFd>,
    updates: Sender<Update>,
    ready: Arc<OwnedFd>,
    helpers: Helpers,
    placement: Placement,
    timer: OwnedFd,
    network: Option<Network>,
    network_started: Option<Instant>,
    intercept: bool,
    site_requests: HashMap<u64, ViewId>,
    views: HashMap<ViewId, View>,
    navigations: HashMap<u64, Navigation>,
    next_fetch: u64,
    next_client: u32,
}

fn session_code(session: Session) -> u32 {
    match session {
        Session::Default => 0,
        Session::Private(number) => number.saturating_add(1),
    }
}

fn site(url: &Url) -> String {
    web_net::Site::of(url)
        .map(|site| site.as_str())
        .unwrap_or_else(|| url.origin().ascii_serialization())
}

fn interest(channel: &Channel) -> PollFlags {
    match channel.wants_write() {
        true => PollFlags::IN | PollFlags::OUT,
        false => PollFlags::IN,
    }
}

fn invalid() -> io::Error {
    io::Error::from(io::ErrorKind::InvalidData)
}

impl Broker {
    pub(crate) fn new(
        commands: Receiver<Command>,
        signal: Arc<OwnedFd>,
        updates: Sender<Update>,
        ready: Arc<OwnedFd>,
        helpers: Helpers,
        placement: Placement,
        timer: OwnedFd,
    ) -> Self {
        Self {
            commands,
            signal,
            updates,
            ready,
            helpers,
            placement,
            timer,
            network: None,
            network_started: None,
            intercept: false,
            site_requests: HashMap::new(),
            views: HashMap::new(),
            navigations: HashMap::new(),
            next_fetch: 0,
            next_client: HOST,
        }
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        loop {
            let mut sources = vec![Source::Commands, Source::Timer];
            if self.network.is_some() {
                sources.push(Source::Network);
            }
            sources.extend(
                self.views
                    .iter()
                    .filter(|(_, view)| view.renderer.is_some())
                    .map(|(id, _)| Source::Renderer(*id)),
            );
            let mut fds: Vec<PollFd<'_>> = sources
                .iter()
                .filter_map(|source| match source {
                    Source::Commands => Some(PollFd::new(&*self.signal, PollFlags::IN)),
                    Source::Timer => Some(PollFd::new(&self.timer, PollFlags::IN)),
                    Source::Network => self
                        .network
                        .as_ref()
                        .map(|network| PollFd::new(&network.channel, interest(&network.channel))),
                    Source::Renderer(view) => self
                        .views
                        .get(view)
                        .and_then(|view| view.renderer.as_ref())
                        .map(|renderer| {
                            PollFd::new(&renderer.channel, interest(&renderer.channel))
                        }),
                })
                .collect();
            match poll(&mut fds, None) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
            let ready: Vec<bool> = fds.iter().map(|fd| !fd.revents().is_empty()).collect();
            drop(fds);
            for (source, ready) in sources.iter().zip(ready) {
                if !ready {
                    continue;
                }
                match source {
                    Source::Commands => {
                        crate::clear(&self.signal);
                        if !self.take_commands() {
                            return Ok(());
                        }
                    }
                    Source::Timer => self.expire()?,
                    Source::Network => {
                        if let Err(error) = self.read_network() {
                            eprintln!("web-host: the network process failed: {error}");
                            self.lose_network();
                        }
                    }
                    Source::Renderer(view) => {
                        if self.read_renderer(*view).is_err() {
                            self.crash(*view);
                        }
                    }
                }
            }
        }
    }

    fn send(&self, view: ViewId, load: LoadId, outcome: Outcome) {
        let update = Update {
            view,
            load,
            outcome,
        };
        if self.updates.send(update).is_ok() {
            crate::signal(&self.ready);
        }
    }

    fn take_commands(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Quit) | Err(TryRecvError::Disconnected) => return false,
                Ok(command) => self.command(command),
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    fn network(&mut self) -> io::Result<&mut Network> {
        if self.network.is_none() {
            self.start_network()?;
        }
        self.network.as_mut().ok_or_else(invalid)
    }

    fn start_network(&mut self) -> io::Result<()> {
        let (ours, theirs) = Channel::pair()?;
        let child = self.helpers.network.launch(theirs.into_fd(), None)?;
        let mut network = Network {
            channel: ours,
            child,
        };
        if self.intercept {
            let enabled = true;
            network
                .channel
                .send(&NetworkControl::Intercept { enabled }, Vec::new())?;
        }
        self.network = Some(network);
        self.network_started = Some(Instant::now());
        let connected: Vec<(ViewId, u32, u32, String)> = self
            .views
            .iter()
            .filter_map(|(id, view)| {
                let renderer = view.renderer.as_ref()?;
                let session = session_code(view.session);
                Some((*id, view.client, session, renderer.site.clone()))
            })
            .collect();
        for (view, client, session, site) in connected {
            let fd = self.connect(client, session, &site)?;
            self.tell_renderer(view, ToRenderer::Network, vec![fd]);
        }
        Ok(())
    }

    fn connect(&mut self, client: u32, session: u32, site: &str) -> io::Result<OwnedFd> {
        let (network_side, renderer_side) = Channel::pair()?;
        let site = site.to_owned();
        let connect = NetworkControl::Connect {
            client,
            session,
            site,
        };
        self.network()?
            .channel
            .send(&connect, vec![network_side.into_fd()])?;
        Ok(renderer_side.into_fd())
    }

    fn tell_network(&mut self, control: NetworkControl, fds: Vec<OwnedFd>) {
        let sent = self
            .network()
            .and_then(|network| network.channel.send(&control, fds));
        if let Err(error) = sent {
            eprintln!("web-host: cannot reach the network process: {error}");
            self.lose_network();
        }
    }

    fn tell_renderer(&mut self, view: ViewId, command: ToRenderer, fds: Vec<OwnedFd>) {
        let sent = self
            .views
            .get_mut(&view)
            .and_then(|view| view.renderer.as_mut())
            .map(|renderer| {
                renderer.channel.send(&command, fds)?;
                renderer.commands += 1;
                if renderer.deadline.is_none() && !renderer.reported {
                    renderer.deadline = Some(Instant::now() + renderer.hang);
                }
                Ok::<(), io::Error>(())
            });
        match sent {
            Some(Err(_)) => self.crash(view),
            Some(Ok(())) => self.arm(),
            None => {}
        }
    }

    fn arm(&mut self) {
        let next = self
            .views
            .values()
            .filter_map(|view| view.renderer.as_ref()?.deadline)
            .min();
        let wait = next.map(|deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1))
        });
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: wait.map_or(0, |wait| wait.as_secs() as i64),
                tv_nsec: wait.map_or(0, |wait| i64::from(wait.subsec_nanos())),
            },
        };
        if let Err(error) = timerfd_settime(&self.timer, TimerfdTimerFlags::empty(), &spec) {
            eprintln!("web-host: cannot arm the hang deadline: {error}");
        }
    }

    fn expire(&mut self) -> io::Result<()> {
        crate::clear(&self.timer);
        let now = Instant::now();
        let mut hung = Vec::new();
        for (id, view) in &mut self.views {
            let Some(renderer) = view.renderer.as_mut() else {
                continue;
            };
            if renderer.deadline.is_some_and(|deadline| deadline <= now) {
                renderer.deadline = None;
                renderer.reported = true;
                hung.push((*id, renderer.load));
            }
        }
        for (view, load) in hung {
            self.send(view, load, Outcome::Unresponsive);
        }
        self.arm();
        Ok(())
    }

    fn lose_network(&mut self) {
        self.network = None;
        let requests: Vec<ViewId> = self.site_requests.drain().map(|(_, view)| view).collect();
        for view in requests {
            self.send(view, LoadId(0), Outcome::SiteData(Vec::new()));
        }
        let recently = self
            .network_started
            .is_some_and(|started| started.elapsed() < RESTART_GAP);
        let navigations: Vec<Navigation> = self.navigations.drain().map(|(_, nav)| nav).collect();
        for navigation in navigations {
            if let Some(view) = self.views.get_mut(&navigation.view) {
                view.navigation = None;
            }
            let message = "the network process stopped".to_owned();
            let url = navigation.url;
            self.send(
                navigation.view,
                navigation.load,
                Outcome::Failed { url, message },
            );
        }
        if !recently && let Err(error) = self.start_network() {
            eprintln!("web-host: cannot restart the network process: {error}");
            self.network = None;
        }
    }

    fn crash(&mut self, view: ViewId) {
        let Some(renderer) = self
            .views
            .get_mut(&view)
            .and_then(|view| view.renderer.take())
        else {
            return;
        };
        let load = renderer.load;
        let url = renderer.url.clone();
        drop(renderer);
        self.send(view, load, Outcome::Crashed { url });
    }

    fn cancel_navigation(&mut self, view: ViewId) {
        let pending = self
            .views
            .get_mut(&view)
            .and_then(|view| view.navigation.take());
        if let Some(id) = pending {
            self.navigations.remove(&id);
            self.tell_network(NetworkControl::Cancel { id }, Vec::new());
        }
    }

    fn end_unused_session(&mut self, session: Session) {
        if session == Session::Default || self.views.values().any(|view| view.session == session) {
            return;
        }
        let session = session_code(session);
        self.tell_network(NetworkControl::EndSession { session }, Vec::new());
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Load {
                view,
                load,
                url,
                viewport,
                session,
            } => {
                self.cancel_navigation(view);
                self.tell_renderer(view, ToRenderer::Stop, Vec::new());
                let entry = self.views.entry(view).or_insert(View {
                    viewport,
                    session,
                    client: HOST,
                    navigation: None,
                    renderer: None,
                });
                entry.viewport = viewport;
                let previous = entry.session;
                if previous != session {
                    entry.session = session;
                    entry.renderer = None;
                    self.end_unused_session(previous);
                }
                self.next_fetch += 1;
                let id = self.next_fetch;
                if let Some(entry) = self.views.get_mut(&view) {
                    entry.navigation = Some(id);
                }
                self.navigations.insert(
                    id,
                    Navigation {
                        view,
                        load,
                        url: url.clone(),
                    },
                );
                let session = session_code(session);
                let destination = Destination::Document;
                let fetch = NetworkControl::Fetch {
                    id,
                    session,
                    url,
                    destination,
                };
                self.tell_network(fetch, Vec::new());
            }
            Command::Resize { view, viewport } => {
                if let Some(entry) = self.views.get_mut(&view) {
                    entry.viewport = viewport;
                    self.tell_renderer(view, ToRenderer::Resize { viewport }, Vec::new());
                }
            }
            Command::Hover { view, load, x, y } => {
                let load = load.0;
                self.tell_renderer(view, ToRenderer::Hover { load, x, y }, Vec::new());
            }
            Command::Fragment { view, load, name } => {
                let load = load.0;
                self.tell_renderer(view, ToRenderer::Fragment { load, name }, Vec::new());
            }
            Command::Stop { view } => {
                self.cancel_navigation(view);
                self.tell_renderer(view, ToRenderer::Stop, Vec::new());
            }
            Command::Close { view } => {
                self.cancel_navigation(view);
                if let Some(closed) = self.views.remove(&view) {
                    self.end_unused_session(closed.session);
                }
            }
            Command::Intercept(enabled) => {
                self.intercept = enabled;
                self.tell_network(NetworkControl::Intercept { enabled }, Vec::new());
            }
            Command::ClearData { session, clear } => {
                let Clear {
                    since,
                    cookies,
                    cache,
                    sites,
                } = clear;
                let session = session_code(session);
                let clear = NetworkControl::Clear {
                    session,
                    since,
                    cookies,
                    cache,
                    sites,
                };
                self.tell_network(clear, Vec::new());
            }
            Command::SiteData { view, session } => {
                self.next_fetch += 1;
                let request = self.next_fetch;
                self.site_requests.insert(request, view);
                let session = session_code(session);
                self.tell_network(NetworkControl::SiteData { request, session }, Vec::new());
            }
            Command::Wait { view } => {
                if let Some(renderer) = self
                    .views
                    .get_mut(&view)
                    .and_then(|view| view.renderer.as_mut())
                    .filter(|renderer| renderer.done < renderer.commands)
                {
                    renderer.deadline = Some(Instant::now() + renderer.hang);
                }
                self.arm();
            }
            Command::Kill { view } => self.crash(view),
            Command::Resume { request, verdict } => {
                self.tell_network(NetworkControl::Resume { request, verdict }, Vec::new());
            }
            Command::Quit => {}
        }
    }

    fn read_network(&mut self) -> io::Result<()> {
        loop {
            let Some(network) = self.network.as_mut() else {
                return Ok(());
            };
            network.channel.flush()?;
            let Some(mut message) = network.channel.receive()? else {
                return Ok(());
            };
            match message.decode::<FromNetwork>()? {
                FromNetwork::Fetched {
                    id,
                    url,
                    content_type,
                    html,
                    charset,
                    ..
                } => {
                    let body = message.fd()?;
                    self.navigated(id, url, content_type, html, charset, body);
                }
                FromNetwork::Failed { id, message } => {
                    if let Some(navigation) = self.navigations.remove(&id) {
                        if let Some(view) = self.views.get_mut(&navigation.view) {
                            view.navigation = None;
                        }
                        let url = navigation.url;
                        let outcome = Outcome::Failed { url, message };
                        self.send(navigation.view, navigation.load, outcome);
                    }
                }
                FromNetwork::Paused {
                    request,
                    client,
                    id,
                    url,
                    destination,
                } => self.paused(request, client, id, url, destination),
                FromNetwork::SiteData { request } => {
                    let fd = message.fd()?;
                    let mapping = Mapping::new(&fd, MAX_SITE_DATA)?;
                    let mut reader = Reader::new(mapping.bytes());
                    let sites: Vec<(String, (u64, u64))> = reader.get()?;
                    reader.finish()?;
                    let sites = sites
                        .into_iter()
                        .map(|(site, (cookies, cache_bytes))| SiteData {
                            site,
                            cookies: cookies as usize,
                            cache_bytes,
                        })
                        .collect();
                    if let Some(view) = self.site_requests.remove(&request) {
                        self.send(view, LoadId(0), Outcome::SiteData(sites));
                    }
                }
            }
        }
    }

    fn paused(&mut self, request: u64, client: u32, id: u64, url: Url, destination: Destination) {
        let owner = match client {
            HOST => self
                .navigations
                .get(&id)
                .map(|navigation| (navigation.view, navigation.load)),
            client => self.views.iter().find_map(|(view_id, view)| {
                let renderer = view.renderer.as_ref().filter(|_| view.client == client)?;
                Some((*view_id, renderer.load))
            }),
        };
        match owner {
            Some((view, load)) => {
                let paused = PausedRequest {
                    request,
                    url,
                    destination,
                };
                self.send(view, load, Outcome::Paused(paused));
            }
            None => {
                let verdict = Verdict::Continue;
                self.tell_network(NetworkControl::Resume { request, verdict }, Vec::new());
            }
        }
    }

    fn navigated(
        &mut self,
        id: u64,
        url: Url,
        content_type: Option<String>,
        html: bool,
        charset: Option<String>,
        body: OwnedFd,
    ) {
        let Some(navigation) = self.navigations.remove(&id) else {
            return;
        };
        let Navigation { view, load, .. } = navigation;
        let Some(entry) = self.views.get_mut(&view) else {
            return;
        };
        entry.navigation = None;
        if let Some(kind) = content_type.filter(|_| !html) {
            return self.send(view, load, Outcome::Unsupported { url, kind });
        }
        let site = site(&url);
        let reusable = entry
            .renderer
            .as_ref()
            .is_some_and(|renderer| renderer.site == site);
        if !reusable {
            entry.renderer = None;
            if let Err(error) = self.launch(view, &site, &url, load) {
                let message = format!("cannot start a renderer: {error}");
                return self.send(view, load, Outcome::Failed { url, message });
            }
        }
        let Some(entry) = self.views.get_mut(&view) else {
            return;
        };
        let viewport = entry.viewport;
        if let Some(renderer) = entry.renderer.as_mut() {
            renderer.load = load;
            renderer.url = url.clone();
            renderer.images.clear();
            renderer.image_bytes = 0;
            renderer.shown = None;
        }
        let commit = ToRenderer::Commit {
            load: load.0,
            url,
            charset,
            viewport,
        };
        self.tell_renderer(view, commit, vec![body]);
    }

    fn launch(&mut self, view: ViewId, site: &str, url: &Url, load: LoadId) -> io::Result<()> {
        let Some(session) = self.views.get(&view).map(|view| session_code(view.session)) else {
            return Ok(());
        };
        let (ours, theirs) = Channel::pair()?;
        self.next_client = self.next_client.wrapping_add(1).max(1);
        let client = self.next_client;
        let network = self.connect(client, session, site)?;
        let child = self
            .helpers
            .renderer
            .launch(theirs.into_fd(), Some(network))?;
        (self.placement)(child.id());
        if let Some(entry) = self.views.get_mut(&view) {
            entry.client = client;
            entry.renderer = Some(Renderer {
                channel: ours,
                child,
                site: site.to_owned(),
                load,
                url: url.clone(),
                fonts: Vec::new(),
                images: Vec::new(),
                image_bytes: 0,
                shown: None,
                commands: 0,
                done: 0,
                deadline: None,
                reported: false,
                hang: self.helpers.unresponsive_after,
            });
        }
        Ok(())
    }

    fn read_renderer(&mut self, view: ViewId) -> io::Result<()> {
        loop {
            let Some(renderer) = self
                .views
                .get_mut(&view)
                .and_then(|view| view.renderer.as_mut())
            else {
                return Ok(());
            };
            renderer.channel.flush()?;
            let Some(message) = renderer.channel.receive()? else {
                self.arm();
                return Ok(());
            };
            if let Some((load, outcome)) = renderer.message(message)? {
                self.send(view, load, outcome);
            }
        }
    }
}

impl Renderer {
    fn message(&mut self, mut message: Message) -> io::Result<Option<(LoadId, Outcome)>> {
        let current = self.load;
        match message.decode::<FromRenderer>()? {
            FromRenderer::Font { id, index } => {
                let mapping = Mapping::new(&message.fd()?, MAX_FONT_BYTES)?;
                if id as usize != self.fonts.len() || self.fonts.len() >= MAX_FONTS {
                    return Err(invalid());
                }
                let data = Blob::new(Arc::new(mapping));
                self.fonts.push(FontData::new(data, index));
                Ok(None)
            }
            FromRenderer::Image {
                load,
                id,
                width,
                height,
                natural_width,
                natural_height,
            } => {
                let fd = message.fd()?;
                if load != current.0 {
                    return Ok(None);
                }
                let length = width as usize * height as usize * 4;
                let valid = id as usize == self.images.len()
                    && width <= MAX_IMAGE_SIDE
                    && height <= MAX_IMAGE_SIDE
                    && self.image_bytes + length <= MAX_IMAGE_BYTES;
                if !valid {
                    return Err(invalid());
                }
                let mapping = Mapping::new(&fd, length)?;
                if mapping.bytes().len() != length {
                    return Err(invalid());
                }
                self.image_bytes += length;
                self.images.push(Arc::new(Image::from_premultiplied(
                    width,
                    height,
                    (natural_width, natural_height),
                    mapping.bytes().into(),
                )));
                Ok(None)
            }
            FromRenderer::Frame {
                load,
                title,
                fragment,
            } => {
                let fd = message.fd()?;
                if load != current.0 {
                    return Ok(None);
                }
                let (list, links, _) = self.patched(&fd, true)?;
                let title = title.map(|title| title.chars().take(MAX_TITLE).collect());
                let frame = Frame {
                    url: self.url.clone(),
                    title,
                    list,
                    links,
                    fragment,
                };
                Ok(Some((current, Outcome::Frame(frame))))
            }
            FromRenderer::Relaid { load } => {
                let fd = message.fd()?;
                if load != current.0 {
                    return Ok(None);
                }
                let (list, links, damage) = self.patched(&fd, false)?;
                Ok(Some((current, Outcome::Relaid(list, links, damage))))
            }
            FromRenderer::Done { commands } => {
                if commands > self.commands || commands < self.done {
                    return Err(invalid());
                }
                let progressed = commands > self.done;
                self.done = commands;
                let recovered = progressed && std::mem::take(&mut self.reported);
                self.deadline = match (self.done < self.commands, progressed) {
                    (false, _) => None,
                    (true, true) => Some(Instant::now() + self.hang),
                    (true, false) => self.deadline,
                };
                Ok(recovered.then_some((current, Outcome::Responsive)))
            }
            FromRenderer::Scroll { load, y } => {
                Ok((load == current.0).then_some((current, Outcome::Scroll(y))))
            }
        }
    }

    fn patched(
        &mut self,
        fd: &OwnedFd,
        fresh: bool,
    ) -> io::Result<(Arc<DisplayList>, Arc<Links>, Option<Rect>)> {
        let mapping = Mapping::new(fd, MAX_LIST)?;
        let patch = Patch::decode(mapping.bytes())?;
        drop(mapping);
        let base = match fresh {
            true => None,
            false => Some(self.shown.clone().ok_or_else(invalid)?),
        };
        let empty = DisplayList::default();
        let (old, old_links) = match &base {
            Some((list, links)) => (list.as_ref(), Some(links)),
            None => (&empty, None),
        };
        let areas: Vec<(Rect, u32)> = old_links
            .map(|links| {
                links
                    .areas
                    .iter()
                    .map(|(rect, target)| (*rect, *target as u32))
                    .collect()
            })
            .unwrap_or_default();
        let targets: &[Url] = old_links.map_or(&[], |links| &links.targets);
        let reframed = patch.width != old.width
            || patch.height != old.height
            || patch.background != old.background;
        let changed = Changed {
            items: (patch.items.removed_range(), patch.items.inserted_range()),
            glyphs: (patch.glyphs.removed_range(), patch.glyphs.inserted_range()),
        };
        let links_changed = patch.links.is_some();
        let painted = patch.apply(&old.items, &old.glyphs, (&areas, targets))?;
        let fonts = painted
            .fonts
            .iter()
            .map(|id| self.fonts.get(*id as usize).cloned())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        let images = painted
            .images
            .iter()
            .map(|id| self.images.get(*id as usize).cloned())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        if !painted
            .targets
            .iter()
            .all(|url| matches!(url.scheme(), "http" | "https"))
        {
            return Err(invalid());
        }
        let list = DisplayList {
            width: painted.width,
            height: painted.height,
            background: painted.background,
            items: painted.items,
            glyphs: painted.glyphs,
            fonts,
            images,
        };
        let damage = match (&base, reframed) {
            (Some(_), false) => damage(old, &list, &changed),
            _ => None,
        };
        let links = match (old_links, links_changed) {
            (Some(links), false) => Arc::clone(links),
            _ => Arc::new(Links {
                areas: painted
                    .links
                    .into_iter()
                    .map(|(rect, target)| (rect, target as usize))
                    .collect(),
                targets: painted.targets,
            }),
        };
        let list = Arc::new(list);
        self.shown = Some((Arc::clone(&list), Arc::clone(&links)));
        Ok((list, links, damage))
    }
}

struct Changed {
    items: (Range<usize>, Range<usize>),
    glyphs: (Range<usize>, Range<usize>),
}

fn union(area: Option<Rect>, rect: Rect) -> Option<Rect> {
    Some(match area {
        None => rect,
        Some(area) => {
            let x = area.x.min(rect.x);
            let y = area.y.min(rect.y);
            Rect {
                x,
                y,
                width: (area.x + area.width).max(rect.x + rect.width) - x,
                height: (area.y + area.height).max(rect.y + rect.height) - y,
            }
        }
    })
}

fn overlaps(range: &Range<u32>, changed: &Range<usize>) -> bool {
    !changed.is_empty()
        && (range.start as usize) < changed.end
        && changed.start < range.end as usize
}

fn damage(old: &DisplayList, new: &DisplayList, changed: &Changed) -> Option<Rect> {
    let mut area = None;
    let removed = old.items.get(changed.items.0.clone()).unwrap_or_default();
    let inserted = new.items.get(changed.items.1.clone()).unwrap_or_default();
    for item in removed.iter().chain(inserted) {
        area = match item {
            Item::Rect { rect, .. } | Item::Glyphs { rect, .. } | Item::Image { rect, .. } => {
                union(area, *rect)
            }
            Item::Tiles { clip, .. } => union(area, *clip),
            Item::PushClip { .. } | Item::PushOpacity { .. } | Item::Pop => return None,
        };
    }
    for (list, glyphs) in [(old, &changed.glyphs.0), (new, &changed.glyphs.1)] {
        for item in &list.items {
            if let Item::Glyphs {
                rect,
                glyphs: range,
                ..
            } = item
                && overlaps(range, glyphs)
            {
                area = union(area, *rect);
            }
        }
    }
    Some(area.unwrap_or_default())
}
