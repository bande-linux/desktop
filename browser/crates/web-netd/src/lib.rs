use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};
use web_ipc::{Channel, Destination, FromNetwork, NetworkControl, ToNetwork, Url, Verdict, sealed};
use web_net::{FetchError, FetchId, Fetcher, Response, Storage};

pub const HOST: u32 = 0;
pub const STORAGE_FLAG: &str = "--storage";
const DEFAULT_SESSION: u32 = 0;
const PAUSE_DEADLINE: Duration = Duration::from_secs(10);
const MAX_CLIENTS: usize = 1024;

pub fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("web-netd: {error}");
            ExitCode::FAILURE
        }
    }
}

fn profile() -> Option<PathBuf> {
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == STORAGE_FLAG {
            return arguments.next().map(PathBuf::from);
        }
    }
    None
}

fn run() -> io::Result<()> {
    let host = Channel::new(web_ipc::inherited_control()?)?;
    let timer = timerfd_create(
        TimerfdClockId::Monotonic,
        TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
    )?;
    Network {
        host,
        timer,
        clients: HashMap::new(),
        sessions: HashMap::new(),
        running: HashMap::new(),
        intercept: false,
        paused: HashMap::new(),
        next_request: 0,
        profile: profile(),
    }
    .run()
}

struct Client {
    channel: Channel,
    session: u32,
    site: Option<Url>,
}

struct Request {
    client: u32,
    id: u64,
    session: u32,
    url: Url,
    destination: Destination,
    top_level: Option<Url>,
}

struct Paused {
    request: Request,
    deadline: Instant,
}

struct Network {
    host: Channel,
    timer: std::os::fd::OwnedFd,
    clients: HashMap<u32, Client>,
    sessions: HashMap<u32, Fetcher>,
    running: HashMap<(u32, FetchId), (u32, u64)>,
    intercept: bool,
    paused: HashMap<u64, Paused>,
    next_request: u64,
    profile: Option<PathBuf>,
}

enum Source {
    Host,
    Timer,
    Client(u32),
    Session(u32),
}

fn interest(channel: &Channel) -> PollFlags {
    match channel.wants_write() {
        true => PollFlags::IN | PollFlags::OUT,
        false => PollFlags::IN,
    }
}

impl Network {
    fn run(&mut self) -> io::Result<()> {
        loop {
            let mut sources = vec![Source::Host, Source::Timer];
            sources.extend(self.clients.keys().map(|client| Source::Client(*client)));
            sources.extend(
                self.sessions
                    .keys()
                    .map(|session| Source::Session(*session)),
            );
            let mut fds: Vec<PollFd<'_>> = sources
                .iter()
                .filter_map(|source| match source {
                    Source::Host => Some(PollFd::new(&self.host, interest(&self.host))),
                    Source::Timer => Some(PollFd::new(&self.timer, PollFlags::IN)),
                    Source::Client(client) => self
                        .clients
                        .get(client)
                        .map(|client| PollFd::new(&client.channel, interest(&client.channel))),
                    Source::Session(session) => self
                        .sessions
                        .get(session)
                        .map(|fetcher| PollFd::from_borrowed_fd(fetcher.fd(), PollFlags::IN)),
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
                    Source::Host => {
                        if !self.read_host()? {
                            return Ok(());
                        }
                    }
                    Source::Timer => self.expire()?,
                    Source::Client(client) => {
                        if self.read_client(*client).is_err() {
                            self.disconnect(*client);
                        }
                    }
                    Source::Session(session) => self.completed(*session)?,
                }
            }
        }
    }

    fn read_host(&mut self) -> io::Result<bool> {
        self.host.flush()?;
        loop {
            let mut message = match self.host.receive() {
                Ok(Some(message)) => message,
                Ok(None) => return Ok(true),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(error) => return Err(error),
            };
            match message.decode::<NetworkControl>()? {
                NetworkControl::Fetch {
                    id,
                    session,
                    url,
                    destination,
                } => self.request(Request {
                    client: HOST,
                    id,
                    session,
                    url,
                    destination,
                    top_level: None,
                })?,
                NetworkControl::Cancel { id } => self.cancel(HOST, id),
                NetworkControl::Connect {
                    client,
                    session,
                    site,
                } => {
                    let channel = Channel::new(message.fd()?)?;
                    if client != HOST && self.clients.len() < MAX_CLIENTS {
                        let site = Url::parse(&site).ok();
                        self.clients.insert(
                            client,
                            Client {
                                channel,
                                session,
                                site,
                            },
                        );
                    }
                }
                NetworkControl::EndSession { session } => {
                    self.sessions.remove(&session);
                    self.running.retain(|(running, _), _| *running != session);
                    self.paused
                        .retain(|_, paused| paused.request.session != session);
                }
                NetworkControl::Intercept { enabled } => {
                    self.intercept = enabled;
                    if !enabled {
                        let paused: Vec<Paused> =
                            self.paused.drain().map(|(_, paused)| paused).collect();
                        for paused in paused {
                            self.start(paused.request)?;
                        }
                        self.arm()?;
                    }
                }
                NetworkControl::Resume { request, verdict } => {
                    if let Some(paused) = self.paused.remove(&request) {
                        self.resume(paused.request, verdict)?;
                        self.arm()?;
                    }
                }
            }
        }
    }

    fn read_client(&mut self, client: u32) -> io::Result<()> {
        loop {
            let Some(channel) = self
                .clients
                .get_mut(&client)
                .map(|client| &mut client.channel)
            else {
                return Ok(());
            };
            channel.flush()?;
            let Some(message) = channel.receive()? else {
                return Ok(());
            };
            let (session, top_level) = self
                .clients
                .get(&client)
                .map(|client| (client.session, client.site.clone()))
                .unwrap_or_default();
            match message.decode::<ToNetwork>()? {
                ToNetwork::Fetch {
                    id,
                    destination: Destination::Document,
                    ..
                } => {
                    self.reply(
                        client,
                        &FromNetwork::Failed {
                            id,
                            message: "renderers cannot navigate".into(),
                        },
                        Vec::new(),
                    );
                }
                ToNetwork::Fetch {
                    id,
                    url,
                    destination,
                } => self.request(Request {
                    client,
                    id,
                    session,
                    url,
                    destination,
                    top_level,
                })?,
                ToNetwork::Cancel { id } => self.cancel(client, id),
            }
        }
    }

    fn disconnect(&mut self, client: u32) {
        self.clients.remove(&client);
        let running: Vec<(u32, FetchId)> = self
            .running
            .iter()
            .filter(|(_, (owner, _))| *owner == client)
            .map(|(key, _)| *key)
            .collect();
        for (session, fetch) in running {
            self.running.remove(&(session, fetch));
            if let Some(fetcher) = self.sessions.get_mut(&session) {
                fetcher.cancel(fetch);
            }
        }
        self.paused
            .retain(|_, paused| paused.request.client != client);
    }

    fn request(&mut self, request: Request) -> io::Result<()> {
        if !self.intercept {
            return self.start(request);
        }
        self.next_request += 1;
        let number = self.next_request;
        let paused = FromNetwork::Paused {
            request: number,
            client: request.client,
            id: request.id,
            url: request.url.clone(),
            destination: request.destination,
        };
        self.host.send(&paused, Vec::new())?;
        self.paused.insert(
            number,
            Paused {
                request,
                deadline: Instant::now() + PAUSE_DEADLINE,
            },
        );
        self.arm()
    }

    fn resume(&mut self, request: Request, verdict: Verdict) -> io::Result<()> {
        match verdict {
            Verdict::Continue => self.start(request),
            Verdict::Redirect { url } => self.start(Request { url, ..request }),
            Verdict::Cancel => {
                let failed = FromNetwork::Failed {
                    id: request.id,
                    message: "the request was blocked".into(),
                };
                self.reply(request.client, &failed, Vec::new());
                Ok(())
            }
        }
    }

    fn arm(&mut self) -> io::Result<()> {
        let now = Instant::now();
        let next = self.paused.values().map(|paused| paused.deadline).min();
        let value = match next {
            Some(deadline) => {
                let wait = deadline
                    .saturating_duration_since(now)
                    .max(Duration::from_millis(1));
                Timespec {
                    tv_sec: wait.as_secs() as i64,
                    tv_nsec: i64::from(wait.subsec_nanos()),
                }
            }
            None => Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
        };
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: value,
        };
        timerfd_settime(&self.timer, TimerfdTimerFlags::empty(), &spec)?;
        Ok(())
    }

    fn expire(&mut self) -> io::Result<()> {
        let mut count = [0u8; 8];
        let _ = rustix::io::read(&self.timer, &mut count);
        let now = Instant::now();
        let expired: Vec<u64> = self
            .paused
            .iter()
            .filter(|(_, paused)| paused.deadline <= now)
            .map(|(number, _)| *number)
            .collect();
        for number in expired {
            if let Some(paused) = self.paused.remove(&number) {
                self.start(paused.request)?;
            }
        }
        self.arm()
    }

    fn start(&mut self, request: Request) -> io::Result<()> {
        let storage = match (request.session, &self.profile) {
            (DEFAULT_SESSION, Some(profile)) => Storage::Disk(profile.clone()),
            _ => Storage::Memory,
        };
        let fetcher = match self.sessions.entry(request.session) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Fetcher::with_storage(storage)?)
            }
        };
        let fetch = fetcher.request(match &request.top_level {
            Some(top_level) => web_net::Request::subresource(request.url, top_level),
            None => web_net::Request::navigation(request.url, None),
        });
        self.running
            .insert((request.session, fetch), (request.client, request.id));
        Ok(())
    }

    fn cancel(&mut self, client: u32, id: u64) {
        self.paused
            .retain(|_, paused| (paused.request.client, paused.request.id) != (client, id));
        let found = self
            .running
            .iter()
            .find(|(_, owner)| **owner == (client, id))
            .map(|(key, _)| *key);
        if let Some((session, fetch)) = found {
            self.running.remove(&(session, fetch));
            if let Some(fetcher) = self.sessions.get_mut(&session) {
                fetcher.cancel(fetch);
            }
        }
    }

    fn completed(&mut self, session: u32) -> io::Result<()> {
        let Some(fetcher) = self.sessions.get_mut(&session) else {
            return Ok(());
        };
        for (fetch, result) in fetcher.completed() {
            let Some((client, id)) = self.running.remove(&(session, fetch)) else {
                continue;
            };
            let (message, fds) = reply(id, result);
            if client == HOST {
                self.host.send(&message, fds)?;
            } else {
                self.reply(client, &message, fds);
            }
        }
        Ok(())
    }

    fn reply(&mut self, client: u32, message: &FromNetwork, fds: Vec<std::os::fd::OwnedFd>) {
        if client == HOST {
            if let Err(error) = self.host.send(message, fds) {
                eprintln!("web-netd: cannot reply to the host: {error}");
            }
            return;
        }
        let sent = self
            .clients
            .get_mut(&client)
            .map(|client| client.channel.send(message, fds));
        if let Some(Err(_)) = sent {
            self.disconnect(client);
        }
    }
}

fn reply(
    id: u64,
    result: Result<Response, FetchError>,
) -> (FromNetwork, Vec<std::os::fd::OwnedFd>) {
    let failed = |message: String| (FromNetwork::Failed { id, message }, Vec::new());
    let response = match result {
        Ok(response) => response,
        Err(error) => return failed(error.to_string()),
    };
    let body = match sealed(&response.body) {
        Ok(body) => body,
        Err(error) => return failed(error.to_string()),
    };
    let content_type = response.content_type.as_ref();
    let fetched = FromNetwork::Fetched {
        id,
        url: response.url,
        status: response.status,
        content_type: content_type.map(|content_type| content_type.essence.clone()),
        html: content_type.is_some_and(|content_type| content_type.is_html()),
        charset: content_type
            .and_then(|content_type| content_type.charset)
            .map(|encoding| encoding.name().to_owned()),
    };
    (fetched, vec![body])
}
