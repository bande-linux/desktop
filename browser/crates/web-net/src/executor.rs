use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;
use url::Url;

use crate::fetch::{Network, fetch};
use crate::reactor::Reactor;
use crate::{Completion, FetchError, FetchId, Response};

const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
const ALPN_HTTP1: &[u8] = b"http/1.1";

pub(crate) enum Command {
    Fetch(FetchId, Url),
    Cancel(FetchId),
    Quit,
}

type Task = Pin<Box<dyn Future<Output = Result<Response, FetchError>>>>;

struct Woken {
    tasks: Mutex<Vec<FetchId>>,
    signal: Arc<OwnedFd>,
}

struct TaskWaker {
    id: FetchId,
    woken: Arc<Woken>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.id);
        signal(&self.woken.signal);
    }
}

pub(crate) fn signal(fd: &OwnedFd) {
    let _ = rustix::io::write(fd, &1u64.to_ne_bytes());
}

pub(crate) fn clear(fd: &OwnedFd) {
    let mut count = [0; 8];
    let _ = rustix::io::read(fd, &mut count);
}

pub(crate) fn tls_config() -> io::Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_platform_verifier())
        .map_err(io::Error::other)?
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN_HTTP1.to_vec()];
    Ok(Arc::new(config))
}

pub(crate) fn spawn(
    commands: Receiver<Command>,
    signal: Arc<OwnedFd>,
    completed: Sender<Completion>,
    ready: Arc<OwnedFd>,
    tls: Arc<ClientConfig>,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("web-net".into())
        .spawn(move || {
            let mut executor = Executor {
                commands,
                completed,
                ready,
                tasks: HashMap::new(),
                woken: Arc::new(Woken {
                    tasks: Mutex::new(Vec::new()),
                    signal,
                }),
            };
            if let Err(error) = executor.run(tls) {
                executor.fail_all(&error);
            }
        })
}

struct Executor {
    commands: Receiver<Command>,
    completed: Sender<Completion>,
    ready: Arc<OwnedFd>,
    tasks: HashMap<FetchId, (Task, Instant)>,
    woken: Arc<Woken>,
}

impl Executor {
    fn run(&mut self, tls: Arc<ClientConfig>) -> io::Result<()> {
        let reactor = Reactor::new(self.woken.signal.as_fd())?;
        let network = Rc::new(Network { reactor, tls });
        loop {
            clear(&self.woken.signal);
            if !self.take_commands(&network) {
                return Ok(());
            }
            self.poll_woken();
            let now = Instant::now();
            let expired: Vec<FetchId> = self
                .tasks
                .iter()
                .filter(|(_, (_, deadline))| *deadline <= now)
                .map(|(id, _)| *id)
                .collect();
            for id in expired {
                self.tasks.remove(&id);
                self.complete(id, Err(FetchError::TimedOut));
            }
            let timeout = self
                .tasks
                .values()
                .map(|(_, deadline)| deadline.saturating_duration_since(now))
                .min();
            network.reactor.wait(timeout)?;
        }
    }

    fn take_commands(&mut self, network: &Rc<Network>) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Fetch(id, url)) => {
                    let task: Task = Box::pin(fetch(Rc::clone(network), url));
                    self.tasks
                        .insert(id, (task, Instant::now() + FETCH_TIMEOUT));
                    self.lock_woken().push(id);
                }
                Ok(Command::Cancel(id)) => {
                    self.tasks.remove(&id);
                }
                Ok(Command::Quit) | Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    fn poll_woken(&mut self) {
        loop {
            let woken = std::mem::take(&mut *self.lock_woken());
            if woken.is_empty() {
                return;
            }
            for id in woken {
                let Some((task, _)) = self.tasks.get_mut(&id) else {
                    continue;
                };
                let waker = Waker::from(Arc::new(TaskWaker {
                    id,
                    woken: Arc::clone(&self.woken),
                }));
                if let Poll::Ready(result) = task.as_mut().poll(&mut Context::from_waker(&waker)) {
                    self.tasks.remove(&id);
                    self.complete(id, result);
                }
            }
        }
    }

    fn lock_woken(&self) -> std::sync::MutexGuard<'_, Vec<FetchId>> {
        self.woken
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn complete(&self, id: FetchId, result: Result<Response, FetchError>) {
        if self.completed.send((id, result)).is_ok() {
            signal(&self.ready);
        }
    }

    fn fail_all(&mut self, error: &io::Error) {
        for (id, _) in std::mem::take(&mut self.tasks) {
            self.complete(
                id,
                Err(FetchError::Connect(io::Error::new(
                    error.kind(),
                    error.to_string(),
                ))),
            );
        }
    }
}
