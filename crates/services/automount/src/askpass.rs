use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

pub const SOCKET_VARIABLE: &str = "BANDE_ASKPASS";

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Asked {
    pub password: bool,
    pub host_key: Option<String>,
}

#[derive(Default)]
struct Answers {
    password: Option<String>,
    trusted_key: String,
    asked: Asked,
}

impl Answers {
    fn answer(&mut self, prompt: &str) -> Option<String> {
        if prompt.contains("(yes/no") {
            let key = fingerprint(prompt)?;
            let trusted = key == self.trusted_key;
            self.asked.host_key = Some(key);
            return trusted.then(|| "yes".to_owned());
        }
        self.asked.password = true;
        self.password.clone()
    }
}

fn fingerprint(prompt: &str) -> Option<String> {
    prompt
        .split_whitespace()
        .find(|word| word.starts_with("SHA256:"))
        .map(|word| word.trim_end_matches('.').to_owned())
}

pub struct Askpass {
    socket: PathBuf,
    answers: Arc<Mutex<Answers>>,
    listener: UnixListener,
}

impl Askpass {
    pub fn start(folder: &Path, password: Option<String>, trusted_key: &str) -> io::Result<Self> {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket = folder.join(format!("askpass-{}-{number}", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        let answers = Arc::new(Mutex::new(Answers {
            password,
            trusted_key: trusted_key.to_owned(),
            asked: Asked::default(),
        }));
        let accepting = listener.try_clone()?;
        let shared = Arc::clone(&answers);
        thread::Builder::new()
            .name("askpass".to_owned())
            .spawn(move || {
                for stream in accepting.incoming() {
                    let Ok(stream) = stream else {
                        return;
                    };
                    if serve(stream, &shared).is_err() {
                        tracing::debug!("an askpass client went away");
                    }
                }
            })?;
        Ok(Self {
            socket,
            answers,
            listener,
        })
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn asked(&self) -> Asked {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .asked
            .clone()
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        let _ = rustix::net::shutdown(&self.listener, rustix::net::Shutdown::Both);
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn serve(mut stream: UnixStream, answers: &Mutex<Answers>) -> io::Result<()> {
    let mut prompt = String::new();
    stream.read_to_string(&mut prompt)?;
    let answer = answers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .answer(&prompt);
    if let Some(answer) = answer {
        stream.write_all(answer.as_bytes())?;
    }
    Ok(())
}

pub fn answer_prompt(prompt: &str) -> io::Result<bool> {
    let socket = std::env::var_os(SOCKET_VARIABLE)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no askpass socket"))?;
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(prompt.as_bytes())?;
    stream.shutdown(Shutdown::Write)?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    if answer.is_empty() {
        return Ok(false);
    }
    println!("{answer}");
    Ok(true)
}
