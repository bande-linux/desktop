use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::Stdio;

use async_process::{Child, ChildStdin, ChildStdout, Command};
use futures_lite::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use oo7::Secret;
use zeroize::Zeroizing;

pub const PROGRAM: &str = "/usr/libexec/bande-secrets-prompt";
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const LONGEST_LINE: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Unlock,
    Allow,
    Create,
    UpdateLogin,
    Network,
    Reveal,
}

impl Kind {
    pub fn flag(self) -> &'static str {
        match self {
            Self::Unlock => "--unlock",
            Self::Allow => "--allow",
            Self::Create => "--create",
            Self::UpdateLogin => "--update-login",
            Self::Network => "--network",
            Self::Reveal => "--reveal",
        }
    }
}

pub enum Reply {
    Cancel,
    Gone,
    Allow(Secret),
    Always(Secret),
    CreateNew,
}

pub struct Request {
    pub kind: Kind,
    pub app: Option<String>,
    pub keychain: String,
    pub items: Vec<String>,
}

impl Request {
    fn header(&self) -> String {
        let line =
            |key: &str, value: &str| format!("{key}\t{}\n", value.replace(['\n', '\t'], " "));
        let mut header = line("keychain", &self.keychain);
        if let Some(app) = &self.app {
            header.push_str(&line("app", app));
        }
        for item in &self.items {
            header.push_str(&line("item", item));
        }
        header.push('\n');
        header
    }
}

pub struct Dialog {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

async fn session_environment(connection: &zbus::Connection) -> HashMap<String, String> {
    let reply = connection
        .call_method(
            Some(SYSTEMD),
            SYSTEMD_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(SYSTEMD_MANAGER, "Environment"),
        )
        .await;
    let Ok(reply) = reply else {
        return HashMap::new();
    };
    let Ok(value) = reply.body().deserialize::<zbus::zvariant::OwnedValue>() else {
        return HashMap::new();
    };
    let Ok(lines) = Vec::<String>::try_from(value) else {
        return HashMap::new();
    };
    lines
        .into_iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

impl Dialog {
    pub async fn open(
        program: &Path,
        connection: &zbus::Connection,
        request: &Request,
    ) -> io::Result<Self> {
        let mut command = Command::new(program);
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            command.envs(session_environment(connection).await);
        }
        let mut child = command
            .arg(request.kind.flag())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("the prompt has no input"))?;
        input.write_all(request.header().as_bytes()).await?;
        input.flush().await?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("the prompt has no output"))?;
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
        })
    }

    pub async fn reply(&mut self) -> Reply {
        let mut line = Zeroizing::new(String::new());
        let read = AsyncReadExt::take(&mut self.output, LONGEST_LINE as u64)
            .read_line(&mut line)
            .await;
        if !matches!(read, Ok(count) if count > 0) {
            return Reply::Gone;
        }
        let text = line.strip_suffix('\n').unwrap_or(&line);
        let (verb, password) = text.split_once('\t').unwrap_or((text, ""));
        match verb {
            "allow" => Reply::Allow(Secret::text(password)),
            "always" => Reply::Always(Secret::text(password)),
            "new" => Reply::CreateNew,
            _ => Reply::Cancel,
        }
    }

    pub async fn wrong(&mut self) -> io::Result<()> {
        self.input.write_all(b"wrong\n").await?;
        self.input.flush().await
    }

    pub async fn finish(mut self) {
        if self.input.write_all(b"done\n").await.is_ok() && self.input.flush().await.is_ok() {
            let _ = self.child.status().await;
        }
    }
}
