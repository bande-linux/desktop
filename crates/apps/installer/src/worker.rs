use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_packages::{Client, Progress};

use crate::deb::{self, Deb, Identity, Machine};
use crate::flow::{self, Outcome, Problem, State};

#[derive(Debug, Clone)]
pub enum Job {
    Inspect { x11_accepted: bool },
    Install(Option<String>),
    AskRemove,
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Read(Option<Deb>),
    Identity(Option<Identity>),
    State(State),
    Progress(Progress),
    Back,
    Refused,
}

pub struct Feed {
    receiver: Receiver<Message>,
    wake: UnixStream,
}

impl Feed {
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<Message> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.receiver.try_iter().collect()
    }
}

#[derive(Clone)]
struct Messages {
    sender: Sender<Message>,
    wake: Arc<UnixStream>,
}

impl Messages {
    fn send(&self, message: Message) -> bool {
        self.sender.send(message).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

fn readable_by_packagekit(path: &Path) -> bool {
    !bande_volumes::on_connected_volume(path)
        && !bande_xdg::runtime_dir().is_some_and(|runtime| path.starts_with(runtime))
}

fn staging_folder() -> io::Result<PathBuf> {
    bande_xdg::cache_home()
        .map(|cache| cache.join("bande-installer"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no cache folder"))
}

fn copier_running(copy: &Path) -> bool {
    copy.file_name()
        .and_then(|name| name.to_str()?.split_once('-'))
        .and_then(|(pid, _)| pid.parse::<u32>().ok())
        .is_some_and(|pid| Path::new("/proc").join(pid.to_string()).exists())
}

fn remove_stale_copies() {
    let entries = match staging_folder().and_then(std::fs::read_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::debug!("cannot look for old copies: {error}");
            return;
        }
    };
    for copy in entries.flatten().map(|entry| entry.path()) {
        if !copier_running(&copy)
            && let Err(error) = std::fs::remove_file(&copy)
        {
            tracing::debug!("cannot remove {}: {error}", copy.display());
        }
    }
}

fn staged_copy(path: &Path) -> io::Result<PathBuf> {
    let directory = staging_folder()?;
    std::fs::create_dir_all(&directory)?;
    let name = path.file_name().unwrap_or("package.deb".as_ref());
    let copy = directory.join(format!("{}-{}", std::process::id(), name.to_string_lossy()));
    if let Err(error) = std::fs::copy(path, &copy) {
        if let Err(removed) = std::fs::remove_file(&copy) {
            tracing::debug!("cannot remove {}: {removed}", copy.display());
        }
        return Err(error);
    }
    Ok(copy)
}

fn not_copied(error: &io::Error) -> State {
    let problem = match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => Problem::NoSpace,
        _ => Problem::Copy,
    };
    State::Failed {
        problem,
        details: error.to_string(),
    }
}

fn run(client: &Client, path: &Path, machine: &Machine, jobs: Receiver<Job>, out: &Messages) {
    remove_stale_copies();
    let mut staged = None;
    serve(client, path, &mut staged, machine, jobs, out);
    if let Some(copy) = staged
        && let Err(error) = std::fs::remove_file(&copy)
    {
        tracing::debug!("cannot remove {}: {error}", copy.display());
    }
}

fn serve(
    client: &Client,
    original: &Path,
    staged: &mut Option<PathBuf>,
    machine: &Machine,
    jobs: Receiver<Job>,
    out: &Messages,
) {
    let mut deb: Option<Deb> = None;
    let progress = |progress: Progress| {
        out.send(Message::Progress(progress));
    };
    for job in jobs {
        if let Job::Inspect { x11_accepted } = job {
            let copied = match (readable_by_packagekit(original), staged.as_ref()) {
                (true, _) => Ok(original.to_owned()),
                (false, Some(copy)) => Ok(copy.clone()),
                (false, None) => staged_copy(original).inspect(|copy| *staged = Some(copy.clone())),
            };
            let path = match &copied {
                Ok(path) => path.as_path(),
                Err(_) => original,
            };
            deb = deb::read(path);
            if !out.send(Message::Read(deb.clone())) {
                break;
            }
            let state = match &copied {
                Err(error) => {
                    tracing::warn!("cannot copy {}: {error}", original.display());
                    not_copied(error)
                }
                _ => flow::inspect(
                    client,
                    path,
                    machine,
                    deb.as_ref(),
                    x11_accepted,
                    &|progress| {
                        out.send(Message::State(State::Refreshing(progress)));
                    },
                ),
            };
            if !out.send(Message::State(state)) {
                break;
            }
            if deb.is_some() && !out.send(Message::Identity(deb::identity(path))) {
                break;
            }
            continue;
        }
        let Some(known) = deb.as_ref() else {
            continue;
        };
        let path = staged.as_deref().unwrap_or(original);
        let outcome = match job {
            Job::Inspect { .. } => continue,
            Job::Install(signed) => flow::install(client, path, signed.as_deref(), &progress),
            Job::AskRemove => Outcome::Done(flow::removal(client, known)),
            Job::Remove => flow::remove(client, known, &progress),
        };
        let message = match outcome {
            Outcome::Done(state) => Message::State(state),
            Outcome::Back => Message::Back,
            Outcome::Refused => Message::Refused,
        };
        if !out.send(message) {
            break;
        }
    }
}

pub fn start(client: Client, path: PathBuf, machine: Machine) -> io::Result<(Sender<Job>, Feed)> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    let (sender, receiver) = mpsc::channel();
    let (jobs, queue) = mpsc::channel();
    let out = Messages {
        sender,
        wake: Arc::new(writer),
    };
    std::thread::Builder::new()
        .name("installer".to_owned())
        .spawn(move || run(&client, &path, &machine, queue, &out))?;
    Ok((
        jobs,
        Feed {
            receiver,
            wake: reader,
        },
    ))
}
